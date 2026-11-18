//! Hosts page: host cards by group, and the editor panel.

use eframe::egui::{self, RichText};
use uuid::Uuid;

use crate::{
    app::App,
    model::{self, AuthMethod, Data, Host},
};

const CARD_INNER_WIDTH: f32 = 240.0;
/// Card width including inner margin (14 per side) and border.
const CARD_OUTER_WIDTH: f32 = CARD_INNER_WIDTH + 30.0;

#[derive(Default)]
pub struct HostsPage {
    search: String,
    editor: Option<Editor>,
    group_form: Option<GroupForm>,
}

/// A small form shown above the host list for managing groups.
enum GroupForm {
    New {
        /// Group the new one goes inside; empty for the top level.
        parent: String,
        name: String,
        error: Option<String>,
    },
    Rename {
        from: String,
        /// Only the last level of the name is edited.
        name: String,
        error: Option<String>,
    },
    Delete(String),
}

/// Full name of the group `name` inside `parent`.
fn inside(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}{}{name}", model::GROUP_SEPARATOR)
    }
}

/// A group's full name for display: `Client > Prod`.
fn shown(group: &str) -> String {
    group.replace(model::GROUP_SEPARATOR, " > ")
}

/// A host card being dragged to another group.
#[derive(Clone)]
struct DraggedHost {
    id: Uuid,
    name: String,
    /// Group it is in now.
    group: String,
}

/// What the user asked for on a group's header row.
enum GroupAction {
    Toggle(String, bool),
    AddHost(String),
    AddGroup(String),
    Rename(String),
    Delete(String),
}

/// Host draft being edited. The port is kept as text so the user can type
/// freely; it is validated on save.
struct Editor {
    draft: Host,
    port: String,
    is_new: bool,
    error: Option<String>,
    confirm_delete: bool,
    /// The group is being typed as a new name instead of picked.
    new_group: bool,
}

impl Editor {
    fn new_host() -> Self {
        Self::from_host(Host::new(), true)
    }

    fn from_host(host: Host, is_new: bool) -> Self {
        Self {
            port: host.port.to_string(),
            draft: host,
            is_new,
            error: None,
            confirm_delete: false,
            new_group: false,
        }
    }

    fn new_host_in(group: &str) -> Self {
        let mut editor = Self::new_host();
        editor.draft.group = group.to_string();
        editor
    }
}

enum EditorAction {
    Save,
    Cancel,
    Delete,
    Duplicate,
}

impl HostsPage {
    pub fn show(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let theme = app.theme;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                super::page_header(
                    ui,
                    &theme,
                    "Hosts",
                    "Servers you connect to over SSH. Drag a host onto a group to move it.",
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                // One button for both, so the header stays short.
                let new = ui.add(super::primary_button(&theme, "+  New"));
                egui::Popup::menu(&new).show(|ui| {
                    if ui.button("New host").clicked() {
                        self.editor = Some(Editor::new_host());
                        ui.close();
                    }
                    if ui.button("New group").clicked() {
                        self.group_form = Some(GroupForm::New {
                            parent: String::new(),
                            name: String::new(),
                            error: None,
                        });
                        ui.close();
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search hosts")
                        .desired_width(220.0),
                );
            });
        });

        self.group_form(app, ui);

        if app.data.hosts.is_empty() && app.data.groups.is_empty() {
            self.empty_state(app, ui);
            return;
        }

        let sections = sections(&app.data, &self.search);
        if sections.is_empty() {
            ui.label(RichText::new("No hosts match your search.").color(theme.text_dim));
            return;
        }

