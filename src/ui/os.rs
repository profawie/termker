//! The remote operating system as a small drawn badge with its name.
//!
//! The badges are termker's own drawings: a round mark in the colour the
//! system is known for, with its initial, a penguin for other Linux systems
//! and a command key for macOS. No vendor artwork is used or shipped.

use eframe::egui::{self, Color32, RichText};

use crate::{
    ssh::{OsFamily, RemoteOs},
    theme::Theme,
};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mark {
    Letter(char),
    Penguin,
    CommandKey,
}

/// Mark and badge colour by `os-release` identifier (or lowercase kernel
/// name where there is no `os-release`).
const KNOWN: &[(&str, Mark, u32)] = &[
    ("ubuntu", Mark::Letter('U'), 0xE95420),
    ("debian", Mark::Letter('D'), 0xA80030),
    ("raspbian", Mark::Letter('R'), 0xC51A4A),
    ("fedora", Mark::Letter('F'), 0x3C6EB4),
    ("rhel", Mark::Letter('R'), 0xCC0000),
    ("centos", Mark::Letter('C'), 0x932279),
    ("rocky", Mark::Letter('R'), 0x10B981),
    ("almalinux", Mark::Letter('A'), 0x0F4266),
    ("ol", Mark::Letter('O'), 0xC74634),
    ("amzn", Mark::Letter('A'), 0xFF9900),
    ("arch", Mark::Letter('A'), 0x1793D1),
    ("manjaro", Mark::Letter('M'), 0x35BF5C),
    ("alpine", Mark::Letter('A'), 0x0D597F),
    ("opensuse", Mark::Letter('S'), 0x73BA25),
    ("opensuse-leap", Mark::Letter('S'), 0x73BA25),
    ("opensuse-tumbleweed", Mark::Letter('S'), 0x73BA25),
    ("suse", Mark::Letter('S'), 0x30BA78),
    ("sles", Mark::Letter('S'), 0x30BA78),
    ("kali", Mark::Letter('K'), 0x367BF0),
    ("linuxmint", Mark::Letter('M'), 0x87CF3E),
    ("pop", Mark::Letter('P'), 0x48B9C7),
    ("gentoo", Mark::Letter('G'), 0x54487A),
    ("nixos", Mark::Letter('N'), 0x5277C3),
    ("void", Mark::Letter('V'), 0x478061),
    ("freebsd", Mark::Letter('F'), 0xAB2B28),
    ("openbsd", Mark::Letter('O'), 0xF2CA30),
    ("netbsd", Mark::Letter('N'), 0xF26711),
    ("dragonfly", Mark::Letter('D'), 0x5B8C2A),
    ("sunos", Mark::Letter('S'), 0xE07B00),
    ("aix", Mark::Letter('A'), 0x1F70C1),
    ("macos", Mark::CommandKey, 0x8E8E93),
];

fn mark_for(os: &RemoteOs) -> (Mark, Color32) {
    let known = |id: &str| KNOWN.iter().find(|(known, ..)| *known == id);
    let found = known(&os.id).or_else(|| os.id_like.iter().find_map(|id| known(id)));
    match found {
        Some(&(_, mark, rgb)) => (mark, rgb_color(rgb)),
        None => match os.family {
            OsFamily::Linux => (Mark::Penguin, rgb_color(0xF2C230)),
            OsFamily::MacOs => (Mark::CommandKey, rgb_color(0x8E8E93)),
            OsFamily::Unix => {
                let letter = os.name.chars().find(|c| c.is_alphanumeric()).unwrap_or('U');
                (
                    Mark::Letter(letter.to_ascii_uppercase()),
                    rgb_color(0x6B7280),
                )
            }
        },
    }
}

