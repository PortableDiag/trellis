//! An append-only log of what changed, so a client can react to *what* moved
//! rather than merely *that* something did.
//!
//! Before this, the only change signal was `doc_revision`, one `AtomicU64` for
//! the whole document. `/api/wait` returned a number, which told a client the
//! document was different and nothing else — so the only correct response was to
//! re-fetch everything and diff it locally. That is fine for a small reader and
//! untenable for sync, collaboration, or a plugin that wants to fire when a card
//! gets `status:: done`.
//!
//! ## What this is, and what it deliberately is not
//!
//! Each entry records **that an entity changed and which parts of it**, not the
//! before and after. A client re-fetches the named entity to see the new value.
//! That choice is what keeps it cheap and, more importantly, makes it impossible
//! to desync: there is no patch to apply, so a malformed one cannot corrupt a
//! client's state. It is not enough for character-level collaborative merging —
//! that wants a CRDT, which is a different and much larger decision.
//!
//! ## Session-only, on purpose
//!
//! The log lives in memory and is **not** written into the document. v0.74.0
//! spent a release shrinking the file; a rotating log inside it would re-grow
//! exactly what was fixed. The one dependent that genuinely needs persistence is
//! "sort baskets by latest change", and that wants an 8-byte timestamp per
//! entity — a smaller, more direct change than persisting the whole log.
//!
//! Because it is session-only, **`epoch` matters**: it is fresh on every launch,
//! and a client that sees a different epoch than it saw last time must resync
//! rather than trust its stored sequence number. Without it, a client holding
//! seq 5000 across a restart would ask for changes since 5000, be told there are
//! none, and silently miss everything.

use serde::Serialize;
use std::collections::VecDeque;

/// A change's sequence number. Deliberately the same value as the document
/// revision that `/api/wait` returns, so a client can long-poll for a revision
/// and then ask this log what that revision *was* with no translation step.
pub type Seq = u64;

/// How many entries to retain. A client that falls further behind than this gets
/// `truncated: true` and has to resync. Generous enough that an offline phone
/// reconnecting after a busy session still gets an incremental update.
pub const DEFAULT_CAP: usize = 5000;

/// What kind of thing changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Entity {
    Node,
    Card,
    Group,
    /// The document as a whole — opened, imported into, restored from history.
    Document,
}

/// What happened to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Created,
    Updated,
    Deleted,
    /// Position, order, or reparenting — a change of *where*, not of content.
    /// Separate from `Updated` because a sync client can apply it cheaply and a
    /// plugin watching content usually wants to ignore it.
    Moved,
}

/// Who made the change. A collaborator needs to know whether an edit was theirs
/// before deciding to redraw over what they are typing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Actor {
    /// A person using this app.
    Ui,
    /// An agent over the HTTP API.
    Api,
}

/// One recorded change.
///
/// Fields are omitted from JSON when empty, so a simple entry stays small — this
/// is a log that can see thousands of entries in a session, and padding every
/// one with nulls is exactly the slop that makes a log tiresome to consume.
#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub seq: Seq,
    /// Unix seconds. The document itself carries no timestamps at all, so this
    /// is the only "when" in Trellis.
    pub ts: u64,
    pub actor: Actor,
    pub entity: Entity,
    pub op: Op,
    /// The node/card/group id. Card ids are document-global, so this identifies
    /// a card on its own; `node` says which basket to fetch it from.
    pub id: u64,
    /// The owning basket, for a card or group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<u64>,
    /// The entity's title as it was at the time. Present so a log is readable —
    /// and useful after a delete, when the title can no longer be looked up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Which fields changed, e.g. `["body", "color"]`. Lets a plugin watching
    /// card text ignore a recolour without fetching anything.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    /// The one piece of *content* an entry carries: the `key:: value` property
    /// that changed. This is what makes "fire when a card gets `status:: done`"
    /// answerable without re-fetching the card. Only the key and value, never
    /// the body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property: Option<(String, String)>,
}

impl Change {
    /// Start an entry. `seq` and `ts` are filled in by [`ChangeLog::push`], so a
    /// call site cannot get the ordering wrong.
    pub fn new(actor: Actor, entity: Entity, op: Op, id: u64) -> Self {
        Self {
            seq: 0,
            ts: 0,
            actor,
            entity,
            op,
            id,
            node: None,
            title: None,
            fields: Vec::new(),
            property: None,
        }
    }

    pub fn in_node(mut self, node: u64) -> Self {
        self.node = Some(node);
        self
    }

    pub fn titled(mut self, title: impl Into<String>) -> Self {
        let t: String = title.into();
        if !t.is_empty() {
            // Keep the log small: a title is a label, not a payload.
            self.title = Some(if t.chars().count() > 120 {
                t.chars().take(120).collect::<String>() + "…"
            } else {
                t
            });
        }
        self
    }

    pub fn field(mut self, f: &str) -> Self {
        self.fields.push(f.to_string());
        self
    }