        let mut edit: Option<Uuid> = None;
        let mut connect: Option<Uuid> = None;
        let mut group_action: Option<GroupAction> = None;
        // A host card dragged onto a group moves there when dropped.
        let dragged = egui::DragAndDrop::payload::<DraggedHost>(ui.ctx());
        let pointer = ui.ctx().pointer_latest_pos();
        let released = ui.input(|input| input.pointer.any_released());
        let mut over_group: Option<String> = None;
        let mut moved: Option<(Uuid, String)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Leave room for the scroll bar next to the group buttons.
            ui.set_width(ui.available_width() - 12.0);
            ui.add_space(4.0);
            // Mark the area from `top` to here as a place to drop the
            // dragged host, unless it is the group the host is already in.
            let mut drop_zone = |ui: &mut egui::Ui, top: f32, indent: f32, group: &str| {
                let Some(dragged) = &dragged else {
                    return;
                };
                if dragged.group == group {
                    return;
                }
                let rect = egui::Rect::from_min_max(
                    egui::pos2(ui.max_rect().left() + indent - 6.0, top - 3.0),
                    egui::pos2(ui.max_rect().right(), ui.cursor().top() - 5.0),
                );
                let over = pointer.is_some_and(|pointer| {
                    rect.contains(pointer) && ui.clip_rect().contains(pointer)
                });
                super::drop_outline(ui, &theme, rect, over);
                if over {
                    over_group = Some(group.to_string());
                    if released {
                        moved = Some((dragged.id, group.to_string()));
                    }
                }
            };
            for section in &sections {
                let indent = section.depth as f32 * INDENT;
                let top = ui.cursor().top();
                if let Some(action) = group_header(ui, app, section, indent) {
                    group_action = Some(action);
                }
                if section.collapsed {
                    ui.add_space(6.0);
                    drop_zone(ui, top, indent, section.name);
                    continue;
                }
                if section.hosts.is_empty() && section.total == 0 && !section.has_groups {
                    ui.horizontal(|ui| {
                        ui.add_space(indent + INDENT);
                        ui.label(
                            RichText::new("No hosts in this group yet.")
                                .small()
                                .color(theme.text_dim),
                        );
                    });
                }
                // Column count follows the available width, so cards are
                // never hidden under the editor panel.
                let gap = ui.spacing().item_spacing.x;
                let columns = ((ui.available_width() - indent + gap) / (CARD_OUTER_WIDTH + gap))
                    .floor()
                    .max(1.0) as usize;
                for row in section.hosts.chunks(columns) {
                    ui.horizontal(|ui| {
                        if indent > 0.0 {
                            ui.add_space(indent - gap);
                        }
                        for host in row {
                            let selected = self
                                .editor
                                .as_ref()
                                .is_some_and(|editor| editor.draft.id == host.id);
                            match host_card(ui, app, host, selected) {
                                CardAction::Edit => edit = Some(host.id),
                                CardAction::Connect => connect = Some(host.id),
                                CardAction::None => {}
                            }
                        }
                    });
                }
                ui.add_space(if section.hosts.is_empty() { 6.0 } else { 12.0 });
                drop_zone(ui, top, indent, section.name);
            }
            // With every host in a group there is no Ungrouped section to
            // drop on, so offer one while dragging.
            if dragged.is_some() && sections.iter().all(|section| !section.name.is_empty()) {
                let top = ui.cursor().top();
                ui.label(
                    RichText::new("Ungrouped")
                        .size(15.0)
                        .strong()
                        .color(theme.text_dim),
                );
                ui.label(
                    RichText::new("Drop here to take the host out of its group.")
                        .small()
                        .color(theme.text_dim),
                );
                ui.add_space(8.0);
                drop_zone(ui, top, 0.0, "");
            }
        });
        if let Some(dragged) = &dragged {
            let hint = match &over_group {
                Some(group) if group.is_empty() => "Move out of its group".to_string(),
                Some(group) => format!("Move to {}", shown(group)),
                None => "Drop on a group to move".to_string(),
            };
            super::drag_chip(ui.ctx(), &theme, &dragged.name, &hint, over_group.is_some());
        }
        drop(sections);
        if let Some((id, group)) = moved {
            app.data.move_host(id, &group);
            // Show where it went.
            app.data.set_group_collapsed(&group, false);
            if let Some(editor) = &mut self.editor
                && editor.draft.id == id
            {
                editor.draft.group = group;
            }
            app.save();
        }
        match group_action {
            Some(GroupAction::Toggle(name, collapsed)) => {
                app.data.set_group_collapsed(&name, collapsed);
                app.save();
            }
            Some(GroupAction::AddHost(name)) => self.editor = Some(Editor::new_host_in(&name)),
            Some(GroupAction::AddGroup(parent)) => {
                self.group_form = Some(GroupForm::New {
                    parent,
                    name: String::new(),
                    error: None,
                });
            }
            Some(GroupAction::Rename(name)) => {
                self.group_form = Some(GroupForm::Rename {
                    name: model::group_leaf(&name).to_string(),
                    from: name,
                    error: None,
                });
            }
            Some(GroupAction::Delete(name)) => self.group_form = Some(GroupForm::Delete(name)),
            None => {}
        }
        if let Some(host) = edit.and_then(|id| app.data.host(id)) {
            self.editor = Some(Editor::from_host(host.clone(), false));
        }
        if let Some(id) = connect {
            self.editor = None;
            app.connect(ui.ctx(), id);
        }
    }

    /// The form for adding, renaming or deleting a group, when one is open.
    fn group_form(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let Some(mut form) = self.group_form.take() else {
            return;
        };
        let theme = app.theme;
        let mut close = false;
        super::card(&theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match &mut form {
                GroupForm::New {
                    parent,
                    name,
                    error,
                } => {
                    let title = if parent.is_empty() {
                        "New group".to_string()
                    } else {
                        format!("New group inside {}", shown(parent))
                    };
                    ui.label(RichText::new(title).strong());
                    match name_form(ui, &theme, name, "Create") {
                        Some(true) => match app.data.add_group(&inside(parent, name)) {
                            Ok(()) => {
                                // Open the groups around it so it is seen.
                                app.data.set_group_collapsed(parent, false);
                                app.save();
                                close = true;
                            }
                            Err(err) => *error = Some(err.to_string()),
                        },
                        Some(false) => close = true,
                        None => {}
                    }
                    ui.label(
                        RichText::new(
                            "Tip: use / to make a group inside another, for example Client/Prod.",
                        )
                        .small()
                        .color(theme.text_dim),
                    );
                    if let Some(error) = error {
                        ui.label(RichText::new(error.as_str()).color(theme.danger));
                    }
                }
                GroupForm::Rename { from, name, error } => {
                    ui.label(RichText::new(format!("Rename group {}", shown(from))).strong());
                    match name_form(ui, &theme, name, "Rename") {
                        Some(true) => {
                            let to = model::group_path(&inside(model::group_parent(from), name));
                            match app.data.rename_group(from, &to) {
                                Ok(()) => {
                                    // An open editor keeps pointing at the group.
                                    if let Some(editor) = &mut self.editor
                                        && model::in_group(&editor.draft.group, from)
                                    {
                                        editor.draft.group =
                                            format!("{to}{}", &editor.draft.group[from.len()..]);
                                    }
                                    app.save();
                                    close = true;
                                }
                                Err(err) => *error = Some(err.to_string()),
                            }
                        }
                        Some(false) => close = true,
                        None => {}
                    }
                    if let Some(error) = error {
                        ui.label(RichText::new(error.as_str()).color(theme.danger));
                    }
                }
                GroupForm::Delete(name) => {
                    ui.label(
                        RichText::new(format!("Delete the group {}?", shown(name)))
                            .strong()
                            .color(theme.danger),
                    );
                    let parent = model::group_parent(name);
                    let to = if parent.is_empty() {
                        "Ungrouped".to_string()
                    } else {
                        shown(parent)
                    };
                    let groups = app
                        .data
                        .groups
                        .iter()
                        .filter(|group| model::in_group(&group.name, name))
                        .count()
                        - 1;
                    let mut note = match app.data.hosts_under_group(name) {
                        0 => "It has no hosts.".to_string(),
                        1 => format!("Its host is kept and moves to {to}."),
                        count => format!("Its {count} hosts are kept and move to {to}."),
                    };
                    match groups {
                        0 => {}
                        1 => note.push_str(" The group inside it is deleted too."),
                        count => note
                            .push_str(&format!(" The {count} groups inside it are deleted too.")),
                    }
                    ui.label(RichText::new(note).color(theme.text_dim));
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("Delete group").color(theme.danger))
                            .clicked()
                        {
                            app.data.remove_group(name);
                            if let Some(editor) = &mut self.editor
                                && model::in_group(&editor.draft.group, name)
                            {
                                editor.draft.group = parent.to_string();
                            }
                            app.save();
                            close = true;
                        }
                        if ui.add(super::primary_button(&theme, "Keep")).clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        ui.add_space(8.0);
        if !close {
            self.group_form = Some(form);
        }
    }

    fn empty_state(&mut self, app: &App, ui: &mut egui::Ui) {
        let theme = app.theme;
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(">_")
                    .monospace()
                    .size(40.0)
                    .color(theme.accent_text),
            );
            ui.label(RichText::new("No hosts yet").size(18.0).strong());
            ui.label(RichText::new("Add your first server to get started.").color(theme.text_dim));
            ui.add_space(8.0);
            if ui
                .add(super::primary_button(&theme, "+  New host"))
                .clicked()
            {
                self.editor = Some(Editor::new_host());
            }
        });
    }

    /// Right-hand panel for adding or editing a host.
    pub fn editor_panel(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let theme = app.theme;
        let mut action = None;

        egui::Panel::right("host_editor")
            .exact_size(340.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme.panel)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ui, |ui| {
                let title = if editor.is_new {
                    "New host"
                } else {
                    "Edit host"
                };
                ui.label(RichText::new(title).size(18.0).strong());
                ui.add_space(8.0);

                field(ui, "Label", &mut editor.draft.label, "My server");
                field(
                    ui,
                    "Address",
                    &mut editor.draft.address,
                    "example.com or 10.0.0.5",
                );
                field(ui, "Port", &mut editor.port, "22");
                group_field(ui, app, editor);

                sign_in_fields(ui, app, &mut editor.draft);

                ui.label(RichText::new("Jump host").small().color(theme.text_dim));
                let current = editor
                    .draft
                    .jump_host
                    .and_then(|id| app.data.host(id))
                    .map(|host| host.display_name().to_string())
                    .unwrap_or_else(|| "None (connect directly)".to_string());
                egui::ComboBox::from_id_salt("jump_host")
                    .selected_text(current)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut editor.draft.jump_host,
                            None,
                            "None (connect directly)",
                        );
                        for other in &app.data.hosts {
                            if other.id == editor.draft.id {
                                continue;
                            }
                            ui.selectable_value(
                                &mut editor.draft.jump_host,
                                Some(other.id),
                                other.display_name(),
                            );
                        }
                    });
                if let Ok(path) = app.data.connection_path(&editor.draft)
                    && path.len() > 1
                {
                    let route: Vec<&str> = path
                        .iter()
                        .map(|host| {
                            if host.id == editor.draft.id {
                                "this host"
                            } else {
                                host.display_name()
                            }
                        })
                        .collect();
                    ui.label(
                        RichText::new(format!("Route: you > {}", route.join(" > ")))
                            .small()
                            .color(theme.text_dim),
                    );
                }

                if let Some(error) = &editor.error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(error).color(theme.danger));
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(super::primary_button(&theme, "Save")).clicked() {
                        action = Some(EditorAction::Save);
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(EditorAction::Cancel);
                    }
                    if !editor.is_new
                        && ui
                            .button("Duplicate")
                            .on_hover_text("Save a copy of this host and edit the copy")
                            .clicked()
                    {
                        action = Some(EditorAction::Duplicate);
                    }
                });

                if !editor.is_new {
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        if editor.confirm_delete {
                            ui.horizontal(|ui| {
                                let confirm = egui::Button::new(
                                    RichText::new("Yes, delete").color(theme.danger),
                                );
                                if ui.add(confirm).clicked() {
                                    action = Some(EditorAction::Delete);
                                }
                                if ui.button("Keep").clicked() {
                                    editor.confirm_delete = false;
                                }
                            });
                            ui.label(
                                RichText::new("Delete this host? This cannot be undone.")
                                    .color(theme.danger),
                            );
                        } else if ui
                            .button(RichText::new("Delete host").color(theme.danger))
                            .clicked()
                        {
                            editor.confirm_delete = true;
                        }
                    });
                }
            });

        match action {
            Some(EditorAction::Save) => {
                let Ok(port) = editor.port.trim().parse::<u16>() else {
                    editor.error = Some(crate::model::HostError::InvalidPort.to_string());
                    return;
                };
                let mut host = editor.draft.clone();
                host.port = port;
                match app.data.upsert_host(host) {
                    Ok(()) => {
                        app.save();
                        self.editor = None;
                    }
                    Err(err) => editor.error = Some(err.to_string()),
                }
            }
            Some(EditorAction::Cancel) => self.editor = None,
            Some(EditorAction::Duplicate) => {
                // The copy takes what the form shows now; the host being
                // edited keeps what was last saved.
                let Ok(port) = editor.port.trim().parse::<u16>() else {
                    editor.error = Some(crate::model::HostError::InvalidPort.to_string());
                    return;
                };
                let mut copy = app.data.duplicate_of(&editor.draft);
                copy.port = port;
                match app.data.upsert_host(copy.clone()) {
                    Ok(()) => {
                        app.save();
                        let copy = app.data.host(copy.id).cloned().unwrap_or(copy);
                        self.editor = Some(Editor::from_host(copy, false));
                    }
                    Err(err) => editor.error = Some(err.to_string()),
                }
            }
            Some(EditorAction::Delete) => {
                app.data.remove_host(editor.draft.id);
                app.save();
                self.editor = None;
            }
            None => {}
        }
    }
}

