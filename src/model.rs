//! Core data model for Trellis, plus all document operations, HTML export and
//! Markdown/HTML import.
//!
//! A document is a *tree of nodes* (borrowed from the outliner world) where
//! every node's body is a *basket*: a free-form 2-D surface holding draggable
//! cards. Structure lives in the tree; spatial thinking lives in the basket.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type NodeId = u64;
pub type CardId = u64;
pub type GroupId = u64;

/// Shared accent-color palette used by the card, group and node color menus.
/// Names mirror the flexible color names the agent API accepts.
pub const SWATCHES: &[(&str, [u8; 3])] = &[
    ("Red", [0xef, 0x44, 0x44]),
    ("Orange", [0xf9, 0x73, 0x16]),
    ("Amber", [0xf5, 0x9e, 0x0b]),
    ("Yellow", [0xea, 0xb3, 0x08]),
    ("Lime", [0x84, 0xcc, 0x16]),
    ("Green", [0x22, 0xc5, 0x5e]),
    ("Teal", [0x14, 0xb8, 0xa6]),
    ("Cyan", [0x06, 0xb6, 0xd4]),
    ("Blue", [0x3b, 0x82, 0xf6]),
    ("Indigo", [0x63, 0x66, 0xf1]),
    ("Violet", [0x8b, 0x5c, 0xf6]),
    ("Pink", [0xec, 0x48, 0x99]),
    ("Slate", [0x64, 0x74, 0x8b]),
    ("Stone", [0x78, 0x71, 0x6c]),
    ("White", [0xff, 0xff, 0xff]),
    ("Black", [0x1e, 0x1e, 0x1e]),
];

/// A named container that a set of cards belong to (via [`Card::group`]). Drawn
/// as a box around its members; dragging its header moves the whole group.
#[derive(Clone, Serialize, Deserialize)]
pub struct CardGroup {
    pub id: GroupId,
    pub title: String,
    pub color: [u8; 3],
}

/// One line of a checklist card.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub done: bool,
    pub text: String,
}

/// One cell of a Table card: text plus optional background / font colors.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct TableCell {
    pub text: String,
    #[serde(default)]
    pub bg: Option<[u8; 3]>,
    #[serde(default)]
    pub fg: Option<[u8; 3]>,
}

impl TableCell {
    pub fn new(text: impl Into<String>) -> Self {
        TableCell { text: text.into(), bg: None, fg: None }
    }
}

/// The grid of a Table card. `rows` is kept rectangular by the Document ops.
#[derive(Clone, Serialize, Deserialize)]
pub struct TableData {
    pub rows: Vec<Vec<TableCell>>,
    /// Per-column widths in canvas units (missing entries = default width).
    #[serde(default)]
    pub col_widths: Vec<f32>,
    /// Style the first row as a header.
    #[serde(default = "default_true")]
    pub header: bool,
}

pub const TABLE_DEFAULT_COL_W: f32 = 110.0;

impl TableData {
    /// A fresh `rows` x `cols` empty table.
    pub fn empty(rows: usize, cols: usize) -> Self {
        TableData {
            rows: vec![vec![TableCell::default(); cols]; rows],
            col_widths: Vec::new(),
            header: true,
        }
    }

    pub fn n_cols(&self) -> usize {
        self.rows.first().map(|r| r.len()).unwrap_or(0)
    }

    pub fn col_width(&self, c: usize) -> f32 {
        self.col_widths.get(c).copied().unwrap_or(TABLE_DEFAULT_COL_W)
    }

    /// Replace all contents with plain text values (import); colors reset.
    pub fn from_values(values: Vec<Vec<String>>) -> Self {
        let cols = values.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
        let mut rows: Vec<Vec<TableCell>> = values
            .into_iter()
            .map(|r| {
                let mut row: Vec<TableCell> = r.into_iter().map(TableCell::new).collect();
                row.resize(cols, TableCell::default());
                row
            })
            .collect();
        if rows.is_empty() {
            rows.push(vec![TableCell::default(); cols]);
        }
        TableData { rows, col_widths: Vec::new(), header: true }
    }

    /// The table as CSV text (used by export and the card copy button).
    pub fn to_csv(&self) -> String {
        let mut w = csv::WriterBuilder::new()
            .flexible(true)
            .from_writer(Vec::new());
        for row in &self.rows {
            let _ = w.write_record(row.iter().map(|c| c.text.as_str()));
        }
        String::from_utf8(w.into_inner().unwrap_or_default()).unwrap_or_default()
    }

    /// The table as an .xlsx file, colors included.
    pub fn to_xlsx(&self) -> Result<Vec<u8>, String> {
        use rust_xlsxwriter::{Color, Format, Workbook};
        let mut wb = Workbook::new();
        let ws = wb.add_worksheet();
        for (r, row) in self.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let mut fmt = Format::new();
                let mut styled = false;
                if let Some([rr, gg, bb]) = cell.bg {
                    fmt = fmt.set_background_color(Color::RGB(
                        ((rr as u32) << 16) | ((gg as u32) << 8) | bb as u32,
                    ));
                    styled = true;
                }
                if let Some([rr, gg, bb]) = cell.fg {
                    fmt = fmt.set_font_color(Color::RGB(
                        ((rr as u32) << 16) | ((gg as u32) << 8) | bb as u32,
                    ));
                    styled = true;
                }
                if self.header && r == 0 {
                    fmt = fmt.set_bold();
                    styled = true;
                }
                let res = if styled {
                    ws.write_with_format(r as u32, c as u16, &cell.text, &fmt)
                } else {
                    ws.write(r as u32, c as u16, &cell.text)
                };
                res.map_err(|e| e.to_string())?;
            }
        }
        wb.save_to_buffer().map_err(|e| e.to_string())
    }
}

/// Parse CSV bytes into rows of strings.
pub fn csv_to_values(bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes);
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        out.push(rec.iter().map(|s| s.to_string()).collect());
    }
    Ok(out)
}

/// Parse the first sheet of an .xlsx file into rows of strings.
pub fn xlsx_to_values(bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    use calamine::Reader;
    let mut wb = calamine::Xlsx::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let sheet = wb
        .sheet_names()
        .first()
        .cloned()
        .ok_or("workbook has no sheets")?;
    let range = wb
        .worksheet_range(&sheet)
        .map_err(|e| e.to_string())?;
    Ok(range
        .rows()
        .map(|r| r.iter().map(|c| c.to_string()).collect())
        .collect())
}

/// One additional image of an Image card. The first image lives in the
/// variant's `data`/`name` fields so pre-multi-image documents load unchanged.
#[derive(Clone, Serialize, Deserialize)]
pub struct ImageEntry {
    pub data: Vec<u8>,
    pub name: String,
}

/// A single freehand stroke on a Sketch card. `points` are in the card's local
/// logical coordinates (top-left of the drawing area = origin, zoom-independent).
#[derive(Clone, Serialize, Deserialize)]
pub struct Stroke {
    pub color: [u8; 3],
    pub width: f32,
    pub points: Vec<[f32; 2]>,
}

/// What a card holds. `Text`/`Code` use the card's `body` string; the others
/// carry their own data.
#[derive(Clone, Serialize, Deserialize)]
pub enum CardKind {
    /// `body` is CommonMark markdown, rendered live.
    Text,
    /// `body` is source code; `lang` selects syntax highlighting.
    Code { lang: String },
    Checklist { items: Vec<ChecklistItem> },
    /// A small spreadsheet: grid of cells with optional colors, CSV/XLSX
    /// import/export.
    Table { table: TableData },
    /// Image bytes embedded directly in the document for portability. `data`/
    /// `name` hold the first image; `extra` any further ones (shown as a grid).
    Image {
        data: Vec<u8>,
        name: String,
        #[serde(default)]
        extra: Vec<ImageEntry>,
        /// Text extracted from the image(s) by OCR. Hidden in the card, but
        /// included in full-text search so screenshots/scans are findable.
        #[serde(default)]
        ocr: String,
    },
    /// A freehand sketch: a list of drawn strokes.
    Sketch {
        #[serde(default)]
        strokes: Vec<Stroke>,
    },
}

impl CardKind {
    pub fn label(&self) -> &'static str {
        match self {
            CardKind::Text => "Text",
            CardKind::Code { .. } => "Code",
            CardKind::Checklist { .. } => "Checklist",
            CardKind::Table { .. } => "Table",
            CardKind::Image { .. } => "Image",
            CardKind::Sketch { .. } => "Sketch",
        }
    }

    /// All images of an Image card in display order: the primary `data`/`name`
    /// pair (when loaded), then `extra`. Empty for other kinds.
    pub fn images(&self) -> Vec<(&[u8], &str)> {
        match self {
            CardKind::Image { data, name, extra, .. } => {
                let mut v: Vec<(&[u8], &str)> = Vec::new();
                if !data.is_empty() {
                    v.push((data.as_slice(), name.as_str()));
                }
                v.extend(extra.iter().map(|e| (e.data.as_slice(), e.name.as_str())));
                v
            }
            _ => Vec::new(),
        }
    }
}

/// A single draggable, resizable card on a node's basket canvas.
#[derive(Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: CardId,
    /// Top-left position in canvas coordinates (independent of pan).
    pub pos: egui::Pos2,
    pub size: egui::Vec2,
    pub title: String,
    /// Markdown / code text. Unused by image and checklist cards.
    pub body: String,
    /// RGB accent used for the card's title bar.
    pub color: [u8; 3],
    pub kind: CardKind,
    /// Membership in a labeled group container. `None` = ungrouped.
    #[serde(default)]
    pub group: Option<GroupId>,
    /// Dock parent: this card sticks to `docked_to` and moves with it. `None` =
    /// free-floating.
    #[serde(default)]
    pub docked_to: Option<CardId>,
    /// Body font-size multiplier (1.0 = default). Applies to text/code cards.
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
    /// Runtime-only: whether the card is in edit mode. Never persisted.
    #[serde(skip)]
    pub editing: bool,
}

fn default_font_scale() -> f32 {
    1.0
}

impl Card {
    pub fn new(id: CardId, pos: egui::Pos2, kind: CardKind) -> Self {
        let editing =
            matches!(
                kind,
                CardKind::Text | CardKind::Code { .. } | CardKind::Table { .. } | CardKind::Sketch { .. }
            );
        Self {
            id,
            pos,
            size: egui::vec2(240.0, 160.0),
            title: String::new(),
            body: String::new(),
            color: [0x3b, 0x82, 0xf6],
            kind,
            group: None,
            docked_to: None,
            font_scale: 1.0,
            editing,
        }
    }

    /// A readable size derived from the card's own content, so cards created
    /// via the API/agents aren't unreadable little squares. Returns `None` for
    /// kinds we don't auto-size (`image`). Heuristic: it errs slightly large so
    /// text is never clipped, and clamps to sane bounds. Mirrors the canvas
    /// rendering constants (`TITLE_H`, padding, `TABLE_ROW_H`).
    pub fn fit_size(&self) -> Option<egui::Vec2> {
        const TITLE_H: f32 = 24.0;
        const PAD: f32 = 6.0;
        const MIN_W: f32 = 140.0;
        const MIN_H: f32 = 90.0;
        const MAX_W: f32 = 900.0;
        const MAX_H: f32 = 1400.0;
        const TEXT_WRAP_W: f32 = 560.0; // cap text width; longer paragraphs wrap

        let fs = if self.font_scale > 0.0 { self.font_scale } else { 1.0 };
        let font_px = 14.0 * fs;
        let line_h = font_px * 1.5;
        let char_w = font_px * 0.6; // generous average width for proportional text

        // Keep the title readable in the title bar: title text plus room for the
        // edit/copy buttons.
        let title_w = self.title.chars().count() as f32 * 8.0 + 96.0;

        let (content_w, content_h) = match &self.kind {
            CardKind::Image { .. } => return None,
            CardKind::Text => wrapped_extent(&self.body, char_w, line_h, TEXT_WRAP_W),
            CardKind::Code { .. } => {
                // Monospace, no wrap: fit the longest line and every line.
                let cw = font_px * 0.62;
                let longest =
                    self.body.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
                let lines = self.body.lines().count().max(1) as f32;
                (longest * cw, lines * line_h)
            }
            CardKind::Checklist { items } => {
                let longest =
                    items.iter().map(|i| i.text.chars().count()).max().unwrap_or(0) as f32;
                // checkbox + text + delete/grip controls
                let w = 26.0 + longest * char_w + 44.0;
                let rows = items.len().max(1) as f32;
                // one row per item (a touch taller than a text line) plus the
                // "+ item" control's row
                (w, rows * (line_h + 6.0) + 28.0)
            }
            CardKind::Table { table } => {
                let cols = table.rows.first().map(|r| r.len()).unwrap_or(0);
                let cols_w: f32 = (0..cols).map(|c| table.col_width(c)).sum();
                let rows = table.rows.len() as f32;
                // + row-number handle; + column-letter strip
                (20.0 + cols_w, 24.0 + rows * 24.0)
            }
            CardKind::Sketch { strokes } => {
                let mut maxx = 0.0f32;
                let mut maxy = 0.0f32;
                for s in strokes {
                    for p in &s.points {
                        maxx = maxx.max(p[0] + s.width);
                        maxy = maxy.max(p[1] + s.width);
                    }
                }
                (maxx, maxy)
            }
        };

        let w = (content_w + PAD * 2.0).max(title_w);
        let h = TITLE_H + PAD * 2.0 + content_h;
        Some(egui::vec2(w.clamp(MIN_W, MAX_W), h.clamp(MIN_H, MAX_H)))
    }
}

