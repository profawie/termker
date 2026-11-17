//! Skrin About: versi aplikasi dan setiap library yang digunakan, dengan
//! versi sebenar dan pautan ke laman webnya.

use eframe::egui::{self, RichText};

use crate::{app::App, libs};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let theme = app.theme;
    super::page_header(
        ui,
        &theme,
        "About",
        "What rterminalfy is built from, so you can verify it yourself.",
    );

    super::card(&theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(">_")
                    .monospace()
                    .size(28.0)
                    .color(theme.accent),
            );
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(format!("{} {}", crate::APP_NAME, crate::APP_VERSION))
                        .size(18.0)
                        .strong(),
                );
                ui.label(
                    RichText::new("SSH client for individuals. Written entirely in Rust.")
                        .color(theme.text_dim),
                );
                ui.hyperlink(env!("CARGO_PKG_REPOSITORY"));
            });
        });
    });

    ui.add_space(16.0);
    ui.label(RichText::new("Libraries").size(16.0).strong());
    ui.label(
        RichText::new(
            "Versions are read from the build's lock file, so this list always \
             matches what is compiled into the app.",
        )
        .color(theme.text_dim),
    );
    ui.add_space(4.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        super::card(&theme).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Grid::new("libraries")
                .num_columns(5)
                .spacing(egui::vec2(24.0, 10.0))
                .striped(false)
                .show(ui, |ui| {
                    for heading in ["Library", "Version", "Used for", "License", "Website"] {
                        ui.label(RichText::new(heading).small().color(theme.text_dim));
                    }
                    ui.end_row();
                    for lib in libs::LIBRARIES {
                        ui.label(RichText::new(lib.name).monospace().strong());
                        ui.label(
                            RichText::new(libs::version_of(lib.name).unwrap_or("unknown"))
                                .monospace(),
                        );
                        ui.label(lib.purpose);
                        ui.label(RichText::new(lib.license).color(theme.text_dim));
                        ui.hyperlink(lib.website);
                        ui.end_row();
                    }
                });
        });
    });
}
