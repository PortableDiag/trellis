# Trellis

A hierarchical, spatial note-taking app for the desktop, written in Rust.

Two proven ideas, woven together:

- **The tree** — an outliner-style hierarchy of nodes for structure and navigation.
- **The basket** — each node's body is not a linear document but a free-form
  2-D canvas where you drop, drag, resize, and arrange rich cards.

Structure lives in the tree; spatial thinking lives in the basket. A trellis is
a lattice that supports branching growth — the tree *and* the weave in one.

![Trellis — the tree on the left, a basket of cards on the right](assets/trellis-welcome.png)

## Features

**Tree**
- Add root / child / sibling nodes; inline rename (double-click); delete subtrees
- Reorder siblings (move up/down), indent / outdent to reshape the hierarchy
- Expand / collapse, per-node color tags, and a per-node **basket background
  color** (right-click → **Basket color**; the black grid stays the default)
- Right-click → **Copy** a node's **id** (for the agent API, `/api/nodes/{id}`)
  or its **path** breadcrumb, so you can point an agent at the exact node

**Basket canvas** — six real card types:
- **Text** — CommonMark markdown, rendered live, with edit/preview toggle. Fenced
  code blocks are syntax-highlighted. The editor has a formatting toolbar (bold,
  italic, headings, lists, quotes, code, links, rules), a **text color** picker
  whose color shows live in the rendered card, a **font-size** selector
  (75%–200%, per card), and **auto-continuing lists** (Enter adds the next
  `-`/`1.`/`- [ ]` marker; empty item ends the list).
- **Code** — dedicated code editor with a language selector and highlighting.
- **Checklist** — real checkboxes with add/remove/edit per item; drag the grip
  to reorder items.
- **Table** — a small spreadsheet: inline cell editing, insert/delete/resize
  rows and columns (right-click the row/column handles), per-cell **background
  and font colors**, an optional header row, and **CSV/XLSX import & export**
  (XLSX export keeps your colors). The copy button copies the table as CSV.
- **Sketch** — a freehand draw surface: pick a **brush color and size**, draw
  with the mouse/pen, **undo the last stroke** or **clear**. Strokes are vector
  (they scale with zoom and export to HTML as inline SVG).
- **Image** — hold **any number of images** (bytes embedded), laid out in a
  grid; give the card a **title** to tell a few apart. **Double-click an image**
  to open it in a full-screen viewer — scroll or `+`/`-` to zoom, drag to pan,
  `←`/`→` (keys or buttons) to flip through the card's images, Esc to close.
  Right-click an image to remove it. **Right-click the card → Extract text (OCR)**
  reads the text out of the image(s) with tesseract so the card becomes
  full-text searchable. **Right-click → Download** saves an image back out to a
  file (or **All N images…** into a folder) to share or reuse.

Cards drag by the title bar, resize from the corner, raise to front on click,
duplicate, recolor, copy/paste into another basket, and delete. Right-click →
**Export Card** saves just that card to share — **PNG**, **Markdown**, **PDF**,
**HTML**, **plain text**, or a portable **JSON card file** for any card, plus
**CSV/XLSX** for tables and **SVG** for sketches (image cards export the image
itself). Bring a card in with right-click canvas → **Import card…** or by
**dragging a JSON card file** onto the canvas. A 🗐 button on
the title bar copies the card's text (checklists as Markdown task lines) to
both the clipboard and the X11 primary selection. Right-click → **Copy** →
**Card id** / **Card path** copies the card's identifier or breadcrumb, so you
can point an agent at that exact card. The canvas pans
and zooms (Ctrl+scroll); each node remembers its view.

**Organizing cards**
- **Group** — Ctrl/Cmd+click cards to multi-select, then "Group N cards" wraps
  them in a labeled container you drag as one; right-click the header to rename,
  recolor, or ungroup. **Click a group's header to raise the whole group to the
  front** — the header stays grabbable even when other cards pile on top of it.
- **Dock** (toggle) — drag one card onto another to stick them so they move
  together; drag a docked card off to detach.
- **Snap** (toggle) — a dragged card's edges snap to nearby cards' edges, with a
  guide line.
- **Autosort** — **Tools → Autosort cards** lays the whole basket out in a tidy,
  non-overlapping grid.

**Documents & interop**
- **Drag & drop** text/Markdown or image files onto a basket to create the
  matching card at the drop point
- Native New / Open / Save / Save As. **Autosave** (Tools → Settings → Document,
  on by default) writes changes a couple of seconds after you pause, like Google
  Docs — debounced, atomic (temp file + rename), and run on a background thread so
  it never freezes the UI. Files are **gzip-compressed** (RON format, `.ron`;
  image-heavy documents shrink dramatically) and older plain-text `.ron` files
  still open.
- **File → Export** the whole tree as **Markdown**, styled **HTML**, **JSON**,
  **PDF** (paginated A4), or a **PNG/GIF** image
- **File → Import** **Markdown**/**HTML** as a new node, or a **JSON**-exported document

**App**
- Full-text **search** across every node title and card (Ctrl+F)
- **View → Themes** — Trellis (default), Light, or Terminal Green
- **Zoom** the whole UI (Ctrl+`+` / Ctrl+`-` / Ctrl+`0`)
- **Agent API** — a key-gated HTTP API (localhost by default; opt-in **LAN
  access** in Settings for phones/other devices) with full parity to the app:
  add/query/edit/remove nodes and cards, move/recolor/resize, convert a card's
  kind, edit tables cell-by-cell (colors, headers, rows/cols), upload images,
  build groups, join/leave and dock cards, and export the document (incl.
  PDF/PNG) — so agents can collaborate on the same notes. **Live updates:**
  `GET /api/wait` long-polls so clients react the instant anything changes. Enable
  it in **Tools → Settings**; see [API.md](API.md).
- **Companion mobile app** — a native Android viewer/capture app talks to the
  agent API over the LAN: browse the tree and baskets, full-text search, zoom
  images, and capture a note or photo into a node, all updating live. Separate
  repo: [trellis-android](https://github.com/PortableDiag/trellis-android).

## Keyboard

| Shortcut | Action |
|---|---|
| Ctrl+Z / Ctrl+Shift+Z | Undo / redo canvas edits (moves, autosort, …) |
| Ctrl+S | Save |
| Ctrl+F | Toggle search |
| Ctrl+N | New document |
| Ctrl+`+` / `-` / `0` | Zoom in / out / reset |
| Ctrl+scroll | Zoom (toggle in Settings; on by default) |

## Build & run

```sh
cargo run --release
```

Requires a recent stable Rust toolchain. Tests: `cargo test` (binary crate — use
`cargo test --bin trellis` to test a single target). Middle-click paste and the
X11 PRIMARY-selection features need `xclip` or `xsel` installed. **OCR** (right-click
an image card → Extract text) needs the `tesseract` CLI (`tesseract-ocr`) installed.

The markdown renderer (`egui_commonmark`) is vendored under `vendor/` and patched
to render inline text-color spans; edit it there, not the crates.io copy.

## Docs

- [API.md](API.md) — the localhost agent HTTP API.
- [CHANGELOG.md](CHANGELOG.md) — version history.

## License

MIT. Vendored `egui_commonmark` / `egui_commonmark_backend` are MIT/Apache-2.0
(see `vendor/*/LICENSE-MIT`).