/// Estimate the `(width, height)` a block of text needs: width is the longest
/// line (capped at `max_w`); height accounts for wrapping at that width. Used by
/// [`Card::fit_size`].
fn wrapped_extent(text: &str, char_w: f32, line_h: f32, max_w: f32) -> (f32, f32) {
    let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
    let natural_w = (longest * char_w).max(char_w * 8.0); // never absurdly thin
    let content_w = natural_w.min(max_w);
    let cols = (content_w / char_w).max(1.0);
    let mut lines = 0.0f32;
    for line in text.lines() {
        let n = line.chars().count() as f32;
        lines += (n / cols).ceil().max(1.0);
    }
    (content_w, lines.max(1.0) * line_h)
}

/// A node in the tree. Its `cards` form the basket shown when it is selected.
#[derive(Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub title: String,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub cards: Vec<Card>,
    /// Group containers for this basket. Membership lives on [`Card::group`].
    #[serde(default)]
    pub groups: Vec<CardGroup>,
    #[serde(default = "default_true")]
    pub expanded: bool,
    /// Optional per-node tag color shown as a dot in the tree.
    #[serde(default)]
    pub color: Option<[u8; 3]>,
    /// Optional per-basket background color. `None` = the theme default
    /// (the black grid canvas).
    #[serde(default)]
    pub bg: Option<[u8; 3]>,
}

fn default_true() -> bool {
    true
}

/// Font used by the PDF/image exporters (also embedded in the PDF).
const EXPORT_FONT: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

/// One laid-out line for the PDF/image exporters. `size` is a point size; an
/// empty `text` is a vertical spacer.
struct ExportLine {
    text: String,
    size: f32,
}

/// Width of `s` in the same units as `size_px`, using the font's advances.
fn text_width(font: &ab_glyph::FontRef, size_px: f32, s: &str) -> f32 {
    use ab_glyph::{Font, PxScale, ScaleFont};
    let scaled = font.as_scaled(PxScale::from(size_px));
    let mut w = 0.0;
    let mut last = None;
    for c in s.chars() {
        let g = scaled.glyph_id(c);
        if let Some(l) = last {
            w += scaled.kern(l, g);
        }
        w += scaled.h_advance(g);
        last = Some(g);
    }
    w
}

/// Greedy word-wrap `text` to `max_w` (same units as `size_px`), preserving the
/// text's own newlines as hard breaks.
fn wrap_text(font: &ab_glyph::FontRef, size_px: f32, text: &str, max_w: f32) -> Vec<String> {
    let space = text_width(font, size_px, " ");
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        let mut cur_w = 0.0;
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let ww = text_width(font, size_px, word);
            if !cur.is_empty() && cur_w + space + ww > max_w {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0.0;
            }
            if !cur.is_empty() {
                cur.push(' ');
                cur_w += space;
            }
            cur.push_str(word);
            cur_w += ww;
        }
        lines.push(cur);
    }
    lines
}

/// Rasterize `text` onto `img` with its baseline at `baseline`, black on white.
fn draw_text(
    img: &mut image::RgbaImage,
    font: &ab_glyph::FontRef,
    size_px: f32,
    x0: f32,
    baseline: f32,
    text: &str,
) {
    use ab_glyph::{Font, PxScale, ScaleFont};
    let scale = PxScale::from(size_px);
    let scaled = font.as_scaled(scale);
    let (w, h) = (img.width(), img.height());
    let mut x = x0;
    let mut last = None;
    for c in text.chars() {
        let gid = scaled.glyph_id(c);
        if let Some(l) = last {
            x += scaled.kern(l, gid);
        }
        let glyph = gid.with_scale_and_position(scale, ab_glyph::point(x, baseline));
        if let Some(og) = font.outline_glyph(glyph) {
            let bb = og.px_bounds();
            og.draw(|gx, gy, cov| {
                let px = bb.min.x + gx as f32;
                let py = bb.min.y + gy as f32;
                if px >= 0.0 && py >= 0.0 && (px as u32) < w && (py as u32) < h {
                    let a = (cov * 255.0) as u32;
                    let p = img.get_pixel_mut(px as u32, py as u32);
                    // Black text: scale existing (white) channels down by coverage.
                    p[0] = ((p[0] as u32 * (255 - a)) / 255) as u8;
                    p[1] = ((p[1] as u32 * (255 - a)) / 255) as u8;
                    p[2] = ((p[2] as u32 * (255 - a)) / 255) as u8;
                }
            });
        }
        x += scaled.h_advance(gid);
        last = Some(gid);
    }
}

/// Drop group containers that no longer have any member cards.
fn prune_groups(n: &mut Node) {
    let used: std::collections::HashSet<GroupId> = n.cards.iter().filter_map(|c| c.group).collect();
    n.groups.retain(|g| used.contains(&g.id));
}

/// The whole document: an arena of nodes plus ordered roots and id counters.
#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub nodes: HashMap<NodeId, Node>,
    pub roots: Vec<NodeId>,
    next_node_id: NodeId,
    next_card_id: CardId,
    #[serde(default = "default_next_id")]
    next_group_id: GroupId,
}

fn default_next_id() -> GroupId {
    1
}

impl Default for Document {
    fn default() -> Self {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let root = doc.add_node(None, "Welcome to Trellis".to_string());
        if let Some(id) = doc.add_card(root, egui::pos2(60.0, 60.0), CardKind::Text) {
            if let Some(c) = doc.card_mut(root, id) {
                c.title = "Read me".to_string();
                c.body = "# The tree *and* the weave\n\nThe **left panel** is a hierarchy of \
                    nodes. Every node opens here as a **basket** — a free canvas of cards.\n\n\
                    - Double-click empty space to drop a text card\n\
                    - Right-click the canvas for other card types\n\
                    - Drag a card's title bar to move it, the corner to resize\n\n\
                    ```rust\nfn main() { println!(\"code cards are highlighted\"); }\n```"
                    .to_string();
                c.size = egui::vec2(360.0, 260.0);
                c.editing = false;
            }
        }
        doc
    }
}

