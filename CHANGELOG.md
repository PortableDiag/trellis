# Changelog

All notable changes to Trellis. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions are the app version in
`Cargo.toml`, each with a matching git tag and GitHub release.

## [0.42.0]

### Fixed
- **Basket PDF/PNG overview now fits the whole basket.** The overview page (and the PNG
  export) captured the *current* on-screen view — so a basket bigger than the window came
  out cropped, showing only the cards that happened to be visible. It now zooms out to fit
  **all** cards in the basket. (Cause: the framebuffer egui captures is the frame *after* the
  screenshot is requested, so the fit-all reframe has to stay applied for that extra frame;
  it was being dropped too early. This also fixes far-off-screen cards being missing from the
  per-card pages.)

## [0.41.0]

### Added
- **Card templates** — reuse a card you make often (a "Today's Todos" checklist, a standard
  table, a heading block). Right-click a card → **Save as template**, then anywhere,
  right-click the canvas → **Insert template ▸** to drop a fresh copy at the click point.
  Templates are saved in app config (they persist across restarts) and carry the card's full
  content, colors, size and font. Delete one from the same submenu (✕ next to its name).

## [0.40.0]

### Added
- **Visual basket export — PNG & WYSIWYG PDF.** Right-click a tree node → **Export basket**
  → **PDF (visual)** or **PNG (overview)**.
  - **PDF** leads with a whole-basket **overview page** (your spatial layout, captured as it
    looks on screen), then a **readable page per card** — each the card rendered WYSIWYG
    (formatting, colors, checkboxes, tables, images, sketches) **plus the card's text as a
    real selectable/searchable layer**, so the file is both visual and copy/search-friendly.
    Long card text flows onto further pages.
  - **PNG** is a single image of the whole basket exactly as arranged.
  - Built by briefly framing the basket (then each card) and screenshotting the framebuffer;
    the view you were on is restored afterward. Complements the Markdown/HTML/JSON basket
    export from 0.39.0.

## [0.39.0]

