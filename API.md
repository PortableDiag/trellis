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
| `rules` | table | conditional formatting — colour cells by value (read; set with the `set_rules` table op) |
| `source` | text, code, **table** | a file this card **mirrors**: `body` becomes a read-only live copy, refreshed while the document is open. Omitted when the card isn't mirroring |
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

**Coloured text works, including inside markdown tables.** The renderer is
patched to honour an inline colour span:

```html
<span style="color:#22c55e">PASS</span>
```

It nests with other inline markup (`<span style="color:#ef4444">**FAIL**</span>`
is bold red) and renders **inside table cells**, which is the cheapest way to get
a readable status column into a *text* card:

```md
| Check | Status |
|---|---|
| TLS  | <span style="color:#22c55e">PASS</span> |
| Auth | <span style="color:#ef4444">**FAIL**</span> |
```

Limits worth knowing before you reach for it:
- **Text colour only — no cell background.** Markdown has no concept of one. For
  a coloured *cell*, use a real **table card** and `set_bg` / `set_fg`.
- **Emoji are in colour on screen since v0.91.0**, so 🔴 and 🟢 are now a red
  circle and a green one and *are* usable as status indicators. Colour comes
  from an emoji font on the machine (Noto Color Emoji on Linux, Apple Color
  Emoji on macOS), painted over the laid-out text — **Settings → Canvas** names
  the file in use. Where no such font exists, notably **Windows** (Segoe UI
  Emoji stores colour as vector layers rather than bitmaps), emoji fall back to
  the monochrome outline shipped since v0.84.0 and two circles again look alike.
  **Exports (PDF/PNG) are still monochrome.** So for a status colour that is the
  same everywhere and for anyone opening the document, a colour span or a table
  card's cell colours remains the portable choice.

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

GET /api/cards/{cid}/link
  → 200 {card, node, node_path, document, link, link_verified, http}   | 404
        the canonical URL for this card — see **Links that open Trellis on a
        card**. Ask for it rather than assembling one: the port and the document
        name are the instance's, not something a client can know.

GET /open/card/{cid}   ·   GET /open/node/{id}        [no key; not under /api]
  → 200 {"opened":"card 1391"}   | 404 (no such target)  | 409 (?doc= mismatch)
        what a `trellis://` link resolves to. Navigation only — it focuses the
        window and reveals the target, and deliberately returns no document
        content, because it is the one route that answers without a key.

GET /api/cards/{cid}
  → 200 {node, node_title, node_path, card:{<card>}}              | 404
        find a card from its **id alone**, without already knowing its basket.
        Card ids are unique per document, so an id is a complete address — but
        every other card route is /nodes/{id}/cards/{cid}, so an id quoted in a
        note or read out of an earlier response could only be resolved by
        walking every basket. The owning node comes back with it, because every
        route that *edits* a card still needs the node.
        Node ids and card ids are separate spaces: the same number can name one
        of each, and this route always answers about the card.

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

POST /api/nodes/{id}/cards {kind?, title?, body?, lang?, items?, rows?, header?, pos?, z?, size?, color?, font_scale?, fit?, image_base64?, inline_images?, source?}
  → 201 {"id":<new>}   | 404 if node doesn't exist
```
`kind` defaults to `"text"` and may be any of `text`, `code`, `checklist`,
`table` (starts as an empty 3×3), `image`, or `sketch` (an empty draw surface). `pos` is `[x,y]` canvas coordinates
(default `[40,40]`); pass distinct positions to avoid stacking cards on top of
each other. `size` is `[w,h]`. **`z`** is depth in the **same units as `pos`** — positive is
toward the viewer, so `z: 200` is as far *forward* as `pos` `+200` is to the
right. See [Depth and time](#depth-and-time) before using it. `color` sets the title-bar accent at creation (see
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

PATCH /api/nodes/{id}/cards/{cid}  {title?, body?, color?, kind?, font_scale?, fit?, lang?, pos?, z?, size?, items?, rows?, header?, inline_images?, source?}
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
The whole tree at once — every root and everything under it (**View → Collapse /
Expand the whole tree**). Always recursive:
```
POST /api/expand  {expanded:true|false}
    → 200 {"expanded":<bool>, "changed":<n>}
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

