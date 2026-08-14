//! Left-hand tree panel: the hierarchical spine of the document.

use crate::model::{Document, NodeId};

/// Root-to-node breadcrumb of titles, e.g. `PROJECT › AREA › ITEM`.
pub(crate) fn node_path(doc: &Document, id: NodeId) -> String {
    doc.node_path(id)
}

/// Actions the tree UI requests; applied by the app after the immutable walk so
/// we never mutate the arena while iterating it.
/// Text/data formats a single basket can be exported to (the visual PDF/PNG are
/// handled separately by the canvas screenshot path).
#[derive(Clone, Copy)]
pub enum BasketFormat {
    Markdown,
    Html,
    Json,
}

pub enum TreeAction {
    Select(NodeId),
    /// Export one basket (node) to a text/data file. `bool` = include subnodes.
    ExportBasket(NodeId, BasketFormat, bool),
    /// Export a basket as a WYSIWYG PDF (overview page + per-card readable pages).
    ExportBasketPdf(NodeId),
    /// Export a basket as a single overview PNG image.
    ExportBasketPng(NodeId),
    /// Import a basket JSON file as a child of this node.
    ImportBasket(NodeId),
    AddRoot,
    AddChild(NodeId),
    AddSibling(NodeId),
    Remove(NodeId),
    Rename(NodeId, String),
    ToggleExpand(NodeId),
    /// Expand (`true`) or collapse (`false`) a node and its whole subtree.
    SetSubtreeExpanded(NodeId, bool),
    /// Expand or collapse *every* root and everything under it.
    SetAllExpanded(bool),
    MoveUp(NodeId),
    MoveDown(NodeId),
    MoveToTop(NodeId),
    MoveToBottom(NodeId),
    Indent(NodeId),
    Outdent(NodeId),
    SetColor(NodeId, Option<[u8; 3]>),
    /// Set (or clear, with `None`) a node's basket background color.
    SetBg(NodeId, Option<[u8; 3]>),
    /// Drag & drop: put `moved` before/after `target` (adopting its parent).
    Reorder { moved: NodeId, target: NodeId, before: bool },
    /// Toggle reorder mode (nodes draggable) on/off.
    ToggleReorder,
    /// Run the plugin at this index against this node.
    RunPlugin(NodeId, usize),
}

