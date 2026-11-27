//! Application state and main layout: sidebar on the left, page on the right.

use eframe::egui::{self, Color32, RichText};

use uuid::Uuid;

use crate::{
    keychain::KeyStore,
    model::{Appearance, Data},
    secrets::SecretStore,
    session::{Session, SessionState},
    ssh::{
        Hop, KnownHosts, Prompt,
        forward::{ForwardEvent, ForwardHandle, ForwardSpec, spawn_forward},
    },
    store::Store,
    theme::Theme,
    ui::{
        self,
        forwards::ForwardsPage,
        hosts::HostsPage,
        keychain::KeychainPage,
        session::SessionAction,
        sftp::SftpPage,
        split::{self, Side, Tile},
    },
};

/// A session being dragged, from its row in the sidebar or from its bar.
#[derive(Clone, Copy)]
struct SessionDrag {
    id: Uuid,
    grab: ui::Grab,
    /// Taken from a pane on the session page, so dropping it on the
    /// sidebar can give it a view of its own again.
    from_pane: bool,
}

/// Where a dragged session would land.
#[derive(Clone, Copy, PartialEq)]
enum DropTarget {
    /// Beside the pane of this session, on this side of it.
    Beside(Uuid, Side),
    /// On its own, out of the split it is in.
    Alone,
}

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
    /// Colors of the app's windows, lists and dialogs.
    pub theme: Theme,
    /// Colors of the terminal.
    pub terminal_theme: Theme,
    page: Page,
    hosts_page: HostsPage,
    forwards_page: ForwardsPage,
    sftp_page: SftpPage,
    keychain_page: KeychainPage,
    pub active_forwards: Vec<ActiveForward>,
    /// Store error message shown at the top.
    notice: Option<Notice>,
    /// Runs all network work off the UI thread.
    runtime: tokio::runtime::Runtime,
    known_hosts: KnownHosts,
    pub key_store: KeyStore,
    pub secrets: SecretStore,
    pub sessions: Vec<Session>,
    /// How open sessions are shown: each view is one or more terminals
    /// side by side. Every session is in exactly one view.
    views: Vec<Tile>,
    drag: Option<SessionDrag>,
    /// Where each pane of the session page was drawn this frame.
    pane_rects: Vec<(Uuid, egui::Rect)>,
    /// Where the session rows in the sidebar were drawn this frame.
    sessions_rect: Option<egui::Rect>,
    /// The termker logo, drawn in the sidebar and on the About page.
    pub logo: egui::TextureHandle,
}

/// The terminal theme from the settings, with the user's own text and
/// background colors applied. A color that cannot be read is ignored.
fn terminal_theme(settings: &termker_core::model::Settings) -> Theme {
    let color = |hex: &Option<String>| hex.as_deref().and_then(crate::theme::from_hex);
    Theme::terminal(&settings.terminal_theme).with_colors(
        color(&settings.terminal_text),
        color(&settings.terminal_background),
    )
}

/// The app's look for the `appearance` setting.
fn ui_theme(appearance: Appearance, ctx: &egui::Context) -> Theme {
    let dark = match appearance {
        Appearance::Dark => true,
        Appearance::Light => false,
        // Dark when the system does not say.
        Appearance::System => ctx.system_theme() != Some(egui::Theme::Light),
    };
    Theme::ui(dark)
}

/// A message shown in a bar at the top of the window until dismissed.
#[derive(Clone)]
struct Notice {
    text: String,
    /// Errors are shown in the theme's danger color.
    error: bool,
}

impl Notice {
    fn info(text: String) -> Self {
        Self { text, error: false }
    }

