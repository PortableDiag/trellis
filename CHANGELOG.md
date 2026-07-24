# Changelog

All notable changes to Trellis. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions are the app version in
`Cargo.toml`, each with a matching git tag and GitHub release.

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
