//! Application state and main layout: sidebar on the left, page on the right.

use eframe::egui::{self, RichText};

use uuid::Uuid;

use crate::{
    model::Data,
    session::{Session, SessionState},
    ssh::{
        Hop, KnownHosts, Prompt,
        forward::{ForwardEvent, ForwardHandle, ForwardSpec, spawn_forward},
    },
    store::Store,
    theme::Theme,
    ui::{self, forwards::ForwardsPage, hosts::HostsPage, session::SessionAction},
};

pub enum ForwardState {
    Connecting(String),
    /// Up and listening on `port`.
    Active {
        port: u16,
    },
    /// Ended, with the reason if it did not stop normally.
    Closed(Option<String>),
}

/// A forward that was started in this run of the app.
pub struct ActiveForward {
    pub forward_id: Uuid,
    pub state: ForwardState,
    /// Connections currently open through it.
    pub connections: usize,
    pub prompt: Option<Prompt>,
    pub secret_input: String,
    /// `None` once the forward has ended; dropping it stops the forward.
    handle: Option<ForwardHandle>,
}

impl ActiveForward {
    fn poll(&mut self) {
        let Some(handle) = &self.handle else {
            return;
        };
        let mut ended = false;
        while let Ok(event) = handle.events.try_recv() {
            match event {
                ForwardEvent::Status(text) => {
                    if matches!(self.state, ForwardState::Connecting(_)) {
                        self.state = ForwardState::Connecting(text);
                    }
                }
                ForwardEvent::Prompt(prompt) => {
                    self.secret_input.clear();
                    self.prompt = Some(prompt);
                }
                ForwardEvent::Active { port } => {
                    self.prompt = None;
                    self.state = ForwardState::Active { port };
                }
                ForwardEvent::Connections(count) => self.connections = count,
                ForwardEvent::Closed { error } => {
                    self.prompt = None;
                    self.secret_input.clear();
                    self.connections = 0;
                    self.state = ForwardState::Closed(error);
                    ended = true;
                }
            }
        }
        if ended {
            self.handle = None;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Hosts,
    Sftp,
    PortForwarding,
    Keychain,
    Settings,
    About,
    /// An open terminal session, by session id.
    Session(Uuid),
}

impl Page {
    const MAIN: [Page; 4] = [
        Page::Hosts,
        Page::Sftp,
        Page::PortForwarding,
        Page::Keychain,
    ];
    const FOOTER: [Page; 2] = [Page::Settings, Page::About];

    fn title(self) -> &'static str {
        match self {
            Page::Hosts => "Hosts",
            Page::Sftp => "SFTP",
            Page::PortForwarding => "Port Forwarding",
            Page::Keychain => "Keychain",
            Page::Settings => "Settings",
            Page::About => "About",
            Page::Session(_) => "Session",
        }
    }
}

pub struct App {
    store: Store,
    pub data: Data,
    pub theme: Theme,
    page: Page,
    hosts_page: HostsPage,
    forwards_page: ForwardsPage,
    pub active_forwards: Vec<ActiveForward>,
    /// Store error message shown at the top.
    notice: Option<String>,
    /// Runs all network work off the UI thread.
    runtime: tokio::runtime::Runtime,
    known_hosts: KnownHosts,
    pub sessions: Vec<Session>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let store = Store::open_default();
        let (data, notice) = match store.load() {
            Ok(data) => (data, None),
            Err(err) => {
                // Keep a copy of the unreadable file before it gets overwritten.
                let backup = store.path().map(|path| {
                    let backup = path.with_extension("corrupt.json");
                    let _ = std::fs::copy(path, &backup);
                    backup
                });
                let notice = match backup {
                    Some(backup) => format!(
                        "Saved data could not be read ({err}). A copy was kept at {}.",
                        backup.display()
                    ),
                    None => format!("Saved data could not be read ({err})."),
                };
                (Data::default(), Some(notice))
            }
        };
        let theme = Theme::by_name(&data.settings.theme);
        theme.apply(&cc.egui_ctx);
        let known_hosts_path = store
            .path()
            .and_then(|path| path.parent())
            .map(|dir| dir.join("known_hosts"))
            .unwrap_or_else(|| std::env::temp_dir().join("rterminalfy_known_hosts"));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime starts");
        Self {
            runtime,
            known_hosts: KnownHosts::new(known_hosts_path),
            sessions: Vec::new(),
            store,
            data,
            theme,
            page: Page::Hosts,
            hosts_page: HostsPage::default(),
            forwards_page: ForwardsPage::default(),
            active_forwards: Vec::new(),
            notice,
        }
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, theme: Theme) {
        self.theme = theme;
        self.data.settings.theme = theme.name.to_string();
        theme.apply(ctx);
        self.save();
    }

