#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod doc;
mod input;
mod job;
mod langs;
mod output;
mod translate;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Traducteur de livres")
            .with_inner_size([860.0, 600.0])
            .with_min_inner_size([520.0, 420.0])
            .with_drag_and_drop(true)
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native("traducteur_livre", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
