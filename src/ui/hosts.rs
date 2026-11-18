//! Hosts page: host cards by group, and the editor panel.

use eframe::egui::{self, RichText};
use uuid::Uuid;

use crate::{
    app::App,
    model::{AuthMethod, Data, Host},
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
        name: String,
        error: Option<String>,
    },
    Rename {
        from: String,
        name: String,
        error: Option<String>,
    },
    Delete(String),
}

/// What the user asked for on a group's header row.
enum GroupAction {
    Toggle(String, bool),
    AddHost(String),
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
}

impl HostsPage {
    pub fn show(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let theme = app.theme;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                super::page_header(ui, &theme, "Hosts", "Servers you connect to over SSH.");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui
                    .add(super::primary_button(&theme, "+  New host"))
                    .clicked()
                {
                    self.editor = Some(Editor::new_host());
                }
                if ui.button("+  New group").clicked() {
                    self.group_form = Some(GroupForm::New {
                        name: String::new(),
                        error: None,
                    });
                }
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
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Leave room for the scroll bar next to the group buttons.
            ui.set_width(ui.available_width() - 12.0);
            for section in &sections {
                if let Some(action) = group_header(ui, app, section) {
                    group_action = Some(action);
                }
                if section.collapsed {
                    ui.add_space(6.0);
                    continue;
                }
                if section.hosts.is_empty() {
                    ui.label(
                        RichText::new("No hosts in this group yet.")
                            .small()
                            .color(theme.text_dim),
                    );
                }
                // Column count follows the available width, so cards are
                // never hidden under the editor panel.
                let gap = ui.spacing().item_spacing.x;
                let columns = ((ui.available_width() + gap) / (CARD_OUTER_WIDTH + gap))
                    .floor()
                    .max(1.0) as usize;
                for row in section.hosts.chunks(columns) {
                    ui.horizontal(|ui| {
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
                ui.add_space(12.0);
            }
        });
        drop(sections);
        match group_action {
            Some(GroupAction::Toggle(name, collapsed)) => {
                app.data.set_group_collapsed(&name, collapsed);
                app.save();
            }
            Some(GroupAction::AddHost(name)) => self.editor = Some(Editor::new_host_in(&name)),
            Some(GroupAction::Rename(name)) => {
                self.group_form = Some(GroupForm::Rename {
                    from: name.clone(),
                    name,
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
                GroupForm::New { name, error } => {
                    ui.label(RichText::new("New group").strong());
                    match name_form(ui, &theme, name, "Create") {
                        Some(true) => match app.data.add_group(name) {
                            Ok(()) => {
                                app.save();
                                close = true;
                            }
                            Err(err) => *error = Some(err.to_string()),
                        },
                        Some(false) => close = true,
                        None => {}
                    }
                    if let Some(error) = error {
                        ui.label(RichText::new(error.as_str()).color(theme.danger));
                    }
                }
                GroupForm::Rename { from, name, error } => {
                    ui.label(RichText::new(format!("Rename group {from}")).strong());
                    match name_form(ui, &theme, name, "Rename") {
                        Some(true) => match app.data.rename_group(from, name) {
                            Ok(()) => {
                                // An open editor keeps pointing at the group.
                                if let Some(editor) = &mut self.editor
                                    && editor.draft.group == *from
                                {
                                    editor.draft.group = name.trim().to_string();
                                }
                                app.save();
                                close = true;
                            }
                            Err(err) => *error = Some(err.to_string()),
                        },
                        Some(false) => close = true,
                        None => {}
                    }
                    if let Some(error) = error {
                        ui.label(RichText::new(error.as_str()).color(theme.danger));
                    }
                }
                GroupForm::Delete(name) => {
                    ui.label(
                        RichText::new(format!("Delete the group {name}?"))
                            .strong()
                            .color(theme.danger),
                    );
                    let note = match app.data.hosts_in_group(name) {
                        0 => "It has no hosts.".to_string(),
                        1 => "Its host is kept and moves to Ungrouped.".to_string(),
                        count => format!("Its {count} hosts are kept and move to Ungrouped."),
                    };
                    ui.label(RichText::new(note).color(theme.text_dim));
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("Delete group").color(theme.danger))
                            .clicked()
                        {
                            app.data.remove_group(name);
                            if let Some(editor) = &mut self.editor
                                && editor.draft.group == *name
                            {
                                editor.draft.group.clear();
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

/// One host card, with its Connect and Edit buttons.
fn host_card(ui: &mut egui::Ui, app: &App, host: &Host, selected: bool) -> CardAction {
    let theme = app.theme;
    let mut frame = super::card(&theme);
    if selected {
        frame = frame.stroke(egui::Stroke::new(1.5, theme.accent_text));
    }
    let mut action = CardAction::None;
    frame.show(ui, |ui| {
        ui.set_width(CARD_INNER_WIDTH);
        ui.vertical(|ui| {
            ui.add(
                egui::Label::new(RichText::new(host.display_name()).size(15.0).strong()).truncate(),
            );
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
    action
}

/// One group on the Hosts page with the hosts to show in it.
struct Section<'a> {
    /// Empty for hosts without a group.
    name: &'a str,
    hosts: Vec<&'a Host>,
    collapsed: bool,
}

/// Groups by name with their hosts by name, then the hosts without a group.
/// While searching, only groups with a matching host are listed, opened.
fn sections<'a>(data: &'a Data, search: &str) -> Vec<Section<'a>> {
    let needle = search.trim().to_lowercase();
    let section = |name: &'a str, collapsed: bool| {
        let mut hosts: Vec<&Host> = data
            .hosts
            .iter()
            .filter(|host| host.group == name)
            .filter(|host| {
                needle.is_empty()
                    || format!(
                        "{} {} {} {}",
                        host.label, host.address, host.username, host.group
                    )
                    .to_lowercase()
                    .contains(&needle)
            })
            .collect();
        hosts.sort_by_key(|host| host.display_name().to_lowercase());
        Section {
            name,
            hosts,
            collapsed: collapsed && needle.is_empty(),
        }
    };
    let mut sections: Vec<Section> = data
        .groups
        .iter()
        .map(|group| section(&group.name, group.collapsed))
        .filter(|section| needle.is_empty() || !section.hosts.is_empty())
        .collect();
    let ungrouped = section("", false);
    if !ungrouped.hosts.is_empty() {
        sections.push(ungrouped);
    }
    sections
}

/// Header row of a group: open/close arrow, name, host count and actions.
fn group_header(ui: &mut egui::Ui, app: &App, section: &Section) -> Option<GroupAction> {
    let theme = app.theme;
    let mut action = None;
    let named = !section.name.is_empty();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let title = if named { section.name } else { "Ungrouped" };
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
        let count = match section.hosts.len() {
            1 => "1 host".to_string(),
            count => format!("{count} hosts"),
        };
        ui.label(RichText::new(count).small().color(theme.text_dim));
        if named {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Delete").clicked() {
                    action = Some(GroupAction::Delete(section.name.to_string()));
                }
                if ui.small_button("Rename").clicked() {
                    action = Some(GroupAction::Rename(section.name.to_string()));
                }
                if ui
                    .small_button("+ Host")
                    .on_hover_text("New host in this group")
                    .clicked()
                {
                    action = Some(GroupAction::AddHost(section.name.to_string()));
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
                    .hint_text("New group name")
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
        NO_GROUP
    } else {
        editor.draft.group.as_str()
    }
    .to_string();
    egui::ComboBox::from_id_salt("host_group")
        .selected_text(current)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut editor.draft.group, String::new(), NO_GROUP);
            for group in &app.data.groups {
                ui.selectable_value(&mut editor.draft.group, group.name.clone(), &group.name);
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
}
