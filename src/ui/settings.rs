//! Settings: light or dark look, terminal colors.

use eframe::egui::{self, RichText};

use crate::{
    app::App,
    model::Appearance,
    theme::{self as colors, TERMINAL_THEMES, Theme},
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
    let custom = app.data.settings.terminal_text.is_some()
        || app.data.settings.terminal_background.is_some();
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for candidate in TERMINAL_THEMES {
            if swatch(ui, candidate, !custom && candidate.name == active).clicked() {
                chosen = Some(*candidate);
            }
        }
        if custom {
            // Preview of the user's own colors; the colors are changed below.
            let own = Theme {
                name: "Custom",
                ..app.terminal_theme
            };
            swatch(ui, &own, true);
        }
    });
    if let Some(chosen) = chosen {
        app.set_terminal_theme(chosen);
    }
    terminal_colors(app, ui, &theme);

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

/// The user's own text and background colors for the terminal, on top of
/// the chosen theme.
fn terminal_colors(app: &mut App, ui: &mut egui::Ui, theme: &Theme) {
    let base = Theme::terminal(&app.data.settings.terminal_theme);
    let custom = app.data.settings.terminal_text.is_some()
        || app.data.settings.terminal_background.is_some();
    let current = app.terminal_theme;
    let mut text = current.text.to_array();
    let mut background = current.bg.to_array();
    let mut changed = false;
    let mut reset = false;

    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.label("Text");
        let mut rgb = [text[0], text[1], text[2]];
        if ui
            .color_edit_button_srgb(&mut rgb)
            .on_hover_text("Click to pick the terminal text color")
            .changed()
        {
            text = [rgb[0], rgb[1], rgb[2], 255];
            changed = true;
        }
        ui.add_space(12.0);
        ui.label("Background");
        let mut rgb = [background[0], background[1], background[2]];
        if ui
            .color_edit_button_srgb(&mut rgb)
            .on_hover_text("Click to pick the terminal background color")
            .changed()
        {
            background = [rgb[0], rgb[1], rgb[2], 255];
            changed = true;
        }
        if custom {
            ui.add_space(12.0);
            if ui
                .button(format!("Use {} colors", base.name))
                .on_hover_text("Go back to the theme's own text and background")
                .clicked()
            {
                reset = true;
            }
        }
    });

    if reset {
        app.set_terminal_theme(base);
    } else if changed {
        let text = egui::Color32::from_rgb(text[0], text[1], text[2]);
        let background = egui::Color32::from_rgb(background[0], background[1], background[2]);
        // A color equal to the theme's own is not saved as a custom one.
        app.set_terminal_colors(
            (text != base.text).then_some(text),
            (background != base.bg).then_some(background),
        );
    }

    let hint = if custom {
        format!(
            "Your own colors on top of {}. Picking a theme above resets them.",
            base.name
        )
    } else {
        "Click a color to choose your own.".to_string()
    };
    ui.label(RichText::new(hint).small().color(theme.text_dim));
    if colors::contrast(app.terminal_theme.text, app.terminal_theme.bg) < 4.5 {
        ui.label(
            RichText::new("The text may be hard to read on this background.")
                .small()
                .color(theme.danger),
        );
    }
}

/// Terminal theme preview: a small box painted with that theme's own colors.
fn swatch(ui: &mut egui::Ui, theme: &Theme, selected: bool) -> egui::Response {
    let size = egui::vec2(168.0, 96.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter_at(rect);
    let border = if selected || response.hovered() {
        egui::Stroke::new(2.0, theme.accent_text)
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
