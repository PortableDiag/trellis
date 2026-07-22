# Changelog

All notable changes to Trellis. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions are the app version in
`Cargo.toml`, each with a matching git tag and GitHub release.

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