impl Document {
    /// An empty document with no nodes. Unlike [`Document::default`], which seeds
    /// a welcome node, this is the blank slate importers build onto.
    pub fn empty() -> Self {
        Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        }
    }

    // --- lookup helpers -----------------------------------------------------

    pub fn card_mut(&mut self, node: NodeId, card: CardId) -> Option<&mut Card> {
        self.nodes
            .get_mut(&node)?
            .cards
            .iter_mut()
            .find(|c| c.id == card)
    }

    pub fn card(&self, node: NodeId, card: CardId) -> Option<&Card> {
        self.nodes.get(&node)?.cards.iter().find(|c| c.id == card)
    }

    /// Store OCR-extracted text on an image card. Returns false if not an image card.
    pub fn set_card_ocr(&mut self, node: NodeId, card: CardId, text: String) -> bool {
        match self.card_mut(node, card).map(|c| &mut c.kind) {
            Some(CardKind::Image { ocr, .. }) => {
                *ocr = text;
                true
            }
            _ => false,
        }
    }

    fn table_mut(&mut self, node: NodeId, card: CardId) -> Option<&mut TableData> {
        match self.card_mut(node, card).map(|c| &mut c.kind) {
            Some(CardKind::Table { table }) => Some(table),
            _ => None,
        }
    }

    pub fn table_set_cell(&mut self, node: NodeId, card: CardId, r: usize, c: usize, text: String) -> bool {
        self.table_mut(node, card)
            .and_then(|t| t.rows.get_mut(r)?.get_mut(c).map(|cell| cell.text = text))
            .is_some()
    }

    pub fn table_set_bg(&mut self, node: NodeId, card: CardId, r: usize, c: usize, bg: Option<[u8; 3]>) -> bool {
        self.table_mut(node, card)
            .and_then(|t| t.rows.get_mut(r)?.get_mut(c).map(|cell| cell.bg = bg))
            .is_some()
    }

    pub fn table_set_fg(&mut self, node: NodeId, card: CardId, r: usize, c: usize, fg: Option<[u8; 3]>) -> bool {
        self.table_mut(node, card)
            .and_then(|t| t.rows.get_mut(r)?.get_mut(c).map(|cell| cell.fg = fg))
            .is_some()
    }

    /// Insert an empty row at `at` (clamped).
    pub fn table_insert_row(&mut self, node: NodeId, card: CardId, at: usize) -> bool {
        let Some(t) = self.table_mut(node, card) else { return false };
        let cols = t.n_cols().max(1);
        let at = at.min(t.rows.len());
        t.rows.insert(at, vec![TableCell::default(); cols]);
        true
    }

    /// Remove a row (a table always keeps at least one).
    pub fn table_remove_row(&mut self, node: NodeId, card: CardId, at: usize) -> bool {
        let Some(t) = self.table_mut(node, card) else { return false };
        if t.rows.len() <= 1 || at >= t.rows.len() {
            return false;
        }
        t.rows.remove(at);
        true
    }

    /// Insert an empty column at `at` (clamped).
    pub fn table_insert_col(&mut self, node: NodeId, card: CardId, at: usize) -> bool {
        let Some(t) = self.table_mut(node, card) else { return false };
        let at = at.min(t.n_cols());
        for row in &mut t.rows {
            row.insert(at, TableCell::default());
        }
        if at < t.col_widths.len() {
            t.col_widths.insert(at, TABLE_DEFAULT_COL_W);
        }
        true
    }

    /// Remove a column (a table always keeps at least one).
    pub fn table_remove_col(&mut self, node: NodeId, card: CardId, at: usize) -> bool {
        let Some(t) = self.table_mut(node, card) else { return false };
        if t.n_cols() <= 1 || at >= t.n_cols() {
            return false;
        }
        for row in &mut t.rows {
            row.remove(at);
        }
        if at < t.col_widths.len() {
            t.col_widths.remove(at);
        }
        true
    }

    pub fn table_set_col_width(&mut self, node: NodeId, card: CardId, c: usize, w: f32) -> bool {
        let Some(t) = self.table_mut(node, card) else { return false };
        if c >= t.n_cols() {
            return false;
        }
        if t.col_widths.len() < t.n_cols() {
            let cols = t.n_cols();
            t.col_widths.resize(cols, TABLE_DEFAULT_COL_W);
        }
        t.col_widths[c] = w.clamp(28.0, 600.0);
        true
    }

    pub fn table_toggle_header(&mut self, node: NodeId, card: CardId) -> bool {
        self.table_mut(node, card)
            .map(|t| t.header = !t.header)
            .is_some()
    }

    /// Set (rather than toggle) the table's header-row flag.
    pub fn table_set_header(&mut self, node: NodeId, card: CardId, header: bool) -> bool {
        self.table_mut(node, card).map(|t| t.header = header).is_some()
    }

    /// Replace the whole table with imported plain values.
    pub fn table_replace(&mut self, node: NodeId, card: CardId, values: Vec<Vec<String>>) -> bool {
        self.table_mut(node, card)
            .map(|t| *t = TableData::from_values(values))
            .is_some()
    }

    /// Append an image to an Image card (the first load fills the primary
    /// slot). Returns false if the card isn't an Image card.
    pub fn add_image(&mut self, node: NodeId, card: CardId, bytes: Vec<u8>, img_name: String) -> bool {
        match self.card_mut(node, card).map(|c| &mut c.kind) {
            Some(CardKind::Image { data, name, extra, .. }) => {
                if data.is_empty() && extra.is_empty() {
                    *data = bytes;
                    *name = img_name;
                } else {
                    extra.push(ImageEntry { data: bytes, name: img_name });
                }
                true
            }
            _ => false,
        }
    }

    /// Remove the `idx`th image (display order) from an Image card. Removing
    /// the primary image promotes the next `extra` entry into its place.
    pub fn remove_image(&mut self, node: NodeId, card: CardId, idx: usize) -> bool {
        match self.card_mut(node, card).map(|c| &mut c.kind) {
            Some(CardKind::Image { data, name, extra, .. }) => {
                if idx == 0 && !data.is_empty() {
                    if extra.is_empty() {
                        data.clear();
                        name.clear();
                    } else {
                        let e = extra.remove(0);
                        *data = e.data;
                        *name = e.name;
                    }
                    true
                } else {
                    // Display index counts the primary image when present.
                    let base = if data.is_empty() { 0 } else { 1 };
                    let i = idx - base;
                    if i < extra.len() {
                        extra.remove(i);
                        true
                    } else {
                        false
                    }
                }
            }
            _ => false,
        }
    }

    /// The ordered sibling list a node lives in (its parent's children, or the
    /// roots for a top-level node).
    fn sibling_list_mut(&mut self, id: NodeId) -> Option<&mut Vec<NodeId>> {
        match self.nodes.get(&id)?.parent {
            Some(p) => self.nodes.get_mut(&p).map(|n| &mut n.children),
            None => Some(&mut self.roots),
        }
    }

    // --- structural edits ---------------------------------------------------

    pub fn add_node(&mut self, parent: Option<NodeId>, title: String) -> NodeId {
        let id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.insert(
            id,
            Node {
                id,
                title,
                parent,
                children: Vec::new(),
                cards: Vec::new(),
                groups: Vec::new(),
                expanded: true,
                color: None,
                bg: None,
            },
        );
        match parent {
            Some(p) => {
                if let Some(pn) = self.nodes.get_mut(&p) {
                    pn.children.push(id);
                }
            }
            None => self.roots.push(id),
        }
        id
    }

    /// Add a sibling immediately after `id`, in the same list.
    pub fn add_sibling(&mut self, id: NodeId, title: String) -> NodeId {
        let parent = self.nodes.get(&id).and_then(|n| n.parent);
        let new_id = self.next_node_id;
        self.next_node_id += 1;
        self.nodes.insert(
            new_id,
            Node {
                id: new_id,
                title,
                parent,
                children: Vec::new(),
                cards: Vec::new(),
                groups: Vec::new(),
                expanded: true,
                color: None,
                bg: None,
            },
        );
        if let Some(list) = self.sibling_list_mut(id) {
            let idx = list.iter().position(|x| *x == id).map_or(list.len(), |i| i + 1);
            list.insert(idx, new_id);
        }
        new_id
    }

    pub fn add_card(&mut self, node: NodeId, pos: egui::Pos2, kind: CardKind) -> Option<CardId> {
        let id = self.next_card_id;
        let n = self.nodes.get_mut(&node)?;
        n.cards.push(Card::new(id, pos, kind));
        self.next_card_id += 1;
        Some(id)
    }

    pub fn duplicate_card(&mut self, node: NodeId, card: CardId) -> Option<CardId> {
        let n = self.nodes.get_mut(&node)?;
        let src = n.cards.iter().find(|c| c.id == card)?.clone();
        let id = self.next_card_id;
        self.next_card_id += 1;
        let mut copy = src;
        copy.id = id;
        copy.pos += egui::vec2(24.0, 24.0);
        n.cards.push(copy);
        Some(id)
    }

    /// Add a copy of `template` (a card from anywhere) to `node`, with a fresh
    /// id and the given position. Used to paste a copied card into a basket.
    pub fn add_card_from(&mut self, node: NodeId, template: &Card, pos: egui::Pos2) -> Option<CardId> {
        let id = self.next_card_id;
        let n = self.nodes.get_mut(&node)?;
        let mut card = template.clone();
        card.id = id;
        card.pos = pos;
        card.editing = false;
        n.cards.push(card);
        self.next_card_id += 1;
        Some(id)
    }

    pub fn remove_card(&mut self, node: NodeId, card: CardId) {
        if let Some(n) = self.nodes.get_mut(&node) {
            n.cards.retain(|c| c.id != card);
            // Detach anything that was docked to the removed card.
            for c in n.cards.iter_mut() {
                if c.docked_to == Some(card) {
                    c.docked_to = None;
                }
            }
            prune_groups(n);
        }
    }

    // --- groups -------------------------------------------------------------

    /// Put `cards` (2 or more) into a fresh group and return its id. No-op
    /// (returns `None`) if fewer than two of them exist in the node.
    pub fn group_cards(
        &mut self,
        node: NodeId,
        cards: &[CardId],
        title: String,
    ) -> Option<GroupId> {
        let gid = self.next_group_id.max(1);
        let n = self.nodes.get_mut(&node)?;
        let count = n.cards.iter().filter(|c| cards.contains(&c.id)).count();
        if count < 2 {
            return None;
        }
        for c in n.cards.iter_mut() {
            if cards.contains(&c.id) {
                c.group = Some(gid);
            }
        }
        n.groups.push(CardGroup { id: gid, title, color: [0x64, 0x74, 0x8b] });
        self.next_group_id = gid + 1;
        Some(gid)
    }

    pub fn ungroup(&mut self, node: NodeId, group: GroupId) {
        if let Some(n) = self.nodes.get_mut(&node) {
            for c in n.cards.iter_mut() {
                if c.group == Some(group) {
                    c.group = None;
                }
            }
            n.groups.retain(|g| g.id != group);
        }
    }

    /// Set a card's group membership. `Some(g)` joins an existing group (the
    /// card leaves any previous one); `None` removes it from its group. Returns
    /// false if the card, or the target group, doesn't exist in the node.
    pub fn set_card_group(&mut self, node: NodeId, card: CardId, group: Option<GroupId>) -> bool {
        let Some(n) = self.nodes.get_mut(&node) else { return false };
        if let Some(g) = group {
            if !n.groups.iter().any(|grp| grp.id == g) {
                return false;
            }
        }
        match n.cards.iter_mut().find(|c| c.id == card) {
            Some(c) => {
                c.group = group;
                true
            }
            None => false,
        }
    }

    pub fn set_group_title(&mut self, node: NodeId, group: GroupId, title: String) {
        if let Some(n) = self.nodes.get_mut(&node) {
            if let Some(g) = n.groups.iter_mut().find(|g| g.id == group) {
                g.title = title;
            }
        }
    }

    pub fn set_group_color(&mut self, node: NodeId, group: GroupId, color: [u8; 3]) {
        if let Some(n) = self.nodes.get_mut(&node) {
            if let Some(g) = n.groups.iter_mut().find(|g| g.id == group) {
                g.color = color;
            }
        }
    }

    /// Move every member of `group` (and anything docked to a member) by `delta`.
    pub fn move_group(&mut self, node: NodeId, group: GroupId, delta: egui::Vec2) {
        let members: Vec<CardId> = self
            .nodes
            .get(&node)
            .map(|n| n.cards.iter().filter(|c| c.group == Some(group)).map(|c| c.id).collect())
            .unwrap_or_default();
        let mut ids: std::collections::HashSet<CardId> = std::collections::HashSet::new();
        for m in members {
            ids.extend(self.dock_tree_ids(node, m));
        }
        if let Some(n) = self.nodes.get_mut(&node) {
            for c in n.cards.iter_mut() {
                if ids.contains(&c.id) {
                    c.pos += delta;
                }
            }
        }
    }

    // --- docking ------------------------------------------------------------

    /// `card` plus every card docked to it, transitively (its dock subtree).
    fn dock_tree_ids(&self, node: NodeId, root: CardId) -> Vec<CardId> {
        let mut ids = vec![root];
        if let Some(n) = self.nodes.get(&node) {
            loop {
                let mut added = false;
                for c in &n.cards {
                    if let Some(p) = c.docked_to {
                        if ids.contains(&p) && !ids.contains(&c.id) {
                            ids.push(c.id);
                            added = true;
                        }
                    }
                }
                if !added {
                    break;
                }
            }
        }
        ids
    }

    /// Stick `child` onto `anchor`. Ignored if it would create a cycle (anchor
    /// is inside child's own dock subtree) or they're the same card.
    pub fn dock_card(&mut self, node: NodeId, child: CardId, anchor: CardId) {
        if child == anchor {
            return;
        }
        if self.dock_tree_ids(node, child).contains(&anchor) {
            return;
        }
        if let Some(c) = self.card_mut(node, child) {
            c.docked_to = Some(anchor);
        }
    }

    pub fn detach_card(&mut self, node: NodeId, card: CardId) {
        if let Some(c) = self.card_mut(node, card) {
            c.docked_to = None;
        }
    }

    /// Move `card` and its whole dock subtree by `delta`.
    pub fn move_card_tree(&mut self, node: NodeId, card: CardId, delta: egui::Vec2) {
        let ids = self.dock_tree_ids(node, card);
        if let Some(n) = self.nodes.get_mut(&node) {
            for c in n.cards.iter_mut() {
                if ids.contains(&c.id) {
                    c.pos += delta;
                }
            }
        }
    }

    /// Bring a card to the front by moving it to the end of the draw order.
    pub fn raise_card(&mut self, node: NodeId, card: CardId) {
        if let Some(n) = self.nodes.get_mut(&node) {
            if let Some(idx) = n.cards.iter().position(|c| c.id == card) {
                if idx + 1 != n.cards.len() {
                    let c = n.cards.remove(idx);
                    n.cards.push(c);
                }
            }
        }
    }

    /// Bring a whole group's member cards to the front, preserving their
    /// relative order. Used so clicking a group header raises it above the pile.
    pub fn raise_group(&mut self, node: NodeId, group: GroupId) {
        if let Some(n) = self.nodes.get_mut(&node) {
            let (mut members, others): (Vec<Card>, Vec<Card>) =
                std::mem::take(&mut n.cards).into_iter().partition(|c| c.group == Some(group));
            n.cards = others;
            n.cards.append(&mut members);
        }
    }

    /// Move a checklist item from index `from` so it lands before original index
    /// `to` (the drag-reorder convention). Returns false if not a checklist or
    /// the indices are a no-op.
    pub fn move_checklist_item(&mut self, node: NodeId, card: CardId, from: usize, to: usize) -> bool {
        match self.card_mut(node, card).map(|c| &mut c.kind) {
            Some(CardKind::Checklist { items }) if from < items.len() => {
                let mut dest = to.min(items.len());
                if dest > from {
                    dest -= 1;
                }
                if dest == from {
                    return false;
                }
                let it = items.remove(from);
                items.insert(dest.min(items.len()), it);
                true
            }
            _ => false,
        }
    }

    fn sketch_mut(&mut self, node: NodeId, card: CardId) -> Option<&mut Vec<Stroke>> {
        match self.card_mut(node, card).map(|c| &mut c.kind) {
            Some(CardKind::Sketch { strokes }) => Some(strokes),
            _ => None,
        }
    }

    /// Append a freehand stroke to a Sketch card. Empty strokes are ignored.
    pub fn sketch_add_stroke(&mut self, node: NodeId, card: CardId, stroke: Stroke) -> bool {
        if stroke.points.is_empty() {
            return false;
        }
        self.sketch_mut(node, card).map(|s| s.push(stroke)).is_some()
    }

    /// Remove the most recent stroke from a Sketch card.
    pub fn sketch_undo(&mut self, node: NodeId, card: CardId) -> bool {
        self.sketch_mut(node, card).map(|s| s.pop()).flatten().is_some()
    }

    /// Erase all strokes from a Sketch card.
    pub fn sketch_clear(&mut self, node: NodeId, card: CardId) -> bool {
        match self.sketch_mut(node, card) {
            Some(s) if !s.is_empty() => {
                s.clear();
                true
            }
            _ => false,
        }
    }

    /// Lay every card in a node out in a tidy, non-overlapping grid. Cards are
    /// clustered by group so a group stays contiguous; docking is cleared (a
    /// grid means nothing stacks). Returns false if the node is empty/missing.
    pub fn autosort(&mut self, node: NodeId) -> bool {
        let Some(n) = self.nodes.get_mut(&node) else { return false };
        let count = n.cards.len();
        if count == 0 {
            return false;
        }
        const GAP: f32 = 24.0;
        // Auto-size every card to its content first, so the tidy grid is also
        // readable (image cards keep their own size — `fit_size` returns `None`).
        for c in n.cards.iter_mut() {
            if let Some(sz) = c.fit_size() {
                c.size = sz;
            }
        }
        let cols = (count as f32).sqrt().ceil().max(1.0) as usize;
        let rows = count.div_ceil(cols);
        // Placement order: cluster grouped cards together, else keep card order.
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by_key(|&i| (n.cards[i].group.map(|g| g as i128).unwrap_or(i128::MAX), i));
        // Per-column width / per-row height = the largest card in that column /
        // row, so varied card sizes pack tightly instead of into uniform cells.
        let mut col_w = vec![0.0f32; cols];
        let mut row_h = vec![0.0f32; rows];
        for (slot, &i) in order.iter().enumerate() {
            col_w[slot % cols] = col_w[slot % cols].max(n.cards[i].size.x);
            row_h[slot / cols] = row_h[slot / cols].max(n.cards[i].size.y);
        }
        let mut col_x = vec![40.0f32; cols];
        for c in 1..cols {
            col_x[c] = col_x[c - 1] + col_w[c - 1] + GAP;
        }
        let mut row_y = vec![40.0f32; rows];
        for r in 1..rows {
            row_y[r] = row_y[r - 1] + row_h[r - 1] + GAP;
        }
        for (slot, &i) in order.iter().enumerate() {
            n.cards[i].pos = egui::pos2(col_x[slot % cols], row_y[slot / cols]);
            n.cards[i].docked_to = None;
        }
        true
    }

    /// Remove a node and its whole subtree; detaches it from its parent/roots.
    pub fn remove_node(&mut self, id: NodeId) {
        let parent = self.nodes.get(&id).and_then(|n| n.parent);
        match parent {
            Some(p) => {
                if let Some(pn) = self.nodes.get_mut(&p) {
                    pn.children.retain(|c| *c != id);
                }
            }
            None => self.roots.retain(|c| *c != id),
        }
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if let Some(n) = self.nodes.remove(&cur) {
                stack.extend(n.children);
            }
        }
    }

    /// Move a node to the top (`top`) or bottom of its sibling list.
    pub fn move_to_edge(&mut self, id: NodeId, top: bool) {
        if let Some(list) = self.sibling_list_mut(id) {
            if let Some(i) = list.iter().position(|x| *x == id) {
                let item = list.remove(i);
                if top {
                    list.insert(0, item);
                } else {
                    list.push(item);
                }
            }
        }
    }

    /// Reorder via drag & drop: place `moved` immediately before/after `target`,
    /// adopting `target`'s parent (so this also reparents across lists). No-ops
    /// if it would drop a node into its own subtree.
    pub fn reorder(&mut self, moved: NodeId, target: NodeId, before: bool) {
        if moved == target
            || !self.nodes.contains_key(&moved)
            || !self.nodes.contains_key(&target)
            || self.is_descendant(target, moved)
        {
            return;
        }
        let new_parent = self.nodes.get(&target).and_then(|n| n.parent);
        if let Some(list) = self.sibling_list_mut(moved) {
            list.retain(|x| *x != moved);
        }
        if let Some(n) = self.nodes.get_mut(&moved) {
            n.parent = new_parent;
        }
        let list = match new_parent {
            Some(p) => self.nodes.get_mut(&p).map(|n| &mut n.children),
            None => Some(&mut self.roots),
        };
        if let Some(list) = list {
            let pos = list
                .iter()
                .position(|x| *x == target)
                .map_or(list.len(), |i| if before { i } else { i + 1 });
            list.insert(pos, moved);
        }
    }

    /// Is `node` inside the subtree rooted at `ancestor`?
    fn is_descendant(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut cur = self.nodes.get(&node).and_then(|n| n.parent);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.nodes.get(&c).and_then(|n| n.parent);
        }
        false
    }

    /// Move a node one slot earlier (`up`) or later among its siblings.
    pub fn move_sibling(&mut self, id: NodeId, up: bool) {
        if let Some(list) = self.sibling_list_mut(id) {
            if let Some(i) = list.iter().position(|x| *x == id) {
                if up && i > 0 {
                    list.swap(i, i - 1);
                } else if !up && i + 1 < list.len() {
                    list.swap(i, i + 1);
                }
            }
        }
    }

    /// Make `id` a child of its previous sibling.
    pub fn indent(&mut self, id: NodeId) {
        let parent = self.nodes.get(&id).and_then(|n| n.parent);
        let list = match parent {
            Some(p) => self.nodes.get(&p).map(|n| n.children.clone()),
            None => Some(self.roots.clone()),
        };
        let Some(list) = list else { return };
        let Some(i) = list.iter().position(|x| *x == id) else { return };
        if i == 0 {
            return; // no previous sibling to adopt it
        }
        let new_parent = list[i - 1];
        // Detach from current list.
        if let Some(l) = self.sibling_list_mut(id) {
            l.retain(|x| *x != id);
        }
        // Attach under the previous sibling.
        if let Some(np) = self.nodes.get_mut(&new_parent) {
            np.children.push(id);
            np.expanded = true;
        }
        if let Some(n) = self.nodes.get_mut(&id) {
            n.parent = Some(new_parent);
        }
    }

    /// Make `id` a sibling of its parent (one level shallower).
    pub fn outdent(&mut self, id: NodeId) {
        let Some(parent) = self.nodes.get(&id).and_then(|n| n.parent) else {
            return; // already a root
        };
        let grandparent = self.nodes.get(&parent).and_then(|n| n.parent);
        // Detach from parent.
        if let Some(pn) = self.nodes.get_mut(&parent) {
            pn.children.retain(|x| *x != id);
        }
        // Insert just after the parent in the grandparent's list (or roots).
        let target: &mut Vec<NodeId> = match grandparent {
            Some(g) => match self.nodes.get_mut(&g) {
                Some(gn) => &mut gn.children,
                None => return,
            },
            None => &mut self.roots,
        };
        let idx = target
            .iter()
            .position(|x| *x == parent)
            .map_or(target.len(), |i| i + 1);
        target.insert(idx, id);
        if let Some(n) = self.nodes.get_mut(&id) {
            n.parent = grandparent;
        }
    }

    // --- import / export ----------------------------------------------------

    /// Build a standalone HTML document from the whole tree.
    pub fn export_html(&self) -> String {
        let mut s = String::new();
        s.push_str(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <title>Trellis export</title>\n<style>\n",
        );
        s.push_str(EXPORT_CSS);
        s.push_str("</style>\n</head>\n<body>\n<main>\n");
        for &r in &self.roots {
            self.export_node_html(r, 1, &mut s);
        }
        s.push_str("</main>\n</body>\n</html>\n");
        s
    }

    fn export_node_html(&self, id: NodeId, depth: usize, s: &mut String) {
        let Some(node) = self.nodes.get(&id) else { return };
        let h = depth.min(6);
        s.push_str(&format!(
            "<section class=\"node\">\n<h{h}>{}</h{h}>\n",
            escape_html(&node.title)
        ));
        for card in &node.cards {
            s.push_str("<article class=\"card\">\n");
            if !card.title.is_empty() {
                s.push_str(&format!("<h4>{}</h4>\n", escape_html(&card.title)));
            }
            s.push_str(&card_body_html(card));
            s.push_str("</article>\n");
        }
        let children = node.children.clone();
        for child in children {
            self.export_node_html(child, depth + 1, s);
        }
        s.push_str("</section>\n");
    }

    /// Serialize the whole document to pretty-printed JSON. Image cards embed
    /// their bytes as a JSON array, so exports stay self-contained.
    pub fn export_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Render the whole tree as a single Markdown document: nodes become
    /// headings (nesting = heading level), cards become their Markdown.
    pub fn export_markdown(&self) -> String {
        let mut s = String::new();
        for &r in &self.roots {
            self.export_node_md(r, 1, &mut s);
        }
        s
    }

    /// Flatten the whole document into a sequence of laid-out text lines, shared
    /// by the PDF and image exporters. Blank lines (empty text) act as spacers.
    fn export_lines(&self) -> Vec<ExportLine> {
        let mut out = Vec::new();
        for &r in &self.roots {
            self.export_node_lines(r, 1, &mut out);
        }
        out
    }

    fn export_node_lines(&self, id: NodeId, depth: usize, out: &mut Vec<ExportLine>) {
        let Some(node) = self.nodes.get(&id) else { return };
        let hsize = match depth {
            1 => 18.0,
            2 => 15.0,
            3 => 13.0,
            _ => 12.0,
        };
        out.push(ExportLine { text: node.title.clone(), size: hsize });
        out.push(ExportLine { text: String::new(), size: 6.0 });
        for card in &node.cards {
            out.extend(card_lines(card));
        }
        for c in node.children.clone() {
            self.export_node_lines(c, depth + 1, out);
        }
    }

    /// Render the whole document to a PDF (A4, paginated). Returns the file bytes.
    pub fn export_pdf(&self) -> Result<Vec<u8>, String> {
        lines_to_pdf(&self.export_lines())
    }

    /// Render the whole document to a raster image (PNG, or GIF if `gif`).
    /// Returns the encoded file bytes. One tall page, black text on white.
    pub fn export_image(&self, gif: bool) -> Result<Vec<u8>, String> {
        lines_to_image(&self.export_lines(), gif)
    }

    /// Render a single card to Markdown. `None` if the card no longer exists.
    pub fn export_card_markdown(&self, node: NodeId, card: CardId) -> Option<String> {
        let c = self.card(node, card)?;
        let mut s = String::new();
        if !c.title.is_empty() {
            s.push_str(&format!("# {}\n\n", c.title));
        }
        s.push_str(&card_body_md(c));
        Some(s)
    }

    /// Render a single card to a standalone, styled HTML document.
    pub fn export_card_html(&self, node: NodeId, card: CardId) -> Option<String> {
        let c = self.card(node, card)?;
        let mut s = String::from(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <title>Trellis card</title>\n<style>\n",
        );
        s.push_str(EXPORT_CSS);
        s.push_str("</style>\n</head>\n<body>\n<main>\n<article class=\"card\">\n");
        if !c.title.is_empty() {
            s.push_str(&format!("<h4>{}</h4>\n", escape_html(&c.title)));
        }
        s.push_str(&card_body_html(c));
        s.push_str("</article>\n</main>\n</body>\n</html>\n");
        Some(s)
    }

    /// Render a single card to plain text (title + flattened body lines).
    pub fn export_card_text(&self, node: NodeId, card: CardId) -> Option<String> {
        let c = self.card(node, card)?;
        let mut s = String::new();
        for l in card_lines(c) {
            s.push_str(&l.text);
            s.push('\n');
        }
        Some(format!("{}\n", s.trim_end()))
    }

    /// Render a sketch card to SVG. `None` for non-sketch cards.
    pub fn export_card_svg(&self, node: NodeId, card: CardId) -> Option<String> {
        let c = self.card(node, card)?;
        match &c.kind {
            CardKind::Sketch { strokes } => Some(sketch_svg(strokes, c.size.x, c.size.y)),
            _ => None,
        }
    }

    /// Serialize a single card to a portable JSON document (see [`CardExport`]).
    pub fn export_card_json(&self, node: NodeId, card: CardId) -> Option<String> {
        let c = self.card(node, card)?;
        let exp = CardExport {
            format: CARD_EXPORT_FORMAT.to_string(),
            version: 1,
            title: c.title.clone(),
            body: c.body.clone(),
            color: c.color,
            size: [c.size.x, c.size.y],
            font_scale: c.font_scale,
            kind: c.kind.clone(),
        };
        serde_json::to_string_pretty(&exp).ok()
    }

    /// Create a new card in `node` at `pos` from a parsed [`CardExport`]. The card
    /// gets a fresh id and is placed as a free-floating, view-mode card.
    pub fn add_card_from_export(
        &mut self,
        node: NodeId,
        pos: egui::Pos2,
        exp: CardExport,
    ) -> Option<CardId> {
        let cid = self.add_card(node, pos, exp.kind)?;
        if let Some(c) = self.card_mut(node, cid) {
            c.title = exp.title;
            c.body = exp.body;
            c.color = exp.color;
            c.size = egui::vec2(exp.size[0].max(40.0), exp.size[1].max(30.0));
            c.font_scale = if exp.font_scale > 0.0 { exp.font_scale } else { 1.0 };
            c.editing = false;
        }
        Some(cid)
    }

    fn export_node_md(&self, id: NodeId, depth: usize, s: &mut String) {
        let Some(node) = self.nodes.get(&id) else { return };
        s.push_str(&format!("{} {}\n\n", "#".repeat(depth.min(6)), node.title));
        for card in &node.cards {
            if !card.title.is_empty() {
                s.push_str(&format!("**{}**\n\n", card.title));
            }
            s.push_str(&card_body_md(card));
        }
        let children = node.children.clone();
        for c in children {
            self.export_node_md(c, depth + 1, s);
        }
    }

    // --- basket (single-node) export/import --------------------------------

    /// Export one basket (node) to Markdown. `with_subnodes` includes the whole
    /// subtree (child nodes become deeper headings); otherwise just this node's
    /// own cards. `None` if the node no longer exists.
    pub fn export_node_markdown(&self, node: NodeId, with_subnodes: bool) -> Option<String> {
        let n = self.nodes.get(&node)?;
        let mut s = String::new();
        if with_subnodes {
            self.export_node_md(node, 1, &mut s);
        } else {
            s.push_str(&format!("# {}\n\n", n.title));
            for card in &n.cards {
                if !card.title.is_empty() {
                    s.push_str(&format!("**{}**\n\n", card.title));
                }
                s.push_str(&card_body_md(card));
            }
        }
        Some(s)
    }

    /// Export one basket (node) to a standalone HTML document. `with_subnodes`
    /// includes the whole subtree. `None` if the node no longer exists.
    pub fn export_node_html_doc(&self, node: NodeId, with_subnodes: bool) -> Option<String> {
        let n = self.nodes.get(&node)?;
        let mut s = String::new();
        s.push_str(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <title>Trellis basket</title>\n<style>\n",
        );
        s.push_str(EXPORT_CSS);
        s.push_str("</style>\n</head>\n<body>\n<main>\n");
        if with_subnodes {
            self.export_node_html(node, 1, &mut s);
        } else {
            s.push_str(&format!("<section class=\"node\">\n<h1>{}</h1>\n", escape_html(&n.title)));
            for card in &n.cards {
                s.push_str("<article class=\"card\">\n");
                if !card.title.is_empty() {
                    s.push_str(&format!("<h4>{}</h4>\n", escape_html(&card.title)));
                }
                s.push_str(&card_body_html(card));
                s.push_str("</article>\n");
            }
            s.push_str("</section>\n");
        }
        s.push_str("</main>\n</body>\n</html>\n");
        Some(s)
    }

    /// Export one basket (node) to a portable JSON bundle (see [`NodeExport`]),
    /// self-contained — image bytes embed inline. `with_subnodes` includes the
    /// whole subtree. `None` if the node no longer exists.
    pub fn export_node_json(&self, node: NodeId, with_subnodes: bool) -> Option<String> {
        let exp = self.node_export(node, with_subnodes)?;
        serde_json::to_string_pretty(&exp).ok()
    }

    fn node_export(&self, id: NodeId, recurse: bool) -> Option<NodeExport> {
        let n = self.nodes.get(&id)?;
        // Cards keep their layout (pos/size/color/content) but shed workspace-only
        // grouping/dock links, which don't survive being lifted out of the doc.
        let cards = n
            .cards
            .iter()
            .map(|c| {
                let mut c = c.clone();
                c.group = None;
                c.docked_to = None;
                c.editing = false;
                c
            })
            .collect();
        let children = if recurse {
            n.children.iter().filter_map(|&c| self.node_export(c, true)).collect()
        } else {
            Vec::new()
        };
        Some(NodeExport {
            format: NODE_EXPORT_FORMAT.to_string(),
            version: 1,
            title: n.title.clone(),
            bg: n.bg,
            cards,
            children,
        })
    }

    /// Import a basket bundle as a new node under `parent` (or a new root when
    /// `None`). Every node and card gets a fresh id; returns the new node's id.
    pub fn add_node_from_export(&mut self, parent: Option<NodeId>, exp: NodeExport) -> NodeId {
        let id = self.add_node(parent, exp.title);
        if let Some(n) = self.nodes.get_mut(&id) {
            n.bg = exp.bg;
        }
        for card in &exp.cards {
            if let Some(cid) = self.add_card_from(id, card, card.pos) {
                if let Some(c) = self.card_mut(id, cid) {
                    c.group = None;
                    c.docked_to = None;
                }
            }
        }
        for child in exp.children {
            self.add_node_from_export(Some(id), child);
        }
        id
    }

    /// Create a new root node from imported text, splitting nothing — the whole
    /// document becomes a single markdown card. `html` chooses conversion.
    pub fn import_as_node(&mut self, title: String, content: &str, html: bool) -> NodeId {
        let markdown = if html {
            html2md::parse_html(content)
        } else {
            content.to_string()
        };
        let id = self.add_node(None, title);
        if let Some(cid) = self.add_card(id, egui::pos2(40.0, 40.0), CardKind::Text) {
            if let Some(c) = self.card_mut(id, cid) {
                c.body = markdown;
                c.size = egui::vec2(460.0, 340.0);
                c.editing = false;
            }
        }
        id
    }

    /// Collect (node, card, snippet) matches for a case-insensitive query.
    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let q = query.to_lowercase();
        let mut hits = Vec::new();
        if q.is_empty() {
            return hits;
        }
        for node in self.nodes.values() {
            if node.title.to_lowercase().contains(&q) {
                hits.push(SearchHit {
                    node: node.id,
                    node_title: node.title.clone(),
                    snippet: "(title)".to_string(),
                });
            }
            for card in &node.cards {
                let hay = format!("{} {}", card.title, searchable_body(card));
                if let Some(pos) = hay.to_lowercase().find(&q) {
                    hits.push(SearchHit {
                        node: node.id,
                        node_title: node.title.clone(),
                        snippet: snippet_around(&hay, pos, q.len()),
                    });
                }
            }
        }
        hits
    }
}