use super::field;

/// Which sign-in choice is selected in the editor.
#[derive(Clone, Copy, PartialEq)]
enum SignIn {
    Password,
    KeychainKey,
    KeyFile,
}

/// Identity picker, and the username and sign-in method used without one.
fn sign_in_fields(ui: &mut egui::Ui, app: &App, draft: &mut Host) {
    let theme = app.theme;
    let small = |text: &str| RichText::new(text).small().color(theme.text_dim);

    if !app.data.identities.is_empty() {
        ui.label(small("Identity"));
        let current = draft
            .identity
            .and_then(|id| app.data.identity(id))
            .map(|identity| identity.name.clone())
            .unwrap_or_else(|| "None (enter details below)".to_string());
        egui::ComboBox::from_id_salt("host_identity")
            .selected_text(current)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut draft.identity, None, "None (enter details below)");
                for identity in &app.data.identities {
                    ui.selectable_value(&mut draft.identity, Some(identity.id), &identity.name);
                }
            });
    }
    if let Some(identity) = draft.identity.and_then(|id| app.data.identity(id)) {
        let sign_in = match identity.key.and_then(|id| app.data.key(id)) {
            Some(key) => format!("key \"{}\"", key.name),
            None => "a password".to_string(),
        };
        ui.label(small(&format!(
            "Signs in as {} with {sign_in}. Change it in Keychain.",
            identity.username
        )));
        return;
    }

    field(ui, "Username", &mut draft.username, "root");
    ui.label(small("Sign in with"));
    let selected = match draft.auth {
        AuthMethod::Password => SignIn::Password,
        AuthMethod::Key { .. } => SignIn::KeychainKey,
        AuthMethod::KeyFile { .. } => SignIn::KeyFile,
    };
    ui.horizontal(|ui| {
        if ui
            .selectable_label(selected == SignIn::Password, "Password")
            .clicked()
        {
            draft.auth = AuthMethod::Password;
        }
        if ui
            .selectable_label(selected == SignIn::KeychainKey, "Keychain key")
            .clicked()
            && selected != SignIn::KeychainKey
        {
            // Start on the first key; with none, the note below explains.
            draft.auth = AuthMethod::Key {
                key: app
                    .data
                    .keys
                    .first()
                    .map(|key| key.id)
                    .unwrap_or_else(Uuid::nil),
            };
        }
        if ui
            .selectable_label(selected == SignIn::KeyFile, "Key file")
            .clicked()
            && selected != SignIn::KeyFile
        {
            draft.auth = AuthMethod::KeyFile {
                path: String::new(),
            };
        }
    });
    match &mut draft.auth {
        AuthMethod::Password => {
            ui.label(small(
                "You are asked for the password when connecting, and can choose to save it then.",
            ));
        }
        AuthMethod::Key { key } => {
            if app.data.keys.is_empty() {
                ui.label(small(
                    "The keychain has no keys yet. Generate or import one on the Keychain page.",
                ));
            } else {
                let current = app
                    .data
                    .key(*key)
                    .map(|entry| entry.name.clone())
                    .unwrap_or_else(|| "Choose a key".to_string());
                egui::ComboBox::from_id_salt("host_key")
                    .selected_text(current)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for entry in &app.data.keys {
                            ui.selectable_value(
                                key,
                                entry.id,
                                format!("{}  ({})", entry.name, entry.algorithm),
                            );
                        }
                    });
            }
        }
        AuthMethod::KeyFile { path } => {
            field(ui, "Key file", path, "~/.ssh/id_ed25519");
        }
    }
}

