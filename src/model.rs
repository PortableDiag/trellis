//! Core data model for Trellis, plus all document operations, HTML export and
//! Markdown/HTML import.
//!
//! A document is a *tree of nodes* (borrowed from the outliner world) where
//! every node's body is a *basket*: a free-form 2-D surface holding draggable
//! cards. Structure lives in the tree; spatial thinking lives in the basket.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type NodeId = u64;
pub type CardId = u64;
pub type GroupId = u64;

/// A named container that a set of cards belong to (via [`Card::group`]). Drawn
/// as a box around its members; dragging its header moves the whole group.
#[derive(Clone, Serialize, Deserialize)]
pub struct CardGroup {
    pub id: GroupId,
    pub title: String,
    pub color: [u8; 3],
}

/// One line of a checklist card.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub done: bool,
    pub text: String,
}

/// What a card holds. `Text`/`Code` use the card's `body` string; the others
/// carry their own data.
#[derive(Clone, Serialize, Deserialize)]
pub enum CardKind {
    /// `body` is CommonMark markdown, rendered live.
    Text,
    /// `body` is source code; `lang` selects syntax highlighting.
    Code { lang: String },
    Checklist { items: Vec<ChecklistItem> },
    /// Image bytes embedded directly in the document for portability.
    Image { data: Vec<u8>, name: String },
}

impl CardKind {
    pub fn label(&self) -> &'static str {
        match self {
            CardKind::Text => "Text",
            CardKind::Code { .. } => "Code",
            CardKind::Checklist { .. } => "Checklist",
            CardKind::Image { .. } => "Image",
        }
    }
}

/// A single draggable, resizable card on a node's basket canvas.
#[derive(Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: CardId,
    /// Top-left position in canvas coordinates (independent of pan).
    pub pos: egui::Pos2,
    pub size: egui::Vec2,
    pub title: String,
    /// Markdown / code text. Unused by image and checklist cards.
    pub body: String,
    /// RGB accent used for the card's title bar.
    pub color: [u8; 3],
    pub kind: CardKind,
    /// Membership in a labeled group container. `None` = ungrouped.
    #[serde(default)]
    pub group: Option<GroupId>,
    /// Dock parent: this card sticks to `docked_to` and moves with it. `None` =
    /// free-floating.
    #[serde(default)]
    pub docked_to: Option<CardId>,
    /// Runtime-only: whether the card is in edit mode. Never persisted.
    #[serde(skip)]
    pub editing: bool,
}

impl Card {
    pub fn new(id: CardId, pos: egui::Pos2, kind: CardKind) -> Self {
        let editing = matches!(kind, CardKind::Text | CardKind::Code { .. });
        Self {
            id,
            pos,
            size: egui::vec2(240.0, 160.0),
            title: String::new(),
            body: String::new(),
            color: [0x3b, 0x82, 0xf6],
            kind,
            group: None,
            docked_to: None,
            editing,
        }
    }
}

/// A node in the tree. Its `cards` form the basket shown when it is selected.
#[derive(Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub title: String,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub cards: Vec<Card>,
    /// Group containers for this basket. Membership lives on [`Card::group`].
    #[serde(default)]
    pub groups: Vec<CardGroup>,
    #[serde(default = "default_true")]
    pub expanded: bool,
    /// Optional per-node tag color shown as a dot in the tree.
    #[serde(default)]
    pub color: Option<[u8; 3]>,
}

fn default_true() -> bool {
    true
}

/// Font used by the PDF/image exporters (also embedded in the PDF).
const EXPORT_FONT: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

/// One laid-out line for the PDF/image exporters. `size` is a point size; an
/// empty `text` is a vertical spacer.
struct ExportLine {
    text: String,
    size: f32,
}

/// Width of `s` in the same units as `size_px`, using the font's advances.
fn text_width(font: &ab_glyph::FontRef, size_px: f32, s: &str) -> f32 {
    use ab_glyph::{Font, PxScale, ScaleFont};
    let scaled = font.as_scaled(PxScale::from(size_px));
    let mut w = 0.0;
    let mut last = None;
    for c in s.chars() {
        let g = scaled.glyph_id(c);
        if let Some(l) = last {
            w += scaled.kern(l, g);
        }
        w += scaled.h_advance(g);
        last = Some(g);
    }
    w
}

/// Greedy word-wrap `text` to `max_w` (same units as `size_px`), preserving the
/// text's own newlines as hard breaks.
fn wrap_text(font: &ab_glyph::FontRef, size_px: f32, text: &str, max_w: f32) -> Vec<String> {
    let space = text_width(font, size_px, " ");
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        let mut cur_w = 0.0;
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let ww = text_width(font, size_px, word);
            if !cur.is_empty() && cur_w + space + ww > max_w {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0.0;
            }
            if !cur.is_empty() {
                cur.push(' ');
                cur_w += space;
            }
            cur.push_str(word);
            cur_w += ww;
        }
        lines.push(cur);
    }
    lines
}