    fn error(text: String) -> Self {
        Self { text, error: true }
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // The app used to be called rterminalfy; bring its data along.
        let moved = match crate::migrate::from_old_name() {
            Ok(crate::migrate::Outcome::NothingToMove) => None,
            Ok(crate::migrate::Outcome::Moved {
                to,
                passwords_failed,
                ..
            }) => Some(Notice::info(if passwords_failed == 0 {
                format!(
                    "Your hosts, keys and saved passwords from {} were moved to {}.",
                    crate::migrate::OLD_APP_NAME,
                    to.display()
                )
            } else {
                format!(
                    "Your hosts and keys from {} were moved to {}. {passwords_failed} saved \
                     password(s) could not be moved and will be asked for again.",
                    crate::migrate::OLD_APP_NAME,
                    to.display()
                )
            })),
            Err(err) => Some(Notice::error(format!(
                "Data from {} could not be moved ({err}). It is still in its old folder.",
                crate::migrate::OLD_APP_NAME
            ))),
        };
        let store = Store::open_default();
        let (data, notice) = match store.load() {
            Ok(data) => (data, moved),
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
                (Data::default(), Some(Notice::error(notice)))
            }
        };
        let theme = ui_theme(data.settings.appearance, &cc.egui_ctx);
        theme.apply(&cc.egui_ctx);
        let terminal_theme = terminal_theme(&data.settings);
        let data_dir = store
            .path()
            .and_then(|path| path.parent())
            .map(|dir| dir.to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir().join(crate::APP_NAME));
        let known_hosts_path = data_dir.join("known_hosts");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime starts");
        Self {
            runtime,
            known_hosts: KnownHosts::new(known_hosts_path),
            key_store: KeyStore::new(data_dir.join("keys")),
            secrets: SecretStore::os(data_dir.join("secrets.json")),
            keychain_page: KeychainPage::default(),
            sessions: Vec::new(),
            views: Vec::new(),
            drag: None,
            pane_rects: Vec::new(),
            sessions_rect: None,
            logo: crate::icon::logo_texture(&cc.egui_ctx),
            store,
            data,
            theme,
            terminal_theme,
            page: Page::Hosts,
            hosts_page: HostsPage::default(),
            forwards_page: ForwardsPage::default(),
            sftp_page: SftpPage::default(),
            active_forwards: Vec::new(),
            notice,
        }
    }

    pub fn set_appearance(&mut self, ctx: &egui::Context, appearance: Appearance) {
        self.data.settings.appearance = appearance;
        self.theme = ui_theme(appearance, ctx);
        self.theme.apply(ctx);
        self.save();
    }

    /// Pick a terminal theme. This drops the user's own text and background
    /// colors, so the theme shows as it is.
    pub fn set_terminal_theme(&mut self, theme: Theme) {
        let settings = &mut self.data.settings;
        settings.terminal_theme = theme.name.to_string();
        settings.terminal_text = None;
        settings.terminal_background = None;
        self.terminal_theme = terminal_theme(settings);
        self.save();
    }

    /// Use the user's own text and background colors in the terminal, on
    /// top of the chosen theme. `None` goes back to the theme's color.
    pub fn set_terminal_colors(&mut self, text: Option<Color32>, background: Option<Color32>) {
        let settings = &mut self.data.settings;
        settings.terminal_text = text.map(crate::theme::to_hex);
        settings.terminal_background = background.map(crate::theme::to_hex);
        self.terminal_theme = terminal_theme(settings);
        self.save();
    }

    /// With the "System" setting, switch when the operating system does.
    fn follow_system_appearance(&mut self, ctx: &egui::Context) {
        if self.data.settings.appearance != Appearance::System {
            return;
        }
        let theme = ui_theme(Appearance::System, ctx);
        if theme != self.theme {
            self.theme = theme;
            theme.apply(ctx);
        }
    }

    pub fn save(&mut self) {
        if let Err(err) = self.store.save(&self.data) {
            self.notice = Some(Notice::error(format!("Could not save changes: {err}")));
        }
    }

    /// Connection path to `host` with its sign-in details looked up.
    pub fn hops(&self, host: &crate::model::Host) -> Result<Vec<Hop>, String> {
        crate::ssh::hops(&self.data, host, &self.key_store)
    }

    /// Show a message in the bar at the top of the window.
    pub fn notify(&mut self, message: String) {
        self.notice = Some(Notice::error(message));
    }

    /// Open a terminal session to a saved host and switch to it.
    pub fn connect(&mut self, ctx: &egui::Context, host_id: Uuid) {
        let Some(host) = self.data.host(host_id) else {
            return;
        };
        let hops = match self.hops(host) {
            Ok(hops) => hops,
            Err(err) => {
                self.notice = Some(Notice::error(err));
                return;
            }
        };
        self.open_session(ctx, host_id, hops, None);
    }

    /// Open a terminal session to a saved host, switch to it, and go to
    /// `folder` there once the shell is ready.
    pub fn connect_in(&mut self, ctx: &egui::Context, host_id: Uuid, folder: &str) {
        let before = self.sessions.len();
        self.connect(ctx, host_id);
        if self.sessions.len() > before
            && let Some(session) = self.sessions.last_mut()
        {
            session.queue_input(termker_core::shell::cd_command(folder).as_bytes());
        }
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
            self.secrets.clone(),
            move || ctx.request_repaint(),
        );
        self.page = Page::Session(session.id);
        match replace {
            Some(index) => {
                // Reconnecting keeps the pane where it was.
                let old = self.sessions[index].id;
                for view in &mut self.views {
                    view.replace(old, session.id);
                }
                self.sessions[index] = session;
            }
            None => {
                self.views.push(Tile::Pane(session.id));
                self.sessions.push(session);
            }
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
            .and_then(|host| self.hops(host));
        self.stop_forward(forward_id);
        let active = match hops {
            Ok(hops) => {
                let ctx = ctx.clone();
                let handle = spawn_forward(
                    self.runtime.handle(),
                    hops,
                    self.known_hosts.clone(),
                    self.secrets.clone(),
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

    /// Start a background SFTP session through `hops`.
    pub fn spawn_sftp(
        &self,
        hops: Vec<Hop>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> crate::sftp::SftpHandle {
        crate::sftp::spawn_sftp(
            self.runtime.handle(),
            hops,
            self.known_hosts.clone(),
            self.secrets.clone(),
            wake,
        )
    }

    /// Copy a file or folder between two places in the background.
    pub fn spawn_copy(
        &self,
        from: crate::sftp::Place,
        to: crate::sftp::Place,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> crate::sftp::CopyHandle {
        crate::sftp::spawn_copy(self.runtime.handle(), from, to, wake)
    }

    /// Stop a forward if it is running.
    pub fn stop_forward(&mut self, forward_id: Uuid) {
        self.active_forwards
            .retain(|active| active.forward_id != forward_id);
    }

    fn close_session(&mut self, index: usize) {
        let closed = self.sessions.remove(index);
        let beside = self
            .view_of(closed.id)
            .and_then(|view| view.panes().into_iter().find(|id| *id != closed.id));
        self.views = split::take_out(std::mem::take(&mut self.views), closed.id);
        if self.page == Page::Session(closed.id) {
            // Move to a terminal shown beside it, the neighbouring session,
            // or back to Hosts.
            self.page = beside
                .or_else(|| {
                    self.sessions
                        .get(index.min(self.sessions.len().saturating_sub(1)))
                        .map(|session| session.id)
                })
                .map(Page::Session)
                .unwrap_or(Page::Hosts);
        }
    }

    /// The view that shows session `id`.
    fn view_of(&self, id: Uuid) -> Option<&Tile> {
        self.views.iter().find(|view| view.contains(id))
    }

    /// The view on screen, if a session page is shown.
    fn shown_view(&self) -> Option<&Tile> {
        match self.page {
            Page::Session(id) => self.view_of(id),
            _ => None,
        }
    }

    /// Where the dragged session would land if let go now.
    fn drop_target(&self, drag: &SessionDrag, pointer: egui::Pos2) -> Option<DropTarget> {
        if let Some((id, rect)) = self
            .pane_rects
            .iter()
            .find(|(_, rect)| rect.contains(pointer))
        {
            return Some(DropTarget::Beside(*id, Side::nearest(*rect, pointer)));
        }
        let over_sidebar = self
            .sessions_rect
            .is_some_and(|rect| rect.expand(6.0).contains(pointer));
        let split = self.view_of(drag.id).is_some_and(Tile::is_split);
        (drag.from_pane && over_sidebar && split).then_some(DropTarget::Alone)
    }

    /// Put the dragged session where it was dropped.
    fn drop_session(&mut self, moved: Uuid, target: DropTarget) {
        match target {
            DropTarget::Beside(beside, _) if beside == moved => return,
            DropTarget::Beside(beside, side) => {
                let mut views = split::take_out(std::mem::take(&mut self.views), moved);
                let placed = views
                    .iter_mut()
                    .any(|view| view.insert(beside, moved, side));
                if !placed {
                    views.push(Tile::Pane(moved));
                }
                self.views = views;
            }
            DropTarget::Alone => {
                self.views = split::take_out(std::mem::take(&mut self.views), moved);
                self.views.push(Tile::Pane(moved));
            }
        }
        self.page = Page::Session(moved);
    }

    /// The dragged session under the pointer, the place it would land, and
    /// letting go of it.
    fn session_drag(&mut self, ui: &egui::Ui) {
        let Some(drag) = self.drag else {
            return;
        };
        let ctx = ui.ctx().clone();
        let (pointer, down, escape) = ctx.input(|input| {
            (
                input.pointer.latest_pos(),
                input.pointer.primary_down(),
                input.key_pressed(egui::Key::Escape),
            )
        });
        let Some(pointer) = pointer else {
            return;
        };
        if escape || !self.sessions.iter().any(|session| session.id == drag.id) {
            self.drag = None;
            return;
        }
        let theme = self.theme;
        let target = self.drop_target(&drag, pointer);
        let (hint, ok) = match target {
            Some(DropTarget::Beside(id, _)) if id == drag.id => ("It is already here", false),
            Some(DropTarget::Beside(id, side)) => {
                if let Some((_, rect)) = self.pane_rects.iter().find(|(pane, _)| *pane == id) {
                    ui::drop_outline(ui, &theme, side.half(*rect).shrink(4.0), true);
                }
                (side.hint(), true)
            }
            Some(DropTarget::Alone) => ("Drop to show it on its own", true),
            None if drag.from_pane => ("Drop on Sessions to show it on its own", false),
            None => ("Drop on a terminal to show both", false),
        };
        if drag.from_pane
            && let Some(rect) = self.sessions_rect
            && self.view_of(drag.id).is_some_and(Tile::is_split)
        {
            ui::drop_outline(
                ui,
                &theme,
                rect.expand(4.0),
                target == Some(DropTarget::Alone),
            );
        }
        if let Some(session) = self.sessions.iter().find(|session| session.id == drag.id) {
            let name = session.name.clone();
            let dot = session_dot(&theme, &session.state);
            ui::drag_float(
                &ctx,
                &theme,
                drag.grab,
                egui::vec2(10.0, 6.0),
                hint,
                ok,
                |ui| {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, dot);
                        ui.add(egui::Label::new(RichText::new(name).strong()).truncate());
                    });
                },
            );
        }
        if !down {
            self.drag = None;
            if let Some(target) = target {
                self.drop_session(drag.id, target);
            }
        }
    }

    /// The session page: every terminal of the view that holds `active`,
    /// laid out as it was split.
    fn session_page(&mut self, ui: &mut egui::Ui, active: Uuid) {
        let Some(view) = self.view_of(active).cloned() else {
            self.page = Page::Hosts;
            return;
        };
        let theme = self.theme;
        let area = ui.available_rect_before_wrap();
        ui.painter().rect_filled(area, 0.0, theme.panel);
        let (panes, dividers) = view.arrange(area, 6.0);
        let split = view.is_split();

        // Dividers first, so a drag on one does not reach the panes.
        for divider in &dividers {
            let id = egui::Id::unique(("split_divider", active)).with(&divider.path);
            let response = ui
                .interact(divider.rect, id, egui::Sense::drag())
                .on_hover_cursor(match divider.axis {
                    split::Axis::SideBySide => egui::CursorIcon::ResizeHorizontal,
                    split::Axis::Stacked => egui::CursorIcon::ResizeVertical,
                });
            if response.dragged()
                && let Some(pointer) = response.interact_pointer_pos()
                && let Some(view) = self.views.iter_mut().find(|view| view.contains(active))
                && let Some(split) = view.split_at_mut(&divider.path)
            {
                split.ratio = divider.ratio_at(pointer);
            }
            if response.hovered() || response.dragged() {
                ui.painter()
                    .rect_filled(divider.rect.shrink(1.0), 2.0, theme.accent_text);
            }
        }

        self.pane_rects = panes.clone();
        let mut activate = None;
        let mut close = None;
        let mut reconnect = None;
        for (id, rect) in panes {
            let Some(index) = self.sessions.iter().position(|session| session.id == id) else {
                continue;
            };
            let mut pane = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("pane", id))
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            pane.set_clip_rect(rect);
            let output = ui::session::show(self, &mut pane, index, id == active, split);
            if output.activate {
                activate = Some(id);
            }
            if let Some(grab) = output.grab {
                self.drag = Some(SessionDrag {
                    id,
                    grab: grab.resized(egui::vec2(220.0, 34.0)),
                    from_pane: true,
                });
            }
            match output.action {
                SessionAction::None => {}
                SessionAction::Close => close = Some(id),
                SessionAction::Reconnect => reconnect = Some(index),
            }
        }
        if let Some(id) = activate {
            self.page = Page::Session(id);
        }
        if let Some(index) = reconnect {
            let session = &self.sessions[index];
            let (host_id, hops) = (session.host_id, session.hops.clone());
            self.open_session(ui.ctx(), host_id, hops, Some(index));
        }
        if let Some(index) =
            close.and_then(|id| self.sessions.iter().position(|session| session.id == id))
        {
            self.close_session(index);
        }
    }

    pub fn data_path(&self) -> Option<String> {
        self.store.path().map(|path| path.display().to_string())
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::Image::new(&self.logo).fit_to_exact_size(egui::vec2(24.0, 24.0)));
            ui.label(RichText::new(crate::APP_NAME).size(18.0).strong());
        });
        ui.add_space(14.0);

        for page in Page::MAIN {
            self.nav_item(ui, page);
        }

        self.sessions_rect = None;
        if !self.sessions.is_empty() {
            ui.add_space(14.0);
            ui.label(RichText::new("SESSIONS").small().color(theme.text_dim));
            let top = ui.cursor().top();
            let mut close = None;
            for index in 0..self.sessions.len() {
                if self.session_item(ui, index) {
                    close = Some(index);
                }
            }
            let rows =
                egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
            self.sessions_rect = Some(rows);
            // Say that sessions can be dragged until two are shown together.
            let alone = self.shown_view().is_some_and(|view| !view.is_split());
            if self.sessions.len() > 1 && alone && self.drag.is_none() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Drag a session onto the terminal to see both at once.")
                        .small()
                        .color(theme.text_dim),
                );
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
        // Shown beside the selected one, on the same page.
        let on_screen = !selected
            && self
                .shown_view()
                .is_some_and(|view| view.contains(session.id));
        let dot = session_dot(&theme, &session.state);
        let id = session.id;
        let name = session.name.clone();
        let mut close = false;
        // The row is laid out by hand so that every row is exactly as wide
        // as the sidebar: the name starts at the same place and the close
        // button sits at the same right edge on every row.
        let close_width = 26.0;
        let (row, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 30.0),
            egui::Sense::click_and_drag(),
        );
        let response = response.on_hover_text(
            "Drag onto a terminal to show both side by side, or one above the other.",
        );
        if response.drag_started() {
            self.drag = Some(SessionDrag {
                id,
                grab: ui::Grab::new(ui, row),
                from_pane: false,
            });
        }
        let dragged = self.drag.is_some_and(|drag| drag.id == id);
        let close_rect = egui::Rect::from_min_max(
            egui::pos2(row.right() - close_width, row.top()),
            row.right_bottom(),
        );
        let close_response = ui
            .interact(close_rect, response.id.with("close"), egui::Sense::click())
            .on_hover_text("Close session");
        let name_rect = egui::Rect::from_min_max(
            row.left_top(),
            egui::pos2(close_rect.left() - 4.0, row.bottom()),
        );

        let painter = ui.painter();
        if dragged {
            painter.rect_stroke(
                name_rect,
                6.0,
                egui::Stroke::new(1.0, theme.accent_text.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        } else if selected {
            painter.rect_filled(name_rect, 6.0, theme.accent);
        } else if response.hovered() && !close_response.hovered() {
            painter.rect_filled(name_rect, 6.0, theme.border);
        } else if on_screen {
            painter.rect_filled(name_rect, 6.0, theme.accent.gamma_multiply(0.25));
        }
        if close_response.hovered() {
            painter.rect_filled(close_rect, 6.0, theme.border);
        }
        painter.circle_filled(
            egui::pos2(name_rect.left() + 14.0, name_rect.center().y),
            4.0,
            if selected { theme.on_accent } else { dot },
        );
        let selected = selected && !dragged;
        let (text_color, close_color) = if selected {
            (theme.on_accent, theme.text)
        } else {
            (theme.text, theme.text_dim)
        };
        painter.text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "x",
            egui::TextStyle::Button.resolve(ui.style()),
            if close_response.hovered() {
                theme.text
            } else {
                close_color
            },
        );
        // The name, cut short with "…" when it does not fit.
        let text_rect = egui::Rect::from_min_max(
            egui::pos2(name_rect.left() + 28.0, name_rect.top()),
            egui::pos2(name_rect.right() - 6.0, name_rect.bottom()),
        );
        let mut job = egui::text::LayoutJob::simple_singleline(
            name,
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_rect.width());
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let at = egui::pos2(
            text_rect.left(),
            text_rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(at, galley, text_color);

        if close_response.clicked() {
            close = true;
        } else if response.clicked() {
            self.page = page;
        }
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
        self.follow_system_appearance(ui.ctx());
        let theme = self.theme;

        // Sessions keep receiving output while another page is shown.
        let mut learned = false;
        for session in &mut self.sessions {
            session.poll();
            // Kept with the host, so its card can show it from now on.
            let host_id = session.host_id;
            if let Some(os) = session.take_new_os() {
                learned |= self.data.set_host_os(host_id, os);
            }
        }
        if learned {
            self.save();
        }
        // Typing `exit` in a session closes its tab as well.
        while let Some(index) = self.sessions.iter().position(Session::logged_out) {
            self.close_session(index);
        }
        let open: Vec<Uuid> = self.sessions.iter().map(|session| session.id).collect();
        self.views = split::sync(std::mem::take(&mut self.views), &open);
        self.pane_rects.clear();
        for forward in &mut self.active_forwards {
            forward.poll();
        }
        self.sftp_page.poll();
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
                    // Button first, from the right, so a long message wraps
                    // instead of pushing it off the window.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Dismiss").clicked() {
                            self.notice = None;
                        }
                        let color = if notice.error {
                            theme.danger
                        } else {
                            theme.text
                        };
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(notice.text).color(color)).wrap(),
                            );
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
        if self.page == Page::Keychain {
            let mut page = std::mem::take(&mut self.keychain_page);
            page.editor_panel(self, ui);
            self.keychain_page = page;
        }

        if let Page::Session(id) = self.page {
            self.session_page(ui, id);
            self.session_drag(ui);
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
                Page::Sftp => {
                    let mut page = std::mem::take(&mut self.sftp_page);
                    page.show(self, ui);
                    self.sftp_page = page;
                }
                Page::PortForwarding => {
                    let mut page = std::mem::take(&mut self.forwards_page);
                    page.show(self, ui);
                    self.forwards_page = page;
                }
                Page::Keychain => {
                    let mut page = std::mem::take(&mut self.keychain_page);
                    page.show(self, ui);
                    self.keychain_page = page;
                }
            });
        self.session_drag(ui);
    }
}

/// Color of the dot that shows how a session is doing.
fn session_dot(theme: &Theme, state: &SessionState) -> Color32 {
    match state {
        SessionState::Connected => theme.accent_text,
        SessionState::Closed(Some(_)) => theme.danger,
        _ => theme.text_dim,
    }
}