enum CardAction {
    None,
    Edit,
    Connect,
}

/// Width of the system badge in the top right corner of a card.
const OS_BADGE_SIZE: f32 = 20.0;

/// One host card, with its Connect and Edit buttons.
fn host_card(ui: &mut egui::Ui, app: &App, host: &Host, selected: bool) -> CardAction {
    let theme = app.theme;
    let mut frame = super::card(&theme);
    if selected {
        frame = frame.stroke(egui::Stroke::new(1.5, theme.accent_text));
    }
    let mut action = CardAction::None;
    // The card can be picked up anywhere and dropped on another group.
    let drag_id = ui.scope_id().with(("drag", host.id));
    if super::drag_area(ui, drag_id) {
        egui::DragAndDrop::set_payload(
            ui.ctx(),
            DraggedHost {
                id: host.id,
                name: host.display_name().to_string(),
                group: host.group.clone(),
            },
        );
    }
    let card = frame.show(ui, |ui| {
        // Dragging over the text moves the card instead of selecting it.
        ui.style_mut().interaction.selectable_labels = false;
        ui.set_width(CARD_INNER_WIDTH);
        ui.vertical(|ui| {
            let name =
                egui::Label::new(RichText::new(host.display_name()).size(15.0).strong()).truncate();
            // The system it ran the last time it was connected to sits top
            // right. The row is as high without one, so cards stay equal.
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match &host.os {
                        Some(os) => {
                            super::os::badge(ui, os, OS_BADGE_SIZE);
                        }
                        None => {
                            ui.allocate_space(egui::vec2(0.0, OS_BADGE_SIZE));
                        }
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(name)
                    });
                });
            });
            ui.add(
                egui::Label::new(
                    RichText::new(app.data.target(host))
                        .monospace()
                        .small()
                        .color(theme.text_dim),
                )
                .truncate(),
            );
            let via = host
                .jump_host
                .and_then(|id| app.data.host(id))
                .map(|jump| format!("via {}", jump.display_name()))
                .unwrap_or_else(|| "direct".to_string());
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(via).small().color(theme.text_dim)).truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Edit").clicked() {
                        action = CardAction::Edit;
                    }
                });
            });
            ui.add_space(2.0);
            let connect = super::primary_button(&theme, "Connect")
                .min_size(egui::vec2(ui.available_width(), 28.0));
            if ui.add(connect).clicked() {
                action = CardAction::Connect;
            }
        });
    });
    super::drag_area_drawn(ui, drag_id, card.response.rect);
    action
}

