//! SFTP page: two file panes side by side. Each pane shows this computer or
//! one of the hosts. Dragging a file or folder from one pane and dropping it
//! on the other copies it to the folder open there.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use uuid::Uuid;

use crate::{
    app::App,
    sftp::{self, Command, CopyEvent, CopyHandle, Entry, EntryKind, Event, Place, SftpHandle},
    ssh::Prompt,
    theme::Theme,
};

const THIS_COMPUTER: &str = "This computer";
/// Transfers visible without scrolling, and the height of one.
const TRANSFER_ROWS_SHOWN: usize = 3;
const TRANSFER_ROW_HEIGHT: f32 = 30.0;

#[derive(Default)]
pub struct SftpPage {
    panes: [Pane; 2],
    /// The left pane opens on this computer the first time the page is shown.
    started: bool,
    transfers: Vec<TransferRow>,
    next_id: u64,
    /// A transfer waiting for the user to agree to replace what is there.
    replace: Option<Replace>,
}

/// A file or folder being dragged from one pane to the other.
#[derive(Clone)]
struct Dragged {
    /// Pane it comes from.
    from: usize,
    entry: Entry,
}

struct Replace {
    /// Pane the item comes from.
    from: usize,
    entry: Entry,
}

enum State {
    Connecting(String),
    Ready,
    Closed(Option<String>),
}

/// A connection to a host, shown in a pane.
struct Remote {
    host_id: Uuid,
    host_name: String,
    handle: SftpHandle,
    state: State,
    prompt: Option<Prompt>,
    secret_input: String,
}

#[derive(Default)]
enum Source {
    /// Nothing chosen yet; the pane lists what can be opened.
    #[default]
    None,
    Local,
    Remote(Box<Remote>),
}

/// Identifies where a pane is looking, to tell when two panes show the same
/// folder and which panes to refresh after a transfer.
#[derive(Clone, PartialEq)]
struct Location {
    /// `None` for this computer.
    host: Option<Uuid>,
    path: String,
}

enum TransferState {
    Running,
    Done,
    Failed(String),
}

struct TransferRow {
    id: u64,
    name: String,
    /// "from > to", by pane title.
    route: String,
    done: u64,
    total: u64,
    /// Links and special files in a folder that were left out.
    skipped: usize,
    state: TransferState,
    handle: CopyHandle,
    /// Folder that receives the item.
    target: Location,
}

/// A small form shown above the file list.
enum Dialog {
    NewFolder(String),
    Rename { from: String, name: String },
    Delete(Entry),
}

#[derive(Default)]
struct Pane {
    source: Source,
    /// Folder currently shown. Empty on this computer means the list of
    /// drives (Windows).
    path: String,
    /// Text in the path box; differs from `path` while the user types.
    path_input: String,
    entries: Vec<Entry>,
    loading: bool,
    error: Option<String>,
    dialog: Option<Dialog>,
}

/// What can be shown in a pane.
#[derive(Clone, Copy, PartialEq)]
enum Choice {
    Local,
    Host(Uuid),
}

/// What the user asked for in a pane that the page has to carry out.
enum PaneAction {
    Show(Choice),
    Close,
    Transfer(Entry),
}

impl Pane {
    fn title(&self) -> String {
        match &self.source {
            Source::None => "Nothing open".to_string(),
            Source::Local => THIS_COMPUTER.to_string(),
            Source::Remote(remote) => remote.host_name.clone(),
        }
    }

    /// The folder shown, if the pane is ready to list and receive files.
    fn location(&self) -> Option<Location> {
        match &self.source {
            Source::Local if !self.path.is_empty() => Some(Location {
                host: None,
                path: self.path.clone(),
            }),
            Source::Remote(remote) if matches!(remote.state, State::Ready) => Some(Location {
                host: Some(remote.host_id),
                path: self.path.clone(),
            }),
            _ => None,
        }
    }

    fn is_local(&self) -> bool {
        matches!(self.source, Source::Local)
    }

    /// Path of `name` in the folder shown.
    fn child(&self, name: &str) -> String {
        if !self.is_local() {
            sftp::join(&self.path, name)
        } else if self.path.is_empty() {
            name.to_string()
        } else {
            Path::new(&self.path).join(name).display().to_string()
        }
    }

