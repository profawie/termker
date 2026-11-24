//! SFTP page: two file panes side by side. Each pane shows this computer or
//! one of the hosts. Dragging files and folders from one pane and dropping
//! them on the other copies them to the folder open there. Rows can be
//! selected several at a time, and the pane last clicked answers to the
//! keyboard.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use uuid::Uuid;

use crate::{
    app::App,
    sftp::{
        self, Command, CopyEvent, CopyHandle, Entry, EntryKind, Event, Place, Removal, SftpHandle,
    },
    ssh::Prompt,
    theme::Theme,
};

use super::selection::Selection;

const THIS_COMPUTER: &str = "This computer";
/// Transfers visible without scrolling, and the height of one.
const TRANSFER_ROWS_SHOWN: usize = 3;
const TRANSFER_ROW_HEIGHT: f32 = 30.0;
/// Height of the line under each file list: what is selected, or a hint.
const FOOTER_HEIGHT: f32 = 30.0;
/// Rows Page Up and Page Down move.
const PAGE_ROWS: isize = 10;
/// Seconds between typed letters that still count as one name to find.
const FIND_PAUSE: f64 = 1.0;
/// Names shown in the question before deleting several.
const DELETE_NAMES_SHOWN: usize = 6;

#[derive(Default)]
pub struct SftpPage {
    panes: [Pane; 2],
    /// The left pane opens on this computer the first time the page is shown.
    started: bool,
    transfers: Vec<TransferRow>,
    next_id: u64,
    /// A transfer waiting for the user to agree to replace what is there.
    replace: Option<Replace>,
    /// Pane the keyboard works in: the one last clicked.
    active: usize,
}

/// Files and folders being dragged from one pane to the other.
#[derive(Clone)]
struct Dragged {
    /// Pane they come from.
    from: usize,
    /// In list order; the row picked up is one of them.
    entries: Vec<Entry>,
    grab: super::Grab,
}

/// Space around a dragged row inside the card it is carried in.
const DRAG_PADDING: egui::Vec2 = egui::vec2(12.0, 6.0);
/// Widest a dragged row is shown.
const DRAG_WIDTH: f32 = 340.0;

struct Replace {
    /// Pane the items come from.
    from: usize,
    entries: Vec<Entry>,
    /// Names among them that the other pane already has.
    taken: Vec<String>,
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
    Delete(Vec<Entry>),
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
    selection: Selection,
    /// Name to select once the next listing arrives: the folder just left
    /// by going up.
    reveal: Option<String>,
    /// Bring the keyboard's row into view on the next frame.
    scroll_to_cursor: bool,
    /// Letters typed to jump to a name, and when the last one came.
    find: String,
    find_at: f64,
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
    Transfer(Vec<Entry>),
}

/// Names of `entries`, in list order.
fn names(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.name.as_str()).collect()
}

