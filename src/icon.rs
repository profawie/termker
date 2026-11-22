//! The termker logo: the window icon and the mark shown inside the app.
//!
//! `assets/icon/termker.svg` is the source drawing. The Windows program
//! carries `termker.ico` (see `build.rs`) and the macOS app carries
//! `termker.icns` (see `build.sh`); the PNG here covers the running window.

use eframe::egui;

/// Full square logo, as Windows and Linux show it.
const LOGO_PNG: &[u8] = include_bytes!("../assets/icon/termker-256.png");

/// macOS draws icons smaller than their canvas, with a clear margin around.
#[cfg(target_os = "macos")]
const DOCK_PNG: &[u8] = include_bytes!("../assets/icon/termker-macos-512.png");

/// Icon of the window, the taskbar button and the macOS Dock.
pub fn window_icon() -> egui::IconData {
    #[cfg(target_os = "macos")]
    let png = DOCK_PNG;
    #[cfg(not(target_os = "macos"))]
    let png = LOGO_PNG;
    eframe::icon_data::from_png_bytes(png).expect("the bundled icon is a valid PNG")
}

/// The logo as a texture, for drawing it in the app.
pub fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let icon =
        eframe::icon_data::from_png_bytes(LOGO_PNG).expect("the bundled logo is a valid PNG");
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [icon.width as usize, icon.height as usize],
        &icon.rgba,
    );
    ctx.load_texture("termker-logo", image, egui::TextureOptions::LINEAR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_icon_is_square_and_complete() {
        let icon = window_icon();
        assert_eq!(icon.width, icon.height);
        assert!(icon.width >= 256);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }
}
