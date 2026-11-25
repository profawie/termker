//! Terminal widget: paints the emulator screen and turns keyboard and mouse
//! input into bytes for the session.

use alacritty_terminal::{
    index::Side,
    selection::SelectionType,
    term::{TermMode, cell::Flags},
    vte::ansi::{CursorShape, Rgb},
};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Stroke, Vec2};

use crate::{
    session::Session,
    terminal::{self, Key, KeyModifiers, Palette, Size},
    theme::Theme,
};

const PADDING: f32 = 6.0;
/// Lines sent as arrow keys per wheel line in full-screen programs.
const MAX_LINES_PER_FRAME: i32 = 50;

/// Show the terminal in the remaining space. `interactive` is false while a
/// prompt is open, so typing goes to the prompt instead. `active` is false
/// for a terminal shown beside the one being typed into: it lets go of the
/// keyboard. Returns `true` when it was clicked, asking to become active.
pub fn show(
    ui: &mut egui::Ui,
    session: &mut Session,
    theme: &Theme,
    font_size: f32,
    interactive: bool,
    active: bool,
) -> bool {
    let font = FontId::monospace(font_size);
    let (cell_width, cell_height) =
        ui.fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
    let cell = Vec2::new(cell_width.max(1.0), cell_height.max(1.0));

    let outer = ui.available_rect_before_wrap();
    let id = ui.scope_id().with(("terminal", session.id));
    let response = ui.interact(outer, id, egui::Sense::click_and_drag());
    ui.advance_cursor_after_rect(outer);
    let area = outer.shrink(PADDING);

    session.resize(Size {
        cols: (area.width() / cell.x).floor().clamp(2.0, 1000.0) as u16,
        rows: (area.height() / cell.y).floor().clamp(1.0, 1000.0) as u16,
    });

    let pressed = response.clicked() || response.drag_started();
    if interactive {
        if pressed {
            response.request_focus();
        } else if !active && response.has_focus() {
            // Another terminal beside it is the one being typed into now.
            response.surrender_focus();
        }
        // Take focus when nothing else has it, so typing works right away.
        if active && ui.memory(|memory| memory.focused().is_none()) {
            response.request_focus();
        }
    }
    let focused = interactive && (active || pressed) && response.has_focus();
    if focused {
        // Keep Tab, arrows and Escape for the terminal instead of moving focus.
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        handle_keyboard(ui, session);
    }
    if interactive {
        handle_mouse(ui, &response, session, area, cell);
    }

    paint(ui, session, theme, &font, area, outer, cell, focused);
    pressed
}

fn handle_keyboard(ui: &mut egui::Ui, session: &mut Session) {
    let events = ui.input(|input| input.events.clone());
    let mode = session.emulator.mode();
    let mac = cfg!(target_os = "macos");
    let mut output: Vec<u8> = Vec::new();

    for event in events {
        match event {
            egui::Event::Text(text) => {
                // On Windows and Linux, Alt+key sends Escape then the key.
                let alt = !mac && ui.input(|input| input.modifiers.alt);
                if alt {
                    output.push(0x1b);
                }
                output.extend_from_slice(text.as_bytes());
            }
            egui::Event::Paste(text) => output.extend(terminal::encode_paste(&text, mode)),
            // Cmd+C on macOS; Ctrl+C elsewhere, where it copies only when
            // something is selected and otherwise interrupts as usual.
            egui::Event::Copy => {
                if let Some(text) = session.emulator.selected_text() {
                    ui.ctx().copy_text(text);
                    session.emulator.clear_selection();
                } else if !mac {
                    output.push(0x03);
                }
            }
            egui::Event::Cut if !mac => output.push(0x18),
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                // Ctrl+Shift+C / Ctrl+Shift+V: explicit copy and paste.
                if !mac && modifiers.ctrl && modifiers.shift && key == egui::Key::C {
                    if let Some(text) = session.emulator.selected_text() {
                        ui.ctx().copy_text(text);
                    }
                    continue;
                }
                if !mac && modifiers.ctrl && modifiers.shift && key == egui::Key::V {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    continue;
                }
                // Shift+PageUp / Shift+PageDown scroll through history.
                if modifiers.shift && matches!(key, egui::Key::PageUp | egui::Key::PageDown) {
                    let page = session.emulator.size().rows as i32 - 1;
                    session.emulator.scroll(if key == egui::Key::PageUp {
                        page
                    } else {
                        -page
                    });
                    continue;
                }
                let mods = KeyModifiers {
                    shift: modifiers.shift,
                    alt: modifiers.alt,
                    ctrl: modifiers.ctrl,
                };
                if let Some(key) = map_key(key, modifiers.ctrl)
                    && let Some(bytes) = terminal::encode_key(key, mods, mode)
                {
                    output.extend(bytes);
                }
            }
            _ => {}
        }
    }

    if !output.is_empty() {
        session.emulator.clear_selection();
        session.emulator.scroll_to_bottom();
        session.send_input(output);
    }
}