    /// Folder above the one shown, or `None` at the top.
    fn parent(&self) -> Option<String> {
        if !self.is_local() {
            return sftp::parent(&self.path);
        }
        match Path::new(&self.path).parent() {
            Some(parent) if !parent.as_os_str().is_empty() => Some(parent.display().to_string()),
            // Above a drive is the list of drives.
            _ if cfg!(windows) && !self.path.is_empty() => Some(String::new()),
            _ => None,
        }
    }

    fn place(&self, path: String) -> Option<Place> {
        match &self.source {
            Source::None => None,
            Source::Local => Some(Place::Local(PathBuf::from(path))),
            Source::Remote(remote) => Some(Place::Remote {
                peer: remote.handle.peer(),
                path,
            }),
        }
    }

    fn show_local(&mut self) {
        *self = Self {
            source: Source::Local,
            ..Self::default()
        };
        self.open(sftp::local::home().display().to_string());
    }

    fn open(&mut self, path: String) {
        self.dialog = None;
        self.error = None;
        match &self.source {
            Source::None => {}
            Source::Local => self.list_local(path),
            Source::Remote(remote) => {
                self.loading = true;
                remote.handle.send(Command::List(path));
            }
        }
    }

    /// Read the folder again without clearing what is on screen.
    fn refresh(&mut self) {
        match &self.source {
            Source::None => {}
            Source::Local => self.list_local(self.path.clone()),
            Source::Remote(remote) => remote.handle.send(Command::List(self.path.clone())),
        }
    }

    fn list_local(&mut self, path: String) {
        let listed = if path.is_empty() {
            // The drives, each shown as a folder.
            Ok(sftp::local::roots()
                .into_iter()
                .map(|root| Entry {
                    name: root.display().to_string(),
                    kind: EntryKind::Dir,
                    size: 0,
                    modified: None,
                    permissions: None,
                })
                .collect())
        } else {
            sftp::local::list(Path::new(&path))
        };
        match listed {
            Ok(entries) => {
                self.entries = entries;
                self.path_input = path.clone();
                self.path = path;
                self.error = None;
            }
            Err(err) => {
                self.path_input = self.path.clone();
                self.error = Some(format!("Could not open {path}: {err}"));
            }
        }
    }

