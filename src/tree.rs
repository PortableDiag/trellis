//! Left-hand tree panel: the hierarchical spine of the document.

use crate::model::{Document, NodeId};

/// Actions the tree UI requests; applied after the immutable walk so we never
/// mutate the arena while iterating it.
pub enum TreeAction {
    Select(NodeId),
    AddChild(NodeId),
    AddRoot,
    Remove(NodeId),
    Rename(NodeId, String),
    ToggleExpand(NodeId),
}

pub fn ui(ui: &mut egui::Ui, doc: &Document, selected: Option<NodeId>) -> Vec<TreeAction> {
    let mut actions = Vec::new();

    ui.horizontal(|ui| {
        ui.heading("Trellis");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("＋ node").on_hover_text("Add a root node").clicked() {
                actions.push(TreeAction::AddRoot);
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        let roots = doc.roots.clone();
        for root in roots {
            node_ui(ui, doc, root, selected, 0, &mut actions);
        }
    });

    actions
}

fn node_ui(
    ui: &mut egui::Ui,
    doc: &Document,
    id: NodeId,
    selected: Option<NodeId>,
    depth: usize,
    actions: &mut Vec<TreeAction>,
) {
    let Some(node) = doc.nodes.get(&id) else { return };
    let is_sel = selected == Some(id);

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 14.0);

        // Expand / collapse triangle (only when there are children).
        if node.children.is_empty() {
            ui.add_space(16.0);
        } else {
            let arrow = if node.expanded { "▾" } else { "▸" };
            if ui.small_button(arrow).clicked() {
                actions.push(TreeAction::ToggleExpand(id));
            }
        }

        let label = egui::SelectableLabel::new(is_sel, &node.title);
        let resp = ui.add(label);
        if resp.clicked() {
            actions.push(TreeAction::Select(id));
        }
        resp.context_menu(|ui| {
            if ui.button("Add child").clicked() {
                actions.push(TreeAction::AddChild(id));
                ui.close_menu();
            }
            if ui.button("Delete subtree").clicked() {
                actions.push(TreeAction::Remove(id));
                ui.close_menu();
            }
            ui.separator();
            let mut title = node.title.clone();
            ui.label("Rename:");
            if ui.text_edit_singleline(&mut title).changed() {
                actions.push(TreeAction::Rename(id, title));
            }
        });
    });

    if node.expanded {
        let children = node.children.clone();
        for child in children {
            node_ui(ui, doc, child, selected, depth + 1, actions);
        }
    }
}
