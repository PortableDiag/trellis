//! Local HTTP API so external agents can read and edit the document.
//!
//! A background thread runs a tiny blocking HTTP server bound to `127.0.0.1` by
//! default, or `0.0.0.0` (all interfaces, for LAN access) when LAN access is
//! enabled in Settings.
//! Each request is authenticated against the key set in Settings, then handed to
//! the UI thread over a channel; the UI thread applies it to the live `Document`
//! and sends a response back. This keeps all document access single-threaded.
//!
//! Auth: send the key as `X-API-Key: <key>` or `Authorization: Bearer <key>`.
//! An empty key (the default) disables the API. `GET /api/health` is unauthenticated.

use crate::model::{Card, CardKind, ChecklistItem, Document, GroupId, NodeId};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::mpsc::{Sender, SyncSender};
use crate::changelog::{Change, ChangeLog};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiny_http::Method;

/// A request routed to the UI thread, plus the channel to answer on.
pub struct ApiCommand {
    pub req: ApiRequest,
    pub resp: SyncSender<ApiResponse>,
    /// The plugin scope this request arrived under, or `None` for the instance's
    /// own key. The subtree half of a scope needs the document to resolve
    /// ancestry, so it is checked in the app loop rather than on this thread.
    pub scope: Option<crate::plugins::Scope>,
}

/// A parsed, validated API request. Document access happens on the UI thread.
pub enum ApiRequest {
    Health,
    Tree,
    ListNodes,
    GetNode(NodeId),
    ListCards(NodeId),
    /// One card on its own — the read counterpart of PATCH/DELETE on the same
    /// path. Without it, re-reading a card you just wrote means pulling the
    /// whole basket back.
    GetCard { node: NodeId, card: u64 },
    CreateNode { parent: Option<NodeId>, title: String },
    UpdateNode { id: NodeId, title: Option<String>, color: Option<[u8; 3]>, bg: Option<[u8; 3]> },
    DeleteNode(NodeId),
    // Reorder / reparent a node in the tree.
    MoveNode { id: NodeId, mv: MoveNodeInput },
    // Expand or collapse a node (optionally its whole subtree).
    SetExpanded { id: NodeId, expanded: bool, recursive: bool },
    AddCard { node: NodeId, input: AddCardInput },
    UpdateCard { node: NodeId, card: u64, patch: UpdateCardInput },
    // ↑ Both accept `fit`. `process` applies `Card::fit_size`, which is only an
    // estimate; the app re-fits precisely afterwards — see `fit_request`.
    DeleteCard { node: NodeId, card: u64 },
    // Reorder a card within its basket (draw / autosort order).
    MoveCard { node: NodeId, card: u64, mv: MoveCardInput },
    // Set an inline key:: value property on a card (e.g. status for the board).
    SetCardProperty { node: NodeId, card: u64, key: String, value: String },
    // Grouping.
    ListGroups(NodeId),
    CreateGroup { node: NodeId, cards: Vec<u64>, title: Option<String> },
    UpdateGroup { node: NodeId, group: GroupId, title: Option<String>, color: Option<[u8; 3]> },
    DeleteGroup { node: NodeId, group: GroupId },
    // Docking.
    DockCard { node: NodeId, card: u64, anchor: u64 },
    DetachCard { node: NodeId, card: u64 },
    // Card group membership (join an existing group / leave).
    SetCardGroup { node: NodeId, card: u64, group: Option<GroupId> },
    // Fine-grained table editing (cell colors, header, widths, row/col ops).
    TableOp { node: NodeId, card: u64, op: TableOpInput },
    // Draw a table card as a chart, or turn the chart off.
    SetChart { node: NodeId, card: u64, spec: Option<ChartInput> },
    // Sketch editing (add stroke / undo / clear).
    SketchOp { node: NodeId, card: u64, op: SketchOpInput },
    // Image bytes.
    AddImage { node: NodeId, card: u64, name: String, bytes: Vec<u8> },
    RemoveImage { node: NodeId, card: u64, index: usize },
    GetImage { node: NodeId, card: u64, index: usize },
    // Arrange a node's cards into a tidy non-overlapping grid.
    Autosort(NodeId),
    // Whole-document export.
    Export(String),
    Search(String),
    // #tags: list all with counts, or (with ?name=) the cards carrying one.
    Tags,
    TagCards(String),
    // key:: value properties: list keys, or (with ?key=[&value=]) matching cards.
    PropertyKeys,
    PropertyCards { key: String, value: Option<String> },
    // Combined dropdown-style query across the tree, and the due-date agenda.
    QueryCards { tag: Option<String>, key: Option<String>, value: Option<String>, text: Option<String> },
    Tasks { include_done: bool, project: Option<NodeId> },
    // Cards grouped by `status::` value — the Kanban board's columns.
    Kanban { project: Option<NodeId> },
    // Cards that [[link]] to a node.
    Backlinks(NodeId),
    // The wiki-link graph (linked nodes + directed edges).
    Graph,
    // Which document this instance has open (and on which port), so an agent
    // driving several instances can check it has the right one.
    Instance,
    // Backup + version-history control — handled by the app loop (they need the
    // doc's on-disk path / config), not `process`, which only sees the Document.
    BackupStatus,
    BackupRun,
    HistoryList,
    HistoryRestore(String),
    OcrAll,
    // Reusable card templates (the UI's Save as template / Insert template). These
    // live in app config, not the Document, so the app loop answers them.
    TemplateList,
    TemplateRegister { node: NodeId, card: u64, title: Option<String> },
    TemplateInsert { index: usize, node: NodeId, pos: Option<[f32; 2]> },
    // Re-snapshot an existing template slot from a (usually edited) card, in place.
    TemplateUpdate { index: usize, node: NodeId, card: u64, title: Option<String> },
    // Stamp a master card for every template that lacks a live one.
    TemplateRebuild,
    TemplateDelete(usize),
}

/// Which card a request asked to size to its content, if any.
///
/// `process` applies [`crate::model::Card::fit_size`], which is an estimate: it
/// has no font context, so for a Text card it guesses the wrapped height and
/// guesses tall, leaving a strip of empty card under the text. The app loop runs
/// on the UI thread where the real fonts *are* available, so after `process` it
/// re-measures — which is what the right-click "Fit to content" has always done.
/// This is how the two paths are kept from disagreeing.
///
/// `None` for the card id on `AddCard`: it doesn't exist until `process` has run,
/// so the caller reads the new id out of the response.
pub fn fit_request(req: &ApiRequest) -> Option<(NodeId, Option<u64>)> {
    match req {
        ApiRequest::AddCard { node, input } if input.fit => Some((*node, None)),
        ApiRequest::UpdateCard { node, card, patch } if patch.fit => Some((*node, Some(*card))),
        _ => None,
    }
}


/// The node a request acts on, for enforcing a subtree-scoped plugin token.
///
/// `None` means the request either names no node (a whole-document read like
/// `/api/tree` or `/api/export`) or names one indirectly. **A subtree-scoped
/// token is refused for those** rather than allowed through — a scope that
/// silently stops applying at the edges is not a scope.
pub fn target_node(req: &ApiRequest) -> Option<NodeId> {
    match req {
        ApiRequest::GetNode(id)
        | ApiRequest::UpdateNode { id, .. }
        | ApiRequest::DeleteNode(id)
        | ApiRequest::MoveNode { id, .. }
        | ApiRequest::SetExpanded { id, .. }
        | ApiRequest::Backlinks(id)
        | ApiRequest::ListCards(id)
        | ApiRequest::ListGroups(id)
        | ApiRequest::Autosort(id) => Some(*id),
        ApiRequest::GetCard { node, .. }
        | ApiRequest::AddCard { node, .. }
        | ApiRequest::UpdateCard { node, .. }
        | ApiRequest::DeleteCard { node, .. }
        | ApiRequest::MoveCard { node, .. }
        | ApiRequest::SetCardProperty { node, .. }
        | ApiRequest::DockCard { node, .. }
        | ApiRequest::DetachCard { node, .. }
        | ApiRequest::SetCardGroup { node, .. }
        | ApiRequest::TableOp { node, .. }
        | ApiRequest::SketchOp { node, .. }
        | ApiRequest::SetChart { node, .. }
        | ApiRequest::AddImage { node, .. }
        | ApiRequest::RemoveImage { node, .. }
        | ApiRequest::GetImage { node, .. }
        | ApiRequest::CreateGroup { node, .. }
        | ApiRequest::UpdateGroup { node, .. }
        | ApiRequest::DeleteGroup { node, .. } => Some(*node),
        ApiRequest::CreateNode { parent, .. } => *parent,
        _ => None,
    }
}

/// Whether a request is harmless for a subtree-scoped token even though it names
/// no node: the instance-level reads a plugin needs to orient itself.
pub fn is_scope_neutral(req: &ApiRequest) -> bool {
    matches!(req, ApiRequest::Health | ApiRequest::Instance | ApiRequest::Tree | ApiRequest::ListNodes)
}

/// Describe what a request is about to change, for the change log.
///
/// Called **before** `process` consumes the request, for the same reason
/// [`fit_request`] is: the request is gone afterwards. Reading the document here
/// also catches the pre-change state — a delete's title cannot be looked up once
/// the thing is deleted.
///
/// A created entity's id does not exist yet, so those come back with `id == 0`
/// and the caller fills it in from the response body (again, as `fit` does).
/// Returns `None` for reads and for the app-intercepted routes, which record
/// their own changes where they are actually handled.
pub fn change_of(req: &ApiRequest, doc: &Document) -> Option<Change> {
    use crate::changelog::{Actor::Api, Entity as E, Op};
    let node_title = |id: &NodeId| doc.nodes.get(id).map(|n| n.title.clone()).unwrap_or_default();
    let card_title = |n: &NodeId, c: &u64| doc.card(*n, *c).map(|c| c.title.clone()).unwrap_or_default();
    let ch = |e, op, id| Change::new(Api, e, op, id);

    Some(match req {
        ApiRequest::CreateNode { title, .. } => ch(E::Node, Op::Created, 0).titled(title.clone()),
        ApiRequest::UpdateNode { id, title, color, bg } => {
            let mut c = ch(E::Node, Op::Updated, *id)
                .titled(title.clone().unwrap_or_else(|| node_title(id)));
            if title.is_some() {
                c = c.field("title");
            }
            if color.is_some() {
                c = c.field("color");
            }
            if bg.is_some() {
                c = c.field("bg");
            }
            c
        }
        ApiRequest::DeleteNode(id) => ch(E::Node, Op::Deleted, *id).titled(node_title(id)),
        ApiRequest::MoveNode { id, .. } => ch(E::Node, Op::Moved, *id).titled(node_title(id)),
        ApiRequest::SetExpanded { id, .. } => {
            ch(E::Node, Op::Updated, *id).titled(node_title(id)).field("expanded")
        }
        ApiRequest::Autosort(id) => {
            // Moves every card in the basket, so it is reported against the
            // basket rather than as N separate card moves.
            ch(E::Node, Op::Updated, *id).titled(node_title(id)).field("autosort")
        }

        ApiRequest::AddCard { node, input } => {
            ch(E::Card, Op::Created, 0).in_node(*node).titled(input.title.clone())
        }
        ApiRequest::UpdateCard { node, card, patch } => {
            let mut c = ch(E::Card, Op::Updated, *card)
                .in_node(*node)
                .titled(patch.title.clone().unwrap_or_else(|| card_title(node, card)));
            for (present, name) in [
                (patch.title.is_some(), "title"),
                (patch.body.is_some(), "body"),
                (patch.color.is_some(), "color"),
                (patch.lang.is_some(), "lang"),
                (patch.pos.is_some(), "pos"),
                (patch.size.is_some(), "size"),
                (patch.items.is_some(), "items"),
                (patch.rows.is_some(), "rows"),
                (patch.kind.is_some(), "kind"),
                (patch.header.is_some(), "header"),
                (patch.font_scale.is_some(), "font_scale"),
                (patch.inline_images.is_some(), "inline_images"),
                (patch.source.is_some(), "source"),
            ] {
                if present {
                    c = c.field(name);
                }
            }
            c
        }
        ApiRequest::DeleteCard { node, card } => {
            ch(E::Card, Op::Deleted, *card).in_node(*node).titled(card_title(node, card))
        }
        ApiRequest::MoveCard { node, card, mv } => {
            // A cross-basket move is reported against the basket it came *from*;
            // the target is named as a field so a client knows to refresh both.
            let c = ch(E::Card, Op::Moved, *card).in_node(*node).titled(card_title(node, card));
            match mv.target_node() {
                Some(to) => c.field(&format!("node={to}")),
                None => c.field("order"),
            }
        }
        // The one entry that carries content, because "fire when a card gets
        // `status:: done`" is the whole point of on-change triggers.
        ApiRequest::SetCardProperty { node, card, key, value } => {
            ch(E::Card, Op::Updated, *card)
                .in_node(*node)
                .titled(card_title(node, card))
                .field("property")
                .property(key.clone(), value.clone())
        }
        ApiRequest::DockCard { node, card, .. } | ApiRequest::DetachCard { node, card } => {
            ch(E::Card, Op::Updated, *card).in_node(*node).titled(card_title(node, card)).field("dock")
        }
        ApiRequest::SetCardGroup { node, card, .. } => {
            ch(E::Card, Op::Updated, *card).in_node(*node).titled(card_title(node, card)).field("group")
        }
        // The sub-operation is named in the field (`table.add_row`) rather than
        // in a separate column: self-describing, and no extra key on every entry.
        ApiRequest::TableOp { node, card, op } => ch(E::Card, Op::Updated, *card)
            .in_node(*node)
            .titled(card_title(node, card))
            .field(&format!("table.{}", op.name())),
        ApiRequest::SketchOp { node, card, op } => ch(E::Card, Op::Updated, *card)
            .in_node(*node)
            .titled(card_title(node, card))
            .field(&format!("sketch.{}", op.name())),
        ApiRequest::SetChart { node, card, spec } => ch(E::Card, Op::Updated, *card)
            .in_node(*node)
            .titled(card_title(node, card))
            .field(if spec.is_some() { "chart" } else { "chart.clear" }),
        ApiRequest::AddImage { node, card, .. } => ch(E::Card, Op::Updated, *card)
            .in_node(*node)
            .titled(card_title(node, card))
            .field("images.add"),
        ApiRequest::RemoveImage { node, card, .. } => ch(E::Card, Op::Updated, *card)
            .in_node(*node)
            .titled(card_title(node, card))
            .field("images.remove"),

        ApiRequest::CreateGroup { node, title, .. } => ch(E::Group, Op::Created, 0)
            .in_node(*node)
            .titled(title.clone().unwrap_or_default()),
        ApiRequest::UpdateGroup { node, group, title, color } => {
            let mut c = ch(E::Group, Op::Updated, *group).in_node(*node);
            if let Some(t) = title {
                c = c.titled(t.clone()).field("title");
            }
            if color.is_some() {
                c = c.field("color");
            }
            c
        }
        ApiRequest::DeleteGroup { node, group } => ch(E::Group, Op::Deleted, *group).in_node(*node),

        // Reads change nothing; app-intercepted routes log where they're handled.
        _ => return None,
    })
}