    fn poll(&mut self) {
        let Source::Remote(remote) = &mut self.source else {
            return;
        };
        while let Ok(event) = remote.handle.events.try_recv() {
            match event {
                Event::Status(text) => {
                    if matches!(remote.state, State::Connecting(_)) {
                        remote.state = State::Connecting(text);
                    }
                }
                Event::Prompt(prompt) => {
                    remote.secret_input.clear();
                    remote.prompt = Some(prompt);
                }
                Event::Connected { home } => {
                    remote.prompt = None;
                    remote.state = State::Ready;
                    self.path = home.clone();
                    self.path_input = home;
                    self.loading = true;
                }
                Event::Listing { path, entries } => {
                    // A listing for a folder the user already left is
                    // ignored.
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
                Event::Closed { error } => {
                    remote.prompt = None;
                    remote.secret_input.clear();
                    remote.state = State::Closed(error);
                }
            }
        }
    }

    /// Carry out a confirmed New folder, Rename or Delete.
    fn create_dir(&mut self, name: &str) {
        let path = self.child(name);
        match &self.source {
            Source::None => {}
            Source::Local => {
                let result = std::fs::create_dir(&path);
                self.after_local("Could not create the folder", result);
            }
            Source::Remote(remote) => remote.handle.send(Command::CreateDir(path)),
        }
    }

    fn rename(&mut self, from: &str, to: &str) {
        let (from, to) = (self.child(from), self.child(to));
        match &self.source {
            Source::None => {}
            Source::Local => {
                // Unlike on a server, a local rename would silently replace
                // a file that has the new name.
                let result = if Path::new(&to).exists() {
                    Err(std::io::Error::other("that name is already taken"))
                } else {
                    std::fs::rename(&from, &to)
                };
                self.after_local("Could not rename", result);
            }
            Source::Remote(remote) => remote.handle.send(Command::Rename { from, to }),
        }
    }

    fn delete(&mut self, entry: &Entry) {
        let path = self.child(&entry.name);
        let is_dir = entry.kind == EntryKind::Dir;
        match &self.source {
            Source::None => {}
            Source::Local => {
                let result = if is_dir {
                    std::fs::remove_dir(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                self.after_local("Could not delete", result);
            }
            Source::Remote(remote) => remote.handle.send(Command::Remove { path, is_dir }),
        }
    }

    fn after_local(&mut self, action: &str, result: std::io::Result<()>) {
        self.refresh();
        if let Err(err) = result {
            self.error = Some(format!("{action}: {err}"));
        }
    }
}

impl SftpPage {
    /// Apply background events. Called every frame so transfers keep
    /// updating while another page is shown.
    pub fn poll(&mut self) {
        for pane in &mut self.panes {
            pane.poll();
        }
        let mut refresh: Vec<Location> = Vec::new();
        for row in &mut self.transfers {
            while let Ok(event) = row.handle.events.try_recv() {
                match event {
                    CopyEvent::Progress { done, total } => {
                        row.done = done;
                        row.total = total;
                    }
                    CopyEvent::Done { error, skipped } => {
                        row.skipped = skipped;
                        row.state = match error {
                            None => TransferState::Done,
                            Some(error) => TransferState::Failed(error),
                        };
                        // A failed folder copy may still have added files.
                        refresh.push(row.target.clone());
                    }
                }
            }
        }
        for pane in &mut self.panes {
            if pane
                .location()
                .is_some_and(|location| refresh.contains(&location))
            {
                pane.refresh();
            }
        }
    }

    pub fn show(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let theme = app.theme;
        if !self.started {
            self.started = true;
            self.panes[0].show_local();
        }
        super::page_header(
            ui,
            &theme,
            "SFTP",
            "Drag a file or folder from one pane and drop it on the other to copy it: computer to host, host to computer, or host to host.",
        );

        self.replace_card(app, ui);

        let mut actions: [Option<PaneAction>; 2] = [None, None];
        let mut rects = [egui::Rect::NOTHING; 2];
        // The panes take the height left above the list of transfers.
        let rows = self.transfers.len().min(TRANSFER_ROWS_SHOWN) as f32;
        let reserved = if self.transfers.is_empty() {
            0.0
        } else {
            44.0 + rows * TRANSFER_ROW_HEIGHT
        };
        let size = egui::vec2(
            ui.available_width(),
            (ui.available_height() - reserved).max(160.0),
        );
        ui.allocate_ui(size, |ui| {
            ui.columns(2, |columns| {
                for (index, column) in columns.iter_mut().enumerate() {
                    column.push_id(index, |ui| {
                        let (action, rect) = pane_ui(ui, app, &mut self.panes[index], index);
                        actions[index] = action;
                        rects[index] = rect;
                    });
                }
            });
        });
        transfers_list(ui, &theme, &mut self.transfers);

        // A row dragged out of one pane is copied where it is dropped on
        // the other.
        if let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(ui.ctx()) {
            let to = 1 - dragged.from;
            let blocked = self.blocked(dragged.from);
            let over = ui
                .ctx()
                .pointer_latest_pos()
                .is_some_and(|pointer| rects[to].contains(pointer));
            let hint = match blocked {
                Some(reason) => reason.to_string(),
                None => format!(
                    "Drop to copy to {} on {}",
                    self.panes[to].path,
                    self.panes[to].title()
                ),
            };
            if blocked.is_none() {
                super::drop_outline(ui, &theme, rects[to], over);
            }
            super::drag_chip(
                ui.ctx(),
                &theme,
                &dragged.entry.name,
                if over || blocked.is_some() {
                    &hint
                } else {
                    "Drop on the other pane to copy"
                },
                over && blocked.is_none(),
            );
            if over && blocked.is_none() && ui.input(|input| input.pointer.any_released()) {
                actions[dragged.from] = Some(PaneAction::Transfer(dragged.entry.clone()));
            }
        }

        for (index, action) in actions.into_iter().enumerate() {
            match action {
                Some(PaneAction::Show(Choice::Local)) => self.panes[index].show_local(),
                Some(PaneAction::Show(Choice::Host(host_id))) => {
                    self.connect(app, ui.ctx(), index, host_id);
                }
                Some(PaneAction::Close) => self.panes[index] = Pane::default(),
                Some(PaneAction::Transfer(entry)) => {
                    let taken = self.panes[1 - index]
                        .entries
                        .iter()
                        .any(|existing| existing.name == entry.name);
                    if taken {
                        self.replace = Some(Replace { from: index, entry });
                    } else {
                        self.transfer(app, ui.ctx(), index, &entry);
                    }
                }
                None => {}
            }
        }
    }

    fn connect(&mut self, app: &mut App, ctx: &egui::Context, index: usize, host_id: Uuid) {
        let Some(host) = app.data.host(host_id) else {
            return;
        };
        let hops = match app.hops(host) {
            Ok(hops) => hops,
            Err(err) => {
                app.notify(err);
                return;
            }
        };
        let ctx = ctx.clone();
        let handle = app.spawn_sftp(hops, move || ctx.request_repaint());
        self.panes[index] = Pane {
            source: Source::Remote(Box::new(Remote {
                host_id,
                host_name: host.display_name().to_string(),
                handle,
                state: State::Connecting("Starting".to_string()),
                prompt: None,
                secret_input: String::new(),
            })),
            ..Pane::default()
        };
    }

    /// Why nothing can be copied out of pane `from` right now, if so.
    fn blocked(&self, from: usize) -> Option<&'static str> {
        let (source, destination) = (&self.panes[from], &self.panes[1 - from]);
        match destination.location() {
            None => Some("Open a folder in the other pane first"),
            Some(location) if Some(&location) == source.location().as_ref() => {
                Some("Both panes show the same folder")
            }
            Some(_) => None,
        }
    }

    /// Copy `entry` from pane `from` into the folder open in the other pane.
    fn transfer(&mut self, app: &App, ctx: &egui::Context, from: usize, entry: &Entry) {
        let (source, destination) = (&self.panes[from], &self.panes[1 - from]);
        let Some(target) = destination.location() else {
            return;
        };
        let (Some(from_place), Some(to_place)) = (
            source.place(source.child(&entry.name)),
            destination.place(destination.child(&entry.name)),
        ) else {
            return;
        };
        let ctx = ctx.clone();
        let handle = app.spawn_copy(from_place, to_place, move || ctx.request_repaint());
        self.next_id += 1;
        self.transfers.push(TransferRow {
            id: self.next_id,
            name: entry.name.clone(),
            route: format!("{} > {}", source.title(), destination.title()),
            done: 0,
            total: 0,
            skipped: 0,
            state: TransferState::Running,
            handle,
            target,
        });
    }

    /// Asks before a transfer replaces something with the same name.
    fn replace_card(&mut self, app: &App, ui: &mut egui::Ui) {
        let Some(replace) = self.replace.take() else {
            return;
        };
        let theme = app.theme;
        let destination = &self.panes[1 - replace.from];
        // Nothing to ask once the other pane moved somewhere else.
        if destination.location().is_none() {
            return;
        }
        let mut answer = None;
        super::card(&theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let name = &replace.entry.name;
            ui.label(
                RichText::new(format!(
                    "{name} already exists in {} on {}",
                    destination.path,
                    destination.title()
                ))
                .strong(),
            );
            let note = if replace.entry.kind == EntryKind::Dir {
                "The folders are merged: files with the same name are replaced, everything else is kept."
            } else {
                "The file there is replaced by this one."
            };
            ui.add(egui::Label::new(RichText::new(note).color(theme.text_dim)).wrap());
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Replace").color(theme.danger))
                    .clicked()
                {
                    answer = Some(true);
                }
                if ui.add(super::primary_button(&theme, "Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        ui.add_space(8.0);
        match answer {
            Some(true) => self.transfer(app, ui.ctx(), replace.from, &replace.entry),
            Some(false) => {}
            None => self.replace = Some(replace),
        }
    }
}

/// Everything a pane can show: this computer, then the hosts by group.
fn choices(app: &App) -> Vec<(Choice, String)> {
    let mut hosts: Vec<_> = app.data.hosts.iter().collect();
    hosts.sort_by_key(|host| {
        (
            host.group.is_empty(),
            host.group.to_lowercase(),
            host.display_name().to_lowercase(),
        )
    });
    std::iter::once((Choice::Local, THIS_COMPUTER.to_string()))
        .chain(hosts.into_iter().map(|host| {
            let label = if host.group.is_empty() {
                host.display_name().to_string()
            } else {
                format!(
                    "{} > {}",
                    host.group.replace(crate::model::GROUP_SEPARATOR, " > "),
                    host.display_name()
                )
            };
            (Choice::Host(host.id), label)
        }))
        .collect()
}

/// One pane: what it shows, where it is, and its files.
fn pane_ui(
    ui: &mut egui::Ui,
    app: &App,
    pane: &mut Pane,
    index: usize,
) -> (Option<PaneAction>, egui::Rect) {
    let theme = app.theme;
    let mut action = None;
    let card = super::card(&theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_min_height(ui.available_height());

        let current = match &pane.source {
            Source::None => None,
            Source::Local => Some(Choice::Local),
            Source::Remote(remote) => Some(Choice::Host(remote.host_id)),
        };
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if current.is_some() && ui.button("Close").clicked() {
                    action = Some(PaneAction::Close);
                }
                if let Source::Remote(remote) = &pane.source {
                    let (dot, status) = match &remote.state {
                        State::Connecting(text) => (theme.text_dim, format!("{text}...")),
                        State::Ready => (theme.accent_text, "Connected".to_string()),
                        State::Closed(None) => (theme.text_dim, "Disconnected".to_string()),
                        State::Closed(Some(_)) => (theme.danger, "Failed".to_string()),
                    };
                    ui.add(
                        egui::Label::new(RichText::new(status).small().color(theme.text_dim))
                            .truncate(),
                    );
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.0, dot);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let selected = if current.is_some() {
                        pane.title()
                    } else {
                        "Choose a place".to_string()
                    };
                    egui::ComboBox::from_id_salt("source")
                        .selected_text(RichText::new(selected).strong())
                        .width(ui.available_width().min(240.0))
                        .show_ui(ui, |ui| {
                            for (choice, label) in choices(app) {
                                if ui
                                    .selectable_label(current == Some(choice), label)
                                    .clicked()
                                    && current != Some(choice)
                                {
                                    action = Some(PaneAction::Show(choice));
                                }
                            }
                        });
                });
            });
        });
        ui.add_space(6.0);

