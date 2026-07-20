//! Trellis — a hierarchical, spatial note-taking app.
//!
//! A tree of nodes (the structure) where every node's body is a free-form
//! basket of draggable, editable cards (the spatial surface).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod images;
mod model;
mod tree;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Trellis")
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([720.0, 460.0])
            .with_icon(load_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Trellis",
        options,
        Box::new(|cc| Ok(Box::new(app::TrellisApp::new(cc)))),
    )
}

/// The window/taskbar icon, baked into the binary at compile time.
fn load_icon() -> egui::IconData {
    let png = include_bytes!("../assets/icon.png");
    let img = image::load_from_memory(png)
        .expect("decode embedded icon")
        .to_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}