### Added
- **Export / import a single basket** — right-click a tree node → **Export basket** (just
  that node's cards) or **Export basket + subnodes** (the whole subtree), in **Markdown**,
  **HTML**, or **JSON**. The JSON is a portable, self-contained basket file (image bytes
  embed inline; card positions/colors preserved) — hand someone a single day's notes
  without exporting your whole archive or going card-by-card. Bring one in with right-click
  a node → **Import basket…**, which adds it as a child (fresh ids; workspace-only
  grouping/dock links are dropped). *(Visual PNG + WYSIWYG PDF of a basket land next.)*

## [0.38.0]

### Added
- **Fit a card to its content** — right-click a card → **Fit to content** resizes it
  so its text/items/table are fully readable, instead of leaving an unreadable little
  square. On the agent API, pass `"fit": true` on card **create** (`POST …/cards`) or
  **update** (`PATCH …/cards/{cid}`); it's applied after all other fields and overrides
  `size`. Works for text, code, checklist, table and sketch cards (image cards keep the
  size their pictures dictate). This makes API/agent-created cards come out readable by
  default.

### Changed
- **Autosort now auto-sizes too** — **Tools → Autosort cards** (and `POST …/autosort`)
  first fits every card to its content, then packs them into a tighter grid whose
  columns/rows follow the actual card sizes (no more one-size-fits-largest cells). The
  result is both tidy *and* readable.

## [0.37.0]

### Changed
- **Export Card → PNG and PDF are now WYSIWYG** — they capture the card exactly as
  it looks on screen (formatted markdown, colors, syntax highlighting, checkboxes,
  table cell colors, images, sketches) instead of a plain-text rendering. Done by
  briefly framing the card, screenshotting the framebuffer, and cropping to the
  card; the PDF wraps that image on a page sized to the card. Markdown / HTML /
  plain-text / JSON / CSV / XLSX / SVG exports are unchanged.

## [0.36.0]

### Added
- **Export / import a card as JSON** — a portable, self-contained card file for
  handing one card to someone else (or moving it between workspaces). Right-click a
  card → **Export Card** → **JSON (card file)** writes it (image bytes embed inline;
  markdown/code body included). To bring one in: right-click the canvas → **Import
  card…** (just above *Paste card*) to pick a `.json` file, **or drag the `.json`
  onto the canvas**. Imported cards get a fresh id and land at the click/drop spot;
  a `.json` that isn't a valid Trellis card falls back to a plain text card, and the
  file is validated by a `"format": "trellis-card"` marker.

## [0.35.0]

### Added
- **Export a single card** — right-click a card → **Export Card** → pick a format,
  so you can share one card without exporting the whole workspace and cropping.
  Available for every card: **PNG** (image render), **Markdown (.md)**, **PDF**,
  **HTML** (formatted), and **Plain text (.txt)**. Kind-specific extras: **CSV** and
  **Excel (.xlsx)** for tables, **SVG** for sketches. Image cards export the image
  itself (re-encoded PNG); sketches rasterize their strokes (PNG) or export vector
  **SVG**; text/code/checklist/table cards render their content. The save dialog
  pre-fills the card's title as the filename.

### Internal
- The whole-document HTML / Markdown / PDF / image exporters were refactored onto
  shared per-card primitives (same output; now reused for single-card export).

## [0.34.0]

### Added
- **Download images from a card** — right-click an image card → **Download image…**
  (single image) or a **Download** submenu (multi-image cards: pick one by name, or
  **All N images…** to save the whole set into a chosen folder). Files keep their
  original name/extension; nameless images save as `image-N.png`, and a folder save
  de-duplicates names so nothing is silently overwritten. Makes it easy to pull an
  image back out of your notes to share or reuse. (Agents can already fetch image
  bytes via `GET /api/nodes/{id}/cards/{cid}/images/{idx}`.)

## [0.33.1]

### Fixed
- **UI freeze during autosave on large documents** — saving serialized + gzipped +
  wrote the whole document on the UI thread, which froze rendering for seconds on a
  big (image-heavy) file. Saves now run on a background thread; the UI only pays a
  quick document clone. `dirty` clears only if nothing changed while the save ran.

## [0.33.0]

### Added
- **OCR image cards** — right-click an image card → **Extract text (OCR)** reads the
  text out of the image(s) with tesseract and stores it (hidden) on the card, so
  screenshots, scans and photos of documents become **full-text searchable**. Runs
  in the background (never freezes the UI). Requires the `tesseract` CLI installed
  (`tesseract-ocr`); the extracted text is also reported as `ocr` in the card's API
  JSON.

## [0.32.0]

### Changed
- **Documents are saved gzip-compressed** — embedded images were serialized as
  decimal byte arrays, which the old pretty-printed RON bloated ~32×. Saves now use
  compact RON + gzip, which shrinks that back to near the raw image size (measured
  ~27× smaller). A large image-heavy document that was ~170 MB becomes single-digit
  MB — and every save/autosave writes that much less to disk. **Older plain-text
  `.ron` files still open** (the loader detects the format); the next save rewrites
  them compressed. File extension is unchanged.

## [0.31.0]

### Added
- **Autosave** (Tools → Settings → Document, **on by default**) — changes are written
  to disk a couple of seconds after you stop editing, like Google Docs. Debounced so
  a drag or a burst of typing never saves mid-gesture, and written **atomically**
  (temp file + rename) so a crash or kill can't corrupt the document. Turn it off to
  save manually with Ctrl+S (changes are still saved on exit either way).

### Changed
- Manual saves are now atomic too (temp file + rename).

## [0.30.2]

### Fixed
- **Crash on any edit (regression in 0.30.0/0.30.1)** — `mark_dirty` recursed into
  itself instead of setting the dirty flag, so the first change of any kind (moving a
  card, typing, adding a node, or an API write) overflowed the stack and aborted.
  **Upgrade from 0.30.0/0.30.1.**

## [0.30.1]

### Added
- **About shows the version** — the Help → About Trellis dialog now lists the running
  version.

## [0.30.0]

### Added
- **Live updates for API clients** — `GET /api/wait?rev=<n>` long-polls: it blocks
  until the document changes (or ~25 s), then returns the new revision. Clients loop
  on it to be woken the instant anything changes, instead of polling on a timer. The
  Trellis mobile app uses this for near-instant updates. Each API request is now
  handled on its own thread, so a long-poll never blocks other requests.

### Changed
- **LAN access applies immediately** — toggling **Tools → Settings → LAN access** now
  rebinds the API server on the spot (no relaunch needed). The status line updates to
  show the reachable URL.

## [0.29.0]

### Added
- **API: read image bytes** — `GET /api/nodes/{id}/cards/{cid}/images/{idx}` returns
  `{index, name, base64}` for an image card's image, so the mobile viewer (and
  agents) can fetch the actual picture, not just its name. `API.md` updated.