pub struct ApiResponse {
    pub status: u16,
    pub body: String,
}

impl ApiResponse {
    fn json(status: u16, v: Value) -> Self {
        Self { status, body: serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".into()) }
    }
    pub fn ok(v: Value) -> Self {
        Self::json(200, v)
    }
    fn created(v: Value) -> Self {
        Self::json(201, v)
    }
    pub fn err(status: u16, msg: &str) -> Self {
        Self::json(status, json!({ "error": msg }))
    }
}

// --- request DTOs -----------------------------------------------------------

#[derive(Deserialize)]
struct CreateNodeInput {
    #[serde(default)]
    parent: Option<NodeId>,
    title: String,
}

/// Where to move a card within its basket's order (which is both the draw order
/// — last is on top — and the order Autosort places cards in). Pick ONE:
/// `before`/`after` another card id, an absolute `index`, or `to:"front"|"back"`
/// (front = drawn on top / laid out last, back = first).
#[derive(Deserialize)]
pub struct MoveCardInput {
    #[serde(default)]
    before: Option<u64>,
    #[serde(default)]
    after: Option<u64>,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    to: Option<String>,
    /// Move the card to a **different** basket. Takes precedence over the
    /// ordering fields (which only make sense within one basket); `pos` places
    /// it on the target canvas.
    #[serde(default)]
    node: Option<NodeId>,
    #[serde(default)]
    pos: Option<[f32; 2]>,
}

impl MoveCardInput {
    /// The basket a card is being moved *to*, when this is a cross-basket move.
    pub fn target_node(&self) -> Option<NodeId> {
        self.node
    }
}

/// Body of `POST /api/nodes/{id}/cards/{cid}/chart` — how to draw a table card
/// as a chart. Omitted fields keep their current value, so you can flip the kind
/// without restating the columns.
#[derive(Deserialize)]
pub struct ChartInput {
    /// `bar` | `line` | `scatter`.
    pub kind: String,
    #[serde(default)]
    pub label_col: Option<usize>,
    #[serde(default)]
    pub value_cols: Option<Vec<usize>>,
    #[serde(default)]
    pub show_table: Option<bool>,
}

/// Where to move a node. Pick ONE placement:
/// - `before` / `after`: put this node immediately before/after that sibling,
///   adopting its parent (this is how you reparent across baskets).
/// - `parent` + `index`: put it under `parent` at a 0-based slot; `parent: null`
///   means top level, omitting `parent` keeps the current one, and an `index`
///   past the end appends.
/// - `parent` + `to`: `"top"` or `"bottom"` of `parent` (or the current parent
///   if `parent` is omitted).
#[derive(Deserialize)]
pub struct MoveNodeInput {
    #[serde(default)]
    before: Option<NodeId>,
    #[serde(default)]
    after: Option<NodeId>,
    /// Absent = keep current parent; present `null` = top level; present id = that node.
    #[serde(default, deserialize_with = "de_parent_field")]
    parent: Option<Option<NodeId>>,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    to: Option<String>,
}

#[derive(Deserialize)]
struct ExpandInput {
    expanded: bool,
    /// Apply to the whole subtree (node + all descendants), not just this node.
    #[serde(default)]
    recursive: bool,
}

#[derive(Deserialize)]
struct UpdateNodeInput {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, deserialize_with = "de_color_opt")]
    color: Option<[u8; 3]>,
    /// Basket background color. A color sets it; `null`/absent leaves it unchanged.
    #[serde(default, deserialize_with = "de_color_opt")]
    bg: Option<[u8; 3]>,
}

#[derive(Deserialize)]
pub struct AddCardInput {
    #[serde(default = "default_kind")]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    items: Option<Vec<ChecklistItemInput>>,
    /// Cell text for a `table` card, row by row — so a populated table (and a
    /// chart drawn from it) can be created in one call instead of create-then-
    /// PATCH. Ragged rows are padded to the widest.
    #[serde(default)]
    rows: Option<Vec<Vec<String>>>,
    /// Style the first row as a header (table cards; default true).
    #[serde(default)]
    header: Option<bool>,
    #[serde(default)]
    pos: Option<[f32; 2]>,
    /// Card size (width, height).
    #[serde(default)]
    size: Option<[f32; 2]>,
    /// RGB title-bar accent (array, hex, or name — see `de_color_opt`).
    #[serde(default, deserialize_with = "de_color_opt")]
    color: Option<[u8; 3]>,
    /// Base64 image bytes for an `image` card's first image (name = `title`).
    #[serde(default)]
    image_base64: Option<String>,
    /// Base64 images to embed inline in a **text** card's body. Each becomes an
    /// entry referenced by a `![alt](trellis:N)` marker you place in `body`
    /// (N = its 0-based index here). Applied before `fit`, so `fit` sizes the
    /// card to show them.
    #[serde(default)]
    inline_images: Option<Vec<String>>,
    /// Body font-size multiplier (1.0 = default), for text/code cards.
    #[serde(default)]
    font_scale: Option<f32>,
    /// Size the card to fit its content (overrides `size`), so API/agent-created
    /// cards aren't unreadable little squares. No effect on image cards.
    #[serde(default)]
    fit: bool,
    /// Mirror a file: the body becomes a read-only live copy of it.
    #[serde(default)]
    source: Option<String>,
}

fn default_kind() -> String {
    "text".to_string()
}

#[derive(Clone, Deserialize)]
struct ChecklistItemInput {
    #[serde(default)]
    done: bool,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
pub struct UpdateCardInput {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    /// RGB title-bar accent (array, hex, or name — see `de_color_opt`).
    #[serde(default, deserialize_with = "de_color_opt")]
    color: Option<[u8; 3]>,
    /// Syntax-highlight language (code cards only).
    #[serde(default)]
    lang: Option<String>,
    /// Absolute top-left position on the basket canvas.
    #[serde(default)]
    pos: Option<[f32; 2]>,
    /// Card size (width, height).
    #[serde(default)]
    size: Option<[f32; 2]>,
    /// Replacement checklist items (checklist cards only).
    #[serde(default)]
    items: Option<Vec<ChecklistItemInput>>,
    /// Replacement cell values (table cards only); colors reset.
    #[serde(default)]
    rows: Option<Vec<Vec<String>>>,
    /// Convert the card to another kind: `text`/`code`/`checklist`/`table`/`image`.
    /// Existing body/items/table are kept when compatible; a new kind starts empty.
    #[serde(default)]
    kind: Option<String>,
    /// Header-row flag (table cards only).
    #[serde(default)]
    header: Option<bool>,
    /// Body font-size multiplier (1.0 = default), for text/code cards.
    #[serde(default)]
    font_scale: Option<f32>,
    /// Replacement inline images (base64) for a **text** card, referenced by
    /// `![alt](trellis:N)` markers in `body`. Replaces the card's whole inline
    /// set. Applied before `fit`.
    #[serde(default)]
    inline_images: Option<Vec<String>>,
    /// Resize the card to fit its content (applied after all other fields).
    /// No effect on image cards.
    #[serde(default)]
    fit: bool,
    /// Mirror a file: the card's body becomes a **read-only** live copy of it,
    /// refreshed while the document is open. Send `""` to detach (the last
    /// content stays, and the card becomes editable again).
    #[serde(default)]
    source: Option<String>,
}

#[derive(Deserialize)]
struct CreateGroupInput {
    /// Ids of the cards to group (need at least two that exist in the node).
    cards: Vec<u64>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Deserialize)]
struct UpdateGroupInput {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, deserialize_with = "de_color_opt")]
    color: Option<[u8; 3]>,
}

#[derive(Deserialize)]
struct DockInput {
    /// The card this one should stick to.
    anchor: u64,
}

#[derive(Deserialize)]
struct GroupCardInput {
    /// The group the card should join.
    group: GroupId,
}

#[derive(Deserialize)]
struct AddImageInput {
    #[serde(default)]
    name: String,
    /// Base64-encoded image file bytes (png/jpeg/gif/bmp/webp).
    data_base64: String,
}

/// One fine-grained table edit. `op` selects the operation; the other fields
/// carry its arguments (unused ones are ignored).
#[derive(Deserialize)]
pub struct TableOpInput {
    /// `set_cell` | `set_bg` | `set_fg` | `insert_row` | `remove_row` |
    /// `insert_col` | `remove_col` | `set_col_width` | `autofit_cols` |
    /// `set_header`.
    op: String,
    #[serde(default)]
    row: Option<usize>,
    #[serde(default)]
    col: Option<usize>,
    /// Index for insert/remove row/col (insert puts the new line *at* this index).
    #[serde(default)]
    at: Option<usize>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    width: Option<f32>,
    #[serde(default)]
    header: Option<bool>,
    /// Cell color for `set_bg`/`set_fg`: array/hex/name, or null/absent to clear.
    #[serde(default)]
    color: Value,
}

impl TableOpInput {
    /// The sub-operation's name, for the change log's `table.<op>` field.
    pub fn name(&self) -> &str {
        &self.op
    }
}

/// One edit to a Sketch card. `op` is `add_stroke` | `undo` | `clear`.
#[derive(Deserialize)]
pub struct SketchOpInput {
    op: String,
    /// Stroke color for `add_stroke` (array/hex/name); defaults to black.
    #[serde(default)]
    color: Value,
    #[serde(default)]
    width: Option<f32>,
    /// Stroke points `[[x,y], …]` in the card's local coordinates.
    #[serde(default)]
    points: Option<Vec<[f32; 2]>>,
}

impl SketchOpInput {
    /// The sub-operation's name, for the change log's `sketch.<op>` field.
    pub fn name(&self) -> &str {
        &self.op
    }
}

// --- server thread ----------------------------------------------------------

