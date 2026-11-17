//! Tetapan: pemilihan tema.

use eframe::egui::{self, RichText};

use crate::{
    app::App,
    theme::{THEMES, Theme},
};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let theme = app.theme;
    super::page_header(ui, &theme, "Settings", "Appearance and preferences.");

    ui.label(RichText::new("Theme").strong());
    ui.add_space(4.0);
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for candidate in THEMES {
            if swatch(ui, candidate, candidate.name == theme.name).clicked() {
                chosen = Some(*candidate);
            }
        }
    });
    if let Some(chosen) = chosen {
        app.set_theme(ui.ctx(), chosen);
    }

    ui.add_space(20.0);
    ui.label(RichText::new("Data").strong());
    let location = app
        .data_path()
        .unwrap_or_else(|| "Not available on this system".to_string());
    ui.label(
        RichText::new(format!("Hosts and settings are stored at {location}")).color(theme.text_dim),
    );
    ui.label(
        RichText::new("Passwords and private keys are never written to this file.")
            .color(theme.text_dim),
    );
}

/// Pratonton tema: kotak kecil yang dilukis dengan warna tema itu sendiri.
fn swatch(ui: &mut egui::Ui, theme: &Theme, selected: bool) -> egui::Response {
    let size = egui::vec2(168.0, 96.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter_at(rect);
    let border = if selected || response.hovered() {
        egui::Stroke::new(2.0, theme.accent)
    } else {
        egui::Stroke::new(1.0, theme.border)
    };
    painter.rect(
        rect,
        egui::CornerRadius::same(8),
        theme.bg,
        border,
        egui::StrokeKind::Inside,
    );
    let mono = egui::FontId::monospace(12.0);
    let left = rect.left() + 12.0;
    painter.text(
        egui::pos2(left, rect.top() + 12.0),
        egui::Align2::LEFT_TOP,
        "$ ssh server",
        mono.clone(),
        theme.text,
    );
    painter.text(
        egui::pos2(left, rect.top() + 30.0),
        egui::Align2::LEFT_TOP,
        "connected",
        mono,
        theme.text_dim,
    );
    let label = if selected {
        format!("{}  (active)", theme.name)
    } else {
        theme.name.to_string()
    };
    painter.text(
        egui::pos2(left, rect.bottom() - 12.0),
        egui::Align2::LEFT_BOTTOM,
        label,
        egui::FontId::proportional(14.0),
        theme.text,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, theme.name)
    });
    response
}
