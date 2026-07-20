//! One-shot importer: turn a plain-text journal into a Trellis document.
//!
//! The source is a long notes file whose entries are separated by day-divider
//! lines like `Monday 7/20/2026:` (weekday + M/D/YYYY, colon optional, typos
//! tolerated). Everything before the first divider is treated as an undated
//! reference preamble.
//!
//! Output shape:
//!   Reference            (preamble, one card per ------ block)
//!   2026                 (year)
//!     July               (month)
//!       Monday 7/20/2026 (day, one card per blank-line block)
//!   2025 …
//!
//! Reads the source read-only and writes a fresh `.ron` — it never edits input.
//!
//! Usage: import_journal [INPUT=Notes.txt] [OUTPUT=Notes.ron]

#[path = "../model.rs"]
mod model;

use model::{CardKind, Document, NodeId};
use std::collections::HashMap;
use std::fs;

// Card grid layout on each node's canvas. Sized to fit a ~900px-wide canvas
// (window minus the tree panel) two cards across without horizontal panning.
const CARD_W: f32 = 300.0;
const CARD_H: f32 = 180.0;
const STEP_X: f32 = 320.0;
const STEP_Y: f32 = 200.0;
const COLS: usize = 2;

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .unwrap_or_else(|| "Notes.txt".to_string());
    let output = args.next().unwrap_or_else(|| "Notes.ron".to_string());

    let text = fs::read_to_string(&input).unwrap_or_else(|e| {
        eprintln!("Could not read {input}: {e}");
        std::process::exit(1);
    });
    let lines: Vec<&str> = text.lines().collect();

    // Locate every day-divider line.
    let dividers: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_day_divider(l))
        .map(|(i, _)| i)
        .collect();

    let mut doc = Document::empty();

    // --- Reference: everything before the first dated entry -----------------
    let preamble_end = dividers.first().copied().unwrap_or(lines.len());
    let ref_blocks = split_blocks(&lines[..preamble_end]);
    if !ref_blocks.is_empty() {
        let node = doc.add_node(None, "Reference".to_string());
        add_cards(&mut doc, node, &ref_blocks);
    }

    // --- Collect dated days -------------------------------------------------
    struct Day {
        year: i32,
        month: u32,
        day: u32,
        title: String,
        blocks: Vec<Vec<String>>,
    }
    let mut days: Vec<Day> = Vec::new();
    for (di, &start) in dividers.iter().enumerate() {
        let end = dividers.get(di + 1).copied().unwrap_or(lines.len());
        let header = lines[start].trim();
        let (month, day, year) = match parse_date(header) {
            Some(d) => d,
            None => continue, // guarded by is_day_divider, but stay safe
        };
        let blocks = split_blocks(&lines[start + 1..end]);
        let title = header.trim_end_matches(':').trim().to_string();
        days.push(Day { year, month, day, title, blocks });
    }

    // Newest first (stable: same-date entries keep file order).
    days.sort_by(|a, b| (b.year, b.month, b.day).cmp(&(a.year, a.month, a.day)));

    // --- Build Year → Month → Day nodes ------------------------------------
    let mut year_nodes: HashMap<i32, NodeId> = HashMap::new();
    let mut month_nodes: HashMap<(i32, u32), NodeId> = HashMap::new();
    for d in &days {
        let yn = *year_nodes
            .entry(d.year)
            .or_insert_with(|| doc.add_node(None, d.year.to_string()));
        let mn = *month_nodes
            .entry((d.year, d.month))
            .or_insert_with(|| doc.add_node(Some(yn), month_name(d.month).to_string()));
        let dn = doc.add_node(Some(mn), d.title.clone());
        add_cards(&mut doc, dn, &d.blocks);
    }

    let ron = ron::ser::to_string_pretty(&doc, ron::ser::PrettyConfig::default())
        .expect("serialize document");
    fs::write(&output, ron).unwrap_or_else(|e| {
        eprintln!("Could not write {output}: {e}");
        std::process::exit(1);
    });

    // Read back through the real deserializer to prove the app can open it,
    // and report the heaviest canvas so we can spot over-fragmented days.
    let back = fs::read_to_string(&output).expect("read back output");
    let reparsed: Document = ron::from_str(&back).expect("output failed to parse as a Document");
    let total_cards: usize = reparsed.nodes.values().map(|n| n.cards.len()).sum();
    let (busiest, max_cards) = reparsed
        .nodes
        .values()
        .map(|n| (n.title.as_str(), n.cards.len()))
        .max_by_key(|(_, c)| *c)
        .unwrap_or(("", 0));

    eprintln!(
        "Imported {} days across {} years → {}",
        days.len(),
        year_nodes.len(),
        output
    );
    eprintln!(
        "Round-trip OK: {} nodes, {} cards. Busiest canvas: \"{}\" with {} cards.",
        reparsed.nodes.len(),
        total_cards,
        busiest,
        max_cards
    );
}