    pub fn property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.property = Some((key.into(), value.into()));
        self
    }

    /// Whether two entries say the same thing about the same entity, ignoring
    /// when they happened. Used to collapse a drag or a burst of typing — see
    /// [`ChangeLog::push`]. Title is excluded deliberately: renaming while typing
    /// shouldn't split the run, and the newer entry's title wins anyway.
    fn same_shape(&self, other: &Change) -> bool {
        self.actor == other.actor
            && self.entity == other.entity
            && self.op == other.op
            && self.id == other.id
            && self.fields == other.fields
            && self.property == other.property
    }
}

/// The rotating in-memory log.
pub struct ChangeLog {
    entries: VecDeque<Change>,
    cap: usize,
    epoch: u64,
}

impl ChangeLog {
    pub fn new(cap: usize, epoch: u64) -> Self {
        Self { entries: VecDeque::new(), cap: cap.max(1), epoch }
    }

    /// Identifies this run of the app. A client must resync when it changes.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Append a change, stamping it with the revision it belongs to and the
    /// current time.
    ///
    /// **Consecutive identical-shape changes collapse into one.** Dragging a card
    /// emits a move every frame and typing emits a body edit per keystroke; left
    /// alone, one drag wrote eight entries and a paragraph would write hundreds,
    /// burying everything worth reading.
    ///
    /// Collapsing is lossless *here specifically* because an entry carries no
    /// before/after content — it says "card 3's body changed", and the client
    /// re-fetches the card either way. Ten merged entries and one entry lead to
    /// exactly the same fetch. That is not true of an operation log, which is one
    /// more reason this isn't one.
    ///
    /// Only the immediately preceding entry merges, and only when actor, entity,
    /// op, id, fields and property all match — so an agent's edit never absorbs a
    /// person's, and a move never absorbs a rename.
    pub fn push(&mut self, seq: Seq, mut change: Change) {
        change.seq = seq;
        change.ts = now_secs();
        if self.entries.back().is_some_and(|last| last.same_shape(&change)) {
            // Re-stamped with the newer seq so a client that already read the
            // old one still sees this and re-fetches once more.
            self.entries.pop_back();
        }
        self.entries.push_back(change);
        while self.entries.len() > self.cap {
            self.entries.pop_front();
        }
    }

