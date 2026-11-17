//! Halaman sementara untuk ciri yang belum siap.

use eframe::egui::{self, RichText};

use crate::app::App;

pub fn show(app: &mut App, ui: &mut egui::Ui, title: &str, description: &str) {
    let theme = app.theme;
    super::page_header(ui, &theme, title, description);
    super::card(&theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Not available yet").strong());
        ui.label(
            RichText::new("This feature is still being built for version 0.1.0.")
                .color(theme.text_dim),
        );
    });
}
