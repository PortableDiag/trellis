# Trellis on the web — proposal

**Status:** proposal / not started. A **separate deployment**, not a replacement
for the desktop app.
**Target:** a Debian VPS, one owner plus a handful of invited collaborators.
**Written:** 2026-08-05, against Trellis v0.75.1.

The desktop app stays the primary way to work. The web version exists for four
things it cannot do:

1. **Backup** — the document lives somewhere other than one machine's disk.
2. **Sync** — edits made anywhere converge.
3. **Sharing** — send someone a read-only link to a basket, not a file.
4. **Reach** — any device with a browser, including ones that will never run a
   native build.

Everything in this document is subordinate to those four. "Full parity" is the
stated goal and this document takes it seriously, but §8 is honest about the
handful of desktop features that cannot cross and what replaces them.

---

## 1. The one finding that shapes everything else

`src/model.rs` is 4,630 lines holding the entire data model, the tag/property
parser, the query surfaces (tags, properties, combined query, tasks, kanban,
backlinks, graph), the table and chart logic, and every export format. The
obvious fear is that it is welded to egui and therefore unusable off the desktop.

**It is not.** Measured:

| coupling | count | what it actually is |
|---|---|---|
| `egui::pos2` / `vec2` | 69 | constructor functions |
| `egui::Pos2` / `Vec2` / `Rect` | 12 | geometry types |
| everything else in egui | **0** | no widgets, no `Context`, no rendering |

And `egui::{Pos2, Vec2, Rect, pos2, vec2}` are **literal re-exports of `emath`**
(`egui-0.29.1/src/lib.rs:447`), a crate with **no mandatory dependencies** that
is *already* a direct dependency in `Cargo.toml`.

So extracting the model into a shared crate is a mechanical `egui::` → `emath::`
substitution, not a rewrite. The other crates `model.rs` reaches for — `printpdf`,
`image`, `ab_glyph`, `calamine`, `rust_xlsxwriter`, `csv`, `pulldown_cmark`,
`html2md` — are all headless and perfectly happy on a server.

**This decides the server language.** See §2.

> **Verify this first.** §12 Phase 0 is nothing but "prove `trellis-core` builds
> without eframe". It should take an hour. If it doesn't, every estimate below
> is wrong and the stack decision should be reopened.

---

## 2. Stack

| layer | choice | why |
|---|---|---|
| Server | **Rust + Axum** | See below. Not a preference — a consequence of §1. |
| Shared logic | **`trellis-core` crate** | `model.rs`, extracted, used by desktop *and* server. |
| Database | **PostgreSQL 15+** | Concurrent writers, real full-text search, JSONB. |
| Blobs | **Filesystem, content-addressed** | Images are most of the bytes; they do not belong in the DB. |
| Client | **TypeScript + React + Vite** | Ecosystem depth for the editor pieces. |
| Basket canvas | **DOM, CSS-transformed** | The crux decision. See §3. |
| Code/text editing | **CodeMirror 6** | Strictly better than the desktop's editor. |
| Markdown | **markdown-it + Shiki** | Matches CommonMark; Shiki gives the same highlighting quality. |
| Reverse proxy / TLS | **Caddy** | Automatic Let's Encrypt in three lines of config. |
| Process management | **systemd** | It is Debian. Nothing else is warranted. |
| Migrations | **sqlx, checked in** | Compile-time-checked queries against a real schema. |

### Why Rust on the server, when the whole world would say TypeScript

Because the alternative is **two implementations of the same semantics**, and
they will drift silently. The document is full of rules that are not obvious and
are already load-bearing:

- `key:: value` requires a **trailing space** after the `::`.
- `#tag` is a tag but `# tag` is an H1 — the space is the whole distinction, and
  getting it wrong inflates every tagged card (this was a real bug, fixed in
  v0.74.2).
- "Today" for `due::` bucketing is the **local** calendar day, not the UTC one
  (v0.71.1 — the agenda jumped a day early every evening west of Greenwich).
- Table `rows` bulk-replace resets column widths; `autofit_cols` sizes columns
  and `fit` sizes the frame, and they are not the same operation (v0.73.0).

Reimplementing that in TypeScript means every one of those bugs gets to happen a
second time, and the two versions disagree about what a document *means*. Sharing
`trellis-core` makes the desktop and the server incapable of disagreeing.