/// Bind the server (reporting bind errors synchronously) and spawn its accept
/// loop. Returns the `Server` handle so the caller can `unblock()` it to restart
/// on a different bind address (e.g. toggling LAN access). `revision` is a shared
/// document-change counter used by the `/api/wait` long-poll for live updates.
///
/// Each request is handled on its own thread, so a long-poll (or any slow
/// request) never blocks the others — and the accept thread only ever blocks in
/// `incoming_requests()`, so `unblock()` stops it promptly.
pub fn serve(
    port: u16,
    lan: bool,
    ctx: egui::Context,
    tx: Sender<ApiCommand>,
    key: Arc<Mutex<String>>,
    revision: Arc<AtomicU64>,
    changes: Arc<Mutex<ChangeLog>>,
    grants: Arc<Mutex<Vec<crate::plugins::Grant>>>,
) -> Result<Arc<tiny_http::Server>, String> {
    // `lan` binds all interfaces so other devices on the network can reach the
    // API (still key-gated); otherwise localhost-only.
    let host = if lan { "0.0.0.0" } else { "127.0.0.1" };
    let server = Arc::new(tiny_http::Server::http((host, port)).map_err(|e| e.to_string())?);
    let accept = Arc::clone(&server);
    std::thread::Builder::new()
        .name("trellis-api".into())
        .spawn(move || {
            for request in accept.incoming_requests() {
                let ctx = ctx.clone();
                let tx = tx.clone();
                let key = Arc::clone(&key);
                let revision = Arc::clone(&revision);
                let changes = Arc::clone(&changes);
                let grants = Arc::clone(&grants);
                std::thread::spawn(move || {
                    let mut request = request;
                    let resp =
                        handle(&mut request, &ctx, &tx, &key, &revision, &changes, &grants);
                    let header = tiny_http::Header::from_bytes(
                        &b"Content-Type"[..],
                        &b"application/json"[..],
                    )
                    .unwrap();
                    // Permissive CORS so browser extensions / bookmarklets / web
                    // clients can call the (still key-gated) API from any origin.
                    let cors = [
                        (&b"Access-Control-Allow-Origin"[..], &b"*"[..]),
                        (&b"Access-Control-Allow-Methods"[..], &b"GET, POST, PATCH, DELETE, OPTIONS"[..]),
                        (&b"Access-Control-Allow-Headers"[..], &b"X-Api-Key, Content-Type"[..]),
                    ];
                    let mut http = tiny_http::Response::from_string(resp.body)
                        .with_status_code(resp.status)
                        .with_header(header);
                    for (name, value) in cors {
                        if let Ok(h) = tiny_http::Header::from_bytes(name, value) {
                            http.add_header(h);
                        }
                    }
                    let _ = request.respond(http);
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(server)
}

fn handle(
    request: &mut tiny_http::Request,
    ctx: &egui::Context,
    tx: &Sender<ApiCommand>,
    key: &Arc<Mutex<String>>,
    revision: &Arc<AtomicU64>,
    changes: &Arc<Mutex<ChangeLog>>,
    grants: &Arc<Mutex<Vec<crate::plugins::Grant>>>,
) -> ApiResponse {
    let method = request.method().clone();
    let raw_url = request.url().to_string();
    let (path, query) = match raw_url.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (raw_url, String::new()),
    };

    // CORS preflight: answer OPTIONS without auth (the browser sends no key on
    // preflight). The actual CORS headers are attached to every response in the
    // serve loop, so a browser extension / bookmarklet can call the API.
    if method == Method::Options {
        return ApiResponse { status: 204, body: String::new() };
    }

    // Everything but health requires a credential: the instance's own key, or a
    // token minted for a plugin. A plugin token carries a scope, and the half of
    // it that can be judged without the document — read-only — is enforced right
    // here, before the request is ever queued for the app.
    let is_health = method == Method::Get && path == "/api/health";
    let mut scope: Option<crate::plugins::Scope> = None;
    if !is_health {
        let configured = key.lock().map(|k| k.clone()).unwrap_or_default();
        if configured.is_empty() {
            return ApiResponse::err(403, "API disabled: set a key in Settings");
        }
        let presented = request_key(request);
        match presented.as_deref() {
            Some(k) if k == configured => {} // the instance key: unrestricted
            Some(k) => {
                let grant = grants.lock().ok().and_then(|g| {
                    g.iter().find(|g: &&crate::plugins::Grant| g.token == k).cloned()
                });
                match grant {
                    Some(g) => {
                        // GET and the CORS preflight are reads; everything else
                        // changes something.
                        let is_read = method == Method::Get;
                        if !g.scope.allows_method(is_read) {
                            return ApiResponse::err(
                                403,
                                &format!(
                                    "plugin '{}' has read-only access to this document",
                                    g.plugin
                                ),
                            );
                        }
                        scope = Some(g.scope);
                    }
                    None => return ApiResponse::err(401, "missing or invalid API key"),
                }
            }
            None => return ApiResponse::err(401, "missing or invalid API key"),
        }
    }

    // Long-poll for live updates: block until the document's revision differs
    // from `rev` (or ~25s elapse), then return the current revision. Clients loop
    // on this — passing back the `rev` they got — to be woken the moment anything
    // changes, instead of polling on a timer. Runs on its own thread (see serve),
    // so it never blocks other requests, and reads the counter without touching
    // the document, so no UI-thread round-trip.
    if method == Method::Get && path == "/api/wait" {
        let since = query_get(&query, "rev").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let start = Instant::now();
        loop {
            let cur = revision.load(Ordering::Relaxed);
            // `epoch` rides along so a client that reconnects after a restart
            // can tell its stored `rev` is meaningless. Added, never replacing
            // the existing fields, so older clients are unaffected.
            let epoch = changes.lock().map(|c| c.epoch()).unwrap_or(0);
            if cur != since {
                return ApiResponse::ok(json!({ "rev": cur, "changed": true, "epoch": epoch }));
            }
            if start.elapsed() > Duration::from_secs(25) {
                return ApiResponse::ok(json!({ "rev": cur, "changed": false, "epoch": epoch }));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    // Served straight off the shared log: it never touches the Document, so
    // unlike the app-intercepted routes it needs no UI-thread round-trip, and it
    // still answers while the UI is busy with a long save.
    if method == Method::Get && path == "/api/changes" {
        let since = query_get(&query, "since").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        let limit = query_get(&query, "limit")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(500)
            .clamp(1, 5000);
        let cur = revision.load(Ordering::Relaxed);
        let Ok(log) = changes.lock() else {
            return ApiResponse::err(500, "change log unavailable");
        };
        let (list, truncated) = log.since(since, limit);
        return ApiResponse::ok(json!({
            "epoch": log.epoch(),
            "rev": cur,
            "since": since,
            "count": list.len(),
            "retained": log.len(),
            "oldest": log.oldest(),
            "truncated": truncated,
            "changes": list,
        }));
    }

    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);

    let req = match route(&method, &path, &query, &body) {
        Ok(r) => r,
        Err((code, msg)) => return ApiResponse::err(code, &msg),
    };

    let (rtx, rrx) = std::sync::mpsc::sync_channel::<ApiResponse>(1);
    if tx.send(ApiCommand { req, resp: rtx, scope }).is_err() {
        return ApiResponse::err(503, "app not accepting requests");
    }
    ctx.request_repaint(); // wake the UI thread to process the command
    match rrx.recv_timeout(Duration::from_secs(5)) {
        Ok(r) => r,
        Err(_) => ApiResponse::err(504, "timed out waiting for the app"),
    }
}

fn request_key(request: &tiny_http::Request) -> Option<String> {
    for h in request.headers() {
        let field = h.field.as_str().as_str().to_ascii_lowercase();
        if field == "x-api-key" {
            return Some(h.value.as_str().to_string());
        }
        if field == "authorization" {
            if let Some(tok) = h.value.as_str().strip_prefix("Bearer ") {
                return Some(tok.to_string());
            }
        }
    }
    None
}

fn route(method: &Method, path: &str, query: &str, body: &str) -> Result<ApiRequest, (u16, String)> {
    let seg: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match (method, seg.as_slice()) {
        (Method::Get, ["api", "health"]) => Ok(ApiRequest::Health),
        (Method::Get, ["api", "tree"]) => Ok(ApiRequest::Tree),
        (Method::Get, ["api", "nodes"]) => Ok(ApiRequest::ListNodes),
        (Method::Post, ["api", "nodes"]) => {
            let i: CreateNodeInput = parse(body)?;
            Ok(ApiRequest::CreateNode { parent: i.parent, title: i.title })
        }
        (Method::Get, ["api", "nodes", id]) => Ok(ApiRequest::GetNode(pid(id)?)),
        (Method::Patch, ["api", "nodes", id]) => {
            let i: UpdateNodeInput = parse(body)?;
            Ok(ApiRequest::UpdateNode { id: pid(id)?, title: i.title, color: i.color, bg: i.bg })
        }
        (Method::Delete, ["api", "nodes", id]) => Ok(ApiRequest::DeleteNode(pid(id)?)),
        (Method::Post, ["api", "nodes", id, "move"]) => {
            let mv: MoveNodeInput = parse(body)?;
            Ok(ApiRequest::MoveNode { id: pid(id)?, mv })
        }
        (Method::Post, ["api", "nodes", id, "expand"]) => {
            let i: ExpandInput = parse(body)?;
            Ok(ApiRequest::SetExpanded { id: pid(id)?, expanded: i.expanded, recursive: i.recursive })
        }
        (Method::Get, ["api", "nodes", id, "backlinks"]) => Ok(ApiRequest::Backlinks(pid(id)?)),
        (Method::Get, ["api", "nodes", id, "cards"]) => Ok(ApiRequest::ListCards(pid(id)?)),
        (Method::Get, ["api", "nodes", nid, "cards", cid]) => {
            Ok(ApiRequest::GetCard { node: pid(nid)?, card: pid(cid)? })
        }
        (Method::Post, ["api", "nodes", id, "cards"]) => {
            let input: AddCardInput = parse(body)?;
            Ok(ApiRequest::AddCard { node: pid(id)?, input })
        }
        (Method::Patch, ["api", "nodes", nid, "cards", cid]) => {
            let patch: UpdateCardInput = parse(body)?;
            Ok(ApiRequest::UpdateCard { node: pid(nid)?, card: pid(cid)?, patch })
        }
        (Method::Delete, ["api", "nodes", nid, "cards", cid]) => {
            Ok(ApiRequest::DeleteCard { node: pid(nid)?, card: pid(cid)? })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "move"]) => {
            let mv: MoveCardInput = parse(body)?;
            Ok(ApiRequest::MoveCard { node: pid(nid)?, card: pid(cid)?, mv })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "property"]) => {
            #[derive(Deserialize)]
            struct PropInput {
                key: String,
                value: String,
            }
            let i: PropInput = parse(body)?;
            Ok(ApiRequest::SetCardProperty { node: pid(nid)?, card: pid(cid)?, key: i.key, value: i.value })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "dock"]) => {
            let i: DockInput = parse(body)?;
            Ok(ApiRequest::DockCard { node: pid(nid)?, card: pid(cid)?, anchor: i.anchor })
        }
        (Method::Delete, ["api", "nodes", nid, "cards", cid, "dock"]) => {
            Ok(ApiRequest::DetachCard { node: pid(nid)?, card: pid(cid)? })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "group"]) => {
            let i: GroupCardInput = parse(body)?;
            Ok(ApiRequest::SetCardGroup { node: pid(nid)?, card: pid(cid)?, group: Some(i.group) })
        }
        (Method::Delete, ["api", "nodes", nid, "cards", cid, "group"]) => {
            Ok(ApiRequest::SetCardGroup { node: pid(nid)?, card: pid(cid)?, group: None })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "chart"]) => {
            let node: NodeId = nid.parse().map_err(|_| (400, format!("bad node id: {nid}")))?;
            let card: u64 = cid.parse().map_err(|_| (400, format!("bad card id: {cid}")))?;
            let i: ChartInput = parse(body)?;
            Ok(ApiRequest::SetChart { node, card, spec: Some(i) })
        }
        (Method::Delete, ["api", "nodes", nid, "cards", cid, "chart"]) => {
            let node: NodeId = nid.parse().map_err(|_| (400, format!("bad node id: {nid}")))?;
            let card: u64 = cid.parse().map_err(|_| (400, format!("bad card id: {cid}")))?;
            Ok(ApiRequest::SetChart { node, card, spec: None })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "table"]) => {
            let op: TableOpInput = parse(body)?;
            Ok(ApiRequest::TableOp { node: pid(nid)?, card: pid(cid)?, op })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "sketch"]) => {
            let op: SketchOpInput = parse(body)?;
            Ok(ApiRequest::SketchOp { node: pid(nid)?, card: pid(cid)?, op })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "images"]) => {
            let i: AddImageInput = parse(body)?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(i.data_base64.trim())
                .map_err(|e| (400, format!("invalid base64 image data: {e}")))?;
            Ok(ApiRequest::AddImage { node: pid(nid)?, card: pid(cid)?, name: i.name, bytes })
        }
        (Method::Delete, ["api", "nodes", nid, "cards", cid, "images", idx]) => {
            let index = idx.parse::<usize>().map_err(|_| (400, format!("bad index: {idx}")))?;
            Ok(ApiRequest::RemoveImage { node: pid(nid)?, card: pid(cid)?, index })
        }
        (Method::Get, ["api", "nodes", nid, "cards", cid, "images", idx]) => {
            let index = idx.parse::<usize>().map_err(|_| (400, format!("bad index: {idx}")))?;
            Ok(ApiRequest::GetImage { node: pid(nid)?, card: pid(cid)?, index })
        }
        (Method::Get, ["api", "nodes", id, "groups"]) => Ok(ApiRequest::ListGroups(pid(id)?)),
        (Method::Post, ["api", "nodes", id, "groups"]) => {
            let i: CreateGroupInput = parse(body)?;
            Ok(ApiRequest::CreateGroup { node: pid(id)?, cards: i.cards, title: i.title })
        }
        (Method::Patch, ["api", "nodes", nid, "groups", gid]) => {
            let i: UpdateGroupInput = parse(body)?;
            Ok(ApiRequest::UpdateGroup { node: pid(nid)?, group: pid(gid)?, title: i.title, color: i.color })
        }
        (Method::Delete, ["api", "nodes", nid, "groups", gid]) => {
            Ok(ApiRequest::DeleteGroup { node: pid(nid)?, group: pid(gid)? })
        }
        (Method::Post, ["api", "nodes", id, "autosort"]) => Ok(ApiRequest::Autosort(pid(id)?)),
        (Method::Get, ["api", "export"]) => {
            Ok(ApiRequest::Export(query_get(query, "format").unwrap_or_else(|| "markdown".into())))
        }
        (Method::Get, ["api", "search"]) => {
            Ok(ApiRequest::Search(query_get(query, "q").unwrap_or_default()))
        }
        (Method::Get, ["api", "graph"]) => Ok(ApiRequest::Graph),
        (Method::Get, ["api", "tags"]) => match query_get(query, "name") {
            Some(name) => Ok(ApiRequest::TagCards(name)),
            None => Ok(ApiRequest::Tags),
        },
        (Method::Get, ["api", "properties"]) => match query_get(query, "key") {
            Some(key) => Ok(ApiRequest::PropertyCards { key, value: query_get(query, "value") }),
            None => Ok(ApiRequest::PropertyKeys),
        },
        (Method::Get, ["api", "query"]) => Ok(ApiRequest::QueryCards {
            tag: query_get(query, "tag"),
            key: query_get(query, "key"),
            value: query_get(query, "value"),
            text: query_get(query, "text"),
        }),
        (Method::Get, ["api", "tasks"]) => Ok(ApiRequest::Tasks {
            include_done: query_get(query, "all").as_deref() == Some("true"),
            project: match query_get(query, "project") {
                Some(v) => Some(
                    v.parse().map_err(|_| (400, format!("bad project node id: {v}")))?,
                ),
                None => None,
            },
        }),
        (Method::Get, ["api", "kanban"]) => Ok(ApiRequest::Kanban {
            project: match query_get(query, "project") {
                Some(v) => Some(
                    v.parse().map_err(|_| (400, format!("bad project node id: {v}")))?,
                ),
                None => None,
            },
        }),
        (Method::Get, ["api", "instance"]) => Ok(ApiRequest::Instance),
        (Method::Get, ["api", "backup"]) => Ok(ApiRequest::BackupStatus),
        (Method::Post, ["api", "backup", "run"]) => Ok(ApiRequest::BackupRun),
        (Method::Post, ["api", "ocr"]) => Ok(ApiRequest::OcrAll),
        (Method::Get, ["api", "history"]) => Ok(ApiRequest::HistoryList),
        (Method::Post, ["api", "history", "restore"]) => {
            #[derive(Deserialize)]
            struct RestoreInput {
                file: String,
            }
            let i: RestoreInput = parse(body)?;
            Ok(ApiRequest::HistoryRestore(i.file))
        }
        (Method::Get, ["api", "templates"]) => Ok(ApiRequest::TemplateList),
        (Method::Post, ["api", "templates", "rebuild"]) => Ok(ApiRequest::TemplateRebuild),
        (Method::Post, ["api", "templates"]) => {
            #[derive(Deserialize)]
            struct RegInput {
                node: NodeId,
                card: u64,
                #[serde(default)]
                title: Option<String>,
            }
            let i: RegInput = parse(body)?;
            Ok(ApiRequest::TemplateRegister { node: i.node, card: i.card, title: i.title })
        }
        (Method::Post, ["api", "templates", idx, "insert"]) => {
            #[derive(Deserialize)]
            struct InsInput {
                node: NodeId,
                #[serde(default)]
                pos: Option<[f32; 2]>,
            }
            let index: usize =
                idx.parse().map_err(|_| (400, format!("bad template index: {idx}")))?;
            let i: InsInput = parse(body)?;
            Ok(ApiRequest::TemplateInsert { index, node: i.node, pos: i.pos })
        }
        (Method::Post, ["api", "templates", idx, "update"]) => {
            #[derive(Deserialize)]
            struct UpdInput {
                node: NodeId,
                card: u64,
                #[serde(default)]
                title: Option<String>,
            }
            let index: usize =
                idx.parse().map_err(|_| (400, format!("bad template index: {idx}")))?;
            let i: UpdInput = parse(body)?;
            Ok(ApiRequest::TemplateUpdate { index, node: i.node, card: i.card, title: i.title })
        }
        (Method::Delete, ["api", "templates", idx]) => {
            let index: usize =
                idx.parse().map_err(|_| (400, format!("bad template index: {idx}")))?;
            Ok(ApiRequest::TemplateDelete(index))
        }
        _ => Err((404, format!("no route for {:?} {}", method, path))),
    }
}

/// Today as days since the Unix epoch (UTC), for bucketing due dates.
/// Today as days since 1970-01-01, **in the machine's local timezone**.
///
/// `due:: 2026-08-15` is a calendar date the user wrote while looking at their
/// own calendar, so "today" has to be their calendar's today. Dividing the UTC
/// clock by 86,400 made the agenda jump a day early every evening west of
/// Greenwich — at 17:00 PDT it is already tomorrow in UTC, so a task due
/// tomorrow was bucketed as due today.
pub fn today_days() -> i64 {
    // Format the local date and run it through the very same parser that reads
    // `due::`, so "today" and a due date can never disagree about what a
    // calendar day is.
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    crate::model::parse_ymd(&today).unwrap_or(0)
}

/// Which agenda bucket a due date falls in, relative to `today` (both in days).
pub fn task_bucket(due_days: Option<i64>, today: i64) -> &'static str {
    match due_days {
        None => "nodate",
        Some(d) if d < today => "overdue",
        Some(d) if d == today => "today",
        Some(d) if d <= today + 7 => "week",
        Some(_) => "later",
    }
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, (u16, String)> {
    serde_json::from_str(body).map_err(|e| (400, format!("invalid JSON body: {e}")))
}

