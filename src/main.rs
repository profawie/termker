// Do not open a console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The termker desktop app: windows, pages and themes. The work behind
//! them lives in the crates under `crates/`:
//!
//! - `termker-core`: saved hosts and settings, keychain, saved passwords
//! - `termker-ssh`: SSH connections, jump hosts, port forwarding, SFTP
//! - `termker-terminal`: terminal emulator and shell sessions

mod app;
mod icon;
mod libs;
mod theme;
mod ui;

// Short names for the library crates, used throughout the app.
use termker_core::{keychain, migrate, model, secrets, store};
use termker_ssh::{self as ssh, sftp};
use termker_terminal::{emulator as terminal, session};

pub use termker_core::APP_NAME;
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_icon(icon::window_icon())
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([860.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
