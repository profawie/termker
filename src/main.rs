// Do not open a console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The rterminalfy desktop app: windows, pages and themes. The work behind
//! them lives in the crates under `crates/`:
//!
//! - `rterminalfy-core`: saved hosts and settings, keychain, saved passwords
//! - `rterminalfy-ssh`: SSH connections, jump hosts, port forwarding, SFTP
//! - `rterminalfy-terminal`: terminal emulator and shell sessions

mod app;
mod libs;
mod theme;
mod ui;

// Short names for the library crates, used throughout the app.
use rterminalfy_core::{keychain, model, secrets, store};
use rterminalfy_ssh::{self as ssh, sftp};
use rterminalfy_terminal::{emulator as terminal, session};

pub use rterminalfy_core::APP_NAME;
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(APP_NAME)
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
