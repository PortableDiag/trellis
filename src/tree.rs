//! Left-hand tree panel: the hierarchical spine of the document.

use crate::model::{Document, NodeId};

/// Actions the tree UI requests; applied by the app after the immutable walk so
/// we never mutate the arena while iterating it.
pub enum TreeAction {
    Select(NodeId),
    AddRoot,
    AddChild(NodeId),
    AddSibling(NodeId),
    Remove(NodeId),
    Rename(NodeId, String),
    ToggleExpand(NodeId),
    MoveUp(NodeId),
    MoveDown(NodeId),
    Indent(NodeId),
    Outdent(NodeId),
    SetColor(NodeId, Option<[u8; 3]>),
}

/// `renaming` holds the node currently being renamed inline and its edit buffer.
pub fn ui(
    ui: &mut egui::Ui,
    doc: &Document,
    selected: Option<NodeId>,
    renaming: &mut Option<(NodeId, String)>,
) -> Vec<TreeAction> {
    let mut actions = Vec::new();

    ui.horizontal(|ui| {
        ui.heading("Trellis");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("＋").on_hover_text("Add a root node").clicked() {
                actions.push(TreeAction::AddRoot);
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let roots = doc.roots.clone();
            for root in roots {
                node_ui(ui, doc, root, selected, renaming, 0, &mut actions);
            }
            ui.add_space(8.0);
        });

    actions
}

#[allow(clippy::too_many_arguments)]
fn node_ui(
    ui: &mut egui::Ui,
    doc: &Document,
    id: NodeId,
    selected: Option<NodeId>,
    renaming: &mut Option<(NodeId, String)>,
    depth: usize,
    actions: &mut Vec<TreeAction>,
) {
    let Some(node) = doc.nodes.get(&id) else { return };
    let is_sel = selected == Some(id);

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * 14.0);

        // Expand / collapse triangle (only when there are children).
        if node.children.is_empty() {
            ui.add_space(18.0);
        } else {
            let arrow = if node.expanded { "▾" } else { "▸" };
            if ui.add(egui::Button::new(arrow).frame(false)).clicked() {
                actions.push(TreeAction::ToggleExpand(id));
            }
        }

        // Colour dot.
        if let Some(c) = node.color {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(
                rect.center(),
                4.0,
                egui::Color32::from_rgb(c[0], c[1], c[2]),
            );
        }

        // Inline rename takes over the row for the node being edited.
        let editing_this = matches!(renaming, Some((rid, _)) if *rid == id);
        if editing_this {
            if let Some((_, buf)) = renaming.as_mut() {
                let resp = ui.add(
                    egui::TextEdit::singleline(buf)
                        .desired_width(f32::INFINITY)
                        .hint_text("node title"),
                );
                resp.request_focus();
                let commit = resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape))
                    || resp.lost_focus();
                if commit {
                    let text = buf.clone();
                    actions.push(TreeAction::Rename(id, text));
                    *renaming = None;
                }
            }
        } else {
            let label = egui::SelectableLabel::new(is_sel, &node.title);
            let resp = ui.add(label);
            if resp.clicked() {
                actions.push(TreeAction::Select(id));
            }
            if resp.double_clicked() {
                *renaming = Some((id, node.title.clone()));
            }
            resp.context_menu(|ui| {
                if ui.button("✏  Rename").clicked() {
                    *renaming = Some((id, node.title.clone()));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("＋  Add child").clicked() {
                    actions.push(TreeAction::AddChild(id));
                    ui.close_menu();
                }
                if ui.button("＋  Add sibling").clicked() {
                    actions.push(TreeAction::AddSibling(id));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("▲  Move up").clicked() {
                    actions.push(TreeAction::MoveUp(id));
                    ui.close_menu();
                }
                if ui.button("▼  Move down").clicked() {
                    actions.push(TreeAction::MoveDown(id));
                    ui.close_menu();
                }
                if ui.button("→  Indent").clicked() {
                    actions.push(TreeAction::Indent(id));
                    ui.close_menu();
                }
                if ui.button("←  Outdent").clicked() {
                    actions.push(TreeAction::Outdent(id));
                    ui.close_menu();
                }
                ui.separator();
                ui.menu_button("🎨  Colour", |ui| {
                    let swatches: [(&str, Option<[u8; 3]>); 6] = [
                        ("None", None),
                        ("Red", Some([0xef, 0x44, 0x44])),
                        ("Amber", Some([0xf5, 0x9e, 0x0b])),
                        ("Green", Some([0x22, 0xc5, 0x5e])),
                        ("Blue", Some([0x3b, 0x82, 0xf6])),
                        ("Violet", Some([0x8b, 0x5c, 0xf6])),
                    ];
                    for (name, col) in swatches {
                        if ui.button(name).clicked() {
                            actions.push(TreeAction::SetColor(id, col));
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                if ui.button("🗑  Delete subtree").clicked() {
                    actions.push(TreeAction::Remove(id));
                    ui.close_menu();
                }
            });
        }
    });

    if node.expanded {
        let children = node.children.clone();
        for child in children {
            node_ui(ui, doc, child, selected, renaming, depth + 1, actions);
        }
    }
}