/// Width one level of nesting is moved to the right.
const INDENT: f32 = 22.0;

/// One group on the Hosts page with the hosts to show in it.
struct Section<'a> {
    /// Full name; empty for hosts without a group.
    name: &'a str,
    /// What the header shows: the last level, or the full path in a search.
    title: String,
    /// How many groups it sits inside.
    depth: usize,
    /// Hosts directly in the group.
    hosts: Vec<&'a Host>,
    /// Hosts in the group and in the groups inside it.
    total: usize,
    has_groups: bool,
    collapsed: bool,
}

/// Groups in tree order with their hosts by name, then the hosts without a
/// group. Groups inside a closed group are left out. While searching, only
/// groups with a matching host are listed, opened, by their full name.
fn sections<'a>(data: &'a Data, search: &str) -> Vec<Section<'a>> {
    let needle = search.trim().to_lowercase();
    let searching = !needle.is_empty();
    let hosts_of = |name: &str| {
        let mut hosts: Vec<&Host> = data
            .hosts
            .iter()
            .filter(|host| host.group == name)
            .filter(|host| {
                !searching
                    || format!(
                        "{} {} {} {}",
                        host.label, host.address, host.username, host.group
                    )
                    .to_lowercase()
                    .contains(&needle)
            })
            .collect();
        hosts.sort_by_key(|host| host.display_name().to_lowercase());
        hosts
    };
    let mut sections = Vec::new();
    // Name of the outermost closed group being skipped.
    let mut closed: Option<&str> = None;
    for group in &data.groups {
        let name = group.name.as_str();
        let hosts = hosts_of(name);
        if searching {
            if !hosts.is_empty() {
                sections.push(Section {
                    name,
                    title: shown(name),
                    depth: 0,
                    total: hosts.len(),
                    hosts,
                    has_groups: false,
                    collapsed: false,
                });
            }
            continue;
        }
        if closed.is_some_and(|closed| model::in_group(name, closed)) {
            continue;
        }
        closed = group.collapsed.then_some(name);
        sections.push(Section {
            name,
            title: model::group_leaf(name).to_string(),
            depth: name.matches(model::GROUP_SEPARATOR).count(),
            hosts,
            total: data.hosts_under_group(name),
            has_groups: data
                .groups
                .iter()
                .any(|other| other.name != name && model::in_group(&other.name, name)),
            collapsed: group.collapsed,
        });
    }
    let ungrouped = hosts_of("");
    if !ungrouped.is_empty() {
        sections.push(Section {
            name: "",
            title: "Ungrouped".to_string(),
            depth: 0,
            total: ungrouped.len(),
            hosts: ungrouped,
            has_groups: false,
            collapsed: false,
        });
    }
    sections
}