/// Rasterize `text` onto `img` with its baseline at `baseline`, black on white.
fn draw_text(
    img: &mut image::RgbaImage,
    font: &ab_glyph::FontRef,
    size_px: f32,
    x0: f32,
    baseline: f32,
    text: &str,
) {
    use ab_glyph::{Font, PxScale, ScaleFont};
    let scale = PxScale::from(size_px);
    let scaled = font.as_scaled(scale);
    let (w, h) = (img.width(), img.height());
    let mut x = x0;
    let mut last = None;
    for c in text.chars() {
        let gid = scaled.glyph_id(c);
        if let Some(l) = last {
            x += scaled.kern(l, gid);
        }
        let glyph = gid.with_scale_and_position(scale, ab_glyph::point(x, baseline));
        if let Some(og) = font.outline_glyph(glyph) {
            let bb = og.px_bounds();
            og.draw(|gx, gy, cov| {
                let px = bb.min.x + gx as f32;
                let py = bb.min.y + gy as f32;
                if px >= 0.0 && py >= 0.0 && (px as u32) < w && (py as u32) < h {
                    let a = (cov * 255.0) as u32;
                    let p = img.get_pixel_mut(px as u32, py as u32);
                    // Black text: scale existing (white) channels down by coverage.
                    p[0] = ((p[0] as u32 * (255 - a)) / 255) as u8;
                    p[1] = ((p[1] as u32 * (255 - a)) / 255) as u8;
                    p[2] = ((p[2] as u32 * (255 - a)) / 255) as u8;
                }
            });
        }
        x += scaled.h_advance(gid);
        last = Some(gid);
    }
}

/// Drop group containers that no longer have any member cards.
fn prune_groups(n: &mut Node) {
    let used: std::collections::HashSet<GroupId> = n.cards.iter().filter_map(|c| c.group).collect();
    n.groups.retain(|g| used.contains(&g.id));
}

/// The whole document: an arena of nodes plus ordered roots and id counters.
#[derive(Serialize, Deserialize)]
pub struct Document {
    pub nodes: HashMap<NodeId, Node>,
    pub roots: Vec<NodeId>,
    next_node_id: NodeId,
    next_card_id: CardId,
    #[serde(default = "default_next_id")]
    next_group_id: GroupId,
}

fn default_next_id() -> GroupId {
    1
}

impl Default for Document {
    fn default() -> Self {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let root = doc.add_node(None, "Welcome to Trellis".to_string());
        if let Some(id) = doc.add_card(root, egui::pos2(60.0, 60.0), CardKind::Text) {
            if let Some(c) = doc.card_mut(root, id) {
                c.title = "Read me".to_string();
                c.body = "# The tree *and* the weave\n\nThe **left panel** is a hierarchy of \
                    nodes. Every node opens here as a **basket** — a free canvas of cards.\n\n\
                    - Double-click empty space to drop a text card\n\
                    - Right-click the canvas for other card types\n\
                    - Drag a card's title bar to move it, the corner to resize\n\n\
                    ```rust\nfn main() { println!(\"code cards are highlighted\"); }\n```"
                    .to_string();
                c.size = egui::vec2(360.0, 260.0);
                c.editing = false;
            }
        }
        doc
    }
}

