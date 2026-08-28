# Changelog

All notable changes to Trellis. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions are the app version in
`Cargo.toml`, each with a matching git tag and GitHub release.

## [Unreleased]

## [0.161.1]

### Fixed
- **The minimap shows the whole day, projections included.** With Time on, a
  card spanning this day from another one is drawn on the canvas but was
  missing from the minimap — the map computed its world box and its dots from
  the resident cards alone, so a two-card day read as one (operator's report),
  and a projection sitting past the residents' bounds was cropped outside the
  map's world entirely. Projections now extend the world box and draw on the
  map — hollow, in the card's accent, where a resident is a solid block: the
  map's version of the canvas's not-a-resident language. A day with nothing of
  its own but work passing through gets a map now too.

## [0.161.0]

### Fixed
- **`PATCH /api/nodes/{id}` honors `null` to clear `color` and `bg`.** Reported
  by an API user: `{"bg": null}` answered 200 and changed nothing — a plain
  `Option` collapsed "clear it" into "field absent", so the documented clear
  was inexpressible and the response dishonest. Both fields now use the same
  double-Option rule as a card's `view`: a color sets, `null` clears (no tag /
  back to the theme default), absent leaves it unchanged. Bad values are still
  a 400 at parse time. Minor bump because a caller that was sending `null` and
  relying on the no-op now gets the clear it asked for.

## [0.160.5]

### Fixed
- **Time-mode projections stand down in a feed too.** The v0.160.4 lesson
  ("every painter that reads stored geometry needs the gate — enumerate them")
  applied to itself: projections, drawn on journal days from other baskets'
  stored positions, were the one enumerated painter not yet gated. A feed
  basket that is also a journal day no longer draws them across the column.

## [0.160.4]

