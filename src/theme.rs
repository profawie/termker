//! Colors. The app's windows, lists and dialogs use a light or dark look
//! ([`Theme::ui`]). The terminal has its own color themes
//! ([`TERMINAL_THEMES`]); the default is "Phosphor", green text on black.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

pub use termker_core::DEFAULT_THEME;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    pub dark: bool,
    /// Main background (for a terminal theme, the terminal background).
    pub bg: Color32,
    /// Sidebar and panel background.
    pub panel: Color32,
    /// Card and input field background.
    pub surface: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    /// Fill of primary buttons and of the selected item.
    pub accent: Color32,
    /// Accent for text, links, outlines and status dots, readable straight
    /// on the backgrounds.
    pub accent_text: Color32,
    /// Text color on top of `accent`.
    pub on_accent: Color32,
    pub danger: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Dark look for the app: soft greys rather than black, easy on the eyes.
pub const UI_DARK: Theme = Theme {
    name: "Dark",
    dark: true,
    bg: rgb(0x24272B),
    panel: rgb(0x1C1F22),
    surface: rgb(0x2E3237),
    border: rgb(0x40454D),
    text: rgb(0xE4E6E9),
    text_dim: rgb(0xA4ABB3),
    accent: rgb(0x0B6BA8),
    accent_text: rgb(0x58A6FF),
    on_accent: rgb(0xFFFFFF),
    danger: rgb(0xF87171),
};

/// Light look for the app.
pub const UI_LIGHT: Theme = Theme {
    name: "Light",
    dark: false,
    bg: rgb(0xF6F7F8),
    panel: rgb(0xECEEF1),
    surface: rgb(0xFFFFFF),
    border: rgb(0xD2D7DD),
    text: rgb(0x1F2328),
    text_dim: rgb(0x57606A),
    accent: rgb(0x0969DA),
    accent_text: rgb(0x0550AE),
    on_accent: rgb(0xFFFFFF),
    danger: rgb(0xB91C1C),
};

/// Color themes for the terminal.
pub const TERMINAL_THEMES: &[Theme] = &[
    Theme {
        name: "Phosphor",
        dark: true,
        bg: rgb(0x000000),
        panel: rgb(0x040804),
        surface: rgb(0x0A150C),
        border: rgb(0x17472A),
        text: rgb(0x33FF66),
        text_dim: rgb(0x1E9E48),
        accent: rgb(0x33FF66),
        accent_text: rgb(0x33FF66),
        on_accent: rgb(0x000000),
        danger: rgb(0xFF5C5C),
    },
    Theme {
        name: "Amber",
        dark: true,
        bg: rgb(0x0A0700),
        panel: rgb(0x120C02),
        surface: rgb(0x1C1405),
        border: rgb(0x4D3608),
        text: rgb(0xFFB000),
        text_dim: rgb(0xC48A0A),
        accent: rgb(0xFFB000),
        accent_text: rgb(0xFFB000),
        on_accent: rgb(0x0A0700),
        danger: rgb(0xFF6B5C),
    },
    Theme {
        name: "Midnight",
        dark: true,
        bg: rgb(0x0B1020),
        panel: rgb(0x0F1630),
        surface: rgb(0x172042),
        border: rgb(0x2A3663),
        text: rgb(0xD6E0FF),
        text_dim: rgb(0x8592BF),
        accent: rgb(0x7C9CFF),
        accent_text: rgb(0x7C9CFF),
        on_accent: rgb(0x0B1020),
        danger: rgb(0xFF7A90),
    },
    Theme {
        name: "Graphite",
        dark: true,
        bg: rgb(0x16181B),
        panel: rgb(0x1C1F23),
        surface: rgb(0x262A2F),
        border: rgb(0x3A4047),
        text: rgb(0xE6E8EA),
        text_dim: rgb(0x979EA6),
        accent: rgb(0x4FD1C5),
        accent_text: rgb(0x4FD1C5),
        on_accent: rgb(0x10201E),
        danger: rgb(0xF47174),
    },
    Theme {
        name: "Paper",
        dark: false,
        bg: rgb(0xFAF8F3),
        panel: rgb(0xF1EEE6),
        surface: rgb(0xFFFFFF),
        border: rgb(0xD5D0C3),
        text: rgb(0x26302A),
        text_dim: rgb(0x555F59),
        accent: rgb(0x1F7A4D),
        accent_text: rgb(0x1F7A4D),
        on_accent: rgb(0xFFFFFF),
        danger: rgb(0xB3261E),
    },
];

impl Theme {
    /// The terminal theme called `name`, or the default one if the name is
    /// unknown.
    pub fn terminal(name: &str) -> Theme {
        TERMINAL_THEMES
            .iter()
            .find(|theme| theme.name == name)
            .or_else(|| {
                TERMINAL_THEMES
                    .iter()
                    .find(|theme| theme.name == DEFAULT_THEME)
            })
            .copied()
            .expect("default theme exists")
    }