    /// Everything after `since`, oldest first, capped at `limit`.
    ///
    /// The `bool` is **truncated**: entries the caller needed have already been
    /// rotated away, so an incremental update is impossible and it must re-read
    /// what it cares about. Reported rather than papered over — silently
    /// returning a partial list is how clients end up quietly out of date.
    pub fn since(&self, since: Seq, limit: usize) -> (Vec<Change>, bool) {
        let truncated = self
            .entries
            .front()
            .is_some_and(|first| since + 1 < first.seq);
        let out: Vec<Change> = self
            .entries
            .iter()
            .filter(|c| c.seq > since)
            .take(limit)
            .cloned()
            .collect();
        (out, truncated)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The oldest sequence still retained, so a client can see how far back it
    /// could catch up from before deciding to resync.
    pub fn oldest(&self) -> Option<Seq> {
        self.entries.front().map(|c| c.seq)
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A fresh epoch for this run, from the OS CSPRNG so two runs can't collide.
pub fn new_epoch() -> u64 {
    let mut b = [0u8; 8];
    if getrandom::fill(&mut b).is_ok() {
        u64::from_le_bytes(b)
    } else {
        now_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> ChangeLog {
        ChangeLog::new(4, 42)
    }

    fn card(id: u64) -> Change {
        Change::new(Actor::Api, Entity::Card, Op::Updated, id)
    }

    #[test]
    fn since_returns_only_what_follows_and_keeps_order() {
        let mut l = log();
        for s in 1..=3 {
            l.push(s, card(100 + s));
        }
        let (got, truncated) = l.since(1, 100);
        assert!(!truncated);
        assert_eq!(got.iter().map(|c| c.seq).collect::<Vec<_>>(), vec![2, 3]);
        assert_eq!(got[0].id, 102, "oldest first");

        let (none, _) = l.since(3, 100);
        assert!(none.is_empty(), "caller is already current");
    }

    /// Rotating away entries a caller still needed must be *reported*. A silent
    /// partial answer is how a client ends up permanently missing an edit.
    #[test]
    fn falling_behind_the_cap_is_reported_as_truncated() {
        let mut l = log(); // cap 4
        for s in 1..=6 {
            l.push(s, card(s));
        }
        assert_eq!(l.len(), 4, "rotated to the cap");
        let (got, truncated) = l.since(1, 100);
        assert!(truncated, "seqs 2 was dropped, so 1 cannot be caught up from");
        assert_eq!(got.first().map(|c| c.seq), Some(3));

        // Exactly caught up to the entry before the oldest is *not* truncated.
        let (_, t2) = l.since(2, 100);
        assert!(!t2, "the next entry it needs (3) is still here");
    }

    #[test]
    fn an_empty_log_is_never_truncated() {
        let l = log();
        let (got, truncated) = l.since(0, 100);
        assert!(got.is_empty() && !truncated);
        assert_eq!(l.oldest(), None);
    }

    #[test]
    fn limit_caps_the_batch_without_claiming_truncation() {
        let mut l = ChangeLog::new(100, 1);
        for s in 1..=10 {
            l.push(s, card(s));
        }
        let (got, truncated) = l.since(0, 3);
        assert_eq!(got.len(), 3);
        assert!(!truncated, "a limited page is not a gap — the rest is still here");
        assert_eq!(got.last().unwrap().seq, 3, "resume from here");
    }

    /// One drag emits a move per frame. Without collapsing, a single card drag
    /// wrote eight entries — measured, not hypothetical.
    #[test]
    fn a_run_of_identical_changes_collapses_to_one() {
        let mut l = ChangeLog::new(100, 1);
        for s in 1..=8 {
            l.push(s, Change::new(Actor::Ui, Entity::Card, Op::Moved, 3).field("pos"));
        }
        let (got, _) = l.since(0, 100);
        assert_eq!(got.len(), 1, "a drag is one change");
        assert_eq!(got[0].seq, 8, "carrying the latest revision");
    }

    #[test]
    fn collapsing_never_merges_across_actor_entity_or_field() {
        let mut l = ChangeLog::new(100, 1);
        l.push(1, Change::new(Actor::Ui, Entity::Card, Op::Updated, 3).field("body"));
        // Different actor: an agent's edit must not vanish into a person's.
        l.push(2, Change::new(Actor::Api, Entity::Card, Op::Updated, 3).field("body"));
        // Different field.
        l.push(3, Change::new(Actor::Api, Entity::Card, Op::Updated, 3).field("color"));
        // Different card.
        l.push(4, Change::new(Actor::Api, Entity::Card, Op::Updated, 4).field("color"));
        // Different property value — each status transition is its own event.
        l.push(5, Change::new(Actor::Api, Entity::Card, Op::Updated, 4).property("status", "doing"));
        l.push(6, Change::new(Actor::Api, Entity::Card, Op::Updated, 4).property("status", "done"));
        assert_eq!(l.since(0, 100).0.len(), 6, "none of these are the same change");
    }

    /// A collapsed run must not swallow an unrelated change that happened in the
    /// middle of it — only the *immediately preceding* entry merges.
    #[test]
    fn an_interleaved_change_breaks_the_run() {
        let mut l = ChangeLog::new(100, 1);
        let mv = || Change::new(Actor::Ui, Entity::Card, Op::Moved, 3).field("pos");
        l.push(1, mv());
        l.push(2, mv());
        l.push(3, Change::new(Actor::Ui, Entity::Card, Op::Deleted, 9));
        l.push(4, mv());
        let (got, _) = l.since(0, 100);
        assert_eq!(got.len(), 3);
        assert_eq!(got[1].op, Op::Deleted, "the delete survives in order");
    }

    #[test]
    fn push_stamps_sequence_and_time() {
        let mut l = log();
        l.push(7, card(1));
        let c = &l.since(0, 1).0[0];
        assert_eq!(c.seq, 7);
        assert!(c.ts > 1_700_000_000, "a real unix timestamp");
    }

    /// The whole point of the detail: a plugin should be able to decide whether
    /// it cares without fetching the card.
    #[test]
    fn entries_carry_enough_to_act_on_without_a_fetch() {
        let c = Change::new(Actor::Api, Entity::Card, Op::Updated, 4821)
            .in_node(62)
            .titled("Deploy checklist")
            .field("body")
            .property("status", "done");
        let v: serde_json::Value = serde_json::to_value(&c).unwrap();
        assert_eq!(v["entity"], "card");
        assert_eq!(v["op"], "updated");
        assert_eq!(v["actor"], "api");
        assert_eq!(v["node"], 62);
        assert_eq!(v["fields"][0], "body");
        assert_eq!(v["property"][0], "status");
        assert_eq!(v["property"][1], "done");
    }

    /// Empty extras must not be serialised — thousands of entries padded with
    /// nulls is the slop this log is meant to avoid.
    #[test]
    fn empty_detail_is_omitted_from_json() {
        let c = Change::new(Actor::Ui, Entity::Node, Op::Created, 9);
        let v: serde_json::Value = serde_json::to_value(&c).unwrap();
        assert!(v.get("node").is_none());
        assert!(v.get("title").is_none());
        assert!(v.get("fields").is_none());
        assert!(v.get("property").is_none());
    }

    #[test]
    fn a_long_title_is_clipped_not_stored_whole() {
        let c = Change::new(Actor::Ui, Entity::Card, Op::Updated, 1).titled("x".repeat(500));
        assert!(c.title.as_ref().unwrap().chars().count() <= 121);
        // An empty title is absent rather than an empty string.
        assert!(Change::new(Actor::Ui, Entity::Card, Op::Updated, 1).titled("").title.is_none());
    }

    #[test]
    fn epochs_differ_between_runs() {
        assert_ne!(new_epoch(), new_epoch(), "a stale client must be able to tell");
    }
}
