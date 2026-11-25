//! Pages and shared UI components.

pub mod about;
pub mod forwards;
pub mod hosts;
pub mod keychain;
pub mod os;
pub mod selection;
pub mod session;
pub mod settings;
pub mod sftp;
pub mod split;
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

#[derive(Clone, Copy, PartialEq)]
pub enum RowIcon {
    Rename,
    Delete,
    NewGroup,
    NewHost,
}

/// A small square button with a drawn icon: a pencil for Rename, a bin for
/// Delete, a folder with a plus for a new group and a screen with a plus for
/// a new host. The name is shown when the pointer rests on it.
pub fn icon_button(ui: &mut egui::Ui, theme: &Theme, icon: RowIcon, name: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    let hovered = response.hovered();
    let color = match (icon, hovered) {
        (RowIcon::Delete, true) => theme.danger,
        (_, true) => theme.text,
        (_, false) => theme.text_dim,
    };
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 6.0, theme.border);
    }
    let stroke = egui::Stroke::new(1.4, color);
    let c = rect.center();
    let at = |x: f32, y: f32| c + egui::vec2(x, y);
    match icon {
        RowIcon::Rename => {
            // A pencil pointing down to the left.
            painter.add(egui::Shape::closed_line(
                vec![at(2.5, -6.5), at(6.5, -2.5), at(-2.5, 6.5), at(-6.5, 2.5)],
                stroke,
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![at(-6.5, 2.5), at(-2.5, 6.5), at(-7.0, 7.0)],
                color,
                stroke,
            ));
            painter.line_segment([at(0.0, -4.0), at(4.0, 0.0)], stroke);
        }
        RowIcon::Delete => {
            // A bin: lid with a handle, and a body with two lines.
            painter.line_segment([at(-6.5, -4.0), at(6.5, -4.0)], stroke);
            painter.line_segment([at(-2.0, -6.5), at(2.0, -6.5)], stroke);
            painter.line_segment([at(-2.0, -6.5), at(-2.0, -4.0)], stroke);
            painter.line_segment([at(2.0, -6.5), at(2.0, -4.0)], stroke);
            painter.add(egui::Shape::line(
                vec![at(-5.0, -4.0), at(-4.0, 7.0), at(4.0, 7.0), at(5.0, -4.0)],
                stroke,
            ));
            painter.line_segment([at(-1.5, -1.0), at(-1.5, 4.0)], stroke);
            painter.line_segment([at(1.5, -1.0), at(1.5, 4.0)], stroke);
        }
        RowIcon::NewGroup => {
            // A folder outline with a plus inside.
            painter.add(egui::Shape::closed_line(
                vec![
                    at(-7.5, -5.5),
                    at(-2.5, -5.5),
                    at(-0.5, -3.5),
                    at(7.5, -3.5),
                    at(7.5, 6.5),
                    at(-7.5, 6.5),
                ],
                stroke,
            ));
            painter.line_segment([at(-3.0, 1.5), at(3.0, 1.5)], stroke);
            painter.line_segment([at(0.0, -1.5), at(0.0, 4.5)], stroke);
        }
        RowIcon::NewHost => {
            // A screen on a stand with a plus inside.
            painter.rect_stroke(
                egui::Rect::from_min_max(at(-7.5, -6.5), at(7.5, 3.5)),
                1.5,
                stroke,
                egui::StrokeKind::Middle,
            );
            painter.line_segment([at(-4.0, 6.5), at(4.0, 6.5)], stroke);
            painter.line_segment([at(0.0, 3.5), at(0.0, 6.5)], stroke);
            painter.line_segment([at(-3.0, -1.5), at(3.0, -1.5)], stroke);
            painter.line_segment([at(0.0, -4.5), at(0.0, 1.5)], stroke);
        }
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(name)
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

/// How a card was picked up, kept while it is dragged so its copy under the
/// pointer stays where the hand took hold of it.
#[derive(Clone, Copy)]
pub struct Grab {
    /// From the top left corner of the card to the pointer.
    offset: egui::Vec2,
    /// Size of the card where it lies.
    pub size: egui::Vec2,
    /// When it was picked up, in egui's clock.
    at: f64,
}

impl Grab {
    /// For a row that has no frame of its own where it lies: the card it
    /// becomes while dragged is `padding` larger on every side and at most
    /// `max_width` wide.
    pub fn framed(mut self, padding: egui::Vec2, max_width: f32) -> Self {
        self.size += 2.0 * padding;
        self.offset += padding;
        if self.size.x > max_width {
            self.size.x = max_width;
            // Keep hold of the card when the row was taken far to the right.
            self.offset.x = self.offset.x.min(max_width - 24.0);
        }
        self
    }

    /// Picked up from `rect` at the place the pointer was pressed.
    pub fn new(ui: &egui::Ui, rect: egui::Rect) -> Self {
        let (pressed, at) = ui.input(|input| (input.pointer.press_origin(), input.time));
        Grab {
            offset: pressed.unwrap_or(rect.center()) - rect.min,
            size: rect.size(),
            at,
        }
    }

    /// For a card drawn at `size` while dragged, whatever size the thing
    /// had where it lies; the pointer keeps hold of it inside.
    pub fn resized(mut self, size: egui::Vec2) -> Self {
        self.size = size;
        self.offset = self
            .offset
            .min(size - egui::vec2(12.0, 8.0))
            .max(egui::vec2(12.0, 8.0));
        self
    }
}

/// Seconds a card takes to rise off the page once it is picked up.
const LIFT_TIME: f64 = 0.12;

/// The dragged card itself, lifted off the page: it follows the pointer
/// from the spot it was taken by, a little larger and with a shadow under
/// it. `content` draws the card's inside, `padding` away from its edge, at
/// the width it has where it lies.
/// A line under the card says what dropping here does; `ok` is false when
/// dropping here does nothing.
pub fn drag_float(
    ctx: &egui::Context,
    theme: &Theme,
    grab: Grab,
    padding: egui::Vec2,
    hint: &str,
    ok: bool,
    content: impl FnOnce(&mut egui::Ui),
) {
    let Some(pointer) = ctx.pointer_latest_pos() else {
        return;
    };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    // 0 on the page, 1 fully lifted.
    let lift = (((ctx.input(|input| input.time) - grab.at) / LIFT_TIME) as f32).clamp(0.0, 1.0);
    if lift < 1.0 {
        ctx.request_repaint();
    }
    let dark = ctx.global_style().visuals.dark_mode;
    let shadow = egui::Shadow {
        offset: [0, (10.0 * lift) as i8],
        blur: (28.0 * lift) as u8,
        spread: 0,
        color: egui::Color32::from_black_alpha(if dark { 150 } else { 70 }),
    };
    let area = egui::Area::new(egui::Id::unique("drag_float"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer - grab.offset - egui::vec2(0.0, 3.0 * lift))
        .interactable(false)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            egui::Frame::new()
                .fill(theme.surface)
                .stroke(egui::Stroke::new(1.5, theme.accent_text))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(padding.x as i8, padding.y as i8))
                .shadow(shadow)
                .show(ui, |ui| {
                    ui.set_width(grab.size.x - 2.0 * padding.x);
                    ui.set_min_height(grab.size.y - 2.0 * padding.y);
                    content(ui);
                });
            if !hint.is_empty() {
                let (fill, text) = if ok {
                    (theme.accent, theme.on_accent)
                } else {
                    (theme.panel, theme.text_dim)
                };
                egui::Frame::new()
                    .fill(fill)
                    .corner_radius(egui::CornerRadius::same(10))
                    .inner_margin(egui::Margin::symmetric(10, 3))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(hint).small().color(text)).extend());
                    });
            }
        });
    // Grown around the pointer, so the spot it is held by does not move.
    let scale = 1.0 + 0.03 * lift;
    ctx.set_transform_layer(
        area.response.layer_id,
        egui::emath::TSTransform::from_translation(pointer.to_vec2())
            * egui::emath::TSTransform::from_scaling(scale)
            * egui::emath::TSTransform::from_translation(-pointer.to_vec2()),
    );
}

