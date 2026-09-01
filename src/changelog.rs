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
    /// *Which* agent, when one named itself with `X-Agent` (or held a scoped
    /// token, whose label is used instead).
    ///
    /// `actor` says a change came in over the API; until v0.143.0 that was all it
    /// said, so with several agents sharing the instance key — the normal setup
    /// here, because it is what lets one agent leave a finding in another
    /// project — "which of them did this" was unanswerable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
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
    /// Name the agent behind this change. No-op for `None`, so the UI path and
    /// an unnamed API caller are untouched.
    pub fn by(mut self, agent: Option<String>) -> Self {
        self.agent = agent;
        self
    }

    pub fn new(actor: Actor, entity: Entity, op: Op, id: u64) -> Self {
        Self {
            seq: 0,
            ts: 0,
            actor,
            agent: None,
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
            && self.agent == other.agent
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
    /// The newest `seq` recorded, or 0 when nothing has been.
    ///
    /// A reader that only wants "anything since I last looked" needs this to
    /// mark its place without re-reading the entries it just handled.
    pub fn newest(&self) -> Seq {
        self.entries.back().map(|c| c.seq).unwrap_or(0)
    }

    pub fn oldest(&self) -> Option<Seq> {
        self.entries.front().map(|c| c.seq)
    }
}

/// One failed API call — a response of 400 or above, as the caller saw it.
///
/// **This exists because nothing else recorded a failure.** `ChangeLog` records
/// what *succeeded*; a 400/403/404/409/500 was answered and forgotten — not
/// even written to stderr, which on the live instances is a terminal nobody is
/// reading anyway. With several agents driving the API all day, the only record
/// of a refused call was the agent that made it, and an agent that mis-reads a
/// response (2026-08-28: a card body PATCHed blank) leaves no trace at all.
///
/// Deliberately **not** a `Change`: a failure changed nothing, so it has no
/// place in a log whose contract is "re-fetch what this names". It has its own
/// counter, its own retention and its own file.
#[derive(Clone, Debug, Serialize)]
pub struct ApiError {
    pub seq: u64,
    /// Unix seconds.
    pub ts: u64,
    pub status: u16,
    pub method: String,
    /// Path plus query string — the query is part of what was asked.
    pub path: String,
    /// `X-Agent`, or a scoped token's label. Absent for an anonymous call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The `error` the caller was sent.
    pub error: String,
    /// The first [`REQUEST_EXCERPT`] characters of the request body, when one
    /// was read. **Never present for a 401**: the body is not read before the
    /// key is checked, so a mistyped credential cannot land in a log file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
}

/// How much of a failed request's body is kept — enough to see *what* was sent
/// (`{"bg": null}`, `{"body": ""}`), not enough to make the log a copy of the
/// document.
pub const REQUEST_EXCERPT: usize = 200;

/// Query parameter names whose VALUE is a credential and must never be logged.
///
/// Matched case-insensitively, and on the whole name only — `key` is redacted,
/// `keyword` is not.
const SECRET_PARAMS: [&str; 8] = [
    "key", "api_key", "apikey", "token", "access_token", "secret", "password", "passphrase",
];

/// Blank out credential values in a recorded path's query string.
///
/// **The 401 protection had a hole.** [`ApiError::request`] is deliberately
/// absent on a 401 so a mistyped key cannot reach the log — but the *path* is
/// recorded whole, query included, and a caller that authenticates the wrong way
/// round (`/api/instance?api_key=…`, which this API does not accept) puts its key
/// in the query. That is exactly the caller most likely to be holding a real key
/// and getting it refused, and the log is a file on disk that outlives the run.
/// Seen live on 2026-08-31, in the log this function now cleans.
///
/// Applied in [`ErrorLog::push`], so it reaches memory and file together and no
/// call site can forget it. The parameter NAME is kept: "somebody tried to
/// authenticate by query string" is the useful half, and it is what tells you to
/// go and tell them the header form.
pub fn redact_query(path: &str) -> String {
    let Some((base, query)) = path.split_once('?') else {
        return path.to_string();
    };
    let cleaned: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, _)) if SECRET_PARAMS.iter().any(|s| name.eq_ignore_ascii_case(s)) => {
                format!("{name}=<redacted>")
            }
            _ => pair.to_string(),
        })
        .collect();
    format!("{base}?{}", cleaned.join("&"))
}

