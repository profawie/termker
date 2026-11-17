//! Keychain page: SSH keys with their certificates, and identities.

use std::{
    sync::mpsc::{self, Receiver},
    time::SystemTime,
};

use eframe::egui::{self, RichText};
use uuid::Uuid;

use crate::{
    app::App,
    fido::{self, SkKind},
    keychain::{self, CertificateInfo, CertificateStatus, KeyError, KeyKind},
    model::{Identity, KeyEntry},
    theme::Theme,
};

const CARD_INNER_WIDTH: f32 = 260.0;
/// Card width including inner margin (14 per side) and border.
const CARD_OUTER_WIDTH: f32 = CARD_INNER_WIDTH + 30.0;

#[derive(Default)]
pub struct KeychainPage {
    panel: Option<Panel>,
}

/// What the right-hand panel is showing.
enum Panel {
    Generate(GenerateForm),
    Import(ImportForm),
    SecurityKey(SecurityKeyForm),
    Key(KeyForm),
    Identity(IdentityForm),
}

struct GenerateForm {
    name: String,
    kind: KeyKind,
    passphrase: String,
    confirm: String,
    error: Option<String>,
    /// Set while the key is being generated in the background.
    working: Option<Receiver<Result<KeyEntry, KeyError>>>,
}

struct SecurityKeyForm {
    name: String,
    kind: SkKind,
    pin: String,
    error: Option<String>,
    /// Set while waiting for the security key to be touched.
    working: Option<Receiver<Result<KeyEntry, String>>>,
}

#[derive(Default)]
struct ImportForm {
    name: String,
    path: String,
    passphrase: String,
    error: Option<String>,
}

struct KeyForm {
    id: Uuid,
    name: String,
    /// Certificate text being pasted, when attaching one.
    certificate_input: Option<String>,
    error: Option<String>,
    /// Short confirmation such as "Public key copied".
    info: Option<String>,
    confirm_delete: bool,
}

impl KeyForm {
    fn new(key: &KeyEntry) -> Self {
        Self {
            id: key.id,
            name: key.name.clone(),
            certificate_input: None,
            error: None,
            info: None,
            confirm_delete: false,
        }
    }
}

struct IdentityForm {
    draft: Identity,
    is_new: bool,
    error: Option<String>,
    confirm_delete: bool,
}

