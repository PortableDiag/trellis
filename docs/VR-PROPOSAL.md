# Trellis in VR — proposal

**Status:** proposal / not started. Tracked as a **separate side project**, not a
feature of the desktop app.
**Target hardware:** Valve **Index** (tethered, SteamVR) and Valve **Frame**
(standalone SteamOS).
**Written:** 2026-08-03, against Trellis v0.66.0.

This document exists so someone — another agent or a future session — can pick
the project up cold. It states what to build, what to build it *on*, what is
already solved, and what to prove before committing to anything large.

---

## 1. The idea

Trellis is a tree of nodes where each node's body is a **free-form 2-D canvas of
cards**. That spatial metaphor is the whole point of the app, and it is exactly
the thing a flat monitor constrains: you pan and zoom because you cannot step
back and *look*.

In VR the basket becomes a wall you stand in front of, the tree becomes rooms or
a corridor, and cards are panels you reach out and move. Nothing about the data
model has to change for this to work — which is the strongest argument for doing
it at all.

Success means: put the headset on, see today's basket at readable size, move and
edit cards with your hands, and take the headset off with the document saved. Not
a demo — a way to actually work.

## 2. Architecture: a separate binary talking to the agent API

**Do not try to render the existing egui canvas in VR.** The desktop app draws
through `eframe`/`glow` into a single flat window; `canvas.rs` paints every card
directly at its screen rect with no transform layer (see the zoom-scaling note in
the README/CHANGELOG). Retrofitting stereo rendering, a 3-D scene graph and
controller input into that is a rewrite wearing a costume.

Instead, the VR client is **its own program** that speaks to Trellis over the
existing HTTP API:

```
  ┌────────────────────┐        HTTP (localhost or LAN)      ┌──────────────────┐
  │  trellis (desktop) │  ◄──────────────────────────────►   │  trellis-vr      │
  │  owns the document │   GET /api/tree, /api/nodes/{id}    │  OpenXR + wgpu   │
  │  autosaves to disk │   PATCH cards, POST cards, …        │  renders panels  │
  └────────────────────┘   GET /api/wait  (live updates)     └──────────────────┘
```

This is viable **only because the agent API is at full parity with the UI** —
nodes/cards CRUD, positions and sizes, groups, docking, tables, sketches, images,
templates, query surfaces, export. Anything the desktop app can do to a document,
the VR client can do too, without touching the desktop codebase. `API.md` is the
canonical reference; read its worked **Examples** section before writing a client.

Consequences worth stating plainly:

- **The desktop app stays the single writer of the file.** The VR client never
  opens `.ron` files. It gets persistence, autosave (~2 s after the last change),
  version history and backups for free, and cannot corrupt a document.
- **A Trellis instance must be running** for VR to have anything to talk to. On
  the Index (tethered) that is the same machine. On Frame (standalone) it is a
  LAN connection to the desktop — enable **LAN access** in Settings and use the
  instance's port. `GET /api/instance` reports which document a port serves, so
  the VR client can show the user what they are about to edit.
- **Live collaboration is already solved.** `GET /api/wait?rev=N` long-polls and
  returns the moment anything changes, so a card the user edits on the desktop
  updates in the headset without polling.

### Suggested repo layout

A new repository, `trellis-vr`, mirroring how the Android client is a separate
repo (`trellis-android`). Do **not** vendor it into this repo; it has a different
toolchain, a different release cadence, and a much heavier dependency tree.

```
trellis-vr/
  src/
    main.rs        session bootstrap, frame loop
    xr/            OpenXR session, swapchains, input actions
    render/        wgpu pipelines: textured quads, text atlas
    api/           Trellis HTTP client (mirrors API.md)
    scene/         node → room, card → panel layout
    ui/            grab/move/resize, keyboard invocation
```

## 3. Hardware targets

The two targets differ in one way that matters more than any other: **where the
code runs.**

### Valve Index — tethered, x86-64 Linux
- Runs on the desktop box, next to the Trellis instance. `127.0.0.1` networking.
- OpenXR runtime: **SteamVR's**, or **Monado** if SteamVR proves awkward on this
  machine. Both expose the standard `openxr` loader, so the client code is the
  same; only the runtime selection differs.