/// Lenient `color` deserializer for the API: accepts an `[r,g,b]` array
/// (0–255), a hex string (`"#ef4444"`, `"ef4444"`, `"#e44"`), or a common color
/// name (`"red"`, `"green"`, …) — so agents can set colors however they phrase
/// it. `null` / absent → `None`. Used on every `color` field the API accepts.
fn de_color_opt<'de, D>(d: D) -> Result<Option<[u8; 3]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Value::deserialize(d)?;
    color_from_value(&v).map_err(serde::de::Error::custom)
}

/// Deserialize a present `parent` field into `Some(..)` so the handler can tell
/// it apart from an omitted one (which `#[serde(default)]` leaves as `None`):
/// `parent: null` → `Some(None)` (top level), `parent: 5` → `Some(Some(5))`.
fn de_parent_field<'de, D>(d: D) -> Result<Option<Option<NodeId>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<NodeId>::deserialize(d)?))
}

fn color_from_value(v: &Value) -> Result<Option<[u8; 3]>, String> {
    match v {
        Value::Null => Ok(None),
        Value::Array(a) => {
            if a.len() != 3 {
                return Err(format!("color array must be [r,g,b], got {} items", a.len()));
            }
            let mut out = [0u8; 3];
            for (i, x) in a.iter().enumerate() {
                let n = x
                    .as_u64()
                    .filter(|n| *n <= 255)
                    .ok_or_else(|| "color components must be integers 0–255".to_string())?;
                out[i] = n as u8;
            }
            Ok(Some(out))
        }
        Value::String(s) => {
            color_from_str(s).map(Some).ok_or_else(|| format!("unrecognized color: {s:?}"))
        }
        other => Err(format!("color must be [r,g,b] or a name/hex string, got {other}")),
    }
}

