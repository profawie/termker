//! SFTP page: pick a host, browse its files, and transfer them.

use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, atomic::Ordering},
};

use eframe::egui::{self, RichText};
use uuid::Uuid;

use crate::{
    app::App,
    sftp::{self, Command, Entry, EntryKind, Event, SftpHandle, Transfer},
    ssh::{Hop, Prompt},
    theme::Theme,
};

#[derive(Default)]
pub struct SftpPage {
    browser: Option<Browser>,
}

enum State {
    Connecting(String),
    Ready,
    Closed(Option<String>),
}

#[derive(Clone, Copy, PartialEq)]
enum Direction {
    Upload,
    Download,
}

enum TransferState {
    Running,
    Done,
    Failed(String),
}

struct TransferRow {
    id: u64,
    name: String,
    direction: Direction,
    done: u64,
    total: u64,
    state: TransferState,
    cancel: Arc<AtomicBool>,
}

/// A small form shown above the file list.
enum Dialog {
    NewFolder(String),
    Rename {
        from: String,
        name: String,
    },
    Delete(Entry),
    /// Uploads whose names already exist in the current folder.
    Overwrite(Vec<PathBuf>),
}

struct Browser {
    host_name: String,
    handle: SftpHandle,
    state: State,
    prompt: Option<Prompt>,
    secret_input: String,
    /// Folder currently shown.
    path: String,
    /// Text in the path box; differs from `path` while the user types.
    path_input: String,
    entries: Vec<Entry>,
    loading: bool,
    error: Option<String>,
    dialog: Option<Dialog>,
    transfers: Vec<TransferRow>,
    next_id: u64,
}

impl Browser {
    fn poll(&mut self) {
        while let Ok(event) = self.handle.events.try_recv() {
            match event {
                Event::Status(text) => {
                    if matches!(self.state, State::Connecting(_)) {
                        self.state = State::Connecting(text);
                    }
                }
                Event::Prompt(prompt) => {
                    self.secret_input.clear();
                    self.prompt = Some(prompt);
                }
                Event::Connected { home } => {
                    self.prompt = None;
                    self.state = State::Ready;
                    self.path = home.clone();
                    self.path_input = home;
                    self.loading = true;
                }
                Event::Listing { path, entries } => {
                    // A listing for a folder the user already left (for
                    // example after an upload there) is ignored.
                    if self.loading || path == self.path {
                        self.path = path.clone();
                        self.path_input = path;
                        self.entries = entries;
                        self.loading = false;
                        self.error = None;
                    }
                }
                Event::Failed(text) => {
                    self.loading = false;
                    self.path_input = self.path.clone();
                    self.error = Some(text);
                }
                Event::Progress { id, done, total } => {
                    if let Some(row) = self.transfers.iter_mut().find(|row| row.id == id) {
                        row.done = done;
                        row.total = total;
                    }
                }
                Event::TransferDone { id, error } => {
                    if let Some(row) = self.transfers.iter_mut().find(|row| row.id == id) {
                        row.state = match error {
                            None => TransferState::Done,
                            Some(error) => TransferState::Failed(error),
                        };
                    }
                }
                Event::Closed { error } => {
                    self.prompt = None;
                    self.secret_input.clear();
                    for row in &mut self.transfers {
                        if matches!(row.state, TransferState::Running) {
                            row.state = TransferState::Failed("Disconnected".to_string());
                        }
                    }
                    self.state = State::Closed(error);
                }
            }
        }
    }

    fn open(&mut self, path: String) {
        self.loading = true;
        self.error = None;
        self.dialog = None;
        self.handle.send(Command::List(path));
    }