impl KeychainPage {
    pub fn show(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let theme = app.theme;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                super::page_header(
                    ui,
                    &theme,
                    "Keychain",
                    "Keys, certificates and identities you sign in with.",
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui
                    .add(super::primary_button(&theme, "+  Generate key"))
                    .clicked()
                {
                    self.panel = Some(Panel::Generate(GenerateForm {
                        name: String::new(),
                        kind: KeyKind::Ed25519,
                        passphrase: String::new(),
                        confirm: String::new(),
                        error: None,
                        working: None,
                    }));
                }
                if ui.button("Import key").clicked() {
                    self.panel = Some(Panel::Import(ImportForm::default()));
                }
                if ui.button("Add security key").clicked() {
                    self.panel = Some(Panel::SecurityKey(SecurityKeyForm {
                        name: String::new(),
                        kind: SkKind::Ed25519,
                        pin: String::new(),
                        error: None,
                        working: None,
                    }));
                }
                if ui.button("New identity").clicked() {
                    self.panel = Some(Panel::Identity(IdentityForm {
                        draft: Identity::new(),
                        is_new: true,
                        error: None,
                        confirm_delete: false,
                    }));
                }
            });
        });

        let mut open_key = None;
        let mut open_identity = None;
        let mut forget = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            section_title(ui, &theme, "KEYS", app.data.keys.len());
            if app.data.keys.is_empty() {
                empty_note(
                    ui,
                    &theme,
                    "No keys yet. Generate a key, then add its public key to your servers.",
                );
            }
            let selected_key = match &self.panel {
                Some(Panel::Key(form)) => Some(form.id),
                _ => None,
            };
            for row in app.data.keys.chunks(columns(ui)) {
                ui.horizontal(|ui| {
                    for key in row {
                        if key_card(ui, &theme, key, selected_key == Some(key.id)) {
                            open_key = Some(key.id);
                        }
                    }
                });
            }
            ui.add_space(16.0);

            section_title(ui, &theme, "IDENTITIES", app.data.identities.len());
            if app.data.identities.is_empty() {
                empty_note(
                    ui,
                    &theme,
                    "An identity is a username with a key or a password. \
                     Use one identity on many hosts and change it in one place.",
                );
            }
            let selected_identity = match &self.panel {
                Some(Panel::Identity(form)) if !form.is_new => Some(form.draft.id),
                _ => None,
            };
            for row in app.data.identities.chunks(columns(ui)) {
                ui.horizontal(|ui| {
                    for identity in row {
                        let selected = selected_identity == Some(identity.id);
                        if identity_card(ui, app, identity, selected) {
                            open_identity = Some(identity.id);
                        }
                    }
                });
            }
            ui.add_space(16.0);

            let saved = app.secrets.saved();
            section_title(ui, &theme, "SAVED PASSWORDS", saved.len());
            if !app.secrets.available() {
                empty_note(
                    ui,
                    &theme,
                    "This system has no credential store, so passwords and passphrases \
                     cannot be saved. You are asked for them each time.",
                );
            } else if saved.is_empty() {
                empty_note(
                    ui,
                    &theme,
                    "Nothing saved. Tick \"Remember\" when you are asked for a password or \
                     passphrase to keep it in this computer's credential store.",
                );
            } else {
                super::card(&theme).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for secret in &saved {
                        ui.horizontal(|ui| {
                            ui.label(&secret.label);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("Forget").clicked() {
                                        forget = Some(secret.key.clone());
                                    }
                                },
                            );
                        });
                    }
                });
            }
        });

        if let Some(key) = forget
            && let Err(err) = app.secrets.forget(&key)
        {
            app.notify(format!("Could not remove the saved secret: {err}"));
        }
        if let Some(key) = open_key.and_then(|id| app.data.key(id)) {
            self.panel = Some(Panel::Key(KeyForm::new(key)));
        }
        if let Some(identity) = open_identity.and_then(|id| app.data.identity(id)) {
            self.panel = Some(Panel::Identity(IdentityForm {
                draft: identity.clone(),
                is_new: false,
                error: None,
                confirm_delete: false,
            }));
        }
    }

    /// Right-hand panel for the form that is open, if any.
    pub fn editor_panel(&mut self, app: &mut App, ui: &mut egui::Ui) {
        let Some(panel) = &mut self.panel else {
            return;
        };
        let theme = app.theme;
        let mut next = Next::Stay;
        egui::Panel::right("keychain_editor")
            .exact_size(360.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme.panel)
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    next = match panel {
                        Panel::Generate(form) => generate_form(ui, app, form),
                        Panel::Import(form) => import_form(ui, app, form),
                        Panel::SecurityKey(form) => security_key_form(ui, app, form),
                        Panel::Key(form) => key_form(ui, app, form),
                        Panel::Identity(form) => identity_form(ui, app, form),
                    };
                });
            });
        match next {
            Next::Stay => {}
            Next::Close => self.panel = None,
            Next::OpenKey(id) => {
                self.panel = app.data.key(id).map(|key| Panel::Key(KeyForm::new(key)));
            }
        }
    }
}

/// What to do with the panel after this frame.
enum Next {
    Stay,
    Close,
    /// Show the details of a key that was just added.
    OpenKey(Uuid),
}

fn columns(ui: &egui::Ui) -> usize {
    let gap = ui.spacing().item_spacing.x;
    ((ui.available_width() + gap) / (CARD_OUTER_WIDTH + gap))
        .floor()
        .max(1.0) as usize
}

