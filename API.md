# Trellis Agent API

A localhost HTTP API for reading and editing a Trellis document while it is open
in the app. Intended for agents (and the user) to collaborate on the same notes:
edits made through the API appear live in the window and are saved with the
document.

- **Base URL:** `http://127.0.0.1:<port>/api` — default port **7373**.
- **Bind address:** `127.0.0.1` only by default. Enable **Tools → Settings → LAN
  access** to bind all interfaces (`0.0.0.0`) so other devices on your network can
  reach it at `http://<this-machine-lan-ip>:<port>/api` (still key-gated; applies on
  restart). Only enable on trusted networks — never expose to the internet without a
  TLS proxy.
- **Format:** JSON request and response bodies; `Content-Type: application/json`.
- **State:** operates on the document currently open in the app. One instance
  serves one document, so **the port is how you address a document**: run an
  instance per document (`trellis ~/work.ron --port 7373 --data-dir …`) and call
  the port belonging to the one you mean. `GET /api/instance` says which document
  the instance you're talking to has open — worth checking before you write.

## Enabling it

The API is **off until a key is set**. In the app: **Tools → Settings → Agent API**,
click **Generate** (or type a key), and **Copy** it. The key and port persist
across restarts. Changing the key takes effect immediately; changing the port
needs an app restart. If the port is busy, the Settings panel shows the bind
error, the status bar says the API is off, and no request is served.

Launching with `--port <PORT>` overrides the saved port for that run, and
`--data-dir <DIR>` gives an instance its own key/port/settings — that pairing is
what lets several instances (one per document) serve different ports at once.
See `trellis --help`.

## Authentication

Every endpoint except `GET /api/health` requires the key, sent as either header:

```
X-API-Key: <key>
Authorization: Bearer <key>
```

- No/empty key configured → `403 {"error":"API disabled: set a key in Settings"}`
- Wrong key → `401 {"error":"missing or invalid API key"}`

## Data model

A document is a **tree of nodes**. Each node has a **basket** of **cards**.

**Node**
| field | type | notes |
|---|---|---|
| `id` | integer | stable within the document |
| `title` | string | |
| `parent` | integer or null | null = root node |
| `children` | array of ids | ordered |
| `color` | `[r,g,b]` or null | 0–255 each; tag dot in the tree |
| `groups` | array | card containers in this basket (see [Groups](#groups)) |
| `touched` | integer or null | unix seconds when this basket last changed — **including any edit to a card in it**, which is what makes "the basket I last worked in" answerable. `null` = unchanged since the field existed |

**Card** — `kind` is one of `text`, `code`, `checklist`, `table`, `image`, `sketch`.
| field | applies to | notes |
|---|---|---|
| `id` | all | |
| `title` | all | shown in the card's title bar (all kinds, incl. `image`) |
| `kind` | all | `"text"` \| `"code"` \| `"checklist"` \| `"table"` \| `"image"`; PATCH can convert an existing card to another kind |
| `pos` | all | `[x,y]` top-left on the basket canvas |
| `size` | all | `[w,h]` in canvas units |
| `color` | all | title-bar accent — set as `[r,g,b]` (0–255), a hex string (`"#ef4444"`, `"#e44"`), or a name (`"red"`, `"green"`, `"blue"`, …) |
| `font_scale` | text, code | body font-size multiplier (1.0 = default; clamped 0.25–4.0) |
| `group` | all | group id this card belongs to, or null — set via the group sub-resource (below) |
| `docked_to` | all | id of the card this one is docked to, or null — set via the dock sub-resource |
| `body` | text, code | Markdown (text) or source (code) |
| `inline_image_names` | text | names of images embedded in the body via `![](trellis:N)` markers (read; present only when the card has inline images). Set/replace with the `inline_images` field on create/PATCH |
| `lang` | code | syntax-highlight language, e.g. `"rust"` |
| `items` | checklist | `[{ "done": bool, "text": string }]` |
| `image_name`, `image_names`, `bytes` | image | first/all image names + total byte count (read); set image bytes via the images sub-resource (below) |
| `rows`, `header` | table | `rows` set: `[["a","b"],…]` bulk-replaces cell text (colors reset); get: cells as `{text,bg,fg}`. `header` (bool) toggles the header row. Fine-grained edits (cell colors, widths, row/col ops) use the table sub-resource (below) |
| `chart` | table | how the table is drawn as a chart (`{kind,label_col,value_cols,show_table}`), or `null` for a plain grid. Set via the chart sub-resource (below) |
| `strokes` | sketch | read: `[{color:[r,g,b], width, points:[[x,y],…]}, …]`. Edit via the sketch sub-resource (below) |
| `touched` | all | unix seconds when this card last changed (read-only; omitted entirely if it never has). The document's only timestamp — unlike `/api/changes` it survives a restart |
| `source` | text, code | a file this card **mirrors**: `body` becomes a read-only live copy, refreshed while the document is open. Omitted when the card isn't mirroring |
| `source_error` | text, code | why the last read failed (`null` when fine). Only present alongside `source` |

**Group** — a labeled container that a set of cards belong to; drawn as a box you
can drag by its header. Membership lives on each card's `group` field.
| field | type | notes |
|---|---|---|
| `id` | integer | stable within the node |
| `title` | string | shown on the group header |
| `color` | `[r,g,b]` | container accent |
| `cards` | array of ids | current members |

Text card bodies are **CommonMark Markdown** (headings, lists, tables, task
lists, fenced code with highlighting, bold/italic/strikethrough). There is no
underline. Use `\n` for line breaks.

## Endpoints

`{id}` is a node id, `{cid}` a card id. Bodies marked `{…}` are JSON. `?` = optional.

### Health
```
GET /api/health        → 200 {"status":"ok","app":"trellis"}   (no auth)
```

### Instance
Which document *this* instance is serving. With one instance per document (each
on its own port), call this first to confirm you're driving the right one.
```
GET /api/instance
  → 200 {"app":"trellis","version":"0.65.1","document":"work.ron",
         "path":"/home/you/work.ron","port":7373,"lan":false,
         "nodes":42,"unsaved_changes":false}
```
`document` is the file name (`"untitled"` for a never-saved document) and `path`
is its full path, or `null` when untitled. `nodes` is the document's node count.
Unlike `/api/health` this needs the key, since it reveals a file path.

### Read
```
GET /api/tree
  → 200 {"roots":[ {id,title,color,cards:<count>,children:[ …recursive… ]} ]}

GET /api/nodes
  → 200 {"nodes":[ {id,title,parent,children:[ids],cards:<count>} ]}

GET /api/nodes/{id}
  → 200 {id,title,parent,children:[ids],color,bg,groups:[<group>…],cards:[<card>…]}   | 404
        bg: basket background color ([r,g,b] or null)

GET /api/nodes/{id}/cards
  → 200 {"cards":[<card>…]}                                      | 404

GET /api/nodes/{id}/cards/{cid}
  → 200 {<card>}                                                 | 404
        one card on its own — the read counterpart of PATCH/DELETE on this same
        path, so re-reading a card you just wrote doesn't mean pulling the whole
        basket back. Same object as an entry in the list above.

GET /api/search?q=<text>
  → 200 {"hits":[ {node,card,node_title,snippet} ]}                   (case-insensitive)
```
Note: `tree` and `nodes` report `cards` as a **count**; `GET /api/nodes/{id}`
returns the **full card objects**.

In every `hits` list (search, tags, properties, query, backlinks), `card` is the
id of the matching card so a client can point straight at it. It is `null` only
for a search hit that matched a **node title** rather than a card.

### Create
```
POST /api/nodes            {title, parent?}
  → 201 {"id":<new>}   | 400 if parent doesn't exist

POST /api/nodes/{id}/cards {kind?, title?, body?, lang?, items?, rows?, header?, pos?, size?, color?, font_scale?, fit?, image_base64?, inline_images?, source?}
  → 201 {"id":<new>}   | 404 if node doesn't exist
```
`kind` defaults to `"text"` and may be any of `text`, `code`, `checklist`,
`table` (starts as an empty 3×3), `image`, or `sketch` (an empty draw surface). `pos` is `[x,y]` canvas coordinates
(default `[40,40]`); pass distinct positions to avoid stacking cards on top of
each other. `size` is `[w,h]`. `color` sets the title-bar accent at creation (see
the accepted formats below). `items` is used only for `checklist`; `lang` only
for `code`. `rows` fills a **table** card's cells row by row (`[["a","b"],…]`,
ragged rows padded to the widest) and `header` styles its first row — so a
populated table, and a chart drawn from it, take one call instead of three. `image_base64` gives an `image` card its first image (base64 file
bytes; the `title` becomes its name). `inline_images` embeds images **inside a
text card's body**: pass an array of base64 file bytes, then reference each in
`body` with a `![alt](trellis:N)` marker (`N` = its 0-based index in the array);
they export as data URIs in HTML/Markdown and show on the card's PDF page.
**`fit: true`** sizes the card to fit its content (overrides `size`), so a card
comes out readable instead of a tiny square — recommended for agent-created
cards. No effect on image cards. Since 0.74.1 it measures text with the real
fonts, so it gives exactly the size the app's right-click → *Fit to content*
does; before that it estimated, and estimated tall, leaving a gap under the
text. Note it sizes to the **title bar** too, so a long title widens the card
even when the body is short.

### Update
```
PATCH /api/nodes/{id}              {title?, color?, bg?}
  → 200 {"id":<id>}    | 404
        color: tag color; bg: basket background color — both setting only
        (can't clear via API; use the app's Default to reset)

PATCH /api/nodes/{id}/cards/{cid}  {title?, body?, color?, kind?, font_scale?, fit?, lang?, pos?, size?, items?, rows?, header?, inline_images?, source?}
  → 200 {<updated card>}   | 404
```
Every field is optional; only those present are changed. `pos`/`size` are
`[x,y]`/`[w,h]`; **`fit: true`** resizes the card to fit its content (applied after
every other field; overrides `size`); `font_scale` sizes text/code body font (1.0 = default);
`lang` applies to code cards, `items` replaces a checklist's items (send them in
the desired order to **reorder** a checklist), `rows` bulk-replaces a table's cell
text, `header` toggles a table's header row, `inline_images` replaces the text
card's embedded inline images (same base64 + `![](trellis:N)` scheme as create). **`kind` converts the card to
another kind** (`text`/`code`/`checklist`/`table`/`image`) — apply it in the same
PATCH as `items`/`rows`/etc. and the new content lands in the converted card. The
response is the full updated card object.