pub struct SearchHit {
    pub node: NodeId,
    pub node_title: String,
    pub snippet: String,
}

/// Format marker written into a single-card JSON export, checked on import so a
/// random `.json` file isn't mistaken for a card.
pub const CARD_EXPORT_FORMAT: &str = "trellis-card";

/// A portable, self-contained single card (its image bytes embed inline). This
/// is what `Export Card → JSON` writes and `Import Card` / a dropped `.json`
/// reads. Position, id, grouping and dock state are intentionally omitted — they
/// only make sense inside the workspace the card came from.
#[derive(Clone, Serialize, Deserialize)]
pub struct CardExport {
    pub format: String,
    pub version: u32,
    pub title: String,
    /// Markdown / code text (text & code cards store their content here, not in
    /// `kind`). Empty for image/checklist/table/sketch cards.
    #[serde(default)]
    pub body: String,
    pub color: [u8; 3],
    pub size: [f32; 2],
    #[serde(default = "default_font_scale")]
    pub font_scale: f32,
    pub kind: CardKind,
}

/// Parse a single-card JSON export, returning `None` unless it's well-formed and
/// carries the [`CARD_EXPORT_FORMAT`] marker.
pub fn parse_card_export(json: &str) -> Option<CardExport> {
    let exp: CardExport = serde_json::from_str(json).ok()?;
    (exp.format == CARD_EXPORT_FORMAT).then_some(exp)
}

