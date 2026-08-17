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
    /// A journal node for a calendar day, created on demand under the configured
    /// root. `None` = today. App-intercepted: the root is a per-instance
    /// **setting**, so only the app loop knows whether daily notes are on at all.
    DailyNote { date: Option<String> },
    /// Read the daily-notes setting: is it on, and which node is the journal
    /// root. The UI shows this in Settings, so the API has to answer it too —
    /// an agent that cannot see the configuration cannot collaborate on it.
    DailyConfig,
    /// Set the journal root (`Some`) or switch daily notes off (`None`) — the
    /// two buttons in Settings, reachable by an agent.
    SetDailyRoot(Option<NodeId>),
    /// Cards whose `[[#id]]` links point at one card. The basket-level
    /// `Backlinks` is useless in a journal document, where every card written on
    /// a day shares one basket.
    CardBacklinks(u64),
    /// One card from its id alone, without already knowing its basket. Ids are
    /// unique per document, so an id *is* an address — but every other card route
    /// is `/nodes/{node}/cards/{card}`, so an id quoted in a note or read out of
    /// a response could only be resolved by walking every basket.
    LocateCard(u64),
    CreateNode { parent: Option<NodeId>, title: String },
    UpdateNode { id: NodeId, title: Option<String>, color: Option<[u8; 3]>, bg: Option<[u8; 3]> },
    DeleteNode(NodeId),
    // Reorder / reparent a node in the tree.
    MoveNode { id: NodeId, mv: MoveNodeInput },
    // Expand or collapse a node (optionally its whole subtree).
    SetExpanded { id: NodeId, expanded: bool, recursive: bool },
    // Expand or collapse every root and everything under it.
    SetAllExpanded { expanded: bool },
    AddCard { node: NodeId, input: AddCardInput },
    UpdateCard { node: NodeId, card: u64, patch: UpdateCardInput },
    // ↑ Both accept `fit`. `process` applies `Card::fit_size`, which is only an
    // estimate; the app re-fits precisely afterwards — see `fit_request`.
    DeleteCard { node: NodeId, card: u64 },
    // Reorder a card within its basket (draw / autosort order).
    MoveCard { node: NodeId, card: u64, mv: MoveCardInput },
    // Set an inline key:: value property on a card (e.g. status for the board).
    SetCardProperty { node: NodeId, card: u64, key: String, value: String },
    /// Set a property on one checklist ITEM — the line, not the card. Needed
    /// because a checklist's lines are separate tasks with separate dates.
    SetItemProperty { node: NodeId, card: u64, item: u64, key: String, value: String },
    /// Remove a property from one checklist item, or clear its date.
    ClearItemProperty { node: NodeId, card: u64, item: u64, key: String },
    /// Tick or untick one checklist item — the done signal, over the API.
    SetItemDone { node: NodeId, card: u64, item: u64, done: bool },
    /// Remove a `key:: value` line outright. Distinct from setting it empty,
    /// which leaves the property present but unreadable — a card whose `due::`
    /// is blank stays on the agenda under "No date" instead of leaving it.
    ClearCardProperty { node: NodeId, card: u64, key: String },
    // Grouping.
    ListGroups(NodeId),
    CreateGroup { node: NodeId, cards: Vec<u64>, title: Option<String> },
    UpdateGroup { node: NodeId, group: GroupId, title: Option<String>, color: Option<[u8; 3]> },
    DeleteGroup { node: NodeId, group: GroupId },
    /// One group from its id alone, without already knowing its basket — the
    /// counterpart of [`LocateCard`]. Group ids come from one document-wide
    /// counter, so an id is a complete address.
    LocateGroup(GroupId),
    /// Move a whole group — container and members — to another basket. Moving
    /// the members one at a time cannot do this: group membership is
    /// basket-local, so each card arrives ungrouped and the group's id, the
    /// thing a `[[#g…]]` link points at, is lost.
    MoveGroup { node: NodeId, group: GroupId, to: NodeId, pos: Option<[f32; 2]> },
    /// Cards whose `[[#g…]]` links point at one group.
    GroupBacklinks(GroupId),
    /// A **card-addressed write**: the same operation as its node-addressed twin,
    /// with the basket left for the app loop to resolve.
    ///
    /// A card id has been a complete address since v0.87.0 — every query surface
    /// hands them out (`/api/search`, `/api/tasks`, `/api/claims`, backlinks,
    /// `/api/changes`) and a `[[#1391]]` link *is* one — but only reads could take
    /// one. Every write needed the basket too, so an agent given an id had to
    /// spend a `GET /api/cards/{cid}` learning the node before it could act, and
    /// then quote a number the human never mentioned.
    ///
    /// **This is deliberately a rewrite, not a second implementation.** The app
    /// loop resolves the id, turns this into the ordinary node-addressed request
    /// and drops it back into the same pipeline — so the scope check, the mirror
    /// check, the change log and `process` are the ones that already exist and
    /// were already audited. Adding a parallel set of write paths that each had to
    /// remember to check a token's scope is exactly how the v0.111.0 escape
    /// happened, one end of a move at a time.
    ByCard { card: u64, op: CardOp },
    /// Move several cards to another basket in one call. Archiving a finished
    /// basket was 55 single-card calls before this existed.
    MoveCards { node: NodeId, cards: Vec<u64>, to: NodeId, pos: Option<[f32; 2]>, gap: f32 },
    /// Set one `key:: value` property on several cards at once — marking a batch
    /// `status:: done` is the case that keeps coming up.
    SetCardsProperty { node: NodeId, cards: Vec<u64>, key: String, value: String },
    /// Create several cards in one call. Same endpoint as the single create,
    /// which accepts an object or an array — the shape table ops took in
    /// v0.82.0, for the same reason: building anything real is many small calls.
    AddCards { node: NodeId, inputs: Vec<AddCardInput> },
    /// Edit the **presentation** of several cards at once: colour, size, depth,
    /// font scale, emphasis, `fit`.
    ///
    /// Deliberately not content. A batch that could set `body` or `items` would
    /// write the same text over every card it names, and one typo'd id list is
    /// then an unrecoverable overwrite of work — while the thing it would be
    /// *used* for, N cards saying the same thing, is the copied-card failure this
    /// codebase fights everywhere else. Content is one card at a time; the batch
    /// is for "make these look the same". A content field here is a 400 naming it
    /// and pointing at the single-card route.
    UpdateCards { node: NodeId, cards: Vec<u64>, patch: UpdateCardInput },
    /// Delete several cards in one call, having checked the whole list first.
    DeleteCards { node: NodeId, cards: Vec<u64> },
    /// Remove one `key:: value` line from several cards at once — the missing
    /// half of [`SetCardsProperty`]. Clearing `due::` off a finished batch was
    /// one call per card while setting it was one call for all of them.
    ClearCardsProperty { node: NodeId, cards: Vec<u64>, key: String },
    // Docking.
    DockCard { node: NodeId, card: u64, anchor: u64 },
    DetachCard { node: NodeId, card: u64 },
    // Card group membership (join an existing group / leave).
    SetCardGroup { node: NodeId, card: u64, group: Option<GroupId> },
    // Fine-grained table editing (cell colors, header, widths, row/col ops).
    /// One or more table edits, applied in order. A batch because building a
    /// styled table is inherently many small ops, and one-per-call made that
    /// both slow and easy to get wrong.
    TableOp { node: NodeId, card: u64, ops: Vec<TableOpInput> },
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
    // Which cards in a basket cover each other, and the repair for it.
    Overlaps(NodeId),
    ResolveOverlaps(NodeId),
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
    // Cards that assert state and say when to re-check it (`verify::`), so a
    // reader is told which claims are out of date before it believes them.
    Claims { expired_only: bool, project: Option<NodeId> },
    // Cards grouped by `status::` value — the Kanban board's columns.
    Kanban { project: Option<NodeId> },
    // Cards that [[link]] to a node.
    Backlinks(NodeId),
    // The wiki-link graph (linked nodes + directed edges).
    Graph,
    // Which document this instance has open (and on which port), so an agent
    // driving several instances can check it has the right one.
    Instance,
    /// Read the app-level settings — the ones that live in the instance's config
    /// rather than in the document.
    SettingsGet,
    /// Change one or more of them. Unknown keys are refused by name.
    SettingsSet(serde_json::Map<String, Value>),
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
    /// `GET /open/card/{cid}` or `/open/node/{id}` — reveal a target in this
    /// running instance. **Unauthenticated and navigation-only**: it focuses a
    /// window and answers ok/not-found, never document content, because a link
    /// has to be clickable from a terminal or a browser and anything that
    /// returned data would let a web page read notes by walking ids.
    Open { kind: OpenKind, id: u64, doc: Option<String> },
    /// `GET /api/cards/{cid}/link` — the canonical URL for a card, so an agent
    /// never builds one by hand. Hand-built identifiers are how the work journal
    /// grew three spellings of the same day.
    CardLink(u64),
    /// `GET /api/groups/{gid}/link` — the canonical URL for a group, minted the
    /// same way and for the same reason as [`CardLink`].
    GroupLink(u64),
    /// Send a card to the desktop as its own OS window, or recall it. Placement
    /// is app config, so this is answered in the app loop, not here.
    SetCardDesktop { card: u64, pos: Option<[f32; 2]>, on: bool },
    /// Which cards are currently out on the desktop.
    ListCardDesktop,
    /// Desktop **mode**: take a whole basket out onto the desktop at once, or
    /// bring it back. The per-card route is the exception; this is the feature.
    SetNodeDesktop { node: NodeId, on: bool },
}

/// `POST /api/cards/{cid}/desktop` — optional screen position for the window.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CardDesktopInput {
    #[serde(default)]
    pos: Option<[f32; 2]>,
}

/// `{key, value}` — the body of every "set one property" route, card or item.
///
/// One definition rather than the two identical inline copies this had: they are
/// the same request shape, and a third caller (the card-addressed routes) is
/// exactly when a copy starts to drift.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PropertyInput {
    key: String,
    value: String,
}

/// What a card-addressed request wants done, once the basket is known.
///
/// One variant per node-addressed twin, and [`resolve_by_card`] is the only place
/// the pairing is written down.
pub enum CardOp {
    Patch(UpdateCardInput),
    Delete,
    SetProperty { key: String, value: String },
    ClearProperty { key: String },
    Move(MoveCardInput),
    ItemDone { item: u64, done: bool },
    SetItemProperty { item: u64, key: String, value: String },
    ClearItemProperty { item: u64, key: String },
}

/// Turn a card-addressed request into the node-addressed one it stands for.
///
/// Called by the app loop, which is where a card id can be resolved to a basket.
/// Keeping the mapping here — a pure function over ids, testable without a
/// document — is what makes "the two routes do the same thing" checkable instead
/// of a claim.
pub fn resolve_by_card(node: NodeId, card: u64, op: CardOp) -> ApiRequest {
    match op {
        CardOp::Patch(patch) => ApiRequest::UpdateCard { node, card, patch },
        CardOp::Delete => ApiRequest::DeleteCard { node, card },
        CardOp::SetProperty { key, value } => {
            ApiRequest::SetCardProperty { node, card, key, value }
        }
        CardOp::ClearProperty { key } => ApiRequest::ClearCardProperty { node, card, key },
        CardOp::Move(mv) => ApiRequest::MoveCard { node, card, mv },
        CardOp::ItemDone { item, done } => ApiRequest::SetItemDone { node, card, item, done },
        CardOp::SetItemProperty { item, key, value } => {
            ApiRequest::SetItemProperty { node, card, item, key, value }
        }
        CardOp::ClearItemProperty { item, key } => {
            ApiRequest::ClearItemProperty { node, card, item, key }
        }
    }
}

/// `POST /api/nodes/{id}/cards/move` — move a list of cards to another basket.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveCardsInput {
    /// The cards to move. Every one must exist in the source basket; an id that
    /// does not is a 400 naming it, not a silent skip.
    cards: Vec<u64>,
    /// Destination basket.
    node: NodeId,
    /// Where the first card lands. Omit to keep every card's current
    /// coordinates, which is what you want when the layout already means
    /// something.
    #[serde(default)]
    pos: Option<[f32; 2]>,
    /// Vertical gap between stacked cards when `pos` is given.
    #[serde(default = "default_gap")]
    gap: f32,
}

fn default_gap() -> f32 {
    20.0
}

/// `DELETE /api/nodes/{id}/cards` — delete a list of cards.
///
/// The list is explicit and there is no "everything in this basket" form: the one
/// batch operation that cannot be walked back should not be reachable by omitting
/// an argument.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteCardsInput {
    cards: Vec<u64>,
}

/// `DELETE /api/nodes/{id}/cards/property` — remove one property from many cards.
///
/// `key` rides in the body rather than the query string, unlike the single-card
/// form: the card list has to be a body anyway, and splitting one request across
/// both is how you end up deleting the wrong property from the right cards.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearCardsPropertyInput {
    cards: Vec<u64>,
    key: String,
}

/// `POST /api/nodes/{id}/cards/property` — one property, many cards.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CardsPropertyInput {
    cards: Vec<u64>,
    key: String,
    value: String,
}

/// `POST /api/nodes/{id}/groups/{gid}/move` — the destination basket, and
/// optionally where the group's top-left corner should land in it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveGroupInput {
    /// The basket to move the group into.
    node: NodeId,
    /// Top-left corner in the destination. Every member moves by the same
    /// delta, so the arrangement inside the group survives.
    #[serde(default)]
    pos: Option<[f32; 2]>,
}

/// What a `trellis://` link (and its `/open/…` HTTP twin) names.
///
/// A bool distinguished the first two; a third kind needs a name, and naming
/// them makes the route table read as the link format does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenKind {
    Node,
    Card,
    Group,
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

/// The cards a **batch edit** asked to be fitted. Unlike a batch create, the ids
/// are in the request — the caller named them — so there is nothing to pair with
/// the response.
pub fn fit_updates(req: &ApiRequest) -> Option<(NodeId, Vec<u64>)> {
    match req {
        ApiRequest::UpdateCards { node, cards, patch } if patch.fit => {
            Some((*node, cards.clone()))
        }
        _ => None,
    }
}