### Fixed
- **Checklist controls are edit-only** — the drag-to-reorder grip, the × delete
  button, and "+ item" now appear only when the checklist card is in **edit** mode.
  In view mode you get just the checkboxes (still tickable) and read-only item text,
  so you can't accidentally move or delete items while using the list.

## [0.28.0]

### Added
- **LAN access for the agent API** — a **Tools → Settings → LAN access** toggle
  binds the API to all interfaces (`0.0.0.0`) instead of localhost, so other
  devices on your network (e.g. a phone, or the forthcoming Trellis mobile
  viewer) can reach it. Still key-gated; the status line and curl example show
  the reachable LAN URL. Off by default; applies on restart. Only enable on
  trusted networks — never expose to the internet without a TLS proxy.

## [0.27.1]

### Changed
- **About dialog** now carries the tagline *"The tree and the weave."*
- **README** shows a screenshot of the default welcome workspace.

## [0.27.0]

### Added
- **Per-basket background color** — right-click a tree node → **Basket color** to
  tint that node's canvas; the grid still draws on top. **Default** restores the
  standard black-grid look (which stays the default for every node). On the agent
  API: `bg` on node JSON and settable via `PATCH /api/nodes/{id}` (`bg` accepts the
  same flexible color input as other colors).
- **View → Themes** submenu — the theme picker is now a named **Themes** menu. The
  current signature look is the **Trellis** theme (the default; unchanged), with
  **Light** and **Terminal Green** alongside. Scaffolding for richer themes
  (e.g. StickyNotes, Futuristic) to follow.
- **Copy card id / path** — right-click a card → **Copy** → **Card id** or **Card
  path** (its node breadcrumb › card title), so you can point an agent at that
  exact card (`/api/nodes/{node}/cards/{id}`). Mirrors the tree's node id/path copy.

## [0.26.0]

### Added
- **Sketch / draw card** — a freehand drawing surface. Pick a brush **color** and
  **size**, draw with the mouse/pen, **undo the last stroke** or **clear**. Edit
  vs view toggle like other cards. Strokes are vector: they scale with zoom, are
  stored in card-local coordinates, and **export to HTML as inline SVG** (Markdown/
  PDF note a stroke count). On the agent API: create with `kind:"sketch"`, read a
  card's `strokes`, and draw via `POST …/cards/{cid}/sketch` (`add_stroke` /
  `undo` / `clear`). `API.md` updated.

## [0.25.1]

### Fixed
- **Checklist item delete** — the `×` delete button was pushed outside the card
  by the full-width item field, so it couldn't be clicked (you could only clear
  the text). The row now reserves space for `×`, so deleting an item removes the
  whole line and checkbox.

## [0.25.0]

### Added
- **Undo / redo** (`Ctrl+Z` / `Ctrl+Shift+Z` / `Ctrl+Y`, also **Edit → Undo/Redo**)
  for canvas edits: card moves and resizes, autosort, add/remove/duplicate/paste,
  color, font size, grouping, docking, image and table structural edits. A whole
  drag collapses into a single undo step. History is per-basket and light
  (snapshots one node, not the whole document); it defers to egui's built-in
  text-field undo while you're typing in a card.

## [0.24.0]

### Added
- **Reorder checklist items** — each item has a drag grip (`⠇`); drag it onto
  another row to reorder, with a drop-line indicator. (Agents reorder by sending
  a checklist's `items` in the new order.)
- **Tools → Autosort cards** — lay every card in the current basket out in a
  tidy, non-overlapping grid (clustered by group; docking cleared). Also on the
  API: `POST /api/nodes/{id}/autosort`.
- **API: font size** — `font_scale` is now settable on card create and PATCH and
  reported in card JSON, exposing the per-card font-size feature to agents.

`API.md` updated for `font_scale`, the autosort endpoint, and checklist reorder.

## [0.23.0]

### Added
- **Per-card font size** — a size selector (`A 100%`) in the text and code card
  toolbars sets that card's body font (75%–200%), applied in both edit and
  rendered views. Stored per card (old documents default to 100%).
- **Drag & drop files** — drop `.txt`/`.md` (or any UTF-8 text) and image files
  (png/jpg/gif/bmp/webp) onto a basket to create the matching card at the drop
  point (text cards get the file contents, images embed the bytes). Multiple
  files fan out; a highlight hint shows while files hover.

## [0.22.0]

### Added — full agent-API parity for cards
Agents can now do everything the GUI can to a card:
- **Convert a card's kind** via `PATCH` (`kind`) — text/code/checklist/table/image;
  kind-specific fields in the same PATCH land in the converted card.
- **Rich table editing** — `POST …/cards/{cid}/table` ops: `set_cell`, `set_bg`,
  `set_fg` (cell colors), `insert_row`/`remove_row`, `insert_col`/`remove_col`,
  `set_col_width`, `set_header`. Plus `header` on the card `PATCH`.
- **Image bytes** — `POST …/cards/{cid}/images` (base64) to attach real images,
  `DELETE …/images/{idx}` to remove one, and `image_base64` on card create.
- **Group join/leave** — `POST`/`DELETE …/cards/{cid}/group` to add an existing
  card to an existing group or remove it (beyond create-new-group).

`API.md` documents every new endpoint, field, and table op with examples.

## [0.21.2]

### Fixed
- **API color names match the palette** — the agent API now accepts every color
  name in the app's 16-swatch palette (adds `lime`, `indigo`, `stone`, and
  splits `orange`/`amber`, `teal`/`cyan` to their true swatch colors). Previously
  a swatch name like `"lime"` returned `400`. `API.md` lists the full set.