/// Format marker written into a basket (node) JSON export.
pub const NODE_EXPORT_FORMAT: &str = "trellis-node";

/// A portable basket: one node's cards (positions/colors preserved; image bytes
/// inline) and, optionally, its whole subtree. This is what **Export basket → JSON**
/// writes and **Import basket** reads. Ids are reassigned on import; workspace-only
/// grouping/dock links are dropped.
#[derive(Clone, Serialize, Deserialize)]
pub struct NodeExport {
    pub format: String,
    pub version: u32,
    pub title: String,
    #[serde(default)]
    pub bg: Option<[u8; 3]>,
    pub cards: Vec<Card>,
    #[serde(default)]
    pub children: Vec<NodeExport>,
}

/// Parse a basket JSON export, returning `None` unless it's well-formed and
/// carries the [`NODE_EXPORT_FORMAT`] marker.
pub fn parse_node_export(json: &str) -> Option<NodeExport> {
    let exp: NodeExport = serde_json::from_str(json).ok()?;
    (exp.format == NODE_EXPORT_FORMAT).then_some(exp)
}

fn searchable_body(card: &Card) -> String {
    match &card.kind {
        CardKind::Text | CardKind::Code { .. } => card.body.clone(),
        CardKind::Checklist { items } => items
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        CardKind::Table { table } => table
            .rows
            .iter()
            .flat_map(|r| r.iter().map(|c| c.text.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        k @ CardKind::Image { ocr, .. } => {
            let names = k.images().iter().map(|(_, n)| *n).collect::<Vec<_>>().join(" ");
            if ocr.is_empty() { names } else { format!("{names} {ocr}") }
        }
        CardKind::Sketch { .. } => String::new(),
    }
}

fn snippet_around(text: &str, pos: usize, len: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    // `pos`/`len` are byte offsets into `text`; map to a char window loosely.
    let start_byte = pos.saturating_sub(20);
    let end_byte = (pos + len + 20).min(text.len());
    let slice = text
        .char_indices()
        .filter(|(i, _)| *i >= start_byte && *i < end_byte)
        .map(|(_, c)| c)
        .collect::<String>();
    let _ = chars;
    let trimmed = slice.replace('\n', " ");
    format!("…{}…", trimmed.trim())
}

fn md_to_html(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let wrapped = hard_wrap(md);
    let parser = Parser::new_ext(&wrapped, Options::all());
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Turn single newlines into Markdown hard breaks so a rendered card matches
/// what the user typed line-for-line. CommonMark treats a lone newline as a
/// "soft break" (rendered as a space), so without this you'd need a blank line
/// between every line; users expect each Enter to break. We append the two
/// trailing spaces that mark a hard break to each non-empty line, skipping
/// fenced code blocks (``` / ~~~) where newlines are already literal.
pub(crate) fn hard_wrap(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + 16);
    let mut in_fence = false;
    let mut lines = md.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
        } else if in_fence || line.trim_end().is_empty() {
            // Code-block content or a blank paragraph separator: leave as-is.
            out.push_str(line);
        } else {
            out.push_str(line.trim_end());
            out.push_str("  "); // two trailing spaces = hard break
        }
        if lines.peek().is_some() {
            out.push('\n');
        }
    }
    if md.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render a Sketch card's strokes as a self-contained inline SVG for HTML export.
/// The inner HTML for one card's body (no `<article>` wrapper or title). Shared
/// by the whole-document and single-card HTML exporters.
fn card_body_html(card: &Card) -> String {
    let mut s = String::new();
    match &card.kind {
        CardKind::Text => s.push_str(&md_to_html(&card.body)),
        CardKind::Code { lang } => {
            let fenced = format!("```{lang}\n{}\n```", card.body);
            s.push_str(&md_to_html(&fenced));
        }
        CardKind::Checklist { items } => {
            s.push_str("<ul class=\"checklist\">\n");
            for it in items {
                let mark = if it.done { "checked" } else { "" };
                s.push_str(&format!(
                    "<li><input type=\"checkbox\" disabled {mark}> {}</li>\n",
                    escape_html(&it.text)
                ));
            }
            s.push_str("</ul>\n");
        }
        CardKind::Table { table } => {
            s.push_str("<table border=\"1\" cellspacing=\"0\" cellpadding=\"4\">\n");
            for (r, row) in table.rows.iter().enumerate() {
                s.push_str("<tr>");
                for cell in row {
                    let tag = if table.header && r == 0 { "th" } else { "td" };
                    let mut style = String::new();
                    if let Some([rr, gg, bb]) = cell.bg {
                        style.push_str(&format!("background:#{rr:02x}{gg:02x}{bb:02x};"));
                    }
                    if let Some([rr, gg, bb]) = cell.fg {
                        style.push_str(&format!("color:#{rr:02x}{gg:02x}{bb:02x};"));
                    }
                    let style_attr = if style.is_empty() {
                        String::new()
                    } else {
                        format!(" style=\"{style}\"")
                    };
                    s.push_str(&format!("<{tag}{style_attr}>{}</{tag}>", escape_html(&cell.text)));
                }
                s.push_str("</tr>\n");
            }
            s.push_str("</table>\n");
        }
        k @ CardKind::Image { .. } => {
            for (data, name) in k.images() {
                let b64 = base64::engine::general_purpose::STANDARD.encode(data);
                let mime = mime_for(name);
                s.push_str(&format!(
                    "<img alt=\"{}\" src=\"data:{mime};base64,{b64}\">\n",
                    escape_html(name)
                ));
            }
        }
        CardKind::Sketch { strokes } => {
            s.push_str(&sketch_svg(strokes, card.size.x, card.size.y));
            s.push('\n');
        }
    }
    s
}

/// The Markdown for one card's body (no title). Shared by the whole-document and
/// single-card Markdown exporters.
fn card_body_md(card: &Card) -> String {
    let mut s = String::new();
    match &card.kind {
        CardKind::Text => {
            s.push_str(card.body.trim_end());
            s.push_str("\n\n");
        }
        CardKind::Code { lang } => {
            s.push_str(&format!("```{lang}\n{}\n```\n\n", card.body));
        }
        CardKind::Checklist { items } => {
            for it in items {
                let mark = if it.done { "x" } else { " " };
                s.push_str(&format!("- [{mark}] {}\n", it.text));
            }
            s.push('\n');
        }
        CardKind::Table { table } => {
            let md_row = |row: &Vec<TableCell>| {
                format!(
                    "| {} |\n",
                    row.iter()
                        .map(|c| c.text.replace('|', "\\|"))
                        .collect::<Vec<_>>()
                        .join(" | ")
                )
            };
            let cols = table.n_cols();
            for (r, row) in table.rows.iter().enumerate() {
                s.push_str(&md_row(row));
                if r == 0 && table.header && cols > 0 {
                    s.push_str(&format!("|{}\n", " --- |".repeat(cols)));
                }
            }
            s.push('\n');
        }
        k @ CardKind::Image { .. } => {
            for (_, name) in k.images() {
                s.push_str(&format!("*(image: {name})*\n\n"));
            }
        }
        CardKind::Sketch { strokes } => {
            s.push_str(&format!("*(sketch: {} strokes)*\n\n", strokes.len()));
        }
    }
    s
}

/// The laid-out lines for one card (title line, then body). Shared by the PDF
/// and image exporters (whole-document and single-card). Ends with a spacer.
fn card_lines(card: &Card) -> Vec<ExportLine> {
    let mut out = Vec::new();
    if !card.title.is_empty() {
        out.push(ExportLine { text: card.title.clone(), size: 12.0 });
    }
    match &card.kind {
        CardKind::Text => {
            let body = card.body.trim_end();
            if !body.is_empty() {
                out.push(ExportLine { text: body.to_string(), size: 10.5 });
            }
        }
        CardKind::Code { .. } => {
            for line in card.body.trim_end().split('\n') {
                out.push(ExportLine { text: line.to_string(), size: 10.0 });
            }
        }
        CardKind::Checklist { items } => {
            for it in items {
                let mark = if it.done { "[x]" } else { "[ ]" };
                out.push(ExportLine { text: format!("{mark} {}", it.text), size: 10.5 });
            }
        }
        CardKind::Table { table } => {
            for row in &table.rows {
                let line = row.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join(" | ");
                out.push(ExportLine { text: line, size: 10.5 });
            }
        }
        k @ CardKind::Image { .. } => {
            for (_, name) in k.images() {
                out.push(ExportLine { text: format!("(image: {name})"), size: 10.5 });
            }
        }
        CardKind::Sketch { strokes } => {
            out.push(ExportLine {
                text: format!("(sketch: {} strokes)", strokes.len()),
                size: 10.5,
            });
        }
    }
    out.push(ExportLine { text: String::new(), size: 5.0 });
    out
}

/// Render a flat list of laid-out lines to a paginated A4 PDF. Shared by the
/// whole-document and single-card PDF exporters.
fn lines_to_pdf(lines: &[ExportLine]) -> Result<Vec<u8>, String> {
    use printpdf::{Mm, PdfDocument};
    let font_ab = ab_glyph::FontRef::try_from_slice(EXPORT_FONT).map_err(|e| e.to_string())?;
    let (w_mm, h_mm, margin) = (210.0_f32, 297.0_f32, 20.0_f32);
    const MM_TO_PT: f32 = 2.834_646;
    let content_w_pt = (w_mm - margin * 2.0) * MM_TO_PT;
    let (doc, page1, layer1) = PdfDocument::new("Trellis export", Mm(w_mm), Mm(h_mm), "Layer 1");
    let font = doc
        .add_external_font(std::io::Cursor::new(EXPORT_FONT))
        .map_err(|e| e.to_string())?;
    let mut layer = doc.get_page(page1).get_layer(layer1);
    let mut y = h_mm - margin;
    for l in lines {
        let leading = (l.size * 1.4) / MM_TO_PT;
        let wrapped = if l.text.is_empty() {
            vec![String::new()]
        } else {
            wrap_text(&font_ab, l.size, &l.text, content_w_pt)
        };
        for line in wrapped {
            if y < margin {
                let (p, lay) = doc.add_page(Mm(w_mm), Mm(h_mm), "Layer");
                layer = doc.get_page(p).get_layer(lay);
                y = h_mm - margin;
            }
            if !line.is_empty() {
                layer.use_text(&line, l.size, Mm(margin), Mm(y), &font);
            }
            y -= leading;
        }
    }
    doc.save_to_bytes().map_err(|e| e.to_string())
}

/// Render a flat list of laid-out lines to a raster image (PNG, or GIF if
/// `gif`). One tall page, black text on white. Shared by the whole-document and
/// single-card image exporters.
fn lines_to_image(lines: &[ExportLine], gif: bool) -> Result<Vec<u8>, String> {
    use ab_glyph::FontRef;
    use image::{Rgba, RgbaImage};
    let font = FontRef::try_from_slice(EXPORT_FONT).map_err(|e| e.to_string())?;
    let scale = 2.0_f32; // px per point
    let margin = 40.0_f32;
    let content_w = 760.0_f32;
    let width = (content_w + margin * 2.0) as u32;

    // Pre-wrap every line, remembering its pixel size, to size the canvas.
    let mut rows: Vec<(String, f32)> = Vec::new();
    for l in lines {
        let px = l.size * scale;
        if l.text.is_empty() {
            rows.push((String::new(), px));
        } else {
            for w in wrap_text(&font, px, &l.text, content_w) {
                rows.push((w, px));
            }
        }
    }
    let total_h: f32 = margin * 2.0 + rows.iter().map(|(_, s)| s * 1.5).sum::<f32>();
    let height = (total_h as u32).max(1);
    let mut img = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));

    let mut y = margin;
    for (text, px) in &rows {
        if !text.is_empty() {
            draw_text(&mut img, &font, *px, margin, y + *px, text);
        }
        y += px * 1.5;
    }
    let mut buf = Vec::new();
    let fmt = if gif { image::ImageFormat::Gif } else { image::ImageFormat::Png };
    img.write_to(&mut std::io::Cursor::new(&mut buf), fmt).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Wrap a raw RGBA image (the WYSIWYG card screenshot) in a single-page PDF sized