/// Whether a row can be copied: drives and special files cannot.
fn copyable(entry: &Entry) -> bool {
    entry.kind != EntryKind::Other
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

    /// Name of the folder shown, as it is listed in the folder above.
    fn name_in_parent(&self) -> Option<String> {
        if self.path.is_empty() {
            return None;
        }
        if !self.is_local() {
            let trimmed = self.path.trim_end_matches('/');
            return trimmed.rsplit('/').next().map(str::to_string);
        }
        match Path::new(&self.path).file_name() {
            Some(name) => Some(name.to_string_lossy().into_owned()),
            // A drive is listed by its full path.
            None => Some(self.path.clone()),
        }
    }

    /// Open the folder above, with the one just left selected.
    fn go_up(&mut self) {
        if let Some(up) = self.parent() {
            self.reveal = self.name_in_parent();
            self.open(up);
        }
    }

    /// The selected rows, in list order.
    fn selected(&self) -> Vec<Entry> {
        self.entries
            .iter()
            .filter(|entry| self.selection.is_selected(&entry.name))
            .cloned()
            .collect()
    }

    /// The row the keyboard is on.
    fn cursor_entry(&self) -> Option<&Entry> {
        let cursor = self.selection.cursor()?;
        self.entries.iter().find(|entry| entry.name == cursor)
    }

    /// Show a listing of `path`. The selection is kept when it is the same
    /// folder read again.
    fn loaded(&mut self, path: String, entries: Vec<Entry>) {
        let same = path == self.path;
        self.entries = entries;
        self.path_input = path.clone();
        self.path = path;
        self.error = None;
        if same {
            self.selection.retain(&names(&self.entries));
        } else {
            self.selection.clear();
        }
        if let Some(name) = self.reveal.take()
            && self.entries.iter().any(|entry| entry.name == name)
        {
            self.selection.select_only(&name);
            self.scroll_to_cursor = true;
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
            Ok(entries) => self.loaded(path, entries),
            Err(err) => {
                self.reveal = None;
                self.path_input = self.path.clone();
                self.error = Some(format!("Could not open {path}: {err}"));
            }
        }
    }

    fn poll(&mut self) {
        let Source::Remote(remote) = &self.source else {
            return;
        };
        let events: Vec<Event> = remote.handle.events.try_iter().collect();
        for event in events {
            let Source::Remote(remote) = &mut self.source else {
                return;
            };
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
                        self.loading = false;
                        self.loaded(path, entries);
                    }
                }
                Event::Failed(text) => {
                    self.loading = false;
                    self.reveal = None;
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

    /// Delete `entries` from the folder shown. Folders go only when empty.
    fn delete(&mut self, entries: &[Entry]) {
        let items: Vec<Removal> = entries
            .iter()
            .map(|entry| Removal {
                path: self.child(&entry.name),
                is_dir: entry.kind == EntryKind::Dir,
            })
            .collect();
        match &self.source {
            Source::None => {}
            Source::Local => {
                let mut failures = Vec::new();
                for (entry, item) in entries.iter().zip(&items) {
                    let result = if item.is_dir {
                        std::fs::remove_dir(&item.path)
                    } else {
                        std::fs::remove_file(&item.path)
                    };
                    if let Err(err) = result {
                        failures.push((entry.name.clone(), err.to_string()));
                    }
                }
                self.refresh();
                if !failures.is_empty() {
                    self.error = Some(sftp::removal_error(&failures, items.len()));
                }
            }
            Source::Remote(remote) => remote.handle.send(Command::Remove(items)),
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
            &format!(
                "Drag files or folders from one pane to the other to copy them, including host to host. {} or Shift+click selects several, and the keyboard works in the pane last clicked.",
                command_key()
            ),
        );

        self.replace_card(app, ui);
        let key_action = self.keyboard(app, ui.ctx());

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
                    let active = self.active == index;
                    column.push_id(index, |ui| {
                        let (action, rect) =
                            pane_ui(ui, app, &mut self.panes[index], index, active);
                        actions[index] = action;
                        rects[index] = rect;
                    });
                }
            });
        });
        transfers_list(ui, &theme, &mut self.transfers);

        // The keyboard follows the pane last clicked.
        let pressed = ui.input(|input| {
            input
                .pointer
                .any_pressed()
                .then(|| input.pointer.interact_pos())
                .flatten()
        });
        if let Some(index) =
            pressed.and_then(|pointer| rects.iter().position(|rect| rect.contains(pointer)))
        {
            self.active = index;
        }
        if actions[self.active].is_none() {
            actions[self.active] = key_action;
        }

        // A row dragged out of one pane is copied where it is dropped on
        // the other.
        if let Some(dragged) = egui::DragAndDrop::payload::<Dragged>(ui.ctx()) {
            let to = 1 - dragged.from;
            let blocked = self.blocked(dragged.from);
            let over = ui
                .ctx()
                .pointer_latest_pos()
                .is_some_and(|pointer| rects[to].contains(pointer));
            let what = match dragged.entries.len() {
                1 => String::new(),
                count => format!("{count} items "),
            };
            let hint = match blocked {
                Some(reason) => reason.to_string(),
                None => format!(
                    "Drop to copy {what}to {} on {}",
                    self.panes[to].path,
                    self.panes[to].title()
                ),
            };
            if blocked.is_none() {
                super::drop_outline(ui, &theme, rects[to], over);
            }
            // The row itself comes along with the pointer.
            let hint = if over || blocked.is_some() {
                &hint
            } else {
                "Drop on the other pane to copy"
            };
            let ok = over && blocked.is_none();
            let entries = &dragged.entries;
            let grab = dragged.grab.framed(DRAG_PADDING, DRAG_WIDTH);
            super::drag_float(ui.ctx(), &theme, grab, DRAG_PADDING, hint, ok, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let bytes: u64 = entries
                            .iter()
                            .filter(|entry| entry.kind != EntryKind::Dir)
                            .map(|entry| entry.size)
                            .sum();
                        if entries.iter().any(|entry| entry.kind != EntryKind::Dir) {
                            fixed(ui, 62.0, &sftp::format_size(bytes), &theme);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let (kind, name) = match entries.as_slice() {
                                [entry] => (entry.kind, entry.name.clone()),
                                _ => (EntryKind::File, format!("{} items", entries.len())),
                            };
                            if entries.len() > 1 {
                                stack_icon(ui, &theme);
                            } else {
                                icon(ui, &theme, kind);
                            }
                            ui.add(egui::Label::new(RichText::new(name).strong()).truncate());
                        });
                    });
                });
            });
            if over && blocked.is_none() && ui.input(|input| input.pointer.any_released()) {
                actions[dragged.from] = Some(PaneAction::Transfer(dragged.entries.clone()));
            }
        }

        for (index, action) in actions.into_iter().enumerate() {
            match action {
                Some(PaneAction::Show(Choice::Local)) => self.panes[index].show_local(),
                Some(PaneAction::Show(Choice::Host(host_id))) => {
                    self.connect(app, ui.ctx(), index, host_id);
                }
                Some(PaneAction::Close) => self.panes[index] = Pane::default(),
                Some(PaneAction::Transfer(entries)) => {
                    let there = names(&self.panes[1 - index].entries);
                    let taken: Vec<String> = entries
                        .iter()
                        .filter(|entry| there.contains(&entry.name.as_str()))
                        .map(|entry| entry.name.clone())
                        .collect();
                    if taken.is_empty() {
                        for entry in &entries {
                            self.transfer(app, ui.ctx(), index, entry);
                        }
                    } else {
                        self.replace = Some(Replace {
                            from: index,
                            entries,
                            taken,
                        });
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

    /// Keys for the active pane. Returns what the page has to carry out.
    fn keyboard(&mut self, app: &mut App, ctx: &egui::Context) -> Option<PaneAction> {
        use egui::Key;
        // Typing in a box or a menu comes first.
        if ctx.text_edit_focused() || egui::Popup::is_any_open(ctx) {
            return None;
        }
        // While asking about replacing, Escape cancels.
        if self.replace.is_some() {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                self.replace = None;
            }
            return None;
        }
        const KEYS: [Key; 17] = [
            Key::ArrowUp,
            Key::ArrowDown,
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::PageUp,
            Key::PageDown,
            Key::Home,
            Key::End,
            Key::Enter,
            Key::Escape,
            Key::Backspace,
            Key::Delete,
            Key::A,
            Key::R,
            Key::N,
            Key::F2,
            Key::F5,
        ];
        let (keys, typed, now) = ctx.input_mut(|input| {
            let command = input.modifiers.command;
            let mut keys = Vec::new();
            let mut typed = String::new();
            input.events.retain(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if KEYS.contains(key) => {
                    keys.push((*key, *modifiers));
                    false
                }
                egui::Event::Text(text) if !command => {
                    typed.push_str(text);
                    false
                }
                _ => true,
            });
            (keys, typed, input.time)
        });

        let mut action = None;
        for (key, modifiers) in keys {
            let pane = &mut self.panes[self.active];
            // A question about deleting: Enter deletes, Escape keeps.
            if let Some(dialog) = &pane.dialog {
                if let (Dialog::Delete(entries), Key::Enter) = (dialog, key) {
                    let entries = entries.clone();
                    pane.dialog = None;
                    pane.delete(&entries);
                } else if key == Key::Escape {
                    pane.dialog = None;
                }
                continue;
            }
            // A list is shown: a folder, or this computer's drives.
            let ready = pane.location().is_some() || (pane.is_local() && pane.path.is_empty());
            if !ready || pane.loading {
                if let (Key::ArrowLeft | Key::ArrowRight, false) = (key, modifiers.any()) {
                    self.active = usize::from(key == Key::ArrowRight);
                }
                continue;
            }
            let drives = pane.path.is_empty();
            let shift = modifiers.shift;
            let names = names(&pane.entries);
            let mut moved = true;
            match key {
                Key::ArrowUp if modifiers.alt || modifiers.command => {
                    moved = false;
                    pane.go_up();
                }
                Key::ArrowUp => pane.selection.step(&names, -1, shift),
                Key::ArrowDown => pane.selection.step(&names, 1, shift),
                Key::PageUp => pane.selection.step(&names, -PAGE_ROWS, shift),
                Key::PageDown => pane.selection.step(&names, PAGE_ROWS, shift),
                Key::Home => pane.selection.jump(&names, 0, shift),
                Key::End => pane
                    .selection
                    .jump(&names, names.len().saturating_sub(1), shift),
                Key::ArrowLeft | Key::ArrowRight if !modifiers.any() => {
                    moved = false;
                    self.active = usize::from(key == Key::ArrowRight);
                }
                Key::A if modifiers.command => {
                    moved = false;
                    pane.selection.select_all(&names);
                }
                Key::Escape => {
                    moved = false;
                    pane.selection.clear();
                }
                Key::Enter => {
                    moved = false;
                    let target = pane
                        .cursor_entry()
                        .filter(|entry| matches!(entry.kind, EntryKind::Dir | EntryKind::Symlink))
                        .map(|entry| pane.child(&entry.name));
                    if let Some(path) = target {
                        pane.open(path);
                    }
                }
                Key::Delete | Key::Backspace
                    if !drives && (key == Key::Delete || modifiers.command) =>
                {
                    moved = false;
                    let selected = pane.selected();
                    if !selected.is_empty() {
                        pane.dialog = Some(Dialog::Delete(selected));
                    }
                }
                Key::Backspace => {
                    moved = false;
                    pane.go_up();
                }
                Key::F2 if !drives => {
                    moved = false;
                    let only = pane.selected();
                    if let [entry] = only.as_slice() {
                        pane.dialog = Some(Dialog::Rename {
                            from: entry.name.clone(),
                            name: entry.name.clone(),
                        });
                    }
                }
                Key::N if !drives && modifiers.command && shift => {
                    moved = false;
                    pane.dialog = Some(Dialog::NewFolder(String::new()));
                }
                Key::R if modifiers.command => {
                    moved = false;
                    pane.open(pane.path.clone());
                }
                Key::F5 if !drives => {
                    moved = false;
                    let selected: Vec<Entry> =
                        pane.selected().into_iter().filter(copyable).collect();
                    if !selected.is_empty() {
                        match self.blocked(self.active) {
                            Some(reason) => app.notify(reason.to_string()),
                            None => action = Some(PaneAction::Transfer(selected)),
                        }
                    }
                }
                _ => moved = false,
            }
            if moved {
                self.panes[self.active].scroll_to_cursor = true;
            }
        }

        // Typing a name's first letters goes to it.
        let typed: String = typed.chars().filter(|c| !c.is_control()).collect();
        let pane = &mut self.panes[self.active];
        if !typed.is_empty() && pane.dialog.is_none() && !pane.loading {
            if now - pane.find_at > FIND_PAUSE {
                pane.find.clear();
            }
            pane.find_at = now;
            // A space on its own is not the start of a name.
            if !(pane.find.is_empty() && typed.trim().is_empty()) {
                pane.find.push_str(&typed.to_lowercase());
                let names = names(&pane.entries);
                if let Some(index) = names
                    .iter()
                    .position(|name| name.to_lowercase().starts_with(&pane.find))
                {
                    pane.selection.jump(&names, index, false);
                    pane.scroll_to_cursor = true;
                }
            }
        }
        action
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
        enum Answer {
            Replace,
            Skip,
            Cancel,
        }
        let mut answer = None;
        let taken_dirs = replace
            .entries
            .iter()
            .filter(|entry| replace.taken.contains(&entry.name))
            .map(|entry| entry.kind == EntryKind::Dir);
        let (mut dirs, mut files) = (false, false);
        for is_dir in taken_dirs {
            dirs |= is_dir;
            files |= !is_dir;
        }
        let all_taken = replace.taken.len() == replace.entries.len();
        super::card(&theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let names = match replace.taken.as_slice() {
                [name] => name.clone(),
                taken => format!("{} of these ({})", taken.len(), list_names(taken)),
            };
            let verb = if replace.taken.len() == 1 {
                "exists"
            } else {
                "exist"
            };
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{names} already {verb} in {} on {}",
                        destination.path,
                        destination.title()
                    ))
                    .strong(),
                )
                .wrap(),
            );
            let note = match (dirs, files) {
                (true, false) => {
                    "The folders are merged: files with the same name are replaced, everything else is kept."
                }
                (false, _) => "The files there are replaced by these.",
                (true, true) => {
                    "Files there are replaced; folders are merged, keeping what is not copied over."
                }
            };
            ui.add(egui::Label::new(RichText::new(note).color(theme.text_dim)).wrap());
            ui.horizontal(|ui| {
                if ui
                    .button(RichText::new("Replace").color(theme.danger))
                    .clicked()
                {
                    answer = Some(Answer::Replace);
                }
                if !all_taken && ui.button("Skip those").clicked() {
                    answer = Some(Answer::Skip);
                }
                if ui.add(super::primary_button(&theme, "Cancel")).clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
        });
        ui.add_space(8.0);
        match answer {
            Some(Answer::Replace) => {
                for entry in &replace.entries {
                    self.transfer(app, ui.ctx(), replace.from, entry);
                }
            }
            Some(Answer::Skip) => {
                for entry in &replace.entries {
                    if !replace.taken.contains(&entry.name) {
                        self.transfer(app, ui.ctx(), replace.from, entry);
                    }
                }
            }
            Some(Answer::Cancel) => {}
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
    active: bool,
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
        file_list(ui, &theme, pane, index, active);
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
        if ui
            .add_enabled(pane.parent().is_some(), egui::Button::new("Up"))
            .on_hover_text("Backspace")
            .clicked()
        {
            pane.go_up();
        }
        if ui
            .button("Refresh")
            .on_hover_text(format!("{}+R", command_key()))
            .clicked()
        {
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
            Dialog::Delete(entries) => {
                let folders = entries
                    .iter()
                    .filter(|entry| entry.kind == EntryKind::Dir)
                    .count();
                let (question, it) = match entries.as_slice() {
                    [entry] => {
                        let what = if folders == 1 { "folder" } else { "file" };
                        (format!("Delete the {what} {}?", entry.name), "it")
                    }
                    _ => (format!("Delete {} items?", entries.len()), "them"),
                };
                ui.add(
                    egui::Label::new(RichText::new(question).strong().color(theme.danger)).wrap(),
                );
                if entries.len() > 1 {
                    let listed: Vec<String> =
                        entries.iter().map(|entry| entry.name.clone()).collect();
                    ui.add(egui::Label::new(list_names(&listed)).wrap());
                }
                let mut note = format!("This removes {it} from {place} and cannot be undone.");
                if folders > 0 {
                    note.push_str(" Folders are only deleted when they are empty.");
                }
                ui.add(egui::Label::new(RichText::new(note).color(theme.text_dim)).wrap());
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new("Delete").color(theme.danger))
                        .clicked()
                    {
                        pane.delete(entries);
                        close = true;
                    }
                    if ui.add(super::primary_button(theme, "Keep")).clicked() {
                        close = true;
                    }
                    ui.label(
                        RichText::new("Enter deletes, Esc keeps")
                            .small()
                            .color(theme.text_dim),
                    );
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

/// The files in a pane. Rows can be selected with the mouse and keyboard,
/// and dragged to the other pane; dragging a selected row takes the whole
/// selection along.
fn file_list(ui: &mut egui::Ui, theme: &Theme, pane: &mut Pane, index: usize, active: bool) {
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
        Delete(Vec<Entry>),
    }
    /// How a click changes the selection.
    enum Pick {
        Only(String),
        Toggle(String),
        Range { to: String, keep: bool },
    }
    let mut action = None;
    let mut pick = None;

    // The date only fits in a wide pane; it is always in the tooltip.
    let show_modified = ui.available_width() > 520.0;
    let drives = pane.path.is_empty();
    let modifiers = ui.input(|input| input.modifiers);
    let click_pick = |name: &str| {
        if modifiers.shift {
            Pick::Range {
                to: name.to_string(),
                keep: modifiers.command,
            }
        } else if modifiers.command {
            Pick::Toggle(name.to_string())
        } else {
            Pick::Only(name.to_string())
        }
    };
    let scroll_to = std::mem::take(&mut pane.scroll_to_cursor);
    let cursor = pane.selection.cursor().map(str::to_string);

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .max_height((ui.available_height() - FOOTER_HEIGHT).max(60.0))
        .show(ui, |ui| {
            // Dragging over a name moves the row instead of selecting text.
            ui.style_mut().interaction.selectable_labels = false;
            let lifted = egui::DragAndDrop::payload::<Dragged>(ui.ctx());
            for entry in &pane.entries {
                let is_dir = entry.kind == EntryKind::Dir;
                let opens = is_dir || entry.kind == EntryKind::Symlink;
                let selected = pane.selection.is_selected(&entry.name);
                let modified = entry.modified.map(sftp::format_time).unwrap_or_default();
                // The whole row can be clicked to select it, and picked up
                // and dropped on the other pane. Drives and special files
                // cannot be copied.
                let draggable = !drives && copyable(entry);
                let row_id = ui.scope_id().with(("drag", &entry.name));
                let sense = if draggable {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::click()
                };
                let mut hovered = false;
                if let Some((response, grab)) = super::press_area(ui, row_id, sense) {
                    hovered = response.hovered();
                    if response.double_clicked() && opens {
                        action = Some(Action::Open(entry.name.clone()));
                    } else if response.clicked() {
                        pick = Some(click_pick(&entry.name));
                    }
                    if let Some(grab) = grab.filter(|_| draggable) {
                        // A selected row brings the rest of the selection.
                        let entries = if selected {
                            pane.entries
                                .iter()
                                .filter(|other| {
                                    copyable(other) && pane.selection.is_selected(&other.name)
                                })
                                .cloned()
                                .collect()
                        } else {
                            pick = Some(Pick::Only(entry.name.clone()));
                            vec![entry.clone()]
                        };
                        egui::DragAndDrop::set_payload(
                            ui.ctx(),
                            Dragged {
                                from: index,
                                entries,
                                grab,
                            },
                        );
                    }
                }
                // While the rows are carried around, only a faint copy
                // stays behind.
                let lifted = lifted.as_ref().is_some_and(|dragged| {
                    dragged.from == index
                        && dragged.entries.iter().any(|other| other.name == entry.name)
                });
                let background = ui.painter().add(egui::Shape::Noop);
                let row = ui.horizontal(|ui| {
                    if lifted {
                        ui.set_opacity(super::DRAGGED_OPACITY);
                    }
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(10.0);
                        if !drives {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if super::icon_button(ui, theme, super::RowIcon::Delete, "Delete")
                                .clicked()
                            {
                                action = Some(Action::Delete(vec![entry.clone()]));
                            }
                            if super::icon_button(ui, theme, super::RowIcon::Rename, "Rename")
                                .clicked()
                            {
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
                            ui.add_space(4.0);
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
                            if opens {
                                // The name opens the folder, unless Ctrl or
                                // Shift is held to select it.
                                let link = egui::Label::new(RichText::new(&entry.name).strong())
                                    .truncate()
                                    .sense(egui::Sense::click());
                                if ui
                                    .add(link)
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text(details)
                                    .clicked()
                                {
                                    if modifiers.command || modifiers.shift {
                                        pick = Some(click_pick(&entry.name));
                                    } else {
                                        action = Some(Action::Open(entry.name.clone()));
                                    }
                                }
                            } else {
                                ui.add(egui::Label::new(&entry.name).truncate())
                                    .on_hover_text(details);
                            }
                        });
                    });
                });
                let rect = row.response.rect.expand2(egui::vec2(0.0, 1.0));
                let fill = if selected {
                    Some(theme.accent.gamma_multiply(0.30))
                } else if hovered {
                    Some(theme.text_dim.gamma_multiply(0.10))
                } else {
                    None
                };
                if let Some(fill) = fill {
                    ui.painter()
                        .set(background, egui::Shape::rect_filled(rect, 4.0, fill));
                }
                let at_cursor = cursor.as_deref() == Some(entry.name.as_str());
                if at_cursor && active {
                    ui.painter().rect_stroke(
                        rect,
                        4.0,
                        egui::Stroke::new(1.0, theme.accent_text.gamma_multiply(0.8)),
                        egui::StrokeKind::Inside,
                    );
                }
                if at_cursor && scroll_to {
                    ui.scroll_to_rect(rect, None);
                }
                super::drag_area_drawn(ui, row_id, row.response.rect);
            }
        });

    match pick {
        Some(Pick::Only(name)) => pane.selection.select_only(&name),
        Some(Pick::Toggle(name)) => pane.selection.toggle(&name),
        Some(Pick::Range { to, keep }) => {
            pane.selection.extend_to(&names(&pane.entries), &to, keep);
        }
        None => {}
    }
    if footer(ui, theme, pane, drives) {
        action = Some(Action::Delete(pane.selected()));
    }

    match action {
        Some(Action::Open(name)) => pane.open(pane.child(&name)),
        Some(Action::Rename(name)) => {
            pane.dialog = Some(Dialog::Rename {
                from: name.clone(),
                name,
            });
        }
        Some(Action::Delete(entries)) => pane.dialog = Some(Dialog::Delete(entries)),
        None => {}
    }
}

