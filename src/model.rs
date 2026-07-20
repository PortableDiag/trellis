//! Core data model for Trellis.
//!
//! The document is a *tree of nodes* (borrowed from the outliner world) where
//! every node's body is a *basket*: a free-form 2-D surface holding draggable
//! cards. Structure lives in the tree; spatial thinking lives in the basket.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type NodeId = u64;
pub type CardId = u64;

/// A single draggable, editable card on a node's basket canvas.
#[derive(Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: CardId,
    /// Top-left position in canvas coordinates (independent of pan/zoom).
    pub pos: egui::Pos2,
    pub size: egui::Vec2,
    pub text: String,
    /// RGB accent used for the card's title bar.
    pub color: [u8; 3],
}

impl Card {
    pub fn new(id: CardId, pos: egui::Pos2) -> Self {
        Self {
            id,
            pos,
            size: egui::vec2(220.0, 140.0),
            text: String::new(),
            color: [0x3b, 0x82, 0xf6],
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
        let root = doc.add_node(None, "Welcome".to_string());
        if let Some(n) = doc.nodes.get_mut(&root) {
            let mut card = Card::new(doc.next_card_id, egui::pos2(60.0, 60.0));
            card.text =
                "This is a basket. Double-click the canvas to drop a card, drag cards around, \
                 and grow the tree on the left."
                    .to_string();
            n.cards.push(card);
        }
        doc.next_card_id += 1;
        doc
    }
}

impl Document {
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

    pub fn add_card(&mut self, node: NodeId, pos: egui::Pos2) -> Option<CardId> {
        let id = self.next_card_id;
        let n = self.nodes.get_mut(&node)?;
        n.cards.push(Card::new(id, pos));
        self.next_card_id += 1;
        Some(id)
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
        // Depth-first collect then drop.
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if let Some(n) = self.nodes.remove(&cur) {
                stack.extend(n.children);
            }
        }
    }
}
