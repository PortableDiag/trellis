//! Importing an **Obsidian vault** — a folder tree of Markdown files — as a tree
//! of baskets.
//!
//! v0.123.0 taught the boundary to speak YAML frontmatter, one dropped file at a
//! time. That is the *field* half. A vault is not a file, it is a **shape**: a
//! folder tree, notes that link to each other by name, and attachments that live
//! beside the notes and are referenced with `![[file.pdf]]`. Importing it one file
//! at a time loses every one of those relationships.
//!
//! The mapping, and why each half is the obvious one:
//!
//! | Obsidian | Trellis |
//! |---|---|
//! | folder | basket |
//! | note (`.md` file) | **card** |
//! | frontmatter | `key:: value` / `#tags` |
//! | `![[file.pdf]]` | an **attachment** on the referencing card |
//! | `[[Note]]` | `[[#<cid>\|Note]]` — a card link |
//!
//! **A note is a card, not a basket**, which is the one call worth stating. A
//! basket is a *space* holding things; a note is a *thing*. The whole vault is
//! notes, so mapping them to baskets would produce a tree of empty containers. It
//! also decides the link question: Trellis's bare `[[Title]]` resolves to a
//! *basket*, so every imported `[[Note]]` would dangle. They are rewritten to
//! `[[#id|Note]]` in a second pass, once every card exists and has an id — the
//! link then resolves, and the pipe keeps the name the author wrote.
//!
//! **Nothing here reads the filesystem except [`read_vault`].** Everything else
//! takes a list of `(path, bytes)`, so the mapping is testable without a fixture
//! directory on disk.

use crate::model::{CardKind, Document, NodeId};
use std::collections::BTreeMap;
use std::path::Path;

/// One file from a vault: a `/`-separated path **relative to the vault root**,
/// and its bytes.
pub struct VaultFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// What an import did, for the status line and the API's answer.
#[derive(Default, serde::Serialize)]
pub struct VaultReport {
    pub root: NodeId,
    pub baskets: usize,
    pub cards: usize,
    pub attachments: usize,
    /// `[[Note]]` links that found their card and became `[[#id|Note]]`.
    pub links_rewritten: usize,
    /// Link targets naming no note in the vault, left exactly as written — a
    /// dangling link the author can still read beats a silently deleted one.
    pub unresolved: Vec<String>,
}

/// One line for the status bar.
///
/// **Unresolved links are named, not just counted.** A number tells you something
/// went wrong without telling you where; the first few names are what a reader can
/// act on, and they are the notes the vault referred to but did not contain.
pub fn describe(r: &VaultReport, name: &str) -> String {
    let mut s = format!(
        "Imported vault \"{name}\": {} card{} in {} basket{}",
        r.cards,
        if r.cards == 1 { "" } else { "s" },
        r.baskets,
        if r.baskets == 1 { "" } else { "s" },
    );
    if r.attachments > 0 {
        s.push_str(&format!(", {} attachment{}", r.attachments, if r.attachments == 1 { "" } else { "s" }));
    }
    if r.links_rewritten > 0 {
        s.push_str(&format!(", {} link{} rewritten", r.links_rewritten, if r.links_rewritten == 1 { "" } else { "s" }));
    }
    if !r.unresolved.is_empty() {
        let shown: Vec<&str> = r.unresolved.iter().take(3).map(|s| s.as_str()).collect();
        s.push_str(&format!(
            " — {} link{} named no note ({}{})",
            r.unresolved.len(),
            if r.unresolved.len() == 1 { "" } else { "s" },
            shown.join(", "),
            if r.unresolved.len() > shown.len() { ", …" } else { "" },
        ));
    }
    s
}

/// Read a vault directory into the list [`import_vault`] takes.
///
/// **Dot-directories are skipped** — `.obsidian` is the app's own config
/// (workspace layout, enabled plugins, theme), `.trash` is deleted notes and
/// `.git` is not vault content at all. None of it is a note, and importing the
/// workspace layout of another program as cards is noise nobody asked for.
///
/// Files come back sorted by path, so an import of the same vault twice produces
/// the same tree in the same order.
pub fn read_vault(root: &Path) -> std::io::Result<Vec<VaultFile>> {
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<VaultFile>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out)?;
        } else if let Ok(bytes) = std::fs::read(&path) {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            out.push(VaultFile {
                path: rel.to_string_lossy().replace('\\', "/"),
                bytes,
            });
        }
    }
    Ok(())
}

/// The folder part of a `/`-separated path, or `""` at the vault root.
fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// The file name part of a `/`-separated path.
fn file_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, f)| f).unwrap_or(path)
}

/// The file name with its extension removed — an Obsidian note's **name**, which
/// is its identity: `title:` in the frontmatter is just another field.
fn stem_of(path: &str) -> &str {
    let f = file_of(path);
    f.rsplit_once('.').map(|(s, _)| s).unwrap_or(f)
}

