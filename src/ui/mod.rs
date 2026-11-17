//! Pages and shared UI components.

pub mod about;
pub mod forwards;
pub mod hosts;
pub mod placeholder;
pub mod session;
pub mod settings;
pub mod terminal;

use eframe::egui::{self, RichText};

use crate::theme::Theme;

/// Page title with a short description under it.
pub fn page_header(ui: &mut egui::Ui, theme: &Theme, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(24.0).strong());
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).color(theme.text_dim));
    }
    ui.add_space(12.0);
}

/// Card frame: `surface` background with a thin border.
pub fn card(theme: &Theme) -> egui::Frame {
    egui::Frame::new()
        .fill(theme.surface)
        .stroke(egui::Stroke::new(1.0, theme.border))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(14))
}

/// Primary action button, filled with `accent`.
pub fn primary_button(theme: &Theme, text: &str) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(text.to_owned())
            .color(theme.on_accent)
            .strong(),
    )
    .fill(theme.accent)
}
