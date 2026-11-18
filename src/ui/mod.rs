//! Pages and shared UI components.

pub mod about;
pub mod forwards;
pub mod hosts;
pub mod keychain;
pub mod session;
pub mod settings;
pub mod sftp;
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

/// Labelled single-line text field that fills the available width.
pub fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    let dim = ui.visuals().weak_text_color();
    ui.label(RichText::new(label).small().color(dim));
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    );
}

/// While something is dragged: its name in a small box next to the pointer,
/// with a line under it saying what dropping here does. `ok` is false when
/// dropping here does nothing.
pub fn drag_chip(ctx: &egui::Context, theme: &Theme, name: &str, hint: &str, ok: bool) {
    let Some(pointer) = ctx.pointer_latest_pos() else {
        return;
    };
    egui::Area::new(egui::Id::unique("drag_chip"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer + egui::vec2(14.0, 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme.surface)
                .stroke(egui::Stroke::new(1.0, theme.accent_text))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(RichText::new(name).strong()).extend());
                    if !hint.is_empty() {
                        let color = if ok {
                            theme.accent_text
                        } else {
                            theme.text_dim
                        };
                        ui.add(egui::Label::new(RichText::new(hint).small().color(color)).extend());
                    }
                });
        });
}

/// Outline of a place the dragged thing can be dropped on, brighter while
/// the pointer is over it.
pub fn drop_outline(ui: &egui::Ui, theme: &Theme, rect: egui::Rect, over: bool) {
    // Above the content, but cut off where the list it belongs to is.
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("drop_outline"),
        ))
        .with_clip_rect(ui.clip_rect());
    if over {
        painter.rect_filled(rect, 8.0, theme.accent_text.gamma_multiply(0.10));
    }
    let width = if over { 2.0 } else { 1.0 };
    let color = if over {
        theme.accent_text
    } else {
        theme.accent_text.gamma_multiply(0.5)
    };
    painter.rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(width, color),
        egui::StrokeKind::Inside,
    );
}

/// Makes an area draggable without taking clicks away from the buttons and
/// links drawn in it. Call this before drawing the area, then
/// [`drag_area_drawn`] with the space it took. Returns `true` in the frame a
/// drag starts.
///
/// The drag has to be registered before the content so the content ends up
/// on top of it; where the content was is remembered from the last frame.
pub fn drag_area(ui: &mut egui::Ui, id: egui::Id) -> bool {
    let Some(rect) = ui.data(|data| data.get_temp::<egui::Rect>(id)) else {
        return false;
    };
    ui.interact(rect, id, egui::Sense::drag()).drag_started()
}

pub fn drag_area_drawn(ui: &egui::Ui, id: egui::Id, rect: egui::Rect) {
    ui.data_mut(|data| data.insert_temp(id, rect));
}