/// Clip a request body to [`REQUEST_EXCERPT`] characters, on a char boundary,
/// marking the cut. Empty in, `None` out — an absent body is not an excerpt.
pub fn request_excerpt(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    let mut out: String = body.chars().take(REQUEST_EXCERPT).collect();
    if body.chars().count() > REQUEST_EXCERPT {
        out.push('…');
    }
    Some(out)
}

/// The on-disk log rotates at this size, once, to `<name>.1`. Two megabytes of
/// failures is thousands of them; older than that is not worth a third file.
const ERROR_FILE_ROTATE_BYTES: u64 = 1 << 20;

/// Where the on-disk copy lives: `<data-dir>/trellis/api-errors.log`, beside
/// `app.ron` — per instance, like every other piece of app state.
pub fn error_log_path(data_dir: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    match data_dir {
        Some(d) => Some(d.join("trellis").join("api-errors.log")),
        None => directories::ProjectDirs::from("dev", "Trellis", "Trellis")
            .map(|p| p.data_dir().join("api-errors.log")),
    }
}

/// The rotating in-memory error log, mirrored to a JSON-lines file.
///
/// In memory it answers *this session* over `GET /api/errors`, with the same
/// `epoch`/`seq`/`truncated` contract as [`ChangeLog`] so a client that already
/// follows the change log needs nothing new. The file answers *last week*: one
/// JSON object per line, appended as each failure happens, so it survives a
/// restart and a crash, and reads with `tail -f` or `jq`.
pub struct ErrorLog {
    entries: VecDeque<ApiError>,
    cap: usize,
    epoch: u64,
    next_seq: u64,
    total: u64,
    file: Option<std::path::PathBuf>,
    /// The first write failure, kept so it can be reported once rather than on
    /// every request — a log that cannot be written must still not stop the API.
    file_error: Option<String>,
}

impl ErrorLog {
    pub fn new(cap: usize, epoch: u64, file: Option<std::path::PathBuf>) -> Self {
        Self {
            entries: VecDeque::new(),
            cap: cap.max(1),
            epoch,
            next_seq: 1,
            total: 0,
            file,
            file_error: None,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Record one failure: stamps `seq` and `ts`, keeps it in memory, and
    /// appends it to the file. Nothing collapses — two identical failures are
    /// two failures, and the count is the point.
    pub fn push(&mut self, mut e: ApiError) {
        // One choke point for both copies: a credential a caller put in the
        // query string never reaches memory OR the file. See `redact_query`.
        e.path = redact_query(&e.path);
        e.seq = self.next_seq;
        self.next_seq += 1;
        e.ts = now_secs();
        self.total += 1;
        self.write_line(&e);
        self.entries.push_back(e);
        while self.entries.len() > self.cap {
            self.entries.pop_front();
        }
    }

    fn write_line(&mut self, e: &ApiError) {
        let Some(path) = self.file.clone() else { return };
        let result = (|| -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) >= ERROR_FILE_ROTATE_BYTES {
                let mut rotated = path.clone().into_os_string();
                rotated.push(".1");
                std::fs::rename(&path, rotated)?;
            }
            let mut line = serde_json::to_value(e).unwrap_or_default();
            // The epoch rides on every line so a reader of the file can tell
            // which run a `seq` belongs to — in memory the endpoint says it once.
            if let Some(obj) = line.as_object_mut() {
                obj.insert("epoch".into(), serde_json::Value::from(self.epoch));
            }
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
            writeln!(f, "{line}")
        })();
        if let Err(err) = result {
            if self.file_error.is_none() {
                self.file_error = Some(format!("{}: {err}", path.display()));
            }
        }
    }