fn ext_of(path: &str) -> String {
    file_of(path).rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// Cards are laid out in a grid rather than a column: a vault folder holding
/// forty notes is one basket, and forty cards stacked vertically is a scroll
/// nobody reads.
const COLS: usize = 4;
const CARD_W: f32 = 300.0;
const CARD_H: f32 = 200.0;
const GAP: f32 = 40.0;

fn grid_pos(i: usize) -> egui::Pos2 {
    let col = i % COLS;
    let row = i / COLS;
    egui::pos2(
        GAP + col as f32 * (CARD_W + GAP),
        GAP + row as f32 * (CARD_H + GAP),
    )
}

/// Import a whole vault under `parent`, as a basket named `vault_name`.
///
/// Three passes, and the order is what makes the links work:
/// 1. **Baskets** for every folder, so a card always has somewhere to go.
/// 2. **Notes** become cards — every card exists, and therefore has an id, before
///    any link is looked at.
/// 3. **Attachments and links**, both of which need to name a card by id.
pub fn import_vault(
    doc: &mut Document,
    parent: Option<NodeId>,
    vault_name: &str,
    files: Vec<VaultFile>,
) -> VaultReport {
    let root = doc.add_node(parent, vault_name.to_string());
    let mut report = VaultReport { root, baskets: 1, ..Default::default() };

    // ---- pass 1: a basket per folder -------------------------------------
    // `BTreeMap` so parents are created before their children ("a" sorts before
    // "a/b") and the whole thing is deterministic.
    let mut baskets: BTreeMap<String, NodeId> = BTreeMap::new();
    baskets.insert(String::new(), root);
    let mut dirs: Vec<&str> = files.iter().map(|f| dir_of(&f.path)).collect();
    dirs.sort_unstable();
    dirs.dedup();
    for dir in dirs {
        if dir.is_empty() {
            continue;
        }
        // Every ancestor, so `a/b/c` also creates `a` and `a/b`.
        let mut built = String::new();
        for part in dir.split('/') {
            let parent_id = *baskets.get(&built).unwrap_or(&root);
            if !built.is_empty() {
                built.push('/');
            }
            built.push_str(part);
            if !baskets.contains_key(&built) {
                let id = doc.add_node(Some(parent_id), part.to_string());
                baskets.insert(built.clone(), id);
                report.baskets += 1;
            }
        }
    }

    // ---- pass 2: a card per note -----------------------------------------
    // Two keys per note, because Obsidian links by the **shortest unambiguous**
    // form: `[[Kestrel Overview]]` and `[[Projects/Kestrel/Kestrel Overview]]`
    // name the same note. Lowercased, as Obsidian's own matching is
    // case-insensitive.
    let mut by_name: BTreeMap<String, (NodeId, crate::model::CardId)> = BTreeMap::new();
    let mut by_stem: BTreeMap<String, Vec<(NodeId, crate::model::CardId)>> = BTreeMap::new();
    let mut per_basket: BTreeMap<NodeId, usize> = BTreeMap::new();
    let mut notes: Vec<(String, NodeId, crate::model::CardId)> = Vec::new();

    for f in files.iter().filter(|f| ext_of(&f.path) == "md") {
        let Ok(text) = String::from_utf8(f.bytes.clone()) else { continue };
        let node = *baskets.get(dir_of(&f.path)).unwrap_or(&root);
        let i = per_basket.entry(node).or_insert(0);
        let pos = grid_pos(*i);
        *i += 1;
        let Some(cid) = doc.add_card(node, pos, CardKind::Text) else { continue };

        let (fields, rest) = crate::model::split_frontmatter(&text);
        // `title:` becomes the card's title, so it must not also become a
        // `title::` property — the round trip would grow one copy per cycle.
        let carried: Vec<(String, String)> = fields
            .iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("title"))
            .cloned()
            .collect();
        let front = crate::model::frontmatter_to_trellis(&carried);
        let titled = fields
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("title"))
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let title = titled.unwrap_or_else(|| stem_of(&f.path).to_string());

        if let Some(c) = doc.card_mut(node, cid) {
            c.title = title;
            c.body = if front.is_empty() { rest.to_string() } else { format!("{front}\n{rest}") };
            c.size = egui::vec2(CARD_W, CARD_H);
            c.editing = false;
        }
        report.cards += 1;

        let full = f.path.trim_end_matches(".md").to_lowercase();
        by_name.insert(full, (node, cid));
        by_stem.entry(stem_of(&f.path).to_lowercase()).or_default().push((node, cid));
        notes.push((f.path.clone(), node, cid));
    }

    // The **bare** name only resolves when it is unambiguous. Two notes called
    // `Index.md` in different folders make `[[Index]]` undecidable, and picking
    // one would point half the vault's links at the wrong card while looking
    // like it worked. Those stay unresolved and are reported, which is the
    // answer the reader can act on. The full-path key still reaches both.
    for (stem, hits) in by_stem {
        if hits.len() == 1 && !by_name.contains_key(&stem) {
            by_name.insert(stem, hits[0]);
        }
    }

    // ---- pass 3a: attachments --------------------------------------------
    // A file goes **on the card that references it** — "the spec belongs to this
    // task" is the whole reason a card can carry one. An unreferenced file still
    // comes in, as its own card, because dropping it would lose bytes nobody can
    // get back from the document afterwards.
    for f in files.iter().filter(|f| !matches!(ext_of(&f.path).as_str(), "md" | "canvas")) {
        let name = file_of(&f.path);
        let owner = notes.iter().find(|(path, node, cid)| {
            let _ = path;
            doc.card(*node, *cid).is_some_and(|c| references(&c.body, name, &f.path))
        });
        match owner {
            Some(&(_, node, cid)) => {
                doc.add_attachment(node, cid, f.bytes.clone(), name.to_string());
            }
            None => {
                let node = *baskets.get(dir_of(&f.path)).unwrap_or(&root);
                let i = per_basket.entry(node).or_insert(0);
                let pos = grid_pos(*i);
                *i += 1;
                if let Some(cid) = doc.add_card(node, pos, CardKind::Text) {
                    if let Some(c) = doc.card_mut(node, cid) {
                        c.title = name.to_string();
                        c.size = egui::vec2(CARD_W, CARD_H);
                        c.editing = false;
                    }
                    doc.add_attachment(node, cid, f.bytes.clone(), name.to_string());
                }
            }
        }
        report.attachments += 1;
    }

    // ---- pass 3b: rewrite note links to card links ------------------------
    // Every non-note file in the vault, by bare name and by full path. A
    // `![[spec.pdf]]` is an **attachment reference**, not a note link: it already
    // became an attachment in pass 3a, so reporting it as a dangling note link
    // is a false alarm on the one signal the reader is meant to act on.
    let assets: std::collections::BTreeSet<String> = files
        .iter()
        .filter(|f| !matches!(ext_of(&f.path).as_str(), "md" | "canvas"))
        .flat_map(|f| [file_of(&f.path).to_lowercase(), f.path.to_lowercase()])
        .collect();
    let mut unresolved: Vec<String> = Vec::new();
    for &(ref path, node, cid) in &notes {
        let Some(card) = doc.card(node, cid) else { continue };
        let (body, n, mut missing) = rewrite_links(&card.body, &by_name, &assets, dir_of(path));
        if let Some(c) = doc.card_mut(node, cid) {
            c.body = body;
        }
        report.links_rewritten += n;
        unresolved.append(&mut missing);
    }
    report.unresolved = unresolved;

    // ---- pass 4: canvases become baskets ----------------------------------
    // Last, because a `file` node links to the card its note already became, and
    // that index only exists after pass 2.
    let bytes_by_name: BTreeMap<String, Vec<u8>> = files
        .iter()
        .filter(|f| !matches!(ext_of(&f.path).as_str(), "md" | "canvas"))
        .flat_map(|f| {
            [
                (f.path.to_lowercase(), f.bytes.clone()),
                (file_of(&f.path).to_lowercase(), f.bytes.clone()),
            ]
        })
        .collect();
    for f in files.iter().filter(|f| ext_of(&f.path) == "canvas") {
        let Ok(json) = std::str::from_utf8(&f.bytes) else { continue };
        let parent = *baskets.get(dir_of(&f.path)).unwrap_or(&root);
        if !import_canvas(
            doc,
            parent,
            stem_of(&f.path),
            json,
            &by_name,
            &bytes_by_name,
            &assets,
            dir_of(&f.path),
            &mut report,
        ) {
            // Not readable as a canvas: keep the bytes rather than lose them.
            let i = per_basket.entry(parent).or_insert(0);
            let pos = grid_pos(*i);
            *i += 1;
            if let Some(cid) = doc.add_card(parent, pos, CardKind::Text) {
                if let Some(c) = doc.card_mut(parent, cid) {
                    c.title = file_of(&f.path).to_string();
                    c.size = egui::vec2(CARD_W, CARD_H);
                    c.editing = false;
                }
                doc.add_attachment(parent, cid, f.bytes.clone(), file_of(&f.path).to_string());
                report.attachments += 1;
            }
        }
    }
    // Sorted and deduped here rather than before pass 4: a canvas text node can
    // name a missing note too, and the same name reported twice is noise.
    report.unresolved.sort();
    report.unresolved.dedup();
    report
}

