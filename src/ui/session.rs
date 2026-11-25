//! Session page: status bar, connection prompts and the terminal.

use eframe::egui::{self, RichText};

use crate::{
    app::App,
    session::SessionState,
    ssh::{HostKeyStatus, Prompt, Secret},
    theme::Theme,
};

/// What the user asked for from the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAction {
    None,
    Close,
    Reconnect,
}

/// What one pane of the session page did this frame.
pub struct PaneOutput {
    pub action: SessionAction,
    /// Clicked: it should be the one typed into.
    pub activate: bool,
    /// Its bar was picked up to move it, in the frame that happened.
    pub grab: Option<super::Grab>,
}

/// One session's pane: its bar and its terminal, in all of `ui`. `active`
/// is the pane typed into; `split` says other panes are shown beside it,
/// so the bar can be dragged to move it.
pub fn show(
    app: &mut App,
    ui: &mut egui::Ui,
    index: usize,
    active: bool,
    split: bool,
) -> PaneOutput {
    let theme = app.theme;
    let terminal_theme = app.terminal_theme;
    let font_size = app.data.settings.terminal_font_size;
    let session = &mut app.sessions[index];
    let mut output = PaneOutput {
        action: SessionAction::None,
        activate: false,
        grab: None,
    };

    // Beside other panes, the bar is a handle: drag it to move the pane.
    let bar_id = egui::Id::unique(("session_bar_drag", session.id));
    if split
        && let Some((response, grab)) = super::press_area(ui, bar_id, egui::Sense::click_and_drag())
    {
        output.grab = grab;
        output.activate |= response.clicked() || response.drag_started();
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }

    let bar = egui::Panel::top(("session_bar", session.id))
        .frame(
            egui::Frame::new()
                .fill(theme.panel)
                .inner_margin(egui::Margin::symmetric(16, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if split {
                    // Pressing the text picks up the pane instead of
                    // selecting the words.
                    ui.style_mut().interaction.selectable_labels = false;
                    grip(ui, &theme).on_hover_text(
                        "Drag to move this terminal. Drop it on Sessions in the sidebar \
                         to show it on its own again.",
                    );
                }
                let (dot, status) = match &session.state {
                    SessionState::Connecting(text) => (theme.text_dim, format!("{text}...")),
                    SessionState::Connected => (theme.accent_text, "Connected".to_string()),
                    SessionState::Closed(None) => (theme.text_dim, "Disconnected".to_string()),
                    SessionState::Closed(Some(_)) => (theme.danger, "Failed".to_string()),
                };
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, dot);
                let title = RichText::new(
                    session
                        .emulator
                        .title()
                        .unwrap_or_else(|| session.name.clone()),
                )
                .strong();
                let title = if active {
                    title
                } else {
                    title.color(theme.text_dim)
                };
                ui.add(egui::Label::new(title).truncate().selectable(false));
                if session.hops.len() > 1 && !split {
                    ui.label(
                        RichText::new(format!("via {}", session.route()))
                            .small()
                            .color(theme.text_dim),
                    );
                }
                ui.label(RichText::new(status).small().color(theme.text_dim));
                if let (Some(os), true, false) = (&session.os, session.is_connected(), split) {
                    ui.add_space(10.0);
                    super::os::summary(ui, &theme, os);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close = if session.is_closed() {
                        "Close tab"
                    } else {
                        "Disconnect"
                    };
                    if ui.button(close).clicked() {
                        output.action = SessionAction::Close;
                    }
                    if session.is_closed()
                        && ui.add(super::primary_button(&theme, "Reconnect")).clicked()
                    {
                        output.action = SessionAction::Reconnect;
                    }
                });
            });
        });
    let bar_rect = bar.response.rect;
    if split {
        super::drag_area_drawn(ui, bar_id, bar_rect);
        if active {
            // A line under the bar marks the pane being typed into.
            let line = egui::Rect::from_min_max(
                egui::pos2(bar_rect.left(), bar_rect.bottom() - 2.0),
                bar_rect.max,
            );
            ui.painter().rect_filled(line, 0.0, theme.accent_text);
        }
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(terminal_theme.bg))
        .show(ui, |ui| {
            if let SessionState::Closed(Some(error)) = &session.state {
                notice(ui, &theme, error);
            }
            let prompting = session.prompt.is_some();
            if prompting {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(16))
                    .show(ui, |ui| {
                        prompt_card_with(
                            ui,
                            &theme,
                            &mut session.prompt,
                            &mut session.secret_input,
                            active,
                        );
                    });
            }
            output.activate |=
                super::terminal::show(ui, session, &terminal_theme, font_size, !prompting, active);
        });

    output
}

/// Six dots that say the bar can be picked up.
fn grip(ui: &mut egui::Ui, theme: &Theme) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10.0, 16.0), egui::Sense::hover());
    let color = if response.hovered() {
        theme.text
    } else {
        theme.text_dim
    };
    for row in 0..3 {
        for col in 0..2 {
            let center = egui::pos2(
                rect.left() + 2.5 + col as f32 * 5.0,
                rect.top() + 3.0 + row as f32 * 5.0,
            );
            ui.painter().circle_filled(center, 1.3, color);
        }
    }
    response
}

fn notice(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    egui::Frame::new()
        .fill(theme.surface)
        .inner_margin(egui::Margin::symmetric(16, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(theme.danger));
        });
}

/// Card for a pending connection prompt. Answering it clears `pending`.
/// `secret_input` holds what is being typed into a password prompt.
pub fn prompt_card(
    ui: &mut egui::Ui,
    theme: &Theme,
    pending: &mut Option<Prompt>,
    secret_input: &mut String,
) {
    prompt_card_with(ui, theme, pending, secret_input, true);
}