impl Document {
    /// An empty document with no nodes. Unlike [`Document::default`], which seeds
    /// a welcome node, this is the blank slate importers build onto.
    pub fn empty() -> Self {
        Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        }
    }

    // --- lookup helpers -----------------------------------------------------

    pub fn card_mut(&mut self, node: NodeId, card: CardId) -> Option<&mut Card> {
        self.nodes
            .get_mut(&node)?
            .cards
            .iter_mut()
            .find(|c| c.id == card)
    }

    /// The ordered sibling list a node lives in (its parent's children, or the
    /// roots for a top-level node).
    fn sibling_list_mut(&mut self, id: NodeId) -> Option<&mut Vec<NodeId>> {
        match self.nodes.get(&id)?.parent {
            Some(p) => self.nodes.get_mut(&p).map(|n| &mut n.children),
            None => Some(&mut self.roots),
        }
    }

    // --- structural edits ---------------------------------------------------

    pub fn add_node(&mut self, parent: Option<NodeId>, title: String) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.insert(
            id,
            Node {
                id,
                title,
                parent,
                children: Vec::new(),
                cards: Vec::new(),
                groups: Vec::new(),
                expanded: true,
                color: None,
            },
        );
        match parent {
            Some(p) => {
                if let Some(pn) = self.nodes.get_mut(&p) {
                    pn.children.push(id);
                }
            }
            None => self.roots.push(id),
        }
        id
    }

    /// Add a sibling immediately after `id`, in the same list.
    pub fn add_sibling(&mut self, id: NodeId, title: String) -> NodeId {
        let parent = self.nodes.get(&id).and_then(|n| n.parent);
        let new_id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.insert(
            new_id,
            Node {
                id: new_id,
                title,
                parent,
                children: Vec::new(),
                cards: Vec::new(),
                groups: Vec::new(),
                expanded: true,
                color: None,
            },
        );
        if let Some(list) = self.sibling_list_mut(id) {
            let idx = list.iter().position(|x| *x == id).map_or(list.len(), |i| i + 1);
            list.insert(idx, new_id);
        }
        new_id
    }

    pub fn add_card(&mut self, node: NodeId, pos: egui::Pos2, kind: CardKind) -> Option<CardId> {
        let id = self.next_card_id;
        let n = self.nodes.get_mut(&node)?;
        n.cards.push(Card::new(id, pos, kind));
        self.next_card_id += 1;
        Some(id)
    }

    pub fn duplicate_card(&mut self, node: NodeId, card: CardId) -> Option<CardId> {
        let n = self.nodes.get_mut(&node)?;
        let src = n.cards.iter().find(|c| c.id == card)?.clone();
        let id = self.next_card_id;
        self.next_card_id += 1;
        let mut copy = src;
        copy.id = id;
        copy.pos += egui::vec2(24.0, 24.0);
        n.cards.push(copy);
        Some(id)
    }

    /// Add a copy of `template` (a card from anywhere) to `node`, with a fresh
    /// id and the given position. Used to paste a copied card into a basket.
    pub fn add_card_from(&mut self, node: NodeId, template: &Card, pos: egui::Pos2) -> Option<CardId> {
        let id = self.next_card_id;
        let n = self.nodes.get_mut(&node)?;
        let mut card = template.clone();
        card.id = id;
        card.pos = pos;
        card.editing = false;
        n.cards.push(card);
        self.next_card_id += 1;
        Some(id)
    }

    pub fn remove_card(&mut self, node: NodeId, card: CardId) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.cards.retain(|c| c.id != card);
            // Detach anything that was docked to the removed card.
            for c in n.cards.iter_mut() {
                if c.docked_to == Some(card) {
                    c.docked_to = None;
                }
            }
            prune_groups(n);
        }
    }

    // --- groups -------------------------------------------------------------

    /// Put `cards` (2 or more) into a fresh group and return its id. No-op
    /// (returns `None`) if fewer than two of them exist in the node.
    pub fn group_cards(
        &mut self,
        node: NodeId,
        cards: &[CardId],
        title: String,
    ) -> Option<GroupId> {
        let gid = self.next_group_id.max(1);
        let n = self.nodes.get_mut(&node)?;
        let count = n.cards.iter().filter(|c| cards.contains(&c.id)).count();
        if count < 2 {
            return None;
        }
        for c in n.cards.iter_mut() {
            if cards.contains(&c.id) {
                c.group = Some(gid);
            }
        }
        n.groups.push(CardGroup { id: gid, title, color: [0x64, 0x74, 0x8b] });
        self.next_group_id = gid + 1;
        Some(gid)
    }

    pub fn ungroup(&mut self, node: NodeId, group: GroupId) {
        if let Some(n) = self.nodes.get_mut(&node) {
            for c in n.cards.iter_mut() {
                if c.group == Some(group) {
                    c.group = None;
                }
            }
            n.groups.retain(|g| g.id != group);
        }
    }

    pub fn set_group_title(&mut self, node: NodeId, group: GroupId, title: String) {
        if let Some(n) = self.nodes.get_mut(&node) {
            if let Some(g) = n.groups.iter_mut().find(|g| g.id == group) {
                g.title = title;
            }
        }
    }

    pub fn set_group_color(&mut self, node: NodeId, group: GroupId, color: [u8; 3]) {
        if let Some(n) = self.nodes.get_mut(&node) {
            if let Some(g) = n.groups.iter_mut().find(|g| g.id == group) {
                g.color = color;
            }
        }
    }

    /// Move every member of `group` (and anything docked to a member) by `delta`.
    pub fn move_group(&mut self, node: NodeId, group: GroupId, delta: egui::Vec2) {
        let members: Vec<CardId> = self
            .nodes
            .get(&node)
            .map(|n| n.cards.iter().filter(|c| c.group == Some(group)).map(|c| c.id).collect())
            .unwrap_or_default();
        let mut ids: std::collections::HashSet<CardId> = std::collections::HashSet::new();
        for m in members {
            ids.extend(self.dock_tree_ids(node, m));
        }
        if let Some(n) = self.nodes.get_mut(&node) {
            for c in n.cards.iter_mut() {
                if ids.contains(&c.id) {
                    c.pos += delta;
                }
            }
        }
    }

    // --- docking ------------------------------------------------------------

    /// `card` plus every card docked to it, transitively (its dock subtree).
    fn dock_tree_ids(&self, node: NodeId, root: CardId) -> Vec<CardId> {
        let mut ids = vec![root];
        if let Some(n) = self.nodes.get(&node) {
            loop {
                let mut added = false;
                for c in &n.cards {
                    if let Some(p) = c.docked_to {
                        if ids.contains(&p) && !ids.contains(&c.id) {
                            ids.push(c.id);
                            added = true;
                        }
                    }
                }
                if !added {
                    break;
                }
            }
        }
        ids
    }

    /// Stick `child` onto `anchor`. Ignored if it would create a cycle (anchor
    /// is inside child's own dock subtree) or they're the same card.
    pub fn dock_card(&mut self, node: NodeId, child: CardId, anchor: CardId) {
        if child == anchor {
            return;
        }
        if self.dock_tree_ids(node, child).contains(&anchor) {
            return;
        }
        if let Some(c) = self.card_mut(node, child) {
            c.docked_to = Some(anchor);
        }
    }

    pub fn detach_card(&mut self, node: NodeId, card: CardId) {
        if let Some(c) = self.card_mut(node, card) {
            c.docked_to = None;
        }
    }

    /// Move `card` and its whole dock subtree by `delta`.
    pub fn move_card_tree(&mut self, node: NodeId, card: CardId, delta: egui::Vec2) {
        let ids = self.dock_tree_ids(node, card);
        if let Some(n) = self.nodes.get_mut(&node) {
            for c in n.cards.iter_mut() {
                if ids.contains(&c.id) {
                    c.pos += delta;
                }
            }
        }
    }

    /// Bring a card to the front by moving it to the end of the draw order.
    pub fn raise_card(&mut self, node: NodeId, card: CardId) {
        if let Some(n) = self.nodes.get_mut(&node) {
            if let Some(idx) = n.cards.iter().position(|c| c.id == card) {
                if idx + 1 != n.cards.len() {
                    let c = n.cards.remove(idx);
                    n.cards.push(c);
                }
            }
        }
    }

    /// Remove a node and its whole subtree; detaches it from its parent/roots.
    pub fn remove_node(&mut self, id: NodeId) {
        let parent = self.nodes.get(&id).and_then(|n| n.parent);
        match parent {
            Some(p) => {
                if let Some(pn) = self.nodes.get_mut(&p) {
                    pn.children.retain(|c| *c != id);
                }
            }
            None => self.roots.retain(|c| *c != id),
        }
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if let Some(n) = self.nodes.remove(&cur) {
                stack.extend(n.children);
            }
        }
    }

    /// Move a node to the top (`top`) or bottom of its sibling list.
    pub fn move_to_edge(&mut self, id: NodeId, top: bool) {
        if let Some(list) = self.sibling_list_mut(id) {
            if let Some(i) = list.iter().position(|x| *x == id) {
                let item = list.remove(i);
                if top {
                    list.insert(0, item);
                } else {
                    list.push(item);
                }
            }
        }
    }

    /// Reorder via drag & drop: place `moved` immediately before/after `target`,
    /// adopting `target`'s parent (so this also reparents across lists). No-ops
    /// if it would drop a node into its own subtree.
    pub fn reorder(&mut self, moved: NodeId, target: NodeId, before: bool) {
        if moved == target
            || !self.nodes.contains_key(&moved)
            || !self.nodes.contains_key(&target)
            || self.is_descendant(target, moved)
        {
            return;
        }
        let new_parent = self.nodes.get(&target).and_then(|n| n.parent);
        if let Some(list) = self.sibling_list_mut(moved) {
            list.retain(|x| *x != moved);
        }
        if let Some(n) = self.nodes.get_mut(&moved) {
            n.parent = new_parent;
        }
        let list = match new_parent {
            Some(p) => self.nodes.get_mut(&p).map(|n| &mut n.children),
            None => Some(&mut self.roots),
        };
        if let Some(list) = list {
            let pos = list
                .iter()
                .position(|x| *x == target)
                .map_or(list.len(), |i| if before { i } else { i + 1 });
            list.insert(pos, moved);
        }
    }

    /// Is `node` inside the subtree rooted at `ancestor`?
    fn is_descendant(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut cur = self.nodes.get(&node).and_then(|n| n.parent);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.nodes.get(&c).and_then(|n| n.parent);
        }
        false
    }

    /// Move a node one slot earlier (`up`) or later among its siblings.
    pub fn move_sibling(&mut self, id: NodeId, up: bool) {
        if let Some(list) = self.sibling_list_mut(id) {
            if let Some(i) = list.iter().position(|x| *x == id) {
                if up && i > 0 {
                    list.swap(i, i - 1);
                } else if !up && i + 1 < list.len() {
                    list.swap(i, i + 1);
                }
            }
        }
    }

    /// Make `id` a child of its previous sibling.
    pub fn indent(&mut self, id: NodeId) {
        let parent = self.nodes.get(&id).and_then(|n| n.parent);
        let list = match parent {
            Some(p) => self.nodes.get(&p).map(|n| n.children.clone()),
            None => Some(self.roots.clone()),
        };
        let Some(list) = list else { return };
        let Some(i) = list.iter().position(|x| *x == id) else { return };
        if i == 0 {
            return; // no previous sibling to adopt it
        }
        let new_parent = list[i - 1];
        // Detach from current list.
        if let Some(l) = self.sibling_list_mut(id) {
            l.retain(|x| *x != id);
        }
        // Attach under the previous sibling.
        if let Some(np) = self.nodes.get_mut(&new_parent) {
            np.children.push(id);
            np.expanded = true;
        }
        if let Some(n) = self.nodes.get_mut(&id) {
            n.parent = Some(new_parent);
        }
    }

    /// Make `id` a sibling of its parent (one level shallower).
    pub fn outdent(&mut self, id: NodeId) {
        let Some(parent) = self.nodes.get(&id).and_then(|n| n.parent) else {
            return; // already a root
        };
        let grandparent = self.nodes.get(&parent).and_then(|n| n.parent);
        // Detach from parent.
        if let Some(pn) = self.nodes.get_mut(&parent) {
            pn.children.retain(|x| *x != id);
        }
        // Insert just after the parent in the grandparent's list (or roots).
        let target: &mut Vec<NodeId> = match grandparent {
            Some(g) => match self.nodes.get_mut(&g) {
                Some(gn) => &mut gn.children,
                None => return,
            },
            None => &mut self.roots,
        };
        let idx = target
            .iter()
            .position(|x| *x == parent)
            .map_or(target.len(), |i| i + 1);
        target.insert(idx, id);
        if let Some(n) = self.nodes.get_mut(&id) {
            n.parent = grandparent;
        }
    }

    // --- import / export ----------------------------------------------------

    /// Build a standalone HTML document from the whole tree.
    pub fn export_html(&self) -> String {
        let mut s = String::new();
        s.push_str(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <title>Trellis export</title>\n<style>\n",
        );
        s.push_str(EXPORT_CSS);
        s.push_str("</style>\n</head>\n<body>\n<main>\n");
        for &r in &self.roots {
            self.export_node_html(r, 1, &mut s);
        }
        s.push_str("</main>\n</body>\n</html>\n");
        s
    }

    fn export_node_html(&self, id: NodeId, depth: usize, s: &mut String) {
        let Some(node) = self.nodes.get(&id) else { return };
        let h = depth.min(6);
        s.push_str(&format!(
            "<section class=\"node\">\n<h{h}>{}</h{h}>\n",
            escape_html(&node.title)
        ));
        for card in &node.cards {
            s.push_str("<article class=\"card\">\n");
            if !card.title.is_empty() {
                s.push_str(&format!("<h4>{}</h4>\n", escape_html(&card.title)));
            }
            match &card.kind {
                CardKind::Text => s.push_str(&md_to_html(&card.body)),
                CardKind::Code { lang } => {
                    let fenced = format!("```{lang}\n{}\n```", card.body);
                    s.push_str(&md_to_html(&fenced));
                }
                CardKind::Checklist { items } => {
                    s.push_str("<ul class=\"checklist\">\n");
                    for it in items {
                        let mark = if it.done { "checked" } else { "" };
                        s.push_str(&format!(
                            "<li><input type=\"checkbox\" disabled {mark}> {}</li>\n",
                            escape_html(&it.text)
                        ));
                    }
                    s.push_str("</ul>\n");
                }
                CardKind::Image { data, name } => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(data);
                    let mime = mime_for(name);
                    s.push_str(&format!(
                        "<img alt=\"{}\" src=\"data:{mime};base64,{b64}\">\n",
                        escape_html(name)
                    ));
                }
            }
            s.push_str("</article>\n");
        }
        let children = node.children.clone();
        for child in children {
            self.export_node_html(child, depth + 1, s);
        }
        s.push_str("</section>\n");
    }

    /// Serialize the whole document to pretty-printed JSON. Image cards embed
    /// their bytes as a JSON array, so exports stay self-contained.
    pub fn export_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Render the whole tree as a single Markdown document: nodes become
    /// headings (nesting = heading level), cards become their Markdown.
    pub fn export_markdown(&self) -> String {
        let mut s = String::new();
        for &r in &self.roots {
            self.export_node_md(r, 1, &mut s);
        }
        s
    }

    /// Flatten the whole document into a sequence of laid-out text lines, shared
    /// by the PDF and image exporters. Blank lines (empty text) act as spacers.
    fn export_lines(&self) -> Vec<ExportLine> {
        let mut out = Vec::new();
        for &r in &self.roots {
            self.export_node_lines(r, 1, &mut out);
        }
        out
    }

    fn export_node_lines(&self, id: NodeId, depth: usize, out: &mut Vec<ExportLine>) {
        let Some(node) = self.nodes.get(&id) else { return };
        let hsize = match depth {
            1 => 18.0,
            2 => 15.0,
            3 => 13.0,
            _ => 12.0,
        };
        out.push(ExportLine { text: node.title.clone(), size: hsize });
        out.push(ExportLine { text: String::new(), size: 6.0 });
        for card in &node.cards {
            if !card.title.is_empty() {
                out.push(ExportLine { text: card.title.clone(), size: 12.0 });
            }
            match &card.kind {
                CardKind::Text => {
                    let body = card.body.trim_end();
                    if !body.is_empty() {
                        out.push(ExportLine { text: body.to_string(), size: 10.5 });
                    }
                }
                CardKind::Code { .. } => {
                    for line in card.body.trim_end().split('\n') {
                        out.push(ExportLine { text: line.to_string(), size: 10.0 });
                    }
                }
                CardKind::Checklist { items } => {
                    for it in items {
                        let mark = if it.done { "[x]" } else { "[ ]" };
                        out.push(ExportLine { text: format!("{mark} {}", it.text), size: 10.5 });
                    }
                }
                CardKind::Image { name, .. } => {
                    out.push(ExportLine { text: format!("(image: {name})"), size: 10.5 });
                }
            }
            out.push(ExportLine { text: String::new(), size: 5.0 });
        }
        for c in node.children.clone() {
            self.export_node_lines(c, depth + 1, out);
        }
    }

    /// Render the whole document to a PDF (A4, paginated). Returns the file bytes.
    pub fn export_pdf(&self) -> Result<Vec<u8>, String> {
        use printpdf::{Mm, PdfDocument};
        let font_ab = ab_glyph::FontRef::try_from_slice(EXPORT_FONT).map_err(|e| e.to_string())?;
        let (w_mm, h_mm, margin) = (210.0_f32, 297.0_f32, 20.0_f32);
        const MM_TO_PT: f32 = 2.834_646;
        let content_w_pt = (w_mm - margin * 2.0) * MM_TO_PT;
        let (doc, page1, layer1) =
            PdfDocument::new("Trellis export", Mm(w_mm), Mm(h_mm), "Layer 1");
        let font = doc
            .add_external_font(std::io::Cursor::new(EXPORT_FONT))
            .map_err(|e| e.to_string())?;
        let mut layer = doc.get_page(page1).get_layer(layer1);
        let mut y = h_mm - margin;
        for l in self.export_lines() {
            let leading = (l.size * 1.4) / MM_TO_PT;
            let wrapped = if l.text.is_empty() {
                vec![String::new()]
            } else {
                wrap_text(&font_ab, l.size, &l.text, content_w_pt)
            };
            for line in wrapped {
                if y < margin {
                    let (p, lay) = doc.add_page(Mm(w_mm), Mm(h_mm), "Layer");
                    layer = doc.get_page(p).get_layer(lay);
                    y = h_mm - margin;
                }
                if !line.is_empty() {
                    layer.use_text(&line, l.size, Mm(margin), Mm(y), &font);
                }
                y -= leading;
            }
        }
        doc.save_to_bytes().map_err(|e| e.to_string())
    }

    /// Render the whole document to a raster image (PNG, or GIF if `gif`).
    /// Returns the encoded file bytes. One tall page, black text on white.
    pub fn export_image(&self, gif: bool) -> Result<Vec<u8>, String> {
        use ab_glyph::FontRef;
        use image::{Rgba, RgbaImage};
        let font = FontRef::try_from_slice(EXPORT_FONT).map_err(|e| e.to_string())?;
        let scale = 2.0_f32; // px per point
        let margin = 40.0_f32;
        let content_w = 760.0_f32;
        let width = (content_w + margin * 2.0) as u32;

        // Pre-wrap every line, remembering its pixel size, to size the canvas.
        let mut rows: Vec<(String, f32)> = Vec::new();
        for l in self.export_lines() {
            let px = l.size * scale;
            if l.text.is_empty() {
                rows.push((String::new(), px));
            } else {
                for w in wrap_text(&font, px, &l.text, content_w) {
                    rows.push((w, px));
                }
            }
        }
        let total_h: f32 = margin * 2.0 + rows.iter().map(|(_, s)| s * 1.5).sum::<f32>();
        let height = (total_h as u32).max(1);
        let mut img = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));

        let mut y = margin;
        for (text, px) in &rows {
            if !text.is_empty() {
                draw_text(&mut img, &font, *px, margin, y + *px, text);
            }
            y += px * 1.5;
        }
        let mut buf = Vec::new();
        let fmt = if gif { image::ImageFormat::Gif } else { image::ImageFormat::Png };
        img.write_to(&mut std::io::Cursor::new(&mut buf), fmt).map_err(|e| e.to_string())?;
        Ok(buf)
    }

    fn export_node_md(&self, id: NodeId, depth: usize, s: &mut String) {
        let Some(node) = self.nodes.get(&id) else { return };
        s.push_str(&format!("{} {}\n\n", "#".repeat(depth.min(6)), node.title));
        for card in &node.cards {
            if !card.title.is_empty() {
                s.push_str(&format!("**{}**\n\n", card.title));
            }
            match &card.kind {
                CardKind::Text => {
                    s.push_str(card.body.trim_end());
                    s.push_str("\n\n");
                }
                CardKind::Code { lang } => {
                    s.push_str(&format!("```{lang}\n{}\n```\n\n", card.body));
                }
                CardKind::Checklist { items } => {
                    for it in items {
                        let mark = if it.done { "x" } else { " " };
                        s.push_str(&format!("- [{mark}] {}\n", it.text));
                    }
                    s.push('\n');
                }
                CardKind::Image { name, .. } => {
                    s.push_str(&format!("*(image: {name})*\n\n"));
                }
            }
        }
        let children = node.children.clone();
        for c in children {
            self.export_node_md(c, depth + 1, s);
        }
    }

    /// Create a new root node from imported text, splitting nothing — the whole
    /// document becomes a single markdown card. `html` chooses conversion.
    pub fn import_as_node(&mut self, title: String, content: &str, html: bool) -> NodeId {
        let markdown = if html {
            html2md::parse_html(content)
        } else {
            content.to_string()
        };
        let id = self.add_node(None, title);
        if let Some(cid) = self.add_card(id, egui::pos2(40.0, 40.0), CardKind::Text) {
            if let Some(c) = self.card_mut(id, cid) {
                c.body = markdown;
                c.size = egui::vec2(460.0, 340.0);
                c.editing = false;
            }
        }
        id
    }

    /// Collect (node, card, snippet) matches for a case-insensitive query.
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let q = query.to_lowercase();
        let mut hits = Vec::new();
        if q.is_empty() {
            return hits;
        }
        for node in self.nodes.values() {
            if node.title.to_lowercase().contains(&q) {
                hits.push(SearchHit {
                    node: node.id,
                    node_title: node.title.clone(),
                    snippet: "(title)".to_string(),
                });
            }
            for card in &node.cards {
                let hay = format!("{} {}", card.title, searchable_body(card));
                if let Some(pos) = hay.to_lowercase().find(&q) {
                    hits.push(SearchHit {
                        node: node.id,
                        node_title: node.title.clone(),
                        snippet: snippet_around(&hay, pos, q.len()),
                    });
                }
            }
        }
        hits
    }
}