Since v0.102.0 the arguments in that table are **required**, and an unknown
field is a 400 naming it — the same rule as every other endpoint since v0.86.0,
which this one had been missed by. Omitting an argument used to substitute a
default silently: `set_cell` with no `text` wrote an empty string over the cell
and answered 200, no `row`/`col` wrote over `0,0`, and `remove_row` with no `at`
deleted the first row. `autofit_cols`'s `col` is the one optional (absent = every
column), and an absent or null `color` still means *clear it*. In a batch the
whole list is checked before anything is applied, so a malformed op leaves the
table untouched rather than half-edited.

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

### Overlapping cards
`fit: true` sizes a card to its content — **width as well as height** — so a card
grown by an edit can end up covering its neighbour, with nothing to say so. This
is the check to run after a batch of edits, and the repair.
```
GET  /api/nodes/{id}/overlaps  → 200 {"node":<id>,"overlaps":[{"a":<cid>,"b":<cid>}, …]}
POST /api/nodes/{id}/overlaps  → 200 {"node":<id>,"moved":<n>}
```
The repair **keeps the layout**: every card's `x` is preserved, so columns
survive, and cards move down only far enough to stop overlapping, in the order
they already sat in. A basket with no overlaps is not touched (`moved: 0`).

This is not [autosort](#autosort), which throws the arrangement away and lays a
grid — the wrong tool for a basket someone arranged on purpose. Cards that travel
together (a group, a dock stack) are treated as one block and never reported
against each other.

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

### Card links — `[[#id]]`
A link works in a card's **body**, in a **table cell**, and (since v0.103.0) in a
card's **title** — which is where the diagram recipe puts one, to tie a figure to
the script that drew it. Titles were already read by the backlink index, so a
title link counted as a link and simply could not be followed; now it renders and
clicks like any other.

`[[Some Basket]]` and `[[42]]` link to a **basket**, as they always have.
**`[[#1391]]` links to a card.** The `#` prefix is how card ids are written
everywhere else (the docs, the Ctrl+O palette), so it reads the way it is spoken.

Card links matter most in a journal-shaped document: every card written on one day
shares a basket, so `[[Tuesday 8/11/2026]]` names the day, not the thing that
happened in it. `[[#1391]]` names the thing.

```
GET /api/cards/{cid}/backlinks
  → 200 {"card":1391,"node":63,"hits":[{node,card,node_title,node_path,snippet}]}
  | 404 (card not found)
```

A link written in a **table cell** counts for backlinks (it always has — cell text
is scanned), though it is not clickable there; links render as clickable in text
card bodies.

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

**Code is not read** (since v0.96.0). A `key:: value` inside an inline
`` `code span` `` or a fenced ```` ``` ```` block is text about a property, not a
property — so a card that *documents* the syntax no longer acquires a due date
and appear on the agenda. Everywhere else is unchanged: a property may sit
mid-sentence, at the end of a checklist item, or on its own line. A **`code`
card** is left alone entirely, because its whole body is code and one may still
be tracked with `status:: done`.
```
GET /api/properties                    → 200 {"properties":[{"key":"due","count":4}, …]}
GET /api/properties?key=due            → 200 {"key":"due","value":null,"hits":[{node,card,node_title,snippet}, …]}
GET /api/properties?key=status&value=open → 200 hits where status == open
```
Set a property on a card (rewrites the `key:: …` line in its body, or appends
one) — e.g. to move a card on the Kanban board:
```
POST /api/nodes/{id}/cards/{cid}/property  {key, value}
DELETE /api/nodes/{id}/cards/{cid}/property?key=<key>
  → 200 {"cleared":true|false,"key":"due"}          | 400 (no key) | 404 (card not found)
        Removes the whole `key:: value` line. `cleared:false` means the card
        never had it — not an error. **Setting a property to "" is not the
        same**: that leaves `due::` present but unparseable, so the task stays
        on the agenda under "No date" instead of leaving it.
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

### Depth and time — the hypercube

**Trellis** is a tree of baskets. A **basket** is the space: `x` and `y` always,
**`z`** when Depth is on, and a **time** axis when Time is on — at which point a
basket is a **hypercube**, and that is the word the app uses for the pair.

Two things the name does *not* mean, because getting this wrong makes the model
harder to explain than it is:

- **The tree is not a dimension.** It is the index *over* baskets — discrete
  containers, not an axis anything moves along. Nodes add no axes.
- **A basket is not "a trellis".** The trellis is the whole lattice: tree *and*
  baskets.

Both axes are **off by default** (canvas group *Hypercube: Depth · Time*, beside
Dock and Snap; also **View → Hypercube**), and both are **view** settings — the
data is on the card either way.

**Alt+drag looks around.** With Depth on, Alt+drag moves the eye: `z = 0` stays
put while near and far cards swing opposite ways. That parallax is what makes
depth legible on a flat screen — a static perspective just looks like "some cards
are bigger". *Reset view* returns to straight on; the angle is per basket.

**`z` — depth.** Same units as `pos`; positive is toward the viewer. Clamped to
`[-1600, 1200]`. With Depth **on** a card is projected through a camera: nearer
cards are larger and cover further ones, and a click lands on the nearest. With
Depth **off** `z` is simply the stacking order, so it is never meaningless — and
it is never discarded, so turning the toggle off cannot cost an arrangement.

```sh
curl -s -X POST -H "$KEY" -H 'Content-Type: application/json' \
  -d '{"kind":"text","title":"In front","body":"…","pos":[200,160],"z":400}' \
  $API/nodes/$NID/cards
curl -s -X PATCH -H "$KEY" -d '{"z":-300}' $API/nodes/$NID/cards/$CID   # push it back
```

**The reader may have Depth off.** So `z` is for **arrangement**; anything that
carries *meaning* still belongs in the text, a `#tag` or a `key:: value`. This is
the same trap as using emoji for status: don't put the message where the reader
may not be looking.

**The overlap check changes shape too.** The pairwise AABB pass recommended after
a batch of `fit: true` edits is 2-D: two cards sharing x/y at different `z` are
*not* colliding, so compare depth as well or it reports collisions that aren't.

**Time — a card is present on every day it spans.** With Time on, a journal day
also shows cards from *other days* whose `start::`→`due::` span contains it — the
**same card**, not a copy, drawn as a projection that names where it lives and
takes you there when clicked. Nothing new to author: it is the `start::` span
from v0.90.0, read as extent.

Two deliberate limits, both learned by running it against a real document:

- **Containment, not the agenda rule.** `/api/tasks` keeps a missed deadline live
  on every later day, which is right for a list of work and wrong for a day: it
  would fill every day with every overdue task in the document.
- **Only from other days.** A card's position means something inside its own
  basket and nothing outside it, so only cards living in a *day* are projected —
  they share the journal's coordinate space. A task in a project basket belongs
  to the Agenda, which is what `/api/tasks` is for.

### Diagrams — a figure is an image card plus its script

**You can already do this**; nothing new is needed in the API. Render a PNG
locally, create an `image` card, and POST the bytes:

```sh
# 1. the card
CID=$(curl -s -X POST -H "X-API-Key: $KEY" -H 'Content-Type: application/json' \
  -d '{"kind":"image","title":"How the sync loop retries","pos":[40,40],"size":[900,540]}' \
  $API/nodes/$NID/cards | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')

# 2. its bytes (base64 in a JSON body — use a file, not a shell argument)
python3 -c "import base64,json;print(json.dumps({'data_base64':base64.b64encode(open('fig.png','rb').read()).decode(),'name':'fig.png'}))" > img.json
curl -s -X POST -H "X-API-Key: $KEY" -H 'Content-Type: application/json' \
  --data-binary @img.json $API/nodes/$NID/cards/$CID/images

# 3. the source, beside it, linked — an image alone cannot be edited
curl -s -X POST -H "X-API-Key: $KEY" -H 'Content-Type: application/json' \
  -d "{\"kind\":\"code\",\"lang\":\"python\",\"title\":\"Source — figure [[#$CID]]\",\"body\":\"…\",\"fit\":true}" \
  $API/nodes/$NID/cards
```

**Both kinds of diagram are welcome, and they are the same mechanism** — a script
that writes a PNG. What differs is which script you write:

- **Auto-layout** (Graphviz, Mermaid, any DOT renderer) when the content really is
  *what connects to what*: a state machine, a dependency graph, a call tree. The
  algorithm places the nodes; you get a correct picture cheaply.
- **Composed** (Pillow, or any drawing library, with coordinates you choose) when
  the *arrangement itself carries meaning* — a timeline, a before/after, a layered
  architecture, anything where "these two are side by side" or "this spans those"
  is the point. An auto-layout engine cannot know that, so it will place things
  in an order that is merely legal.

A worked, commented example of the composed kind — the one that produced *The
4th dimension* figure — is in **Build & Test Harness → *Diagram recipe***. Its
header states the method: hand-placed geometry, a palette where every colour
means something, four type sizes, and a caption that states the failure rather
than only the happy path.

**Always keep the script.** An image card is pixels; the script is the thing that
can be corrected next month. Post it as a `code` card in the same basket and link
the two with `[[#id]]` so each shows up in the other's backlinks.

### Links that open Trellis on a card

An agent can hand over a **link**, not just an id. Clicking it opens the running
instance on that exact card.

```
trellis://<port>/card/<cid>               trellis://7374/card/1391
trellis://<port>/node/<id>                trellis://7373/node/63
trellis://<port>/card/<cid>?doc=<file>    optional, verified on arrival
http://127.0.0.1:<port>/open/card/<cid>   the same thing, no registration needed
```

**Never build one by hand — ask for it:**

```sh
curl -s -H "X-API-Key: $KEY" $API/cards/1391/link
# → {"card":1391,"node":63,"node_path":"Trellis › Trellis Open Items",
#    "document":"Personal.ron",
#    "link":"trellis://7374/card/1391",
#    "link_verified":"trellis://7374/card/1391?doc=Personal.ron",
#    "http":"http://127.0.0.1:7374/open/card/1391"}
```

**The port is the address**, because one instance serves one document — which is
what makes this work with several instances running at once (two documents plus a
development build is the normal case here). A link goes to the instance on that
port and nowhere else.

**`doc=` is optional and is a *check*, not a lookup.** Given, the instance refuses
with `409` if it is serving a different document; omitted, the port is taken at
its word. Worth including in anything durable — a session report, a card, a chat
message — because **card ids are unique within a document, not across documents**:
`1365` is a real card in *both* of this operator's documents, so a link aimed at
the wrong port lands on a real card that is not the one meant.

**`/open/...` is unauthenticated and navigation-only.** It focuses a window and
answers `{"opened":…}` or a 404; it never returns document content, because a
route with no key that could read cards by walking ids would be a hole. It sits
deliberately outside `/api`.

**Registration.** `trellis://` needs the desktop to know the scheme. Trellis
registers itself on a new install, and again if the binary moves; *Settings →
Agent API → Register now* and *Tools → Register trellis:// links…* are the
explicit controls. It will **not** overwrite a working registration, so a
development build cannot hijack the handler. The `http://127.0.0.1` form needs
none of this and works today — which is why it exists.

**A date property stops at the date.** `due`, `start` and `date` take the first
token of their value, because a date has no spaces. Write
`due:: 2026-08-15 — still blocked` and the date is `2026-08-15`; the prose after
it is ignored rather than swallowing the value. Before v0.94.0 the whole tail was
the value, which did not parse as a date, so the task lost its deadline silently
and was filed under *No date*. Other properties keep their spaces —
`status:: in progress` is one value.

### Tracking work — read this before creating task cards

**One task is one card, and it never moves and is never copied.** This is the
single most important convention in the API, and the one agents get wrong.

The failure looks reasonable while you are doing it: you keep a "today's tasks"
card and copy it forward to tomorrow, or you stamp a fresh checklist into each
new basket. What you have then built is **N cards that Trellis reads as N
separate tasks**, each with its own `status::` and `due::`. The Agenda and Kanban
show the same work several times, no card is authoritative, and "what is actually
outstanding" stops having an answer. Nothing warns you.

Do this instead:

```sh
# 1. Create the task ONCE, in the basket that owns the work.
#    The properties are ordinary text in the body. The `::` needs a trailing
#    space, or it is not parsed as a property.
curl -s -H "X-API-Key: $KEY" -H 'Content-Type: application/json' \
  -d '{"kind":"text","title":"Migrate the service to the new host",
       "body":"status:: todo\ndue:: 2026-08-15\n#infra\n\nContext, links, whatever."}' \
  $API/nodes/$NID/cards

# 2. Change its state in place. Never create a second card for the same work.
curl -s -X POST -H "X-API-Key: $KEY" -d '{"key":"status","value":"doing"}' \
  $API/nodes/$NID/cards/$CID/property
curl -s -X POST -H "X-API-Key: $KEY" -d '{"key":"due","value":"2026-08-18"}' \
  $API/nodes/$NID/cards/$CID/property     # slipped a day? edit the date, don't copy

# 2b. Drop a date entirely. DELETE removes the `due::` line; setting it to ""
#     would leave the property there, unreadable, and the card would sit under
#     "No date" instead of leaving the agenda.
curl -s -X DELETE -H "X-API-Key: $KEY" \
  "$API/nodes/$NID/cards/$CID/property?key=due"

# 3. Read the whole picture back from the views, not from a list you maintain.
curl -s -H "X-API-Key: $KEY" $API/tasks             # bucketed by due date
curl -s -H "X-API-Key: $KEY" $API/kanban            # grouped by status
curl -s -H "X-API-Key: $KEY" "$API/tasks?project=$PROJECT"   # one project only
```

**The views are the daily list.** `GET /api/tasks` already answers "what is due
today, what is overdue, what is coming" across every basket in the document —
that is what replaces the card you were tempted to copy. If you want a written
record of *what happened* on a given day, that is a journal entry (see
[Daily notes](#daily-notes)), which is a different thing from a task and should
not carry `due::`.

### A long task list is ONE card, not many

**A checklist item carrying its own `due::` is its own task.** This is how you keep
twenty live tasks without adding twenty cards — the unit of work is the *line*, and
the card is just the container:

```sh
curl -s -H "X-API-Key: $KEY" -H 'Content-Type: application/json' -d '{
 "kind":"checklist","title":"Release week","fit":true,
 "items":[
  {"done":false,"text":"Drop the legacy path  start:: 2026-08-11  due:: 2026-08-15"},
  {"done":false,"text":"Benchmark the two options  due:: 2026-08-15"},
  {"done":true, "text":"Default bumped  due:: 2026-08-12"}
 ]}' $API/nodes/$NID/cards
```

Each dated line becomes its own row on the Agenda and the Kanban, with its own
date and its own done state. **The checkbox is the done signal** — ticking it is
enough, and `status:: done` on the line says the same thing.

A checklist whose items carry dates is **not** also listed as one task in its own
right, so a list never double-counts itself. A checklist with no dated items keeps
behaving exactly as before: the card is the task.

**Every item has a stable `id`**, returned by `GET` on the card and by `/api/tasks`.
Address the line, not its position:

```sh
POST   /api/nodes/{id}/cards/{cid}/items/{item}/property {key, value}
DELETE /api/nodes/{id}/cards/{cid}/items/{item}/property?key=due
POST   /api/nodes/{id}/cards/{cid}/items/{item}/done     {done}
```

Reading a card back gives you the ids to use:

```sh
curl -s -H "X-API-Key: $KEY" $API/nodes/$NID/cards/$CID
# → {"kind":"checklist","items":[{"id":60,"done":false,"text":"… due:: 2026-08-15"}, …]}

# Slip one line's date; its siblings are untouched.
curl -s -X POST -H "X-API-Key: $KEY" -d '{"key":"due","value":"2026-08-20"}' \
  $API/nodes/$NID/cards/$CID/items/60/property

# Tick it.
curl -s -X POST -H "X-API-Key: $KEY" -d '{"done":true}' \
  $API/nodes/$NID/cards/$CID/items/60/done
```

**Why ids matter:** an item used to be identified by its position, so reordering a
list silently renamed every task in it. Ids are what let a line be linked to,
rescheduled, and followed over time.

**A wholesale `PATCH {"items":[…]}` reads the array two ways, decided by the
payload as a whole:**

- **You send ids back** (the natural read-modify-write: GET the card, edit the
  array, PATCH it) — each id names its line, so **reordering and deleting from
  the middle keep every survivor's identity**. A line with no id is new.
- **You send no ids at all** — ids carry across **by position**, so the ordinary
  edits an older client makes (change text, tick a box, append) still preserve
  identity.

Never mix the two in one payload expecting both: the rule is chosen once per
request, because a new line inheriting a position's id while another line claims
that id explicitly would hand one identity to two lines.

### `start::` — a task that spans days

`start:: 2026-08-11  due:: 2026-08-15` means the work is **in flight for those five
days**, not just due on the last one. A started task reads as **today** on the
Agenda every day until it is done or overdue — so multi-day work stays visible
instead of hiding under a future date until it is already late.

`/api/tasks` returns `start` and `live_today` alongside `due` and `bucket`. A task
with no `start::` behaves exactly as it always has.

**Sub-steps belong inside the task card**, as a checklist, not as separate cards —
unless a sub-step has a real due date of its own, in which case make it a card and
link the two with `[[#id]]` so each shows up in the other's backlinks.

**Before creating a task card, check whether it already exists.**
`GET /api/query?q=...` or `GET /api/search?q=...` costs one call and is the
difference between updating the task and duplicating it.

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

**`node_path` is the root-to-basket breadcrumb** (`Newsletter › Open
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

### Daily notes

A journal node for one calendar day, created on demand. **Opt-in and per
instance**: it does nothing until a *journal root* is chosen in
**Tools → Settings → Daily notes**, so a work document can keep a journal while a
personal one never grows one. Nothing dated is ever created any other way —
ordinary node creation knows nothing about journals.

```
POST /api/daily            {date?}      date = "YYYY-MM-DD"; omitted = today
  → 200 {"node":976,"created":false,"title":"Tuesday 8/11/2026","path":"2026 › August › Tuesday 8/11/2026"}
  | 400 (date isn't a real calendar day)
  | 404 {"error":"daily notes are off for this instance — Tools → Settings → Daily notes"}

GET    /api/daily          → 200 {"enabled":true,"root":5,"root_title":"2026","root_path":"2026"}
POST   /api/daily/root     {node}       turn it on / move the journal root
DELETE /api/daily/root                  turn it off (nothing is deleted)
```

`POST`, not `GET`, because it creates the node when the day's is not there yet.
`created` says which happened, so an agent can tell "I opened today's note" from
"I started it".

**Pass `date` rather than building a title yourself.** Writing
`"Wednesday 8/12/2026"` by hand is how a journal ends up with two nodes for one
day: the next writer spells it `08/12` or misses the weekday, and nothing
notices. Hand the endpoint a date and it does the matching.

`GET /api/daily` and the two `root` routes are the API half of **Tools → Settings
→ Daily notes**, so an agent can see whether the feature is on for this instance,
and switch it on, exactly as a person can.

The structure is `<root> → <month> → <day>`, with the root being the year. Two
behaviours worth knowing:

- **A day is matched by the date its title parses to, not by string.** A journal
  kept by hand drifts — `8/11/2026` beside `6/09/2026`, a misspelled weekday,
  dashes instead of slashes — and all of those resolve to the same day. This is
  what stops a second node appearing for a day that already exists. New nodes are
  written `Tuesday 8/11/2026`.
- **A new year becomes a sibling of the old root, not a child**, and the stored
  root follows it, so January does not end up nested inside last year.

Today's note is where you record *what happened*. It is not where tasks live —
see [Tracking work](#tracking-work--read-this-before-creating-task-cards). A task
is one card with `status::`/`due::` that stays in its own basket; copying it into
a daily note creates a second task.

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
insert. Since v0.101.0 the app **says so**: a master that no longer matches its
snapshot is marked **✎ edited** on its title bar, and its right-click menu offers
*Update template “name” from this card* directly.

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
Point a **text**, **code** or **table** card at a file and it becomes a
**read-only live copy**, re-read while the document is open (checked every ~3 s;
only files whose modification time changed are re-read).

**A table card mirrors a CSV/TSV into its cells** — live data with real cell
colours and column widths, which a markdown table cannot do. The delimiter comes
from the extension: `.tsv`/`.tab` are tab-separated, anything else is CSV.

```
POST /api/nodes/{id}/cards {"kind":"table","source":"/srv/metrics.csv","header":true}
```

A refresh replaces **cell text only** — column widths, the header flag, the chart
spec and the formatting rules all survive, so a table you sized and coloured stays
that way while the data moves under it.

**Conditional formatting** — colour cells by what they contain:
```
POST /api/nodes/{id}/cards/{cid}/table {"op":"set_rules","rules":[…]}
```
| key | |
|---|---|
| `col` | column index to test; omit for every column |
| `when` | `gt` `lt` `ge` `le` `eq` `ne` `contains` `empty` `not_empty` |
| `value` | compared against — a **number or string**; numbers use the decorated parser (`1,234.5`, `$12`, `40%`, `(3)` = −3) |
| `bg` / `fg` | `[r,g,b]`, hex or a colour name |

**First matching rule wins**, so send them most-specific first. A cell matching no
rule is **cleared**, which is why a value that stops being an error loses its red.
Header rows are never coloured — a header is a label, not a value. A non-numeric
cell never matches an ordering rule (`gt`/`lt`/…), so blanks and text don't get
coloured as though they were zero. `"rules": []` clears the formatting.
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

## Scoped tokens

Besides the instance key, the API accepts **scoped tokens**. They are ordinary
credentials — same `X-API-Key` (or `Authorization: Bearer`) header — but limited,
and the limit is enforced here rather than trusted. There are two kinds, minted
in different places and revoked independently:

| prefix | minted in | held by |
|---|---|---|
| `agent_` | *Settings → Agent API → Agent tokens* | an agent or service elsewhere, named after it |
| `plug_` | *Tools → Plugins…*, on approving a plugin | a plugin on this machine; never shown to you |

Both carry the same two limits:

- **Read-only** — refused on anything but `GET`, with
  `403 {"error":"'<name>' has read-only access to this document"}`. Checked
  before the request reaches the document.
- **Confined to a subtree** — may only act on that node and its descendants. A
  request that names no node is **refused** rather than allowed through, except
  for the structural reads needed to orient (`/api/health`, `/api/instance`,
  `/api/tree`, `/api/nodes` — titles and shape, never card content).
  `403 {"error":"outside the basket this token was given access to"}`.

**A subtree token cannot mirror files.** `source` on a card is refused with
`403 {"error":"a token confined to a basket cannot mirror files"}`, regardless of
*Files agents may mirror* — otherwise the token could point a card in its own
basket at any readable file and fetch the contents back, which would make the
confinement meaningless.

**A subtree token cannot use the whole-document query surfaces.** `/api/search`,
`/api/tasks`, `/api/kanban`, `/api/graph`, `/api/tags` and `/api/query` name no
node, so they are refused. That is the point — those are exactly the calls that
read everything — but it means an agent given a basket of its own sees only what
is in that basket. Give it the instance key or a whole-document token if it needs
the agenda, and understand that this is the same as letting it read everything.

Revoking is immediate and affects only that token; the instance key and every
other token keep working.

## Errors

All errors return `{"error":"<message>"}` with the status code:

| code | meaning |
|---|---|
| 400 | bad JSON body, **unknown field**, bad id, or missing parent |
| 401 | wrong API key |
| 403 | API disabled (no key set), or outside a scoped token's basket |
| 404 | node/card not found, or unknown route |
| 503 | app not accepting requests |
| 504 | app didn't respond in time (window busy/hung) |

### Unknown fields are rejected (since v0.86.0)

A field the API doesn't know is a **400 that names it**, rather than a 200 that
quietly ignores it:

```
PATCH /api/nodes/1/cards/2   {"x": 10, "y": 20}
→ 400 {"error":"invalid JSON body: unknown field `x`, expected one of `title`,
   `body`, `color`, `lang`, `pos`, `size`, `items`, `rows`, `kind`, `header`,
   `font_scale`, `inline_images`, `fit`, `source`"}
```

The error lists every field that body accepts, so a typo or a wrong guess tells
you the right name immediately. (`pos` is the one above — a card's position is
`"pos": [x, y]`, not `x`/`y`.)

Before this, such a request returned **200** and did nothing, which is
indistinguishable from success. If you are writing a client, this is the change
most likely to surface a bug you already had.

## Examples

### Working under an agent token

If the user issued you a token (*Settings → Agent API → Agent tokens*) rather
than the instance key, you are confined to one basket. Find it first — the
structural reads are the only whole-instance calls you have:

```sh
TOKEN=agent_…                       # yours; `plug_…` if you are a plugin
API=http://127.0.0.1:7374/api

# Which document, and what shape is it? Both allowed at any scope.
curl -s -H "X-API-Key: $TOKEN" $API/instance
curl -s -H "X-API-Key: $TOKEN" $API/tree

# Your basket is normally the one named after you. Everything you do goes in it.
MINE=$(curl -s -H "X-API-Key: $TOKEN" $API/tree \
       | python3 -c 'import sys,json
t=json.load(sys.stdin)
print(next(r["id"] for r in t["roots"] if r["title"]=="SCOUT"))')

curl -s -H "X-API-Key: $TOKEN" -H 'Content-Type: application/json' \
  -d '{"kind":"text","title":"Nightly check","body":"status:: done\ndue:: 2026-08-20"}' \
  $API/nodes/$MINE/cards

# These are refused (403) — they name no basket, so they would read everything:
#   /search  /tasks  /kanban  /graph  /tags  /query
# So is `source` (mirroring a file), and any node outside your subtree.
```

**Check the status code, not `curl`'s exit code** — `curl` exits 0 on a 403 or a
400, so a refused write looks like a successful one otherwise. Add `-w '%{http_code}'`
or `-f`.

### Everything else

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

# A live table from a CSV, coloured by value. Rules re-apply on every refresh,
# and column widths survive it — so size it once and it stays sized.
CID=$(curl -s -X POST -H "$K" -H 'Content-Type: application/json' \
  -d '{"kind":"table","title":"Live metrics","source":"/srv/metrics.csv","header":true,"fit":true}' \
  "$B/nodes/$NODE/cards" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
curl -s -X POST -H "$K" -H 'Content-Type: application/json' -d '[
  {"op":"set_rules","rules":[
    {"col":1,"when":"gt","value":1000,"bg":"red","fg":"white"},
    {"col":1,"when":"gt","value":100,"bg":[232,163,61]},
    {"col":1,"when":"le","value":100,"bg":"green"},
    {"col":2,"when":"eq","value":"DEGRADED","bg":"red","fg":"white"}]},
  {"op":"autofit_cols"}]' "$B/nodes/$NODE/cards/$CID/table"

# Coloured status text inside a markdown table (text card — no cell backgrounds;
# for those use a table card + set_bg). Since v0.91.0 emoji do render in colour
# on screen, but not in exports and not without a colour emoji font (Windows),
# so a span is still the choice that looks the same everywhere.
curl -s -X POST -H "$K" -H 'Content-Type: application/json' -d '{
  "kind":"text","title":"Checks","fit":true,
  "body":"| Check | Status |\n|---|---|\n| TLS | <span style=\"color:#22c55e\">PASS</span> |\n| Auth | <span style=\"color:#ef4444\">**FAIL**</span> |"
}' "$B/nodes/$NODE/cards"

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

# Resolve a card id you were handed — a note, an earlier response, a colleague —
# without knowing which basket it is in. The owning node comes back with it, so
# the next call can be an edit.
curl -s -H "X-API-Key: $KEY" $API/cards/1391
# → {"node":63,"node_title":"Trellis Open Items",
#    "node_path":"Trellis › Trellis Open Items","card":{"id":1391,…}}
NID=$(curl -s -H "X-API-Key: $KEY" $API/cards/1391 | python3 -c 'import json,sys;print(json.load(sys.stdin)["node"])')

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
# Every argument the op table lists is REQUIRED, and an unknown field is a 400
# naming it. There are no silent defaults: a `set_cell` with no `text` used to
# blank the cell and answer 200, and a `remove_row` with no `at` deleted row 0.
# A batch is checked in full before anything is applied.
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

# ...but on a basket someone arranged on purpose, autosort destroys the layout.
# Ask what actually collides, then repair it in place: `x` is preserved and cards
# only ever move down. Run this after any batch of `fit: true` edits, because fit
# grows a card's WIDTH as well as its height.
curl -s -H "X-API-Key: $KEY" $API/nodes/$NID/overlaps
# → {"node":63,"overlaps":[{"a":1391,"b":1402}, …]}
curl -s -H "X-API-Key: $KEY" -X POST $API/nodes/$NID/overlaps
# → {"node":63,"moved":4}          ("moved":0 = nothing was covering anything)

# Fold the whole tree, or open it (View → Collapse / Expand the whole tree)
curl -s -H "X-API-Key: $KEY" -X POST -d '{"expanded":false}' $API/expand
# → {"expanded":false,"changed":242}

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
              ["HOST-1","heartbeat","clean, no fatal errors since 12:53"],
              ["HOST-2","memory","last OOM cycling was mid-June"]]}' \
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