        match &mut pane.source {
            Source::None => {
                if let Some(choice) = chooser(ui, app) {
                    action = Some(PaneAction::Show(choice));
                }
                return;
            }
            Source::Local => {}
            Source::Remote(remote) => {
                if remote.prompt.is_some() {
                    super::session::prompt_card(
                        ui,
                        &theme,
                        &mut remote.prompt,
                        &mut remote.secret_input,
                    );
                }
                match &remote.state {
                    State::Ready => {}
                    State::Connecting(_) => return,
                    State::Closed(error) => {
                        if let Some(error) = error {
                            ui.add(
                                egui::Label::new(RichText::new(error).color(theme.danger)).wrap(),
                            );
                        }
                        if ui.button("Connect again").clicked() {
                            action = Some(PaneAction::Show(Choice::Host(remote.host_id)));
                        }
                        return;
                    }
                }
            }
        }

        toolbar(ui, pane);
        dialog(ui, &theme, pane);
        if let Some(error) = &pane.error {
            ui.add(egui::Label::new(RichText::new(error).color(theme.danger)).wrap());
        }
        file_list(ui, &theme, pane, index);
    });
    (action, card.response.rect)
}

/// List of places for a pane that shows nothing yet.
fn chooser(ui: &mut egui::Ui, app: &App) -> Option<Choice> {
    let theme = app.theme;
    let mut picked = None;
    ui.label(RichText::new("Choose what to show in this pane.").color(theme.text_dim));
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            for (choice, label) in choices(app) {
                let button = egui::Button::new(label)
                    .min_size(egui::vec2(ui.available_width(), 30.0))
                    .truncate();
                if ui.add(button).clicked() {
                    picked = Some(choice);
                }
            }
            if app.data.hosts.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Add a host on the Hosts page to open its files here.")
                        .small()
                        .color(theme.text_dim),
                );
            }
        });
    picked
}