/// Map an egui key to a terminal key. Plain characters are not mapped here:
/// they arrive as text events.
fn map_key(key: egui::Key, ctrl: bool) -> Option<Key> {
    use egui::Key as K;
    Some(match key {
        K::Enter => Key::Enter,
        K::Backspace => Key::Backspace,
        K::Tab => Key::Tab,
        K::Escape => Key::Escape,
        K::ArrowUp => Key::Up,
        K::ArrowDown => Key::Down,
        K::ArrowRight => Key::Right,
        K::ArrowLeft => Key::Left,
        K::Home => Key::Home,
        K::End => Key::End,
        K::PageUp => Key::PageUp,
        K::PageDown => Key::PageDown,
        K::Insert => Key::Insert,
        K::Delete => Key::Delete,
        K::F1 => Key::F(1),
        K::F2 => Key::F(2),
        K::F3 => Key::F(3),
        K::F4 => Key::F(4),
        K::F5 => Key::F(5),
        K::F6 => Key::F(6),
        K::F7 => Key::F(7),
        K::F8 => Key::F(8),
        K::F9 => Key::F(9),
        K::F10 => Key::F(10),
        K::F11 => Key::F(11),
        K::F12 => Key::F(12),
        K::Space if ctrl => Key::Ctrl(' '),
        K::OpenBracket if ctrl => Key::Ctrl('['),
        K::CloseBracket if ctrl => Key::Ctrl(']'),
        K::Backslash if ctrl => Key::Ctrl('\\'),
        K::Minus if ctrl => Key::Ctrl('_'),
        other if ctrl => {
            let name = other.name();
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => Key::Ctrl(c),
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn handle_mouse(
    ui: &mut egui::Ui,
    response: &egui::Response,
    session: &mut Session,
    area: Rect,
    cell: Vec2,
) {
    let size = session.emulator.size();
    let locate = |pos: Pos2| {
        let x = ((pos.x - area.left()) / cell.x).max(0.0);
        let y = ((pos.y - area.top()) / cell.y).max(0.0);
        let col = (x as usize).min(size.cols as usize - 1);
        let row = (y as usize).min(size.rows as usize - 1);
        let side = if x.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        (col, row, side)
    };

    if let Some(pos) = response.interact_pointer_pos() {
        let (col, row, side) = locate(pos);
        if response.double_clicked() {
            session
                .emulator
                .start_selection(SelectionType::Semantic, col, row, side);
        } else if response.triple_clicked() {
            session
                .emulator
                .start_selection(SelectionType::Lines, col, row, side);
        } else if response.drag_started() {
            session
                .emulator
                .start_selection(SelectionType::Simple, col, row, side);
        } else if response.dragged() {
            session.emulator.update_selection(col, row, side);
        } else if response.clicked() {
            session.emulator.clear_selection();
        }
    }

    if response.hovered() {
        let delta = ui.input(|input| input.smooth_scroll_delta.y);
        if delta != 0.0 {
            let total = session.scroll_remainder + delta / cell.y;
            let lines = (total.trunc() as i32).clamp(-MAX_LINES_PER_FRAME, MAX_LINES_PER_FRAME);
            session.scroll_remainder = total.fract();
            if lines == 0 {
                return;
            }
            let mode = session.emulator.mode();
            if mode.contains(TermMode::ALT_SCREEN) {
                // Full-screen programs have no scrollback; send arrow keys.
                let key = if lines > 0 { Key::Up } else { Key::Down };
                if let Some(bytes) = terminal::encode_key(key, KeyModifiers::default(), mode) {
                    session.send_input(bytes.repeat(lines.unsigned_abs() as usize));
                }
            } else {
                session.emulator.scroll(lines);
            }
        }
    }
}

fn color32(rgb: Rgb) -> Color32 {
    Color32::from_rgb(rgb.r, rgb.g, rgb.b)
}

fn rgb(color: Color32) -> Rgb {
    let [r, g, b, _] = color.to_array();
    Rgb { r, g, b }
}

/// Mix `over` onto `base`; `amount` 0 keeps `base`, 1 gives `over`.
fn mix(base: Color32, over: Color32, amount: f32) -> Color32 {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    let ([r1, g1, b1, _], [r2, g2, b2, _]) = (base.to_array(), over.to_array());
    Color32::from_rgb(channel(r1, r2), channel(g1, g2), channel(b1, b2))
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &egui::Ui,
    session: &Session,
    theme: &Theme,
    font: &FontId,
    area: Rect,
    outer: Rect,
    cell: Vec2,
    focused: bool,
) {
    let painter = ui.painter_at(outer);
    painter.rect_filled(outer, 0.0, theme.bg);

    let palette = Palette {
        foreground: rgb(theme.text),
        background: rgb(theme.bg),
        cursor: rgb(theme.accent),
        dark: theme.dark,
    };
    let selection_bg = mix(theme.bg, theme.accent, 0.35);
    let content = session.emulator.term.renderable_content();
    let offset = content.display_offset as i32;
    let cell_pos = |col: usize, row: i32| {
        Pos2::new(
            area.left() + col as f32 * cell.x,
            area.top() + row as f32 * cell.y,
        )
    };

    // A run of same-colored ASCII cells is painted as one piece of text.
    struct Run {
        col: usize,
        row: i32,
        color: Color32,
        text: String,
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut underlines: Vec<(Pos2, Color32)> = Vec::new();
    let cursor_point = content.cursor.point;
    let mut cursor_char = ' ';

    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset;
        let col = indexed.point.column.0;
        let cell_data = indexed.cell;
        let flags = cell_data.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }

        let mut fg = color32(palette.resolve(cell_data.fg, content.colors));
        let mut bg = color32(palette.resolve(cell_data.bg, content.colors));
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = mix(bg, fg, 0.6);
        }
        let selected = content
            .selection
            .is_some_and(|range| range.contains(indexed.point));
        if selected {
            bg = selection_bg;
        }

        let width = if flags.contains(Flags::WIDE_CHAR) {
            2.0
        } else {
            1.0
        };
        if bg != theme.bg {
            let rect = Rect::from_min_size(cell_pos(col, row), Vec2::new(cell.x * width, cell.y));
            painter.rect_filled(rect, 0.0, bg);
        }
        if indexed.point == cursor_point {
            cursor_char = cell_data.c;
        }

        let c = cell_data.c;
        if c == ' ' || c == '\0' || flags.contains(Flags::HIDDEN) {
            continue;
        }
        if flags.intersects(Flags::ALL_UNDERLINES) {
            underlines.push((cell_pos(col, row), fg));
        }
        match runs.last_mut() {
            Some(run)
                if c.is_ascii()
                    && run.row == row
                    && run.color == fg
                    && run.col + run.text.len() == col
                    && run.text.is_ascii() =>
            {
                run.text.push(c);
            }
            _ => runs.push(Run {
                col,
                row,
                color: fg,
                text: c.to_string(),
            }),
        }
    }

    for run in &runs {
        painter.text(
            cell_pos(run.col, run.row),
            egui::Align2::LEFT_TOP,
            &run.text,
            font.clone(),
            run.color,
        );
    }
    for (pos, color) in underlines {
        let y = pos.y + cell.y - 1.5;
        painter.hline(pos.x..=pos.x + cell.x, y, Stroke::new(1.0, color));
    }

    // Cursor, only when the view is not scrolled back into history.
    let cursor_row = cursor_point.line.0 + offset;
    let visible = content.mode.contains(TermMode::SHOW_CURSOR)
        && content.cursor.shape != CursorShape::Hidden
        && offset == 0
        && session.is_connected();
    if visible {
        let rect = Rect::from_min_size(cell_pos(cursor_point.column.0, cursor_row), cell);
        match content.cursor.shape {
            _ if !focused => {
                painter.rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(1.0, theme.accent),
                    egui::StrokeKind::Inside,
                );
            }
            CursorShape::Beam => {
                painter.rect_filled(
                    Rect::from_min_size(rect.min, Vec2::new(2.0, cell.y)),
                    0.0,
                    theme.accent,
                );
            }
            CursorShape::Underline => {
                painter.rect_filled(
                    Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 2.0), rect.max),
                    0.0,
                    theme.accent,
                );
            }
            _ => {
                painter.rect_filled(rect, 0.0, theme.accent);
                if cursor_char != ' ' && cursor_char != '\0' {
                    painter.text(
                        rect.min,
                        egui::Align2::LEFT_TOP,
                        cursor_char,
                        font.clone(),
                        theme.on_accent,
                    );
                }
            }
        }
    }

    // Small marker while looking at history.
    if offset > 0 {
        painter.text(
            outer.right_top() + Vec2::new(-10.0, 6.0),
            egui::Align2::RIGHT_TOP,
            format!("{offset} lines up"),
            FontId::proportional(12.0),
            theme.text_dim,
        );
    }
}
