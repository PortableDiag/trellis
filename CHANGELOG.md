# Changelog

All notable changes to Trellis. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); versions are the app version in
`Cargo.toml`, each with a matching git tag and GitHub release.

## [Unreleased]

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
- **A date property stops at the date.** `due:: 2026-08-15 — RUN 1 DONE 8/12: …`
  is a real line from a real checklist, and the value ran to the end of it. That
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
  *"ALICE can read and change ALICE and everything under it."*

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
  already worked around it by renaming a basket "LANAgent Open Items" by hand.
  Rows now read `Super Weapon News › Open Items`.
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