/// "Cmd" on a Mac, "Ctrl" elsewhere.
fn command_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    }
}

/// Every key the file list answers to.
fn shortcuts() -> String {
    let cmd = command_key();
    [
        "Click: select one".to_string(),
        format!("{cmd}+click: add or remove one"),
        "Shift+click: select everything between".to_string(),
        format!("{cmd}+A: select all, Esc: select none"),
        "Up, Down, Page Up, Page Down, Home, End: move (with Shift: select)".to_string(),
        "Type a name: go to it".to_string(),
        "Enter or double click: open a folder".to_string(),
        "Backspace: folder above".to_string(),
        format!("Delete or {cmd}+Backspace: delete what is selected"),
        "F2: rename".to_string(),
        "F5: copy what is selected to the other pane".to_string(),
        format!("{cmd}+Shift+N: new folder, {cmd}+R: refresh"),
        "Left, Right: work in the left or right pane".to_string(),
    ]
    .join("\n")
}

/// The line under a file list: what is selected, or how to select. Returns
/// true when Delete was clicked.
fn footer(ui: &mut egui::Ui, theme: &Theme, pane: &mut Pane, drives: bool) -> bool {
    let mut delete = false;
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), FOOTER_HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_height(FOOTER_HEIGHT);
            if pane.selection.is_empty() {
                let cmd = command_key();
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Click to select, {cmd} or Shift+click for more. Hover for keys."
                        ))
                        .small()
                        .color(theme.text_dim),
                    )
                    .truncate(),
                )
                .on_hover_text(shortcuts());
                return;
            }
            let selected = pane.selected();
            let mut text = format!("{} selected", selected.len());
            let bytes: u64 = selected
                .iter()
                .filter(|entry| entry.kind != EntryKind::Dir)
                .map(|entry| entry.size)
                .sum();
            if selected.iter().any(|entry| entry.kind != EntryKind::Dir) {
                text.push_str(&format!(", {}", sftp::format_size(bytes)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                if !drives
                    && super::icon_button(ui, theme, super::RowIcon::Delete, "Delete selected")
                        .clicked()
                {
                    delete = true;
                }
                if ui.small_button("Clear").on_hover_text("Esc").clicked() {
                    pane.selection.clear();
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(text).strong()).truncate())
                        .on_hover_text(shortcuts());
                });
            });
        },
    );
    delete
}

/// `names` joined by commas, cut short after a few.
fn list_names(names: &[String]) -> String {
    let mut text = names
        .iter()
        .take(DELETE_NAMES_SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > DELETE_NAMES_SHOWN {
        text.push_str(&format!(" and {} more", names.len() - DELETE_NAMES_SHOWN));
    }
    text
}

/// Two sheets of paper, one behind the other: several items.
fn stack_icon(ui: &mut egui::Ui, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let painter = ui.painter();
    let c = rect.center();
    let stroke = egui::Stroke::new(1.2, theme.text_dim);
    let back = egui::Rect::from_min_max(c + egui::vec2(-3.5, -8.0), c + egui::vec2(7.0, 5.0));
    let front = egui::Rect::from_min_max(c + egui::vec2(-7.0, -4.5), c + egui::vec2(3.5, 8.0));
    painter.rect_stroke(back, 1.5, stroke, egui::StrokeKind::Middle);
    painter.rect_filled(front, 1.5, theme.surface);
    painter.rect_stroke(
        front,
        1.5,
        egui::Stroke::new(1.2, theme.accent_text),
        egui::StrokeKind::Middle,
    );
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
