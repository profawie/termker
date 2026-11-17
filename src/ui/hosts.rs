//! Halaman Hosts: senarai kad host mengikut kumpulan, dan panel penyunting.

use std::collections::BTreeMap;

use eframe::egui::{self, RichText};
use uuid::Uuid;

use crate::{
    app::App,
    model::{Data, Host},
};

const CARD_INNER_WIDTH: f32 = 240.0;
/// Lebar kad termasuk margin dalaman (14 setiap sisi) dan sempadan.
const CARD_OUTER_WIDTH: f32 = CARD_INNER_WIDTH + 30.0;

#[derive(Default)]
pub struct HostsPage {
    search: String,
    editor: Option<Editor>,
}

/// Draf host yang sedang disunting. Port disimpan sebagai teks supaya
/// pengguna boleh menaip dengan bebas; ia disahkan semasa menyimpan.
struct Editor {
    draft: Host,
    port: String,
    is_new: bool,
    error: Option<String>,
    confirm_delete: bool,
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
        }
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
                ui.add(
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search hosts")
                        .desired_width(220.0),
                );
            });
        });

        if app.data.hosts.is_empty() {
            self.empty_state(app, ui);
            return;
        }

        let groups = grouped(&app.data, &self.search);
        if groups.is_empty() {
            ui.label(RichText::new("No hosts match your search.").color(theme.text_dim));
            return;
        }

        let mut edit: Option<Uuid> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Kumpulan bernama dahulu, host tanpa kumpulan di hujung.
            let ordered = groups
                .iter()
                .filter(|(group, _)| !group.is_empty())
                .chain(groups.iter().filter(|(group, _)| group.is_empty()));
            for (group, hosts) in ordered {
                let title = if group.is_empty() { "Ungrouped" } else { group };
                ui.label(
                    RichText::new(format!("{title}  ({})", hosts.len()))
                        .small()
                        .color(theme.text_dim),
                );
                // Bilangan lajur mengikut lebar yang ada, supaya kad tidak
                // terlindung di bawah panel penyunting.
                let gap = ui.spacing().item_spacing.x;
                let columns = ((ui.available_width() + gap) / (CARD_OUTER_WIDTH + gap))
                    .floor()
                    .max(1.0) as usize;
                for row in hosts.chunks(columns) {
                    ui.horizontal(|ui| {
                        for host in row {
                            let selected = self
                                .editor
                                .as_ref()
                                .is_some_and(|editor| editor.draft.id == host.id);
                            if host_card(ui, app, host, selected) {
                                edit = Some(host.id);
                            }
                        }
                    });
                }
                ui.add_space(12.0);
            }
        });
        if let Some(host) = edit.and_then(|id| app.data.host(id)) {
            self.editor = Some(Editor::from_host(host.clone(), false));
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
                    .color(theme.accent),
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

    /// Panel kanan untuk menambah atau menyunting host.
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
                field(ui, "Username", &mut editor.draft.username, "root");
                field(
                    ui,
                    "Group",
                    &mut editor.draft.group,
                    "Optional, e.g. Production",
                );

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

fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    let dim = ui.visuals().weak_text_color();
    ui.label(RichText::new(label).small().color(dim));
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    );
}

/// Kad satu host. Memulangkan `true` jika pengguna mahu menyuntingnya.
fn host_card(ui: &mut egui::Ui, app: &App, host: &Host, selected: bool) -> bool {
    let theme = app.theme;
    let mut frame = super::card(&theme);
    if selected {
        frame = frame.stroke(egui::Stroke::new(1.5, theme.accent));
    }
    let mut edit = false;
    frame.show(ui, |ui| {
        ui.set_width(CARD_INNER_WIDTH);
        ui.vertical(|ui| {
            ui.add(
                egui::Label::new(RichText::new(host.display_name()).size(15.0).strong()).truncate(),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(host.target())
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
                        edit = true;
                    }
                });
            });
        });
    });
    edit
}

/// Host yang sepadan dengan carian, dikumpulkan dan disusun mengikut nama.
fn grouped<'a>(data: &'a Data, search: &str) -> BTreeMap<&'a str, Vec<&'a Host>> {
    let needle = search.trim().to_lowercase();
    let mut groups: BTreeMap<&str, Vec<&Host>> = BTreeMap::new();
    for host in &data.hosts {
        let haystack = format!(
            "{} {} {} {}",
            host.label, host.address, host.username, host.group
        )
        .to_lowercase();
        if needle.is_empty() || haystack.contains(&needle) {
            groups.entry(host.group.as_str()).or_default().push(host);
        }
    }
    for hosts in groups.values_mut() {
        hosts.sort_by_key(|host| host.display_name().to_lowercase());
    }
    groups
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
        data
    }

    #[test]
    fn groups_are_sorted_with_hosts_by_name() {
        let data = data();
        let groups = grouped(&data, "");
        assert_eq!(groups.keys().copied().collect::<Vec<_>>(), ["", "Prod"]);
        let prod: Vec<_> = groups["Prod"].iter().map(|h| h.label.as_str()).collect();
        assert_eq!(prod, ["db", "web"]);
    }

    #[test]
    fn search_matches_any_field_case_insensitively() {
        let data = data();
        assert_eq!(grouped(&data, "PROD").values().flatten().count(), 2);
        assert_eq!(grouped(&data, "lab.int").values().flatten().count(), 1);
        assert!(grouped(&data, "nothing").is_empty());
    }
}