/// Resolve a link target written **relative to `from_dir`** into a vault-relative
/// path. `./` is dropped and `../` climbs one folder; a target that climbs past
/// the vault root, or that has no relative part at all, comes back unchanged so
/// the vault-relative and bare-name lookups still get their chance at it.
fn normalise(target: &str, from_dir: &str) -> String {
    if !target.starts_with("./") && !target.starts_with("../") {
        return target.to_string();
    }
    let mut parts: Vec<&str> = if from_dir.is_empty() {
        Vec::new()
    } else {
        from_dir.split('/').collect()
    };
    let mut rest = target;
    loop {
        if let Some(r) = rest.strip_prefix("./") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("../") {
            if parts.pop().is_none() {
                // Climbed out of the vault: not a path we can name.
                return target.to_string();
            }
            rest = r;
        } else {
            break;
        }
    }
    parts.push(rest);
    parts.join("/")
}

/// Does this note body reference `name`? Both the `[[file.pdf]]` form Obsidian
/// writes and the `](path)` Markdown form count.
fn references(body: &str, name: &str, path: &str) -> bool {
    let hay = body.to_lowercase();
    let n = name.to_lowercase();
    let p = path.to_lowercase();
    hay.contains(&format!("[[{n}]]"))
        || hay.contains(&format!("[[{p}]]"))
        || hay.contains(&format!("({n})"))
        || hay.contains(&format!("({p})"))
}

