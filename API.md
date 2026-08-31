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
| `emphasis` | all | attention: `"glow"` (a steady accent halo) or `"pulse"` (the same halo breathing, 1.8s). Omitted when the card has none. **Separate from `color` on purpose** — the accent is how a person organises a basket, so borrowing it to shout destroys the organisation. There is no flash: above ~3 Hz it is a photosensitive-seizure risk |
| `emphasis_intensity` | all | halo strength `0.0`–`1.0` (clamped; default `1.0`) |
| `emphasis_until` | all | unix seconds after which it lapses. Present only when set; `emphasis_live` beside it says what is in force **now**, so a reader needn't consult its own clock |
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
| `source_tail` | text, code | **tail mode** — show only the last N lines of `source`, refreshed faster (≈0.6 s vs 3 s) and pinned to the bottom, for a file that *grows*. PATCH `0` to turn it off. **The 1 MB `source` limit does not apply to a tail**, because it seeks from the end instead of loading the file. Omitted when off |
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
         "lan_host":"192.168.0.101","lan_hosts":["192.168.0.101","100.64.100.6"],
         "nodes":42,"unsaved_changes":false,"stale_claims":0,
         "channels":2,"channels_waiting":1,"stale_plugins":0,"api_errors":0}
```
`document` is the file name (`"untitled"` for a never-saved document) and `path`
is its full path, or `null` when untitled. `nodes` is the document's node count.
Unlike `/api/health` this needs the key, since it reveals a file path.

`attachment_bytes` is the total size of every file embedded in the document — what
the attachments cost on each whole-document save, and otherwise invisible until a
backup gets slow.

`lan_host` is an address **another device on the network** can reach this instance
on, and `lan_hosts` is every candidate, best first. Both are `null`/empty when LAN
access is off, because then there honestly is no such address. They exist because
`port` alone is not enough to build a link for a phone: everything the app mints
says `127.0.0.1`, which on the phone *is* the phone — see `GET /go/…`.

They are a **hint, not an answer**. The addresses come from asking the routing
table (a UDP socket is `connect`ed and its own local address read back; no packet
is sent), which means a machine whose default route is a **VPN** reports the VPN
first unless a private-LAN probe finds something better — RFC 1918 is preferred
over CGNAT for exactly that reason. A box on two LANs and a VPN reports three
addresses and only the reader knows which network their phone is on, so anything
building links should let that be overridden.

**`stale_claims` is a warning about the document you are about to read.** It
counts the cards that state how something *is* and are past their own
[`verify::` date](#claims--which-stated-facts-are-out-of-date). Anything above
zero means part of this workspace is telling you something that was last
confirmed too long ago — read `GET /api/claims?expired=true` before you quote it
back to anyone. It rides on this endpoint because this is the call you already
make first.

**`channels_waiting` is somebody asking you a question.** `channels` counts the
[channel cards](#channels--a-card-that-is-a-conversation) in this document, and
`channels_waiting` counts those whose **last message came from the operator** —
somebody typed into a card and no agent has come back to it. Above zero, call
`GET /api/channels` — **unfiltered by name**: filtering by `?agent=<a name you
guessed>` is how a waiting message goes unread, because nothing ever told you
the name a channel was created with.

**The boundary is the project subtree, never a name.** A document can hold
several projects' workspaces, each with its own channel, and a waiting channel
belongs to the agent of the project whose subtree holds it. Answer only the
channels under *your* project's root — `?project=<its node id>` filters
structurally, or read `node_path` in the listing — and **report** another
project's waiting channel to the operator rather than draining it: a reply from
the wrong project's agent clears the flag under the agent the message was for.
On a document that holds a single project this collapses to the older rule (the
port is the boundary). Note `channels_waiting` is **document-wide**, so on a
shared document a positive count may be somebody else's channel — look before
concluding it is yours.

It is here for the same reason `stale_claims` is: **a channel only works if the
agent looks**, and until this shipped the only thing that made an agent look was
being told to in a prompt. A message sat unread for a day while the card it was
typed into worked perfectly — the transport was fine and the *noticing* was not.
No configuration, no plugin, and nothing to install: the count is on the call you
already make, so an agent that knows nothing about channels still finds out that
one is waiting.

Waiting is deliberately **identity-free** — "the operator spoke last", not "you
have not replied". This endpoint is scope-neutral and the instance key identifies
nobody, so there is no reliable *you* to compare against; and the case worth
catching is the one where nobody at all has answered.

**`stale_plugins` counts installed plugins whose release copy is newer.** A
plugin release does not install itself: plugins run from `<data-dir>/plugins/`,
the repo only *ships* them, so a release can be tagged and documented while
every instance keeps executing the old copy — with no symptom beyond a feature
that silently does nothing. Above zero, read [`GET /api/plugins`](#plugins) for
which ones. It rides here for the same reason `stale_claims` does: staleness
should be noticed at read-in, not after a feature quietly fails.

**`api_errors` counts the API calls this run has refused or failed** — every
response of 400 or above since the app started. It is the fourth read-in flag,
and it exists because the other three are about the *document* while this one
is about the *agents*: with several of them driving the API all day, the only
record of a refused call used to be the agent that made it, and an agent that
misreads a response leaves no trace at all. Above zero, read
[`GET /api/errors`](#what-failed-the-error-log) — which calls, by whom, and
what they were told — and mention what you find, because the operator has been
relying on you to.

### Plugins

The installed plugins, each compared against the release copy in the `plugins/`
directory of the checkout the running binary was built in (found from the
binary's own path; `source` is `null` — and nothing is ever stale — when the
binary does not run out of a checkout).

```
GET /api/plugins
  → 200 {"count":2,"stale":1,"source":"/path/to/repo/plugins","plugins":[
         {"name":"notify","title":"Notifications","version":"1.1.0",
          "available":"1.2.0","stale":true,"approved":true}, …]}
```

`available` is the release copy's version whenever the repo carries a plugin of
the same name — equal means current — and `null` for a plugin the repo does not
ship. `stale` is true only when the release copy is strictly *newer* (numeric,
per segment: `1.10` beats `1.9`); an installed copy ahead of the repo is a build
not yet released, not a problem. Only **installed** plugins are listed — a
release the operator never installed is a choice, not a gap.

**Reading the gap is an agent's job; closing it is the operator's.** There is
deliberately no update endpoint: updating replaces executable code, which is
exactly what the approval model exists to keep behind a human act. The operator
updates from **Tools → Plugins → Update**, which copies the release's code and
manifest over the installed copy and leaves its `config.json` and `state.json`
— credentials and state belong to the instance, not the release. Approval
survives an update, because a grant is keyed by plugin *name*, not file hash.

Scope-neutral, like `/api/instance`: app metadata with no document content, and
a confined agent should still be able to notice a stale plugin at read-in.

### Settings

The app's own settings — theme, canvas toggles, panels, notifications, retention
— so an agent can set a machine up, or put one back the way it was, without
anybody clicking. These are **instance** settings (per `--data-dir`), not
document settings: they live beside the key and the port, and are never written
into the `.ron`.

```
GET  /api/settings          → 200 {theme, tree_sort, minimap, dock_mode, snap_mode,
                                   grid_mode, depth_mode, cube_mode, time_mode, notify_digest, notify_agent,
                                   zoom_enabled, autosave, stick_windows, agenda_open,
                                   agenda_show_done, agenda_project, kanban_open,
                                   kanban_show_done, kanban_project, tags_open,
                                   find_open, backlinks_open, history_keep,
                                   history_gap_mins}
POST /api/settings  {…}     → 200 the settings as they now are   | 400
```

| setting | type | what it is |
|---|---|---|
| `theme` | string | `Trellis`, `Light`, `TerminalGreen`, `StickyNotes`, `Futuristic`, `SynthWave`, `Blueprint`, `Silkscreen`, `Phosphor` |
| `tree_sort` | string | `manual`, `name`, `name_desc`, `recent`, `tasks` — orders the **root** projects only |
| `minimap`, `dock_mode`, `snap_mode`, `zoom_enabled` | bool | canvas behaviour |
| `grid_mode` | bool | quantise a dragged or resized card to the canvas grid (32 world units, the step `draw_grid` paints). Independent of `snap_mode`, which **wins on any axis it claims** — only an axis no card edge aligned to is quantised |
| `depth_mode`, `time_mode` | bool | the two hypercube axes |
| `cube_mode` | bool | the compressed-workspace-cube **viewer** — same projection as Depth plus the reading gestures (click-to-isolate with *Go to card* / *View only this*, z flight on Ctrl+Shift+scroll / PageUp·PageDown, culling past the camera). A separate mode: setting it `true` turns `depth_mode` and `time_mode` off (and either of those turns it off), because its gestures claim inputs those modes use for editing. An agent that has built a cube basket switches the view on with this |
| `notify_digest`, `notify_agent` | bool | desktop notifications |
| `autosave` | bool | background saves; with it off a change sits dirty in memory |
| `stick_windows` | bool | detached Agenda/Kanban follow the main window |
| `agenda_open`, `kanban_open`, `tags_open`, `find_open`, `backlinks_open` | bool | which panels are showing |
| `agenda_show_done`, `kanban_show_done` | bool | include finished work |
| `agenda_project`, `kanban_project` | node id or `null` | scope a view to one project; `null` = all |
| `history_keep` | integer | snapshots kept (clamped 1–200) |
| `history_gap_mins` | integer | minimum minutes between snapshots (clamped ≤ 1440) |

**A patch is validated in full before any of it is applied**, so a bad third key
cannot leave the first two set. An **unknown name is a 400 that lists what was
expected**, and a known name with the wrong type is refused rather than coerced —
the same rule as every other input since v0.86.0. An empty object is refused too:
it is not a change.

**Deliberately not settable here:** the **API key, port and LAN flag**, and the
**file-mirroring policy**. A caller must not be able to widen its own reach — an
agent that could turn on LAN access, or point the mirror policy at `/`, would be
escalating with the credential you gave it for notes. Those stay in
**Tools → Settings**.

### Cube — a range of baskets as a volume

The cube **operation**: align several baskets' cards along z — each basket one
slice, first slice deepest, last nearest — and switch to [Cube
mode](#settings) to traverse them. Built for a run of journal days: *what did
each day look like*, flown through rather than opened one by one.

```
POST /api/cube      {"nodes": [<basket id>, …]}     (slice order = list order)
  → 200 {"cube_mode":true,"slices":N,"cards":M}
  | 400 (empty list, or the scene could not be built)
  | 404 (naming every id that is not a basket — the whole list is validated
         before anything changes)
DELETE /api/cube
  → 200 {"cube_mode":false}                          (leave the cube view)
