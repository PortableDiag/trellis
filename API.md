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

**Card** — `kind` is one of `text`, `code`, `checklist`, `image`.
| field | applies to | notes |
|---|---|---|
| `id` | all | |
| `title` | all | |
| `kind` | all | `"text"` \| `"code"` \| `"checklist"` \| `"image"` |
| `body` | text, code | Markdown (text) or source (code) |
| `lang` | code | syntax-highlight language, e.g. `"rust"` |
| `items` | checklist | `[{ "done": bool, "text": string }]` |
| `image_name`, `bytes` | image | image bytes can't be set via the API |

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
  → 200 {id,title,parent,children:[ids],color,cards:[<card>…]}   | 404

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

POST /api/nodes/{id}/cards {kind?, title?, body?, lang?, items?, pos?}
  → 201 {"id":<new>}   | 404 if node doesn't exist
```
`kind` defaults to `"text"`. `pos` is `[x,y]` canvas coordinates (default
`[40,40]`); pass distinct positions to avoid stacking cards on top of each other.
`items` is used only for `checklist`; `lang` only for `code`.

### Update
```
PATCH /api/nodes/{id}              {title?, color?}
  → 200 {"id":<id>}    | 404
        color is [r,g,b]; setting only (can't clear via API)

PATCH /api/nodes/{id}/cards/{cid}  {title?, body?}
  → 200 {"id":<cid>}   | 404
```

### Delete
```
DELETE /api/nodes/{id}             → 200 {"deleted":<id>}    | 404   (removes the whole subtree)
DELETE /api/nodes/{id}/cards/{cid} → 200 {"deleted":<cid>}   | 404
```

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

# Find something
curl -s -H "X-API-Key: $KEY" "$API/search?q=agenda"

# Rename / retag a node
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"title":"Renamed","color":[59,130,246]}' $API/nodes/$NID

# Edit a card body
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"body":"updated text"}' $API/nodes/$NID/cards/1
```

## Notes for agents collaborating on notes

- **Discover before writing:** `GET /api/tree` (structure) or `GET /api/search?q=`
  to find the right node instead of creating duplicates.
- **Placement:** the canvas is spatial. Give cards distinct `pos` values (e.g.
  step `x` by ~320 and `y` by ~200) so they don't overlap.
- **Deletes are destructive:** `DELETE /api/nodes/{id}` removes the entire
  subtree. Confirm the id first; there is no undo via the API.
- **Concurrency:** the app is the single writer — requests are applied one at a
  time on the UI thread, so there are no partial writes, but there is also no
  transaction across multiple calls. Read-modify-write can race with a human
  editing in the window; keep changes small and re-read if it matters.
- **Persistence:** API edits mark the document dirty and are written on save /
  autosave-on-exit, same as manual edits.
