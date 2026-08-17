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
/// Identifies one checklist item within the document. Unique like a card id,
/// so an item can be linked to and tracked on its own.
pub type ItemId = u64;
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
///
/// The `id` is what makes a checklist line a *thing* rather than a position.
/// Without it an item is identified by its index, so reordering the list silently
/// renames every task in it, a link can't point at one, the change log can't say
/// which moved, and nothing can follow an item across time. Identity is the
/// prerequisite for treating items as tasks at all.
///
/// `#[serde(default)]` means every checklist saved before this loads with `id: 0`
/// — the "not assigned yet" value — and [`Document::ensure_item_ids`] fills them
/// in on load. Old documents are never rewritten by hand.
#[derive(Clone, Serialize, Deserialize)]
pub struct ChecklistItem {
    #[serde(default)]
    pub id: ItemId,
    pub done: bool,
    pub text: String,
}

impl ChecklistItem {
    /// A new item with no id yet — `ensure_item_ids` assigns one. Used where a
    /// card is being built before it belongs to a document.
    pub fn new(text: impl Into<String>) -> Self {
        ChecklistItem { id: 0, done: false, text: text.into() }
    }
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
    /// Draw this table as a chart instead of a grid. `None` = plain table, which
    /// is what every table saved before charts existed deserializes to.
    #[serde(default)]
    pub chart: Option<ChartSpec>,
    /// Conditional formatting: colour cells by their value. Re-applied after
    /// every refresh of a `source`-backed table, so live data stays coloured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<CellRule>,
}

/// Colour a column's cells by what they contain.
///
/// Deliberately a small, explicit rule list rather than a formula language: the
/// point is "colour column 2 by value", and every extra capability here is one
/// more thing to specify, document and get wrong.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellRule {
    /// Column index this rule tests. `None` = every column.
    #[serde(default)]
    pub col: Option<usize>,
    /// `gt` `lt` `ge` `le` `eq` `ne` `contains` `empty` `not_empty`.
    pub when: String,
    /// The value compared against. Numeric comparisons use the same decorated
    /// parser as charts (`1,234.5`, `$12`, `40%`, `(3)` = −3) so a table and its
    /// chart never disagree about what a cell means.
    ///
    /// Accepts a JSON **number or string** — `"value": 1000` is what anyone
    /// writing a numeric threshold types, and rejecting it would be pedantry.
    #[serde(default, deserialize_with = "de_scalar_string")]
    pub value: String,
    #[serde(default)]
    pub bg: Option<[u8; 3]>,
    #[serde(default)]
    pub fg: Option<[u8; 3]>,
}

/// Deserialize a number, string or bool into a `String`, so a numeric threshold
/// can be written as a number.
fn de_scalar_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
}

impl CellRule {
    /// Whether this rule matches `text`.
    pub fn matches(&self, text: &str) -> bool {
        let t = text.trim();
        match self.when.as_str() {
            "empty" => t.is_empty(),
            "not_empty" => !t.is_empty(),
            "contains" => t.to_lowercase().contains(&self.value.trim().to_lowercase()),
            "eq" | "ne" => {
                // Compare as numbers when both sides look numeric, so `1,200`
                // and `1200` match; fall back to case-insensitive text.
                let same = match (parse_number(t), parse_number(&self.value)) {
                    (Some(a), Some(b)) => (a - b).abs() < f64::EPSILON,
                    _ => t.eq_ignore_ascii_case(self.value.trim()),
                };
                if self.when == "eq" { same } else { !same }
            }
            "gt" | "lt" | "ge" | "le" => {
                // A non-numeric cell matches no ordering rule — it has no
                // position on the scale, and guessing one would colour a header
                // or a blank as though it were data.
                let (Some(a), Some(b)) = (parse_number(t), parse_number(&self.value)) else {
                    return false;
                };
                match self.when.as_str() {
                    "gt" => a > b,
                    "lt" => a < b,
                    "ge" => a >= b,
                    _ => a <= b,
                }
            }
            _ => false,
        }
    }
}

/// Parse a spreadsheet-ish number: `1,234.5`, `$12`, `40%`, `(3)` = -3.
/// Returns `None` for anything that isn't a number, so the caller can treat it
/// as a gap rather than a zero.
pub fn parse_number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let negated = t.starts_with('(') && t.ends_with(')');
    let core: String = t
        .trim_start_matches('(')
        .trim_end_matches(')')
        .chars()
        .filter(|c| !matches!(c, ',' | '$' | '£' | '€' | '%' | ' ' | '\u{a0}'))
        .collect();
    let v: f64 = core.parse().ok()?;
    if !v.is_finite() {
        return None;
    }
    Some(if negated { -v } else { v })
}

/// How a table is drawn as a chart. The table stays the single source of the
/// data — this only says how to read it, so editing a cell updates the chart.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct ChartSpec {
    pub kind: ChartKind,
    /// Column supplying each point's label / x-axis category (0-based).
    #[serde(default)]
    pub label_col: usize,
    /// Columns plotted as series. Empty = every numeric column except
    /// `label_col`, so a chart keeps working when you add a column.
    #[serde(default)]
    pub value_cols: Vec<usize>,
    /// Show the source grid under the chart as well.
    #[serde(default)]
    pub show_table: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    Bar,
    Line,
    Scatter,
    /// Proportions of a whole. Unlike the others this plots a **single** series
    /// (the first one), because a pie can only show one set of parts.
    Pie,
}

impl ChartKind {
    pub const ALL: [ChartKind; 4] =
        [ChartKind::Bar, ChartKind::Line, ChartKind::Scatter, ChartKind::Pie];

    pub fn label(self) -> &'static str {
        match self {
            ChartKind::Bar => "Bar",
            ChartKind::Line => "Line",
            ChartKind::Scatter => "Scatter",
            ChartKind::Pie => "Pie",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            ChartKind::Bar => "bar",
            ChartKind::Line => "line",
            ChartKind::Scatter => "scatter",
            ChartKind::Pie => "pie",
        }
    }

    /// Parse an API/serde value; `None` for anything unknown so callers can
    /// report a useful error instead of silently picking a chart type.
    pub fn from_key(s: &str) -> Option<ChartKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bar" => Some(ChartKind::Bar),
            "line" => Some(ChartKind::Line),
            "scatter" | "points" => Some(ChartKind::Scatter),
            "pie" | "doughnut" | "donut" => Some(ChartKind::Pie),
            _ => None,
        }
    }
}

impl Default for ChartSpec {
    fn default() -> Self {
        ChartSpec {
            kind: ChartKind::Bar,
            label_col: 0,
            value_cols: Vec::new(),
            show_table: false,
        }
    }
}

/// Tallest a card may be sized to by "Fit to content" / the API's `fit`.
///
/// A guard against one runaway card, not a layout opinion. It used to be 1400,
/// which a long note exceeds easily — and because a text card has no per-card
/// scroll, exceeding it meant the bottom was **silently clipped** rather than
/// merely tall. Clipping content is the worse failure, so the cap is generous.
pub const FIT_MAX_H: f32 = 6000.0;

pub const TABLE_DEFAULT_COL_W: f32 = 110.0;
/// Narrowest and widest a column may be set to, by drag, by `set_col_width`, or
/// by autofit.
pub const TABLE_MIN_COL_W: f32 = 28.0;
pub const TABLE_MAX_COL_W: f32 = 600.0;
/// Table cells render at the plain body text style — a table ignores the card's
/// `font_scale`, unlike a text or code card.
const TABLE_FONT_PX: f32 = 12.5;