```

The scene is **temporary and composed of `![[#id]]` embeds** — live views of
the real cards, each slice keeping its basket's own arrangement. Nothing is
created, copied or saved; the document is untouched, and closing the cube (or
*Go to card*, or turning Cube off) simply drops the view. In the app the same
operation is **right-click a basket → Open as cube…**, which offers a from/to
range over its child baskets.

Traversal, once open: click a card to **isolate** it (everything else ghosts)
with *Go to card* / *View only this*; **Ctrl+Shift+scroll** or
**PageUp/PageDown** fly the camera through the slices (one PageDown = one
slice); Esc steps back out. *Go to card* follows the slice's embed to the
original, in its real basket, in flat mode.

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

GET /open/card/{cid} · /open/node/{id} · /open/group/{gid}   [no key; not under /api]
  → 200 {"opened":"card 1391"}   | 404 (no such target)  | 409 (?doc= mismatch)
        what a `trellis://` link resolves to. Navigation only — it focuses the
        window and reveals the target, and deliberately returns no document
        content, because it is the one route that answers without a key.

GET /go/card/{cid} · /go/node/{id} · /go/group/{gid}          [no key; not under /api]
  → 200 <html>   | 404 (no such target)
        a **page that hands the reader's own device off** to `trellis://`, instead
        of moving this window. `/open/…` is for the machine Trellis is running on;
        `/go/…` is for the phone in your hand.

        It exists because of one external constraint, measured rather than assumed:
        **Telegram silently strips a link with a custom scheme.** A message
        containing `<a href="trellis://…">` is accepted with `ok:true` and arrives
        with **no link entity at all**; a bare `trellis://…` is not auto-linked
        either. Only `http(s)` survives. So a notification cannot carry a tappable
        link to a card without an `http` hop — and this is that hop.

        The page builds the `trellis://` URL from **`location.host`**, the one
        address known to be reachable from the device reading it. A link minted by
        the desktop says `127.0.0.1`, which on a phone is the phone. It offers a
        **link to tap, not a redirect**: an automatic jump to a custom scheme is
        what in-app browsers block, while a user gesture is the case they allow.

        Build one with `lan_host` from `GET /api/instance`:
        `http://192.168.0.101:7374/go/card/1391`.

GET /api/cards/{cid}
  → 200 {node, node_title, node_path, card:{<card>}}              | 404
        find a card from its **id alone**, without already knowing its basket.
        Card ids are unique per document, so an id is a complete address — but
        every other card route is /nodes/{id}/cards/{cid}, so an id quoted in a
        note or read out of an earlier response could only be resolved by
        walking every basket. The owning node comes back with it, so a client
        that wants the basket for its own reasons (a breadcrumb, a link) has it
        without a second call.
        **The card is under `card`**, not at the top level: `.body` of this
        response is nothing, `.card.body` is the text. An agent read the
        wrong level, appended its note to an empty string and PATCHed that
        back over a 28 KB message board (2026-08-28; restored from its own
        saved response). If you are reading a card only to add to it, do not
        read it at all — `POST /api/cards/{cid}/append` does that on the
        server (*Adding to a shared card without overwriting it*, under
        Examples).
        Node ids and card ids are separate spaces: the same number can name one
        of each, and this route always answers about the card.

A **list** of ids is an address too (v0.119.0), and unlike the batch routes it does
not care which baskets they are in:

```
GET  /api/cards?ids=1391,1392,1393
  → 200 {"count":N,"cards":[{node, node_path, card:{…}}], "missing":[ids]}
  | 400 (no ids=, or one that is not a number)
POST   /api/cards/property   {cards:[ids], key, value}
DELETE /api/cards/property   {cards:[ids], key}
  → 200 {"updated"|"cleared":N, "cards":[ids], "key":"status", "value":"done"}
  | 400 (empty list, or a POST with no value)  | 404 (an id in no basket — named)
```

Every whole-document query — `/api/tasks`, `/api/claims`, `/api/query`,
`/api/search`, `/api/properties`, `/api/tags` — hands back ids from **different
baskets**, which is exactly the list you then want to act on. The basket-addressed
batch validates against one basket, correctly, so *"mark these five done"* meant
grouping the ids by basket first at one lookup per card, to satisfy an argument the
caller never cared about.

**A missing id is fatal to the write and merely reported by the read.** `missing` on
the `GET` tells you precisely what you got; a partial write is the case where you
cannot tell how far it got, which is why one bad id refuses the whole thing.

**These are whole-document routes, so a token confined to a basket is refused** —
the same rule `/api/tasks`, `/api/search`, `/api/kanban` and `/api/query` already
follow, for the same reason: a route that names no basket cannot be checked against
one. A confined token still has `/api/nodes/{id}/cards/…` for its own basket. That
is a real cost, stated rather than glossed.

Since v0.117.0 the **writes take a bare card id too**, so an id is a complete
address for doing something and not only for looking it up:

PATCH  /api/cards/{cid}                    {…}      same body as the node form
DELETE /api/cards/{cid}
POST   /api/cards/{cid}/property           {key, value}   (400 on checklist/table/image/sketch)
DELETE /api/cards/{cid}/property?key=due
POST   /api/cards/{cid}/move               {node, pos?} or {before|after|index|to}
PATCH  /api/cards/{cid}/items/{item}       {text?, done?}
POST   /api/cards/{cid}/items/{item}/done  {done}
POST   /api/cards/{cid}/items/{item}/property   {key, value}
DELETE /api/cards/{cid}/items/{item}/property?key=due
POST   /api/cards/{cid}/append             {text, at?, separator?}
POST   /api/cards/{cid}/items              {text, done?, at?}
DELETE /api/cards/{cid}/items/{item}
POST   /api/cards/{cid}/table              {op, …} or [{op, …}, …]
POST   /api/cards/{cid}/sketch             {op, …}
POST   /api/cards/{cid}/chart              {kind, …}
DELETE /api/cards/{cid}/chart
POST   /api/cards/{cid}/dock               {anchor}
DELETE /api/cards/{cid}/dock
POST   /api/cards/{cid}/group              {group}
DELETE /api/cards/{cid}/group
GET    /api/cards/{cid}/attachments
POST   /api/cards/{cid}/attachments        {name, data_base64}
GET    /api/cards/{cid}/attachments/{idx}
DELETE /api/cards/{cid}/attachments/{idx}
POST   /api/cards/{cid}/images             {data_base64, name?}
GET    /api/cards/{cid}/images/{idx}
DELETE /api/cards/{cid}/images/{idx}
GET    /api/cards/{cid}/export?format=markdown|html|json
  → exactly what the /nodes/{id}/cards/{cid}/… twin returns
  | 404 (no card with that id in this document)
  | 403 (a token confined to a basket, for **any** id it cannot reach)

  Every card id you are ever handed comes without a basket: `/api/search`,
  `/api/tasks`, `/api/claims`, `/api/query`, `/api/properties`, `/api/tags`,
  backlinks, `/api/changes` and the `[[#1391]]` links people paste all name the
  card. Before this, acting on one meant a `GET /api/cards/{cid}` first to learn
  the node — so the cheapest possible edit cost two round trips, and an agent
  ended up quoting a node number the human never mentioned.

  **These are the same operations, not new ones.** The app loop resolves the id to
  its basket and hands the ordinary node-addressed request on, so the scope check,
  the mirror-policy check, the change log and the code that applies the edit are
  the ones that were already there. That is deliberate: a parallel set of write
  paths, each having to remember to check a token's scope, is how a confined token
  could carry its own card out of its basket until v0.111.0 — one unchecked end at
  a time.

  **A confined token gets 403 for an id it cannot reach, whether or not that id
  exists.** Distinguishing "no such card" from "a card in someone else's basket"
  would make this route a way to probe the rest of the document one id at a time.

  **The kind-specific ops joined them in v0.142.0, and their absence was never a
  decision.** v0.117.0 shipped eight routes, v0.118.0 added `append` and the
  single-item pair, and `table`, `sketch`, `chart`, `dock`, `group`, `images`,
  `attachments` and `export` were simply not reached — nothing anywhere recorded a
  reason. The gap then hardened into a *rule*, because a workspace card described
  the basket form as "still the only form for the kind-specific ops", which reads
  as a design decision and was only ever a description of where the work stopped.
  The symptom was `POST /api/cards/{cid}/table` answering **404** while every other
  way of naming that card worked. Each of these is `{node, card, …}` the moment the
  id resolves; none of them ever needed the caller to supply the basket.
  With the instance key a missing id is an ordinary 404.

  The batch routes stay basket-addressed (`/nodes/{id}/cards/…`): a batch is
  validated against one basket, and a list of ids gathered from a whole-document
  query can span several.

GET /api/docs[?section=<name>]
  → 200 {"version":"0.120.0","format":"markdown","section":null,
         "sections":["Enabling it","Authentication",…],"content":"# Trellis Agent API…"}
  | 404 (no section matching that name — the error lists the ones there are)
        **This document, served by the instance that implements it.** It is
        `include_str!`-ed at build time, so it is not a copy that can drift: it is
        this file, as of the commit the running binary was built from.

        Two things that makes possible. An agent that is not on this machine — the
        phone, a LAN agent, anything holding a token — can read the reference at
        all; every prompt and runbook otherwise says "read
        /media/veracrypt1/Rust/trellis/API.md", which needs the filesystem and
        describes whatever is *checked out* there rather than whatever is
        *installed*. And it answers the question that costs real time: not "what
        does the API do" but "what does the API **this port is serving** do".
        `GET /api/instance` gives you the version; this gives you its manual.

        `section` matches a `##` heading case-insensitively, on a substring, so
        `?section=example` finds *Examples*. Use it: the whole reference is ~100 KB,
        and an agent rarely needs more than one part of it. `sections` comes back
        either way, so one call orients you and the next is narrow.

        Allowed at **any** scope, including a token confined to a basket: it is
        static text with no document content in it, and an agent that cannot read
        how the API works is not confined, just broken.

GET /api/search?q=<text>
  → 200 {"hits":[ {node,card,node_title,snippet} ]}                   (case-insensitive)
```
Note: `tree` and `nodes` report `cards` as a **count**; `GET /api/nodes/{id}`
returns the **full card objects**.

Every card object carries **`empty`** — whether it has content *of its own kind*.
Read that rather than `body`: a **checklist keeps its content in `items` and a
table in `rows`, so neither has a `body` at all**, and an audit that treats a
missing body as an empty card will read a 23-line working list as noise. (One
nearly deleted two.) The title is deliberately not counted as content, so a titled
card with nothing in it reports `empty: true` — which is the state worth noticing.

**A bare `[[Title]]` link resolves to the linking card's own project first.**
Duplicate basket titles are normal, not an edge case: "one `Archive` basket per
project" is the archiving convention, so a real document has dozens. The rule is
(1) a basket under the same root as the card the link is written in, then (2) the
**lowest node id**, so the answer never changes between runs. Before v0.121.0 the
lookup walked a `HashMap`, whose order Rust seeds per process — measured against
three baskets called `Archive`, the same link in the same document resolved to
node 7, 7, 5, 3, 3, 7 over six runs of the same binary. Use `[[42]]` or
`[[#1391]]` when you mean a specific one and there is any doubt.

In every `hits` list (search, tags, properties, query, backlinks), `card` is the
id of the matching card so a client can point straight at it. It is `null` only
for a search hit that matched a **node title** rather than a card.

### Create
```
POST /api/nodes            {title, parent?}
  → 201 {"id":<new>}   | 400 if parent doesn't exist

POST /api/nodes/{id}/cards {kind?, title?, body?, lang?, items?, rows?, header?, pos?, z?, size?, color?, font_scale?, fit?, image_base64?, inline_images?, source?}
  → 201 {"id":<cid>}

POST /api/nodes/{id}/cards [ {…}, {…} ]      the SAME endpoint, given an array
  → 201 {"created":N,"ids":[cids]}   | 400 (empty array)  | 404 (node)

  An array creates a batch, an object creates one — the shape table ops took in
  v0.82.0, and for the same reason: building anything real is many small calls.
  Every element is validated to the same strictness, `fit` is honoured per card
  (re-measured with the real fonts, as for a single create), checklist item ids
  are backfilled, and **every `source` in the batch** is checked against the
  mirror policy rather than only the first.
  → 201 {"id":<new>}   | 404 if node doesn't exist
```
`kind` defaults to `"text"` and may be any of `text`, `code`, `checklist`,
`table` (starts as an empty 3×3), `image`, or `sketch` (an empty draw surface). `pos` is `[x,y]` canvas coordinates
(default `[40,40]`); pass distinct positions to avoid stacking cards on top of
each other — except in a **[feed](#update) basket**, where `pos` is irrelevant
to the reader (the feed lays out newest-first on its own) and you can simply
omit it. `size` is `[w,h]`. **`z`** is depth in the **same units as `pos`** — positive is
toward the viewer, so `z: 200` is as far *forward* as `pos` `+200` is to the
right. See [Depth and time](#depth-and-time) before using it. `color` sets the title-bar accent at creation (see
the accepted formats below). `items` is used only for `checklist`; `lang` only
for `code`.

**`body` is refused on the kinds that cannot show one** — `checklist`, `table`,
`image` and `sketch` answer **400** naming the field that works (`items`, `rows`) or
the sub-resource. A checklist's lines and a table's cells *are* its content, so text
in their `body` is stored nowhere the reader sees it and, the trap that matters,
**is never read as a property**: a checklist card's properties come from its title
and items alone. `kind: "checklist"` with a `body` carrying `due:: …` used to answer
201 and drop the body, so the card silently never reached the Agenda. Send the dated
line as an **item** instead. The same check applies to `PATCH`, on the kind the card
*will be* — `{"kind":"text","body":"…"}` converts the card and keeps the body, which
is a legitimate call and unaffected. `rows` fills a **table** card's cells row by row (`[["a","b"],…]`,
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
PATCH /api/nodes/{id}              {title?, color?, bg?, feed?}
  → 200 {"id":<id>}    | 404
        color: tag color; bg: basket background color. A color sets it,
        an explicit null CLEARS it (no tag / back to the theme default),
        an absent field leaves it unchanged — the same null-vs-absent rule
        as a card's view. Until v0.161.0 null was a silent 200 no-op.
        feed: read the basket as a FEED — one computed column, newest card
        first (by creation order, deliberately not `touched`, so editing an
        old entry never teleports it to the top). For baskets where time is
        the structure: handoffs, ops checks, release logs. The stored x/y
        arrangement is untouched and returns exactly when feed goes false —
        the promise Depth makes about z, made about layout. While a basket is
        a feed, INSERTING NEEDS NO POSITION: just POST the card and the feed
        places it — no reading the column bottom, no overlap repair. Depth/
        Time/Cube and card dragging stand down on a feed basket; `End` jumps
        to the newest card and `Home` to the top in every basket either way.

PATCH /api/nodes/{id}/cards/{cid}  {title?, body?, color?, kind?, font_scale?, fit?, lang?, pos?, z?, size?, items?, rows?, header?, inline_images?, source?}
  → 200 {<updated card>}   | 404
```
Every field is optional; only those present are changed. `pos`/`size` are
`[x,y]`/`[w,h]`; **`fit: true`** resizes the card to fit its content (applied after
every other field; overrides `size`); `font_scale` sizes text/code body font (1.0 = default);
`body` **replaces the body wholesale** — a card read from `GET /api/cards/{cid}`
keeps its text under `card.body`, and a `body` built from the wrong level is a
blank card with your one line on it. To add to a card, `…/append` (below) never
sends the body back at all. `lang` applies to code cards, `items` replaces a checklist's items (send them in
the desired order to **reorder** a checklist), `rows` bulk-replaces a table's cell
text, `header` toggles a table's header row, `inline_images` replaces the text
card's embedded inline images (same base64 + `![](trellis:N)` scheme as create).
A `body` is refused for the kinds that cannot show one, judged on the kind the card
**will be** after the patch (see create, above). **`kind` converts the card to
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
takes precedence over the ordering fields above. A `node` naming the basket the
card is already in is a **400, not a no-op reposition**: this op drops group and
dock membership (they are basket-local), so a same-place "move" would quietly
ungroup the card. To reposition a card on its own canvas, `PATCH` its `pos`.
```
POST /api/nodes/{id}/cards/move  {cards:[ids], node:<target>, pos?:[x,y], gap?:20}
  → 200 {"moved":N,"node":<target>,"cards":[ids]}
  | 400 (empty list / already in that basket)  | 404 (node, destination, or a card
        that is not in the source basket — named)

  **The whole list is validated before anything moves.** One bad id refuses the
  batch, because a partial move leaves you unable to tell how far it got — the
  same rule table ops follow. `pos` places the first card and stacks the rest
  below it by each card's height plus `gap`; omit it to keep every card's current
  coordinates, which is what you want when the layout already means something.
  Ids survive, so `[[#id]]` links and backlinks to a moved card still resolve.

POST /api/nodes/{id}/cards/property  {cards:[ids], key, value}
  → 200 {"updated":N,"cards":[ids],"key":"status","value":"done"}
  | 400 (empty list)  | 404 (node, or a card not in it — named)

  One `key:: value` on many cards. Validated up front, same as the batch move.

DELETE /api/nodes/{id}/cards/property  {cards:[ids], key}
  → 200 {"cleared":N,"cards":[ids],"key":"due"}
  | 400 (empty list)  | 404 (node, or a card not in it — named)

  The other half: **remove** a `key:: value` line from many cards. `cleared`
  counts the cards that actually carried it and `cards` names them, so "8 of the
  20 had a due date" is legible rather than hidden. A card that never had the
  property is not an error — asking for it to be gone and getting that is the
  point. `key` goes in the body, not the query string as it does on the
  single-card form: the card list has to be a body anyway, and splitting one
  request across both is how you delete the wrong property from the right cards.

PATCH /api/nodes/{id}/cards  {cards:[ids], color?, size?, fit?, font_scale?, z?,
                              emphasis?, emphasis_intensity?, emphasis_minutes?}
  → 200 {"updated":N,"cards":[ids]}
  | 400 (empty/missing list, a content field, an unknown field)
  | 404 (node, or a card not in it — named)

  **Presentation only, and that is deliberate.** `title`, `body`, `items`, `rows`,
  `kind`, `lang`, `header`, `source` and `inline_images` are refused **by name**,
  with the single-card route in the message. Each of those *is* the card: writing
  one across a list means every card in the list ends up saying the same thing —
  which is the copied-card failure the task model exists to prevent — and one
  typo'd id list would be an unrecoverable overwrite of somebody's work. Content
  is one card at a time; the batch is for *make these look the same*.

  Everything else behaves exactly as the single-card `PATCH` does, because it is
  the same code applying it: the same 80×60 size floor, the same `z` clamp, the
  same emphasis expiry. `fit` is re-measured with the real fonts, per card.

  There is no `pos`: it would stack every card in the list on one point, hiding
  all but the last. Use the batch **move**, whose `pos` + `gap` stacks a column.

DELETE /api/nodes/{id}/cards  {cards:[ids]}
  → 200 {"deleted":N,"cards":[ids]}
  | 400 (empty list)  | 404 (node, or a card not in it — named)

  Validated in full before a single card is removed, which matters more here than
  anywhere else on the batch surface: a half-finished delete cannot be undone by
  re-sending the request. There is no "everything in this basket" form on
  purpose — the one batch operation you cannot walk back should not be reachable
  by *omitting* an argument. To clear a basket, list its ids; to keep the work,
  move them to the project's `Archive` instead (a moved card keeps its id).

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

POST   /api/nodes/{id}/groups            {cards:[ids], title?, color?}
  → 201 {"id":<gid>}   | 400 (need ≥2 existing cards)  | 404

PATCH  /api/nodes/{id}/groups/{gid}      {title?, color?}
  → 200 {"id":<gid>}   | 404

DELETE /api/nodes/{id}/groups/{gid}      → 200 {"ungrouped":<gid>}   | 404   (cards remain, container removed)

GET    /api/groups/{gid}                 → 200 {node, node_title, node_path, group:{id,title,color,cards:[ids]}}  | 404

POST   /api/nodes/{id}/groups/{gid}/move {node:<target>, pos?:[x,y]}
  → 200 {"group":<gid>,"node":<target>,"moved":N}
  | 400 (already in that basket)  | 404 (node / group / destination)
```

**Moving a group needs this route — moving its cards does not work.** Group
membership is basket-local, so `cards/{cid}/move {node}` deliberately drops it:
each card arrives ungrouped and you are left rebuilding the container by hand,
with a **new id**. The id is what `[[#g…]]` points at, so rebuilding breaks every
link already written to that group. This route carries the container, the
members, the title, the colour and the id together.

`pos` places the group's **top-left corner** in the destination; every member
moves by the same delta, so the arrangement inside the group survives. Omit it to
keep the current coordinates. Docking is kept between cards that travel together
and cut where it would name a card left behind.

### Desktop mode (Linux/X11)
Send a card out of the canvas to become its own borderless OS window, sitting
among your other applications instead of inside Trellis.
```
POST   /api/nodes/{id}/desktop      -> 200 {"node":63,"desktop":true,"cards":[ids]}
  | 404 (node)  | 501 (not Linux)  | 503 (no frame drawn yet, so no screen to place on)
DELETE /api/nodes/{id}/desktop      -> 200 {"node":63,"desktop":false,"cards":[]}

  **This is the feature.** One call takes the whole basket out onto the desktop,
  the way VMware's Unity puts a guest's windows on the host — the per-card route
  below is the exception, not the main event. Only one basket is out at a time:
  two baskets of windows on one desktop is a pile with no way to tell which
  document you are looking at, so turning a second one on recalls the first.

  **The arrangement survives.** The basket's bounding box is fitted to the screen
  and every card is placed by the same scale, so the layout you built is the
  layout you get. Windows keep their real size — scaling a card's window would
  shrink its text — so only the spacing between them compresses.

GET    /api/desktop        -> 200 {"supported":true,"cards":[{"card":1815,"pos":[760,430]}]}

POST   /api/cards/{cid}/desktop  {pos?:[x,y]}
  -> 200 {"card":1815,"desktop":true,"pos":[760,430]}
  | 404 (no card with that id)  | 501 (not Linux)

DELETE /api/cards/{cid}/desktop  -> 200 {"card":1815,"desktop":false,"was_on_desktop":true}
```
`pos` is **screen** pixels; omit it and the window opens near where the card sits
on the canvas. The window is undecorated, transparent-cornered, keeps no taskbar
entry, and is dragged by its own title strip.

**One real window per card, not one overlay.** An overlay is a single window and
therefore sits entirely above or entirely below every other application - a card
could never be behind a browser and in front of a terminal, which is the point.
Windows are **not** always-on-top for the same reason: a card that can never go
behind anything is a HUD, not part of the desktop.

**Placement is app config, not document state** - `card id -> [x, y]` per
instance, the same rule that keeps templates and the backup schedule out of the
`.ron`. A screen coordinate belongs to one machine; a document opened elsewhere,
or read by the Android app, must not carry it.

**Linux/X11 only.** Wayland has no protocol for an application to position its own
windows; macOS and Windows need their own pass and return **501** for now.

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

### YAML frontmatter — at the boundary only
Trellis does **not** adopt frontmatter as its internal model: `key:: value` already
does that job, works on a single checklist line (which is what lets one card hold N
independently-dated tasks), and reaches a caller as parsed JSON in `properties`
rather than as a block of text to parse. Frontmatter earns its place at the **edges**,
where other tools speak it.

- **Import.** A `.md` **dropped into a basket** has a leading `---` block read and
  mapped onto what Trellis scans: `tags:` becomes `#tags`, everything else becomes
  `key:: value`, and `title:` becomes the card's title. Without this an imported
  Obsidian note's `due: 2026-09-01` is inert prose — the property parser needs `::`
  and YAML uses a single colon.
- **Export.** `GET /api/nodes/{id}/cards/{cid}/export?format=markdown|html|json`
  (and **Copy → Markdown** on a card) emits the block, so a card lands in Obsidian
  with its dates, status and tags intact. A card with no properties and no tags gets
  no block rather than an empty one. The **whole-document** export
  (`GET /api/export`) does not carry frontmatter and should not: a file of many
  cards has no one set of fields that describes it.

```
GET /api/nodes/{id}/cards/{cid}/export?format=markdown|html|json
  → 200 {card, format, content}   | 400 (unknown format)  | 404
```

A deliberate subset is understood: `key: value`, quoted scalars, `key: [a, b]` and
`key:` + `- item` lists. **Nested mappings are skipped, not flattened** — guessing at
structure is how an import quietly invents data — and an opening `---` with no
closing fence is treated as ordinary content rather than swallowing the document.

### Attachments
Files carried by a card — **the bytes, not a path to them**. A pointer to a file on
one machine's disk is worthless the moment the document is opened on the phone,
restored from a backup, or read by anyone else, which is the same reason images are
embedded.

**On the card, not in a card kind.** Any card can carry attachments, whatever its
kind, so "drop the spec onto the task card about it" works without a container card
in between. They ride along through card export/import and templates.
```
GET    /api/nodes/{id}/cards/{cid}/attachments
  → 200 {card, attachments:[{index, name, ext, bytes}]}   | 404
  (names and SIZES only — listing must not drag every attached file through the
   response; fetch one by index for its content)

GET    /api/nodes/{id}/cards/{cid}/attachments/{idx}
  → 200 {index, name, ext, bytes, base64}   | 404

POST   /api/nodes/{id}/cards/{cid}/attachments   {name, data_base64}
  → 201 {card, index, name, bytes}   | 400 (no name / bad base64)  | 404

DELETE /api/nodes/{id}/cards/{cid}/attachments/{idx}
  → 200 {card, removed}   | 404
```
**The cost is real and worth knowing before you use this.** The document is one
gzip-compressed RON file written **whole**, atomically, on every save — so an
embedded file is re-serialised on each autosave and copied into every
version-history snapshot and every backup archive. `attachment_bytes` on
`GET /api/instance` is the running total. The app warns above **10 MB** on a drop
and lets you go ahead anyway; the API sets no limit at all, deliberately, because a
policy buried in the model is one an API caller would inherit without ever being
told about it.

In the app: **drop any file** onto a basket. Onto a card, it attaches to that card;
onto empty canvas, it becomes a card named after the file. Click an attachment to
save a copy back out; the `×` in edit mode detaches it.

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

### Import an Obsidian vault
A folder of Markdown notes on **this machine's disk** becomes a tree of baskets.
```
POST /api/import/vault   {"path":"/abs/path/to/Vault", "parent":<node id>?}
  → 200 {"root":<node id>,"baskets":N,"cards":N,"attachments":N,
         "links_rewritten":N,"unresolved":["Note name",…]}
  | 400 path is not a directory / the folder holds no files to import
  | 404 parent node not found
  | 403 for a scoped token
```
`parent` omitted imports the vault as a new **root project** — a vault is
somebody's whole notes, and burying it under whatever basket happened to be
selected is the wrong default. The same import is on **File → Import → Obsidian
vault…**, and dropping a **folder** onto a basket does it too.

The mapping:

| Obsidian | Trellis |
|---|---|
| folder | basket (nested, the same shape as the vault) |
| note (`.md`) | **card** |
| YAML frontmatter | `key:: value`, and `tags:` → `#tags` |
| `title:` field | the card's title (the file name otherwise) |
| `![[file.pdf]]` | an **attachment** on the card that names it |
| `[[Note]]` | `[[#<cid>\|Note]]` — a real card link |

**A note is a card, not a basket.** A basket is a *space* holding things and a
note is a *thing*; mapping notes to baskets would produce a tree of empty
containers. It also settles the link question, because Trellis's bare
`[[Title]]` resolves to a **basket** — so every imported note link would dangle.
Links are rewritten to `[[#id|Note]]` in a second pass, after every card exists
and has an id, and the pipe keeps the name the author wrote. `[[Note#Heading]]`
and `[[Note#^block]]` resolve to the card and keep the subpath as the label:
there is no sub-card address to point at, but the reader still sees which section
was meant.

**A link naming no note is left exactly as written** and reported in
`unresolved` — a dangling link someone can read and fix beats content silently
deleted. A **bare** name that two notes share is deliberately *not* resolved
(the full path still is): picking one would point half the vault's links at the
wrong card while looking like it worked.

**A `.canvas` becomes a basket.** [JSON Canvas](https://jsoncanvas.org) is an open
format and it is a Trellis basket almost exactly — nodes with `x`, `y`, `width`,
`height`, arranged in space and boxed into labelled groups. Importing the only
genuinely spatial file in a vault as bytes on a card would make it unreadable, so:

| canvas | Trellis |
|---|---|
| the file | a **basket** (under the folder's basket) |
| `text` node | a text card at the same place and size |
| `file` node → a note | a card **linking** to the card that note became |
| `file` node → an asset | an image card, or a card carrying the file |
| `link` node | a card holding the URL |
| `group` node | a **card group**, from what falls inside its rectangle |
| `edge` | `→ [[#id]]` in the card the arrow leaves, with its label |

Coordinates are shifted so the arrangement lands on screen with **every relative
position unchanged** — the layout is the content. Colours come across, both
Obsidian's `"1"`–`"6"` presets and a raw `#rrggbb`. Group membership is read off
the geometry by each card's **centre**, so a card straddling an edge belongs to
the group it is mostly in rather than to both. A `[[Note]]` written inside a
canvas text node is rewritten like any other. Unknown fields are ignored on
purpose: strictness is the rule for API *input*, and a canvas written by a newer
Obsidian is *reading*. A `.canvas` that will not parse keeps its bytes as an
attachment rather than vanishing.

Dot-directories are skipped — `.obsidian` is the other app's workspace layout and
plugin config, `.trash` is deleted notes, `.git` is not vault content. An
unreferenced attachment still comes in, as its own card, because bytes discarded
here cannot be recovered from the document afterwards.

**It reads an arbitrary local path**, so like `source` file mirroring it is a
whole-document route and is refused for a scoped token.

### Card links — `[[#id]]`
A link works in a card's **body**, in a **table cell**, and (since v0.103.0) in a
card's **title** — which is where the diagram recipe puts one, to tie a figure to
the script that drew it. Titles were already read by the backlink index, so a
title link counted as a link and simply could not be followed; now it renders and
clicks like any other.

`[[Some Basket]]` and `[[42]]` link to a **basket**, as they always have.
**`[[#1391]]` links to a card.** The `#` prefix is how card ids are written
everywhere else (the docs, the Ctrl+O palette), so it reads the way it is spoken.
**`[[#g146]]` links to a group.**

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

#### Unlinked mentions — what *should* point here

```
GET /api/cards/{cid}/mentions
  → 200 {"card":1391,"node":63,"hits":[{node,card,node_title,node_path,snippet}]}
  | 404 (card not found)
```

The mirror of backlinks: cards whose text **names** this card — by its title or any
`alias::` — **without linking to it**. Backlinks answer *what points here*; this
answers *what should*. It is worth much more since aliases (v0.126.0), because a
card is usually called several things in prose and only one of them is its title.

Matching is **whole-word and case-insensitive**, and **never inside code**. A
substring match would report `Notes` inside `Notebook`, and a name quoted in a
fenced block or a code span is being *discussed* rather than referred to — the same
rule that stops prose about a property becoming one. A name shorter than **three
characters** is skipped entirely: a card called `Go` would otherwise "mention" half
the document, and a list that long is not read at all, which is worse than not
offering one. A card that already links here is a backlink, not a mention, so it is
left out — every row is something you might actually want to turn into a link.

#### The local graph — what is around *this* card

```
GET /api/cards/{cid}/graph?depth=2
  → 200 {card, depth, cards:[{card,node,node_path,depth,title}],
         edges:[{from,to}], capped, cap}
  | 404 (card not found)
```

`GET /api/graph` is whole-document and **basket**-level: it answers *how do the
projects connect*. This answers *what is around this*, which is the question you
have while reading one card — and in a journal-shaped document a basket is a **day**,
so a basket-level edge says almost nothing.

**Both directions.** A card you link to and a card that links to you are equally
its neighbours; following only out-links would make the answer depend on which end
you happened to write the link from.

Each card carries the `depth` it was first reached at, and the walk is
**breadth-first**, so that depth is the *shortest* path rather than whichever the
walk took first. `depth` defaults to **2** and is clamped to 1–5: two hops is a
neighbourhood, more is a hairball with extra steps.

`cap` bounds the walk at 200 cards — a hub card links to everything, and a "local"
graph that returns the whole document is not local. **`capped: true` says the bound
bit**, rather than truncating silently.

#### Group links — `[[#g146]]`
A group had an id and no way to address it: the only way to point anyone at one
was to name a card **inside** it, which says "somewhere near here" rather than
naming the thing. `[[#g146]]` names the group, and following it centres the canvas
on the container and flashes it — the same reveal a card link does.

The `g` is what separates the two id spaces. Card ids and group ids come from
different counters, so the same number can name both; nothing written before this
existed changes meaning, because `g146` never parsed as a card id.

```
GET /api/groups/{gid}/backlinks
  → 200 {"group":146,"node":366,"hits":[{node,card,node_title,node_path,snippet}]}
  | 404 (group not found)

GET /api/groups/{gid}/link
  → 200 {group, title, node, node_path, document, link, link_verified, http, wikilink}
  | 404
```

`wikilink` is the `[[#g146]]` form — what you paste into a card. `link` is the
`trellis://` form, for leaving the app. In the UI: right-click a group's header
→ **Copy** → *Group link*, and **Ctrl+O** accepts `g146` or `#g146`.

### Aliases — reaching a card by another name
A card that carries an `alias::` (or `aliases::`) property can be linked to by
that name.

```
alias:: Start Here             on the card
aliases:: Start Here, Front Door   several names, one property

[[Start Here]]                 → that card
```

Obsidian notes carry `aliases:` in their frontmatter and a note becomes a **card**
here, so without this every alias in an imported vault was inert text.

**A basket still wins.** `[[Name]]` has always meant a basket, and links already
written must keep meaning what they meant, so an alias is consulted **only when no
basket has that title**. It can rescue a link that used to dangle; it can never
redirect one that worked.

Matching is case-insensitive. Two cards claiming one alias is undecidable, so the
tie is broken the way duplicate basket titles are: **same project first, then the
lowest card id** — never `HashMap` order. A checklist card's aliases come from its
**title and items**, like all its properties, never its body.

### Block references — `[[#id^item]]` names one checklist line
```
[[#1391^766]]     links to card 1391 (the reveal scrolls to the card)
![[#1391^766]]    embeds just that one line
```
Since v0.90.0 a checklist item with its own `due::` is a task in its own right,
with a **stable id** — so a line is a thing worth pointing at, and this is
Obsidian's block reference expressed in the id space this app already had. Item
ids come back from `GET /api/nodes/{id}/cards` and the task surfaces.

The **link** resolves to the card, because that is what a reveal can scroll to and
flash. What the item part buys is the **embed**: showing one line instead of
pasting a 23-line working list in to point at one task. A reference naming no such
item, or a card that is not a checklist, says so in the frame.

### Embeds — `![[#id]]` shows a card inside another
The complement of `[[#id]]`. A link says *go and look at that*; an **embed** says
*show it here*.

```
![[#1391]]      in any card body — renders card 1391's content in place
[[#1391]]       unchanged: still a link
```

It exists because of the rule this app is built on — **one task is one card, never
copied**. Until now, seeing a card's content in two places meant duplicating it,
and a copied task card is a second task with its own `status::` and `due::`,
counted twice, with nothing warning you. An embed is the answer: one card, shown
wherever it is needed, and editing it changes every view of it.

**It is a view, never the stored text.** The body on disk keeps `![[#id]]` —
`GET /api/cards/{cid}` returns exactly what was written. Expanding on save would
be the copy the feature exists to avoid, and leaving it alone is also what
Obsidian writes, so an exported card still round-trips. Same rule as block-HTML
conversion, which is likewise applied at render.

The embedded card renders as a **blockquote** headed by its title and ending with
`from [[#id]]`, so the source is always one click away — an embed that cannot be
traced back to the card it came from is where "which one is real?" starts. A
**checklist** embeds as its items and a **table** as its rows, because that is
where those kinds keep their content; reading `body` would render an empty frame.

An embed counts as a **link** for `GET /api/cards/{cid}/backlinks` and the link
graph, which is what you want: the card is genuinely referenced.

Three things it refuses to do, each reported in place rather than silently:
- **A cycle** — a card embedding itself, directly or round a chain. This is the
  `unconditional_recursion` shape that has shipped a crash in this project twice.
- **More than 4 levels** of nesting. A chain with no cycle in it can still be
  arbitrarily long, and each level is another whole card pasted into a frame
  someone is trying to skim.
- **A target that does not exist** — it says so, because a silently blank frame is
  the answer this project refuses everywhere else.

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
        never had it — not an error.

        **Setting a property to `""` is not the same, but not for the reason this
        page used to give.** An empty value is not parsed as a property at all, so
        the task does leave the agenda — what it leaves behind is a dangling
        `due:: ` line in the body for the next reader to puzzle over. The case that
        actually traps you is a **non-empty value that is not a date**: `due:: soon`
        *is* a property, so the card is still a task, and with nothing to sort by it
        sits under **"No date"** indefinitely. Measured, both ways, on 2026-08-17.
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
trellis://127.0.0.1:<port>/card/<cid>            trellis://127.0.0.1:7374/card/1391
trellis://127.0.0.1:<port>/node/<id>             trellis://127.0.0.1:7373/node/63
trellis://127.0.0.1:<port>/group/<gid>           trellis://127.0.0.1:7374/group/146
trellis://127.0.0.1:<port>/card/<cid>?doc=<file> optional, verified on arrival
http://127.0.0.1:<port>/open/card/<cid>          the same thing, no registration needed
```

Inside a document, prefer the wiki-link forms — `[[#1391]]` for a card,
`[[#g146]]` for a group. A `trellis://` link is for handing an address to
something outside the app.

**Why `127.0.0.1:` is in there.** Links used to be minted as
`trellis://7374/card/1391`, which puts the port where a URL keeps its **host** —
and a bare integer is a legal IPv4 address, so a desktop URL handler is entitled
to normalise it. KDE's does: `7374` arrives as **`0.0.28.206`** (that is
`0x00001CCE` as a dotted quad) and the link fails. With the port in the port
position there is nothing left to rewrite.

**Old links still work.** A bare port, and the dotted-quad form a normaliser
produces, are both accepted — a link written into a card or a session report a
year ago has to keep opening. Only loopback is accepted as a host: a clicked link
must never reach another machine.

**Never build one by hand — ask for it:**

```sh
curl -s -H "X-API-Key: $KEY" $API/cards/1391/link
# → {"card":1391,"node":63,"node_path":"Trellis › Trellis Open Items",
#    "document":"Personal.ron",
#    "link":"trellis://127.0.0.1:7374/card/1391",
#    "link_verified":"trellis://127.0.0.1:7374/card/1391?doc=Personal.ron",
#    "http":"http://127.0.0.1:7374/open/card/1391"}

curl -s -H "X-API-Key: $KEY" $API/groups/146/link
# → {"group":146,"title":"Design — how it was scoped","node":366,
#    "node_path":"VolumePerApp","document":"Personal.ron",
#    "wikilink":"[[#g146]]",
#    "link":"trellis://127.0.0.1:7374/group/146", …}
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

# 2b. Drop a date entirely. DELETE removes the `due::` line. Setting it to ""
#     also takes the card off the agenda (an empty value is not a property), but
#     leaves a dangling `due:: ` in the body. A value that is not a date —
#     `due:: soon` — is the real trap: still a property, still a task, parked
#     under "No date" for good.
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
PATCH  /api/nodes/{id}/cards/{cid}/items/{item}          {text?, done?}
```

**Editing one line's text** (v0.163.0) is the `PATCH`, and it is the reason the
list above is not enough on its own: until it shipped, `/done`, `/property` and
`DELETE` could each address a line while changing its *wording* meant rewriting
the whole `items` array — the one call that re-ids lines by position. Both fields
are optional; sending neither is a 400 rather than a 200 that changed nothing.

```sh
PATCH  /api/nodes/{id}/cards/{cid}/items/{item}   {text?, done?}
  → 200 {"card":<cid>, "item":<item>, "text":"…", "done":false}
  | 400 (neither field)   | 404 (card, or no such item on it)
```

Takes a bare card id too: `PATCH /api/cards/{cid}/items/{item}`.

**Add and remove a line one at a time** (v0.118.0) rather than rewriting the array:

```sh
POST   /api/nodes/{id}/cards/{cid}/items          {text, done?, at?:<index>}
  → 201 {"card":<cid>, "item":<new id>, "index":<n>, "count":<N>}
  | 400 (not a checklist card)   | 404 (card)
DELETE /api/nodes/{id}/cards/{cid}/items/{item}
  → 200 {"card":<cid>, "item":<item>, "deleted":true, "count":<N>}
  | 400 (not a checklist card)   | 404 (card, or no such item on it)
```

Both take a bare card id too: `POST /api/cards/{cid}/items`,
`DELETE /api/cards/{cid}/items/{item}`.

**Why not just `PATCH` the `items` array?** Because that carries the existing ids
across **by position**. If a line was reordered or removed between your read and
your write — by the person you are working with, in the app — every id after it
changes hands, and an id is what `…/done` and `…/property` address. A dated line
*is* a task, so that quietly reassigns which task is which. Adding or removing one
line touches one line, and the new item's id comes back so you can address it
without reading the card again. `at` inserts at a 0-based position; omit it to
append. The wholesale rewrite is still there and still right for *replacing* a
list wholesale.

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

### Channels — talking to an agent on a card

A **channel** is an ordinary card whose body is a running conversation. You write
into it from the desktop or the Android app; an agent reads it, replies into it,
and its reply reaches your phone through the notification plugin you already have.
Point two agents at one and it is an agent-to-agent log you can read in real time.

```
PATCH /api/cards/{cid}   {"channel": {"participants":["alice","operator"], "primary": true}}
  → the card is now a channel        | {"channel": null} makes it an ordinary card again

POST /api/nodes/{id}/cards   {"title":"…", "channel": {"participants":[…], "primary": false}}
  → a card BORN a channel (v0.163.0), so it takes one call rather than create-then-patch
  | 400 ('<project>' already has a primary channel: card <cid> in <path>)

POST /api/nodes/{id}/cards/{cid}/say   {text}
POST /api/cards/{cid}/say              (same, by card id alone)
  → 200 {"card":<cid>,"node":<id>,"seq":<n>,"from":"alice","at":"<rfc3339>"}
  | 400 (empty text, or the card is not a channel)

GET /api/nodes/{id}/cards/{cid}/channel[?since=<seq>]
GET /api/cards/{cid}/channel[?since=<seq>]
  → 200 {card, node, title, participants, primary, seq, since, count,
         messages:[{seq, from, at, text}]}

GET /api/channels[?agent=<name>][&project=<id>]
  → 200 {count, channels:[{card, node, node_path, title, participants, primary, seq}]}
```

A channel card carries **`channel`** in its own JSON — on `GET /api/cards/{cid}`
and in every basket listing — so a client can tell a conversation from an ordinary
card without a second request, exactly as `view` does for a saved view.

**`?project=<node id>` is the filter an agent should use**: it scopes the
listing to one project's subtree, which is the answering boundary (see
[`channels_waiting`](#instance) — answer your project's channels, report the
rest). `?agent=` matches `participants` and is for a name you were *assigned*,
never one you guessed.


**In the app:** card menu → **Make a channel…**, and on Android the card reader's
**Channel…** — same fields, same rules. The API is not the only way in.

**Something has to answer — and in practice that is a running agent, not a
plugin.** A message waits until an agent looks: at its checkpoints, or the
moment it lands if the agent's harness runs an [`/api/wait`](#live-updates-long-poll)
watcher in a background task (loop the long-poll, check `channels_waiting` after
each return, end the task when it goes above zero so the completion notification
wakes the session; restart it after answering and after a harness timeout). **A
watcher dies with its session** — nothing watches between sessions, so a message
sent while no agent is running waits for the next one. The **`claude` plugin**
(`plugins/claude/`) was an attempt to answer with no session running; it stays
in the repo as a reference but is installed nowhere — it never ran once in
practice, and its design is kept here because the `roots` mapping below is the
same project-boundary rule agents follow.

**It answers in the channel's own project directory.** `node_path`'s first segment
is the root basket, and the plugin's **`roots`** setting maps those names to
checkouts, one `Project = /path` per line. A card in a NodeJS project saying *"add
a log check to the boot sequence"* is then answered by an agent running in that
project. **A channel whose project is not mapped is skipped**, and the log names
the line to add — a reply worked out in the wrong repository is indistinguishable
from a real one, which is worse than no reply. Give **each agent its own copy** of
the plugin (`plugins/<agent>/`, with `name` changed in its `plugin.json`): the
manifest name is what it answers as, and each copy carries its own approval,
token, `roots` map and cursor, so twelve agents never share a working tree.

**Say who you are with a header.** Every write may carry `X-Agent: <name>`, and
that is what a message is attributed to:

```sh
curl -s -H "X-API-Key: $KEY" -H "X-Agent: alice" \
  -d '{"text":"Found it — the 404 was an omission, not a rule."}' \
  $API/cards/1391/say
```

The name also lands on `/api/changes`, so *which* agent made a change is now
answerable. With no header the message is from **`operator`**.

> **Keep the header ASCII.** A non-ASCII header *value* is rejected by the HTTP
> layer before Trellis sees it, and the connection is closed with **no reply at
> all** — `curl` reports *empty reply from server*, not a status code. That is not
> specific to `X-Agent` (any header does it, on any route, and it predates this
> feature), but it is the one an agent now sets from a name someone chose. The
> validator only accepts letters, digits, `-`, `_` and `.` anyway; the difference
> is that an ASCII violation answers **400** and a non-ASCII one answers nothing.

`X-Agent` is **declared, not derived**, and that is deliberate. Deriving it from
the credential is the obvious design and it fails the case this exists for: the
normal setup is several agents all holding the **instance key**, so that one can
leave a finding in another project's workspace — and a shared key names nobody. It
is not a security boundary either, since anything holding that key can already
write any text under any name by editing the body. Where a **scoped `agent_…`
token** is used the token's own label is authoritative and overrides the header,
so the confined case keeps the stronger guarantee for free. The name is validated
(1–40 chars, letters/digits/`-`/`_`/`.`) because it is written into a message
header line, and a name containing ` · ` could forge a message boundary.

**The body is the log.** Messages are appended as blocks under a heading:

```
### @alice · 2026-08-21T14:03:22Z · #7
Found it — the 404 was an omission, not a rule.
---
```

The closing `---` is load-bearing. A message needs an **end**, not just a start:
without one a block runs to the next header, so anything you type at the *bottom*
of the card — the natural place to type — is swallowed into the last agent's
message and attributed to it, which is the exact confusion a channel exists to
remove. A horizontal rule is what a person separating a log by hand would have
written anyway, and it renders as a divider everywhere. The cost: an agent whose
own text contains a lone `---` splits its message, and the remainder reads as
operator text. Visible, recoverable, and pinned by a test.

That is a Markdown heading on purpose: it renders everywhere a card body already
renders — the canvas, the exports, and the phone — so there is no new UI, and it
is still one line a parser can key on exactly. **This is why a channel is a field
and not a new `CardKind`**: it does not draw differently, which is the only thing
that would justify a variant (see *Why new card kinds are avoided*). And not a
`channel::` property, because a property fires on prose **about** channels.

**Anything without a header is attributed to you.** Type into the card from the
Android app and that text comes back as a message from `operator` with `seq: 0`.
That is not leniency — it is what makes replying from the phone work with no
feature at all, and it is why unheaded text is always returned regardless of
`?since=`. A written message never has `seq: 0`.

**A reply seals it.** `seq: 0` would otherwise be permanent, and a client using
`?since=` as its cursor would read the same loose text for ever — the channel
plugin answered one question on four consecutive runs before this was fixed. So
`say` gives any unheaded text at the **end** of the body a header on its way past,
numbered and stamped `operator`. `seq: 0` is the brief state between typing and
being answered, not a place a message stays. Text wedged *between* two existing
messages is left exactly as written: renumbering somebody's words to tidy the file
is worse than the untidiness, and there is no honest timestamp for it.

**`participants` is addressing, not an access list.** It is how an agent *finds*
its conversations — `GET /api/channels?agent=alice` — without being told a card id.
A message from a name that is not listed is recorded under that name rather than
refused, because an agent dropping a bug report into another project's channel is
the point of the design, not an intrusion.

**`primary` marks the workspace's own channel**, the one an agent drains when it
was given a project rather than a card. At most one per project — but an
agent-to-agent channel is a *second* channel in the same workspace, which is why
the constraint is a flag rather than "one channel per project".

`seq` is stored on the channel, not counted from the body, because you can edit
that body by hand; counting would renumber every earlier message the first time
you did.

### Saved views — a query you can keep, as a card
Every other view here is **fixed**: Find cards, the Agenda and the Kanban each
answer one question someone else chose. A saved view is your question, kept.

A view is an ordinary card carrying a **`view` field**. Set it with `PATCH`, clear
it with `view: null`:
```
PATCH /api/cards/{cid}
{"view": {
   "scope":   <node id>,              // optional; whole document if omitted
   "filters": [{"key":"status","op":"eq","value":"blocked"},
               {"key":"due","op":"le","value":"2026-09-30"}],
   "columns": ["due","status","basket"],
   "sort":    {"key":"due","dir":"asc"},
   "limit":   50
}}

GET /api/cards/{cid}/run
  → 200 {"card":…,"columns":[…],"count":N,
         "rows":[{"node":…,"node_title":…,"card":…,"title":…,"values":[…]}]}
  | 400 this card is not a saved view
  | 404 card not found
  | 403 for a scoped token
```

**It is a field, not a `CardKind` and not a property.** A new kind is expensive
in a way the compiler mostly hides (see *Why new card kinds are avoided*) and buys
nothing here — a view is a text card that draws
something derived, exactly as a `source` mirror and a table's `chart` already are.
And a magic `view::` *property* would fire on prose **about** views, which is the
false-property class this project has already fixed twice. A switch that triggers
on writing is a bug generator.

**The rows are never stored.** They are computed on read, so a view cannot go
stale — storing them would be the copy this app exists to prevent. `omitting`
`view` on an unrelated `PATCH` leaves it alone; only an explicit `null` clears it.

**Filters** are ANDed. `op` is one of `eq`, `ne`, `lt`, `le`, `gt`, `ge`,
`contains`, `exists`. `key` is a property key or one of the pseudo-keys `title`,
`basket`, `id`, `kind`, `touched`, `tag`, `text` — so a view can select on what
the document knows structurally, not only on what someone wrote.

**Values compare as what they are.** Two dates compare through the same
`parse_ymd` the Agenda uses, so a view and the Agenda cannot disagree about what
a day is; two numbers compare numerically, so `priority:: 10` is not below
`priority:: 9`; anything else compares as text, case-insensitively. `tag` and
`text` are haystacks, so `eq` on them means *has that tag* — the same meaning
`tag=` has always had on `/api/query` — with or without the `#`.

`exists` asks whether the key is **there**, which is not "is not empty": an empty
`due::` is not parsed as a property at all, and this does not claim it is.

**Sorting** puts cards with no value for the sort key **last in both directions** —
an empty first row reads as a broken view. `limit` truncates **after** sorting, so
"top 5 by date" means the first five by date rather than five arbitrary rows put
in order.

**A view never returns its own card.** A row that opens the card you are looking
at is noise, and an invitation to a loop.

**Not in this version, on purpose:** formulas, summaries and group-by. Formulas
are a small expression language needing an infinite-loop guard; filter + columns +
sort + limit is the useful part. The Agenda, Kanban and Find panels are also
**not** re-expressed on top of this — rewriting three working panels onto a new
engine is how three working panels break.

**A property has to land somewhere the card is read from.** A card's properties
come from its **title** and its **content** — and content means the body only for
`text` and `code`. A checklist's content is its **items**, a table's is its
**cells**, an image's is its name and OCR, a sketch has none. So
`POST …/cards/{cid}/property` answers **400** on those four rather than writing
into a body nothing reads: until v0.146.1 it answered 200, echoed the value and
stored nothing, which an agent reported after setting `status::` on a table and
believing it. The message names where it can go instead — the **item** route for a
checklist (`POST …/items/{item}/property`, and a dated line is its own task), the
**title** otherwise. `DELETE` and both batch forms follow the same rule; a batch
refuses wholesale and names the card.

### Properties the app cannot read
```
GET /api/properties/problems
  → 200 {"count":N,"problems":[{"node":…,"node_title":…,"card":…,"card_title":…,
                                "item":<item id|null>,"key":"due",
                                "value":"next friday","why":"…"}]}
  | 403 for a scoped token   (whole document, names no basket)
```
Only `due::`, `start::` and `verify::` are judged — the keys this app **acts** on.
An arbitrary `owner:: ada` is not wrong, it is just a value, and flagging every
key the app has no opinion about would bury the three that matter.

This is the useful half of "typed properties", done the way this model wants it.
Obsidian gives every property a type because YAML is stringly and it edits them in
a side panel; `key:: value` here is inline text that the Agenda, Kanban, query and
claims surfaces already interpret, so a type system would be a second syntax for
something already working — the reasoning that kept frontmatter at the boundary
rather than inside.

What was missing was the **diagnosis**. v0.120.1's finding was that `due::`
surprises people: an empty value is not parsed as a property at all, `status::
done` alone already hides an agenda row, and the real trap is a **non-empty
non-date** — a card that looks scheduled, never reaches the Agenda, and says
nothing about why. `verify::` at least counts an unreadable date as stale;
`due::` and `start::` were simply silent.

A **checklist** is judged by title and items, never body, like everywhere else —
and since an item with its own `due::` is its own task, `item` names the line.
Results are ordered by node id so two runs can be diffed against each other.

### Claims — which stated facts are out of date

**Read this before you trust a workspace card.** A card that says *"both
instances run v0.109.0"* or *"the operator still owes a bot token"* was true when
someone wrote it. Nothing in a document distinguishes a fact from a fact **as of
a date**, so an agent reads a year-old assertion in the same voice as a fresh
one — which has cost real sessions: work redone, and the operator asked for
something already delivered.

A card that asserts state says when that assertion should be re-checked:

```
verify:: 2026-09-01
check:: GET /api/instance
```

`verify::` is the date it goes out of date. `check::` is free text naming the
command, endpoint or file that settles it — so re-establishing the claim is
cheaper than doubting it. Both are ordinary `key:: value` properties (the `::`
needs its trailing space), so they work in any card, need no migration, and are
visible in the card itself rather than hidden in metadata.

```
GET /api/claims                    → 200 {"today_days":N,"count":<n>,"stale":<n>,
                                          "claims":[{node,node_title,node_path,project,
                                                     project_title,card,title,verify,check,
                                                     touched,bucket}, …]}
GET /api/claims?expired=true         only the ones not to be trusted
GET /api/claims?project=<node id>    scope to one project (any node, as /api/tasks)
  | 400 (bad id)   | 404 (node not found)
```

`bucket` is one of:

| bucket | meaning |
|---|---|
| `expired` | past its `verify::` date — **do not repeat what this card says** |
| `unparsed` | `verify::` is not a readable `YYYY-MM-DD`, so it has no expiry at all — counted as stale, never as fresh |
| `today` | due for a check today |
| `soon` | due within a week |
| `ok` | current |

Results come back **worst first**, and `stale` counts `expired` + `unparsed` —
the same number `/api/instance` reports.

**Why this is not `due::`.** A task is finished once and leaves the agenda; a
claim about the world is never finished, it only goes out of date. Modelling
currency as a task would fill the agenda with rows that can never be completed,
and the Agenda's usefulness depends on things leaving it.

**Why not just use `touched`.** `touched` moves when a card is *edited* — fixing
a typo in a stale card would make it look freshly confirmed. Editing and
confirming are different acts, and only one of them is evidence.

**A checklist's items are not scanned.** A claim is the card's assertion, so the
card is the unit — the opposite of `due::`, where the line is the unit of work.

Two conventions worth keeping (they are what make the mechanism work rather than
merely exist):

- **Don't write down a fact that has an authoritative source** — a version, a
  line count, a test count. Name the source instead. Where the value has to
  appear, put the `check::` beside it.
- **When two cards disagree, the fresher `verify::` wins**, and the stale one is
  a defect to fix in the same visit — not a fact to repeat.

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

### A card that is a web page (`html`)

The body is **HTML/CSS/JS** and the card shows it rendered — code, page, or both.
Built for an agent to construct a view of your data: read whatever cards you need,
bake the numbers into a self-contained page, write it into a card, render it.

```
PATCH /api/nodes/{id}/cards/{cid}  {"html": {"view":"split", "allow":"none"}}
POST  /api/nodes/{id}/cards/{cid}/html/render
POST  /api/cards/{cid}/html/render
GET   /api/nodes/{id}/cards/{cid}/html/png            the rendered picture, base64
GET   /api/cards/{cid}/html/png
PATCH /api/nodes/{id}/cards/{cid}  {"html": null}     ordinary card again
```

`html/png` returns `{card, width, height, base64}`, or **404** when the card has
never been rendered. It exists because the bytes are deliberately *not* in the
card JSON — they would be megabytes in every basket listing — and because the
**phone cannot render a page itself**: it has no browser and no business running
one, so it shows what the desktop produced. Rendering is the gated action;
reading back what was already rendered is an ordinary document read.

| field | |
|---|---|
| `view` | `code`, `render`, `split` (default — source above the page) or `vsplit` (side by side). The **Split** button cycles the two: pressing it while already split flips the orientation |
| `allow` | what the page may do when it renders. **`none` by default** |

`GET /api/cards/{cid}` reports `view`, `allow`, `width`, `height`, `rendered`,
`error`, and **`stale`** — true when the body has been edited since the picture
was taken. The PNG itself is never in a listing; it would be megabytes.

**`allow` is the security boundary, and it is enforced by a Content-Security-Policy
written into the page** — not by a browser flag. `--disable-javascript` was
measured against a page that reports whether its script ran: it made **no
difference at all**, the screenshot was byte-identical to the ungated one. A gate
that looks right and does nothing is worse than no gate, so the policy is one this
app writes and you can read in `model::html_csp`.

| `allow` | what runs |
|---|---|
| `none` | the page's own markup. **No scripts and not one outbound request** |
| `network` | may fetch images, styles and fonts over https. Still no scripts |
| `scripts` | everything — for a page you trust, chosen per card |

An unrecognised `view` or `allow` is a **400 naming it**, never a silent fallback:
a typo that quietly landed on a *permissive* default would be a hole.

**Why a picture rather than a live view.** Trellis paints to a GL surface and has
no DOM. An embedded webview is an OS surface that cannot composite inside the
canvas — it would float above the app and refuse to scroll, zoom or export with
the card. A PNG is just an image: it zooms, pans, projects through Depth, exports,
and shows on the phone, which cannot run a browser at all. Rendering shells out to
Chrome or Chromium; with neither installed the render fails and says so.

**Rendering needs no permission.** Building and rendering a page is the feature,
and a sandboxed render produces a picture inside the document and nothing else.

**Raising the permission does.** An API caller may only ever set `allow` to
`none`; `network` and `scripts` are **403**, because those make *this machine*
fetch or execute on the page's behalf, and granting yourself that by writing a
card is not a thing a caller should be able to do. They are the operator's to
grant from the card menu. Lowering to `none` is always allowed.

```sh
# an agent building a view of four cards
curl -s -H "X-API-Key: $KEY" $API/nodes/1 | jq '.cards[]'      # read the data
curl -s -H "X-API-Key: $KEY" -X POST $API/nodes/1/cards \
  -d '[{"kind":"text","title":"Dashboard","body":"<!doctype html>…"}]'
curl -s -H "X-API-Key: $KEY" -X PATCH $API/cards/42 \
  -d '{"html":{"view":"render","allow":"none"}}'
curl -s -H "X-API-Key: $KEY" -X POST $API/cards/42/html/render
```

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

- **`body` is read-only while `source` is set** — unless you turn write-back on
  (below). `PATCH {"body":…}` otherwise returns **409** rather than accepting an
  edit the next refresh would overwrite. Detach with `{"source":""}`, which keeps
  the text that was there.
- **A failed read keeps the last good text** and reports why in `source_error`
  (missing file, unmounted disk, a directory, not UTF-8, or over the 1 MB limit).
  It recovers on its own when the file comes back.

#### Writing a card back to its file

Off by default, per card, and **never continuous**:

```
PATCH /api/nodes/{id}/cards/{cid}  {"source_write": true}   let this card be edited
POST  /api/nodes/{id}/cards/{cid}/source/write              write it over the file
GET   /api/nodes/{id}/cards/{cid}/source/diff               show the difference, change nothing
POST  /api/cards/{cid}/source/write
GET   /api/cards/{cid}/source/diff
```

With `source_write` on, the body may be edited and `source_dirty` becomes true;
while it is true **the refresh poll leaves the card alone**, so nothing is
replaced under the cursor. Both flags are reported on `GET /api/cards/{cid}`.

**The conflict rule: it asks, and shows a diff.** If the file's mtime moved since
the card last read it, `source/write` returns **409** carrying the `diff` that
caused the refusal — nothing is written and nothing is merged. `source/diff`
is the unattended half: it shows the difference and does nothing else, returning
`differs`, `file_changed_since_read` and the diff. In the app the same refusal
opens a window offering *overwrite the file*, *discard my edits and take the
file*, or *leave both alone*. There is deliberately no merge: a merge would be a
third version nobody wrote.

The diff is a list of `{tag, text}` with `tag` `" "`, `"-"` (only in the file) or
`"+"` (only in the card) — so `-` reads as *what the file has* and `+` as *what
you would write over it*.

**Refused, and why:**
- **Not a table, image, sketch or checklist** — 400. A table mirroring a CSV
  would be written back through the inverse of the parser, so quoting, delimiter,
  trailing newline and number format all get re-decided by us and the file would
  change on every save even with no edit.
- **Not in tail mode** — 400, because writing back what the tail shows would
  replace the whole file with its last few lines.
- **Agents are refused by default** — 403 until *Settings → Agent API → Let
  agents write mirrored files back*. This is a **separate permission** from the
  mirror read policy on purpose: reading a file you should not see leaks it,
  writing to one destroys it, and "whoever is at the machine already has the
  filesystem" — the reason the app's own file picker is unrestricted — does not
  extend to a network caller.
- **A basket-confined token** — 403, exactly as it cannot mirror a file at all.

The write is **temp-then-rename** beside the target (never across filesystems,
where rename is not atomic), carrying the original's mode across, and restores a
trailing newline so writing an unedited POSIX text file back is a no-op rather
than a whole-file diff.

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

### What failed (the error log)
The change log records what succeeded. This records what **did not**: every
response of **400 or above** the API has answered this run — refusals (400, 403,
404, 409), faults (500, 503, 504) and failed authentication (401) alike.
```
GET /api/errors?since=<seq>&limit=<n>
  → 200 {"epoch":…, "since":…, "count":N, "total":T, "retained":M,
         "oldest":<seq|null>, "newest":<seq>, "truncated":false,
         "file":"/home/you/.local/share/trellis/api-errors.log", "file_error":null,
         "errors":[ … ]}
  | 403 for a scoped token — the log is whole-document
```
`limit` defaults to 200 (max 2000). `total` is every failure this run, including
ones already rotated out of memory (the last 5000 are kept); it is the same
number `api_errors` on [`/api/instance`](#instance) reports.

Each entry:

| field | meaning |
|---|---|
| `seq` | this failure's own sequence number — **not** a document revision; nothing changed |
| `ts` | unix seconds |
| `status` | the HTTP status the caller was sent |
| `method`, `path` | the request, path **with its query string** |
| `agent` | `X-Agent`, or a scoped token's label; absent for an anonymous call |
| `error` | the `error` message the caller was sent |
| `request` | the first 200 characters of the request body, when one was read. **Never present on a 401** — the body is not read before the key is checked, so a mistyped credential cannot land in the log |

```jsonc
{"seq":12,"ts":1787962687,"status":404,"method":"PATCH","path":"/api/cards/9903",
 "agent":"claude","error":"no such card 9903","request":"{\"body\":\"\"}"}
```

**It is also a file.** Every entry is appended, as it happens, to
`<data-dir>/trellis/api-errors.log` beside `app.ron` — one JSON object per line
with `epoch` added, so it survives a restart and a crash, and reads with
`tail -f` or `jq`. It rotates once at 1 MB to `api-errors.log.1`. `file` in the
answer says where it is for *this* instance, and `file_error` carries the first
write failure if the file could not be written — the log is a record, not a
gate, so a bad path never refuses a request.

**What to do with it.** Read it at read-in when `api_errors` is above zero, and
after a batch of your own writes — a 400 you did not notice is a card you think
you wrote. Nothing in an entry is a secret the caller did not already send, but
the excerpt can show *what* was sent, which is usually the whole diagnosis:
`{"bg": null}` against a build older than v0.161.0, `{"body": ""}` from a
response read at the wrong level. Same `epoch`/`seq`/`truncated` contract as the
change log, so a client that already follows one needs nothing new for the
other.

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

### Archiving finished work — a batch move

Finished cards belong in the project's `Archive` basket, not deleted: a moved card
**keeps its id**, so every `[[#id]]` link and backlink to it still resolves.

Moving them one at a time is what `POST …/cards/move` exists to avoid — clearing a
basket of 55 cards was 55 calls before it.

```sh
# Every card in the basket that is status:: done.
DONE=$(curl -s -H "X-API-Key: $KEY" "$API/nodes/$NID/cards" \
  | python3 -c 'import json,sys,re
cards=json.load(sys.stdin)["cards"]
print(json.dumps([c["id"] for c in cards
                  if re.search(r"^status::\s*done", c.get("body") or "", re.M)]))')

# One call. pos stacks them down a column so the archive reads as a list;
# omit it to keep the coordinates each card already had.
curl -s -H "X-API-Key: $KEY" \
  -d "{\"cards\":$DONE,\"node\":$ARCHIVE,\"pos\":[40,40],\"gap\":20}" \
  $API/nodes/$NID/cards/move
# -> {"moved":48,"node":378,"cards":[...]}
```

**The whole list is validated before anything moves** — one bad id refuses the
batch rather than moving what it can, because a partial move leaves you unable to
tell how far it got. Marking a batch done is the same shape:

```sh
curl -s -H "X-API-Key: $KEY" \
  -d '{"cards":[1246,1730],"key":"status","value":"done"}' \
  $API/nodes/$NID/cards/property
```

…and so is taking a property back off them. Note that **`status:: done` is already
enough to take a card off the agenda** — the default `/api/tasks` and the Agenda
panel both hide done rows, and `?all=true` is how you see them again. Clearing the
date is about the card reading cleanly afterwards, not about the agenda:

```sh
curl -s -X DELETE -H "X-API-Key: $KEY" \
  -d '{"cards":[1246,1730],"key":"due"}' \
  $API/nodes/$NID/cards/property
# -> {"cleared":2,"cards":[1246,1730],"key":"due"}
```

### Making a set of cards look like a set

One call, and the same code that applies a single-card `PATCH` — so the size floor,
the depth clamp and the emphasis expiry are identical:

```sh
# Grey out an archived batch, and drop any emphasis they were carrying.
curl -s -X PATCH -H "X-API-Key: $KEY" \
  -d '{"cards":[1836,1837],"color":"#64748b","emphasis":"none"}' \
  $API/nodes/$ARCHIVE/cards

# Size a set to its content. `fit` is re-measured with the real fonts, per card.
curl -s -X PATCH -H "X-API-Key: $KEY" \
  -d '{"cards":[1840,1841,1842],"fit":true}' \
  $API/nodes/$NID/cards
# -> {"updated":3,"cards":[1840,1841,1842]}
```

**Content is refused here, by name.** `body`, `title`, `items`, `rows`, `kind`,
`lang`, `header`, `source` and `inline_images` are single-card fields: writing one
across a list means every card ends up saying the same thing, and a typo in the id
list would overwrite work irrecoverably. The 400 names the field and points at
`PATCH /api/nodes/{id}/cards/{cid}`.

### Deleting a batch — and why you probably want the move instead

```sh
curl -s -X DELETE -H "X-API-Key: $KEY" \
  -d '{"cards":[1901,1902,1903]}' $API/nodes/$NID/cards
# -> {"deleted":3,"cards":[1901,1902,1903]}
```

Validated in full first, because a half-finished delete cannot be undone by
re-sending it. There is deliberately **no** "everything in this basket" form: the
one batch operation you cannot walk back should not be reachable by leaving an
argument out. Scratch cards an agent made for itself are what this is for —
anything with reasoning worth keeping goes to `Archive` with the batch move above,
where it keeps its id.

### Acting on a card somebody named

The human pastes `[[#1391]]`, or you read an id out of the agenda. That is the
whole address:

```sh
# Mark it done and take its date off, so it leaves the agenda instead of
# sitting there forever under a due:: nobody can read.
curl -s -H "X-API-Key: $KEY" -d '{"key":"status","value":"done"}' $API/cards/1391/property
curl -s -X DELETE -H "X-API-Key: $KEY" "$API/cards/1391/property?key=due"

# Tick one line of a working list — the line is the task, not the card.
curl -s -H "X-API-Key: $KEY" -d '{"done":true}' $API/cards/1391/items/60/done

# Send it to the archive. It keeps its id, so [[#1391]] still resolves.
curl -s -H "X-API-Key: $KEY" -d "{\"node\":$ARCHIVE}" $API/cards/1391/move
```

No `GET /api/cards/1391` first to learn the basket, and nothing in the reply
mentions a node number the person you are working with never used.

**Since v0.142.0 that includes what the card *is*, not only what it says.** The
kind-specific ops take a bare id too, so a table someone pasted the id of is
editable without a lookup, and a card comes out as a note file the same way:

```sh
# Two table edits, applied in order — the batch form works by id as well.
curl -s -H "X-API-Key: $KEY" -H 'Content-Type: application/json' \
  -d '[{"op":"insert_row","at":1},{"op":"set_cell","row":1,"col":0,"text":"new"}]' \
  $API/cards/1391/table

# Attach the spec to the task card about it — the bytes ride in the document.
curl -s -H "X-API-Key: $KEY" \
  -d "{\"name\":\"spec.md\",\"data_base64\":\"$(base64 -w0 spec.md)\"}" \
  $API/cards/1391/attachments

# And one card out as a note file, frontmatter written from its properties.
curl -s -H "X-API-Key: $KEY" "$API/cards/1391/export?format=markdown"
```

Use the table op surface rather than rebuilding `rows`: a wholesale rewrite resets
every column width, which is a deliberate layout thrown away.

### Closing out a list of tasks the agenda handed you

```sh
# The agenda spans baskets, which is the point of it.
IDS=$(curl -s -H "X-API-Key: $KEY" "$API/tasks" \
  | python3 -c 'import json,sys; print(",".join(str(t["card"]) for t in json.load(sys.stdin)["tasks"]))')

# Read them all at once, wherever they live.
curl -s -H "X-API-Key: $KEY" "$API/cards?ids=$IDS"

# Mark them done and take their dates off, so they LEAVE the agenda rather than
# sitting there under a due:: nobody can read. Two calls, any number of baskets.
curl -s -H "X-API-Key: $KEY" \
  -d "{\"cards\":[$IDS],\"key\":\"status\",\"value\":\"done\"}" $API/cards/property
curl -s -X DELETE -H "X-API-Key: $KEY" \
  -d "{\"cards\":[$IDS],\"key\":\"due\"}" $API/cards/property
```

### Adding to a shared card without overwriting it

A card two of you write to — a message board, a running log, a handoff — is where
read-modify-write goes wrong: `GET` the body, add your line, `PATCH` it back, and
whatever the other one typed in between is gone. Append does it in one call, on the
server:

```sh
POST /api/nodes/{id}/cards/{cid}/append  {text, at?:"end"|"start", separator?}
POST /api/cards/{cid}/append             (same, by card id alone)
  → 200 {"card":<cid>, "at":"end", "added":<chars>, "body_len":<chars>}
  | 400 (empty text, a bad `at`, or a kind whose body is not its content)
  | 409 (the card mirrors a file — its body is read-only)
```

```sh
curl -s -H "X-API-Key: $KEY" \
  -d '{"text":"**2026-08-17 (agent)** — shipped v0.118.0."}' \
  $API/cards/275/append

# Newest-first boards: put it at the top, with a rule between entries.
curl -s -H "X-API-Key: $KEY" \
  -d '{"text":"**newest**","at":"start","separator":"\n\n---\n\n"}' \
  $API/cards/275/append
```

`separator` defaults to a **blank line** — a Markdown paragraph break, which is
what naive string concatenation gets wrong by running two paragraphs together. An
empty body takes the text with no separator at all. Send `""` to join with nothing.

**Refused where `body` is not what the card shows**, naming the route that works: a
checklist's lines and a table's cells are its content, and text appended to *their*
body is stored, displayed nowhere, and — the trap — not read as a property either,
because a checklist card's properties come from its title and items alone. A 200
that changed nothing anyone can see is the worst available answer.

### Building a basket in one call

```sh
curl -s -H "X-API-Key: $KEY" -d '[
  {"title":"Findings","body":"…","pos":[40,40],"fit":true},
  {"kind":"checklist","title":"Follow-ups","items":[{"text":"rotate the key","done":false}]},
  {"kind":"table","title":"Versions","rows":[["ver","date"],["0.114.2","2026-08-16"]],"header":true}
]' $API/nodes/$NID/cards
# -> {"created":3,"ids":[1840,1841,1842]}
```

The ids come back **in the order you sent them**, so a follow-up call can address
each card without a second lookup.

### Pointing someone at a group

A group has an id like a card does, and until v0.111.0 nothing could read it — the
only way to name one was to name a card inside it, which says *somewhere near here*
rather than naming the thing.

```sh
# Find a group from its id alone, and mint its address.
curl -s -H "X-API-Key: $KEY" $API/groups/146
# -> {"node":366,"node_path":"VolumePerApp","group":{"id":146,"title":…,"cards":[…]}}

curl -s -H "X-API-Key: $KEY" $API/groups/146/link
# -> {"wikilink":"[[#g146]]", "link":"trellis://127.0.0.1:7374/group/146", …}

# Move the whole group to another basket — container, members, title, colour AND
# id together. Moving its cards individually cannot do this: membership is
# basket-local, so each card arrives ungrouped and the rebuilt group gets a NEW
# id, breaking every [[#g…]] already written to it.
curl -s -H "X-API-Key: $KEY" -d '{"node":328}' $API/nodes/366/groups/146/move
```

Paste `[[#g146]]` into a card to link it. `GET /api/groups/146/backlinks` says what
points at it.

### Putting a basket on the desktop (Linux/X11)

Desktop mode is a **mode**: one call takes the whole basket out as real OS windows,
among the user's other applications, keeping the arrangement it has on the canvas.

```sh
curl -s -H "X-API-Key: $KEY" -d '' $API/nodes/$NID/desktop
# -> {"node":63,"desktop":true,"cards":[1536,1246,278]}

curl -s -H "X-API-Key: $KEY" $API/desktop      # what is out, and where
curl -s -X DELETE -H "X-API-Key: $KEY" $API/nodes/$NID/desktop   # all back

# One card on its own, if that is what you want:
curl -s -H "X-API-Key: $KEY" -d '{"pos":[760,430]}' $API/cards/1815/desktop
```

Only one basket is out at a time — turning a second on recalls the first. Placement
is **app config, per instance**: a screen coordinate belongs to one machine, so it
never travels with the document. Non-Linux returns **501**.

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

### Importing an Obsidian vault
```bash
curl -sX POST localhost:7373/api/import/vault -H "X-API-Key: $K" \
  -H 'Content-Type: application/json' \
  -d '{"path":"/home/you/Vault"}'
# → {"root":312,"baskets":9,"cards":74,"attachments":6,
#    "links_rewritten":58,"unresolved":["Some Note"]}
```
`parent` omitted makes it a new **root project**. Read `unresolved` — those are
links the vault referred to but did not contain; they are kept verbatim in the
cards so you can fix them. A `.canvas` becomes a **basket** keeping its
arrangement, colours, groups and connectors.

### Saving a query as a view card
The complement of asking `/api/query` again every time. Make an ordinary card,
then give it a `view`:
```bash
CID=$(curl -sX POST localhost:7373/api/nodes/42/cards -H "X-API-Key: $K" \
  -H 'Content-Type: application/json' \
  -d '{"title":"Blocked, soonest first","pos":[40,40],"size":[420,260]}' | jq .id)

curl -sX PATCH localhost:7373/api/cards/$CID -H "X-API-Key: $K" \
  -H 'Content-Type: application/json' -d '{"view":{
    "filters":[{"key":"status","op":"eq","value":"blocked"},
               {"key":"due","op":"le","value":"2026-09-30"}],
    "columns":["due","status","basket"],
    "sort":{"key":"due","dir":"asc"},
    "limit":50}}'

curl -s localhost:7373/api/cards/$CID/run -H "X-API-Key: $K"
# → {"card":…,"columns":["due","status","basket"],"count":7,"rows":[…]}
```
The rows are **computed on read** — nothing is stored on the card, so the view
cannot go stale. An unrelated `PATCH` leaves the view alone; `{"view":null}`
clears it. `GET /api/cards/{cid}` reports the `view` back, so you can read what
you wrote.

### Finding dates the app cannot read
Before trusting an agenda, check nothing fell off it silently:
```bash
curl -s localhost:7373/api/properties/problems -H "X-API-Key: $K"
# → {"count":1,"problems":[{"card":1391,"key":"due","value":"next",
#     "why":"\"next\" is not a date this app can read …"}]}
```
A `due::` that will not parse makes a card **look** scheduled while it never
reaches the Agenda. Note the value reported is what the parser **read** — a
date-shaped property stops at the first word, so `due:: next friday` is `next`.

### Showing one card inside another
```bash
curl -sX POST localhost:7373/api/cards/1400/append -H "X-API-Key: $K" \
  -H 'Content-Type: application/json' \
  -d '{"text":"Standing work:\n\n![[#1391]]"}'
```
`![[#1391]]` renders card 1391 inside this one; `[[#1391]]` only links to it.
`![[#1391^766]]` shows a single checklist line. The body on disk keeps the
`![[…]]` — expansion happens at render, so there is never a second copy to drift.

### Keeping a log — make the basket a feed

A running record — handoffs, ops checks, decisions — wants *newest first, no
navigation*, and the writer should never think about coordinates:

```sh
# Once: declare the basket a feed.
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"feed": true}' $API/nodes/$LOG

# Every entry after that is just a create — no pos, no column math, no
# overlap repair. The feed shows the newest entry at the top.
curl -s -H "X-API-Key: $KEY" -H 'Content-Type: application/json' \
  -d '{"kind":"text","title":"2026-08-26 — deploy check","fit":true,
       "body":"All four assets verified by name.\n#ops"}' \
  $API/nodes/$LOG/cards
```

The x/y arrangement underneath is preserved untouched — `{"feed": false}`
returns the canvas exactly as it was. Entries stay ordinary cards: link them
with `[[#id]]`, archive the finished ones, query them from the panels. The
feed sorts by **creation order**, so editing an old entry never moves it.

### Tag and un-tag a node

A node's `color` (the tag dot in the tree) and `bg` (the basket background)
follow the same null-vs-absent rule as a card's `view` (since v0.161.0 — before
that, `null` was accepted and silently stored nothing):

```sh
# A color sets it — array, hex or name, like every other color input.
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"color":"teal","bg":[16,24,32]}' $API/nodes/$NID

# An explicit null CLEARS: no tag dot / back to the theme-default canvas.
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"bg":null}' $API/nodes/$NID

# An absent field is left alone — this touches only the title.
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"title":"Renamed"}' $API/nodes/$NID
```

### Reading what failed

`api_errors` on `/api/instance` above zero means calls have been refused this
run. Read them — yours and everyone else's — before you report:

```sh
# Every failure this run, oldest first: who, what, why, and what they sent.
curl -s -H "X-API-Key: $KEY" "$API/errors" | jq '.errors[] | {seq,status,agent,method,path,error,request}'

# Only what happened since you last looked — keep `newest` from the last answer.
curl -s -H "X-API-Key: $KEY" "$API/errors?since=$LAST"

# The same record on disk, for last week: one JSON object per line.
tail -n 20 ~/.local/share/trellis/trellis/api-errors.log | jq -c '{ts,status,agent,path,error}'
```

A 400 you did not notice is a card you think you wrote. The `request` excerpt is
usually the whole diagnosis — `{"bg": null}` against a build older than v0.161.0,
`{"body": ""}` from a response read at the wrong level — and `agent` says whose
prompt needs the fix.

### Everything else

```sh
KEY=<your key>
API=http://127.0.0.1:7373/api

# Confirm which document this port is serving before writing to it — with an
# instance per document (work on 7373, personal on 7374), the port is the address.
curl -s -H "X-API-Key: $KEY" $API/instance
# → {"app":"trellis","document":"work.ron","path":"/home/you/work.ron","port":7373,
#    "nodes":42,"unsaved_changes":false,"stale_claims":3}
#
# `stale_claims` above zero means this workspace is asserting things nobody has
# confirmed lately. Find out WHICH before quoting any of it back:
curl -s -H "X-API-Key: $KEY" "$API/claims?expired=true"
# → {"count":3,"stale":3,"claims":[
#      {"card":1246,"title":"Next steps — read this first","verify":"2026-08-14",
#       "check":"GET /api/instance","bucket":"expired",
#       "node_path":"Trellis › Trellis Open Items"}, …]}
#
# Re-check it, correct the card, and push the date out — the property endpoint
# edits one line rather than rewriting the body:
curl -s -X POST -H "X-API-Key: $KEY" \
     -d '{"key":"verify","value":"2026-09-15"}' $API/nodes/63/cards/1246/property
#
# Mark a card of your own as a claim when you write one. Anything that states
# how something IS — a version, a count, what somebody owes you — earns these
# two lines, and the `::` needs its trailing space:
#   verify:: 2026-09-15
#   check:: GET /api/instance

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

# Say "look at this" without taking the accent colour, which is how the basket is
# organised. Two looks: `glow` (steady) and `pulse` (a slow breath — there is no
# flash, because above ~3 Hz it is a seizure risk).
curl -s -X PATCH -H "X-API-Key: $KEY" \
  -d '{"emphasis":"pulse","emphasis_intensity":0.9,"emphasis_minutes":30}' \
  $API/nodes/$NID/cards/$CID
# → {"emphasis":"pulse","emphasis_intensity":0.9,"emphasis_until":1786741084,
#    "emphasis_live":"pulse", …}
#
# **Always send `emphasis_minutes` from an agent.** Emphasis that never expires
# accumulates until every card is shouting and none of them mean anything; the
# expiry is what keeps the channel worth having. It is evaluated when the card is
# drawn, so a lapsed highlight costs no edit, no `touched` and no change-log entry.
# Omit it (or send 0) only for a highlight a *person* asked for.
#
# Turn it off — this also clears any expiry, so no stale timer is left behind:
curl -s -X PATCH -H "X-API-Key: $KEY" -d '{"emphasis":"none"}' $API/nodes/$NID/cards/$CID
# An unknown value is refused rather than ignored:
#   {"error":"invalid JSON body: unknown emphasis \"strobe\" (expected \"none\", \"glow\" or \"pulse\")"}

# Read the app's settings, and change some. Instance settings (per --data-dir),
# so work and personal each have their own.
curl -s -H "X-API-Key: $KEY" $API/settings
curl -s -X POST -H "X-API-Key: $KEY" \
  -d '{"theme":"Blueprint","tree_sort":"name","notify_digest":true}' $API/settings
# → 200 with every setting as it now stands, not just the ones you sent.
#
# `tree_sort` orders the ROOT projects only and is a view: the document keeps its
# own order, so a project added later appears in place instead of at the bottom.
curl -s -X POST -H "X-API-Key: $KEY" -d '{"tree_sort":"tasks"}' $API/settings
#
# Scope a panel to one project, or clear it with null:
curl -s -X POST -H "X-API-Key: $KEY" -d "{\"kanban_project\":$NID}" $API/settings
curl -s -X POST -H "X-API-Key: $KEY" -d '{"kanban_project":null}' $API/settings
#
# A typo is refused by name rather than ignored:
#   {"error":"unknown setting \"thmee\". Settable: theme, tree_sort, …"}
# So is the API key, port, LAN flag and mirror policy — a caller must not be able
# to widen its own reach. Those stay in Tools → Settings.

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

# Group cards 1 and 2 into a container (color is optional, same formats as cards)
curl -s -H "X-API-Key: $KEY" -d '{"cards":[1,2],"title":"Cluster","color":"#8a4fff"}' $API/nodes/$NID/groups

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
- **A log-style basket should be a feed, and then placement is free:** where a
  basket is a chronological record — session handoffs, ops checks, a release
  log — set `PATCH /api/nodes/{id} {"feed": true}` once and stop doing
  position math for ever: `POST` the card with no `pos` and the feed shows it
  first. (Before feeds the convention was *append below the lowest card in the
  column, never at `[40,40]`, then check `…/overlaps`* — that recipe is still
  right for a chronological basket someone wants kept as a **canvas**, but a
  feed is the better answer and the reader lands on the newest entry.)
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