/// Turn every `[[Note]]` naming an imported note into `[[#id|Note]]`.
///
/// Returns the new text, how many were rewritten, and the targets that named
/// nothing. **A target that resolves to no note is left exactly as written**: it
/// is a link the author can still read and fix, where a deleted one is content
/// silently thrown away.
///
/// `![[…]]` embeds are rewritten too — the leading `!` is preserved, and the
/// link at least reaches the card whose content it wanted to inline.
fn rewrite_links(
    text: &str,
    by_name: &BTreeMap<String, (NodeId, crate::model::CardId)>,
    assets: &std::collections::BTreeSet<String>,
    from_dir: &str,
) -> (String, usize, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut rewritten = 0usize;
    let mut missing = Vec::new();
    let b = text.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'[' && i + 1 < b.len() && b[i + 1] == b'[' {
            if let Some(end) = text[i + 2..].find("]]") {
                let inner = &text[i + 2..i + 2 + end];
                let (target, label) = match inner.split_once('|') {
                    Some((t, l)) => (t.trim(), Some(l.trim().to_string())),
                    None => (inner.trim(), None),
                };
                // `Note#Heading` and `Note#^block` address a piece of a note.
                // Trellis has no sub-card address, so the card is the answer and
                // the label keeps what the author actually wrote.
                let base = target.split('#').next().unwrap_or(target).trim();
                if base.is_empty() {
                    // `[[#Heading]]` — a link inside the same note. Not ours.
                    out.push_str(&text[i..i + 2 + end + 2]);
                    i += 2 + end + 2;
                    continue;
                }
                // `../Reference/Glossary` is relative to the **linking note's
                // folder**. Obsidian usually writes vault-relative links, but it
                // accepts what the author typed, so a hand-written one must not
                // be reported as missing when the note is right there.
                let key = normalise(base, from_dir).trim_end_matches(".md").to_lowercase();
                match by_name.get(&key) {
                    Some((_, cid)) => {
                        let shown = label.unwrap_or_else(|| target.to_string());
                        out.push_str(&format!("[[#{cid}|{shown}]]"));
                        rewritten += 1;
                    }
                    None if assets.contains(&normalise(base, from_dir).to_lowercase())
                        || assets.contains(&base.to_lowercase()) =>
                    {
                        // An attachment reference. It already rode in as bytes on
                        // this card, so the text stays as the author wrote it and
                        // nothing is reported: there is nothing to fix.
                        out.push_str(&text[i..i + 2 + end + 2]);
                    }
                    None => {
                        missing.push(base.to_string());
                        out.push_str(&text[i..i + 2 + end + 2]);
                    }
                }
                i += 2 + end + 2;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap_or('\u{0}');
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, rewritten, missing)
}


// ---------------------------------------------------------------------------
// JSON Canvas — the spatial half of a vault
// ---------------------------------------------------------------------------

/// An Obsidian **canvas** (`.canvas`), which is a Trellis basket in all but name.
///
/// This is the one file in a vault whose shape Trellis already has: nodes with
/// `x`, `y`, `width`, `height`, holding text or pointing at a note, arranged in
/// space and boxed into labelled groups. Importing it as *bytes on a card* — the
/// answer every other non-Markdown file gets — would take the only genuinely
/// spatial thing in the vault and make it unreadable.
///
/// [JSON Canvas](https://jsoncanvas.org) is an open format, and the mapping is
/// almost an identity:
///
/// | canvas | Trellis |
/// |---|---|
/// | the file | a **basket** |
/// | `text` node | a text card at the same place and size |
/// | `file` node | a card **linking** to the imported note |
/// | `link` node | a card holding the URL |
/// | `group` node | a card **group**, from what falls inside its rectangle |
/// | `edge` | a `[[#id]]` link in the card the arrow leaves |
///
/// **Unknown fields are ignored, deliberately.** Strictness is this project's
/// rule for API *input*; a document written by another program is *reading*, and
/// a canvas from a newer Obsidian must still open.
#[derive(serde::Deserialize)]
struct CanvasFile {
    #[serde(default)]
    nodes: Vec<CanvasNode>,
    #[serde(default)]
    edges: Vec<CanvasEdge>,
}

#[derive(serde::Deserialize)]
struct CanvasNode {
    id: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    #[serde(default)]
    width: f32,
    #[serde(default)]
    height: f32,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    subpath: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    color: Option<String>,
}

#[derive(serde::Deserialize)]
struct CanvasEdge {
    #[serde(rename = "fromNode")]
    from: String,
    #[serde(rename = "toNode")]
    to: String,
    #[serde(default)]
    label: Option<String>,
}

/// Obsidian's six preset colours, which a canvas stores as `"1"`–`"6"`; anything
/// else is a `#rrggbb`. An unreadable value means "no colour set", not an error —
/// a canvas that opens with the wrong border beats one that does not open.
fn canvas_color(c: &str) -> Option<[u8; 3]> {
    match c {
        "1" => Some([0xe9, 0x58, 0x4f]),
        "2" => Some([0xe8, 0x96, 0x40]),
        "3" => Some([0xe0, 0xc1, 0x46]),
        "4" => Some([0x68, 0xb6, 0x69]),
        "5" => Some([0x53, 0xb6, 0xd1]),
        "6" => Some([0xa0, 0x74, 0xc4]),
        _ => {
            let h = c.strip_prefix('#')?;
            if h.len() != 6 {
                return None;
            }
            Some([
                u8::from_str_radix(&h[0..2], 16).ok()?,
                u8::from_str_radix(&h[2..4], 16).ok()?,
                u8::from_str_radix(&h[4..6], 16).ok()?,
            ])
        }
    }
}

/// Import one `.canvas` as a basket under `parent`.
///
/// `by_name` is the note index built in pass 2, so a `file` node can link to the
/// card that note already became rather than duplicating its text.
#[allow(clippy::too_many_arguments)]
fn import_canvas(
    doc: &mut Document,
    parent: NodeId,
    title: &str,
    json: &str,
    by_name: &BTreeMap<String, (NodeId, crate::model::CardId)>,
    assets: &BTreeMap<String, Vec<u8>>,
    asset_names: &std::collections::BTreeSet<String>,
    from_dir: &str,
    report: &mut VaultReport,
) -> bool {
    let Ok(canvas) = serde_json::from_str::<CanvasFile>(json) else { return false };
    let node = doc.add_node(Some(parent), title.to_string());
    report.baskets += 1;

    // Canvas coordinates are centred on wherever the author happened to work and
    // are freely negative; a basket's origin is its top-left. Shift the whole
    // arrangement so it lands on screen **without changing any relative
    // position** — the layout is the content here.
    let min_x = canvas.nodes.iter().map(|n| n.x).fold(f32::INFINITY, f32::min);
    let min_y = canvas.nodes.iter().map(|n| n.y).fold(f32::INFINITY, f32::min);
    let (dx, dy) = if min_x.is_finite() { (GAP - min_x, GAP - min_y) } else { (0.0, 0.0) };

    let mut by_id: BTreeMap<String, crate::model::CardId> = BTreeMap::new();
    let mut groups: Vec<&CanvasNode> = Vec::new();

    for n in &canvas.nodes {
        if n.kind == "group" {
            groups.push(n);
            continue;
        }
        let pos = egui::pos2(n.x + dx, n.y + dy);
        let (title, body, kind) = match n.kind.as_str() {
            "file" => {
                let target = n.file.clone().unwrap_or_default();
                let name = stem_of(&target).to_string();
                let key = target.trim_end_matches(".md").to_lowercase();
                if let Some((_, cid)) = by_name.get(&key) {
                    // The note is already a card. Point at it rather than
                    // copying its text: two cards saying the same thing is the
                    // duplication the whole task model exists to prevent.
                    let sub = n.subpath.clone().unwrap_or_default();
                    (name.clone(), format!("[[#{cid}|{name}{sub}]]"), CardKind::Text)
                } else if let Some(bytes) = assets.get(&target.to_lowercase()) {
                    let file_name = file_of(&target).to_string();
                    let is_image = matches!(
                        ext_of(&target).as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp"
                    );
                    let cid = doc.add_card(node, pos, CardKind::Text);
                    if let Some(cid) = cid {
                        if is_image {
                            doc.add_image(node, cid, bytes.clone(), file_name.clone());
                        } else {
                            doc.add_attachment(node, cid, bytes.clone(), file_name.clone());
                            report.attachments += 1;
                        }
                        if let Some(c) = doc.card_mut(node, cid) {
                            c.title = file_name;
                            c.pos = pos;
                            c.size = egui::vec2(n.width.max(80.0), n.height.max(60.0));
                            c.editing = false;
                            if let Some(rgb) = n.color.as_deref().and_then(canvas_color) {
                                c.color = rgb;
                            }
                        }
                        by_id.insert(n.id.clone(), cid);
                        report.cards += 1;
                    }
                    continue;
                } else {
                    // A node pointing at a file the vault does not contain. Say
                    // so on the card rather than dropping it.
                    (name, format!("_missing: {target}_"), CardKind::Text)
                }
            }
            "link" => {
                let url = n.url.clone().unwrap_or_default();
                (url.clone(), format!("[{url}]({url})"), CardKind::Text)
            }
            // "text", and anything a newer Obsidian invents: the text is the
            // content, and a card that shows it is better than one that does not.
            //
            // A canvas text node is **Markdown**, so it can hold `[[Note]]` links
            // exactly as a note can — and they need the same rewrite, or the
            // spatial half of the vault imports with every link dead.
            _ => {
                let text = n.text.clone().unwrap_or_default();
                let (text, rewritten, mut missing) =
                    rewrite_links(&text, by_name, asset_names, from_dir);
                report.links_rewritten += rewritten;
                report.unresolved.append(&mut missing);
                (String::new(), text, CardKind::Text)
            }
        };
        let Some(cid) = doc.add_card(node, pos, kind) else { continue };
        if let Some(c) = doc.card_mut(node, cid) {
            c.title = title;
            c.body = body;
            c.pos = pos;
            c.size = egui::vec2(n.width.max(80.0), n.height.max(60.0));
            c.editing = false;
            if let Some(rgb) = n.color.as_deref().and_then(canvas_color) {
                c.color = rgb;
            }
        }
        by_id.insert(n.id.clone(), cid);
        report.cards += 1;
    }

    // ---- groups -----------------------------------------------------------
    // A canvas group is a **rectangle**: membership is "what is inside it", not a
    // list. A Trellis group is a list, so it is read off the geometry — by each
    // card's centre, so a card overlapping an edge belongs to whichever group it
    // is mostly in rather than to both.
    for g in groups {
        let inside: Vec<crate::model::CardId> = canvas
            .nodes
            .iter()
            .filter(|n| n.kind != "group")
            .filter(|n| {
                let cx = n.x + n.width / 2.0;
                let cy = n.y + n.height / 2.0;
                cx >= g.x && cx <= g.x + g.width && cy >= g.y && cy <= g.y + g.height
            })
            .filter_map(|n| by_id.get(&n.id).copied())
            .collect();
        // `group_cards` needs two: a box drawn round a single card is a label,
        // and Trellis has no one-card group to make of it.
        if inside.len() < 2 {
            continue;
        }
        let label = g.label.clone().unwrap_or_default();
        if let Some(gid) = doc.group_cards(node, &inside, label) {
            if let Some(rgb) = g.color.as_deref().and_then(canvas_color) {
                doc.set_group_color(node, gid, rgb);
            }
        }
    }

    // ---- edges ------------------------------------------------------------
    // Trellis has no edge. An arrow between two cards *is* a link with a
    // direction and sometimes a label, so it becomes one — which also puts it in
    // the backlink index and the link graph, where an edge drawn on a canvas was
    // never visible at all.
    for e in &canvas.edges {
        let (Some(&from), Some(&to)) = (by_id.get(&e.from), by_id.get(&e.to)) else { continue };
        let label = e.label.clone().unwrap_or_default();
        let line = if label.is_empty() {
            format!("→ [[#{to}]]")
        } else {
            format!("→ [[#{to}|{label}]]")
        };
        if let Some(c) = doc.card_mut(node, from) {
            if !c.body.is_empty() && !c.body.ends_with('\n') {
                c.body.push('\n');
            }
            c.body.push_str(&line);
            c.body.push('\n');
        }
        report.links_rewritten += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, text: &str) -> VaultFile {
        VaultFile { path: path.into(), bytes: text.as_bytes().to_vec() }
    }

    /// The whole mapping in one pass: folders become nested baskets, notes become
    /// cards with their frontmatter parsed, and a `[[Note]]` link becomes a card
    /// link that actually resolves.
    #[test]
    fn a_vault_becomes_a_tree_of_baskets_and_cards() {
        let mut doc = Document::empty();
        let report = import_vault(
            &mut doc,
            None,
            "TestVault",
            vec![
                f("Welcome.md", "---\ntags: [meta]\nstatus: draft\n---\n\nStart at [[Kestrel Overview]].\n"),
                f("Projects/Kestrel/Kestrel Overview.md", "---\ndue: 2026-09-15\n---\n\n# Scope\nBack to [[Welcome]].\n"),
                f("Reference/Glossary.md", "No frontmatter here.\n"),
            ],
        );
        assert_eq!(report.cards, 3);
        // root + Projects + Projects/Kestrel + Reference
        assert_eq!(report.baskets, 4);
        assert!(report.unresolved.is_empty(), "every link found its note: {:?}", report.unresolved);
        assert_eq!(report.links_rewritten, 2);

        // The nesting is real, not flattened.
        let root = report.root;
        let projects = doc
            .nodes
            .values()
            .find(|n| n.title == "Projects" && n.parent == Some(root))
            .expect("Projects basket");
        assert!(doc.nodes.values().any(|n| n.title == "Kestrel" && n.parent == Some(projects.id)));

        // Frontmatter came across as the lines Trellis reads, and `title:` was
        // not one of the fields here so the **file name** is the title.
        let welcome = doc
            .nodes
            .values()
            .flat_map(|n| n.cards.iter())
            .find(|c| c.title == "Welcome")
            .expect("Welcome card");
        assert!(welcome.body.contains("#meta"), "tags became tags: {}", welcome.body);
        assert!(welcome.body.contains("status:: draft"), "fields became properties: {}", welcome.body);

        // And the link resolves to a card, which a bare `[[Title]]` never would:
        // that form names a *basket*.
        let kestrel = doc
            .nodes
            .values()
            .flat_map(|n| n.cards.iter())
            .find(|c| c.title == "Kestrel Overview")
            .expect("Kestrel card");
        assert!(
            welcome.body.contains(&format!("[[#{}|Kestrel Overview]]", kestrel.id)),
            "note link became a card link: {}",
            welcome.body
        );
        assert!(matches!(
            doc.resolve_link_target(&format!("#{}", kestrel.id)),
            Some(crate::model::LinkTarget::Card { .. })
        ));
    }

    /// An attachment rides on the card that referenced it — and one nobody
    /// referenced still comes in rather than being dropped.
    #[test]
    fn attachments_land_on_the_card_that_names_them() {
        let mut doc = Document::empty();
        let report = import_vault(
            &mut doc,
            None,
            "V",
            vec![
                f("Note.md", "The spec: ![[spec.pdf]]\n"),
                VaultFile { path: "Attachments/spec.pdf".into(), bytes: b"%PDF-1.4 invented".to_vec() },
                VaultFile { path: "Attachments/orphan.bin".into(), bytes: b"\x00\x01\x02".to_vec() },
            ],
        );
        assert_eq!(report.attachments, 2);
        let note = doc
            .nodes
            .values()
            .flat_map(|n| n.cards.iter())
            .find(|c| c.title == "Note")
            .expect("Note card");
        assert_eq!(note.attachments.len(), 1, "the referencing card carries the file");
        assert_eq!(note.attachments[0].name, "spec.pdf");
        // The unreferenced one is a card of its own — bytes nobody can recover
        // later are not something an import may quietly discard.
        let orphan = doc
            .nodes
            .values()
            .flat_map(|n| n.cards.iter())
            .find(|c| c.title == "orphan.bin")
            .expect("orphan card");
        assert_eq!(orphan.attachments.len(), 1);
    }

    /// A link naming no note is **left as written**, and reported.
    #[test]
    fn a_dangling_link_is_kept_and_reported() {
        let mut doc = Document::empty();
        let report = import_vault(&mut doc, None, "V", vec![f("A.md", "see [[Nowhere]] and [[A]]\n")]);
        assert_eq!(report.unresolved, vec!["Nowhere".to_string()]);
        let a = doc.nodes.values().flat_map(|n| n.cards.iter()).next().unwrap();
        assert!(a.body.contains("[[Nowhere]]"), "kept verbatim: {}", a.body);
        assert!(a.body.contains(&format!("[[#{}|A]]", a.id)));
    }

    /// The forms Obsidian writes that are not a plain name: an alias pipe, a
    /// heading subpath, a block subpath, and a same-note heading link.
    #[test]
    fn link_forms_obsidian_writes_are_all_handled() {
        let mut doc = Document::empty();
        import_vault(
            &mut doc,
            None,
            "V",
            vec![
                f("Hub.md", "[[Target|an alias]] [[Target#Scope]] [[Target#^blk]] [[#OwnHeading]]\n"),
                f("Target.md", "x\n"),
            ],
        );
        let cards: Vec<_> = doc.nodes.values().flat_map(|n| n.cards.iter()).collect();
        let target = cards.iter().find(|c| c.title == "Target").unwrap();
        let hub = cards.iter().find(|c| c.title == "Hub").unwrap();
        let id = target.id;
        // The pipe label the author wrote survives.
        assert!(hub.body.contains(&format!("[[#{id}|an alias]]")), "{}", hub.body);
        // A subpath keeps what was written as the label — there is no sub-card
        // address to point at, but the reader still sees which section was meant.
        assert!(hub.body.contains(&format!("[[#{id}|Target#Scope]]")), "{}", hub.body);
        assert!(hub.body.contains(&format!("[[#{id}|Target#^blk]]")), "{}", hub.body);
        // A link with no note part addresses this note; not ours to rewrite.
        assert!(hub.body.contains("[[#OwnHeading]]"), "{}", hub.body);
    }

    /// An `![[spec.pdf]]` embed is an **attachment reference**, not a dangling
    /// note link. Reporting it as unresolved is a false alarm on the one signal
    /// the reader is meant to act on — found by importing a real vault, where
    /// two of the three "missing" links were files that had imported perfectly.
    #[test]
    fn an_attachment_reference_is_not_a_missing_note() {
        let mut doc = Document::empty();
        let report = import_vault(
            &mut doc,
            None,
            "V",
            vec![
                f("Note.md", "![[diagram.png]] and ![[Attachments/spec.pdf]] and [[Ghost]]\n"),
                VaultFile { path: "Attachments/diagram.png".into(), bytes: vec![1, 2, 3] },
                VaultFile { path: "Attachments/spec.pdf".into(), bytes: vec![4, 5, 6] },
            ],
        );
        assert_eq!(
            report.unresolved,
            vec!["Ghost".to_string()],
            "only the genuinely missing note is reported"
        );
        let note = doc.nodes.values().flat_map(|n| n.cards.iter()).find(|c| c.title == "Note").unwrap();
        assert_eq!(note.attachments.len(), 2, "both files rode in on the card that named them");
    }

    /// A link written **relative to the linking note's folder** resolves. Obsidian
    /// normally writes vault-relative links but accepts what the author typed.
    #[test]
    fn a_relative_link_resolves_against_the_linking_note() {
        let mut doc = Document::empty();
        let report = import_vault(
            &mut doc,
            None,
            "V",
            vec![
                f("Projects/Kestrel/Overview.md", "see [[../../Reference/Glossary]]\n"),
                f("Reference/Glossary.md", "g\n"),
            ],
        );
        assert!(report.unresolved.is_empty(), "resolved: {:?}", report.unresolved);
        assert_eq!(report.links_rewritten, 1);
    }

    /// Climbing past the vault root names nothing, and must not panic or invent a
    /// path — it comes back as written and is reported.
    #[test]
    fn a_relative_link_that_escapes_the_vault_is_reported_not_invented() {
        assert_eq!(normalise("../../x", ""), "../../x");
        assert_eq!(normalise("../Reference/G", "Projects/Kestrel"), "Projects/Reference/G");
        assert_eq!(normalise("./Sibling", "Projects"), "Projects/Sibling");
        assert_eq!(normalise("Plain", "Projects"), "Plain");
    }

    /// Two notes sharing a bare name make `[[Index]]` undecidable. Picking one
    /// would point half the vault's links at the wrong card while looking like it
    /// worked, so neither wins — but the **full path** still resolves.
    #[test]
    fn an_ambiguous_bare_name_stays_unresolved_and_the_full_path_still_works() {
        let mut doc = Document::empty();
        let report = import_vault(
            &mut doc,
            None,
            "V",
            vec![
                f("A/Index.md", "a\n"),
                f("B/Index.md", "b\n"),
                f("Hub.md", "[[Index]] then [[A/Index]]\n"),
            ],
        );
        assert_eq!(report.unresolved, vec!["Index".to_string()]);
        assert_eq!(report.links_rewritten, 1, "the unambiguous full path resolved");
    }

    const CANVAS: &str = r##"{
      "nodes": [
        {"id":"n1","type":"text","x":-400,"y":-300,"width":260,"height":80,
         "text":"# Heading\nwith a [[Target]] link","color":"1"},
        {"id":"n2","type":"file","x":-60,"y":-300,"width":300,"height":200,
         "file":"Target.md"},
        {"id":"n3","type":"file","x":-400,"y":-160,"width":260,"height":140,
         "file":"pic.png"},
        {"id":"n4","type":"link","x":-60,"y":-60,"width":300,"height":160,
         "url":"https://example.invalid"},
        {"id":"n5","type":"text","x":-400,"y":40,"width":260,"height":60,
         "text":"hex","color":"#8a4fff"},
        {"id":"far","type":"text","x":9000,"y":9000,"width":100,"height":60,"text":"outside"},
        {"id":"g1","type":"group","x":-440,"y":-360,"width":700,"height":520,
         "label":"Boxed","color":"4"}
      ],
      "edges": [
        {"id":"e1","fromNode":"n1","fromSide":"right","toNode":"n2","toSide":"left","label":"why"},
        {"id":"e2","fromNode":"n3","fromSide":"bottom","toNode":"n5","toSide":"top"}
      ]
    }"##;

    /// A `.canvas` is the one file in a vault whose shape Trellis already has, so
    /// it becomes a **basket**, not bytes on a card. Every node type, the
    /// geometry, the colours, the group and the edges.
    #[test]
    fn a_canvas_becomes_a_basket_that_keeps_its_arrangement() {
        let mut doc = Document::empty();
        let report = import_vault(
            &mut doc,
            None,
            "V",
            vec![
                f("Board.canvas", CANVAS),
                f("Target.md", "the target note\n"),
                VaultFile { path: "pic.png".into(), bytes: vec![9, 9, 9] },
            ],
        );
        let board = doc
            .nodes
            .values()
            .find(|n| n.title == "Board")
            .expect("the canvas became a basket, not an attachment");
        // Six nodes minus the group, which is a group rather than a card.
        assert_eq!(board.cards.len(), 6, "one card per non-group node");

        // **Relative geometry is the content.** The whole arrangement is shifted
        // so it lands on screen, and the gaps between nodes are unchanged.
        let by_text = |needle: &str| {
            board.cards.iter().find(|c| c.body.contains(needle)).unwrap_or_else(|| panic!("{needle}"))
        };
        let n1 = by_text("# Heading");
        let n5 = by_text("hex");
        assert_eq!(n1.pos.x, n5.pos.x, "same column in the canvas, same column here");
        assert_eq!(n5.pos.y - n1.pos.y, 340.0, "the vertical gap is preserved exactly");
        assert!(n1.pos.x >= 0.0 && n1.pos.y >= 0.0, "negative canvas coords land on screen");
        assert_eq!(n1.size, egui::vec2(260.0, 80.0), "size comes across");

        // Obsidian's preset palette and a raw hex both land.
        assert_eq!(n1.color, [0xe9, 0x58, 0x4f], "preset \"1\" is red");
        assert_eq!(n5.color, [0x8a, 0x4f, 0xff], "#8a4fff parsed");

        // A link inside a canvas text node is rewritten like any other.
        // By **content**, not by title: the canvas's `file` node card is also
        // called "Target", and `doc.nodes` is a `HashMap` — searching it by title
        // picks one of the two in per-process hash order. That is the exact
        // nondeterminism v0.121.0 fixed in link resolution, and a test can have
        // it too.
        let target = doc
            .nodes
            .values()
            .flat_map(|n| n.cards.iter())
            .find(|c| c.body.contains("the target note"))
            .expect("Target note");
        assert!(n1.body.contains(&format!("[[#{}|Target]]", target.id)), "{}", n1.body);

        // An edge is a link with a direction, and a labelled one keeps its label.
        let n2 = board.cards.iter().find(|c| c.title == "Target").unwrap();
        assert!(n1.body.contains(&format!("→ [[#{}|why]]", n2.id)), "{}", n1.body);

        // A `file` node pointing at a note **links** to the card that note became
        // rather than copying its text — two cards saying the same thing is the
        // duplication the task model exists to prevent.
        assert!(n2.body.contains(&format!("[[#{}|Target]]", target.id)), "{}", n2.body);

        // A `link` node keeps its URL as something clickable.
        assert!(board.cards.iter().any(|c| c.body.contains("https://example.invalid")));

        // The group is read off the **geometry** — what falls inside the
        // rectangle — and the node far outside it is not a member.
        let g = board.groups.first().expect("a group");
        assert_eq!(g.title, "Boxed");
        assert_eq!(g.color, [0x68, 0xb6, 0x69], "preset \"4\" is green");
        let members = board.cards.iter().filter(|c| c.group == Some(g.id)).count();
        assert_eq!(members, 5, "the five inside the rectangle");
        let outside = board.cards.iter().find(|c| c.body.contains("outside")).unwrap();
        assert_eq!(outside.group, None, "the node beyond the rectangle stays out");
        assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
    }

    /// A `.canvas` that is not readable as one keeps its bytes rather than
    /// vanishing — the same rule as any other file the importer cannot map.
    #[test]
    fn an_unreadable_canvas_falls_back_to_its_bytes() {
        let mut doc = Document::empty();
        let report = import_vault(&mut doc, None, "V", vec![f("Broken.canvas", "{not json at all")]);
        assert_eq!(report.attachments, 1);
        let c = doc
            .nodes
            .values()
            .flat_map(|n| n.cards.iter())
            .find(|c| c.title == "Broken.canvas")
            .expect("kept as a card carrying the file");
        assert_eq!(c.attachments.len(), 1);
    }

    /// `title:` in the frontmatter wins over the file name, and does not also
    /// survive as a `title::` property — that is the copy-per-cycle bug the
    /// single-file drop path already guards against.
    #[test]
    fn a_title_field_names_the_card_exactly_once() {
        let mut doc = Document::empty();
        import_vault(&mut doc, None, "V", vec![f("file-name.md", "---\ntitle: Real Title\n---\n\nbody\n")]);
        let c = doc.nodes.values().flat_map(|n| n.cards.iter()).next().unwrap();
        assert_eq!(c.title, "Real Title");
        assert!(!c.body.contains("title::"), "not repeated as a property: {}", c.body);
    }
}