## [0.21.1]

### Fixed
- **Group header z-order** — the header no longer bleeds through cards on
  hover. Its interaction sits behind the cards, so only the visible part of a
  header responds; clicking that visible part raises the whole group (all its
  cards) to the front, and the header lifts above the cards only while you're
  actually dragging it.

## [0.21.0]

### Added
- **Editable checklist titles** — checklist cards now have the edit/view toggle
  (and double-click-title), so you can name them like every other card kind.
- **Bigger color palette** — the card, group and node color menus now share a
  16-swatch palette (red → black) shown as a grid of color chips instead of the
  old six named buttons.

### Changed
- **Group headers stay grabbable** — a group's header handle is now interacted
  above the cards, so you can grab it even when cards pile on top; **clicking a
  header raises the whole group to the front**, and the header you're hovering
  or dragging is drawn on top so it's visible while in use.

## [0.20.0]

### Fixed
- **Agent API: card color on create** — `POST …/cards` now accepts `color` (and
  `size`). Previously these were silently dropped at creation, so an agent that
  "set a card red" on create saw success but no color change.

### Changed
- **Agent API: flexible color input** — every `color` field (nodes, cards,
  groups, create or update) now accepts an `[r,g,b]` array, a hex string
  (`"#ef4444"`, `"#e44"`), or a color name (`"red"`, `"green"`, …). An
  unrecognized color returns `400`, so success means it was applied. Docs
  updated; `POST …/cards` documents `table`/`image` kinds and `size`/`color`.

## [0.19.0]

### Added
- **Copy node id / path** — right-click a tree node → **Copy** → **Node id**
  (the identifier the agent API uses, `/api/nodes/{id}`) or **Node path** (the
  root-to-node breadcrumb, e.g. `HOUSE › ATTIC › VELUX WINDOW`). Both copy to the
  clipboard and the X11 primary selection, so you can tell an agent exactly which
  node you're working on.

## [0.18.1]

### Fixed
- **Image viewer** — scroll-wheel zoom now zooms toward the pointer instead of
  the image center, so you can zoom into the top (or any edge) of a long
  screenshot without fighting the pan.

## [0.18.0]

### Added
- **Table cards** — a spreadsheet card type: grid of cells with inline editing,
  insert/delete rows and columns via the row-number / column-letter handles,
  draggable column widths, optional header row, and per-cell **background and
  font colors**. **Import and export CSV/XLSX** from the card's edit toolbar
  (XLSX export preserves colors). Tables flow through HTML/Markdown/PDF/image
  export, full-text search, the title-bar copy button (as CSV), and the agent
  API (`kind: "table"`, `rows` in card JSON and PATCH).

## [0.17.1]

### Fixed
- Cards added from the right-click menu now appear at the spot you right-clicked.
  Previously the click position was lost by the time a menu item was chosen, so
  new cards landed at the canvas origin ("the top area"). If the position is
  ever unavailable, new cards fall back to the center of the visible canvas
  instead of the origin. (Double-click already placed text cards correctly.)

## [0.17.0]

### Added
- Image cards can hold **multiple images**, shown as a grid ("add image" appends;
  right-click an image to remove it; removing all returns the card to the
  "Load image…" state). Existing single-image documents load unchanged.