    fn start_transfer(&mut self, direction: Direction, remote: String, local: PathBuf) {
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let name = match direction {
            Direction::Upload => local
                .file_name()
                .map(|name| name.to_string_lossy().to_string()),
            Direction::Download => remote.rsplit('/').next().map(str::to_string),
        }
        .unwrap_or_default();
        self.transfers.push(TransferRow {
            id: self.next_id,
            name,
            direction,
            done: 0,
            total: 0,
            state: TransferState::Running,
            cancel: cancel.clone(),
        });
        let transfer = Transfer {
            id: self.next_id,
            remote,
            local,
            cancel,
        };
        self.handle.send(match direction {
            Direction::Upload => Command::Upload(transfer),
            Direction::Download => Command::Download(transfer),
        });
    }

    /// Upload local files into the current folder, asking first when a name
    /// is already taken there.
    fn upload(&mut self, files: Vec<PathBuf>) {
        let (existing, fresh): (Vec<PathBuf>, Vec<PathBuf>) = files
            .into_iter()
            .filter(|file| file.is_file())
            .partition(|file| {
                let name = file
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string());
                self.entries
                    .iter()
                    .any(|entry| Some(&entry.name) == name.as_ref())
            });
        self.upload_now(fresh);
        if !existing.is_empty() {
            self.dialog = Some(Dialog::Overwrite(existing));
        }
    }

    fn upload_now(&mut self, files: Vec<PathBuf>) {
        for file in files {
            if let Some(name) = file
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
            {
                let remote = sftp::join(&self.path, &name);
                self.start_transfer(Direction::Upload, remote, file);
            }
        }
    }
}

impl SftpPage {
    /// Apply background events. Called every frame so transfers keep
    /// updating while another page is shown.
    pub fn poll(&mut self) {
        if let Some(browser) = &mut self.browser {
            browser.poll();
        }
    }

    pub fn show(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let theme = app.theme;
        let Some(browser) = &mut self.browser else {
            if let Some(host_id) = host_picker(app, ui) {
                self.connect(app, ui.ctx(), host_id);
            }
            return;
        };

        let mut disconnect = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("SFTP").size(24.0).strong());
            ui.label(
                RichText::new(&browser.host_name)
                    .size(16.0)
                    .color(theme.text_dim),
            );
            let (dot, status) = match &browser.state {
                State::Connecting(text) => (theme.text_dim, format!("{text}...")),
                State::Ready => (theme.accent, "Connected".to_string()),
                State::Closed(None) => (theme.text_dim, "Disconnected".to_string()),
                State::Closed(Some(_)) => (theme.danger, "Failed".to_string()),
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, dot);
            ui.label(RichText::new(status).small().color(theme.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if matches!(browser.state, State::Closed(_)) {
                    "Back to hosts"
                } else {
                    "Disconnect"
                };
                if ui.button(label).clicked() {
                    disconnect = true;
                }
            });
        });
        ui.add_space(8.0);

        if let State::Closed(Some(error)) = &browser.state {
            ui.label(RichText::new(error).color(theme.danger));
        }
        if browser.prompt.is_some() {
            super::session::prompt_card(ui, &theme, &mut browser.prompt, &mut browser.secret_input);
        }
        if matches!(browser.state, State::Ready) {
            // Files dropped onto the window are uploaded here.
            let dropped: Vec<PathBuf> = ui.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .collect()
            });
            if !dropped.is_empty() {
                browser.upload(dropped);
            }
            transfers_panel(ui, &theme, browser);
            toolbar(ui, &theme, browser);
            dialog(ui, &theme, browser);
            if let Some(error) = &browser.error {
                ui.label(RichText::new(error).color(theme.danger));
            }
            file_list(ui, &theme, browser);
        }

        if disconnect {
            self.browser = None;
        }
    }

    fn connect(&mut self, app: &mut App, ctx: &egui::Context, host_id: Uuid) {
        let Some(host) = app.data.host(host_id) else {
            return;
        };
        let Ok(path) = app.data.connection_path(host) else {
            return;
        };
        let hops: Vec<Hop> = path.into_iter().map(Hop::from).collect();
        let ctx = ctx.clone();
        let handle = app.spawn_sftp(hops, move || ctx.request_repaint());
        self.browser = Some(Browser {
            host_name: host.display_name().to_string(),
            handle,
            state: State::Connecting("Starting".to_string()),
            prompt: None,
            secret_input: String::new(),
            path: String::new(),
            path_input: String::new(),
            entries: Vec::new(),
            loading: false,
            error: None,
            dialog: None,
            transfers: Vec::new(),
            next_id: 0,
        });
    }
}