The cost is honest: Rust is slower to write UI-adjacent server code in, and the
front end is TypeScript regardless, so the project is bilingual either way. The
trade is worth it because the *hard* part is the semantics, not the HTTP.

**Rejected:** Node/TypeScript server (duplicates the model), Go (same), Python
(same, plus the export pipeline would need rewriting against different PDF and
XLSX libraries).

---

## 3. The canvas: DOM, not Canvas2D, not WebGL

This is the decision most likely to be argued with, so here is the reasoning in
full.

The desktop canvas is immediate-mode: `canvas.rs` paints every card directly at
its screen rect, with **no transform layer**, which is why every pixel dimension
has to be multiplied by the zoom by hand — and why forgetting to do that shipped
a real bug (table cell rects scaled while the font didn't, v0.59.1).

The web does not have to work that way.

**Each card is an absolutely-positioned `<div>` inside a wrapper carrying
`transform: translate(Xpx, Ypx) scale(Z)`.** Pan and zoom are one CSS property on
one element. Card content scales correctly *by construction* — the class of bug
that produced v0.59.1 cannot occur.

What DOM buys that a canvas would cost:

| capability | DOM | Canvas2D / WebGL |
|---|---|---|
| Text editing, carets, selection | free | reimplement, including IME |
| Markdown rendering | free (markdown-it → HTML) | lay out rich text by hand |
| Syntax highlighting | free (Shiki) | hand-rolled |
| Find-in-page, screen readers, zoom | free | absent |
| Copy/paste, drag-drop, links | free | reimplement |
| Per-card scroll for long cards | free | reimplement |

That last row deserves attention: a desktop text card has **no per-card scroll**,
which is exactly why exceeding the fit height cap silently truncated content
(v0.74.2). On the web, `overflow: auto` on the card body makes the entire class
of problem disappear.

**The usual objection is performance**, and it does not apply here. DOM struggles
past a couple of thousand elements. But a *basket* is not a document — the real
work document is 9,871 cards across 956 baskets, roughly ten cards per basket. A
heavy basket is tens of cards, each a handful of elements. That is nothing.
(Virtualise by viewport only if a pathological basket ever appears.)

**Within a card**, use the right tool per kind:

| card kind | rendering |
|---|---|
| Text | markdown-it → HTML in a div; CodeMirror 6 when editing |
| Code | CodeMirror 6 with the language mode |
| Checklist | real `<input type=checkbox>` + list |
| Table | `<table>`, contenteditable cells |
| Sketch | inline **SVG `<polyline>`** — strokes are already vector points |
| Image | `<img>`, lazy-loaded from the blob endpoint |
| Chart | hand-rolled **SVG** (~300 lines for bar/line/scatter/pie) |

Charts do not need a library. The whole spec is `{kind, label_col, value_cols,
show_table}` and the data is a small grid; a charting dependency would be more
code than the charts.

**One real DOM cost, stated:** a card with a CSS `scale()` transform renders text
by scaling a rasterised layer in some browsers at extreme zoom, which looks
slightly soft. Mitigate by re-laying-out at zoom-stable breakpoints if it ever
becomes objectionable. It has never been a blocker for any of the many
whiteboard apps built this way.

---

## 4. Storage: replacing the single RON file

### The problem

The desktop document is **one gzip-compressed RON file, loaded entirely into
memory and saved atomically**. On the real work document that is 956 nodes /
9,871 cards / ~16 MB. That model is excellent for a single-user local app — it
gives atomicity and portability for free — and it cannot survive contact with
concurrent editors or partial loading. Two browsers editing different baskets
would each write the whole file and the last one would win, silently.

### The schema

Relational, with kind-specific data in JSONB. Node and card ids are
**document-scoped integers, preserved exactly** — every existing agent script,
the Android app, and the whole API surface address cards by those ids, and
lossless round-trip demands it.

```sql
CREATE TABLE documents (
  id           uuid PRIMARY KEY,
  owner_id     uuid NOT NULL REFERENCES users(id),
  name         text NOT NULL,
  rev          bigint NOT NULL DEFAULT 0,   -- mirrors the desktop revision counter
  next_node_id bigint NOT NULL,             -- the document's own id counters:
  next_card_id bigint NOT NULL,             -- losing these makes re-import collide
  next_group_id bigint NOT NULL,
  created_at   timestamptz NOT NULL DEFAULT now(),
  updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE nodes (
  document_id  uuid NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  node_key     bigint NOT NULL,             -- the desktop's NodeId
  title        text NOT NULL,
  parent_key   bigint,                      -- NULL = root
  ord          integer NOT NULL,            -- position among siblings
  color        integer[3],
  bg           integer[3],
  expanded     boolean NOT NULL DEFAULT true,
  updated_at   timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (document_id, node_key)
);
CREATE INDEX ON nodes (document_id, parent_key, ord);

CREATE TABLE cards (
  document_id  uuid NOT NULL,
  card_key     bigint NOT NULL,             -- the desktop's CardId (document-global)
  node_key     bigint NOT NULL,             -- which basket it sits in
  ord          integer NOT NULL,            -- z-order / layout order
  kind         text NOT NULL,               -- text|code|checklist|table|image|sketch
  title        text NOT NULL DEFAULT '',
  body         text NOT NULL DEFAULT '',    -- markdown or source
  pos_x        real NOT NULL, pos_y  real NOT NULL,
  w            real NOT NULL, h      real NOT NULL,
  color        integer[3] NOT NULL,
  font_scale   real NOT NULL DEFAULT 1.0,
  group_key    bigint,                      -- basket-local group id
  docked_to    bigint,                      -- another card_key
  payload      jsonb NOT NULL DEFAULT '{}', -- kind-specific: see below
  updated_at   timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (document_id, card_key),
  FOREIGN KEY (document_id, node_key) REFERENCES nodes(document_id, node_key)
             ON DELETE CASCADE
);
CREATE INDEX ON cards (document_id, node_key, ord);
```

`payload` carries what varies by kind, because these are only ever read and
written as a unit:

| kind | payload |
|---|---|
| `checklist` | `{items: [{done, text}]}` |
| `table` | `{cells: [[{text,bg,fg}]], col_widths: [...], header: bool, chart: {...}\|null}` |
| `sketch` | `{strokes: [{color, width, points: [[x,y]]}]}` |
| `image` | `{ocr: "..."}` — the bytes live in `images` |
| `text` | `{}` (inline image refs live in `card_images`) |

Normalising a 20×10 table into 200 rows would buy joins and nothing else; no
query ever needs a single cell.

**Images are out of band**, content-addressed, deduplicated:

```sql
CREATE TABLE images (
  id       uuid PRIMARY KEY,
  sha256   bytea NOT NULL UNIQUE,      -- same screenshot pasted twice stored once
  bytes    integer NOT NULL,
  mime     text NOT NULL,
  path     text NOT NULL               -- /var/lib/trellis/blobs/ab/cd/abcd…
);
CREATE TABLE card_images (
  document_id uuid, card_key bigint, image_id uuid REFERENCES images(id),
  ord      integer NOT NULL,           -- inline `![](trellis:N)` indexes THIS
  name     text NOT NULL,
  role     text NOT NULL               -- 'primary' | 'extra' | 'inline'
);
```

`ord` is load-bearing: a text card's body references inline images as
`![](trellis:N)` where `N` is a position in the card's `inline_images` vector.
Reorder them on import and every inline image in the document points at the
wrong picture.

### Derived tables, materialised on write

`/api/tags`, `/api/properties`, `/api/query`, `/api/tasks` and `/api/kanban`
currently scan the entire document per request. That is fine at 9,871 cards
in-process and untenable on a server holding many documents. Parse once, on
write, using `trellis-core`'s own parser:

```sql
CREATE TABLE card_tags  (document_id uuid, card_key bigint, tag text);
CREATE TABLE card_props (document_id uuid, card_key bigint, key text, value text);
CREATE TABLE card_links (document_id uuid, card_key bigint, target_title text);
```

Plus a generated search column, which replaces the desktop's linear scan and will
be *faster* than the native app:

```sql
ALTER TABLE cards ADD COLUMN search_tsv tsvector
  GENERATED ALWAYS AS (to_tsvector('english', title || ' ' || body)) STORED;
CREATE INDEX ON cards USING GIN (search_tsv);
```

(OCR text and table cell text must be folded in too — they are searchable on the
desktop. That needs a trigger rather than a generated column, since it reads
`payload`.)

### Lossless `.ron` round-trip

This is the acceptance test for the entire storage layer, and it should be
written **before** the importer:

> Import the real 16 MB document → export it → deserialise both to `Document`
> → assert structural equality.

Run it against the actual work document, not a fixture. Fields that must survive
and are easy to lose:

- node / card / **group** ids, and the three `next_*_id` counters
- ordering of `children`, `cards`, checklist `items`, sketch `points`
- `expanded`, per-node `color` **and** `bg` (two different fields)
- `font_scale`, `group`, `docked_to`
- table `col_widths` and the `chart` spec — `rows` alone loses colours
- inline image **order**, and per-image `name`
- OCR text on image cards

`Card::editing` is `#[serde(skip)]` and correctly absent from persistence.

---

## 5. Sync with the desktop

This is the feature that justifies the project, and it has a hard prerequisite.

### The change log is a blocker, not a nice-to-have

The desktop today exposes `GET /api/wait?rev=N`, which long-polls and returns
when the revision counter moves. **A client learns that something changed and can
never learn what.** The only correct response is to re-fetch the entire document —
16 MB — on every edit. That is not sync, it is polling with extra steps.

**Build the append-only change log first.** It is already the shared prerequisite
for three other things on the roadmap (plugin `on-change` triggers, notifications,
and sort-by-recently-changed), so it earns its keep even if the webapp stalls.

Minimum viable record — deliberately a change *notification* log, not a full
oplog:

| field | why |
|---|---|
| `seq` | monotonic, per document; the sync cursor |
| `ts` | timestamp |
| `actor` | `ui` \| `api:<token>` \| `sync` — needed to avoid echoing your own writes back |
| `entity` | `node` \| `card` \| `group` |
| `entity_id` | which one |
| `op` | `created` \| `updated` \| `deleted` \| `moved` |

The client re-fetches the named entity rather than applying a diff. That is
enormously cheaper to build than a real oplog, cannot desynchronise from a
malformed patch, and is entirely sufficient: fetching one card is a few KB.

Add `GET /api/changes?since=<seq>` and leave `/api/wait` exactly as it is, so
the Android app and existing agents are unaffected.

### The protocol

Both sides store `last_seq` for the peer. A cycle is:

1. `GET /api/changes?since=<local cursor>` from the server; fetch and apply each
   named entity.
2. Push local changes since the server's cursor.
3. Advance both cursors in one transaction.

The desktop end should be **the first plugin**, not built into the app — it is
exactly the shape the plugin framework is being designed for (an executable
handed a base URL and a key, using the existing HTTP API), and it keeps sync
churn out of a stable codebase.

### Conflicts

Per **field**, not per card, and not uniformly:

| field class | rule | why |
|---|---|---|
| `pos`, `size`, `color`, `ord`, `expanded` | last writer wins | Losing a card's position is a shrug. |
| `title`, checklist `done` flags | last writer wins | Small, obvious, cheap to redo. |
| **`body`** | **keep both** | Silently overwriting a paragraph of notes is the one unrecoverable outcome. |
| deletions | tombstone with retention | Otherwise a stale peer resurrects deleted cards. |

For a `body` conflict, write the loser into a **new card beside the original**,
titled `Conflicted copy — <timestamp>`, coloured to stand out. Notes are not
code; a duplicate card the user reconciles in five seconds is strictly better
than a merge algorithm that is confidently wrong. This is the single most
important decision in the sync design.

### Do one-way first

**Phases 1–4 should be desktop → server only, with the server read-only.** That
is a live backup and an any-device viewer — most of the stated motivation — and
it is *safe*, because the server never writes back and cannot lose an edit. Turn
on two-way only once the change log has been running in anger for a while.

---

## 6. Collaboration: presence and locks, not CRDTs

**Recommendation: do not build real-time collaborative text editing.**

The temptation is Yjs or Automerge, and they are genuinely good. The cost is not
the library — it is that every card body becomes a CRDT document with its own
persistence and history, and **the desktop app would have to speak the same CRDT
or become permanently second-class at the thing the user says is their favourite
way to work.** That is a rewrite of the desktop editing path in service of a
feature for "a few collaborators".

What gets 90% of the value for 10% of the cost:

- **Presence** — who is in this basket, avatars in the corner, cursor positions.
- **Card-level soft locks** — opening a card for editing claims it; others see it
  greyed with "Alice is editing"; the lock expires after ~30 s of inactivity.
- **Live spatial ops** — create, delete, move, resize, recolor, group, dock
  broadcast immediately over WebSocket. These are naturally conflict-free
  (they are last-writer-wins on independent fields) and they are *most of what
  makes a shared canvas feel alive*.

The result: two people can work in the same basket comfortably. They cannot type
in the same paragraph simultaneously. For this deployment that is the right
trade.

If it later turns out to be wrong, the upgrade path is Yjs with a per-card
`Y.Text`, the server as relay and persistence. Budget **months**, and re-read the
desktop consequence above before starting.

---

## 7. Sharing

```sql
CREATE TABLE share_links (
  token         text PRIMARY KEY,          -- 32 bytes from the CSPRNG, base64url
  document_id   uuid NOT NULL,
  root_node_key bigint NOT NULL,           -- the shared subtree root
  mode          text NOT NULL DEFAULT 'read',
  expires_at    timestamptz,               -- default now() + 30 days
  passphrase    text,                      -- optional argon2id hash
  revoked_at    timestamptz,
  created_by    uuid NOT NULL
);
```

A share link serves a **stripped client**: the shared subtree only, no tree above
it, no export of the whole document, no API token, no account.

### Two footguns worth writing down

**1. Scope must be evaluated at request time, from the live tree.** If the set of
visible nodes is snapshotted when the link is minted, then dragging a node into a
shared subtree later silently publishes it. Compute descendants of
`root_node_key` on every request. The failure mode of getting this wrong is
exposing private notes without any UI ever indicating it.

**2. A share link is a bearer token in a URL.** It lands in browser history,
`Referer` headers, chat logs, and anything that unfurls links. Given the user has
stated these documents contain sensitive material, mitigations are not optional:

- Default expiry of 30 days, shown at mint time, not buried.
- `Referrer-Policy: no-referrer` and `X-Robots-Tag: noindex` on shared routes.
- Optional passphrase.
- A revocation list in the UI showing every live link, what it exposes, and when
  it was last fetched.
- **A confirmation step that lists exactly which cards become visible** before
  the link is created. A count is not enough — show the titles.

**Recommend against read-write share links entirely in v1.** An anonymous bearer
token with write access to a notes document is a bad idea that is hard to walk
back once someone has used it.

---

## 8. Parity inventory

Systematic pass over the README feature list and API.md. "Better" means the web
version is genuinely superior, not merely equal.

### Tree

| feature | disposition |
|---|---|
| Add root/child/sibling, rename, delete subtree | Direct |
| Reorder, indent/outdent | Direct |
| Expand/collapse, Expand all/Collapse all | Direct |
| Per-node colour tag, per-basket background colour | Direct |
| Copy node id / path | Direct |

### Cards

| feature | disposition | note |
|---|---|---|
| Text: CommonMark, live render, edit/preview | Direct | |
| Text: formatting toolbar, colour picker, font size | Direct | |
| Text: auto-continuing lists | Direct | CodeMirror handles it |
| Code: language selector, highlighting | **Better** | CodeMirror 6 beats the desktop editor |
| Checklist: checkboxes, add/remove, drag reorder | Direct | |
| Table: inline edit, row/col ops, per-cell colours | Direct | |
| Table: CSV/XLSX import & export | Server-side | Reuse `calamine` / `rust_xlsxwriter` |
| Table → chart (bar/line/scatter/pie) | Redesign | Hand-rolled SVG; visually near-identical |
| Sketch: draw, brush size/colour, undo, clear | Direct | SVG polylines |
| Image: multi-image grid, lightbox, zoom/pan | **Better** | Lazy loading, native pinch-zoom |
| Drag/resize/raise/duplicate/recolor/copy-paste | Direct | |
| Inline images in text bodies (`trellis:N`) | Direct | Order is load-bearing — see §4 |

### Organising

| feature | disposition | note |
|---|---|---|
| Groups, dock, snap | Direct | |
| Autosort | Server-side | Reuse `Document::autosort` verbatim |
| **Fit to content** | **Better** | The DOM measures text for real. The desktop maintains *two* implementations — a precise one and a font-free estimate — which has caused three separate bugs (v0.57.2, v0.74.1, v0.74.2). The web needs one. |
| Minimap | Direct | Easy — render scaled rects |

### Views and query

| feature | disposition | note |
|---|---|---|
| Full-text search | **Better** | Postgres GIN index vs a linear scan |
| Quick switcher | Direct | |
| Tags panel, Find cards, Properties | Server-side | Indexed, instant |
| Agenda, Kanban (incl. project filter + colouring) | Server-side | Same semantics via `trellis-core` |
| Wiki-links, Backlinks | Server-side | |
| Link graph | Direct | Force-directed in SVG or Canvas |
| Themes | Direct | CSS custom properties |
| Zoom | **Better** | One CSS transform; no per-dimension scaling to forget |

### Documents and interop

| feature | disposition | note |
|---|---|---|
| New / Open / Save / Save As | Redesign | Documents are DB rows; no file dialogs |
| Autosave | Redesign | DB transactions per operation |
| Import Markdown / HTML / JSON / basket / card | Server-side | Reuse existing code |
| Export document → MD / HTML / JSON | Server-side | Reuse existing code |
| Export document → PDF (paginated) / PNG / GIF | Server-side | Reuse `printpdf` / `image` |
| Export basket → MD / HTML / JSON | Server-side | |
| Drag & drop files onto a basket | **Better** | Browser DnD is excellent |
| Web clipper extension | **Better** | Points at a public origin; no LAN needed |
| Android app | Unchanged | Works if API shapes are preserved (§9) |

### The ones that do not cross cleanly

| feature | disposition | what actually happens |
|---|---|---|
| **Export Card → PNG/PDF (WYSIWYG)** | **Degraded** | The desktop captures the real framebuffer and crops it (`CardShot`). A browser cannot screenshot itself. Options: `html-to-image` client-side (imperfect — misses some CSS, fonts can shift), or headless Chromium server-side (high fidelity, but ~400 MB and a lot of RAM on the VPS, see §11). **Recommend client-side for v1 and be explicit that it is not pixel-identical.** This will never match exactly. |
| **Basket PDF (overview + per-card pages)** | **Degraded** | Same mechanism, same answer. |
| **Snip to card** | **Dropped** | A web page cannot capture your screen. **But the workflow survives**: Win+Shift+S / Cmd+Shift+4 put the region on the clipboard, and Ctrl+V into a basket creates the image card. Arguably fewer steps than the desktop. |
| **OCR** | **Better, server-side** | `tesseract` runs on the VPS as a background job. No per-user install, no Requirements window, works from a phone. |
| **Backup** (scp / rclone / gpg) | **Redesign** | Becomes the *server's* backup (§11). Also largely moot — the webapp existing *is* the backup. |
| **Version history** | **Better** | The desktop keeps 25 full-document snapshots; at 16 MB each that is 400 MB to see one card's history. The change log gives **per-entity** history for a fraction of the space. Genuine improvement. |
| **Templates** | **Redesign** | Currently per-instance config (per `--data-dir`). Become per-user DB rows. **Semantics change** — say so in the UI, since two desktop instances deliberately have separate template libraries today. |
| **X11 primary selection / middle-click paste** | **Dropped** | No web equivalent exists. Platform-specific by nature. |
| **Native file dialogs** | **Replaced** | Browser file input and downloads. |
| **`--port` / `--data-dir` multi-instance** | **Replaced** | Multiple documents per account (§9). |
| **Local `.ron` file ownership** | **Replaced** | The DB is authoritative server-side; the desktop keeps its file. Sync reconciles (§5). |

---

## 9. The agent API under multi-tenancy

The API is a first-class surface at full parity, and the Android app plus an
unknown number of the user's own scripts depend on its exact shapes. Breaking it
would be the most expensive mistake this project could make.

### The core substitution

Today: **one instance = one document = one port = one key.** `GET /api/instance`
answers "which document is this port serving?" and the port *is* the document's
address.

On the server there is one origin and many documents. The clean substitution:

> **The token replaces the port as the document address.**

Issue a per-`(user, document)` API token. Every existing path works **unchanged**:

```
GET https://notes.example/api/tree
Authorization: Bearer <token for work.ron>
```

`GET /api/instance` keeps its exact meaning — "which document am I talking to" —
and answers it from the token. An existing agent script needs its base URL and
key changed and nothing else. The Android app needs nothing but a new host entry.

For agents that work across several documents, add a **parallel** namespaced form
sharing the same handlers:

```
GET /api/d/{document_id}/tree
```

Both routes, one implementation. Nothing is deprecated.

### What else changes

| concern | approach |
|---|---|
| Auth header | Keep both `Authorization: Bearer` and `X-API-Key` — both already work |
| Token scopes | `read` / `write` / `admin`, and optionally a subtree root. Dovetails exactly with the scoped-token decision pending for the desktop plugin framework — **decide it once for both.** |
| `/api/wait` | Keep the shape; back it with Postgres `LISTEN`/`NOTIFY` instead of polling |
| `/api/changes` | New (§5), and should be added to the **desktop** too |
| Rate limiting | Per token. Absent on the desktop because localhost; mandatory on a public origin |
| `/api/health` | Stays unauthenticated |
| App-intercepted endpoints | Backup, history, OCR, templates and `/api/instance` are answered in the desktop's pump loop because they need app state. On the server they are ordinary handlers — **simpler**, since there is no UI thread to bounce off |

**The three-surface rule still applies.** API.md endpoints, API.md Examples, and
the in-app Settings → Endpoints list must stay in sync. A server adds a fourth
surface; keep the route table generated from one source if at all possible.

---

## 10. Security

### The actual threat model

One owner plus a handful of invited collaborators, on a VPS, holding personal and
work notes that the user has said contain sensitive material. The realistic
threats, in order of likelihood:

1. **A share link leaks** (§7) — by far the most likely.
2. **Credential stuffing** on the login form.
3. **An over-scoped agent token** left in a script or a repo.
4. **The VPS is compromised** — via the app, a dependency, or the provider.

Not in the model: targeted attack by a well-resourced adversary. Designing for
that would mean not putting the notes on a server at all.

### Measures

| area | choice |
|---|---|
| Passwords | argon2id, sensible parameters |
| 2FA | **TOTP, and turn it on** — this is the cheapest large win available |
| Sessions | HttpOnly + Secure + SameSite=Lax cookies, short-lived, rotating |
| Federated login | **Rejected** — do not make access to your own notes depend on a third party |
| Transport | TLS only, HSTS, no plaintext port open |
| CSRF | SameSite plus a token on state-changing form posts |
| Tokens | Stored hashed; shown once at creation; revocable; last-used timestamp visible |
| Uploads | Sniff content type, never trust the filename, serve blobs from a separate path with `Content-Disposition` and a strict CSP |

### Encryption at rest — the honest version

**Full-disk encryption on a VPS is mostly theatre.** The host has the key in RAM
while it runs and the provider can image the running machine. It protects against
a decommissioned disk being resold. That is all it protects against.

The real options:

| option | protects against | costs |
|---|---|---|
| Nothing (TLS only) | — | — |
| `pgcrypto` on card bodies, key in the app config | A stolen DB dump | Kills server-side search, tags, tasks, kanban |
| True end-to-end encryption | The server operator | Kills search, all query surfaces, the agent API, and OCR — i.e. most of the product |

**Recommendation: no encryption at rest; treat the VPS as trusted, and say so
plainly in the UI.** Then solve the actual problem a different way:

> **Sync only the subtrees you choose.** If a basket holds credentials, do not
> put it on the server. Subtree-scoped sync is a better answer than encryption
> because it is comprehensible — the user can see exactly what left the machine —
> and it costs nothing in features.

This is the user's call to make, and it should be made deliberately rather than
by default. It is flagged again in §13.

---

## 11. Deployment on Debian

### Packages

```sh
apt install postgresql caddy tesseract-ocr gnupg rclone
# Chromium only if server-side card export (§8) is later judged necessary
```

The app ships as **one static binary** — the same property that makes the desktop
release simple.

### systemd

```ini
[Unit]
Description=Trellis server
After=network.target postgresql.service
Requires=postgresql.service

[Service]
ExecStart=/usr/local/bin/trellis-server
User=trellis
Environment=TRELLIS_DATA=/var/lib/trellis
EnvironmentFile=/etc/trellis/env      # DB URL, secret key — 0600, not in git
Restart=on-failure
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
NoNewPrivileges=true
ReadWritePaths=/var/lib/trellis

[Install]
WantedBy=multi-user.target
```

### Caddy

Automatic Let's Encrypt, including renewal. This is the entire config:

```
notes.example.com {
    encode gzip
    reverse_proxy 127.0.0.1:8080
    header {
        Strict-Transport-Security "max-age=31536000"
        X-Content-Type-Options nosniff
        Referrer-Policy no-referrer
    }
}
```

nginx is equally capable but you hand-roll certbot and a renewal hook. There is
no reason to here.

### Sizing

Working from the real numbers — 956 nodes, 9,871 cards, 16 MB total of which
~4 MB is actual notes and ~12 MB images:

| resource | need |
|---|---|
| Postgres | 20–40 MB including indexes and the change log |
| Blobs | ~16 MB, growing with screenshots |
| RAM | Rust server ~50 MB idle; Postgres ~200 MB tuned small |
| **VPS** | **2 vCPU / 2 GB / 40 GB is generous.** The cheapest tier at most providers. |

Two things would change that: **headless Chromium** for server-side card export
(budget 4 GB), and many concurrent collaborators (not this deployment).

### Backing up the server

The webapp is a backup of the desktop; it still needs its own.

```sh
pg_dump | gpg -c > trellis-$(date +%F).sql.gpg   # nightly, via systemd timer
rclone sync /var/lib/trellis/blobs remote:trellis-blobs
```

Same tools the desktop already knows, so there is nothing new to learn or
install. Test a restore before trusting it.

---

## 12. Effort and phasing

**This is a large project.** Estimates below assume focused sessions and are
deliberately not optimistic. The phasing exists so that value lands early and the
project can be stopped at several points without having wasted the work.

| phase | what | effort | unlocks |
|---|---|---|---|
| **0** | **Spike:** extract `trellis-core`, prove it builds with no eframe/egui | **~1 hour** | Validates the entire stack decision (§1) |
| **1** | Schema + `.ron` import/export with the round-trip test; read-only viewer: tree, all six card kinds, search, tags/agenda/kanban | 2–4 weeks | **Any-device read access, and a real backup** |
| **2** | Share links: tokens, subtree scoping, expiry, revocation UI | ~1 week | **The sharing use case — a large fraction of the "why"** |
| **3** | The change log — desktop *and* server, `GET /api/changes` | 1–2 weeks | Also unlocks plugin on-change triggers, notifications, sort-by-recent **on the desktop** |
| **4** | One-way sync, desktop → server (server read-only) | 2–3 weeks | **The webapp becomes a live mirror.** Safe by construction |
| **5** | Editing on the web: all six kinds, spatial ops, groups, dock, tables, sketches | 4–8 weeks | Web becomes a real client |
| **6** | Two-way sync + conflict handling | 3–4 weeks | Edits converge. **Where data loss lives** — needs a real test corpus |
| **7** | Collaboration: presence + card locks | ~2 weeks | Two people in a basket |
| **8** | Multi-tenant agent API (§9) | ~2 weeks | Can move earlier if agents matter sooner |

**To something genuinely useful: phases 0–2, roughly 4–6 weeks.**
**To honest parity: 6+ months.**

### The recommendation

**Build phases 0–4, then stop and reassess.**

That delivers backup, sharing, and read access from any device — which is most of
what was actually asked for — without paying for the editor or accepting the
risk of two-way sync. Phase 3 pays for itself on the desktop alone.

Phase 5 is the moment the project roughly triples in size, and it is **not
optional**: the brief is full parity in features and capabilities with the
desktop app, and editing is not a feature you can leave out of a note-taking
app. The phasing above is therefore about *order of delivery*, not about scope
— phases 1–4 are what makes the project useful while phase 5 is being built,
not a place to stop.

What genuinely will not reach parity is a short, specific list — WYSIWYG card
export, screen capture, the X11 primary selection, native file dialogs (§9) —
and those are limits of the browser, not decisions.

---

## 13. Decisions that are yours, not mine

1. **Encryption at rest / E2EE.** My recommendation is no encryption, a trusted
   VPS, and **subtree-scoped sync so sensitive baskets never leave the machine**
   (§10). But the notes are yours and the threat model is yours.
2. **Multi-user, or just you plus share links?** Real accounts for collaborators
   add meaningful auth and permission complexity. Share links alone may cover it.
3. **Money.** VPS ~$10–20/mo, domain ~$15/yr. Server-side WYSIWYG card export
   needs a bigger box. Nothing else costs anything.
4. **Does the sync agent live in the desktop app or as the first plugin?** I
   recommend the plugin — it is exactly the shape the plugin framework is being
   designed for, and it keeps sync churn out of a stable codebase.
5. **Domain name.** Unchosen. Nothing depends on it until Phase 2, when share
   links start being handed to other people and the name becomes permanent.

## 14. Open questions

- Does `trellis-core` really extract cleanly? (Phase 0. Everything rests on it.)
- Should the server own `.ron` export well enough that the desktop could
  eventually *open a document over HTTP*, making the file format an interchange
  detail rather than the source of truth? Interesting, and out of scope for v1.
- Is the change log worth building even if the webapp never happens? (I think
  clearly yes — three desktop features are blocked on it.)
- Does WYSIWYG card export matter enough to justify Chromium on the VPS, or is
  "close enough" genuinely fine? Needs a look at how often it is actually used.
