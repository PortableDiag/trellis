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
    /// Images embedded inline in a Text card's body, referenced from the
    /// markdown as `![alt](trellis:N)` where `N` indexes this vec. Kept on the
    /// card so it stays self-contained through copy/export. Empty for cards with
    /// no inline images (i.e. every card until one is added).
    #[serde(default)]
    pub inline_images: Vec<ImageEntry>,
    /// Runtime-only: whether the card is in edit mode. Never persisted.
    #[serde(skip)]
    pub editing: bool,
}

fn default_font_scale() -> f32 {
    1.0
}

impl Card {
    /// This card's inline `key:: value` properties (parsed from title + body).
    pub fn properties(&self) -> Vec<(String, String)> {
        extract_properties(&format!("{}\n{}", self.title, searchable_body(self)))
    }

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
            inline_images: Vec::new(),
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
        let line_h = font_px * 1.3; // ≈ egui's rendered row height for a 14px font
        let char_w = font_px * 0.5; // average glyph advance for proportional text

        // Keep the title readable in the title bar: title text plus room for the
        // edit/copy buttons.
        let title_w = self.title.chars().count() as f32 * 8.0 + 96.0;

        // Text couples width and height: a long title widens the card, and the
        // body then wraps at that wider width (so it needs fewer lines). Decide
        // the final width first, then measure height at the *actual* wrap width —
        // otherwise the card comes out far too tall, its height computed for a
        // narrow wrap it never actually renders at.
        if let CardKind::Text = self.kind {
            // Image markers reduce to their alt text; markup that occupies no
            // rendered width (`*`, `` ` ``) is dropped so the estimate tracks the
            // CommonMark render, not the raw source.
            let stripped = strip_size_markup(&strip_inline_markers(&self.body));
            let longest =
                stripped.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
            let natural_w = (longest * char_w).max(char_w * 8.0);
            let imgs = self.inline_image_sizes(TEXT_WRAP_W);
            let img_w = imgs.iter().map(|(iw, _)| *iw).fold(0.0_f32, f32::max);
            let content_w = natural_w.min(TEXT_WRAP_W).max(img_w);
            let w = (content_w + PAD * 2.0).max(title_w).clamp(MIN_W, MAX_W);
            let wrap_w = (w - PAD * 2.0).max(char_w);
            let mut content_h = wrapped_height(&stripped, char_w, line_h, wrap_w);
            for (_iw, ih) in &imgs {
                content_h += ih + 6.0; // each inline image stacks under the text
            }
            let h = (TITLE_H + PAD * 2.0 + content_h).clamp(MIN_H, MAX_H);
            return Some(egui::vec2(w, h));
        }

        let (content_w, content_h) = match &self.kind {
            CardKind::Image { .. } => return None,
            CardKind::Text => unreachable!("handled above"),
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

    /// Display `(width, height)` of each inline image actually referenced by the
    /// body, in appearance order, scaled so its width fits `max_w`. Reads only
    /// the image header (cheap; no full decode and no egui), so [`Card::fit_size`]
    /// can call it off the UI thread.
    pub(crate) fn inline_image_sizes(&self, max_w: f32) -> Vec<(f32, f32)> {
        let mut out = Vec::new();
        for idx in inline_refs(&self.body) {
            if let Some(entry) = self.inline_images.get(idx) {
                if let Some((iw, ih)) = image_dimensions(&entry.data) {
                    let iw = iw.max(1.0);
                    let scale = (max_w / iw).min(1.0);
                    out.push((iw * scale, ih * scale));
                }
            }
        }
        out
    }
}

/// Replace each `![alt](trellis:N)` inline-image marker in `body` with the
/// result of `f(alt, n)`. Anything that isn't a well-formed marker passes
/// through unchanged. One scanner shared by the live renderer (→ a `bytes://`
/// image), the HTML/Markdown exporters (→ a `data:` URI image), and the
/// plain-text / search paths (→ just the alt text).
pub(crate) fn map_inline_images(body: &str, mut f: impl FnMut(&str, usize) -> String) -> String {
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'!' && bytes.get(i + 1) == Some(&b'[') {
            if let Some((alt, idx, end)) = parse_inline_marker(body, i) {
                out.push_str(&f(alt, idx));
                i = end;
                continue;
            }
        }
        let ch = body[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Parse `![alt](trellis:N)` at `start` (pointing at `!`). Returns `(alt, N,
/// end_byte)` with `end_byte` just past the closing `)`, or `None` if the span
/// isn't a well-formed Trellis inline-image marker.
fn parse_inline_marker(body: &str, start: usize) -> Option<(&str, usize, usize)> {
    let rest = body[start..].strip_prefix("![")?;
    let close_alt = rest.find(']')?;
    let alt = &rest[..close_alt];
    let after = rest[close_alt + 1..].strip_prefix("(trellis:")?;
    let close = after.find(')')?;
    let num = &after[..close];
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let idx: usize = num.parse().ok()?;
    let end = start + 2 + alt.len() + 1 + "(trellis:".len() + num.len() + 1;
    Some((alt, idx, end))
}

/// Indices of the inline images referenced by `body`, in appearance order
/// (duplicates included).
pub(crate) fn inline_refs(body: &str) -> Vec<usize> {
    let mut v = Vec::new();
    map_inline_images(body, |_, n| {
        v.push(n);
        String::new()
    });
    v
}

/// The body with inline-image markers reduced to their alt text — for text
/// width/height estimation, plain-text export, copy, and full-text search.
pub(crate) fn strip_inline_markers(body: &str) -> String {
    map_inline_images(body, |alt, _| alt.to_string())
}

/// Intrinsic pixel size of an encoded image, read from its header only (no full
/// decode). `None` if the bytes aren't a recognizable image.
fn image_dimensions(bytes: &[u8]) -> Option<(f32, f32)> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    Some((w as f32, h as f32))
}

/// Estimate the height a block of text needs when wrapped at `wrap_w`: each
/// source line contributes at least one row, plus one more per `wrap_w`-worth of
/// overflow. Used by the off-thread [`Card::fit_size`] estimate; the interactive
/// Fit action measures the real galley instead (see `app::fit_card_size`).
fn wrapped_height(text: &str, char_w: f32, line_h: f32, wrap_w: f32) -> f32 {
    let cols = (wrap_w / char_w).max(1.0);
    let mut rows = 0.0f32;
    for line in text.lines() {
        let n = line.chars().count() as f32;
        rows += (n / cols).ceil().max(1.0);
    }
    rows.max(1.0) * line_h
}

/// Drop inline markup that occupies no width when rendered — emphasis `*`, code
/// `` ` ``, strikethrough `~` — so text measurement tracks the CommonMark render
/// rather than counting the raw markers. Shared by the `fit_size` estimate and
/// the interactive Fit measurement.
pub(crate) fn strip_size_markup(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '*' | '`' | '~')).collect()
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