pub struct SearchHit {
    pub node: NodeId,
    pub node_title: String,
    pub snippet: String,
}

fn searchable_body(card: &Card) -> String {
    match &card.kind {
        CardKind::Text | CardKind::Code { .. } => card.body.clone(),
        CardKind::Checklist { items } => items
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        CardKind::Image { name, .. } => name.clone(),
    }
}

fn snippet_around(text: &str, pos: usize, len: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    // `pos`/`len` are byte offsets into `text`; map to a char window loosely.
    let start_byte = pos.saturating_sub(20);
    let end_byte = (pos + len + 20).min(text.len());
    let slice = text
        .char_indices()
        .filter(|(i, _)| *i >= start_byte && *i < end_byte)
        .map(|(_, c)| c)
        .collect::<String>();
    let _ = chars;
    let trimmed = slice.replace('\n', " ");
    format!("…{}…", trimmed.trim())
}

fn md_to_html(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let wrapped = hard_wrap(md);
    let parser = Parser::new_ext(&wrapped, Options::all());
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Turn single newlines into Markdown hard breaks so a rendered card matches
/// what the user typed line-for-line. CommonMark treats a lone newline as a
/// "soft break" (rendered as a space), so without this you'd need a blank line
/// between every line; users expect each Enter to break. We append the two
/// trailing spaces that mark a hard break to each non-empty line, skipping
/// fenced code blocks (``` / ~~~) where newlines are already literal.
pub(crate) fn hard_wrap(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + 16);
    let mut in_fence = false;
    let mut lines = md.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
        } else if in_fence || line.trim_end().is_empty() {
            // Code-block content or a blank paragraph separator: leave as-is.
            out.push_str(line);
        } else {
            out.push_str(line.trim_end());
            out.push_str("  "); // two trailing spaces = hard break
        }
        if lines.peek().is_some() {
            out.push('\n');
        }
    }
    if md.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn mime_for(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else {
        "application/octet-stream"
    }
}