/// Which cards of a **batch** create asked to be fitted, by their index in the
/// batch. The app loop pairs these with the ids the response hands back, because
/// a created card has no id until it exists.
pub fn fit_batch(req: &ApiRequest) -> Vec<usize> {
    match req {
        ApiRequest::AddCards { inputs, .. } => {
            inputs.iter().enumerate().filter(|(_, i)| i.fit).map(|(n, _)| n).collect()
        }
        _ => Vec::new(),
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
        | ApiRequest::Autosort(id)
        | ApiRequest::Overlaps(id)
        | ApiRequest::ResolveOverlaps(id) => Some(*id),
        ApiRequest::GetCard { node, .. }
        | ApiRequest::AddCard { node, .. }
        | ApiRequest::UpdateCard { node, .. }
        | ApiRequest::DeleteCard { node, .. }
        | ApiRequest::MoveCard { node, .. }
        | ApiRequest::SetCardProperty { node, .. }
        | ApiRequest::ClearCardProperty { node, .. }
        | ApiRequest::SetItemProperty { node, .. }
        | ApiRequest::ClearItemProperty { node, .. }
        | ApiRequest::SetItemDone { node, .. }
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
        | ApiRequest::SetNodeDesktop { node, .. }
        | ApiRequest::MoveGroup { node, .. }
        | ApiRequest::MoveCards { node, .. }
        | ApiRequest::SetCardsProperty { node, .. }
        | ApiRequest::AddCards { node, .. }
        | ApiRequest::UpdateCards { node, .. }
        | ApiRequest::DeleteCards { node, .. }
        | ApiRequest::ClearCardsProperty { node, .. }
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

/// Every file a request asks a card to mirror.
///
/// Split out so the app loop can check them against the mirror policy before the
/// request is applied — `process` cannot, since the setting lives in the app.
///
/// **Plural on purpose.** A batch create carries one `source` per card, and
/// checking only the first would let the second reach any file the policy
/// forbids — the mirror check is the one place an API request can touch the
/// filesystem, so it has to see all of them. A singular `source_request` sat
/// beside this until v0.115.1, left behind when the app loop moved to the batch
/// form: nothing but its own tests called it, and its doc comment still claimed
/// to be what the app loop used.
pub fn source_requests(req: &ApiRequest) -> Vec<String> {
    let raw: Vec<Option<String>> = match req {
        ApiRequest::AddCard { input, .. } => vec![input.source.clone()],
        ApiRequest::UpdateCard { patch, .. } => vec![patch.source.clone()],
        ApiRequest::AddCards { inputs, .. } => inputs.iter().map(|i| i.source.clone()).collect(),
        _ => Vec::new(),
    };
    // Detaching (`""`) reaches no file, so it is always allowed.
    raw.into_iter()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
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
        // Touches every node, so it is reported once against the document
        // rather than as N separate node updates.
        ApiRequest::SetAllExpanded { .. } => ch(E::Node, Op::Updated, 0).field("expanded.all"),
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
                (patch.z.is_some(), "z"),
                (patch.emphasis.is_some(), "emphasis"),
                (patch.emphasis_intensity.is_some(), "emphasis_intensity"),
                (patch.emphasis_minutes.is_some(), "emphasis_until"),
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
        ApiRequest::TableOp { node, card, ops } => {
            let mut c = ch(E::Card, Op::Updated, *card).in_node(*node).titled(card_title(node, card));
            for op in ops {
                c = c.field(&format!("table.{}", op.name()));
            }
            c
        }
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
        // Reported against the basket it came *from*, with the destination as a
        // field — the same shape as a cross-basket card move, so a client that
        // knows to refresh both ends already handles this one.
        ApiRequest::AddCards { node, inputs } => ch(E::Card, Op::Created, 0)
            .in_node(*node)
            .field(&format!("batch={}", inputs.len())),
        ApiRequest::MoveCards { node, cards, to, .. } => {
            let mut c = ch(E::Card, Op::Moved, cards.first().copied().unwrap_or(0)).in_node(*node);
            c = c.field(&format!("node={to}"));
            c.field(&format!("batch={}", cards.len()))
        }
        ApiRequest::SetCardsProperty { node, cards, key, value } => {
            ch(E::Card, Op::Updated, cards.first().copied().unwrap_or(0))
                .in_node(*node)
                .field(&format!("{}={}", key.to_lowercase(), value))
                .field(&format!("batch={}", cards.len()))
        }
        ApiRequest::UpdateCards { node, cards, .. } => {
            ch(E::Card, Op::Updated, cards.first().copied().unwrap_or(0))
                .in_node(*node)
                .field(&format!("batch={}", cards.len()))
        }
        ApiRequest::DeleteCards { node, cards } => {
            ch(E::Card, Op::Deleted, cards.first().copied().unwrap_or(0))
                .in_node(*node)
                .field(&format!("batch={}", cards.len()))
        }
        ApiRequest::ClearCardsProperty { node, cards, key } => {
            ch(E::Card, Op::Updated, cards.first().copied().unwrap_or(0))
                .in_node(*node)
                .field(&format!("-{}", key.to_lowercase()))
                .field(&format!("batch={}", cards.len()))
        }
        ApiRequest::MoveGroup { node, group, to, .. } => {
            ch(E::Group, Op::Moved, *group).in_node(*node).field(&format!("node={to}"))
        }

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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

impl MoveNodeInput {
    /// Where this move would put the node, when it changes parent. `before`/
    /// `after` adopt the sibling's parent, which only the tree can resolve.
    pub fn destination(&self) -> Option<MoveDest> {
        if let Some(p) = self.parent {
            return Some(MoveDest::Parent(p));
        }
        if let Some(sib) = self.before.or(self.after) {
            return Some(MoveDest::Sibling(sib));
        }
        None
    }
}

/// Where a request is trying to put something, when that is somewhere other
/// than where it already is.
///
/// A subtree-scoped token is checked against the node a request *names*
/// ([`target_node`]), which for a move is where the thing is coming **from**.
/// That alone lets a confined token move its own card or basket out into the
/// rest of the document — a write outside the scope, reached by relocating
/// something inside it. The destination is resolved separately because
/// `before`/`after` name a sibling, and only the tree knows its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveDest {
    /// A basket a card or group is being moved into.
    Basket(NodeId),
    /// A node's new parent; `None` is the top level.
    Parent(Option<NodeId>),
    /// A node whose parent this move will adopt.
    Sibling(NodeId),
}

/// The destination of a move request, if it has one. `None` means the request
/// cannot relocate anything outside where it already is.
pub fn move_destination(req: &ApiRequest) -> Option<MoveDest> {
    match req {
        ApiRequest::MoveCard { mv, .. } => mv.target_node().map(MoveDest::Basket),
        ApiRequest::MoveGroup { to, .. } => Some(MoveDest::Basket(*to)),
        ApiRequest::MoveCards { to, .. } => Some(MoveDest::Basket(*to)),
        ApiRequest::MoveNode { mv, .. } => mv.destination(),
        _ => None,
    }
}

/// Body of `POST /api/nodes/{id}/cards/{cid}/chart` — how to draw a table card
/// as a chart. Omitted fields keep their current value, so you can flip the kind
/// without restating the columns.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct DoneInput {
    pub done: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyRootInput {
    /// The node to keep the journal under — the year, in a year-per-root tree.
    pub node: NodeId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyInput {
    /// `YYYY-MM-DD`. Omitted (or an empty body) means today — the common case.
    /// Accepting a date is what lets an agent stop hand-building dated node
    /// titles, which is how a journal drifts into two nodes for one day.
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandInput {
    expanded: bool,
    /// Apply to the whole subtree (node + all descendants), not just this node.
    #[serde(default)]
    recursive: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandAllInput {
    expanded: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    /// Depth, in the **same units as `pos`** — positive is toward the viewer, so
    /// a larger `z` is nearer. Same units deliberately: "200 nearer" is then the
    /// same size of move as "200 right".
    ///
    /// A basket is a volume. With the canvas's **Depth** toggle off this is just
    /// the stacking order, so it is never meaningless — but it also means the
    /// reader may not see it. **Put arrangement in `z`; put meaning in the text,
    /// a `#tag` or a `key:: value`**, or you have written something they may
    /// never look at.
    #[serde(default)]
    z: Option<f32>,
    /// Attention: `"none"`, `"glow"` or `"pulse"`.
    ///
    /// A separate channel from `color`, because the accent is how a *person*
    /// organises a basket and borrowing it to shout destroys that organisation.
    /// There is no flash on purpose: above about 3 Hz it is a seizure risk.
    #[serde(default, deserialize_with = "de_emphasis_opt")]
    emphasis: Option<String>,
    /// Halo strength, 0.0–1.0 (clamped). Defaults to 1.0.
    #[serde(default)]
    emphasis_intensity: Option<f32>,
    /// Minutes until the emphasis lapses. **Set this.** Emphasis that never
    /// expires accumulates until every card is shouting and none of them mean
    /// anything; `0` or omitting the field with `emphasis` set to `none` clears
    /// it outright.
    #[serde(default)]
    emphasis_minutes: Option<i64>,
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
#[serde(deny_unknown_fields)]
struct ChecklistItemInput {
    /// The item's stable id, as `GET` returns it.
    ///
    /// **Accepting this is what makes a read-modify-write round-trip work** —
    /// the natural way any client edits a list is to GET the card, change the
    /// array and PATCH it back, and since v0.90.0 that array comes back carrying
    /// ids. Rejecting them turned the obvious pattern into a 400.
    ///
    /// When present it is *honoured*, so identity follows the item rather than
    /// its position: reorder the array, or delete a line from the middle, and
    /// every surviving item keeps the id it had. Omit it and the old
    /// positional carry-over still applies, so existing clients are unaffected.
    #[serde(default)]
    id: u64,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Depth, in the **same units as `pos`** — positive is toward the viewer, so
    /// a larger `z` is nearer. Same units deliberately: "200 nearer" is then the
    /// same size of move as "200 right".
    ///
    /// A basket is a volume. With the canvas's **Depth** toggle off this is just
    /// the stacking order, so it is never meaningless — but it also means the
    /// reader may not see it. **Put arrangement in `z`; put meaning in the text,
    /// a `#tag` or a `key:: value`**, or you have written something they may
    /// never look at.
    #[serde(default)]
    z: Option<f32>,
    /// Attention: `"none"`, `"glow"` or `"pulse"`.
    ///
    /// A separate channel from `color`, because the accent is how a *person*
    /// organises a basket and borrowing it to shout destroys that organisation.
    /// There is no flash on purpose: above about 3 Hz it is a seizure risk.
    #[serde(default, deserialize_with = "de_emphasis_opt")]
    emphasis: Option<String>,
    /// Halo strength, 0.0–1.0 (clamped). Defaults to 1.0.
    #[serde(default)]
    emphasis_intensity: Option<f32>,
    /// Minutes until the emphasis lapses. **Set this.** Emphasis that never
    /// expires accumulates until every card is shouting and none of them mean
    /// anything; `0` or omitting the field with `emphasis` set to `none` clears
    /// it outright.
    #[serde(default)]
    emphasis_minutes: Option<i64>,
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
#[serde(deny_unknown_fields)]
struct CreateGroupInput {
    /// Ids of the cards to group (need at least two that exist in the node).
    cards: Vec<u64>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateGroupInput {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, deserialize_with = "de_color_opt")]
    color: Option<[u8; 3]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
/// carry its arguments (ones the op does not use are ignored).
///
/// **Strict, like every other API input since v0.86.0.** This struct was the one
/// that was missed, and the hole was reported from use: the *op* name was
/// checked but its *fields* were not, so `set_cell` with a misspelt `text`
/// parsed, defaulted to the empty string, wrote a blank cell and answered 200.
/// A silent success that destroys data is worse than the typo it came from.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableOpInput {
    /// Conditional-formatting rules for `set_rules`. Replaces the whole list, so
    /// sending `[]` clears the formatting.
    #[serde(default)]
    rules: Option<Vec<crate::model::CellRule>>,
    /// `set_cell` | `set_bg` | `set_fg` | `insert_row` | `remove_row` |
    /// `insert_col` | `remove_col` | `set_col_width` | `autofit_cols` |
    /// `set_header` | `set_rules`.
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

/// A table request: one op, or a list of them.
///
/// Accepting both because agents reasonably assume a list works — and when it
/// didn't, serde's error was *"invalid type: map, expected a string"*, which
/// names neither the array nor the limitation. Combined with `curl` exiting 0 on
/// a 400, that produced edits reported as applied that never landed.
#[derive(Deserialize)]
pub enum TableOpBody {
    One(TableOpInput),
    Many(Vec<TableOpInput>),
}

impl TableOpBody {
    fn into_vec(self) -> Vec<TableOpInput> {
        match self {
            TableOpBody::One(o) => vec![o],
            TableOpBody::Many(v) => v,
        }
    }
}

impl TableOpInput {
    /// The arguments this op needs, or a 400 saying which one is missing.
    ///
    /// Every argument used to be `unwrap_or(default)`, which turned an omission
    /// into a silent edit of something else: `set_cell` with no `text` blanked a
    /// cell, with no `row`/`col` wrote over `0,0` — usually a header — and
    /// `remove_row` with no `at` deleted the first row. None of those defaults
    /// were ever documented; API.md has always listed these fields as the op's
    /// arguments. `autofit_cols` is the sole documented optional (`col?` = every
    /// column), and a `color` that is absent or null is the documented way to
    /// clear one.
    fn validate(&self) -> Result<(), String> {
        let need = |ok: bool, field: &str| {
            if ok { Ok(()) } else { Err(format!("table op `{}` needs `{field}`", self.op)) }
        };
        match self.op.as_str() {
            "set_cell" => {
                need(self.row.is_some(), "row")?;
                need(self.col.is_some(), "col")?;
                need(self.text.is_some(), "text")
            }
            "set_bg" | "set_fg" => {
                need(self.row.is_some(), "row")?;
                need(self.col.is_some(), "col")
            }
            "insert_row" | "remove_row" | "insert_col" | "remove_col" => {
                need(self.at.is_some(), "at")
            }
            "set_col_width" => {
                need(self.col.is_some(), "col")?;
                need(self.width.is_some(), "width")
            }
            "set_header" => need(self.header.is_some(), "header"),
            "set_rules" => need(self.rules.is_some(), "rules"),
            "autofit_cols" => Ok(()),
            other => Err(format!("unknown table op: {other}")),
        }
    }

    /// The sub-operation's name, for the change log's `table.<op>` field.
    pub fn name(&self) -> &str {
        &self.op
    }
}

/// One edit to a Sketch card. `op` is `add_stroke` | `undo` | `clear`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
    // `/open/...` joins health as unauthenticated. It carries no document data
    // and only moves the window, so a key would buy nothing except making the
    // links unclickable — which is the entire feature.
    let is_health = method == Method::Get
        && (path == "/api/health" || path.starts_with("/open/"));
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
                            // Name the holder, not the kind: the same grant list
                            // now carries plugins and agent tokens, and "'SCOUT'
                            // has read-only access" is what tells whoever reads
                            // the log which credential to go and look at.
                            return ApiResponse::err(
                                403,
                                &format!("'{}' has read-only access to this document", g.plugin),
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
        // Deliberately outside /api: this is a link target, not part of the
        // agent surface, and it is the one route that answers without a key.
        (Method::Get, ["api", "cards", cid, "link"]) => Ok(ApiRequest::CardLink(pid(cid)?)),
        (Method::Get, ["api", "groups", gid, "link"]) => Ok(ApiRequest::GroupLink(pid(gid)?)),
        (Method::Get, ["api", "desktop"]) => Ok(ApiRequest::ListCardDesktop),
        (Method::Post, ["api", "nodes", id, "desktop"]) => {
            Ok(ApiRequest::SetNodeDesktop { node: pid(id)?, on: true })
        }
        (Method::Delete, ["api", "nodes", id, "desktop"]) => {
            Ok(ApiRequest::SetNodeDesktop { node: pid(id)?, on: false })
        }
        (Method::Post, ["api", "cards", cid, "desktop"]) => {
            let i: CardDesktopInput =
                if body.trim().is_empty() { CardDesktopInput { pos: None } } else { parse(body)? };
            Ok(ApiRequest::SetCardDesktop { card: pid(cid)?, pos: i.pos, on: true })
        }
        (Method::Delete, ["api", "cards", cid, "desktop"]) => {
            Ok(ApiRequest::SetCardDesktop { card: pid(cid)?, pos: None, on: false })
        }
        (Method::Get, ["open", "card", cid]) => Ok(ApiRequest::Open {
            kind: OpenKind::Card,
            id: pid(cid)?,
            doc: query_get(query, "doc"),
        }),
        (Method::Get, ["open", "node", id]) => Ok(ApiRequest::Open {
            kind: OpenKind::Node,
            id: pid(id)?,
            doc: query_get(query, "doc"),
        }),
        (Method::Get, ["open", "group", gid]) => Ok(ApiRequest::Open {
            kind: OpenKind::Group,
            id: pid(gid)?,
            doc: query_get(query, "doc"),
        }),
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
        (Method::Post, ["api", "expand"]) => {
            let i: ExpandAllInput = parse(body)?;
            Ok(ApiRequest::SetAllExpanded { expanded: i.expanded })
        }
        (Method::Get, ["api", "nodes", id, "backlinks"]) => Ok(ApiRequest::Backlinks(pid(id)?)),
        (Method::Get, ["api", "nodes", id, "cards"]) => Ok(ApiRequest::ListCards(pid(id)?)),
        (Method::Get, ["api", "nodes", nid, "cards", cid]) => {
            Ok(ApiRequest::GetCard { node: pid(nid)?, card: pid(cid)? })
        }
        (Method::Get, ["api", "cards", cid, "backlinks"]) => Ok(ApiRequest::CardBacklinks(pid(cid)?)),
        (Method::Get, ["api", "cards", cid]) => Ok(ApiRequest::LocateCard(pid(cid)?)),
        // Card-addressed writes. Every one is the same operation as its
        // /api/nodes/{id}/cards/{cid}/… twin — the app loop resolves the basket
        // and hands the request on, so there is one implementation, not two.
        (Method::Patch, ["api", "cards", cid]) => Ok(ApiRequest::ByCard {
            card: pid(cid)?,
            op: CardOp::Patch(parse(body)?),
        }),
        (Method::Delete, ["api", "cards", cid]) => {
            Ok(ApiRequest::ByCard { card: pid(cid)?, op: CardOp::Delete })
        }
        (Method::Post, ["api", "cards", cid, "property"]) => {
            let i: PropertyInput = parse(body)?;
            Ok(ApiRequest::ByCard {
                card: pid(cid)?,
                op: CardOp::SetProperty { key: i.key, value: i.value },
            })
        }
        (Method::Delete, ["api", "cards", cid, "property"]) => {
            let key = query_get(query, "key").unwrap_or_default();
            if key.trim().is_empty() {
                return Err((400, "property to clear: /property?key=due".into()));
            }
            Ok(ApiRequest::ByCard { card: pid(cid)?, op: CardOp::ClearProperty { key } })
        }
        (Method::Post, ["api", "cards", cid, "move"]) => {
            Ok(ApiRequest::ByCard { card: pid(cid)?, op: CardOp::Move(parse(body)?) })
        }
        (Method::Post, ["api", "cards", cid, "items", iid, "done"]) => {
            let i: DoneInput = parse(body)?;
            Ok(ApiRequest::ByCard {
                card: pid(cid)?,
                op: CardOp::ItemDone { item: pid(iid)?, done: i.done },
            })
        }
        (Method::Post, ["api", "cards", cid, "items", iid, "property"]) => {
            let i: PropertyInput = parse(body)?;
            Ok(ApiRequest::ByCard {
                card: pid(cid)?,
                op: CardOp::SetItemProperty { item: pid(iid)?, key: i.key, value: i.value },
            })
        }
        (Method::Delete, ["api", "cards", cid, "items", iid, "property"]) => {
            let key = query_get(query, "key").unwrap_or_default();
            if key.trim().is_empty() {
                return Err((400, "property to clear: /property?key=due".into()));
            }
            Ok(ApiRequest::ByCard {
                card: pid(cid)?,
                op: CardOp::ClearItemProperty { item: pid(iid)?, key },
            })
        }
        (Method::Get, ["api", "groups", gid, "backlinks"]) => {
            Ok(ApiRequest::GroupBacklinks(pid(gid)?))
        }
        (Method::Get, ["api", "groups", gid]) => Ok(ApiRequest::LocateGroup(pid(gid)?)),
        // POST, not GET: it creates the node when it isn't there yet.
        (Method::Get, ["api", "daily"]) => Ok(ApiRequest::DailyConfig),
        (Method::Post, ["api", "daily", "root"]) => {
            let i: DailyRootInput = parse(body)?;
            Ok(ApiRequest::SetDailyRoot(Some(i.node)))
        }
        (Method::Delete, ["api", "daily", "root"]) => Ok(ApiRequest::SetDailyRoot(None)),
        (Method::Post, ["api", "daily"]) => {
            let i: DailyInput = if body.trim().is_empty() { DailyInput { date: None } } else { parse(body)? };
            Ok(ApiRequest::DailyNote { date: i.date })
        }
        (Method::Post, ["api", "nodes", id, "cards"]) => {
            // An array creates a batch; an object creates one. Deciding on the
            // first non-space byte keeps the single-card path byte-identical to
            // what it has always been.
            if body.trim_start().starts_with('[') {
                let inputs: Vec<AddCardInput> = parse(body)?;
                return Ok(ApiRequest::AddCards { node: pid(id)?, inputs });
            }
            let input: AddCardInput = parse(body)?;
            Ok(ApiRequest::AddCard { node: pid(id)?, input })
        }
        // Literal arms first: "move" and "property" sit where a card id would.
        (Method::Post, ["api", "nodes", nid, "cards", "move"]) => {
            let i: MoveCardsInput = parse(body)?;
            Ok(ApiRequest::MoveCards {
                node: pid(nid)?, cards: i.cards, to: i.node, pos: i.pos, gap: i.gap,
            })
        }
        (Method::Post, ["api", "nodes", nid, "cards", "property"]) => {
            let i: CardsPropertyInput = parse(body)?;
            Ok(ApiRequest::SetCardsProperty {
                node: pid(nid)?, cards: i.cards, key: i.key, value: i.value,
            })
        }
        (Method::Delete, ["api", "nodes", nid, "cards", "property"]) => {
            let i: ClearCardsPropertyInput = parse(body)?;
            Ok(ApiRequest::ClearCardsProperty {
                node: pid(nid)?, cards: i.cards, key: i.key,
            })
        }
        // The batch edit and batch delete are the 4-segment collection, so they
        // cannot collide with a card id — but they do share a path with the
        // create, which is why the method is what separates them.
        (Method::Patch, ["api", "nodes", id, "cards"]) => {
            let (cards, patch) = batch_patch(body)?;
            Ok(ApiRequest::UpdateCards { node: pid(id)?, cards, patch })
        }
        (Method::Delete, ["api", "nodes", id, "cards"]) => {
            let i: DeleteCardsInput = parse(body)?;
            Ok(ApiRequest::DeleteCards { node: pid(id)?, cards: i.cards })
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
        (Method::Post, ["api", "nodes", nid, "cards", cid, "items", iid, "property"]) => {
            let i: PropertyInput = parse(body)?;
            Ok(ApiRequest::SetItemProperty {
                node: pid(nid)?, card: pid(cid)?, item: pid(iid)?, key: i.key, value: i.value,
            })
        }
        (Method::Delete, ["api", "nodes", nid, "cards", cid, "items", iid, "property"]) => {
            let key = query_get(query, "key").unwrap_or_default();
            if key.trim().is_empty() {
                return Err((400, "property to clear: /property?key=due".into()));
            }
            Ok(ApiRequest::ClearItemProperty {
                node: pid(nid)?, card: pid(cid)?, item: pid(iid)?, key,
            })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "items", iid, "done"]) => {
            let i: DoneInput = parse(body)?;
            Ok(ApiRequest::SetItemDone {
                node: pid(nid)?, card: pid(cid)?, item: pid(iid)?, done: i.done,
            })
        }
        (Method::Delete, ["api", "nodes", nid, "cards", cid, "property"]) => {
            let key = query_get(query, "key").unwrap_or_default();
            if key.trim().is_empty() {
                return Err((400, "property to clear: /property?key=due".into()));
            }
            Ok(ApiRequest::ClearCardProperty { node: pid(nid)?, card: pid(cid)?, key })
        }
        (Method::Post, ["api", "nodes", nid, "cards", cid, "property"]) => {
            let i: PropertyInput = parse(body)?;
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
            // Parsed by shape rather than through an untagged enum: untagged
            // reports only "data did not match any variant", which hides the
            // real problem (a bad field, a wrong type) behind a message naming
            // neither. Pick the branch from the first character and let serde's
            // own error through.
            let op: TableOpBody = if body.trim_start().starts_with('[') {
                TableOpBody::Many(parse(body)?)
            } else {
                TableOpBody::One(parse(body)?)
            };
            Ok(ApiRequest::TableOp { node: pid(nid)?, card: pid(cid)?, ops: op.into_vec() })
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
        (Method::Post, ["api", "nodes", nid, "groups", gid, "move"]) => {
            let i: MoveGroupInput = parse(body)?;
            Ok(ApiRequest::MoveGroup {
                node: pid(nid)?,
                group: pid(gid)?,
                to: i.node,
                pos: i.pos,
            })
        }
        (Method::Delete, ["api", "nodes", nid, "groups", gid]) => {
            Ok(ApiRequest::DeleteGroup { node: pid(nid)?, group: pid(gid)? })
        }
        (Method::Post, ["api", "nodes", id, "autosort"]) => Ok(ApiRequest::Autosort(pid(id)?)),
        (Method::Get, ["api", "nodes", id, "overlaps"]) => Ok(ApiRequest::Overlaps(pid(id)?)),
        (Method::Post, ["api", "nodes", id, "overlaps"]) => {
            Ok(ApiRequest::ResolveOverlaps(pid(id)?))
        }
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
        (Method::Get, ["api", "claims"]) => Ok(ApiRequest::Claims {
            expired_only: query_get(query, "expired").as_deref() == Some("true"),
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
        (Method::Get, ["api", "settings"]) => Ok(ApiRequest::SettingsGet),
        (Method::Post, ["api", "settings"]) => {
            let v: Value = serde_json::from_str(body)
                .map_err(|e| (400, format!("invalid JSON body: {e}")))?;
            match v {
                Value::Object(m) if !m.is_empty() => Ok(ApiRequest::SettingsSet(m)),
                _ => Err((400, "expected a JSON object of setting names to values".to_string())),
            }
        }
        (Method::Get, ["api", "backup"]) => Ok(ApiRequest::BackupStatus),
        (Method::Post, ["api", "backup", "run"]) => Ok(ApiRequest::BackupRun),
        (Method::Post, ["api", "ocr"]) => Ok(ApiRequest::OcrAll),
        (Method::Get, ["api", "history"]) => Ok(ApiRequest::HistoryList),
        (Method::Post, ["api", "history", "restore"]) => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
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
            #[serde(deny_unknown_fields)]
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
            #[serde(deny_unknown_fields)]
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
            #[serde(deny_unknown_fields)]
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
/// Today, as the journal writes it. Local calendar day, from the same clock
/// `today_days()` reads — an agenda that says "today" and a journal node named
/// for a different day would be its own bug.
pub fn today_daily_date() -> crate::model::DailyDate {
    from_naive(chrono::Local::now().date_naive())
}

/// The same, for an explicit `YYYY-MM-DD`. `None` if it isn't a real date —
/// 2026-02-30 is rejected rather than rounded, because a journal node named for
/// a day that does not exist is worse than an error.
pub fn daily_date_from(s: &str) -> Option<crate::model::DailyDate> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok().map(from_naive)
}

/// `YYYY-MM-DD` for a day relative to today — what the Agenda's reschedule
/// shortcuts write into `due::`. Months are added calendar-wise (chrono clamps
/// 31 Jan + 1 month to 28/29 Feb rather than overflowing into March).
pub fn date_from_today(days: i64, months: u32) -> String {
    let mut d = chrono::Local::now().date_naive();
    if months > 0 {
        d = d.checked_add_months(chrono::Months::new(months)).unwrap_or(d);
    }
    if days != 0 {
        d = d.checked_add_signed(chrono::Duration::days(days)).unwrap_or(d);
    }
    d.format("%Y-%m-%d").to_string()
}

fn from_naive(d: chrono::NaiveDate) -> crate::model::DailyDate {
    use chrono::Datelike;
    crate::model::DailyDate {
        year: d.year(),
        month: d.month(),
        day: d.day(),
        weekday: d.format("%A").to_string(),
        month_name: d.format("%B").to_string(),
    }
}

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

/// The bucket for a task that may occupy a **span** of days.
///
/// A task with `start:: 2026-08-11  due:: 2026-08-15` is genuinely in flight on
/// the 12th, 13th and 14th — it is not "later", it is *now*, and burying it
/// under a future date is how a multi-day piece of work disappears until the day
/// it is already late. Started work therefore reads as **today** until it is
/// overdue, which is where the "sliding" behaviour comes from: the card never
/// moves, the window does.
pub fn task_bucket_spanning(t: &crate::model::TaskItem, today: i64) -> &'static str {
    match (t.start_days, t.due_days) {
        (Some(s), Some(d)) if s <= today && today < d => "today",
        (Some(s), None) if s <= today => "today",
        _ => task_bucket(t.due_days, today),
    }
}

/// Which currency bucket a `verify::` date falls in, relative to `today`.
///
/// The asymmetry with [`task_bucket`] is deliberate and is the whole point: an
/// overdue task is *late*, an expired claim is **not to be trusted**. So the
/// past tense is one bucket rather than three, and a date nobody can parse is
/// `unparsed` rather than quietly counted as fresh — a claim whose expiry cannot
/// be read has, in effect, no expiry at all, which is the thing this exists to
/// stop.
pub fn claim_bucket(verify_days: Option<i64>, today: i64) -> &'static str {
    match verify_days {
        None => "unparsed",
        Some(d) if d < today => "expired",
        Some(d) if d == today => "today",
        Some(d) if d <= today + 7 => "soon",
        Some(_) => "ok",
    }
}

/// How many claims in `doc` are past their check date (or carry a date nobody
/// can parse). This is the number [`ApiRequest::Instance`] reports, because
/// `/api/instance` is the one call every agent already makes first.
pub fn stale_claim_count(doc: &crate::model::Document) -> usize {
    let today = today_days();
    doc.claims()
        .iter()
        .filter(|c| matches!(claim_bucket(c.verify_days, today), "expired" | "unparsed"))
        .count()
}

/// Content fields: legal on a single card, refused in a batch.
///
/// Each of these *is* the card — writing one across a list means every card in
/// the list ends up saying the same thing. See [`ApiRequest::UpdateCards`].
const BATCH_FORBIDDEN: [&str; 9] =
    ["title", "body", "items", "rows", "kind", "lang", "header", "source", "inline_images"];

/// Split `cards` off a batch-PATCH body and validate the rest as an ordinary
/// card patch.
///
/// The remainder is deserialized into [`UpdateCardInput`] — the very struct the
/// single-card `PATCH` uses — so a misspelt field is still the 400 naming it that
/// v0.86.0 promised, and there is no second list of legal fields to drift from
/// the first. What this function adds is the *refusal*: a content field reaching
/// a whole list of cards is rejected by name, with the route that does accept it.
fn batch_patch(body: &str) -> Result<(Vec<u64>, UpdateCardInput), (u16, String)> {
    let mut map: serde_json::Map<String, Value> = parse(body)?;
    let Some(cards) = map.remove("cards") else {
        return Err((400, "batch edit needs \"cards\": [ids]".into()));
    };
    let cards: Vec<u64> = serde_json::from_value(cards)
        .map_err(|e| (400, format!("invalid \"cards\" list: {e}")))?;
    for f in BATCH_FORBIDDEN {
        if map.contains_key(f) {
            return Err((
                400,
                format!(
                    "\"{f}\" is content, not presentation, so it is refused for a list of \
                     cards — it would write the same {f} over every one of them. Use PATCH \
                     /api/nodes/{{id}}/cards/{{cid}} for that, one card at a time. A batch \
                     may set: color, size, fit, font_scale, z, emphasis, emphasis_intensity, \
                     emphasis_minutes."
                ),
            ));
        }
    }
    // The "expected one of …" list serde produces here is `UpdateCardInput`'s
    // whole field set, which includes the content fields this route refuses — so
    // say so, rather than listing `body` as expected and then rejecting it.
    let patch: UpdateCardInput = serde_json::from_value(Value::Object(map)).map_err(|e| {
        (
            400,
            format!(
                "invalid JSON body: {e} (of those, {} are single-card only)",
                BATCH_FORBIDDEN.join(", ")
            ),
        )
    })?;
    Ok((cards, patch))
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

/// Reject an emphasis name at parse time, so a typo is a 400 that lists what was
/// expected rather than a field that silently does nothing — the same rule the
/// rest of the API's input has followed since v0.86.0.
fn de_emphasis_opt<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    if crate::model::Emphasis::from_key(&s).is_none() {
        return Err(serde::de::Error::custom(format!(
            "unknown emphasis {s:?} (expected \"none\", \"glow\" or \"pulse\")"
        )));
    }
    Ok(Some(s))
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
        ApiRequest::SetItemProperty { node, card, item, key, value } => {
            let ok = doc.set_item_property(node, card, item, &key, &value);
            if ok {
                (true, ApiResponse::ok(json!({ "item": item, "key": key, "value": value })))
            } else {
                (false, ApiResponse::err(404, "no such checklist item on that card"))
            }
        }
        ApiRequest::ClearItemProperty { node, card, item, key } => {
            if doc.item_mut(node, card, item).is_none() {
                return (false, ApiResponse::err(404, "no such checklist item on that card"));
            }
            let cleared = doc.clear_item_property(node, card, item, &key);
            (cleared, ApiResponse::ok(json!({ "item": item, "key": key, "cleared": cleared })))
        }
        ApiRequest::SetItemDone { node, card, item, done } => {
            let ok = doc.set_item_done(node, card, item, done);
            if ok {
                (true, ApiResponse::ok(json!({ "item": item, "done": done })))
            } else {
                (false, ApiResponse::err(404, "no such checklist item on that card"))
            }
        }
        ApiRequest::ClearCardProperty { node, card, key } => {
            if doc.card(node, card).is_none() {
                return (false, ApiResponse::err(404, "card not found"));
            }
            let removed = doc.clear_card_property(node, card, &key);
            (removed, ApiResponse::ok(json!({ "cleared": removed, "key": key })))
        }
        ApiRequest::CardBacklinks(card) => match doc.locate_card(card) {
            Some(node) => {
                let hits: Vec<Value> = doc
                    .backlinks_card(node, card)
                    .into_iter()
                    .map(|h| json!({
                        "node": h.node,
                        "card": h.card,
                        "node_title": h.node_title,
                        "node_path": doc.node_path(h.node),
                        "snippet": h.snippet,
                    }))
                    .collect();
                (false, ApiResponse::ok(json!({ "card": card, "node": node, "hits": hits })))
            }
            None => (false, ApiResponse::err(404, "card not found")),
        },
        ApiRequest::GetCard { node, card } => match doc.card(node, card) {
            Some(c) => (false, ApiResponse::ok(card_json(c))),
            None => (false, ApiResponse::err(404, "card not found")),
        },
        // The basket comes back with the card: an id alone is enough to *find* a
        // card, but every route that edits one still needs the node, so returning
        // it here saves the caller a second lookup it has no way to perform.
        ApiRequest::LocateCard(card) => match doc.locate_card(card) {
            Some(node) => {
                let c = match doc.card(node, card) {
                    Some(c) => c,
                    None => return (false, ApiResponse::err(404, "card not found")),
                };
                (
                    false,
                    ApiResponse::ok(json!({
                        "node": node,
                        "node_title": doc.nodes.get(&node).map(|n| n.title.clone()).unwrap_or_default(),
                        "node_path": doc.node_path(node),
                        "card": card_json(c),
                    })),
                )
            }
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
        ApiRequest::SetAllExpanded { expanded } => {
            let changed = doc.set_all_expanded(expanded);
            (changed > 0, ApiResponse::ok(json!({ "expanded": expanded, "changed": changed })))
        }
        ApiRequest::AddCard { node, input } => match add_one(doc, node, input) {
            Ok(cid) => (true, ApiResponse::created(json!({ "id": cid }))),
            Err(e) => (false, e),
        },
        // Same creation path as the single card, called once per input — the
        // two cannot drift because there is only one of them. Validated up
        // front like the batch move: an unknown node refuses the whole batch
        // rather than half-creating it.
        ApiRequest::AddCards { node, inputs } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            if inputs.is_empty() {
                return (false, ApiResponse::err(400, "the array must contain at least one card"));
            }
            let mut ids = Vec::with_capacity(inputs.len());
            for input in inputs {
                match add_one(doc, node, input) {
                    Ok(cid) => ids.push(cid),
                    Err(e) => return (!ids.is_empty(), e),
                }
            }
            (true, ApiResponse::created(json!({ "created": ids.len(), "ids": ids })))
        }
        ApiRequest::UpdateCard { node, card, patch } => match doc.card_mut(node, card) {
            Some(c) => {
                // A mirrored body belongs to the file. Silently accepting an edit
                // that the next refresh overwrites would look like data loss, so
                // it's refused rather than ignored.
                //
                // Checked here, before **anything** is applied: this test used to
                // sit after `title`, so a refused request had already renamed the
                // card. A 409 that changed something is worse than either
                // outcome, because the caller has no way to know what stuck.
                if patch.body.is_some() && c.source.is_some() {
                    return (
                        false,
                        ApiResponse::err(
                            409,
                            "this card mirrors a file — its body is read-only. \
                             Send \"source\": \"\" to detach it first.",
                        ),
                    );
                }
                apply_presentation(c, &patch);
                if let Some(t) = patch.title {
                    c.title = t;
                }
                if let Some(b) = patch.body {
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
                        // **Carry the existing ids across, by position.** A
                        // wholesale replace is how most clients edit a checklist,
                        // and minting fresh ids here would silently destroy the
                        // identity of every line on every edit — breaking links
                        // to items and making each task look new to anything
                        // tracking it. Positional carry-over is right for the
                        // common cases (edit a line, tick a box, append); a
                        // client that *reorders* by rewriting the array is
                        // telling us these are different lines, and gets that.
                        // Reordering without losing identity is what the
                        // checklist op surface is for.
                        let old_ids: Vec<crate::model::ItemId> = it.iter().map(|i| i.id).collect();
                        // Which rule applies is decided by the payload as a
                        // whole, never per item. Mixing them hands the same id
                        // to two lines: a new line at position 2 would inherit
                        // the old position-2 id while another line claims that
                        // id explicitly, and two items with one identity is
                        // worse than no identity at all.
                        let id_aware = items.iter().any(|i| i.id != 0);
                        *it = items
                            .into_iter()
                            .enumerate()
                            .map(|(n, i)| ChecklistItem {
                                id: if id_aware {
                                    // The client speaks ids: an id names the
                                    // line, so reordering and deleting from the
                                    // middle keep every survivor's identity, and
                                    // a line without one is simply new.
                                    i.id
                                } else {
                                    // A client that sends no ids at all gets the
                                    // positional carry-over, so the ordinary
                                    // edits it makes still preserve identity.
                                    old_ids.get(n).copied().unwrap_or(0)
                                },
                                done: i.done,
                                text: i.text,
                            })
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
        // The basket rides along for the same reason it does on LocateCard:
        // every route that edits a group is still /nodes/{node}/groups/{gid},
        // so an id alone finds it but cannot change it.
        ApiRequest::LocateGroup(group) => match doc.locate_group(group) {
            Some(node) => {
                let n = match doc.nodes.get(&node) {
                    Some(n) => n,
                    None => return (false, ApiResponse::err(404, "group not found")),
                };
                let g = match n.groups.iter().find(|g| g.id == group) {
                    Some(g) => g,
                    None => return (false, ApiResponse::err(404, "group not found")),
                };
                let cards: Vec<u64> =
                    n.cards.iter().filter(|c| c.group == Some(group)).map(|c| c.id).collect();
                (
                    false,
                    ApiResponse::ok(json!({
                        "node": node,
                        "node_title": n.title.clone(),
                        "node_path": doc.node_path(node),
                        "group": {
                            "id": g.id,
                            "title": g.title,
                            "color": g.color,
                            "cards": cards,
                        },
                    })),
                )
            }
            None => (false, ApiResponse::err(404, "group not found")),
        },
        ApiRequest::GroupBacklinks(group) => match doc.locate_group(group) {
            Some(node) => {
                let hits: Vec<Value> = doc
                    .backlinks_group(group)
                    .into_iter()
                    .map(|h| json!({
                        "node": h.node,
                        "card": h.card,
                        "node_title": h.node_title,
                        "node_path": doc.node_path(h.node),
                        "snippet": h.snippet,
                    }))
                    .collect();
                (false, ApiResponse::ok(json!({ "group": group, "node": node, "hits": hits })))
            }
            None => (false, ApiResponse::err(404, "group not found")),
        },
        ApiRequest::MoveGroup { node, group, to, pos } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            if !doc.nodes.contains_key(&to) {
                return (false, ApiResponse::err(404, "destination node not found"));
            }
            if !group_exists(doc, node, group) {
                return (false, ApiResponse::err(404, "group not found"));
            }
            if node == to {
                return (false, ApiResponse::err(400, "group is already in that node"));
            }
            let at = pos.map(|p| egui::pos2(p[0], p[1]));
            match doc.move_group_to_node(node, group, to, at) {
                Some(moved) => (
                    true,
                    ApiResponse::ok(json!({ "group": group, "node": to, "moved": moved })),
                ),
                None => (false, ApiResponse::err(404, "group not found")),
            }
        }
        ApiRequest::MoveCards { node, cards, to, pos, gap } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            if !doc.nodes.contains_key(&to) {
                return (false, ApiResponse::err(404, "destination node not found"));
            }
            if node == to {
                return (false, ApiResponse::err(400, "those cards are already in that node"));
            }
            if cards.is_empty() {
                return (false, ApiResponse::err(400, "cards must name at least one card"));
            }
            // Validate the whole list BEFORE moving any of it. A partial move
            // leaves the caller with no way to know how far it got — the same
            // reason table ops validate a batch up front.
            for cid in &cards {
                if doc.card(node, *cid).is_none() {
                    return (
                        false,
                        ApiResponse::err(404, &format!("card {cid} is not in node {node}")),
                    );
                }
            }
            let mut moved = Vec::with_capacity(cards.len());
            let mut cursor = pos.map(|p| egui::pos2(p[0], p[1]));
            for cid in &cards {
                // Height is read before the move, while the card is still here.
                let h = doc.card(node, *cid).map(|c| c.size.y).unwrap_or(0.0);
                if doc.move_card_to_node(node, *cid, to, cursor).is_some() {
                    moved.push(*cid);
                    if let Some(c) = cursor.as_mut() {
                        c.y += h + gap;
                    }
                }
            }
            (
                true,
                ApiResponse::ok(json!({ "moved": moved.len(), "node": to, "cards": moved })),
            )
        }
        ApiRequest::UpdateCards { node, cards, patch } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            if cards.is_empty() {
                return (false, ApiResponse::err(400, "cards must name at least one card"));
            }
            for cid in &cards {
                if doc.card(node, *cid).is_none() {
                    return (
                        false,
                        ApiResponse::err(404, &format!("card {cid} is not in node {node}")),
                    );
                }
            }
            let mut done = Vec::with_capacity(cards.len());
            for cid in &cards {
                if let Some(c) = doc.card_mut(node, *cid) {
                    apply_presentation(c, &patch);
                    // An estimate, like every other `fit` on this thread; the app
                    // loop re-measures each of these with the real fonts.
                    if patch.fit {
                        if let Some(sz) = c.fit_size() {
                            c.size = sz;
                        }
                    }
                    done.push(*cid);
                }
            }
            (true, ApiResponse::ok(json!({ "updated": done.len(), "cards": done })))
        }
        ApiRequest::DeleteCards { node, cards } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            if cards.is_empty() {
                return (false, ApiResponse::err(400, "cards must name at least one card"));
            }
            // Validated in full before a single card is removed. This matters
            // more here than anywhere else in the batch surface: a half-finished
            // delete cannot be undone by re-sending the request.
            for cid in &cards {
                if doc.card(node, *cid).is_none() {
                    return (
                        false,
                        ApiResponse::err(404, &format!("card {cid} is not in node {node}")),
                    );
                }
            }
            for cid in &cards {
                doc.remove_card(node, *cid);
            }
            (true, ApiResponse::ok(json!({ "deleted": cards.len(), "cards": cards })))
        }
        ApiRequest::ClearCardsProperty { node, cards, key } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            if cards.is_empty() {
                return (false, ApiResponse::err(400, "cards must name at least one card"));
            }
            for cid in &cards {
                if doc.card(node, *cid).is_none() {
                    return (
                        false,
                        ApiResponse::err(404, &format!("card {cid} is not in node {node}")),
                    );
                }
            }
            // A card that never carried the property is not an error — asking for
            // `due::` to be gone from a list and getting that is the point. The
            // count says how many lines were actually removed and `cards` names
            // which, so "8 of 20 had one" is legible rather than hidden.
            let mut cleared = Vec::new();
            for cid in &cards {
                if doc.clear_card_property(node, *cid, &key) {
                    cleared.push(*cid);
                }
            }
            (
                !cleared.is_empty(),
                ApiResponse::ok(json!({
                    "cleared": cleared.len(),
                    "cards": cleared,
                    "key": key.to_lowercase(),
                })),
            )
        }
        ApiRequest::SetCardsProperty { node, cards, key, value } => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            if cards.is_empty() {
                return (false, ApiResponse::err(400, "cards must name at least one card"));
            }
            for cid in &cards {
                if doc.card(node, *cid).is_none() {
                    return (
                        false,
                        ApiResponse::err(404, &format!("card {cid} is not in node {node}")),
                    );
                }
            }
            let mut done = Vec::with_capacity(cards.len());
            for cid in &cards {
                if doc.set_card_property(node, *cid, &key, &value) {
                    done.push(*cid);
                }
            }
            (
                true,
                ApiResponse::ok(json!({
                    "updated": done.len(),
                    "cards": done,
                    "key": key.to_lowercase(),
                    "value": value,
                })),
            )
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
        ApiRequest::TableOp { node, card, ops } => {
            if ops.is_empty() {
                return (false, ApiResponse::err(400, "no table ops given"));
            }
            // Check the whole batch before touching the card. A malformed op is
            // knowable without applying anything, so it must not be discovered
            // half way through — that would leave the earlier edits in place and
            // the table in a state nobody asked for.
            for (i, op) in ops.iter().enumerate() {
                if let Err(e) = op.validate() {
                    return (
                        false,
                        ApiResponse::err(
                            400,
                            &format!("table op {}/{}: {e} (nothing was applied)", i + 1, ops.len()),
                        ),
                    );
                }
            }
            // Applied in order, stopping at the first failure and saying which
            // one — a batch that half-applies and reports plain success is
            // exactly the failure this batching was added to prevent.
            let total = ops.len();
            for (i, op) in ops.into_iter().enumerate() {
                let name = op.op.clone();
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
                    // Colour cells by value. Applied immediately and re-applied
                    // after every `source` refresh, so live data stays coloured.
                    "set_rules" => match doc.table_mut(node, card) {
                        Some(t) => {
                            t.rules = op.rules.clone().unwrap_or_default();
                            t.apply_rules();
                            true
                        }
                        None => false,
                    },
                    other => {
                        return (false, ApiResponse::err(400, &format!("unknown table op: {other}")));
                    }
            };
                if !ok {
                    return (
                        i > 0,
                        ApiResponse::err(
                            400,
                            &format!(
                                "table op {}/{} ({name}) failed (not a table, or index out of \
                                 range); {} earlier op(s) were applied",
                                i + 1,
                                total,
                                i
                            ),
                        ),
                    );
                }
            }
            let c = doc.card_mut(node, card).unwrap();
            (true, ApiResponse::ok(card_json(c)))
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
        ApiRequest::Overlaps(node) => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            let pairs: Vec<Value> = doc
                .overlapping_cards(node)
                .into_iter()
                .map(|(a, b)| json!({ "a": a, "b": b }))
                .collect();
            (false, ApiResponse::ok(json!({ "node": node, "overlaps": pairs })))
        }
        ApiRequest::ResolveOverlaps(node) => {
            if !doc.nodes.contains_key(&node) {
                return (false, ApiResponse::err(404, "node not found"));
            }
            let moved = doc.resolve_overlaps(node);
            (moved > 0, ApiResponse::ok(json!({ "node": node, "moved": moved })))
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
                        // Present when the task is one LINE of a checklist —
                        // the address a client needs to change that line rather
                        // than the whole card.
                        "item": t.item,
                        "title": t.title,
                        "due": t.due,
                        // Present when the task spans days rather than naming a
                        // single deadline.
                        "start": t.start,
                        "live_today": t.live_on(today),
                        "done": t.done,
                        "bucket": task_bucket_spanning(&t, today),
                    })
                })
                .collect();
            (false, ApiResponse::ok(json!({ "today_days": today, "count": tasks.len(), "tasks": tasks })))
        }
        ApiRequest::Claims { expired_only, project } => {
            let today = today_days();
            if let Some(p) = project {
                if !doc.nodes.contains_key(&p) {
                    return (false, ApiResponse::err(404, "project node not found"));
                }
            }
            let mut claims: Vec<(&'static str, Value)> = doc
                .claims()
                .into_iter()
                .filter(|c| project.map_or(true, |p| doc.is_under(c.node, p)))
                .map(|c| {
                    let bucket = claim_bucket(c.verify_days, today);
                    (
                        bucket,
                        json!({
                            "node": c.node,
                            "node_title": c.node_title,
                            "node_path": c.node_path,
                            "project": c.root,
                            "project_title": c.root_title,
                            "card": c.card,
                            "title": c.title,
                            "verify": c.verify,
                            // How to settle the claim, when the card said.
                            "check": c.check,
                            // When the card was last EDITED — not when anyone
                            // confirmed it. The two are different questions and
                            // conflating them is what makes a stale card look
                            // fresh.
                            "touched": c.touched,
                            "bucket": bucket,
                        }),
                    )
                })
                .filter(|(b, _)| !expired_only || matches!(*b, "expired" | "unparsed"))
                .collect();
            // Worst first: whatever a caller does with this, the untrustworthy
            // claims are the ones it must not miss.
            let rank = |b: &str| match b {
                "expired" => 0,
                "unparsed" => 1,
                "today" => 2,
                "soon" => 3,
                _ => 4,
            };
            claims.sort_by_key(|(b, _)| rank(b));
            let stale = claims.iter().filter(|(b, _)| matches!(*b, "expired" | "unparsed")).count();
            let out: Vec<Value> = claims.into_iter().map(|(_, v)| v).collect();
            (
                false,
                ApiResponse::ok(json!({
                    "today_days": today,
                    "count": out.len(),
                    "stale": stale,
                    "claims": out,
                })),
            )
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
        | ApiRequest::SettingsGet
        | ApiRequest::SettingsSet(_)
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
        | ApiRequest::TemplateDelete(_)
        | ApiRequest::Open { .. }
        | ApiRequest::CardLink(_)
        | ApiRequest::GroupLink(_)
        | ApiRequest::SetCardDesktop { .. }
        | ApiRequest::SetNodeDesktop { .. }
        | ApiRequest::ListCardDesktop
        | ApiRequest::DailyNote { .. }
        | ApiRequest::DailyConfig
        | ApiRequest::SetDailyRoot(_) => {
            (false, ApiResponse::err(500, "request not handled by the app loop"))
        }
        // A card-addressed request is rewritten into its node-addressed twin by
        // the app loop, which is where an id can be resolved to a basket. Reaching
        // `process` means that did not happen, and applying it here — with no way
        // to check the token's scope against the basket it lands in — is exactly
        // the hole this shape exists to avoid.
        ApiRequest::ByCard { .. } => {
            (false, ApiResponse::err(500, "card-addressed request reached process unresolved"))
        }
    }
}

/// Create one card from an [`AddCardInput`].
///
/// Extracted so the single create and the batch create are the *same* code:
/// two copies of this would drift, and the drift would show up as a card that
/// behaves differently depending on how it was made.
fn add_one(doc: &mut Document, node: NodeId, input: AddCardInput) -> Result<u64, ApiResponse> {
    if !doc.nodes.contains_key(&node) {
                return Err(ApiResponse::err(404, "node not found"));
            }
            let kind = match input.kind.as_str() {
                "code" => CardKind::Code { lang: input.lang.clone().unwrap_or_else(|| "text".into()) },
                "checklist" => CardKind::Checklist {
                    items: input
                        .items
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|i| ChecklistItem { id: 0, done: i.done, text: i.text })
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
                        if let Some(z) = input.z {
                            c.z = z.clamp(crate::canvas::Z_MIN, crate::canvas::Z_MAX);
                        }
                        apply_emphasis(
                            c,
                            input.emphasis.as_deref(),
                            input.emphasis_intensity,
                            input.emphasis_minutes,
                        );
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
                    Ok(cid)
                }
                None => Err(ApiResponse::err(404, "node not found")),
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
        "z": c.z,
        "size": [c.size.x, c.size.y],
        "color": c.color,
        "group": c.group,
        "docked_to": c.docked_to,
        "font_scale": c.font_scale,
    });
    // Only when it is set, so a document full of ordinary cards does not grow a
    // field per card in every response.
    if c.emphasis != crate::model::Emphasis::None {
        v["emphasis"] = json!(c.emphasis.key());
        v["emphasis_intensity"] = json!(c.emphasis_intensity);
        if let Some(until) = c.emphasis_until {
            v["emphasis_until"] = json!(until);
            // Say whether it is still in force, so a reader does not have to know
            // what the clock says here — the same reason `/api/tasks` buckets
            // dates rather than handing them over raw.
            v["emphasis_live"] =
                json!(c.live_emphasis(crate::changelog::now_secs() as i64).key());
        }
    }
    // Only when there is one: a card never edited since this existed reports no
    // time rather than a made-up one.
    if let Some(t) = c.touched {
        v["touched"] = json!(t);
    }
    if let CardKind::Table { table } = &c.kind {
        if !table.rules.is_empty() {
            v["rules"] = serde_json::to_value(&table.rules).unwrap_or(Value::Null);
        }
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
                // The id is what a caller addresses to change one line —
                // `…/items/{id}/property`. Without it here, the only way to
                // learn an item's id is /api/tasks, which lists only the dated
                // ones, so an undated line would be unreachable.
                .map(|i| json!({ "id": i.id, "done": i.done, "text": i.text }))
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

/// Apply the three emphasis inputs to a card, as one unit.
///
/// Shared by create and PATCH so the rules cannot drift between them — which is
/// the failure the table ops had before v0.102.0, where each path grew its own
/// quiet defaults.
///
/// **An unknown value is ignored rather than refused here** only because the
/// field is validated at parse time; anything reaching this is already one of
/// the three names. Intensity is clamped rather than rejected: a caller asking
/// for 3.0 wants "as loud as possible", and there is no reading of that which
/// should fail the whole request.
/// The fields a **batch** edit may set, applied to one card.
///
/// Called by both the single-card `PATCH` and the batch, so every clamp and
/// default is written once. Two copies of "size has an 80×60 floor" is how the
/// two paths end up disagreeing — which already happened once with `fit`, where
/// the menu measured the real galley and the API only estimated.
///
/// `fit` is **not** here: it has to run after content fields the batch cannot
/// set, and the app loop re-measures it with real fonts afterwards.
fn apply_presentation(c: &mut crate::model::Card, p: &UpdateCardInput) {
    if let Some(col) = p.color {
        c.color = col;
    }
    if let Some(fs) = p.font_scale {
        c.font_scale = fs.clamp(0.25, 4.0);
    }
    if let Some(z) = p.z {
        // Clamped to the same range the canvas gesture uses: beyond it a card is
        // through the camera or too small to read, and both are ways of losing a
        // card the user cannot easily undo.
        c.z = z.clamp(crate::canvas::Z_MIN, crate::canvas::Z_MAX);
    }
    apply_emphasis(c, p.emphasis.as_deref(), p.emphasis_intensity, p.emphasis_minutes);
    if let Some([w, h]) = p.size {
        c.size = egui::vec2(w, h).max(egui::vec2(80.0, 60.0));
    }
}

fn apply_emphasis(
    c: &mut crate::model::Card,
    emphasis: Option<&str>,
    intensity: Option<f32>,
    minutes: Option<i64>,
) {
    use crate::model::Emphasis;
    if let Some(name) = emphasis {
        if let Some(e) = Emphasis::from_key(name) {
            c.emphasis = e;
            if e == Emphasis::None {
                // Turning it off clears the expiry too: a lapsed timer on a card
                // with no emphasis is a trap for whoever reads it next.
                c.emphasis_until = None;
            }
        }
    }
    if let Some(v) = intensity {
        c.emphasis_intensity = v.clamp(0.0, 1.0);
    }
    if let Some(m) = minutes {
        c.emphasis_until = if m <= 0 {
            None
        } else {
            Some(crate::changelog::now_secs() as i64 + m * 60)
        };
    }
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
            (ApiRequest::Overlaps(5), Some(5)),
            (ApiRequest::ResolveOverlaps(5), Some(5)),
            // Whole-document, so a confined token is refused rather than
            // silently allowed to fold someone else's tree.
            (ApiRequest::SetAllExpanded { expanded: true }, None),
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
        // Locating a card names no basket until the document resolves one, so it
        // must not be neutral either — the app loop resolves it and checks the
        // basket it lands in. Neutral here would hand a confined token every card
        // in the document, one id at a time.
        assert!(!is_scope_neutral(&ApiRequest::LocateCard(1)));
        assert_eq!(target_node(&ApiRequest::LocateCard(1)), None);
    }


    /// Agents must keep working by default — blocking them outright would defeat
    /// the point of the feature. Only credential-shaped paths are refused.
    #[test]
    fn the_default_mirror_policy_allows_agents_but_not_credentials() {
        use crate::model::{mirror_allowed, MirrorPolicy};
        let d: Vec<String> = vec![];
        assert!(mirror_allowed("/srv/app/README.md", MirrorPolicy::SafeDefault, &d).is_ok());
        assert!(mirror_allowed("/var/log/app.log", MirrorPolicy::SafeDefault, &d).is_ok());
        for bad in ["/home/u/.ssh/id_rsa", "/home/u/.aws/credentials", "/home/u/key.pem",
                    "/etc/shadow", "/home/u/.git-credentials"] {
            assert!(mirror_allowed(bad, MirrorPolicy::SafeDefault, &d).is_err(), "{bad} allowed");
        }
        // "Anywhere" means anywhere, including those.
        assert!(mirror_allowed("/home/u/.ssh/id_rsa", MirrorPolicy::Anywhere, &d).is_ok());
    }

    /// A directory list must survive `..` — a textual prefix check would let
    /// `/allowed/../../etc/shadow` through, making the setting decorative.
    #[test]
    fn a_directory_list_is_resolved_not_string_matched() {
        use crate::model::{mirror_allowed, MirrorPolicy};
        let tmp = std::env::temp_dir();
        let root = tmp.join(format!("trellis-mirror-{}", std::process::id()));
        let inside = root.join("ok.md");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&inside, "x").unwrap();
        let dirs = vec![root.to_string_lossy().to_string()];

        assert!(mirror_allowed(&inside.to_string_lossy(), MirrorPolicy::OnlyDirs, &dirs).is_ok());
        let escape = root.join("..").join("..").join("etc").join("shadow");
        assert!(
            mirror_allowed(&escape.to_string_lossy(), MirrorPolicy::OnlyDirs, &dirs).is_err(),
            "traversal escaped the allow-list"
        );
        assert!(mirror_allowed("/somewhere/else.md", MirrorPolicy::OnlyDirs, &dirs).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Only a request that actually names a file is checked; detaching reaches
    /// no file and must not be refused.
    #[test]
    fn only_a_real_source_is_policy_checked() {
        let mk = |json: &str| -> ApiRequest {
            ApiRequest::UpdateCard { node: 1, card: 1, patch: serde_json::from_str(json).unwrap() }
        };
        assert_eq!(source_requests(&mk(r#"{"source":"/srv/a.md"}"#)), vec!["/srv/a.md"]);
        assert!(source_requests(&mk(r#"{"source":""}"#)).is_empty(), "detach reaches no file");
        assert!(source_requests(&mk(r#"{"body":"x"}"#)).is_empty());
    }


    /// The exact failure another agent hit: a list of ops was rejected with
    /// serde's *"invalid type: map, expected a string"*, which names neither the
    /// array nor the one-op-per-call limit. With `curl` exiting 0 on a 400, that
    /// read as success and the edits never landed.
    #[test]
    fn a_batch_of_table_ops_applies_in_order() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "B".into());
        let c = doc
            .add_card(n, emath::pos2(0.0, 0.0), CardKind::Table { table: crate::model::TableData::empty(3, 3) })
            .unwrap();
        let body = r#"[{"op":"set_cell","row":0,"col":0,"text":"hi"},
                       {"op":"set_header","header":true},
                       {"op":"set_bg","row":0,"col":0,"color":"red"}]"#;
        let req = route(&Method::Post, &format!("/api/nodes/{n}/cards/{c}/table"), "", body).unwrap();
        let (changed, resp) = process(&mut doc, req);
        assert!(changed);
        assert_eq!(resp.status, 200, "{}", resp.body);

        let card = doc.card(n, c).unwrap();
        let CardKind::Table { table } = &card.kind else { panic!("not a table") };
        assert_eq!(table.rows[0][0].text, "hi");
        assert!(table.header);
        assert_eq!(table.rows[0][0].bg, Some([239, 68, 68]));
    }

    /// Reported from use: the *op* was checked but its *fields* were not, so a
    /// misspelt argument parsed, defaulted, and destroyed the cell it claimed to
    /// set — returning 200. Both halves are pinned here: an unknown field is
    /// refused by name, and a missing one is refused by name.
    #[test]
    fn a_table_op_refuses_a_typo_instead_of_writing_a_blank_cell() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "B".into());
        let c = doc
            .add_card(n, emath::pos2(0.0, 0.0), CardKind::Table { table: crate::model::TableData::empty(3, 3) })
            .unwrap();
        let path = format!("/api/nodes/{n}/cards/{c}/table");
        // Put something there, so a silent blanking would be visible.
        let req = route(&Method::Post, &path, "", r#"{"op":"set_cell","row":1,"col":1,"text":"keep"}"#).unwrap();
        assert_eq!(process(&mut doc, req).1.status, 200);

        let cell = |doc: &Document| {
            let CardKind::Table { table } = &doc.card(n, c).unwrap().kind else { panic!() };
            table.rows[1][1].text.clone()
        };
        assert_eq!(cell(&doc), "keep");

        // `value` is not a field — the 400 must name it.
        let bad = r#"{"op":"set_cell","row":1,"col":1,"value":"oops"}"#;
        match route(&Method::Post, &path, "", bad) {
            Err((status, msg)) => {
                assert_eq!(status, 400);
                assert!(msg.contains("value"), "the error did not name the field: {msg}");
            }
            Ok(req) => {
                let (_, resp) = process(&mut doc, req);
                assert_eq!(resp.status, 400, "an unknown field was accepted: {}", resp.body);
            }
        }
        assert_eq!(cell(&doc), "keep", "the cell was written anyway");

        // Right field names, one missing: still refused, and named.
        let req = route(&Method::Post, &path, "", r#"{"op":"set_cell","row":1,"col":1}"#).unwrap();
        let (changed, resp) = process(&mut doc, req);
        assert_eq!(resp.status, 400, "a set_cell with no text was accepted");
        assert!(resp.body.contains("text"), "{}", resp.body);
        assert!(!changed);
        assert_eq!(cell(&doc), "keep", "the cell was blanked");

        // The same for the destructive ones: no `at` used to mean row 0.
        let req = route(&Method::Post, &path, "", r#"{"op":"remove_row"}"#).unwrap();
        let (_, resp) = process(&mut doc, req);
        assert_eq!(resp.status, 400, "remove_row with no index deleted a row");
        assert!(resp.body.contains("at"), "{}", resp.body);

        // A bad op in a batch stops the whole batch before anything is applied.
        let batch = r#"[{"op":"set_cell","row":0,"col":0,"text":"first"},
                        {"op":"set_cell","row":0,"col":1}]"#;
        let req = route(&Method::Post, &path, "", batch).unwrap();
        let (changed, resp) = process(&mut doc, req);
        assert_eq!(resp.status, 400);
        assert!(resp.body.contains("nothing was applied"), "{}", resp.body);
        assert!(!changed);
        let CardKind::Table { table } = &doc.card(n, c).unwrap().kind else { panic!() };
        assert_eq!(table.rows[0][0].text, "", "the first op of a rejected batch landed");
    }

    /// A single op object must keep working exactly as before — every existing
    /// caller sends one.
    #[test]
    fn a_single_table_op_still_works() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "B".into());
        let c = doc
            .add_card(n, emath::pos2(0.0, 0.0), CardKind::Table { table: crate::model::TableData::empty(3, 3) })
            .unwrap();
        let req = route(&Method::Post, &format!("/api/nodes/{n}/cards/{c}/table"), "",
                        r#"{"op":"set_cell","row":0,"col":0,"text":"solo"}"#).unwrap();
        let (changed, resp) = process(&mut doc, req);
        assert!(changed && resp.status == 200);
        let CardKind::Table { table } = &doc.card(n, c).unwrap().kind else { panic!() };
        assert_eq!(table.rows[0][0].text, "solo");
    }

    /// A batch that fails partway must say **which** op failed and how many were
    /// applied. Reporting a bare failure would leave the caller unable to tell
    /// what state the table is in.
    #[test]
    fn a_failing_op_names_itself_and_what_already_applied() {
        let mut doc = Document::default();
        let n = doc.add_node(None, "B".into());
        let c = doc
            .add_card(n, emath::pos2(0.0, 0.0), CardKind::Table { table: crate::model::TableData::empty(3, 3) })
            .unwrap();
        let body = r#"[{"op":"set_cell","row":0,"col":0,"text":"kept"},
                       {"op":"set_cell","row":99,"col":99,"text":"out of range"}]"#;
        let req = route(&Method::Post, &format!("/api/nodes/{n}/cards/{c}/table"), "", body).unwrap();
        let (changed, resp) = process(&mut doc, req);
        assert_eq!(resp.status, 400);
        assert!(changed, "the first op did apply, so the document is dirty");
        assert!(resp.body.contains("2/2"), "names the failing op: {}", resp.body);
        assert!(resp.body.contains("1 earlier op"), "says what landed: {}", resp.body);
        let CardKind::Table { table } = &doc.card(n, c).unwrap().kind else { panic!() };
        assert_eq!(table.rows[0][0].text, "kept");
    }


    /// The trap this feature is most likely to reintroduce: a `source` refresh
    /// runs every few seconds, so if it rebuilt the table it would re-flatten
    /// the user's columns continuously.
    #[test]
    fn filling_a_table_keeps_widths_header_and_rules() {
        use crate::model::{CellRule, TableData};
        let mut t = TableData::empty(2, 3);
        t.col_widths = vec![300.0, 80.0, 150.0];
        t.header = true;
        t.rules = vec![CellRule {
            col: Some(1), when: "gt".into(), value: "100".into(),
            bg: Some([1, 2, 3]), fg: None,
        }];
        t.fill_values(vec![
            vec!["Service".into(), "Latency".into(), "Status".into()],
            vec!["db".into(), "1240".into(), "SLOW".into()],
        ]);
        assert_eq!(t.col_widths, vec![300.0, 80.0, 150.0], "widths must survive a refresh");
        assert!(t.header);
        assert_eq!(t.rules.len(), 1);
        assert_eq!(t.rows[1][1].bg, Some([1, 2, 3]), "rules re-applied to new data");
        assert_eq!(t.rows[0][1].bg, None, "the header is a label, not a value");
    }

    /// A cell that stops matching must lose its colour, or a value that stops
    /// being an error keeps a red background forever.
    #[test]
    fn rules_clear_colour_when_a_value_stops_matching() {
        use crate::model::{CellRule, TableData};
        let mut t = TableData::empty(1, 1);
        t.header = false;
        t.rules = vec![CellRule {
            col: None, when: "gt".into(), value: "100".into(),
            bg: Some([9, 9, 9]), fg: None,
        }];
        t.fill_values(vec![vec!["500".into()]]);
        assert_eq!(t.rows[0][0].bg, Some([9, 9, 9]));
        t.fill_values(vec![vec!["5".into()]]);
        assert_eq!(t.rows[0][0].bg, None, "stale colour must not persist");
    }

    #[test]
    fn rule_comparisons_handle_decorated_numbers_and_text() {
        use crate::model::CellRule;
        let r = |when: &str, v: &str| CellRule {
            col: None, when: when.into(), value: v.into(), bg: None, fg: None,
        };
        assert!(r("gt", "1000").matches("1,240"));
        assert!(r("gt", "1000").matches("$1,240.50"));
        assert!(r("lt", "0").matches("(3)"), "(3) is -3");
        assert!(!r("gt", "100").matches("N/A"), "non-numeric has no place on a scale");
        assert!(r("eq", "1200").matches("1,200"), "numeric equality ignores formatting");
        assert!(r("eq", "fail").matches("FAIL"), "text equality is case-insensitive");
        assert!(r("contains", "grad").matches("DEGRADED"));
        assert!(r("empty", "").matches("   "));
        assert!(r("not_empty", "").matches("x"));
    }

    /// TSV is picked from the extension, not sniffed.
    #[test]
    fn delimiter_comes_from_the_extension() {
        use crate::model::delimited_to_values;
        let csv = delimited_to_values("/tmp/x.csv", "a,b\n1,2").unwrap();
        assert_eq!(csv[1], vec!["1", "2"]);
        let tsv = delimited_to_values("/tmp/x.tsv", "a\tb\n1\t2").unwrap();
        assert_eq!(tsv[1], vec!["1", "2"]);
        // A CSV parser on tab data yields one column, which is the failure mode
        // the extension check exists to avoid.
        assert_eq!(delimited_to_values("/tmp/x.csv", "a\tb").unwrap()[0].len(), 1);
        assert!(delimited_to_values("/tmp/x.csv", "").is_err(), "an empty file is an error");
    }

    /// A numeric threshold written as a JSON number must work — it is what
    /// anyone writing one types.
    #[test]
    fn a_rule_threshold_accepts_a_number_or_a_string() {
        let n: crate::model::CellRule =
            serde_json::from_str(r#"{"when":"gt","value":1000}"#).unwrap();
        assert_eq!(n.value, "1000");
        let s: crate::model::CellRule =
            serde_json::from_str(r#"{"when":"eq","value":"FAIL"}"#).unwrap();
        assert_eq!(s.value, "FAIL");
    }

    /// A misspelled or invented field must be a 400 that **names** it, not a
    /// 200 that quietly does nothing.
    ///
    /// This cost real time three separate ways before it was fixed: `{"x":…,
    /// "y":…}` on a card PATCH (the field is `pos`) reported success for five
    /// cards and moved none of them. A write that reports success without
    /// writing is indistinguishable from one that worked, which is the worst
    /// failure an API can have.
    #[test]
    fn an_unknown_field_is_rejected_and_named() {
        let err = match route(&Method::Patch, "/api/nodes/1/cards/2", "", r#"{"x": 10, "y": 20}"#) {
            Err(e) => e,
            Ok(_) => panic!("an unknown field was accepted"),
        };
        assert_eq!(err.0, 400);
        assert!(err.1.contains('x'), "the error names the offending field: {}", err.1);
        assert!(err.1.contains("pos"), "and lists what was expected: {}", err.1);

        // The same on create, and on a node.
        let code = |body: &str, path: &str| match route(&Method::Post, path, "", body) {
            Err((c, _)) => c,
            Ok(_) => panic!("an unknown field was accepted: {body}"),
        };
        assert_eq!(code(r#"{"titel":"typo"}"#, "/api/nodes/1/cards"), 400);
        assert_eq!(code(r#"{"title":"ok","colour":"red"}"#, "/api/nodes"), 400);
    }

    /// The flip side, and the reason this is worth a test rather than a glance:
    /// every field the docs promise must still be accepted. These are exactly
    /// the payloads the shipped clients send.
    #[test]
    fn every_documented_field_still_parses() {
        assert!(route(
            &Method::Post,
            "/api/nodes/1/cards",
            "",
            r#"{"kind":"text","title":"t","body":"b","color":"amber","fit":true,
                "pos":[10,20],"size":[300,200],"font_scale":1.2,"lang":"rust",
                "header":true,"rows":[["a"]],"items":[{"done":false,"text":"i"}],
                "image_base64":"","inline_images":[],"source":""}"#
        )
        .is_ok());
        assert!(route(
            &Method::Patch,
            "/api/nodes/1/cards/2",
            "",
            r#"{"title":"t","body":"b","pos":[1,2],"size":[3,4],"fit":true,"kind":"code"}"#
        )
        .is_ok());
        assert!(route(&Method::Post, "/api/nodes", "", r#"{"title":"n","parent":3}"#).is_ok());
        assert!(route(
            &Method::Post,
            "/api/nodes/1/cards/2/move",
            "",
            r#"{"node":4,"pos":[5,6]}"#
        )
        .is_ok());
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
        // The group routes mirror the card ones exactly, and the more specific
        // paths must not be shadowed by the bare-id lookup below them.
        // The literal arms must not be shadowed by the {cid} arm at the same length.
        assert!(matches!(
            route(&Method::Post, "/api/nodes/5/cards/move", "",
                  "{\"cards\":[1,2],\"node\":7}").unwrap(),
            ApiRequest::MoveCards { node: 5, to: 7, .. }
        ));
        assert!(matches!(
            route(&Method::Post, "/api/nodes/5/cards/property", "",
                  "{\"cards\":[1],\"key\":\"status\",\"value\":\"done\"}").unwrap(),
            ApiRequest::SetCardsProperty { node: 5, .. }
        ));
        // Still strict, like everything else since v0.86.0.
        assert!(route(&Method::Post, "/api/nodes/5/cards/move", "",
                      "{\"cards\":[1],\"nodes\":7}").is_err());
        assert!(matches!(
            route(&Method::Get, "/api/groups/9", "", "").unwrap(),
            ApiRequest::LocateGroup(9)
        ));
        assert!(matches!(
            route(&Method::Get, "/api/groups/9/backlinks", "", "").unwrap(),
            ApiRequest::GroupBacklinks(9)
        ));
        assert!(matches!(
            route(&Method::Get, "/api/groups/9/link", "", "").unwrap(),
            ApiRequest::GroupLink(9)
        ));
        assert!(matches!(
            route(&Method::Post, "/api/nodes/5/groups/9/move", "", "{\"node\":7}").unwrap(),
            ApiRequest::MoveGroup { node: 5, group: 9, to: 7, pos: None }
        ));
        // Strict input, like every other route: a misspelt field is a 400 that
        // names it, not a silent default.
        assert!(route(&Method::Post, "/api/nodes/5/groups/9/move", "", "{\"nodes\":7}").is_err());
        assert!(matches!(
            route(&Method::Get, "/open/group/9", "", "").unwrap(),
            ApiRequest::Open { kind: OpenKind::Group, id: 9, .. }
        ));
        // The by-id lookup is its own top-level route: no node in the path,
        // because not knowing the node is the entire reason to call it.
        assert!(matches!(
            route(&Method::Get, "/api/cards/9", "", "").unwrap(),
            ApiRequest::LocateCard(9)
        ));
        // Claims: the default is every claim, and `expired=true` narrows it to
        // the ones a reader must not trust.
        assert!(matches!(
            route(&Method::Get, "/api/claims", "", "").unwrap(),
            ApiRequest::Claims { expired_only: false, project: None }
        ));
        assert!(matches!(
            route(&Method::Get, "/api/claims", "expired=true&project=7", "").unwrap(),
            ApiRequest::Claims { expired_only: true, project: Some(7) }
        ));
        // Daily notes: the action, and the setting behind it, both reachable.
        assert!(matches!(
            route(&Method::Post, "/api/daily", "", "").unwrap(),
            ApiRequest::DailyNote { date: None }
        ));
        assert!(matches!(
            route(&Method::Post, "/api/daily", "", r#"{"date":"2026-08-12"}"#).unwrap(),
            ApiRequest::DailyNote { date: Some(d) } if d == "2026-08-12"
        ));
        assert!(matches!(
            route(&Method::Get, "/api/daily", "", "").unwrap(),
            ApiRequest::DailyConfig
        ));
        assert!(matches!(
            route(&Method::Post, "/api/daily/root", "", r#"{"node":5}"#).unwrap(),
            ApiRequest::SetDailyRoot(Some(5))
        ));
        assert!(matches!(
            route(&Method::Delete, "/api/daily/root", "", "").unwrap(),
            ApiRequest::SetDailyRoot(None)
        ));
        // A typo'd field is a 400, like everywhere else since v0.86.0.
        assert!(route(&Method::Post, "/api/daily", "", r#"{"day":"2026-08-12"}"#).is_err());
        // Clearing a property needs the key, and says so rather than 404ing.
        assert!(matches!(
            route(&Method::Delete, "/api/nodes/3/cards/4/property", "key=due", "").unwrap(),
            ApiRequest::ClearCardProperty { node: 3, card: 4, key } if key == "due"
        ));
        assert!(route(&Method::Delete, "/api/nodes/3/cards/4/property", "", "").is_err());
        assert!(route(&Method::Get, "/api/cards/nine", "", "").is_err());
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
            let (dirty, resp) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, ops: vec![op] });
            assert!(dirty, "op {body}");
            assert_eq!(resp.status, 200, "op {body}");
        }
        let CardKind::Table { table } = &doc.nodes[&nid].cards[0].kind else { panic!() };
        assert_eq!(table.rows[0][0].bg, Some([0xef, 0x44, 0x44]));
        assert_eq!(table.rows.len(), 3);
        assert!(!table.header);
        // An unknown op is a 400.
        let op: TableOpInput = serde_json::from_str(r#"{"op":"bogus"}"#).unwrap();
        let (_d, resp) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, ops: vec![op] });
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

    /// A claim's bucket, and the two rules that make it different from a task's:
    /// everything in the past is one bucket, and a date nobody can parse counts
    /// as stale rather than fresh.
    #[test]
    fn claim_bucket_treats_the_past_and_the_unreadable_as_stale() {
        let today = 100;
        assert_eq!(claim_bucket(Some(99), today), "expired");
        assert_eq!(claim_bucket(Some(1), today), "expired", "long past is still one bucket");
        assert_eq!(claim_bucket(Some(100), today), "today");
        assert_eq!(claim_bucket(Some(107), today), "soon");
        assert_eq!(claim_bucket(Some(108), today), "ok");
        // The one that matters: `verify:: soon` parses to no date at all, and a
        // claim with an unreadable expiry has, in effect, no expiry.
        assert_eq!(claim_bucket(None, today), "unparsed");
    }

    /// The endpoint an agent is meant to call before believing a workspace:
    /// stale claims first, `expired=true` narrowing to only those.
    #[test]
    fn claims_endpoint_reports_stale_first() {
        use crate::model::{CardKind, Document};
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "Workspace".into()).into();
        let mk = |doc: &mut Document, title: &str, body: &str| {
            let cid = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
            let c = doc.card_mut(nid, cid).unwrap();
            c.title = title.into();
            c.body = body.into();
            cid
        };
        // Yesterday, in the same units the endpoint compares against.
        let today = today_days();
        let past = crate::api::date_from_today(-1, 0);
        let future = crate::api::date_from_today(30, 0);
        let stale = mk(&mut doc, "Both instances run 0.109.0", &format!("verify:: {past}\ncheck:: GET /api/instance"));
        let fresh = mk(&mut doc, "The keystore is backed up", &format!("verify:: {future}"));
        // A card with no `verify::` is not a claim and must not appear —
        // otherwise every card in the document becomes something to re-check.
        mk(&mut doc, "ordinary note", "no properties here");

        let (dirty, resp) = process(&mut doc, ApiRequest::Claims { expired_only: false, project: None });
        assert!(!dirty, "a read must never mark the document dirty");
        let v: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(v["count"], 2, "only cards carrying verify:: are claims");
        assert_eq!(v["stale"], 1);
        assert_eq!(v["today_days"], today);
        // Worst first, so a caller that reads only the head sees the problem.
        assert_eq!(v["claims"][0]["card"], stale);
        assert_eq!(v["claims"][0]["bucket"], "expired");
        assert_eq!(v["claims"][0]["check"], "GET /api/instance");
        assert_eq!(v["claims"][1]["card"], fresh);
        assert_eq!(v["claims"][1]["bucket"], "ok");
        // `check::` is optional and absent means null, not a missing field.
        assert!(v["claims"][1]["check"].is_null());

        let (_d, resp) = process(&mut doc, ApiRequest::Claims { expired_only: true, project: None });
        let v: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["claims"][0]["card"], stale);
    }

    /// A whole-document query names no basket, so a confined token is refused —
    /// the same rule as `/api/tasks`, and for the same reason.
    #[test]
    fn claims_is_not_reachable_from_a_confined_token() {
        let req = ApiRequest::Claims { expired_only: false, project: None };
        assert!(target_node(&req).is_none());
        assert!(!is_scope_neutral(&req));
    }

    /// The direction that was missing: an id, on its own, back to a card. An
    /// agent that reads "card 1391" in a note had no way to resolve it without
    /// walking every basket, and neither did the operator.
    /// The natural client pattern is GET, edit the array, PATCH it back. Since
    /// `GET` returns item ids, `PATCH` has to accept them — rejecting them turned
    /// the obvious round-trip into a 400. And honouring them means identity
    /// follows the *line*, not its position.
    #[test]
    fn a_checklist_round_trips_and_ids_survive_reordering() {
        use crate::model::{CardKind, ChecklistItem};
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Checklist {
            items: vec![ChecklistItem::new("a"), ChecklistItem::new("b"), ChecklistItem::new("c")],
        }).unwrap();
        doc.ensure_item_ids();
        let ids: Vec<u64> = match &doc.card(n, c).unwrap().kind {
            CardKind::Checklist { items } => items.iter().map(|i| i.id).collect(),
            _ => unreachable!(),
        };

        // GET → the ids are in the payload.
        let (_d, got) = process(&mut doc, ApiRequest::GetCard { node: n, card: c });
        let v: Value = serde_json::from_str(&got.body).unwrap();
        assert_eq!(v["items"][0]["id"], ids[0]);

        // PATCH the very same array back, reordered — this must not 400, and
        // every line must keep the id it had.
        let body = serde_json::json!({"items":[
            {"id": ids[2], "done": true,  "text": "c"},
            {"id": ids[0], "done": false, "text": "a edited"},
            {"done": false, "text": "brand new"}
        ]});
        let req = route(&Method::Patch, &format!("/api/nodes/{n}/cards/{c}"), "", &body.to_string())
            .expect("PATCH with ids must parse, not 400");
        let (_d, resp) = process(&mut doc, req);
        assert_eq!(resp.status, 200);

        let after: Vec<(u64, bool, String)> = match &doc.card(n, c).unwrap().kind {
            CardKind::Checklist { items } =>
                items.iter().map(|i| (i.id, i.done, i.text.clone())).collect(),
            _ => unreachable!(),
        };
        assert_eq!(after[0].0, ids[2], "the moved line keeps its identity");
        assert_eq!(after[1].0, ids[0]);
        assert!(after[0].1, "and its done state came along");
        assert_eq!(after[1].2, "a edited");
        assert_eq!(after[2].0, 0, "a line with no id is new; ensure_item_ids assigns it");
        // No id is handed to two lines — the failure this rule exists to stop.
        let live: Vec<u64> = after.iter().map(|(i, _, _)| *i).filter(|i| *i != 0).collect();
        let mut uniq = live.clone(); uniq.sort_unstable(); uniq.dedup();
        assert_eq!(live.len(), uniq.len(), "duplicate item id after a round-trip");
        // The deleted middle line's id is not reused by anything here.
        assert!(!after.iter().any(|(i, _, _)| *i == ids[1]));
    }

    /// A journal node named for a day that does not exist is worse than an
    /// error, so an impossible date is refused rather than rounded.
    /// The whole point of a span: work in flight reads as *now*, not as a future
    /// date that hides it until it is already late.
    #[test]
    fn a_started_task_reads_as_today_until_it_is_overdue() {
        use crate::model::TaskItem;
        let mk = |start: Option<i64>, due: Option<i64>, done: bool| TaskItem {
            node: 1, node_title: String::new(), node_path: String::new(),
            root: 1, root_title: String::new(), card: 1, item: None,
            title: String::new(),
            due: String::new(), due_days: due,
            start: start.map(|_| "s".into()), start_days: start,
            done,
        };
        let today = 100;

        // A plain deadline is unchanged — every task written before spans existed.
        assert_eq!(task_bucket_spanning(&mk(None, Some(105), false), today), "week");
        assert_eq!(task_bucket_spanning(&mk(None, Some(100), false), today), "today");
        assert_eq!(task_bucket_spanning(&mk(None, Some(99), false), today), "overdue");

        // Started, still running: it is happening NOW, not "later".
        assert_eq!(task_bucket_spanning(&mk(Some(98), Some(105), false), today), "today");
        // Not started yet: it keeps its future date.
        assert_eq!(task_bucket_spanning(&mk(Some(102), Some(105), false), today), "week");
        // Past its end: overdue wins, so a span can't hide a missed deadline.
        assert_eq!(task_bucket_spanning(&mk(Some(90), Some(95), false), today), "overdue");
        // Open-ended work that has started is also now. (Defensive: `tasks()`
        // requires a `due::` to emit anything, so a start-only task cannot reach
        // the agenda today — if that ever changes, this arm is already correct.)
        assert_eq!(task_bucket_spanning(&mk(Some(90), None, false), today), "today");

        // live_on is the day-shaped question: is this on me on that day?
        let span = mk(Some(98), Some(105), false);
        assert!(!span.live_on(97), "before it starts");
        assert!(span.live_on(98) && span.live_on(101) && span.live_on(105));
        assert!(span.live_on(200), "an unfinished deadline keeps applying");
        let finished = mk(Some(98), Some(105), true);
        assert!(!finished.live_on(101), "done is never live");
    }

    /// The Agenda's shortcuts write real dates, and a month is a calendar month:
    /// 31 January + 1 month is the end of February, not the 3rd of March.
    #[test]
    fn relative_dates_are_calendar_dates() {
        let today = date_from_today(0, 0);
        assert_eq!(today, chrono::Local::now().format("%Y-%m-%d").to_string());
        let tomorrow = date_from_today(1, 0);
        assert_eq!(
            crate::model::parse_ymd(&tomorrow).unwrap(),
            crate::model::parse_ymd(&today).unwrap() + 1
        );
        let week = date_from_today(7, 0);
        assert_eq!(
            crate::model::parse_ymd(&week).unwrap(),
            crate::model::parse_ymd(&today).unwrap() + 7
        );
        // A month lands on a real day, whatever today is.
        assert!(crate::model::parse_ymd(&date_from_today(0, 1)).is_some());
    }

    #[test]
    fn a_daily_date_must_be_a_real_calendar_day() {
        let d = daily_date_from("2026-08-11").unwrap();
        assert_eq!((d.year, d.month, d.day), (2026, 8, 11));
        assert_eq!(d.weekday, "Tuesday");
        assert_eq!(d.month_name, "August");
        assert_eq!(d.title(), "Tuesday 8/11/2026");

        assert!(daily_date_from("2026-02-30").is_none());
        assert!(daily_date_from("2026-13-01").is_none());
        assert!(daily_date_from("11/08/2026").is_none());
        assert!(daily_date_from("").is_none());
        // A leap day that exists, and one that does not.
        assert!(daily_date_from("2028-02-29").is_some());
        assert!(daily_date_from("2026-02-29").is_none());
    }

    #[test]
    fn locate_card_finds_a_card_from_its_id_alone() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "A".into());
        let b = doc.add_node(Some(a), "B".into());
        let cid = doc.add_card(b, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(b, cid).unwrap().title = "the one".into();

        let (dirty, resp) = process(&mut doc, ApiRequest::LocateCard(cid));
        assert!(!dirty, "a read must never mark the document dirty");
        assert_eq!(resp.status, 200);
        let got: Value = serde_json::from_str(&resp.body).unwrap();
        // The basket comes back too: an id is enough to *find* a card, but every
        // route that edits one still needs the node.
        assert_eq!(got["node"], b);
        assert_eq!(got["node_title"], "B");
        assert_eq!(got["node_path"], doc.node_path(b));
        assert_eq!(got["card"]["title"], "the one");
        assert_eq!(got["card"]["id"], cid);

        // The card body is the same shape GET /nodes/{n}/cards/{c} returns, so a
        // caller can hand it to the same code.
        let (_d, direct) = process(&mut doc, ApiRequest::GetCard { node: b, card: cid });
        let direct: Value = serde_json::from_str(&direct.body).unwrap();
        assert_eq!(got["card"], direct);

        let (_d, resp) = process(&mut doc, ApiRequest::LocateCard(9999));
        assert_eq!(resp.status, 404);
    }

    /// Node ids and card ids are separate spaces, so the same number can name
    /// both. The route must answer about the card and never fall back to a node
    /// that happens to share the number.
    #[test]
    fn locate_card_is_not_confused_by_a_node_with_the_same_number() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "a node".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, cid).unwrap().title = "a card".into();

        let (_d, resp) = process(&mut doc, ApiRequest::LocateCard(n));
        // Either the number is also a card id — then it must resolve to *that
        // card* — or it is not, and this is a 404. What it must never be is the
        // node dressed up as a card.
        if resp.status == 200 {
            let got: Value = serde_json::from_str(&resp.body).unwrap();
            assert_eq!(got["card"]["id"], n, "resolved the node id as if it were a card");
        } else {
            assert_eq!(resp.status, 404);
        }
    }

    #[test]
    fn autofit_cols_via_api_widens_the_wordy_column() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc
            .add_card(nid, egui::pos2(0.0, 0.0), CardKind::Table {
                table: crate::model::TableData::from_values(vec![
                    vec!["Host".into(), "Result".into()],
                    vec!["HOST-1".into(), "a verdict far too long for 110 pixels".into()],
                ]),
            })
            .unwrap();

        let op: TableOpInput = serde_json::from_str(r#"{"op":"autofit_cols"}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, ops: vec![op] });
        assert!(dirty);
        assert_eq!(resp.status, 200);
        let CardKind::Table { table } = &doc.nodes[&nid].cards[0].kind else { panic!() };
        assert!(table.col_width(1) > crate::model::TABLE_DEFAULT_COL_W);
        let fitted = table.col_width(1);

        // An out-of-range `col` is a 400, like the other indexed ops.
        let op: TableOpInput = serde_json::from_str(r#"{"op":"autofit_cols","col":7}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, ops: vec![op] });
        assert!(!dirty);
        assert_eq!(resp.status, 400);

        // Idempotent: fitting again on unchanged content changes nothing.
        let op: TableOpInput = serde_json::from_str(r#"{"op":"autofit_cols"}"#).unwrap();
        let (_d, _r) = process(&mut doc, ApiRequest::TableOp { node: nid, card: cid, ops: vec![op] });
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

    /// The point of the whole feature: a group survives a basket change with
    /// its id, so a `[[#g…]]` link written before the move still lands.
    /// The operation that motivated this: archiving a finished basket was 55
    /// single-card calls, one per card.
    #[test]
    fn desktop_routes_parse_and_are_answered_by_the_app_loop() {
        assert!(matches!(
            route(&Method::Post, "/api/cards/9/desktop", "", "{\"pos\":[10,20]}").unwrap(),
            ApiRequest::SetCardDesktop { card: 9, on: true, pos: Some([10.0, 20.0]) }
        ));
        // An empty body is allowed: "put it somewhere sensible".
        assert!(matches!(
            route(&Method::Post, "/api/cards/9/desktop", "", "").unwrap(),
            ApiRequest::SetCardDesktop { card: 9, on: true, pos: None }
        ));
        assert!(matches!(
            route(&Method::Delete, "/api/cards/9/desktop", "", "").unwrap(),
            ApiRequest::SetCardDesktop { card: 9, on: false, .. }
        ));
        assert!(matches!(
            route(&Method::Get, "/api/desktop", "", "").unwrap(),
            ApiRequest::ListCardDesktop
        ));
        assert!(route(&Method::Post, "/api/cards/9/desktop", "", "{\"posn\":[1,2]}").is_err());
        // Placement is app config, so `process` must refuse to answer these —
        // they are handled where the window state lives.
        let mut doc = Document::empty();
        for req in [
            ApiRequest::SetCardDesktop { card: 9, pos: None, on: true },
            ApiRequest::ListCardDesktop,
        ] {
            let (dirty, resp) = process(&mut doc, req);
            assert!(!dirty);
            assert_eq!(resp.status, 500, "must fall through to the app loop");
        }
    }

    #[test]
    fn cards_can_be_created_in_a_batch_on_the_same_endpoint() {
        // An array batches; an object stays the single create it always was.
        assert!(matches!(
            route(&Method::Post, "/api/nodes/5/cards", "", r#"[{"title":"a"},{"title":"b"}]"#).unwrap(),
            ApiRequest::AddCards { node: 5, .. }
        ));
        assert!(matches!(
            route(&Method::Post, "/api/nodes/5/cards", "", r#"{"title":"a"}"#).unwrap(),
            ApiRequest::AddCard { node: 5, .. }
        ));
        // Still strict inside every element of the array.
        assert!(route(&Method::Post, "/api/nodes/5/cards", "", r#"[{"titel":"a"}]"#).is_err());

        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let req = route(&Method::Post, &format!("/api/nodes/{n}/cards"), "",
            r#"[{"title":"one","pos":[10,10]},{"kind":"checklist","title":"two",
                 "items":[{"text":"x","done":false}]},{"title":"three","fit":true}]"#).unwrap();
        // fit_batch names WHICH entries asked to be fitted, by index.
        assert_eq!(fit_batch(&req), vec![2]);
        let (dirty, resp) = process(&mut doc, req);
        assert!(dirty);
        assert_eq!(resp.status, 201);
        let got: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(got["created"], 3);
        let ids: Vec<u64> = got["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        assert_eq!(ids.len(), 3);
        assert_eq!(doc.card(n, ids[0]).unwrap().title, "one");
        assert_eq!(doc.card(n, ids[0]).unwrap().pos, egui::pos2(10.0, 10.0));
        assert!(matches!(doc.card(n, ids[1]).unwrap().kind, CardKind::Checklist { .. }));

        // An unknown node refuses the whole batch rather than half-creating it.
        let (_d, resp) = process(&mut doc, ApiRequest::AddCards { node: 999, inputs: vec![] });
        assert_eq!(resp.status, 404);
        let req = route(&Method::Post, &format!("/api/nodes/{n}/cards"), "", "[]").unwrap();
        let (_d, resp) = process(&mut doc, req);
        assert_eq!(resp.status, 400, "an empty array is a mistake, not a no-op");
    }

    /// A batch carries one `source` per card, and the mirror policy is the one
    /// place an API request can reach the filesystem — so it has to see all of
    /// them, not just the first.
    #[test]
    fn every_source_in_a_batch_is_offered_to_the_mirror_check() {
        let req = route(&Method::Post, "/api/nodes/5/cards", "",
            r#"[{"title":"a","source":"/tmp/one.md"},
                {"title":"b"},
                {"title":"c","source":"/tmp/two.md"},
                {"title":"d","source":"   "}]"#).unwrap();
        assert_eq!(source_requests(&req), vec!["/tmp/one.md", "/tmp/two.md"]);
        // The single-card path is unchanged.
        let one = route(&Method::Post, "/api/nodes/5/cards", "",
            r#"{"title":"a","source":"/tmp/one.md"}"#).unwrap();
        assert_eq!(source_requests(&one), vec!["/tmp/one.md"]);
        // And a scoped token is checked against the basket it names.
        assert_eq!(target_node(&ApiRequest::AddCards { node: 7, inputs: vec![] }), Some(7));
    }

    #[test]
    fn desktop_mode_routes_take_a_whole_basket() {
        assert!(matches!(
            route(&Method::Post, "/api/nodes/63/desktop", "", "").unwrap(),
            ApiRequest::SetNodeDesktop { node: 63, on: true }
        ));
        assert!(matches!(
            route(&Method::Delete, "/api/nodes/63/desktop", "", "").unwrap(),
            ApiRequest::SetNodeDesktop { node: 63, on: false }
        ));
        // Placement is app config, so `process` must not answer these.
        let mut doc = Document::empty();
        let (dirty, resp) =
            process(&mut doc, ApiRequest::SetNodeDesktop { node: 63, on: true });
        assert!(!dirty);
        assert_eq!(resp.status, 500, "answered in the app loop, where the windows live");
        // A subtree-scoped token is checked against the basket it names.
        assert_eq!(
            target_node(&ApiRequest::SetNodeDesktop { node: 63, on: true }),
            Some(63)
        );
    }

    #[test]
    fn cards_move_in_a_batch_and_the_whole_list_is_validated_first() {
        let mut doc = Document::empty();
        let from = doc.add_node(None, "from".into());
        let to = doc.add_node(None, "to".into());
        let a = doc.add_card(from, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(from, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let c = doc.add_card(from, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(from, a).unwrap().size = egui::vec2(300.0, 100.0);
        doc.card_mut(from, b).unwrap().size = egui::vec2(300.0, 50.0);

        // One bad id refuses the WHOLE batch — a partial move leaves the caller
        // unable to tell how far it got.
        let (dirty, resp) = process(&mut doc, ApiRequest::MoveCards {
            node: from, cards: vec![a, 9999], to, pos: None, gap: 20.0,
        });
        assert!(!dirty, "a refused batch must not move anything");
        assert_eq!(resp.status, 404);
        assert!(resp.body.contains("9999"), "the error names the offending id");
        assert_eq!(doc.locate_card(a), Some(from), "nothing moved");

        // pos stacks the cards down by height + gap, so an archive is readable.
        let (dirty, resp) = process(&mut doc, ApiRequest::MoveCards {
            node: from, cards: vec![a, b, c], to, pos: Some([40.0, 40.0]), gap: 20.0,
        });
        assert!(dirty);
        assert_eq!(resp.status, 200);
        let got: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(got["moved"], 3);
        assert_eq!(doc.card(to, a).unwrap().pos, egui::pos2(40.0, 40.0));
        assert_eq!(doc.card(to, b).unwrap().pos, egui::pos2(40.0, 160.0));
        assert_eq!(doc.card(to, c).unwrap().pos, egui::pos2(40.0, 230.0));
        // Ids survive, so links written to an archived card still resolve.
        assert_eq!(doc.locate_card(a), Some(to));
        assert!(doc.nodes[&from].cards.is_empty());

        // Refusals that name the reason.
        let (_d, resp) = process(&mut doc, ApiRequest::MoveCards {
            node: to, cards: vec![a], to, pos: None, gap: 20.0 });
        assert_eq!(resp.status, 400, "same basket");
        let (_d, resp) = process(&mut doc, ApiRequest::MoveCards {
            node: to, cards: vec![], to: from, pos: None, gap: 20.0 });
        assert_eq!(resp.status, 400, "empty list");
        let (_d, resp) = process(&mut doc, ApiRequest::MoveCards {
            node: to, cards: vec![a], to: 999, pos: None, gap: 20.0 });
        assert_eq!(resp.status, 404, "unknown destination");
    }

    #[test]
    fn one_property_can_be_set_on_many_cards() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();

        let (_d, resp) = process(&mut doc, ApiRequest::SetCardsProperty {
            node: n, cards: vec![a, 9999], key: "status".into(), value: "done".into() });
        assert_eq!(resp.status, 404, "validated up front, like the batch move");

        let (dirty, resp) = process(&mut doc, ApiRequest::SetCardsProperty {
            node: n, cards: vec![a, b], key: "status".into(), value: "done".into() });
        assert!(dirty);
        let got: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(got["updated"], 2);
        assert_eq!(got["key"], "status");
        for cid in [a, b] {
            let props = doc.card(n, cid).unwrap().properties();
            assert!(props.iter().any(|(k, v)| k == "status" && v == "done"),
                    "card {cid} carries status:: done");
        }
    }

    #[test]
    fn a_group_moves_between_baskets_over_the_api() {
        let mut doc = Document::empty();
        let from = doc.add_node(None, "from".into());
        let to = doc.add_node(None, "to".into());
        let a = doc.add_card(from, egui::pos2(40.0, 60.0), CardKind::Text).unwrap();
        let b = doc.add_card(from, egui::pos2(40.0, 200.0), CardKind::Text).unwrap();
        let g = doc.group_cards(from, &[a, b], "design".into()).unwrap();

        let (dirty, resp) = process(
            &mut doc,
            ApiRequest::MoveGroup { node: from, group: g, to, pos: Some([10.0, 10.0]) },
        );
        assert!(dirty);
        assert_eq!(resp.status, 200);
        let got: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(got["moved"], 2);
        assert_eq!(got["node"], to);
        assert_eq!(doc.locate_group(g), Some(to));

        // Reads by id alone now answer from the new basket.
        let (_d, resp) = process(&mut doc, ApiRequest::LocateGroup(g));
        assert_eq!(resp.status, 200);
        let got: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(got["node"], to);
        assert_eq!(got["group"]["title"], "design");
        assert_eq!(got["group"]["cards"].as_array().unwrap().len(), 2);

        // Every failure mode is named rather than silently doing nothing.
        let (_d, resp) =
            process(&mut doc, ApiRequest::MoveGroup { node: to, group: g, to, pos: None });
        assert_eq!(resp.status, 400, "moving a group into its own basket");
        let (_d, resp) =
            process(&mut doc, ApiRequest::MoveGroup { node: to, group: g, to: 999, pos: None });
        assert_eq!(resp.status, 404, "unknown destination");
        let (_d, resp) =
            process(&mut doc, ApiRequest::MoveGroup { node: to, group: 999, to: from, pos: None });
        assert_eq!(resp.status, 404, "unknown group");
        let (_d, resp) = process(&mut doc, ApiRequest::LocateGroup(999));
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn group_backlinks_over_the_api() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let g = doc.group_cards(n, &[a, b], "pair".into()).unwrap();
        let p = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, p).unwrap().body = format!("see [[#g{g}]]");

        let (dirty, resp) = process(&mut doc, ApiRequest::GroupBacklinks(g));
        assert!(!dirty, "a backlinks read must not dirty the document");
        assert_eq!(resp.status, 200);
        let got: Value = serde_json::from_str(&resp.body).unwrap();
        let hits = got["hits"].as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["card"], p);
        let (_d, resp) = process(&mut doc, ApiRequest::GroupBacklinks(999));
        assert_eq!(resp.status, 404);
    }

    /// A subtree-scoped token is checked against the node a request *names*,
    /// which for a move is where the thing is coming **from**. Without a check
    /// at the far end, a confined token could carry its own card, group or
    /// basket out into the rest of the document.
    #[test]
    fn a_move_declares_its_destination_for_the_scope_check() {
        // Cross-basket card move.
        let mv: MoveCardInput = serde_json::from_str(r#"{"node":42}"#).unwrap();
        assert_eq!(
            move_destination(&ApiRequest::MoveCard { node: 1, card: 2, mv }),
            Some(MoveDest::Basket(42))
        );
        // Reordering inside one basket relocates nothing.
        let mv: MoveCardInput = serde_json::from_str(r#"{"to":"front"}"#).unwrap();
        assert_eq!(move_destination(&ApiRequest::MoveCard { node: 1, card: 2, mv }), None);

        assert_eq!(
            move_destination(&ApiRequest::MoveGroup { node: 1, group: 2, to: 42, pos: None }),
            Some(MoveDest::Basket(42))
        );

        // A node reparent, in all three forms it can be written.
        let mv: MoveNodeInput = serde_json::from_str(r#"{"parent":42,"to":"bottom"}"#).unwrap();
        assert_eq!(
            move_destination(&ApiRequest::MoveNode { id: 1, mv }),
            Some(MoveDest::Parent(Some(42)))
        );
        let mv: MoveNodeInput = serde_json::from_str(r#"{"parent":null,"to":"bottom"}"#).unwrap();
        assert_eq!(
            move_destination(&ApiRequest::MoveNode { id: 1, mv }),
            Some(MoveDest::Parent(None)),
            "the top level is a destination too, and it is outside every subtree"
        );
        let mv: MoveNodeInput = serde_json::from_str(r#"{"before":42}"#).unwrap();
        assert_eq!(
            move_destination(&ApiRequest::MoveNode { id: 1, mv }),
            Some(MoveDest::Sibling(42)),
            "before/after adopt the sibling's parent, which only the tree knows"
        );
        // A pure reorder among current siblings names no new home.
        let mv: MoveNodeInput = serde_json::from_str(r#"{"index":3}"#).unwrap();
        assert_eq!(move_destination(&ApiRequest::MoveNode { id: 1, mv }), None);

        // Anything that cannot relocate is None, so the check waves it through.
        assert_eq!(move_destination(&ApiRequest::LocateCard(1)), None);
        // The group move still declares its source, so the near end is checked
        // by the ordinary path.
        assert_eq!(
            target_node(&ApiRequest::MoveGroup { node: 7, group: 2, to: 42, pos: None }),
            Some(7)
        );
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

    /// The settings routes exist and take the shape the app loop expects: an
    /// object, and not an empty one — an empty patch is not a change.
    #[test]
    fn settings_routes_take_an_object() {
        assert!(matches!(
            route(&Method::Get, "/api/settings", "", "").unwrap(),
            ApiRequest::SettingsGet
        ));
        assert!(matches!(
            route(&Method::Post, "/api/settings", "", r#"{"theme":"Light"}"#).unwrap(),
            ApiRequest::SettingsSet(_)
        ));
        assert!(route(&Method::Post, "/api/settings", "", "[]").is_err());
        assert!(
            route(&Method::Post, "/api/settings", "", "{}").is_err(),
            "an empty patch changes nothing and should say so"
        );
    }

    // --- the batch surface: edit, delete, clear a property --------------------

    /// Three cards, one call, and every one of them takes the change.
    #[test]
    fn a_batch_edit_reaches_every_card_it_names() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let ids: Vec<u64> = (0..3)
            .map(|i| {
                doc.add_card(nid, egui::pos2(0.0, i as f32 * 100.0), CardKind::Text).unwrap()
            })
            .collect();
        let req = route(
            &Method::Patch,
            &format!("/api/nodes/{nid}/cards"),
            "",
            &format!(r#"{{"cards":{ids:?},"color":"red","size":[300,200],"font_scale":1.5}}"#),
        )
        .unwrap();
        let (dirty, resp) = process(&mut doc, req);
        assert!(dirty);
        assert_eq!(resp.status, 200);
        let v: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(v["updated"], 3);
        for c in &doc.nodes[&nid].cards {
            assert_eq!(c.color, [0xef, 0x44, 0x44], "the colour name resolved");
            assert_eq!(c.size, egui::vec2(300.0, 200.0));
            assert_eq!(c.font_scale, 1.5);
        }
    }

    /// A batch sets presentation, never content. Refused **by name**, with the
    /// route that does accept it — a list of cards all saying the same thing is
    /// the copied-card failure, and one typo'd id list would be unrecoverable.
    #[test]
    fn a_batch_edit_refuses_content_fields_by_name() {
        for field in ["title", "body", "items", "rows", "kind", "lang", "source"] {
            let body = format!(r#"{{"cards":[1,2],"{field}":"x"}}"#);
            let err = match route(&Method::Patch, "/api/nodes/5/cards", "", &body) {
                Err(e) => e,
                Ok(_) => panic!("{field} should be refused for a list of cards"),
            };
            assert_eq!(err.0, 400);
            assert!(err.1.contains(field), "the 400 has to name the field: {}", err.1);
            assert!(
                err.1.contains("cards/{cid}"),
                "and say where it IS accepted: {}",
                err.1
            );
        }
        // A misspelt presentation field is still the v0.86.0 400 naming it,
        // because the remainder is parsed by the single-card struct itself.
        // (`unwrap_err` will not compile here: ApiRequest has no Debug.)
        let err = match route(&Method::Patch, "/api/nodes/5/cards", "", r#"{"cards":[1],"colr":"red"}"#) {
            Err(e) => e,
            Ok(_) => panic!("a misspelt field must not be accepted"),
        };
        assert_eq!(err.0, 400);
        assert!(err.1.contains("colr"), "{}", err.1);
        // And `cards` is not optional: a patch with no list would be a silent
        // no-op that reads as success.
        assert!(route(&Method::Patch, "/api/nodes/5/cards", "", r#"{"color":"red"}"#).is_err());
    }

    /// One bad id refuses the whole batch, and nothing is half-applied — the same
    /// rule the batch move and table ops follow.
    #[test]
    fn one_bad_id_refuses_the_whole_batch() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let a = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let before = doc.nodes[&nid].cards[0].color;

        let patch: UpdateCardInput = serde_json::from_str(r#"{"color":"red"}"#).unwrap();
        let (dirty, resp) = process(
            &mut doc,
            ApiRequest::UpdateCards { node: nid, cards: vec![a, 9999], patch },
        );
        assert!(!dirty);
        assert_eq!(resp.status, 404);
        assert!(resp.body.contains("9999"), "the 404 names the id: {}", resp.body);
        assert_eq!(doc.nodes[&nid].cards[0].color, before, "nothing was applied");

        // Same for the delete, where a partial run cannot be walked back at all.
        let (dirty, resp) =
            process(&mut doc, ApiRequest::DeleteCards { node: nid, cards: vec![a, 9999] });
        assert!(!dirty);
        assert_eq!(resp.status, 404);
        assert_eq!(doc.nodes[&nid].cards.len(), 1, "the good card is still here");
    }

    /// Deleting a list is one call, and the cards are actually gone.
    #[test]
    fn a_batch_delete_removes_exactly_the_list() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let ids: Vec<u64> = (0..4)
            .map(|i| {
                doc.add_card(nid, egui::pos2(0.0, i as f32 * 100.0), CardKind::Text).unwrap()
            })
            .collect();
        let req = route(
            &Method::Delete,
            &format!("/api/nodes/{nid}/cards"),
            "",
            &format!(r#"{{"cards":[{},{}]}}"#, ids[1], ids[2]),
        )
        .unwrap();
        let (dirty, resp) = process(&mut doc, req);
        assert!(dirty);
        assert_eq!(resp.status, 200);
        let v: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(v["deleted"], 2);
        let left: Vec<u64> = doc.nodes[&nid].cards.iter().map(|c| c.id).collect();
        assert_eq!(left, vec![ids[0], ids[3]]);
        // An empty list is a mistake, not a no-op.
        assert_eq!(
            process(&mut doc, ApiRequest::DeleteCards { node: nid, cards: vec![] }).1.status,
            400
        );
    }

    /// Setting a property on a list had no counterpart: clearing one was a call
    /// per card. And a card that never carried it is not an error — the count
    /// says how many lines actually went.
    #[test]
    fn clearing_a_property_off_a_list_is_one_call() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let ids: Vec<u64> = (0..3)
            .map(|i| {
                doc.add_card(nid, egui::pos2(0.0, i as f32 * 100.0), CardKind::Text).unwrap()
            })
            .collect();
        // Two of the three get a due date.
        let (_d, _r) = process(
            &mut doc,
            ApiRequest::SetCardsProperty {
                node: nid,
                cards: vec![ids[0], ids[1]],
                key: "due".into(),
                value: "2026-08-20".into(),
            },
        );
        let req = route(
            &Method::Delete,
            &format!("/api/nodes/{nid}/cards/property"),
            "",
            &format!(r#"{{"cards":{ids:?},"key":"DUE"}}"#),
        )
        .unwrap();
        let (dirty, resp) = process(&mut doc, req);
        assert!(dirty);
        let v: Value = serde_json::from_str(&resp.body).unwrap();
        assert_eq!(v["cleared"], 2, "only the two that had one");
        assert_eq!(v["key"], "due", "the key is reported lowercased, as it is stored");
        for c in &doc.nodes[&nid].cards {
            assert!(doc.card_property(nid, c.id, "due").is_none());
        }
    }

    /// `property` sits exactly where a card id would on the DELETE path. The
    /// literal arm has to win, or clearing a property from a list would try to
    /// parse "property" as an id.
    #[test]
    fn the_batch_property_path_is_not_shadowed_by_a_card_id() {
        assert!(matches!(
            route(
                &Method::Delete,
                "/api/nodes/5/cards/property",
                "",
                r#"{"cards":[1],"key":"due"}"#
            )
            .unwrap(),
            ApiRequest::ClearCardsProperty { node: 5, .. }
        ));
        // And the single-card DELETE still resolves an id on the same shape.
        assert!(matches!(
            route(&Method::Delete, "/api/nodes/5/cards/7", "", "").unwrap(),
            ApiRequest::DeleteCard { node: 5, card: 7 }
        ));
    }

    /// Every batch route names its basket, so a confined token is checked against
    /// it. A batch that reported `None` here would be refused outright — safe,
    /// but it would also mean an agent could not use the batch surface in its own
    /// basket, which is where it is most useful.
    #[test]
    fn the_batch_routes_name_their_basket_for_the_scope_check() {
        let patch: UpdateCardInput = serde_json::from_str("{}").unwrap();
        assert_eq!(
            target_node(&ApiRequest::UpdateCards { node: 7, cards: vec![1], patch }),
            Some(7)
        );
        assert_eq!(target_node(&ApiRequest::DeleteCards { node: 7, cards: vec![1] }), Some(7));
        assert_eq!(
            target_node(&ApiRequest::ClearCardsProperty {
                node: 7,
                cards: vec![1],
                key: "due".into()
            }),
            Some(7)
        );
    }

    /// `fit` on a batch has to reach the app loop's precise re-measure, or the
    /// same flag would mean two different sizes depending on how many cards you
    /// sent.
    #[test]
    fn fit_on_a_batch_edit_is_offered_to_the_precise_refit() {
        let patch: UpdateCardInput = serde_json::from_str(r#"{"fit":true}"#).unwrap();
        assert_eq!(
            fit_updates(&ApiRequest::UpdateCards { node: 3, cards: vec![8, 9], patch }),
            Some((3, vec![8, 9]))
        );
        let patch: UpdateCardInput = serde_json::from_str(r#"{"color":"red"}"#).unwrap();
        assert_eq!(
            fit_updates(&ApiRequest::UpdateCards { node: 3, cards: vec![8], patch }),
            None
        );
    }

    /// A 409 that had already changed something was worse than either outcome:
    /// the caller could not tell what stuck. The mirror check now runs before any
    /// field is applied.
    #[test]
    fn a_refused_edit_to_a_mirrored_card_changes_nothing() {
        let mut doc = Document::empty();
        let nid = doc.add_node(None, "n".into());
        let cid = doc.add_card(nid, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(nid, cid).unwrap().source = Some("/tmp/whatever.md".into());
        doc.card_mut(nid, cid).unwrap().title = "before".into();
        let patch: UpdateCardInput =
            serde_json::from_str(r#"{"title":"after","body":"mine now"}"#).unwrap();
        let (dirty, resp) = process(&mut doc, ApiRequest::UpdateCard { node: nid, card: cid, patch });
        assert!(!dirty);
        assert_eq!(resp.status, 409);
        assert_eq!(
            doc.card(nid, cid).unwrap().title,
            "before",
            "the title used to be renamed before the body was refused"
        );
    }

    // --- a card id is a complete address for writes too ----------------------

    /// Each card-addressed route parses to the operation it stands for.
    #[test]
    fn card_addressed_writes_parse() {
        let cases: Vec<(Method, &str, &str, &str)> = vec![
            (Method::Patch, "/api/cards/9", "", r#"{"color":"red"}"#),
            (Method::Delete, "/api/cards/9", "", ""),
            (Method::Post, "/api/cards/9/property", "", r#"{"key":"status","value":"done"}"#),
            (Method::Delete, "/api/cards/9/property", "key=due", ""),
            (Method::Post, "/api/cards/9/move", "", r#"{"node":3}"#),
            (Method::Post, "/api/cards/9/items/4/done", "", r#"{"done":true}"#),
            (Method::Post, "/api/cards/9/items/4/property", "", r#"{"key":"due","value":"2026-09-01"}"#),
            (Method::Delete, "/api/cards/9/items/4/property", "key=due", ""),
        ];
        for (m, path, q, body) in cases {
            match route(&m, path, q, body) {
                Ok(ApiRequest::ByCard { card: 9, .. }) => {}
                Ok(_) => panic!("{path} did not parse as a card-addressed write"),
                Err(e) => panic!("{path} failed to parse: {e:?}"),
            }
        }
        // Clearing still needs to say WHICH property, as it does on the node form.
        assert!(route(&Method::Delete, "/api/cards/9/property", "", "").is_err());
        // And the reads that already took a bare id are untouched.
        assert!(matches!(
            route(&Method::Get, "/api/cards/9", "", "").unwrap(),
            ApiRequest::LocateCard(9)
        ));
        assert!(matches!(
            route(&Method::Get, "/api/cards/9/link", "", "").unwrap(),
            ApiRequest::CardLink(9)
        ));
    }

    /// The card-addressed form is a **rewrite**, so it must land on exactly the
    /// request the node-addressed route produces — that is what makes "one
    /// implementation, not two" true rather than claimed.
    #[test]
    fn resolving_a_card_op_lands_on_its_node_addressed_twin() {
        let patch: UpdateCardInput = serde_json::from_str(r#"{"color":"red"}"#).unwrap();
        assert!(matches!(
            resolve_by_card(3, 9, CardOp::Patch(patch)),
            ApiRequest::UpdateCard { node: 3, card: 9, .. }
        ));
        assert!(matches!(
            resolve_by_card(3, 9, CardOp::Delete),
            ApiRequest::DeleteCard { node: 3, card: 9 }
        ));
        let set = resolve_by_card(3, 9, CardOp::SetProperty {
            key: "status".into(),
            value: "done".into(),
        });
        match &set {
            ApiRequest::SetCardProperty { node: 3, card: 9, key, value } => {
                assert_eq!((key.as_str(), value.as_str()), ("status", "done"));
            }
            _ => panic!("wrong twin"),
        }
        assert!(matches!(
            resolve_by_card(3, 9, CardOp::ClearProperty { key: "due".into() }),
            ApiRequest::ClearCardProperty { node: 3, card: 9, .. }
        ));
        let mv: MoveCardInput = serde_json::from_str(r#"{"node":5}"#).unwrap();
        assert!(matches!(
            resolve_by_card(3, 9, CardOp::Move(mv)),
            ApiRequest::MoveCard { node: 3, card: 9, .. }
        ));
        assert!(matches!(
            resolve_by_card(3, 9, CardOp::ItemDone { item: 4, done: true }),
            ApiRequest::SetItemDone { node: 3, card: 9, item: 4, done: true }
        ));
        assert!(matches!(
            resolve_by_card(3, 9, CardOp::SetItemProperty {
                item: 4,
                key: "due".into(),
                value: "x".into()
            }),
            ApiRequest::SetItemProperty { node: 3, card: 9, item: 4, .. }
        ));
        assert!(matches!(
            resolve_by_card(3, 9, CardOp::ClearItemProperty { item: 4, key: "due".into() }),
            ApiRequest::ClearItemProperty { node: 3, card: 9, item: 4, .. }
        ));
    }

    /// **The security property, pinned from both ends.**
    ///
    /// Before the rewrite a card-addressed write names no basket, so `target_node`
    /// is `None` — which the scope check reads as *refuse*. After the rewrite it
    /// names the basket the card actually lives in, so a confined token is checked
    /// against that. Fail either half and a token confined to one basket could
    /// edit any card in the document by id.
    #[test]
    fn a_card_addressed_write_is_never_scope_neutral() {
        let req = ApiRequest::ByCard { card: 9, op: CardOp::Delete };
        assert_eq!(target_node(&req), None, "unresolved: no basket, so refused");
        assert!(!is_scope_neutral(&req), "and never waved through as an orientation read");
        // Resolved, it is checked against the basket it landed in.
        assert_eq!(target_node(&resolve_by_card(42, 9, CardOp::Delete)), Some(42));
    }

    /// And if the rewrite is ever skipped, `process` refuses rather than applying
    /// a write whose scope nobody checked.
    #[test]
    fn an_unresolved_card_write_is_refused_by_process() {
        let mut doc = Document::empty();
        let (dirty, resp) =
            process(&mut doc, ApiRequest::ByCard { card: 9, op: CardOp::Delete });
        assert!(!dirty);
        assert_eq!(resp.status, 500, "must be resolved by the app loop, never here");
    }

}