fn section_title(ui: &mut egui::Ui, theme: &Theme, title: &str, count: usize) {
    ui.label(
        RichText::new(format!("{title}  ({count})"))
            .small()
            .color(theme.text_dim),
    );
}

fn empty_note(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    super::card(theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(text).color(theme.text_dim));
    });
}

fn dim(ui: &mut egui::Ui, theme: &Theme, text: impl Into<String>) {
    ui.add(egui::Label::new(RichText::new(text).small().color(theme.text_dim)).wrap());
}

fn secret_field(ui: &mut egui::Ui, theme: &Theme, label: &str, value: &mut String) {
    ui.label(RichText::new(label).small().color(theme.text_dim));
    ui.add(
        egui::TextEdit::singleline(value)
            .password(true)
            .desired_width(f32::INFINITY),
    );
}

/// Short text for a certificate's state, and whether it is a problem.
fn certificate_state(info: &CertificateInfo) -> (String, bool) {
    match info.status(SystemTime::now()) {
        CertificateStatus::Valid => (format!("Certificate valid until {}", info.expires()), false),
        CertificateStatus::NotYetValid => ("Certificate not valid yet".to_string(), true),
        CertificateStatus::Expired => (format!("Certificate expired {}", info.expires()), true),
    }
}

/// One key card. Returns true when it was clicked open.
fn key_card(ui: &mut egui::Ui, theme: &Theme, key: &KeyEntry, selected: bool) -> bool {
    let mut frame = super::card(theme);
    if selected {
        frame = frame.stroke(egui::Stroke::new(1.5, theme.accent));
    }
    let mut open = false;
    frame.show(ui, |ui| {
        ui.set_width(CARD_INNER_WIDTH);
        ui.vertical(|ui| {
            ui.add(egui::Label::new(RichText::new(&key.name).size(15.0).strong()).truncate());
            ui.add(
                egui::Label::new(
                    RichText::new(format!("{}  ·  {}", key.algorithm, key.protection()))
                        .small()
                        .color(theme.text_dim),
                )
                .truncate(),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(&key.fingerprint)
                        .monospace()
                        .small()
                        .color(theme.text_dim),
                )
                .truncate(),
            );
            let certificate = key.certificate.as_deref().and_then(CertificateInfo::parse);
            let (text, problem) = match &certificate {
                Some(info) => certificate_state(info),
                None => ("No certificate".to_string(), false),
            };
            let color = if problem {
                theme.danger
            } else {
                theme.text_dim
            };
            ui.add(egui::Label::new(RichText::new(text).small().color(color)).truncate());
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui.button("Copy public key").clicked() {
                    ui.ctx().copy_text(key.public_key.clone());
                }
                if ui.button("Details").clicked() {
                    open = true;
                }
            });
        });
    });
    open
}

/// One identity card. Returns true when Edit was clicked.
fn identity_card(ui: &mut egui::Ui, app: &App, identity: &Identity, selected: bool) -> bool {
    let theme = app.theme;
    let mut frame = super::card(&theme);
    if selected {
        frame = frame.stroke(egui::Stroke::new(1.5, theme.accent));
    }
    let mut open = false;
    frame.show(ui, |ui| {
        ui.set_width(CARD_INNER_WIDTH);
        ui.vertical(|ui| {
            ui.add(egui::Label::new(RichText::new(&identity.name).size(15.0).strong()).truncate());
            ui.add(
                egui::Label::new(
                    RichText::new(&identity.username)
                        .monospace()
                        .small()
                        .color(theme.text_dim),
                )
                .truncate(),
            );
            let sign_in = match identity.key.and_then(|id| app.data.key(id)) {
                Some(key) => format!("Key: {}", key.name),
                None => "Password".to_string(),
            };
            ui.add(
                egui::Label::new(RichText::new(sign_in).small().color(theme.text_dim)).truncate(),
            );
            ui.horizontal(|ui| {
                let hosts = app.data.identity_usage(identity.id);
                let used = match hosts {
                    0 => "Not used yet".to_string(),
                    1 => "Used by 1 host".to_string(),
                    n => format!("Used by {n} hosts"),
                };
                ui.label(RichText::new(used).small().color(theme.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Edit").clicked() {
                        open = true;
                    }
                });
            });
        });
    });
    open
}