/// Roughly how wide `s` renders in the proportional body font, in pixels.
///
/// A character *count* can't do this job: `WWWWW` is about three times the width
/// of `iiiii`, so a flat average sized a column of capitals too narrow and
/// clipped it — which is the exact failure this whole feature exists to remove.
/// So each character contributes its own approximate advance, as a fraction of
/// the font size. It errs generous: a column a few pixels roomier than needed
/// reads fine, one a few pixels short does not.
fn cell_text_width(s: &str) -> f32 {
    let ems: f32 = s
        .chars()
        .map(|c| match c {
            'i' | 'l' | 'j' | 'I' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' | '`' => 0.32,
            ' ' | 'f' | 'r' | 't' | '(' | ')' | '[' | ']' | '-' => 0.42,
            'm' | 'w' | 'M' | 'W' | '@' | '%' => 1.02,
            'A'..='Z' => 0.78,
            '0'..='9' => 0.62,
            // Ordinary lowercase. Erring a little high matters most here: the
            // shortfall is per character, so it only shows up on the longest
            // string in the column — exactly the one that must not clip.
            _ => 0.62,
        })
        .sum();
    ems * TABLE_FONT_PX
}

impl TableData {
    /// The width column `c` needs for its longest cell to render without being
    /// clipped.
    ///
    /// An estimate, like [`Card::fit_size`] — the API worker has no egui font
    /// context to measure with — so it deliberately errs *wide*: a column a few
    /// pixels too generous is merely roomy, while one a few pixels short clips
    /// the text, which is the whole problem this solves. Cell text renders at
    /// the plain body font (a table ignores the card's `font_scale`, unlike a
    /// text or code card) and is inset 4px on each side, drawn with
    /// `layout_no_wrap` — so the width a cell needs is its text plus that
    /// padding, with no wrapping to fall back on.
    pub fn autofit_width(&self, c: usize) -> f32 {
        const CELL_PAD: f32 = 14.0; // the 4px inset each side, plus slack
        // Never narrower than this: a one-character column shrunk to the hard
        // minimum reads as a rendering fault rather than a narrow column.
        const MIN_READABLE: f32 = 48.0;

        let widest = self
            .rows
            .iter()
            .filter_map(|r| r.get(c))
            .map(|cell| cell_text_width(&cell.text))
            .fold(0.0_f32, f32::max);
        (widest + CELL_PAD).clamp(MIN_READABLE, TABLE_MAX_COL_W)
    }

    /// A fresh `rows` x `cols` empty table.
    pub fn empty(rows: usize, cols: usize) -> Self {
        TableData {
            rows: vec![vec![TableCell::default(); cols]; rows],
            col_widths: Vec::new(),
            header: true,
            chart: None,
            rules: Vec::new(),
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
        TableData { rows, col_widths: Vec::new(), header: true, chart: None, rules: Vec::new() }
    }

    /// Read the table as chart data: `(labels, series)` where each series is a
    /// column name and one value per label row. Non-numeric cells become gaps
    /// (`None`) rather than zeros — a blank cell in a status grid is "no data",
    /// and plotting it as 0 would invent a reading that isn't there.
    ///
    /// Numbers may carry the usual decoration: thousands separators, a leading
    /// currency symbol, a trailing `%`, and parenthesised negatives.
    pub fn chart_data(&self, spec: &ChartSpec) -> (Vec<String>, Vec<(String, Vec<Option<f64>>)>) {
        let body_start = if self.header { 1 } else { 0 };
        let n_cols = self.n_cols();
        let label_col = spec.label_col.min(n_cols.saturating_sub(1));

        // Which columns to plot: the explicit list, else every column that has
        // at least one parseable number in the body (excluding the label column).
        let cols: Vec<usize> = if spec.value_cols.is_empty() {
            (0..n_cols)
                .filter(|c| *c != label_col)
                .filter(|c| {
                    self.rows
                        .iter()
                        .skip(body_start)
                        .any(|r| r.get(*c).and_then(|cell| parse_number(&cell.text)).is_some())
                })
                .collect()
        } else {
            spec.value_cols.iter().copied().filter(|c| *c < n_cols).collect()
        };

        let labels: Vec<String> = self
            .rows
            .iter()
            .skip(body_start)
            .enumerate()
            .map(|(i, r)| {
                let t = r.get(label_col).map(|c| c.text.trim()).unwrap_or("");
                if t.is_empty() {
                    format!("{}", i + 1)
                } else {
                    t.to_string()
                }
            })
            .collect();

        let series = cols
            .into_iter()
            .map(|c| {
                let name = if self.header {
                    self.rows
                        .first()
                        .and_then(|r| r.get(c))
                        .map(|cell| cell.text.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("Column {}", c + 1))
                } else {
                    format!("Column {}", c + 1)
                };
                let vals = self
                    .rows
                    .iter()
                    .skip(body_start)
                    .map(|r| r.get(c).and_then(|cell| parse_number(&cell.text)))
                    .collect();
                (name, vals)
            })
            .collect();

        (labels, series)
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

/// How image bytes are stored in a saved document.
///
/// Serde's default for `Vec<u8>` is a sequence, which RON writes as a decimal
/// list — `data:[137,80,78,71,…]` — so every byte costs about 3.5 characters.
/// On a document with real screenshots in it that dominates everything else: a
/// 16 MB set of images was occupying 56 MB of a 60 MB document, and gzip then
/// spent seconds undoing the bloat on every save.
///
/// Written as **base64** (1.33× instead of 3.5×). Read as *either*, so every
/// document, template, basket export and history snapshot written before this
/// change still loads untouched — the decimal form is simply the other arm of
/// the visitor.
mod image_bytes {
    use base64::Engine as _;
    use serde::de::{Error as DeError, SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Vec<u8>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("image bytes: base64 text, or a list of byte values")
            }
            /// Current form.
            fn visit_str<E: DeError>(self, s: &str) -> Result<Vec<u8>, E> {
                base64::engine::general_purpose::STANDARD.decode(s).map_err(E::custom)
            }
            /// Pre-base64 documents: a decimal array.
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Vec<u8>, A::Error> {
                let mut out = Vec::with_capacity(a.size_hint().unwrap_or(0));
                while let Some(b) = a.next_element::<u8>()? {
                    out.push(b);
                }
                Ok(out)
            }
            /// A self-describing format that carries real bytes (not RON/JSON,
            /// but cheap to accept and it keeps the visitor total).
            fn visit_bytes<E: DeError>(self, b: &[u8]) -> Result<Vec<u8>, E> {
                Ok(b.to_vec())
            }
            fn visit_byte_buf<E: DeError>(self, b: Vec<u8>) -> Result<Vec<u8>, E> {
                Ok(b)
            }
        }
        d.deserialize_any(V)
    }
}

/// One additional image of an Image card. The first image lives in the
/// variant's `data`/`name` fields so pre-multi-image documents load unchanged.
#[derive(Clone, Serialize, Deserialize)]
pub struct ImageEntry {
    #[serde(with = "image_bytes")]
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
        #[serde(with = "image_bytes")]
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
    /// Depth, in the **same units as `pos`** — positive is toward the viewer.
    ///
    /// A basket is a volume, not a plane. With **Depth** off this is the
    /// stacking order and nothing more, so the coordinate never becomes
    /// meaningless; with it on, this is a real position and the camera projects
    /// it. Same units as x/y deliberately: "200 nearer" is then the same size of
    /// move as "200 right", which is what lets an agent reason about depth with
    /// the arithmetic it already uses for position.
    ///
    /// `serde(default)` = every document written before depth existed loads with
    /// every card at `0.0`, i.e. coplanar, i.e. exactly what it looks like today.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub z: f32,
    /// Attention: none, a steady glow, or a slow pulse. See [`Emphasis`].
    #[serde(default, skip_serializing_if = "is_no_emphasis")]
    pub emphasis: Emphasis,
    /// How strong the halo is, 0.0–1.0. Clamped on read as well as on write.
    #[serde(default = "default_intensity", skip_serializing_if = "is_full_intensity")]
    pub emphasis_intensity: f32,
    /// Unix seconds after which the emphasis stops being drawn, or `None` for
    /// "until someone turns it off".
    ///
    /// **This is the field that decides whether the feature is any good.** An
    /// agent that can shout permanently produces a document where everything
    /// shouts, and then nothing does. Agents set an expiry; a person setting it
    /// by hand does not have to. Expiry is evaluated at draw time and the field
    /// is left alone — a card that lapsed is not a card that was edited, and
    /// rewriting the document to un-highlight things would touch `touched` and
    /// spam the change log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emphasis_until: Option<i64>,
    pub size: egui::Vec2,
    pub title: String,
    /// Markdown / code text. Unused by image and checklist cards.
    pub body: String,
    /// RGB accent used for the card's title bar.
    pub color: [u8; 3],
    pub kind: CardKind,
    /// When this was last changed, in unix seconds — the only timestamp Trellis
    /// stores. Nothing in the document carried a "when" before, so sorting by
    /// recent activity was impossible; the in-memory change log answers it within
    /// a session but is gone on restart.
    ///
    /// `None` means "not changed since this was added" — every card in a document
    /// written before this existed reads as `None` rather than pretending to a
    /// time nobody recorded. Old builds ignore the field (nothing here sets
    /// `deny_unknown_fields`), so unlike the v0.74.0 storage change this is
    /// readable in both directions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touched: Option<u64>,
    /// A file this card mirrors. When set, `body` is a **read-only cache** of
    /// that file's contents, refreshed while the document is open.
    ///
    /// Deliberately a field on an ordinary text/code card rather than a new
    /// `CardKind`: a new variant costs ~180 exhaustive match arms across the
    /// model, API, canvas and app, and buys nothing here — a mirrored file is
    /// still markdown or still code, and should render, search, export, and carry
    /// `#tags` exactly like any other card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Modification time of `source` when it was last read, so a poll can tell
    /// whether anything changed without re-reading the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_mtime: Option<u64>,
    /// Why the last read failed, or `None` if it succeeded. Kept in the document
    /// so reopening still shows "file not found" rather than silently presenting
    /// stale content as current. `body` keeps the last good read either way —
    /// losing the cached text because a disk was unmounted would be worse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_error: Option<String>,
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

/// How loudly a card asks to be looked at.
///
/// **Why this exists at all.** An agent working in a document has one way to say
/// "this one matters": the accent colour, which is also how a human organises a
/// basket, so using it for attention destroys the organisation. This is a
/// separate channel, and it is deliberately a *small* one.
///
/// **Why it is one field rather than three.** Flashing, pulsing and glowing as
/// independent flags is eight states, most of them meaningless, and every
/// renderer — desktop, phone, PDF — has to answer for all of them.
///
/// **Why there is no flash.** Anything above about 3 Hz is a photosensitive
/// seizure risk, and it is the one visual effect here that can actually hurt
/// somebody. `Pulse` is a slow sine that never reaches zero: it reads as alive
/// rather than as an alarm.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Emphasis {
    #[default]
    None,
    /// A steady accent halo.
    Glow,
    /// The same halo, breathing between 40% and 100%.
    Pulse,
}

impl Emphasis {
    pub fn key(self) -> &'static str {
        match self {
            Emphasis::None => "none",
            Emphasis::Glow => "glow",
            Emphasis::Pulse => "pulse",
        }
    }

    pub fn from_key(s: &str) -> Option<Emphasis> {
        match s {
            "none" | "" => Some(Emphasis::None),
            "glow" => Some(Emphasis::Glow),
            "pulse" => Some(Emphasis::Pulse),
            _ => None,
        }
    }
}

fn is_no_emphasis(e: &Emphasis) -> bool {
    *e == Emphasis::None
}

fn default_intensity() -> f32 {
    1.0
}

fn is_full_intensity(v: &f32) -> bool {
    (*v - 1.0).abs() < f32::EPSILON
}

fn default_font_scale() -> f32 {
    1.0
}

/// Skip `z` when it is zero, so a flat document's file is byte-identical to what
/// it was before depth existed — a diff of a 2-D document should not be noise.
fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

impl Card {
    /// The emphasis actually in force at `now` (unix seconds), after expiry.
    ///
    /// Everything that draws attention goes through this rather than reading the
    /// field, so a lapsed highlight is invisible everywhere at once — canvas,
    /// minimap, phone — without anyone having to remember to check.
    pub fn live_emphasis(&self, now: i64) -> Emphasis {
        match self.emphasis_until {
            Some(t) if now > t => Emphasis::None,
            _ => self.emphasis,
        }
    }

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
            z: 0.0,
            emphasis: Emphasis::None,
            emphasis_intensity: 1.0,
            emphasis_until: None,
            // A brand-new card has never been *changed*; the first edit stamps it.
            touched: None,
            source: None,
            source_mtime: None,
            source_error: None,
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
            let h = (TITLE_H + PAD * 2.0 + content_h).clamp(MIN_H, FIT_MAX_H);
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
        Some(egui::vec2(w.clamp(MIN_W, MAX_W), h.clamp(MIN_H, FIT_MAX_H)))
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
/// ATX heading level of a line (`# ` → 1 … `###### ` → 6), else `None`.
///
/// Used by both height measurements: a heading renders **larger than body text**,
/// so measuring every line at the body size under-counts a heading-heavy card and
/// its bottom gets clipped.
pub(crate) fn heading_level(line: &str) -> Option<u8> {
    let t = line.trim_start();
    let hashes = t.bytes().take_while(|b| *b == b'#').count();
    // CommonMark: 1–6 hashes, and a space must follow (`#tag` is not a heading —
    // which matters here, since #tags are used throughout).
    (1..=6).contains(&hashes).then_some(())?;
    t[hashes..].starts_with(' ').then_some(hashes as u8)
}

/// The font size the CommonMark renderer uses for a heading of `level`, given the
/// body and H1 sizes. Mirrors `egui_commonmark_backend`'s `Style::to_richtext`:
/// H1 is the Heading text style, H2–H6 interpolate down towards body by fixed
/// factors. Keep in step with `vendor/egui_commonmark_backend/src/misc.rs`.
pub(crate) fn heading_font_px(level: u8, body_px: f32, heading_px: f32) -> f32 {
    let diff = heading_px - body_px;
    match level {
        1 => heading_px,
        2 => body_px + diff * 0.835,
        3 => body_px + diff * 0.668,
        4 => body_px + diff * 0.501,
        5 => body_px + diff * 0.334,
        _ => body_px + diff * 0.167,
    }
}

/// Estimated height of `text` wrapped at `wrap_w`, counting heading lines at
/// their larger rendered size. `char_w`/`line_h` are the *body* metrics; heading
/// lines scale both by the ratio of their font size to the body's.
fn wrapped_height(text: &str, char_w: f32, line_h: f32, wrap_w: f32) -> f32 {
    // Body size the caller's metrics were derived from, so a heading's scale is
    // relative to it. Matches egui's defaults (Body 12.5 / Heading 18.0).
    const BODY_PX: f32 = 12.5;
    const HEAD_PX: f32 = 18.0;
    let mut total = 0.0f32;
    let mut rows_any = false;
    for line in text.lines() {
        rows_any = true;
        let scale = match heading_level(line) {
            Some(l) => heading_font_px(l, BODY_PX, HEAD_PX) / BODY_PX,
            None => 1.0,
        };
        let cols = (wrap_w / (char_w * scale)).max(1.0);
        let n = line.chars().count() as f32;
        let rows = (n / cols).ceil().max(1.0);
        total += rows * line_h * scale;
        // The renderer forces a newline before every heading.
        if heading_level(line).is_some() {
            total += line_h * 0.5;
        }
    }
    if !rows_any {
        total = line_h;
    }
    total.max(line_h)
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
    /// When this was last changed, in unix seconds — the only timestamp Trellis
    /// stores. Nothing in the document carried a "when" before, so sorting by
    /// recent activity was impossible; the in-memory change log answers it within
    /// a session but is gone on restart.
    ///
    /// `None` means "not changed since this was added" — every card in a document
    /// written before this existed reads as `None` rather than pretending to a
    /// time nobody recorded. Old builds ignore the field (nothing here sets
    /// `deny_unknown_fields`), so unlike the v0.74.0 storage change this is
    /// readable in both directions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub touched: Option<u64>,
}

fn default_true() -> bool {
    true
}



impl TableData {
    /// Replace cell **text** from parsed rows, keeping everything the user or a
    /// previous call set up: column widths, the header flag, the chart spec and
    /// the formatting rules.
    ///
    /// This exists because `from_values` builds a fresh `TableData` and so drops
    /// `col_widths` — fine for a one-off import, ruinous on a `source` refresh
    /// that runs every few seconds, which would re-flatten the columns
    /// continuously while someone was trying to read them.
    pub fn fill_values(&mut self, values: Vec<Vec<String>>) {
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
        self.rows = rows;
        // Widths are per column, so trim if the file lost columns but never
        // grow — a missing entry already means "default width".
        self.col_widths.truncate(cols);
        self.apply_rules();
    }

    /// Re-colour every cell from the rules. First matching rule wins, so the
    /// order a caller sends them in is the order they take effect.
    ///
    /// Cells the rules don't match are **cleared**, not left alone: otherwise a
    /// value that stops being an error would keep its red background forever,
    /// which is worse than no colour at all.
    pub fn apply_rules(&mut self) {
        if self.rules.is_empty() {
            return;
        }
        let header_rows = usize::from(self.header);
        for (r, row) in self.rows.iter_mut().enumerate() {
            if r < header_rows {
                continue; // a header is a label, not a value
            }
            for (c, cell) in row.iter_mut().enumerate() {
                let hit = self
                    .rules
                    .iter()
                    .find(|rule| rule.col.map_or(true, |rc| rc == c) && rule.matches(&cell.text));
                match hit {
                    Some(rule) => {
                        cell.bg = rule.bg;
                        cell.fg = rule.fg;
                    }
                    None => {
                        cell.bg = None;
                        cell.fg = None;
                    }
                }
            }
        }
    }
}

/// Parse a delimited file into rows, picking the delimiter from the extension.
///
/// `.tsv`/`.tab` are tab-separated; everything else is treated as CSV. The `csv`
/// crate handles quoting and embedded newlines either way, so this is a
/// parameter rather than a second parser.
pub fn delimited_to_values(path: &str, text: &str) -> Result<Vec<Vec<String>>, String> {
    let lower = path.to_ascii_lowercase();
    let delim = if lower.ends_with(".tsv") || lower.ends_with(".tab") { b'\t' } else { b',' };
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delim)
        .from_reader(text.as_bytes());
    let mut out = Vec::new();
    for rec in rdr.records() {
        out.push(rec.map_err(|e| e.to_string())?.iter().map(|s| s.to_string()).collect());
    }
    if out.is_empty() {
        return Err(format!("{path} has no rows"));
    }
    Ok(out)
}

/// Largest file a pointer card will mirror.
///
/// The body is held in memory, rendered as markdown, indexed by search and
/// written into the document, so this is a practical ceiling rather than a
/// security boundary — pointing a card at a 2 GB log should fail cleanly instead
/// of taking the app down with it.
pub const SOURCE_MAX_BYTES: u64 = 1_048_576;


/// Paths an agent is refused by default, matched case-insensitively anywhere in
/// the resolved path.
///
/// Not a security boundary — a determined caller with the API key has other
/// avenues — but it removes the one-line disaster: a leaked key pointing a card
/// at a private key and reading it back through `GET .../cards/{cid}`.
pub const MIRROR_DENY: &[&str] = &[
    "/.ssh/", "/.gnupg/", "/.aws/", "/.config/gcloud/", "/.kube/", "/.docker/config.json",
    "/.netrc", "/.pgpass", "/.npmrc", "/.pypirc", "/.git-credentials",
    "/etc/shadow", "/etc/passwd", "/etc/sudoers",
    "id_rsa", "id_ed25519", "id_ecdsa", ".pem", ".p12", ".pfx", ".keystore", "credentials.json",
];

/// How much of the filesystem **agents** may mirror. The user's own file dialog
/// is never governed by this: someone at the machine already has the filesystem,
/// so restricting them would be theatre. What this bounds is the API — otherwise
/// anything holding the key can point a card at a file and read it straight back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MirrorPolicy {
    /// Anywhere except the credential paths in [`MIRROR_DENY`]. The default:
    /// linking a README or a log is exactly what this feature is for, so
    /// blocking agents wholesale would defeat it.
    SafeDefault,
    /// Only inside the listed directories.
    OnlyDirs,
    /// No restriction at all.
    Anywhere,
}

impl MirrorPolicy {
    pub fn from_key(s: &str) -> Self {
        match s {
            "dirs" => MirrorPolicy::OnlyDirs,
            "any" => MirrorPolicy::Anywhere,
            _ => MirrorPolicy::SafeDefault,
        }
    }
    pub fn key(&self) -> &'static str {
        match self {
            MirrorPolicy::SafeDefault => "safe",
            MirrorPolicy::OnlyDirs => "dirs",
            MirrorPolicy::Anywhere => "any",
        }
    }
}

/// Whether an **agent** may mirror `path`.
///
/// Symlinks and `..` are resolved **before** comparing. Without that any list is
/// decorative — `/allowed/../../etc/shadow` would pass a textual prefix check.
pub fn mirror_allowed(path: &str, policy: MirrorPolicy, dirs: &[String]) -> Result<(), String> {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path));
    match policy {
        MirrorPolicy::Anywhere => Ok(()),
        MirrorPolicy::SafeDefault => {
            let hay = real.to_string_lossy().to_ascii_lowercase();
            match MIRROR_DENY.iter().find(|d| hay.contains(&d.to_ascii_lowercase())) {
                Some(hit) => Err(format!(
                    "{path} looks like a credential file ({hit}). Agents can mirror \
                     anything else; change this in Settings → Agent API."
                )),
                None => Ok(()),
            }
        }
        MirrorPolicy::OnlyDirs => {
            let ok = dirs.iter().filter(|d| !d.trim().is_empty()).any(|d| {
                let dir = std::fs::canonicalize(d).unwrap_or_else(|_| std::path::PathBuf::from(d));
                real.starts_with(&dir)
            });
            if ok {
                Ok(())
            } else {
                Err(format!(
                    "{path} is outside the directories agents may mirror \
                     (Settings → Agent API)."
                ))
            }
        }
    }
}

/// Read a file for a pointer card: its text and modification time.
///
/// Binary files are refused rather than rendered as mojibake — the check is
/// simply whether it is valid UTF-8, which is also what makes the result safe to
/// put in a `String`.
pub fn read_source(path: &str) -> Result<(String, u64), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
    if meta.is_dir() {
        return Err(format!("{path} is a directory"));
    }
    if meta.len() > SOURCE_MAX_BYTES {
        return Err(format!(
            "{path} is {} KB — the limit is {} KB",
            meta.len() / 1024,
            SOURCE_MAX_BYTES / 1024
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{path} is not UTF-8 text"))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok((text, mtime))
}

/// The modification time of a pointer's file, for deciding whether to re-read.
/// A `stat` is cheap enough to run over every pointer card on a timer; reading
/// the file is not.
pub fn source_mtime(path: &str) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Font used by the PDF/image exporters (also embedded in the PDF).
const EXPORT_FONT: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

/// Emoji fallback for the exporters. DejaVu has **zero** emoji coverage, so
/// without this an export was worse than the screen: every emoji in a card came
/// out as a blank or a box. Outline (monochrome) Noto Emoji, the same file the
/// UI uses — a colour bitmap font has no outlines for `ab_glyph` to rasterize or
/// for a PDF to embed.
const EXPORT_EMOJI_FONT: &[u8] = include_bytes!("../assets/NotoEmoji.ttf");

/// The exporters' font stack: text, plus emoji for what the text font lacks.
struct ExportFonts<'a> {
    text: ab_glyph::FontRef<'a>,
    emoji: ab_glyph::FontRef<'a>,
}

impl ExportFonts<'static> {
    fn load() -> Result<Self, String> {
        Ok(ExportFonts {
            text: ab_glyph::FontRef::try_from_slice(EXPORT_FONT).map_err(|e| e.to_string())?,
            emoji: ab_glyph::FontRef::try_from_slice(EXPORT_EMOJI_FONT)
                .map_err(|e| e.to_string())?,
        })
    }
}

impl<'a> ExportFonts<'a> {
    /// Which font draws this char: the text font unless it has no glyph for it.
    /// `true` means the emoji font was chosen — the PDF path needs to know,
    /// since it embeds the two separately.
    fn pick(&self, c: char) -> (&ab_glyph::FontRef<'a>, bool) {
        use ab_glyph::Font as _;
        if self.text.glyph_id(c).0 != 0 {
            (&self.text, false)
        } else {
            (&self.emoji, true)
        }
    }

    /// Split a string into the longest runs that share one font, in order.
    /// Kerning is not carried across a run boundary — there is no meaningful
    /// kern pair between a word and an emoji anyway.
    fn runs(&self, s: &str) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = Vec::new();
        for c in s.chars() {
            let (_, is_emoji) = self.pick(c);
            match out.last_mut() {
                Some((run, e)) if *e == is_emoji => run.push(c),
                _ => out.push((c.to_string(), is_emoji)),
            }
        }
        out
    }
}

/// One laid-out line for the PDF/image exporters. `size` is a point size; an
/// empty `text` is a vertical spacer.
struct ExportLine {
    text: String,
    size: f32,
}

/// Width of `s` in the same units as `size_px`, using the font's advances.
///
/// Measured per character against whichever font will actually draw it, so an
/// emoji's width is the emoji font's — measuring everything with DejaVu would
/// give every emoji the width of its missing-glyph box, and wrapping would be
/// wrong wherever one appeared.
fn text_width(fonts: &ExportFonts, size_px: f32, s: &str) -> f32 {
    use ab_glyph::{Font, PxScale, ScaleFont};
    let mut w = 0.0;
    let mut last: Option<(ab_glyph::GlyphId, bool)> = None;
    for c in s.chars() {
        let (font, is_emoji) = fonts.pick(c);
        let scaled = font.as_scaled(PxScale::from(size_px));
        let g = scaled.glyph_id(c);
        if let Some((l, was_emoji)) = last {
            if was_emoji == is_emoji {
                w += scaled.kern(l, g);
            }
        }
        w += scaled.h_advance(g);
        last = Some((g, is_emoji));
    }
    w
}