/// Header row of a group: open/close arrow, name, host count and actions.
fn group_header(
    ui: &mut egui::Ui,
    app: &App,
    section: &Section,
    indent: f32,
) -> Option<GroupAction> {
    let theme = app.theme;
    let mut action = None;
    let named = !section.name.is_empty();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if indent > 0.0 {
            ui.add_space(indent - 6.0);
        }
        let title = section.title.as_str();
        if named {
            let (rect, arrow) =
                ui.allocate_exact_size(egui::vec2(14.0, 20.0), egui::Sense::click());
            let center = rect.center();
            let points = if section.collapsed {
                vec![
                    center + egui::vec2(-3.0, -5.0),
                    center + egui::vec2(4.0, 0.0),
                    center + egui::vec2(-3.0, 5.0),
                ]
            } else {
                vec![
                    center + egui::vec2(-5.0, -3.0),
                    center + egui::vec2(5.0, -3.0),
                    center + egui::vec2(0.0, 4.0),
                ]
            };
            ui.painter().add(egui::Shape::convex_polygon(
                points,
                theme.text_dim,
                egui::Stroke::NONE,
            ));
            let name = ui.add(
                egui::Label::new(RichText::new(title).size(15.0).strong())
                    .truncate()
                    .sense(egui::Sense::click()),
            );
            let hint = if section.collapsed {
                "Show hosts"
            } else {
                "Hide hosts"
            };
            if arrow
                .union(name)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(hint)
                .clicked()
            {
                action = Some(GroupAction::Toggle(
                    section.name.to_string(),
                    !section.collapsed,
                ));
            }
        } else {
            ui.label(
                RichText::new(title)
                    .size(15.0)
                    .strong()
                    .color(theme.text_dim),
            );
        }
        let count = match section.total {
            1 => "1 host".to_string(),
            count => format!("{count} hosts"),
        };
        ui.label(RichText::new(count).small().color(theme.text_dim));
        if named {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let group = section.name.to_string();
                if super::icon_button(ui, &theme, super::RowIcon::Delete, "Delete group").clicked()
                {
                    action = Some(GroupAction::Delete(group.clone()));
                }
                if super::icon_button(ui, &theme, super::RowIcon::Rename, "Rename group").clicked()
                {
                    action = Some(GroupAction::Rename(group.clone()));
                }
                if super::icon_button(
                    ui,
                    &theme,
                    super::RowIcon::NewHost,
                    "New host in this group",
                )
                .clicked()
                {
                    action = Some(GroupAction::AddHost(group.clone()));
                }
                if super::icon_button(
                    ui,
                    &theme,
                    super::RowIcon::NewGroup,
                    "New group inside this one",
                )
                .clicked()
                {
                    action = Some(GroupAction::AddGroup(group));
                }
            });
        }
    });
    ui.add_space(2.0);
    action
}

