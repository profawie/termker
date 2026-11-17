//! Halaman dan komponen antara muka.

pub mod about;
pub mod hosts;
pub mod placeholder;
pub mod settings;

use eframe::egui::{self, RichText};

use crate::theme::Theme;

/// Tajuk halaman dengan keterangan ringkas di bawahnya.
pub fn page_header(ui: &mut egui::Ui, theme: &Theme, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(24.0).strong());
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).color(theme.text_dim));
    }
    ui.add_space(12.0);
}

/// Bingkai kad: latar `surface` dengan sempadan nipis.
pub fn card(theme: &Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.surface)
        .stroke(egui::Stroke::new(1.0, theme.border))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(14))
}

/// Butang tindakan utama, berwarna `accent`.
pub fn primary_button(theme: &Theme, text: &str) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.to_owned())
            .color(theme.on_accent)
            .strong(),
    )
    .fill(theme.accent)
}
