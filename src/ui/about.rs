//! About screen: the app version and every library used, with the version
//! actually built in and a link to its website.

use eframe::egui::{self, RichText};

use crate::{app::App, libs, theme::Theme};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let theme = app.theme;
    super::page_header(
        ui,
        &theme,
        "About",
        "What termker is built from, so you can verify it yourself.",
    );

    super::card(&theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(">_")
                    .monospace()
                    .size(28.0)
                    .color(theme.accent_text),
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
    ui.label(
        RichText::new(format!("Libraries ({})", libs::LIBRARIES.len()))
            .size(16.0)
            .strong(),
    );
    ui.add(
        egui::Label::new(
            RichText::new(
                "Versions are read from the build's lock file, so this list always \
                 matches what is compiled into the app. Each library links to its \
                 website and to that exact version on crates.io.",
            )
            .color(theme.text_dim),
        )
        .wrap(),
    );
    ui.add_space(6.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.set_width(ui.available_width() - 12.0);
        for category in libs::Category::ALL {
            ui.label(
                RichText::new(category.title().to_uppercase())
                    .small()
                    .color(theme.text_dim),
            );
            super::card(&theme).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let libraries = libs::LIBRARIES
                    .iter()
                    .filter(|lib| lib.category == category);
                for (index, lib) in libraries.enumerate() {
                    if index > 0 {
                        ui.add_space(4.0);
                        ui.separator();
                        ui.add_space(4.0);
                    }
                    library(ui, &theme, lib);
                }
            });
            ui.add_space(10.0);
        }
    });
}

/// One library: name, version and license on the first line, then what it
/// is used for and its links. Everything wraps, so nothing is cut off in a
/// narrow window.
fn library(ui: &mut egui::Ui, theme: &Theme, lib: &libs::LibraryInfo) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(lib.name).monospace().size(15.0).strong());
        ui.label(
            RichText::new(lib.version())
                .monospace()
                .color(theme.accent_text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(lib.license).small().color(theme.text_dim))
                .on_hover_text("License");
        });
    });
    ui.add(egui::Label::new(lib.purpose).wrap());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new("Website").small().color(theme.text_dim));
        ui.hyperlink_to(lib.website_label(), lib.website)
            .on_hover_text(lib.website);
        ui.add_space(10.0);
        ui.label(RichText::new("Registry").small().color(theme.text_dim));
        let registry = lib.registry_url();
        ui.hyperlink_to(registry.trim_start_matches("https://"), registry.as_str())
            .on_hover_text(registry.as_str());
    });
}
