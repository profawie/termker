//! Terminal emulation, independent of the UI toolkit: the screen state
//! (via `alacritty_terminal`), key encoding and color resolution.

use std::sync::{Arc, Mutex};

use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, Term, TermMode},
    vte::ansi::{self, NamedColor, Rgb},
};

/// Collects what the emulator wants to tell the outside world.
#[derive(Clone, Default)]
pub struct Listener {
    replies: Arc<Mutex<Vec<u8>>>,
    title: Arc<Mutex<Option<String>>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            // Answers to queries from the remote side, e.g. cursor position.
            Event::PtyWrite(text) => self
                .replies
                .lock()
                .unwrap()
                .extend_from_slice(text.as_bytes()),
            Event::Title(title) => *self.title.lock().unwrap() = Some(title),
            Event::ResetTitle => *self.title.lock().unwrap() = None,
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.cols as usize
    }
}

pub struct Emulator {
    pub term: Term<Listener>,
    parser: ansi::Processor,
    listener: Listener,
    size: Size,
}

impl Emulator {
    pub fn new(size: Size) -> Self {
        let listener = Listener::default();
        Self {
            term: Term::new(Config::default(), &size, listener.clone()),
            parser: ansi::Processor::new(),
            listener,
            size,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// Feed bytes from the remote side. Returns bytes that must be sent back
    /// (replies to terminal queries).
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.parser.advance(&mut self.term, bytes);
        std::mem::take(&mut *self.listener.replies.lock().unwrap())
    }

    /// Returns `true` if the size actually changed.
    pub fn resize(&mut self, size: Size) -> bool {
        if size == self.size || size.cols == 0 || size.rows == 0 {
            return false;
        }
        self.size = size;
        self.term.resize(size);
        true
    }

    /// Window title set by the remote side, if any.
    pub fn title(&self) -> Option<String> {
        self.listener.title.lock().unwrap().clone()
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn scroll(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    /// Grid point for a cell of the visible screen, accounting for scrollback.
    pub fn point_at(&self, col: usize, row: usize) -> Point {
        let offset = self.term.grid().display_offset() as i32;
        let col = col.min(self.size.cols.saturating_sub(1) as usize);
        Point::new(Line(row as i32 - offset), Column(col))
    }

    pub fn start_selection(&mut self, kind: SelectionType, col: usize, row: usize, side: Side) {
        let point = self.point_at(col, row);
        self.term.selection = Some(Selection::new(kind, point, side));
    }

    pub fn update_selection(&mut self, col: usize, row: usize, side: Side) {
        let point = self.point_at(col, row);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, side);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// Selected text, or `None` if nothing (or only whitespace) is selected.
    pub fn selected_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.trim().is_empty())
    }

    /// Text of one visible row, for tests and accessibility.
    #[cfg(test)]
    pub fn row_text(&self, row: usize) -> String {
        let line = self.point_at(0, row).line;
        (0..self.size.cols as usize)
            .map(|col| self.term.grid()[line][Column(col)].c)
            .collect::<String>()
            .trim_end()
            .to_string()
    }
}

/// Keys the terminal encodes itself (everything that is not plain text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
    /// A letter or symbol pressed together with Ctrl.
    Ctrl(char),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyModifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Encode a key press as the bytes an xterm-compatible terminal would send.
pub fn encode_key(key: Key, mods: KeyModifiers, mode: TermMode) -> Option<Vec<u8>> {
    // xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4).
    let modifier = 1 + mods.shift as u8 + 2 * mods.alt as u8 + 4 * mods.ctrl as u8;
    let csi_letter = |letter: char| {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{letter}")
        } else if mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{letter}")
        } else {
            format!("\x1b[{letter}")
        }
    };
    let csi_tilde = |code: u8| {
        if modifier > 1 {
            format!("\x1b[{code};{modifier}~")
        } else {
            format!("\x1b[{code}~")
        }
    };
    let text = match key {
        Key::Enter => "\r".to_string(),
        Key::Backspace if mods.alt => "\x1b\x7f".to_string(),
        Key::Backspace => "\x7f".to_string(),
        Key::Tab if mods.shift => "\x1b[Z".to_string(),
        Key::Tab => "\t".to_string(),
        Key::Escape => "\x1b".to_string(),
        Key::Up => csi_letter('A'),
        Key::Down => csi_letter('B'),
        Key::Right => csi_letter('C'),
        Key::Left => csi_letter('D'),
        Key::Home => csi_letter('H'),
        Key::End => csi_letter('F'),
        Key::Insert => csi_tilde(2),
        Key::Delete => csi_tilde(3),
        Key::PageUp => csi_tilde(5),
        Key::PageDown => csi_tilde(6),
        Key::F(n @ 1..=4) if modifier == 1 => {
            format!("\x1bO{}", (b'P' + n - 1) as char)
        }
        Key::F(n @ 1..=4) => format!("\x1b[1;{modifier}{}", (b'P' + n - 1) as char),
        Key::F(n @ 5..=12) => csi_tilde([15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5]),
        Key::F(_) => return None,
        Key::Ctrl(c) => {
            let byte = match c.to_ascii_lowercase() {
                c @ 'a'..='z' => c as u8 - b'a' + 1,
                ' ' | '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '7' => 0x1f,
                '?' | '8' => 0x7f,
                _ => return None,
            };
            return Some(if mods.alt {
                vec![0x1b, byte]
            } else {
                vec![byte]
            });
        }
    };
    Some(text.into_bytes())
}

