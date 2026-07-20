# Trellis

A hierarchical, spatial note-taking app for the desktop, written in Rust.

Two proven ideas, woven together:

- **The tree** — an outliner-style hierarchy of nodes for structure and navigation.
- **The basket** — each node's body is not a linear document but a free-form
  2-D canvas where you drop, drag, and arrange rich cards.

Structure lives in the tree; spatial thinking lives in the basket. A trellis is
a lattice that supports branching growth — the tree *and* the weave in one.

## Status

Early v0. Working today:

- Tree panel: add root / child nodes, rename, delete subtrees, expand/collapse.
- Basket canvas per node: double-click to drop a card, drag by the title bar to
  move, right-click the title to delete, drag empty space to pan. Each card is
  freely editable text.
- Autosave to disk (RON) on exit and `Ctrl+S`; reloads on launch.

## Roadmap

- Rich text inside cards (bold/italic/lists, inline code).
- Card types: images, files, links, code blocks with syntax highlighting.
- HTML import/export per node (CherryTree-style fidelity).
- Zoom, card resize, and connectors between cards.
- Full-text search across the tree.

## Build

```sh
cargo run --release
```

Requires a recent stable Rust toolchain.

## License

MIT.