- **Knuckles controllers give per-finger tracking**, which is the reason to
  target Index first: pinch-to-grab a card is a natural gesture and the hardware
  reports it without extra hand-tracking machinery.
- 120 Hz native. Budget accordingly (§6).
- **This is the development target.** Tethered means fast iteration: rebuild,
  rerun, no deploy step.

### Valve Frame — standalone SteamOS
- Standalone headset running SteamOS on ARM. Two possible deployment models, and
  **which one to use is the first hardware decision the project must make**:
  1. **Native standalone build** — cross-compile the client for `aarch64` and run
     it on the headset, talking to the desktop Trellis over the LAN. Fully
     wireless, no PC rendering. Requires an ARM64 Rust toolchain and whatever
     SteamOS packaging the device expects.
  2. **PC streaming** — run the same x86-64 build as the Index target on the
     desktop and stream frames to the headset. Zero extra porting work, but adds
     latency and a dependency on the streaming link.
- Start with (2) because it is free once the Index target works, and only pursue
  (1) if wireless standalone use turns out to matter.

**Verify current specifics before relying on them.** Frame is recent hardware;
confirm its OpenXR runtime, controller/hand-tracking API surface, ARM toolchain
story and packaging against Valve's current documentation rather than trusting
this paragraph. The architecture above does not depend on those details — only
the deployment step does.

### Text input
This is where VR productivity projects usually die, so it is called out as a
first-class problem rather than an afterthought.

- **SteamVR provides a system virtual keyboard overlay**, and the Valve hardware
  has good support for it. Prefer invoking the platform keyboard over drawing our
  own — a hand-rolled VR keyboard is a project in itself.
- Fall back to a laser-pointer keyboard only if the platform one is unavailable.
- Consider **voice dictation** for long prose; card *bodies* are the long-form
  content and typing them in VR will be unpleasant regardless of keyboard.
- Realistic v1 stance: **editing structure and layout in VR, authoring long text
  on the desktop.** Moving, grouping, resizing, checking off checklist items,
  changing `status::`, and short title edits are all comfortable in VR. A
  1,500-word note is not.

## 4. Scene model — mapping Trellis onto space

A first mapping that respects the existing data model:

| Trellis            | VR                                                        |
|--------------------|-----------------------------------------------------------|
| Node (basket)      | A wall / curved panel surface you stand in front of        |
| Card               | A floating quad, positioned from the card's `pos`/`size`   |
| Card `color`       | Panel edge accent (same accent the desktop draws)          |
| Group              | A framed cluster that moves as one                         |
| Dock               | Panels that travel together                                |
| Tree hierarchy     | Navigation: adjacent rooms, or a corridor of walls         |
| `#tags` / `due::`  | Filter/highlight — surfaces that already exist in the API  |

Card canvas coordinates map to metres with a single scale factor; keep the 2-D
layout faithful at first so a basket looks like the desktop basket. **Resist
inventing a new spatial model until the faithful one has been used in anger** —
if cards land somewhere unexpected in VR, that is a layout bug, and a faithful
mapping makes it obvious.

Card content rendering, cheapest first:
1. **Title + accent only** — enough to prove layout and interaction.
2. **Text/Markdown body** rendered to a texture (CPU rasterise, upload, cache;
   invalidate on the `/api/wait` revision bump).
3. **Images** — `GET /api/nodes/{id}/cards/{cid}/images/{idx}` returns the bytes.
4. **Tables/checklists** — structured, so they can be laid out natively.
5. **Sketches** — vector strokes, natural to draw directly.

## 5. Phases

Each phase ends in something demonstrable. Do not start a phase before the
previous one works.

### Phase 0 — Spike (do this before promising anything)
**Goal: prove the stack exists and behaves on this machine.**
- An OpenXR session opens against the Index and renders a solid colour per eye.
- One textured quad floats in space at a fixed position.
- The quad shows **a real card title fetched from `GET /api/nodes/{id}`** on the
  running Trellis instance.
- Controller pose is tracked and drawn as a ray.

Acceptance: headset on, see a real card title from a real document, no crash for
five minutes. **If this takes more than a couple of sessions, stop and reassess
the runtime situation before continuing.**