fn rgb_color(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Black or white, whichever reads better on `fill`.
fn ink_on(fill: Color32) -> Color32 {
    let luminance = 0.299 * fill.r() as f32 + 0.587 * fill.g() as f32 + 0.114 * fill.b() as f32;
    if luminance > 150.0 {
        Color32::from_rgb(0x1A, 0x1A, 0x1A)
    } else {
        Color32::WHITE
    }
}

/// What the pointer shows over a badge: everything that is known.
fn details(os: &RemoteOs) -> String {
    let mut text = format!("{} ({})", os.title(), os.family.label());
    if !os.kernel.is_empty() {
        text.push_str(&format!("\nKernel: {}", os.kernel));
    }
    text
}

/// The round badge alone, `size` points wide.
pub fn badge(ui: &mut egui::Ui, os: &RemoteOs, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let (mark, fill) = mark_for(os);
    let painter = ui.painter();
    let c = rect.center();
    let r = size / 2.0;
    painter.circle_filled(c, r, fill);
    let ink = ink_on(fill);
    // Positions in units of the radius, so every size looks the same.
    let at = |x: f32, y: f32| c + egui::vec2(x, y) * r;
    match mark {
        Mark::Letter(letter) => {
            painter.text(
                c,
                egui::Align2::CENTER_CENTER,
                letter,
                egui::FontId::proportional(size * 0.68),
                ink,
            );
        }
        Mark::Penguin => {
            let black = Color32::from_rgb(0x1A, 0x1A, 0x1A);
            let orange = Color32::from_rgb(0xE8, 0x7A, 0x10);
            // Feet, body, head, belly, eyes and beak.
            painter.circle_filled(at(-0.26, 0.66), 0.16 * r, orange);
            painter.circle_filled(at(0.26, 0.66), 0.16 * r, orange);
            painter.add(egui::Shape::ellipse_filled(
                at(0.0, 0.18),
                egui::vec2(0.46, 0.52) * r,
                black,
            ));
            painter.circle_filled(at(0.0, -0.36), 0.34 * r, black);
            painter.add(egui::Shape::ellipse_filled(
                at(0.0, 0.26),
                egui::vec2(0.28, 0.38) * r,
                Color32::WHITE,
            ));
            painter.circle_filled(at(-0.13, -0.42), 0.08 * r, Color32::WHITE);
            painter.circle_filled(at(0.13, -0.42), 0.08 * r, Color32::WHITE);
            painter.add(egui::Shape::convex_polygon(
                vec![at(-0.14, -0.28), at(0.14, -0.28), at(0.0, -0.1)],
                orange,
                egui::Stroke::NONE,
            ));
        }
        Mark::CommandKey => {
            // A square with a loop on each corner.
            let stroke = egui::Stroke::new((0.13 * r).max(1.0), ink);
            let inner = 0.2;
            let loop_radius = 0.17 * r;
            painter.rect_stroke(
                egui::Rect::from_min_max(at(-inner, -inner), at(inner, inner)),
                0.0,
                stroke,
                egui::StrokeKind::Middle,
            );
            let corner = inner + 0.17;
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                painter.circle_stroke(at(x * corner, y * corner), loop_radius, stroke);
            }
        }
    }
    response.on_hover_text(details(os))
}

/// Badge, name with version, and family, in one line: the session bar.
pub fn summary(ui: &mut egui::Ui, theme: &Theme, os: &RemoteOs) {
    let details = details(os);
    badge(ui, os, 20.0);
    ui.add(egui::Label::new(RichText::new(os.title()).color(theme.text)).truncate())
        .on_hover_text(&details);
    ui.add(
        egui::Label::new(
            RichText::new(os.family_text())
                .small()
                .color(theme.text_dim),
        )
        .truncate(),
    )
    .on_hover_text(details);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(text: &str) -> RemoteOs {
        termker_core::os::parse(text).unwrap()
    }

    #[test]
    fn badge_follows_the_system_then_its_relatives_then_the_family() {
        let ubuntu = os("TERMKER_UNAME=Linux 6.8.0\nNAME=Ubuntu\nID=ubuntu\nID_LIKE=debian\n");
        assert_eq!(mark_for(&ubuntu).0, Mark::Letter('U'));

        // An unknown system built on Debian gets Debian's badge.
        let derived = os("TERMKER_UNAME=Linux 6.8.0\nNAME=Devuan\nID=devuan\nID_LIKE=debian\n");
        assert_eq!(
            mark_for(&derived),
            mark_for(&os("TERMKER_UNAME=Linux 6.1.0\nNAME=Debian\nID=debian\n"))
        );

        let unknown = os("TERMKER_UNAME=Linux 6.8.0\nNAME=Homemade\nID=homemade\n");
        assert_eq!(mark_for(&unknown).0, Mark::Penguin);
        assert_eq!(
            mark_for(&os("TERMKER_UNAME=Linux 5.10.0\n")).0,
            Mark::Penguin
        );
        assert_eq!(
            mark_for(&os("TERMKER_UNAME=Darwin 24.1.0\n")).0,
            Mark::CommandKey
        );
        assert_eq!(
            mark_for(&os("TERMKER_UNAME=OpenBSD 7.6\n")).0,
            Mark::Letter('O')
        );
        assert_eq!(
            mark_for(&os("TERMKER_UNAME=HP-UX B.11.31\n")).0,
            Mark::Letter('H')
        );
    }

    #[test]
    fn letters_stay_readable_on_light_and_dark_badges() {
        assert_eq!(ink_on(rgb_color(0xA80030)), Color32::WHITE);
        assert_ne!(ink_on(rgb_color(0xF2CA30)), Color32::WHITE);
    }
}