fn generate_form(ui: &mut egui::Ui, app: &mut App, form: &mut GenerateForm) -> Next {
    let theme = app.theme;
    ui.label(RichText::new("Generate key").size(18.0).strong());
    ui.add_space(8.0);

    // Pick up the result of a generation that was running.
    if let Some(result) = form.working.as_ref().and_then(|rx| rx.try_recv().ok()) {
        form.working = None;
        match result {
            Ok(entry) => {
                let id = entry.id;
                app.data.keys.push(entry);
                app.save();
                return Next::OpenKey(id);
            }
            Err(err) => form.error = Some(err.to_string()),
        }
    }
    let working = form.working.is_some();

    ui.add_enabled_ui(!working, |ui| {
        super::field(ui, "Name", &mut form.name, "e.g. Work laptop");
        ui.label(RichText::new("Type").small().color(theme.text_dim));
        egui::ComboBox::from_id_salt("key_kind")
            .selected_text(form.kind.title())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for kind in KeyKind::ALL {
                    ui.selectable_value(&mut form.kind, kind, kind.title());
                }
            });
        dim(ui, &theme, form.kind.hint());
        ui.add_space(4.0);
        secret_field(ui, &theme, "Passphrase (optional)", &mut form.passphrase);
        if !form.passphrase.is_empty() {
            secret_field(ui, &theme, "Repeat passphrase", &mut form.confirm);
        }
        dim(
            ui,
            &theme,
            "A passphrase encrypts the private key on this computer. \
             You are asked for it when the key is used, and can choose to save it then.",
        );
    });

    if let Some(error) = &form.error {
        ui.add_space(4.0);
        ui.label(RichText::new(error).color(theme.danger));
    }
    ui.add_space(12.0);
    let mut next = Next::Stay;
    ui.horizontal(|ui| {
        if working {
            ui.spinner();
            ui.label("Generating the key");
        } else if ui.add(super::primary_button(&theme, "Generate")).clicked() {
            if form.name.trim().is_empty() {
                form.error = Some("Name is required.".to_string());
            } else if form.passphrase != form.confirm {
                form.error = Some("The two passphrases are different.".to_string());
            } else {
                form.error = None;
                let (tx, rx) = mpsc::channel();
                let store = app.key_store.clone();
                let (kind, name) = (form.kind, form.name.clone());
                let passphrase = std::mem::take(&mut form.passphrase);
                form.confirm.clear();
                let ctx = ui.ctx().clone();
                // RSA generation can take several seconds, so keep it off
                // the UI thread.
                std::thread::spawn(move || {
                    let _ = tx.send(keychain::generate(&store, kind, &name, &passphrase));
                    ctx.request_repaint();
                });
                form.working = Some(rx);
            }
        }
        if !working && ui.button("Cancel").clicked() {
            next = Next::Close;
        }
    });
    next
}