/// to the image at 150 DPI. Used by the per-card PDF export.
pub fn image_rgba_to_pdf(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    use printpdf::{ColorBits, ColorSpace, Image, ImageTransform, ImageXObject, Mm, PdfDocument, Px};
    if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
        return Err("empty or malformed image".to_string());
    }
    // Drop alpha, compositing over white (cards are opaque, so this is exact).
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for px in rgba.chunks_exact(4) {
        let a = px[3] as u32;
        let over = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
        rgb.extend_from_slice(&[over(px[0]), over(px[1]), over(px[2])]);
    }
    let dpi = 150.0_f32;
    const MM_PER_PT: f32 = 25.4 / 72.0;
    let w_mm = width as f32 / dpi * 72.0 * MM_PER_PT;
    let h_mm = height as f32 / dpi * 72.0 * MM_PER_PT;
    let (doc, page, layer) = PdfDocument::new("Trellis card", Mm(w_mm), Mm(h_mm), "Layer 1");
    let xobj = ImageXObject {
        width: Px(width as usize),
        height: Px(height as usize),
        color_space: ColorSpace::Rgb,
        bits_per_component: ColorBits::Bit8,
        interpolate: false,
        image_data: rgb,
        image_filter: None,
        smask: None,
        clipping_bbox: None,
    };
    Image::from(xobj).add_to_layer(
        doc.get_page(page).get_layer(layer),
        ImageTransform { dpi: Some(dpi), ..Default::default() },
    );
    doc.save_to_bytes().map_err(|e| e.to_string())
}

fn sketch_svg(strokes: &[Stroke], w: f32, h: f32) -> String {
    let w = w.max(1.0);
    let h = h.max(1.0);
    let mut s = format!(
        "<svg viewBox=\"0 0 {w:.0} {h:.0}\" width=\"{w:.0}\" height=\"{h:.0}\" \
         xmlns=\"http://www.w3.org/2000/svg\" style=\"max-width:100%;height:auto\">"
    );
    for st in strokes {
        let [r, g, b] = st.color;
        let pts = st
            .points
            .iter()
            .map(|p| format!("{:.1},{:.1}", p[0], p[1]))
            .collect::<Vec<_>>()
            .join(" ");
        s.push_str(&format!(
            "<polyline points=\"{pts}\" fill=\"none\" stroke=\"#{r:02x}{g:02x}{b:02x}\" \
             stroke-width=\"{:.1}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
            st.width
        ));
    }
    s.push_str("</svg>");
    s
}

fn mime_for(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else {
        "application/octet-stream"
    }
}