/// Greedy word-wrap `text` to `max_w` (same units as `size_px`), preserving the
/// text's own newlines as hard breaks.
fn wrap_text(fonts: &ExportFonts, size_px: f32, text: &str, max_w: f32) -> Vec<String> {
    let space = text_width(fonts, size_px, " ");
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut cur = String::new();
        let mut cur_w = 0.0;
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let ww = text_width(fonts, size_px, word);
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
    fonts: &ExportFonts,
    size_px: f32,
    x0: f32,
    baseline: f32,
    text: &str,
) {
    use ab_glyph::{Font, PxScale, ScaleFont};
    let scale = PxScale::from(size_px);
    let (w, h) = (img.width(), img.height());
    let mut x = x0;
    let mut last: Option<(ab_glyph::GlyphId, bool)> = None;
    for c in text.chars() {
        let (font, is_emoji) = fonts.pick(c);
        let scaled = font.as_scaled(scale);
        let gid = scaled.glyph_id(c);
        if let Some((l, was_emoji)) = last {
            if was_emoji == is_emoji {
                x += scaled.kern(l, gid);
            }
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
        last = Some((gid, is_emoji));
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
    next_item_id: ItemId,
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
            next_item_id: 1,
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
    ///
    /// `allow(dead_code)` because this file is compiled into two binaries: the
    /// `import_journal` importer builds onto this, the app never does.
    #[allow(dead_code)]
    pub fn empty() -> Self {
        Document {
            nodes: HashMap::new(),
            roots: Vec::new(),
            next_node_id: 1,
            next_card_id: 1,
            next_item_id: 1,
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

    /// Find (or create) the journal node for one calendar day, and say what it
    /// took to get there.
    ///
    /// The shape is `<year> → <month> → <day>`, which is the structure a hand-kept
    /// journal already grows into. Everything here is **find-first**: an existing
    /// node is adopted rather than duplicated, because the whole failure this
    /// replaces is ending up with two nodes for one day.
    ///
    /// Matching a day is by **parsed date, not by title text**. A journal kept by
    /// hand drifts — `8/11/2026` beside `6/09/2026`, a misspelled weekday, a
    /// different separator — and a string comparison would sail past all of those
    /// and make a second node for a day that already exists.
    ///
    /// `root` is the year. When the year turns over, the sibling for the new year
    /// is found or created and returned as `root`, so the caller can follow it.
    /// Nothing here is automatic: it runs only when the user (or an agent) asks
    /// for today, never as a side effect of creating an ordinary node.
    pub fn ensure_daily(&mut self, root: NodeId, date: DailyDate) -> Option<DailyNode> {
        let root_node = self.nodes.get(&root)?;
        let parent_of_root = root_node.parent;

        // 1. The year. If the designated root is itself titled with a year and the
        //    calendar has moved on, use that year's sibling instead of nesting
        //    2027 inside 2026.
        let year_node = match root_node.title.trim().parse::<i32>() {
            Ok(y) if y != date.year => {
                let want = date.year.to_string();
                let siblings: Vec<NodeId> = match parent_of_root {
                    Some(p) => self.nodes.get(&p).map(|n| n.children.clone()).unwrap_or_default(),
                    None => self.roots.clone(),
                };
                match siblings.iter().find(|id| {
                    self.nodes.get(id).is_some_and(|n| n.title.trim() == want)
                }) {
                    Some(&id) => id,
                    None => self.add_node(parent_of_root, want),
                }
            }
            _ => root,
        };

        // 2. The month, by name, case-insensitively.
        let month = self.find_or_create_child(year_node, &date.month_name);
        // 3. The day, by the date its title parses to.
        let existing = self.nodes.get(&month).map(|n| n.children.clone()).unwrap_or_default();
        let found = existing.iter().find(|id| {
            self.nodes
                .get(id)
                .and_then(|n| parse_daily_title(&n.title))
                .is_some_and(|(y, m, d)| (y, m, d) == (date.year, date.month, date.day))
        });
        if let Some(&id) = found {
            return Some(DailyNode { node: id, created: false, root: year_node });
        }
        let id = self.add_node(Some(month), date.title());
        // Newest first, by date — not simply "at the top". Today lands first in
        // the normal flow either way, but back-filling an older day has to drop
        // it into place rather than above days that came after it. Siblings whose
        // titles don't parse are stepped over, keeping the order they had.
        let pos = existing
            .iter()
            .position(|sib| {
                self.nodes
                    .get(sib)
                    .and_then(|n| parse_daily_title(&n.title))
                    .is_some_and(|(y, m, d)| (y, m, d) < (date.year, date.month, date.day))
            })
            .unwrap_or(existing.len());
        self.move_node(id, Some(month), pos);
        Some(DailyNode { node: id, created: true, root: year_node })
    }

    /// A child with this title (case-insensitive), or a new one appended.
    fn find_or_create_child(&mut self, parent: NodeId, title: &str) -> NodeId {
        let kids = self.nodes.get(&parent).map(|n| n.children.clone()).unwrap_or_default();
        let want = title.to_lowercase();
        for id in kids {
            if self.nodes.get(&id).is_some_and(|n| n.title.trim().to_lowercase() == want) {
                return id;
            }
        }
        self.add_node(Some(parent), title.to_string())
    }

    /// Mint an item id. Document-wide like card ids, so an item is addressable
    /// on its own rather than only as "the third line of card 10167".
    pub fn mint_item_id(&mut self) -> ItemId {
        let id = self.next_item_id;
        self.next_item_id += 1;
        id
    }

    /// Give every checklist item an id, and keep the counter ahead of them all.
    ///
    /// Runs after loading a document: items written before ids existed arrive as
    /// `0`, and items added by code paths that had no document to hand are also
    /// `0`. Idempotent, so calling it more than once costs a walk and changes
    /// nothing. Returns how many it assigned, which is what makes it testable.
    pub fn ensure_item_ids(&mut self) -> usize {
        // First pass: never re-use an id that is already out there, including in
        // a document written by a newer build with a higher counter.
        let mut highest = 0;
        for n in self.nodes.values() {
            for c in &n.cards {
                if let CardKind::Checklist { items } = &c.kind {
                    for i in items {
                        highest = highest.max(i.id);
                    }
                }
            }
        }
        if self.next_item_id <= highest {
            self.next_item_id = highest + 1;
        }
        let mut next = self.next_item_id;
        let mut assigned = 0;
        for n in self.nodes.values_mut() {
            for c in &mut n.cards {
                if let CardKind::Checklist { items } = &mut c.kind {
                    for i in items {
                        if i.id == 0 {
                            i.id = next;
                            next += 1;
                            assigned += 1;
                        }
                    }
                }
            }
        }
        self.next_item_id = next;
        assigned
    }

    /// Which basket holds this card? Card ids are unique across the document, so
    /// an id is a complete address on its own — but every other lookup here takes
    /// `(node, card)`, so an id read out of an API response or quoted in a note
    /// could not be turned back into a card by anyone who didn't already know the
    /// basket. This is the missing direction.
    pub fn locate_card(&self, card: CardId) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|(_, n)| n.cards.iter().any(|c| c.id == card))
            .map(|(&id, _)| id)
    }

    /// Which basket holds a group, from its id alone.
    ///
    /// The counterpart of [`locate_card`]. Group ids come from one document-wide
    /// counter (`next_group_id`), exactly like card ids, so an id names one group
    /// in the whole document and is a complete address on its own.
    pub fn locate_group(&self, group: GroupId) -> Option<NodeId> {
        self.nodes
            .iter()
            .find(|(_, n)| n.groups.iter().any(|g| g.id == group))
            .map(|(&id, _)| id)
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

    pub fn table_mut(&mut self, node: NodeId, card: CardId) -> Option<&mut TableData> {
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
        t.col_widths[c] = w.clamp(TABLE_MIN_COL_W, TABLE_MAX_COL_W);
        true
    }

    /// Size columns to their content: `Some(c)` fits one column, `None` fits
    /// every column. Returns false only if the card isn't a table (or `c` is out
    /// of range) — an empty table is a no-op, not a failure.
    pub fn table_autofit_cols(&mut self, node: NodeId, card: CardId, col: Option<usize>) -> bool {
        let Some(t) = self.table_mut(node, card) else { return false };
        let cols = t.n_cols();
        if let Some(c) = col {
            if c >= cols {
                return false;
            }
        }
        if t.col_widths.len() < cols {
            t.col_widths.resize(cols, TABLE_DEFAULT_COL_W);
        }
        // Measure first, then write: `autofit_width` reads the rows while
        // `col_widths` needs the mutable borrow.
        let widths: Vec<(usize, f32)> = (0..cols)
            .filter(|c| col.is_none() || col == Some(*c))
            .map(|c| (c, t.autofit_width(c)))
            .collect();
        for (c, w) in widths {
            t.col_widths[c] = w;
        }
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
                touched: None,
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
                touched: None,
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

    /// Move a whole group — its container and every member card — to another
    /// basket, keeping the group's id, title, colour and internal layout.
    ///
    /// This exists because [`move_card_to_node`] deliberately drops group
    /// membership: a group is basket-local, so a card moved on its own cannot
    /// stay in a group that did not come with it. Moving the members one at a
    /// time therefore dissolves the group and leaves the caller to rebuild it,
    /// which loses the id — and the id is what a `[[#g…]]` link points at.
    ///
    /// `pos` places the group's **top-left corner**, translating every member by
    /// the same delta so the arrangement inside the group survives the move.
    /// Docking is kept only between cards that travel together; a dock to a card
    /// left behind is cut, because it would name a card in another basket.
    ///
    /// Returns how many cards moved, or `None` if the basket, the group or the
    /// destination does not exist.
    pub fn move_group_to_node(
        &mut self,
        from: NodeId,
        group: GroupId,
        to: NodeId,
        pos: Option<egui::Pos2>,
    ) -> Option<usize> {
        if from == to || !self.nodes.contains_key(&to) {
            return None;
        }
        let n = self.nodes.get(&from)?;
        let gidx = n.groups.iter().position(|g| g.id == group)?;
        let members: Vec<CardId> =
            n.cards.iter().filter(|c| c.group == Some(group)).map(|c| c.id).collect();

        // Translate so the group's top-left lands on `pos`, keeping the layout.
        let delta = match pos {
            Some(p) => {
                let mut min = egui::pos2(f32::MAX, f32::MAX);
                for c in n.cards.iter().filter(|c| c.group == Some(group)) {
                    min.x = min.x.min(c.pos.x);
                    min.y = min.y.min(c.pos.y);
                }
                if min.x == f32::MAX { egui::Vec2::ZERO } else { p - min }
            }
            None => egui::Vec2::ZERO,
        };

        let n = self.nodes.get_mut(&from)?;
        let container = n.groups.remove(gidx);
        let mut moved: Vec<Card> = Vec::with_capacity(members.len());
        let mut i = 0;
        while i < n.cards.len() {
            if n.cards[i].group == Some(group) {
                let mut c = n.cards.remove(i);
                c.pos += delta;
                moved.push(c);
            } else {
                i += 1;
            }
        }
        // Cards staying behind cannot dock to one that left.
        for other in n.cards.iter_mut() {
            if other.docked_to.is_some_and(|d| members.contains(&d)) {
                other.docked_to = None;
            }
        }
        for c in moved.iter_mut() {
            if c.docked_to.is_some_and(|d| !members.contains(&d)) {
                c.docked_to = None;
            }
        }
        let count = moved.len();
        let dest = self.nodes.get_mut(&to)?;
        dest.groups.push(container);
        dest.cards.extend(moved);
        Some(count)
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

    /// The pairs of cards in a basket whose rectangles overlap, as
    /// `(a, b)` with `a` before `b` in card order.
    ///
    /// Cards that travel together — a dock stack, a group — are *expected* to
    /// sit on top of each other, so they are never reported against each other.
    pub fn overlapping_cards(&self, node: NodeId) -> Vec<(CardId, CardId)> {
        let Some(n) = self.nodes.get(&node) else { return Vec::new() };
        let cluster = self.card_clusters(node);
        let mut out = Vec::new();
        for (i, a) in n.cards.iter().enumerate() {
            for b in n.cards.iter().skip(i + 1) {
                if cluster.get(&a.id) == cluster.get(&b.id) {
                    continue;
                }
                if rects_overlap(a.pos, a.size, b.pos, b.size) {
                    out.push((a.id, b.id));
                }
            }
        }
        out
    }

    /// Which cards move together: a group, or a card and everything docked to
    /// it. Returns a cluster key per card id.
    fn card_clusters(&self, node: NodeId) -> std::collections::HashMap<CardId, CardId> {
        let mut key: std::collections::HashMap<CardId, CardId> = std::collections::HashMap::new();
        let Some(n) = self.nodes.get(&node) else { return key };
        for c in &n.cards {
            key.insert(c.id, c.id);
        }
        // Resolve to the lowest id in each connected set, iterating until stable
        // — a dock chain and a group can link three cards in any order.
        loop {
            let mut changed = false;
            let mut link = |key: &mut std::collections::HashMap<CardId, CardId>, a: CardId, b: CardId| {
                let (ka, kb) = (key[&a], key[&b]);
                if ka != kb {
                    let lo = ka.min(kb);
                    for v in key.values_mut() {
                        if *v == ka || *v == kb {
                            *v = lo;
                        }
                    }
                    changed = true;
                }
            };
            for c in &n.cards {
                if let Some(p) = c.docked_to {
                    if key.contains_key(&p) {
                        link(&mut key, c.id, p);
                    }
                }
            }
            for (i, a) in n.cards.iter().enumerate() {
                if let Some(g) = a.group {
                    for b in n.cards.iter().skip(i + 1) {
                        if b.group == Some(g) {
                            link(&mut key, a.id, b.id);
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        key
    }

    /// Push overlapping cards **down** until nothing in the basket covers
    /// anything else, and return how many cards moved.
    ///
    /// This is the counterpart to [`Document::autosort`], not a variant of it.
    /// Autosort throws a layout away and lays a grid; this keeps the layout —
    /// every `x` is preserved, so columns survive, and cards move only far
    /// enough to stop overlapping, in the order they already sat in. A basket
    /// that does not overlap is not touched at all.
    ///
    /// It exists because **`fit: true` changes a card's width as well as its
    /// height**, so a card grown to fit its content can silently end up over its
    /// neighbour — with nothing to warn you. Deliberate layouts (a roadmap, a
    /// board) could not be repaired with autosort without destroying them.
    pub fn resolve_overlaps(&mut self, node: NodeId) -> usize {
        const GAP: f32 = 12.0;
        let cluster = self.card_clusters(node);
        let Some(n) = self.nodes.get_mut(&node) else { return 0 };
        // One box per cluster, in reading order: top edge, then left edge.
        let mut boxes: Vec<(CardId, egui::Pos2, egui::Vec2)> = Vec::new();
        for c in n.cards.iter() {
            let k = cluster[&c.id];
            match boxes.iter_mut().find(|(bk, ..)| *bk == k) {
                Some((_, pos, size)) => {
                    let max = egui::pos2(
                        (pos.x + size.x).max(c.pos.x + c.size.x),
                        (pos.y + size.y).max(c.pos.y + c.size.y),
                    );
                    pos.x = pos.x.min(c.pos.x);
                    pos.y = pos.y.min(c.pos.y);
                    *size = max - *pos;
                }
                None => boxes.push((k, c.pos, c.size)),
            }
        }
        boxes.sort_by(|a, b| {
            a.1.y.partial_cmp(&b.1.y).unwrap_or(std::cmp::Ordering::Equal).then(
                a.1.x.partial_cmp(&b.1.x).unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        let mut placed: Vec<(egui::Pos2, egui::Vec2)> = Vec::new();
        let mut shift: std::collections::HashMap<CardId, f32> = std::collections::HashMap::new();
        for (k, pos, size) in boxes {
            let mut pos = pos;
            // Repeat: dropping below one card can land on the next one down.
            loop {
                let hit = placed
                    .iter()
                    .filter(|(p, s)| rects_overlap(pos, size, *p, *s))
                    .map(|(p, s)| p.y + s.y)
                    .fold(f32::NEG_INFINITY, f32::max);
                if hit == f32::NEG_INFINITY {
                    break;
                }
                pos.y = hit + GAP;
            }
            placed.push((pos, size));
            shift.insert(k, pos.y);
        }
        let mut moved = 0;
        // Re-derive each cluster's original top so the delta is applied to every
        // member, keeping the shape of a group or a dock stack intact.
        let mut top: std::collections::HashMap<CardId, f32> = std::collections::HashMap::new();
        for c in n.cards.iter() {
            let k = cluster[&c.id];
            let e = top.entry(k).or_insert(f32::INFINITY);
            *e = e.min(c.pos.y);
        }
        for c in n.cards.iter_mut() {
            let k = cluster[&c.id];
            let delta = shift[&k] - top[&k];
            if delta.abs() > 0.01 {
                c.pos.y += delta;
                moved += 1;
            }
        }
        moved
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

    /// Fold or open the **whole tree**: every root and everything under it.
    /// Returns how many nodes actually changed.
    ///
    /// Recursive, matching the per-node *Expand all* / *Collapse all* and the
    /// Android toolbar. The alternative — fold the roots and leave each
    /// subtree's inner shape alone — means the tree remembers a state you cannot
    /// see, so reopening a project gives you a shape you did not ask for.
    pub fn set_all_expanded(&mut self, expanded: bool) -> usize {
        self.roots
            .clone()
            .into_iter()
            .map(|r| self.set_subtree_expanded(r, expanded, true))
            .sum()
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
            z: c.z,
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
            // Depth rides along even when the basket is being viewed flat: a
            // card that carried depth out must carry it back in, or "export then
            // import" quietly loses the arrangement.
            c.z = exp.z;
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
    ///
    /// `allow(dead_code)`: the app reads properties through the task, agenda and
    /// query surfaces rather than one card at a time, so these two accessors
    /// exist for the test suite — which is where the *"a checklist card's
    /// properties come from its title and items, never its body"* rule is
    /// pinned. Asserting on `extract_properties` by hand instead would duplicate
    /// the logic under test.
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

    /// One checklist item, by id, wherever it sits in the list.
    ///
    /// By id and never by index: an index is a position, and positions move when
    /// the list is edited, which is exactly what item ids exist to stop.
    pub fn item_mut(&mut self, node: NodeId, card: CardId, item: ItemId)
        -> Option<&mut ChecklistItem>
    {
        match &mut self.card_mut(node, card)?.kind {
            CardKind::Checklist { items } => items.iter_mut().find(|i| i.id == item),
            _ => None,
        }
    }

    /// Set a `key:: value` property on one checklist item, rewriting it in place
    /// if it is already there.
    pub fn set_item_property(
        &mut self, node: NodeId, card: CardId, item: ItemId, key: &str, value: &str,
    ) -> bool {
        let key = key.to_lowercase();
        let Some(it) = self.item_mut(node, card, item) else { return false };
        let marker = format!("{key}:: ");
        if let Some(pos) = it.text.to_lowercase().find(&marker) {
            let after = &it.text[pos + marker.len()..];
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            let tail = after[end..].to_string();
            it.text = format!("{}{key}:: {value}{tail}", &it.text[..pos]);
        } else {
            if !it.text.ends_with(' ') && !it.text.is_empty() {
                it.text.push_str("  ");
            }
            it.text.push_str(&format!("{key}:: {value}"));
        }
        true
    }

    /// Remove a `key:: value` property from one checklist item.
    pub fn clear_item_property(
        &mut self, node: NodeId, card: CardId, item: ItemId, key: &str,
    ) -> bool {
        let key = key.to_lowercase();
        let Some(it) = self.item_mut(node, card, item) else { return false };
        let marker = format!("{key}:: ");
        let Some(pos) = it.text.to_lowercase().find(&marker) else { return false };
        let after = &it.text[pos + marker.len()..];
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        let joined = format!("{}{}", &it.text[..pos], &after[end..]);
        it.text = joined.split_whitespace().collect::<Vec<_>>().join(" ");
        true
    }

    /// Tick or untick one checklist item.
    pub fn set_item_done(
        &mut self, node: NodeId, card: CardId, item: ItemId, done: bool,
    ) -> bool {
        match self.item_mut(node, card, item) {
            Some(it) => { it.done = done; true }
            None => false,
        }
    }

    /// Remove an inline `key:: value` line entirely.
    ///
    /// Distinct from setting it empty, which leaves `due:: ` behind — a property
    /// that still exists and parses as an unreadable date, so the card stays on
    /// the agenda under "No date" instead of leaving it. Returns false if the
    /// card had no such line.
    pub fn clear_card_property(&mut self, node: NodeId, card: CardId, key: &str) -> bool {
        let key = key.to_lowercase();
        let Some(c) = self.card_mut(node, card) else { return false };
        let prefix = format!("{key}:: ");
        let before = c.body.lines().count();
        let kept: Vec<&str> = c
            .body
            .lines()
            .filter(|l| {
                let t = l.trim_start().to_lowercase();
                !(t.starts_with(&prefix) || t.trim_end() == format!("{key}::"))
            })
            .collect();
        if kept.len() == before {
            return false;
        }
        c.body = kept.join("\n");
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
                let root = self.root_of(node.id);
                m.entry(status.to_lowercase()).or_default().push(KanbanCard {
                    node: node.id,
                    card: card.id,
                    title,
                    node_title: node.title.clone(),
                    node_path: self.node_path(node.id),
                    root,
                    root_title: self
                        .nodes
                        .get(&root)
                        .map(|n| n.title.clone())
                        .unwrap_or_default(),
                    color: card.color,
                    due,
                    tags,
                });
            }
        }
        m
    }

    /// The top-level ancestor of `id` — the "project" a node belongs to. Returns
    /// `id` itself when it is already a root.
    pub fn root_of(&self, id: NodeId) -> NodeId {
        let mut cur = id;
        while let Some(p) = self.nodes.get(&cur).and_then(|n| n.parent) {
            cur = p;
        }
        cur
    }

    /// Is `id` `ancestor`, or somewhere beneath it? Used to filter a view down to
    /// one project (or any sub-branch of one).
    pub fn is_under(&self, id: NodeId, ancestor: NodeId) -> bool {
        let mut cur = Some(id);
        while let Some(n) = cur {
            if n == ancestor {
                return true;
            }
            cur = self.nodes.get(&n).and_then(|x| x.parent);
        }
        false
    }

    /// Root-to-node breadcrumb of titles, e.g. `Newsletter › Open Items`.
    ///
    /// Task and Kanban views need this, not just the parent's title: basket names
    /// like "Open Items" repeat across projects, and the bare name gives no clue
    /// which project a task belongs to.
    pub fn node_path(&self, id: NodeId) -> String {
        let mut parts = Vec::new();
        let mut cur = Some(id);
        while let Some(nid) = cur {
            match self.nodes.get(&nid) {
                Some(n) => {
                    parts.push(n.title.as_str());
                    cur = n.parent;
                }
                None => break,
            }
        }
        parts.reverse();
        parts.join(" › ")
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
    ///
    /// Card links resolve to their **basket** here, so every existing caller —
    /// the graph, the old backlinks — keeps working and simply sees the basket a
    /// linked card lives in. Use [`resolve_link_target`] when you need to know
    /// that a card specifically was named.
    pub fn resolve_link(&self, target: &str) -> Option<NodeId> {
        match self.resolve_link_target(target)? {
            LinkTarget::Node(id) => Some(id),
            LinkTarget::Card { node, .. } => Some(node),
            LinkTarget::Group { node, .. } => Some(node),
        }
    }

    /// Resolve a `[[wiki-link]]` to whatever it names.
    ///
    /// `[[#1391]]` is a **card** — the `#` prefix is how card ids are written in
    /// the docs and accepted by the Ctrl+O palette, so it reads the way it is
    /// already spoken. `[[42]]` and `[[Some Basket]]` stay node links, unchanged,
    /// because that is what every link written before this existed means.
    ///
    /// `[[#g146]]` is a **group**. It hangs off the same `#` because a group id
    /// is the same kind of thing as a card id — an address that is not a basket —
    /// and the `g` is what tells them apart. Nothing written before this can
    /// collide: `g146` never parsed as a card id, so a link that means a group
    /// today fell through to a title lookup and found nothing.
    pub fn resolve_link_target(&self, target: &str) -> Option<LinkTarget> {
        let t = target.trim();
        if let Some(rest) = t.strip_prefix('#') {
            let rest = rest.trim();
            if let Some(digits) = rest.strip_prefix(['g', 'G']) {
                let id: GroupId = digits.trim().parse().ok()?;
                let node = self.locate_group(id)?;
                return Some(LinkTarget::Group { node, group: id });
            }
            let id: CardId = rest.parse().ok()?;
            let node = self.locate_card(id)?;
            return Some(LinkTarget::Card { node, card: id });
        }
        if let Ok(id) = t.parse::<NodeId>() {
            if self.nodes.contains_key(&id) {
                return Some(LinkTarget::Node(id));
            }
        }
        let tl = t.to_lowercase();
        self.nodes
            .values()
            .find(|n| n.title.to_lowercase() == tl)
            .map(|n| LinkTarget::Node(n.id))
    }

    /// Cards anywhere whose `[[links]]` point at one specific card.
    ///
    /// The counterpart of [`backlinks`], which answers for a whole basket. A
    /// basket-level answer is useless in a document whose baskets are days: every
    /// card written on the 11th shares one basket, so "what links here" would
    /// return the day, not the thing.
    pub fn backlinks_card(&self, node: NodeId, card: CardId) -> Vec<SearchHit> {
        let mut hits = Vec::new();
        for n in self.nodes.values() {
            for c in &n.cards {
                if c.id == card && n.id == node {
                    continue; // a card linking to itself is noise, not a backlink
                }
                let hay = format!("{}\n{}", c.title, searchable_body(c));
                let points_here = extract_wikilinks(&hay).iter().any(|t| {
                    matches!(self.resolve_link_target(t),
                             Some(LinkTarget::Card { card: tc, .. }) if tc == card)
                });
                if points_here {
                    hits.push(SearchHit {
                        node: n.id,
                        card: Some(c.id),
                        node_title: n.title.clone(),
                        snippet: snippet_around(&hay, 0, 0),
                    });
                }
            }
        }
        hits
    }

    /// Cards anywhere whose `[[#g…]]` links point at one group.
    ///
    /// A member card is not skipped the way [`backlinks_card`] skips the card
    /// itself: a card that names the group it belongs to is making a real
    /// reference, not linking to itself.
    pub fn backlinks_group(&self, group: GroupId) -> Vec<SearchHit> {
        let mut hits = Vec::new();
        for n in self.nodes.values() {
            for c in &n.cards {
                let hay = format!("{}\n{}", c.title, searchable_body(c));
                let points_here = extract_wikilinks(&hay).iter().any(|t| {
                    matches!(self.resolve_link_target(t),
                             Some(LinkTarget::Group { group: tg, .. }) if tg == group)
                });
                if points_here {
                    hits.push(SearchHit {
                        node: n.id,
                        card: Some(c.id),
                        node_title: n.title.clone(),
                        snippet: snippet_around(&hay, 0, 0),
                    });
                }
            }
        }
        hits
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
                // A checklist whose ITEMS carry their own dates is a list of
                // tasks, not one task. Emitting per item is what keeps a long
                // working list compact on the canvas while still reaching the
                // agenda — the alternative is a card per line, which in a
                // journal-shaped document doubles the document every day.
                if let CardKind::Checklist { items } = &card.kind {
                    let mut any = false;
                    for it in items {
                        let iprops = extract_properties(&it.text);
                        let Some((_, due)) = iprops.iter().find(|(k, _)| k == "due") else {
                            continue;
                        };
                        any = true;
                        let root = self.root_of(node.id);
                        out.push(TaskItem {
                            node: node.id,
                            node_title: node.title.clone(),
                            node_path: self.node_path(node.id),
                            root,
                            root_title: self.nodes.get(&root).map(|n| n.title.clone()).unwrap_or_default(),
                            card: card.id,
                            item: Some(it.id),
                            // The checkbox is the done signal, and a `status::`
                            // on the line can say so too — either counts, so
                            // ticking the box is never overruled by text.
                            done: it.done
                                || iprops.iter().any(|(k, v)| {
                                    k == "status"
                                        && matches!(v.to_lowercase().as_str(),
                                                    "done" | "complete" | "completed" | "closed")
                                }),
                            title: strip_properties(&it.text),
                            due: due.clone(),
                            due_days: parse_ymd(due),
                            start: iprops.iter().find(|(k, _)| k == "start").map(|(_, v)| v.clone()),
                            start_days: iprops
                                .iter()
                                .find(|(k, _)| k == "start")
                                .and_then(|(_, v)| parse_ymd(v)),
                        });
                    }
                    // A checklist whose items carry dates has already spoken for
                    // itself; falling through would list the card again as a
                    // duplicate of its own contents.
                    if any {
                        continue;
                    }
                }
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
                let root = self.root_of(node.id);
                out.push(TaskItem {
                    node: node.id,
                    node_title: node.title.clone(),
                    node_path: self.node_path(node.id),
                    root,
                    root_title: self.nodes.get(&root).map(|n| n.title.clone()).unwrap_or_default(),
                    card: card.id,
                    item: None,
                    title,
                    due: due.clone(),
                    due_days: parse_ymd(due),
                    start: props.iter().find(|(k, _)| k == "start").map(|(_, v)| v.clone()),
                    start_days: props
                        .iter()
                        .find(|(k, _)| k == "start")
                        .and_then(|(_, v)| parse_ymd(v)),
                    done,
                });
            }
        }
        out
    }

    /// Every card that **asserts state** and says when that assertion should be
    /// re-checked — a `verify:: YYYY-MM-DD` property.
    ///
    /// This is deliberately *not* `due::`. A task is finished once and leaves
    /// the agenda; a claim about the world is never finished, it only goes out
    /// of date — "both instances run 0.109.0" was true when it was written and
    /// false a release later, with nothing in the document to say so. Mixing the
    /// two would put permanent entries in the agenda, where the whole point is
    /// that things leave it.
    ///
    /// `check:: <how>` rides alongside as free text: the command, endpoint or
    /// file that settles the claim (`check:: GET /api/instance`). A reader who
    /// finds a claim expired then knows how to re-establish it rather than
    /// having to guess, and a plugin can act on the checkable ones.
    ///
    /// A checklist's items are **not** scanned. A claim is the card's assertion
    /// about the world, not a line's — the opposite of `due::`, where the line
    /// is the unit of work.
    pub fn claims(&self) -> Vec<Claim> {
        let mut out = Vec::new();
        for node in self.nodes.values() {
            for card in &node.cards {
                let props = card.properties();
                let Some((_, verify)) = props.iter().find(|(k, _)| k == "verify") else { continue };
                let title = if card.title.trim().is_empty() {
                    searchable_body(card).lines().next().unwrap_or("").chars().take(60).collect()
                } else {
                    card.title.clone()
                };
                let root = self.root_of(node.id);
                out.push(Claim {
                    node: node.id,
                    node_title: node.title.clone(),
                    node_path: self.node_path(node.id),
                    root,
                    root_title: self.nodes.get(&root).map(|n| n.title.clone()).unwrap_or_default(),
                    card: card.id,
                    title,
                    verify: verify.clone(),
                    verify_days: parse_ymd(verify),
                    check: props.iter().find(|(k, _)| k == "check").map(|(_, v)| v.clone()),
                    touched: card.touched,
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

/// Convert **block** HTML in a card body into Markdown, so a card that holds
/// HTML renders as content instead of vanishing.
///
/// CommonMark says a raw HTML block passes straight through, and the card
/// renderer draws no HTML at all — so a table pasted from a page, or anything
/// the web clipper could not translate, was **dropped on the floor**: not shown,
/// not an error, just gone.
///
/// Converting rather than *implementing* HTML is the whole point. `html2md` is
/// already a dependency (File → Import HTML uses it), and going through Markdown
/// means headings, lists, tables, links and emphasis all arrive already
/// supported by the renderer — where an HTML subset would mean picking which
/// tags to honour and re-answering that question forever.
///
/// **Inline HTML is deliberately untouched.** `<span style="color:…">` is how a
/// card's text colour is stored, and the renderer honours it directly; running
/// it through a converter would throw the colour away. Only `HtmlBlock` spans
/// are rewritten.
///
/// The body on disk is never changed — this is a view, applied at render.
pub fn html_blocks_to_md(text: &str) -> std::borrow::Cow<'_, str> {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    // The overwhelmingly common card has no HTML at all; don't parse it twice.
    if !text.contains('<') {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut spans: Vec<std::ops::Range<usize>> = Vec::new();
    for (ev, range) in Parser::new_ext(text, Options::all()).into_offset_iter() {
        if matches!(ev, Event::Start(Tag::HtmlBlock)) {
            spans.push(range);
        }
    }
    if spans.is_empty() {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for r in spans {
        // Nested spans can't happen, but a defensive skip keeps the splice sane.
        if r.start < last {
            continue;
        }
        out.push_str(&text[last..r.start]);
        let md = html2md::parse_html(&text[r.clone()]);
        let md = md.trim();
        // An HTML block that carries no text (a comment, a stray `<div>`) turns
        // into nothing; leave the gap rather than the markup.
        if !md.is_empty() {
            out.push_str(md);
            out.push('\n');
        }
        last = r.end;
    }
    out.push_str(&text[last..]);
    std::borrow::Cow::Owned(out)
}

/// Rewrite `[[Target]]` / `[[Target|Display]]` into Markdown links
/// `[Display](trellis:<encoded target>)` so the card renderer shows them as
/// clickable links; the app intercepts the `trellis:` scheme to navigate.
/// Split text into runs of plain text and `[[wiki-links]]`.
///
/// `wikilinks_to_md` exists for renderers that understand Markdown. A table cell
/// is painted as a single galley with no Markdown anywhere in sight, so it needs
/// the *pieces* — what to draw, and which parts are a link — rather than a
/// rewritten string.
///
/// Each item is `(display text, Some(target))` for a link, or
/// `(text, None)` for ordinary text. Same rules as `wikilinks_to_md`: `|` splits
/// display from target, both are trimmed, and an empty target is not a link.
pub fn wikilink_segments(text: &str) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut plain = String::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if i + 1 < b.len() && b[i] == b'[' && b[i + 1] == b'[' {
            if let Some(end) = text[i + 2..].find("]]") {
                let inner = &text[i + 2..i + 2 + end];
                let mut parts = inner.splitn(2, '|');
                let target = parts.next().unwrap_or("").trim();
                let display = parts
                    .next()
                    .map(|d| d.trim())
                    .filter(|d| !d.is_empty())
                    .unwrap_or(target);
                if !target.is_empty() {
                    if !plain.is_empty() {
                        out.push((std::mem::take(&mut plain), None));
                    }
                    out.push((display.to_string(), Some(target.to_string())));
                    i = i + 2 + end + 2;
                    continue;
                }
            }
        }
        let ch = text[i..].chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }
    if !plain.is_empty() {
        out.push((plain, None));
    }
    out
}

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
/// The readable part of a checklist line: its text with the `key:: value`
/// properties taken out.
///
/// A task row should read "Fix the CTE", not "Fix the CTE  due:: 2026-08-15
/// status:: doing" — the properties are how the line is *tracked*, not what it
/// says. They stay in the underlying text, which is what the user edits.
pub(crate) fn strip_properties(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_fence = false;
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        // Code is left exactly as written, for the same reason
        // [`extract_properties`] does not read properties out of it.
        if is_code_fence(line) {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        let mut rest = line;
        while let Some(pos) = rest.find(":: ") {
            let base = line.len() - rest.len();
            // Walk back over the key to the whitespace that starts it.
            let key_start = rest[..pos]
                .rfind(char::is_whitespace)
                .map(|i| i + 1)
                .unwrap_or(0);
            let key = &rest[key_start..pos];
            let is_prop = !key.is_empty()
                && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                && !in_code_span(line, base + key_start);
            if !is_prop {
                // Not a property — keep the text up to here and carry on past it.
                out.push_str(&rest[..pos + 3]);
                rest = &rest[pos + 3..];
                continue;
            }
            out.push_str(&rest[..key_start]);
            let after = &rest[pos + 3..];
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            rest = &after[end..];
        }
        out.push_str(rest);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Is byte offset `pos` inside an inline `` `code span` `` on this line?
///
/// Counts the backticks before it: an odd number means a span is open. Cheap,
/// and it matches how the Markdown renderer treats the line closely enough —
/// the case being caught is prose *quoting* the syntax, which is always written
/// between backticks.
fn in_code_span(line: &str, pos: usize) -> bool {
    line[..pos].bytes().filter(|&c| c == b'`').count() % 2 == 1
}

/// Does this line open or close a fenced code block?
fn is_code_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

pub(crate) fn extract_properties(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    // **Prose about a property is not a property.** A card that *documents* the
    // syntax — a session report, a handoff, this changelog — was acquiring the
    // properties it described: the card discussing `due:: 2026-08-15` grew a due
    // date, landed on the Agenda and took its own Kanban column. Measured across
    // both live documents: 801 real properties, 13 false ones, and every false
    // one sat inside backticks or a fence.
    //
    // So code is skipped and **nothing else is** — the rule considered first,
    // "a property must be on its own line", would have dropped two live
    // deadlines, because a checklist item carries its `due::` at the end of the
    // sentence it belongs to. A `Code` *card* is left alone for the same reason:
    // one in the work document legitimately holds `status:: done`.
    let mut in_fence = false;
    for line in text.lines() {
        if is_code_fence(line) {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
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
                if ks < i && !in_code_span(line, ks) {
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
            let mut value = line[vs..ve].trim().to_string();
            // **A date is one token.** `due`, `start` and `date` hold a calendar
            // date, so a value running to the end of the line is always a
            // sentence that happens to begin with one — `due:: 2026-08-15 — RUN
            // 1 DONE 8/12: …` is a real line from a real checklist. Taking the
            // whole tail failed *twice over*: it does not parse as a date, so the
            // task silently lost its deadline and fell into "No date"; and the
            // Agenda then rendered 300 characters where a date goes, which drove
            // the panel to the full width of the window.
            //
            // Only these keys. A free-text property (`status:: in progress`,
            // `owner:: Jane Doe`) legitimately has spaces in it.
            if matches!(key.as_str(), "due" | "start" | "date") {
                if let Some(first) = value.split_whitespace().next() {
                    value = first.to_string();
                }
            }
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
    /// Root-to-basket breadcrumb, so two projects' "Open Items" are tellable
    /// apart at a glance.
    pub node_path: String,
    /// Top-level ancestor — the project this task belongs to.
    pub root: NodeId,
    pub root_title: String,
    pub card: CardId,
    /// Set when the task is one **line of a checklist** rather than a whole
    /// card. This is what lets a 23-item list be 23 tasks without becoming 23
    /// cards — the unit of work is the line, and the card is the container.
    pub item: Option<ItemId>,
    pub title: String,
    /// The raw `due` value as written (e.g. `2026-08-15`).
    pub due: String,
    /// `due` parsed to days since the Unix epoch, or `None` if unparseable.
    pub due_days: Option<i64>,
    /// `start:: YYYY-MM-DD` — when the task becomes live. A task with a start
    /// **occupies a range of days** rather than a single deadline, which is what
    /// makes it appear on every day it is actually in flight instead of only on
    /// the day it is due. `None` = a plain deadline, as before.
    pub start: Option<String>,
    pub start_days: Option<i64>,
    pub done: bool,
}

/// A card that asserts state, and the date its assertion is next due a check.
///
/// The workspace failure this exists for: a card said "both instances serve
/// 0.103.1" and an agent repeated it four releases later, because nothing in the
/// document distinguished *a fact* from *a fact as of a date*. Every field here
/// is about answering "how old is this claim, and how would I settle it?"
/// without reading the card's prose.
#[derive(Debug, Clone)]
pub struct Claim {
    pub node: NodeId,
    pub node_title: String,
    /// Root-to-basket breadcrumb, so a claim can be cited without a bare id.
    pub node_path: String,
    pub root: NodeId,
    pub root_title: String,
    pub card: CardId,
    pub title: String,
    /// The raw `verify` value as written (e.g. `2026-09-01`).
    pub verify: String,
    /// `verify` parsed to days since the Unix epoch, or `None` if unparseable —
    /// which is itself reported rather than silently treated as fresh.
    pub verify_days: Option<i64>,
    /// `check:: <how>` — the command, endpoint or file that settles this claim.
    pub check: Option<String>,
    /// When the card was last edited. Deliberately separate from `verify`:
    /// `touched` moves for a typo fix, so it says when the card changed, never
    /// when anyone confirmed what it says.
    pub touched: Option<u64>,
}

impl TaskItem {
    /// Is this task live on `day` (days since epoch)?
    ///
    /// With a `start::` it is live from that day through its due date; without
    /// one it is live only on its due date. Overdue work counts as live on every
    /// later day too, because an unfinished deadline does not stop applying just
    /// because the date passed — that is the difference between a calendar and
    /// a task list.
    pub fn live_on(&self, day: i64) -> bool {
        if self.done {
            return false;
        }
        match (self.start_days, self.due_days) {
            (Some(s), Some(d)) => day >= s && (day <= d || d < day),
            (Some(s), None) => day >= s,
            (None, Some(d)) => day >= d,
            (None, None) => false,
        }
    }
}

impl Document {
    /// Cards that are **live on `day`** (days since the Unix epoch) and live
    /// somewhere other than `exclude` — the cards a day should show in addition
    /// to its own.
    ///
    /// This is the whole of the time axis: a card is present in a day when that
    /// day falls inside its `start::`→`due::` span, which is a *derived* fact,
    /// never an authored one. Nothing is copied and nothing has to be kept in
    /// sync; the card exists once, and each day is a different point on the axis
    /// looking at it.
    ///
    /// Deduplicated by card, because a checklist with several dated items yields
    /// one task per item and they all point at the same card.
    pub fn cards_live_on(&self, day: i64, exclude: NodeId) -> Vec<(NodeId, CardId)> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for t in self.tasks() {
            if t.node == exclude {
                continue;
            }
            // **Containment, not `live_on`.** `live_on` keeps an unfinished
            // deadline live on every later day, which is right for an agenda —
            // a missed deadline still applies — and wrong for a slice of time.
            // Without this a day fills with every overdue task in the document,
            // which is what the first run of this looked like.
            let inside = match (t.start_days, t.due_days) {
                (Some(s), Some(d)) => day >= s && day <= d,
                (None, Some(d)) => day == d,
                _ => false,
            };
            if !inside || t.done {
                continue;
            }
            // **Only cards that live in a day.** A card's position means
            // something within its own basket and nothing outside it, so
            // projecting one from a project basket lands it at coordinates that
            // are arbitrary here — in practice a pile. A card living in another
            // *day* shares the journal's coordinate space, and is exactly the
            // case this exists for: work written on the 11th that is still in
            // flight on the 12th. Everything else is the Agenda's job.
            let from_a_day = self
                .nodes
                .get(&t.node)
                .map(|n| parse_daily_title(&n.title).is_some())
                .unwrap_or(false);
            if !from_a_day {
                continue;
            }
            if seen.insert((t.node, t.card)) {
                out.push((t.node, t.card));
            }
        }
        out
    }
}

/// A card on the Kanban board — its status column plus the bits the board shows.
pub struct KanbanCard {
    pub node: NodeId,
    pub card: CardId,
    pub title: String,
    pub node_title: String,
    /// Root-to-basket breadcrumb (see `TaskItem::node_path`).
    pub node_path: String,
    /// Top-level ancestor — the project this card belongs to.
    pub root: NodeId,
    pub root_title: String,
    /// The card's accent color `[r,g,b]` (shown as the card's border on the board).
    pub color: [u8; 3],
    /// The `due::` value if the card has one (e.g. `2026-08-15`).
    pub due: Option<String>,
    /// `#tags` on the card, in first-seen order.
    pub tags: Vec<String>,
}

/// What a `[[wiki-link]]` names.
///
/// Card links exist because a basket is not always a meaningful destination: in a
/// journal-shaped document every card written on one day shares a basket, so
/// linking to the day says nothing about the thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Node(NodeId),
    Card { node: NodeId, card: CardId },
    Group { node: NodeId, group: GroupId },
}

/// The calendar day a journal node stands for, plus the names to write.
///
/// Passed in rather than read from the clock so the logic is testable and so
/// "today" is decided in exactly one place (`chrono::Local`, same as `due::`).
#[derive(Clone, Debug, PartialEq)]
pub struct DailyDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    /// `"Tuesday"` — written into the title, never parsed back out.
    pub weekday: String,
    /// `"August"` — the month node's title.
    pub month_name: String,
}

impl DailyDate {
    /// `Tuesday 8/11/2026` — unpadded, matching the dominant hand-kept form.
    /// Only ever used for a node being **created**; finding one goes by
    /// [`parse_daily_title`], which accepts every spelling already in the tree.
    pub fn title(&self) -> String {
        format!("{} {}/{}/{}", self.weekday, self.month, self.day, self.year)
    }
}

/// Where [`Document::ensure_daily`] landed.
#[derive(Clone, Copy, Debug)]
pub struct DailyNode {
    pub node: NodeId,
    /// False when an existing node was adopted — the common case, and the point.
    pub created: bool,
    /// The year node used, which differs from the one passed in after a year
    /// rollover. The caller should store this back as the journal root.
    pub root: NodeId,
}

/// Pull a date out of a journal node's title, however it was written.
///
/// Accepts `M/D/YYYY` with or without zero padding, and `-` or `.` in place of
/// `/`, anywhere in the string — so `Tuesday 8/11/2026`, `6/09/2026` and even
/// `Wednedsay 7/15/2026` all resolve. Deliberately tolerant: this is what stops
/// a second node being created for a day that is already there under a slightly
/// different spelling.
pub fn parse_daily_title(title: &str) -> Option<(i32, u32, u32)> {
    let bytes: Vec<char> = title.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut nums: Vec<u32> = Vec::new();
        let mut cur = String::new();
        let mut j = i;
        while j < bytes.len() {
            let c = bytes[j];
            if c.is_ascii_digit() {
                cur.push(c);
                j += 1;
            } else if matches!(c, '/' | '-' | '.') && !cur.is_empty() && nums.len() < 2 {
                nums.push(cur.parse().ok()?);
                cur.clear();
                j += 1;
            } else {
                break;
            }
        }
        if !cur.is_empty() {
            if let Ok(n) = cur.parse::<u32>() {
                nums.push(n);
            }
        }
        if nums.len() == 3 {
            let (m, d, y) = (nums[0], nums[1], nums[2]);
            if (1..=12).contains(&m) && (1..=31).contains(&d) && y >= 1000 {
                return Some((y as i32, m, d));
            }
        }
        i = if j > start { j } else { start + 1 };
    }
    None
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
    /// Depth. `serde(default)` so a card file or template written before depth
    /// existed still loads, and one written *with* depth still loads in an older
    /// build — export must never be lossy in either direction.
    #[serde(default)]
    pub z: f32,
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

pub fn searchable_body(card: &Card) -> String {
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

/// Do two card rectangles cover any of the same canvas? Touching edges don't
/// count — a card placed exactly against another is a layout, not a collision.
fn rects_overlap(ap: egui::Pos2, asz: egui::Vec2, bp: egui::Pos2, bsz: egui::Vec2) -> bool {
    const EPS: f32 = 0.5;
    ap.x + asz.x - EPS > bp.x
        && bp.x + bsz.x - EPS > ap.x
        && ap.y + asz.y - EPS > bp.y
        && bp.y + bsz.y - EPS > ap.y
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
            // Block HTML is converted here too, or the searchable text would say
            // `<table>` where the page beside it shows a table.
            let stripped = strip_inline_markers(&card.body);
            let stripped = html_blocks_to_md(&stripped);
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

/// Points per millimetre, for the PDF exporters (which lay out in mm but size
/// text in points).
const MM_TO_PT: f32 = 2.834_646;

/// Draw one already-wrapped line into a PDF layer, switching embedded font per
/// run.
///
/// A PDF text operator names exactly one font, so a line mixing words and emoji
/// has to be written as several placed runs — `use_text` with the whole string
/// would drop every character the named font lacks. With no emoji in the line
/// this is a single run, i.e. exactly what it did before.
fn pdf_line(
    layer: &printpdf::PdfLayerReference,
    fonts: &ExportFonts,
    text_font: &printpdf::IndirectFontRef,
    emoji_font: &printpdf::IndirectFontRef,
    line: &str,
    size: f32,
    x_mm: f32,
    y_mm: f32,
) {
    use printpdf::Mm;
    let mut x = x_mm;
    for (run, is_emoji) in fonts.runs(line) {
        let f = if is_emoji { emoji_font } else { text_font };
        layer.use_text(&run, size, Mm(x), Mm(y_mm), f);
        // Advance by the run's own width, so the next run starts where this one
        // ended rather than back at the margin.
        x += text_width(fonts, size, &run) / MM_TO_PT;
    }
}

/// Render a flat list of laid-out lines to a paginated A4 PDF. Shared by the
/// whole-document and single-card PDF exporters.
fn lines_to_pdf(lines: &[ExportLine]) -> Result<Vec<u8>, String> {
    use printpdf::{Mm, PdfDocument};
    let fonts = ExportFonts::load()?;
    let (w_mm, h_mm, margin) = (210.0_f32, 297.0_f32, 20.0_f32);
    let content_w_pt = (w_mm - margin * 2.0) * MM_TO_PT;
    let (doc, page1, layer1) = PdfDocument::new("Trellis export", Mm(w_mm), Mm(h_mm), "Layer 1");
    let font = doc
        .add_external_font(std::io::Cursor::new(EXPORT_FONT))
        .map_err(|e| e.to_string())?;
    let emoji = doc
        .add_external_font(std::io::Cursor::new(EXPORT_EMOJI_FONT))
        .map_err(|e| e.to_string())?;
    let mut layer = doc.get_page(page1).get_layer(layer1);
    let mut y = h_mm - margin;
    for l in lines {
        let leading = (l.size * 1.4) / MM_TO_PT;
        let wrapped = if l.text.is_empty() {
            vec![String::new()]
        } else {
            wrap_text(&fonts, l.size, &l.text, content_w_pt)
        };
        for line in wrapped {
            if y < margin {
                let (p, lay) = doc.add_page(Mm(w_mm), Mm(h_mm), "Layer");
                layer = doc.get_page(p).get_layer(lay);
                y = h_mm - margin;
            }
            if !line.is_empty() {
                pdf_line(&layer, &fonts, &font, &emoji, &line, l.size, margin, y);
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
    use image::{Rgba, RgbaImage};
    let fonts = ExportFonts::load()?;
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
            for w in wrap_text(&fonts, px, &l.text, content_w) {
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
            draw_text(&mut img, &fonts, *px, margin, y + *px, text);
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
    let fonts = ExportFonts::load()?;
    let (w_mm, h_mm, margin) = (210.0_f32, 297.0_f32, 16.0_f32);
    let content_w_mm = w_mm - margin * 2.0;
    let content_w_pt = content_w_mm * MM_TO_PT;

    let (doc, mut cur_page, mut cur_layer) =
        PdfDocument::new("Trellis basket", Mm(w_mm), Mm(h_mm), "Layer 1");
    let font =
        doc.add_external_font(std::io::Cursor::new(EXPORT_FONT)).map_err(|e| e.to_string())?;
    let emoji = doc
        .add_external_font(std::io::Cursor::new(EXPORT_EMOJI_FONT))
        .map_err(|e| e.to_string())?;

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
            pdf_line(&layer, &fonts, &font, &emoji, &page.title, 15.0, margin, y - 5.0);
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
                    wrap_text(&fonts, size, raw, content_w_pt)
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
                        pdf_line(&tl, &fonts, &font, &emoji, &line, size, margin, y);
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

    /// The point of the whole design: a compact list of many tasks, in ONE card.
    #[test]
    fn a_checklist_item_with_a_date_is_its_own_task() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Open Items".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Checklist {
            items: vec![
                ChecklistItem::new("Fix the importer  due:: 2026-08-15"),
                ChecklistItem::new("Ship the migration  due:: 2026-08-15  status:: doing"),
                ChecklistItem::new("no date on this one — not a task"),
                ChecklistItem { id: 0, done: true, text: "Already done  due:: 2026-08-12".into() },
            ],
        }).unwrap();
        doc.ensure_item_ids();

        let tasks = doc.tasks();
        assert_eq!(tasks.len(), 3, "one task per dated ITEM, and the undated line is not one");
        assert!(tasks.iter().all(|t| t.card == c && t.item.is_some()));

        // The row reads as the work, not as the metadata.
        let fix = tasks.iter().find(|t| t.title.starts_with("Fix the importer")).unwrap();
        assert_eq!(fix.title, "Fix the importer");
        assert_eq!(fix.due, "2026-08-15");
        assert!(!fix.done);

        // The checkbox is the done signal.
        let done = tasks.iter().find(|t| t.due == "2026-08-12").unwrap();
        assert!(done.done, "a ticked box means done");

        // Every task has a distinct identity, so they can be told apart.
        let ids: Vec<_> = tasks.iter().map(|t| t.item.unwrap()).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids[0] != ids[1] && ids[1] != ids[2]);
    }

    /// A checklist that carries dates on its ITEMS must not ALSO be listed as
    /// one task in its own right — that would double-count every list.
    #[test]
    fn a_dated_checklist_is_not_counted_twice() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Checklist {
            items: vec![ChecklistItem::new("a  due:: 2026-08-15")],
        }).unwrap();
        // The card itself also carries a date, the old way.
        doc.card_mut(n, c).unwrap().title = "Sprint  due:: 2026-08-20".into();
        doc.ensure_item_ids();

        let tasks = doc.tasks();
        assert_eq!(tasks.len(), 1, "the items speak for the card");
        assert_eq!(tasks[0].due, "2026-08-15");
        assert!(tasks[0].item.is_some());
    }

    /// A checklist with no dated items keeps behaving exactly as before, so
    /// every existing checklist in every document is unaffected.
    #[test]
    fn an_undated_checklist_still_behaves_the_old_way() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Checklist {
            items: vec![ChecklistItem::new("one"), ChecklistItem { id: 0, done: true, text: "two".into() }],
        }).unwrap();
        // A checklist card's properties are read from its title and items —
        // never its body, which a checklist doesn't use.
        doc.card_mut(n, c).unwrap().title = "Sprint  due:: 2026-08-15".into();
        doc.ensure_item_ids();

        let tasks = doc.tasks();
        assert_eq!(tasks.len(), 1, "the card is the task, as it always was");
        assert_eq!(tasks[0].item, None);
        assert!(!tasks[0].done, "not every box is ticked");
    }

    #[test]
    fn strip_properties_leaves_the_readable_half() {
        assert_eq!(strip_properties("Fix the CTE  due:: 2026-08-15"), "Fix the CTE");
        assert_eq!(
            strip_properties("Ship it  due:: 2026-08-15  status:: doing"),
            "Ship it"
        );
        assert_eq!(strip_properties("no properties here"), "no properties here");
        // Whatever `extract_properties` calls a property, this must remove —
        // the two have to agree or a row would show text the parser had already
        // claimed. Both treat any alphanumeric run before ":: " as a key.
        for probe in ["see:: ", "ratio 3:: 4", "x  due:: 2026-08-15"] {
            let props = extract_properties(probe);
            let left = strip_properties(probe);
            for (k, _) in &props {
                assert!(
                    !left.contains(&format!("{k}:: ")),
                    "{probe:?}: {k} was parsed as a property but left in {left:?}"
                );
            }
        }
    }

    /// Prose *about* a property is not a property. Every line here is real text
    /// from the two live documents; before this, each one gave its card a due
    /// date or a status it never asked for.
    #[test]
    fn a_property_quoted_in_code_is_not_a_property() {
        let quoted = [
            "`due:: 2026-08-15 — still blocked` made the whole sentence the value",
            "nine cards carrying `status:: todo`, two with `due::` dates",
            "(bracketed `[status:: …]` in the title).",
            "a card in **[[Some Basket]]** with `due::`",
        ];
        for line in quoted {
            assert!(
                extract_properties(line).is_empty(),
                "{line:?} was read as carrying {:?}",
                extract_properties(line)
            );
            // …and the text is left exactly as written, backticks and all.
            assert_eq!(strip_properties(line), line.split_whitespace().collect::<Vec<_>>().join(" "));
        }

        // A fenced block is code too — the `\n` cases the recipe card hits.
        let fenced = "due:: 2026-08-15\n```\nstart:: 8/11\n```\nstatus:: done";
        let props = extract_properties(fenced);
        assert_eq!(props, vec![
            ("due".into(), "2026-08-15".to_string()),
            ("status".into(), "done".to_string()),
        ]);
    }

    /// The other half of the same call: everything real keeps working. These are
    /// live lines too — the ones the rejected "a property must be on its own
    /// line" rule would have silently dropped.
    #[test]
    fn a_real_property_still_parses_wherever_it_sits() {
        let real = [
            // One space, mid-sentence, at the end of a checklist item.
            ("… the decision is parked meanwhile. due:: 2026-08-15", "due", "2026-08-15"),
            // The app writes two spaces; both must work.
            ("F. Second pass: everything re-checked  due:: 2026-08-15", "due", "2026-08-15"),
            // A value with spaces in it is still a value.
            ("#verify #flows status:: in progress", "status", "in progress"),
            // Its own line, and bracketed inline — the documented forms.
            ("status:: done", "status", "done"),
            ("#handoff  [date:: 2026-08-04]", "date", "2026-08-04"),
            // A backtick *after* the property does not reach back over it.
            ("due:: 2026-08-15 see `status:: x`", "due", "2026-08-15"),
        ];
        for (line, key, value) in real {
            let props = extract_properties(line);
            assert!(
                props.iter().any(|(k, v)| k == key && v == value),
                "{line:?} lost {key}:: {value} — parsed {props:?}"
            );
        }
    }

    /// Identity is the whole point: a checklist line has to survive being
    /// edited, ticked and reordered, or it cannot be a task, a link target, or
    /// anything that spans time.
    #[test]
    fn item_ids_are_assigned_once_and_survive_edits() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        // As a document written before ids existed would load: all zero.
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Checklist {
            items: vec![ChecklistItem::new("a"), ChecklistItem::new("b"), ChecklistItem::new("c")],
        }).unwrap();

        assert_eq!(doc.ensure_item_ids(), 3, "three items needed ids");
        let ids: Vec<ItemId> = match &doc.card(n, c).unwrap().kind {
            CardKind::Checklist { items } => items.iter().map(|i| i.id).collect(),
            _ => unreachable!(),
        };
        assert!(ids.iter().all(|&i| i != 0), "no item left unassigned");
        assert_eq!(ids.len(), 3);
        assert!(ids[0] != ids[1] && ids[1] != ids[2], "ids must be distinct");

        // Idempotent: running again assigns nothing and changes nothing.
        assert_eq!(doc.ensure_item_ids(), 0);
        let again: Vec<ItemId> = match &doc.card(n, c).unwrap().kind {
            CardKind::Checklist { items } => items.iter().map(|i| i.id).collect(),
            _ => unreachable!(),
        };
        assert_eq!(ids, again, "a second pass must not renumber anything");

        // A new item added later gets an id no existing one holds.
        let fresh = doc.mint_item_id();
        assert!(!ids.contains(&fresh));
    }

    /// The counter must never hand out an id that is already in the document —
    /// including one written by a newer build with a higher counter.
    #[test]
    fn ensure_item_ids_never_reuses_an_existing_id() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Checklist {
            items: vec![
                ChecklistItem { id: 900, done: false, text: "from the future".into() },
                ChecklistItem::new("needs one"),
            ],
        }).unwrap();
        doc.ensure_item_ids();
        let ids: Vec<ItemId> = match &doc.card(n, c).unwrap().kind {
            CardKind::Checklist { items } => items.iter().map(|i| i.id).collect(),
            _ => unreachable!(),
        };
        assert_eq!(ids[0], 900, "an existing id is left alone");
        assert!(ids[1] > 900, "the new one must clear everything already used");
        assert!(doc.mint_item_id() > ids[1]);
    }

    /// `[[#id]]` names a card; the older forms keep meaning a node, because
    /// that is what every link written before card links existed meant.
    #[test]
    fn a_hash_link_names_a_card_and_the_old_forms_still_name_nodes() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "Project Falcon".into());
        let b = doc.add_node(None, "Other".into());
        let c1 = doc.add_card(a, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();

        assert_eq!(doc.resolve_link_target("Project Falcon"), Some(LinkTarget::Node(a)));
        assert_eq!(doc.resolve_link_target(&a.to_string()), Some(LinkTarget::Node(a)));
        assert_eq!(
            doc.resolve_link_target(&format!("#{c1}")),
            Some(LinkTarget::Card { node: a, card: c1 })
        );
        // Whitespace and a missing card are both handled without panicking.
        assert_eq!(
            doc.resolve_link_target(&format!("# {c1} ")),
            Some(LinkTarget::Card { node: a, card: c1 })
        );
        assert_eq!(doc.resolve_link_target("#999999"), None);
        assert_eq!(doc.resolve_link_target("#notanumber"), None);
        let _ = b;

        // Old callers see the card's basket, so the graph and node backlinks
        // keep working rather than silently dropping card links.
        assert_eq!(doc.resolve_link(&format!("#{c1}")), Some(a));
    }

    /// Card backlinks must answer for the card, not its day — the whole reason
    /// they exist in a journal-shaped document.
    #[test]
    fn card_backlinks_find_the_card_not_its_basket() {
        let mut doc = Document::empty();
        let day = doc.add_node(None, "Tuesday 8/11/2026".into());
        let target = doc.add_card(day, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let sibling = doc.add_card(day, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let other = doc.add_node(None, "Elsewhere".into());
        let linker = doc.add_card(other, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(other, linker).unwrap().body = format!("same failure as [[#{target}]]");

        let hits = doc.backlinks_card(day, target);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].card, Some(linker));

        // The sibling shares the basket and is NOT a backlink of the target.
        assert!(doc.backlinks_card(day, sibling).is_empty());

        // A card linking to itself is noise, not a backlink.
        doc.card_mut(day, target).unwrap().body = format!("see [[#{target}]]");
        assert_eq!(doc.backlinks_card(day, target).len(), 1, "still just the real linker");
    }

    /// A link written in a TABLE CELL already counted for backlinks before this
    /// change; it must keep counting, and now resolve to a card.
    #[test]
    fn a_link_inside_a_table_cell_still_counts() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "A".into());
        let target = doc.add_card(a, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let t = doc.add_card(a, egui::pos2(0.0, 0.0), CardKind::Table {
            table: TableData {
                rows: vec![vec![TableCell {
                    text: format!("see [[#{target}]]"),
                    ..Default::default()
                }]],
                col_widths: vec![],
                header: false,
                chart: None,
                rules: vec![],
            },
        }).unwrap();
        let hits = doc.backlinks_card(a, target);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].card, Some(t));
    }

    /// Clearing a property must remove the line, not blank it. A `due::` with
    /// nothing after it is still a property: the card stays on the agenda under
    /// "No date" instead of leaving it, which is the opposite of what "clear"
    /// means to the person who clicked it.
    #[test]
    fn clearing_a_property_removes_the_line_rather_than_emptying_it() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c).unwrap().body = "status:: doing\ndue:: 2026-08-15\nnotes here".into();

        assert!(doc.clear_card_property(n, c, "due"));
        let body = &doc.card(n, c).unwrap().body;
        assert!(!body.contains("due::"), "the line must be gone, not blank: {body:?}");
        assert!(body.contains("status:: doing"), "other properties untouched");
        assert!(body.contains("notes here"));
        assert!(doc.card_property(n, c, "due").is_none());

        // Clearing what isn't there is false, not a panic or a stray edit.
        assert!(!doc.clear_card_property(n, c, "due"));
        assert_eq!(doc.card_property(n, c, "status").as_deref(), Some("doing"));
    }

    /// The titles in a hand-kept journal are not uniform. These are all real
    /// forms taken from a live work document, and every one of them has to
    /// resolve — a title this misses becomes a duplicate node for a day that
    /// already exists, which is the exact failure the feature removes.
    #[test]
    fn a_day_is_recognised_however_its_title_was_written() {
        assert_eq!(parse_daily_title("Tuesday 8/11/2026"), Some((2026, 8, 11)));
        assert_eq!(parse_daily_title("Tuesday 6/09/2026"), Some((2026, 6, 9)));  // zero-padded
        assert_eq!(parse_daily_title("Wednedsay 7/15/2026"), Some((2026, 7, 15))); // misspelled
        assert_eq!(parse_daily_title("8/1/2026"), Some((2026, 8, 1)));           // no weekday
        assert_eq!(parse_daily_title("Friday 08-07-2026"), Some((2026, 8, 7)));  // dashes
        // Not dates.
        assert_eq!(parse_daily_title("August"), None);
        assert_eq!(parse_daily_title("2026"), None);
        assert_eq!(parse_daily_title("Sprint 3/4 done"), None); // 3/4 is not a date
        assert_eq!(parse_daily_title("13/40/2026"), None);      // out of range
    }

    /// Find-first is the whole contract: asking twice must land on one node.
    #[test]
    fn ensure_daily_adopts_an_existing_day_instead_of_duplicating_it() {
        let mut doc = Document::empty();
        let year = doc.add_node(None, "2026".into());
        let month = doc.add_node(Some(year), "August".into());
        // Written by hand, in a form the feature would not itself produce.
        let existing = doc.add_node(Some(month), "Tuesday 08/11/2026".into());

        let date = DailyDate {
            year: 2026, month: 8, day: 11,
            weekday: "Tuesday".into(), month_name: "August".into(),
        };
        let r = doc.ensure_daily(year, date.clone()).unwrap();
        assert_eq!(r.node, existing, "made a second node for a day already there");
        assert!(!r.created);
        assert_eq!(doc.nodes[&month].children.len(), 1);

        // And again — still one.
        let r2 = doc.ensure_daily(year, date).unwrap();
        assert_eq!(r2.node, existing);
        assert_eq!(doc.nodes[&month].children.len(), 1);
    }

    /// A back-filled day drops into date order rather than landing on top of
    /// days that came after it — the reason the position is computed and not
    /// simply 0.
    #[test]
    fn a_backfilled_day_lands_in_date_order() {
        let mut doc = Document::empty();
        let year = doc.add_node(None, "2026".into());
        let aug = doc.add_node(Some(year), "August".into());
        let d13 = doc.add_node(Some(aug), "Thursday 8/13/2026".into());
        let d11 = doc.add_node(Some(aug), "Tuesday 8/11/2026".into());

        // 8/12 belongs between them, not above 8/13.
        let mid = doc.ensure_daily(year, DailyDate {
            year: 2026, month: 8, day: 12,
            weekday: "Wednesday".into(), month_name: "August".into(),
        }).unwrap();
        assert_eq!(doc.nodes[&aug].children, vec![d13, mid.node, d11]);

        // An older day than any present goes last.
        let old = doc.ensure_daily(year, DailyDate {
            year: 2026, month: 8, day: 1,
            weekday: "Saturday".into(), month_name: "August".into(),
        }).unwrap();
        assert_eq!(doc.nodes[&aug].children, vec![d13, mid.node, d11, old.node]);
    }

    /// A missing month is created, and today lands at the top: a journal is read
    /// downward from the newest day.
    #[test]
    fn ensure_daily_creates_the_month_and_puts_today_first() {
        let mut doc = Document::empty();
        let year = doc.add_node(None, "2026".into());
        let august = doc.add_node(Some(year), "August".into());
        let older = doc.add_node(Some(august), "Monday 8/3/2026".into());

        let r = doc.ensure_daily(year, DailyDate {
            year: 2026, month: 8, day: 11,
            weekday: "Tuesday".into(), month_name: "August".into(),
        }).unwrap();
        assert!(r.created);
        assert_eq!(doc.nodes[&august].children, vec![r.node, older], "today must be first");
        assert_eq!(doc.nodes[&r.node].title, "Tuesday 8/11/2026");

        // A month that does not exist yet is created under the same year.
        let sep = doc.ensure_daily(year, DailyDate {
            year: 2026, month: 9, day: 1,
            weekday: "Tuesday".into(), month_name: "September".into(),
        }).unwrap();
        let sep_month = doc.nodes[&sep.node].parent.unwrap();
        assert_eq!(doc.nodes[&sep_month].title, "September");
        assert_eq!(doc.nodes[&sep_month].parent, Some(year));
    }

    /// January must not end up inside last year. The new year is a *sibling* of
    /// the old root, and the caller is told so it can follow.
    #[test]
    fn a_new_year_becomes_a_sibling_not_a_child() {
        let mut doc = Document::empty();
        let y2026 = doc.add_node(None, "2026".into());

        let r = doc.ensure_daily(y2026, DailyDate {
            year: 2027, month: 1, day: 2,
            weekday: "Saturday".into(), month_name: "January".into(),
        }).unwrap();

        assert_ne!(r.root, y2026, "root must move to the new year");
        assert_eq!(doc.nodes[&r.root].title, "2027");
        assert_eq!(doc.nodes[&r.root].parent, doc.nodes[&y2026].parent, "same level as 2026");
        assert!(doc.nodes[&y2026].children.is_empty(), "2027 must not nest inside 2026");

        // Asking again reuses the 2027 it just made.
        let again = doc.ensure_daily(r.root, DailyDate {
            year: 2027, month: 1, day: 2,
            weekday: "Saturday".into(), month_name: "January".into(),
        }).unwrap();
        assert_eq!(again.node, r.node);
        assert!(!again.created);
    }

    /// Month matching ignores case, so a journal titled "AUGUST" or "august"
    /// does not sprout a second August beside it.
    #[test]
    fn month_matching_ignores_case() {
        let mut doc = Document::empty();
        let year = doc.add_node(None, "2026".into());
        let shouty = doc.add_node(Some(year), "AUGUST".into());
        let r = doc.ensure_daily(year, DailyDate {
            year: 2026, month: 8, day: 11,
            weekday: "Tuesday".into(), month_name: "August".into(),
        }).unwrap();
        assert_eq!(doc.nodes[&r.node].parent, Some(shouty));
        assert_eq!(doc.nodes[&year].children.len(), 1);
    }

    /// A card id is only a usable address if it names exactly one card in the
    /// whole document — that is what lets `/api/cards/{id}` and the Ctrl+O
    /// palette resolve a bare number. `next_card_id` is document-wide, so ids
    /// never restart per basket; this pins that, because a per-node counter
    /// would make every lookup here ambiguous instead of merely wrong.
    #[test]
    fn a_card_id_locates_exactly_one_basket() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "A".into());
        let b = doc.add_node(None, "B".into());
        let c1 = doc.add_card(a, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let c2 = doc.add_card(b, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let c3 = doc.add_card(b, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();

        assert_ne!(c1, c2, "ids must not restart in a new basket");
        assert_ne!(c2, c3);
        assert_eq!(doc.locate_card(c1), Some(a));
        assert_eq!(doc.locate_card(c2), Some(b));
        assert_eq!(doc.locate_card(c3), Some(b));
        assert_eq!(doc.locate_card(9999), None, "an unknown id resolves to nothing");
    }

    /// Moving a card between baskets has to move where its id resolves, or the
    /// palette would keep sending you to the basket it used to live in.
    #[test]
    fn locate_card_follows_a_card_to_its_new_basket() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "A".into());
        let b = doc.add_node(None, "B".into());
        let c = doc.add_card(a, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        assert_eq!(doc.locate_card(c), Some(a));
        doc.move_card_to_node(a, c, b, None);
        assert_eq!(doc.locate_card(c), Some(b));
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
            items: vec![ChecklistItem { id: 0, done: true, text: "ship it".into() }],
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
            items: vec![ChecklistItem { id: 0, done: false, text: long.into() }],
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

    /// A date property takes its first token, so prose after the date does not
    /// swallow it.
    ///
    /// Two failures in one: the value did not parse as a date, so a task with
    /// a deadline was bucketed
    /// under "No date" and its owner could not see it was due; and the Agenda
    /// drew the whole sentence where a date goes, which stretched the panel over
    /// the entire window.
    #[test]
    fn a_date_property_stops_at_the_date() {
        let p = extract_properties(
            "Benchmark → pick a default  due:: 2026-08-15 — first pass done, 4/4",
        );
        assert!(p.contains(&("due".to_string(), "2026-08-15".to_string())), "{p:?}");
        assert!(parse_ymd("2026-08-15").is_some());

        let s = extract_properties("in flight  start:: 2026-08-11  due:: 2026-08-15  notes here");
        assert!(s.contains(&("start".to_string(), "2026-08-11".to_string())), "{s:?}");
        assert!(s.contains(&("due".to_string(), "2026-08-15".to_string())), "{s:?}");

        // A free-text property still keeps its spaces — only dates are one token.
        let f = extract_properties("status:: in progress");
        assert!(f.contains(&("status".to_string(), "in progress".to_string())), "{f:?}");
    }

    #[test]
    fn wikilink_segments_split_text_from_links() {
        let segs = wikilink_segments("see [[#10215]] and [[Roadmap|the plan]] end");
        assert_eq!(segs[0], ("see ".to_string(), None));
        assert_eq!(segs[1], ("#10215".to_string(), Some("#10215".to_string())));
        assert_eq!(segs[2], (" and ".to_string(), None));
        assert_eq!(segs[3], ("the plan".to_string(), Some("Roadmap".to_string())));
        assert_eq!(segs[4], (" end".to_string(), None));

        // No links at all is one plain run — the common case, and it must not
        // allocate a link section for a cell full of ordinary text.
        let plain = wikilink_segments("just a value");
        assert_eq!(plain.len(), 1);
        assert!(plain[0].1.is_none());

        // An empty target is not a link; the brackets stay as written.
        assert_eq!(wikilink_segments("[[]]")[0].0, "[[]]");
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

    /// The time axis is *containment*, and only between days.
    ///
    /// Both halves were found by running it: without containment a day fills
    /// with every overdue task in the document (`live_on` deliberately keeps a
    /// missed deadline live forever, which is an agenda rule, not a calendar
    /// one), and without the day restriction a card is projected at coordinates
    /// that mean nothing outside its own basket, landing in a pile.
    #[test]
    fn a_day_shows_only_spanning_work_from_other_days() {
        let mut doc = Document::empty();
        let d11 = doc.add_node(None, "Tuesday 8/11/2026".into());
        let d12 = doc.add_node(None, "Wednesday 8/12/2026".into());
        let proj = doc.add_node(None, "Open Items".into());
        let day = parse_ymd("2026-08-12").unwrap();

        let spanning = doc.add_card(d11, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(d11, spanning).unwrap().body =
            "start:: 2026-08-11\ndue:: 2026-08-15".into();
        let overdue = doc.add_card(d11, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(d11, overdue).unwrap().body = "due:: 2026-06-01".into();
        let elsewhere = doc.add_card(proj, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(proj, elsewhere).unwrap().body =
            "start:: 2026-08-11\ndue:: 2026-08-15".into();

        let got = doc.cards_live_on(day, d12);
        assert_eq!(got, vec![(d11, spanning)], "only the spanning card, only from a day");

        // Outside the span it is simply not there.
        assert!(doc.cards_live_on(parse_ymd("2026-08-20").unwrap(), d12).is_empty());
        // And a day never projects itself.
        assert!(doc.cards_live_on(parse_ymd("2026-08-11").unwrap(), d11).is_empty());
    }

    /// Depth must survive leaving the document and coming back, and a card file
    /// written before depth existed must still load. Export being lossy in
    /// either direction is the failure that would make the Depth toggle a trap:
    /// arrange a basket, export it, import it, and the arrangement is gone.
    #[test]
    fn depth_round_trips_through_export_and_old_files_still_load() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Deep".into());
        let c = doc.add_card(n, egui::pos2(10.0, 20.0), CardKind::Text).unwrap();
        doc.card_mut(n, c).unwrap().z = 240.0;

        let json = doc.export_card_json(n, c).expect("export");
        assert!(json.contains("\"z\""), "depth is written out: {json}");
        let exp = parse_card_export(&json).expect("parse");
        assert_eq!(exp.z, 240.0);

        let n2 = doc.add_node(None, "Flat".into());
        let c2 = doc.add_card_from_export(n2, egui::pos2(0.0, 0.0), exp).unwrap();
        assert_eq!(doc.card(n2, c2).unwrap().z, 240.0, "depth came back");

        // A card file from before depth existed has no `z` at all.
        let old = json.replace("\"z\": 240.0,", "");
        let exp_old = parse_card_export(&old).expect("an older card file still parses");
        assert_eq!(exp_old.z, 0.0, "missing depth reads as the flat plane, not an error");
    }

    /// A flat document must serialize exactly as it did before depth existed —
    /// otherwise every 2-D document's file changes the first time it is saved by
    /// this build, which is noise in a diff and in version history.
    #[test]
    fn a_flat_card_writes_no_depth_field() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Flat".into());
        let c = doc.add_card(n, egui::pos2(1.0, 2.0), CardKind::Text).unwrap();
        let flat = ron::ser::to_string(doc.card(n, c).unwrap()).unwrap();
        assert!(!flat.contains("z:"), "no depth field for a card that has none: {flat}");
        doc.card_mut(n, c).unwrap().z = 5.0;
        let deep = ron::ser::to_string(doc.card(n, c).unwrap()).unwrap();
        assert!(deep.contains("z:"), "and it appears once there is depth: {deep}");
        // ...and reading it back is the same card.
        let back: Card = ron::from_str(&deep).unwrap();
        assert_eq!(back.z, 5.0);
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
    fn chart_number_parsing_accepts_decorated_cells() {
        assert_eq!(parse_number("42"), Some(42.0));
        assert_eq!(parse_number(" 1,234.5 "), Some(1234.5));
        assert_eq!(parse_number("$12"), Some(12.0));
        assert_eq!(parse_number("40%"), Some(40.0));
        assert_eq!(parse_number("(3)"), Some(-3.0));
        // Not numbers — must be gaps, never 0, or a blank status cell would
        // plot as a real reading.
        assert_eq!(parse_number(""), None);
        assert_eq!(parse_number("pass"), None);
        assert_eq!(parse_number("-"), None);
    }

    #[test]
    fn tasks_and_kanban_carry_the_full_basket_path() {
        // Two projects, each with an "Open Items" basket — the case that had an
        // agent attribute a task to the wrong project.
        let mut doc = Document::empty();
        let a = doc.add_node(None, "Backend".into());
        let b = doc.add_node(None, "Newsletter".into());
        let a_open = doc.add_node(Some(a), "Open Items".into());
        let b_open = doc.add_node(Some(b), "Open Items".into());
        for (n, t) in [(a_open, "ship the agent"), (b_open, "publish the piece")] {
            let cid = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
            let c = doc.card_mut(n, cid).unwrap();
            c.title = t.to_string();
            c.body = "due:: 2026-08-15\nstatus:: doing".to_string();
        }

        let tasks = doc.tasks();
        assert_eq!(tasks.len(), 2);
        // The bare parent title is identical for both — that's the bug.
        assert_eq!(tasks[0].node_title, tasks[1].node_title);
        // The path tells them apart, and names the project first.
        let mut paths: Vec<&str> = tasks.iter().map(|t| t.node_path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, vec!["Backend › Open Items", "Newsletter › Open Items"]);

        let board = doc.cards_by_status();
        let doing = board.get("doing").expect("both cards are status:: doing");
        let mut kpaths: Vec<&str> = doing.iter().map(|c| c.node_path.as_str()).collect();
        kpaths.sort();
        assert_eq!(kpaths, vec!["Backend › Open Items", "Newsletter › Open Items"]);
    }

    #[test]
    fn chart_kind_keys_round_trip_and_reject_junk() {
        for k in ChartKind::ALL {
            assert_eq!(ChartKind::from_key(k.key()), Some(k), "{} must round-trip", k.key());
        }
        assert_eq!(ChartKind::from_key("PIE"), Some(ChartKind::Pie), "case-insensitive");
        assert_eq!(ChartKind::from_key("donut"), Some(ChartKind::Pie), "common alias");
        // Unknown kinds are rejected so the API can say so instead of silently
        // picking a chart the caller didn't ask for.
        assert_eq!(ChartKind::from_key("radar"), None);
        assert_eq!(ChartKind::from_key(""), None);
    }

    #[test]
    fn pie_uses_the_first_series_and_ignores_non_positive() {
        // A pie divides a whole, so negatives/blanks/zeros have no arc — the
        // percentages must be of the positive values only.
        let t = TableData::from_values(vec![
            vec!["Part".into(), "Share".into(), "Other".into()],
            vec!["Huge".into(), "800".into(), "1".into()],
            vec!["Small".into(), "90".into(), "1".into()],
            vec!["Negative".into(), "-50".into(), "1".into()],
            vec!["Blank".into(), "".into(), "1".into()],
        ]);
        let spec = ChartSpec { kind: ChartKind::Pie, ..ChartSpec::default() };
        let (labels, series) = t.chart_data(&spec);
        assert_eq!(labels, vec!["Huge", "Small", "Negative", "Blank"]);
        // First series is what the pie draws.
        assert_eq!(series[0].0, "Share");
        let vals = &series[0].1;
        let positive: f64 = vals.iter().flatten().filter(|v| **v > 0.0).sum();
        assert_eq!(positive, 890.0, "only Huge + Small count toward the whole");
        assert_eq!(vals[3], None, "a blank cell is a gap, not a zero slice");
        assert_eq!(vals[2], Some(-50.0), "the negative is present but not plottable");
    }

    #[test]
    fn chart_data_reads_labels_series_and_gaps() {
        let t = TableData::from_values(vec![
            vec!["Week".into(), "Sales".into(), "Notes".into(), "Cost".into()],
            vec!["W1".into(), "10".into(), "good".into(), "4".into()],
            vec!["W2".into(), "".into(), "meh".into(), "5".into()],
            vec!["W3".into(), "30".into(), "".into(), "6".into()],
        ]);
        let spec = ChartSpec::default(); // label_col 0, auto series
        let (labels, series) = t.chart_data(&spec);
        assert_eq!(labels, vec!["W1", "W2", "W3"]);
        // "Notes" holds no numbers, so it is not offered as a series.
        let names: Vec<&str> = series.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["Sales", "Cost"]);
        assert_eq!(series[0].1, vec![Some(10.0), None, Some(30.0)], "blank stays a gap");
        assert_eq!(series[1].1, vec![Some(4.0), Some(5.0), Some(6.0)]);

        // An explicit column list wins over the auto-detection.
        let spec = ChartSpec { value_cols: vec![3], ..ChartSpec::default() };
        let (_, series) = t.chart_data(&spec);
        assert_eq!(series.len(), 1);
        assert_eq!(series[0].0, "Cost");
    }

    #[test]
    fn table_without_a_chart_field_still_loads() {
        // Tables saved before charts existed have no `chart` key at all.
        let t: TableData = ron::from_str(
            "(rows: [[(text: \"a\", bg: None, fg: None)]], col_widths: [], header: true)",
        )
        .expect("pre-chart table must still deserialize");
        assert!(t.chart.is_none());
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
    fn image_bytes_write_as_base64_and_still_read_the_old_decimal_form() {
        // A pre-base64 card: image bytes as a decimal array, which is what every
        // document, template and history snapshot on disk today contains. This
        // must keep loading byte-for-byte or people lose their screenshots.
        let legacy = r#"(
            id: 1, pos: (x: 0.0, y: 0.0), size: (x: 10.0, y: 10.0),
            title: "", body: "", color: (1, 2, 3),
            kind: Image(data: [137, 80, 78, 71, 13, 10, 26, 10], name: "old.png"),
        )"#;
        let card: Card = ron::from_str(legacy).expect("decimal image bytes still load");
        let imgs = card.kind.images();
        assert_eq!(imgs[0].0, &[137, 80, 78, 71, 13, 10, 26, 10]);

        // Re-saving that card writes base64 — and that reloads identically.
        let out = ron::ser::to_string(&card).unwrap();
        assert!(out.contains("iVBORw0KGgo="), "expected base64, got: {out}");
        assert!(!out.contains("137, 80"), "decimal array should be gone: {out}");
        let back: Card = ron::from_str(&out).expect("base64 image bytes load");
        assert_eq!(back.kind.images()[0].0, &[137, 80, 78, 71, 13, 10, 26, 10]);

        // Multi-image cards keep both arms too (`extra` is a Vec<ImageEntry>).
        let multi = r#"(
            id: 2, pos: (x: 0.0, y: 0.0), size: (x: 10.0, y: 10.0),
            title: "", body: "", color: (1, 2, 3),
            kind: Image(data: [1, 2], name: "a.png",
                        extra: [(data: [3, 4], name: "b.png")], ocr: ""),
        )"#;
        let card: Card = ron::from_str(multi).expect("legacy multi-image loads");
        let round: Card = ron::from_str(&ron::ser::to_string(&card).unwrap()).unwrap();
        let imgs = round.kind.images();
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[0].0, &[1, 2]);
        assert_eq!(imgs[1].0, &[3, 4]);

        // The point of the exercise: base64 is far smaller than the decimal form.
        let big = Card::new(
            3,
            egui::pos2(0.0, 0.0),
            CardKind::Image {
                data: (0u8..=255).cycle().take(60_000).collect(),
                name: "big.png".into(),
                extra: Vec::new(),
                ocr: String::new(),
            },
        );
        let encoded = ron::ser::to_string(&big).unwrap();
        assert!(
            encoded.len() < 60_000 * 2,
            "60k bytes should serialize near 80k chars, got {}",
            encoded.len()
        );
        let back: Card = ron::from_str(&encoded).unwrap();
        assert_eq!(back.kind.images()[0].0.len(), 60_000);
    }

    #[test]
    fn autofit_cols_sizes_columns_to_their_longest_cell() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let c = doc
            .add_card(
                n,
                egui::pos2(0.0, 0.0),
                CardKind::Table {
                    table: TableData::from_values(vec![
                        vec!["Check".into(), "Notes".into()],
                        vec![
                            "DNS".into(),
                            "a long note that would clip badly at the default width".into(),
                        ],
                    ]),
                },
            )
            .unwrap();

        // A fresh table has no explicit widths at all: every column renders at
        // the default, which is exactly the situation this op fixes.
        let CardKind::Table { table } = &doc.nodes[&n].cards[0].kind else { panic!() };
        assert!(table.col_widths.is_empty());
        assert_eq!(table.col_width(1), TABLE_DEFAULT_COL_W);

        assert!(doc.table_autofit_cols(n, c, None));
        let CardKind::Table { table } = &doc.nodes[&n].cards[0].kind else { panic!() };
        // The wordy column grows; the short one doesn't grow past it.
        assert!(
            table.col_width(1) > TABLE_DEFAULT_COL_W,
            "wide column should widen, got {}",
            table.col_width(1)
        );
        assert!(table.col_width(0) < table.col_width(1));

        // Width is measured per glyph, not per character: the same number of
        // characters needs far more room in capitals than in narrow lowercase.
        // A flat average clipped "UPGRADE" and "WWWWW MMMMM QQQQQ" on screen.
        assert!(
            cell_text_width("WWWWW") > cell_text_width("iiiii") * 2.0,
            "wide glyphs must not be averaged away"
        );
        assert!(cell_text_width("UPGRADE") + 12.0 < 110.0, "sanity: still a narrow column");

        // Bounded: a pathological cell can't produce an unusable card.
        doc.table_set_cell(n, c, 1, 1, "x".repeat(500));
        doc.table_autofit_cols(n, c, None);
        let CardKind::Table { table } = &doc.nodes[&n].cards[0].kind else { panic!() };
        assert_eq!(table.col_width(1), TABLE_MAX_COL_W);

        // `col` fits just that column and leaves the others alone.
        doc.table_set_col_width(n, c, 0, 300.0);
        assert!(doc.table_autofit_cols(n, c, Some(1)));
        let CardKind::Table { table } = &doc.nodes[&n].cards[0].kind else { panic!() };
        assert_eq!(table.col_width(0), 300.0, "untargeted column must not move");

        // Out of range is a failure, not a silent no-op.
        assert!(!doc.table_autofit_cols(n, c, Some(9)));

        // Not a table at all.
        let t = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        assert!(!doc.table_autofit_cols(n, t, None));
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
            next_item_id: 1,
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
            next_item_id: 1,
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
            next_item_id: 1,
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
                    items: vec![ChecklistItem { id: 0, done: true, text: "done item".into() }],
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
            next_item_id: 1,
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

    /// DejaVu has no emoji at all, so before the fallback these came out blank.
    /// Pinned at the font-stack level rather than by inspecting pixels: what
    /// went wrong was "no font in the stack can draw this", and that is exactly
    /// what this asserts.
    #[test]
    fn the_export_font_stack_covers_emoji_dejavu_lacks() {
        use ab_glyph::Font as _;
        let fonts = ExportFonts::load().expect("fonts load");
        for (c, name) in [
            ('\u{1F534}', "red circle"),
            // Post-Unicode-12, and missing from the subset egui bundles — the
            // glyph that started this.
            ('\u{1F7E2}', "green circle"),
            ('\u{2705}', "check mark"),
            ('\u{1F6D1}', "stop sign"),
        ] {
            let (font, is_emoji) = fonts.pick(c);
            assert!(is_emoji, "{name} should come from the emoji font");
            assert_ne!(font.glyph_id(c).0, 0, "{name} has no glyph");
        }
        // Ordinary text must still come from DejaVu, not be dragged into the
        // emoji font by an over-eager fallback.
        assert!(!fonts.pick('A').1);
        assert!(!fonts.pick('—').1);
    }

    /// An emoji in a card must not break the exporters, and must be placed as
    /// its own run rather than dropped from the line.
    #[test]
    fn exports_render_a_card_containing_emoji() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Status \u{1F7E2}".into());
        let c = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(n, c).unwrap().body = "build \u{2705} deploy \u{1F534} rollback".into();

        let fonts = ExportFonts::load().unwrap();
        let runs = fonts.runs("build \u{2705} ok");
        assert_eq!(runs.len(), 3, "text | emoji | text");
        assert!(!runs[0].1 && runs[1].1 && !runs[2].1);
        assert_eq!(runs[1].0, "\u{2705}");

        let pdf = doc.export_pdf().expect("pdf with emoji");
        assert!(pdf.starts_with(b"%PDF"));
        let png = doc.export_image(false).expect("png with emoji");
        assert_eq!(&png[1..4], b"PNG");
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
                    ChecklistItem { id: 0, done: true, text: "a".into() },
                    ChecklistItem { id: 0, done: false, text: "b".into() },
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
            next_item_id: 1,
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
            ChecklistItem { id: 0, done: false, text: "a".into() },
            ChecklistItem { id: 0, done: false, text: "b".into() },
            ChecklistItem { id: 0, done: false, text: "c".into() },
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
                    id: 0,
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

    #[test]
    fn a_group_link_resolves_and_does_not_collide_with_a_card_link() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "Basket".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let g = doc.group_cards(n, &[a, b], "pair".into()).unwrap();
        assert_eq!(doc.locate_group(g), Some(n));
        assert_eq!(
            doc.resolve_link_target(&format!("#g{g}")),
            Some(LinkTarget::Group { node: n, group: g })
        );
        // Upper case is accepted the way `#` links already tolerate spacing.
        assert_eq!(
            doc.resolve_link_target(&format!("#G{g}")),
            Some(LinkTarget::Group { node: n, group: g })
        );
        // The card form is untouched, and the two id spaces stay separate: a
        // group id and a card id may be the same number and mean different
        // things.
        assert_eq!(
            doc.resolve_link_target(&format!("#{a}")),
            Some(LinkTarget::Card { node: n, card: a })
        );
        // An unknown group is no link at all, never a fall-through to a title.
        assert_eq!(doc.resolve_link_target("#g9999"), None);
        // `resolve_link` collapses a group to its basket, like a card link.
        assert_eq!(doc.resolve_link(&format!("#g{g}")), Some(n));
    }

    #[test]
    fn a_group_moves_to_another_basket_whole() {
        let mut doc = Document::empty();
        let from = doc.add_node(None, "from".into());
        let to = doc.add_node(None, "to".into());
        let a = doc.add_card(from, egui::pos2(100.0, 100.0), CardKind::Text).unwrap();
        let b = doc.add_card(from, egui::pos2(140.0, 220.0), CardKind::Text).unwrap();
        let loose = doc.add_card(from, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let g = doc.group_cards(from, &[a, b], "design".into()).unwrap();
        // A card left behind that was docked to one that leaves must be cut, or
        // it would name a card in another basket.
        doc.card_mut(from, loose).unwrap().docked_to = Some(a);

        let moved = doc.move_group_to_node(from, g, to, Some(egui::pos2(10.0, 10.0)));
        assert_eq!(moved, Some(2));

        // The id survives, which is the whole point: `[[#g…]]` still resolves.
        assert_eq!(doc.locate_group(g), Some(to));
        assert_eq!(
            doc.resolve_link_target(&format!("#g{g}")),
            Some(LinkTarget::Group { node: to, group: g })
        );
        // Title and colour came with it, and membership held.
        let dest = &doc.nodes[&to];
        assert_eq!(dest.groups.iter().find(|x| x.id == g).map(|x| x.title.as_str()), Some("design"));
        assert_eq!(doc.card(to, a).unwrap().group, Some(g));
        assert_eq!(doc.card(to, b).unwrap().group, Some(g));
        // Relative layout preserved: `a` lands on the requested corner and `b`
        // keeps its offset from it.
        assert_eq!(doc.card(to, a).unwrap().pos, egui::pos2(10.0, 10.0));
        assert_eq!(doc.card(to, b).unwrap().pos, egui::pos2(50.0, 130.0));
        // Source is left clean.
        assert!(doc.nodes[&from].groups.iter().all(|x| x.id != g));
        assert!(doc.card(from, a).is_none());
        assert_eq!(doc.card(from, loose).unwrap().docked_to, None);
        // A second move to the same basket is refused rather than silently
        // duplicating the container.
        assert_eq!(doc.move_group_to_node(to, g, to, None), None);
        assert_eq!(doc.move_group_to_node(to, 9999, from, None), None);
    }

    #[test]
    fn group_backlinks_find_cards_that_name_it() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let other = doc.add_node(None, "other".into());
        let a = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        let g = doc.group_cards(n, &[a, b], "pair".into()).unwrap();
        let pointer = doc.add_card(other, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(other, pointer).unwrap().body = format!("see [[#g{g}]] for the design");
        // A card pointing at a *card* is not a group backlink.
        let card_pointer = doc.add_card(other, egui::pos2(0.0, 0.0), CardKind::Text).unwrap();
        doc.card_mut(other, card_pointer).unwrap().body = format!("see [[#{a}]]");

        let hits = doc.backlinks_group(g);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].card, Some(pointer));
    }

    /// A card holding HTML showed nothing at all before this: CommonMark passes
    /// a raw HTML block through, and the card renderer draws no HTML.
    #[test]
    fn block_html_becomes_markdown_and_inline_html_is_left_alone() {
        use std::borrow::Cow;
        // Nothing to do — and cheaply, without a second parse.
        assert!(matches!(html_blocks_to_md("plain **markdown**"), Cow::Borrowed(_)));
        assert!(matches!(html_blocks_to_md("2 < 3 and 4 > 1"), Cow::Borrowed(_)));

        // The colour span is how a card stores its text colour. If this is ever
        // converted, every coloured card loses its colour.
        let colored = r#"<span style="color:#ff0000">red</span> text"#;
        assert!(matches!(html_blocks_to_md(colored), Cow::Borrowed(_)), "inline HTML was rewritten");

        let table = "before\n\n<table><tr><th>a</th></tr><tr><td>1</td></tr></table>\n\nafter";
        let out = html_blocks_to_md(table);
        assert!(out.starts_with("before"), "{out:?}");
        assert!(out.trim_end().ends_with("after"), "{out:?}");
        assert!(!out.contains("<table>"), "the markup survived: {out:?}");
        assert!(out.contains('a') && out.contains('1'), "the content was lost: {out:?}");

        // A heading is the simplest proof the conversion is real, not a strip.
        let out = html_blocks_to_md("<h2>Title</h2>");
        assert!(out.contains("Title"));
        assert!(!out.contains("<h2>"), "{out:?}");
    }

    /// The repair has to keep the layout, or it is just autosort with extra
    /// steps: every `x` survives, cards only ever move down, and a basket that
    /// does not overlap is not touched at all.
    #[test]
    fn resolve_overlaps_pushes_down_and_keeps_every_column() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let a = doc.add_card(n, egui::pos2(40.0, 40.0), CardKind::Text).unwrap();
        let b = doc.add_card(n, egui::pos2(40.0, 60.0), CardKind::Text).unwrap();
        let far = doc.add_card(n, egui::pos2(900.0, 40.0), CardKind::Text).unwrap();
        for (id, size) in [(a, [200.0, 100.0]), (b, [200.0, 100.0]), (far, [200.0, 100.0])] {
            doc.card_mut(n, id).unwrap().size = egui::vec2(size[0], size[1]);
        }
        assert_eq!(doc.overlapping_cards(n), vec![(a, b)], "a and b cover each other");

        let moved = doc.resolve_overlaps(n);
        assert_eq!(moved, 1, "only the card in the way moves");
        assert!(doc.overlapping_cards(n).is_empty(), "still overlapping");
        // The column is the layout: x is never touched, and nothing moves up.
        assert_eq!(doc.card(n, a).unwrap().pos, egui::pos2(40.0, 40.0), "the first card held still");
        assert_eq!(doc.card(n, b).unwrap().pos.x, 40.0, "b left its column");
        assert!(doc.card(n, b).unwrap().pos.y >= 140.0, "b did not clear a");
        assert_eq!(doc.card(n, far).unwrap().pos, egui::pos2(900.0, 40.0), "an innocent card moved");

        // Idempotent: a tidy basket is left exactly as it is.
        assert_eq!(doc.resolve_overlaps(n), 0);
    }

    /// Cards that travel together are *meant* to sit on each other. Reporting a
    /// dock stack as an overlap would make the check cry wolf on every basket
    /// that uses docking, and "repairing" it would tear the stack apart.
    #[test]
    fn a_dock_stack_is_one_block_not_an_overlap() {
        let mut doc = Document::empty();
        let n = doc.add_node(None, "n".into());
        let host = doc.add_card(n, egui::pos2(40.0, 40.0), CardKind::Text).unwrap();
        let stuck = doc.add_card(n, egui::pos2(50.0, 50.0), CardKind::Text).unwrap();
        for id in [host, stuck] {
            doc.card_mut(n, id).unwrap().size = egui::vec2(200.0, 100.0);
        }
        doc.dock_card(n, stuck, host);
        assert!(doc.overlapping_cards(n).is_empty(), "the stack reported itself");
        assert_eq!(doc.resolve_overlaps(n), 0, "the stack was pulled apart");

        // A third card landing on the stack is a real overlap, and the stack
        // keeps its shape when the intruder is pushed clear.
        let other = doc.add_card(n, egui::pos2(60.0, 60.0), CardKind::Text).unwrap();
        doc.card_mut(n, other).unwrap().size = egui::vec2(200.0, 100.0);
        assert!(!doc.overlapping_cards(n).is_empty());
        doc.resolve_overlaps(n);
        assert!(doc.overlapping_cards(n).is_empty());
        let (h, s) = (doc.card(n, host).unwrap().pos, doc.card(n, stuck).unwrap().pos);
        assert_eq!(s - h, egui::vec2(10.0, 10.0), "the stack lost its shape");
    }

    /// Collapse-all is recursive, which is the whole decision: it must reach
    /// nodes that are not roots, or reopening a project shows a shape nobody
    /// chose. `changed` counts what actually moved, so a second call is 0.
    #[test]
    fn set_all_expanded_folds_every_root_and_everything_under_it() {
        let mut doc = Document::empty();
        let a = doc.add_node(None, "a".into());
        let b = doc.add_node(None, "b".into());
        let a1 = doc.add_node(Some(a), "a1".into());
        let a2 = doc.add_node(Some(a1), "a2".into());

        assert_eq!(doc.set_all_expanded(true), 0, "new nodes are already expanded");
        let changed = doc.set_all_expanded(false);
        assert_eq!(changed, 4, "both roots and both descendants");
        for id in [a, b, a1, a2] {
            assert!(!doc.nodes[&id].expanded, "{id} was left open");
        }
        assert_eq!(doc.set_all_expanded(false), 0, "nothing left to fold");
        assert_eq!(doc.set_all_expanded(true), 4);
    }

    /// Emphasis lapses on its own, and that is the whole point of the field.
    ///
    /// An agent that can highlight for ever produces a document where everything
    /// is highlighted, so `emphasis_until` is checked at draw time rather than
    /// trusted from the field.
    #[test]
    fn emphasis_expires_without_the_document_being_rewritten() {
        let mut c = Card::new(1, egui::pos2(0.0, 0.0), CardKind::Text);
        c.emphasis = Emphasis::Pulse;
        c.emphasis_until = Some(1_000);

        assert_eq!(c.live_emphasis(999), Emphasis::Pulse, "still inside its window");
        assert_eq!(c.live_emphasis(1_000), Emphasis::Pulse, "the last second counts");
        assert_eq!(c.live_emphasis(1_001), Emphasis::None, "lapsed");
        // The field itself is untouched: a lapse is not an edit, and rewriting it
        // would stamp `touched` and fill the change log with things nobody did.
        assert_eq!(c.emphasis, Emphasis::Pulse);

        // No expiry means "until someone turns it off" — what a person setting it
        // by hand gets.
        c.emphasis_until = None;
        assert_eq!(c.live_emphasis(i64::MAX), Emphasis::Pulse);
    }

    /// A card written before emphasis existed loads with none of it, and a card
    /// with none of it writes nothing — the file does not grow for a feature
    /// nobody used.
    #[test]
    fn emphasis_is_absent_from_a_card_that_has_none() {
        let c = Card::new(1, egui::pos2(0.0, 0.0), CardKind::Text);
        let ron = ron::ser::to_string(&c).unwrap();
        assert!(!ron.contains("emphasis"), "{ron}");
        let back: Card = ron::from_str(&ron).unwrap();
        assert_eq!(back.emphasis, Emphasis::None);
        assert_eq!(back.emphasis_intensity, 1.0, "the default survives a round trip");
    }
}
