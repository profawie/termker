//! Port Forwarding page: saved rules, their live status, and the editor.

use eframe::egui::{self, RichText};
use uuid::Uuid;

use crate::{
    app::{ActiveForward, App, ForwardState},
    model::{Forward, ForwardKind, HostError},
};

#[derive(Default)]
pub struct ForwardsPage {
    editor: Option<Editor>,
}

/// Forward draft being edited. Ports are kept as text and validated on save.
struct Editor {
    draft: Forward,
    bind_port: String,
    dest_port: String,
    is_new: bool,
    error: Option<String>,
    confirm_delete: bool,
}

impl Editor {
    fn new(forward: Forward, is_new: bool) -> Self {
        let port_text = |port: u16| {
            if port == 0 {
                String::new()
            } else {
                port.to_string()
            }
        };
        Self {
            bind_port: port_text(forward.bind_port),
            dest_port: port_text(forward.dest_port),
            draft: forward,
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

enum RowAction {
    Start,
    Stop,
    Edit,
}

impl ForwardsPage {
    pub fn show(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let theme = app.theme;
        let first_host = app.data.hosts.first().map(|host| host.id);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                super::page_header(
                    ui,
                    &theme,
                    "Port Forwarding",
                    "Reach services through your hosts with local, remote and dynamic tunnels.",
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let button = super::primary_button(&theme, "+  New forward");
                if ui.add_enabled(first_host.is_some(), button).clicked()
                    && let Some(host_id) = first_host
                {
                    self.editor = Some(Editor::new(Forward::new(host_id), true));
                }
            });
        });

        if first_host.is_none() {
            empty_state(
                ui,
                app,
                "Add a host first",
                "A forward runs through one of your hosts.",
            );
            return;
        }
        if app.data.forwards.is_empty() {
            empty_state(
                ui,
                app,
                "No forwards yet",
                "Create one to open a tunnel through a host.",
            );
            return;
        }

        let mut action: Option<(Uuid, RowAction)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for index in 0..app.data.forwards.len() {
                let id = app.data.forwards[index].id;
                let selected = self
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.draft.id == id);
                if let Some(row_action) = row(ui, app, index, selected) {
                    action = Some((id, row_action));
                }
            }
        });

        match action {
            Some((id, RowAction::Start)) => app.start_forward(ui.ctx(), id),
            Some((id, RowAction::Stop)) => app.stop_forward(id),
            Some((id, RowAction::Edit)) => {
                if let Some(forward) = app.data.forwards.iter().find(|forward| forward.id == id) {
                    self.editor = Some(Editor::new(forward.clone(), false));
                }
            }
            None => {}
        }
    }

    /// Right-hand panel for adding or editing a forward.
    pub fn editor_panel(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let theme = app.theme;
        let mut action = None;

        egui::Panel::right("forward_editor")
            .exact_size(360.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme.panel)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ui, |ui| {
                let title = if editor.is_new {
                    "New forward"
                } else {
                    "Edit forward"
                };
                ui.label(RichText::new(title).size(18.0).strong());
                ui.add_space(8.0);

                field(
                    ui,
                    "Label",
                    &mut editor.draft.label,
                    "Optional, e.g. Database",
                );

                ui.label(RichText::new("Type").small().color(theme.text_dim));
                ui.horizontal(|ui| {
                    for kind in ForwardKind::ALL {
                        ui.selectable_value(&mut editor.draft.kind, kind, kind.title());
                    }
                });

                ui.label(RichText::new("Through host").small().color(theme.text_dim));
                let host_name = app
                    .data
                    .host(editor.draft.host_id)
                    .map(|host| host.display_name().to_string())
                    .unwrap_or_else(|| "Choose a host".to_string());
                egui::ComboBox::from_id_salt("forward_host")
                    .selected_text(&host_name)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for host in &app.data.hosts {
                            ui.selectable_value(
                                &mut editor.draft.host_id,
                                host.id,
                                host.display_name(),
                            );
                        }
                    });

                let remote = editor.draft.kind == ForwardKind::Remote;
                let listen_label = if remote {
                    "Listen address on the host"
                } else {
                    "Listen address on this computer"
                };
                field(
                    ui,
                    listen_label,
                    &mut editor.draft.bind_address,
                    "127.0.0.1",
                );
                field(ui, "Listen port", &mut editor.bind_port, "8080");
                if editor.draft.kind != ForwardKind::Dynamic {
                    let dest_label = if remote {
                        "Destination, as seen from this computer"
                    } else {
                        "Destination, as seen from the host"
                    };
                    field(
                        ui,
                        dest_label,
                        &mut editor.draft.dest_host,
                        "localhost or db.internal",
                    );
                    field(ui, "Destination port", &mut editor.dest_port, "5432");
                }

                // Preview in plain words, using what has been typed so far.
                let mut preview = editor.draft.clone();
                preview.bind_port = editor.bind_port.trim().parse().unwrap_or(0);
                preview.dest_port = editor.dest_port.trim().parse().unwrap_or(0);
                ui.add_space(4.0);
                ui.label(
                    RichText::new(preview.describe(&host_name))
                        .small()
                        .color(theme.text_dim),
                );
                if !editor.draft.binds_loopback() && !editor.draft.bind_address.trim().is_empty() {
                    ui.label(
                        RichText::new(
                            "This listen address is reachable from other machines on the \
                             network, not only from the one that listens.",
                        )
                        .small()
                        .color(theme.danger),
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
                                RichText::new("Delete this forward? This cannot be undone.")
                                    .color(theme.danger),
                            );
                        } else if ui
                            .button(RichText::new("Delete forward").color(theme.danger))
                            .clicked()
                        {
                            editor.confirm_delete = true;
                        }
                    });
                }
            });

        match action {
            Some(EditorAction::Save) => {
                let mut forward = editor.draft.clone();
                let Ok(bind_port) = editor.bind_port.trim().parse::<u16>() else {
                    editor.error = Some(HostError::InvalidPort.to_string());
                    return;
                };
                forward.bind_port = bind_port;
                if forward.kind != ForwardKind::Dynamic {
                    let Ok(dest_port) = editor.dest_port.trim().parse::<u16>() else {
                        editor.error = Some(HostError::InvalidPort.to_string());
                        return;
                    };
                    forward.dest_port = dest_port;
                }
                let id = forward.id;
                match app.data.upsert_forward(forward) {
                    Ok(()) => {
                        // A running forward keeps its old settings; stop it
                        // so the list never shows a rule that is not in effect.
                        app.stop_forward(id);
                        app.save();
                        self.editor = None;
                    }
                    Err(err) => editor.error = Some(err.to_string()),
                }
            }
            Some(EditorAction::Cancel) => self.editor = None,
            Some(EditorAction::Delete) => {
                let id = editor.draft.id;
                app.stop_forward(id);
                app.data.remove_forward(id);
                app.save();
                self.editor = None;
            }
            None => {}
        }
    }
}