/// Parse a hex (`#rgb` / `#rrggbb`, `#` optional) or named color into RGB.
fn color_from_str(s: &str) -> Option<[u8; 3]> {
    // Named colors match the shared swatch palette (crate::model::SWATCHES).
    let named: Option<[u8; 3]> = match s.trim().to_ascii_lowercase().as_str() {
        "red" => Some([0xef, 0x44, 0x44]),
        "orange" => Some([0xf9, 0x73, 0x16]),
        "amber" => Some([0xf5, 0x9e, 0x0b]),
        "yellow" => Some([0xea, 0xb3, 0x08]),
        "lime" => Some([0x84, 0xcc, 0x16]),
        "green" => Some([0x22, 0xc5, 0x5e]),
        "teal" => Some([0x14, 0xb8, 0xa6]),
        "cyan" => Some([0x06, 0xb6, 0xd4]),
        "blue" => Some([0x3b, 0x82, 0xf6]),
        "indigo" => Some([0x63, 0x66, 0xf1]),
        "purple" | "violet" => Some([0x8b, 0x5c, 0xf6]),
        "pink" | "magenta" => Some([0xec, 0x48, 0x99]),
        "slate" | "gray" | "grey" => Some([0x64, 0x74, 0x8b]),
        "stone" => Some([0x78, 0x71, 0x6c]),
        "white" => Some([0xff, 0xff, 0xff]),
        "black" => Some([0x1e, 0x1e, 0x1e]),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let h = s.trim().strip_prefix('#').unwrap_or_else(|| s.trim());
    match h.len() {
        3 => {
            let mut out = [0u8; 3];
            for (i, ch) in h.chars().enumerate() {
                let d = ch.to_digit(16)? as u8;
                out[i] = d * 16 + d;
            }
            Some(out)
        }
        6 => {
            let mut out = [0u8; 3];
            for i in 0..3 {
                out[i] = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()?;
            }
            Some(out)
        }
        _ => None,
    }
}

fn pid(s: &str) -> Result<u64, (u16, String)> {
    s.parse::<u64>().map_err(|_| (400, format!("bad id: {s}")))
}

fn query_get(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        pair.split_once('=')
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// --- request processing (UI thread) -----------------------------------------

/// Apply a request to the document. Returns `(document_changed, response)`.
pub fn process(doc: &mut Document, req: ApiRequest) -> (bool, ApiResponse) {
    match req {
        ApiRequest::Health => (false, ApiResponse::ok(json!({ "status": "ok", "app": "trellis" }))),
        ApiRequest::Tree => (false, ApiResponse::ok(json!({ "roots": tree_nodes(doc, &doc.roots) }))),
        ApiRequest::ListNodes => {
            let list: Vec<Value> = doc
                .nodes
                .values()
                .map(|n| {
                    json!({
                        "id": n.id,
                        "title": n.title,
                        "parent": n.parent,
                        "children": n.children,
                        "cards": n.cards.len(),
                    })
                })
                .collect();
            (false, ApiResponse::ok(json!({ "nodes": list })))
        }
        ApiRequest::GetNode(id) => match doc.nodes.get(&id) {
            Some(n) => (false, ApiResponse::ok(node_json(n))),
            None => (false, ApiResponse::err(404, "node not found")),
        },
        ApiRequest::ListCards(id) => match doc.nodes.get(&id) {
            Some(n) => {
                let cards: Vec<Value> = n.cards.iter().map(card_json).collect();
                (false, ApiResponse::ok(json!({ "cards": cards })))
            }
            None => (false, ApiResponse::err(404, "node not found")),
        },
        ApiRequest::GetCard { node, card } => match doc.card(node, card) {
            Some(c) => (false, ApiResponse::ok(card_json(c))),
            None => (false, ApiResponse::err(404, "card not found")),
        },
        ApiRequest::CreateNode { parent, title } => {
            if let Some(p) = parent {
                if !doc.nodes.contains_key(&p) {
                    return (false, ApiResponse::err(400, "parent node not found"));
                }
            }
            let id = doc.add_node(parent, title);
            (true, ApiResponse::created(json!({ "id": id })))
        }
        ApiRequest::UpdateNode { id, title, color, bg } => match doc.nodes.get_mut(&id) {
            Some(n) => {
                if let Some(t) = title {
                    n.title = t;
                }
                if let Some(c) = color {
                    n.color = Some(c);
                }
                if let Some(c) = bg {
                    n.bg = Some(c);
                }
                (true, ApiResponse::ok(json!({ "id": id })))
            }
            None => (false, ApiResponse::err(404, "node not found")),
        },
        ApiRequest::DeleteNode(id) => {
            if !doc.nodes.contains_key(&id) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            doc.remove_node(id);
            (true, ApiResponse::ok(json!({ "deleted": id })))
        }
        ApiRequest::MoveNode { id, mv } => {
            if !doc.nodes.contains_key(&id) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            if mv.before.is_none()
                && mv.after.is_none()
                && mv.index.is_none()
                && mv.to.is_none()
                && mv.parent.is_none()
            {
                return (
                    false,
                    ApiResponse::err(
                        400,
                        "specify a placement: before, after, index (+parent), or to:top|bottom (+parent)",
                    ),
                );
            }
            let moved = if let Some(t) = mv.before.or(mv.after) {
                if !doc.nodes.contains_key(&t) {
                    return (false, ApiResponse::err(400, "target node not found"));
                }
                doc.reorder(id, t, mv.before.is_some())
            } else {
                let parent = match mv.parent {
                    Some(p) => p, // present: Some(None)=top level, Some(Some(x))=that node
                    None => doc.nodes.get(&id).and_then(|n| n.parent), // keep current
                };
                if let Some(p) = parent {
                    if !doc.nodes.contains_key(&p) {
                        return (false, ApiResponse::err(400, "parent node not found"));
                    }
                }
                let index = if let Some(i) = mv.index {
                    i
                } else {
                    match mv.to.as_deref() {
                        Some("top") => 0,
                        // Default (parent-only reparent, or to:bottom) appends.
                        Some("bottom") | None => usize::MAX,
                        Some(other) => {
                            return (
                                false,
                                ApiResponse::err(400, &format!("bad 'to' value {other:?} (use \"top\" or \"bottom\")")),
                            );
                        }
                    }
                };
                doc.move_node(id, parent, index)
            };
            if !moved {
                return (
                    false,
                    ApiResponse::err(400, "move rejected (no-op, or would nest a node inside its own subtree)"),
                );
            }
            let n = &doc.nodes[&id];
            let index = match n.parent {
                Some(p) => doc.nodes[&p].children.iter().position(|x| *x == id),
                None => doc.roots.iter().position(|x| *x == id),
            };
            (true, ApiResponse::ok(json!({ "id": id, "parent": n.parent, "index": index })))
        }
        ApiRequest::SetExpanded { id, expanded, recursive } => {
            if !doc.nodes.contains_key(&id) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            let changed = if recursive {
                doc.set_subtree_expanded(id, expanded, true)
            } else {
                let n = doc.nodes.get_mut(&id).unwrap();
                let c = usize::from(n.expanded != expanded);
                n.expanded = expanded;
                c
            };
            (changed > 0, ApiResponse::ok(json!({ "id": id, "expanded": expanded, "changed": changed })))
        }
        ApiRequest::AddCard { node, input } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            let kind = match input.kind.as_str() {
                "code" => CardKind::Code { lang: input.lang.clone().unwrap_or_else(|| "text".into()) },
                "checklist" => CardKind::Checklist {
                    items: input
                        .items
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|i| ChecklistItem { done: i.done, text: i.text })
                        .collect(),
                },
                "table" => {
                    let mut t = match input.rows.clone() {
                        Some(rows) if !rows.is_empty() => crate::model::TableData::from_values(rows),
                        _ => crate::model::TableData::empty(3, 3),
                    };
                    if let Some(h) = input.header {
                        t.header = h;
                    }
                    CardKind::Table { table: t }
                }
                "image" => CardKind::Image {
                    data: Vec::new(),
                    name: input.title.clone(),
                    extra: Vec::new(),
                    ocr: String::new(),
                },
                "sketch" => CardKind::Sketch { strokes: Vec::new() },
                _ => CardKind::Text,
            };
            let pos = input
                .pos
                .map(|[x, y]| egui::pos2(x, y))
                .unwrap_or_else(|| egui::pos2(40.0, 40.0));
            let fit = input.fit;
            let img_name = input.title.clone();
            match doc.add_card(node, pos, kind) {
                Some(cid) => {
                    if let Some(c) = doc.card_mut(node, cid) {
                        c.title = input.title;
                        c.body = input.body;
                        c.editing = false;
                        // The body is filled in by the app's refresh pass; the
                        // request only names the file.
                        c.source = input.source.filter(|s| !s.trim().is_empty());
                        if let Some(col) = input.color {
                            c.color = col;
                        }
                        if let Some([w, h]) = input.size {
                            c.size = egui::vec2(w, h).max(egui::vec2(80.0, 60.0));
                        }
                        if let Some(fs) = input.font_scale {
                            c.font_scale = fs.clamp(0.25, 4.0);
                        }
                    }
                    // Optional initial image bytes for an image card.
                    if let Some(b64) = input.image_base64 {
                        if let Ok(bytes) =
                            base64::engine::general_purpose::STANDARD.decode(b64.trim())
                        {
                            doc.add_image(node, cid, bytes, img_name);
                        }
                    }
                    // Optional inline images embedded in a text card's body.
                    if let Some(list) = input.inline_images {
                        for (i, b64) in list.iter().enumerate() {
                            if let Ok(bytes) =
                                base64::engine::general_purpose::STANDARD.decode(b64.trim())
                            {
                                doc.add_inline_image(node, cid, bytes, format!("inline-{i}"));
                            }
                        }
                    }
                    // Fit to content last, once body/items/images are all set.
                    if fit {
                        if let Some(c) = doc.card_mut(node, cid) {
                            if let Some(sz) = c.fit_size() {
                                c.size = sz;
                            }
                        }
                    }
                    (true, ApiResponse::created(json!({ "id": cid })))
                }
                None => (false, ApiResponse::err(404, "node not found")),
            }
        }
        ApiRequest::UpdateCard { node, card, patch } => match doc.card_mut(node, card) {
            Some(c) => {
                if let Some(t) = patch.title {
                    c.title = t;
                }
                if let Some(b) = patch.body {
                    // A mirrored body belongs to the file. Silently accepting an
                    // edit that the next refresh overwrites would look like data
                    // loss, so it's refused rather than ignored.
                    if c.source.is_some() {
                        return (
                            false,
                            ApiResponse::err(
                                409,
                                "this card mirrors a file — its body is read-only. \
                                 Send \"source\": \"\" to detach it first.",
                            ),
                        );
                    }
                    c.body = b;
                }
                if let Some(s) = patch.source {
                    let s = s.trim().to_string();
                    if s.is_empty() {
                        // Detach: keep the text that's there, drop the link.
                        c.source = None;
                        c.source_mtime = None;
                        c.source_error = None;
                    } else {
                        c.source = Some(s);
                        // Force the next poll to read it.
                        c.source_mtime = None;
                    }
                }
                if let Some(col) = patch.color {
                    c.color = col;
                }
                if let Some(fs) = patch.font_scale {
                    c.font_scale = fs.clamp(0.25, 4.0);
                }
                // Convert to another kind first, so kind-specific fields below
                // (lang/items/rows/header) land in the new kind. Existing
                // body/items/table content is preserved where it stays valid.
                if let Some(k) = &patch.kind {
                    let new = match k.as_str() {
                        "text" => Some(CardKind::Text),
                        "code" => Some(CardKind::Code { lang: "text".into() }),
                        "checklist" => Some(CardKind::Checklist { items: Vec::new() }),
                        "table" => {
                            Some(CardKind::Table { table: crate::model::TableData::empty(3, 3) })
                        }
                        "image" => Some(CardKind::Image {
                            data: Vec::new(),
                            name: c.title.clone(),
                            extra: Vec::new(),
                            ocr: String::new(),
                        }),
                        "sketch" => Some(CardKind::Sketch { strokes: Vec::new() }),
                        _ => None,
                    };
                    if let Some(nk) = new {
                        if std::mem::discriminant(&nk) != std::mem::discriminant(&c.kind) {
                            c.kind = nk;
                        }
                    }
                }
                if let Some(h) = patch.header {
                    if let CardKind::Table { table } = &mut c.kind {
                        table.header = h;
                    }
                }
                if let Some(lang) = patch.lang {
                    if let CardKind::Code { lang: l } = &mut c.kind {
                        *l = lang;
                    }
                }
                if let Some([x, y]) = patch.pos {
                    c.pos = egui::pos2(x, y);
                }
                if let Some([w, h]) = patch.size {
                    c.size = egui::vec2(w, h).max(egui::vec2(80.0, 60.0));
                }
                if let Some(rows) = patch.rows {
                    if let CardKind::Table { table } = &mut c.kind {
                        // `rows` replaces the *data*; the chart is a view setting
                        // on that data, so refilling a table must not silently
                        // turn its chart back into a grid.
                        let chart = table.chart.take();
                        *table = crate::model::TableData::from_values(rows);
                        table.chart = chart;
                    }
                }
                if let Some(items) = patch.items {
                    if let CardKind::Checklist { items: it } = &mut c.kind {
                        *it = items
                            .into_iter()
                            .map(|i| ChecklistItem { done: i.done, text: i.text })
                            .collect();
                    }
                }
                // Replacement inline images for a text card (base64 → entries).
                if let Some(list) = patch.inline_images {
                    c.inline_images.clear();
                    for (i, b64) in list.iter().enumerate() {
                        if let Ok(bytes) =
                            base64::engine::general_purpose::STANDARD.decode(b64.trim())
                        {
                            c.inline_images.push(crate::model::ImageEntry {
                                data: bytes,
                                name: format!("inline-{i}"),
                            });
                        }
                    }
                }
                // Fit to content last, once every other field has been applied.
                if patch.fit {
                    if let Some(sz) = c.fit_size() {
                        c.size = sz;
                    }
                }
                (true, ApiResponse::ok(card_json(c)))
            }
            None => (false, ApiResponse::err(404, "card not found")),
        },
        ApiRequest::DeleteCard { node, card } => {
            let existed = doc
                .nodes
                .get(&node)
                .map(|n| n.cards.iter().any(|c| c.id == card))
                .unwrap_or(false);
            if !existed {
                return (false, ApiResponse::err(404, "card not found"));
            }
            doc.remove_card(node, card);
            (true, ApiResponse::ok(json!({ "deleted": card })))
        }
        ApiRequest::MoveCard { node, card, mv } => {
            let Some(cur) = doc.card_index(node, card) else {
                return (false, ApiResponse::err(404, "card not found"));
            };
            // Moving to another basket is its own thing: the card leaves this
            // node entirely, so the within-basket ordering fields don't apply.
            if let Some(target) = mv.node {
                if target == node {
                    return (false, ApiResponse::err(400, "card is already in that node"));
                }
                if !doc.nodes.contains_key(&target) {
                    return (false, ApiResponse::err(404, "target node not found"));
                }
                let p = mv.pos.map(|[x, y]| egui::pos2(x, y));
                return match doc.move_card_to_node(node, card, target, p) {
                    Some(id) => (
                        true,
                        ApiResponse::ok(json!({ "card": id, "node": target, "moved": true })),
                    ),
                    None => (false, ApiResponse::err(500, "could not move the card")),
                };
            }
            if mv.before.is_none() && mv.after.is_none() && mv.index.is_none() && mv.to.is_none() {
                return (
                    false,
                    ApiResponse::err(400, "specify a placement: before, after, index, or to:front|back"),
                );
            }
            let index = if let Some(t) = mv.before.or(mv.after) {
                let Some(tpos) = doc.card_index(node, t) else {
                    return (false, ApiResponse::err(400, "target card not found"));
                };
                // move_card lifts the card out first, so a target after it shifts down one.
                let tpos = if cur < tpos { tpos - 1 } else { tpos };
                if mv.after.is_some() { tpos + 1 } else { tpos }
            } else if let Some(i) = mv.index {
                i
            } else {
                match mv.to.as_deref() {
                    Some("back") => 0,
                    Some("front") => usize::MAX,
                    other => {
                        return (
                            false,
                            ApiResponse::err(400, &format!("bad 'to' value {other:?} (use \"front\" or \"back\")")),
                        );
                    }
                }
            };
            doc.move_card(node, card, index);
            let idx = doc.card_index(node, card);
            (true, ApiResponse::ok(json!({ "card": card, "index": idx })))
        }
        ApiRequest::SetCardProperty { node, card, key, value } => {
            if doc.set_card_property(node, card, &key, &value) {
                (true, ApiResponse::ok(json!({ "card": card, "key": key.to_lowercase(), "value": value })))
            } else {
                (false, ApiResponse::err(404, "card not found"))
            }
        }
        ApiRequest::ListGroups(node) => match doc.nodes.get(&node) {
            Some(n) => (false, ApiResponse::ok(json!({ "groups": groups_json(n) }))),
            None => (false, ApiResponse::err(404, "node not found")),
        },
        ApiRequest::CreateGroup { node, cards, title } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            match doc.group_cards(node, &cards, title.unwrap_or_else(|| "Group".into())) {
                Some(gid) => (true, ApiResponse::created(json!({ "id": gid }))),
                None => (false, ApiResponse::err(400, "need at least two existing cards to group")),
            }
        }
        ApiRequest::UpdateGroup { node, group, title, color } => {
            if !group_exists(doc, node, group) {
                return (false, ApiResponse::err(404, "group not found"));
            }
            if let Some(t) = title {
                doc.set_group_title(node, group, t);
            }
            if let Some(c) = color {
                doc.set_group_color(node, group, c);
            }
            (true, ApiResponse::ok(json!({ "id": group })))
        }
        ApiRequest::DeleteGroup { node, group } => {
            if !group_exists(doc, node, group) {
                return (false, ApiResponse::err(404, "group not found"));
            }
            doc.ungroup(node, group);
            (true, ApiResponse::ok(json!({ "ungrouped": group })))
        }
        ApiRequest::DockCard { node, card, anchor } => {
            let both = doc
                .nodes
                .get(&node)
                .map(|n| {
                    n.cards.iter().any(|c| c.id == card) && n.cards.iter().any(|c| c.id == anchor)
                })
                .unwrap_or(false);
            if !both {
                return (false, ApiResponse::err(404, "card or anchor not found"));
            }
            doc.dock_card(node, card, anchor);
            let docked = doc.card_mut(node, card).and_then(|c| c.docked_to);
            if docked == Some(anchor) {
                (true, ApiResponse::ok(json!({ "card": card, "docked_to": docked })))
            } else {
                // dock_card refuses cycles / self-docks.
                (false, ApiResponse::err(400, "cannot dock (would form a cycle)"))
            }
        }
        ApiRequest::DetachCard { node, card } => match doc.card_mut(node, card) {
            Some(_) => {
                doc.detach_card(node, card);
                (true, ApiResponse::ok(json!({ "card": card, "docked_to": Value::Null })))
            }
            None => (false, ApiResponse::err(404, "card not found")),
        },
        ApiRequest::SetCardGroup { node, card, group } => {
            if doc.set_card_group(node, card, group) {
                let c = doc.card_mut(node, card).unwrap();
                (true, ApiResponse::ok(card_json(c)))
            } else {
                (false, ApiResponse::err(404, "card or group not found"))
            }
        }
        ApiRequest::SetChart { node, card, spec } => {
            let Some(c) = doc.card_mut(node, card) else {
                return (false, ApiResponse::err(404, "node or card not found"));
            };
            let CardKind::Table { table } = &mut c.kind else {
                return (
                    false,
                    ApiResponse::err(400, "charts are drawn from a table card's cells; convert the card to a table first"),
                );
            };
            match spec {
                None => {
                    table.chart = None;
                    (true, ApiResponse::ok(json!({ "chart": Value::Null })))
                }
                Some(i) => {
                    let Some(kind) = crate::model::ChartKind::from_key(&i.kind) else {
                        return (
                            false,
                            ApiResponse::err(400, "kind must be one of: bar, line, scatter"),
                        );
                    };
                    let cur = table.chart.clone().unwrap_or_default();
                    let spec = crate::model::ChartSpec {
                        kind,
                        label_col: i.label_col.unwrap_or(cur.label_col),
                        value_cols: i.value_cols.unwrap_or(cur.value_cols),
                        show_table: i.show_table.unwrap_or(cur.show_table),
                    };
                    table.chart = Some(spec.clone());
                    (
                        true,
                        ApiResponse::ok(json!({ "chart": {
                            "kind": spec.kind.key(),
                            "label_col": spec.label_col,
                            "value_cols": spec.value_cols,
                            "show_table": spec.show_table,
                        }})),
                    )
                }
            }
        }
        ApiRequest::TableOp { node, card, op } => {
            let ok = match op.op.as_str() {
                "set_cell" => doc.table_set_cell(
                    node,
                    card,
                    op.row.unwrap_or(0),
                    op.col.unwrap_or(0),
                    op.text.unwrap_or_default(),
                ),
                "set_bg" => {
                    let color = color_from_value(&op.color).unwrap_or(None);
                    doc.table_set_bg(node, card, op.row.unwrap_or(0), op.col.unwrap_or(0), color)
                }
                "set_fg" => {
                    let color = color_from_value(&op.color).unwrap_or(None);
                    doc.table_set_fg(node, card, op.row.unwrap_or(0), op.col.unwrap_or(0), color)
                }
                "insert_row" => doc.table_insert_row(node, card, op.at.unwrap_or(0)),
                "remove_row" => doc.table_remove_row(node, card, op.at.unwrap_or(0)),
                "insert_col" => doc.table_insert_col(node, card, op.at.unwrap_or(0)),
                "remove_col" => doc.table_remove_col(node, card, op.at.unwrap_or(0)),
                "set_col_width" => {
                    doc.table_set_col_width(node, card, op.col.unwrap_or(0), op.width.unwrap_or(110.0))
                }
                // No `col` = every column, which is the usual case: a table built
                // by an agent has every column at the 110px default.
                "autofit_cols" => doc.table_autofit_cols(node, card, op.col),
                "set_header" => doc.table_set_header(node, card, op.header.unwrap_or(true)),
                other => {
                    return (false, ApiResponse::err(400, &format!("unknown table op: {other}")));
                }
            };
            if ok {
                let c = doc.card_mut(node, card).unwrap();
                (true, ApiResponse::ok(card_json(c)))
            } else {
                (false, ApiResponse::err(400, "table op failed (not a table, or index out of range)"))
            }
        }
        ApiRequest::SketchOp { node, card, op } => {
            let ok = match op.op.as_str() {
                "add_stroke" => {
                    let color = color_from_value(&op.color).unwrap_or(None).unwrap_or([0, 0, 0]);
                    let stroke = crate::model::Stroke {
                        color,
                        width: op.width.unwrap_or(3.0),
                        points: op.points.unwrap_or_default(),
                    };
                    doc.sketch_add_stroke(node, card, stroke)
                }
                "undo" => doc.sketch_undo(node, card),
                "clear" => doc.sketch_clear(node, card),
                other => {
                    return (false, ApiResponse::err(400, &format!("unknown sketch op: {other}")));
                }
            };
            if ok {
                let c = doc.card_mut(node, card).unwrap();
                (true, ApiResponse::ok(card_json(c)))
            } else {
                (false, ApiResponse::err(400, "sketch op failed (not a sketch, or nothing to change)"))
            }
        }
        ApiRequest::AddImage { node, card, name, bytes } => {
            if doc.add_image(node, card, bytes, name) {
                let c = doc.card_mut(node, card).unwrap();
                (true, ApiResponse::created(card_json(c)))
            } else {
                (false, ApiResponse::err(404, "card not found or not an image card"))
            }
        }
        ApiRequest::RemoveImage { node, card, index } => {
            if doc.remove_image(node, card, index) {
                let c = doc.card_mut(node, card).unwrap();
                (true, ApiResponse::ok(card_json(c)))
            } else {
                (false, ApiResponse::err(404, "card/image not found or not an image card"))
            }
        }
        ApiRequest::GetImage { node, card, index } => {
            let img = doc
                .nodes
                .get(&node)
                .and_then(|n| n.cards.iter().find(|c| c.id == card))
                .map(|c| c.kind.images())
                .and_then(|imgs| imgs.get(index).map(|(d, n)| (d.to_vec(), n.to_string())));
            match img {
                Some((bytes, name)) => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    (false, ApiResponse::ok(json!({
                        "index": index,
                        "name": name,
                        "base64": b64,
                    })))
                }
                None => (false, ApiResponse::err(404, "card/image not found or not an image card")),
            }
        }
        ApiRequest::Autosort(node) => {
            if doc.autosort(node) {
                (true, ApiResponse::ok(json!({ "sorted": node })))
            } else {
                (false, ApiResponse::err(404, "node not found or has no cards"))
            }
        }
        ApiRequest::Export(format) => export_response(doc, &format),
        ApiRequest::Search(q) => {
            let hits: Vec<Value> = doc
                .search(&q)
                .into_iter()
                .map(|h| json!({ "node": h.node, "card": h.card, "node_title": h.node_title, "snippet": h.snippet }))
                .collect();
            (false, ApiResponse::ok(json!({ "hits": hits })))
        }
        ApiRequest::Tags => {
            let tags: Vec<Value> = doc
                .tag_counts()
                .into_iter()
                .map(|(tag, count)| json!({ "tag": tag, "count": count }))
                .collect();
            (false, ApiResponse::ok(json!({ "tags": tags })))
        }
        ApiRequest::TagCards(tag) => {
            let hits: Vec<Value> = doc
                .cards_with_tag(&tag)
                .into_iter()
                .map(|h| json!({ "node": h.node, "card": h.card, "node_title": h.node_title, "snippet": h.snippet }))
                .collect();
            (false, ApiResponse::ok(json!({ "tag": tag.trim_start_matches('#'), "hits": hits })))
        }
        ApiRequest::PropertyKeys => {
            let keys: Vec<Value> = doc
                .property_keys()
                .into_iter()
                .map(|(key, count)| json!({ "key": key, "count": count }))
                .collect();
            (false, ApiResponse::ok(json!({ "properties": keys })))
        }
        ApiRequest::PropertyCards { key, value } => {
            let hits: Vec<Value> = doc
                .cards_with_property(&key, value.as_deref())
                .into_iter()
                .map(|h| json!({ "node": h.node, "card": h.card, "node_title": h.node_title, "snippet": h.snippet }))
                .collect();
            (false, ApiResponse::ok(json!({ "key": key, "value": value, "hits": hits })))
        }
        ApiRequest::QueryCards { tag, key, value, text } => {
            let hits: Vec<Value> = doc
                .query_cards(tag.as_deref(), key.as_deref(), value.as_deref(), text.as_deref())
                .into_iter()
                .map(|h| json!({ "node": h.node, "card": h.card, "node_title": h.node_title, "snippet": h.snippet }))
                .collect();
            (false, ApiResponse::ok(json!({ "count": hits.len(), "hits": hits })))
        }
        ApiRequest::Tasks { include_done, project } => {
            let today = today_days();
            if let Some(p) = project {
                if !doc.nodes.contains_key(&p) {
                    return (false, ApiResponse::err(404, "project node not found"));
                }
            }
            let tasks: Vec<Value> = doc
                .tasks()
                .into_iter()
                .filter(|t| include_done || !t.done)
                // `project` accepts any node, not just a root: filtering to a
                // sub-branch is the same question asked more narrowly.
                .filter(|t| project.map_or(true, |p| doc.is_under(t.node, p)))
                .map(|t| {
                    json!({
                        "node": t.node,
                        "node_title": t.node_title,
                        "node_path": t.node_path,
                        "project": t.root,
                        "project_title": t.root_title,
                        "card": t.card,
                        "title": t.title,
                        "due": t.due,
                        "done": t.done,
                        "bucket": task_bucket(t.due_days, today),
                    })
                })
                .collect();
            (false, ApiResponse::ok(json!({ "today_days": today, "count": tasks.len(), "tasks": tasks })))
        }
        ApiRequest::Kanban { project } => {
            let today = today_days();
            if let Some(p) = project {
                if !doc.nodes.contains_key(&p) {
                    return (false, ApiResponse::err(404, "project node not found"));
                }
            }
            let columns: Vec<Value> = doc
                .cards_by_status()
                .into_iter()
                // Same semantics as /api/tasks?project= — any node, not just a root.
                .map(|(status, cards)| {
                    let cards: Vec<_> = cards
                        .into_iter()
                        .filter(|c| project.map_or(true, |p| doc.is_under(c.node, p)))
                        .collect();
                    (status, cards)
                })
                .filter(|(_, cards)| !cards.is_empty())
                .map(|(status, cards)| {
                    let cards: Vec<Value> = cards
                        .into_iter()
                        .map(|kc| {
                            json!({
                                "node": kc.node,
                                "node_title": kc.node_title,
                                "node_path": kc.node_path,
                                "project": kc.root,
                                "project_title": kc.root_title,
                                "card": kc.card,
                                "title": kc.title,
                                "due": kc.due,
                                "tags": kc.tags,
                                "color": kc.color,
                            })
                        })
                        .collect();
                    json!({ "status": status, "count": cards.len(), "cards": cards })
                })
                .collect();
            (false, ApiResponse::ok(json!({ "today_days": today, "columns": columns })))
        }
        ApiRequest::Backlinks(id) => {
            if !doc.nodes.contains_key(&id) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            let hits: Vec<Value> = doc
                .backlinks(id)
                .into_iter()
                .map(|h| json!({ "node": h.node, "card": h.card, "node_title": h.node_title, "snippet": h.snippet }))
                .collect();
            (false, ApiResponse::ok(json!({ "node": id, "count": hits.len(), "hits": hits })))
        }
        ApiRequest::Graph => {
            let (ids, edges) = doc.link_graph();
            let nodes: Vec<Value> = ids
                .iter()
                .map(|id| json!({ "id": id, "title": doc.nodes.get(id).map(|n| n.title.clone()).unwrap_or_default() }))
                .collect();
            let edges: Vec<Value> = edges.iter().map(|(u, v)| json!([u, v])).collect();
            (false, ApiResponse::ok(json!({ "nodes": nodes, "edges": edges })))
        }
        // Backup requests are intercepted and answered by the app loop (they need
        // the backup config + document file). This is only reached if that
        // interception is ever missed — report it rather than silently no-op.
        ApiRequest::Instance
        | ApiRequest::BackupStatus
        | ApiRequest::BackupRun
        | ApiRequest::HistoryList
        | ApiRequest::HistoryRestore(_)
        | ApiRequest::OcrAll
        | ApiRequest::TemplateList
        | ApiRequest::TemplateRegister { .. }
        | ApiRequest::TemplateInsert { .. }
        | ApiRequest::TemplateUpdate { .. }
        | ApiRequest::TemplateRebuild
        | ApiRequest::TemplateDelete(_) => {
            (false, ApiResponse::err(500, "request not handled by the app loop"))
        }
    }
}