/// List of hosts to open. Returns the one the user picked.
fn host_picker(app: &App, ui: &mut egui::Ui) -> Option<Uuid> {
    let theme = app.theme;
    super::page_header(
        ui,
        &theme,
        "SFTP",
        "Browse, upload and download files on your hosts.",
    );
    if app.data.hosts.is_empty() {
        ui.label(
            RichText::new("Add a host first, then open its files here.").color(theme.text_dim),
        );
        return None;
    }
    let mut picked = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for host in &app.data.hosts {
            super::card(&theme).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(host.display_name()).size(15.0).strong());
                        ui.label(
                            RichText::new(host.target())
                                .monospace()
                                .small()
                                .color(theme.text_dim),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(super::primary_button(&theme, "Open files"))
                            .clicked()
                        {
                            picked = Some(host.id);
                        }
                    });
                });
            });
            ui.add_space(4.0);
        }
    });
    picked
}

fn toolbar(ui: &mut egui::Ui, theme: &Theme, browser: &mut Browser) {
    ui.horizontal(|ui| {
        let up = sftp::parent(&browser.path);
        if ui
            .add_enabled(up.is_some(), egui::Button::new("Up"))
            .clicked()
            && let Some(up) = up
        {
            browser.open(up);
        }
        if ui.button("Refresh").clicked() {
            browser.open(browser.path.clone());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(super::primary_button(theme, "Upload files"))
                .clicked()
                && let Some(files) = rfd::FileDialog::new()
                    .set_title("Upload files")
                    .pick_files()
            {
                browser.upload(files);
            }
            if ui.button("New folder").clicked() {
                browser.dialog = Some(Dialog::NewFolder(String::new()));
            }
            let path = ui.add(
                egui::TextEdit::singleline(&mut browser.path_input)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
            if path.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                let target = browser.path_input.trim().to_string();
                if !target.is_empty() {
                    browser.open(target);
                }
            }
        });
    });
    ui.add_space(4.0);
}

fn dialog(ui: &mut egui::Ui, theme: &Theme, browser: &mut Browser) {
    let Some(mut current) = browser.dialog.take() else {
        return;
    };
    let mut close = false;
    super::card(theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match &mut current {
            Dialog::NewFolder(name) => {
                ui.label(RichText::new("New folder").strong());
                match name_form(ui, theme, name, "Create") {
                    Some(true) => {
                        let path = sftp::join(&browser.path, name.trim());
                        browser.handle.send(Command::CreateDir(path));
                        close = true;
                    }
                    Some(false) => close = true,
                    None => {}
                }
            }
            Dialog::Rename { from, name } => {
                ui.label(RichText::new(format!("Rename {from}")).strong());
                match name_form(ui, theme, name, "Rename") {
                    Some(true) => {
                        browser.handle.send(Command::Rename {
                            from: sftp::join(&browser.path, from),
                            to: sftp::join(&browser.path, name.trim()),
                        });
                        close = true;
                    }
                    Some(false) => close = true,
                    None => {}
                }
            }
            Dialog::Delete(entry) => {
                let what = if entry.kind == EntryKind::Dir {
                    "folder"
                } else {
                    "file"
                };
                ui.label(
                    RichText::new(format!("Delete the {what} {}?", entry.name))
                        .strong()
                        .color(theme.danger),
                );
                ui.label(
                    RichText::new("This removes it from the server and cannot be undone.")
                        .color(theme.text_dim),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new("Delete").color(theme.danger))
                        .clicked()
                    {
                        browser.handle.send(Command::Remove {
                            path: sftp::join(&browser.path, &entry.name),
                            is_dir: entry.kind == EntryKind::Dir,
                        });
                        close = true;
                    }
                    if ui.add(super::primary_button(theme, "Keep")).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Overwrite(files) => {
                let names: Vec<String> = files
                    .iter()
                    .filter_map(|file| file.file_name())
                    .map(|name| name.to_string_lossy().to_string())
                    .collect();
                ui.label(RichText::new("Replace existing files?").strong());
                ui.label(
                    RichText::new(format!("Already in this folder: {}", names.join(", ")))
                        .color(theme.text_dim),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new("Replace").color(theme.danger))
                        .clicked()
                    {
                        browser.upload_now(std::mem::take(files));
                        close = true;
                    }
                    if ui.add(super::primary_button(theme, "Skip these")).clicked() {
                        close = true;
                    }
                });
            }
        }
    });
    ui.add_space(4.0);
    if !close {
        browser.dialog = Some(current);
    }
}