    pub fn save(&mut self) {
        if let Err(err) = self.store.save(&self.data) {
            self.notice = Some(format!("Could not save changes: {err}"));
        }
    }

    /// Open a terminal session to a saved host and switch to it.
    pub fn connect(&mut self, ctx: &egui::Context, host_id: Uuid) {
        let Some(host) = self.data.host(host_id) else {
            return;
        };
        let hops: Vec<Hop> = match self.data.connection_path(host) {
            Ok(path) => path.into_iter().map(Hop::from).collect(),
            Err(err) => {
                self.notice = Some(err.to_string());
                return;
            }
        };
        self.open_session(ctx, host_id, hops, None);
    }

    /// Start a session, optionally replacing the one at `replace`.
    fn open_session(
        &mut self,
        ctx: &egui::Context,
        host_id: Uuid,
        hops: Vec<Hop>,
        replace: Option<usize>,
    ) {
        let ctx = ctx.clone();
        let session = Session::open(
            self.runtime.handle(),
            host_id,
            hops,
            self.known_hosts.clone(),
            move || ctx.request_repaint(),
        );
        self.page = Page::Session(session.id);
        match replace {
            Some(index) => self.sessions[index] = session,
            None => self.sessions.push(session),
        }
    }

    /// Start a saved forward, replacing an earlier run of the same rule.
    pub fn start_forward(&mut self, ctx: &egui::Context, forward_id: Uuid) {
        let Some(forward) = self
            .data
            .forwards
            .iter()
            .find(|forward| forward.id == forward_id)
        else {
            return;
        };
        let bind = (forward.bind_address.clone(), forward.bind_port);
        let dest = (forward.dest_host.clone(), forward.dest_port);
        let spec = match forward.kind {
            crate::model::ForwardKind::Local => ForwardSpec::Local { bind, dest },
            crate::model::ForwardKind::Remote => ForwardSpec::Remote { bind, dest },
            crate::model::ForwardKind::Dynamic => ForwardSpec::Dynamic { bind },
        };
        let hops: Result<Vec<Hop>, String> = self
            .data
            .host(forward.host_id)
            .ok_or_else(|| "The host for this forward no longer exists.".to_string())
            .and_then(|host| {
                self.data
                    .connection_path(host)
                    .map(|path| path.into_iter().map(Hop::from).collect())
                    .map_err(|err| err.to_string())
            });
        self.stop_forward(forward_id);
        let active = match hops {
            Ok(hops) => {
                let ctx = ctx.clone();
                let handle = spawn_forward(
                    self.runtime.handle(),
                    hops,
                    self.known_hosts.clone(),
                    spec,
                    move || ctx.request_repaint(),
                );
                ActiveForward {
                    forward_id,
                    state: ForwardState::Connecting("Starting".to_string()),
                    connections: 0,
                    prompt: None,
                    secret_input: String::new(),
                    handle: Some(handle),
                }
            }
            Err(error) => ActiveForward {
                forward_id,
                state: ForwardState::Closed(Some(error)),
                connections: 0,
                prompt: None,
                secret_input: String::new(),
                handle: None,
            },
        };
        self.active_forwards.push(active);
    }

    /// Stop a forward if it is running.
    pub fn stop_forward(&mut self, forward_id: Uuid) {
        self.active_forwards
            .retain(|active| active.forward_id != forward_id);
    }

    fn close_session(&mut self, index: usize) {
        let closed = self.sessions.remove(index);
        if self.page == Page::Session(closed.id) {
            // Move to the neighbouring session, or back to Hosts.
            self.page = self
                .sessions
                .get(index.min(self.sessions.len().saturating_sub(1)))
                .map(|session| Page::Session(session.id))
                .unwrap_or(Page::Hosts);
        }
    }

