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

pub fn show(app: &mut App, ui: &mut egui::Ui, index: usize) -> SessionAction {
    let theme = app.theme;
    let terminal_theme = app.terminal_theme;
    let font_size = app.data.settings.terminal_font_size;
    let session = &mut app.sessions[index];
    let mut action = SessionAction::None;

    egui::Panel::top(("session_bar", session.id))
        .frame(
            egui::Frame::new()
                .fill(theme.panel)
                .inner_margin(egui::Margin::symmetric(16, 8)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (dot, status) = match &session.state {
                    SessionState::Connecting(text) => (theme.text_dim, format!("{text}...")),
                    SessionState::Connected => (theme.accent, "Connected".to_string()),
                    SessionState::Closed(None) => (theme.text_dim, "Disconnected".to_string()),
                    SessionState::Closed(Some(_)) => (theme.danger, "Failed".to_string()),
                };
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, dot);
                ui.label(
                    RichText::new(
                        session
                            .emulator
                            .title()
                            .unwrap_or_else(|| session.name.clone()),
                    )
                    .strong(),
                );
                if session.hops.len() > 1 {
                    ui.label(
                        RichText::new(format!("via {}", session.route()))
                            .small()
                            .color(theme.text_dim),
                    );
                }
                ui.label(RichText::new(status).small().color(theme.text_dim));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close = if session.is_closed() {
                        "Close tab"
                    } else {
                        "Disconnect"
                    };
                    if ui.button(close).clicked() {
                        action = SessionAction::Close;
                    }
                    if session.is_closed()
                        && ui.add(super::primary_button(&theme, "Reconnect")).clicked()
                    {
                        action = SessionAction::Reconnect;
                    }
                });
            });
        });

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
                        prompt_card(ui, &theme, &mut session.prompt, &mut session.secret_input);
                    });
            }
            super::terminal::show(ui, session, &terminal_theme, font_size, !prompting);
        });

    action
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
    let Some(prompt) = pending.take() else {
        return;
    };
    let mut keep = Some(prompt);
    super::card(theme).show(ui, |ui| {
        ui.set_max_width(560.0);
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
                ui.label(RichText::new(&fingerprint).monospace());
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
                match secret_form(ui, theme, &title, error, can_remember, secret_input) {
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
                match secret_form(ui, theme, &title, error, can_remember, secret_input) {
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
    if !field.has_focus() && !field.lost_focus() {
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