fn toolbar(ui: &mut egui::Ui, pane: &mut Pane) {
    ui.horizontal(|ui| {
        let up = pane.parent();
        if ui
            .add_enabled(up.is_some(), egui::Button::new("Up"))
            .clicked()
            && let Some(up) = up
        {
            pane.open(up);
        }
        if ui.button("Refresh").clicked() {
            pane.open(pane.path.clone());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // No folders can be made in the list of drives.
            if ui
                .add_enabled(!pane.path.is_empty(), egui::Button::new("New folder"))
                .clicked()
            {
                pane.dialog = Some(Dialog::NewFolder(String::new()));
            }
            let path = ui.add(
                egui::TextEdit::singleline(&mut pane.path_input)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
            if path.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                let target = pane.path_input.trim().to_string();
                if !target.is_empty() {
                    pane.open(target);
                }
            }
        });
    });
    ui.add_space(4.0);
}

fn dialog(ui: &mut egui::Ui, theme: &Theme, pane: &mut Pane) {
    let Some(mut current) = pane.dialog.take() else {
        return;
    };
    let mut close = false;
    let place = if pane.is_local() {
        "this computer"
    } else {
        "the server"
    };
    super::card(theme).fill(theme.bg).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match &mut current {
            Dialog::NewFolder(name) => {
                ui.label(RichText::new("New folder").strong());
                match name_form(ui, theme, name, "Create") {
                    Some(true) => {
                        pane.create_dir(name.trim());
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
                        pane.rename(from, name.trim());
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
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Delete the {what} {}?", entry.name))
                            .strong()
                            .color(theme.danger),
                    )
                    .wrap(),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "This removes it from {place} and cannot be undone."
                        ))
                        .color(theme.text_dim),
                    )
                    .wrap(),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new("Delete").color(theme.danger))
                        .clicked()
                    {
                        pane.delete(entry);
                        close = true;
                    }
                    if ui.add(super::primary_button(theme, "Keep")).clicked() {
                        close = true;
                    }
                });
            }
        }
    });
    ui.add_space(4.0);
    if !close {
        pane.dialog = Some(current);
    }
}

