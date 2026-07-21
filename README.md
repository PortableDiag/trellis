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
  code blocks are syntax-highlighted.
- **Code** — dedicated code editor with a language selector and highlighting.
- **Checklist** — real checkboxes with add/remove/edit per item.
- **Image** — pick a file from disk; the bytes are embedded in the document.

Cards drag by the title bar, resize from the corner, raise to front on click,
duplicate, recolor, and delete. The canvas pans; each node remembers its scroll.

**Documents & interop**
- Native New / Open / Save / Save As (RON format), plus autosave on exit
- **Export** the whole tree to a standalone, styled **HTML** file
- **Import** Markdown or HTML as a new node (HTML is converted to markdown)

**App**
- Full-text **search** across every node title and card (Ctrl+F)
- Dark / light **theme** toggle
- **Zoom** the whole UI (Ctrl+`+` / Ctrl+`-` / Ctrl+`0`)
- **Agent API** — a localhost, key-gated HTTP API to add/query/edit/remove nodes
  and cards, so agents can collaborate on the same notes. Enable it in
  **Tools → Settings**; see [API.md](API.md).

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

Requires a recent stable Rust toolchain. Tests: `cargo test`.

## License

MIT.
