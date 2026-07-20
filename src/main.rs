//! Trellis — a hierarchical, spatial note-taking app.
//!
//! A tree of nodes (the structure) where every node's body is a free-form
//! basket of draggable, editable cards (the spatial surface).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod model;
mod tree;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Trellis")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Trellis",
        options,
        Box::new(|cc| Ok(Box::new(app::TrellisApp::new(cc)))),
    )
}