/// Name input with a confirm button and Cancel. Returns `Some(true)` when a
/// valid name was confirmed, `Some(false)` on Cancel, `None` while open.
fn name_form(ui: &mut egui::Ui, theme: &Theme, name: &mut String, confirm: &str) -> Option<bool> {
    let mut result = None;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Cancel").clicked() {
                result = Some(false);
            }
            let valid = sftp::valid_name(name.trim());
            let button = ui.add_enabled(valid, super::primary_button(theme, confirm));
            let field = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
            if !field.has_focus() && !field.lost_focus() {
                field.request_focus();
            }
            let entered =
                field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if (button.clicked() || entered) && valid {
                result = Some(true);
            }
        });
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

/// The files in a pane. Rows can be dragged to the other pane.
fn file_list(ui: &mut egui::Ui, theme: &Theme, pane: &mut Pane, index: usize) {
    if pane.loading {
        ui.label(RichText::new("Loading...").color(theme.text_dim));
        return;
    }
    if pane.entries.is_empty() {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("This folder is empty").color(theme.text_dim));
            ui.label(
                RichText::new("Drag files or folders here from the other pane.")
                    .small()
                    .color(theme.text_dim),
            );
        });
        return;
    }

    enum Action {
        Open(String),
        Rename(String),
        Delete(Entry),
    }
    let mut action = None;

    // The date only fits in a wide pane; it is always in the tooltip.
    let show_modified = ui.available_width() > 520.0;
    let drives = pane.path.is_empty();

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            // Dragging over a name moves the row instead of selecting text.
            ui.style_mut().interaction.selectable_labels = false;
            for entry in &pane.entries {
                let is_dir = entry.kind == EntryKind::Dir;
                let modified = entry.modified.map(sftp::format_time).unwrap_or_default();
                // The whole row can be picked up and dropped on the other
                // pane. Drives and special files cannot be copied.
                let draggable = !drives && entry.kind != EntryKind::Other;
                let drag_id = ui.scope_id().with(("drag", &entry.name));
                if draggable && super::drag_area(ui, drag_id) {
                    egui::DragAndDrop::set_payload(
                        ui.ctx(),
                        Dragged {
                            from: index,
                            entry: entry.clone(),
                        },
                    );
                }
                let row = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        if !drives {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if icon_button(ui, theme, RowIcon::Delete, "Delete").clicked() {
                                action = Some(Action::Delete(entry.clone()));
                            }
                            if icon_button(ui, theme, RowIcon::Rename, "Rename").clicked() {
                                action = Some(Action::Rename(entry.name.clone()));
                            }
                            ui.spacing_mut().item_spacing.x = 6.0;
                        }
                        if show_modified {
                            fixed(ui, 118.0, &modified, theme);
                        }
                        let size = if is_dir {
                            String::new()
                        } else {
                            sftp::format_size(entry.size)
                        };
                        fixed(ui, 62.0, &size, theme);

                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            icon(ui, theme, entry.kind);
                            let mut details = entry.name.clone();
                            if !modified.is_empty() {
                                details.push_str(&format!("\nModified {modified} UTC"));
                            }
                            if let Some(mode) = entry.permissions {
                                details.push_str(&format!(
                                    "\nPermissions {}",
                                    sftp::format_permissions(mode)
                                ));
                            }
                            if is_dir || entry.kind == EntryKind::Symlink {
                                let link = egui::Label::new(RichText::new(&entry.name).strong())
                                    .truncate()
                                    .sense(egui::Sense::click());
                                if ui
                                    .add(link)
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text(details)
                                    .clicked()
                                {
                                    action = Some(Action::Open(entry.name.clone()));
                                }
                            } else {
                                ui.add(egui::Label::new(&entry.name).truncate())
                                    .on_hover_text(details);
                            }
                        });
                    });
                });
                if draggable {
                    super::drag_area_drawn(ui, drag_id, row.response.rect);
                }
            }
        });

    match action {
        Some(Action::Open(name)) => pane.open(pane.child(&name)),
        Some(Action::Rename(name)) => {
            pane.dialog = Some(Dialog::Rename {
                from: name.clone(),
                name,
            });
        }
        Some(Action::Delete(entry)) => pane.dialog = Some(Dialog::Delete(entry)),
        None => {}
    }
}

