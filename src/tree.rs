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
    /// A pattern for the basket canvas, or `None` for the flat color.
    SetBgFill(NodeId, Option<crate::model::Fill>),
    /// This basket's card style by name, or `None` to follow the app theme.
    SetStyle(NodeId, Option<String>),
    /// Drag & drop: put `moved` before/after `target` (adopting its parent).
    Reorder { moved: NodeId, target: NodeId, before: bool },
    /// Toggle reorder mode (nodes draggable) on/off.
    ToggleReorder,
    /// Run the plugin at this index against this node.
    RunPlugin(NodeId, usize),
    /// Open this node's child baskets as a compressed-workspace cube — the
    /// range picker follows in the app, since the tree row has no room for one.
    OpenAsCube(NodeId),
}

/// `renaming` holds the node currently being renamed inline and its edit buffer.
/// How the **root** nodes are ordered in the tree.
///
/// **Roots only, and a view rather than a rewrite.** The point of this is that a
/// new project lands where it belongs instead of at the bottom waiting to be
/// dragged into place — which a one-shot sort would fix once and then let rot
/// again on the next project. A view mode keeps fixing it, and it leaves the
/// document's own order alone so nothing else that reads the file is surprised.
///
/// Sub-nodes keep the order they were given: inside a project, order is usually
/// meaning (a checklist of phases, a journal by month), and sorting that would
/// destroy information rather than tidy it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TreeSort {
    /// The document's own order — drag, indent and move up/down decide it.
    #[default]
    Manual,
    NameAsc,
    NameDesc,
    /// Most recently touched first, using the `touched` stamp a basket gets when
    /// any card in it changes.
    Recent,
    /// Most open tasks first — "where is the work" answered at a glance.
    Tasks,
}

impl TreeSort {
    pub const ALL: [(TreeSort, &'static str); 5] = [
        (TreeSort::Manual, "Manual (drag to order)"),
        (TreeSort::NameAsc, "Name A → Z"),
        (TreeSort::NameDesc, "Name Z → A"),
        (TreeSort::Recent, "Recently changed"),
        (TreeSort::Tasks, "Open tasks"),
    ];

    pub fn key(self) -> &'static str {
        match self {
            TreeSort::Manual => "manual",
            TreeSort::NameAsc => "name",
            TreeSort::NameDesc => "name_desc",
            TreeSort::Recent => "recent",
            TreeSort::Tasks => "tasks",
        }
    }

    pub fn from_key(s: &str) -> TreeSort {
        match s {
            "name" => TreeSort::NameAsc,
            "name_desc" => TreeSort::NameDesc,
            "recent" => TreeSort::Recent,
            "tasks" => TreeSort::Tasks,
            _ => TreeSort::Manual,
        }
    }
}

/// The roots in display order.
///
/// Case-insensitive by name, because a raw byte sort files every lowercase title
/// after every uppercase one — which put a handful of domain-named projects in a
/// clump at the end of this document and looked like a bug. Ties fall back to
/// the document's order so the list never shuffles between frames.
pub fn sorted_roots(doc: &Document, sort: TreeSort) -> Vec<NodeId> {
    let mut roots = doc.roots.clone();
    if sort == TreeSort::Manual {
        return roots;
    }
    let title = |id: &NodeId| {
        doc.nodes.get(id).map(|n| n.title.to_lowercase()).unwrap_or_default()
    };
    match sort {
        TreeSort::NameAsc => roots.sort_by_key(title),
        TreeSort::NameDesc => {
            roots.sort_by_key(title);
            roots.reverse();
        }
        TreeSort::Recent => {
            // Newest first; a basket that has never been touched sorts last
            // rather than first, which is what "recently changed" means.
            roots.sort_by_key(|id| {
                std::cmp::Reverse(doc.nodes.get(id).and_then(|n| n.touched).unwrap_or(0))
            });
        }
        TreeSort::Tasks => {
            let mut open: std::collections::HashMap<NodeId, usize> =
                std::collections::HashMap::new();
            for t in doc.tasks() {
                if !t.done {
                    *open.entry(t.root).or_default() += 1;
                }
            }
            roots.sort_by_key(|id| std::cmp::Reverse(open.get(id).copied().unwrap_or(0)));
        }
        TreeSort::Manual => {}
    }
    roots
}

