# Trellis Agent API

A localhost HTTP API for reading and editing a Trellis document while it is open
in the app. Intended for agents (and the user) to collaborate on the same notes:
edits made through the API appear live in the window and are saved with the
document.

- **Base URL:** `http://127.0.0.1:<port>/api` — default port **7373**.
- **Bind address:** `127.0.0.1` only. Not reachable from other machines.
- **Format:** JSON request and response bodies; `Content-Type: application/json`.
- **State:** operates on the document currently open in the app. There is no
  multi-document addressing — whatever the user has open is what you edit.

## Enabling it

The API is **off until a key is set**. In the app: **Tools → Settings → Agent API**,
click **Generate** (or type a key), and **Copy** it. The key and port persist
across restarts. Changing the key takes effect immediately; changing the port
needs an app restart. If the port is busy, the Settings panel shows the bind
error and the API stays down.

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

**Card** — `kind` is one of `text`, `code`, `checklist`, `table`, `image`.
| field | applies to | notes |
|---|---|---|
| `id` | all | |
| `title` | all | shown in the card's title bar (all kinds, incl. `image`) |
| `kind` | all | `"text"` \| `"code"` \| `"checklist"` \| `"table"` \| `"image"`; PATCH can convert an existing card to another kind |
| `pos` | all | `[x,y]` top-left on the basket canvas |
| `size` | all | `[w,h]` in canvas units |
| `color` | all | title-bar accent — set as `[r,g,b]` (0–255), a hex string (`"#ef4444"`, `"#e44"`), or a name (`"red"`, `"green"`, `"blue"`, …) |
| `group` | all | group id this card belongs to, or null — set via the group sub-resource (below) |
| `docked_to` | all | id of the card this one is docked to, or null — set via the dock sub-resource |
| `body` | text, code | Markdown (text) or source (code) |
| `lang` | code | syntax-highlight language, e.g. `"rust"` |
| `items` | checklist | `[{ "done": bool, "text": string }]` |
| `image_name`, `image_names`, `bytes` | image | first/all image names + total byte count (read); set image bytes via the images sub-resource (below) |
| `rows`, `header` | table | `rows` set: `[["a","b"],…]` bulk-replaces cell text (colors reset); get: cells as `{text,bg,fg}`. `header` (bool) toggles the header row. Fine-grained edits (cell colors, widths, row/col ops) use the table sub-resource (below) |

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

### Read
```
GET /api/tree
  → 200 {"roots":[ {id,title,color,cards:<count>,children:[ …recursive… ]} ]}

GET /api/nodes
  → 200 {"nodes":[ {id,title,parent,children:[ids],cards:<count>} ]}

GET /api/nodes/{id}
  → 200 {id,title,parent,children:[ids],color,groups:[<group>…],cards:[<card>…]}   | 404

GET /api/nodes/{id}/cards
  → 200 {"cards":[<card>…]}                                      | 404

GET /api/search?q=<text>
  → 200 {"hits":[ {node,node_title,snippet} ]}                   (case-insensitive)
```
Note: `tree` and `nodes` report `cards` as a **count**; `GET /api/nodes/{id}`
returns the **full card objects**.

### Create
```
POST /api/nodes            {title, parent?}
  → 201 {"id":<new>}   | 400 if parent doesn't exist

POST /api/nodes/{id}/cards {kind?, title?, body?, lang?, items?, pos?, size?, color?, image_base64?}
  → 201 {"id":<new>}   | 404 if node doesn't exist
```
`kind` defaults to `"text"` and may be any of `text`, `code`, `checklist`,
`table` (starts as an empty 3×3), or `image`. `pos` is `[x,y]` canvas coordinates
(default `[40,40]`); pass distinct positions to avoid stacking cards on top of
each other. `size` is `[w,h]`. `color` sets the title-bar accent at creation (see
the accepted formats below). `items` is used only for `checklist`; `lang` only
for `code`. `image_base64` gives an `image` card its first image (base64 file
bytes; the `title` becomes its name).

### Update
```
PATCH /api/nodes/{id}              {title?, color?}
  → 200 {"id":<id>}    | 404
        color: setting only (can't clear via API)

PATCH /api/nodes/{id}/cards/{cid}  {title?, body?, color?, kind?, lang?, pos?, size?, items?, rows?, header?}
  → 200 {<updated card>}   | 404
```
Every field is optional; only those present are changed. `pos`/`size` are
`[x,y]`/`[w,h]`; `lang` applies to code cards, `items` replaces a checklist's
items, `rows` bulk-replaces a table's cell text, `header` toggles a table's
header row. **`kind` converts the card to another kind** (`text`/`code`/
`checklist`/`table`/`image`) — apply it in the same PATCH as `items`/`rows`/etc.
and the new content lands in the converted card. The response is the full updated
card object.

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
POST /api/nodes/{id}/cards/{cid}/table  {op, …}
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
| `set_col_width` | `col`, `width` | set a column's pixel width |
| `set_header` | `header` | set the header-row flag (bool) |

### Images
Attach or remove image bytes on an `image` card (grid layout; first image is the
primary). Bytes are png/jpeg/gif/bmp/webp.
```
POST   /api/nodes/{id}/cards/{cid}/images        {data_base64, name?}
  → 201 {<updated card>}   | 400 (bad base64)  | 404 (not an image card)

DELETE /api/nodes/{id}/cards/{cid}/images/{idx}  → 200 {<updated card>}   | 404
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

# Export the whole document to PDF and save it
curl -s -H "X-API-Key: $KEY" "$API/export?format=pdf" \
  | python3 -c 'import sys,json,base64;open("trellis.pdf","wb").write(base64.b64decode(json.load(sys.stdin)["base64"]))'
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
- **Persistence:** API edits mark the document dirty and are written on save /
  autosave-on-exit, same as manual edits.