/// `renaming` holds the node currently being renamed inline and its edit buffer.
pub fn ui(
    ui: &mut egui::Ui,
    doc: &Document,
    selected: Option<NodeId>,
    renaming: &mut Option<(NodeId, String)>,
    reorder_mode: bool,
    scroll_to: Option<NodeId>,
    // Approved plugins that asked to appear here, as (index, title). Only
    // approved ones are passed in — an unapproved plugin must not be one click
    // away from running.
    node_plugins: &[(usize, String)],
) -> Vec<TreeAction> {
    let mut actions = Vec::new();

    ui.horizontal(|ui| {
        ui.heading("Trellis");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("+").on_hover_text("Add a root node").clicked() {
                actions.push(TreeAction::AddRoot);
            }
            if ui
                .selectable_label(reorder_mode, "Reorder")
                .on_hover_text("Reorder mode: drag nodes to move them")
                .clicked()
            {
                actions.push(TreeAction::ToggleReorder);
            }
            // Right-click → Expand all / Collapse all has always worked on one
            // node, including a root. Doing it to the whole tree was the missing
            // part: with 38 top-level projects, folding the tree was 38
            // right-clicks. Small icon buttons — the header already carries two.
            // Laid out right-to-left, so expand is pushed first to draw second:
            // the pair reads ⊟ ⊞ on screen, collapse before expand.
            if ui.small_button("⊞").on_hover_text("Expand the whole tree").clicked() {
                actions.push(TreeAction::SetAllExpanded(true));
            }
            if ui.small_button("⊟").on_hover_text("Collapse the whole tree").clicked() {
                actions.push(TreeAction::SetAllExpanded(false));
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let roots = doc.roots.clone();
            for root in roots {
                node_ui(ui, doc, root, selected, renaming, reorder_mode, scroll_to, 0, node_plugins, &mut actions);
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
    reorder_mode: bool,
    scroll_to: Option<NodeId>,
    depth: usize,
    node_plugins: &[(usize, String)],
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

        // Color dot.
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
                // Grab focus only on the first frame of editing this node.
                // Requesting every frame would hog focus from card editors and
                // prevent the field from ever losing focus (so it couldn't exit).
                let focus_key = egui::Id::new("trellis_rename_focused");
                let focused = ui.memory(|m| m.data.get_temp::<NodeId>(focus_key));
                if focused != Some(id) {
                    resp.request_focus();
                    ui.memory_mut(|m| m.data.insert_temp(focus_key, id));
                }
                let clear_focus = |ui: &egui::Ui| {
                    ui.memory_mut(|m| m.data.remove::<NodeId>(focus_key));
                };
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    *renaming = None; // Escape cancels: discard the edited buffer.
                    clear_focus(ui);
                } else if resp.lost_focus() {
                    // Enter or clicking away commits the new title.
                    actions.push(TreeAction::Rename(id, buf.clone()));
                    *renaming = None;
                    clear_focus(ui);
                }
            }
        } else {
            // In reorder mode the row is a drag source (draggable, grab cursor);
            // otherwise it's a plain selectable label so a click just selects.
            let resp = if reorder_mode {
                let egui::InnerResponse { inner: resp, response: drag } = ui.dnd_drag_source(
                    ui.make_persistent_id(("tree_drag", id)),
                    id,
                    |ui| ui.add(egui::SelectableLabel::new(is_sel, &node.title)),
                );
                // When another node is dragged over this row, show where it will
                // land and perform the move on release.
                if drag.dnd_hover_payload::<NodeId>().is_some() {
                    let rect = drag.rect;
                    let before = ui
                        .input(|i| i.pointer.hover_pos())
                        .map_or(true, |p| p.y < rect.center().y);
                    let y = if before { rect.top() } else { rect.bottom() };
                    ui.painter().hline(
                        rect.x_range(),
                        y,
                        egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                    );
                    if let Some(moved) = drag.dnd_release_payload::<NodeId>() {
                        actions.push(TreeAction::Reorder { moved: *moved, target: id, before });
                    }
                }
                resp
            } else {
                ui.add(egui::SelectableLabel::new(is_sel, &node.title))
            };
            if scroll_to == Some(id) {
                resp.scroll_to_me(Some(egui::Align::Center));
            }
            if resp.clicked() {
                actions.push(TreeAction::Select(id));
            }
            if resp.double_clicked() {
                *renaming = Some((id, node.title.clone()));
            }
            resp.context_menu(|ui| {
                if !node_plugins.is_empty() {
                    ui.menu_button("Plugins", |ui| {
                        for (idx, title) in node_plugins {
                            if ui.button(title).clicked() {
                                actions.push(TreeAction::RunPlugin(id, *idx));
                                ui.close_menu();
                            }
                        }
                    });
                    ui.separator();
                }
                if ui.button("Rename").clicked() {
                    *renaming = Some((id, node.title.clone()));
                    ui.close_menu();
                }
                ui.menu_button("Copy", |ui| {
                    if ui
                        .button("Node id")
                        .on_hover_text("The id agents use: /api/nodes/{id}")
                        .clicked()
                    {
                        crate::canvas::copy_both(ui, &id.to_string());
                        ui.close_menu();
                    }
                    if ui
                        .button("Node path")
                        .on_hover_text(node_path(doc, id))
                        .clicked()
                    {
                        crate::canvas::copy_both(ui, &node_path(doc, id));
                        ui.close_menu();
                    }
                });
                ui.separator();
                if ui.button("+  Add child").clicked() {
                    actions.push(TreeAction::AddChild(id));
                    ui.close_menu();
                }
                if ui.button("+  Add sibling").clicked() {
                    actions.push(TreeAction::AddSibling(id));
                    ui.close_menu();
                }
                if !node.children.is_empty() {
                    ui.separator();
                    if ui.button("▾  Expand all").on_hover_text("Open this node and every subnode under it").clicked() {
                        actions.push(TreeAction::SetSubtreeExpanded(id, true));
                        ui.close_menu();
                    }
                    if ui.button("▸  Collapse all").on_hover_text("Fold this whole branch away").clicked() {
                        actions.push(TreeAction::SetSubtreeExpanded(id, false));
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Move to top").clicked() {
                    actions.push(TreeAction::MoveToTop(id));
                    ui.close_menu();
                }
                if ui.button("▲  Move up").clicked() {
                    actions.push(TreeAction::MoveUp(id));
                    ui.close_menu();
                }
                if ui.button("▼  Move down").clicked() {
                    actions.push(TreeAction::MoveDown(id));
                    ui.close_menu();
                }
                if ui.button("Move to bottom").clicked() {
                    actions.push(TreeAction::MoveToBottom(id));
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
                ui.menu_button("Color", |ui| {
                    if ui.button("None").clicked() {
                        actions.push(TreeAction::SetColor(id, None));
                        ui.close_menu();
                    }
                    if let Some(col) = crate::canvas::swatch_grid(ui) {
                        actions.push(TreeAction::SetColor(id, Some(col)));
                        ui.close_menu();
                    }
                });
                ui.menu_button("Basket color", |ui| {
                    if ui.button("Default").clicked() {
                        actions.push(TreeAction::SetBg(id, None));
                        ui.close_menu();
                    }
                    if let Some(col) = crate::canvas::swatch_grid(ui) {
                        actions.push(TreeAction::SetBg(id, Some(col)));
                        ui.close_menu();
                    }
                });
                ui.separator();
                let export_menu = |ui: &mut egui::Ui, actions: &mut Vec<TreeAction>, subs: bool| {
                    if ui.button("Markdown (.md)").clicked() {
                        actions.push(TreeAction::ExportBasket(id, BasketFormat::Markdown, subs));
                        ui.close_menu();
                    }
                    if ui.button("HTML (.html)").clicked() {
                        actions.push(TreeAction::ExportBasket(id, BasketFormat::Html, subs));
                        ui.close_menu();
                    }
                    if ui.button("JSON (basket file)").clicked() {
                        actions.push(TreeAction::ExportBasket(id, BasketFormat::Json, subs));
                        ui.close_menu();
                    }
                };
                ui.menu_button("Export basket", |ui| {
                    if ui
                        .button("PDF (visual)")
                        .on_hover_text("Overview page + a readable page per card, with searchable text")
                        .clicked()
                    {
                        actions.push(TreeAction::ExportBasketPdf(id));
                        ui.close_menu();
                    }
                    if ui
                        .button("PNG (overview)")
                        .on_hover_text("One image of the whole basket as arranged")
                        .clicked()
                    {
                        actions.push(TreeAction::ExportBasketPng(id));
                        ui.close_menu();
                    }
                    ui.separator();
                    export_menu(ui, actions, false);
                })
                .response
                .on_hover_text("Share just this basket's cards");
                ui.menu_button("Export basket + subnodes", |ui| export_menu(ui, actions, true))
                    .response
                    .on_hover_text("Share this basket and everything nested under it");
                if ui
                    .button("Import basket…")
                    .on_hover_text("Add a basket JSON file as a child of this node")
                    .clicked()
                {
                    actions.push(TreeAction::ImportBasket(id));
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Delete subtree").clicked() {
                    actions.push(TreeAction::Remove(id));
                    ui.close_menu();
                }
            });
        }
    });

    if node.expanded {
        let children = node.children.clone();
        for child in children {
            node_ui(ui, doc, child, selected, renaming, reorder_mode, scroll_to, depth + 1, node_plugins, actions);
        }
    }
}
