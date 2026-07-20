//! Application state and the eframe update loop that stitches the panels together.

use crate::canvas::{self, CanvasAction};
use crate::model::{Document, NodeId};
use crate::tree::{self, TreeAction};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct TrellisApp {
    doc: Document,
    selected: Option<NodeId>,
    /// Per-node canvas pan offset, so each basket remembers its scroll.
    pans: HashMap<NodeId, egui::Vec2>,
    save_path: PathBuf,
    status: String,
}

impl TrellisApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let save_path = save_path();
        let doc = std::fs::read_to_string(&save_path)
            .ok()
            .and_then(|s| ron::from_str::<Document>(&s).ok())
            .unwrap_or_default();
        let selected = doc.roots.first().copied();

        // A slightly warmer default look.
        cc.egui_ctx.style_mut(|s| {
            s.visuals.window_rounding = 8.0.into();
        });

        Self {
            doc,
            selected,
            pans: HashMap::new(),
            save_path,
            status: String::new(),
        }
    }

    fn save(&mut self) {
        match ron::ser::to_string_pretty(&self.doc, ron::ser::PrettyConfig::default()) {
            Ok(s) => {
                if let Some(dir) = self.save_path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::write(&self.save_path, s) {
                    Ok(_) => self.status = format!("Saved → {}", self.save_path.display()),
                    Err(e) => self.status = format!("Save failed: {e}"),
                }
            }
            Err(e) => self.status = format!("Serialize failed: {e}"),
        }
    }

    fn apply_tree(&mut self, actions: Vec<TreeAction>) {
        for a in actions {
            match a {
                TreeAction::Select(id) => self.selected = Some(id),
                TreeAction::AddRoot => {
                    let id = self.doc.add_node(None, "Untitled".to_string());
                    self.selected = Some(id);
                }
                TreeAction::AddChild(parent) => {
                    let id = self.doc.add_node(Some(parent), "Untitled".to_string());
                    if let Some(n) = self.doc.nodes.get_mut(&parent) {
                        n.expanded = true;
                    }
                    self.selected = Some(id);
                }
                TreeAction::Remove(id) => {
                    self.doc.remove_node(id);
                    if self.selected == Some(id) {
                        self.selected = self.doc.roots.first().copied();
                    }
                }
                TreeAction::Rename(id, title) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.title = title;
                    }
                }
                TreeAction::ToggleExpand(id) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.expanded = !n.expanded;
                    }
                }
            }
        }
    }

    fn apply_canvas(&mut self, node: NodeId, actions: Vec<CanvasAction>) {
        for a in actions {
            match a {
                CanvasAction::AddCard(pos) => {
                    self.doc.add_card(node, pos);
                }
                CanvasAction::MoveCard(cid, delta) => {
                    if let Some(n) = self.doc.nodes.get_mut(&node) {
                        if let Some(c) = n.cards.iter_mut().find(|c| c.id == cid) {
                            c.pos += delta;
                        }
                    }
                }
                CanvasAction::EditCard(cid, text) => {
                    if let Some(n) = self.doc.nodes.get_mut(&node) {
                        if let Some(c) = n.cards.iter_mut().find(|c| c.id == cid) {
                            c.text = text;
                        }
                    }
                }
                CanvasAction::RemoveCard(cid) => {
                    if let Some(n) = self.doc.nodes.get_mut(&node) {
                        n.cards.retain(|c| c.id != cid);
                    }
                }
            }
        }
    }
}

impl eframe::App for TrellisApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Ctrl+S to save.
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save();
        }

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("💾 Save (Ctrl+S)").clicked() {
                    self.save();
                }
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::SidePanel::left("tree")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                let actions = tree::ui(ui, &self.doc, self.selected);
                self.apply_tree(actions);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(sel) = self.selected {
                if let Some(node) = self.doc.nodes.get(&sel) {
                    let pan = self.pans.entry(sel).or_default();
                    let mut pan_local = *pan;
                    let actions = canvas::ui(ui, node, &mut pan_local);
                    *self.pans.get_mut(&sel).unwrap() = pan_local;
                    self.apply_canvas(sel, actions);
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("No node selected. Add one on the left to start a basket.");
                });
            }
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save();
    }
}

fn save_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "Trellis", "Trellis")
        .map(|d| d.data_dir().join("document.ron"))
        .unwrap_or_else(|| PathBuf::from("trellis-document.ron"))
}