const EXPORT_CSS: &str = "\
:root{color-scheme:light dark}\
body{font-family:-apple-system,Segoe UI,Roboto,sans-serif;line-height:1.55;margin:0;\
background:#faf9f7;color:#1b1b1b}\
main{max-width:820px;margin:0 auto;padding:2.5rem 1.25rem}\
section.node{margin:1.25rem 0;padding-left:1rem;border-left:2px solid #e2ded7}\
h1,h2,h3,h4,h5,h6{line-height:1.2}\
article.card{background:#fff;border:1px solid #e6e2db;border-radius:8px;padding:.85rem 1rem;\
margin:.75rem 0;box-shadow:0 1px 2px rgba(0,0,0,.04)}\
article.card h4{margin:.1rem 0 .5rem;color:#555}\
ul.checklist{list-style:none;padding-left:0}\
ul.checklist li{margin:.2rem 0}\
img{max-width:100%;border-radius:6px}\
pre{background:#1e1e1e;color:#eee;padding:.75rem 1rem;border-radius:6px;overflow:auto}\
code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace}\
:not(pre)>code{background:#eee;padding:.1rem .3rem;border-radius:4px}\
@media(prefers-color-scheme:dark){body{background:#17181a;color:#e6e6e6}\
section.node{border-left-color:#333}article.card{background:#202225;border-color:#333}\
article.card h4{color:#aaa}:not(pre)>code{background:#333}}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_wrap_breaks_single_newlines_but_not_code_or_blank_lines() {
        // Single newlines get two trailing spaces (a Markdown hard break)...
        assert_eq!(hard_wrap("a\nb"), "a  \nb  ");
        // ...blank paragraph separators are left alone...
        assert_eq!(hard_wrap("a\n\nb"), "a  \n\nb  ");
        // ...and fenced code blocks are untouched.
        assert_eq!(hard_wrap("```\nx\ny\n```"), "```\nx\ny\n```");
    }

    #[test]
    fn hard_wrap_renders_as_line_breaks_in_html() {
        // The whole point: two lines become two visual lines (<br>), not one.
        assert!(md_to_html("line one\nline two").contains("<br"));
    }

    #[test]
    fn grouping_and_docking() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(50.0, 0.0), CardKind::Text).unwrap();
        let c = doc.add_card(n, egui::pos2(100.0, 0.0), CardKind::Text).unwrap();

        // Group needs 2+ cards.
        assert!(doc.group_cards(n, &[a], "x".into()).is_none());
        let g = doc.group_cards(n, &[a, b], "Pair".into()).unwrap();
        assert_eq!(doc.card_mut(n, a).unwrap().group, Some(g));
        assert_eq!(doc.card_mut(n, b).unwrap().group, Some(g));
        assert_eq!(doc.nodes[&n].groups.len(), 1);

        // Ungroup clears membership and drops the container.
        doc.ungroup(n, g);
        assert_eq!(doc.card_mut(n, a).unwrap().group, None);
        assert!(doc.nodes[&n].groups.is_empty());

        // Dock c onto a; moving a drags c along, b stays put.
        doc.dock_card(n, c, a);
        assert_eq!(doc.card_mut(n, c).unwrap().docked_to, Some(a));
        doc.move_card_tree(n, a, egui::vec2(10.0, 5.0));
        assert_eq!(doc.card_mut(n, a).unwrap().pos, egui::pos2(10.0, 5.0));
        assert_eq!(doc.card_mut(n, c).unwrap().pos, egui::pos2(110.0, 5.0));
        assert_eq!(doc.card_mut(n, b).unwrap().pos, egui::pos2(50.0, 0.0));
    }

    #[test]
    fn dock_rejects_cycles_and_remove_detaches() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.dock_card(n, b, a); // b sticks to a
        doc.dock_card(n, a, b); // would cycle → ignored
        assert_eq!(doc.card_mut(n, a).unwrap().docked_to, None);
        // Removing the anchor detaches its dependents.
        doc.remove_card(n, a);
        assert_eq!(doc.card_mut(n, b).unwrap().docked_to, None);
    }

    #[test]
    fn ron_round_trips() {
        let doc = Document::default();
        let s = ron::ser::to_string(&doc).unwrap();
        let back: Document = ron::from_str(&s).unwrap();
        assert_eq!(doc.roots, back.roots);
        assert_eq!(doc.nodes.len(), back.nodes.len());
    }

    #[test]
    fn indent_then_outdent_restores_shape() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let a = doc.add_node(None, "a".into());
        let b = doc.add_node(None, "b".into());
        // b indents under a...
        doc.indent(b);
        assert_eq!(doc.nodes[&b].parent, Some(a));
        assert_eq!(doc.nodes[&a].children, vec![b]);
        assert_eq!(doc.roots, vec![a]);
        // ...and outdents back to a root sibling of a.
        doc.outdent(b);
        assert_eq!(doc.nodes[&b].parent, None);
        assert!(doc.nodes[&a].children.is_empty());
        assert_eq!(doc.roots, vec![a, b]);
    }

    #[test]
    fn move_to_edge_and_reorder() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "a".into());
        let b = doc.add_node(None, "b".into());
        let c = doc.add_node(None, "c".into());
        // roots: [a, b, c]
        doc.move_to_edge(c, true);
        assert_eq!(doc.roots, vec![c, a, b]);
        doc.move_to_edge(c, false);
        assert_eq!(doc.roots, vec![a, b, c]);
        // Drop c before a.
        doc.reorder(c, a, true);
        assert_eq!(doc.roots, vec![c, a, b]);
        // Drop a after b.
        doc.reorder(a, b, false);
        assert_eq!(doc.roots, vec![c, b, a]);
    }

    #[test]
    fn reorder_reparents_and_blocks_cycles() {
        let mut doc = Document::empty();
        let parent = doc.add_node(None, "p".into());
        let child = doc.add_node(Some(parent), "c".into());
        let other = doc.add_node(None, "o".into());
        // Move `other` under parent, before child.
        doc.reorder(other, child, true);
        assert_eq!(doc.nodes[&other].parent, Some(parent));
        assert_eq!(doc.nodes[&parent].children, vec![other, child]);
        assert!(doc.roots.contains(&parent) && !doc.roots.contains(&other));
        // Dropping a parent into its own child is refused.
        doc.reorder(parent, child, true);
        assert_eq!(doc.nodes[&parent].parent, None);
    }

    #[test]
    fn remove_node_drops_whole_subtree() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let a = doc.add_node(None, "a".into());
        let b = doc.add_node(Some(a), "b".into());
        let c = doc.add_node(Some(b), "c".into());
        doc.remove_node(a);
        assert!(!doc.nodes.contains_key(&a));
        assert!(!doc.nodes.contains_key(&b));
        assert!(!doc.nodes.contains_key(&c));
        assert!(doc.roots.is_empty());
    }

    #[test]
    fn export_html_includes_content_and_checklist() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let n = doc.add_node(None, "Node & <title>".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, cid).unwrap().body = "**bold**".into();
        let lid = doc
            .add_card(
                n,
                egui::pos2(0.0, 0.0),
                CardKind::Checklist {
                    items: vec![ChecklistItem { done: true, text: "done item".into() }],
                },
            )
            .unwrap();
        let _ = lid;
        let html = doc.export_html();
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("Node &amp; &lt;title&gt;")); // escaped
        assert!(html.contains("checked"));
        assert!(html.contains("done item"));
    }

    #[test]
    fn import_html_becomes_markdown_card() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let id = doc.import_as_node("page".into(), "<h1>Hi</h1><p>there</p>", true);
        let node = &doc.nodes[&id];
        assert_eq!(node.cards.len(), 1);
        assert!(node.cards[0].body.contains("Hi"));
    }

    #[test]
    fn paste_card_into_another_node() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "a".into());
        let b = doc.add_node(None, "b".into());
        let cid = doc.add_card(a, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(a, cid).unwrap().body = "hello".into();
        let template = doc.nodes[&a].cards[0].clone();
        let new_id = doc.add_card_from(b, &template, egui::pos2(5.0, 5.0)).unwrap();
        assert_ne!(new_id, cid); // fresh id
        assert_eq!(doc.nodes[&b].cards.len(), 1);
        assert_eq!(doc.nodes[&b].cards[0].body, "hello");
        assert_eq!(doc.nodes[&b].cards[0].pos, egui::pos2(5.0, 5.0));
        // Original untouched.
        assert_eq!(doc.nodes[&a].cards.len(), 1);
    }

    #[test]
    fn export_pdf_and_image_produce_valid_files() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Report".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c).unwrap().title = "Intro".into();
        doc.card_mut(n, c).unwrap().body =
            "A fairly long paragraph that should wrap across several lines when \
             laid out into a fixed-width page so we exercise the wrapper too."
                .into();

        let pdf = doc.export_pdf().expect("pdf");
        assert!(pdf.starts_with(b"%PDF"), "PDF magic header");

        let png = doc.export_image(false).expect("png");
        assert_eq!(&png[1..4], b"PNG", "PNG magic header");

        let gif = doc.export_image(true).expect("gif");
        assert!(gif.starts_with(b"GIF8"), "GIF magic header");
    }

    #[test]
    fn export_markdown_has_headings_and_cards() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Title".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, cid).unwrap().body = "**bold** body".into();
        let md = doc.export_markdown();
        assert!(md.contains("# Title"));
        assert!(md.contains("**bold** body"));
    }

    #[test]
    fn search_finds_titles_and_bodies() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let n = doc.add_node(None, "Groceries".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, cid).unwrap().body = "buy avocados".into();
        assert_eq!(doc.search("grocer").len(), 1);
        assert_eq!(doc.search("avocado").len(), 1);
        assert_eq!(doc.search("zzz").len(), 0);
    }
}