fn security_key_form(ui: &mut egui::Ui, app: &mut App, form: &mut SecurityKeyForm) -> Next {
    let theme = app.theme;
    ui.label(RichText::new("Add security key").size(18.0).strong());
    ui.add_space(8.0);

    if let Some(result) = form.working.as_ref().and_then(|rx| rx.try_recv().ok()) {
        form.working = None;
        match result {
            Ok(entry) => {
                let id = entry.id;
                app.data.keys.push(entry);
                app.save();
                return Next::OpenKey(id);
            }
            Err(error) => form.error = Some(error),
        }
    }
    let working = form.working.is_some();

    dim(
        ui,
        &theme,
        "Creates an SSH key on a FIDO2 security key such as a YubiKey. The private key \
         cannot be copied off the security key, and every sign-in needs a touch.",
    );
    ui.add_space(4.0);
    ui.add_enabled_ui(!working, |ui| {
        super::field(ui, "Name", &mut form.name, "e.g. YubiKey 5C");
        ui.label(RichText::new("Type").small().color(theme.text_dim));
        egui::ComboBox::from_id_salt("security_key_kind")
            .selected_text(form.kind.title())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for kind in SkKind::ALL {
                    ui.selectable_value(&mut form.kind, kind, kind.title());
                }
            });
        dim(ui, &theme, form.kind.hint());
        ui.add_space(4.0);
        secret_field(
            ui,
            &theme,
            "Security key PIN (if it has one)",
            &mut form.pin,
        );
        dim(
            ui,
            &theme,
            "The PIN is sent to the security key and is never saved.",
        );
    });

    if let Some(error) = &form.error {
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(error).color(theme.danger)).wrap());
    }
    ui.add_space(12.0);
    let mut next = Next::Stay;
    ui.horizontal(|ui| {
        if working {
            ui.spinner();
            ui.label(RichText::new("Touch your security key now").strong());
        } else if ui
            .add(super::primary_button(&theme, "Create key"))
            .clicked()
        {
            if form.name.trim().is_empty() {
                form.error = Some("Name is required.".to_string());
            } else {
                form.error = None;
                let (tx, rx) = mpsc::channel();
                let store = app.key_store.clone();
                let (kind, name) = (form.kind, form.name.clone());
                let pin = std::mem::take(&mut form.pin);
                let ctx = ui.ctx().clone();
                // Waiting for the touch blocks, so it runs on its own thread.
                std::thread::spawn(move || {
                    let pin = (!pin.is_empty()).then_some(pin);
                    let result = fido::platform()
                        .make_credential(kind, fido::APPLICATION, name.trim(), pin.as_deref())
                        .map_err(|err| match err {
                            fido::FidoError::PinRequired => {
                                "This security key needs its PIN. Enter it and try again."
                                    .to_string()
                            }
                            other => other.to_string(),
                        })
                        .and_then(|credential| {
                            keychain::add_security_key(&store, &credential, &name)
                                .map_err(|err| err.to_string())
                        });
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
                form.working = Some(rx);
            }
        }
        if !working && ui.button("Cancel").clicked() {
            next = Next::Close;
        }
    });
    next
}

fn import_form(ui: &mut egui::Ui, app: &mut App, form: &mut ImportForm) -> Next {
    let theme = app.theme;
    ui.label(RichText::new("Import key").size(18.0).strong());
    ui.add_space(8.0);

    ui.label(
        RichText::new("Private key file")
            .small()
            .color(theme.text_dim),
    );
    ui.horizontal(|ui| {
        if ui.button("Browse").clicked()
            && let Some(file) = rfd::FileDialog::new()
                .set_title("Choose a private key file")
                .pick_file()
        {
            if form.name.trim().is_empty()
                && let Some(stem) = file.file_stem()
            {
                form.name = stem.to_string_lossy().into_owned();
            }
            form.path = file.display().to_string();
        }
        ui.add(
            egui::TextEdit::singleline(&mut form.path)
                .hint_text("~/.ssh/id_ed25519")
                .desired_width(f32::INFINITY),
        );
    });
    super::field(ui, "Name", &mut form.name, "e.g. Work laptop");
    secret_field(
        ui,
        &theme,
        "Passphrase (PuTTY and PEM files only)",
        &mut form.passphrase,
    );
    dim(
        ui,
        &theme,
        "OpenSSH keys are copied as they are and keep their passphrase. \
         PuTTY (.ppk) and PEM keys are converted to OpenSSH format. \
         The original file is not changed.",
    );

    if let Some(error) = &form.error {
        ui.add_space(4.0);
        ui.label(RichText::new(error).color(theme.danger));
    }
    ui.add_space(12.0);
    let mut next = Next::Stay;
    ui.horizontal(|ui| {
        if ui.add(super::primary_button(&theme, "Import")).clicked() {
            match import(app, form) {
                Ok(id) => next = Next::OpenKey(id),
                Err(error) => form.error = Some(error),
            }
        }
        if ui.button("Cancel").clicked() {
            next = Next::Close;
        }
    });
    next
}

fn import(app: &mut App, form: &mut ImportForm) -> Result<Uuid, String> {
    if form.path.trim().is_empty() {
        return Err("Choose the private key file to import.".to_string());
    }
    if form.name.trim().is_empty() {
        return Err("Name is required.".to_string());
    }
    let path = crate::ssh::expand_home(form.path.trim());
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("Could not read {}: {err}", path.display()))?;
    let entry = keychain::import(&app.key_store, &text, &form.name, &form.passphrase)
        .map_err(|err| err.to_string())?;
    form.passphrase.clear();
    let id = entry.id;
    app.data.keys.push(entry);
    app.save();
    Ok(id)
}