/// A tree row's label, truncated to the width the panel actually has.
///
/// **A tree row must never dictate the panel's width.** `SelectableLabel` lays a
/// title out at its natural width, the tree's `ScrollArea` is vertical-only, so a
/// long title has nowhere to go but outward — and because egui clamps a resizable
/// `SidePanel` to its content's minimum, the sidebar then *snaps back* when you
/// try to drag it in. Reported exactly that way: "the sidebar got all wide and
/// won't move back." Two 57–59 character node titles in one project did it.
///
/// The title is not lost: the full text is on the row's tooltip.
fn row_label(ui: &egui::Ui, title: &str, selected: bool) -> egui::SelectableLabel {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = ui.visuals().text_color();
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), font, color);
    // `available_width` is what is left after the indent, so a deeply nested row
    // truncates sooner — which is right: it has less room.
    job.wrap.max_width = ui.available_width();
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    egui::SelectableLabel::new(selected, ui.fonts(|f| f.layout_job(job)))
}

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
    sort: TreeSort,
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
            // Folding the whole tree lives in **View**, not here. As buttons they
            // sat directly under the menu bar, in the path of a pointer heading
            // for Edit or View, and got clicked by accident — an expensive
            // misclick, because it moves every node in the document at once.
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let roots = sorted_roots(doc, sort);
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

        // Expand / collapse triangle (only when there are children). Both
        // branches must occupy the SAME fixed-size widget slot: the old childless
        // branch was a raw 18px spacer while the arrow was a button sized by its
        // glyph plus padding — and a spacer is not a widget, so the arrow rows
        // also picked up an item-spacing gap the spacer rows did not. Net effect:
        // the color dots of arrow rows sat ~6px right of the others, a column
        // that never quite lined up (operator's screenshot, 2026-08-26).
        let arrow_slot = egui::vec2(18.0, ui.spacing().interact_size.y);
        if node.children.is_empty() {
            ui.allocate_exact_size(arrow_slot, egui::Sense::hover());
        } else {
            let arrow = if node.expanded { "▾" } else { "▸" };
            if ui.add_sized(arrow_slot, egui::Button::new(arrow).frame(false)).clicked() {
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
                    |ui| ui.add(row_label(ui, &node.title, is_sel)),
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
                ui.add(row_label(ui, &node.title, is_sel))
            };
            // Truncation hides nothing: the whole title is one hover away.
            let resp = resp.on_hover_text(&node.title);
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
                // The id, readable in place — the same line the card menu grew
                // in v0.153.1, for the other id space.
                //
                // **Bare, because a basket's id is bare.** The rule across both
                // menus is *what goes inside the brackets*: a card links as
                // `[[#1391]]` so its menu says `#1391`, a basket links as
                // `[[42]]` so its menu says `42`. v0.163.2 wrote `node 42` to
                // keep a bare `#42` from reading as card 42 — right worry, wrong
                // fix, since the `#` is exactly the part a basket does not have.
                // The prefix was a label that appears nowhere else in the app.
                // Full size and full contrast, per v0.153.2.
                ui.label(egui::RichText::new(format!("{id}")).monospace().strong())
                    .on_hover_text(format!(
                        "[[{id}]] links to this basket from a card  ·  /api/nodes/{id} \
                         is how an agent reaches it"
                    ));
                ui.separator();
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
                if !node.children.is_empty()
                    && ui
                        .button("Open as cube…")
                        .on_hover_text(
                            "Align a range of this basket's child baskets along z — each \
                             child a slice, oldest deepest — in a temporary Cube view. \
                             Nothing is created or copied: the slices are live views of \
                             the real cards.",
                        )
                        .clicked()
                {
                    actions.push(TreeAction::OpenAsCube(id));
                    ui.close_menu();
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
                ui.menu_button("Basket pattern", |ui| {
                    if let Some(f) = crate::canvas::fill_menu(ui, node.bg_fill.as_ref()) {
                        actions.push(TreeAction::SetBgFill(id, f));
                    }
                });
                ui.menu_button("Basket style", |ui| {
                    ui.label(
                        egui::RichText::new("Overrides the app theme, for this basket only")
                            .small()
                            .weak(),
                    );
                    let cur = node.style.as_deref();
                    if ui.radio(cur.is_none(), "Follow the app theme").clicked() {
                        actions.push(TreeAction::SetStyle(id, None));
                        ui.close_menu();
                    }
                    for (st, label) in crate::canvas::CardStyle::ALL {
                        if ui.radio(cur == Some(st.key()), label).clicked() {
                            actions.push(TreeAction::SetStyle(id, Some(st.key().to_string())));
                            ui.close_menu();
                        }
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

#[cfg(test)]
mod tests {

    /// Sorting governs the **roots** and nothing else: inside a project, order is
    /// usually meaning — phases, months — and sorting it would destroy
    /// information rather than tidy it.
    #[test]
    fn sorting_orders_roots_and_leaves_children_alone() {
        use crate::model::Document;
        let mut doc = Document::empty();
        let zebra = doc.add_node(None, "Zebra".into());
        let apple = doc.add_node(None, "apple.com".into());
        let mango = doc.add_node(None, "Mango".into());
        // Children, deliberately not alphabetical: this is the case that must
        // survive untouched.
        let c_z = doc.add_node(Some(zebra), "zzz first".into());
        let c_a = doc.add_node(Some(zebra), "aaa second".into());

        let manual = super::sorted_roots(&doc, super::TreeSort::Manual);
        assert_eq!(manual, vec![zebra, apple, mango], "manual is the document's order");

        // Case-insensitive: a byte sort files every lowercase title after every
        // uppercase one, which clumped the domain-named projects at the end.
        let by_name = super::sorted_roots(&doc, super::TreeSort::NameAsc);
        assert_eq!(by_name, vec![apple, mango, zebra], "apple.com sorts with the a's");
        assert_eq!(
            super::sorted_roots(&doc, super::TreeSort::NameDesc),
            vec![zebra, mango, apple]
        );

        // The children are untouched by any of it.
        assert_eq!(doc.nodes.get(&zebra).unwrap().children, vec![c_z, c_a]);
    }

    /// Recently changed puts a basket that has never been touched **last**, not
    /// first — "recently changed" has to mean what it says or the order is noise.
    #[test]
    fn recent_puts_the_never_touched_at_the_bottom() {
        use crate::model::Document;
        let mut doc = Document::empty();
        let old = doc.add_node(None, "Old".into());
        let fresh = doc.add_node(None, "Fresh".into());
        let never = doc.add_node(None, "Never".into());
        doc.nodes.get_mut(&old).unwrap().touched = Some(1_000);
        doc.nodes.get_mut(&fresh).unwrap().touched = Some(2_000);
        assert_eq!(
            super::sorted_roots(&doc, super::TreeSort::Recent),
            vec![fresh, old, never]
        );
    }
}