/// Name input with a confirm button and Cancel. Returns `Some(true)` when a
/// valid name was confirmed, `Some(false)` on Cancel, `None` while open.
fn name_form(ui: &mut egui::Ui, theme: &Theme, name: &mut String, confirm: &str) -> Option<bool> {
    let mut result = None;
    ui.horizontal(|ui| {
        let field = ui.add(egui::TextEdit::singleline(name).desired_width(280.0));
        if !field.has_focus() && !field.lost_focus() {
            field.request_focus();
        }
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let valid = sftp::valid_name(name.trim());
        let button = ui.add_enabled(valid, super::primary_button(theme, confirm));
        if (button.clicked() || entered) && valid {
            result = Some(true);
        }
        if ui.button("Cancel").clicked() {
            result = Some(false);
        }
    });
    if !name.trim().is_empty() && !sftp::valid_name(name.trim()) {
        ui.label(
            RichText::new("A name cannot contain / or be . or ..")
                .small()
                .color(theme.danger),
        );
    }
    result
}

fn file_list(ui: &mut egui::Ui, theme: &Theme, browser: &mut Browser) {
    if browser.loading {
        ui.label(RichText::new("Loading...").color(theme.text_dim));
        return;
    }
    if browser.entries.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("This folder is empty").color(theme.text_dim));
            ui.label(
                RichText::new("Drop files here or use Upload files.")
                    .small()
                    .color(theme.text_dim),
            );
        });
        return;
    }

    enum Action {
        Open(String),
        Download(String),
        Rename(String),
        Delete(Entry),
    }
    let mut action = None;
    super::card(theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("sftp_files")
                .num_columns(5)
                .spacing(egui::vec2(20.0, 6.0))
                .striped(false)
                .show(ui, |ui| {
                    for heading in ["Name", "Size", "Modified (UTC)", "Permissions", ""] {
                        ui.label(RichText::new(heading).small().color(theme.text_dim));
                    }
                    ui.end_row();
                    for entry in &browser.entries {
                        let is_dir = entry.kind == EntryKind::Dir;
                        let marker = match entry.kind {
                            EntryKind::Dir => "[dir] ",
                            EntryKind::Symlink => "[link]",
                            _ => "      ",
                        };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(
                                RichText::new(marker)
                                    .monospace()
                                    .small()
                                    .color(theme.text_dim),
                            );
                            if is_dir || entry.kind == EntryKind::Symlink {
                                let link = egui::Label::new(RichText::new(&entry.name).strong())
                                    .sense(egui::Sense::click());
                                if ui
                                    .add(link)
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    action = Some(Action::Open(entry.name.clone()));
                                }
                            } else {
                                ui.label(&entry.name);
                            }
                        });
                        let size = if is_dir {
                            String::new()
                        } else {
                            sftp::format_size(entry.size)
                        };
                        ui.label(RichText::new(size).color(theme.text_dim));
                        let modified = entry.modified.map(sftp::format_time).unwrap_or_default();
                        ui.label(RichText::new(modified).color(theme.text_dim));
                        let permissions = entry
                            .permissions
                            .map(sftp::format_permissions)
                            .unwrap_or_default();
                        ui.label(
                            RichText::new(permissions)
                                .monospace()
                                .size(12.0)
                                .color(theme.text_dim),
                        );
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if entry.kind == EntryKind::File
                                && ui.small_button("Download").clicked()
                            {
                                action = Some(Action::Download(entry.name.clone()));
                            }
                            if ui.small_button("Rename").clicked() {
                                action = Some(Action::Rename(entry.name.clone()));
                            }
                            if ui.small_button("Delete").clicked() {
                                action = Some(Action::Delete(entry.clone()));
                            }
                        });
                        ui.end_row();
                    }
                });
        });
    });

    match action {
        Some(Action::Open(name)) => browser.open(sftp::join(&browser.path, &name)),
        Some(Action::Download(name)) => {
            if let Some(local) = rfd::FileDialog::new()
                .set_title("Save file as")
                .set_file_name(&name)
                .save_file()
            {
                browser.start_transfer(
                    Direction::Download,
                    sftp::join(&browser.path, &name),
                    local,
                );
            }
        }
        Some(Action::Rename(name)) => {
            browser.dialog = Some(Dialog::Rename {
                from: name.clone(),
                name,
            });
        }
        Some(Action::Delete(entry)) => browser.dialog = Some(Dialog::Delete(entry)),
        None => {}
    }
}