/// Bytes to send for pasted text. Line endings become carriage returns, and
/// the text is wrapped in bracketed-paste markers when the remote program
/// asked for them, so a pasted command is not run before the user confirms.
pub fn encode_paste(text: &str, mode: TermMode) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        // Strip the end marker so pasted text cannot break out of the paste.
        let text = text.replace("\x1b[201~", "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

/// The 16 base ANSI colors for dark and light backgrounds.
const ANSI_DARK: [u32; 16] = [
    0x1D1F21, 0xE05561, 0x8CC265, 0xD9B45B, 0x4AA5F0, 0xC162DE, 0x42B3C2, 0xC5C8C6, 0x6B7078,
    0xFF616E, 0xA5E075, 0xF0C674, 0x6CB8F5, 0xDE73FF, 0x4CD1E0, 0xFFFFFF,
];
const ANSI_LIGHT: [u32; 16] = [
    0x1D1F21, 0xC4283A, 0x3F7D1C, 0x8F6A00, 0x1D5FBF, 0x9A2FB8, 0x0F7A8A, 0x6B7078, 0x4A4F57,
    0xA61B2C, 0x2F6614, 0x735500, 0x154A99, 0x7A2394, 0x0B6170, 0x26302A,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub dark: bool,
}

const fn rgb(hex: u32) -> Rgb {
    Rgb {
        r: (hex >> 16) as u8,
        g: (hex >> 8) as u8,
        b: hex as u8,
    }
}

impl Palette {
    /// Resolve a cell color. `overrides` holds colors the remote program
    /// redefined with escape sequences.
    pub fn resolve(
        &self,
        color: ansi::Color,
        overrides: &alacritty_terminal::term::color::Colors,
    ) -> Rgb {
        match color {
            ansi::Color::Spec(rgb) => rgb,
            ansi::Color::Indexed(index) => overrides[index as usize].unwrap_or(self.indexed(index)),
            ansi::Color::Named(name) => overrides[name].unwrap_or(match name {
                NamedColor::Background => self.background,
                NamedColor::Cursor => self.cursor,
                name if (name as usize) < 16 => self.indexed(name as u8),
                // Foreground, and the dim/bright foreground variants.
                _ => self.foreground,
            }),
        }
    }

    fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => rgb(if self.dark { ANSI_DARK } else { ANSI_LIGHT }[index as usize]),
            // 6x6x6 color cube.
            16..=231 => {
                let index = index - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                Rgb {
                    r: level(index / 36),
                    g: level(index / 6 % 6),
                    b: level(index % 6),
                }
            }
            // 24-step grayscale ramp.
            _ => {
                let v = 8 + 10 * (index - 232);
                Rgb { r: v, g: v, b: v }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size { cols: 40, rows: 10 };

    fn keys(key: Key) -> String {
        String::from_utf8(encode_key(key, KeyModifiers::default(), TermMode::empty()).unwrap())
            .unwrap()
    }

    #[test]
    fn output_lands_on_the_screen() {
        let mut emulator = Emulator::new(SIZE);
        emulator.feed(b"hello\r\nworld");
        assert_eq!(emulator.row_text(0), "hello");
        assert_eq!(emulator.row_text(1), "world");
    }

    #[test]
    fn escape_sequences_are_interpreted_not_printed() {
        let mut emulator = Emulator::new(SIZE);
        emulator.feed(b"\x1b[31mred\x1b[0m \x1b[2;5Hx");
        assert_eq!(emulator.row_text(0), "red");
        assert_eq!(emulator.row_text(1), "    x");
    }

    #[test]
    fn cursor_position_query_is_answered() {
        let mut emulator = Emulator::new(SIZE);
        emulator.feed(b"abc");
        assert_eq!(emulator.feed(b"\x1b[6n"), b"\x1b[1;4R");
    }

    #[test]
    fn title_follows_the_remote_side() {
        let mut emulator = Emulator::new(SIZE);
        assert_eq!(emulator.title(), None);
        emulator.feed(b"\x1b]0;user@host: ~\x07");
        assert_eq!(emulator.title().as_deref(), Some("user@host: ~"));
    }

    #[test]
    fn resize_reports_only_real_changes() {
        let mut emulator = Emulator::new(SIZE);
        assert!(!emulator.resize(SIZE));
        assert!(!emulator.resize(Size { cols: 0, rows: 10 }));
        assert!(emulator.resize(Size { cols: 80, rows: 24 }));
        assert_eq!(emulator.size(), Size { cols: 80, rows: 24 });
    }

    #[test]
    fn scrollback_keeps_old_lines_and_selection_copies_text() {
        let mut emulator = Emulator::new(SIZE);
        for i in 0..30 {
            emulator.feed(format!("line {i}\r\n").as_bytes());
        }
        assert_eq!(emulator.row_text(0), "line 21");
        emulator.scroll(5);
        assert_eq!(emulator.row_text(0), "line 16");

        emulator.start_selection(SelectionType::Simple, 0, 0, Side::Left);
        emulator.update_selection(6, 0, Side::Right);
        assert_eq!(emulator.selected_text().as_deref(), Some("line 16"));
        emulator.clear_selection();
        assert_eq!(emulator.selected_text(), None);

        emulator.scroll_to_bottom();
        assert_eq!(emulator.row_text(0), "line 21");
    }

    #[test]
    fn basic_keys() {
        assert_eq!(keys(Key::Enter), "\r");
        assert_eq!(keys(Key::Backspace), "\x7f");
        assert_eq!(keys(Key::Tab), "\t");
        assert_eq!(keys(Key::Up), "\x1b[A");
        assert_eq!(keys(Key::Delete), "\x1b[3~");
        assert_eq!(keys(Key::PageDown), "\x1b[6~");
        assert_eq!(keys(Key::F(1)), "\x1bOP");
        assert_eq!(keys(Key::F(5)), "\x1b[15~");
        assert_eq!(keys(Key::F(12)), "\x1b[24~");
    }

    #[test]
    fn control_keys() {
        assert_eq!(keys(Key::Ctrl('c')), "\x03");
        assert_eq!(keys(Key::Ctrl('D')), "\x04");
        assert_eq!(keys(Key::Ctrl('z')), "\x1a");
        assert_eq!(keys(Key::Ctrl('[')), "\x1b");
        assert_eq!(keys(Key::Ctrl(' ')), "\0");
        assert_eq!(
            encode_key(Key::Ctrl('!'), KeyModifiers::default(), TermMode::empty()),
            None
        );
    }

    #[test]
    fn arrows_follow_application_cursor_mode_and_modifiers() {
        let none = KeyModifiers::default();
        assert_eq!(
            encode_key(Key::Up, none, TermMode::APP_CURSOR).unwrap(),
            b"\x1bOA"
        );
        let ctrl = KeyModifiers { ctrl: true, ..none };
        assert_eq!(
            encode_key(Key::Right, ctrl, TermMode::APP_CURSOR).unwrap(),
            b"\x1b[1;5C"
        );
        let shift = KeyModifiers {
            shift: true,
            ..none
        };
        assert_eq!(
            encode_key(Key::Tab, shift, TermMode::empty()).unwrap(),
            b"\x1b[Z"
        );
    }

    #[test]
    fn paste_is_bracketed_only_when_requested() {
        assert_eq!(encode_paste("a\r\nb\nc", TermMode::empty()), b"a\rb\rc");
        assert_eq!(
            encode_paste("ls\n", TermMode::BRACKETED_PASTE),
            b"\x1b[200~ls\r\x1b[201~"
        );
        // A pasted end marker must not terminate the paste early.
        assert_eq!(
            encode_paste("x\x1b[201~rm -rf", TermMode::BRACKETED_PASTE),
            b"\x1b[200~xrm -rf\x1b[201~"
        );
    }

    #[test]
    fn palette_resolves_named_indexed_and_true_color() {
        let palette = Palette {
            foreground: rgb(0x33FF66),
            background: rgb(0x000000),
            cursor: rgb(0x33FF66),
            dark: true,
        };
        let none = alacritty_terminal::term::color::Colors::default();
        let named = |name| palette.resolve(ansi::Color::Named(name), &none);
        assert_eq!(named(NamedColor::Foreground), rgb(0x33FF66));
        assert_eq!(named(NamedColor::Background), rgb(0x000000));
        assert_eq!(named(NamedColor::Red), rgb(ANSI_DARK[1]));
        let indexed = |i| palette.resolve(ansi::Color::Indexed(i), &none);
        assert_eq!(indexed(16), rgb(0x000000));
        assert_eq!(indexed(196), rgb(0xFF0000));
        assert_eq!(indexed(231), rgb(0xFFFFFF));
        assert_eq!(indexed(232), rgb(0x080808));
        assert_eq!(indexed(255), rgb(0xEEEEEE));
        let spec = ansi::Color::Spec(rgb(0x123456));
        assert_eq!(palette.resolve(spec, &none), rgb(0x123456));
    }
}