fn key_form(ui: &mut egui::Ui, app: &mut App, form: &mut KeyForm) -> Next {
    let theme = app.theme;
    let Some(key) = app.data.key(form.id).cloned() else {
        return Next::Close;
    };
    ui.label(RichText::new("Key").size(18.0).strong());
    ui.add_space(8.0);

    super::field(ui, "Name", &mut form.name, "Name");
    dim(
        ui,
        &theme,
        format!("{}  ·  {}", key.algorithm, key.protection()),
    );
    ui.add_space(6.0);

    ui.label(RichText::new("Fingerprint").small().color(theme.text_dim));
    ui.add(egui::Label::new(RichText::new(&key.fingerprint).monospace().small()).wrap());
    ui.add_space(6.0);

    ui.label(RichText::new("Public key").small().color(theme.text_dim));
    let mut public = key.public_key.clone();
    ui.add(
        egui::TextEdit::multiline(&mut public)
            .font(egui::TextStyle::Monospace)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .interactive(false),
    );
    dim(
        ui,
        &theme,
        "Add this line to ~/.ssh/authorized_keys on each server.",
    );
    ui.horizontal_wrapped(|ui| {
        if ui.button("Copy public key").clicked() {
            ui.ctx().copy_text(key.public_key.clone());
            form.info = Some("Public key copied.".to_string());
        }
        if ui.button("Save public key").clicked()
            && let Some(dest) = rfd::FileDialog::new()
                .set_title("Save public key")
                .set_file_name(format!("{}.pub", file_name(&key.name)))
                .save_file()
        {
            match std::fs::write(&dest, format!("{}\n", key.public_key)) {
                Ok(()) => form.info = Some(format!("Saved to {}.", dest.display())),
                Err(err) => form.error = Some(format!("Could not save: {err}")),
            }
        }
        if ui.button("Export private key").clicked()
            && let Some(dest) = rfd::FileDialog::new()
                .set_title("Export private key")
                .set_file_name(file_name(&key.name))
                .save_file()
        {
            match app.key_store.export(key.id, &dest) {
                Ok(()) => form.info = Some(format!("Exported to {}.", dest.display())),
                Err(err) => form.error = Some(format!("Could not export: {err}")),
            }
        }
    });
    if key.is_security_key() {
        dim(
            ui,
            &theme,
            "The private key stays on the security key. The file kept here, and an exported \
             copy, is only a handle that works together with that security key.",
        );
    } else if !key.encrypted {
        dim(
            ui,
            &theme,
            "This private key has no passphrase. Anyone who gets the exported file can use it.",
        );
    }
    ui.add_space(10.0);

    ui.label(RichText::new("Certificate").small().color(theme.text_dim));
    certificate_section(ui, app, form, &key);

    if let Some(info) = &form.info {
        ui.add_space(4.0);
        ui.label(RichText::new(info).color(theme.accent));
    }
    if let Some(error) = &form.error {
        ui.add_space(4.0);
        ui.label(RichText::new(error).color(theme.danger));
    }

    ui.add_space(12.0);
    let mut next = Next::Stay;
    ui.horizontal(|ui| {
        if ui.add(super::primary_button(&theme, "Save")).clicked() {
            let name = form.name.trim();
            if name.is_empty() {
                form.error = Some("Name is required.".to_string());
            } else {
                if let Some(entry) = app.data.keys.iter_mut().find(|entry| entry.id == key.id) {
                    entry.name = name.to_string();
                }
                app.save();
                next = Next::Close;
            }
        }
        if ui.button("Close").clicked() {
            next = Next::Close;
        }
    });

    ui.add_space(16.0);
    if form.confirm_delete {
        let usage = app.data.key_usage(key.id);
        let warning = match usage {
            0 => "Delete this key? The private key file is removed and cannot be recovered."
                .to_string(),
            n => format!(
                "Delete this key? It is used in {n} place(s), which will go back to \
                 password sign-in. The private key file is removed and cannot be recovered."
            ),
        };
        ui.add(egui::Label::new(RichText::new(warning).color(theme.danger)).wrap());
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(
                    RichText::new("Yes, delete").color(theme.danger),
                ))
                .clicked()
            {
                match app.key_store.remove(key.id) {
                    Ok(()) => {
                        // A saved passphrase for a deleted key is useless.
                        let path = app.key_store.path(key.id).display().to_string();
                        let _ = app.secrets.forget(&crate::secrets::passphrase_key(&path));
                        app.data.remove_key(key.id);
                        app.save();
                        next = Next::Close;
                    }
                    Err(err) => form.error = Some(format!("Could not delete the key file: {err}")),
                }
            }
            if ui.button("Keep").clicked() {
                form.confirm_delete = false;
            }
        });
    } else if ui
        .button(RichText::new("Delete key").color(theme.danger))
        .clicked()
    {
        form.confirm_delete = true;
    }
    next
}