/// Group picker in the host editor: an existing group, none, or a new name.
fn group_field(ui: &mut egui::Ui, app: &App, editor: &mut Editor) {
    let theme = app.theme;
    ui.label(RichText::new("Group").small().color(theme.text_dim));
    if editor.new_group {
        ui.horizontal(|ui| {
            let choose = ui.button("Choose existing");
            ui.add(
                egui::TextEdit::singleline(&mut editor.draft.group)
                    .hint_text("New group, e.g. Client/Prod")
                    .desired_width(f32::INFINITY),
            );
            if choose.clicked() {
                editor.new_group = false;
                if app.data.group(&editor.draft.group).is_none() {
                    editor.draft.group.clear();
                }
            }
        });
        return;
    }
    const NO_GROUP: &str = "No group";
    let current = if editor.draft.group.is_empty() {
        NO_GROUP.to_string()
    } else {
        shown(&editor.draft.group)
    };
    egui::ComboBox::from_id_salt("host_group")
        .selected_text(current)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut editor.draft.group, String::new(), NO_GROUP);
            for group in &app.data.groups {
                ui.selectable_value(
                    &mut editor.draft.group,
                    group.name.clone(),
                    shown(&group.name),
                );
            }
            ui.separator();
            if ui.selectable_label(false, "+  New group...").clicked() {
                editor.new_group = true;
                editor.draft.group.clear();
            }
        });
}