### Fixed
- **A feed draws nothing from stored geometry.** The first real feed basket
  with a card group in it (operator's screenshot) showed the group frames as
  stray full-width bars: the cards render in computed feed slots, but group
  frames and dock connectors were still painted from the stored x/y. Both now
  stand down in a feed, like the minimap and marquee already did. Groups and
  docking themselves are untouched — they return with the canvas.

## [0.160.3]

### Fixed
- **README catches up with the day.** Three sections join the feature tour:
  the **Cube** (fly through a range of days; the temporary embed-built view,
  isolate, the two exits), the **Feed** (a log basket reads newest-first,
  placement-free inserts, End/Home navigation), and **plugin staleness**
  (Tools → Plugins says when a release is newer than the installed copy, and
  the Update button keeps settings and state). Doc-only.

## [0.160.2]

### Fixed
- **The tree's color dots line up.** A row with an expand arrow drew its dot
  ~6px right of an arrowless row's: the childless branch was a raw 18px
  spacer while the arrow was a button sized by its glyph plus padding — and a
  spacer is not a widget, so arrow rows also picked up an item-spacing gap
  the others did not. Both branches now occupy one fixed-size widget slot,
  so the dots form a straight column whether or not a row can expand.

## [0.160.1]

### Fixed
- **The docs teach the feed.** API.md's card-create section, the *Notes for
  agents* placement bullet and a new worked example (*Keeping a log — make the
  basket a feed*) now say what v0.160.0 made true: a chronological basket
  should carry `feed: true`, and then inserting needs no `pos`, no
  append-below-the-column, and no overlap repair. The old column recipe stays
  documented for chronological baskets deliberately kept as canvases, with the
  version gate spelled out (an older binary answers 400 to the field). Doc-only
  release — API.md is compiled in, so a doc correction ships as a version.

## [0.160.0]

### Added
- **A basket can declare how it is read: canvas, or feed.** Designed from
  scratch after the operator challenged the log-basket convention ("so anytime
  I go to those workspaces I have to scroll around to the last cards, that's
  the plan?"): a log has no arrangement — its y axis was doing *time*, badly.
  A **feed** basket renders as one top-anchored column, **newest entry
  first** — open it and the latest handoff is simply there.
  - Toggle: **Feed** in the canvas toolbar, or `PATCH /api/nodes/{id}
    {"feed": true}`. The flag lives on the node (a document edit, in the
    change log as `feed`).
  - The layout is **computed on read and never stored** — same principle as
    view cards and the cube scene. The x/y arrangement underneath is
    untouched and returns exactly when feed goes off: the promise Depth makes
    about z, made about layout. No position may be written *from* a feed
    (drag/resize/z writes are dropped at the canvas exit), and
    minimap/marquee/Depth/Time/Cube stand down on a feed basket.
  - Sorted by **creation order** (the card id — the document-wide counter),
    deliberately not `touched`: fixing a typo in an old entry must not
    teleport it to the top.
  - **Inserting into a feed basket needs no position**: just POST the card.
    The read-the-column-bottom / append / repair-overlaps recipe evaporates.
- **Jump-to-newest, in every basket.** `End` centers and flashes the most
  recently touched card (falling back to the newest-created); `Home` is Reset
  view. For the canvas baskets that stay canvases — arriving is the whole
  trip. Both stand down while an editor holds the caret, like PageUp/Down.

## [0.159.1]

### Fixed
- **A composed cube peels — the deepest days are reachable by flight.**
  Reported live from the first real cube: "can't scroll past the 4th-to-last
  card." The lens cull removes a passed card only ~4.6 slices after the camera
  passes it, and the camera stops 400 past the deepest slice — so at the
  bottom of a cube the slices already flown through ballooned over the view
  and the last few days could never be read. A composed scene's slices are
  discrete planes, so it now **peels**: half a flight step past a slice culls
  it, one layer per PageDown, all the way down, and flying back up restores
  the layers one at a time. Ordinary baskets viewed in Cube mode keep the
  lens cull — their cards sit at arbitrary depths, and a card slightly nearer
  than the camera is the front row, not a passed layer.
- **The flight rendered one step late.** The dolly was read at the top of the
  frame, before the wheel/keys moved the camera, so every draw was one flight
  step behind the input. Caught by the headless harness while pinning the
  peel, invisible in live use only because the next repaint caught up.

## [0.159.0]

### Added
- **The cube operation: a range of baskets becomes a volume.** This is the
  compressed-workspace-cube feature as actually designed (the operator, after
  two approximations shipped around it): *select a range of day baskets, and
  the cube operation aligns those workspaces' cards along z in a temporary
  area you traverse*. Not a basket you decorate, not a mode you point at one
  workspace — an operation over several.
  - **Right-click a basket → Open as cube…** offers a from/to range over its
    child baskets (a month of dailies → pick the week you mean). Each basket
    becomes one slice, first deepest, last nearest, keeping its own x/y
    arrangement (staggered a little per slice so the layers read as layers).
  - **`POST /api/cube {"nodes":[…]}`** is the same operation for an agent —
    slice order is list order, the whole list is validated before anything
    changes (a 404 names every id that is not a basket), and
    **`DELETE /api/cube`** leaves the view. Full UI parity, as always.
  - **The scene is temporary and made of `![[#id]]` embeds** — live views of
    the real cards. Nothing is created, copied or saved; the document is
    untouched; the scene renders under a node id no document can reach, so
    stray canvas mutations against it land nowhere. *Go to card* follows a
    slice's embed home, exactly as in any cube.
  - The scene keeps its **own camera** (pan/zoom/orbit/dolly) under that
    sentinel id — flying through a cube never scrambles the view of the
    basket you were in. The fly clamp now adapts to the basket's real z range,
    so a month of slices is reachable end to end, and there is no empty space
    beyond the content to get lost in.
  - Building a new cube resets the cube camera; closing the cube (toolbar,
    Settings, API, or *Go to card*) drops the scene everywhere — one
    `exit_cube` for every exit, because three exits each forgetting one piece
    is how half-exited modes happen.

## [0.158.0]

### Changed
- **Cube is its own mode — Depth and Time get their exact old behavior back.**
  v0.157.0 hung the cube's reading gestures on Depth mode itself, and the
  operator reported it within the hour: a plain click already means something
  on every card (select, place the caret, work), Depth+Time was in daily use,
  and suddenly every ordinary click ghosted the basket and raised a popup.
  The cube was always meant to be a **new** feature, not a change to existing
  ones. So there is now a **Cube** toggle beside Depth and Time (and a
  `cube_mode` setting on `GET/POST /api/settings`): same projection as Depth,
  plus the reading gestures — plain-click isolate with *Go to card* / *View
  only this*, Ctrl+Shift+scroll / PageUp·PageDown flight, culling past the
  camera. Cube is mutually exclusive with Depth and Time (its gestures claim
  inputs they use for editing); *Go to card* exits Cube and leaves Depth
  untouched. In Depth mode: no isolation, no popup, no flight, and any dolly a
  Cube session left in a basket is ignored — pinned by a headless regression
  test that clicks and flies in Depth mode and asserts nothing happens.

## [0.157.0]

### Added
- **Depth mode learns to be read, not just looked at** — the three pieces the
  compressed-workspace-cube prototype (card 2113) said were missing before a
  cube could be a productivity tool rather than a neat visual:
  - **Click-to-isolate.** Click a card in Depth mode and every other card drops
    to a ghost, so the volume can be read *into* without rearranging it. Click
    another card (ghosts stay clickable) to move on; Esc or empty canvas steps
    back out. The isolated card draws on top of the ghosts whatever slice it is
    in. Modifier clicks keep their old meanings — Ctrl+click still selects.
  - **The isolate popup: two exits.** *Go to card* leaves the cube for the
    card's real workspace in regular flat mode — a cube slice is an `![[#id]]`
    embed, so it follows the embed to the original (a card with no embed is its
    own destination). *View only this* stays in the cube: the camera flies to
    the card's slice and frames it for reading — checking a week of daily test
    results is click, read, click the next.
  - **Fly through z.** Ctrl+Shift+scroll is continuous camera travel;
    PageUp/PageDown step one cube slice (380 world units). This is a dolly, not
    a zoom: every card's effective depth shifts together, so a far slice
    arrives at the front row at full scale, and a slice flown *past* is culled
    rather than smeared across the viewport. Reset view zeroes it along with
    the orbit, and the camera state is per basket, like pan and orbit.
  - The Depth hint line names all of it, and PageUp/PageDown stand down while a
    card's editor has the caret.
  - **The canvas gained a headless input harness.** The fly gesture's first
    draft was gated on `canvas_resp.hovered()` — which goes false over any
    card — and looked completely healthy in code review; input plumbing can
    only be tested by running a frame, and running a frame must not require a
    display. The tests now drive the real canvas through `egui::Context::run`
    with synthetic keys, wheel and clicks, and pin all of it: PageUp/PageDown
    fly, Ctrl+Shift+wheel dollies (including egui folding a shift+wheel onto
    the x axis — the same trap the per-card Z gesture once hit), a plain wheel
    still pans, click isolates/switches/clears, Esc exits. One harness gotcha
    worth keeping: per-event modifiers never reach `InputState::modifiers` —
    the held-key state rides on `RawInput::modifiers`.

### Fixed
- **The channel boundary is the project subtree, never a name — and API.md now
  says so** (operator ruling, 2026-08-26, after a Trellis session picked up a
  message meant for another project's agent). A document can hold several
  projects' workspaces, each with its own channel, and a waiting channel
  belongs to the agent of the project whose subtree holds it: answer only the
  channels under your project's root (`GET /api/channels?project=<node id>`,
  or `node_path` in the listing) and report another project's waiting channel
  rather than draining it — a reply from the wrong agent clears the flag under
  the one the message was for. On a single-project document this collapses to
  the older port-is-the-boundary rule. `channels_waiting` on `/api/instance`
  stays document-wide, so the docs now warn a positive count may be somebody
  else's channel.
- **API.md stopped recommending the `claude` answerer plugin.** It told
  readers to install and approve it while the plugin is installed nowhere and
  never ran once (removed from both instances 2026-08-22). The section now
  says what actually answers a channel — a running agent at its checkpoints,
  or immediately via a background `/api/wait` watcher — states that a watcher
  dies with its session, and keeps the plugin only as the design reference for
  the `roots` project mapping, which is the same boundary rule.

## [0.156.0]

### Added
- **A stale plugin says so, where somebody will look.** A plugin release does
  not install itself — plugins run from `<data-dir>/plugins/`, the repo only
  ships them — so a release could be tagged, changelogged and documented while
  every instance kept executing the old copy, with no symptom beyond a feature
  that silently did nothing. That cost a day of link-less notifications once
  and recurred twice in a week. Now the app compares each installed
  `plugin.json` version against the release copy in the checkout beside the
  running binary (found from the binary's own path; nothing is compared, and
  nothing reported stale, when it runs from anywhere else):
  - **Tools → Plugins** shows *"v1.2.0 available"* beside a stale plugin, with
    an **Update** button that copies the release's code and manifest over the
    installed copy and leaves its `config.json` and `state.json` alone —
    exactly the update that was done by hand three times. Approval survives,
    because a grant is keyed by plugin name. Nothing installed is ever deleted.
  - **`GET /api/plugins`** lists installed plugins with `version`, `available`,
    `stale` and `approved`; **`stale_plugins` on `/api/instance`** is the
    count, riding on the call every agent makes first — the same read-in
    contract as `stale_claims`. Both scope-neutral, read-only. There is
    deliberately **no update endpoint**: updating replaces executable code,
    which is exactly what the approval model keeps behind a human act, so the
    API reports the gap and the operator closes it.
  - Versions compare numerically per segment (`1.10` beats `1.9`), and stale
    means strictly newer — an installed copy ahead of the repo is a build not
    yet released, not a problem. Deliberately **no auto-copy on launch**, for
    the same reason there is no endpoint.

### Fixed
- **API.md stopped teaching the channel-miss.** The `/api/instance` section
  answered `channels_waiting` with `GET /api/channels?agent=<your name>` — the
  exact name-guess filter that has now made a waiting message go unread twice
  (nothing ever tells an agent the name a channel was created with). It now
  says what the prompts already say: read `GET /api/channels` unfiltered; the
  answering boundary is the document you were started on, never a name.

## [0.155.2]

### Fixed
- **A same-node card move says what to do instead.** `POST …/cards/{cid}/move`
  with a `node` equal to the card's current basket answered a bare
  `400 "card is already in that node"` — correct (a move drops group/dock
  membership, so letting it "reposition" would quietly ungroup the card) but
  unhelpful: an API user reasonably tried it for an in-place reposition and had
  to discover `PATCH {"pos"}` on their own. Both the single and batch refusals
  now name the tool that works, and API.md documents the refusal and the
  reason at the move route.

## [0.155.1]

### Fixed
- **A property write lands where the parser reads.** `POST …/property` only
  recognised a property at the start of a line, while the extractor reads
  `key:: value` anywhere outside code — so setting `status` on a card whose
  `status:: doing` sat at the end of a prose line appended a standalone
  `status:: done` and the Kanban kept counting the old inline one (first
  occurrence wins). Reported live from card 9913, second sighting of the
  class. The setter now rewrites the first occurrence in place — wherever it
  is, bracketed tag lines included — using the extractor's own scanner
  (factored out and shared, so the two cannot drift again). A date key
  (`due`/`start`/`date`) replaces only its one-token value, so the sentence
  after `due:: 2026-08-15 — notes` survives a reschedule. `DELETE …/property`
  now removes **every** occurrence for the same reason: clearing only the
  standalone line resurrected the inline value it left behind. Batch and
  cross-basket property routes inherit both fixes.

## [0.155.0]

### Added
- **Group create takes `color`.** `POST /api/nodes/{id}/groups` accepts an
  optional `color` (same formats as everywhere else: array, hex, or name), so
  a group arrives styled in one call instead of create-then-PATCH. It was
  already accepted on `PATCH …/groups/{gid}`; the asymmetry surfaced when an
  API user, refused the field on create, resorted to titling the group with a
  colour legend.

## [0.154.0]

### Added
- **`cloud-backup` plugin: scheduled off-site backup that proves itself.**
  Copies the newest local backup archive to a self-hosted CloudAPI gateway (an
  object-storage gateway that mints short-lived, prefix-scoped S3 credentials
  in front of R2), on the plugin schedule (default 6 h) or on demand. Every
  run **downloads the object back and compares SHA-256** — an untested backup
  is a belief — and prunes the cloud copy beyond a configured count, oldest
  first, each by explicit name, never a sweep. It uploads what the backup
  module already wrote, so with backup encryption on the archive is gpg
  ciphertext before it leaves the machine and the gateway prefix is
  `encrypted: false` (its documented already-sealed case: a second layer would
  put the key inside the thing being backed up). Object names are
  `backups/<document>/<archive>`, because two instances write
  identically-named archives. Credentials live in the plugin's own per-instance
  `config.json`, outside the document and outside this repo — unconfigured, the
  plugin does nothing, which is how the integration ships publicly while the
  access stays personal.

## [0.153.5]

### Fixed
- **Every yes/no the app asks is in-app now, because the native ones never
  showed.** `rfd` 0.15's default Linux backend is the xdg-desktop-portal, and
  the portal has no message-dialog API — `MessageDialog::show()` returns
  immediately, drawing nothing. v0.153.4 found and fixed that for the template
  ✕; this release fixes the two remaining users. The **unsaved-changes ask**
  guarded File → New / Open… / Import JSON and version-history Restore — with
  changes pending, all four **silently did nothing**. The **large-attachment
  ask** meant a dropped file over 10 MB was **silently refused**. Both now go
  through one in-app confirm window (queued, front-first, Esc declines), which
  says what proceeding discards and what an attachment costs. File dialogs are
  untouched — file choosing is what the portal exists for, and those work.

## [0.153.4]

### Fixed
- **The template-delete confirmation actually appears.** v0.153.3's ask was a
  native `rfd` dialog, and a live probe showed it never rendering on this
  system — the ✕ click landed, no dialog appeared, and nothing was deleted: a
  gate that fails safe but leaves a button that seems dead. The ask is now an
  **in-app window** ("Delete the template …? Delete / Keep", Esc keeps), drawn
  by the same machinery as every other window in the app, so it cannot
  silently no-op. Same class as the `--disable-javascript` finding: a gate
  that looks right and does nothing is worse than no gate.

## [0.153.3]

*(Never tagged on its own — this entry shipped inside v0.153.4's commit, so
there is no `v0.153.3` tag or release. Kept because the two fixes are worth
recording separately.)*

### Fixed
- **Deleting a template asks first.** The ✕ beside each row of the
  Insert-template list deleted immediately — and it does not only drop a config
  entry: the template's **master card** is removed with it, which is document
  content. It sits one slip from the row you meant to click, so it now asks
  "Delete the template …?", naming what goes and that already-stamped cards
  stay. The ✕'s tooltip says the same. Raised by the operator, who asked the
  right question before clicking rather than after.

## [0.153.2]

### Fixed
- **The menu's `#id` is readable now.** v0.153.1 drew it weak and small — dim
  grey at reduced size, on the theme's dark menu — and the operator could not
  read the one thing the line exists to show. Full size, full contrast, still
  monospace.

## [0.153.1]

### Changed
- **A card's context menu says which card it is.** `#id` sits at the top of the
  right-click menu, readable in place — before this, seeing an id meant
  Copy → Card id and pasting it somewhere to look. The Copy entries are
  unchanged for when you need it on the clipboard.

## [0.153.0]

### Added
- **The Link graph can be steered.** Drag pans, the wheel zooms at the pointer
  (plain or Ctrl — pinch works too), **Alt+drag rotates** about the centre (the
  canvas's "look around" gesture), and double-click resets the view. The header
  line names every gesture. Labels stay horizontal while the layout turns.

### Fixed
- **The Link graph is a map again at 364 nodes, not piles of confetti.** Three
  defects, all scale-dependent, so the window looked fine on small documents:
  the layout's ideal-separation floor (30px) was actually being hit at this
  node count, and with attraction growing as distance² every linked cluster
  crushed itself into a ~30px pile; disconnected clusters repel each other
  forever (repulsion is all-pairs, and nothing pulled back), so they drifted
  to the movement cap for 300 steps and the fit-to-window scale shrank every
  cluster to a dot — fixed with a gentle linear gravity toward the centre,
  plus cooling so the layout settles instead of oscillating; and all labels
  painted unconditionally, overprinting into grey smears — now a label draws
  only where no better-connected node's label already sits (hubs win the
  pixels), and a suppressed name comes back on hover, on a plate, over
  everything. Draw order was also HashMap iteration — per-process random —
  and is now degree-ranked, so which label wins no longer changes between
  runs.

## [0.152.1]

### Changed
- **The View menu asks for the search window once, not twice.** "Search…" (beside
  Go to node…) and "Find…" (at the bottom) had always been the same operation —
  both open the full-text search window, whose own heading is **Search**. The
  duplicate was only noticed when v0.152.0 stamped **Ctrl+F** beside both.
  "Find…" is gone; "Search…" stays where it was, key and tooltip intact.

## [0.152.0]

### Added
- **Card hotkeys.** The context menu's verbs, without the trip to the menu,
  acting on the selection (Ctrl+click a card, or drag a box): **Ctrl+A** selects
  every card in the basket, **Ctrl+C** copies the selected card, **Ctrl+V**
  pastes it where the pointer is (centre of the view when the pointer is
  elsewhere), **Ctrl+D** duplicates the selection, and **Del** deletes it —
  however many cards that was, one **Ctrl+Z** brings them all back, because the
  frame's batch snapshots once. All of them are silent while a text field has
  the keyboard, so Ctrl+C stays "copy the selected text" and Delete stays
  "delete a character" while you type.

### Changed
- **Every action with a hotkey now says so.** The card menu shows the key beside
  Duplicate, Copy → The card and Delete card, and their tooltips explain that
  the key acts on the *selection*; Paste card shows Ctrl+V. The menu bar shows
  New Ctrl+N, Save Ctrl+S, Undo Ctrl+Z, Redo Ctrl+Y, Today's note Ctrl+T,
  Go to node… Ctrl+O, and Search/Find Ctrl+F beside their entries instead of
  (or in addition to) burying them in a tooltip. The canvas hint line gains
  `ctrl+a` / `ctrl+c/v/d` / `del`.

## [0.151.1]

### Changed
- **README catches up with the day.** The web-page card and its permission model,
  the mirror write-back and its conflict rule, `channels_waiting`, and the compose
  box on both surfaces. Also corrects a claim it had been carrying: the Android
  basket canvas render of a channel card was described as **unverified**, and it
  has since been checked on a device — headings, code spans and the rules between
  messages all draw.


## [0.151.0]

### Added
- **Split cycles its own orientation.** Pressing **Split** while already split
  flips stacked ↔ side by side, so the second press is the setting rather than a
  fourth button nobody would go looking for. The arrow on the button says what
  the *next* press gives. `view` gains **`vsplit`**; `split` is unchanged.

### Changed
- **Rendering a web page needs no permission; raising the page's permission
  does.** The operator's question was the right one — *why would a user not trust
  their own agent to create a render-safe page?* They would. Building and
  rendering a page **is** the feature, and a sandboxed render produces a picture
  inside the document and nothing else.

  What actually needed a human was never the render: it was `network` and
  `scripts`, which make **this machine** fetch or execute on the page's behalf.
  So the gate moved to where the risk is — an API caller may only ever set
  `allow: none`, and `network`/`scripts` are **403** with the card menu named as
  where a person grants them. Lowering to `none` is always allowed. The
  *Let agents render web-page cards* setting is **gone**.

- **The rendered page is centred in its pane**, instead of sitting against a
  corner with all the slack on one side.

- **The file-mirroring setting says what it does.** It was reported as unclear,
  and it was: *"Let agents write mirrored files back"* named a mechanism, not a
  consequence. It now reads **"Let agents overwrite files on this machine"** and
  explains that this one is *not* about trusting the agent's judgement — it is
  that the damage lands **outside** Trellis, where version history cannot undo
  it. That is exactly why it survives while the render gate did not.

- **The in-app API Examples cover today's features** — building a page from your
  data and rendering it, editing a mirrored file and writing it back, and finding
  out somebody is waiting for you.


## [0.150.1]

### Changed
- **The card menu is sorted into sections.** Same 39 entries, grouped by what
  they are for: *this card* (Edit, Duplicate, Fit to content) · *how it looks*
  (Color, Emphasis) · **make it something** (channel, web page, file mirror) ·
  *take it elsewhere* (Copy, Export, Download, OCR) · *what is around it* (local
  graph, unlinked mentions) · *where it sits* (desktop, dock, group) · *templates
  and plugins* · Delete.

  Reported as *"copy in 2 stops, and Make a channel and Make a web page both
  spread out"*, and both were exactly that: **Copy card** sat a dozen entries
  away from the **Copy** submenu holding card link / id / path, so the word
  "copy" named two unrelated places; and the three things an ordinary card can
  *become* were scattered across the whole menu, which meant you had to already
  know each one existed to find it.

  **Copy card** is now *Copy → The card*, so there is one Copy. Verified by
  comparing the set of menu entries before and after — 39 either way, with that
  single intended rename.


## [0.150.0]

### Added
- **`GET …/cards/{cid}/html/png`** — the rendered picture of a web-page card, as
  base64. The bytes are deliberately absent from the card JSON (megabytes in
  every basket listing), and the phone cannot render a page itself: it has no
  browser and no business running one. So it shows what the desktop produced.
  404 when the card has never been rendered, which is a state rather than a
  failure. Rendering is the gated action; reading back what was already rendered
  is an ordinary document read, answered in `process` with no browser involved.


## [0.149.1]

### Fixed
- **A channel card's own `verify::` lines were counted as a question.** An hour
  after `channels_waiting` shipped it was reporting **1** on the operator's
  restarted instance, and the "message" was the A2A card's own claim metadata —
  `verify::` and `check::` sit in the body as unheaded text, which
  `parse_channel` reports as the operator speaking. Any channel card carrying a
  claim would have read as *waiting for an answer* for ever, which is a counter
  that cries wolf permanently and therefore stops being read at all — the exact
  failure the count exists to prevent.

  A trailing operator block whose non-blank lines are **all** bare `key:: value`
  is metadata, not speech. One line of prose anywhere in it and it is a message
  again, because somebody writing *"look at this, `due:: tomorrow`"* is talking.


## [0.149.0]

### Added
- **A card can be a web page you write.** The body is HTML/CSS/JS and the card
  shows it rendered — **Code**, **Page**, or both side by side. Built for the
  thing it was asked for: an agent reading whatever cards it needs, baking the
  numbers into a self-contained page, and writing that into a card as a view of
  your data.

  `PATCH {"html":{"view":"split","allow":"none"}}` then
  `POST …/cards/{cid}/html/render`; *Make it a web page…* and a **Web page**
  submenu in the card menu. `GET /api/cards/{cid}` reports it, including
  **`stale`** — the body edited since the picture was taken.

  **A field on an ordinary text card, not a seventh `CardKind`** — measured today
  at 15 sites the compiler catches and ~3 more it does not, plus Android, which
  has no compiler involved. The body stays a body, so the source is searchable,
  exportable and editable by everything that already edits one.

  **Rendered to a picture on purpose.** Trellis paints to a GL surface and has no
  DOM; an embedded webview is an OS surface that cannot composite inside the
  canvas, so it would float above the app and refuse to scroll, zoom or export
  with the card. A PNG zooms, pans, projects through Depth, exports, and shows on
  the phone, which cannot run a browser at all.

### Security
- **What a page may do is gated per card, and the gate was measured before it was
  trusted.** `none` (the default) permits the page's own markup and **not one
  outbound request**; `network` may fetch images, styles and fonts but still
  cannot script; `scripts` is everything, chosen for that card. The level is
  shown in the card header, not hidden in a menu.

  Enforced by a **Content-Security-Policy written into the page**, because the
  obvious alternative does not work: `--disable-javascript` was tested against a
  page that reports whether its script ran and made **no difference at all** —
  the screenshot was byte-identical to the ungated one. A gate that looks right
  and does nothing is worse than no gate. Verified end to end through the app at
  both levels: the same page renders *SCRIPTS BLOCKED* sandboxed and *SCRIPTS
  RAN* when allowed.

  An unrecognised `view` or `allow` is a **400 naming it** rather than a silent
  fallback — a typo landing quietly on a permissive default is a hole — and
  `html_csp` falls to the safe end as a second line.

  **Agents are refused by default**, behind *Settings → Agent API → Let agents
  render web-page cards*. Not because the HTML is dangerous to read, but because
  rendering starts a browser process on this machine over content the caller
  wrote — the same reasoning that stops the currency plugin executing a
  `check::`. The app's own *Render* is never gated by it.


## [0.148.0]

### Added
- **A card can be written back to its file, and a conflict is shown rather than
  resolved.** Mirroring has been one-way since it shipped; this is the writer.
  Off per card (`source_write`), and **never continuous** — the write is an
  explicit action, because continuous two-way is what the lost-update,
  poll-overwrites-your-typing and save-by-rename failures all attack at once.

  *Write back to file…* and *Compare with the file…* in the card menu;
  `POST …/cards/{cid}/source/write` and `GET …/cards/{cid}/source/diff` over the
  API, both addressable by card or by basket.

  **The conflict rule, which is the whole feature: it asks, and shows a diff.**
  If the file's mtime moved since the card last read it, nothing is written and
  the refusal *carries the diff that caused it* — 409 over the API, and in the
  app a window offering **overwrite the file**, **discard my edits and take the
  file**, or **leave both alone**. There is deliberately no merge: a merge is a
  third version nobody wrote, and every one of the five data-loss paths this was
  assessed against turns out to be a version of resolving silently.

  While a writable card has unwritten edits the **refresh poll leaves it alone**,
  so the file cannot replace what you are typing — but `source_mtime` is left
  where it was, so the conflict still surfaces the moment you try to write.

  Refused, each for a stated reason: **tables, images, sketches and checklists**
  (a table mirroring a CSV would be re-serialised by us, so the file would change
  on every save with no edit), **tail mode** (it would replace the file with its
  last few lines), and **agents by default** — a separate permission from the
  mirror *read* policy, because reading a file leaks it and writing to one
  destroys it. The write is temp-then-rename beside the target, carrying the
  original's mode, and writes the body byte for byte with no tidying.

- **A channel card has a compose box on the desktop.** Reported: *"I can't seem
  to edit the channel card or send messages, it just does nothing when I click on
  it."* Exactly right — the desktop could *make* a channel and *read* one, and
  the only way to say anything was to double-click the handle and hand-append
  below every `### @name` header. Clicking did nothing because a card is edited by
  double-clicking its handle, which is not something anyone should have to know to
  answer a question.

  The row is **pinned to the bottom of the card frame, outside the scroll area** —
  inside it, it sat after the conversation, and a conversation only grows, so the
  one control that makes the card usable was permanently below the fold. Send, or
  Ctrl+Enter; plain Enter is a newline. It goes through the same `ChannelSay` the
  API uses, so the header, timestamp and sequence number are written once for both
  surfaces. The card scrolls to the newest message, like a tail.

### Fixed
- **Ctrl+Enter in the compose box did nothing** while the tooltip promised it
  worked. A multiline `TextEdit` swallows Enter to insert a newline, so reading
  the key back from `input()` after the widget sees an event that is already gone.
  It is `consume_key`-ed before the field is drawn, which also stops the stray
  newline.


## [0.147.0]

### Added
- **`/api/instance` says when somebody is waiting.** Two new fields: `channels`,
  the number of channel cards in the document, and **`channels_waiting`**, how
  many of them have the **operator speaking last** — somebody typed into a card
  and no agent has come back to it.

  This is the piece that was missing, and its absence is why a message sat
  unread for a day while the card it was typed into worked perfectly. The
  transport was fine; the **noticing** was not. A channel only works if an agent
  looks at it, and until now the only thing that made an agent look was being
  told to in a prompt — so it worked for agents who had been briefed and failed
  silently for everyone else.

  It rides on `/api/instance` for exactly the reason `stale_claims` does: that is
  the call every agent already makes first. **No plugin, no configuration and
  nothing to install** — an agent that has never heard of channels still finds
  out that one is waiting, and `GET /api/channels?agent=<name>` is one hop away.

  Waiting is deliberately **identity-free**: "the operator spoke last", not "you
  have not replied". This endpoint is scope-neutral and the instance key
  identifies nobody, so there is no reliable *you* to compare against — and the
  case worth catching is the one where nobody at all has answered.

### Removed
- **The `claude` plugin is no longer installed on either instance.** It was never
  approved and therefore never ran once; the operator's judgement was that a
  channel should not need a plugin or a settings form at all, and that is right —
  the agent is already working in the workspace and can read the card. The
  waker's remaining job is covered by the counter above. The plugin stays in the
  repository for anyone who wants an unattended answerer, but it is no longer
  part of how a channel works.


## [0.146.2]

### Fixed
- **The channel agent works in the project's directory, not in one directory for
  the whole document** (`claude` plugin 1.0.0 → 1.1.0). Reported by the operator,
  and the criticism was exact: a card in a NodeJS project saying *"add a log check
  to the boot sequence"* was answered by `claude -p` running in whatever single
  folder the plugin's `cwd` setting named — for them, the Trellis repo. The reply
  would have been confident, well-formed, and about the wrong codebase. Twelve
  projects would have shared one working tree.

  The information was already there and thrown away: `GET /api/channels` returns
  `node_path`, whose first segment is the channel's root basket. A **`roots`**
  setting now maps those names to directories, one `Project = /path` per line, and
  each channel is answered in its own.

  **A channel whose project is not mapped is skipped, and says which line to add.**
  Not answered from a fallback — a reply computed in the wrong repository looks
  exactly like a real answer, which is worse than silence, and the cursor is left
  alone so the message is answered as soon as the map names its project. The old
  global `cwd` survives only as an explicit opt-in fallback and now defaults to
  **blank**; it previously defaulted to the plugin's own folder, so an
  unconfigured install answered everything from a directory containing nothing but
  the plugin script, which reads as the model being useless rather than as the
  plugin being unconfigured.

  **The agent name comes from the manifest**, so the plugin is copyable per agent:
  copy the folder to `plugins/<agent>/`, change `name` in its `plugin.json`, and
  that install answers `?agent=<agent>` and posts under that name — with its own
  approval, token, `roots` map and cursor, which is what keeps two agents out of
  one working tree. Verified with a second install: it selected only its own
  channel and kept a separate cursor.

  **One project's failure no longer abandons the rest.** The run used to exit on
  the first error; with a single global directory that was invisible, because
  there was only ever one thing to fail. Every answerable channel is now answered
  and the failures are reported together at the end.

  Verified on a scratch instance with three projects and two real checkouts: each
  channel's reply quoted the marker file from *its own* directory, and the
  unmapped project got no reply at all.

### Changed
- The plugin's settings are relabelled. The operator's words were *"you have a
  bunch of input boxes for settings, even I don't know what most do"* — each now
  says what it decides and what happens if it is left alone, with a worked example
  on the one that matters.


## [0.146.1]

### Fixed
- **Setting a property on a card that cannot carry one is refused, not
  confirmed.** Reported from use by an agent working through the API: `POST
  …/cards/{cid}/property` on a **table** answered **200**, echoed the value back,
  and stored nothing.

  `set_card_property` writes into `body`. A card's properties are parsed from its
  title and its **content**, and `searchable_body` decides what content is — the
  body for text and code, the **items** for a checklist, the **cells** for a
  table, names and OCR for an image, nothing for a sketch. So on four of the six
  kinds the write went somewhere nothing reads.

  **It was broader than reported.** Measured across every kind before touching
  anything: text and code work; **checklist, table, image and sketch** all
  accepted, echoed and discarded. Checklist is the sharpest — that is the
  task-carrying kind, so an agent marking a working list `status:: done` was told
  it had. Every route was affected: the node-addressed one, its card-addressed
  twin, the basket batch and the whole-document batch, and `DELETE` reported
  `cleared` on a property that could never have been there.

  All of them now answer **400** naming where the property *can* go — the item
  route for a checklist (a dated line is its own task), the title otherwise. The
  batch refuses wholesale and names the offending card, the same rule every other
  batch here follows. Refused rather than silently redirected into the title:
  appending `status:: done` to a card's visible title is clutter nobody asked for.

  The same class as the `append` guard (v0.118.0) and worse in effect — append at
  least stored the text somewhere. *"A 200 that changed nothing anyone can see is
  the worst answer available"* was already written down; this route had not been
  held to it.

## [0.146.0]

### Added
- **Something answers the channel now** — the `claude` plugin, and it is the half
  that makes channels worth having. The operator put it plainly: *"I have to tell
  you I replied? why wouldn't I just write one message in the terminal?"* Exactly
  right. A card that only an already-summoned agent ever reads is **strictly worse
  than a terminal**, because summoning the agent is the thing you were trying to
  avoid. v0.143.0 shipped the transport and called the feature done.

  Trellis already runs plugins **on-change**, mints them a scoped token and gates
  them behind approval — so the waker is a plugin, not a new daemon nobody
  remembers to start. It asks `GET /api/channels?agent=claude` what is addressed
  to it, reads each conversation past a stored per-card cursor, and for anything
  that is not its own runs `claude -p` and posts the answer with `say`. The card
  can now **start a turn**.

  Guards, because each answer is a real model call: it never answers its own
  words; a channel seen for the first time adopts its current position rather than
  working through a backlog; at most three answers per run; a failure is reported
  and **nothing is posted**, with the cursor left where it was so the message is
  answered next time rather than lost.

### Fixed
- **`seq: 0` was permanent, so `?since=` could never advance past it.** Unheaded
  operator text is returned on every read regardless of the cursor — deliberately,
  so a message typed on the phone is never missed. Left unnumbered for ever, that
  makes the cursor useless: **the plugin answered the same question on four
  consecutive runs**, an unbounded sequence of model calls, and only stopped
  because it was being run by hand.

  `say` now gives unheaded text at the **end** of the body a header on its way
  past, stamped `operator` and numbered. `seq: 0` becomes the brief state between
  typing and being answered rather than a place a message stays, and `?since=` is
  a real cursor for **every** client rather than one each has to special-case.
  Text wedged *between* two messages is left exactly as written — renumbering
  somebody's words to tidy the file is worse than the untidiness.

- **The plugin's replies were attributed to `operator`.** A scoped token carries
  its own label and the server rightly prefers it, but run under the instance key
  there is no label — so the reply came back as the operator's, and the loop guard
  read the plugin's own words as something to answer. It sends `X-Agent` as well
  now, which is correct under either credential. Both defects were found by
  running it against a live channel, not by reading it.

## [0.145.1]

### Fixed
- **A channel card did not say it was one.** `card_json` emitted `view` but never
  `channel`, so `GET /api/cards/{cid}` and every basket listing described a
  conversation as an ordinary card.

  Not cosmetic: the **Android** Channel dialog shipped an hour earlier reads that
  field to decide whether it is *creating* or *editing*. With it missing the phone
  would always have offered "Make it a channel", never *Stop being a channel*, and
  tapping Update on an existing channel would have **reset its participants** to
  the defaults and cleared `primary`.

  Found by auditing the day's work for exactly this class after the operator asked
  whether the API/UI split had been half-done anywhere else — not by a test, and
  not by using it.

## [0.145.0]

### Added
- **Make a channel from the app** — card menu → **Make a channel…** (or
  *Channel…* once it is one). v0.143.0 shipped channels with **no way to create
  one except the HTTP API**, so the operator's own feature was reachable only by
  an agent or a `curl`. That is the missing half, and it should not have shipped
  without it.

  The window names who the card is addressed to (comma-separated), offers the
  **workspace's own channel** tick, and — once it is a channel — a *Stop being a
  channel* button that leaves the card and everything said in it exactly as they
  are. Names are validated against the same rule the `X-Agent` header is held to,
  because a name is written into a message header line.

  **The one-primary-per-project rule now has one implementation.** It moved to
  `Document::other_primary_channel`, called by both the API's `PATCH` and this
  window. Two copies of a uniqueness rule is how two surfaces end up disagreeing
  about what is legal, and the operator finds out by way of a document with two of
  something that should be one.

- **Android can make one too** (viewer **v0.37.0**, versionCode 44) — card reader
  menu → **Channel…**, with the same fields and the same name rule. The phone is
  the half that matters most here: a channel exists so the operator can talk to an
  agent from the sofa, and needing a terminal to create one defeats the point. The
  card is re-read before the dialog opens, so the buttons reflect what the card
  actually is now rather than what it was when the screen opened — an agent may
  have changed it in between.

### Fixed
- **The Channel window drew only while the Backup window was open.** Its call had
  been nested inside `if self.show_backup`. It compiled, the menu item worked, the
  action fired and the state was set — and nothing appeared. Found by driving the
  real UI: right-click, click the item, screenshot. Every part of that path was
  correct except the one line no test looks at.

## [0.144.1]

### Fixed
- **A long node title made the sidebar wide, and it would not go back.** Reported
  in those words. One 59-character title — `Prod Health Watch — INSTANCE 1: LINK /
  sfo3 / 64.23.236.178` — pushed the tree panel from its 240 px default to about
  **450 px**, and dragging the splitter in **snapped it straight back out**.

  Three things had to line up, which is why it looked arbitrary: a
  `SelectableLabel` lays its text out at natural width; the tree's `ScrollArea` is
  **vertical-only**, so a wide row has nowhere to go but outward; and egui clamps a
  resizable `SidePanel` to its content's minimum, so the row was setting a floor
  the user could not get under. It only bit in a project that *had* such a title,
  and only while that branch was expanded — which is exactly why it read as "for
  some reason".

  A tree row is now laid out against `ui.available_width()` and truncated with an
  ellipsis, so a row can never dictate the panel's width. The panel drags freely
  in both directions and the label re-truncates to match. Nothing is lost: the full
  title is on the row's hover tooltip, and a deeply nested row truncates sooner
  because it genuinely has less room.

  Verified by reproducing it — a fresh instance with that exact title, measured
  before and after, then dragged narrow and hovered.

### Known-unverified
- **A channel card has not been looked at on a phone.** v0.143.0 said it "renders
  as an ordinary note on the canvas, in the exports and on the phone". The first
  two were verified by rendering one and looking (and that is how the setext-rule
  defect in v0.144.0 was found). The **phone** was reasoned, not checked, and the
  reasoning only covers the *card reader*: headings and thematic breaks are core
  CommonMark, which Markwon core handles. The **basket canvas** is the specific
  unknown — it draws a body onto a bare `Canvas` with no TextView, which is
  exactly where Markwon's `TableRowSpan` failed before and needed `flattenTables`
  as a workaround. A `ThematicBreakSpan` is the same kind of object. Recorded
  rather than assumed; needs a device.

## [0.144.0]

### Added
- **Click an inline `code span` to copy it.** Reported from real use in the
  sharpest possible form: getting a wallet address out of a card meant selecting
  it by hand out of a rendered **table cell**, and coming away with the whole line.

  There was nothing to click. The renderer draws a span with `ui.label`, which is
  not a hit target at all, so the only copy affordances a text card had were the
  **whole body** button and the **code-block** button — neither of which is the
  thing you wanted. A span is exactly where the un-typeable values live: an
  address, a hash, an id, a service name, a path. Those are the things you need
  *exactly*, and precisely what a hand-drag gets wrong by a character.

  Hovering shows a pointer cursor and **Click to copy**; clicking copies just that
  span and the tooltip says **Copied** for a moment, because a copy that gives no
  sign is indistinguishable from a click that missed — which is the complaint being
  answered, and it would be perverse to reproduce it in the fix.

  Only in the plain path: inside a link the click belongs to the link, inside an
  image it is alt text, and a code *block* keeps its own copy button. Works inside
  markdown tables, which is where this was reported.

  In the vendored renderer, so the fix is durable — the same reasoning as the
  earlier vendored patches.

### Fixed
- **A channel message ended in a heading instead of a rule.** `say` wrote the
  closing `---` directly under the message text, and in CommonMark text
  immediately above `---` is a **setext heading underline** — so the last line of
  every message was silently promoted to an H2 and the divider was never drawn.

  Invisible to every test, and they all passed: `parse_channel` only asks whether a
  line is `---`, which it was. It took **rendering a real card and looking at it**.
  There is now a test that asserts the *bytes* — a blank line above every
  terminator — because the bytes are what the renderer reads and the parser cannot
  tell the difference.

## [0.143.1]

### Changed
- **The notify plugin (v1.2.0) quotes a channel message instead of counting it.**
  v0.143.0 shipped channels and left the notification saying *"an agent made 1
  change"* — the one thing you cannot act on from the sofa, and the half of the
  feature that makes it a conversation rather than a log you have to go and read.

  A change-log entry deliberately holds no content, so the text is fetched:
  a `channel.say` entry is followed to `GET /api/cards/{cid}/channel?since=…`
  against a **per-card cursor** kept in the plugin's state, not the change-log
  position — the two count different things, and a channel's own `seq` is the one
  that survives the log rotating.

  - **Your own messages are skipped.** They are `from: operator`, typed on the
    phone a moment ago; telling someone what they just said is noise.
  - **A card seen for the first time reports only its newest message**, rather
    than replaying a conversation's whole history the first time it is touched.
  - **Quoted messages and ordinary edits are kept apart**, so a conversation does
    not read as *"3 edits"*, and a card whose message was quoted is not listed
    again below it.

- **The plugin escapes HTML.** It never did. That was survivable while every label
  was a card title and became a real bug the moment arbitrary message text went
  into the same string: Telegram parses these messages as HTML, and one `<` makes
  the Bot API reject the whole notification with a 400 naming a byte offset. Card
  titles, property keys and values are escaped now as well.

  Installed into both instances' `<data-dir>/plugins/`, because a plugin release
  does not install itself — the trap that cost a day of link-less notifications on
  2026-08-19.

## [0.143.0]

### Added
- **A card can be a conversation.** Give any card a `channel` and its body becomes
  a running log: the operator writes into it from the desktop or the Android app,
  an agent reads it, replies into it, and the reply arrives on the phone through
  the notification plugin that already exists. Point **two agents** at one card and
  it is an agent-to-agent log the operator can read — and interrupt — in real time.

  ```
  PATCH /api/cards/{cid}  {"channel":{"participants":["alice","operator"],"primary":true}}
  POST  /api/cards/{cid}/say          {text}        ·  POST /api/nodes/{id}/cards/{cid}/say
  GET   /api/cards/{cid}/channel[?since=<seq>]      ·  GET  /api/nodes/{id}/cards/{cid}/channel
  GET   /api/channels[?agent=<name>][&project=<id>]
  ```

  - **A field, not a `CardKind`.** A channel does not *render* differently, which
    is the only thing that would justify a variant — and the note on `CardKind`
    now says what one really costs (below). Not a `channel::` property either: a
    property fires on prose *about* channels, the false-property class this
    project has now fixed three times.
  - **The body is the log.** Messages are appended as
    `### @alice · <rfc3339> · #7` blocks closed by `---`. A Markdown heading and a
    rule, on purpose: the card renders as an ordinary note on the canvas, in the
    exports and on the phone, with **no new UI anywhere**.
  - **A message needs an end, not just a start.** Without the closing `---` a block
    runs to the next header, so anything the operator types at the *bottom* of the
    card — the natural place — is swallowed into the last agent's message and
    attributed to it. That is the exact confusion a channel exists to remove. Cost:
    a lone `---` inside a message splits it, so `say` reports `split: true` rather
    than refusing text the caller meant.
  - **Anything without a header is attributed to the operator.** There is no "post
    a message" affordance in the Android app and there never needs to be one:
    typing into the card *is* the person talking. Such a message has `seq: 0`,
    which no written message ever has, so it is returned regardless of `?since=`.
  - **`participants` is addressing, not an access list.** It is how an agent finds
    its conversations (`GET /api/channels?agent=alice`) without being told a card
    id. A message from a name not on the list is recorded under that name, because
    an agent leaving a finding in another project's channel is the point.
  - **One `primary` channel per project**, refused by name against the card that
    already holds the flag — but a *non*-primary channel in the same workspace
    stays legal, because an agent-to-agent card is a second channel there and
    forbidding it would forbid half the feature.
  - `seq` is stored, not counted from the body, because the operator can edit that
    body by hand and counting would renumber every earlier message.

- **`X-Agent: <name>` says who is calling**, on any request. It attributes a
  message, and it lands on `GET /api/changes`, so *which* agent made a change is
  answerable — until now `actor` said only `api`, and with several agents sharing
  one key that was unanswerable.

  **Declared, not derived, and that is the design.** Taking the name off the
  credential is the obvious approach and it fails the case this exists for: the
  normal deployment is several agents all holding the **instance key** so they can
  work across every workspace and leave findings in each other's projects, and a
  shared key names nobody. It is not a security boundary either — anything holding
  that key can already write any text under any name by editing the body. Where a
  scoped `agent_…` token *is* used, its label is authoritative and overrides the
  header, so the confined case keeps the stronger guarantee for free. The name is
  validated (1–40 chars, letters/digits/`-`/`_`/`.`) because it is written into a
  header line: a name containing ` · ` could otherwise forge a message boundary and
  put words in another agent's mouth. `SayInput` has **no** `from` field at all, so
  a caller trying to sign as somebody else gets a 400.

### Changed
- **What a new `CardKind` actually costs, measured instead of repeated.** *"~180
  exhaustive match sites"* had been carried in `model.rs`, `API.md`, the VR
  proposal, the workspace's *Project facts* and *Release Log* cards, and the
  Refresher prompt — and **had never been run**. Adding a seventh variant and
  compiling gives **14** failing sites. The real cost is the part the compiler does
  *not* find: 26 of the 44 `CardKind` match blocks carry a `_ =>` arm, plus ~20
  `if let` and ~12 `matches!`, all of which compile clean and silently exclude the
  new kind — blank render, empty export, invisible to search, an empty rectangle on
  Android, which is a separate repo dispatching on kind *strings*. The note on
  `CardKind` now states that, and says how to re-take the numbers rather than quote
  these.

## [0.142.0]

### Added
- **The other half of "a card id is a complete address."** Sixteen routes that
  had only ever existed in the `/api/nodes/{id}/cards/{cid}/…` form now take a
  bare card id:

  ```
  POST   /api/cards/{cid}/table    {op, …} | [{op, …}, …]
  POST   /api/cards/{cid}/sketch   {op, …}
  POST   /api/cards/{cid}/chart    {kind, …}       DELETE /api/cards/{cid}/chart
  POST   /api/cards/{cid}/dock     {anchor}        DELETE /api/cards/{cid}/dock
  POST   /api/cards/{cid}/group    {group}         DELETE /api/cards/{cid}/group
  GET    /api/cards/{cid}/attachments              POST   …/attachments {name, data_base64}
  GET    /api/cards/{cid}/attachments/{idx}        DELETE …/attachments/{idx}
  POST   /api/cards/{cid}/images   {data_base64}
  GET    /api/cards/{cid}/images/{idx}             DELETE …/images/{idx}
  GET    /api/cards/{cid}/export?format=markdown|html|json
  ```

  **Their absence was never a decision.** v0.117.0 shipped eight card-addressed
  writes, v0.118.0 added `append` and the single-item pair, and the kind-specific
  ops were simply not reached — nothing in the changelog, the reference or any
  session report records a reason. The gap then hardened into a *rule*: a
  workspace card described the basket form as "still the only form for the
  kind-specific ops", which reads as a design decision and was only ever a
  description of where the work stopped. The symptom was
  `POST /api/cards/{cid}/table` answering **404** while every other way of naming
  that card worked, on a surface whose whole premise is that an id is an address.

  **Still one implementation, not two.** Each new route parses its body and hands
  a `CardOp` to the app loop, which resolves the id to a basket and rewrites it
  into the ordinary node-addressed request — so the scope check, the mirror check,
  the change log and the apply code are the audited ones. The table body's
  shape-sniffing and the base64 decodes moved into shared helpers rather than
  being copied, because those are precisely where a second copy would have
  drifted, and a test asserts both forms read the same body.

  **The scope check is pinned for every one of them**, at both ends: unresolved
  they name no basket and are refused; resolved they are checked against the
  basket the card actually lives in. A `CardOp` that forgets this is a new
  unchecked end, which is how a confined token could carry its own card out of its
  basket until v0.111.0.

  Verified live against a scratch instance: **404 before, 200 after**, on all
  sixteen — including a batch of table ops, an attachment round-tripped by bytes,
  an image added and deleted on an image card, and a markdown export whose YAML
  frontmatter carries the card's `status::` and `#tag`.

### Fixed
- **Both route-parity tests could be satisfied by a *different* route.** They
  asked only that a doc line contain a route's literal segments *in order*, so
  `api` … `cards` … `table` was answered by the **node**-addressed line — and all
  sixteen routes above passed both tests before either surface mentioned them.
  They now compare the **whole path** with ids blanked (`/api/cards/{}/table`),
  and blank any `{…}` rather than a fixed list of placeholder names, because that
  list was itself stale: the template routes' hole is `idx` in the matcher and
  `{index}` in both doc surfaces.

  Two things fell out of tightening it:

  - **An elided path no longer counts.** The panel wrote `(unstick: DELETE
    …/dock)`, which stands equally well for the node-addressed route and its new
    twin. Six routes had only ever been listed that way; they are spelled out now.
  - **The route scan was reading its own tests.** It keys on lines starting with
    `(Method::`, which a test-case tuple also does — one carrying a JSON array in
    its body argument parsed as the path `/{"op":"insert_row` and was reported
    undocumented. A real arm's first segment is the URL's, and there are only
    three of those.

  448 → **450** tests.

## [0.141.2]

### Fixed
- **The Endpoints panel is now held by a test, not by whoever remembers it.**
  v0.141.1 fixed the panel by hand after a read caught it, and said out loud that
  **nothing enforces reference → panel**. That sentence was the defect. It is a
  test now — `every_route_appears_in_the_endpoints_panel` — so a route that never
  reaches Settings → Endpoints fails at the commit that added it, the same way
  `every_route_is_documented_in_the_reference` has held API.md since v0.120.0.

  It found two the hand-check had not, both there since the day they shipped:

  - `GET /api/nodes/{id}/cards/{cid}/export?format=…` (v0.123.0) — **one card as
    a note file**, with YAML frontmatter written from its properties and tags.
    The panel listed only the whole-document `/api/export`, so the route that
    lands a card in Obsidian intact was unfindable from inside the app.
  - `POST /api/nodes/{id}/cards/{cid}/append` (v0.118.0) — the node-addressed
    twin. Every other `…/cards/{cid}/…` op is listed in that block; this one was
    only reachable via the card-addressed form further up.

  **Matched on the path, not the method.** The panel is written for a person and
  folds a verb pair onto one line — `POST …/dock  (unstick: DELETE …/dock)` — so
  a test demanding the method sit before the path reports a dozen routes that are
  plainly there. The path is what makes a route findable; the wording around it is
  the panel's business. Proved in both directions: the test fails naming the route
  when either new line is removed.

  Docs-only, and a version anyway, for the reason v0.141.1 gave: API.md is
  compiled in since v0.120.0, so `GET /api/docs` cannot serve a correction that
  has not shipped.

## [0.141.1]

### Fixed
- **The in-app Settings → Endpoints list catches up** with the two routes that
  shipped today: `GET /api/cards/{cid}/mentions` (v0.140.0) and
  `GET /api/cards/{cid}/graph` (v0.141.0). API.md had both — the parity test made
  sure of that, twice — but **nothing enforces reference → panel**, so that half
  drifts silently and only a read catches it.

  Docs-only, and a version anyway: API.md is compiled in since v0.120.0, so
  `GET /api/docs` cannot serve a correction that has not shipped. That is the
  price of docs that cannot drift from the build, and it is the right way round.

## [0.141.0]

### Added
- **The local graph** — card menu → *Local graph…*, and
  `GET /api/cards/{cid}/graph?depth=2`. One card's neighbourhood, grouped by how
  many links away each card is.

  `GET /api/graph` is whole-document and **basket**-level: it answers *how do the
  projects connect*. This answers *what is around **this***, which is the question
  you have while reading one card — and in a journal-shaped document a basket is a
  **day**, so a basket-level edge says almost nothing.

  - **Both directions.** A card you link to and a card that links to you are
    equally its neighbours; following only out-links would make the answer depend
    on which end you happened to write the link from.
  - **Breadth-first**, so the depth reported is the *shortest* path rather than
    whichever the walk took first.
  - **Bounded at 200 cards, and it says when the bound bit.** A hub card links to
    everything, and a "local" graph that returns the whole document is not local.
  - Depth defaults to 2 and is clamped to 1–5: two hops is a neighbourhood, more
    is a hairball with extra steps.

  **A list, not a hairball.** The Link graph window already draws the
  whole-document picture; at card level the useful question is *what is one link
  away, what is two*, and a ranked list answers it at a glance where a
  force-directed blob does not.

## [0.140.0]

### Added
- **Unlinked mentions** — card menu → *Unlinked mentions…*, and
  `GET /api/cards/{cid}/mentions`. Cards whose text **names** this card, by its
  title or any `alias::`, **without linking to it**. Backlinks answer *what points
  here*; this answers *what should*. Worth much more since aliases (v0.126.0),
  because a card is usually called several things in prose and only one of them is
  its title.

  The interesting part is everything it must **not** report:
  - **Whole-word, case-insensitive** — a substring match would report `Notes`
    inside `Notebook`.
  - **Never inside code** — a name in a fenced block or a code span is being
    *discussed*, not referred to. Same rule that stops prose about a property
    becoming one.
  - **Names under three characters are skipped entirely** — a card called `Go`
    would otherwise "mention" half the document, and a list that long is not read
    at all, which is worse than not offering one.
  - **A card that already links here is a backlink, not a mention**, so every row
    in the list is something you might actually want to turn into a link.

  The route arrived undocumented and the **route-to-reference parity test failed
  the commit**, which is exactly what it is for.

## [0.139.0]

### Added
- **Zoom to fit** — beside *Reset view*, frames every card in the basket. Zoom is
  clamped to 20% like every other zoom path, so a basket bigger than that can
  frame **says so on screen** rather than quietly doing less than the button
  reads. (The note needed a timestamp, not a flag: a click lasts one frame, and a
  message drawn only in that frame is a message nobody sees.)

- **Hold Ctrl while dragging a card to drag it freely** — Snap and Grid are both
  aids, and every aid needs a way to be overruled for the one card that has to sit
  just *there*. Ctrl+*click* toggles selection; Ctrl+*drag* was unused.

- **Hold Shift while dragging to lock to one axis** — whichever the pointer has
  travelled furthest along. Nudging a card sideways without losing the row it is
  lined up with was otherwise a steady hand. An exactly diagonal drag resolves to
  horizontal rather than flickering between the two as the pointer crosses the
  diagonal.

- **Level of detail below 45% zoom** — a card draws its **title only**, with a
  rule at the bottom edge to say there is more here than the strip shows. At that
  zoom the body is 5–6 px of line height: not read, just grey texture — while
  still laying out its full Markdown to produce it. Titles are what you navigate
  by when you are zoomed out to see the shape of a basket.

### Not built — it already existed
- *Clone card* was on the wanted list; **Duplicate** in the card menu has done it
  all along. Crossed off rather than built twice.

## [0.138.0]

### Added
- **Merge cards** — select two or more and press **Merge**. This is **extract's
  other half** (v0.135.0): extract moves text out and leaves a view of it, merge
  brings cards back together, and between them splitting and joining a note never
  means *copying* it.

  **Every `[[#id]]` pointing at an absorbed card is repointed at the survivor**
  before it is deleted. An absorbed card stops existing, and a dangling id-link is
  worse than a dangling title-link because an id carries no name to guess from.
  The rewrite reaches a checklist's `items` and a table's `rows`, not just bodies
  — neither keeps its content in `body`. `![[#id]]` embeds move too; a `|display`
  half is preserved exactly as written, and a group link (`[[#g12]]`) is left
  alone because it shares the `#` but not the id space.

  **The survivor is the topmost, then leftmost card**, so the same selection always
  merges the same way regardless of the order you clicked. Content is appended in
  that same reading order, and **each absorbed card's title becomes a `##`
  heading**, so nothing a card was called is lost by joining it.

  **A merged checklist renumbers colliding item ids.** Item ids are per-card, so
  two lists can both have an item 1 — and since v0.90.0 a dated item id *addresses
  a task*, so leaving a collision would point two tasks at one id.

  **Mixed kinds are refused by name**, and so are tables, images and sketches:
  joining a table to a checklist has no meaning that is not an invention, and
  inventing one silently is how content goes missing.

## [0.137.0]

### Added
- **"Move to…" on the selection.** With cards selected, a **Move to…** button
  beside *Group* and *Arrange* opens the **Ctrl+O palette as a basket picker**;
  pick one and the whole selection goes there.

  Shift+drag has selected cards and dragged them as one since v0.107.0, and the
  batch move has existed over the API since v0.112.0 — but the gesture that
  already means *these cards, together* could not move them to another basket.

  **The palette is reused rather than duplicated.** It already fuzzy-matches every
  node in the document; a second picker would be a second thing that matches
  differently. It gains a *purpose* instead: while picking a destination, the
  header says so and **card and group rows are hidden**, because you are choosing
  a place and a card is not one.

  **The arrangement travels.** Moving cards one at a time drops each at the
  destination's origin, so a layout you built is gone the moment it arrives. The
  selection's bounding box is translated as a whole and dropped **below
  everything already there**, which is also what stops it landing on top of the
  cards that were there first. The view follows to the destination, rather than
  leaving you in the basket the cards just left.

  **Group and dock membership are dropped**, because both are basket-local — a
  card cannot stay in a group that did not come with it. That is
  `move_card_to_node`'s existing rule, not a new one, and it is why moving a whole
  *group* remains a different operation: rebuilding a group gives it a new id and
  breaks every `[[#g…]]` written to it.

  Both baskets get an undo point, because both changed.

## [0.136.0]

### Added
- **Tail mode for a mirrored file** — right-click a mirroring card → **Tail mode**
  → last 50 / 200 / 1000 lines, or `PATCH {"source_tail": n}` over the API (`0`
  turns it off).

  A mirror (v0.78.0) shows a file **from the top**, which pins a growing log to
  its least interesting end and gives you no way to reach the other one. A tail
  shows the end, refreshes at **0.6 s instead of 3 s**, and the card's scroll area
  **sticks to the bottom**, so the newest line is on screen without scrolling.

  **The 1 MB mirror limit does not apply to a tail**, and that is the point rather
  than a side effect: the cap exists because a mirror loads the whole file, and a
  growing log was exactly the file it locked out. A tail seeks from the end and
  reads backwards in 64 KB chunks until it has the lines asked for, so the cost is
  proportional to what is shown rather than to the file.

  **A partial line at the seek boundary is dropped**, because the first line of a
  tail is the one place a half-line reads as real content — and invalid UTF-8 at a
  chunk boundary is trimmed for the same reason, since a chunk can land
  mid-character in a perfectly valid file.

  Turning tail on or off clears `source_mtime` so the next poll re-reads: the file
  has not changed, but what we want out of it has.

## [0.135.0]

### Added
- **Extract the selected text into a card of its own** — select in a text card's
  editor, press **extract**, and the text moves out into a new card with an
  `![[#id]]` **embed** left where it was.

  **This is "one task is one card, never copied" applied to prose.** Before embeds
  (v0.125.0) the only way to split a card was to copy text into a new one and
  leave the original behind — two sources of truth from the moment you finish.
  Extract *moves* the text and leaves a **view** of it, so there is exactly one
  copy, the card reads exactly as it did, and the two can never drift.

  - **The embed lands on its own line.** An embed is a block — it renders a whole
    card — so left inline it is swallowed into the surrounding paragraph. Newlines
    are added only where there is not one already, so extracting a whole paragraph
    does not pile blank lines up behind it.
  - The new card takes its **title from the first non-blank line**, so it is
    findable in the Ctrl+O palette (v0.132.0) instead of being one more
    "(untitled card)", and is **fitted** through the same path as *Fit to content*
    so it opens readable.
  - It **renders rather than opening in edit mode**: `add_card` opens a new card
    for typing, which is right when it is blank and wrong here — the text is
    already written, and edit mode would show it, and the embed left behind, as
    raw Markdown.
  - The button is **disabled with a reason** when nothing is selected, rather than
    present and inert.

  Merging two cards is the same feature's other half, and is not built.

## [0.134.0]

### Added
- **Hover preview of a `[[…]]` link** — rest the pointer on a link in a card body
  and the card, basket or group it points at appears in a popup, without
  navigating. **Embed when it should always be visible, hover when you just want a
  look**: `![[#id]]` (v0.125.0) is the same content permanently, this is the same
  content borrowed, so following a reference costs nothing and loses your place in
  nothing.

  A preview is a **glance, not a second canvas**: the target's title and the first
  twelve lines of what it holds, with a count of the rest. **Embeds inside a
  preview are not expanded** — a preview that recursively renders other cards is
  how one hover paints half the document.

  It reads `preview_text`, not `body`: a **checklist keeps its content in `items`
  and a table in `rows`**, so previewing `body` would show a working list as empty
  — the trap that once had an audit reach for delete. Unlike `searchable_body`,
  which joins everything with spaces because search only wants a haystack, this
  keeps the line structure, since a preview that has lost it is unreadable.

### Changed
- **Vendored `egui_commonmark` now renders links itself.** The upstream helper
  draws the link and **drops the response**, so nothing downstream could tell the
  pointer was over one. The rendering is otherwise unchanged, link-hook branch
  included; the hovered destination is published through egui's own data store and
  cleared at the start of every body render, so a preview closes when the pointer
  leaves rather than sticking to the last link it was on.

## [0.133.0]

### Added
- **Align, distribute and arrange a selection** — an **Arrange** menu beside the
  *Group* button, on the shift+drag selection that has existed since v0.107.0.
  Align left / centre / right / top / middle / bottom; distribute horizontally or
  vertically; re-lay as a row, a column or a grid.

  **This is not Autosort.** Autosort throws the arrangement away and re-lays the
  whole basket; these act only on the cards you picked and leave the rest alone.
  Every operation is a change of **position** only — nothing is resized,
  reordered, grouped or copied — so there is no model change and nothing to
  migrate.

  - **Alignment and distribution work on the selection's own bounding box**, so
    the outermost cards never move and the arrangement stays where you built it.
  - **Distribution equalises the gaps between adjacent cards, not their centres.**
    With mixed card sizes equal centres leaves visibly uneven space, which is the
    thing you were trying to fix. It needs three cards — with two there is one gap
    and nothing to equalise — so those two items are *disabled with a reason*
    rather than present and inert.
  - **Row, column and grid re-lay in reading order of where the cards already
    are**, so the order you arranged them in survives being arranged. The grid
    uses a uniform cell (the widest and tallest card, plus the canvas grid step),
    because a ragged grid is not an arrangement.
  - A card already where the layout wants it is **not written at all**, so an
    arrangement that changes nothing changes nothing — and the change log gets one
    entry naming how many cards moved, not one row per card.

  It needed a new action: `MoveCard` deliberately drags the *whole* selection when
  the card it names is in it, which is right for a drag and exactly wrong here,
  where each card goes where the layout puts it.

## [0.132.0]

### Added
- **Callouts render as titled coloured blocks** — `> [!warning]`, `> [!note]`,
  `> [!tip]` and the rest of Obsidian's set, motivated rather than cosmetic now
  that vault import (v0.124.0) means they arrive in real content.

  **The premise turned out to be half wrong, and probing it first changed the
  work.** The card asking for this said callouts render "as a blockquote with a
  stray `[!warning]` in it". Rendered on a real card, `[!note]` and `[!warning]`
  already drew correctly — the renderer ships GitHub's five alert types and had
  them on by default. What was actually broken was narrower and worse:
  - **Obsidian's wider set** (`info`, `bug`, `question`, `success`, `example`,
    `failure`, `abstract`, `todo`, `quote`, and their aliases) fell through to a
    literal. Now mapped, in one bundle.
  - **A same-line title killed the callout outright.** `> [!tip] Custom title`
    lost the *type* as well as the title, because the alert parser reads every
    text event up to the first break to find the identifier and the title is
    swallowed into it. `split_callout_titles` moves the title onto its own line as
    bold text, so both the type heading and the title survive.

  Every identifier maps onto one of the **five glyphs the bundled font is known to
  draw**, separated by colour rather than by picking a more apt character: emoji
  are monochrome outlines here by standing decision, and a glyph outside the
  bundled font renders as a hollow box, which has bitten this project before. An
  approximate icon that draws beats a perfect one that does not.

  The rewrite runs in the HTML export too, in the same order, so an exported card
  still matches the card it came from.

- **Card titles in the Ctrl+O switcher.** The palette could resolve a card by its
  **id** since v0.87.0 but never by its **name**, so the one thing you actually
  remember about a card was the one thing you could not type. Rows show
  `card #<id>` and the basket path, and Enter reveals the card as the id rows
  already did.

  Three rules keep it *reach* rather than *discovery*, which is Ctrl+F's job: only
  the **title** is matched, never the body; a card with **no title** is skipped,
  because matching its body-derived label would fill the palette with rows nobody
  can predict; and an **empty query** offers no cards at all, where the fuzzy
  matcher matches everything and the list would become every card in the document.
  Cards sort after every basket rather than interleaving by score — the same call
  the id rows already make — so the palette's first screen never changes shape
  because a card happened to score well.

## [0.131.0]

### Added
- **Zoom to selection.** A button beside *Reset view*, shown only while cards are
  selected, that frames the selection: it zooms and pans so the selected cards'
  bounding box fills the canvas with a tenth left as breathing room. The canvas
  already knew the box from the marquee (v0.107.0); this is the other direction.

  The scale obeys the same `MIN_ZOOM`/`MAX_ZOOM` clamp as the scroll wheel and
  `Ctrl +`, so framing one small card cannot leave the canvas at a zoom no other
  path could produce, and a zero-area selection keeps the zoom it had rather than
  asking for an infinite one. A selection is a set of card **ids** and a basket
  may not hold all of them, so a selection naming nothing here frames nothing
  rather than framing the empty corner at the origin.

- **Random card** (*View → Random card*) — open something at random, anywhere in
  the document. Genuinely useful for rediscovery once a document is this old.

  **Uniform over cards, not over baskets.** Picking a basket and then a card
  inside it would make a card in a two-card basket far likelier than one in a
  basket of fifty, which for rediscovery is backwards — the crowded baskets are
  where the forgotten things are. The draw is across the flattened list, from the
  OS CSPRNG that already mints the API key rather than a new dependency.

## [0.130.0]

### Added
- **Snap to grid.** A third toggle, **Grid**, beside Dock and Snap: a dragged or
  resized card lands on the 32-unit grid the canvas already paints. Off by
  default, like Snap and Dock, and remembered per instance. Over the API it is
  `grid_mode` on `GET`/`POST /api/settings`.

  **Snap wins over Grid, per axis.** Object snapping (that *is* Snap) runs first;
  an axis that aligned to another card's edge keeps that alignment, and only an
  axis no card claimed is quantised. The other order would drag a deliberate
  edge alignment back off by up to half a step, so turning Grid on would quietly
  break Snap — which is why the rule is a named function (`grid_after_snap`) with
  a test per case rather than a condition buried in the drag handler.

  **One constant, so the grid you snap to is the grid you can see.** `draw_grid`
  used its own `32.0` literal; both now read `GRID_STEP`, pinned by a test. A grid
  you snap to but cannot see is worse than no grid.

  **Resize quantises the resulting edge, not the size.** A card whose top-left is
  already on the grid then has *both* corners on it, which is what "on the grid"
  has to mean for outlines to line up. Both the drag and the resize track the
  pointer's intended position rather than the frame's delta — quantising a delta
  would round most frames to zero and the card would never move at all. Resize
  needed the same grab-offset memory the move path already had.

  A card out on the desktop (v0.114.0) is a real OS window, not a card on the
  canvas, so nothing quantises it.

## [0.129.0]

### Fixed
- **A long checklist item wraps, and can be read.** Every item was laid out on
  one line and clipped at the card's edge, so a working list — the shape this app
  pushes you toward, since a dated checklist line has been a task in its own right
  since v0.90.0 — showed only its first hundred-odd characters per row. The cause
  is one layout call: both view branches paint the row inside `ui.horizontal(…)`,
  which hands its children **unbounded width**, so `ui.label` and the link
  `LayoutJob` had no width to wrap at. `job.wrap.max_width = ui.available_width()`
  looked like it set one and did not.

  The row now measures the card's usable width **outside** the horizontal layout,
  subtracts whatever the row has already spent (checkbox, and the grip in edit
  mode, taken from the cursor rather than from constants), and hands the text that
  as an explicit wrap width — which is what the *edit* branch has always done for
  its field. Link runs wrap with it, and the click test still asks the galley
  which character was hit, so following a `[[#id]]` in a wrapped item lands on the
  right run.

- **Fit to content sizes a checklist for the layout it now really draws.**
  `fit_size` counted one row per item, which was correct only while items did not
  wrap. It decides the width first and measures each item at the width it wraps
  to, the same order the `Text` branch uses. The list that prompted this — eight
  items averaging ~250 characters — fitted to **258 px** before and **458 px**
  now, with every item visible and no empty gap under the last one.

  **This is v0.128.2's diff, and v0.128.2 was right about the arithmetic and
  wrong about the renderer.** It shipped this sizing while items still rendered on
  one line, so it sized for a wrap that never happened and was reverted the same
  hour. It is correct now only because the renderer was fixed first. The two must
  move together, and the test that pins the height says so — that assertion has
  now been written in both directions, and both versions passed, because the
  number it pins is meaningless without the renderer beside it.

- **A checkbox sits beside the line it belongs to.** Making items wrap exposed
  that the row was `ui.horizontal(…)`, which **centres** its children: the row's
  height is settled from the checkbox before the text block grows past it, so the
  box ended up floating above the item's first line. The row is `horizontal_top`
  now. Invisible while every item was one line, wrong the moment they wrapped —
  and reported from the screenshot taken to check the wrap, which is the second
  time in this cycle that looking at the render found what reading the code did
  not.

  Verified the way the last two attempts were not: by fitting the real card in a
  scratch instance and **looking at it**.

## [0.128.3]

### Fixed
- **Reverts v0.128.2, which was wrong.** That release made *Fit to content* size a
  checklist as if its items wrapped onto extra rows. They do not: both view
  branches paint inside `ui.horizontal(…)`, which gives its children unbounded
  width, so a long item renders on **one line and is clipped** at the card's edge.
  Sizing for a wrap that never happens made the card far too tall and left a large
  empty gap under the list — the opposite of the complaint it was meant to fix.

  The mistake was trusting a comment over the pixels: the renderer's own comment
  says the text "wraps within the card", and it does not. Caught by looking at the
  rendered card, which is the standard this project already sets for the emoji and
  clipboard fixes and which the first attempt skipped.

### Known, not fixed
- **A long checklist item is clipped and unreadable**, and no amount of fitting
  helps because the card's width is capped. That is the real defect underneath
  v0.128.2, it is filed in the workspace, and fixing it means making items wrap —
  at which point `fit_size` must measure each item at the width it wraps to, the
  way the `Text` branch already does.

## [0.128.2]

### Fixed
- **Fit to content sized a checklist for a layout it never renders at**, so a
  card with long items came out roughly a third of the height it needed and its
  last items were cut off. `fit_size` counted **one row per item**; but the width
  a long item wants is clamped to the 900 px maximum, and anything longer then
  **wraps**. Eight items averaging ~250 characters — a perfectly ordinary working
  list — fitted to **258 px** whatever they contained.

  It now decides the width first and measures each item at the width it will
  actually wrap to. That is the same order the `Text` branch has always used, and
  its comment describes the mirror-image trap: measure at a wrap width the card
  does not render at and it comes out *too tall*. The checklist branch had the
  inverse, and came out too short.

  It mattered more than it looks: since v0.90.0 a dated checklist line is a task
  in its own right, so "one card, many dated lines" is the shape this app pushes
  you toward — and `fit` was quietly useless on exactly that shape, in the UI
  (right-click → Fit to content) and over the API (`"fit": true`) alike.

## [0.128.1]

### Fixed
- **The in-app Settings → Endpoints list had drifted from `API.md`.** Three routes
  shipped today — `POST /api/import/vault`, `GET /api/properties/problems` and
  `GET /api/cards/{cid}/run` — were in the reference and the route table but not in
  the list a person reads inside the app. A test enforces routes→reference; nothing
  enforces reference→panel, so that half drifts silently.
- **`API.md` gained worked Examples for all four of today's features.** The
  reference said what the endpoints are; the Examples section is the copy-paste
  pattern an agent actually starts from, and it had none for vault import, saved
  views, property problems or embeds.

## [0.128.0]

### Added
- **Saved views — a query you can keep, as a card.** Every other view here is
  **fixed**: Find cards, the Agenda and the Kanban each answer one question
  somebody else chose. Now you can say *"cards where `status:: blocked` and
  `due::` is this month, by date"* and keep it. The **Find cards** panel already
  builds a query, so it grew a **Save as view card** button — that is the on-ramp.

  A view is an ordinary card carrying an optional **`view` field**, beside
  `chart`, `source` and `attachments`. **Not a `CardKind`** (a seventh variant
  costs ~180 exhaustive match arms and buys nothing — a view is a text card that
  draws something derived, exactly as a `source` mirror already is), and
  **deliberately not a `view::` property**: a magic property would fire on prose
  *about* views, which is the false-property class this project has already fixed
  twice. A switch that triggers on writing is a bug generator.

  **The rows are never stored.** They are computed on read, so a view cannot go
  stale — storing them would be the copy this app exists to prevent. An unrelated
  `PATCH` leaves a view alone; only an explicit `view: null` clears it.

  Filters AND together, with `eq` / `ne` / `lt` / `le` / `gt` / `ge` / `contains`
  / `exists`, over property keys or the pseudo-keys `title`, `basket`, `id`,
  `kind`, `touched`, `tag`, `text`. **Values compare as what they are**: two dates
  through the same `parse_ymd` the Agenda uses — so a view and the Agenda cannot
  disagree about what a day is — two numbers numerically, so `priority:: 10` is
  not below `priority:: 9`, anything else as text.

  Sorting puts cards with **no value last in both directions**, because an empty
  first row reads as a broken view, and `limit` truncates **after** the sort, so
  "top 5 by date" is the first five by date rather than five arbitrary rows put in
  order. A view never returns its own card. Over the API:
  `PATCH /api/cards/{cid} {"view": {…}}` and `GET /api/cards/{cid}/run`, which is
  the same function the canvas draws from — one implementation, not two that drift.

### Notes
- **Formulas, summaries and group-by are deliberately not in this version**, and
  neither are Agenda, Kanban and Find re-expressed on top of the new engine.
  Formulas are a small expression language needing an infinite-loop guard;
  rewriting three working panels onto a new engine is how three working panels
  break. Design agreed on [[#1934]] before any code was written.

## [0.127.2]

### Fixed
- **An imported canvas connector lost its direction.** Obsidian's connectors are
  **nondirectional**, **unidirectional** either way, or **bidirectional**, stored
  as an arrowhead on each end (`fromEnd` / `toEnd`). v0.124.0 read neither field,
  so all four imported as a forward arrow — a connector drawn as a plain
  association was silently promoted to a flow, and one drawn backwards pointed the
  wrong way. They now come in as `-->`, `<--`, `<->` and `---`.

  Found because the operator asked whether the feature scope had covered "card
  connectors". It had not: the canvas was scoped from its **file format**, which
  shows that `fromEnd` exists but not that it is a *user-facing choice with four
  settings*. The canvas's own menus live in a string table the first sweep never
  opened.

## [0.127.1]

### Fixed
- **A card showing more than one single-line editor at once stole the X PRIMARY
  selection from the whole desktop.** Select a cell's contents and type over it,
  move to the next cell and do the same — the ordinary way anyone fills in a table
  — and pasting stopped working *everywhere*, not just in Trellis. Reported as
  "something with the tables breaks my clipboard"; the reporter's hunch that it
  was "all tables" was close, and the actual rule is **more than one single-line
  editor visible at once**. That is a table (one per **cell**), a checklist (one
  per **item**), and two cards being edited at the same time (one per **title**).
  The fix lives in the helper all of them share, so all of them are covered.

  An unfocused `egui::TextEdit` keeps its last cursor range in memory, so a cell
  where text was once selected goes on reporting that selection every frame for as
  long as the card stays in edit mode. One editor makes that harmless: the text
  never changes, so the dedupe in `set_primary_selection` stops after the first
  write. **A table is N editors at once** — two cells with different stale
  selections each defeat that single global dedupe, so Trellis took PRIMARY back
  and forth tens of times a second, killing and respawning the `xclip` that serves
  it. Any other application reading the selection raced an owner that was being
  killed.

  The fix is one condition: **only the focused editor may mirror its selection.**
  That is also the honest rule on its own terms — a selection you cannot see, in a
  widget that does not have the keyboard, is not the user's selection.

  Measured, old build against new, same fixture and the same clicks: with two
  cells selected, the old binary destroyed an externally-set selection
  immediately and then alternated between the two cell values on every sample;
  the new binary never wrote either one. Diagnosed from the live instance first —
  the value overwriting the operator's selection was traced by search back to a
  specific three-by-three table card, and it alternated between exactly two
  fragments of it.

## [0.127.0]

### Added
- **`GET /api/properties/problems` — the date properties this app cannot read.**
  `due::`, `start::` and `verify::` are the keys it *acts* on; a non-empty
  non-date in one of them makes a card **look** scheduled while it never reaches
  the Agenda, and nothing said why. `verify::` at least counted an unreadable date
  as stale — `due::` and `start::` were simply silent. That is v0.120.1's finding,
  finally given a surface.

  It also names a genuine surprise: a date-shaped property **stops at the first
  word**, so `due:: next friday` is read as `next`. The string in the card and the
  string the app holds are not the same, which is most of why the silence was
  confusing.

  A **checklist** is judged by title and items, never body, and since an item with
  its own `due::` is its own task the offending **line** is named. Keys the app has
  no opinion about (`owner:: ada`) are not flagged — burying three keys that matter
  under every key that does not is how a diagnostic stops being read.

### Notes
- **This is the useful half of "typed properties", and deliberately not the rest.**
  Obsidian gives every property a type because YAML is stringly and it edits
  properties in a side panel. Here `key:: value` is inline text the Agenda, Kanban,
  query and claims surfaces already interpret, so a type system would be a second
  syntax for something already working — the reasoning that kept frontmatter at
  the boundary rather than inside. What was actually missing was the diagnosis.
- Run against both live documents (973 and 419 nodes): **zero problems, zero false
  positives**, and one planted bad value found.

## [0.126.0]

### Added
- **A card can be reached by an alias.** `alias:: Start Here` (or
  `aliases:: Start Here, Front Door`) on a card makes `[[Start Here]]` open it.
  Obsidian notes carry `aliases:` in their frontmatter and a note becomes a
  **card** here, so until now every alias in an imported vault was inert text.

  **A basket still wins.** `[[Name]]` has always meant a basket, and links already
  written must keep meaning what they meant — so an alias is consulted only when
  no basket has that title. Additive by construction: it can rescue a link that
  used to dangle and can never redirect one that worked. Ties break the way
  duplicate basket titles do, **same project first then the lowest card id**,
  never `HashMap` order.

- **`[[#1391^766]]` names one checklist line.** Obsidian's block reference, in the
  id space this app already had: since v0.90.0 a checklist item with its own
  `due::` is a task in its own right and carries a **stable id**, so a line is a
  thing worth pointing at.

  The **link** resolves to the card, because that is what a reveal can scroll to
  and flash. What the item part buys is the **embed** — `![[#1391^766]]` shows
  that one line instead of pasting a 23-line working list in to point at one task.
  Deliberately **not** a new `LinkTarget` variant: the enum is matched in 35
  places and a variant every one of them would treat as "the card" is a cost with
  no reader. A reference naming no such item, or a card that is not a checklist,
  says so in the frame.

## [0.125.0]

### Added
- **`![[#id]]` shows a card inside another card.** The complement of `[[#id]]`:
  a link says *go and look at that*, an embed says *show it here*.

  It exists because of the rule this app is built on — **one task is one card,
  never copied**. Until now, seeing a card's content in two places meant
  duplicating it, and a copied task card is a second task with its own `status::`
  and `due::`, counted twice, with nothing warning you. An embed is the answer:
  one card, shown wherever it is needed, and editing it changes every view of it.
  Taken from Obsidian's note transclusion, which is the one thing a vault does
  that this app had no answer to at all.

  **A view, never the stored text.** The body on disk keeps `![[#id]]`, so
  `GET /api/cards/{cid}` returns what was written — expanding on save would be
  the copy the feature exists to avoid, and it is also what Obsidian writes, so
  an exported card still round-trips. Same rule as block-HTML conversion.

  A **checklist** embeds as its items and a **table** as its rows, because that is
  where those kinds keep their content — reading `body` would render an empty
  frame, which is the near-deletion `empty` was added to prevent, one layer along.
  An embed counts as a link for backlinks and the link graph.

  Three refusals, each **reported in the frame** rather than silently: a **cycle**
  (a card embedding itself, directly or round a chain — the
  `unconditional_recursion` shape that has shipped a crash here twice), nesting
  **more than four deep**, and a **target that does not exist**.

### Fixed
- **A test of the v0.124.0 canvas importer was hash-order dependent**, and failed
  the **Windows** release build while passing on Linux and macOS — so v0.124.0
  published three of its four assets. It looked up a card by title through
  `Document::nodes`, a `HashMap`, and the fixture has two cards called `Target`
  (the note, and the canvas node pointing at it). That is the same nondeterminism
  v0.121.0 fixed in link resolution; a test can have it too. Two neighbouring
  tests that relied on "the fixture happens to hold one card" were tightened to
  name what they mean.

## [0.124.0]

### Added
- **Import a whole Obsidian vault.** v0.123.0 taught the boundary to speak YAML
  frontmatter, one dropped file at a time — the *field* half. A vault is not a
  file, it is a **shape**: a folder tree, notes that link to each other by name,
  and attachments referenced with `![[file.pdf]]`. Importing it a file at a time
  loses every one of those relationships.

  **File → Import → Obsidian vault…**, dropping a **folder** onto a basket, or
  `POST /api/import/vault`. Folder → basket, note → **card**, frontmatter →
  `key:: value` / `#tags`, `![[file.pdf]]` → an attachment on the card that names
  it.

  **A note is a card, not a basket.** A basket is a *space* holding things and a
  note is a *thing*; mapping notes to baskets would build a tree of empty
  containers. It also settles the link question — Trellis's bare `[[Title]]`
  resolves to a **basket**, so every imported note link would dangle. Links are
  rewritten to `[[#id|Note]]` in a second pass, once every card exists and has an
  id, and the pipe keeps the name the author wrote. `[[Note#Heading]]` and
  `[[Note#^block]]` reach the card and keep the subpath as the label.

  **A link naming no note is left exactly as written** and reported, because a
  dangling link someone can read and fix beats content silently deleted. A
  **bare** name two notes share is deliberately not resolved — picking one would
  point half the vault's links at the wrong card while looking like it worked.

- **A `.canvas` becomes a basket.** [JSON Canvas](https://jsoncanvas.org) is an
  open format, and it is a Trellis basket almost exactly: nodes with `x`, `y`,
  `width`, `height`, arranged in space and boxed into labelled groups. This is
  the one file in a vault whose shape Trellis already has, so importing it as
  bytes on a card would take the only genuinely spatial thing there and make it
  unreadable.

  `text` nodes become cards, a `file` node **links** to the card its note already
  became rather than copying the text, a `link` node keeps its URL, a `group`
  becomes a card group read off **the geometry**, and an `edge` becomes
  `→ [[#id]]` with its label — which also puts it in the backlink index and the
  link graph, where an arrow drawn on a canvas was never visible at all.
  Coordinates shift so the arrangement lands on screen with every relative
  position unchanged; the layout is the content. Obsidian's `"1"`–`"6"` presets
  and raw `#rrggbb` both come across.

### Fixed
- **A dropped `.md` was titled with its extension.** `Glossary.md` became a card
  called "Glossary.md". A note's name is the file name **without** it — Obsidian's
  own identity rule, and what the vault importer uses.
- **A dropped folder did nothing at all.** It has no bytes to read, so it fell
  through the whole drop chain: no card, no error, no status line. Dragging a
  vault in is the gesture people try first.

### Notes
- Two defects were found by **driving the importer against a real vault** rather
  than by reading it: an `![[spec.pdf]]` that had imported perfectly was reported
  as a dangling note link, and `[[../../Reference/Glossary]]` — a link relative to
  the linking note's folder — did not resolve. Neither was visible in the code.

## [0.123.1]

### Fixed
- **A `body` on a card kind that cannot show one was silently dropped.** Creating a
  card with `kind: "checklist"` and a `body` carrying `due:: 2026-09-01` answered
  **201** and discarded the body: the card never reached the Agenda, and nothing
  said why. `PATCH` did the same. Reported by an agent it bit.

  The guard already existed — on `append`, and only there, whose own comment calls
  this outcome *"worse than a 400"*. `append` is the half an agent reaches last;
  **create is what everyone hits first**, because `kind` and `body` are chosen in
  the same call. The codebase disagreed with itself, and the unguarded half won.

  The check now lives in one function used by all three. It is judged on the kind
  the card **will be**, not the kind it is, so `PATCH {"kind":"text","body":"…"}`
  on a checklist still converts the card and keeps the body — testing the current
  kind would have broken a legitimate call that has always worked.

- **A batch create could stop half-way.** With the check above, a three-card batch
  holding one bad card answered 400 **having already created the cards before it** —
  the partial write this project refuses everywhere else. Unreachable until now:
  `add_one` could only fail on *node not found*, the same answer for every element,
  so the loop could never stop mid-list. The whole array is validated before
  anything is created, like the batch move, edit and delete.

## [0.123.0]

### Added
- **Drop any file into a basket and the bytes come with it.** A PDF, a spreadsheet,
  a zip, an audio file — stored **inside the document**, not as a path to it,
  because a path is worthless the moment the notes are opened on the phone,
  restored from a backup, or read by anyone else. Drop it *on a card* to attach it
  there ("the spec belongs to this task"), or on empty canvas for a card named after
  the file. Click an attachment to save a copy back out.

  **It replaces a silent failure.** Only images and UTF-8 text were handled; anything
  else fell off the end of the chain with no card, no error and no status line.

  **On the card, not in a new card kind.** A seventh `CardKind` touches ~180
  exhaustive match sites, and a file card could not express "attached to *this*
  task" anyway — so `attachments` sits beside `inline_images` and any card can carry
  one. They ride through card export/import and templates, and a card holding only a
  file reports `empty: false`.

  Over the API: `GET`/`POST /api/nodes/{id}/cards/{cid}/attachments` and
  `GET`/`DELETE …/attachments/{idx}`. The listing returns names and **sizes**, never
  the bytes.

  **The cost is stated rather than hidden.** The document is one gzip-compressed RON
  file written *whole* on every save, so an embedded file is re-serialised on each
  autosave and copied into every version-history snapshot and every backup archive.
  The app **warns above 10 MB and lets you go ahead** (the operator's call);
  `attachment_bytes` on `GET /api/instance` is the running total. The API sets no
  limit deliberately — a policy buried in the model is one a caller inherits without
  ever being told about it.

- **YAML frontmatter, at the boundary only.** Trellis does not adopt it as an
  internal model: `key:: value` already does that job, works on a single checklist
  line, and reaches a caller as parsed JSON rather than text to parse. What it is
  *for* is the edge, where other tools speak it.

  **Import** — a `.md` dropped in has its leading `---` block read: `tags:` becomes
  `#tags`, `title:` becomes the card's title, and everything else becomes
  `key:: value`. Without this an Obsidian note's `due: 2026-09-01` is inert prose,
  because the property parser needs `::` and YAML uses one colon.

  **Export** — `GET /api/nodes/{id}/cards/{cid}/export?format=markdown|html|json`
  (new) and **Copy → Markdown** emit the block, so a card lands in another tool with
  its dates, status and tags intact. The whole-document export deliberately does not:
  a file of many cards has no single set of fields describing it.

  A deliberate subset is understood — `key: value`, quoted scalars, `key: [a, b]`,
  and `key:` + `- item` lists. **Nested mappings are skipped, not flattened**, because
  guessing at structure is how an import quietly invents data; and an opening `---`
  with no closing fence is treated as ordinary content rather than swallowing the
  document.

### Changed
- `GET /api/instance` reports **`attachment_bytes`**.

## [0.122.2]

### Fixed
- **Colour emoji painted over whatever was on top of them.** Reported as "the emoji
  show through other windows on top of them", and reproduced: a card scrolled under
  the **minimap** had its title and body correctly hidden while its emoji floated
  over the map.

  Colour emoji are painted by this app rather than by the text renderer — the glyph
  is found in the frame's shapes and a coloured quad is drawn over it. That quad was
  **appended to the end of its layer's paint list**, and most of this app draws into
  one layer (`background`), where later means on top. So the end of the list is the
  very front, and every emoji was hoisted above the minimap, the toolbar, the status
  bar and any card drawn after its own.

  The colour now **replaces the entry the glyph came from** with
  `[original, colour…]`, so it sits at exactly the text's depth, inherits that
  entry's clip rect, and anything added to the list afterwards still covers it.
  Measured on screen against the minimap: 103 green and 176 near-white emoji pixels
  bleeding over it before, **0 and 0** after.

## [0.122.1]

### Fixed
- **A `[[link]]` in a checklist item was literal text.** Only the card title, the
  Markdown body and table cells ever linkified; a checklist item was painted with a
  plain label, so every `[[#1391]]` in one rendered as its own brackets and did
  nothing when clicked.

  That is the wrong half to have missed. Since v0.90.0 a **checklist item is a task
  in its own right**, so "one card, N dated lines, each pointing at the card it is
  about" is the shape this app pushes people toward — and every one of those links
  was dead. Reported from a work-instance bug queue whose eleven lines each named a
  card.

  Same treatment table cells already had: the runs are built, link runs get the
  link colour and an underline, and a click follows the run it actually landed in
  (asked of the galley, so it stays right at any zoom).

## [0.122.0]

### Added
- **A link a phone can actually follow — `GET /go/card/{cid}`** (and `/go/node/`,
  `/go/group/`). Where `/open/…` moves the window on the machine Trellis is running
  on, `/go/…` serves a small page that opens the target in the **Android app on the
  device that loaded it**, and leaves the desktop alone.

  It exists because of one constraint, measured rather than assumed: **Telegram
  silently strips a link with a custom scheme.** A message containing
  `<a href="trellis://…">` is accepted with `ok:true` and arrives with **no link
  entity at all** — the anchor is gone, and a bare `trellis://…` is not auto-linked
  either. Only `http(s)` survives. So a notification could not carry a tappable link
  to a card without an `http` hop, and this is that hop.

  The page builds the `trellis://` URL from **`location.host`**, the one address
  known to be reachable from the device reading it — a link minted here would say
  `127.0.0.1`, which on a phone is the phone. And it offers a **link to tap, not a
  redirect**: an automatic jump to a custom scheme is what in-app browsers block,
  while a user gesture is the case they allow. Unauthenticated, like `/open/…`, for
  the same reason: a link nobody can click buys nothing.

- **`lan_host` and `lan_hosts` on `GET /api/instance`** — the address(es) another
  device can reach this instance on, so a plugin can build the link above. Found by
  asking the routing table rather than enumerating interfaces (a UDP socket is
  `connect`ed and its own local address read back; **no packet is sent**).

  One probe was not enough, and the machine this was written on is why: its default
  route is a **VPN**, so "the route off this machine" was confidently the one
  address a phone cannot use. Each private range is probed as well and **RFC 1918
  is preferred over CGNAT**. It is still a hint — that box reports three addresses
  and only the reader knows which network their phone is on — so it is offered as
  `lan_hosts` too, and the notify plugin can override it.

- **The notify plugin (v1.1.0) links every card it names.** A digest line or an
  agent-edit line is now a link that opens that card in Trellis on whatever you
  tapped it from. `link_host` overrides the auto-detected address, which matters on
  a VPN.

### Fixed
- **A card that *quotes* the link syntax no longer acquires the link.**
  `wikilinks_to_md` and `extract_wikilinks` scanned raw text with no idea what code
  was, so a card documenting `` `[[Title]]` `` had its example rewritten and
  rendered as `` `[[Title]](trellis:Title)` `` — the URL leaking into text meant to
  read as literal source. Worse, `extract_wikilinks` feeds **backlinks and the link
  graph**, so a card explaining `` `[[Archive]]` `` appeared in Archive's backlinks
  as though it pointed there.

  Exactly the false-property defect v0.96.0 fixed one layer along, with the same
  remedy and the same two helpers. Measured against the operator's own document:
  **18 backlink hits and 7 graph edges removed, every one of them inside backticks**
  — `` `[[wiki-link]]` ``, `` `[[#g146]]` ``, `` `[[#1391]]` ``, and two
  deliberately-invalid examples (`` `[[#999999]]` ``, `` `[[No Such Basket]]` ``).
  **No real link was lost**: all seven `trellis://` forms the desktop accepts still
  resolve, and 23 genuine links in those same cards still count.

  A link is also **line-scoped** now. It could previously span a newline, producing
  a target with a newline in it that could never resolve — the defect going away,
  not behaviour being lost.

  The same hole was in the Android mirror, and is fixed there too (viewer v0.35.0).

### Changed
- **A response carries its own content type.** Every reply was labelled
  `application/json` from a fixed header, which was harmless while every consumer
  was a program that already knew — but a page meant for a **browser** is not
  rendered at all under the wrong type.

## [0.121.0]

### Fixed
- **A `[[Title]]` link could open a different basket after every restart.** The
  lookup was `self.nodes.values().find(…)` and `nodes` is a `HashMap`, whose
  iteration order Rust seeds **per process**. Measured against three baskets called
  `Archive`: the same link, in the same document, resolved to node **7, 7, 5, 3, 3,
  7** over six runs of the same binary.

  Duplicate basket titles are not an edge case — *"one `Archive` basket per
  project"* is the archiving convention this project has been promoting all week,
  and the operator's document has **47** baskets called `Archive` and 19 duplicated
  titles across 99 baskets. Reported by an agent working in another workspace, who
  noticed a card claiming generic names were prefixed to prevent exactly this.

  Two rules now, in order: **the linking card's own project wins**, then the
  **lowest node id**. So `[[Archive]]` written inside a project means that
  project's archive — the only reading anyone intends — and where that cannot
  decide, the answer is at least the same on every run and every machine. Card,
  group and numeric-id links are untouched: they were never ambiguous.

  Every caller that knows where the link was written now says so — backlinks (both
  card- and basket-level), the link graph, and the canvas click.

- **`GET` on a card now says whether it is `empty`.** A **checklist keeps its
  content in `items` and a table in `rows`, so neither carries a `body` at all** —
  and an agent auditing a workspace read two checklist cards as "completely empty"
  and came within one step of deleting them as noise. They held 23 lines.

  `empty` is computed per kind in one place, so nobody has to branch on `kind` to
  answer the question and there is one definition rather than one per caller. The
  title is deliberately not content: a titled card with nothing in it reports
  `true`, which is exactly the state worth noticing.

## [0.120.1]

### Fixed
- **A claim this project had been repeating about `due::` was wrong, in the docs
  and in a tooltip.** Three surfaces said that setting a date property to `""`
  leaves it *"present but unparseable, so the task sits under No date instead of
  leaving the agenda"*. Measured, on a scratch instance, both cases:

  - `due:: ` (empty) is **not parsed as a property at all**, so the card stops
    being a task and *does* leave the agenda. What it actually leaves is a dangling
    `due:: ` line in the body — untidy, and confusing to the next reader, but not
    an agenda problem.
  - The case that really traps you is a **non-empty value that is not a date**.
    `due:: soon` *is* a property, so the card is still a task, and with nothing to
    sort by it sits in **"No date"** indefinitely.
  - And `status:: done` is **already enough** to take a row off the agenda: the
    default `/api/tasks` and the Agenda panel both hide done rows (`?all=true`
    shows them). The docs implied it was not.

  `Document::tasks()` settles it in one line — `let Some((_, due)) = props… else {
  continue }` — so a card with no `due` is never a task, which is why the *"No
  date"* bucket cannot mean *"no date"*.

- **The Agenda's "Clear date" tooltip described its own button backwards.** It
  said the task *"moves to No date rather than leaving the agenda"*; the button
  calls `clear_card_property` / `clear_item_property`, so the row goes entirely.

  This one is worth naming as a class: the wrong explanation had been copied
  forward into `API.md` twice, an in-app tooltip and two Prompt Manager prompts,
  and every copy sounded confident. A `check::` on a card is supposed to be a
  command whose output disagrees with the card — the same standard applies to a
  sentence in the docs, and nobody had run one.

### Note
- Because `API.md` is compiled into the binary since v0.120.0, **a documentation
  correction now requires a release** for `GET /api/docs` to serve it. That is the
  cost of the docs being unable to drift from the build, and it is the right way
  round — but it does mean doc-only fixes ship as versions.

## [0.120.0]

### Added
- **`GET /api/docs[?section=<name>]` — the reference, served by the instance that
  implements it.**

  Not a second copy of the docs: `API.md` is `include_str!`-ed at build time, so
  what comes back *is* the file, as of the commit the running binary was built
  from. There is nothing to keep in sync, which is the only reason this is worth
  having rather than a summary someone has to remember to update.

  Two things it fixes. **An agent that is not on this machine can read the
  reference at all** — every prompt and runbook says *"read
  /media/veracrypt1/Rust/trellis/API.md"*, which needs this filesystem, and the
  phone, a LAN agent and anything else holding a token have neither. And it answers
  the question that actually costs time: not *what does the API do* but **what does
  the API this port is serving do**. Twice in one day a route was documented that
  the serving instance did not have — the mirror of *a build is not an install*.
  `GET /api/instance` gives the version; this gives that version's manual.

  `?section=` matches a `##` heading case-insensitively on a substring, so
  `?section=example` finds *Examples*; the whole reference is ~100 KB and an agent
  rarely needs more than one part. `sections` is returned either way, so one call
  orients and the next is narrow. Allowed at **any** scope, including a
  basket-confined token: it is static text with no document content, and an agent
  that cannot read how the API works is not confined, just broken.

### Changed
- **Route-to-reference parity is a test now, not a runbook step.** The release
  runbook said: regex the route matcher, normalise the placeholders, compare
  against `API.md`, expect zero missing. Which meant it ran when someone
  remembered. `every_route_is_documented_in_the_reference` parses the route table
  out of `api.rs` at compile time and fails naming any route absent from the
  embedded reference, so an endpoint that never reached the docs breaks the build
  that added it rather than a session three weeks later that cannot find out how
  the thing works. It caught `GET /api/docs` itself on the first run.

## [0.119.1]

### Fixed
- **The Settings window can be read to the bottom.** It had no scroll area, is
  anchored to the centre and is not resizable, so it simply grew to whatever its
  content needed — and expanding **Endpoints** (100+ lines, and longer after every
  release that adds a route) pushed *both* ends of the window off the display with
  no way to reach either. The body now scrolls, capped to the viewport.

  Reported from use, and confirmed by driving it: open Settings, expand Endpoints,
  scroll to *"Full reference: API.md in the source repo."* A settings window you
  cannot read the bottom of is the same defect as a doc surface nobody updated —
  and the list that broke it is one this project keeps telling itself to keep
  current.

- **Two more windows with the same defect**: **Requirements** and **Backup** both
  hold lists that grow (external tools; backup destinations) and neither scrolled.
  Both now use the same capped scroll area.

- **A tooltip contained 26 literal spaces mid-sentence.** The Desktop-mode hover
  text was written as a plain multi-line string, so the *source indentation* became
  part of the text. Rust only strips that when the line ends with `\`. Found by a
  check that had to run before re-indenting the window body — the re-indent was
  safe precisely because every other literal in that window is backslash-continued.

### Added
- **Four more worked examples in Settings → Examples**, matching the reference:
  acting on a card by its id alone, appending to a shared card, doing a whole list
  of cards across baskets, and archiving a basket's finished cards in one call.

## [0.119.0]

### Added
- **A list of card ids is an address, whatever baskets they are in.**

  ```
  GET    /api/cards?ids=1391,1392,1393
  POST   /api/cards/property   {cards:[ids], key, value}
  DELETE /api/cards/property   {cards:[ids], key}
  ```

  Every whole-document query — `/api/tasks`, `/api/claims`, `/api/query`,
  `/api/search`, `/api/properties`, `/api/tags` — hands back ids from **different
  baskets**, and that is deliberate: an agenda that only covered one basket would be
  useless. But the batch routes validate against one basket, correctly, so *"mark
  these five done"* meant grouping the list by basket first, at one lookup per card,
  to satisfy an argument the caller never had. Reading them was one call each.

  This was left as a documented limitation when the card-addressed writes shipped in
  v0.117.0. It is the last place where the API asked a caller to know something it
  had never been told.

  **A missing id is fatal to the write and merely reported by the read.** The `GET`
  returns `missing`, so you know exactly what you got; a partial *write* is the case
  where you cannot tell how far it got, which is why one bad id refuses the whole
  thing — the rule the rest of the batch surface follows.

  **Whole-document, so a token confined to a basket is refused**, exactly as it is
  for `/api/tasks`, `/api/search`, `/api/kanban` and `/api/query`: a route that names
  no basket cannot be checked against one. A confined token still has
  `/api/nodes/{id}/cards/…` for its own basket. Stated as a cost rather than glossed,
  and pinned by a test.

## [0.118.0]

### Added
- **`POST /api/cards/{cid}/append {text, at?, separator?}`** — add to a card's body
  without sending the body back.

  A card two people write to is where read-modify-write goes wrong: `GET` the body,
  add a line, `PATCH` it back, and whatever the other one typed in between is gone.
  A **message board, a running log, a handoff card** is exactly that card, and it
  is exactly where a human and an agent both write. Append happens on the server,
  in one call, and does not need an 18 KB body shipped in both directions to add a
  line to it.

  `separator` defaults to a **blank line** — the Markdown paragraph break that
  naive concatenation gets wrong by running two paragraphs together — and an empty
  body takes the text with no separator at all. `at: "start"` for newest-first
  boards.

  **Refused where `body` is not what the card shows**, naming the route that does
  work: a checklist's lines and a table's cells are its content, and text appended
  to *their* body is stored, displayed nowhere, and not read as a property either,
  because a checklist card's properties come from its title and items alone. A 200
  that changed nothing anyone can see is the worst answer available. Mirrored cards
  answer 409, as they do for `PATCH`.

- **`POST …/cards/{cid}/items {text, done?, at?}` and `DELETE …/items/{item}`** —
  add or remove **one** checklist line, and get the new line's id back.

  The alternative was rewriting the whole `items` array, which carries the existing
  ids across **by position**. If a line was reordered or removed between the read
  and the write — by the person you are working with, in the app — every id after it
  changes hands. Since v0.90.0 an item id is what `…/items/{item}/done` and
  `…/items/{item}/property` address, and a dated line **is a task**, so a
  positional rewrite quietly reassigns which task is which. One line in, one line
  out, and every other line's id stays where it was. The wholesale rewrite is still
  right for *replacing* a list.

  Both are reachable by bare card id as well, through the same v0.117.0 rewrite.

### Fixed
- **The change log now describes item-level edits.** `…/items/{item}/done`,
  `…/items/{item}/property` and its `DELETE` were absent from `change_of`, so they
  fell through to *"the document changed somehow"* — which is precisely what the
  change log was built to stop being the only available answer. Ticking a line and
  moving a line's date are among the most common writes in the API, and a client
  watching `/api/changes` for an agent's edits could see that something had happened
  and nothing more. Found by reading the log while driving the new routes, not by
  reading the code.

## [0.117.0]

### Added
- **A card id is a complete address for *writing* to a card, not only for finding
  one.** Eight routes take a bare `{cid}`:

  ```
  PATCH  /api/cards/{cid}                          DELETE /api/cards/{cid}
  POST   /api/cards/{cid}/property                 DELETE /api/cards/{cid}/property?key=
  POST   /api/cards/{cid}/move
  POST   /api/cards/{cid}/items/{item}/done
  POST   /api/cards/{cid}/items/{item}/property    DELETE …/items/{item}/property?key=
  ```

  Card ids have been document-unique since the beginning and readable by id since
  v0.87.0 — but every *write* was `/nodes/{id}/cards/{cid}/…`, and a card id is
  what you are always handed: by `/api/search`, `/api/tasks`, `/api/claims`,
  `/api/query`, `/api/properties`, `/api/tags`, backlinks, `/api/changes`, and by
  the `[[#1391]]` links people paste into cards and messages. So the cheapest
  possible edit cost **two round trips** — one to learn the basket — and an agent
  answering "mark 1391 done" ended up quoting a node number the person it was
  working with had never mentioned.

  **These are the same operations, not new ones.** The app loop resolves the id to
  its basket, rewrites the request into its ordinary node-addressed twin, and drops
  it back into the same pipeline — so the scope check, the mirror-policy check, the
  change log and the code that applies the edit are the ones that already existed
  and were already audited. One `resolve_by_card`, a pure function over ids, is the
  only place the pairing is written down, and a test asserts each op lands on the
  right twin. A parallel set of write paths that each had to remember to check a
  token's scope is precisely how a confined token could carry its own card out of
  its basket until v0.111.0, one unchecked end at a time.

  Verified against a running instance with a token confined to one basket: its own
  card by id **200**, a card in another basket **403**, moving its own card *out*
  by id **403** (the v0.111.0 escape, through the new door), reordering inside its
  own basket **200**, and the private card unchanged throughout.

  **A confined token gets 403 for any id it cannot reach, existing or not.**
  Answering 404 for "no such card" and 403 for "someone else's card" would turn
  this into a way to probe the rest of the document one id at a time — the same
  reasoning that already governs `GET /api/cards/{cid}`. With the instance key a
  missing id is an ordinary 404.

  The batch routes stay basket-addressed: a batch is validated against one basket,
  and a list of ids gathered from a whole-document query can span several.

### Changed
- **One `PropertyInput`** instead of two identical inline `{key, value}` structs.
  They were the same request shape, and the card-addressed routes would have been
  a third copy.

## [0.116.0]

### Added
- **The batch surface is finished.** Creating, moving and setting one property on
  a list of cards already existed; editing them, deleting them and taking a
  property back *off* them did not — so the half of a workflow that tidies up was
  still one call per card while the half that made the mess was one call for all
  of them.

  - **`DELETE /api/nodes/{id}/cards/property {cards, key}`** — the missing
    counterpart to the batch set. `cleared` counts the cards that actually carried
    the property and `cards` names them, so *"8 of the 20 had a due date"* is
    legible rather than hidden; a card that never had it is not an error. This one
    matters because setting `due::` to `""` is **not** the same as removing it —
    the empty line stays, unreadable, and the card sits under *No date* forever
    instead of leaving the agenda.
  - **`PATCH /api/nodes/{id}/cards {cards, …}`** — restyle a set: `color`, `size`,
    `fit`, `font_scale`, `z`, `emphasis` and its two companions.
  - **`DELETE /api/nodes/{id}/cards {cards}`** — delete a list, validated in full
    before a single card goes.

  All three follow the rule the batch move set: **the whole list is checked before
  anything changes**, so one bad id refuses the batch and names itself. For the
  delete that is not a nicety — a half-finished delete cannot be undone by
  re-sending the request. There is deliberately no *"everything in this basket"*
  form: the one batch operation you cannot walk back should not be reachable by
  leaving an argument out.

  **The batch edit is presentation only, and refuses content by name.** `title`,
  `body`, `items`, `rows`, `kind`, `lang`, `header`, `source` and `inline_images`
  come back as a 400 that names the field and points at the single-card route.
  Each of those *is* the card: writing one across a list means every card in the
  list ends up saying the same thing — the copied-card failure the task model
  exists to prevent — and one typo'd id list would be an unrecoverable overwrite
  of somebody's work. Content is one card at a time; the batch is for *make these
  look the same*. There is no `pos` for the same reason in reverse: it would stack
  every card on one point, and the batch **move** already has `pos` + `gap` for
  laying out a column.

  The fields it does accept are applied by **the same code the single-card `PATCH`
  runs** — one extracted `apply_presentation`, not a second copy — so the 80×60
  size floor, the depth clamp and the emphasis expiry cannot drift apart. `fit` is
  re-measured with the real fonts per card in the app loop, exactly as it is for
  one card; the compiler caught that being missing, because `fit_updates` sat
  unused until it was wired in.

### Fixed
- **A refused edit no longer half-applies.** `PATCH` on a card that mirrors a file
  answers 409 for a `body` — but that check sat *after* `title` was written, so a
  request that was refused had already renamed the card, and the caller had no way
  to know what stuck. The mirror check now runs before any field is applied.

## [0.115.1]

### Changed
- **The build is warning-free again**, and the two warnings it was carrying were
  each hiding something worth knowing.

  `Plugin.enabled` was **constructed `false` and never read by anything**, while
  its own doc comment said *"a plugin is inert until approved"* — as if that field
  were the gate. The real gate is the `Grant` list, checked through `is_approved`
  on the manual, node-menu, card-menu, schedule and on-change paths. Nothing was
  unguarded, which is exactly why this was worth removing rather than wiring up: a
  duplicate that *looks* like the permission check is something a future reader
  trusts instead of going to find the one that runs. The reasoning moved onto the
  struct, pointing at where approval actually lives.

  `TreeSort::label()` was dead because the sort menu iterates `TreeSort::ALL`,
  which already pairs each variant with its label. `key()`/`from_key()` are still
  used, for settings persistence.

- **`import_journal` no longer emits 20 unfixable warnings.** It `#[path]`-includes
  `model.rs` to get `Document` and `CardKind` and uses a fraction of it, so every
  unused item in the model was reported as dead code *in that binary* — warnings
  that could not be fixed and never meant anything. `#![allow(dead_code)]` is now
  set on that binary alone; the app compiles the same file with warnings on and
  answers for it. A build that always warns teaches you to skim the output, and
  `unconditional_recursion` has shipped a crash here twice.

- **`api::source_request` (singular) is gone.** v0.115.0 moved the app loop's
  mirror-policy check to the plural `source_requests`, and the singular was left
  behind with a doc comment still claiming to be the thing the app loop calls —
  kept alive only by its own tests. Those assertions now go through
  `source_requests`, so the coverage is unchanged and the comment is true again.

- **Three items in `model.rs` are annotated instead of deleted**, because they are
  not actually dead: `Document::empty()` is what `import_journal` builds onto, and
  `card_properties`/`card_property` are how the test suite pins *"a checklist
  card's properties come from its title and items, never its body"*. `model.rs` is
  compiled into two binaries, so an item live in one is reported dead in the other.

- **Two warnings came from our own vendored `egui_commonmark`.** Vendoring dropped
  the `macros` feature and the `egui_commonmark_macros` dependency but left two
  `#[cfg(feature = "macros")]` re-exports behind, pointing at a crate that is not
  in this copy — blocks that could never be enabled, warning on every build. Trellis
  renders through `CommonMarkViewer`, never the compile-time macro.

  No behaviour change on any surface: 320 tests green (236 app + 84
  `import_journal`), no endpoint, document-format or UI difference, and
  `cargo build --release` and `cargo test --release` now both emit **zero
  warnings** — from any crate, so the next real one will be visible.

## [0.115.0]

### Added
- **`POST /api/nodes/{id}/cards` accepts an array** and creates the whole batch,
  returning `{"created":N,"ids":[…]}` with the ids **in the order you sent them**.
  The same endpoint still takes a single object; an array is what switches it.

  This was deferred once as "looks like the batch move but isn't", and that was
  right — creating a card is not a pure document operation. `fit` is re-measured
  with the real fonts **in the app loop**, checklist items are given their ids
  after the fact, and `source` is checked against the mirror policy before the
  request applies. All three had to become plural, and the mirror check
  especially: it is the one place an API request can reach the filesystem, so a
  batch that only had its **first** `source` checked would let the second reach
  anything the policy forbids.

  The creation path itself was **extracted, not copied** — one `add_one`, called
  by both routes, because two copies would drift and the drift would show up as a
  card that behaves differently depending on how it was made.

### Fixed
- **Resizing a desktop-mode window now resizes the card.** The window resized
  fine, but the size was never written back, so recalling the card or restarting
  reverted it — which reads as the resize having been ignored. A desktop window
  *is* the card, so it now updates `card.size`, guarded by a one-pixel epsilon:
  writing back a value that differs by a fraction would change the builder, which
  would command the window, which is the flashing loop from v0.114.1 again with
  size instead of position.

## [0.114.2]

### Fixed
- **`--help` and the README documented an environment variable that no longer
  exists.** `TRELLIS_RESTART_DELAY_MS` was replaced by `TRELLIS_RESTART_WAIT_SECS`
  in v0.114.1 — the restart now waits for the API port to be free rather than for
  a guessed number of milliseconds — and both doc surfaces still named the old
  one. A documented variable that does nothing is the same defect as a runbook
  that says `assembleDebug`.

### Changed
- **API.md gained worked examples** for the endpoints added this cycle: archiving
  a basket with the batch move, addressing and moving a group, and putting a
  basket on the desktop. The reference told you the shape; the examples give a
  copy-paste pattern, which is what an agent actually starts from.

## [0.114.1]

### Fixed
- **File → Restart left the new window with no API, and reading a stale
  document.** Both because it did not wait for the old process to finish.

  `restart()` called the ordinary save, which runs on a **background thread**, and
  then immediately launched the child. So the child opened the file as it stood
  *before* this process finished writing — measured at **19 seconds** before —
  and any edit made in the new window would have written that older copy back
  over the good one. The save is synchronous now: the child is not launched until
  the document on disk is final.

  The child then waited a flat **1.5 seconds** for the port. On a large document
  over a slow volume the old instance held it for **20 seconds**, so the new
  window bound nothing and came up silently API-less — no agents, no plugins, no
  clipper, and nothing obviously wrong to look at. It now **waits for the port to
  actually be free** rather than for a guessed duration, up to a deadline, and
  says so on stderr if it gives up.

- **A history snapshot could be written truncated, and then offered for restore.**
  Snapshots were written straight to their final name, so an interrupted write
  left a short `.gz` with a perfectly good timestamped filename. Observed: a
  **55 KB** entry sitting beside 12.8 MB ones. Snapshots are now written to a
  `.part` file and renamed into place, the way the document itself has always
  been saved, and the history list **skips any snapshot that does not
  decompress** — the moment you need a restore is the worst possible moment to
  find out the file is empty.

## [0.114.0]

### Added
- **Desktop mode is a mode.** The **Desktop** toggle on the canvas, beside Dock
  and Snap, takes **every card in the open basket** onto your desktop as real
  windows at once, and takes them all back. Also in **View**, and over the API as
  `POST`/`DELETE /api/nodes/{id}/desktop`.

  **This is what v0.113.0 should have been.** That release shipped only the
  per-card action and called the feature done, when the request was *"the cards
  of a selected workspace are seen on the screen"* — the workspace, not one card
  at a time. VMware's Unity, which the request named, is a switch you flip; it
  does not ask you to promote each window by hand. Splitting that into phases and
  shipping the half that was not the feature was the wrong call.

  **The arrangement survives the trip.** The basket's bounding box is fitted to
  the screen and every card placed by the same scale, so the layout you built is
  the layout you get. Windows keep their real size — scaling a window would
  shrink its text — so only the spacing compresses.

  **Positions come from the document, not from the drawn screen rects.** The
  first version used the on-screen rectangles, which put every card that was
  scrolled out of view past the edge of the display; the window manager then
  clamped them, flattening the arrangement into a row along the bottom of the
  screen. Measured: horizontal spacing was exact and vertical was crushed to
  0.725 of it, with three windows sharing one bottom edge. Now a single uniform
  scale, verified at 0.595 on both axes with nothing clamped.

  Only one basket is out at a time — two baskets of windows on one desktop is a
  pile with no way to tell which document you are looking at — so turning a
  second one on recalls the first.

## [0.113.1]

### Fixed
- **A desktop card window flashed and jumped**, badly enough to be unusable.
  The builder was re-applying `with_position` every frame from the window's own
  reported position, so the app commanded the window, the window manager moved
  it, the app read the new position and commanded it again.

  **This is the v0.99.1 bug, and the changelog entry that shipped it cited that
  bug by name.** A delta chased against a lagging reading cannot converge.
  Where a window is *created* is now fixed for the life of that window; where it
  currently *is* is observed for persistence only and never fed back into a
  builder.

- **Right-click did nothing on a desktop card.** The card's context menu hangs
  off its title-bar interaction, and the strip added to drag the window covered
  exactly that and won the hit-test — silently removing the menu from a card the
  moment it left the canvas. The strip now carries the menu itself.

- **That menu offered *Send to desktop* on a card that was plainly already out.**
  Suppressing the "on the desktop" placeholder had been done by passing an empty
  set, which also told the menu nothing was out. The two questions are now
  separate: `on_desktop` says *which cards are out* (what the menu needs) and
  `as_window` says *where the drawing is happening* (what suppresses the
  placeholder).

All three were found by driving the running app and looking at it — none is
visible in the source, and the first was reported from the operator's screen.

## [0.113.0]

### Added
- **Desktop mode - a card can leave the canvas and become its own OS window**
  (Linux/X11). Right-click a card, *Send to desktop*, or
  `POST /api/cards/{cid}/desktop`. It is the same card: edit it in the window and
  the basket has the change, because both draw through the one `card_ui`.

  **One real window per card, not a transparent overlay.** The overlay is the
  cheap route and it is the wrong one: a single window sits entirely above or
  entirely below every other application, so a card could never be behind a
  browser and in front of a terminal - which is the whole premise. Only genuine
  top-level windows take part in the window manager's stacking.

  **Not always-on-top**, for the same reason. A card that can never go behind
  anything is a HUD, not part of the desktop.

  **Placement is app config, not document state.** A screen coordinate belongs to
  one machine; a document opened on another box, or read by the Android app, must
  not carry window geometry. Same rule that keeps templates and the backup
  schedule out of the `.ron`.

  Dragging hands the move to the **window manager** (`StartDrag`) rather than
  chasing the pointer delta - a delta measured inside a window that is itself
  moving cannot converge, which is exactly how a stuck panel walked off the
  screen in v0.99.1.

  A card that is out is drawn faintly in its basket, labelled *on the desktop*, so
  the layout does not develop a hole where a card used to be.

  **Measured before it was built**, on this machine, as a throwaway binary: a
  transparent undecorated viewport composites under `kwin_x11` with the glow
  backend; `StartDrag` moves an undecorated window; and 50 such windows held a
  steady 59.9 FPS - so the design needed neither a cap nor an opaque fallback.

  **Linux/X11 only.** Wayland has no protocol for an application to position its
  own windows at all; macOS and Windows need their own pass and answer **501**.

## [0.112.0]

### Added
- **`POST /api/nodes/{id}/cards/move {cards, node, pos?, gap?}` — move a batch of
  cards to another basket in one call.** Archiving the Trellis workspace was **55
  single-card calls**, one per card, which is what made the gap obvious: the
  operation an agent actually performs is *"these cards, over there"*, and the API
  only offered *"this card, over there"*, repeated.

  **The whole list is validated before anything moves.** One bad id refuses the
  batch rather than moving what it can — a partial move leaves the caller unable
  to tell how far it got, which is the same reasoning behind batched table ops in
  v0.82.0. `pos` places the first card and stacks the rest below it by height plus
  `gap`, so an archive reads as a column instead of a pile; omit it to keep the
  coordinates each card already had.

- **`POST /api/nodes/{id}/cards/property {cards, key, value}`** — one `key:: value`
  on many cards. Marking a batch `status:: done` was the same one-call-per-card
  problem.

Both are scope-checked at **both** ends for a confined token, like every other
move since v0.111.0.

## [0.111.0]

### Added
- **A group can be linked to, and moved.** `[[#g146]]` names a group the way
  `[[#1391]]` names a card. Following one centres the canvas on the container
  and flashes it.

  **The gap was specific.** A group already had an id — from the same kind of
  document-wide counter that gives cards theirs — and nothing could read it.
  There was no link form, no `GET` that took one, no Ctrl+O row, and the id
  appeared nowhere in the app. So the only way to point anyone at a group was to
  name a card **inside** it, which says *somewhere near here* rather than naming
  the thing. Reported exactly that way: *"I can link cards and such but I cant
  link a group."*

  The `g` is what keeps the two id spaces apart — card ids and group ids come
  from different counters, so the same number can name both. Nothing written
  before this changes meaning: `g146` never parsed as a card id.

- **`POST /api/nodes/{id}/groups/{gid}/move {node, pos?}` — move a whole group to
  another basket**, container, members, title, colour and **id** together.

  **Moving the cards was not a workaround, it was a different operation.** Group
  membership is basket-local, so `cards/{cid}/move {node}` drops it by design:
  every card arrives ungrouped and the container has to be rebuilt by hand —
  with a **new id**, which breaks every `[[#g…]]` link already written to it.
  `pos` places the group's top-left corner and moves every member by the same
  delta, so the arrangement inside survives.

- **`GET /api/groups/{gid}`**, **`…/backlinks`** and **`…/link`** — find a group
  from its id alone, see what points at it, and mint its address. `link` returns
  the `[[#g146]]` form to paste into a card alongside the `trellis://` form for
  leaving the app.

- **`trellis://127.0.0.1:<port>/group/<gid>`**, and **Ctrl+O accepts `g146`**.

- **Copy → *Group link*** on a group's header, and **Copy → *Card link*** on a
  card. Both hand you the `[[#…]]` form. The app could only ever copy a bare
  number, so the README's advice was to type the brackets yourself.

### Fixed
- **A scoped token's move was only checked at the end it started from.**
  A subtree-confined token is checked against the node a request *names*, and for
  a move that is where the thing is coming **from**. The destination went
  unchecked, so a token confined to one basket could move its own card — or
  reparent its own basket — **out into the rest of the document**. A write
  outside the scope, reached by relocating something inside it.

  Both ends are checked now, for cards, groups and nodes alike. `before`/`after`
  are resolved through the sibling's parent, and the top level counts as outside
  every subtree. Found while adding the group move, which would have inherited
  the same hole.

## [0.110.2]

### Fixed
- **A link's port survives the desktop's URL parser.** `trellis://7374/card/9`
  put the port where a URL keeps its **host**, and a bare integer is a legal IPv4
  address — so KDE normalised `7374` to **`0.0.28.206`** (`0x00001CCE` as a
  dotted quad) and the link failed on arrival with *"port and id must be
  numbers"*. Links are now minted as
  **`trellis://127.0.0.1:<port>/card/<id>`**, where nothing is left to rewrite.

  **Every link already written still works.** A bare port is still accepted, and
  so is the dotted quad a normaliser produces — it is unpacked back into the port
  it came from. A link in a card or a session report is a durable thing and must
  not stop opening because the format improved. Only loopback is accepted as a
  host: a clicked link must never reach another machine.

## [0.110.1]

### Fixed
- **A `trellis://` link clicked out of a sentence works.** Links are read out of
  prose, not address bars, and a terminal hands over whatever its URL detector
  decided the link was — including the full stop that ended the sentence, the
  comma, the closing bracket, the em dash, or the `<…>` delimiters someone
  wrapped it in. Those are now trimmed before parsing.

  **Two different failures, neither of them visible.** A trailing `.` or `,` rode
  into `?doc=`, so the receiving instance compared `Personal.ron.` against
  `Personal.ron` and refused with a **409 nobody ever saw**. And `<trellis://…>`
  did not match the scheme at all, so it fell through to the argument parser, was
  taken for a **file name**, and opened a second instance on another document —
  a window flashing up and vanishing while the link went nowhere.

- **Anything that looks like one of our links is handled as a link**, never as a
  file. A malformed link is now an error, not a new window.

- **A failed link says so where it can be seen.** The desktop file launches the
  handler with no terminal attached, so every `eprintln!` went to the void and a
  refused link was indistinguishable from a working one. Failures now raise a
  desktop notification naming the reason.

## [0.110.0]

### Added
- **`verify::` — a card can say when what it claims should be re-checked**, and
  **View → Claims** / **`GET /api/claims`** list the ones that are out of date.

  **The problem is specific.** A card that states how something *is* — "both
  instances run v0.109.0", "the operator still owes a bot token" — was true when
  it was written. Nothing in a document distinguishes a fact from a fact *as of a
  date*, so it is read in the same voice a year later. That has cost real
  sessions: work redone against a stale runbook, and the operator asked for
  something already delivered.

  A card that asserts state now carries `verify:: 2026-09-01` (when to re-check)
  and optionally `check:: GET /api/instance` (what settles it). Both are ordinary
  `key:: value` properties, so nothing had to migrate and the dates are visible
  in the card rather than hidden in metadata.

  **`stale_claims` rides on `GET /api/instance`** — the one call every agent
  already makes first — so a workspace that has gone out of date says so *before*
  it is read rather than after it is believed. `GET /api/claims` returns every
  claim worst-first with a `bucket`, `?expired=true` narrows it to the ones not
  to be trusted, and `?project=<id>` scopes it like `/api/tasks`. The View menu
  carries the count, because a currency panel nobody opens is no better than the
  stale card it was built to catch.

  **Deliberately not `due::`.** A task is finished once and leaves the agenda; a
  claim is never finished, it only goes out of date — modelling one as the other
  would fill the agenda with rows that can never be completed. **And deliberately
  not `touched`**: that moves when a card is *edited*, so fixing a typo in a
  stale card would make it look freshly confirmed. Editing and confirming are
  different acts and only one is evidence. An unreadable `verify::` counts as
  stale rather than fresh, since a claim whose expiry cannot be parsed has no
  expiry at all.

### Fixed
- **Two more tests were not running.** `extract_properties_parses_fields_not_code`
  and `card_export_round_trips_inline_images` had lost their `#[test]`
  attributes, exactly as two others had in v0.103.3 — they compile, they read as
  tests, and they never ran. Both pass. The suite reports **302** where it ran
  295.

## [0.109.0]

### Added
- **`GET` / `POST /api/settings`** — the app's own settings over the API, so an
  agent can set a machine up (or put one back) without anybody clicking.
  Twenty-three of them: theme, tree sort, the canvas toggles, both hypercube
  axes, which panels are open and what they are scoped to, notifications, and
  history retention.

  **This closes a gap that had opened quietly.** The rule is that everything a
  person can do, an agent can do — and settings had never been part of it. The
  theme has been unreachable since it shipped, and this session added three more
  (two notification toggles and the project sort) without noticing.

  **What is deliberately *not* settable:** the API key, the port, the LAN flag
  and the file-mirroring policy. A caller must not be able to widen its own
  reach — an agent that could switch on LAN access, or point the mirror policy at
  `/`, would be escalating with a credential given to it for notes. Those stay in
  Tools → Settings, and the error says so by name.

  A patch is validated in full before any of it is applied, an unknown name is a
  400 listing what was expected, a known name with the wrong type is refused
  rather than coerced, and an empty object is refused because it is not a change.
  The response is every setting as it now stands, not an echo of what was sent.

## [0.108.0]

### Added
- **View → Sort projects** — the top-level projects by name (A→Z, Z→A), by
  **recently changed**, or by **open tasks**; **Manual** is the document's own
  order. **Remembered across restarts**, because a sort you re-pick every launch
  is a chore rather than an ordering.

  **A view, not a rewrite, and that is the feature.** The reason this was wanted
  is that a new project lands at the bottom and has to be dragged into place — so
  a one-shot sort would fix it once and let it rot again on the very next
  project. As a view it keeps fixing it: a project added while the sort is on
  appears where it belongs, and the document's stored order is left alone for
  everything else that reads the file.

  **Roots only.** Sub-nodes keep the order they were given: inside a project the
  order is usually meaning — phases, months of a journal — and sorting that
  destroys information rather than tidying it.

  Two smaller rules, each with a reason. Names sort **case-insensitively**,
  because a byte sort files every lowercase title after every uppercase one and
  clumped the domain-named projects at the end of the real document. And
  reordering a project by hand while a sort is on **turns the sort off** — the
  move is what was asked for, so the sort steps aside instead of silently putting
  it back.

## [0.107.0]

### Added
- **Shift+drag a box to select several cards**, then drag any one of them to move
  the whole set; **Esc** clears it. Asked for in those words, and the last
  unbuilt item on *Desired features*.

  **Shift rather than a mode**, because a mode is a thing to remember to leave,
  and plain drag has meant *pan* for the entire life of the canvas — so it still
  does. **A box takes what it touches**, not only what it fully encloses: a
  selection box is drawn roughly, and the card clipped by three pixels is exactly
  the one you meant. A drag of a few pixels is treated as a click that wobbled,
  so it cannot silently clear a selection you already had. Ctrl+click still adds
  or removes one card at a time, and a marquee **replaces** the selection rather
  than adding to it — it is a statement about what you want selected.

  **What could not be verified by driving it, and why that is recorded rather
  than glossed:** synthetic pointer input does not hold `primary_down` across
  frames, so egui never sees a synthetic drag at all and the interaction cannot
  be exercised from a script. The geometry — which cards a box takes, including
  the grazing case and the empty-space case — is extracted into `cards_in` and
  covered by a test; the drag gesture itself wants a human hand.

## [0.106.0]

### Added
- **Desktop notifications**, in Settings → Canvas → Notifications and off by
  default: what is **overdue or due today** when the document opens, and a nudge
  when an **agent changes something** while you are in another window.

  **Why this exists next to the Telegram plugin rather than instead of it.** The
  distinction is the operator's and it is the right one: a desktop notification
  is **dismissible**, a message is not. Swipe one away and it is gone; a Telegram
  message sits in a list until it is dealt with. So the two answer different
  questions — *this just happened, look now* versus *this is outstanding and will
  still be outstanding later* — and neither replaces the other.

  Three rules keep it worth leaving on. **Nothing while the window has focus**:
  an agent's edit is already on the canvas, and announcing what you can see is
  how a notifier teaches you to ignore it. **An empty digest is not sent** — a
  notifier that reports "nothing is due" is one you learn to skip. **One
  notification per batch**, because an agent writing a basket makes twenty change
  entries and twenty popups is an attack.

  Linux goes through `notify-send`, which joins the optional-tools list
  (Tools → Requirements…) rather than becoming a hard dependency; macOS uses
  `osascript` and Windows the built-in toast API. The honest limit is stated in
  the Settings panel: **nothing fires while Trellis is closed**, because a
  desktop app is not a service.

## [0.105.0]

### Added
- **Emphasis: a card can ask to be looked at.** Right-click → **Emphasis** →
  *Glow* (a steady accent halo) or *Pulse* (the same halo breathing over 1.8s),
  and `emphasis` / `emphasis_intensity` / `emphasis_minutes` on card create and
  PATCH.

  **Three decisions worth recording, because they are what makes it usable
  rather than a novelty.**

  - **It is a separate channel from `color`.** The accent is how a *person*
    organises a basket, so an agent borrowing it to shout destroys the
    organisation it is shouting about.
  - **There is no flash, and there will not be one.** Anything above about 3 Hz
    is a photosensitive-seizure risk — the one visual effect here that can
    actually hurt somebody. `Pulse` is a slow sine that never dims below 40%: it
    reads as alive rather than as an alarm.
  - **It expires.** `emphasis_minutes` sets a lapse time, and agents are told to
    always send it: emphasis that never expires accumulates until every card is
    shouting and none of them mean anything. Expiry is evaluated **at draw
    time**, so a lapsed highlight costs no edit, no `touched` and no change-log
    entry — a card that stopped being urgent is not a card that was modified.

  One field rather than three flags (`flash`/`pulse`/`glow` would be eight
  states, most meaningless, and every renderer would have to answer for all of
  them). An unknown value is a **400 naming it**, like the rest of the API's
  input since v0.86.0. A pulsing card asks for a repaint **only while it is on
  screen**, because egui redraws on request and an unconditional timer would burn
  a core on an idle window for ever.

## [0.104.0]

### Added
- **Three themes drawn from instruments rather than interfaces.** Each paints the
  card itself, because a theme that only recolours the chrome is a palette, not a
  look.
  - **Blueprint** — cyan linework on Prussian blue. A card is a drawing sheet:
    square corners, a thin rule, a **title block** (the double rule under the
    heading is the whole convention, so it is drawn rather than implied) and
    registration ticks at each corner. The canvas was already a board you arrange
    things on, so the metaphor is not borrowed — it is what the app is.
  - **Silkscreen** — white legend and gold pads on solder-mask green, each card a
    part with a **pin-1 dot**. The title indents past the pad rather than sitting
    on top of it, which is what a real legend does and what the first attempt
    got wrong on screen.
  - **Phosphor** — a storage oscilloscope: P31 blue-green on a graticule, no
    fills, a bright beam rule under each title, and the accent glow the neon
    themes use. Deliberately not another Terminal Green: that is a console, this
    is an instrument.

  All three are `Theme` variants with their own `CardStyle`, so they cost the
  same as the six that came before and nothing else changed shape.

## [0.103.5]

### Fixed
- **The X11 selection could switch itself off for the rest of the session.**
  `set_primary_selection` took its owner mutex with `let Ok(..) else { return }`,
  so a single panic anywhere on that thread left the lock poisoned and every
  later selection returned at that line. Copying by selection would simply stop
  working, with nothing logged and nothing visible — which is exactly what "my
  clipboard broke" looks like. A poisoned lock is now recovered: what it guards
  is one process handle, and the worst a panic can leave is a handle to a process
  that has already exited.
- **A zombie helper per selection, sitting under Trellis until the next one.**
  `xclip` and `xsel` **daemonize by forking**: the process Trellis spawns writes
  the selection, forks, and exits, and the fork — re-parented to init — is what
  actually serves PRIMARY. So the handle Trellis kept was normally a corpse, and
  it was only reaped when the *next* selection came along. Measured on a live
  instance: a defunct `xclip` parented by Trellis, 1h58m old. The child is now
  reaped where it is spawned, and the handle kept only when the helper really did
  stay in the foreground.

  **This is not the v0.85.1 bug returning, and the investigation is the reason
  to say so.** Giving PRIMARY to a new owner sends the old one `SelectionClear`
  and `xclip` exits by itself, so one live helper survives a whole session's
  dragging rather than one per character. The leak is still fixed; what was left
  was corpses, which hold no selection and break nothing on their own.

## [0.103.4]

### Changed
- **Folding the whole tree moved from the tree header into the View menu.**
  Reported from use: the `⊟` / `⊞` buttons sat directly beneath the menu bar, in
  the path of a pointer heading for **Edit** or **View**, and were being clicked
  by accident. That is an expensive misclick — it re-folds every node in the
  document at once — and it happened while reaching for something else, which is
  the worst kind. They are now **View → Collapse the whole tree** and **Expand
  the whole tree**, above *Hypercube*.

  Nothing else changed: the same `TreeAction`, the same one change-log entry for
  the whole tree, and `POST /api/expand {expanded}` is untouched. The tree header
  keeps **Reorder** and **+**, which are the two it had before v0.97.0.

## [0.103.3]

### Changed
- **Examples and fixtures are written from shapes, not from anyone's notes.**
  v0.103.2 replaced six test strings that had been copied out of a live document
  and stopped there: it never looked outside the two test functions it was
  editing, so a fixture further down the same file, the worked examples in
  API.md and a quoted line in this changelog were left as they were. All of them
  now use invented names and invented work — host names, project names, basket
  paths, task lines. Nothing about the tests changed; they pin the same shapes.

### Fixed
- **Two dead `#[test]` attributes.** A stray `#[test]` sat above the doc comment
  of a test that already had one, which rustc reports as `duplicated attribute`.
  That is the same warning that once hid a test which had not run since v0.71.1.

## [0.103.2]

### Changed
- **Test fixtures no longer quote real notes.** The v0.96.0 property tests were
  written from lines lifted verbatim out of the live documents — which is what
  made them good tests, and what made them the wrong thing to commit: this repo
  is public and those documents are not. Replaced with neutral lines of the same
  *shape*, which is all the tests were ever pinning (a `due::` after one space at
  the end of a sentence, the two-space form the app writes, a free-text value
  with spaces in it, and a property quoted inside backticks).
- **Docs: worked examples for the calls added this week** —
  `GET`/`POST /api/nodes/{id}/overlaps` and `POST /api/expand` now appear in
  API.md's *Examples*, not just its reference, and the table-op example states
  that arguments are required and unknown fields are refused.

## [0.103.1]

### Changed
- **A `trellis://` link also asks for attention, not just focus.** Following a
  link raises the window, and a raise can be *refused* — focus-stealing
  prevention (KWin's is on by default) ignores one from an app the user was not
  just interacting with, which is exactly this case, since the click was in a
  terminal. The window would jump to the card silently and the link would look
  like it had done nothing. `RequestUserAttention` is the sanctioned way to say
  so: whatever the policy, the taskbar entry lights up, and window managers
  clear it the moment the window is focused.

  **Insurance, not a diagnosis.** This was written for a report of a clicked
  link that fired and changed nothing — but measuring it here showed the raise
  *succeeding* (a minimized instance un-minimized and took focus), so refusal
  was not that cause. It is still the right thing to send for a navigation the
  user triggered from another application.

## [0.103.0]

### Fixed
- **A `[[wiki-link]]` in a card title is a link.** Spotted in the work document:
  an agent titled a figure *"Dry at a glance … (source: [[#10239]])"* and titled
  its script *"Source — figure [[#10238]]"* — the pattern the diagram recipe
  teaches, tying a picture to the code that drew it. Both rendered as their own
  brackets.

  What made it a defect rather than a missing feature: **the link was already
  real.** The backlink index reads titles, so `GET /api/cards/10238/backlinks`
  returned that hit — the card was linked, and the one place you could not
  follow the link was the card itself.

  Fixed the way table cells were in v0.94.0: a title is laid out as runs, link
  runs get the link colour and an underline, and a click that lands on one
  follows it. Only a click *on the link text* does — a drag from anywhere on the
  title bar still moves the card.

## [0.102.0]

### Fixed
- **Table ops are strict, like every other endpoint.** Reported by an agent
  working through the API: unknown *ops* were a 400, but an unknown *field* on a
  valid op passed silently — so `set_cell` with a misspelt `text` wrote an empty
  string over the cell and returned **200**. The v0.86.0 rule (*an unknown field
  is a 400 naming it*) was applied to every input struct except this one, and
  the docs have been claiming it ever since.

  Looking at it found the larger half: **every** argument was
  `unwrap_or(default)`, so an omission silently edited something else. No `text`
  blanked a cell. No `row`/`col` wrote over `0,0` — usually the header. No `at`
  made `remove_row` **delete the first row**. None of those defaults were ever
  documented; API.md has always listed these fields as the op's arguments. They
  are required now, and the 400 names the op and the field.

  A batch is validated **in full before anything is applied**, so a malformed op
  leaves the table untouched instead of half-edited. `autofit_cols`'s `col` stays
  optional (absent = every column) and an absent or null `color` still clears.
  `SketchOpInput` had the same gap and is closed with it.

  Minor bump, not a patch: a caller relying on an undocumented default will now
  get a 400 — which is the point.

## [0.101.1]

### Fixed
- **File → Restart works after an upgrade — which is the only time it is
  used.** Reported: *"restart only works for the same version of trellis"*, and
  it was exactly right. Restart relaunched `current_exe()`, which on Linux reads
  `/proc/self/exe` — a link to the **inode** the process is running, not to the
  path. Installing a new build unlinks that inode, so the link reads
  `…/trellis (deleted)` and the relaunch failed with *No such file or
  directory*. The one case the feature exists for was the one case it could not
  do.

  The marker is stripped and the path used only if a file is there now — that
  file *is* the new build — with `argv[0]` as the fallback, since the desktop
  entry and both launch scripts pass an absolute path. A bare `argv[0]` (a PATH
  lookup rather than a file) is never spawned blind.

  Verified by doing it: replaced the binary under a running instance, hit
  Restart, and the new process came up on the same port with its `/proc/self/exe`
  pointing at the replacement rather than the deleted inode.

## [0.101.0]

### Added
- **An edited template master says so.** A master card that no longer matches
  its stored snapshot is marked **✎ edited** on its title bar, and its
  right-click menu offers *Update template “name” from this card* — its own
  slot, rather than a list of every template to pick out of.

  Reported from use, and the report is the whole design note: a template was
  created from an old card, the values were deleted from the **master** in the
  Templates basket, and inserting it *still produced the original values*.
  Nothing was broken — the stored snapshot is the authority and inserting stamps
  it, exactly as intended, so that a stray edit cannot silently change every
  future insert. But nothing said the two had diverged, so an edited master
  looked authoritative and wasn't: the insert kept producing old content, with
  no error and nothing to notice. The rule stays; it is now visible.

  Size and depth are ignored when comparing — dragging a master, or letting Fit
  resize it, is layout rather than a change to the template. Image bytes are
  ignored too, so replacing a picture with a different one under the same name
  is the single edit this will not spot; comparing megabytes of base64 every
  frame to learn what the file name already says is not worth it. The check runs
  only for masters in the basket being drawn, so it costs nothing outside the
  Templates basket.

## [0.100.0]

### Added
- **Fix overlapping cards** — right-click a basket, or
  `POST /api/nodes/{id}/overlaps`; `GET` the same path asks *which* cards cover
  each other without changing anything.

  This closes the longest-standing known hazard. **`fit: true` sizes a card to
  its content — width as well as height** — so a card grown by an edit can end
  up sitting on its neighbour with nothing to say so, and the only repair was
  Autosort, which throws the whole arrangement away. That made it useless on
  exactly the baskets that matter: the ones someone arranged on purpose.

  **It keeps the layout.** Every card's `x` is preserved, so columns survive;
  cards move *down* only far enough to clear, in the order they already sat in;
  a basket with nothing overlapping is not touched. Cards that travel together —
  a group, a dock stack — are treated as one block, so the check does not cry
  wolf on docking and the repair cannot pull a stack apart.

  Measured against the real personal document: **312 baskets, 177 overlapping
  pairs across 53 of them.** The two worst went to zero with every column
  byte-identical, and running it again moved nothing.

## [0.99.2]

### Fixed
- **Sticking now actually tracks — measured, on a real desktop.** v0.99.1 fixed
  the runaway but two guards were wrong, and only running it showed that.

  **The monitor clamp did the opposite of its job.** `monitor_size` describes one
  monitor while window positions are in whole-desktop coordinates, so on a
  multi-monitor desk it pinned the panel near the origin: the main window moved
  200px and the panel moved 117, or none at all. egui exposes no origin for a
  monitor, so the guard cannot be written correctly and it is gone. A wrong
  guard is worse than none — the runaway it insured against is fixed at its
  cause.

  **The settle guard counted frames.** egui only repaints when something
  happens, so an idle app draws almost none and a counter set to 8 was still
  armed minutes later, eating exactly the event it existed to let through:
  dragging the panel yourself was ignored, and the next move of the main window
  yanked it back to where it used to be. It is wall-clock now (400 ms), and the
  guard that does the real work is simply asking whether the panel is sitting
  where we last put it.

  Verified by driving both windows: the panel matches the main window's movement
  exactly, keeps a new offset after you drag it, still tracks after an idle, and
  does not move at all with 📌 off.

## [0.99.1]

### Fixed
- **A stuck window no longer walks off the screen.** v0.99.0 nudged a detached
  panel by the main window's frame-to-frame delta, measured from the panel's own
  reported position. Both readings lag the window manager, and `OuterPosition`
  is answered with a position that differs from the one asked for by the
  window's decoration inset — so every move left a residue, the next move added
  to it, and the Agenda drifted sideways until it was off the desktop.
  **Chasing a moving target with a measurement of where you already are cannot
  converge.**

  It now holds a *target* instead: the target moves with the main window, and
  the command is sent **once per target**, so a stale or offset reading can
  never feed back into it. Dragging the panel yourself re-teaches the offset,
  guarded by a short settle window so the panel moving *because Trellis moved
  it* is not mistaken for you moving it. As a backstop the target is clamped to
  the monitor: a panel that cannot be seen cannot be used.

## [0.99.0]

### Added
- **A detached Agenda or Kanban sticks to the main window.** Move Trellis and
  its detached panels come with it, keeping the offset you put them at — 📌 in
  the panel header, on by default, remembered next launch.

  **Relative, not anchored.** The panel moves by the same delta the main window
  moved, so dragging the app across the desk brings its board along, while a
  board parked on a second monitor stays where you parked it instead of being
  yanked into a fixed slot. Nothing is sent on a frame where the main window did
  not move, so dragging the detached window itself is never fought over.

  One switch covers both panels: someone who wants their windows to travel with
  the app wants it for the Agenda and the board alike.

  Positioning a window from inside the app is an X11/Windows/macOS capability —
  under Wayland the compositor owns window placement, so the toggle is there but
  the move is ignored.

## [0.98.0]

### Fixed
- **HTML in a text card renders instead of vanishing.** CommonMark passes a raw
  HTML block straight through and the card renderer draws no HTML, so a table
  pasted from a page — or anything the web clipper could not translate — was
  **dropped on the floor**: not shown, not an error, just gone. Block HTML is now
  converted to Markdown for display.

  **Converted, not implemented.** `html2md` was already a dependency (File →
  Import HTML uses it), and going through Markdown means headings, lists, tables,
  links and emphasis all arrive already supported. Rendering an HTML *subset*
  instead would mean choosing which tags to honour and re-answering that question
  every time someone pastes a new one.

  **Inline HTML is deliberately untouched**, because `<span style="color:…">` is
  how a card's text colour is stored — converting it would throw the colour away.
  A test pins that.

  The body on disk is never rewritten; this is a view. The PDF/PNG text layer
  converts too, so the searchable text no longer says `<table>` beside a page
  showing a table.

## [0.97.0]

### Added
- **Fold the whole tree in one click.** `⊟` and `⊞` in the tree header, and
  `POST /api/expand {expanded}` for parity. Right-click → *Expand all* /
  *Collapse all* has always worked on one node, including a root; what was
  missing was doing it to every root at once, which with 38 top-level projects
  was 38 right-clicks.

  **Recursive**, matching the per-node menu and the Android toolbar (which has
  had this since v0.10.0 — this is desktop parity, not new ground). The
  alternative, folding the roots and leaving each subtree's inner shape alone,
  makes the tree remember a state you cannot see: reopening a project gives you
  a shape you never chose.

  `expanded` is document state, so this marks the document dirty and writes a
  version-history snapshot — already true of the per-node toggle. It is one
  change-log entry for the tree, not one per node.

## [0.96.0]

### Fixed
- **Writing about a property no longer creates one.** A card that *documents*
  the syntax — a session report, a handoff, a release note — was acquiring the
  properties it described. Four cards in the personal document were sitting on
  the Agenda because of it, including the **Message board** itself and one whose
  due date parsed as `` 2026-08-08`) ``. A `key:: value` inside an inline
  `` `code span` `` or a fenced block is now read as what it is: text about a
  property.

  This also settles the one that could not be fixed by editing. A **mirrored**
  session report discusses `due::` in its prose, and `PATCH` correctly refuses to
  touch it (409) because the file on disk owns the body — so the only place the
  fix could live was the parser, which is where it now is.

  **Measured before it was written**, across both live documents: **801 real
  properties, 13 false ones — and every false one was inside backticks or a
  fence.** That measurement rejected the rule this started as. *"A property must
  be on its own line"* is wrong: a checklist item carries its `due::` at the end
  of the sentence it belongs to, so the rule would have silently dropped two live
  deadlines. A **`code` card** is left alone for the same reason — its whole body
  is code, and one in the work document is legitimately tracked with
  `status:: done`.

## [0.95.1]

### Fixed
- **`--help` describes the whole binary again.** It has accepted a
  `trellis://…` link as its first argument since v0.93.0 — that is what the
  registered URL handler runs — and honoured `TRELLIS_EMOJI_FONT` and
  `TRELLIS_RESTART_DELAY_MS`, none of which it mentioned. `--help` is a
  documentation surface, and a stale one is the same defect as a stale runbook:
  the Android release runbook said `assembleDebug` long after that was wrong,
  which is how six releases went out debug-signed.

## [0.95.0]

### Changed
- **Depth and Time are named: together they make a basket a *hypercube*.** They
  are grouped under that word on the canvas and in a new **View → Hypercube**
  submenu, where one click turns both on or both off.

  The buttons keep their own plain names, because a label should say what it
  does — someone looking for "make cards overlap in 3-D" scans for *Depth*, not
  for a numbered mode. The group exists to give the pair a reason to be a pair.

  **The group's state is derived, not stored:** it is lit only when both are on.
  A third setting that could disagree with the two real ones would be a bug
  waiting to happen.

  The model, stated in the docs so it stops being ambiguous: a **basket** is the
  space — `x`, `y`, then `z`, then time. The **tree is not a dimension**, it is
  the index over baskets. And a basket is not "a trellis": the trellis is the
  whole lattice, tree and baskets. The app is still **Trellis**; a hypercube is
  what one of its baskets becomes.

## [0.94.0]

### Fixed
- **A date property stops at the date.** In `due:: 2026-08-15 — <prose>` the
  value ran to the end of the line. That
  failed **twice**: the value did not parse as a date, so a task with a deadline
  was filed under *No date* and its owner could not see it was due; and the
  Agenda then drew a 300-character string where a date goes, which stretched the
  panel across the entire window. `due`, `start` and `date` now take their first
  token — a date has no spaces. Other properties keep theirs (`status:: in
  progress` still works).
- **The Agenda cannot be stretched by its contents.** The panel has a maximum
  width and the date is elided like the title. The parser no longer produces a
  runaway value; this makes one unable to matter again.
- **`[[wiki-links]] in table cells are links.** A cell is painted as a single
  run of text with no Markdown involved, so an evidence column full of `[[#10215]]`
  rendered as its own brackets. Link runs are now drawn in the link colour,
  underlined, and clicking one follows it — resolved exactly as a link in a text
  card is.

### Added
- **The Agenda and the Kanban remember themselves.** Whether each is open, each
  *Show completed* toggle, and where each lives all persist. An Agenda that closes
  itself every launch is one you forget exists.
- **Both can be detached into a real window.** *Detach* / *Dock* in each header.
  A window inside the app window cannot be moved to a second monitor or left
  beside the canvas, which is most of what a board is for. The choice is
  remembered, and closing the detached window closes the panel.
- **Alt+drag looks around a depth arrangement.** Moving the eye is what makes
  depth legible on a flat screen: cards at `z = 0` stay put while near and far
  ones swing in opposite directions. *Reset view* returns to straight on, and the
  angle is per basket, like pan and zoom. It is also the first half of the camera
  **pose** that VR needs.

## [0.93.0]

### Added
- **Links that open Trellis on a card.** `trellis://7374/card/1391` — click it in
  a browser, a terminal or a chat window and the instance serving that port comes
  forward with the card revealed. `GET /api/cards/{cid}/link` mints one, so an
  agent never assembles it by hand.

  **The port is the address**, because one instance serves one document — which is
  also what makes it work with several instances running, the normal case here.
  `?doc=Personal.ron` is optional and is a *check*: the instance refuses with 409
  if it is serving something else. That matters because **card ids are unique
  within a document, not across documents** — `1365` is a real card in both of
  this operator's documents, so a link to the wrong port would otherwise land on a
  real card that is not the one meant.

  `/open/...` is **unauthenticated and navigation-only** — it focuses the window
  and answers `{"opened":…}`, never document content, because a keyless route that
  could read cards by walking ids would be a hole. It sits outside `/api`.

  **`http://127.0.0.1:<port>/open/card/<cid>` works with nothing installed.** The
  `trellis://` form needs the desktop to know the scheme, so Trellis registers
  itself on a new install and again if the binary moves, with *Settings → Agent
  API → Register now* and *Tools → Register trellis:// links…* as explicit
  controls. It **will not overwrite a working registration**, so a development
  build cannot hijack the handler from the installed one.

  The scheme name is one constant, and the follower accepts **`hypercube://`**
  as well — a link pasted into a note outlives a rename.

- **File → Restart** — save and start this same instance again: same document,
  same port, same data directory. The new process waits before binding, because
  the old one still holds the API port for the moment it takes to exit, and a
  failed bind is not fatal: it starts *without* an API, which looks healthy and
  answers nothing.

## [0.92.0]

Two axes the canvas never had, both **off by default**, both **view** settings.

### Added
- **Depth — a basket is a volume (`z`).** Cards get a real depth instead of a
  stacking order: nearer ones are larger and cover further ones, a click lands on
  the **nearest**, and **Shift+scroll over a card slides it** toward or away from
  you. Toggle it with **Depth**, beside Dock and Snap.

  It is a **camera**, not scale-and-dim: each card is projected through a pinhole
  at a fixed distance, which is what a billboarded quad does in a 3-D scene. That
  matters because the next step is VR as a **second renderer** of the same scene —
  a 2.5-D effect would have to be thrown away to get there. Cards stay parallel to
  the view plane, so each is laid out at its **effective** size and its text is
  rasterized for the size it is drawn at rather than stretched.

  **Turning Depth off cannot cost you anything.** `z` stays on the card and simply
  becomes the stacking order, so the coordinate is never meaningless and an
  arrangement is never discarded. A flat document's file is unchanged — `z` is
  omitted entirely when it is zero.

- **Time — a task is present on every day it spans.** With **Time** on, a journal
  day also shows cards from other days whose `start::`→`due::` span contains it:
  the **same card**, not a copy, drawn as a projection that names where it lives
  and takes you there when clicked. Nothing new to author — it is the `start::`
  span from v0.90.0, read as extent.

  This replaces copying a card forward, which made a *second* task with its own
  `status::` and `due::`, counted twice, with nothing warning you.

  Two limits, both found by running it against a real document rather than
  reasoning about it: it uses **containment**, not the agenda's rule that a missed
  deadline stays live forever (which filled a day with every overdue task in the
  document); and it projects **only cards that live in other days**, because a
  card's position means something inside its own basket and nothing outside it —
  projecting from a project basket produced a pile. Work living elsewhere is the
  Agenda's job.

- **`z` over the API**, on card create, patch and read, in the **same units as
  `pos`** so "200 nearer" is the same size of move as "200 right". Documented with
  the trap that matters: the reader may have Depth **off**, so `z` is for
  arrangement — meaning still belongs in text, a `#tag` or a `key:: value`.
  Export, import and templates carry it in both directions, and a card file
  written before depth existed still loads.

## [0.91.0]

### Added
- **Emoji are in colour.** 🔴 and 🟢 are a red circle and a green one, not two
  identical grey ones — on cards, titles, the tree, panels and menus alike.

  The recorded reason this wasn't worth doing was sound about the *font stack*:
  egui rasterizes glyph **outlines**, and every colour-emoji format is something
  else (Noto Color Emoji is CBDT/CBLC bitmaps, Apple Color Emoji is `sbix`,
  Segoe UI Emoji is COLR/CPAL vector layers). Adding such a font renders blank
  glyphs, silently.

  So the colour doesn't come from the text stack. egui has already decided where
  every glyph goes; at the end of the frame Trellis reads those positions back
  and paints the font's own bitmap over each emoji. **Layout is untouched** — the
  monochrome outline font still supplies the advance width, so wrapping,
  selection and hit-testing are exactly what they were — and where no colour font
  exists (**Windows**, whose Segoe UI Emoji has no per-glyph bitmap) nothing is
  painted and the monochrome glyph already there stands. **Settings → Canvas**
  names the font in use, because "still grey" otherwise reads as a bug rather
  than a missing font.

  Scanning the frame is what makes it consistent: hooking each place text is
  drawn would have covered titles but not markdown bodies, which is exactly the
  partial, inconsistent set that made this look not worth building.

  **Not covered:** PDF/PNG **exports**, which render through their own font stack
  and stay monochrome; and a ZWJ sequence (👨‍👩‍👧) paints as its component glyphs,
  since that is how it is laid out.

## [0.90.2]

### Fixed
- **A checklist card can be read and written back again.** `GET` started
  returning item `id`s in v0.90.0, but `PATCH` rejected them — so the natural way
  any client edits a list (GET the card, change the array, PATCH it back) failed
  with `unknown field \`id\``. Found by hitting it while editing a real card.

  Sending ids back is now not just accepted but **honoured**: each id names its
  line, so **reordering the array or deleting from the middle keeps every
  survivor's identity**, which the positional rule could not do. A payload with
  no ids still carries them across by position, so older clients are unaffected.

  The rule is chosen once per request rather than per item — a new line
  inheriting a position's id while another line claimed that id explicitly would
  give one identity to two lines, which a test now pins.


## [0.90.1]

### Fixed
- **A long task no longer turns the Agenda into a wall of text.** Now that a
  checklist line can be a task, a task's text often carries its own context —
  real rows reached 300+ characters and swamped the panel. Agenda and Kanban rows
  show the first ~80 characters, broken on a word, with the whole thing on hover.
  The card still holds the full text; only the row is shortened. Truncation
  counts **characters, not bytes**, so a line full of arrows and em-dashes can't
  panic on a split character.


## [0.90.0]

Tasks stop being a thing you copy and start being a thing that exists.

### Added
- **A checklist item with its own `due::` is its own task.** Twenty live tasks now
  cost **one card**, not twenty — each dated line gets its own row on the Agenda
  and Kanban, with its own date and its own checkbox as the done signal.

  This is the shape a working list already had; it just wasn't connected to
  anything. A 23-row list was one task at best, so five real deadlines could sit
  in a card with nothing tracking them. A checklist whose items carry dates is no
  longer listed as a task in its own right (no double-counting), and a checklist
  with no dated items behaves exactly as it always did.

- **Checklist items have stable ids.** An item used to be identified by its
  *position*, so reordering a list silently renamed every task in it, nothing
  could link to a line, and the change log couldn't say which one moved. Identity
  is the prerequisite for a task that survives being edited — and for one that
  spans time at all.

  Documents written before this load unchanged and are backfilled on open. A
  wholesale `PATCH {"items":[…]}` **carries existing ids across by position**, so
  the common edits (change text, tick a box, append) preserve identity.

  New: `POST/DELETE …/cards/{cid}/items/{item}/property` and
  `POST …/items/{item}/done` to change one line rather than the whole card.

- **`start::` — tasks that span days.** `start:: 2026-08-11  due:: 2026-08-15` is
  work *in flight for five days*, not work due on the last one. A started task
  reads as **today** on the Agenda every day until it's done or overdue, so
  multi-day work stays visible instead of hiding under a future date until it is
  already late. The card never moves; the window does. `/api/tasks` gains `start`
  and `live_today`.

- **Card links — `[[#1391]]`.** Wiki-links could only ever name a *basket*. In a
  journal-shaped document every card written on one day shares a basket, so
  `[[Tuesday 8/11/2026]]` names the day rather than the thing that happened in it,
  and the workaround was writing "card 9895" as prose. `[[#id]]` names the card,
  and following one lands **on** it — recentred and flashing — not merely in its
  basket. `[[Basket]]` and `[[42]]` are unchanged.

  New: **`GET /api/cards/{cid}/backlinks`** — what refers to this card. The
  basket-level answer is useless when the basket is a day.

  A link in a **table cell** already counted for backlinks and still does; it
  renders clickable in text card bodies.


## [0.89.0]

### Added
- **Reschedule a task from the Agenda, without opening it.** Right-click any task
  row: *Today · Tomorrow · In 3 days · Next week · Next month · Clear date*, each
  showing the date it will write, so you pick a day rather than work one out.

  This is the other half of *"one card, never copied"*. Moving a task used to mean
  clicking through to the card and editing the `due::` line by hand — and that
  friction is exactly why people copy a task card to the next day instead, which
  silently creates a **second task** with its own status and date. The correct
  workflow is now also the quickest one.

- **`DELETE /api/nodes/{id}/cards/{cid}/property?key=<key>`** — remove a
  `key:: value` line outright, which the API previously could not do. **Setting a
  property to `""` is not the same thing**: that leaves `due::` present but
  unparseable, so the task sits on the agenda under "No date" instead of leaving
  it. `cleared:false` means the card never had the property, which is an answer,
  not an error.


## [0.88.0]

### Added
- **Daily notes — opt-in, and off until you say so.** Choose a *journal root* in
  **Tools → Settings → Daily notes**, and **Ctrl+T** (View → Today's note) opens
  today's node, creating `<root> → <month> → <day>` only as far as it needs to.

  **Nothing dated is ever created any other way.** Ordinary node creation knows
  nothing about journals, so a document whose owner never asked for one cannot
  grow one — which was the constraint that kept this feature unbuilt for so long.

  The setting lives in the instance's config, so it is **per document**: a work
  document can keep a journal while a personal one never does.

  Two behaviours that matter on a journal kept by hand for months:
  - **A day already in the tree is adopted, never duplicated.** Matching is by the
    date the title *parses to*, not by string, so `8/11/2026` beside `6/09/2026`,
    a misspelled weekday, or dashes instead of slashes all resolve to the same
    day. A string comparison would sail past every one of those and create a
    second node for a day that already exists — the exact failure this replaces.
  - **A new year becomes a sibling of the old root, not a child**, and the stored
    root follows it, so January does not end up nested inside last year.

  A newly created day drops into **date order** rather than simply on top, so
  back-filling an older day lands it in place instead of above the days that came
  after it. Days already out of order are left alone.

- **`POST /api/daily {date?}`** — the same thing for agents, with full parity on
  the setting behind it: **`GET /api/daily`** reports whether it is on and which
  node is the root, and **`POST`/`DELETE /api/daily/root`** are the two buttons in
  Settings. An agent that cannot see or change the configuration cannot
  collaborate on it.

  **Pass `date` rather than building a title.** Writing `"Wednesday 8/12/2026"` by
  hand is how a journal ends up with two nodes for one day, and it is what agents
  were already doing.

### Changed
- **API.md now says how to track work**, before the endpoints that do it. One task
  is one card that never moves and is never copied; a copied task card is *N*
  separate tasks as far as the Agenda and Kanban are concerned, each with its own
  `status::` and `due::`, and nothing warns you. The Agenda is the daily list —
  that is what replaces the card people (and agents) were copying forward. Added
  because both a human and an agent independently arrived at the copying pattern.


## [0.87.0]

### Added
- **Go to a card by its id — and see ids at all.** `Ctrl+O` (View → Go to node…)
  already jumped to a **node** id; typing a **card** id found nothing, and said
  *"No matching nodes"*, which reads as the feature being broken. It was the
  common case: card ids run into the thousands while node ids stop in the
  hundreds, so nearly every id an agent quotes in a note is a card id.

  A card id now resolves, and Enter **reveals the card** — the canvas recenters
  and the card flashes, the same path the Agenda and Kanban rows use — rather
  than just opening its basket and leaving you to find it. Each row now also
  **prints the id** (`card #1391`, `node #63`) and the basket path, which is the
  first place in the app a card id is *visible* rather than only copyable behind
  right-click → Copy.

  Node ids and card ids are separate spaces, so one number can name both; both
  rows are offered rather than guessing, with the node first. Cards very often
  have no title, so a row falls back to the first real line of the body — a
  palette full of "(untitled card)" answers nothing.

- **`GET /api/cards/{cid}`** — the same lookup for agents, which is where the
  problem starts: an id read out of an earlier response or quoted in a card could
  only be resolved by walking every basket, because every other card route is
  `/nodes/{id}/cards/{cid}`. Returns the card **and its basket**
  (`{node, node_title, node_path, card}`), since every route that *edits* a card
  still needs the node. A confined agent token may resolve ids **inside its own
  basket** and is refused (403) for anything outside it: the route names no
  basket until the document resolves one, so the check happens after resolution,
  where the tree exists.

### Changed
- **Dry backup plugin 1.3.0** (shipped 2026-08-07, versioned independently of the
  app — recorded here for the trail).
- **Dry backup plugin 1.3.0 — a published link is checked anonymously before you
  are given it.** Publishing already verified `isPublicObject`, read back from
  Dry's own state rather than echoed, and **that check passed the whole time the
  link was dead**: the flag was set and a logged-out visitor was still bounced to
  a sign-in page. Checking the intent is not checking the outcome.

  The plugin now fetches the returned URL **with no credentials at all** and
  follows the redirect chain. Landing on a sign-in page is an error naming
  exactly that, and the URL is deliberately *not* offered as usable — a share
  link that only works for the person who made it is worse than an error, because
  you send it to someone before finding out. If the check cannot run at all, it
  says so instead of passing by default: *unverified* and *verified good* must not
  look the same.

  A redirect is not itself the failure. Dry's viewer canonicalises its own URLs
  even for a nonsense id, so treating any 3xx as "not public" would call a good
  link broken the moment a hop was added; what is judged is where the chain
  lands. A 2xx means the viewer served something rather than turning us away —
  necessary, not proof the card's text is on the page.

## [0.86.0]

### Changed
- **A request with an unknown field is now rejected instead of silently
  ignored.** Sending a field the API doesn't know used to return **200** and do
  nothing with it; it now returns **400** naming the field and listing what was
  expected:

  ```
  PATCH …/cards/2  {"x":10,"y":20}
  → 400 {"error":"invalid JSON body: unknown field `x`, expected one of
     `title`, `body`, `color`, `lang`, `pos`, `size`, `items`, `rows`, `kind`,
     `header`, `font_scale`, `inline_images`, `fit`, `source`"}
  ```

  This is a **breaking change for a client that sends fields the API never
  had** — such a client was already being ignored, but it was being ignored
  *quietly*. A write reported as applied that never landed is the worst failure
  an API can have, and this shape had already cost real time three separate
  ways: the example above is a genuine one, where five cards reported success
  and none of them moved.

  Applies to every JSON body the API accepts — nodes, cards, moves, groups,
  docking, charts, properties, templates and history. **Document *reading* stays
  tolerant** of unknown fields on purpose, so a newer document still opens in an
  older build; this is only about the API's input.

## [0.85.1]

### Fixed
- **Selecting text no longer floods the system with `xclip` processes.** Owning
  the X11 PRIMARY selection means handing it to `xclip`/`xsel`, which
  **daemonize and stay resident** to serve it. Trellis spawned one per selection
  change and tracked none of them, so dragging across a card title left one
  resident process per character — and the survivors then competed for selection
  ownership, breaking the clipboard **for the whole desktop**, not just for
  Trellis.

  Exactly one now lives at a time: the previous owner is killed and reaped
  before the next is spawned, and its handle is kept so it can be retired. The
  existing "did the text actually change" guard was never enough on its own,
  because a drag changes the text on every frame. Present since v0.79.0.
- **A token confined to a basket can no longer mirror files.** It could create a
  card **inside its own basket** with `source` pointing at any file the global
  mirror policy allowed, then read the contents back — so the confinement leaked
  the filesystem completely, including another Trellis document sitting on disk.
  Found by testing the new scope rather than by reading it. `source` is now
  refused outright for a subtree-scoped token (`403`), whatever the policy says:
  an agent penned into one basket has no business reading the disk. The instance
  key is unaffected, and so is your own File → Mirror a file….

## [0.85.0]

### Added
- **Give each agent a token of its own** (*Tools → Settings → Agent API → Agent
  tokens*). Type the agent's name, and Trellis mints a token, creates a root
  basket of that name, and confines the token to it: the agent can read and write
  its own workspace and **nothing else in the document**. Read-only is a
  checkbox; an existing basket or the whole document are the other choices.

  This replaces handing out the instance key, which is unrestricted and can only
  be revoked by regenerating it — breaking every other client at once. Each token
  revokes on its own, and the list says in plain words what each one can reach:
  *"SCOUT can read and change SCOUT and everything under it."*

  **The confinement is enforced on the way in.** A request naming a basket
  outside the scope is refused, and so is one that names **no** basket — which
  includes `/api/search`, `/api/tasks`, `/api/kanban` and `/api/graph`, since
  those read the whole document. That is the point rather than an oversight, but
  it does mean a confined agent cannot see your agenda; an agent that needs the
  agenda needs whole-document access, and that is a different decision to make
  deliberately. Only `/api/health`, `/api/instance`, `/api/tree` and `/api/nodes`
  are exempt — titles and shape, never card content — so the agent can find its
  own basket.

  Agent tokens are prefixed `agent_` and plugin tokens `plug_`, so a token found
  in a config file somewhere says which list to revoke it from.

### Fixed
- **Go to node accepts a node id.** Typing `12` or `#12` jumps straight to that
  node, ranked above every fuzzy title hit. Ids are what the API, `/api/tree` and
  every error message talk in, so typing one you just read somewhere is the
  obvious thing to try — and it used to find nothing at all unless the digits
  happened to appear in a title. A query that merely *contains* digits (`v2`,
  `Q4 2026`) is still a title search.
- **The Dock/Snap and Reset view buttons no longer float above windows.** They
  were drawn in the foreground layer to sit above the cards, which also put them
  above Settings, the Kanban board and every other window — visibly painted
  through them, and stealing the clicks that landed there. They now sit in the
  layer between the cards and the windows, which is what "above the cards" meant.

## [0.84.0]

### Added
- **A running plugin shows what it is doing, and can be stopped.** Its output is
  read **line by line while it runs** rather than collected at the end, so the
  Plugins window shows the latest line as it appears; a line that is a JSON
  object with `progress` (a percentage) drives a real progress bar. **Cancel**
  stops it.

  A plugin with no percentage gets a spinner rather than a made-up bar — a bar
  that isn't measuring anything is a lie about how far along the run is. Cancel
  kills the **process group**, so a plugin that shelled out takes its children
  with it: killing just the one process would have left the real work running
  *and* hung the read, making Cancel appear to do nothing. A cancelled run is
  logged as cancelled, not as a failure.

- **Plugins can be invoked from a card.** A manifest declaring the `card-menu`
  trigger appears in a card's right-click menu and is handed `TRELLIS_CARD` /
  `TRELLIS_CARD_TITLE` alongside the basket's. It receives the card's **id**, not
  its contents, and reads what it needs over the API under the scope it was
  approved for — so the trigger grants nothing new. (It was in the documented
  trigger set and had been removed in 0.80.0 rather than left declared, because a
  trigger nothing implements is the same lie as a permission that isn't checked.)

- **Emoji render.** Trellis now ships the full **outline** Noto Emoji, so
  characters newer than the subset egui bundles — 🟢 🟡 🛑 and everything else
  past Unicode 12 — draw instead of coming out as empty boxes. The PDF and image
  exporters get it too; they used DejaVu alone, which has *zero* emoji coverage,
  so an export was worse than the screen.

  **They are monochrome, and that is not a font choice.** The text stack
  rasterizes glyph *outlines*, and every colour emoji format is a bitmap
  (CBDT/CBLC) or layered format it cannot read — adding NotoColorEmoji renders
  nothing at all. 🔴 and 🟢 are two identical grey circles, so for status use a
  table card's cell colours or an inline `<span style="color:…">`.

### Fixed
- **A `[[wiki-link]]` clicked inside a card navigates**, instead of opening your
  browser. The click was read at a fixed point in the frame that ran *before* the
  canvas drew, so a card's link was always noticed a frame late — by which time
  the window manager had already been asked to open it. Links in the Backlinks
  panel, which draws earlier, had always worked; that difference was the clue.

## [0.83.0]

### Added
- **A table card can mirror a CSV/TSV file.** Point `source` at one and it fills
  the cells, re-read while the document is open — live data with **real cell
  colours and column widths**, which a markdown table can't do. The delimiter
  comes from the extension (`.tsv`/`.tab` → tab, else comma).

  A refresh replaces **cell text only**: column widths, the header flag, the
  chart spec and the formatting rules all survive it. That matters more than it
  sounds — the poll runs every few seconds, so a refresh that rebuilt the table
  would re-flatten your columns continuously while you were reading them.

- **Conditional formatting — colour cells by value.** `POST …/table
  {"op":"set_rules","rules":[…]}` with `col`, `when`
  (`gt`/`lt`/`ge`/`le`/`eq`/`ne`/`contains`/`empty`/`not_empty`), `value`, and
  `bg`/`fg`. First matching rule wins, and rules re-apply after every refresh, so
  live data stays coloured.

  A cell matching no rule is **cleared**, so a value that stops being an error
  loses its red instead of keeping it forever. Header rows are never coloured — a
  header is a label, not a value — and a non-numeric cell never matches an
  ordering rule, so a blank isn't treated as zero. Thresholds accept a **number
  or a string**; numbers use the same decorated parser as charts, so a table and
  its chart can't disagree about what a cell means.

### Fixed
- **A malformed table request says what's wrong with it.** The batch/single form
  was an untagged enum, so any mistake came back as *"data did not match any
  variant"* — naming neither the field nor the problem. The shape is now picked
  from the body and serde's real error comes through (*"invalid type: integer
  `1000`, expected a string"*).

## [0.82.0]

### Added
- **Table edits can be sent as a batch.** `POST …/table` now accepts an **array**
  of ops as well as a single one, applied in order. Building a styled table is
  inherently many small edits, and one-per-call made that both slow and easy to
  get wrong.

  Reported by an agent working over the API: sending a list was rejected with
  serde's *"invalid type: map, expected a string"* — an error naming neither the
  array nor the limitation — and because `curl` exits 0 on a 400, the edits were
  reported as applied when nothing had landed. If one op in a batch fails the
  response now says **which one and how many already applied**, so a caller can
  tell what state the table is in rather than guessing.

- **A notifications plugin** (`plugins/notify/`) — a task digest of overdue and
  due-today cards on a schedule, and a nudge when an agent changes the document,
  to Telegram. No app change was needed: it is built entirely on the change log
  (v0.76.0) and the schedule/on-change triggers (v0.81.0), which is what those
  were for. **With no bot token it prints the message rather than sending it**,
  so the wording can be checked — and the plugin tested — without a Telegram
  account. Nothing fires while Trellis is closed; the README says so rather than
  leaving it to be discovered.

## [0.81.1]

### Added
- **Plugins can ask for their settings in the Plugins window.** A manifest
  declares a `config` list — key, label, help, and `secret` for anything that
  shouldn't be shown — and Trellis renders the form, writing the values to that
  plugin's own `config.json` at mode `600`. Previously a plugin's credentials
  meant hand-editing a JSON file in a directory you'd have to be told how to
  find, which is not a setting anyone would ever change. Trellis owns the form;
  the plugin still owns the file, so a plugin's secrets never enter Trellis's
  config.
- **The Dry backup plugin takes either credential** — an **MCP token** (sent as
  a Bearer header, and what Dry recommends for headless callers) or the profile
  **access key**. The MCP token is preferred because regenerating your access key
  in Dry invalidates the old one, which would otherwise break the plugin quietly.

## [0.81.0]

### Added
- **Plugins can run on a schedule, or when the document changes.** A manifest
  declares `"triggers": ["schedule"]` with `interval_mins`, or `["on-change"]`
  with `debounce_secs`. An on-change plugin is handed `TRELLIS_SINCE` and
  `TRELLIS_REV`, so it reads exactly the entries it hasn't seen out of
  `GET /api/changes` — which is what that log was built for.

  Changes are **debounced**: a burst of edits is one run at the end, not one per
  keystroke. Measured: six rapid changes produced a single run covering all of
  them. Both triggers only run **while Trellis is open** — it is a desktop app,
  not a service — and the Plugins window says so rather than leaving you to find
  out.

- **A limit on what agents may mirror into a card** (*Settings → Agent API →
  Files agents may mirror*). v0.78.0 let anything holding the API key point a
  card at any file you can read and fetch it back, widening a leaked key from
  "all your notes" to "every file on the machine".

  The default is **anywhere except credential paths** — `.ssh`, `.aws`, `.gnupg`,
  `*.pem`, `.git-credentials` and similar — so an agent linking a README or a log
  still just works, which is the point of the feature. You can narrow it to a
  list of folders, or turn it off entirely. **Your own file picker is never
  restricted**: someone at the machine already has the filesystem. Paths are
  resolved before checking, so `..` can't step outside a folder list.

## [0.80.0]

### Added
- **Plugins** (*Tools → Plugins…*) — third-party integrations that aren't Trellis
  code. A plugin is a separate program Trellis launches, which talks back over the
  same API an agent uses. Out-of-process on purpose: a plugin that crashes is a
  non-zero exit code in a log pane, not a lost document — and it means there's no
  second data API to keep in sync with the first.

  **You approve a scope, once, in plain words.** A plugin declares what it needs
  and Trellis states it as a sentence — *"wants to read your whole Personal.ron
  document"* — before anything runs. Trellis then mints and stores the token
  itself; you never see, copy or rotate a credential, and **Revoke** kills it
  immediately. **The scope is enforced, not trusted**: a read-only plugin is
  refused on any write by the API before the request reaches your document, and a
  plugin confined to one basket is refused outside it.

  Plugins run from Tools → Plugins or from a basket's right-click menu, and live
  per instance, so approving one for personal notes doesn't approve it for work.

- **A Dry backup plugin** (`plugins/dry-backup/`) — copies a document's baskets
  and cards into a [Dry](https://dry.ai) space. One-way and **safe to re-run**:
  items are keyed by their Trellis id, so a second run updates rather than
  duplicating, and nothing is ever deleted from Dry. Run it on the whole document
  or right-click a single basket. It asks only for read access — and gets only
  read access.

## [0.79.0]

### Fixed
- **Tables are workable with select-and-middle-click again.** Table cells used a
  plain text field, so the ordinary X11 way of moving text around — select it,
  middle-click to paste — did nothing to or from a table, even though every
  other editor in Trellis has supported it for versions. Cells are now wired the
  same as the card body: **selecting text in a cell offers it to other apps**,
  and **middle-click pastes at the cursor**.

### Added
- **Right-click a table cell → Copy cell / Copy row / Copy column.** When a card
  isn't in edit mode its cells are painted, not editable, so there was no way to
  get a value out short of opening the editor or exporting the whole table. All
  three copy to the clipboard **and** the primary selection, so either paste
  works; rows go out tab-separated and columns newline-separated, which is what a
  spreadsheet expects.

## [0.78.0]

### Added
- **Mirror a file in a card.** Right-click a text or code card → **Mirror a
  file…**, and its body becomes a **read-only live copy** of that file, kept up
  to date while the document is open. Point one at a README, a config, a spec —
  the file stays the single source of truth and Trellis shows it in context,
  next to your notes about it. **Stop mirroring** keeps the text and hands the
  card back to you.

  The mirrored text is stored like any other body, so it is searchable, carries
  `#tags` and `key:: value`, and exports normally. Over the API: `source` on card
  create/PATCH, `{"source":""}` to detach. Editing a mirrored body is **refused**
  (409) rather than silently overwritten by the next refresh.

  A failed read keeps the last good text and says why (`source_error`) — a
  mirror that empties itself because a disk was unmounted is worse than a stale
  one — and it recovers on its own when the file comes back. Files are re-read
  only when their modification time changes; only text and code cards can
  mirror, up to 1 MB, UTF-8 only.

  Not a log tailer: it polls every ~3 s and shows the file from the top.

  > **Worth knowing before you enable LAN access.** A caller who can create cards
  > can point one at any file you can read, then fetch it back through the API.
  > The API is key-gated so this is not a way past authentication, but it widens
  > what a leaked key is worth from "all your notes" to "any file on this
  > machine". A directory allow-list is on the roadmap.

## [0.77.0]

### Added
- **Nodes and cards now record when they last changed** (`touched`, unix seconds,
  on both). Nothing in a Trellis document carried a timestamp before, so "show me
  the basket I was last working in" had no answer that survived closing the app —
  v0.76.0's change log knows, but it lives in memory and starts empty every run.

  **Editing a card stamps its basket too**, which is the point: work in a basket
  is editing its cards, not renaming it. A card that has never been edited reports
  no time rather than a made-up one, and the field isn't written at all until
  something changes — an untouched document gains no bytes.

  Readable in **both** directions: an older build ignores the field, so unlike the
  v0.74.0 image-storage change this is not one-way. Exposed on `GET
  /api/nodes/{id}` and every card object.

  This is the last piece "sort baskets by latest change" was waiting on; the
  sorting itself is still to come.

## [0.76.0]

### Added
- **`GET /api/changes` — what changed, not just that something did.** The only
  change signal was a revision counter: `/api/wait` told a client the document
  was different and nothing else, so the one correct response was to re-fetch
  everything and diff it. That's fine for a small reader and hopeless for sync,
  collaboration, or a plugin that wants to fire when a card gets `status:: done`.

  Each entry says **who** (a person in the app, or an agent), **what** (node /
  card / group), **which operation** (created / updated / deleted / moved),
  **which fields** (`["body","color"]`, `table.set_cell`, `images.add`, …) and,
  for a `key:: value` change, **the key and value** — so a client can decide
  whether it cares without fetching anything. Deletes carry the title, captured
  before the thing was deleted.

  Pair it with the long-poll: wait for a revision, ask what happened since the
  one you last handled, re-read only what's named. `/api/wait` now also returns
  an `epoch`, which changes when the app restarts — the log is in memory, so a
  sequence number from a previous run is meaningless and a client that sees a new
  epoch should re-read. `truncated: true` says entries you needed have rotated
  away (the last 5000 are kept).

  An entry records *that* something changed, never the old and new values —
  you re-read the entity named. That's what makes the log impossible to desync
  from, and it's why a card drag is **one** `moved` entry rather than one per
  frame: consecutive identical changes collapse, which loses nothing when there's
  no content in the entry to lose. (Measured: one drag wrote eight entries before
  collapsing, and typing would have written one per keystroke.)

  This is the shared foundation for plugin `on-change` triggers, "an agent
  replied" notifications, sorting baskets by latest change, and any hosted sync —
  each of which was independently blocked on it.

## [0.75.1]

### Fixed
- **The Windows binary is published again.** 0.75.0 built and passed 109 of its
  110 tests on Windows, but one test asserted a Unix-shaped path — `/d/work`
  isn't an absolute path on Windows, where absolute means a drive letter, so
  `--data-dir /d/work` correctly became `D:/d/work` and the test called that a
  failure. The app was right and the test was wrong; the test now uses a path
  that is genuinely absolute on the platform it's running on. Nothing about how
  Trellis behaves changed — but the failing test stopped the Windows build being
  attached to the 0.75.0 release, so that release has Linux and macOS only.

## [0.75.0]

### Added
- **Windows and macOS builds.** Every release now attaches a ready-to-run binary
  for **Linux (x86_64)**, **Windows (x86_64)** and **macOS (universal — Apple
  silicon and Intel in one file)**, plus a proper **`Trellis.app`** bundle for
  macOS so it has an icon and opens like an application rather than dropping you
  into a Terminal. The Windows `.exe` links the C runtime statically, so there's
  no Visual C++ redistributable to hunt down first.

  > **Both are unsigned.** Windows SmartScreen will warn on first run (*More
  > info → Run anyway*), and macOS Gatekeeper will refuse outright until you
  > right-click → **Open** once, or run
  > `xattr -d com.apple.quarantine /path/to/Trellis.app`. Signing them needs
  > paid developer certificates from Microsoft and Apple.

- **Tools → Requirements…** — one window listing every optional external tool
  (Tesseract, GnuPG, rclone, OpenSSH, and on Linux xclip and a screenshot tool),
  whether it's installed, and **what exactly it switches on**. Missing ones come
  with a way to fix it rather than a name to google: an **Install** button that
  runs the real command where the platform has a package manager we can drive
  (winget on Windows, Homebrew on macOS), the **exact command to copy** on Linux
  (installing there needs a root password, which an app has no business asking
  for), and a download link everywhere else. "Install tesseract-ocr" is the
  package name on exactly one distribution and helps nobody else.
  Failures now point here too — OCR without Tesseract says where to get it.

### Fixed
- **The agent API key is now generated by the operating system's secure random
  generator on every platform.** It was read straight from `/dev/urandom`, which
  doesn't exist on Windows — there the read failed and fell back to a mix of the
  process id and the clock, both of which are guessable. On Linux and macOS the
  key was always sound; anyone who generated a key on a **Windows** build should
  press **Generate** again in *Settings → Agent API*.
- **`--data-dir` now really does give an instance its own settings on Windows
  and macOS.** It worked by setting `XDG_DATA_HOME`, which only Linux and BSD
  honour, so on the other two everything except the autosave slot — API key,
  port, theme, backup schedule, template library — stayed shared between
  instances, and running work and personal documents side by side didn't
  actually keep them apart. The settings file is now named outright. **Existing
  Linux instances are unaffected**: the path is exactly the one that was being
  produced before (`<data-dir>/trellis/app.ron`), so there is nothing to move.
- **Snip to card works on Windows and macOS.** It only ever knew about Linux
  screenshot tools. macOS uses the built-in region capture; Windows opens the
  Snipping Tool overlay (the Win+Shift+S one) and picks the result up. Capture is
  also judged by whether an image actually arrived rather than by the tool's exit
  code, which the various Linux tools disagree about — so cancelling reliably
  cancels instead of sometimes reporting an error, and a leftover file from a
  previous cancel can't come back as a duplicate screenshot.

## [0.74.3]

### Added
- **`GET /api/history` reports the retention settings** — `keep` and
  `min_gap_mins` alongside the snapshot list, so an agent can see what governs
  the snapshots it's looking at (and why an expected one is no longer there).
  Read-only, matching how `GET /api/backup` reports its schedule; both are set
  in the app.

## [0.74.2]

### Fixed
- **"Fit to content" now accounts for markdown headings.** Height was measured
  with every line at the body font size, but the card renders headings *larger* —
  so a note full of `##` headings was measured short and its bottom was clipped.
  Each line is now measured at the size it will actually render at, reading the
  body and heading sizes from the theme and interpolating H2–H6 the way the
  CommonMark renderer does. (`#tag` is still not a heading — there has to be a
  space after the hashes — so tagged cards don't inflate.)
- **Long cards fit instead of being cut off.** Fit clamped height to 1400px, and
  a text card has no per-card scroll, so anything longer than that lost its
  bottom silently rather than simply being tall. The cap is now 6000px: still a
  guard against one runaway card, but clipping content is the worse failure.

## [0.74.1]

### Fixed
- **`"fit": true` over the API left a strip of empty card under the text**, which
  disappeared the moment you used right-click → *Fit to content* on the same
  card. Two different measurements were in play: the menu action lays out the
  real text with egui's fonts, while the API path could only *estimate* the
  wrapped height — and it estimated tall. The estimate exists because
  `Card::fit_size` has to stay font-free, but API requests are applied on the UI
  thread, where the real fonts *are* available, so the card is now re-measured
  the same way the menu action does. Both paths give the same size; agent-created
  cards no longer arrive with a gap.

## [0.74.0]

> **One-way format change.** Documents saved by this version store image bytes
> as base64. Older builds can't read that, so a document saved here won't open
> in 0.73.x or earlier. This version reads *both* forms, so upgrading needs
> nothing — but keep a backup if you might roll back. Your version-history
> snapshots and existing basket exports are unaffected and still load.

### Changed
- **Documents with images are much smaller and save far faster.** Image bytes
  were stored the way serde writes a `Vec<u8>` — a decimal list, `[137,80,78,…]`
  — costing about 3.5 characters per byte. On a real document, 16 MB of
  screenshots occupied **56 MB of a 60 MB document**, and gzip then spent
  seconds undoing that bloat on every single save. Images are now stored as
  **base64** (1.33× instead of 3.5×). Measured on that document: **20.4 MB →
  16.1 MB on disk (21% smaller), and compression dropped from 5.1 s to 0.6 s
  (9× faster)**. Loading, saving, autosave, backups and history snapshots all
  get proportionally cheaper.
- Old documents load unchanged and are converted the next time they're saved;
  no migration step and nothing to click.

### Fixed
- **Closing the app no longer appears to hang on a large document.** The save on
  exit is deliberately synchronous, and it was also writing a version-history
  snapshot — a full read *and* write of the document, on top of the save that
  had just happened. That's now skipped on exit: the document is still saved,
  and history still has the state from before it. Closing a 16 MB document went
  from multi-second to **~1.2 s**.

### Added
- **Version-history retention is configurable** — *Tools → Settings → Version
  history*: how many snapshots to keep (default 25) and the minimum minutes
  between them (default 3). Both used to be compile-time constants. A snapshot
  is a complete copy of the document, so a large one wants fewer, spaced wider;
  the settings panel shows what the current choice costs for *this* document.
  Values are clamped on load, so a hand-edited config can't switch history off
  or fill the disk.

## [0.73.1]

### Added
- **`GET /api/nodes/{id}/cards/{cid}` — read one card.** The path already
  accepted PATCH and DELETE, but not GET, so an agent that wanted to check a
  card it had just written had to fetch the entire basket and filter it. It
  returns the same card object that appears in the basket listing. Reported by
  an agent working over the API.

## [0.73.0]

### Added
- **`autofit_cols` — tables built over the API are readable now.** Columns are
  110px until something changes them and cell text doesn't wrap, so a table an
  agent filled in with `rows` clipped every long cell, and the only fix was
  guessing a pixel width per column by hand. `POST
  /api/nodes/{id}/cards/{cid}/table {"op":"autofit_cols"}` sizes every column to
  its longest cell — or just one with `{"op":"autofit_cols","col":2}`. Widths are
  bounded at 600px, so a single runaway cell can't produce an unusable card.
  Note the order: `autofit_cols` sizes the **columns**, then `"fit": true` on the
  card sizes the **frame** around them (`fit` on its own never widened a column,
  which is why "always pass fit" still gave you narrow columns).

## [0.72.0]

### Added
- **Worked examples in the app** — **Tools → Settings → Agent API → Examples** is
  a new section of copy-paste `curl` commands, filled in with *this* instance's
  host, port and key: what document am I talking to, read the tree, add a card,
  add a task that lands in the Agenda and Kanban, build a populated table in one
  call, chart it, filter tasks to one project, and long-poll for changes. The
  endpoint list says what exists; these say how to drive it.

### Changed
- API.md's Card data model now lists the table card's **`chart`** field, which was
  documented in the Charts section but missing from the field table.

## [0.71.1]

### Fixed
- **The agenda was a day ahead in the evening.** "Today" was computed from the
  UTC clock, but a `due::` date is a calendar date you wrote looking at your own
  calendar. West of Greenwich the two disagree for part of every day — in
  California, from 5pm until midnight UTC is already tomorrow, so a task due
  tomorrow showed under **Today** and today's showed as **Overdue**. Today is now
  the machine's *local* calendar day, run through the same parser that reads
  `due::` so the two can't disagree. This fixes the Agenda, the Kanban board's
  overdue highlighting, and `today_days` in the API (so the phone app inherits it).
- **Version-history timestamps are shown in local time.** Snapshots are stamped
  in UTC on disk — deliberately, since that stays monotonic and avoids the hour
  that repeats at a DST fall-back — but the list showed that raw, so a snapshot
  taken at 09:20 read back as 16:20. Filenames are unchanged; only the display
  converts.

## [0.71.0]

### Added
- **The Kanban board can be filtered by project too**, with the same dropdown and
  × as the Agenda, and its cards are colour-coded by project the same way. Each
  view remembers its **own** filter — they answer different questions, so scoping
  the board to one project doesn't narrow your agenda.
- **`GET /api/kanban?project=<node id>`**, matching `/api/tasks?project=`; kanban
  cards also carry `project` and `project_title`.

## [0.70.0]

### Added
- **Filter the Agenda by project.** A dropdown at the top of the panel narrows
  the list to one top-level project, with an × to go back to all of them. The
  choice is remembered between launches, and a project that no longer exists
  falls back to "all" rather than silently hiding every task.
- **Colour in the Agenda.** Each row gets a dot, and its project name is drawn,
  in that project's colour — the node's own colour tag when it has one (so it
  matches the tree), otherwise a stable colour picked from a palette. A long
  agenda now groups by project at a glance instead of reading as a grey wall.
- **`GET /api/tasks?project=<node id>`** filters the same way for agents. It
  accepts any node, not just a root, so you can narrow to a sub-branch with the
  same parameter; tasks also now carry `project` and `project_title`.

## [0.69.0]

### Fixed
- **The Agenda and Kanban board now show each task's full basket path.** They
  showed only the basket's own name, and project folders reuse names — with an
  "Open Items" under two different projects, a task read simply as "Open Items"
  with nothing to say which project it belonged to. That is not hypothetical: it
  led an agent to treat another project's task as its own, and someone had
  already worked around it by renaming a basket by hand.
  Rows now read `Newsletter › Open Items`.
- **`GET /api/tasks` and `GET /api/kanban` gained `node_path`** alongside
  `node_title`, so anything reading the API can attribute a task correctly.
  `node_title` is unchanged, so existing clients keep working — but `node_path`
  is the one to show.
- The Android app's Agenda and Kanban show the path too, falling back to the
  basket name when talking to an older desktop.

## [0.68.0]

### Added
- **Pie charts.** A table card can now be drawn as a **pie** (`kind: "pie"`, or
  `donut`), joining bar/line/scatter. Slices are labelled with their percentage
  when there's room, the legend lists every slice, and hovering one lifts it and
  shows its exact value.

  A pie divides a single whole, so it behaves differently from the x/y charts —
  deliberately, and both rules are documented:
  - It draws the **first** series only. Other columns are ignored rather than
    stacked, because a pie can only show one set of parts.
  - **Only positive values get a slice.** Blanks, zeros and negatives are
    skipped, and percentages are of the positive total. A negative has no arc,
    and quietly folding it in as its magnitude would misstate every other slice.
    A table with nothing positive says so instead of drawing an empty circle.

### Fixed
- **`show_table` now works with a pie** — the grid is drawn under the chart, as
  it already was for the other kinds.

## [0.67.0]

### Added
- **Charts.** A **table** card can now be drawn as a **bar, line or scatter**
  chart — pick one from the table's toolbar (or `POST
  /api/nodes/{id}/cards/{cid}/chart {kind}`; `DELETE` the same path goes back to a
  grid). The table stays the data: the chart is a *view* of the same cells, so
  editing a cell — or a `rows` PATCH, or a table op — redraws it, and **Show grid**
  keeps the spreadsheet visible underneath.
  - Series come from the columns, named by the header row. Leave `value_cols`
    empty and every numeric column is plotted, so adding a column just works.
  - Numbers may be decorated: `1,234.5`, `$12`, `40%`, `(3)` = −3.
  - **A non-numeric cell is a gap, not a zero** — a line breaks across it rather
    than diving to 0, because plotting a blank status cell as a measured 0 would
    invent data. A lone reading between two gaps still shows, as a dot.
  - *Pie charts are not in this version.*
- **`rows` and `header` on card creation.** `POST /api/nodes/{id}/cards` now
  accepts a table's cells directly, so a populated table — and a chart drawn from
  it — takes one call instead of create-then-PATCH.

### Fixed
- **A `rows` PATCH no longer turns a chart back into a plain grid.** `rows`
  replaces the table's *data*; the chart is a view setting on that data and now
  survives the refill.

## [0.66.0]

### Added
- **Saved templates now live somewhere you can see them.** A template used to be
  an invisible entry in the app's settings — you could stamp copies of it, but
  there was nothing to look at and nothing to edit, so keeping a "master" card
  around was a convention you had to know about (and every agent invented its own
  place to put one). Now **saving a template also puts its master card in a
  root-level `Templates` basket**, created the first time you need it:
  - Edit the master, then right-click → **Update template**, and every copy you
    stamp afterwards uses the new version.
  - Saving a template *from* a card already in `Templates` adopts that card as
    the master rather than cloning it.
  - Deleting a template deletes its master card too.
  - **Tools → Rebuild Templates basket** (`POST /api/templates/rebuild`) gives a
    master card to every template saved before this existed. It only fills in
    what's missing, so it's safe to run twice.

  The stored snapshot stays the authority — inserting always stamps *it*, never
  the master — so editing a master changes nothing until you update. A stray edit
  must not silently change every future insert.
- **Move a card to another basket over the API** —
  `POST /api/nodes/{id}/cards/{cid}/move {node:<target>, pos?:[x,y]}`. Previously
  `move` could only reorder a card *within* its basket, so relocating one meant
  rebuilding it by hand (which resets a table's column widths). Group membership
  and docking are dropped in the move, since both reference ids that only mean
  something in the old basket.

### Changed
- `GET /api/templates` reports `master_node` / `master_card` for each template
  (null when it hasn't got a master), and register/update return them too.
  Existing saved templates load unchanged — the stored format is backwards- and
  forwards-compatible.

## [0.65.1]

### Changed
- **Docs brought current for running several instances.** A three-way audit of
  the 49 live routes against API.md and the in-app **Settings → Endpoints** list
  came back at full parity; the rest of the docs now match the behaviour too:
  - **Templates and backup settings are per instance**, not per document — they
    live in the app config, so instances with different `--data-dir`s have
    independent template libraries and backup schedules, and a backup covers the
    document that instance has open. Now stated in API.md and the README instead
    of only being implied.
  - **Version history** is per document (a hidden `.<name>.history/` beside the
    file), and edits made over the API are snapshotted like any other.
  - **What "saved" means for an agent:** an API edit is written to disk about two
    seconds after the last change, debounced, on a worker thread, even when the
    window is idle — no save endpoint and nothing to press. API.md's persistence
    note said only "on save / autosave-on-exit", which undersold it.
  - The **web clipper** README notes that with an instance per document, the
    port (and key) chooses which document clips land in.
  - The **Settings → Port** hint mentions `--port` / `--data-dir` and points at
    `GET /api/instance`, and the README's Docs list links the clipper and
    `trellis --help`.

## [0.65.0]

### Added
- **Run separate documents side by side.** Trellis now takes command-line
  arguments: `trellis [FILE] [--port PORT] [--data-dir DIR]`. `FILE` opens a
  document (creating it if the path is new), `--port` sets the agent API port for
  that run, and `--data-dir` gives an instance its **own settings** — API key,
  port, theme, backup config and autosave slot. Together they let you run one
  instance per document, each on its own port:

  ```sh
  trellis ~/work.ron     --port 7373 --data-dir ~/.local/share/trellis-work
  trellis ~/personal.ron --port 7374 --data-dir ~/.local/share/trellis-personal
  ```

  Keeping, say, work and personal notes in separate documents gives each its own
  version history and backups, and means an agent pointed at one can't read or
  rewrite the other. With no arguments Trellis behaves exactly as before.
- **The window title shows the open document** (`work.ron — Trellis`), and
  follows New / Open / Save As — so two instances are tellable apart in the
  taskbar. It used to always read "Trellis".
- **`GET /api/instance`** — which document an instance is serving (`document`,
  `path`, `port`, `lan`, node count, unsaved-changes flag) plus the app version.
  With one instance per document, the port is how you address a document, so an
  agent can confirm it's driving the right one before writing. Needs the API key
  (unlike `/api/health`), since it reveals a file path.

### Fixed
- **A failed API bind is no longer easy to miss.** If the port is already taken
  (typically a second instance on the same port) the status bar now says the
  agent API is off, instead of only the Settings panel showing it — that instance
  serves no requests, and the mistake used to be silent.

## [0.64.0]

### Added
- **Editable templates — "Update template".** A template was a frozen snapshot;
  editing the card you registered never changed it. Now you can keep a **master
  card** (e.g. in a Templates node), edit it, then **right-click → Update template
  → pick which** to re-snapshot that template *in place* — it keeps its index and
  name, and every future **Insert template** stamps the new version. Over the API:
  `POST /api/templates/{index}/update {node, card, title?}`. This makes a Templates
  node a real, maintainable template library (edit the master, update, reuse).

## [0.63.1]

### Fixed
- **A canvas drag that passes over the minimap no longer hijacks the view.** The
  minimap claimed any drag while the pointer was over it, so a normal drag that
  strayed into the bottom-right corner would grab the reticle and teleport you
  elsewhere. It now only takes over when the **press begins inside the map** —
  click into the minimap first, then drag. Clicking the map to jump still works.

## [0.63.0]

### Added
- **`GET /api/kanban`** — cards grouped by their `status::` value into columns
  (each card with its title, basket, `due::`, `#tags`, and accent color), so the
  Kanban board is reachable over the API. Read-only; the existing card `property`
  endpoint changes a card's column. Powers the Android app's new read-only Kanban.
  API.md + the in-app Endpoints list updated.

## [0.62.1]

### Fixed
- **SynthWave no longer paints every bold label pink.** The theme's *strong* text
  color (used by every `.strong()` label — Kanban card titles, and the Search /
  Tags / Agenda / Backlinks headers) was the loud pink accent, so emphasis text
  read pink-on-dark all over the app. Strong text is now a bright near-white;
  pink stays an accent on active borders, selection, links, and window edges.

## [0.62.0]

### Fixed
- **Card titles are always readable now.** They used egui's *strong* text color,
  which is the theme's loud active-widget accent — so SynthWave rendered **pink
  title text on a blue card**, a low-contrast eyesore. Titles now pick a light or
  dark color from the title bar's own brightness, in every theme.

### Changed
- **Neon themes glow, and Futuristic skews harder.** Futuristic and SynthWave draw
  a soft accent **glow** behind each card for a radiant look. Futuristic's beveled
  tech-panel corners are bigger (a more pronounced angular skew, with a bright edge
  on the top-right cut) while the card content stays axis-aligned.
- **Futuristic is now clearly its own theme** — a teal-tinted dark with brighter
  cyan, lifted well off Trellis's neutral gray so the two dark themes no longer
  look near-identical.
- **SynthWave reworked into a Hotline-Miami look** — a dark, near-black interface
  (readable, not a wash of purple) with hot pink + electric cyan used only as
  *accents*: card edges, glow, selection, active widgets, and links.

## [0.61.0]

### Added
- **Three new themes** (View → Themes), selectable like the others:
  - **Sticky Notes** — cards are solid single-color paper (header and body the same color, like a
    real sticky), yellow by default; recoloring a card paints the whole note. On a cork-board canvas.
  - **Futuristic** — a Minority-Report holographic-blue HUD, with cards drawn as beveled tech panels
    (top-right and bottom-left corners cut) and cyan-accent edges.
  - **SynthWave** — the classic outrun neon palette: hot pink (#FF3864) + electric cyan (#2DE2E6)
    on deep violet (#0D0221), with purple UI chrome.
  Themes now drive card *rendering* (a per-theme `CardStyle`), not just colors.

## [0.60.0]

### Fixed
- **Kanban board no longer sprawls sideways.** Cards in a column were laid out *horizontally* (the
  column's group inherited the row's layout direction), so a column was as wide as all its cards in a
  row — a `done` column with a dozen cards ran far off the right edge and forced endless horizontal
  scrolling. Cards now **stack vertically**, and each column **scrolls its own cards**, so a tall
  column never overflows the board.

### Changed
- **Kanban board is much nicer.** Columns now **divide the window width** (no horizontal scroll until
  you genuinely have more columns than fit). Each card shows its **accent color** (as its border),
  its **`due::` date** (red when overdue), and its **`#tags`** — not just the title. A **Show done**
  toggle hides the finished pile to focus on active work. Bigger default window.

## [0.59.1]

### Fixed
- **Tables now scale uniformly when you zoom the canvas** (`ctrl+scroll`). The card frame and cell
  text scaled with zoom, but the cell **rectangles** (column widths, row heights, handles) were
  drawn at a fixed pixel size — so the grid stayed full-size inside a shrinking card and columns got
  clipped. Every table dimension is now multiplied by the zoom, so the whole table shrinks and grows
  as one, like every other card.

## [0.59.0]

### Added
- **Minimap** (**Settings → Canvas → Minimap**, on by default) — a small overview of the whole
  basket in the canvas's bottom-right corner, with an amber reticle showing your current view.
  Each card is a dot in its own color, so you can spot cards that sit far from the main cluster
  without zooming out. **Click or drag on the minimap to jump the view there.** Pure view aid — no
  document or API change.

## [0.58.0]

### Added
- **Card templates over the API.** The reusable card snapshots you make with right-click
  **Save as template** / **Insert template** are now fully agent-drivable, so an agent can build a
  layout once and stamp it out repeatedly — e.g. a Task / Local / Prod verification grid it fills
  in as it works:
  - `GET /api/templates` — list saved templates (`index`, `title`, `kind`).
  - `POST /api/templates {node, card, title?}` — snapshot an existing card as a template (build a
    table with headers + cell colors first, then register it). Captures the card's whole
    definition, tables and all.
  - `POST /api/templates/{index}/insert {node, pos?}` — stamp a template into a basket as a new
    card; returns the created card.
  - `DELETE /api/templates/{index}` — remove a template.

  Templates persist in app config and are shared by the UI and the API. API.md and the in-app
  Settings → Endpoints list updated to match.

## [0.57.4]

### Docs
- **In-app Settings → Endpoints list brought back to full parity with API.md.** It was missing the
  card **dock**, **group-membership**, **table**, **sketch**, and **images** endpoints and the
  **groups** CRUD; all are now listed (the sub-verbs — `GET`/`DELETE`/`PATCH` variants — noted
  inline). The card create/patch field lists and the `card` id now returned on search hits are
  reflected too. API.md stays the canonical reference and was already current.

## [0.57.3]

### Changed
- **Search, Find, Tags, and Backlinks now reveal the exact card, not just its basket.** Clicking
  a result recenters the canvas on the matching card and flashes a brief highlight — the same
  reveal the Agenda and Kanban board got in v0.57.1 — instead of only selecting its node. Search
  in particular previously just selected the node and never moved the canvas, so a match in a
  basket you were already viewing (or scrolled off) appeared to do nothing. This completes the
  "reveal-on-click parity" across every result panel.

### API
- Every `hits` list (`/api/search`, `/api/tags?name=`, `/api/properties`, `/api/query`,
  `/api/nodes/{id}/backlinks`) now includes a **`card`** id on each hit, so an agent can point
  straight at the matching card. It is `null` only for a search hit that matched a node **title**
  rather than a card.

## [0.57.2]

### Fixed
- **"Fit to content" no longer leaves a Text card almost twice as tall as its text.** A card
  whose title is long enough to widen it (the width floor keeps the title readable) had its
  height measured for a *narrower* wrap than it actually renders at — so the estimate reserved
  far too many lines and the card came out ~2× tall with a big empty gap under the text. The
  height is now measured at the card's real wrap width, with a rendered-accurate line height and
  markdown syntax (`**`, `` ` ``) excluded. The right-click **Fit to content** action goes
  further and measures the *actual* rendered text with egui's fonts, so the card hugs its content
  exactly. The same tightened estimate applies to cards sized via the API `fit` flag / import.

## [0.57.1]

### Fixed
- **Agenda / Kanban clicks now reveal the actual card.** Clicking a task in the Agenda panel
  (or a card on the Kanban board) previously only selected its *node* — if you were already
  viewing that basket, or the card was scrolled off in a large basket, nothing appeared to
  happen. Now the canvas recenters on the exact card and flashes a brief highlight outline so
  the click clearly lands.

## [0.57.0]

### Added
- **Web clipper browser extension** (`web-clipper/`) — a small Manifest V3 extension for
  Chrome/Edge that clips the current page, or the selected text, into a Trellis basket over the
  LAN API. Load it unpacked; see `web-clipper/README.md`.
- **CORS on the API** — the key-gated API now sends permissive CORS headers and answers OPTIONS
  preflight, so browser extensions, bookmarklets, and future web clients can call it from any
  origin (the API key is still required).

## [0.56.0]

### Added
- **Kanban board** (**View → Kanban board**) — every card with a `status::` property shown as a
  column by status (To do / Doing / Done, plus any other status values you use). Drag a card to
  another column to change its `status`; click to jump to its basket. No new card type — it reads
  the same properties the agenda uses, so a task is one card seen two ways. API to move cards:
  `POST /api/nodes/{id}/cards/{cid}/property {key, value}`.

## [0.55.0]

### Added
- **Snip to card** (**Tools → Snip to card**) — capture a screen region straight into an image
  card in the current basket (uses the first available region-screenshot tool: spectacle,
  gnome-screenshot, maim, scrot, or ImageMagick import). Runs off the UI thread.
- **OCR all images** (**Tools → OCR all images**) — run OCR over every image card that doesn't
  have extracted text yet, in one background pass, making old scans/screenshots searchable.
  Also over the API: `POST /api/ocr`.

## [0.54.0]

### Added
- **Link graph** (**View → Link graph**) — a force-directed visualization of the `[[wiki-link]]`
  web: nodes that participate in links, edges between them, click a node to open it. Rebuilt each
  time it's opened so it reflects current links. API: `GET /api/graph` (nodes + edges).

## [0.53.0]

### Added
- **Version history over the API** — `GET /api/history` lists snapshots; `POST /api/history/restore
  {file}` restores one (path-traversal-guarded). Closes the API-parity gap for version history.

### Changed
- Refreshed the in-app **Settings → Endpoints** list to include every current endpoint (tags,
  properties, query, tasks, backlinks, history), so it stays a complete quick reference.

## [0.52.0]

### Added
- **Wiki-links + backlinks** — write `[[Node Title]]` (or `[[id]]`, or `[[Target|shown text]]`)
  in any card and it renders as a clickable link that jumps to that node. **View → Backlinks**
  shows every card that links *to* the selected node ("linked here"), click to jump. API:
  `GET /api/nodes/{id}/backlinks`. Foundation for the graph view.

## [0.51.0]

### Added
- **Version history** (**Tools → Version history**) — as you save, Trellis keeps automatic
  timestamped snapshots of the document in a hidden sibling folder (`.<name>.history/`), up to
  25, at least a few minutes apart. Browse them and **Restore** an older version as the current
  document (then save to keep it). This is a local safety net, distinct from the external Backup
  module. Snapshots are written off the UI thread alongside each save.

## [0.50.0]

### Added
- **Find cards** (**View → Find cards**) — a cross-tree query panel driven entirely by
  dropdowns (pick a `#tag`, pick a property + value) plus an optional text box; no syntax to
  remember. Live results link back to their basket. API: `GET /api/query?tag=&key=&value=&text=`.
- **Task agenda** (**View → Agenda**) — every card with a `due:: <date>` shown as a task,
  grouped **Overdue / Today / This week / Later**, across every basket, click to jump. A task
  is done when it has `status:: done` (or a fully-checked checklist); completed tasks are
  hidden unless you tick "Show completed". One canonical task, no copying. API: `GET /api/tasks`.

### Fixed
- Replaced UI glyphs that rendered as empty squares in the bundled font (the backup window's
  add/remove buttons, the tree's Expand-all/Collapse-all items, the Tags panel back button)
  with glyphs the font actually includes.

## [0.49.0]

### Added
- **Card properties** — inline `key:: value` fields (Dataview-style) written in any card are
  parsed as metadata, e.g. `due:: 2026-08-15`, `priority:: high`, `status:: open`. The `::`
  must be followed by a space, so code like `std::fmt` and URLs aren't matched; a whole line
  or a bracketed `[due:: 2026-08-15]` both work. A card's JSON now includes
  `properties:[{key,value}]`, and the API can list/filter by them: `GET /api/properties`,
  `?key=<k>`, `?key=<k>&value=<v>`. This is the foundation for due dates + the task agenda.

## [0.48.0]

### Added
- **#tags across baskets** (**View → Tags…**). Write `#tags` in any card and they're indexed
  document-wide; the Tags panel lists every tag with a count, and clicking one shows the cards
  that carry it (click a result to jump to its basket). Tags are lowercased and support nesting
  (`#work/urgent`); a Markdown `# Heading`, a URL `page#frag`, and a bare `#123` are not tags.
  Also over the API: `GET /api/tags` and `GET /api/tags?name=<tag>`.

## [0.47.0]

### Added
- **Quick switcher** (Ctrl+O, or **View → Go to node…**) — a centered palette that
  fuzzy-matches any node by title or full path. ↑/↓ to move, Enter (or click) to jump:
  it selects the node, expands its ancestors, and scrolls it into view. Fast navigation
  for a deep tree without hand-scrolling the sidebar. Ranking prefers a title substring,
  then a title subsequence, then a path match.

## [0.46.0]

### Added
- **Backup module** (**Tools → Backup…**) — scheduled, full-document backups to external
  destinations. This is backup, *not* version control: each run writes a complete,
  self-contained copy (the same compressed format Trellis saves). Destinations:
  **Disk** (a local/mounted folder), **Network (SFTP)** via `scp`, and **Cloud** via
  `rclone` (S3, Drive, Dropbox, B2, …). Optional **encryption** with `gpg` symmetric
  AES-256 (passphrase fed to gpg off-argv; plaintext streamed, never written to disk).
  Configurable interval and per-disk retention (keep newest N). Runs on a worker thread so
  a slow target never freezes the canvas. Also over the API: `GET /api/backup` (status) and
  `POST /api/backup/run` (back up now). Restore a backup with
  `gpg -d file.ron.gz.gpg > file.ron.gz` (if encrypted), then open the `.ron.gz` in Trellis.

## [0.45.0]

### Added
- **Reorder cards within a basket** — `POST /api/nodes/{id}/cards/{cid}/move` with
  `{before|after:<cid>}`, `{index:<n>}`, or `{to:"front"|"back"}`. Card order is both the
  draw order (last = on top) and the order Autosort places cards in, so an agent can now lay
  a basket out in a chosen reading order and then autosort it.
- **Expand / collapse a whole branch** — right-click a node → **Expand all** / **Collapse
  all** to open or fold its entire subtree in one click, for working with big node sets. Also
  over the API: `POST /api/nodes/{id}/expand {expanded, recursive?}`. `GET /api/nodes/{id}`
  now reports the node's `expanded` flag.

## [0.44.0]

### Added
- **Reorder / reparent nodes over the API** — `POST /api/nodes/{id}/move`. Placement is
  one of `{before|after:<nid>}` (drop next to a sibling, adopting its parent),
  `{parent?, index:<n>}` (absolute slot; past-the-end appends), or
  `{parent?, to:"top"|"bottom"}`. `parent` omitted keeps the current one, `null` promotes
  to the top level, or an id reparents. Rejects (400) any move that would nest a node inside
  its own subtree. Closes the gap where agents could create nodes but not order them — the
  sidebar renders raw child order, so this lets an agent place a basket exactly where a user
  would drag it. See `API.md`.

## [0.43.0]

### Added
- **Inline images in Text cards.** Drag an image file onto a Text card to embed it in the
  body (or right-click in edit mode to insert at the cursor), referenced by an
  `![alt](trellis:N)` marker. Fit-to-content accounts for the image size; HTML/Markdown
  exports embed it as a data URI; the PDF text layer and search use the alt text. API:
  `inline_images` (base64 list) on card create/update, applied before `fit`; card JSON
  reports `inline_image_names`. Card export/import (JSON) round-trips the images.

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
  root-to-node breadcrumb, e.g. `PROJECT › AREA › ITEM`). Both copy to the
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
