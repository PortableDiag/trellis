# Trellis

A hierarchical, spatial note-taking app for the desktop, written in Rust.

Two proven ideas, woven together:

- **The tree** — an outliner-style hierarchy of nodes for structure and navigation.
- **The basket** — each node's body is not a linear document but a free-form
  2-D canvas where you drop, drag, resize, and arrange rich cards.

Structure lives in the tree; spatial thinking lives in the basket. A trellis is
a lattice that supports branching growth — the tree *and* the weave in one.

## Features

**Tree**
- Add root / child / sibling nodes; inline rename (double-click); delete subtrees
- Reorder siblings (move up/down), indent / outdent to reshape the hierarchy
- Expand / collapse, per-node color tags

**Basket canvas** — four real card types:
- **Text** — CommonMark markdown, rendered live, with edit/preview toggle. Fenced
  code blocks are syntax-highlighted. The editor has a formatting toolbar (bold,
  italic, headings, lists, quotes, code, links, rules), a **text color** picker
  whose color shows live in the rendered card, and **auto-continuing lists**
  (Enter adds the next `-`/`1.`/`- [ ]` marker; empty item ends the list).
- **Code** — dedicated code editor with a language selector and highlighting.
- **Checklist** — real checkboxes with add/remove/edit per item.
- **Image** — pick a file from disk (bytes embedded); give it a **title** to tell
  a few apart.

Cards drag by the title bar, resize from the corner, raise to front on click,
duplicate, recolor, copy/paste into another basket, and delete. The canvas pans
and zooms (Ctrl+scroll); each node remembers its view.

**Organizing cards**
- **Group** — Ctrl/Cmd+click cards to multi-select, then "Group N cards" wraps
  them in a labeled container you drag as one; right-click the header to rename,
  recolor, or ungroup.
- **Dock** (toggle) — drag one card onto another to stick them so they move
  together; drag a docked card off to detach.
- **Snap** (toggle) — a dragged card's edges snap to nearby cards' edges, with a
  guide line.

**Documents & interop**
- Native New / Open / Save / Save As (RON format), plus autosave on exit
- **File → Export** the whole tree as **Markdown**, styled **HTML**, **JSON**,
  **PDF** (paginated A4), or a **PNG/GIF** image
- **File → Import** **Markdown**/**HTML** as a new node, or a **JSON**-exported document

**App**
- Full-text **search** across every node title and card (Ctrl+F)
- Dark / light **theme** toggle
- **Zoom** the whole UI (Ctrl+`+` / Ctrl+`-` / Ctrl+`0`)
- **Agent API** — a localhost, key-gated HTTP API to add/query/edit/remove nodes
  and cards, move/recolor/resize them, build groups, dock cards, and export the
  document (incl. PDF/PNG) — so agents can collaborate on the same notes. Enable
  it in **Tools → Settings**; see [API.md](API.md).

## Keyboard

| Shortcut | Action |
|---|---|
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
X11 PRIMARY-selection features need `xclip` or `xsel` installed.

The markdown renderer (`egui_commonmark`) is vendored under `vendor/` and patched
to render inline text-color spans; edit it there, not the crates.io copy.

## Docs

- [API.md](API.md) — the localhost agent HTTP API.
- [CHANGELOG.md](CHANGELOG.md) — version history.
- [docs/](docs/) — development session reports (context for future work).

## License

MIT. Vendored `egui_commonmark` / `egui_commonmark_backend` are MIT/Apache-2.0
(see `vendor/*/LICENSE-MIT`).