/// Certificate details, or the form to attach one.
fn certificate_section(ui: &mut egui::Ui, app: &mut App, form: &mut KeyForm, key: &KeyEntry) {
    let theme = app.theme;
    let set_certificate = |app: &mut App, certificate: Option<String>| {
        if let Some(entry) = app.data.keys.iter_mut().find(|entry| entry.id == key.id) {
            entry.certificate = certificate;
        }
        app.save();
    };

    if let Some(input) = &mut form.certificate_input {
        // A certificate is one very long line; keep the box a fixed height.
        egui::ScrollArea::vertical()
            .id_salt("certificate_input")
            .max_height(110.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(input)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("Paste the contents of the -cert.pub file")
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
            });
        let mut done = false;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Load from file").clicked()
                && let Some(file) = rfd::FileDialog::new()
                    .set_title("Choose a certificate file (-cert.pub)")
                    .pick_file()
            {
                match std::fs::read_to_string(&file) {
                    Ok(text) => *input = text,
                    Err(err) => form.error = Some(format!("Could not read the file: {err}")),
                }
            }
            if ui.add(super::primary_button(&theme, "Attach")).clicked() {
                match keychain::check_certificate(input, key) {
                    Ok(certificate) => {
                        set_certificate(app, Some(certificate));
                        form.error = None;
                        form.info = Some("Certificate attached.".to_string());
                        done = true;
                    }
                    Err(err) => form.error = Some(err.to_string()),
                }
            }
            if ui.button("Cancel").clicked() {
                done = true;
            }
        });
        if done {
            form.certificate_input = None;
        }
        return;
    }

    match key.certificate.as_deref().and_then(CertificateInfo::parse) {
        Some(info) => {
            let (state, problem) = certificate_state(&info);
            let color = if problem { theme.danger } else { theme.accent };
            ui.label(RichText::new(state).color(color));
            egui::Grid::new("certificate_details")
                .num_columns(2)
                .spacing([10.0, 2.0])
                .show(ui, |ui| {
                    let mut row = |label: &str, value: String| {
                        ui.label(RichText::new(label).small().color(theme.text_dim));
                        ui.add(egui::Label::new(RichText::new(value).small()).wrap());
                        ui.end_row();
                    };
                    row("Key ID", info.key_id.clone());
                    row("Serial", info.serial.to_string());
                    row("Sign in as", info.principals_text());
                    row("Signed by", info.ca_fingerprint.clone());
                });
            dim(
                ui,
                &theme,
                "Presented when signing in with this key. \
                 Each server decides whether it trusts the signing authority.",
            );
            ui.horizontal(|ui| {
                if ui.button("Replace").clicked() {
                    form.certificate_input = Some(String::new());
                }
                if ui.button("Remove certificate").clicked() {
                    set_certificate(app, None);
                    form.info = Some("Certificate removed.".to_string());
                }
            });
        }
        None => {
            dim(
                ui,
                &theme,
                "If your organisation signs keys, attach the user certificate \
                 (-cert.pub) it issued for this key.",
            );
            if ui.button("Attach certificate").clicked() {
                form.certificate_input = Some(String::new());
            }
        }
    }
}