### Phase 1 — Read-only viewer
- `GET /api/tree`, pick a node, lay its cards out faithfully from `pos`/`size`.
- Title + body text rendered to textures; images displayed.
- Navigate the tree (point at a child, teleport/fade to that basket).
- `GET /api/wait` for live updates from the desktop.

Acceptance: browse a real document comfortably; desktop edits appear in-headset.

### Phase 2 — Spatial editing
- Grab a panel and move it → `PATCH /api/nodes/{id}/cards/{cid} {pos}`.
- Resize → same endpoint with `size`.
- Group/ungroup, dock/undock via the existing group and dock endpoints.
- Batch position writes: **debounce and send on release**, not every frame.

Acceptance: rearrange a basket in VR, take the headset off, and the desktop shows
exactly that layout.

### Phase 3 — Content editing
- Checklist items toggled by poking them.
- `status::`/`due::` changes via `POST …/cards/{cid}/property` — the same
  property endpoint the Kanban board uses, so Agenda and Kanban update for free.
- Short text edits through the platform virtual keyboard.
- Card creation from a template: `GET /api/templates`, then
  `POST /api/templates/{i}/insert {node, pos}` — the template library is already
  a managed, visible thing (see the Templates basket, v0.66.0).

### Phase 4 — Things only VR can do
Deliberately last, and only if phases 1–3 are genuinely pleasant to use.
- Depth as a dimension: pull urgent cards forward, push done ones back.
- The wiki-link graph (`GET /api/graph`) as an actual walkable 3-D graph — this
  is the single most compelling VR-native feature the existing API already
  supports.
- Stand inside the whole document: baskets as rooms you walk between.

## 6. Constraints and risks

- **Frame budget is unforgiving.** At 120 Hz you have ~8 ms. Dropping frames in
  VR is nausea, not jank. Render card content to **cached textures** and re-upload
  only when the document revision changes; never re-rasterise text per frame.
- **Text legibility is the make-or-break detail.** Panel text that is comfortable
  on a monitor is often unreadable in a headset. Expect to tune scale, and expect
  to need higher-resolution textures than feel reasonable.
- **Linux VR is the real risk, not Rust.** SteamVR on Linux and Monado both have
  sharp edges. Phase 0 exists precisely to find out which runtime works here
  before any Trellis-specific code is written.
- **Latency to the API is fine; do not architect around it.** Reads are
  localhost or LAN and the document is small. Cache the tree, subscribe with
  `/api/wait`, and write on interaction end.
- **The desktop app must not be modified for VR.** If the VR client needs
  something the API cannot express, add it to the API as a normal endpoint —
  documented in `API.md`, the worked Examples, and the in-app Settings →
  Endpoints list, per the existing three-surface rule. That keeps the VR project
  from becoming a source of churn in a stable app.
- **Do not add a `CardKind` for VR.** A new variant touches ~180 exhaustive match
  sites across model/api/canvas/app. Everything VR needs is expressible with the
  existing kinds.

## 7. Getting started (concrete first steps)

1. Confirm the headset works on this box at all: which OpenXR runtime is
   installed, and does a stock OpenXR sample render? Record the answer — it
   determines everything after it.
2. `cargo new trellis-vr`; add `openxr`, `wgpu`, `pollster`, `ureq` (or
   `reqwest`), `serde`/`serde_json`.
3. Bring up an OpenXR session with a colour-per-eye clear. **Stop and verify in
   the headset.**
4. Add one textured quad; verify.
5. Add the API client: `GET /api/instance` to confirm which document is being
   served, then `GET /api/tree`. Put a real card title on the quad.
6. That is Phase 0 complete. Reassess before Phase 1.

**Connection details for the client:** the API is key-gated; send
`X-API-Key: <key>` on every request except `/api/health`. Get the key from
Trellis's **Tools → Settings → Agent API**. For a headset on the LAN, enable
**LAN access** there too. One instance serves one document and **the port is how
you address a document** — see `API.md` §Enabling it and `GET /api/instance`.

## 8. Open questions

- Frame: native ARM build or PC streaming? (Decides the toolchain story.)
- Which OpenXR runtime on this machine — SteamVR or Monado?
- Hand tracking via Knuckles capacitive sensing, or controller-ray only for v1?
- Is the walkable link graph (Phase 4) actually the headline feature, and should
  it be pulled earlier as the thing that justifies the project?