**Color format** — anywhere the API takes a `color` (nodes, cards, groups, on
create or update) you may send an `[r,g,b]` array (0–255 each), a hex string
(`"#ef4444"`, `"ef4444"`, or shorthand `"#e44"`), or a color name from the app's
swatch palette: `"red"`, `"orange"`, `"amber"`, `"yellow"`, `"lime"`, `"green"`,
`"teal"`, `"cyan"`, `"blue"`, `"indigo"`, `"purple"`/`"violet"`,
`"pink"`/`"magenta"`, `"slate"`/`"gray"`, `"stone"`, `"white"`, `"black"`.
Card/group colors are a **title-bar / container accent**, not a full fill. An
unrecognized color is a `400`, so a successful response means the color was
applied.

### Delete
```
DELETE /api/nodes/{id}             → 200 {"deleted":<id>}    | 404   (removes the whole subtree)
DELETE /api/nodes/{id}/cards/{cid} → 200 {"deleted":<cid>}   | 404
```

### Move / reorder
Reposition a node among its siblings or reparent it under another node. Sidebar
order is the raw child order, so this is how an agent sets where a basket lands.
Pick exactly one placement per call:
```
POST /api/nodes/{id}/move  {before:<nid>}            place {id} immediately before that sibling (adopts its parent)
POST /api/nodes/{id}/move  {after:<nid>}             place {id} immediately after that sibling (adopts its parent)
POST /api/nodes/{id}/move  {parent?, index:<n>}      put under parent at 0-based slot n (index past the end appends)
POST /api/nodes/{id}/move  {parent?, to:"top"|"bottom"}   move to the top/bottom of parent
    → 200 {"id":<id>, "parent":<pid|null>, "index":<n>}     new location
    → 400   empty body, bad "to", unknown target, or a move that would nest a node in its own subtree
    → 404   {id} not found
```
`parent`: omit to keep the current parent, `null` to move to the top level, or a
node id to reparent. `before`/`after` ignore `parent`/`index`/`to` (they take the
target's parent). Examples:
```
# put the 2022 year node right before 2023
curl -sX POST $API/nodes/811/move -H "X-API-Key: $KEY" -d '{"before":612}'
# send a day to the top of its month
curl -sX POST $API/nodes/958/move -H "X-API-Key: $KEY" -d '{"to":"top"}'
# reparent a stray card-basket under another node, at the end
curl -sX POST $API/nodes/814/move -H "X-API-Key: $KEY" -d '{"parent":766}'
```

### Expand / collapse
Open or fold a node in the sidebar. `recursive` applies it to the whole subtree
(node + every descendant) — the one-click way to tidy a big branch.
```
POST /api/nodes/{id}/expand  {expanded:true|false, recursive?:false}
    → 200 {"id":<id>, "expanded":<bool>, "changed":<n>}    | 404
```
(`GET /api/nodes/{id}` now includes the node's `expanded` flag.)

### Reorder a card
Set a card's place in its basket's order — which is both the draw order (last =
on top) and the sequence Autosort lays cards out in. Pick one placement:
```
POST /api/nodes/{id}/cards/{cid}/move  {before:<cid>}          before that card
POST /api/nodes/{id}/cards/{cid}/move  {after:<cid>}           after that card
POST /api/nodes/{id}/cards/{cid}/move  {index:<n>}             absolute 0-based slot (past end = last)
POST /api/nodes/{id}/cards/{cid}/move  {to:"front"|"back"}     front = on top / laid out last
    → 200 {"card":<cid>, "index":<n>}    | 400 (bad/empty placement) | 404 (card not found)
```
Move a card to a **different** basket with `node` (and optionally `pos`), which
takes precedence over the ordering fields above:
```
POST /api/nodes/{id}/cards/{cid}/move  {node:<target id>, pos?:[x,y]}
    → 200 {"card":<cid>, "node":<target>, "moved":true}
    | 400 (already in that node) | 404 (card or target node not found)
```
The card keeps its content, size and colors; `pos` places it on the target canvas
(without one it keeps its coordinates). **Group membership and docking are
dropped** — both reference ids local to the old basket — and anything docked to it
is detached.

Tip: to lay a basket out in a specific reading order, `move` the cards into that
order, then `POST …/autosort`.

### Groups
Bundle 2+ cards into a labeled container that moves as one.
```
GET    /api/nodes/{id}/groups            → 200 {"groups":[ {id,title,color,cards:[ids]} ]}   | 404

POST   /api/nodes/{id}/groups            {cards:[ids], title?}
  → 201 {"id":<gid>}   | 400 (need ≥2 existing cards)  | 404

PATCH  /api/nodes/{id}/groups/{gid}      {title?, color?}
  → 200 {"id":<gid>}   | 404

DELETE /api/nodes/{id}/groups/{gid}      → 200 {"ungrouped":<gid>}   | 404   (cards remain, container removed)
```

### Docking
Stick one card to another so they move together (`card` docks onto `anchor`).
```
POST   /api/nodes/{id}/cards/{cid}/dock  {anchor:<cid>}
  → 200 {"card":<cid>,"docked_to":<anchor>}   | 400 (would form a cycle)  | 404

DELETE /api/nodes/{id}/cards/{cid}/dock  → 200 {"card":<cid>,"docked_to":null}   | 404
```
Moving a card in the app (or via `pos`) moves everything docked to it. A card
can be both grouped and docked.

### Card group membership
Add an existing card to an existing group, or remove it — beyond the bulk
`POST …/groups` that creates a new group from 2+ cards.
```
POST   /api/nodes/{id}/cards/{cid}/group  {group:<gid>}
  → 200 {<updated card>}   | 404 (card or group not found)

DELETE /api/nodes/{id}/cards/{cid}/group  → 200 {<updated card>}   | 404
```

### Table editing
Fine-grained edits to a `table` card. One operation per request; `op` selects it.
```
POST /api/nodes/{id}/cards/{cid}/table  {op, …}          one op
POST /api/nodes/{id}/cards/{cid}/table  [{op, …}, {op, …}]   several, applied in order
  → 200 {<updated card>}   | 400 (unknown op / not a table / index out of range)  | 404
```
| `op` | args | effect |
|---|---|---|
| `set_cell` | `row`, `col`, `text` | set one cell's text |
| `set_bg` | `row`, `col`, `color` | cell background (color format below; null/absent clears) |
| `set_fg` | `row`, `col`, `color` | cell font color (null/absent clears) |
| `insert_row` | `at` | insert a blank row at index `at` |
| `remove_row` | `at` | delete row `at` (never below 1 row) |
| `insert_col` | `at` | insert a blank column at index `at` |
| `remove_col` | `at` | delete column `at` (never below 1 col) |
| `set_col_width` | `col`, `width` | set a column's pixel width (28–600) |
| `autofit_cols` | `col`? | size columns to their content — every column, or just `col` |
| `set_header` | `header` | set the header-row flag (bool) |

Columns are **110px** until something changes them, and cell text does not wrap —
so a table built from `rows` clips anything longer than that. **`autofit_cols`
after filling a table** and the columns size themselves to their longest cell
(bounded at 600px, so one runaway cell can't produce an unusable card). Note that
`"fit": true` on a table card sizes the card's *frame* around the widths the
columns already have; it does not widen the columns. Fit the columns first, then
the card:

```bash
curl -s -H "X-API-Key: $KEY" -d '{"op":"autofit_cols"}' $API/nodes/$NID/cards/1/table
curl -s -H "X-API-Key: $KEY" -X PATCH -d '{"fit":true}' $API/nodes/$NID/cards/1
```

### Sketch editing
Draw on a `sketch` card programmatically. One operation per request.
```
POST /api/nodes/{id}/cards/{cid}/sketch  {op, …}
  → 200 {<updated card>}   | 400 (unknown op / not a sketch / nothing to change)  | 404
```
| `op` | args | effect |
|---|---|---|
| `add_stroke` | `points` `[[x,y],…]`, `color` (array/hex/name), `width` | append a freehand stroke (points are in the card's local coordinates) |
| `undo` | — | remove the last stroke |
| `clear` | — | remove all strokes |

### Images
Attach or remove image bytes on an `image` card (grid layout; first image is the
primary). Bytes are png/jpeg/gif/bmp/webp.
```
POST   /api/nodes/{id}/cards/{cid}/images        {data_base64, name?}
  → 201 {<updated card>}   | 400 (bad base64)  | 404 (not an image card)

GET    /api/nodes/{id}/cards/{cid}/images/{idx}  → 200 {index, name, base64}   | 404
  (the image's raw bytes, base64-encoded — index 0 is the primary image)

DELETE /api/nodes/{id}/cards/{cid}/images/{idx}  → 200 {<updated card>}   | 404
```

### Autosort
Arrange a node's cards into a tidy, non-overlapping grid (the same as the app's
**Tools → Autosort cards**). Cards are clustered by group; docking is cleared.
```
POST /api/nodes/{id}/autosort  → 200 {"sorted":<id>}   | 404 (no node / no cards)
```

### Export
Export the **whole document** in a portable format.
```
GET /api/export?format=<fmt>
  → 200 text formats:   {"format":<fmt>,"content":"<string>"}     (markdown|html|json)
  → 200 binary formats: {"format":<fmt>,"base64":"<b64 bytes>"}   (pdf|png|gif)
  | 400 unknown format
```
`format` defaults to `markdown`. `pdf` is a paginated A4 document; `png`/`gif`
are a single rendered image of the document text. Decode `base64` to get the file.

### Backlinks
`[[Node Title]]` (or `[[id]]`, or `[[Target|shown text]]`) written in a card is a
wiki-link; in the app it renders as a clickable link that navigates to that node.
This lists the cards that link *to* a node.
```
GET /api/nodes/{id}/backlinks → 200 {"node":<id>,"count":N,"hits":[{node,card,node_title,snippet}, …]}  | 404
```

### Link graph
The wiki-link graph: nodes that take part in at least one `[[link]]`, and the
directed edges between them (source → target). Powers **View → Link graph**.
```
GET /api/graph → 200 {"nodes":[{id,title}, …], "edges":[[from,to], …]}
```

### Tags
`#tags` written anywhere in a card (body, title, checklist items, table cells)
are indexed across the whole document. A tag starts at a `#` on a word boundary
whose first char is a letter — so `# Heading`, `page#frag`, and `#123` are not
tags; `#work/urgent` (nested) is. Tags are lowercased.
```
GET /api/tags            → 200 {"tags":[{"tag":"todo","count":3}, …]}   (all tags, by name)
GET /api/tags?name=todo  → 200 {"tag":"todo","hits":[{node,card,node_title,snippet}, …]}
```

### Properties
Inline `key:: value` fields (Dataview-style) written in a card are parsed as
metadata — e.g. `due:: 2026-08-15`, `priority:: high`, `status:: open`. The `::`
must be followed by a space (so `std::fmt` and URLs aren't matched). Works as a
whole line or bracketed inline (`[due:: 2026-08-15]`). Keys are lowercased. A
card's parsed properties are included in its JSON as `properties:[{key,value}]`.
```
GET /api/properties                    → 200 {"properties":[{"key":"due","count":4}, …]}
GET /api/properties?key=due            → 200 {"key":"due","value":null,"hits":[{node,card,node_title,snippet}, …]}
GET /api/properties?key=status&value=open → 200 hits where status == open
```
Set a property on a card (rewrites the `key:: …` line in its body, or appends
one) — e.g. to move a card on the Kanban board:
```
POST /api/nodes/{id}/cards/{cid}/property  {key, value}
    → 200 {"card":<cid>,"key":<k>,"value":<v>}   | 404
```

### Find cards (combined query)
AND-combine a `#tag`, a property (`key`, optional `value`), and free `text`
across the whole tree. Powers the **View → Find cards** panel.
```
GET /api/query?tag=todo&key=status&value=open&text=release
    → 200 {"count":N,"hits":[{node,card,node_title,snippet}, …]}
```
All params optional, but at least one of `tag`/`key`/`text` is needed (else empty).

### Tasks (agenda)
Every card carrying a `due:: <date>` property, as tasks. A task is **done** if it
has `status:: done|complete|closed` or (for a checklist) all items are checked.
`bucket` is relative to today (UTC): `overdue` · `today` · `week` (next 7 days) ·
`later` · `nodate` (unparseable date). Powers the **View → Agenda** panel.
```
GET /api/tasks           → 200 {"today_days":N,"count":M,"tasks":[{node,node_title,node_path,project,project_title,card,title,due,done,bucket}, …]}
GET /api/tasks?project=<node id>   only tasks under that node (a project, or any sub-branch)
  | 400 (bad id)   | 404 (node not found)
GET /api/tasks?all=true  → include completed tasks too (default excludes them)
```

`project` / `project_title` are the task's **top-level ancestor** — the project
it belongs to. `?project=<node id>` filters to one; it accepts any node, not just
a root, so you can narrow to a sub-branch with the same parameter.

**`node_path` is the root-to-basket breadcrumb** (`Super Weapon News › Open
Items`), and it is what you should show or reason about — **`node_title` alone is
ambiguous**. Basket names like "Open Items" or "Session Handoffs" repeat under
every project, so the bare title cannot say which project a task belongs to, and
has already led an agent to attribute a task to the wrong one.

### Kanban
Cards grouped by their `status::` value — the **View → Kanban** board's columns.
Each card carries its title, basket (`node_title`), full basket path
(`node_path`), `due::` date, `#tags`, and
accent `color`. Read-only; change a card's column with the card `property`
endpoint (`POST …/cards/{cid}/property {key:"status", value:"done"}`).
```
GET /api/kanban → 200 {"today_days":N,"columns":[{"status":"doing","count":2,"cards":[{node,node_title,node_path,project,project_title,card,title,due,tags,color}, …]}, …]}
GET /api/kanban?project=<node id>   only cards under that node (same filter as /api/tasks)
  | 400 (bad id)   | 404 (node not found)
```

### OCR
Run OCR (tesseract) over every image card that has images but no extracted text
yet, so scans/screenshots become full-text searchable. Runs on a background
worker; poll `/api/search` for the results.
```
POST /api/ocr → 200 {"started":<bool>,"cards":<n queued>}
```
(The **Snip to card** capture — grab a screen region into an image card — is
UI-only, since it needs a human to select the region on screen.)

### Version history
Browse the automatic save-time snapshots and restore one (replaces the current
document in memory — save to keep it). Snapshots live in `.<name>.history/`.
```
GET  /api/history                 → 200 {"count":N, "keep":N, "min_gap_mins":N,
                                          "snapshots":[{file,when,bytes}, …]}   (newest first)
     keep / min_gap_mins are the retention settings (Settings → Version history):
     how many snapshots are kept and the minimum gap between them. Read-only here —
     they are set in the app, like the backup schedule.
POST /api/history/restore  {file} → 200 {"restored":true}   | 400 bad name | 404 not found
```

### Backup
Trigger a full-document backup, or read the backup status. Destinations,
schedule, and encryption are configured in the app (**Tools → Backup…**); this
endpoint runs the same job on demand.
```
GET  /api/backup       → 200 {enabled, interval_mins, encrypt, running,
                               last_backup_secs_ago, last_result, destinations:[…]}
POST /api/backup/run   → 200 {"started":true}
                       | 400 no enabled destinations   | 409 a backup is already running
```
The run happens on a worker thread; poll `GET /api/backup` (`running`,
`last_result`) for the outcome.

Backup settings live in the app config, so they are **per instance**, and a run
backs up **the document that instance has open**. Running an instance per
document therefore gives each document its own schedule and destinations —
configure them separately.

### Templates
Reusable card snapshots — the same ones as the app's right-click **Save as
template** / **Insert template**. A template captures a card's *whole* definition
(kind, title, body, size, colors — including a table's columns, header flag,
per-cell colors and widths, or a checklist's items), so it's ideal for stamping
out a fixed layout (e.g. a Task / Local / Prod verification grid) again and
again. Templates persist in the app config, so they survive restarts and are
shared by the UI and the API. Index is a 0-based position in the list.

**Every template has a master card in the root-level `Templates` basket.**
Registering one stamps its master there (creating the basket the first time), so
a saved template is something you can *see and edit* rather than an invisible
config entry. Edit the master, then `update` that slot, and every later insert
stamps the new version; the basket is kept in step with the stored snapshot.
Registering a card that already sits in `Templates` adopts that card as the
master instead of cloning it. Deleting a template removes its master too.

The snapshot in config is the authority — `insert` always stamps *it*, never the
master — so editing a master changes nothing until you `update`. That is
deliberate: a stray edit to a master must not silently change every future
insert.

The config, not the document, is where templates live, so a library is **per
instance, not per document**: instances started with different `--data-dir`s have
independent template lists and their own `Templates` basket, and one instance's
templates are invisible to another. To reuse a template elsewhere, insert it into
a basket in the other instance and register it there.
```
GET    /api/templates
  → 200 {"count":N,"templates":[{index, title, kind, master_node, master_card}, …]}
  master_node/master_card locate the template's master card, or are null if it
  hasn't got one (see rebuild below).

POST   /api/templates              {node, card, title?}
  → 200 {"index":<n>,"title":"...","master_node":<id>,"master_card":<cid>}
  | 404 (node/card not found)
  Snapshot an existing card as a template, and stamp its master into the
  `Templates` basket. Build the card however you like first (e.g. create a table,
  set its cells/colors), then register it — optional `title` overrides the card's
  title as the template name.

POST   /api/templates/rebuild      → 200 {"node":<id>,"stamped":N,"already_present":M,"templates":T}
  Give every template that hasn't got a live master card one, creating the
  `Templates` basket if needed. Use it on a library registered before masters
  existed; it only touches templates that are missing one, so it's safe to repeat.
  (Tools → Rebuild Templates basket in the app.)

POST   /api/templates/{index}/insert  {node, pos?}
  → 200 {"node":<id>,"card":{<created card>}}   | 404 (no template / node)
  Stamp the template into a basket as a new card (`pos` defaults to [40,40]).

POST   /api/templates/{index}/update  {node, card, title?}
  → 200 {"updated":<index>,"title":"...","master_node":<id>,"master_card":<cid>}
  | 404 (no template / node / card)
  Re-snapshot an existing template slot from a card, in place — the template keeps
  its index (and its current name unless `title` is given). This is the template
  editor flow: edit the master in the `Templates` basket, then update, and every
  future insert stamps the new version. Updating from some *other* card also
  refreshes the master so the basket keeps showing what inserts will stamp.

DELETE /api/templates/{index}      → 200 {"deleted":<index>,"title":"..."}   | 404
  Removes the template and its master card.
```
There is no separate "create a template from scratch" body — register from a card
so the template captures exactly what you'd see on the canvas. Editing a card you
registered does **not** change the stored template until you `update` it (or the
right-click **Update template**) — templates are snapshots, not live links.

### Charts
Draw a **table** card as a chart. The table stays the data — the chart is a view
of the same cells, so editing a cell (or a `rows` PATCH, or a `table` op) redraws
it. No separate card kind, and the grid is still there underneath when you want it.
```
POST   /api/nodes/{id}/cards/{cid}/chart  {kind, label_col?, value_cols?, show_table?}
  → 200 {"chart":{kind, label_col, value_cols, show_table}}
  | 400 (not a table card / bad kind)   | 404 (node or card not found)

DELETE /api/nodes/{id}/cards/{cid}/chart  → 200 {"chart":null}
  Back to a plain grid.
```
- **`kind`** — `bar`, `line`, `scatter`, or `pie` (`donut` is accepted as an
  alias).
- **`label_col`** (default `0`) — the column supplying each point's label / x-axis
  category.
- **`value_cols`** — columns to plot. Omit or leave empty to plot **every numeric
  column** except `label_col`, so the chart keeps working when you add a column.
- **`show_table`** (default `false`) — also show the source grid under the chart.
- Omitted fields keep their current value, so you can flip `kind` without
  restating the columns.

A card's JSON carries `chart` (the object above, or `null`) alongside `rows`.

**Pie is different in two ways**, because it divides a single whole rather than
plotting x against y:
- It draws the **first** series only — `value_cols[0]` if you gave one, else the
  first numeric column. The other columns are ignored, not stacked.
- Only **positive** values get a slice. Blanks, zeros and negatives are skipped,
  and percentages are of the positive total — a negative has no arc, and folding
  it in as its magnitude would misstate every other slice. `show_table` still
  works, and hovering a slice shows its exact value and percentage.

**How cells are read:** the header row supplies series names when `header` is
true. Numbers may be decorated — `1,234.5`, `$12`, `40%`, `(3)` = −3. A cell that
isn't a number is a **gap**, not a zero: a line breaks across it and a bar is
omitted, because plotting a blank status cell as 0 would invent a reading that
was never taken. A lone value between two gaps still shows, as a dot.

### Mirror a file (`source`)
Point a **text** or **code** card at a file and its body becomes a **read-only
live copy**, re-read while the document is open (checked every ~3 s; only files
whose modification time changed are re-read).
```
POST  /api/nodes/{id}/cards        {kind:"text", title:"README", source:"/srv/app/README.md"}
PATCH /api/nodes/{id}/cards/{cid}  {source:"/srv/app/README.md"}   attach or re-point
PATCH /api/nodes/{id}/cards/{cid}  {source:""}                     detach, keeping the text
```
The mirrored text is stored in the document like any other body, so it is
searchable, carries `#tags` and `key:: value`, and exports normally — the file
stays authoritative and Trellis holds a cache of it.

- **`body` is read-only while `source` is set.** `PATCH {"body":…}` returns
  **409** rather than accepting an edit the next refresh would overwrite. Detach
  first with `{"source":""}` — which keeps the text that was there.
- **A failed read keeps the last good text** and reports why in `source_error`
  (missing file, unmounted disk, a directory, not UTF-8, or over the 1 MB limit).
  It recovers on its own when the file comes back.
- Only text and code cards mirror. There is no write-back: edit the file.

> **What agents may mirror is limited.** By default a card can mirror any file
> *except* credential paths (`.ssh`, `.aws`, `.gnupg`, `*.pem`, …); a refused
> `source` returns **403**. *Settings → Agent API → Files agents may mirror*
> narrows it to a folder list or removes the limit. The app's own file picker is
> never restricted.
>
> **Read this before enabling LAN access.** A caller who can create cards can
> point one at **any file this user can read**, and then fetch its contents back
> through `GET /api/nodes/{id}/cards/{cid}` or `GET /api/export`. The API is
> key-gated, so this is not a way past authentication — but it does widen what a
> leaked key is worth from "all your notes" to "any file on the machine". A
> directory allow-list is on the roadmap; until then, treat the API key as
> equivalent to filesystem read access.

### Live updates (long-poll)
Be woken the instant the document changes, instead of polling on a timer.
```
GET /api/wait?rev=<n>
  → 200 {"rev":<current>,"changed":true,"epoch":<n>}   (as soon as the revision differs from n)
  → 200 {"rev":<current>,"changed":false,"epoch":<n>}  (after ~25s with no change; just re-request)
```
The server holds the request open until the document's change counter differs from
`rev` (any add/edit/move/remove, from the app or another agent), or ~25 s elapse.
Loop it, passing back the `rev` you last received, to react immediately. Start with
`rev=0` to get the current revision on the first call. Requests are handled
concurrently, so an open `/wait` never blocks your other calls.

### What changed (the change log)
`/api/wait` tells you *that* the document moved. This tells you **what** moved, so
you can re-read one card instead of the whole document.
```
GET /api/changes?since=<seq>&limit=<n>
  → 200 {"epoch":…, "rev":…, "since":…, "count":N, "retained":M,
         "oldest":<seq|null>, "truncated":false, "changes":[ … ]}
```
Pair it with the long-poll: wait for a `rev`, then ask what happened since the
`seq` you last processed. `limit` defaults to 500 (max 5000).

Each entry:

| field | meaning |
|---|---|
| `seq` | the revision this change belongs to — the same number `/api/wait` returns |
| `ts` | unix seconds (the document itself stores no timestamps) |
| `actor` | `ui` (a person in the app) or `api` (an agent) |
| `entity` | `node` · `card` · `group` · `document` |
| `op` | `created` · `updated` · `deleted` · `moved` |
| `id` | the node/card/group id. Card ids are document-global |
| `node` | the owning basket, for a card or group |
| `title` | its title at the time — present even for a delete, when it can no longer be looked up |
| `fields` | which parts changed: `["body","color"]`, `table.set_cell`, `images.add`, … |
| `property` | `["status","done"]` — only for a `key:: value` change |

Absent fields are omitted rather than sent as null.

```jsonc
{"seq":4,"ts":1785950176,"actor":"api","entity":"card","op":"updated","id":2,
 "node":62,"title":"Deploy checklist","fields":["property"],
 "property":["status","done"]}
```

The log is per-session. For "when did this last change" **across** restarts, read
`touched` on the node or card instead — the two are stamped together, from the
same place, so they cannot disagree.

**Three things to get right:**

- **`epoch` changes when the app restarts.** The log is in memory, so a stored
  `seq` from a previous run means nothing. Different `epoch` than you last saw →
  re-read what you care about and start again from that run's `rev`.
- **`truncated: true`** means entries you needed have already rotated away (the
  log keeps the last 5000). Incremental catch-up is impossible; re-read.
- **An entry says what changed, never the old and new values.** Re-fetch the
  entity named. That is what makes the log impossible to desync from — there is
  no patch to misapply — and it is why consecutive identical changes collapse
  into one entry (a card drag is *one* `moved`, not one per frame).

## Plugin tokens

Besides the instance key, the API accepts **tokens minted for approved plugins**
(*Tools → Plugins…*). They are ordinary credentials — same `X-API-Key` header —
but **scoped**, and the scope is enforced here rather than trusted:

- A **read-only** plugin's token is refused on anything but `GET`, with
  `403 {"error":"plugin '<name>' has read-only access to this document"}`. This is
  checked before the request reaches the document.
- A plugin confined to a **subtree** may only act on that node and its
  descendants. A request that names no node is **refused** rather than allowed
  through, except for the structural reads it needs to orient itself
  (`/api/health`, `/api/instance`, `/api/tree`, `/api/nodes` — titles and shape,
  never card content). `403 {"error":"outside the basket this plugin was given
  access to"}`.

Plugin tokens are prefixed `plug_` so they're tellable from the instance key at a
glance. Revoking a plugin deletes its token and takes effect immediately.

## Errors

All errors return `{"error":"<message>"}` with the status code:

| code | meaning |
|---|---|
| 400 | bad JSON body, bad id, or missing parent |
| 401 | wrong API key |
| 403 | API disabled (no key set) |
| 404 | node/card not found, or unknown route |
| 503 | app not accepting requests |
| 504 | app didn't respond in time (window busy/hung) |

## Examples

```sh
KEY=<your key>
API=http://127.0.0.1:7373/api

# Confirm which document this port is serving before writing to it — with an
# instance per document (work on 7373, personal on 7374), the port is the address.
curl -s -H "X-API-Key: $KEY" $API/instance
# → {"app":"trellis","document":"work.ron","path":"/home/you/work.ron","port":7373,…}

# See the whole tree
curl -s -H "X-API-Key: $KEY" $API/tree

# Create a node, capture its id
NID=$(curl -s -H "X-API-Key: $KEY" -d '{"title":"Meeting notes"}' $API/nodes \
      | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

# Add a Markdown card to it
curl -s -H "X-API-Key: $KEY" \
  -d '{"kind":"text","title":"Agenda","body":"# Agenda\n- item one\n- item two"}' \
  $API/nodes/$NID/cards

# Add a checklist card
curl -s -H "X-API-Key: $KEY" \
  -d '{"kind":"checklist","title":"TODO","items":[{"done":false,"text":"ship it"}]}' \
  $API/nodes/$NID/cards

# Add a code card
curl -s -H "X-API-Key: $KEY" \
  -d '{"kind":"code","title":"snippet","lang":"rust","body":"fn main() {}"}' \
  $API/nodes/$NID/cards

# Add a card and color its title bar in one call (name, hex, or [r,g,b] all work)
curl -s -H "X-API-Key: $KEY" \
  -d '{"kind":"text","title":"Important","body":"read me","color":"red","size":[300,180]}' \
  $API/nodes/$NID/cards

# Find something
curl -s -H "X-API-Key: $KEY" "$API/search?q=agenda"

# Rename / retag a node
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"title":"Renamed","color":[59,130,246]}' $API/nodes/$NID

# Version history: list the snapshots and see the retention that governs them
curl -s -H "X-API-Key: $KEY" $API/history        # {count, keep, min_gap_mins, snapshots:[…]}

# Read back the one card you just wrote (rather than the whole basket)
curl -s -H "X-API-Key: $KEY" $API/nodes/$NID/cards/$CID

# Edit a card body
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"body":"updated text"}' $API/nodes/$NID/cards/1

# Move + recolor a card (spatial edits)
curl -s -X PATCH -H "X-API-Key: $KEY" \
  -d '{"pos":[360,40],"size":[300,220],"color":[34,197,94]}' $API/nodes/$NID/cards/1

# Group cards 1 and 2 into a container
curl -s -H "X-API-Key: $KEY" -d '{"cards":[1,2],"title":"Cluster"}' $API/nodes/$NID/groups

# Dock card 2 onto card 1 (they now move together)
curl -s -H "X-API-Key: $KEY" -d '{"anchor":1}' $API/nodes/$NID/cards/2/dock

# Convert card 1 to a checklist and fill it in one PATCH
curl -s -X PATCH -H "X-API-Key: $KEY" \
  -d '{"kind":"checklist","items":[{"done":false,"text":"first"}]}' $API/nodes/$NID/cards/1

# Several table ops in one call — applied in order. Building a styled table is
# inherently many small edits, so send them as an array rather than N requests.
curl -s -X POST -H "$K" -H 'Content-Type: application/json' \
  -d '[{"op":"set_header","header":true},
       {"op":"set_bg","row":0,"col":0,"color":"red"},
       {"op":"set_fg","row":1,"col":0,"color":"blue"},
       {"op":"autofit_cols"}]' \
  "$B/nodes/$NODE/cards/$CID/table"
# If one fails the response says which and how many already applied:
#   {"error":"table op 3/4 (set_bg) failed …; 2 earlier op(s) were applied"}
# **Check the status code.** curl exits 0 on a 400, so `curl` succeeding is not
# the same as the edit landing — use -f, or read the body.

# Table card: color a cell red, add a row, drop the header
curl -s -H "X-API-Key: $KEY" -d '{"op":"set_bg","row":0,"col":0,"color":"red"}' $API/nodes/$NID/cards/1/table
curl -s -H "X-API-Key: $KEY" -d '{"op":"insert_row","at":1}'                    $API/nodes/$NID/cards/1/table
curl -s -H "X-API-Key: $KEY" -d '{"op":"set_header","header":false}'            $API/nodes/$NID/cards/1/table

# Upload an image into an image card
curl -s -H "X-API-Key: $KEY" \
  -d "{\"name\":\"receipt.png\",\"data_base64\":\"$(base64 -w0 receipt.png)\"}" \
  $API/nodes/$NID/cards/1/images

# Add existing card 3 to group 1 (then it moves with the group)
curl -s -H "X-API-Key: $KEY" -d '{"group":1}' $API/nodes/$NID/cards/3/group

# Tidy the basket: arrange all its cards into a non-overlapping grid
curl -s -H "X-API-Key: $KEY" -X POST $API/nodes/$NID/autosort

# Draw a stroke on a sketch card (card 1)
curl -s -H "X-API-Key: $KEY" \
  -d '{"op":"add_stroke","color":"blue","width":3,"points":[[10,10],[40,60],[80,20]]}' \
  $API/nodes/$NID/cards/1/sketch

# Reorder a checklist (card 1): just send items in the new order
curl -s -X PATCH -H "X-API-Key: $KEY" \
  -d '{"items":[{"done":false,"text":"first"},{"done":true,"text":"second"}]}' \
  $API/nodes/$NID/cards/1

# Export the whole document to PDF and save it
curl -s -H "X-API-Key: $KEY" "$API/export?format=pdf" \
  | python3 -c 'import sys,json,base64;open("trellis.pdf","wb").write(base64.b64decode(json.load(sys.stdin)["base64"]))'

# Templates: build a Task / Local / Prod verification grid once, then reuse it.
# 1) Make a table card and set it up (header row + column titles).
CID=$(curl -s -H "X-API-Key: $KEY" -d '{"kind":"table","title":"Verify"}' \
      $API/nodes/$NID/cards | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
curl -s -H "X-API-Key: $KEY" -d '{"op":"set_header","header":true}' $API/nodes/$NID/cards/$CID/table
curl -s -H "X-API-Key: $KEY" -d '{"op":"set_cell","row":0,"col":0,"text":"Task"}'  $API/nodes/$NID/cards/$CID/table
curl -s -H "X-API-Key: $KEY" -d '{"op":"set_cell","row":0,"col":1,"text":"Local"}' $API/nodes/$NID/cards/$CID/table
curl -s -H "X-API-Key: $KEY" -d '{"op":"set_cell","row":0,"col":2,"text":"Prod"}'  $API/nodes/$NID/cards/$CID/table

# 2) Register that card as a reusable template; capture its index.
IDX=$(curl -s -H "X-API-Key: $KEY" -d "{\"node\":$NID,\"card\":$CID,\"title\":\"Local/Prod verify\"}" \
      $API/templates | python3 -c 'import sys,json;print(json.load(sys.stdin)["index"])')

# 3) Later — stamp a fresh copy into any basket; the response includes the new card.
#    Then fill/colour its cells (set_cell / set_bg green|red) as you verify each task.
curl -s -H "X-API-Key: $KEY" -d "{\"node\":$NID}" $API/templates/$IDX/insert

# 3b) The "template editor" flow. Registering stamped a master card into the
#     root-level Templates basket — find it, edit it, then re-snapshot the SAME
#     template in place (keeps its index + name). Every later insert now stamps
#     the new version. (Add a column, then update.)
curl -s -H "X-API-Key: $KEY" $API/templates      # master_node / master_card per template
curl -s -H "X-API-Key: $KEY" -d '{"op":"insert_col","at":3}' $API/nodes/$NID/cards/$CID/table
curl -s -H "X-API-Key: $KEY" -d "{\"node\":$NID,\"card\":$CID}" $API/templates/$IDX/update

# Build a readable table: fill it, size the columns to the text, then the card
# to the columns. Without the autofit every column stays 110px wide and the
# long cells are clipped, because cell text does not wrap.
CID=$(curl -s -H "X-API-Key: $KEY" -d '{"kind":"table","title":"Deploy verification",
      "rows":[["Host","Check","Result"],
              ["ALICE","crypto heartbeat","clean, no fatal errors since 12:53"],
              ["GATEWAY","memory","last OOM cycling was mid-June"]]}' \
      $API/nodes/$NID/cards | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
curl -s -H "X-API-Key: $KEY" -d '{"op":"autofit_cols"}' $API/nodes/$NID/cards/$CID/table
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"fit":true}' $API/nodes/$NID/cards/$CID
# Just one column (the others keep whatever width you gave them):
curl -s -H "X-API-Key: $KEY" -d '{"op":"autofit_cols","col":2}' $API/nodes/$NID/cards/$CID/table

# Chart a table: make the populated table and chart it in two calls.
CID=$(curl -s -H "X-API-Key: $KEY" -d '{"kind":"table","title":"Quarterly revenue",
      "rows":[["Quarter","Revenue","Costs"],["Q1","1200","800"],["Q2","1850","900"]],
      "size":[520,300]}' $API/nodes/$NID/cards | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
curl -s -X POST -H "X-API-Key: $KEY" -d '{"kind":"bar"}' $API/nodes/$NID/cards/$CID/chart
# Pie of one column (proportions of a whole; only positive values get a slice):
curl -s -X POST -H "X-API-Key: $KEY" \
     -d '{"kind":"pie","label_col":0,"value_cols":[1]}' $API/nodes/$NID/cards/$CID/chart
# Line chart of one column, with the grid kept visible underneath:
curl -s -X POST -H "X-API-Key: $KEY" \
     -d '{"kind":"line","value_cols":[1],"show_table":true}' $API/nodes/$NID/cards/$CID/chart
# Back to a plain table:
curl -s -X DELETE -H "X-API-Key: $KEY" $API/nodes/$NID/cards/$CID/chart

# Got templates from before the Templates basket existed? Give them master cards
# (creates the basket; only fills in what's missing, so it's safe to repeat).
curl -s -X POST -H "X-API-Key: $KEY" $API/templates/rebuild

# Move a card to another basket (group/dock membership is dropped).
curl -s -X POST -H "X-API-Key: $KEY" \
     -d '{"node":965,"pos":[40,40]}' $API/nodes/7/cards/3/move

# Mirror a file into a card: point it at the file, and the body tracks it.
CID=$(curl -s -X POST -H "$K" -H 'Content-Type: application/json' \
  -d '{"kind":"text","title":"README","source":"/srv/app/README.md","fit":true}' \
  "$B/nodes/$NODE/cards" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
# Read it back — `source` and `source_error` tell you whether the mirror is healthy.
curl -s -H "$K" "$B/nodes/$NODE/cards/$CID"
# The body is the file's, so editing it is refused (409) until you detach:
curl -s -X PATCH -H "$K" -H 'Content-Type: application/json' -d '{"source":""}' \
  "$B/nodes/$NODE/cards/$CID"

# React to what changes: long-poll for a revision, then ask what moved. Loop it,
# carrying `rev` into the next wait and `seq` into the next changes call.
curl -s -H "$K" "$B/wait?rev=$REV"
#   → {"rev":41,"changed":true,"epoch":6424652370349836957}
curl -s -H "$K" "$B/changes?since=$SEQ"
#   → {"epoch":…,"rev":41,"count":1,"truncated":false,"changes":[
#        {"seq":41,"ts":1785950176,"actor":"ui","entity":"card","op":"updated",
#         "id":4821,"node":62,"title":"Deploy checklist","fields":["property"],
#         "property":["status","done"]}]}
# Re-read only what's named — one card, not the document. If `epoch` differs from
# last time (the app restarted) or `truncated` is true, re-read and start over.

# Everything an agent did, this session:
curl -s -H "$K" "$B/changes?since=0&limit=5000" \
  | python3 -c 'import json,sys;[print(c["seq"],c["op"],c["entity"],c["id"],c.get("fields",[])) for c in json.load(sys.stdin)["changes"] if c["actor"]=="api"]'

# List the saved templates, or delete one by index.
curl -s -H "X-API-Key: $KEY" $API/templates
curl -s -X DELETE -H "X-API-Key: $KEY" $API/templates/$IDX
```

## Notes for agents collaborating on notes

- **Discover before writing:** `GET /api/tree` (structure) or `GET /api/search?q=`
  to find the right node instead of creating duplicates.
- **Placement:** the canvas is spatial. Give cards distinct `pos` values (e.g.
  step `x` by ~320 and `y` by ~200) so they don't overlap. Read a card's `pos`/
  `size` back from `GET /api/nodes/{id}` before repositioning.
- **Organize spatially:** use **groups** for a named, lasting cluster you drag as
  one box, or **docking** to stick a couple of related cards together. Either is
  reversible (`DELETE …/groups/{gid}` ungroups; `DELETE …/dock` detaches).
- **Handing off a snapshot:** `GET /api/export?format=pdf` (or `png`) returns the
  whole document as a base64 file — handy for sharing a rendered copy.
- **Deletes are destructive:** `DELETE /api/nodes/{id}` removes the entire
  subtree. Confirm the id first; there is no undo via the API.
- **Concurrency:** the app is the single writer — requests are applied one at a
  time on the UI thread, so there are no partial writes, but there is also no
  transaction across multiple calls. Read-modify-write can race with a human
  editing in the window; keep changes small and re-read if it matters.
- **Persistence:** you don't have to trigger a save. An API edit marks the
  document dirty and autosave writes it to disk **~2 seconds after the last
  change** (debounced, so a burst of calls saves once at the end), on a worker
  thread, even if the window is idle and unfocused. Each save also writes a
  version-history snapshot. If the user has turned autosave off in Settings, the
  change sits in memory until they save — there is no save endpoint. Check
  `unsaved_changes` in `GET /api/instance` if it matters.
- **Which document you're editing:** one instance serves one document, so the
  **port identifies the document**. Before writing to a box that runs several
  instances, call `GET /api/instance` and check `document`/`path`.