- **Full-screen image viewer**: double-click any image in a card to open it in a
  shadowbox — scroll or `+`/`-` to zoom, drag to pan, `←`/`→` (keys or on-screen
  buttons) to move through the card's images, double-click to toggle fit/200%,
  Esc / `×` / backdrop click to close.
- Exports (HTML/PDF/PNG/Markdown), full-text search, and the API card JSON
  (`image_names`) now cover all images of a card.

## [0.16.2]

### Fixed
- Crash (stack overflow) when opening any file dialog — the v0.16.1
  dialog-parenting helpers accidentally called themselves recursively.

## [0.16.1]

### Fixed
- File and message dialogs (Open, Save As, Import/Export, Load image…) are now
  parented to the main window, so they no longer open behind the app.

## [0.16.0]

### Added
- Copy button (🗐) on card title bars, left of the edit/view toggle: copies
  the card's text — Text/Code bodies as-is, checklists as Markdown task lines —
  to **both** the system clipboard and the X11 PRIMARY selection (middle-click
  paste). Image cards have no text and no copy button.

## [0.15.0]

### Changed
- Inline text-color spans (`<span style="color:#rrggbb">`) now render **live** in
  the card view, not only in PDF/HTML export. `egui_commonmark` and its backend are
  vendored under `vendor/` (MIT/Apache-2.0) and patched to honor color spans;
  all other markdown rendering is unchanged.

## [0.14.1]

### Fixed
- The numbered-list toolbar button now numbers a multi-line selection `1.`, `2.`,
  `3.`… instead of prefixing every line with `1.`.

## [0.14.0]

### Added
- Auto-continuing lists: pressing Enter on a list line in the body editor inserts
  the next marker — numbered (`1.` → `2.`, also `1)`), bullets (`-`/`*`/`+`), and
  task items (`- [ ]`), preserving indentation. Enter on an empty item ends the
  list. Shift+Enter still inserts a plain newline.

## [0.13.0]

### Added
- **Export** the whole document to **PDF** (paginated A4) and to **PNG/GIF**
  (a rendered image), alongside Markdown/HTML/JSON. File → Export.
- **Agent API** brought to full feature parity:
  - Card `PATCH` also accepts `color`, `lang`, `pos`, `size`, checklist `items`,
    and returns the updated card; card JSON reports `pos`, `size`, `color`,
    `group`, `docked_to`.
  - Groups: `GET/POST/PATCH/DELETE /api/nodes/{id}/groups[/{gid}]`.
  - Docking: `POST/DELETE /api/nodes/{id}/cards/{cid}/dock`.
  - Export: `GET /api/export?format=markdown|html|json|pdf|png|gif`.
  - Node JSON includes its groups. `API.md` fully updated.

### Dependencies
- Added `printpdf` and `ab_glyph` (text is embedded with the bundled DejaVuSans).

## [0.12.0]

### Added
- **Snap** mode: a toggle (canvas button + Settings, persisted) that snaps a
  dragged card's edges to nearby cards' edges, with an amber guide line.

## [0.11.0]

### Added
- **Groups**: Ctrl/Cmd+click cards to multi-select, then the "Group N cards" button
  wraps them in a labeled container you drag by its header; right-click to rename,
  recolor, or ungroup.
- **Docking** (toggleable "Dock" mode): drag one card onto another to stick them so
  they move together; a green target highlight while dragging, a dot on a docked
  card, and a connector line. Drag a docked card off to detach. Cycle-safe.

### Data model
- `Card` gains `group` and `docked_to`; `Node` gains `groups`; all `#[serde(default)]`
  so existing documents load unchanged.

## [0.10.0]

### Added
- Editable titles on **image cards** (double-click the title bar / edit toggle),
  to tell a few images apart.

### Changed
- Single newlines now render as line breaks (a `hard_wrap` pass adds Markdown hard
  breaks, skipping fenced code blocks), in both the live viewer and the HTML export.

## [0.9.0]

### Added
- Text **color picker** in the editor toolbar: select text, pick a color, and it is
  wrapped in an inline color span (renders in export; see 0.15.0 for live rendering).

## [0.8.1]

### Fixed
- Middle-click (X11 primary-selection) paste and selection mirroring now work in
  singleline fields — card **title**, code **lang**, and **checklist items** —
  matching the body editor. Requires `xclip`/`xsel`.

## [0.8.0] and earlier

Copy/paste cards between baskets, File Import/Export submenus, X primary-selection
sync, reorder mode, color schemes, tree drag-and-drop reorder, and the core tree +
basket app. See the git history for details.