const EXPORT_CSS: &str = "\
:root{color-scheme:light dark}\
body{font-family:-apple-system,Segoe UI,Roboto,sans-serif;line-height:1.55;margin:0;\
background:#faf9f7;color:#1b1b1b}\
main{max-width:820px;margin:0 auto;padding:2.5rem 1.25rem}\
section.node{margin:1.25rem 0;padding-left:1rem;border-left:2px solid #e2ded7}\
h1,h2,h3,h4,h5,h6{line-height:1.2}\
article.card{background:#fff;border:1px solid #e6e2db;border-radius:8px;padding:.85rem 1rem;\
margin:.75rem 0;box-shadow:0 1px 2px rgba(0,0,0,.04)}\
article.card h4{margin:.1rem 0 .5rem;color:#555}\
ul.checklist{list-style:none;padding-left:0}\
ul.checklist li{margin:.2rem 0}\
img{max-width:100%;border-radius:6px}\
pre{background:#1e1e1e;color:#eee;padding:.75rem 1rem;border-radius:6px;overflow:auto}\
code{font-family:ui-monospace,SFMono-Regular,Menlo,monospace}\
:not(pre)>code{background:#eee;padding:.1rem .3rem;border-radius:4px}\
@media(prefers-color-scheme:dark){body{background:#17181a;color:#e6e6e6}\
section.node{border-left-color:#333}article.card{background:#202225;border-color:#333}\
article.card h4{color:#aaa}:not(pre)>code{background:#333}}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_wrap_breaks_single_newlines_but_not_code_or_blank_lines() {
        // Single newlines get two trailing spaces (a Markdown hard break)...
        assert_eq!(hard_wrap("a\nb"), "a  \nb  ");
        // ...blank paragraph separators are left alone...
        assert_eq!(hard_wrap("a\n\nb"), "a  \n\nb  ");
        // ...and fenced code blocks are untouched.
        assert_eq!(hard_wrap("```\nx\ny\n```"), "```\nx\ny\n```");
    }

    #[test]
    fn hard_wrap_renders_as_line_breaks_in_html() {
        // The whole point: two lines become two visual lines (<br>), not one.
        assert!(md_to_html("line one\nline two").contains("<br"));
    }

    #[test]
    fn basket_export_import_round_trips_node_and_subtree() {
        let mut doc = Document::empty();
        let day = doc.add_node(None, "Wednesday".into());
        let c1 = doc.add_card(day, egui::pos2(60.0, 60.0), CardKind::Text).unwrap();
        doc.card_mut(day, c1).unwrap().body = "morning notes".into();
        doc.card_mut(day, c1).unwrap().title = "Journal".into();
        let sub = doc.add_node(Some(day), "Meeting".into());
        doc.add_card(sub, egui::pos2(20.0, 20.0), CardKind::Checklist {
            items: vec![ChecklistItem { done: true, text: "ship it".into() }],
        })
        .unwrap();

        // Markdown / HTML reflect the content; +subnodes pulls in the child.
        let md = doc.export_node_markdown(day, false).unwrap();
        assert!(md.contains("morning notes") && !md.contains("Meeting"));
        let md_sub = doc.export_node_markdown(day, true).unwrap();
        assert!(md_sub.contains("Meeting") && md_sub.contains("ship it"));
        assert!(doc.export_node_html_doc(day, true).unwrap().contains("morning notes"));

        // JSON round-trips: import rebuilds the node + subtree with fresh ids and
        // the card content/position preserved.
        let json = doc.export_node_json(day, true).unwrap();
        let exp = parse_node_export(&json).expect("valid basket file");
        let n_nodes = doc.nodes.len();
        let new = doc.add_node_from_export(None, exp);
        assert_eq!(doc.nodes.len(), n_nodes + 2, "node + its child were imported");
        let nn = &doc.nodes[&new];
        assert_eq!(nn.title, "Wednesday");
        let jcard = nn.cards.iter().find(|c| c.title == "Journal").unwrap();
        assert_eq!(jcard.body, "morning notes");
        assert_eq!(jcard.pos, egui::pos2(60.0, 60.0), "position preserved");
        assert_ne!(jcard.id, c1, "imported card got a fresh id");
        // The imported child node came along.
        assert!(doc.nodes.values().filter(|n| n.title == "Meeting").count() >= 2);
        // A non-basket JSON is rejected.
        assert!(parse_node_export("{\"format\":\"trellis-card\",\"version\":1}").is_none());
    }

    #[test]
    fn fit_size_grows_to_fit_content_and_skips_images() {
        let default = egui::vec2(240.0, 160.0);

        // A checklist with a long item gets much wider than the default square,
        // and wide enough that the text isn't clipped.
        let long = "buy oat milk, eggs, bread, coffee, and a birthday card for mum";
        let mut cl = Card::new(1, egui::pos2(0.0, 0.0), CardKind::Checklist {
            items: vec![ChecklistItem { done: false, text: long.into() }],
        });
        cl.title = "Groceries".into();
        let sz = cl.fit_size().expect("checklist fits");
        assert!(sz.x > default.x, "checklist should widen: {sz:?}");
        // roughly checkbox + text-at-~8.4px/char + controls
        assert!(sz.x >= long.chars().count() as f32 * 8.0, "wide enough for text: {sz:?}");

        // A multi-line text card grows taller than the default.
        let mut txt = Card::new(2, egui::pos2(0.0, 0.0), CardKind::Text);
        txt.body = (0..12).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let sz = txt.fit_size().expect("text fits");
        assert!(sz.y > default.y, "12 lines should grow taller: {sz:?}");

        // Everything stays within the sane clamp bounds.
        let mut huge = Card::new(3, egui::pos2(0.0, 0.0), CardKind::Text);
        huge.body = "x".repeat(5000);
        let sz = huge.fit_size().unwrap();
        assert!(sz.x <= 900.0 && sz.y <= 1400.0, "clamped: {sz:?}");

        // Image cards opt out (their size is driven by the pictures, not text).
        let img = Card::new(4, egui::pos2(0.0, 0.0), CardKind::Image {
            data: vec![],
            name: String::new(),
            extra: vec![],
            ocr: String::new(),
        });
        assert!(img.fit_size().is_none());
    }

    #[test]
    fn table_ops_keep_grid_rectangular_and_roundtrip_csv_xlsx() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let c = doc
            .add_card(n, egui::pos2(0.0, 0.0), CardKind::Table { table: TableData::empty(2, 2) })
            .unwrap();

        assert!(doc.table_set_cell(n, c, 0, 0, "Name".into()));
        assert!(doc.table_set_cell(n, c, 0, 1, "Qty".into()));
        assert!(doc.table_set_cell(n, c, 1, 0, "Apples, \"red\"".into()));
        assert!(doc.table_set_cell(n, c, 1, 1, "3".into()));
        assert!(doc.table_set_bg(n, c, 0, 0, Some([255, 0, 0])));
        assert!(doc.table_set_fg(n, c, 1, 1, Some([0, 0, 255])));
        assert!(!doc.table_set_cell(n, c, 9, 9, "out of range".into()));

        // Row/col ops keep the grid rectangular and never empty.
        assert!(doc.table_insert_row(n, c, 1));
        assert!(doc.table_insert_col(n, c, 0));
        {
            let CardKind::Table { table } = &doc.nodes[&n].cards[0].kind else { panic!() };
            assert_eq!(table.rows.len(), 3);
            assert!(table.rows.iter().all(|r| r.len() == 3));
        }
        assert!(doc.table_remove_row(n, c, 1));
        assert!(doc.table_remove_col(n, c, 0));
        assert!(doc.table_set_col_width(n, c, 0, 200.0));

        let CardKind::Table { table } = doc.nodes[&n].cards[0].kind.clone() else { panic!() };
        assert_eq!(table.col_width(0), 200.0);

        // CSV round-trip, quoting included.
        let csv = table.to_csv();
        let back = csv_to_values(csv.as_bytes()).unwrap();
        assert_eq!(back[1][0], "Apples, \"red\"");
        assert_eq!(back[0], vec!["Name", "Qty"]);

        // XLSX round-trip through calamine; colors live in the file.
        let xlsx = table.to_xlsx().unwrap();
        assert_eq!(&xlsx[..2], b"PK"); // zip magic
        let back = xlsx_to_values(&xlsx).unwrap();
        assert_eq!(back[0], vec!["Name", "Qty"]);
        assert_eq!(back[1][1], "3");

        // Exports and search cover the table.
        let html = doc.export_html();
        assert!(html.contains("<th style=\"background:#ff0000;\">Name</th>"));
        assert!(html.contains("<td style=\"color:#0000ff;\">3</td>"));
        let md = doc.export_markdown();
        assert!(md.contains("| Name | Qty |"));
        assert!(md.contains("| --- | --- |"));
    }

    #[test]
    fn image_cards_hold_multiple_images_and_legacy_ron_loads() {
        // A pre-multi-image card (no `extra` field in the RON) still loads.
        let legacy = r#"(
            id: 1, pos: (x: 0.0, y: 0.0), size: (x: 10.0, y: 10.0),
            title: "", body: "", color: (1, 2, 3),
            kind: Image(data: [9, 9], name: "old.png"),
        )"#;
        let card: Card = ron::from_str(legacy).expect("legacy image card RON loads");
        let imgs = card.kind.images();
        assert_eq!(imgs.len(), 1);
        assert_eq!(imgs[0].1, "old.png");

        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let c = doc
            .add_card(
                n,
                egui::pos2(0.0, 0.0),
                CardKind::Image { data: Vec::new(), name: String::new(), extra: Vec::new(), ocr: String::new() },
            )
            .unwrap();

        // First load fills the primary slot; later loads append.
        assert!(doc.add_image(n, c, vec![1], "a.png".into()));
        assert!(doc.add_image(n, c, vec![2], "b.png".into()));
        assert!(doc.add_image(n, c, vec![3], "c.png".into()));
        let names: Vec<String> = doc.card_mut(n, c).unwrap().kind.images()
            .iter().map(|(_, s)| s.to_string()).collect();
        assert_eq!(names, ["a.png", "b.png", "c.png"]);

        // Removing the primary promotes the next image; indices stay stable.
        assert!(doc.remove_image(n, c, 0));
        assert!(doc.remove_image(n, c, 1));
        let names: Vec<String> = doc.card_mut(n, c).unwrap().kind.images()
            .iter().map(|(_, s)| s.to_string()).collect();
        assert_eq!(names, ["b.png"]);
        assert!(!doc.remove_image(n, c, 5));
        assert!(doc.remove_image(n, c, 0));
        assert!(doc.card_mut(n, c).unwrap().kind.images().is_empty());
    }

    #[test]
    fn node_bg_roundtrips_and_legacy_ron_defaults_to_none() {
        // A pre-bg node (no `bg` field in the RON) still loads, defaulting to None.
        let legacy = r#"(
            id: 1, title: "n", parent: None, children: [], cards: [],
        )"#;
        let node: Node = ron::from_str(legacy).expect("legacy node RON loads");
        assert_eq!(node.bg, None);

        // A basket color set on a node survives a RON round-trip.
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        assert_eq!(doc.nodes[&n].bg, None);
        doc.nodes.get_mut(&n).unwrap().bg = Some([0x22, 0x33, 0x44]);
        let ron = ron::to_string(&doc).expect("serialize");
        let back: Document = ron::from_str(&ron).expect("deserialize");
        assert_eq!(back.nodes[&n].bg, Some([0x22, 0x33, 0x44]));
    }

    #[test]
    fn grouping_and_docking() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(50.0, 0.0), CardKind::Text).unwrap();
        let c = doc.add_card(n, egui::pos2(100.0, 0.0), CardKind::Text).unwrap();

        // Group needs 2+ cards.
        assert!(doc.group_cards(n, &[a], "x".into()).is_none());
        let g = doc.group_cards(n, &[a, b], "Pair".into()).unwrap();
        assert_eq!(doc.card_mut(n, a).unwrap().group, Some(g));
        assert_eq!(doc.card_mut(n, b).unwrap().group, Some(g));
        assert_eq!(doc.nodes[&n].groups.len(), 1);

        // Ungroup clears membership and drops the container.
        doc.ungroup(n, g);
        assert_eq!(doc.card_mut(n, a).unwrap().group, None);
        assert!(doc.nodes[&n].groups.is_empty());

        // Dock c onto a; moving a drags c along, b stays put.
        doc.dock_card(n, c, a);
        assert_eq!(doc.card_mut(n, c).unwrap().docked_to, Some(a));
        doc.move_card_tree(n, a, egui::vec2(10.0, 5.0));
        assert_eq!(doc.card_mut(n, a).unwrap().pos, egui::pos2(10.0, 5.0));
        assert_eq!(doc.card_mut(n, c).unwrap().pos, egui::pos2(110.0, 5.0));
        assert_eq!(doc.card_mut(n, b).unwrap().pos, egui::pos2(50.0, 0.0));
    }

    #[test]
    fn dock_rejects_cycles_and_remove_detaches() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.dock_card(n, b, a); // b sticks to a
        doc.dock_card(n, a, b); // would cycle → ignored
        assert_eq!(doc.card_mut(n, a).unwrap().docked_to, None);
        // Removing the anchor detaches its dependents.
        doc.remove_card(n, a);
        assert_eq!(doc.card_mut(n, b).unwrap().docked_to, None);
    }

    #[test]
    fn ron_round_trips() {
        let doc = Document::default();
        let s = ron::ser::to_string(&doc).unwrap();
        let back: Document = ron::from_str(&s).unwrap();
        assert_eq!(doc.roots, back.roots);
        assert_eq!(doc.nodes.len(), back.nodes.len());
    }

    #[test]
    fn indent_then_outdent_restores_shape() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let a = doc.add_node(None, "a".into());
        let b = doc.add_node(None, "b".into());
        // b indents under a...
        doc.indent(b);
        assert_eq!(doc.nodes[&b].parent, Some(a));
        assert_eq!(doc.nodes[&a].children, vec![b]);
        assert_eq!(doc.roots, vec![a]);
        // ...and outdents back to a root sibling of a.
        doc.outdent(b);
        assert_eq!(doc.nodes[&b].parent, None);
        assert!(doc.nodes[&a].children.is_empty());
        assert_eq!(doc.roots, vec![a, b]);
    }

    #[test]
    fn move_to_edge_and_reorder() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "a".into());
        let b = doc.add_node(None, "b".into());
        let c = doc.add_node(None, "c".into());
        // roots: [a, b, c]
        doc.move_to_edge(c, true);
        assert_eq!(doc.roots, vec![c, a, b]);
        doc.move_to_edge(c, false);
        assert_eq!(doc.roots, vec![a, b, c]);
        // Drop c before a.
        doc.reorder(c, a, true);
        assert_eq!(doc.roots, vec![c, a, b]);
        // Drop a after b.
        doc.reorder(a, b, false);
        assert_eq!(doc.roots, vec![c, b, a]);
    }

    #[test]
    fn reorder_reparents_and_blocks_cycles() {
        let mut doc = Document::empty();
        let parent = doc.add_node(None, "p".into());
        let child = doc.add_node(Some(parent), "c".into());
        let other = doc.add_node(None, "o".into());
        // Move `other` under parent, before child.
        doc.reorder(other, child, true);
        assert_eq!(doc.nodes[&other].parent, Some(parent));
        assert_eq!(doc.nodes[&parent].children, vec![other, child]);
        assert!(doc.roots.contains(&parent) && !doc.roots.contains(&other));
        // Dropping a parent into its own child is refused.
        doc.reorder(parent, child, true);
        assert_eq!(doc.nodes[&parent].parent, None);
    }

    #[test]
    fn remove_node_drops_whole_subtree() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let a = doc.add_node(None, "a".into());
        let b = doc.add_node(Some(a), "b".into());
        let c = doc.add_node(Some(b), "c".into());
        doc.remove_node(a);
        assert!(!doc.nodes.contains_key(&a));
        assert!(!doc.nodes.contains_key(&b));
        assert!(!doc.nodes.contains_key(&c));
        assert!(doc.roots.is_empty());
    }

    #[test]
    fn export_html_includes_content_and_checklist() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let n = doc.add_node(None, "Node & <title>".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, cid).unwrap().body = "**bold**".into();
        let lid = doc
            .add_card(
                n,
                egui::pos2(0.0, 0.0),
                CardKind::Checklist {
                    items: vec![ChecklistItem { done: true, text: "done item".into() }],
                },
            )
            .unwrap();
        let _ = lid;
        let html = doc.export_html();
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("Node &amp; &lt;title&gt;")); // escaped
        assert!(html.contains("checked"));
        assert!(html.contains("done item"));
    }

    #[test]
    fn import_html_becomes_markdown_card() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let id = doc.import_as_node("page".into(), "<h1>Hi</h1><p>there</p>", true);
        let node = &doc.nodes[&id];
        assert_eq!(node.cards.len(), 1);
        assert!(node.cards[0].body.contains("Hi"));
    }

    #[test]
    fn paste_card_into_another_node() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "a".into());
        let b = doc.add_node(None, "b".into());
        let cid = doc.add_card(a, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(a, cid).unwrap().body = "hello".into();
        let template = doc.nodes[&a].cards[0].clone();
        let new_id = doc.add_card_from(b, &template, egui::pos2(5.0, 5.0)).unwrap();
        assert_ne!(new_id, cid); // fresh id
        assert_eq!(doc.nodes[&b].cards.len(), 1);
        assert_eq!(doc.nodes[&b].cards[0].body, "hello");
        assert_eq!(doc.nodes[&b].cards[0].pos, egui::pos2(5.0, 5.0));
        // Original untouched.
        assert_eq!(doc.nodes[&a].cards.len(), 1);
    }

    #[test]
    fn export_pdf_and_image_produce_valid_files() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Report".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c).unwrap().title = "Intro".into();
        doc.card_mut(n, c).unwrap().body =
            "A fairly long paragraph that should wrap across several lines when \
             laid out into a fixed-width page so we exercise the wrapper too."
                .into();

        let pdf = doc.export_pdf().expect("pdf");
        assert!(pdf.starts_with(b"%PDF"), "PDF magic header");

        let png = doc.export_image(false).expect("png");
        assert_eq!(&png[1..4], b"PNG", "PNG magic header");

        let gif = doc.export_image(true).expect("gif");
        assert!(gif.starts_with(b"GIF8"), "GIF magic header");
    }

    #[test]
    fn export_markdown_has_headings_and_cards() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Title".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, cid).unwrap().body = "**bold** body".into();
        let md = doc.export_markdown();
        assert!(md.contains("# Title"));
        assert!(md.contains("**bold** body"));
    }

    #[test]
    fn export_single_card_in_each_format() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Node".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        {
            let c = doc.card_mut(n, cid).unwrap();
            c.title = "My Card".into();
            c.body = "**bold** body".into();
        }

        let md = doc.export_card_markdown(n, cid).unwrap();
        assert!(md.contains("# My Card"));
        assert!(md.contains("**bold** body"));

        let html = doc.export_card_html(n, cid).unwrap();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<h4>My Card</h4>"));

        let txt = doc.export_card_text(n, cid).unwrap();
        assert!(txt.contains("My Card"));
        assert!(txt.contains("**bold** body"));

        // JSON export carries the format marker and round-trips the body.
        let json = doc.export_card_json(n, cid).unwrap();
        assert_eq!(parse_card_export(&json).unwrap().body, "**bold** body");

        // A non-existent card yields None rather than panicking.
        let missing: CardId = 999_999;
        assert!(doc.export_card_markdown(n, missing).is_none());
        assert!(doc.export_card_json(n, missing).is_none());
        // SVG is sketch-only. (PNG/PDF are produced by the live app via screenshot.)
        assert!(doc.export_card_svg(n, cid).is_none());
    }

    #[test]
    fn sketch_svg_export_lists_strokes_and_image_pdf_wraps_a_raster() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());

        // Sketch card: SVG export lists the stroke as a polyline.
        let sk = doc
            .add_card(n, egui::pos2(0.0, 0.0), CardKind::Sketch {
                strokes: vec![Stroke { color: [255, 0, 0], width: 3.0, points: vec![[1.0, 1.0], [8.0, 8.0]] }],
            })
            .unwrap();
        assert!(doc.export_card_svg(n, sk).unwrap().contains("<polyline"));
        // Non-sketch cards have no SVG.
        let t = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        assert!(doc.export_card_svg(n, t).is_none());

        // A raw RGBA card screenshot wraps into a valid one-page PDF.
        let rgba = vec![200u8; 4 * 4 * 4]; // 4×4 opaque image
        let pdf = image_rgba_to_pdf(&rgba, 4, 4).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        assert!(image_rgba_to_pdf(&[], 0, 0).is_err());
    }

    #[test]
    fn card_json_export_import_round_trips() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let cid = doc
            .add_card(n, egui::pos2(5.0, 5.0), CardKind::Checklist {
                items: vec![
                    ChecklistItem { done: true, text: "a".into() },
                    ChecklistItem { done: false, text: "b".into() },
                ],
            })
            .unwrap();
        {
            let c = doc.card_mut(n, cid).unwrap();
            c.title = "Todo".into();
            c.body = "a note in the body field".into();
            c.color = [10, 20, 30];
            c.size = egui::vec2(300.0, 200.0);
        }

        let json = doc.export_card_json(n, cid).unwrap();
        let exp = parse_card_export(&json).expect("valid card file");

        // Import into a different node; it gets a fresh id but preserves content.
        let m = doc.add_node(None, "m".into());
        let new = doc.add_card_from_export(m, egui::pos2(0.0, 0.0), exp).unwrap();
        assert_ne!(new, cid);
        let c = doc.card(m, new).unwrap();
        assert_eq!(c.title, "Todo");
        assert_eq!(c.body, "a note in the body field");
        assert_eq!(c.color, [10, 20, 30]);
        assert_eq!(c.size, egui::vec2(300.0, 200.0));
        let CardKind::Checklist { items } = &c.kind else { panic!("kind not preserved") };
        assert_eq!(items.len(), 2);
        assert!(items[0].done && !items[1].done);

        // Wrong / missing format marker and non-JSON are rejected.
        assert!(parse_card_export("{\"format\":\"nope\",\"version\":1}").is_none());
        assert!(parse_card_export("not json at all").is_none());
    }

    #[test]
    fn search_finds_titles_and_bodies() {
        let mut doc = Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_group_id: 1,
        };
        let n = doc.add_node(None, "Groceries".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, cid).unwrap().body = "buy avocados".into();
        assert_eq!(doc.search("grocer").len(), 1);
        assert_eq!(doc.search("avocado").len(), 1);
        assert_eq!(doc.search("zzz").len(), 0);

        // An image card becomes searchable once OCR text is stored on it.
        let img = doc
            .add_card(n, egui::pos2(0.0, 0.0), CardKind::Image {
                data: Vec::new(),
                name: String::new(),
                extra: Vec::new(),
                ocr: String::new(),
            })
            .unwrap();
        assert_eq!(doc.search("invoice").len(), 0);
        assert!(doc.set_card_ocr(n, img, "Electric bill invoice total 42.00".into()));
        assert_eq!(doc.search("invoice").len(), 1);
    }

    #[test]
    fn move_checklist_item_reorders() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let items = vec![
            ChecklistItem { done: false, text: "a".into() },
            ChecklistItem { done: false, text: "b".into() },
            ChecklistItem { done: false, text: "c".into() },
        ];
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Checklist { items }).unwrap();
        // Move "c" (idx 2) to the front (before idx 0).
        assert!(doc.move_checklist_item(n, cid, 2, 0));
        let CardKind::Checklist { items } = &doc.nodes[&n].cards[0].kind else { panic!() };
        assert_eq!(items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["c", "a", "b"]);
        // No-op move returns false.
        assert!(!doc.move_checklist_item(n, cid, 1, 1));
    }

    #[test]
    fn sketch_strokes_add_undo_clear_and_export() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Sketch { strokes: Vec::new() }).unwrap();
        let s = |pts: Vec<[f32; 2]>| Stroke { color: [255, 0, 0], width: 2.0, points: pts };
        assert!(doc.sketch_add_stroke(n, cid, s(vec![[0.0, 0.0], [10.0, 10.0]])));
        assert!(!doc.sketch_add_stroke(n, cid, s(vec![]))); // empty ignored
        assert!(doc.sketch_add_stroke(n, cid, s(vec![[5.0, 5.0]])));
        let CardKind::Sketch { strokes } = &doc.nodes[&n].cards[0].kind else { panic!() };
        assert_eq!(strokes.len(), 2);
        assert!(doc.sketch_undo(n, cid));
        let CardKind::Sketch { strokes } = &doc.nodes[&n].cards[0].kind else { panic!() };
        assert_eq!(strokes.len(), 1);
        // SVG export contains a polyline with the stroke color.
        let svg = sketch_svg(strokes, 100.0, 80.0);
        assert!(svg.contains("<polyline"));
        assert!(svg.contains("#ff0000"));
        assert!(doc.sketch_clear(n, cid));
        assert!(!doc.sketch_clear(n, cid)); // already empty
    }

    #[test]
    fn autosort_lays_cards_in_a_nonoverlapping_grid() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let ids: Vec<_> = (0..5)
            .map(|_| doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap())
            .collect();
        // A checklist with a long item: autosort should auto-size it wide.
        let wide = doc
            .add_card(n, egui::pos2(0.0, 0.0), CardKind::Checklist {
                items: vec![ChecklistItem {
                    done: false,
                    text: "a genuinely long checklist item that needs a wide card".into(),
                }],
            })
            .unwrap();
        assert!(doc.autosort(n));
        // No two cards overlap even with varied sizes, and none stayed stacked.
        let rects: Vec<egui::Rect> = doc.nodes[&n]
            .cards
            .iter()
            .map(|c| egui::Rect::from_min_size(c.pos, c.size))
            .collect();
        for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                assert!(!rects[i].intersects(rects[j]), "cards {i} and {j} overlap");
            }
        }
        // The wide checklist was auto-sized past the default 240px square.
        let wide_card = doc.nodes[&n].cards.iter().find(|c| c.id == wide).unwrap();
        assert!(wide_card.size.x > 240.0, "autosort should auto-size: {:?}", wide_card.size);
        assert_eq!(ids.len(), 5);
    }

    #[test]
    fn raise_group_moves_members_to_front_keeping_order() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        // Group the two outer cards, leaving `b` between them in draw order.
        let g = doc.group_cards(n, &[a, c], "pair".into()).unwrap();
        // b is on top (added last among ungrouped); raising the group must put
        // a and c after b, preserving a-before-c.
        doc.raise_group(n, g);
        let order: Vec<CardId> = doc.nodes[&n].cards.iter().map(|c| c.id).collect();
        assert_eq!(order, vec![b, a, c]);
    }
}
