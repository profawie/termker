//! Application state and main layout: sidebar on the left, page on the right.

use eframe::egui::{self, RichText};

use crate::{
    model::Data,
    store::Store,
    theme::Theme,
    ui::{self, hosts::HostsPage},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Hosts,
    Sftp,
    PortForwarding,
    Keychain,
    Settings,
    About,
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
        }
    }
}

pub struct App {
    store: Store,
    pub data: Data,
    pub theme: Theme,
    page: Page,
    hosts_page: HostsPage,
    /// Store error message shown at the top.
    notice: Option<String>,
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
        Self {
            store,
            data,
            theme,
            page: Page::Hosts,
            hosts_page: HostsPage::default(),
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
                Page::Sftp => ui::placeholder::show(
                    self,
                    ui,
                    "SFTP",
                    "Browse, upload and download files on your hosts.",
                ),
                Page::PortForwarding => ui::placeholder::show(
                    self,
                    ui,
                    "Port Forwarding",
                    "Local, remote and dynamic tunnels through your hosts.",
                ),
                Page::Keychain => ui::placeholder::show(
                    self,
                    ui,
                    "Keychain",
                    "Generate keys and manage identities, certificates and FIDO2 keys.",
                ),
            });
    }
}
