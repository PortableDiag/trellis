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
    #[serde(default = "default_true")]
    pub expanded: bool,
    /// Optional per-node tag color shown as a dot in the tree.
    #[serde(default)]
    pub color: Option<[u8; 3]>,
}

fn default_true() -> bool {
    true
}

/// The whole document: an arena of nodes plus ordered roots and id counters.
#[derive(Serialize, Deserialize)]
pub struct Document {
    pub nodes: HashMap<NodeId, Node>,
    pub roots: Vec<NodeId>,
    next_node_id: NodeId,
    next_card_id: CardId,
}

impl Default for Document {
    fn default() -> Self {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
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

    pub fn remove_card(&mut self, node: NodeId, card: CardId) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.cards.retain(|c| c.id != card);
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
    let parser = Parser::new_ext(md, Options::all());
    let mut out = String::new();
    html::push_html(&mut out, parser);
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
    fn remove_node_drops_whole_subtree() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
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
        };
        let id = doc.import_as_node("page".into(), "<h1>Hi</h1><p>there</p>", true);
        let node = &doc.nodes[&id];
        assert_eq!(node.cards.len(), 1);
        assert!(node.cards[0].body.contains("Hi"));
    }

    #[test]
    fn search_finds_titles_and_bodies() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
        };
        let n = doc.add_node(None, "Groceries".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, cid).unwrap().body = "buy avocados".into();
        assert_eq!(doc.search("grocer").len(), 1);
        assert_eq!(doc.search("avocado").len(), 1);
        assert_eq!(doc.search("zzz").len(), 0);
    }
}