    /// Everything after `since`, oldest first, capped at `limit`; the `bool` is
    /// **truncated** — entries the caller needed have rotated out of memory.
    /// The file still has them.
    pub fn since(&self, since: u64, limit: usize) -> (Vec<ApiError>, bool) {
        let truncated = self.entries.front().is_some_and(|first| since + 1 < first.seq);
        let out: Vec<ApiError> =
            self.entries.iter().filter(|c| c.seq > since).take(limit).cloned().collect();
        (out, truncated)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Failures this run, including ones rotated out of memory.
    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn newest(&self) -> u64 {
        self.entries.back().map(|c| c.seq).unwrap_or(0)
    }

    pub fn oldest(&self) -> Option<u64> {
        self.entries.front().map(|c| c.seq)
    }

    pub fn file(&self) -> Option<&std::path::Path> {
        self.file.as_deref()
    }

    pub fn file_error(&self) -> Option<&str> {
        self.file_error.as_deref()
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

    fn api_err(status: u16, path: &str) -> ApiError {
        ApiError {
            seq: 0,
            ts: 0,
            status,
            method: "PATCH".into(),
            path: path.into(),
            agent: Some("claude".into()),
            error: "no such card".into(),
            request: request_excerpt(r#"{"body":""}"#),
        }
    }

    /// **A failure is counted, kept, and written — and never collapsed.** The
    /// change log merges repeats because a client re-fetches either way; here the
    /// count IS the information, so two identical 404s are two entries.
    #[test]
    fn error_log_keeps_every_failure_and_mirrors_it_to_disk() {
        let dir = std::env::temp_dir().join(format!("trellis-errlog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("trellis").join("api-errors.log");
        let mut l = ErrorLog::new(2, 77, Some(file.clone()));
        l.push(api_err(404, "/api/cards/903"));
        l.push(api_err(404, "/api/cards/903"));
        l.push(api_err(400, "/api/nodes/1/cards?x=1"));
        assert_eq!(l.total(), 3, "the count survives rotation out of memory");
        assert_eq!(l.len(), 2, "only `cap` stay in memory");
        let (got, truncated) = l.since(0, 10);
        assert_eq!(got.len(), 2);
        assert!(truncated, "seq 1 has rotated away and the caller is told");
        assert_eq!(got[0].seq, 2);
        assert!(got[0].ts > 1_700_000_000);
        assert_eq!(l.newest(), 3);
        assert_eq!(l.oldest(), Some(2));
        assert!(l.file_error().is_none(), "{:?}", l.file_error());

        // The file has all three, one JSON object per line, each stamped with
        // the epoch so a `seq` can be placed in a run.
        let text = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<serde_json::Value> =
            text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["seq"], 1);
        assert_eq!(lines[0]["epoch"], 77);
        assert_eq!(lines[0]["status"], 404);
        assert_eq!(lines[0]["agent"], "claude");
        assert_eq!(lines[0]["request"], r#"{"body":""}"#);
        assert_eq!(lines[2]["path"], "/api/nodes/1/cards?x=1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The file rotates once, at a megabyte, and the API never stops.** A
    /// write failure is remembered once and does not refuse the request that
    /// triggered it — the log is a record, not a gate.
    #[test]
    fn error_file_rotates_and_a_bad_path_is_reported_not_fatal() {
        let dir = std::env::temp_dir().join(format!("trellis-errrot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("api-errors.log");
        std::fs::write(&file, vec![b'x'; (ERROR_FILE_ROTATE_BYTES + 1) as usize]).unwrap();
        let mut l = ErrorLog::new(10, 1, Some(file.clone()));
        l.push(api_err(500, "/api/backup"));
        let rotated = dir.join("api-errors.log.1");
        assert!(rotated.exists(), "the full file moved aside");
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1, "a fresh file holds the new line");
        let _ = std::fs::remove_dir_all(&dir);

        // A path that cannot be created — a component of it is a FILE, which
        // no OS will make a directory of. (The first cut used `/proc/…`, which
        // Windows happily creates under the current drive: 0.162.0's Windows
        // CI job failed on exactly that and shipped without its asset.)
        let block = std::env::temp_dir().join(format!("trellis-errblock-{}", std::process::id()));
        std::fs::write(&block, b"not a directory").unwrap();
        let mut bad = ErrorLog::new(10, 1, Some(block.join("sub").join("api-errors.log")));
        bad.push(api_err(400, "/api/x"));
        assert_eq!(bad.total(), 1);
        assert_eq!(bad.len(), 1);
        assert!(bad.file_error().is_some());
        let _ = std::fs::remove_file(&block);

        // No file at all is a valid configuration.
        let mut none = ErrorLog::new(10, 1, None);
        none.push(api_err(400, "/api/x"));
        assert_eq!(none.total(), 1);
    }

    /// **An excerpt is a glimpse, not a copy.** Clipped on a character boundary,
    /// marked, and absent (not empty) when there was no body.
    #[test]
    fn request_excerpt_clips_and_marks() {
        assert_eq!(request_excerpt(""), None);
        assert_eq!(request_excerpt("   "), None);
        assert_eq!(request_excerpt(r#"{"a":1}"#).as_deref(), Some(r#"{"a":1}"#));
        let long = "é".repeat(REQUEST_EXCERPT + 5);
        let got = request_excerpt(&long).unwrap();
        assert_eq!(got.chars().count(), REQUEST_EXCERPT + 1);
        assert!(got.ends_with('…'));
    }
}