fn empty_state(ui: &mut egui::Ui, app: &App, title: &str, text: &str) {
    let theme = app.theme;
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new("<->")
                .monospace()
                .size(36.0)
                .color(theme.accent),
        );
        ui.label(RichText::new(title).size(18.0).strong());
        ui.label(RichText::new(text).color(theme.text_dim));
    });
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

/// One forward: what it does, its status, and its buttons. A pending
/// connection prompt is shown inside the same card.
fn row(ui: &mut egui::Ui, app: &mut App, index: usize, selected: bool) -> Option<RowAction> {
    let theme = app.theme;
    let forward = app.data.forwards[index].clone();
    let host_name = app
        .data
        .host(forward.host_id)
        .map(|host| host.display_name().to_string())
        .unwrap_or_else(|| "deleted host".to_string());
    let active: Option<&mut ActiveForward> = app
        .active_forwards
        .iter_mut()
        .find(|active| active.forward_id == forward.id);

    let mut frame = super::card(&theme);
    if selected {
        frame = frame.stroke(egui::Stroke::new(1.5, theme.accent));
    }
    let mut action = None;
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (dot, status) = match active.as_ref().map(|active| &active.state) {
            None => (theme.text_dim, "Stopped".to_string()),
            Some(ForwardState::Connecting(text)) => (theme.text_dim, format!("{text}...")),
            Some(ForwardState::Active { port }) => {
                let connections = active.as_ref().map_or(0, |active| active.connections);
                let noun = if connections == 1 {
                    "connection"
                } else {
                    "connections"
                };
                (
                    theme.accent,
                    format!("Listening on port {port}, {connections} {noun}"),
                )
            }
            Some(ForwardState::Closed(None)) => (theme.text_dim, "Stopped".to_string()),
            Some(ForwardState::Closed(Some(error))) => (theme.danger, error.clone()),
        };
        let running = matches!(
            active.as_ref().map(|active| &active.state),
            Some(ForwardState::Connecting(_) | ForwardState::Active { .. })
        );

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(forward.display_name()).size(15.0).strong());
                    ui.label(
                        RichText::new(format!("{} via {host_name}", forward.kind.title()))
                            .small()
                            .color(theme.text_dim),
                    );
                });
                // Without a label the summary is already the title.
                if !forward.label.is_empty() {
                    ui.label(
                        RichText::new(forward.summary())
                            .monospace()
                            .small()
                            .color(theme.text_dim),
                    );
                }
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.0, dot);
                    ui.label(RichText::new(status).small().color(dot));
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if running {
                    if ui.button("Stop").clicked() {
                        action = Some(RowAction::Stop);
                    }
                } else if ui.add(super::primary_button(&theme, "Start")).clicked() {
                    action = Some(RowAction::Start);
                }
                if ui.button("Edit").clicked() {
                    action = Some(RowAction::Edit);
                }
            });
        });

        if let Some(active) = active
            && active.prompt.is_some()
        {
            ui.add_space(8.0);
            super::session::prompt_card(ui, &theme, &mut active.prompt, &mut active.secret_input);
        }
    });
    ui.add_space(4.0);
    action
}
