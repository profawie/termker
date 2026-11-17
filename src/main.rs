// Jangan buka tetingkap konsol di Windows untuk binaan release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod libs;
mod model;
mod store;
mod theme;
mod ui;

pub const APP_NAME: &str = "rterminalfy";
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