fn identity_form(ui: &mut egui::Ui, app: &mut App, form: &mut IdentityForm) -> Next {
    let theme = app.theme;
    let title = if form.is_new {
        "New identity"
    } else {
        "Edit identity"
    };
    ui.label(RichText::new(title).size(18.0).strong());
    ui.add_space(8.0);

    super::field(ui, "Name", &mut form.draft.name, "e.g. Deploy user");
    super::field(ui, "Username", &mut form.draft.username, "root");

    ui.label(RichText::new("Sign in with").small().color(theme.text_dim));
    let current = form
        .draft
        .key
        .and_then(|id| app.data.key(id))
        .map(|key| format!("Key: {}", key.name))
        .unwrap_or_else(|| "Password".to_string());
    egui::ComboBox::from_id_salt("identity_key")
        .selected_text(current)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut form.draft.key, None, "Password");
            for key in &app.data.keys {
                ui.selectable_value(
                    &mut form.draft.key,
                    Some(key.id),
                    format!("Key: {}", key.name),
                );
            }
        });
    if form.draft.key.is_none() {
        dim(
            ui,
            &theme,
            "You are asked for the password when connecting, and can choose to save it then.",
        );
    }

    if let Some(error) = &form.error {
        ui.add_space(4.0);
        ui.label(RichText::new(error).color(theme.danger));
    }
    ui.add_space(12.0);
    let mut next = Next::Stay;
    ui.horizontal(|ui| {
        if ui.add(super::primary_button(&theme, "Save")).clicked() {
            match app.data.upsert_identity(form.draft.clone()) {
                Ok(()) => {
                    app.save();
                    next = Next::Close;
                }
                Err(err) => form.error = Some(err.to_string()),
            }
        }
        if ui.button("Cancel").clicked() {
            next = Next::Close;
        }
    });

    if !form.is_new {
        ui.add_space(16.0);
        if form.confirm_delete {
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Delete this identity? Hosts that use it go back to their own \
                         username and sign-in method.",
                    )
                    .color(theme.danger),
                )
                .wrap(),
            );
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new(
                        RichText::new("Yes, delete").color(theme.danger),
                    ))
                    .clicked()
                {
                    app.data.remove_identity(form.draft.id);
                    app.save();
                    next = Next::Close;
                }
                if ui.button("Keep").clicked() {
                    form.confirm_delete = false;
                }
            });
        } else if ui
            .button(RichText::new("Delete identity").color(theme.danger))
            .clicked()
        {
            form.confirm_delete = true;
        }
    }
    next
}

/// A key name made safe to suggest as a file name.
fn file_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "id_key".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name(" Work laptop "), "Work_laptop");
        assert_eq!(file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(file_name("a/b\\c:d"), "a_b_c_d");
        assert_eq!(file_name("..."), "id_key");
        assert_eq!(file_name(""), "id_key");
    }
}