fn tree_nodes(doc: &Document, ids: &[NodeId]) -> Vec<Value> {
    ids.iter()
        .filter_map(|id| doc.nodes.get(id))
        .map(|n| {
            json!({
                "id": n.id,
                "title": n.title,
                "color": n.color,
                "cards": n.cards.len(),
                "children": tree_nodes(doc, &n.children),
            })
        })
        .collect()
}

fn node_json(n: &crate::model::Node) -> Value {
    json!({
        "id": n.id,
        "title": n.title,
        "parent": n.parent,
        "children": n.children,
        "expanded": n.expanded,
        "color": n.color,
        "bg": n.bg,
        "touched": n.touched,
        "groups": groups_json(n),
        "cards": n.cards.iter().map(card_json).collect::<Vec<_>>(),
    })
}

/// JSON for a node's groups, each with its member card ids.
fn groups_json(n: &crate::model::Node) -> Vec<Value> {
    n.groups
        .iter()
        .map(|g| {
            json!({
                "id": g.id,
                "title": g.title,
                "color": g.color,
                "cards": n.cards.iter().filter(|c| c.group == Some(g.id)).map(|c| c.id).collect::<Vec<_>>(),
            })
        })
        .collect()
}

fn group_exists(doc: &Document, node: NodeId, group: GroupId) -> bool {
    doc.nodes.get(&node).map(|n| n.groups.iter().any(|g| g.id == group)).unwrap_or(false)
}

/// Export the whole document in `format`. Text formats return `content`; binary
/// formats (pdf/png/gif) return standard base64 in `base64`.
fn export_response(doc: &Document, format: &str) -> (bool, ApiResponse) {
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let resp = match format {
        "markdown" | "md" => ApiResponse::ok(json!({ "format": "markdown", "content": doc.export_markdown() })),
        "html" => ApiResponse::ok(json!({ "format": "html", "content": doc.export_html() })),
        "json" => match doc.export_json() {
            Ok(s) => ApiResponse::ok(json!({ "format": "json", "content": s })),
            Err(e) => ApiResponse::err(500, &e.to_string()),
        },
        "pdf" => match doc.export_pdf() {
            Ok(b) => ApiResponse::ok(json!({ "format": "pdf", "base64": b64(&b) })),
            Err(e) => ApiResponse::err(500, &e),
        },
        "png" => match doc.export_image(false) {
            Ok(b) => ApiResponse::ok(json!({ "format": "png", "base64": b64(&b) })),
            Err(e) => ApiResponse::err(500, &e),
        },
        "gif" => match doc.export_image(true) {
            Ok(b) => ApiResponse::ok(json!({ "format": "gif", "base64": b64(&b) })),
            Err(e) => ApiResponse::err(500, &e),
        },
        other => ApiResponse::err(400, &format!("unknown export format: {other}")),
    };
    (false, resp)
}