/// [`prompt_card`], where `take_focus` says whether its password field
/// takes the keyboard: false while another pane is being typed into.
fn prompt_card_with(
    ui: &mut egui::Ui,
    theme: &Theme,
    pending: &mut Option<Prompt>,
    secret_input: &mut String,
    take_focus: bool,
) {
    let Some(prompt) = pending.take() else {
        return;
    };
    let mut keep = Some(prompt);
    super::card(theme).show(ui, |ui| {
        ui.set_max_width(ui.available_width().min(560.0));
        match keep.take() {
            Some(Prompt::HostKey {
                hop,
                address,
                port,
                algorithm,
                fingerprint,
                status,
                reply,
            }) => {
                let changed = status == HostKeyStatus::Changed;
                if changed {
                    ui.label(RichText::new("Warning: host key has changed").size(16.0).strong().color(theme.danger));
                    ui.label(format!(
                        "The key {hop} ({address}:{port}) presents now is different from the one saved earlier. \
                         Someone may be intercepting the connection, or the server was reinstalled. \
                         Continue only if you know why it changed."
                    ));
                } else {
                    ui.label(RichText::new("Unknown host").size(16.0).strong());
                    ui.label(format!(
                        "This is the first connection to {hop} ({address}:{port}). \
                         Check that the fingerprint matches the server before you trust it."
                    ));
                }
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{algorithm} key fingerprint")).small().color(theme.text_dim));
                ui.add(egui::Label::new(RichText::new(&fingerprint).monospace()).wrap());
                ui.add_space(8.0);
                let mut answer = None;
                ui.horizontal(|ui| {
                    if changed {
                        if ui.add(super::primary_button(theme, "Cancel")).clicked() {
                            answer = Some(false);
                        }
                        let replace = RichText::new("Replace saved key and connect").color(theme.danger);
                        if ui.button(replace).clicked() {
                            answer = Some(true);
                        }
                    } else {
                        if ui.add(super::primary_button(theme, "Trust and connect")).clicked() {
                            answer = Some(true);
                        }
                        if ui.button("Cancel").clicked() {
                            answer = Some(false);
                        }
                    }
                });
                match answer {
                    Some(answer) => {
                        let _ = reply.send(answer);
                    }
                    None => {
                        keep = Some(Prompt::HostKey {
                            hop,
                            address,
                            port,
                            algorithm,
                            fingerprint,
                            status,
                            reply,
                        });
                    }
                }
            }
            Some(Prompt::Password {
                hop,
                username,
                retry,
                can_remember,
                reply,
            }) => {
                let title = format!("Password for {username}@{hop}");
                let error = retry.then_some("That password was not accepted. Try again.");
                match secret_form(ui, theme, &title, error, can_remember, secret_input, take_focus) {
                    Some(answer) => {
                        let _ = reply.send(answer);
                    }
                    None => {
                        keep = Some(Prompt::Password {
                            hop,
                            username,
                            retry,
                            can_remember,
                            reply,
                        });
                    }
                }
            }
            Some(Prompt::Passphrase {
                key_path,
                retry,
                can_remember,
                reply,
            }) => {
                let title = format!("Passphrase for {key_path}");
                let error = retry.then_some("That passphrase did not unlock the key. Try again.");
                match secret_form(ui, theme, &title, error, can_remember, secret_input, take_focus) {
                    Some(answer) => {
                        let _ = reply.send(answer);
                    }
                    None => {
                        keep = Some(Prompt::Passphrase {
                            key_path,
                            retry,
                            can_remember,
                            reply,
                        });
                    }
                }
            }
            None => {}
        }
    });
    *pending = keep;
}

/// Masked input with Connect and Cancel. Returns `Some(Some(secret))` when
/// submitted, `Some(None)` when cancelled, `None` while still open.
fn secret_form(
    ui: &mut egui::Ui,
    theme: &Theme,
    title: &str,
    error: Option<&str>,
    can_remember: bool,
    input: &mut String,
    take_focus: bool,
) -> Option<Option<Secret>> {
    ui.label(RichText::new(title).size(16.0).strong());
    if let Some(error) = error {
        ui.label(RichText::new(error).color(theme.danger));
    }
    let field = ui.add(
        egui::TextEdit::singleline(input)
            .password(true)
            .desired_width(320.0),
    );
    if take_focus && !field.has_focus() && !field.lost_focus() {
        field.request_focus();
    }
    let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));

    // The tick box starts empty for every prompt, so nothing is saved
    // unless the user asks for it this time.
    let remember_id = egui::Id::unique("remember_secret");
    let mut remember =
        can_remember && ui.data(|data| data.get_temp::<bool>(remember_id).unwrap_or(false));
    if can_remember {
        ui.checkbox(&mut remember, "Remember in this computer's keychain");
        ui.data_mut(|data| data.insert_temp(remember_id, remember));
    }

    let mut result = None;
    ui.horizontal(|ui| {
        if ui.add(super::primary_button(theme, "Connect")).clicked() || entered {
            result = Some(Some(Secret {
                value: std::mem::take(input),
                remember,
            }));
        }
        if ui.button("Cancel").clicked() {
            input.clear();
            result = Some(None);
        }
    });
    if result.is_some() {
        ui.data_mut(|data| data.remove::<bool>(remember_id));
    }
    let note = if remember {
        "Saved in the system credential store after a successful sign-in. \
         Remove it any time on the Keychain page."
    } else {
        "Not saved. It is used once for this connection."
    };
    ui.label(RichText::new(note).small().color(theme.text_dim));
    result
}