/// Name input with a confirm button and Cancel. Returns `Some(true)` when a
/// name was confirmed, `Some(false)` on Cancel, `None` while open.
fn name_form(
    ui: &mut egui::Ui,
    theme: &crate::theme::Theme,
    name: &mut String,
    confirm: &str,
) -> Option<bool> {
    let mut result = None;
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(name)
                .hint_text("e.g. Production")
                .desired_width(280.0),
        );
        if !field.has_focus() && !field.lost_focus() {
            field.request_focus();
        }
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let valid = !name.trim().is_empty();
        let button = ui.add_enabled(valid, super::primary_button(theme, confirm));
        if (button.clicked() || entered) && valid {
            result = Some(true);
        }
        if ui.button("Cancel").clicked() {
            result = Some(false);
        }
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Data {
        let mut data = Data::default();
        for (label, group) in [("web", "Prod"), ("db", "Prod"), ("lab", "")] {
            data.upsert_host(Host {
                label: label.into(),
                address: format!("{label}.internal"),
                group: group.into(),
                ..Host::new()
            })
            .unwrap();
        }
        data.add_group("Empty").unwrap();
        data
    }

    fn names<'a>(sections: &'a [Section<'a>]) -> Vec<&'a str> {
        sections.iter().map(|section| section.name).collect()
    }

    #[test]
    fn groups_come_first_by_name_then_ungrouped_hosts() {
        let data = data();
        let sections = sections(&data, "");
        assert_eq!(names(&sections), ["Empty", "Prod", ""]);
        let prod: Vec<_> = sections[1].hosts.iter().map(|h| h.label.as_str()).collect();
        assert_eq!(prod, ["db", "web"]);
    }

    #[test]
    fn search_matches_any_field_and_hides_groups_without_a_match() {
        let data = data();
        let count = |search| {
            sections(&data, search)
                .iter()
                .map(|section| section.hosts.len())
                .sum::<usize>()
        };
        assert_eq!(count("PROD"), 2);
        assert_eq!(names(&sections(&data, "PROD")), ["Prod"]);
        assert_eq!(count("lab.int"), 1);
        assert!(sections(&data, "nothing").is_empty());
    }

    #[test]
    fn a_collapsed_group_opens_while_searching() {
        let mut data = data();
        data.set_group_collapsed("Prod", true);
        assert!(sections(&data, "")[1].collapsed);
        assert!(!sections(&data, "web")[0].collapsed);
    }

    #[test]
    fn groups_inside_groups_are_indented_and_hidden_when_the_outer_one_is_closed() {
        let mut data = data();
        data.upsert_host(Host {
            label: "server1".into(),
            address: "s1.internal".into(),
            group: "Klien1/Prod".into(),
            ..Host::new()
        })
        .unwrap();
        let listed = sections(&data, "");
        assert_eq!(
            names(&listed),
            ["Empty", "Klien1", "Klien1/Prod", "Prod", ""]
        );
        assert_eq!((listed[1].depth, listed[1].title.as_str()), (0, "Klien1"));
        assert_eq!((listed[2].depth, listed[2].title.as_str()), (1, "Prod"));
        // The outer group counts the hosts of the groups inside it.
        assert_eq!((listed[1].hosts.len(), listed[1].total), (0, 1));
        assert!(listed[1].has_groups);

        // A search lists the inner group by its full name.
        let found = sections(&data, "server1");
        assert_eq!(found[0].title, "Klien1 > Prod");

        data.set_group_collapsed("Klien1", true);
        assert_eq!(names(&sections(&data, "")), ["Empty", "Klien1", "Prod", ""]);
    }
}
