//! Settings: light or dark look, terminal colors.

use eframe::egui::{self, RichText};

use crate::{
    app::App,
    model::Appearance,
    theme::{TERMINAL_THEMES, Theme},
};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let theme = app.theme;
    super::page_header(ui, &theme, "Settings", "Appearance and preferences.");

    ui.label(RichText::new("Appearance").strong());
    ui.label(
        RichText::new("Colors of the sidebar, pages and dialogs.")
            .small()
            .color(theme.text_dim),
    );
    ui.add_space(4.0);
    let current = app.data.settings.appearance;
    let mut appearance = current;
    ui.horizontal(|ui| {
        for (value, label) in [
            (Appearance::System, "System"),
            (Appearance::Dark, "Dark"),
            (Appearance::Light, "Light"),
        ] {
            ui.selectable_value(&mut appearance, value, label);
        }
    });
    if appearance != current {
        app.set_appearance(ui.ctx(), appearance);
    }

    ui.add_space(20.0);
    ui.label(RichText::new("Terminal colors").strong());
    ui.label(
        RichText::new("Used inside terminal sessions only.")
            .small()
            .color(theme.text_dim),
    );
    ui.add_space(4.0);
    let active = app.terminal_theme.name;
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for candidate in TERMINAL_THEMES {
            if swatch(ui, candidate, candidate.name == active).clicked() {
                chosen = Some(*candidate);
            }
        }
    });
    if let Some(chosen) = chosen {
        app.set_terminal_theme(chosen);
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

/// Terminal theme preview: a small box painted with that theme's own colors.
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