    /// Append an image to a card's inline-image set and return its index, so the
    /// caller can splice a `![alt](trellis:N)` marker into the body. Works on any
    /// card kind (the marker only renders on Text cards).
    pub fn add_inline_image(
        &mut self,
        node: NodeId,
        card: CardId,
        bytes: Vec<u8>,
        img_name: String,
    ) -> Option<usize> {
        let c = self.card_mut(node, card)?;
        c.inline_images.push(ImageEntry { data: bytes, name: img_name });
        Some(c.inline_images.len() - 1)
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

    /// Move a card to `index` within its basket's card list, clamped to the
    /// list length (so a huge `index` sends it to the end). That list order is
    /// the draw order (last = on top) and the sequence [`Document::autosort`]
    /// lays cards out in, so this is how an agent sets card order. Returns
    /// whether it moved. `index` is the slot in the *resulting* list, counted
    /// after the card is lifted out.
    pub fn move_card(&mut self, node: NodeId, card: CardId, index: usize) -> bool {
        let Some(n) = self.nodes.get_mut(&node) else { return false };
        let Some(from) = n.cards.iter().position(|c| c.id == card) else { return false };
        let c = n.cards.remove(from);
        let to = index.min(n.cards.len());
        n.cards.insert(to, c);
        true
    }

    /// Move a card to a **different** basket, keeping its content, size and
    /// colors. Returns the card's id in its new home, or `None` if either node
    /// or the card is unknown.
    ///
    /// Group membership and docking are dropped: both reference ids that are
    /// local to the old basket, so carrying them over would point at the wrong
    /// group (or a card that isn't there). Anything docked *to* this card in the
    /// old basket is detached for the same reason. `pos` places it on the target
    /// canvas; without one it keeps its current coordinates.
    pub fn move_card_to_node(
        &mut self,
        from: NodeId,
        card: CardId,
        to: NodeId,
        pos: Option<egui::Pos2>,
    ) -> Option<CardId> {
        if from == to || !self.nodes.contains_key(&to) {
            return None;
        }
        let n = self.nodes.get_mut(&from)?;
        let idx = n.cards.iter().position(|c| c.id == card)?;
        let mut c = n.cards.remove(idx);
        for other in n.cards.iter_mut() {
            if other.docked_to == Some(card) {
                other.docked_to = None;
            }
        }
        prune_groups(n);
        c.group = None;
        c.docked_to = None;
        if let Some(p) = pos {
            c.pos = p;
        }
        let id = c.id;
        self.nodes.get_mut(&to)?.cards.push(c);
        Some(id)
    }

    /// Index of `card` in its basket's list, or `None` if the node/card is
    /// unknown. Lets callers translate a before/after target into a slot.
    pub fn card_index(&self, node: NodeId, card: CardId) -> Option<usize> {
        self.nodes.get(&node)?.cards.iter().position(|c| c.id == card)
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

    /// Set `expanded` on every node in the subtree rooted at `id` (including
    /// `id` when `include_root`), so a big branch opens or folds in one action.
    /// Returns how many nodes actually changed.
    pub fn set_subtree_expanded(&mut self, id: NodeId, expanded: bool, include_root: bool) -> usize {
        let mut stack: Vec<NodeId> = if include_root {
            vec![id]
        } else {
            self.nodes.get(&id).map(|n| n.children.clone()).unwrap_or_default()
        };
        let mut changed = 0;
        while let Some(cur) = stack.pop() {
            if let Some(n) = self.nodes.get_mut(&cur) {
                if n.expanded != expanded {
                    n.expanded = expanded;
                    changed += 1;
                }
                stack.extend(n.children.iter().copied());
            }
        }
        changed
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
    /// adopting `target`'s parent (so this also reparents across lists). Returns
    /// whether it moved; no-ops (returns `false`) if it would drop a node into
    /// its own subtree.
    pub fn reorder(&mut self, moved: NodeId, target: NodeId, before: bool) -> bool {
        if moved == target
            || !self.nodes.contains_key(&moved)
            || !self.nodes.contains_key(&target)
            || self.is_descendant(target, moved)
        {
            return false;
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
        true
    }

    /// Move `moved` under `parent` (`None` = top level) to `index`, clamped to
    /// the destination list's length (so a huge `index` appends). Reparents as
    /// needed. Returns whether it moved; no-ops (returns `false`) on an unknown
    /// node or a move that would drop a node into its own subtree. `index` is the
    /// slot in the *resulting* sibling list, so moving within one parent counts
    /// positions after the node is lifted out.
    pub fn move_node(&mut self, moved: NodeId, parent: Option<NodeId>, index: usize) -> bool {
        if !self.nodes.contains_key(&moved) {
            return false;
        }
        if let Some(p) = parent {
            if p == moved || !self.nodes.contains_key(&p) || self.is_descendant(p, moved) {
                return false;
            }
        }
        if let Some(list) = self.sibling_list_mut(moved) {
            list.retain(|x| *x != moved);
        }
        if let Some(n) = self.nodes.get_mut(&moved) {
            n.parent = parent;
        }
        let list = match parent {
            Some(p) => self.nodes.get_mut(&p).map(|n| &mut n.children),
            None => Some(&mut self.roots),
        };
        if let Some(list) = list {
            let pos = index.min(list.len());
            list.insert(pos, moved);
            true
        } else {
            false
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
            inline_images: c.inline_images.clone(),
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
            c.inline_images = exp.inline_images;
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
                    card: None,
                    node_title: node.title.clone(),
                    snippet: "(title)".to_string(),
                });
            }
            for card in &node.cards {
                let hay = format!("{} {}", card.title, searchable_body(card));
                if let Some(pos) = hay.to_lowercase().find(&q) {
                    hits.push(SearchHit {
                        node: node.id,
                        card: Some(card.id),
                        node_title: node.title.clone(),
                        snippet: snippet_around(&hay, pos, q.len()),
                    });
                }
            }
        }
        hits
    }

    /// Every `#tag` used across the document with how many cards use it, sorted
    /// by tag name. Tags are lowercased so `#Todo` and `#todo` are one tag.
    pub fn tag_counts(&self) -> Vec<(String, usize)> {
        let mut m: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for node in self.nodes.values() {
            for card in &node.cards {
                let hay = format!("{} {}", card.title, searchable_body(card));
                for t in extract_tags(&hay) {
                    *m.entry(t).or_insert(0) += 1;
                }
            }
        }
        m.into_iter().collect()
    }

    /// Cards carrying `tag` (with or without a leading `#`, case-insensitive),
    /// as search hits with a snippet around the tag.
    pub fn cards_with_tag(&self, tag: &str) -> Vec<SearchHit> {
        let want = tag.trim_start_matches('#').to_lowercase();
        let mut hits = Vec::new();
        if want.is_empty() {
            return hits;
        }
        for node in self.nodes.values() {
            for card in &node.cards {
                let hay = format!("{} {}", card.title, searchable_body(card));
                if extract_tags(&hay).iter().any(|t| *t == want) {
                    let needle = format!("#{want}");
                    let snippet = hay
                        .to_lowercase()
                        .find(&needle)
                        .map(|pos| snippet_around(&hay, pos, needle.len()))
                        .unwrap_or_else(|| snippet_around(&hay, 0, 0));
                    hits.push(SearchHit {
                        node: node.id,
                        card: Some(card.id),
                        node_title: node.title.clone(),
                        snippet,
                    });
                }
            }
        }
        hits
    }

    /// The `key:: value` properties on one card (parsed from its title + body).
    pub fn card_properties(&self, node: NodeId, card: CardId) -> Vec<(String, String)> {
        match self.card(node, card) {
            Some(c) => {
                let hay = format!("{}\n{}", c.title, searchable_body(c));
                extract_properties(&hay)
            }
            None => Vec::new(),
        }
    }

    /// Value of property `key` on a card (last one wins), or `None`.
    pub fn card_property(&self, node: NodeId, card: CardId, key: &str) -> Option<String> {
        let key = key.to_lowercase();
        self.card_properties(node, card)
            .into_iter()
            .filter(|(k, _)| *k == key)
            .next_back()
            .map(|(_, v)| v)
    }

    /// Set (or insert) an inline `key:: value` property on a card by editing its
    /// body: rewrites the first line-level `key:: …` if present, else appends a
    /// new line. Returns whether the card exists. Used by the Kanban board to
    /// change a card's `status`.
    pub fn set_card_property(&mut self, node: NodeId, card: CardId, key: &str, value: &str) -> bool {
        let key = key.to_lowercase();
        let Some(c) = self.card_mut(node, card) else { return false };
        let prefix = format!("{key}:: ");
        let mut lines: Vec<String> = c.body.lines().map(|l| l.to_string()).collect();
        let mut replaced = false;
        for line in lines.iter_mut() {
            let trimmed = line.trim_start();
            if trimmed.to_lowercase().starts_with(&prefix) {
                let indent = &line[..line.len() - trimmed.len()];
                *line = format!("{indent}{key}:: {value}");
                replaced = true;
                break;
            }
        }
        if !replaced {
            lines.push(format!("{key}:: {value}"));
        }
        c.body = lines.join("\n");
        true
    }

    /// Cards that carry a `status::` property, grouped by status value (for a
    /// Kanban board). Each entry: `(node, card, card title, node title)`.
    #[allow(clippy::type_complexity)]
    pub fn cards_by_status(&self) -> std::collections::BTreeMap<String, Vec<KanbanCard>> {
        let mut m: std::collections::BTreeMap<String, Vec<KanbanCard>> =
            std::collections::BTreeMap::new();
        for node in self.nodes.values() {
            for card in &node.cards {
                let props = card.properties();
                let Some((_, status)) = props.iter().find(|(k, _)| k == "status") else {
                    continue;
                };
                let title = if card.title.trim().is_empty() {
                    searchable_body(card).lines().next().unwrap_or("").chars().take(50).collect()
                } else {
                    card.title.clone()
                };
                let due = props.iter().find(|(k, _)| k == "due").map(|(_, v)| v.clone());
                let tags = extract_tags(&format!("{} {}", card.title, searchable_body(card)));
                m.entry(status.to_lowercase()).or_default().push(KanbanCard {
                    node: node.id,
                    card: card.id,
                    title,
                    node_title: node.title.clone(),
                    color: card.color,
                    due,
                    tags,
                });
            }
        }
        m
    }

    /// Every property key used across the document with how many cards use it.
    pub fn property_keys(&self) -> Vec<(String, usize)> {
        let mut m: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for node in self.nodes.values() {
            for card in &node.cards {
                let hay = format!("{}\n{}", card.title, searchable_body(card));
                let mut seen = std::collections::BTreeSet::new();
                for (k, _) in extract_properties(&hay) {
                    if seen.insert(k.clone()) {
                        *m.entry(k).or_insert(0) += 1;
                    }
                }
            }
        }
        m.into_iter().collect()
    }

    /// Resolve a `[[wiki-link]]` target to a node: a numeric id, else the first
    /// node whose title matches case-insensitively.
    pub fn resolve_link(&self, target: &str) -> Option<NodeId> {
        let t = target.trim();
        if let Ok(id) = t.parse::<NodeId>() {
            if self.nodes.contains_key(&id) {
                return Some(id);
            }
        }
        let tl = t.to_lowercase();
        self.nodes.values().find(|n| n.title.to_lowercase() == tl).map(|n| n.id)
    }

    /// The wiki-link graph: the nodes that participate in at least one link
    /// (as source or target) and the de-duplicated directed edges between them.
    /// Day-to-day nodes with no links are left out so the graph stays legible.
    pub fn link_graph(&self) -> (Vec<NodeId>, Vec<(NodeId, NodeId)>) {
        let mut edges: Vec<(NodeId, NodeId)> = Vec::new();
        let mut involved = std::collections::BTreeSet::new();
        for n in self.nodes.values() {
            for card in &n.cards {
                let hay = format!("{}\n{}", card.title, searchable_body(card));
                for target in extract_wikilinks(&hay) {
                    if let Some(t) = self.resolve_link(&target) {
                        if t != n.id {
                            edges.push((n.id, t));
                            involved.insert(n.id);
                            involved.insert(t);
                        }
                    }
                }
            }
        }
        edges.sort();
        edges.dedup();
        (involved.into_iter().collect(), edges)
    }

    /// Cards anywhere whose `[[links]]` point at `node` — the "linked here"
    /// backlinks. Each hit is the linking card's basket + a snippet.
    pub fn backlinks(&self, node: NodeId) -> Vec<SearchHit> {
        let mut hits = Vec::new();
        for n in self.nodes.values() {
            for card in &n.cards {
                let hay = format!("{}\n{}", card.title, searchable_body(card));
                let links = extract_wikilinks(&hay);
                if links.iter().any(|t| self.resolve_link(t) == Some(node)) {
                    hits.push(SearchHit {
                        node: n.id,
                        card: Some(card.id),
                        node_title: n.title.clone(),
                        snippet: snippet_around(&hay, 0, 0),
                    });
                }
            }
        }
        hits
    }

    /// AND-combine filters across every card: an optional `#tag`, an optional
    /// property `key` (optionally `= value`), and optional `text` (substring in
    /// title or body). Returns hits for cards matching *all* provided filters;
    /// empty if no filter is set. Powers the Find-cards panel.
    pub fn query_cards(
        &self,
        tag: Option<&str>,
        key: Option<&str>,
        value: Option<&str>,
        text: Option<&str>,
    ) -> Vec<SearchHit> {
        let tag = tag.map(|t| t.trim_start_matches('#').to_lowercase()).filter(|t| !t.is_empty());
        let key = key.map(|k| k.to_lowercase()).filter(|k| !k.is_empty());
        let value = value.map(|v| v.to_lowercase()).filter(|v| !v.is_empty());
        let text = text.map(|t| t.to_lowercase()).filter(|t| !t.is_empty());
        if tag.is_none() && key.is_none() && text.is_none() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for node in self.nodes.values() {
            for card in &node.cards {
                let hay = format!("{} {}", card.title, searchable_body(card));
                let hay_lc = hay.to_lowercase();
                if let Some(t) = &tag {
                    if !extract_tags(&hay).iter().any(|x| x == t) {
                        continue;
                    }
                }
                if let Some(k) = &key {
                    let props = extract_properties(&hay);
                    let ok = props.iter().any(|(pk, pv)| {
                        pk == k && value.as_ref().map_or(true, |w| pv.to_lowercase() == *w)
                    });
                    if !ok {
                        continue;
                    }
                }
                if let Some(t) = &text {
                    if !hay_lc.contains(t) {
                        continue;
                    }
                }
                hits.push(SearchHit {
                    node: node.id,
                    card: Some(card.id),
                    node_title: node.title.clone(),
                    snippet: snippet_around(&hay, 0, 0),
                });
            }
        }
        hits
    }

    /// Every card that carries a `due::` date, as a task, across all baskets.
    /// A task is "done" if it has `status:: done|complete|closed` or (for a
    /// checklist) every item is checked. `due_days` is the date parsed to days
    /// since the epoch (or `None` if unparseable), so the caller can bucket it.
    pub fn tasks(&self) -> Vec<TaskItem> {
        let mut out = Vec::new();
        for node in self.nodes.values() {
            for card in &node.cards {
                let props = card.properties();
                let Some((_, due)) = props.iter().find(|(k, _)| k == "due") else { continue };
                let done = props.iter().any(|(k, v)| {
                    k == "status"
                        && matches!(v.to_lowercase().as_str(), "done" | "complete" | "completed" | "closed")
                }) || matches!(&card.kind, CardKind::Checklist { items }
                        if !items.is_empty() && items.iter().all(|i| i.done));
                let title = if card.title.trim().is_empty() {
                    searchable_body(card).lines().next().unwrap_or("").chars().take(60).collect()
                } else {
                    card.title.clone()
                };
                out.push(TaskItem {
                    node: node.id,
                    node_title: node.title.clone(),
                    card: card.id,
                    title,
                    due: due.clone(),
                    due_days: parse_ymd(due),
                    done,
                });
            }
        }
        out
    }

    /// Cards that have property `key` (optionally `= value`, case-insensitive),
    /// each as a search hit whose snippet is `key: value`.
    pub fn cards_with_property(&self, key: &str, value: Option<&str>) -> Vec<SearchHit> {
        let key = key.to_lowercase();
        let want_val = value.map(|v| v.to_lowercase());
        let mut hits = Vec::new();
        for node in self.nodes.values() {
            for card in &node.cards {
                let hay = format!("{}\n{}", card.title, searchable_body(card));
                let props = extract_properties(&hay);
                if let Some((_, v)) = props.iter().find(|(k, v)| {
                    *k == key && want_val.as_ref().map_or(true, |w| v.to_lowercase() == *w)
                }) {
                    hits.push(SearchHit {
                        node: node.id,
                        card: Some(card.id),
                        node_title: node.title.clone(),
                        snippet: format!("{key}: {v}"),
                    });
                }
            }
        }
        hits
    }
}

/// Targets of `[[wiki-links]]` in `text`, in order. `[[Target|Display]]` yields
/// `Target`. Whitespace-trimmed; empties skipped.
pub(crate) fn extract_wikilinks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'[' && b[i + 1] == b'[' {
            if let Some(end) = text[i + 2..].find("]]") {
                let inner = &text[i + 2..i + 2 + end];
                let target = inner.split('|').next().unwrap_or("").trim();
                if !target.is_empty() {
                    out.push(target.to_string());
                }
                i = i + 2 + end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Rewrite `[[Target]]` / `[[Target|Display]]` into Markdown links
/// `[Display](trellis:<encoded target>)` so the card renderer shows them as
/// clickable links; the app intercepts the `trellis:` scheme to navigate.
pub fn wikilinks_to_md(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if i + 1 < b.len() && b[i] == b'[' && b[i + 1] == b'[' {
            if let Some(end) = text[i + 2..].find("]]") {
                let inner = &text[i + 2..i + 2 + end];
                let mut parts = inner.splitn(2, '|');
                let target = parts.next().unwrap_or("").trim();
                let display = parts.next().map(|d| d.trim()).filter(|d| !d.is_empty()).unwrap_or(target);
                if !target.is_empty() {
                    out.push_str(&format!("[{display}](trellis:{})", encode_link(target)));
                    i = i + 2 + end + 2;
                    continue;
                }
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Percent-encode the bytes that would break a Markdown link URL (spaces,
/// parens, and non-ASCII). Enough for round-tripping a node title through a URL.
fn encode_link(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Decode a `%XX` percent-encoded string produced by [`encode_link`].
pub fn decode_link(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Extract inline `key:: value` properties from `text` (Dataview-style). The
/// `::` must be followed by a space, which keeps code like `std::fmt` and URLs
/// from being mistaken for properties. Two forms are recognized:
/// a whole line `due:: 2026-08-15`, and a bracketed inline `[due:: 2026-08-15]`
/// (or with parens) so a property can sit inside other text. Keys are lowercased;
/// values are trimmed. Order is preserved; a later value for a key wins in
/// [`Document::card_property`].
pub(crate) fn extract_properties(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let b = line.as_bytes();
        // Pass 1: find every `key:: ` marker on the line.
        // (key_start, colon_index, value_start, opener_byte)
        let mut marks: Vec<(usize, usize, usize, u8)> = Vec::new();
        let mut i = 0;
        while i + 2 < b.len() {
            if b[i] == b':' && b[i + 1] == b':' && (b[i + 2] == b' ' || b[i + 2] == b'\t') {
                let mut ks = i;
                while ks > 0 {
                    let c = b[ks - 1];
                    if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                        ks -= 1;
                    } else {
                        break;
                    }
                }
                if ks < i {
                    let opener = if ks > 0 { b[ks - 1] } else { 0 };
                    let mut vs = i + 2;
                    while vs < b.len() && (b[vs] == b' ' || b[vs] == b'\t') {
                        vs += 1;
                    }
                    marks.push((ks, i, vs, opener));
                    i = vs; // keep scanning the value so later fields are found too
                    continue;
                }
            }
            i += 1;
        }
        // Pass 2: each field's value runs to the next field, a closing bracket
        // (if it was opened with `[`/`(`), or the end of the line.
        for (mi, &(ks, ci, vs, opener)) in marks.iter().enumerate() {
            let key = line[ks..ci].to_lowercase();
            let ve = if opener == b'[' || opener == b'(' {
                let close = if opener == b'[' { ']' } else { ')' };
                line[vs..].find(close).map(|p| vs + p).unwrap_or(line.len())
            } else if mi + 1 < marks.len() {
                marks[mi + 1].0
            } else {
                line.len()
            };
            let value = line[vs..ve].trim().to_string();
            if !value.is_empty() {
                out.push((key, value));
            }
        }
    }
    out
}

/// Extract `#tag` tokens from `text`, lowercased and de-duplicated. A tag starts
/// at a `#` on a word boundary (not mid-word, so URL fragments like `page#frag`
/// are ignored) whose first character is a letter (so a Markdown `# Heading` and
/// a bare `#123` are not tags). It runs over letters, digits, `-`, `_`, and `/`
/// (the last allows nested tags like `#work/urgent`).
pub(crate) fn extract_tags(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            let boundary = i == 0 || {
                let p = bytes[i - 1];
                !(p.is_ascii_alphanumeric() || p == b'#')
            };
            let start = i + 1;
            if boundary && start < bytes.len() && bytes[start].is_ascii_alphabetic() {
                let mut j = start;
                while j < bytes.len() {
                    let c = bytes[j];
                    if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'/' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                let tag = text[start..j].to_lowercase();
                if !out.contains(&tag) {
                    out.push(tag);
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

pub struct SearchHit {
    pub node: NodeId,
    /// The specific card that matched, so callers can reveal it (recenter +
    /// flash), not just its basket. `None` for a node-title match, which has no
    /// card to point at.
    pub card: Option<CardId>,
    pub node_title: String,
    pub snippet: String,
}

/// A card carrying a `due::` date, surfaced by [`Document::tasks`] for the agenda.
pub struct TaskItem {
    pub node: NodeId,
    pub node_title: String,
    pub card: CardId,
    pub title: String,
    /// The raw `due` value as written (e.g. `2026-08-15`).
    pub due: String,
    /// `due` parsed to days since the Unix epoch, or `None` if unparseable.
    pub due_days: Option<i64>,
    pub done: bool,
}

/// A card on the Kanban board — its status column plus the bits the board shows.
pub struct KanbanCard {
    pub node: NodeId,
    pub card: CardId,
    pub title: String,
    pub node_title: String,
    /// The card's accent color `[r,g,b]` (shown as the card's border on the board).
    pub color: [u8; 3],
    /// The `due::` value if the card has one (e.g. `2026-08-15`).
    pub due: Option<String>,
    /// `#tags` on the card, in first-seen order.
    pub tags: Vec<String>,
}

/// Parse a `YYYY-MM-DD` date to days since 1970-01-01 (UTC), or `None`. Uses
/// Howard Hinnant's days-from-civil algorithm (inverse of the stamp formatter).
pub fn parse_ymd(s: &str) -> Option<i64> {
    let mut it = s.trim().splitn(3, '-');
    let y: i64 = it.next()?.trim().parse().ok()?;
    let m: i64 = it.next()?.trim().parse().ok()?;
    let d: i64 = it.next()?.trim().parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
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
    /// Inline images referenced by the body (`![alt](trellis:N)`), embedded so
    /// the card file (and any template built from it) is self-contained.
    #[serde(default)]
    pub inline_images: Vec<ImageEntry>,
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

/// A Text card's body with inline-image markers rewritten to self-contained
/// `data:` URIs, for the HTML and Markdown exporters (so the file needs no
/// sidecar images). Unreferenced markers collapse to their alt text.
fn inline_body_with_data_uris(card: &Card) -> String {
    map_inline_images(&card.body, |alt, n| match card.inline_images.get(n) {
        Some(e) => {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&e.data);
            let mime = mime_for(&e.name);
            format!("![{alt}](data:{mime};base64,{b64})")
        }
        None => alt.to_string(),
    })
}

fn searchable_body(card: &Card) -> String {
    match &card.kind {
        CardKind::Text => strip_inline_markers(&card.body),
        CardKind::Code { .. } => card.body.clone(),
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
        CardKind::Text => s.push_str(&md_to_html(&inline_body_with_data_uris(card))),
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
            s.push_str(inline_body_with_data_uris(card).trim_end());
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
            // The selectable text layer carries the words; inline images become
            // their alt text (the picture shows on the WYSIWYG screenshot page).
            let stripped = strip_inline_markers(&card.body);
            let body = stripped.trim_end();
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

/// One page of a basket's visual PDF export: a screenshot (RGBA) plus optional
/// selectable text (a card's content). Assembled by [`basket_pdf`].
pub struct ShotPage {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
    pub title: String,
    pub text: String,
}

/// Build a basket PDF: each [`ShotPage`] becomes an A4 page with its screenshot
/// at the top (a real visual of the basket / card) and the card's text below it
/// as a genuine, selectable/searchable text layer — overflowing text flows onto
/// further pages. The first page is normally the whole-basket overview.
pub fn basket_pdf(pages: &[ShotPage]) -> Result<Vec<u8>, String> {
    use printpdf::{ColorBits, ColorSpace, Image, ImageTransform, ImageXObject, Mm, PdfDocument, Px};
    if pages.is_empty() {
        return Err("nothing to export".to_string());
    }
    let font_ab = ab_glyph::FontRef::try_from_slice(EXPORT_FONT).map_err(|e| e.to_string())?;
    let (w_mm, h_mm, margin) = (210.0_f32, 297.0_f32, 16.0_f32);
    const MM_TO_PT: f32 = 2.834_646;
    let content_w_mm = w_mm - margin * 2.0;
    let content_w_pt = content_w_mm * MM_TO_PT;

    let (doc, mut cur_page, mut cur_layer) =
        PdfDocument::new("Trellis basket", Mm(w_mm), Mm(h_mm), "Layer 1");
    let font =
        doc.add_external_font(std::io::Cursor::new(EXPORT_FONT)).map_err(|e| e.to_string())?;

    for (pi, page) in pages.iter().enumerate() {
        if pi > 0 {
            let (p, l) = doc.add_page(Mm(w_mm), Mm(h_mm), "Layer");
            cur_page = p;
            cur_layer = l;
        }
        let mut y = h_mm - margin;

        // Heading (selectable).
        if !page.title.is_empty() {
            let layer = doc.get_page(cur_page).get_layer(cur_layer);
            layer.use_text(&page.title, 15.0, Mm(margin), Mm(y - 5.0), &font);
            y -= 9.0;
        }

        // Screenshot: fit content width, cap height, placed just under the heading.
        if page.w > 0 && page.h > 0 && page.rgba.len() >= (page.w * page.h * 4) as usize {
            let mut rgb = Vec::with_capacity((page.w * page.h * 3) as usize);
            for px in page.rgba.chunks_exact(4) {
                let a = px[3] as u32;
                let over = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
                rgb.extend_from_slice(&[over(px[0]), over(px[1]), over(px[2])]);
            }
            let aspect = page.h as f32 / page.w as f32;
            let mut draw_w = content_w_mm;
            let mut draw_h = draw_w * aspect;
            // Leave the lower part of the page for the text layer.
            let max_h = (y - margin) * if page.text.is_empty() { 1.0 } else { 0.66 };
            if max_h > 0.0 && draw_h > max_h {
                draw_h = max_h;
                draw_w = draw_h / aspect;
            }
            let dpi = page.w as f32 / (draw_w / 25.4);
            let img_bottom = (y - draw_h).max(margin);
            let xobj = ImageXObject {
                width: Px(page.w as usize),
                height: Px(page.h as usize),
                color_space: ColorSpace::Rgb,
                bits_per_component: ColorBits::Bit8,
                interpolate: false,
                image_data: rgb,
                image_filter: None,
                smask: None,
                clipping_bbox: None,
            };
            Image::from(xobj).add_to_layer(
                doc.get_page(cur_page).get_layer(cur_layer),
                ImageTransform {
                    translate_x: Some(Mm(margin)),
                    translate_y: Some(Mm(img_bottom)),
                    dpi: Some(dpi),
                    ..Default::default()
                },
            );
            y = img_bottom - 7.0;
        }

        // Selectable body text below the image; paginates when it overflows.
        if !page.text.is_empty() {
            let size = 11.0;
            let leading = (size * 1.4) / MM_TO_PT;
            let mut tl = doc.get_page(cur_page).get_layer(cur_layer);
            for raw in page.text.lines() {
                let wrapped = if raw.is_empty() {
                    vec![String::new()]
                } else {
                    wrap_text(&font_ab, size, raw, content_w_pt)
                };
                for line in wrapped {
                    if y < margin {
                        let (p, l) = doc.add_page(Mm(w_mm), Mm(h_mm), "Layer");
                        cur_page = p;
                        cur_layer = l;
                        tl = doc.get_page(cur_page).get_layer(cur_layer);
                        y = h_mm - margin;
                    }
                    if !line.is_empty() {
                        tl.use_text(&line, size, Mm(margin), Mm(y), &font);
                    }
                    y -= leading;
                }
            }
        }
    }
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
    fn basket_pdf_embeds_image_and_text() {
        // A tiny white screenshot + selectable text → a valid, multi-page PDF.
        let rgba = vec![255u8; 4 * 4 * 4];
        let pages = vec![
            ShotPage { rgba: rgba.clone(), w: 4, h: 4, title: "Overview".into(), text: String::new() },
            ShotPage {
                rgba,
                w: 4,
                h: 4,
                title: "Card A".into(),
                text: "hello searchable world".into(),
            },
        ];
        let bytes = basket_pdf(&pages).unwrap();
        assert!(bytes.starts_with(b"%PDF"), "produces a PDF");
        assert!(bytes.len() > 200);
        assert!(basket_pdf(&[]).is_err(), "empty export is an error");
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

    /// A tiny encoded PNG of the given size, for inline-image tests.
    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    #[test]
    fn parse_ymd_matches_known_dates() {
        assert_eq!(parse_ymd("1970-01-01"), Some(0));
        assert_eq!(parse_ymd("2026-07-30"), Some(20664)); // 20664 days after epoch
        assert_eq!(parse_ymd("2026-07-31").unwrap(), parse_ymd("2026-07-30").unwrap() + 1);
        assert_eq!(parse_ymd("not-a-date"), None);
        assert_eq!(parse_ymd("2026-13-01"), None); // bad month
    }

    #[test]
    fn tasks_and_query_gather_across_baskets() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "Project".into());
        let b = doc.add_node(None, "Daily".into());
        let c1 = doc.add_card(a, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(a, c1).unwrap().title = "Ship release".into();
        doc.card_mut(a, c1).unwrap().body = "#todo due:: 2026-08-15".into();
        let c2 = doc.add_card(b, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(b, c2).unwrap().body = "call vendor due:: 2026-08-01 status:: done".into();

        let tasks = doc.tasks();
        assert_eq!(tasks.len(), 2);
        let done: Vec<_> = tasks.iter().filter(|t| t.done).collect();
        assert_eq!(done.len(), 1); // the status:: done one
        assert!(tasks.iter().any(|t| t.title == "Ship release" && !t.done));

        // Combined query: tag AND property.
        assert_eq!(doc.query_cards(Some("todo"), Some("due"), None, None).len(), 1);
        assert_eq!(doc.query_cards(Some("todo"), None, None, Some("ship")).len(), 1);
        assert_eq!(doc.query_cards(Some("nope"), None, None, None).len(), 0);
        // No filters -> nothing.
        assert_eq!(doc.query_cards(None, None, None, None).len(), 0);
    }

    #[test]
    fn set_card_property_replaces_or_appends() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "N".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c).unwrap().body = "ship it\nstatus:: todo".into();
        // Replace an existing property.
        assert!(doc.set_card_property(n, c, "status", "doing"));
        assert_eq!(doc.card_property(n, c, "status").as_deref(), Some("doing"));
        // Append a new one.
        doc.set_card_property(n, c, "priority", "high");
        assert_eq!(doc.card_property(n, c, "priority").as_deref(), Some("high"));
        // Board groups by status.
        let board = doc.cards_by_status();
        assert_eq!(board.get("doing").map(|v| v.len()), Some(1));
        assert!(board.get("todo").is_none());
    }

    #[test]
    fn wikilinks_parse_render_and_backlink() {
        assert_eq!(extract_wikilinks("see [[Project Falcon]] and [[42|the plan]]"),
            vec!["Project Falcon".to_string(), "42".to_string()]);
        let md = wikilinks_to_md("go to [[Project Falcon]]!");
        assert_eq!(md, "go to [Project Falcon](trellis:Project%20Falcon)!");
        assert_eq!(decode_link("Project%20Falcon"), "Project Falcon");

        let mut doc = Document::empty();
        let target = doc.add_node(None, "Project Falcon".into());
        let other = doc.add_node(None, "Daily".into());
        let c = doc.add_card(other, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(other, c).unwrap().body = "notes about [[Project Falcon]]".into();
        assert_eq!(doc.resolve_link("Project Falcon"), Some(target));
        assert_eq!(doc.resolve_link(&target.to_string()), Some(target));
        assert_eq!(doc.resolve_link("nope"), None);
        let bl = doc.backlinks(target);
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].node, other);
    }

    #[test]
    fn extract_properties_parses_fields_not_code() {
        let p = extract_properties("due:: 2026-08-15\npriority:: high\nsee std::fmt for [status:: done] end");
        assert!(p.contains(&("due".to_string(), "2026-08-15".to_string())));
        assert!(p.contains(&("priority".to_string(), "high".to_string())));
        // bracketed inline works and stops at ']'
        assert!(p.contains(&("status".to_string(), "done".to_string())));
        // `std::fmt` (no space after ::) is NOT a property
        assert!(!p.iter().any(|(k, _)| k == "std"));
    }

    #[test]
    fn document_property_queries() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "N".into());
        let c1 = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c1).unwrap().body = "ship it\ndue:: 2026-08-15\nstatus:: open".into();
        let c2 = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c2).unwrap().body = "later\ndue:: 2026-09-01".into();
        assert_eq!(doc.card_property(n, c1, "due").as_deref(), Some("2026-08-15"));
        let keys: std::collections::HashMap<_, _> = doc.property_keys().into_iter().collect();
        assert_eq!(keys.get("due"), Some(&2));
        assert_eq!(keys.get("status"), Some(&1));
        assert_eq!(doc.cards_with_property("due", None).len(), 2);
        assert_eq!(doc.cards_with_property("status", Some("open")).len(), 1);
        assert_eq!(doc.cards_with_property("status", Some("closed")).len(), 0);
    }

    #[test]
    fn extract_tags_finds_real_tags_only() {
        let t = extract_tags("meeting notes #Work #work/urgent see http://x/page#frag and # Heading (#todo) end #123");
        // #Work and #work are the same tag (lowercased, deduped); nested kept.
        assert_eq!(t, vec!["work".to_string(), "work/urgent".to_string(), "todo".to_string()]);
        // URL fragment (page#frag), Markdown heading (# Heading), and #123 excluded.
        assert!(!t.contains(&"frag".to_string()));
        assert!(!t.iter().any(|x| x == "heading"));
        assert!(!t.iter().any(|x| x == "123"));
    }

    #[test]
    fn tag_counts_and_cards_with_tag_span_the_tree() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "A".into());
        let b = doc.add_node(None, "B".into());
        let c1 = doc.add_card(a, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(a, c1).unwrap().body = "fix the #bug today".into();
        let c2 = doc.add_card(b, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(b, c2).unwrap().body = "another #bug and a #idea".into();
        let counts: std::collections::HashMap<_, _> = doc.tag_counts().into_iter().collect();
        assert_eq!(counts.get("bug"), Some(&2));
        assert_eq!(counts.get("idea"), Some(&1));
        // cards_with_tag gathers across baskets (with or without the #).
        assert_eq!(doc.cards_with_tag("#bug").len(), 2);
        assert_eq!(doc.cards_with_tag("idea").len(), 1);
    }

    #[test]
    fn inline_markers_map_strip_and_collect_refs() {
        let body = "before ![a cat](trellis:0) middle ![](trellis:2) after ![x](trellis:notnum)";
        // Refs are collected in order; the malformed one is ignored.
        assert_eq!(inline_refs(body), vec![0, 2]);
        // Strip reduces markers to their alt text (empty alt disappears).
        assert_eq!(
            strip_inline_markers(body),
            "before a cat middle  after ![x](trellis:notnum)"
        );
        // map lets a caller splice arbitrary replacements.
        let mapped = map_inline_images(body, |alt, n| format!("<{n}:{alt}>"));
        assert!(mapped.contains("<0:a cat>") && mapped.contains("<2:>"));
    }

    #[test]
    fn fit_size_text_grows_for_inline_image() {
        let mut plain = Card::new(1, egui::pos2(0.0, 0.0), CardKind::Text);
        plain.body = "a short note".into();
        let base = plain.fit_size().unwrap();

        let mut withimg = Card::new(2, egui::pos2(0.0, 0.0), CardKind::Text);
        withimg.body = "a short note\n\n![pic](trellis:0)".into();
        withimg.inline_images = vec![ImageEntry { data: png_bytes(300, 200), name: "pic.png".into() }];
        let grown = withimg.fit_size().unwrap();

        // The 200px-tall image forces the card materially taller than text alone.
        assert!(grown.y > base.y + 150.0, "image should add height: {base:?} -> {grown:?}");
    }

    #[test]
    fn card_export_round_trips_inline_images() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let idx = doc.add_inline_image(n, cid, png_bytes(20, 10), "p.png".into()).unwrap();
        doc.card_mut(n, cid).unwrap().body = format!("see ![p](trellis:{idx})");

        let json = doc.export_card_json(n, cid).unwrap();
        let exp = parse_card_export(&json).expect("valid card export");
        assert_eq!(exp.inline_images.len(), 1);

        let m = doc.add_node(None, "m".into());
        let cid2 = doc.add_card_from_export(m, egui::pos2(5.0, 5.0), exp).unwrap();
        assert_eq!(doc.card(m, cid2).unwrap().inline_images.len(), 1);
    }

    #[test]
    fn inline_image_html_export_embeds_data_uri() {
        let mut card = Card::new(1, egui::pos2(0.0, 0.0), CardKind::Text);
        card.body = "look: ![pic](trellis:0)".into();
        card.inline_images = vec![ImageEntry { data: png_bytes(4, 4), name: "pic.png".into() }];
        let html = card_body_html(&card);
        assert!(html.contains("<img"), "renders an <img>: {html}");
        assert!(html.contains("data:image/png;base64,"), "embeds a data URI: {html}");
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