/// How much of a card stays visible where it lies while its copy is being
/// dragged.
pub const DRAGGED_OPACITY: f32 = 0.3;

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
/// [`drag_area_drawn`] with the space it took. Returns how it was picked up
/// in the frame a drag starts.
///
/// The drag has to be registered before the content so the content ends up
/// on top of it; where the content was is remembered from the last frame.
pub fn drag_area(ui: &mut egui::Ui, id: egui::Id) -> Option<Grab> {
    press_area(ui, id, egui::Sense::drag()).and_then(|(_, grab)| grab)
}

/// Like [`drag_area`], with `sense` saying what else the area answers to,
/// such as clicks. Returns the area's response, and how it was picked up
/// in the frame a drag starts.
pub fn press_area(
    ui: &mut egui::Ui,
    id: egui::Id,
    sense: egui::Sense,
) -> Option<(egui::Response, Option<Grab>)> {
    let rect = ui.data(|data| data.get_temp::<egui::Rect>(id))?;
    let response = ui.interact(rect, id, sense);
    if !response.drag_started() {
        return Some((response, None));
    }
    let grab = Grab::new(ui, rect);
    Some((response, Some(grab)))
}

pub fn drag_area_drawn(ui: &egui::Ui, id: egui::Id, rect: egui::Rect) {
    ui.data_mut(|data| data.insert_temp(id, rect));
}