/// Lay out one card per block in a 3-column grid; the first line becomes the
/// card title, the whole block (with hard line breaks) the body.
fn add_cards(doc: &mut Document, node: NodeId, blocks: &[Vec<String>]) {
    for (i, block) in blocks.iter().enumerate() {
        let col = i % COLS;
        let row = i / COLS;
        let pos = egui::pos2(
            40.0 + col as f32 * STEP_X,
            40.0 + row as f32 * STEP_Y,
        );
        let Some(cid) = doc.add_card(node, pos, CardKind::Text) else { continue };
        if let Some(c) = doc.card_mut(node, cid) {
            c.title = block.first().map(|l| truncate(l, 42)).unwrap_or_default();
            // Two trailing spaces = a Markdown hard break, so each source line
            // stays on its own line instead of being reflowed into a paragraph.
            c.body = block.join("  \n");
            c.size = egui::vec2(CARD_W, CARD_H);
            c.editing = false;
        }
    }
}

/// Split a line range into blocks separated by blank or dash-only lines.
fn split_blocks(lines: &[&str]) -> Vec<Vec<String>> {
    let mut blocks = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for &line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || is_dash_line(trimmed) {
            if !cur.is_empty() {
                blocks.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(line.trim_end().to_string());
        }
    }
    if !cur.is_empty() {
        blocks.push(cur);
    }
    blocks
}

/// A day divider: line starts with a weekday (typos tolerated via 3-letter
/// prefix) and contains an M/D/YYYY date.
fn is_day_divider(line: &str) -> bool {
    let t = line.trim();
    let Some(first) = t.split_whitespace().next() else { return false };
    let f3: String = first.chars().take(3).flat_map(|c| c.to_lowercase()).collect();
    let weekday = matches!(
        f3.as_str(),
        "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun"
    );
    weekday && parse_date(t).is_some()
}

/// First `M/D/YYYY` token in the string → `(month, day, year)`.
fn parse_date(s: &str) -> Option<(u32, u32, i32)> {
    for tok in s.split_whitespace() {
        let tok = tok.trim_end_matches(':');
        let parts: Vec<&str> = tok.split('/').collect();
        if parts.len() == 3 {
            if let (Ok(m), Ok(d), Ok(y)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<i32>(),
            ) {
                if (1..=12).contains(&m) && (1..=31).contains(&d) && y > 1900 {
                    return Some((m, d, y));
                }
            }
        }
    }
    None
}

fn is_dash_line(s: &str) -> bool {
    s.len() >= 5 && s.chars().all(|c| c == '-')
}

fn month_name(m: u32) -> String {
    const NAMES: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July",
        "August", "September", "October", "November", "December",
    ];
    NAMES.get((m as usize).wrapping_sub(1)).map(|s| s.to_string()).unwrap_or_else(|| format!("Month {m}"))
}

/// Char-safe truncation with an ellipsis.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}