    pub fn data_path(&self) -> Option<String> {
        self.store.path().map(|path| path.display().to_string())
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(">_")
                    .monospace()
                    .size(20.0)
                    .color(theme.accent),
            );
            ui.label(RichText::new(crate::APP_NAME).size(18.0).strong());
        });
        ui.add_space(14.0);

        for page in Page::MAIN {
            self.nav_item(ui, page);
        }

        if !self.sessions.is_empty() {
            ui.add_space(14.0);
            ui.label(RichText::new("SESSIONS").small().color(theme.text_dim));
            let mut close = None;
            for index in 0..self.sessions.len() {
                if self.session_item(ui, index) {
                    close = Some(index);
                }
            }
            if let Some(index) = close {
                self.close_session(index);
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("v{}", crate::APP_VERSION))
                    .small()
                    .color(theme.text_dim),
            );
            ui.add_space(4.0);
            for page in Page::FOOTER.into_iter().rev() {
                self.nav_item(ui, page);
            }
        });
    }

    /// One open session in the sidebar. Returns `true` if its close button
    /// was clicked.
    fn session_item(&mut self, ui: &mut egui::Ui, index: usize) -> bool {
        let theme = self.theme;
        let session = &self.sessions[index];
        let page = Page::Session(session.id);
        let selected = self.page == page;
        let dot = match session.state {
            SessionState::Connected => theme.accent,
            SessionState::Closed(Some(_)) => theme.danger,
            _ => theme.text_dim,
        };
        let name = session.name.clone();
        let mut close = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let close_width = 26.0;
            let text_color = if selected {
                theme.on_accent
            } else {
                theme.text
            };
            let mut button =
                egui::Button::new(RichText::new(format!("    {name}")).color(text_color))
                    .min_size(egui::vec2(ui.available_width() - close_width - 4.0, 30.0))
                    .truncate()
                    .frame_when_inactive(false);
            if selected {
                button = button.fill(theme.accent).frame_when_inactive(true);
            }
            let response = ui.add(button);
            let center = egui::pos2(response.rect.left() + 14.0, response.rect.center().y);
            ui.painter()
                .circle_filled(center, 4.0, if selected { theme.on_accent } else { dot });
            if response.clicked() {
                self.page = page;
            }
            let close_button = egui::Button::new("x")
                .min_size(egui::vec2(close_width, 30.0))
                .frame_when_inactive(false);
            if ui
                .add(close_button)
                .on_hover_text("Close session")
                .clicked()
            {
                close = true;
            }
        });
        close
    }

    fn nav_item(&mut self, ui: &mut egui::Ui, page: Page) {
        let theme = self.theme;
        let selected = self.page == page;
        let text = if selected {
            RichText::new(page.title()).color(theme.on_accent).strong()
        } else {
            RichText::new(page.title())
        };
        let mut button = egui::Button::new(text)
            .min_size(egui::vec2(ui.available_width(), 34.0))
            .frame_when_inactive(false);
        if selected {
            button = button.fill(theme.accent).frame_when_inactive(true);
        }
        if ui.add(button).clicked() {
            self.page = page;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let theme = self.theme;

        // Sessions keep receiving output while another page is shown.
        for session in &mut self.sessions {
            session.poll();
        }
        for forward in &mut self.active_forwards {
            forward.poll();
        }
        // A forward whose host was deleted cannot keep running.
        let forwards = &self.data.forwards;
        self.active_forwards.retain(|active| {
            forwards
                .iter()
                .any(|forward| forward.id == active.forward_id)
        });

        egui::Panel::left("sidebar")
            .exact_size(196.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme.panel)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| self.sidebar(ui));

        if let Some(notice) = self.notice.clone() {
            egui::Panel::top("notice")
                .frame(
                    egui::Frame::new()
                        .fill(theme.surface)
                        .inner_margin(egui::Margin::symmetric(16, 8)),
                )
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(notice).color(theme.danger));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Dismiss").clicked() {
                                self.notice = None;
                            }
                        });
                    });
                });
        }

        // The Hosts page owns its editor panel on the right.
        if self.page == Page::Hosts {
            let mut page = std::mem::take(&mut self.hosts_page);
            page.editor_panel(self, ui);
            self.hosts_page = page;
        }
        if self.page == Page::PortForwarding {
            let mut page = std::mem::take(&mut self.forwards_page);
            page.editor_panel(self, ui);
            self.forwards_page = page;
        }

        if let Page::Session(id) = self.page {
            match self.sessions.iter().position(|session| session.id == id) {
                Some(index) => match ui::session::show(self, ui, index) {
                    SessionAction::None => {}
                    SessionAction::Close => self.close_session(index),
                    SessionAction::Reconnect => {
                        let session = &self.sessions[index];
                        let (host_id, hops) = (session.host_id, session.hops.clone());
                        self.open_session(ui.ctx(), host_id, hops, Some(index));
                    }
                },
                None => self.page = Page::Hosts,
            }
            return;
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme.bg)
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(ui, |ui| match self.page {
                Page::Hosts => {
                    let mut page = std::mem::take(&mut self.hosts_page);
                    page.show(self, ui);
                    self.hosts_page = page;
                }
                Page::Settings => ui::settings::show(self, ui),
                Page::About => ui::about::show(self, ui),
                Page::Session(_) => {}
                Page::Sftp => ui::placeholder::show(
                    self,
                    ui,
                    "SFTP",
                    "Browse, upload and download files on your hosts.",
                ),
                Page::PortForwarding => {
                    let mut page = std::mem::take(&mut self.forwards_page);
                    page.show(self, ui);
                    self.forwards_page = page;
                }
                Page::Keychain => ui::placeholder::show(
                    self,
                    ui,
                    "Keychain",
                    "Generate keys and manage identities, certificates and FIDO2 keys.",
                ),
            });
    }
}