#[derive(Clone, Copy, PartialEq)]
enum RowIcon {
    Rename,
    Delete,
}

/// A small square button with a drawn icon: a pencil for Rename, a bin for
/// Delete. The name is shown when the pointer rests on it.
fn icon_button(ui: &mut egui::Ui, theme: &Theme, icon: RowIcon, name: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    let hovered = response.hovered();
    let color = match (icon, hovered) {
        (RowIcon::Delete, true) => theme.danger,
        (_, true) => theme.text,
        (_, false) => theme.text_dim,
    };
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 6.0, theme.border);
    }
    let stroke = egui::Stroke::new(1.4, color);
    let c = rect.center();
    let at = |x: f32, y: f32| c + egui::vec2(x, y);
    match icon {
        RowIcon::Rename => {
            // A pencil pointing down to the left.
            painter.add(egui::Shape::closed_line(
                vec![at(2.5, -6.5), at(6.5, -2.5), at(-2.5, 6.5), at(-6.5, 2.5)],
                stroke,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![at(-6.5, 2.5), at(-2.5, 6.5), at(-7.0, 7.0)],
                color,
                stroke,
            ));
            painter.line_segment([at(0.0, -4.0), at(4.0, 0.0)], stroke);
        }
        RowIcon::Delete => {
            // A bin: lid with a handle, and a body with two lines.
            painter.line_segment([at(-6.5, -4.0), at(6.5, -4.0)], stroke);
            painter.line_segment([at(-2.0, -6.5), at(2.0, -6.5)], stroke);
            painter.line_segment([at(-2.0, -6.5), at(-2.0, -4.0)], stroke);
            painter.line_segment([at(2.0, -6.5), at(2.0, -4.0)], stroke);
            painter.add(egui::Shape::line(
                vec![at(-5.0, -4.0), at(-4.0, 7.0), at(4.0, 7.0), at(5.0, -4.0)],
                stroke,
            ));
            painter.line_segment([at(-1.5, -1.0), at(-1.5, 4.0)], stroke);
            painter.line_segment([at(1.5, -1.0), at(1.5, 4.0)], stroke);
        }
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(name)
}