fn transfers_panel(ui: &mut egui::Ui, theme: &Theme, browser: &mut Browser) {
    if browser.transfers.is_empty() {
        return;
    }
    egui::Panel::bottom("sftp_transfers")
        .resizable(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(0, 8)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Transfers").strong());
                let finished = browser
                    .transfers
                    .iter()
                    .any(|row| !matches!(row.state, TransferState::Running));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if finished && ui.small_button("Clear finished").clicked() {
                        browser
                            .transfers
                            .retain(|row| matches!(row.state, TransferState::Running));
                    }
                });
            });
            egui::ScrollArea::vertical()
                .max_height(140.0)
                .id_salt("sftp_transfer_rows")
                .show(ui, |ui| {
                    for row in browser.transfers.iter().rev() {
                        ui.horizontal(|ui| {
                            let arrow = match row.direction {
                                Direction::Upload => "Upload  ",
                                Direction::Download => "Download",
                            };
                            ui.label(
                                RichText::new(arrow)
                                    .monospace()
                                    .small()
                                    .color(theme.text_dim),
                            );
                            ui.add(egui::Label::new(&row.name).truncate());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| match &row.state {
                                    TransferState::Running => {
                                        if ui.small_button("Cancel").clicked() {
                                            row.cancel.store(true, Ordering::Relaxed);
                                        }
                                        let fraction = if row.total == 0 {
                                            0.0
                                        } else {
                                            row.done as f32 / row.total as f32
                                        };
                                        ui.label(
                                            RichText::new(format!(
                                                "{} of {}",
                                                sftp::format_size(row.done),
                                                sftp::format_size(row.total)
                                            ))
                                            .small()
                                            .color(theme.text_dim),
                                        );
                                        ui.add(
                                            egui::ProgressBar::new(fraction)
                                                .desired_width(180.0)
                                                .fill(theme.accent),
                                        );
                                    }
                                    TransferState::Done => {
                                        ui.label(
                                            RichText::new(format!(
                                                "Done, {}",
                                                sftp::format_size(row.total)
                                            ))
                                            .small()
                                            .color(theme.accent),
                                        );
                                    }
                                    TransferState::Failed(error) => {
                                        ui.label(RichText::new(error).small().color(theme.danger));
                                    }
                                },
                            );
                        });
                    }
                });
        });
}