    /// The app's dark or light look.
    pub fn ui(dark: bool) -> Theme {
        if dark { UI_DARK } else { UI_LIGHT }
    }

    /// Use this theme for the app's windows, lists and dialogs.
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(if self.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });

        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let radius = CornerRadius::same(6);

        visuals.override_text_color = Some(self.text);
        visuals.panel_fill = self.bg;
        visuals.window_fill = self.panel;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.extreme_bg_color = self.surface;
        visuals.faint_bg_color = self.surface;
        visuals.hyperlink_color = self.accent_text;
        visuals.selection.bg_fill = self.accent.gamma_multiply(0.35);
        visuals.selection.stroke = Stroke::new(1.0, self.accent_text);
        visuals.warn_fg_color = self.danger;
        visuals.error_fg_color = self.danger;

        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_fill = self.panel;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        widgets.noninteractive.corner_radius = radius;

        widgets.inactive.bg_fill = self.surface;
        widgets.inactive.weak_bg_fill = self.surface;
        widgets.inactive.bg_stroke = Stroke::new(1.0, self.border);
        widgets.inactive.fg_stroke = Stroke::new(1.0, self.text);
        widgets.inactive.corner_radius = radius;

        widgets.hovered.bg_fill = self.accent.gamma_multiply(0.18);
        widgets.hovered.weak_bg_fill = self.accent.gamma_multiply(0.18);
        widgets.hovered.bg_stroke = Stroke::new(1.0, self.accent_text);
        widgets.hovered.fg_stroke = Stroke::new(1.5, self.text);
        widgets.hovered.corner_radius = radius;

        widgets.active.bg_fill = self.accent.gamma_multiply(0.3);
        widgets.active.weak_bg_fill = self.accent.gamma_multiply(0.3);
        widgets.active.bg_stroke = Stroke::new(1.0, self.accent_text);
        widgets.active.fg_stroke = Stroke::new(2.0, self.text);
        widgets.active.corner_radius = radius;

        widgets.open = widgets.hovered;

        ctx.set_visuals(visuals);
        ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::vec2(12.0, 6.0);
            style.spacing.interact_size.y = 28.0;
            for (text_style, font) in style.text_styles.iter_mut() {
                font.size = match text_style {
                    egui::TextStyle::Small => 12.0,
                    egui::TextStyle::Heading => 22.0,
                    _ => 14.0,
                };
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_terminal_theme_is_green_on_black() {
        let theme = Theme::terminal(DEFAULT_THEME);
        assert!(theme.dark);
        assert_eq!(theme.bg, Color32::BLACK);
        let [r, g, b, _] = theme.text.to_array();
        assert!(g > r && g > b, "default text must be green");
    }

    #[test]
    fn app_looks_are_not_black_or_green_text() {
        assert_ne!(UI_DARK.bg, Color32::BLACK);
        const { assert!(UI_DARK.dark && !UI_LIGHT.dark) };
        for theme in [UI_DARK, UI_LIGHT] {
            let [r, g, b, _] = theme.text.to_array();
            assert!(
                g.abs_diff(r) < 16 && g.abs_diff(b) < 16,
                "{} text is grey",
                theme.name
            );
        }
    }

    #[test]
    fn app_buttons_are_blue_with_white_text() {
        for theme in [UI_DARK, UI_LIGHT] {
            let [r, g, b, _] = theme.accent.to_array();
            assert!(b > g && g > r, "{} buttons are blue", theme.name);
            assert_eq!(theme.on_accent, Color32::WHITE);
        }
    }

    #[test]
    fn unknown_theme_falls_back_to_default() {
        assert_eq!(Theme::terminal("missing").name, DEFAULT_THEME);
    }

    #[test]
    fn theme_names_are_unique() {
        for (i, a) in TERMINAL_THEMES.iter().enumerate() {
            assert!(TERMINAL_THEMES[i + 1..].iter().all(|b| a.name != b.name));
        }
    }

    /// WCAG contrast ratio between text and background must be at least 4.5.
    #[test]
    fn text_is_readable_on_every_background() {
        fn luminance(c: Color32) -> f32 {
            let [r, g, b, _] = c.to_array();
            let lin = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
        }
        fn contrast(a: Color32, b: Color32) -> f32 {
            let (la, lb) = (luminance(a), luminance(b));
            (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
        }
        for theme in TERMINAL_THEMES.iter().chain([&UI_DARK, &UI_LIGHT]) {
            for bg in [theme.bg, theme.panel, theme.surface] {
                assert!(contrast(theme.text, bg) >= 4.5, "{} text", theme.name);
                assert!(contrast(theme.text_dim, bg) >= 4.5, "{} dim", theme.name);
            }
            assert!(
                contrast(theme.on_accent, theme.accent) >= 4.5,
                "{} accent",
                theme.name
            );
            for bg in [theme.bg, theme.panel, theme.surface] {
                assert!(
                    contrast(theme.accent_text, bg) >= 4.5,
                    "{} accent text",
                    theme.name
                );
            }
        }
    }
}