/// Small picture for a folder, a file or a link.
fn icon(ui: &mut egui::Ui, theme: &Theme, kind: EntryKind) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let painter = ui.painter();
    let c = rect.center();
    match kind {
        EntryKind::Dir => {
            let color = theme.accent_text;
            // Tab, then the body.
            painter.rect_filled(
                egui::Rect::from_min_max(c + egui::vec2(-8.0, -6.5), c + egui::vec2(-1.0, -2.0)),
                1.5,
                color,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(c + egui::vec2(-8.0, -4.0), c + egui::vec2(8.0, 6.5)),
                2.0,
                color,
            );
        }
        _ => {
            // A sheet of paper with a folded corner.
            let stroke = egui::Stroke::new(1.2, theme.text_dim);
            let (left, right, top, bottom, fold) = (-5.5, 5.5, -7.5, 7.5, 4.0);
            let outline = vec![
                c + egui::vec2(left, top),
                c + egui::vec2(right - fold, top),
                c + egui::vec2(right, top + fold),
                c + egui::vec2(right, bottom),
                c + egui::vec2(left, bottom),
            ];
            painter.add(egui::Shape::closed_line(outline, stroke));
            painter.line_segment(
                [
                    c + egui::vec2(right - fold, top),
                    c + egui::vec2(right - fold, top + fold),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    c + egui::vec2(right - fold, top + fold),
                    c + egui::vec2(right, top + fold),
                ],
                stroke,
            );
            if kind == EntryKind::Symlink {
                // An arrow: this is a link to something else.
                let arrow = egui::Stroke::new(1.5, theme.accent_text);
                let (from, to) = (c + egui::vec2(-3.0, 4.5), c + egui::vec2(2.5, -1.0));
                painter.line_segment([from, to], arrow);
                painter.line_segment([to, to + egui::vec2(-4.0, 0.0)], arrow);
                painter.line_segment([to, to + egui::vec2(0.0, 4.0)], arrow);
            }
        }
    }
}

/// Dim text in a column of fixed width, so rows line up.
fn fixed(ui: &mut egui::Ui, width: f32, text: &str, theme: &Theme) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.add(egui::Label::new(RichText::new(text).small().color(theme.text_dim)).truncate());
        },
    );
}

fn transfers_list(ui: &mut egui::Ui, theme: &Theme, transfers: &mut Vec<TransferRow>) {
    if transfers.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Transfers").strong());
        let finished = transfers
            .iter()
            .any(|row| !matches!(row.state, TransferState::Running));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if finished && ui.small_button("Clear finished").clicked() {
                transfers.retain(|row| matches!(row.state, TransferState::Running));
            }
        });
    });
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("sftp_transfer_rows")
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for row in transfers.iter().rev() {
                ui.push_id(row.id, |ui| transfer_row(ui, theme, row));
            }
        });
}

fn transfer_row(ui: &mut egui::Ui, theme: &Theme, row: &TransferRow) {
    ui.horizontal(|ui| {
        ui.set_height(TRANSFER_ROW_HEIGHT - 2.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match &row.state {
                TransferState::Running => {
                    if ui.small_button("Cancel").clicked() {
                        row.handle.cancel();
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
                            .desired_width(160.0)
                            .fill(theme.accent),
                    );
                }
                TransferState::Done => {
                    let mut text = format!("Done, {}", sftp::format_size(row.total));
                    if row.skipped > 0 {
                        text.push_str(&format!(
                            " ({} links or special files skipped)",
                            row.skipped
                        ));
                    }
                    ui.label(RichText::new(text).small().color(theme.accent_text));
                }
                TransferState::Failed(error) => {
                    ui.add(
                        egui::Label::new(RichText::new(error).small().color(theme.danger))
                            .truncate(),
                    )
                    .on_hover_text(error);
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(&row.name).truncate());
                ui.add(
                    egui::Label::new(RichText::new(&row.route).small().color(theme.text_dim))
                        .truncate(),
                );
            });
        });
    });
}