pub(crate) fn card_json(c: &Card) -> Value {
    let mut v = json!({
        "id": c.id,
        "title": c.title,
        "kind": c.kind.label().to_lowercase(),
        "pos": [c.pos.x, c.pos.y],
        "size": [c.size.x, c.size.y],
        "color": c.color,
        "group": c.group,
        "docked_to": c.docked_to,
        "font_scale": c.font_scale,
    });
    // Only when there is one: a card never edited since this existed reports no
    // time rather than a made-up one.
    if let Some(t) = c.touched {
        v["touched"] = json!(t);
    }
    if let Some(s) = &c.source {
        v["source"] = json!(s);
        v["source_error"] = json!(c.source_error);
    }
    let props = c.properties();
    if !props.is_empty() {
        v["properties"] = json!(props
            .iter()
            .map(|(k, val)| json!({ "key": k, "value": val }))
            .collect::<Vec<_>>());
    }
    match &c.kind {
        CardKind::Text => {
            v["body"] = json!(c.body);
            if !c.inline_images.is_empty() {
                v["inline_image_names"] = json!(c
                    .inline_images
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>());
            }
        }
        CardKind::Code { lang } => {
            v["body"] = json!(c.body);
            v["lang"] = json!(lang);
        }
        CardKind::Checklist { items } => {
            v["items"] = json!(items
                .iter()
                .map(|i| json!({ "done": i.done, "text": i.text }))
                .collect::<Vec<_>>());
        }
        CardKind::Table { table } => {
            v["header"] = json!(table.header);
            v["chart"] = match &table.chart {
                Some(c) => json!({
                    "kind": c.kind.key(),
                    "label_col": c.label_col,
                    "value_cols": c.value_cols,
                    "show_table": c.show_table,
                }),
                None => Value::Null,
            };
            v["rows"] = json!(table
                .rows
                .iter()
                .map(|row| row
                    .iter()
                    .map(|c| json!({"text": c.text, "bg": c.bg, "fg": c.fg}))
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>());
        }
        k @ CardKind::Image { ocr, .. } => {
            let images = k.images();
            v["image_name"] = json!(images.first().map(|(_, n)| *n).unwrap_or(""));
            v["image_names"] = json!(images.iter().map(|(_, n)| *n).collect::<Vec<_>>());
            v["bytes"] = json!(images.iter().map(|(d, _)| d.len()).sum::<usize>());
            v["ocr"] = json!(ocr);
        }
        CardKind::Sketch { strokes } => {
            v["strokes"] = json!(strokes
                .iter()
                .map(|s| json!({ "color": s.color, "width": s.width, "points": s.points }))
                .collect::<Vec<_>>());
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_id(resp: &ApiResponse) -> u64 {
        serde_json::from_str::<Value>(&resp.body).unwrap()["id"].as_u64().unwrap()
    }

    /// The bug this guards: `today_days()` used to divide the UTC clock, so from
    /// 17:00 PDT until midnight it returned tomorrow's date and a task due
    /// tomorrow was bucketed "today".
    #[test]
    fn today_is_the_local_calendar_day_not_the_utc_one() {
        let local = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(
            today_days(),
            crate::model::parse_ymd(&local).unwrap(),
            "today must equal the local calendar date parsed the same way due:: is"
        );

        // A task due on the local calendar's tomorrow must never read as today.
        let today = today_days();
        assert_eq!(task_bucket(Some(today), today), "today");
        assert_eq!(task_bucket(Some(today + 1), today), "week");
        assert_eq!(task_bucket(Some(today - 1), today), "overdue");

        // And it must track the local date even when UTC has already rolled over.
        let utc_days = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_secs() / 86_400) as i64)
            .unwrap();
        assert!(
            (today - utc_days).abs() <= 1,
            "local and UTC days differ by at most one"
        );
    }

    /// A property change is the entry a plugin trigger is built on, so the key
    /// and value have to survive into the log — that is the one piece of content
    /// an entry carries.
    #[test]
    fn a_property_change_records_its_key_and_value() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "Project".into());
        let c = doc.add_card(n, emath::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c).unwrap().title = "Ship it".into();

        let req = ApiRequest::SetCardProperty {
            node: n,
            card: c,
            key: "status".into(),
            value: "done".into(),
        };
        let ch = change_of(&req, &doc).expect("a property set is a change");
        assert_eq!(ch.property, Some(("status".into(), "done".into())));
        assert_eq!(ch.fields, vec!["property"]);
        assert_eq!(ch.node, Some(n));
        assert_eq!(ch.title.as_deref(), Some("Ship it"));
    }

    /// The description is taken before `process` runs precisely so a delete can
    /// still name what it deleted — afterwards there is nothing left to look up.
    #[test]
    fn a_delete_is_described_while_the_card_still_exists() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "Basket".into());
        let c = doc.add_card(n, emath::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c).unwrap().title = "Doomed".into();

        let req = ApiRequest::DeleteCard { node: n, card: c };
        let ch = change_of(&req, &doc).unwrap();
        assert_eq!(ch.title.as_deref(), Some("Doomed"));

        // And once it's gone, the same description would be nameless — which is
        // the whole reason for the ordering.
        let (_changed, _resp) = process(&mut doc, ApiRequest::DeleteCard { node: n, card: c });
        let after = change_of(&ApiRequest::DeleteCard { node: n, card: c }, &doc).unwrap();
        assert_eq!(after.title, None);
    }

    /// Only the fields actually sent are reported, so a client watching card text
    /// can ignore a recolour without fetching anything.
    #[test]
    fn an_update_reports_only_the_fields_it_was_given() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "Basket".into());
        let c = doc.add_card(n, emath::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let patch: UpdateCardInput =
            serde_json::from_str(r#"{"body":"new text","color":"red"}"#).unwrap();
        let ch = change_of(&ApiRequest::UpdateCard { node: n, card: c, patch }, &doc).unwrap();
        assert_eq!(ch.fields, vec!["body", "color"]);
        assert!(!ch.fields.contains(&"title".to_string()));
    }

    /// A card's edit has to stamp its **basket** too, or "sort baskets by latest
    /// change" would only ever notice renames — and work in a basket is editing
    /// its cards.
    #[test]
    fn touched_is_absent_until_something_changes_then_covers_card_and_basket() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "Basket".into());
        let c = doc.add_card(n, emath::pos2(0.0, 0.0), CardKind::Text).unwrap();
        assert_eq!(doc.card(n, c).unwrap().touched, None, "never edited");
        assert_eq!(doc.nodes[&n].touched, None);

        // `stamp_touched` lives in the app loop, so mimic what it writes.
        let now = crate::changelog::now_secs();
        doc.card_mut(n, c).unwrap().touched = Some(now);
        doc.nodes.get_mut(&n).unwrap().touched = Some(now);

        let v = card_json(doc.card(n, c).unwrap());
        assert_eq!(v["touched"], now);
        assert_eq!(doc.nodes[&n].touched, Some(now), "the basket counts as worked in");
    }

    /// The field must be optional in *both* directions: a document written before
    /// it existed still loads, and a document carrying it still loads in a build
    /// that doesn't know the field. Nothing here sets `deny_unknown_fields`, and
    /// this pins that — unlike the v0.74.0 image change, this is not one-way.
    #[test]
    fn documents_round_trip_with_and_without_touched() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "Basket".into());
        let c = doc.add_card(n, emath::pos2(0.0, 0.0), CardKind::Text).unwrap();

        // Untouched: the field must not even be written.
        let ron = ron::ser::to_string(&doc).unwrap();
        assert!(!ron.contains("touched"), "an unedited document gains no bytes");
        let back: Document = ron::from_str(&ron).unwrap();
        assert_eq!(back.card(n, c).unwrap().touched, None);

        doc.card_mut(n, c).unwrap().touched = Some(1_785_950_176);
        let ron = ron::ser::to_string(&doc).unwrap();
        assert!(ron.contains("touched"));
        let back: Document = ron::from_str(&ron).unwrap();
        assert_eq!(back.card(n, c).unwrap().touched, Some(1_785_950_176));
    }


    /// A mirrored body belongs to the file. Accepting a body edit that the next
    /// refresh silently overwrites would look exactly like data loss, so it is
    /// refused with a code the caller can act on.
    #[test]
    fn editing_a_mirrored_body_is_refused_not_ignored() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "Basket".into());
        let c = doc.add_card(n, emath::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c).unwrap().source = Some("/tmp/whatever.md".into());
        doc.card_mut(n, c).unwrap().body = "from the file".into();

        let patch: UpdateCardInput = serde_json::from_str(r#"{"body":"my edit"}"#).unwrap();
        let (changed, resp) = process(&mut doc, ApiRequest::UpdateCard { node: n, card: c, patch });
        assert_eq!(resp.status, 409);
        assert!(!changed);
        assert_eq!(doc.card(n, c).unwrap().body, "from the file", "left alone");
    }

    /// Detaching keeps the text that was mirrored — the point is to capture a
    /// snapshot, not to empty the card.
    #[test]
    fn detaching_a_source_keeps_the_text_and_reopens_editing() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "Basket".into());
        let c = doc.add_card(n, emath::pos2(0.0, 0.0), CardKind::Text).unwrap();
        {
            let card = doc.card_mut(n, c).unwrap();
            card.source = Some("/tmp/whatever.md".into());
            card.source_mtime = Some(123);
            card.source_error = Some("gone".into());
            card.body = "mirrored text".into();
        }
        let patch: UpdateCardInput = serde_json::from_str(r#"{"source":""}"#).unwrap();
        let (changed, resp) = process(&mut doc, ApiRequest::UpdateCard { node: n, card: c, patch });
        assert_eq!(resp.status, 200);
        assert!(changed);
        let card = doc.card(n, c).unwrap();
        assert_eq!(card.source, None);
        assert_eq!(card.source_mtime, None);
        assert_eq!(card.source_error, None, "a stale error must not outlive the link");
        assert_eq!(card.body, "mirrored text", "the snapshot stays");

        // …and a body edit is accepted again.
        let patch: UpdateCardInput = serde_json::from_str(r#"{"body":"mine now"}"#).unwrap();
        let (_c, resp) = process(&mut doc, ApiRequest::UpdateCard { node: n, card: c, patch });
        assert_eq!(resp.status, 200);
        assert_eq!(doc.card(n, c).unwrap().body, "mine now");
    }

    #[test]
    fn source_is_reported_on_read_only_when_set() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "Basket".into());
        let c = doc.add_card(n, emath::pos2(0.0, 0.0), CardKind::Text).unwrap();
        assert!(card_json(doc.card(n, c).unwrap()).get("source").is_none());

        doc.card_mut(n, c).unwrap().source = Some("/tmp/notes.md".into());
        doc.card_mut(n, c).unwrap().source_error = Some("no such file".into());
        let v = card_json(doc.card(n, c).unwrap());
        assert_eq!(v["source"], "/tmp/notes.md");
        assert_eq!(v["source_error"], "no such file", "a broken mirror says so");
    }

    /// The reader must refuse what it can't render rather than producing mojibake
    /// or loading a huge file into a card body.
    #[test]
    fn the_source_reader_refuses_directories_binaries_and_huge_files() {
        use crate::model::read_source;
        let dir = std::env::temp_dir().join(format!("trellis-src-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        assert!(read_source(&dir.to_string_lossy()).unwrap_err().contains("directory"));
        assert!(read_source("/definitely/not/here.md").is_err());

        let bin = dir.join("bin");
        std::fs::write(&bin, [0xff, 0xfe, 0x00, 0x01]).unwrap();
        assert!(read_source(&bin.to_string_lossy()).unwrap_err().contains("UTF-8"));

        let big = dir.join("big");
        std::fs::write(&big, vec![b'x'; (crate::model::SOURCE_MAX_BYTES + 1) as usize]).unwrap();
        assert!(read_source(&big.to_string_lossy()).unwrap_err().contains("limit"));

        let ok = dir.join("ok.md");
        std::fs::write(&ok, "# hello").unwrap();
        let (text, mtime) = read_source(&ok.to_string_lossy()).unwrap();
        assert_eq!(text, "# hello");
        assert!(mtime > 0, "a real mtime, so a poll can tell when it changes");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reads must never enter the log — a `GET` that recorded a change would wake
    /// every long-polling client for nothing.
    #[test]
    fn reads_are_not_changes() {
        let doc = Document::default();
        for req in [
            ApiRequest::Tree,
            ApiRequest::ListNodes,
            ApiRequest::Search("x".into()),
            ApiRequest::Tags,
            ApiRequest::Graph,
            ApiRequest::Kanban { project: None },
        ] {
            assert!(change_of(&req, &doc).is_none(), "a read recorded a change");
        }
    }


    /// A read-only plugin token must be refused on anything that writes. This is
    /// checked on the API thread, before the request is queued, so it holds even
    /// if the app loop is busy.
    #[test]
    fn scope_gates_writes_by_method_not_by_route() {
        use crate::plugins::Scope;
        let ro = Scope { read_only: true, subtree: None };
        assert!(ro.allows_method(true), "GET is fine");
        assert!(!ro.allows_method(false), "anything else is not");
        // An unrestricted grant writes.
        assert!(Scope::default().allows_method(false));
    }

    /// Every request that names a node must be attributable to one, or a subtree
    /// scope has holes. Anything not attributable has to be explicitly declared
    /// harmless instead of defaulting to allowed.
    #[test]
    fn every_node_bearing_request_reports_its_target() {
        let cases: Vec<(ApiRequest, Option<NodeId>)> = vec![
            (ApiRequest::GetNode(7), Some(7)),
            (ApiRequest::DeleteNode(7), Some(7)),
            (ApiRequest::ListCards(9), Some(9)),
            (ApiRequest::GetCard { node: 3, card: 1 }, Some(3)),
            (ApiRequest::DeleteCard { node: 3, card: 1 }, Some(3)),
            (ApiRequest::SetCardProperty { node: 4, card: 1, key: "k".into(), value: "v".into() }, Some(4)),
            (ApiRequest::Autosort(5), Some(5)),
            (ApiRequest::CreateNode { parent: Some(2), title: "x".into() }, Some(2)),
            // A root-level create belongs to no subtree, so it must not resolve.
            (ApiRequest::CreateNode { parent: None, title: "x".into() }, None),
            (ApiRequest::Export("markdown".into()), None),
            (ApiRequest::Search("x".into()), None),
        ];
        for (req, want) in cases {
            assert_eq!(target_node(&req), want, "wrong target for a scoped request");
        }
    }

    /// The reads a confined plugin needs to find its own basket — and nothing
    /// that would leak content from outside it. `/api/tree` and `/api/nodes`
    /// return titles and structure only, never card bodies.
    #[test]
    fn only_structural_reads_are_scope_neutral() {
        assert!(is_scope_neutral(&ApiRequest::Instance));
        assert!(is_scope_neutral(&ApiRequest::Tree));
        assert!(is_scope_neutral(&ApiRequest::ListNodes));
        // These read card *content* across the whole document, so a confined
        // token must not get them for free.
        assert!(!is_scope_neutral(&ApiRequest::Search("secret".into())));
        assert!(!is_scope_neutral(&ApiRequest::Export("markdown".into())));
        assert!(!is_scope_neutral(&ApiRequest::Tags));
        assert!(!is_scope_neutral(&ApiRequest::Tasks { include_done: true, project: None }));
        assert!(!is_scope_neutral(&ApiRequest::Kanban { project: None }));
        assert!(!is_scope_neutral(&ApiRequest::Graph));
    }

    #[test]
    fn routes_parse() {
        assert!(matches!(route(&Method::Get, "/api/tree", "", "").unwrap(), ApiRequest::Tree));
        assert!(matches!(
            route(&Method::Get, "/api/nodes/5", "", "").unwrap(),
            ApiRequest::GetNode(5)
        ));
        assert!(matches!(
            route(&Method::Delete, "/api/nodes/5/cards/9", "", "").unwrap(),
            ApiRequest::DeleteCard { node: 5, card: 9 }
        ));
        assert!(matches!(
            route(&Method::Get, "/api/nodes/5/cards", "", "").unwrap(),
            ApiRequest::ListCards(5)
        ));
        // The single-card read must not be shadowed by the list route above it.
        assert!(matches!(
            route(&Method::Get, "/api/nodes/5/cards/9", "", "").unwrap(),
            ApiRequest::GetCard { node: 5, card: 9 }
        ));
        assert!(matches!(
            route(&Method::Get, "/api/search", "q=hello%20world", "").unwrap(),
            ApiRequest::Search(q) if q == "hello world"
        ));
        assert!(matches!(
            route(&Method::Get, "/api/instance", "", "").unwrap(),
            ApiRequest::Instance
        ));
        assert!(route(&Method::Get, "/api/bogus", "", "").is_err());
        assert!(route(&Method::Get, "/api/nodes/notanumber", "", "").is_err());
    }

    #[test]
    fn app_intercepted_routes_report_when_the_app_loop_misses_them() {
        // `Instance` needs the doc's path + server settings, so `process` only
        // sees it if the app-loop interception is ever missed.
        let mut doc = Document::empty();
        let (dirty, resp) = process(&mut doc, ApiRequest::Instance);
        assert!(!dirty);
        assert_eq!(resp.status, 500);
    }

    #[test]
    fn create_read_update_delete_node() {
        let mut doc = Document::empty();
        let (dirty, resp) =
            process(&mut doc, ApiRequest::CreateNode { parent: None, title: "Test".into() });
        assert!(dirty);
        assert_eq!(resp.status, 201);
        let id = body_id(&resp);

        let (_, got) = process(&mut doc, ApiRequest::GetNode(id));
        assert_eq!(got.status, 200);
        assert!(got.body.contains("Test"));

        let (_, up) = process(
            &mut doc,
            ApiRequest::UpdateNode { id, title: Some("Renamed".into()), color: None, bg: None },
        );
        assert_eq!(up.status, 200);
        assert_eq!(doc.nodes[&id].title, "Renamed");

        let (_, del) = process(&mut doc, ApiRequest::DeleteNode(id));
        assert_eq!(del.status, 200);
        assert!(!doc.nodes.contains_key(&id));
    }

    #[test]
    fn node_patch_sets_basket_bg_and_json_reports_it() {
        let mut doc = Document::empty();
        let id = doc.add_node(None, "n".into());
        // Flexible color input ("red") is accepted for bg, like other colors.
        let i: UpdateNodeInput = serde_json::from_str(r#"{"bg":"red"}"#).unwrap();
        let (dirty, resp) = process(
            &mut doc,
            ApiRequest::UpdateNode { id, title: None, color: None, bg: i.bg },
        );
        assert!(dirty);
        assert_eq!(resp.status, 200);
        assert_eq!(doc.nodes[&id].bg, Some([0xef, 0x44, 0x44]));

        let (_, got) = process(&mut doc, ApiRequest::GetNode(id));
        let v: Value = serde_json::from_str(&got.body).unwrap();
        assert_eq!(v["bg"], json!([239, 68, 68]));
    }

    #[test]
    fn add_card_then_search_finds_it() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "Node".into());
        let input: AddCardInput =
            serde_json::from_str(r#"{"kind":"text","title":"hi","body":"needle"}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::AddCard { node: nid, input });
        assert!(dirty);
        assert_eq!(resp.status, 201);

        let (_, s) = process(&mut doc, ApiRequest::Search("needle".into()));
        assert_eq!(s.status, 200);
        assert!(s.body.contains("needle"));
    }

    #[test]
    fn table_card_create_patch_and_json() {
        use crate::model::TableData;
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc
            .add_card(nid, egui::pos2(0.0, 0.0), CardKind::Table { table: TableData::empty(2, 2) })
            .unwrap();

        // PATCH replaces cell values.
        let patch: UpdateCardInput =
            serde_json::from_str(r#"{"rows":[["a","b"],["c","d"]]}"#).unwrap();
        let (dirty, resp) =
            process(&mut doc, ApiRequest::UpdateCard { node: nid, card: cid, patch });
        assert!(dirty);
        assert_eq!(resp.status, 200);
        let v: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(v["rows"][1][1]["text"], "d");
        assert_eq!(v["header"], true);

        let CardKind::Table { table } = &doc.nodes[&nid].cards[0].kind else { panic!() };
        assert_eq!(table.rows[0][1].text, "b");
    }

    #[test]
    fn update_card_sets_color_and_position() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let patch: UpdateCardInput =
            serde_json::from_str(r#"{"color":[1,2,3],"pos":[40,50],"size":[300,200]}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::UpdateCard { node: nid, card: cid, patch });
        assert!(dirty);
        assert_eq!(resp.status, 200);
        let c = doc.card_mut(nid, cid).unwrap();
        assert_eq!(c.color, [1, 2, 3]);
        assert_eq!(c.pos, egui::pos2(40.0, 50.0));
        assert_eq!(c.size, egui::vec2(300.0, 200.0));
    }

    #[test]
    fn create_and_patch_with_fit_sizes_card_to_content() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());

        // Create a checklist with a long item and fit:true — it should come out
        // wider than the default 240px square so the text is readable.
        let input: AddCardInput = serde_json::from_str(
            r#"{"kind":"checklist","title":"Groceries",
                 "items":[{"done":false,"text":"buy oat milk, eggs, bread, coffee and a card for mum"}],
                 "fit":true}"#,
        )
        .unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::AddCard { node: nid, input });
        assert!(dirty);
        let cid = serde_json::from_str::<Value>(&resp.body).unwrap()["id"].as_u64().unwrap();
        assert!(doc.card_mut(nid, cid).unwrap().size.x > 240.0, "create fit should widen");

        // Shrink it back to a square, then PATCH fit:true to re-fit.
        let patch: UpdateCardInput = serde_json::from_str(r#"{"size":[200,120]}"#).unwrap();
        process(&mut doc, ApiRequest::UpdateCard { node: nid, card: cid, patch });
        assert!(doc.card_mut(nid, cid).unwrap().size.x <= 200.0);

        let patch: UpdateCardInput = serde_json::from_str(r#"{"fit":true}"#).unwrap();
        let (_, resp) = process(&mut doc, ApiRequest::UpdateCard { node: nid, card: cid, patch });
        assert_eq!(resp.status, 200);
        assert!(doc.card_mut(nid, cid).unwrap().size.x > 240.0, "patch fit should widen");
    }

    #[test]
    fn create_text_card_with_inline_images_and_reports_names() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let png_b64 = {
            let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
            let mut buf = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut buf, image::ImageFormat::Png)
                .unwrap();
            base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
        };
        let body = format!(
            r#"{{"kind":"text","title":"note","body":"see ![p](trellis:0)","inline_images":["{png_b64}"],"fit":true}}"#
        );
        let input: AddCardInput = serde_json::from_str(&body).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::AddCard { node: nid, input });
        assert!(dirty);
        let cid = serde_json::from_str::<Value>(&resp.body).unwrap()["id"].as_u64().unwrap();
        let c = doc.card(nid, cid).unwrap();
        assert_eq!(c.inline_images.len(), 1, "inline image stored");
        let j = card_json(c);
        assert_eq!(
            j["inline_image_names"].as_array().map(|a| a.len()),
            Some(1),
            "card_json reports the inline image"
        );
    }

    #[test]
    fn create_card_applies_color_and_size_in_flexible_formats() {
        // Named, hex, short-hex and array all parse to RGB; the create endpoint
        // now applies color + size (previously silently dropped).
        assert_eq!(color_from_str("red"), Some([0xef, 0x44, 0x44]));
        // Every named swatch in the shared palette resolves via the API too.
        for (name, rgb) in crate::model::SWATCHES {
            assert_eq!(color_from_str(name), Some(*rgb), "swatch {name}");
        }
        assert_eq!(color_from_str("#22c55e"), Some([0x22, 0xc5, 0x5e]));
        assert_eq!(color_from_str("#e44"), Some([0xee, 0x44, 0x44]));
        assert_eq!(color_from_str("nonsense"), None);
        assert_eq!(color_from_value(&json!([1, 2, 3])).unwrap(), Some([1, 2, 3]));
        assert!(color_from_value(&json!([1, 2])).is_err());
        assert!(color_from_value(&json!([1, 2, 999])).is_err());
        assert_eq!(color_from_value(&Value::Null).unwrap(), None);

        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let input: AddCardInput = serde_json::from_str(
            r#"{"kind":"text","title":"T","color":"red","size":[321,222]}"#,
        )
        .unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::AddCard { node: nid, input });
        assert!(dirty);
        assert_eq!(resp.status, 201);
        let c = &doc.nodes[&nid].cards[0];
        assert_eq!(c.color, [0xef, 0x44, 0x44]);
        assert_eq!(c.size, egui::vec2(321.0, 222.0));

        // Every kind is creatable via the API.
        for (kind, want) in [
            ("code", true),
            ("checklist", true),
            ("table", true),
            ("image", true),
            ("text", true),
        ] {
            let input: AddCardInput =
                serde_json::from_str(&format!(r#"{{"kind":"{kind}"}}"#)).unwrap();
            let (_d, resp) = process(&mut doc, ApiRequest::AddCard { node: nid, input });
            assert_eq!(resp.status == 201, want, "kind {kind}");
        }
    }

    #[test]
    fn patch_can_change_card_kind() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        // text -> checklist, then set items in the same surface.
        let patch: UpdateCardInput =
            serde_json::from_str(r#"{"kind":"checklist"}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::UpdateCard { node: nid, card: cid, patch });
        assert!(dirty);
        assert_eq!(resp.status, 200);
        assert!(matches!(doc.nodes[&nid].cards[0].kind, CardKind::Checklist { .. }));
        let patch: UpdateCardInput =
            serde_json::from_str(r#"{"items":[{"done":false,"text":"a"}]}"#).unwrap();
        process(&mut doc, ApiRequest::UpdateCard { node: nid, card: cid, patch });
        let CardKind::Checklist { items } = &doc.nodes[&nid].cards[0].kind else { panic!() };
        assert_eq!(items[0].text, "a");
    }

    #[test]
    fn table_ops_via_api() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc
            .add_card(nid, egui::pos2(0.0, 0.0), CardKind::Table { table: crate::model::TableData::empty(2, 2) })
            .unwrap();
        // Color a cell (by name), add a row, turn the header off.
        for body in [
            r#"{"op":"set_bg","row":0,"col":0,"color":"red"}"#,
            r#"{"op":"insert_row","at":1}"#,
            r#"{"op":"set_header","header":false}"#,
        ] {
            let op: TableOpInput = serde_json::from_str(body).unwrap();
            let (dirty, resp) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, op });
            assert!(dirty, "op {body}");
            assert_eq!(resp.status, 200, "op {body}");
        }
        let CardKind::Table { table } = &doc.nodes[&nid].cards[0].kind else { panic!() };
        assert_eq!(table.rows[0][0].bg, Some([0xef, 0x44, 0x44]));
        assert_eq!(table.rows.len(), 3);
        assert!(!table.header);
        // An unknown op is a 400.
        let op: TableOpInput = serde_json::from_str(r#"{"op":"bogus"}"#).unwrap();
        let (_d, resp) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, op });
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn get_one_card_returns_it_and_404s_for_a_stranger() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(nid, cid).unwrap().title = "just this one".into();

        let (dirty, resp) = process(&mut doc, ApiRequest::GetCard { node: nid, card: cid });
        assert!(!dirty, "a read must never mark the document dirty");
        assert_eq!(resp.status, 200);
        let got: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(got["title"], "just this one");
        assert_eq!(got["id"], cid);

        // Same shape as the card in the basket listing — one card, not a wrapper.
        let (_d, listed) = process(&mut doc, ApiRequest::ListCards(nid));
        let listed: Value = serde_json::from_str(&listed.body).unwrap();
        assert_eq!(listed["cards"][0], got);

        // A card id that isn't in this node is a 404, not someone else's card.
        let other = doc.add_node(None, "other".into());
        let (_d, resp) = process(&mut doc, ApiRequest::GetCard { node: other, card: cid });
        assert_eq!(resp.status, 404);
        let (_d, resp) = process(&mut doc, ApiRequest::GetCard { node: nid, card: 9999 });
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn autofit_cols_via_api_widens_the_wordy_column() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc
            .add_card(nid, egui::pos2(0.0, 0.0), CardKind::Table {
                table: crate::model::TableData::from_values(vec![
                    vec!["Host".into(), "Result".into()],
                    vec!["ALICE".into(), "a verdict far too long for 110 pixels".into()],
                ]),
            })
            .unwrap();

        let op: TableOpInput = serde_json::from_str(r#"{"op":"autofit_cols"}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, op });
        assert!(dirty);
        assert_eq!(resp.status, 200);
        let CardKind::Table { table } = &doc.nodes[&nid].cards[0].kind else { panic!() };
        assert!(table.col_width(1) > crate::model::TABLE_DEFAULT_COL_W);
        let fitted = table.col_width(1);

        // An out-of-range `col` is a 400, like the other indexed ops.
        let op: TableOpInput = serde_json::from_str(r#"{"op":"autofit_cols","col":7}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, op });
        assert!(!dirty);
        assert_eq!(resp.status, 400);

        // Idempotent: fitting again on unchanged content changes nothing.
        let op: TableOpInput = serde_json::from_str(r#"{"op":"autofit_cols"}"#).unwrap();
        let (_d, _r) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, op });
        let CardKind::Table { table } = &doc.nodes[&nid].cards[0].kind else { panic!() };
        assert_eq!(table.col_width(1), fitted);
    }

    #[test]
    fn image_bytes_add_and_remove_via_api() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc
            .add_card(nid, egui::pos2(0.0, 0.0), CardKind::Image {
                data: Vec::new(),
                name: String::new(),
                extra: Vec::new(),
                ocr: String::new(),
            })
            .unwrap();
        let (dirty, resp) = process(
            &mut doc,
            ApiRequest::AddImage { node: nid, card: cid, name: "pic".into(), bytes: vec![1, 2, 3, 4] },
        );
        assert!(dirty);
        assert_eq!(resp.status, 201);
        assert_eq!(doc.nodes[&nid].cards[0].kind.images().len(), 1);

        // GET the image back as base64 (what the mobile viewer uses).
        let (_d, got) =
            process(&mut doc, ApiRequest::GetImage { node: nid, card: cid, index: 0 });
        assert_eq!(got.status, 200);
        let v: Value = serde_json::from_str(&got.body).unwrap();
        assert_eq!(v["name"], "pic");
        assert_eq!(
            v["base64"],
            base64::engine::general_purpose::STANDARD.encode([1, 2, 3, 4])
        );
        // Out-of-range index is a 404.
        let (_d, miss) =
            process(&mut doc, ApiRequest::GetImage { node: nid, card: cid, index: 9 });
        assert_eq!(miss.status, 404);

        let (_d, resp) =
            process(&mut doc, ApiRequest::RemoveImage { node: nid, card: cid, index: 0 });
        assert_eq!(resp.status, 200);
        assert_eq!(doc.nodes[&nid].cards[0].kind.images().len(), 0);
    }

    #[test]
    fn sketch_create_and_ops_via_api() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let input: AddCardInput = serde_json::from_str(r#"{"kind":"sketch"}"#).unwrap();
        let (_d, resp) = process(&mut doc, ApiRequest::AddCard { node: nid, input });
        assert_eq!(resp.status, 201);
        let cid = body_id(&resp);
        let op: SketchOpInput = serde_json::from_str(
            r#"{"op":"add_stroke","color":"blue","width":2,"points":[[0,0],[10,10]]}"#,
        )
        .unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::SketchOp { node: nid, card: cid, op });
        assert!(dirty);
        assert_eq!(resp.status, 200);
        let v: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(v["strokes"][0]["color"], json!([0x3b, 0x82, 0xf6]));
        // clear
        let op: SketchOpInput = serde_json::from_str(r#"{"op":"clear"}"#).unwrap();
        let (_d, resp) = process(&mut doc, ApiRequest::SketchOp { node: nid, card: cid, op });
        assert_eq!(resp.status, 200);
        // undo on empty → 400
        let op: SketchOpInput = serde_json::from_str(r#"{"op":"undo"}"#).unwrap();
        let (_d, resp) = process(&mut doc, ApiRequest::SketchOp { node: nid, card: cid, op });
        assert_eq!(resp.status, 400);
    }

    #[test]
    fn font_scale_and_autosort_via_api() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        // font_scale on PATCH, echoed back in the card JSON.
        let patch: UpdateCardInput = serde_json::from_str(r#"{"font_scale":1.5}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::UpdateCard { node: nid, card: cid, patch });
        assert!(dirty);
        assert_eq!(resp.status, 200);
        assert_eq!(doc.nodes[&nid].cards[0].font_scale, 1.5);
        let v: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(v["font_scale"], 1.5);
        // Autosort endpoint arranges the node's cards.
        doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::Autosort(nid));
        assert!(dirty);
        assert_eq!(resp.status, 200);
    }

    #[test]
    fn card_joins_and_leaves_a_group_via_api() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let a = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let c = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let g = doc.group_cards(nid, &[a, b], "grp".into()).unwrap();
        // Join c to the existing group, then leave.
        let (dirty, resp) =
            process(&mut doc, ApiRequest::SetCardGroup { node: nid, card: c, group: Some(g) });
        assert!(dirty);
        assert_eq!(resp.status, 200);
        assert_eq!(doc.card_mut(nid, c).unwrap().group, Some(g));
        let (_d, resp) =
            process(&mut doc, ApiRequest::SetCardGroup { node: nid, card: c, group: None });
        assert_eq!(resp.status, 200);
        assert_eq!(doc.card_mut(nid, c).unwrap().group, None);
        // Joining a non-existent group is a 404.
        let (_d, resp) =
            process(&mut doc, ApiRequest::SetCardGroup { node: nid, card: c, group: Some(999) });
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn group_dock_and_export_via_api() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let a = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();

        // Group two cards.
        let (dirty, resp) = process(
            &mut doc,
            ApiRequest::CreateGroup { node: nid, cards: vec![a, b], title: Some("Pair".into()) },
        );
        assert!(dirty);
        assert_eq!(resp.status, 201);
        let gid = body_id(&resp);
        assert_eq!(doc.card_mut(nid, a).unwrap().group, Some(gid));

        // Dock a onto b.
        let (_, dr) = process(&mut doc, ApiRequest::DockCard { node: nid, card: a, anchor: b });
        assert_eq!(dr.status, 200);
        assert_eq!(doc.card_mut(nid, a).unwrap().docked_to, Some(b));
        // Self-cycle refused.
        let (_, cyc) = process(&mut doc, ApiRequest::DockCard { node: nid, card: b, anchor: a });
        assert_eq!(cyc.status, 400);

        // Export as PDF returns base64.
        let (_, ex) = process(&mut doc, ApiRequest::Export("pdf".into()));
        assert_eq!(ex.status, 200);
        assert!(ex.body.contains("\"base64\""));
    }

    #[test]
    fn move_node_reorders_reparents_and_guards_cycles() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "A".into());
        let b = doc.add_node(None, "B".into());
        let c = doc.add_node(None, "C".into());
        assert_eq!(doc.roots, vec![a, b, c]);

        // Helper: route a move body for `id`, then apply it.
        let mv = |doc: &mut Document, id: NodeId, body: &str| {
            let req = route(&Method::Post, &format!("/api/nodes/{id}/move"), "", body).unwrap();
            process(doc, req)
        };

        // before: put C ahead of A -> [C, A, B].
        let (dirty, r) = mv(&mut doc, c, &format!(r#"{{"before":{a}}}"#));
        assert!(dirty && r.status == 200);
        assert_eq!(doc.roots, vec![c, a, b]);

        // to:bottom moves within the current parent -> [C, B, A].
        let (_, r) = mv(&mut doc, a, r#"{"to":"bottom"}"#);
        assert_eq!(r.status, 200);
        assert_eq!(doc.roots, vec![c, b, a]);

        // parent + to:top reparents B under C at the front.
        let (_, r) = mv(&mut doc, b, &format!(r#"{{"parent":{c},"to":"top"}}"#));
        assert_eq!(r.status, 200);
        assert_eq!(doc.nodes[&c].children, vec![b]);
        assert_eq!(doc.roots, vec![c, a]);
        assert_eq!(doc.nodes[&b].parent, Some(c));

        // parent:null promotes B back to the top level at index 0.
        let (_, r) = mv(&mut doc, b, r#"{"parent":null,"index":0}"#);
        assert_eq!(r.status, 200);
        assert_eq!(doc.nodes[&b].parent, None);
        assert_eq!(doc.roots, vec![b, c, a]);

        // Cycle guard: C cannot move under its own (former) subtree. Re-nest a
        // under c first, then try to move c under a.
        let (_, _) = mv(&mut doc, a, &format!(r#"{{"parent":{c}}}"#));
        assert_eq!(doc.nodes[&a].parent, Some(c));
        let (dirty, r) = mv(&mut doc, c, &format!(r#"{{"parent":{a}}}"#));
        assert!(!dirty && r.status == 400);
        assert_eq!(doc.nodes[&c].parent, None); // unchanged

        // Empty placement and unknown target are rejected.
        assert_eq!(mv(&mut doc, c, "{}").1.status, 400);
        assert_eq!(mv(&mut doc, c, r#"{"before":99999}"#).1.status, 400);
    }

    #[test]
    fn move_card_reorders_within_basket() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let ids = |doc: &Document| doc.nodes[&n].cards.iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(&doc), vec![a, b, c]);

        let mv = |doc: &mut Document, card, body: &str| {
            let req = route(&Method::Post, &format!("/api/nodes/{n}/cards/{card}/move"), "", body).unwrap();
            process(doc, req)
        };
        // a to front (end of draw order).
        assert_eq!(mv(&mut doc, a, r#"{"to":"front"}"#).1.status, 200);
        assert_eq!(ids(&doc), vec![b, c, a]);
        // c to back.
        mv(&mut doc, c, r#"{"to":"back"}"#);
        assert_eq!(ids(&doc), vec![c, b, a]);
        // b before c (front-of-list edge).
        mv(&mut doc, b, &format!(r#"{{"before":{c}}}"#));
        assert_eq!(ids(&doc), vec![b, c, a]);
        // b after a (target sits after b, so it lands last).
        mv(&mut doc, b, &format!(r#"{{"after":{a}}}"#));
        assert_eq!(ids(&doc), vec![c, a, b]);
        // absolute index.
        mv(&mut doc, b, r#"{"index":0}"#);
        assert_eq!(ids(&doc), vec![b, c, a]);
        // guards.
        assert_eq!(mv(&mut doc, b, "{}").1.status, 400);
        assert_eq!(mv(&mut doc, b, r#"{"before":99999}"#).1.status, 400);
    }

    #[test]
    fn missing_node_is_404() {
        let mut doc = Document::empty();
        assert_eq!(process(&mut doc, ApiRequest::GetNode(999)).1.status, 404);
        assert_eq!(process(&mut doc, ApiRequest::DeleteNode(999)).1.status, 404);
    }

    #[test]
    fn create_node_with_missing_parent_is_400() {
        let mut doc = Document::empty();
        let (dirty, resp) =
            process(&mut doc, ApiRequest::CreateNode { parent: Some(42), title: "x".into() });
        assert!(!dirty);
        assert_eq!(resp.status, 400);
    }
}
