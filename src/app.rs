//! Application state and the eframe update loop that stitches the panels
//! together: menu bar, tree, basket canvas, search, and all file operations.

use crate::canvas::{self, CanvasAction, Env};
use crate::images::TextureCache;
use crate::model::{CardId, CardKind, ChecklistItem, Document, NodeId};
use crate::tree::{self, TreeAction};
use crate::api::{self, ApiCommand};
use egui_commonmark::CommonMarkCache;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use raw_window_handle::{HasDisplayHandle as _, HasWindowHandle as _};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Parent-window handles for rfd dialogs. Without a parent, X11/portal file
/// dialogs get no transient-for hint and can open *behind* the app window.
/// Raw handles are `Copy + 'static`, so they can be captured each frame from
/// eframe and lent back out for the (blocking, modal) dialog call, during
/// which the window is guaranteed alive.
#[derive(Clone, Copy)]
struct DialogParent {
    window: raw_window_handle::RawWindowHandle,
    display: raw_window_handle::RawDisplayHandle,
}

impl raw_window_handle::HasWindowHandle for DialogParent {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.window) })
    }
}

impl raw_window_handle::HasDisplayHandle for DialogParent {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(self.display) })
    }
}
use emath::TSTransform;

const MIN_CARD: egui::Vec2 = egui::Vec2::new(140.0, 90.0);

/// State of the full-screen image viewer (shadowbox).
struct Lightbox {
    node: NodeId,
    card: crate::model::CardId,
    /// Display index into the card's image list.
    index: usize,
    /// Zoom on top of fit-to-screen; 1.0 = fit.
    zoom: f32,
    /// Drag offset from screen center, in points.
    pan: egui::Vec2,
}

/// eframe storage keys.
const LAST_DOC_KEY: &str = "last_doc_path";
const API_KEY_KEY: &str = "api_key";
const API_PORT_KEY: &str = "api_port";
const API_LAN_KEY: &str = "api_lan";
const TEMPLATES_KEY: &str = "card_templates";
const GRANTS_KEY: &str = "plugin_grants";
const MIRROR_MODE_KEY: &str = "mirror_policy";
const MIRROR_DIRS_KEY: &str = "mirror_dirs";
const DEFAULT_API_PORT: u16 = 7373;
const ZOOM_ENABLED_KEY: &str = "zoom_enabled";
const DOCK_MODE_KEY: &str = "dock_mode";
const SNAP_MODE_KEY: &str = "snap_mode";
const MINIMAP_KEY: &str = "minimap";
const THEME_KEY: &str = "theme";
const AUTOSAVE_KEY: &str = "autosave";
const AGENDA_PROJECT_KEY: &str = "agenda_project";
const KANBAN_PROJECT_KEY: &str = "kanban_project";
const BACKUP_KEY: &str = "backup";
const HISTORY_KEEP_KEY: &str = "history_keep";
const HISTORY_GAP_KEY: &str = "history_gap_mins";
/// How long the document must be idle (no further changes) before an autosave
/// fires — so continuous editing (e.g. dragging a card) never saves mid-gesture.
const AUTOSAVE_IDLE: Duration = Duration::from_secs(2);

/// Selectable themes, listed under **View → Themes**. `Trellis` is the default
/// signature look (dark chrome + black grid); Light/Terminal Green are alternate
/// color schemes. To add a richer theme (e.g. StickyNotes, Futuristic) that
/// styles windows and colors differently, add a variant here, to `ALL`, and to
/// `from_key`/`key`/`visuals`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Theme {
    Trellis,
    Light,
    TerminalGreen,
    StickyNotes,
    Futuristic,
    SynthWave,
}

impl Theme {
    const ALL: [(Theme, &'static str); 6] = [
        (Theme::Trellis, "Trellis"),
        (Theme::Light, "Light"),
        (Theme::TerminalGreen, "Terminal Green"),
        (Theme::StickyNotes, "Sticky Notes"),
        (Theme::Futuristic, "Futuristic"),
        (Theme::SynthWave, "SynthWave"),
    ];

    fn from_key(s: &str) -> Theme {
        match s {
            "Light" => Theme::Light,
            "TerminalGreen" => Theme::TerminalGreen,
            "StickyNotes" => Theme::StickyNotes,
            "Futuristic" => Theme::Futuristic,
            "SynthWave" => Theme::SynthWave,
            // "Dark" is the pre-rename key for the default look; keep it mapping
            // to Trellis so existing settings load unchanged.
            _ => Theme::Trellis,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Theme::Trellis => "Trellis",
            Theme::Light => "Light",
            Theme::TerminalGreen => "TerminalGreen",
            Theme::StickyNotes => "StickyNotes",
            Theme::Futuristic => "Futuristic",
            Theme::SynthWave => "SynthWave",
        }
    }

    fn visuals(self) -> egui::Visuals {
        match self {
            Theme::Light => egui::Visuals::light(),
            Theme::Trellis => egui::Visuals::dark(),
            Theme::TerminalGreen => terminal_green_visuals(),
            Theme::StickyNotes => sticky_notes_visuals(),
            Theme::Futuristic => futuristic_visuals(),
            Theme::SynthWave => synthwave_visuals(),
        }
    }
}

/// A phosphor green-on-black terminal scheme.
fn terminal_green_visuals() -> egui::Visuals {
    use egui::{Color32, Stroke};
    let green = Color32::from_rgb(0x33, 0xff, 0x6a);
    let dim = Color32::from_rgb(0x1e, 0xa8, 0x48);
    let bg = Color32::from_rgb(0x04, 0x09, 0x05);
    let panel = Color32::from_rgb(0x08, 0x10, 0x0a);

    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(green);
    v.hyperlink_color = green;
    v.panel_fill = panel;
    v.window_fill = panel;
    v.extreme_bg_color = bg;
    v.faint_bg_color = Color32::from_rgb(0x0c, 0x17, 0x0f);
    v.code_bg_color = bg;
    v.window_stroke = Stroke::new(1.0, dim);
    v.selection.bg_fill = green.gamma_multiply(0.22);
    v.selection.stroke = Stroke::new(1.0, green);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = panel;
    w.noninteractive.weak_bg_fill = panel;
    w.noninteractive.fg_stroke = Stroke::new(1.0, dim);
    w.inactive.bg_fill = Color32::from_rgb(0x0e, 0x1c, 0x12);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x0e, 0x1c, 0x12);
    w.inactive.fg_stroke = Stroke::new(1.0, green);
    w.hovered.bg_fill = Color32::from_rgb(0x13, 0x28, 0x19);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x13, 0x28, 0x19);
    w.hovered.fg_stroke = Stroke::new(1.5, green);
    w.hovered.bg_stroke = Stroke::new(1.0, dim);
    w.active.bg_fill = Color32::from_rgb(0x18, 0x33, 0x20);
    w.active.weak_bg_fill = Color32::from_rgb(0x18, 0x33, 0x20);
    w.active.fg_stroke = Stroke::new(1.5, green);
    w.active.bg_stroke = Stroke::new(1.0, green);
    w.open.fg_stroke = Stroke::new(1.0, green);
    v
}

/// Warm sticky-note look: yellow-paper cards on a cork board, dark-ink text.
fn sticky_notes_visuals() -> egui::Visuals {
    use egui::{Color32, Stroke};
    let paper = Color32::from_rgb(0xff, 0xee, 0x93); // sticky yellow (card / panel fill)
    let paper_hi = Color32::from_rgb(0xff, 0xf6, 0xc2);
    let board = Color32::from_rgb(0x9c, 0x7c, 0x4c); // cork board (canvas background)
    let ink = Color32::from_rgb(0x39, 0x2b, 0x10); // dark brown ink

    let mut v = egui::Visuals::light();
    v.override_text_color = Some(ink);
    v.hyperlink_color = Color32::from_rgb(0x8a, 0x53, 0x00);
    v.panel_fill = paper;
    v.window_fill = paper;
    v.extreme_bg_color = board;
    v.faint_bg_color = paper_hi;
    v.code_bg_color = Color32::from_rgb(0xf3, 0xdd, 0x82);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(0xd8, 0xbd, 0x64));
    v.selection.bg_fill = Color32::from_rgb(0xff, 0xc9, 0x4d).gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(0xc9, 0x7a, 0x00));

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = paper;
    w.noninteractive.weak_bg_fill = paper;
    w.noninteractive.fg_stroke = Stroke::new(1.0, ink);
    w.inactive.bg_fill = paper_hi;
    w.inactive.weak_bg_fill = paper_hi;
    w.inactive.fg_stroke = Stroke::new(1.0, ink);
    w.hovered.bg_fill = Color32::from_rgb(0xff, 0xe0, 0x6b);
    w.hovered.weak_bg_fill = Color32::from_rgb(0xff, 0xe0, 0x6b);
    w.hovered.fg_stroke = Stroke::new(1.5, ink);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xd8, 0xa8, 0x2a));
    w.active.bg_fill = Color32::from_rgb(0xff, 0xd4, 0x4a);
    w.active.weak_bg_fill = Color32::from_rgb(0xff, 0xd4, 0x4a);
    w.active.fg_stroke = Stroke::new(1.5, ink);
    w.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xc9, 0x7a, 0x00));
    v
}

/// A translucent-cyan sci-fi HUD ("Minority Report") look on near-black.
fn futuristic_visuals() -> egui::Visuals {
    use egui::{Color32, Stroke};
    let cyan = Color32::from_rgb(0x4d, 0xe4, 0xff);
    let dim = Color32::from_rgb(0x35, 0xa6, 0xcc);
    let text = Color32::from_rgb(0xe2, 0xfb, 0xff);
    let bg = Color32::from_rgb(0x03, 0x0a, 0x12);
    // A distinctly teal-tinted panel (lifted well off Trellis's neutral gray so
    // the two dark themes don't read the same).
    let panel = Color32::from_rgb(0x0b, 0x22, 0x30);

    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(text);
    v.hyperlink_color = cyan;
    v.panel_fill = panel;
    v.window_fill = panel;
    v.extreme_bg_color = bg;
    v.faint_bg_color = Color32::from_rgb(0x0d, 0x28, 0x38);
    v.code_bg_color = bg;
    v.window_stroke = Stroke::new(1.0, dim);
    v.selection.bg_fill = cyan.gamma_multiply(0.22);
    v.selection.stroke = Stroke::new(1.0, cyan);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = panel;
    w.noninteractive.weak_bg_fill = panel;
    w.noninteractive.fg_stroke = Stroke::new(1.0, dim);
    w.inactive.bg_fill = Color32::from_rgb(0x11, 0x2b, 0x3b);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x11, 0x2b, 0x3b);
    w.inactive.fg_stroke = Stroke::new(1.0, text);
    w.hovered.bg_fill = Color32::from_rgb(0x18, 0x3a, 0x4e);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x18, 0x3a, 0x4e);
    w.hovered.fg_stroke = Stroke::new(1.5, cyan);
    w.hovered.bg_stroke = Stroke::new(1.0, dim);
    w.active.bg_fill = Color32::from_rgb(0x1e, 0x48, 0x60);
    w.active.weak_bg_fill = Color32::from_rgb(0x1e, 0x48, 0x60);
    w.active.fg_stroke = Stroke::new(1.5, cyan);
    w.active.bg_stroke = Stroke::new(1.0, cyan);
    w.open.fg_stroke = Stroke::new(1.0, cyan);
    v
}

/// Synthwave / Hotline-Miami: a dark, near-black interface (readable, not a wash
/// of purple) with hot pink + electric cyan used only as *accents* — edges,
/// strokes, selection, active widgets, links — over the dark chrome.
fn synthwave_visuals() -> egui::Visuals {
    use egui::{Color32, Stroke};
    let pink = Color32::from_rgb(0xff, 0x3b, 0x6b); // hot pink (accent only)
    let cyan = Color32::from_rgb(0x2d, 0xe6, 0xf0); // electric cyan (accent only)
    let magenta = Color32::from_rgb(0xb0, 0x24, 0x6e); // muted magenta chrome edge
    let text = Color32::from_rgb(0xe8, 0xe6, 0xee); // clean light lavender-white
    let bright = Color32::from_rgb(0xf6, 0xf3, 0xfc); // strong/emphasis text (near-white)
    let dim = Color32::from_rgb(0x92, 0x88, 0xa6); // muted lavender-grey (secondary)
    let bg = Color32::from_rgb(0x08, 0x06, 0x0d); // near-black, faint cool violet
    let panel = Color32::from_rgb(0x15, 0x12, 0x1e); // dark panel (barely-there violet)

    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(text);
    v.hyperlink_color = cyan;
    v.panel_fill = panel;
    v.window_fill = panel;
    v.extreme_bg_color = bg;
    v.faint_bg_color = Color32::from_rgb(0x1c, 0x18, 0x28);
    v.code_bg_color = bg;
    v.window_stroke = Stroke::new(1.0, magenta);
    v.selection.bg_fill = pink.gamma_multiply(0.28);
    v.selection.stroke = Stroke::new(1.0, pink);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = panel;
    w.noninteractive.weak_bg_fill = panel;
    w.noninteractive.fg_stroke = Stroke::new(1.0, dim);
    w.inactive.bg_fill = Color32::from_rgb(0x20, 0x1b, 0x2c);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x20, 0x1b, 0x2c);
    w.inactive.fg_stroke = Stroke::new(1.0, text);
    w.hovered.bg_fill = Color32::from_rgb(0x2a, 0x23, 0x38);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x2a, 0x23, 0x38);
    w.hovered.fg_stroke = Stroke::new(1.5, cyan);
    w.hovered.bg_stroke = Stroke::new(1.0, cyan);
    w.active.bg_fill = Color32::from_rgb(0x33, 0x2a, 0x42);
    w.active.weak_bg_fill = Color32::from_rgb(0x33, 0x2a, 0x42);
    // Strong/emphasis text (egui's strong_text_color = active.fg) must be readable,
    // NOT the loud pink — pink stays an accent on the active border below, on
    // selection, links, and window edges. Otherwise every `.strong()` label
    // (Kanban titles, search/tag/agenda/backlinks headers) renders pink.
    w.active.fg_stroke = Stroke::new(1.5, bright);
    w.active.bg_stroke = Stroke::new(1.0, pink);
    w.open.fg_stroke = Stroke::new(1.0, bright);
    v
}

/// How a canvas action participates in undo history.
enum UndoKind {
    /// Not an undo point (text typing, selection, view/zoom, z-order).
    None,
    /// A one-shot edit; each gets its own undo step.
    Discrete,
    /// Part of a continuous gesture (a drag); frames sharing this tag while the
    /// pointer is held collapse into a single undo step.
    Continuous(&'static str),
}

fn undo_kind(a: &CanvasAction) -> UndoKind {
    use CanvasAction as A;
    match a {
        A::MoveCard(..) | A::MoveGroup(..) => UndoKind::Continuous("move"),
        A::ResizeCard(..) => UndoKind::Continuous("resize"),
        A::TableSetColWidth(..) => UndoKind::Continuous("colwidth"),
        A::AddCard(..)
        | A::PasteCard(_)
        | A::ImportCard(_)
        | A::DropFiles(..)
        | A::Remove(_)
        | A::Duplicate(_)
        | A::FitCard(_)
        | A::InsertTemplate(..)
        | A::SetColor(..)
        | A::SetFontScale(..)
        | A::ChecklistToggle(..)
        | A::ChecklistAdd(_)
        | A::ChecklistRemove(..)
        | A::ChecklistMove(..)
        | A::SketchAddStroke(..)
        | A::SketchUndo(_)
        | A::SketchClear(_)
        | A::LoadImage(_)
        | A::InsertInlineImage(..)
        | A::RemoveImage(..)
        | A::GroupSelected
        | A::Ungroup(_)
        | A::DockCard(..)
        | A::DetachCard(_)
        | A::SetGroupColor(..)
        | A::TableSetBg(..)
        | A::TableSetFg(..)
        | A::TableInsertRow(..)
        | A::TableRemoveRow(..)
        | A::TableInsertCol(..)
        | A::TableRemoveCol(..)
        | A::TableToggleHeader(_)
        | A::TableSetChart(..)
        | A::TableImport(_) => UndoKind::Discrete,
        _ => UndoKind::None,
    }
}

/// Filename to pre-fill when downloading an image from a card: the image's
/// stored name, or a synthesized `image-N.png` (1-based) when it has none, so a
/// nameless image still saves with a sensible name and extension.
fn download_image_name(stored: &str, index: usize) -> String {
    if stored.trim().is_empty() {
        format!("image-{}.png", index + 1)
    } else {
        stored.to_string()
    }
}

/// Error string when a card lookup fails during export (it was deleted between
/// opening the menu and the action running).
fn card_gone() -> String {
    "card not found".to_string()
}

/// A pending single-card WYSIWYG screenshot export, advanced across frames in
/// `update`: reframe the view onto the card, screenshot the framebuffer, crop to
/// the card, then save as PNG or PDF.
struct CardShot {
    node: NodeId,
    card: crate::model::CardId,
    /// `true` = export as PDF (embed the rendered image), `false` = PNG.
    pdf: bool,
    /// The node's view before we reframed onto the card; restored afterwards.
    saved_view: TSTransform,
    phase: ShotPhase,
}

enum ShotPhase {
    /// Reframe the view to fit the card, render one frame, then request a shot.
    Framing,
    /// Screenshot requested; waiting for the framebuffer event next frame.
    Requested,
}

/// Which visual file a basket screenshot export is producing.
#[derive(Clone, Copy)]
enum BasketFmt {
    /// A single overview image of the whole basket.
    Png,
    /// Overview page + one readable page per card (with selectable text).
    Pdf,
}

/// One target in a basket export's screenshot queue.
#[derive(Clone, Copy)]
enum ShotKind {
    /// The whole basket, fit in view — the overview page.
    Overview,
    /// A single card, fit in view — a readable per-card page.
    Card(crate::model::CardId),
}

/// A pending multi-shot basket export (overview + per-card pages), advanced one
/// screenshot per frame like [`CardShot`], collecting images until the queue is
/// drained, then assembling a PNG or WYSIWYG PDF.
struct BasketShot {
    node: NodeId,
    fmt: BasketFmt,
    saved_view: TSTransform,
    queue: Vec<ShotKind>,
    idx: usize,
    captured: Vec<crate::model::ShotPage>,
    phase: ShotPhase,
    /// Set once the canvas has actually rendered a frame with the current shot's
    /// reframe applied. The screenshot is only requested after this — otherwise
    /// the first shot (kicked off mid-frame) would capture the un-reframed view.
    framed: bool,
}

/// Encode a raw RGBA buffer as PNG bytes.
fn encode_png(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())
        .ok_or_else(|| "bad image buffer".to_string())?;
    let mut buf = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

/// A view transform that fits a card (with margin) centered in `canvas_rect` at
/// ≤100% zoom, so a screenshot captures the whole card unclipped.
fn framed_view(canvas_rect: egui::Rect, card_pos: egui::Pos2, card_size: egui::Vec2) -> TSTransform {
    let margin = 24.0_f32;
    let avail = (canvas_rect.size() - egui::Vec2::splat(margin * 2.0)).max(egui::Vec2::splat(1.0));
    let s = (avail.x / card_size.x.max(1.0))
        .min(avail.y / card_size.y.max(1.0))
        .min(1.0)
        .max(0.05);
    let scaled = card_size * s;
    let offset = (canvas_rect.size() - scaled) * 0.5;
    TSTransform { scaling: s, translation: offset - card_pos.to_vec2() * s }
}

/// Precise "Fit to content" size for the interactive right-click action.
///
/// For **Text** cards this measures the actual rendered text with egui's fonts,
/// laid out at the fitted card width, so the card's height matches what's really
/// drawn. `Card::fit_size` must stay egui-free (it runs off the UI thread for the
/// API / import path), so it can only *estimate* — and its estimate ran ~2× tall
/// when a long title widened the card past the width it measured wrapping at. On
/// the UI thread we have the real font metrics and the true wrap width, so we use
/// them. Non-text kinds fall back to the estimate (which drives their width too).
fn fit_card_size(ctx: &egui::Context, c: &crate::model::Card) -> Option<egui::Vec2> {
    let base = c.fit_size()?;
    if !matches!(c.kind, CardKind::Text) {
        return Some(base);
    }
    const TITLE_H: f32 = 24.0;
    const PAD: f32 = 6.0;
    const MIN_H: f32 = 90.0;
    let fs = if c.font_scale > 0.0 { c.font_scale } else { 1.0 };
    let w = base.x; // keep the estimate's width; only the height was wrong
    let wrap_w = (w - PAD * 2.0).max(1.0);
    // Same text the CommonMark view shows: image markers → alt text, zero-width
    // markup (`*`, `` ` ``) dropped. Single newlines already break lines in a
    // galley, matching the card's hard-wrap render.
    let text = crate::model::strip_size_markup(&crate::model::strip_inline_markers(&c.body));

    // Read the sizes the renderer will actually use rather than assuming: the
    // card body draws at TextStyle::Body and headings scale up towards
    // TextStyle::Heading. Measuring everything at one size under-counts a
    // heading-heavy card, which is what clipped long notes.
    let (body_px, heading_px) = {
        let style = ctx.style();
        (
            style.text_styles.get(&egui::TextStyle::Body).map_or(12.5, |f| f.size),
            style.text_styles.get(&egui::TextStyle::Heading).map_or(18.0, |f| f.size),
        )
    };

    // Measure line by line so each heading is laid out at its own size. The card
    // hard-wraps before rendering, so a line here is a line there.
    let mut content_h = 0.0;
    for line in text.lines() {
        let level = crate::model::heading_level(line);
        let px = match level {
            Some(l) => crate::model::heading_font_px(l, body_px, heading_px),
            None => body_px,
        } * fs;
        let galley = ctx.fonts(|f| {
            f.layout(line.to_owned(), egui::FontId::proportional(px), egui::Color32::WHITE, wrap_w)
        });
        content_h += galley.size().y;
        if level.is_some() {
            content_h += body_px * fs * 0.5; // the newline the renderer inserts
        }
    }
    if text.lines().next().is_none() {
        content_h = body_px * fs;
    }
    for (_iw, ih) in c.inline_image_sizes(wrap_w) {
        content_h += ih + 6.0; // inline images stack under the text
    }
    let h = (TITLE_H + PAD * 2.0 + content_h).clamp(MIN_H, crate::model::FIT_MAX_H);
    Some(egui::vec2(w, h))
}

/// A short human label for a card kind, used in status messages.
fn card_kind_label(kind: &CardKind) -> &'static str {
    match kind {
        CardKind::Text => "text",
        CardKind::Code { .. } => "code",
        CardKind::Checklist { .. } => "checklist",
        CardKind::Table { .. } => "table",
        CardKind::Image { .. } => "image",
        CardKind::Sketch { .. } => "sketch",
    }
}

/// One draggable Kanban card: an accent-colored frame with the title, its `due::`
/// date (red when overdue), up to three `#tags`, and its basket. Click to reveal
/// the card; drag it to another column to change its `status`.
fn kanban_card_ui(
    ui: &mut egui::Ui,
    kc: &crate::model::KanbanCard,
    today: i64,
    pcolor: egui::Color32,
    jump: &mut Option<(NodeId, CardId)>,
) {
    let accent = egui::Color32::from_rgb(kc.color[0], kc.color[1], kc.color[2]);
    let src = ui.dnd_drag_source(
        egui::Id::new(("kb", kc.node, kc.card)),
        (kc.node, kc.card),
        |ui| {
            egui::Frame::none()
                .fill(ui.visuals().faint_bg_color)
                .stroke(egui::Stroke::new(1.5, accent))
                .rounding(4.0)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width()); // fill the column
                    ui.label(egui::RichText::new(&kc.title).strong());
                    if kc.due.is_some() || !kc.tags.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            if let Some(due) = &kc.due {
                                let overdue =
                                    crate::model::parse_ymd(due).map_or(false, |d| d < today);
                                let col = if overdue {
                                    egui::Color32::from_rgb(0xff, 0x6b, 0x6b)
                                } else {
                                    ui.visuals().weak_text_color()
                                };
                                ui.small(egui::RichText::new(format!("⏳ {due}")).color(col));
                            }
                            for t in kc.tags.iter().take(3) {
                                ui.small(egui::RichText::new(format!("#{t}")).weak());
                            }
                        });
                    }
                    // Project half in its own colour so a mixed board still
                    // groups by project at a glance (see the Agenda).
                    let mut job = egui::text::LayoutJob::default();
                    let small = egui::TextStyle::Small.resolve(ui.style());
                    let (proj, rest) = match kc.node_path.split_once(" › ") {
                        Some((a, b)) => (a.to_string(), format!(" › {b}")),
                        None => (kc.node_path.clone(), String::new()),
                    };
                    job.append(
                        &proj,
                        0.0,
                        egui::TextFormat { font_id: small.clone(), color: pcolor, ..Default::default() },
                    );
                    if !rest.is_empty() {
                        job.append(
                            &rest,
                            0.0,
                            egui::TextFormat {
                                font_id: small,
                                color: ui.visuals().weak_text_color(),
                                ..Default::default()
                            },
                        );
                    }
                    ui.label(job);
                });
        },
    );
    if src.response.clicked() {
        *jump = Some((kc.node, kc.card));
    }
}

/// Title of the basket that holds template master cards. Created on demand the
/// first time a template is registered, and reused thereafter.
const TEMPLATES_NODE_TITLE: &str = "Templates";

/// Where a template's master card lives, so deleting or re-snapshotting a
/// template can find it again.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct MasterRef {
    pub node: NodeId,
    pub card: CardId,
}

/// A registered template: the card snapshot the app stamps copies from, plus the
/// master card in the **Templates** basket that it was taken from.
///
/// The snapshot is the authority — inserts always stamp *it*, never the master —
/// but the master is what you edit to change a template, via "Update template".
/// `master` is `None` for templates registered before the Templates basket
/// existed (and for any whose master has since been deleted); **Rebuild
/// Templates basket** stamps those back in.
///
/// `card` is flattened so this serializes exactly like the bare `CardExport` the
/// old config held — an existing `card_templates` value loads unchanged, with
/// `master` defaulting to `None`.
#[derive(Clone, Serialize, Deserialize)]
pub struct Template {
    #[serde(flatten)]
    pub card: crate::model::CardExport,
    #[serde(default)]
    pub master: Option<MasterRef>,
}

/// Command-line startup overrides (see `main`). Each field falls back to the
/// saved setting when `None`, so a bare `trellis` behaves exactly as before.
#[derive(Default)]
pub struct Startup {
    /// Document to open instead of the one from the last session.
    pub doc: Option<PathBuf>,
    /// Agent-API port for this run, overriding the saved one.
    pub port: Option<u16>,
    /// This instance's private settings/autosave directory.
    pub data_dir: Option<PathBuf>,
}

/// Colour for a project (top-level node) in task views.
///
/// Uses the node's own colour tag when it has one, so the Agenda matches the dot
/// in the tree. Untagged projects still need to be tellable apart, so they fall
/// back to a fixed palette indexed by node id — stable for the life of the
/// document, and never the same colour twice until the palette wraps.
pub fn project_color(doc: &Document, root: NodeId) -> egui::Color32 {
    const FALLBACK: [egui::Color32; 8] = [
        egui::Color32::from_rgb(0x5c, 0xa8, 0xe6),
        egui::Color32::from_rgb(0xe1, 0x6f, 0x6f),
        egui::Color32::from_rgb(0x54, 0xbd, 0x86),
        egui::Color32::from_rgb(0xe3, 0xac, 0x53),
        egui::Color32::from_rgb(0xa4, 0x8b, 0xe0),
        egui::Color32::from_rgb(0x4c, 0xbf, 0xbf),
        egui::Color32::from_rgb(0xdd, 0x7f, 0xbb),
        egui::Color32::from_rgb(0x9a, 0xa8, 0xb5),
    ];
    match doc.nodes.get(&root).and_then(|n| n.color) {
        Some([r, g, b]) => egui::Color32::from_rgb(r, g, b),
        None => FALLBACK[(root as usize) % FALLBACK.len()],
    }
}

/// The open document's file name, or `untitled` — what identifies an instance to
/// a human (window title) and to an agent (`GET /api/instance`).
pub fn doc_display_name(path: Option<&std::path::Path>) -> String {
    path.and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "untitled".to_string())
}

/// Window/taskbar title for a document. Leads with the file name so two
/// instances (e.g. work and personal) are distinguishable in a window list,
/// where the trailing app name is usually what gets truncated away.
pub fn window_title(path: Option<&std::path::Path>) -> String {
    format!("{} — Trellis", doc_display_name(path))
}

pub struct TrellisApp {
    doc: Document,
    selected: Option<NodeId>,
    /// Per-node canvas view (pan + zoom), so each basket remembers its position.
    views: HashMap<NodeId, TSTransform>,
    md_cache: CommonMarkCache,
    tex_cache: TextureCache,
    renaming: Option<(NodeId, String)>,

    /// Path of the document on disk, if any. `None` = never saved.
    doc_path: Option<PathBuf>,
    /// Fallback autosave location used when the document is untitled.
    autosave_path: PathBuf,
    /// Last title pushed to the window manager, so we only send a viewport
    /// command when the open document actually changes (see `sync_window_title`).
    window_title: String,
    dialog_parent: Option<DialogParent>,
    /// Full-screen image viewer, opened by double-clicking an image card image.
    lightbox: Option<Lightbox>,
    dirty: bool,
    /// Autosave: when on, changes are written to disk shortly after you pause.
    autosave: bool,
    /// When the document last changed, for the autosave idle-debounce.
    last_change: Option<Instant>,
    /// When pointer cards (`Card::source`) were last checked for changes on
    /// disk. Polled rather than watched: a `stat` per pointer every few seconds
    /// costs microseconds, while inotify/FSEvents/ReadDirectoryChangesW is three
    /// platform implementations and a watcher dependency for the same answer.
    last_source_poll: Option<Instant>,
    /// A background save is in flight (guards against overlapping saves).
    saving: bool,
    /// Background-save completion channel: (path, result, revision-at-save-start).
    save_tx: Sender<(PathBuf, Result<(), String>, u64)>,
    save_rx: Receiver<(PathBuf, Result<(), String>, u64)>,
    status: String,

    /// Version-history browse/restore window.
    show_history: bool,
    /// Backup settings (destinations, schedule, encryption); persisted as JSON.
    backup_cfg: crate::backup::BackupConfig,
    /// Version-history retention: how many snapshots to keep, and the minimum
    /// minutes between them. Settings rather than constants because a snapshot
    /// is a full copy of the document — a large one wants fewer, spaced wider.
    history_keep: usize,
    history_gap_mins: u64,
    show_backup: bool,
    /// A backup is running on a worker thread (one at a time).
    backing_up: bool,
    /// When the last scheduled/manual backup started, for the interval timer.
    last_backup: Option<Instant>,
    /// Last backup's human-readable result, shown in the Backup window.
    backup_status: String,
    /// Background-backup completion channel: per-destination outcomes.
    backup_tx: Sender<Vec<crate::backup::DestOutcome>>,
    backup_rx: Receiver<Vec<crate::backup::DestOutcome>>,

    search_open: bool,
    search_query: String,
    /// Quick switcher (Ctrl+O): jump to any node by fuzzy-matching its title/path.
    switcher_open: bool,
    switcher_query: String,
    switcher_index: usize,
    /// A node the tree should scroll into view next frame (set by the switcher).
    scroll_to: Option<NodeId>,
    /// A card the canvas should recenter on next frame (agenda/Kanban row click).
    /// One-shot: cleared right after the canvas consumes it.
    focus_card: Option<CardId>,
    /// The card to flash-highlight on the canvas, and the `ctx` time the flash ends.
    highlight_card: Option<CardId>,
    highlight_until: f64,
    /// Tags panel: browse #tags and the cards that carry them.
    tags_open: bool,
    tag_selected: Option<String>,
    /// Find-cards panel: dropdown query (tag / property / text) across the tree.
    find_open: bool,
    find_tag: Option<String>,
    find_key: Option<String>,
    find_value: String,
    find_text: String,
    /// Agenda panel: open tasks (`due::` dates) grouped by when they're due.
    agenda_open: bool,
    agenda_show_done: bool,
    /// Agenda filter: show only tasks under this project (top-level node).
    /// `None` = every project. Persisted, since it's a working context you'd
    /// rather not reset on every launch.
    agenda_project: Option<NodeId>,
    /// The same filter for the Kanban board, kept separate on purpose: the two
    /// views answer different questions and you may want different scopes.
    kanban_project: Option<NodeId>,
    /// Backlinks panel: cards that `[[link]]` to the selected node.
    backlinks_open: bool,
    /// Kanban board window: cards grouped by `status::`, drag between columns.
    kanban_open: bool,
    /// Kanban: show the `done` column (it piles up; hide it to focus on active work).
    kanban_show_done: bool,
    /// Link-graph window state (force-directed layout, rebuilt when opened).
    graph_open: bool,
    graph_built: bool,
    graph_layout: HashMap<NodeId, egui::Pos2>,
    graph_edges: Vec<(NodeId, NodeId)>,
    show_about: bool,
    theme: Theme,
    /// Whether Ctrl+scroll / Ctrl +/- zoom the canvas (Settings; on by default).
    zoom_enabled: bool,
    /// When on, tree nodes are draggable for reordering (off = click selects).
    reorder_mode: bool,
    /// When on, dragging a card onto another docks (sticks) it there; dragging a
    /// docked card off detaches it. Off = plain moves never change dock bonds.
    dock_mode: bool,
    /// When on, a dragged card's edges snap to nearby cards' edges.
    snap_mode: bool,
    /// When on, a small overview map in the canvas's bottom-right shows the whole
    /// basket and a reticle of the current view (Settings; on by default).
    minimap_enabled: bool,
    /// A copied card, ready to paste into any basket.
    card_clipboard: Option<crate::model::Card>,
    /// Runtime multi-selection of cards in the current basket, used to build a
    /// group. Cleared when the selected node changes. Never persisted.
    card_sel: std::collections::HashSet<crate::model::CardId>,
    /// Which node `card_sel` belongs to, so it resets when the basket changes.
    card_sel_node: Option<NodeId>,
    /// Every drawn card's on-screen rect (points), refreshed each frame by the
    /// canvas. Used to crop a framebuffer screenshot to one card (WYSIWYG export).
    card_rects: HashMap<crate::model::CardId, egui::Rect>,
    /// In-flight "screenshot a single card" export, driven across a few frames.
    card_shot: Option<CardShot>,
    /// A pending multi-shot basket (overview + per-card) visual export.
    basket_shot: Option<BasketShot>,
    /// Saved reusable card templates (persist in app config).
    templates: Vec<Template>,
    /// `bytes://` URIs of text-card inline images already registered with egui
    /// this session (so each is uploaded once, not every frame).
    inline_sent: std::collections::HashSet<String>,
    /// Bumped whenever the document is replaced, and mixed into inline-image
    /// URIs so a new document's images can't collide with the previous one's
    /// cached textures.
    inline_epoch: u64,

    // Agent HTTP API.
    api_rx: Option<Receiver<ApiCommand>>,
    /// Shared with the server thread so key edits take effect without a restart.
    api_shared_key: Arc<Mutex<String>>,
    /// Sender cloned for the server thread; kept so we can restart the server live.
    api_tx: Sender<ApiCommand>,
    /// Live server handle, kept so we can `unblock()` it to rebind on a LAN toggle.
    api_server: Option<Arc<tiny_http::Server>>,
    /// Document-change counter shared with the API's `/api/wait` long-poll.
    doc_revision: Arc<AtomicU64>,
    /// What changed, not merely that something did. Shared with the API server
    /// thread, which serves `/api/changes`. In memory only — see changelog.rs
    /// for why it deliberately isn't part of the document.
    changes: Arc<Mutex<crate::changelog::ChangeLog>>,
    /// Tokens minted for approved plugins, shared with the API thread so it can
    /// authenticate them. Persisted in config, so approval survives a restart
    /// and revoking is real rather than cosmetic.
    grants: Arc<Mutex<Vec<crate::plugins::Grant>>>,
    /// Plugins found on disk this session, and any manifests that wouldn't parse.
    plugins: Vec<crate::plugins::Plugin>,
    plugin_errors: Vec<String>,
    show_plugins: bool,
    /// The `--data-dir` this instance was launched with, so the plugins folder
    /// can be re-derived after startup.
    startup_data_dir: Option<PathBuf>,
    /// How much of the filesystem agents may mirror into a card. The UI's own
    /// file picker is never restricted by this — see `model::MirrorPolicy`.
    mirror_policy: crate::model::MirrorPolicy,
    mirror_dirs: Vec<String>,
    /// Finished runs, newest last, for the Plugins window's log pane.
    plugin_log: Vec<crate::plugins::RunResult>,
    plugin_rx: Receiver<crate::plugins::RunResult>,
    plugin_tx: Sender<crate::plugins::RunResult>,
    /// Plugins currently running, so the UI can say so and not start a second.
    plugin_running: std::collections::HashSet<String>,
    /// Last run time per scheduled plugin, and the change-log sequence each
    /// on-change plugin has already been told about.
    plugin_last_run: std::collections::HashMap<String, Instant>,
    plugin_seen_seq: std::collections::HashMap<String, u64>,
    /// Revision at the last observed change, for the on-change debounce.
    plugin_change_at: Option<(u64, Instant)>,
    /// Editable values for each plugin's declared settings, loaded from its own
    /// config.json and written back on Save.
    plugin_config: std::collections::HashMap<String, std::collections::BTreeMap<String, String>>,
    /// OCR results from background tesseract threads: (node, card, text-or-error).
    ocr_tx: Sender<(NodeId, CardId, Result<String, String>)>,
    ocr_rx: Receiver<(NodeId, CardId, Result<String, String>)>,
    /// Snip (region screenshot) results: (target node, png-bytes-or-error).
    snip_tx: Sender<(NodeId, Result<Vec<u8>, String>)>,
    snip_rx: Receiver<(NodeId, Result<Vec<u8>, String>)>,
    /// Cloned egui context, so background threads (OCR) can wake the UI when done.
    egui_ctx: egui::Context,
    api_key: String,
    api_port: u16,
    /// When true the API binds all interfaces (LAN access), not just localhost.
    api_lan: bool,
    api_status: String,
    show_settings: bool,

    /// The external-tools window. Its probe of PATH is cached in
    /// `req_scan` — hitting the filesystem for every tool on every frame would
    /// be wasteful, and a tool the user just installed only needs to appear when
    /// they ask for it.
    show_requirements: bool,
    /// (label, enables, url, present, builtin, install) per tool, as of the
    /// last scan.
    req_scan: Vec<(String, String, String, bool, bool, crate::deps::Install)>,
    /// Set when an install command was started, to explain the console window
    /// that just appeared and that the list won't update by itself.
    req_note: String,

    /// Per-node undo/redo history. Each entry snapshots one node before a canvas
    /// edit (moves, autosort, add/remove, etc.); a whole drag coalesces into one.
    undo: Vec<(NodeId, crate::model::Node)>,
    redo: Vec<(NodeId, crate::model::Node)>,
    /// Coalesce key for the in-progress gesture, so a drag is one undo step.
    undo_coalesce: Option<&'static str>,
}

impl TrellisApp {
    pub fn new(cc: &eframe::CreationContext<'_>, startup: Startup) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        setup_fonts(&cc.egui_ctx);
        // With `--data-dir` the instance keeps its autosave slot beside its own
        // settings (eframe writes `app.ron` under `<dir>/trellis/`).
        let autosave_path = match &startup.data_dir {
            Some(d) => d.join("trellis").join("autosave.ron"),
            None => default_autosave_path(),
        };

        // Which document to open: an explicit command-line path wins, else the
        // one from the last session, else the autosave slot, else a fresh
        // welcome document.
        let mut status = "Ready".to_string();
        let mut doc_path: Option<PathBuf> = None;
        let mut doc: Option<Document> = None;
        if let Some(p) = &startup.doc {
            if !p.exists() {
                // A path that isn't there yet means "start a document here".
                doc = Some(Document::default());
                doc_path = Some(p.clone());
                status = format!("New document — saves to {}", p.display());
            } else {
                match read_document(p) {
                    Ok(d) => {
                        doc = Some(d);
                        doc_path = Some(p.clone());
                    }
                    // It exists but won't load. Leave `doc_path` unset so an
                    // autosave can never write an empty document over whatever
                    // is really in that file.
                    Err(e) => status = format!("Could not open {}: {e}", p.display()),
                }
            }
        } else if let Some(p) = cc
            .storage
            .and_then(|s| s.get_string(LAST_DOC_KEY))
            .map(PathBuf::from)
        {
            if let Ok(d) = read_document(&p) {
                doc = Some(d);
                doc_path = Some(p);
            }
        }
        let doc = doc
            .or_else(|| read_document(&autosave_path).ok())
            .unwrap_or_default();
        let selected = doc.roots.first().copied();

        cc.egui_ctx.style_mut(|s| {
            s.visuals.window_rounding = 8.0.into();
        });
        // We manage zoom ourselves (so it can be toggled and reset), so turn off
        // egui's built-in keyboard zoom to avoid double-stepping.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let zoom_enabled = cc
            .storage
            .and_then(|s| s.get_string(ZOOM_ENABLED_KEY))
            .map(|s| s != "false")
            .unwrap_or(true);
        // Autosave defaults ON — a notes app should keep your changes safe.
        let autosave = cc
            .storage
            .and_then(|s| s.get_string(AUTOSAVE_KEY))
            .map(|s| s != "false")
            .unwrap_or(true);
        let dock_mode = cc
            .storage
            .and_then(|s| s.get_string(DOCK_MODE_KEY))
            .map(|s| s == "true")
            .unwrap_or(false);
        let snap_mode = cc
            .storage
            .and_then(|s| s.get_string(SNAP_MODE_KEY))
            .map(|s| s == "true")
            .unwrap_or(false);
        let minimap_enabled = cc
            .storage
            .and_then(|s| s.get_string(MINIMAP_KEY))
            .map(|s| s != "false")
            .unwrap_or(true);
        let theme = cc
            .storage
            .and_then(|s| s.get_string(THEME_KEY))
            .map(|s| Theme::from_key(&s))
            .unwrap_or(Theme::Trellis);
        let backup_cfg = cc
            .storage
            .and_then(|s| s.get_string(BACKUP_KEY))
            .map(|s| crate::backup::BackupConfig::parse(&s))
            .unwrap_or_default();
        // Clamped on load as well as in the UI: a hand-edited app.ron shouldn't
        // be able to switch history off or fill the disk.
        let history_keep = cc
            .storage
            .and_then(|s| s.get_string(HISTORY_KEEP_KEY))
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(HISTORY_KEEP)
            .clamp(*HISTORY_KEEP_RANGE.start(), *HISTORY_KEEP_RANGE.end());
        let history_gap_mins = cc
            .storage
            .and_then(|s| s.get_string(HISTORY_GAP_KEY))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(HISTORY_MIN_GAP_SECS / 60)
            .clamp(*HISTORY_GAP_MINS_RANGE.start(), *HISTORY_GAP_MINS_RANGE.end());

        // Agent API: load config, then start the localhost server. It binds
        // regardless of key so toggling the key in Settings works live; requests
        // are rejected while the key is empty.
        let api_key = cc
            .storage
            .and_then(|s| s.get_string(API_KEY_KEY))
            .unwrap_or_default();
        // `--port` wins for this run (and is then persisted like any setting), so
        // a launcher pins an instance's port regardless of what was saved.
        let api_port = startup
            .port
            .or_else(|| {
                cc.storage
                    .and_then(|s| s.get_string(API_PORT_KEY))
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(DEFAULT_API_PORT);
        let api_lan = cc
            .storage
            .and_then(|s| s.get_string(API_LAN_KEY))
            .map(|s| s == "true")
            .unwrap_or(false);
        let api_shared_key = Arc::new(Mutex::new(api_key.clone()));
        let (api_tx, api_rx) = std::sync::mpsc::channel::<ApiCommand>();
        let doc_revision = Arc::new(AtomicU64::new(0));
        let changes = Arc::new(Mutex::new(crate::changelog::ChangeLog::new(
            crate::changelog::DEFAULT_CAP,
            crate::changelog::new_epoch(),
        )));
        // Approvals persist: a plugin the user allowed stays allowed until they
        // revoke it, and a token that survives a restart is what makes revoking
        // meaningful rather than "it stops working when you close the app".
        let stored_grants: Vec<crate::plugins::Grant> = cc
            .storage
            .and_then(|s| s.get_string(GRANTS_KEY))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let grants = Arc::new(Mutex::new(stored_grants));
        let plugins_root = crate::plugins::plugins_dir(startup.data_dir.as_deref());
        let (plugins, plugin_errors) = match &plugins_root {
            Some(d) => {
                // Created on demand so there is somewhere obvious to drop one.
                let _ = std::fs::create_dir_all(d);
                crate::plugins::scan(d)
            }
            None => (Vec::new(), Vec::new()),
        };
        let (plugin_tx, plugin_rx) = std::sync::mpsc::channel();
        let (ocr_tx, ocr_rx) = std::sync::mpsc::channel();
        let (snip_tx, snip_rx) = std::sync::mpsc::channel();
        let (save_tx, save_rx) = std::sync::mpsc::channel();
        let (backup_tx, backup_rx) = std::sync::mpsc::channel();
        let (api_server, api_status) = match api::serve(
            api_port,
            api_lan,
            cc.egui_ctx.clone(),
            api_tx.clone(),
            Arc::clone(&api_shared_key),
            Arc::clone(&doc_revision),
            Arc::clone(&changes),
            Arc::clone(&grants),
        ) {
            Ok(server) => (Some(server), api_status_line(api_lan, api_port)),
            Err(e) => (None, format!("Failed to start on port {api_port}: {e}")),
        };
        // A failed bind (usually a second instance on the same port) leaves this
        // instance without an API — say so in the status bar, not just in
        // Settings, so agent calls aren't silently answered by the other one.
        if api_server.is_none() {
            status = format!("Agent API off — port {api_port} is unavailable");
        }

        Self {
            doc,
            selected,
            views: HashMap::new(),
            md_cache: CommonMarkCache::default(),
            tex_cache: TextureCache::default(),
            renaming: None,
            doc_path,
            autosave_path,
            // Empty so the first frame always pushes the real title.
            window_title: String::new(),
            dialog_parent: None,
            lightbox: None,
            dirty: false,
            autosave,
            last_change: None,
            saving: false,
            save_tx,
            save_rx,
            status,
            backup_cfg,
            history_keep,
            history_gap_mins,
            show_history: false,
            show_backup: false,
            backing_up: false,
            last_backup: None,
            backup_status: String::new(),
            backup_tx,
            backup_rx,
            search_open: false,
            search_query: String::new(),
            switcher_open: false,
            switcher_query: String::new(),
            switcher_index: 0,
            scroll_to: None,
            focus_card: None,
            highlight_card: None,
            highlight_until: 0.0,
            tags_open: false,
            tag_selected: None,
            find_open: false,
            find_tag: None,
            find_key: None,
            find_value: String::new(),
            find_text: String::new(),
            agenda_open: false,
            agenda_show_done: false,
            agenda_project: cc
                .storage
                .and_then(|st| st.get_string(AGENDA_PROJECT_KEY))
                .and_then(|v| v.parse().ok()),
            kanban_project: cc
                .storage
                .and_then(|st| st.get_string(KANBAN_PROJECT_KEY))
                .and_then(|v| v.parse().ok()),
            backlinks_open: false,
            kanban_open: false,
            kanban_show_done: true,
            graph_open: false,
            graph_built: false,
            graph_layout: HashMap::new(),
            graph_edges: Vec::new(),
            show_about: false,
            theme,
            zoom_enabled,
            minimap_enabled,
            reorder_mode: false,
            dock_mode,
            snap_mode,
            card_clipboard: None,
            card_rects: HashMap::new(),
            card_shot: None,
            basket_shot: None,
            inline_sent: std::collections::HashSet::new(),
            inline_epoch: 0,
            templates: cc
                .storage
                .and_then(|s| s.get_string(TEMPLATES_KEY))
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            card_sel: std::collections::HashSet::new(),
            card_sel_node: None,
            api_rx: Some(api_rx),
            api_shared_key,
            api_tx,
            api_server,
            doc_revision,
            changes,
            grants,
            plugins,
            plugin_errors,
            show_plugins: false,
            startup_data_dir: startup.data_dir.clone(),
            mirror_policy: crate::model::MirrorPolicy::from_key(
                &cc.storage.and_then(|s| s.get_string(MIRROR_MODE_KEY)).unwrap_or_default(),
            ),
            mirror_dirs: cc
                .storage
                .and_then(|s| s.get_string(MIRROR_DIRS_KEY))
                .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
                .unwrap_or_default(),
            plugin_log: Vec::new(),
            plugin_rx,
            plugin_tx,
            plugin_running: std::collections::HashSet::new(),
            plugin_last_run: std::collections::HashMap::new(),
            plugin_seen_seq: std::collections::HashMap::new(),
            plugin_change_at: None,
            plugin_config: std::collections::HashMap::new(),
            ocr_tx,
            ocr_rx,
            snip_tx,
            snip_rx,
            egui_ctx: cc.egui_ctx.clone(),
            api_key,
            api_port,
            api_lan,
            api_status,
            show_settings: false,
            last_source_poll: None,
            show_requirements: false,
            req_scan: Vec::new(),
            req_note: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_coalesce: None,
        }
    }

    /// Drop all undo/redo history (on load/new — the old snapshots belong to a
    /// different document).
    fn reset_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.undo_coalesce = None;
    }

    /// Snapshot `node` onto the undo stack before it is mutated (clears redo).
    /// Cheap: clones one node, not the whole document. Capped history.
    fn push_undo(&mut self, node: NodeId) {
        if let Some(n) = self.doc.nodes.get(&node) {
            self.redo.clear();
            self.undo.push((node, n.clone()));
            const MAX_UNDO: usize = 40;
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
    }

    /// Restore the most recent still-present snapshot (Ctrl+Z). Pushes the
    /// current state onto the redo stack so it can be reapplied.
    fn undo(&mut self) {
        while let Some((nid, node)) = self.undo.pop() {
            if let Some(cur) = self.doc.nodes.get(&nid) {
                self.redo.push((nid, cur.clone()));
                self.doc.nodes.insert(nid, node);
                self.selected = Some(nid);
                self.note_node(nid, crate::changelog::Op::Updated, "undo");
                self.undo_coalesce = None;
                self.status = "Undo".to_string();
                return;
            }
        }
        self.status = "Nothing to undo".to_string();
    }

    fn redo(&mut self) {
        while let Some((nid, node)) = self.redo.pop() {
            if let Some(cur) = self.doc.nodes.get(&nid) {
                self.undo.push((nid, cur.clone()));
                self.doc.nodes.insert(nid, node);
                self.selected = Some(nid);
                self.note_node(nid, crate::changelog::Op::Updated, "redo");
                self.undo_coalesce = None;
                self.status = "Redo".to_string();
                return;
            }
        }
        self.status = "Nothing to redo".to_string();
    }

    /// Zoom the selected node's canvas view by `factor` (menu buttons).
    fn zoom_selected(&mut self, factor: f32) {
        if let Some(sel) = self.selected {
            let v = self.views.entry(sel).or_insert(TSTransform::IDENTITY);
            v.scaling = (v.scaling * factor).clamp(canvas::MIN_ZOOM, canvas::MAX_ZOOM);
        }
    }

    /// The zoom percentage of the selected node's canvas (for the status bar).
    fn current_zoom_pct(&self) -> f32 {
        self.selected
            .and_then(|s| self.views.get(&s))
            .map_or(1.0, |v| v.scaling)
            * 100.0
    }

    /// Mark the document changed: flags it dirty for save and bumps the shared
    /// revision so the API's `/api/wait` long-poll wakes live clients.
    fn mark_dirty(&mut self) {
        self.dirty = true;
        self.last_change = Some(Instant::now());
        self.doc_revision.fetch_add(1, Ordering::Relaxed);
    }

    /// Mark the document changed **and say what changed**.
    ///
    /// Prefer this over bare `mark_dirty` at every call site that knows what it
    /// just did. The two are deliberately one call: a change recorded under a
    /// different revision than the one `/api/wait` reports would let a client ask
    /// for changes "since" a revision whose entry it had already been given, or
    /// miss one entirely.
    fn note(&mut self, change: crate::changelog::Change) {
        self.mark_dirty();
        self.stamp_touched(&change);
        let seq = self.doc_revision.load(Ordering::Relaxed);
        if let Ok(mut log) = self.changes.lock() {
            log.push(seq, change);
        }
    }

    /// Record *when* an entity last changed, on the entity itself.
    ///
    /// The change log answers "what changed" only within a session — it lives in
    /// memory. Sorting baskets by recent activity has to survive a restart, so
    /// the time is also written into the document. This is the single place it is
    /// set, so it cannot drift from what the log says.
    ///
    /// **A card's change also stamps its basket.** That is the whole point: "sort
    /// baskets by latest change" means the basket someone last *worked in*, and
    /// work in a basket is editing its cards, not renaming it.
    fn stamp_touched(&mut self, change: &crate::changelog::Change) {
        use crate::changelog::Entity;
        let ts = Some(crate::changelog::now_secs());
        match change.entity {
            Entity::Card => {
                if let Some(node) = change.node {
                    // A deleted card is already gone — only the basket is left to
                    // stamp, which is correct: deleting from it *is* activity.
                    if let Some(c) = self.doc.card_mut(node, change.id) {
                        c.touched = ts;
                    }
                    if let Some(n) = self.doc.nodes.get_mut(&node) {
                        n.touched = ts;
                    }
                }
            }
            Entity::Node => {
                if let Some(n) = self.doc.nodes.get_mut(&change.id) {
                    n.touched = ts;
                }
            }
            // A group lives in a basket and has no identity outside it.
            Entity::Group => {
                if let Some(node) = change.node {
                    if let Some(n) = self.doc.nodes.get_mut(&node) {
                        n.touched = ts;
                    }
                }
            }
            // Whole-document events (a history restore) name no entity.
            Entity::Document => {}
        }
    }

    /// Record a UI-originated change to a card, looking its title up for the log.
    fn note_card(&mut self, node: NodeId, card: CardId, op: crate::changelog::Op, field: &str) {
        use crate::changelog::{Actor, Change, Entity};
        let title = self.doc.card(node, card).map(|c| c.title.clone()).unwrap_or_default();
        let mut c = Change::new(Actor::Ui, Entity::Card, op, card).in_node(node).titled(title);
        if !field.is_empty() {
            c = c.field(field);
        }
        self.note(c);
    }

    /// Record a UI-originated change to a node.
    fn note_node(&mut self, node: NodeId, op: crate::changelog::Op, field: &str) {
        use crate::changelog::{Actor, Change, Entity};
        let title = self.doc.nodes.get(&node).map(|n| n.title.clone()).unwrap_or_default();
        let mut c = Change::new(Actor::Ui, Entity::Node, op, node).titled(title);
        if !field.is_empty() {
            c = c.field(field);
        }
        self.note(c);
    }

    /// Stop and rebind the API server on the current port/LAN setting, so a LAN
    /// toggle takes effect immediately without relaunching the app. The accept
    /// thread only blocks in `incoming_requests()`, so `unblock()` frees the
    /// socket promptly; we retry the bind briefly while it releases.
    fn restart_api(&mut self, ctx: &egui::Context) {
        if let Some(old) = self.api_server.take() {
            old.unblock();
        }
        let mut result = Err("bind timed out".to_string());
        for _ in 0..40 {
            match api::serve(
                self.api_port,
                self.api_lan,
                ctx.clone(),
                self.api_tx.clone(),
                Arc::clone(&self.api_shared_key),
                Arc::clone(&self.doc_revision),
                Arc::clone(&self.changes),
                Arc::clone(&self.grants),
            ) {
                Ok(server) => {
                    result = Ok(server);
                    break;
                }
                Err(e) => {
                    result = Err(e);
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
            }
        }
        match result {
            Ok(server) => {
                self.api_server = Some(server);
                self.api_status = api_status_line(self.api_lan, self.api_port);
            }
            Err(e) => {
                self.api_server = None;
                self.api_status = format!("Failed to restart on port {}: {e}", self.api_port);
            }
        }
    }

    /// Apply finished OCR results from background threads to the document.
    fn pump_ocr(&mut self) {
        let results: Vec<_> = std::iter::from_fn(|| self.ocr_rx.try_recv().ok()).collect();
        for (node, card, res) in results {
            match res {
                Ok(text) => {
                    let words = text.split_whitespace().count();
                    if self.doc.set_card_ocr(node, card, text) {
                        self.note_card(node, card, crate::changelog::Op::Updated, "ocr");
                        self.status = format!("OCR done — {words} words, now searchable");
                    }
                }
                Err(e) => self.status = format!("OCR failed: {e}"),
            }
        }
    }

    /// OCR every image card that has images but no extracted text yet, on one
    /// background worker (sequential, so we don't spawn 100 tesseract processes).
    /// Returns how many cards were queued.
    fn ocr_all(&mut self) -> usize {
        let mut targets: Vec<(NodeId, CardId, Vec<Vec<u8>>)> = Vec::new();
        for (nid, node) in &self.doc.nodes {
            for card in &node.cards {
                if let CardKind::Image { ocr, .. } = &card.kind {
                    if ocr.trim().is_empty() {
                        let imgs: Vec<Vec<u8>> = card.kind.images().iter().map(|(d, _)| d.to_vec()).collect();
                        if !imgs.is_empty() {
                            targets.push((*nid, card.id, imgs));
                        }
                    }
                }
            }
        }
        if targets.is_empty() {
            self.status = "No image cards need OCR".into();
            return 0;
        }
        let count = targets.len();
        self.status = format!("OCR running on {count} image card(s)…");
        let tx = self.ocr_tx.clone();
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            for (nid, cid, imgs) in targets {
                let res = ocr_images(&imgs);
                let _ = tx.send((nid, cid, res));
                ctx.request_repaint();
            }
        });
        count
    }

    /// Capture a screen region into an image card in the selected basket. The
    /// region-select tool runs on a worker thread (it takes over the screen), so
    /// the UI never blocks; the captured PNG comes back through `snip_rx`.
    fn start_snip(&mut self) {
        let Some(node) = self.selected else {
            self.status = "Select a basket first, then Snip".into();
            return;
        };
        self.status = "Select a screen region to capture…".into();
        let tx = self.snip_tx.clone();
        let ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let res = capture_region();
            let _ = tx.send((node, res));
            ctx.request_repaint();
        });
    }

    /// Turn finished snips into image cards in their target basket.
    /// Re-read pointer cards whose file changed on disk.
    ///
    /// Runs on a timer rather than every frame, and `stat`s before reading: the
    /// common case is "nothing changed", which must cost almost nothing.
    ///
    /// Refreshing is **not** recorded as a document change by the user — it is
    /// recorded as one so clients and autosave see it (the cached body really did
    /// change), but the file, not Trellis, is the author.
    fn pump_sources(&mut self, force: bool) {
        const POLL: Duration = Duration::from_secs(3);

        // Ask for a wake-up while any card mirrors a file. egui only calls
        // `update` when something requests a repaint, so on an idle window this
        // poll would otherwise never run — the file changed on disk and the card
        // sat there stale until the user happened to move the mouse. Requested
        // before the timer check, or the first early return silences it forever.
        let any_source =
            self.doc.nodes.values().any(|n| n.cards.iter().any(|c| c.source.is_some()));
        if any_source {
            self.egui_ctx.request_repaint_after(POLL);
        }

        if !force {
            match self.last_source_poll {
                Some(t) if t.elapsed() < POLL => return,
                _ => {}
            }
        }
        self.last_source_poll = Some(Instant::now());
        if !any_source {
            return;
        }

        // Collect first: reading files while holding a borrow on the document
        // would mean re-borrowing it mutably per card.
        let mut stale: Vec<(NodeId, CardId, String)> = Vec::new();
        for (nid, node) in &self.doc.nodes {
            for card in &node.cards {
                let Some(path) = &card.source else { continue };
                let now = crate::model::source_mtime(path);
                // Re-read when the file changed, when it has never been read, or
                // when it is currently in error (so a file that comes back —
                // an unmounted disk, a file recreated — recovers on its own).
                let changed = match (now, card.source_mtime) {
                    (Some(a), Some(b)) => a != b,
                    _ => true,
                };
                if force || changed || card.source_error.is_some() {
                    stale.push((*nid, card.id, path.clone()));
                }
            }
        }

        for (nid, cid, path) in stale {
            let result = crate::model::read_source(&path);
            let Some(card) = self.doc.card_mut(nid, cid) else { continue };
            let before = (card.body.clone(), card.source_error.clone());
            match result {
                Ok((text, mtime)) => {
                    card.body = text;
                    card.source_mtime = Some(mtime);
                    card.source_error = None;
                }
                Err(e) => {
                    // Keep the last good text: a mirror that empties itself
                    // because a disk was unmounted is worse than a stale one.
                    card.source_error = Some(e);
                }
            }
            if before != (self.doc.card(nid, cid).map(|c| c.body.clone()).unwrap_or_default(),
                          self.doc.card(nid, cid).and_then(|c| c.source_error.clone()))
            {
                self.note_card(nid, cid, crate::changelog::Op::Updated, "source");
            }
        }
    }

    fn pump_snip(&mut self) {
        let done: Vec<_> = std::iter::from_fn(|| self.snip_rx.try_recv().ok()).collect();
        for (node, res) in done {
            match res {
                Ok(bytes) if !bytes.is_empty() => {
                    if !self.doc.nodes.contains_key(&node) {
                        continue;
                    }
                    let kind = CardKind::Image {
                        data: Vec::new(),
                        name: String::new(),
                        extra: Vec::new(),
                        ocr: String::new(),
                    };
                    if let Some(cid) = self.doc.add_card(node, egui::pos2(40.0, 40.0), kind) {
                        self.doc.add_image(node, cid, bytes, "snip.png".to_string());
                        self.selected = Some(node);
                        self.note_card(node, cid, crate::changelog::Op::Created, "snip");
                        self.status = "Snip added as an image card".into();
                    }
                }
                Ok(_) => self.status = "Snip cancelled".into(),
                Err(e) => self.status = format!("Snip failed: {e}"),
            }
        }
    }

    /// Show the open document in the window/taskbar title, so several instances
    /// (work / personal) are tellable apart. Only sends a viewport command when
    /// the title actually changes — it's a window-manager round-trip.
    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let want = window_title(self.doc_path.as_deref());
        if want != self.window_title {
            self.window_title = want.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want));
        }
    }

    /// Whether `node` is `root` or a descendant of it.
    ///
    /// Walks parents rather than descending, so the cost is the depth of one
    /// node, not the size of the subtree. The hop limit is a cycle guard: the
    /// tree should never contain one, but an auth check must terminate whatever
    /// the document says.
    fn node_is_within(&self, node: NodeId, root: NodeId) -> bool {
        let mut cur = Some(node);
        for _ in 0..10_000 {
            match cur {
                Some(id) if id == root => return true,
                Some(id) => cur = self.doc.nodes.get(&id).and_then(|n| n.parent),
                None => return false,
            }
        }
        false
    }

    /// The token for a plugin, minting one the first time it is approved.
    ///
    /// The user approves a *scope*; this is the plumbing behind that approval and
    /// is deliberately never shown to them. Re-approving after a manifest change
    /// re-mints, so a plugin cannot widen its own access by editing its manifest
    /// and keeping the token it was granted under the old one.
    fn grant_for(&mut self, p: &crate::plugins::Plugin) -> String {
        let name = p.manifest.name.clone();
        let want = p.manifest.scope.clone();
        let mut g = match self.grants.lock() {
            Ok(g) => g,
            Err(_) => return String::new(),
        };
        if let Some(existing) = g.iter_mut().find(|g| g.plugin == name) {
            if existing.scope == want {
                return existing.token.clone();
            }
            // The manifest asks for something different than was approved.
            existing.scope = want;
            existing.token = crate::plugins::mint_token();
            return existing.token.clone();
        }
        let token = crate::plugins::mint_token();
        g.push(crate::plugins::Grant { plugin: name, token: token.clone(), scope: want });
        token
    }

    fn is_approved(&self, name: &str) -> bool {
        self.grants.lock().map(|g| g.iter().any(|g| g.plugin == name)).unwrap_or(false)
    }

    fn revoke(&mut self, name: &str) {
        if let Ok(mut g) = self.grants.lock() {
            g.retain(|g| g.plugin != name);
        }
    }

    /// Launch a plugin on a worker thread.
    ///
    /// Never on the UI thread: a plugin can take minutes, and blocking here
    /// would freeze the window *and* stall autosave. The result comes back
    /// through `plugin_rx`.
    fn run_plugin(&mut self, idx: usize, ctx: Vec<(String, String)>) {
        let Some(p) = self.plugins.get(idx).cloned() else { return };
        if !self.is_approved(&p.manifest.name) {
            self.status = format!("{} isn't approved yet — Tools → Plugins", p.manifest.title);
            return;
        }
        if !self.plugin_running.insert(p.manifest.name.clone()) {
            self.status = format!("{} is already running", p.manifest.title);
            return;
        }
        let token = self.grant_for(&p);
        // Always loopback: a plugin runs on this machine, so the LAN setting is
        // irrelevant to it and 127.0.0.1 works whether or not LAN is enabled.
        let base = format!("http://127.0.0.1:{}/api", self.api_port);
        self.status = format!("Running {}…", p.manifest.title);
        let tx = self.plugin_tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let r = crate::plugins::run(&p, &token, &base, &ctx);
            let _ = tx.send(r);
            egui_ctx.request_repaint();
        });
    }

    /// Collect finished plugin runs.
    /// Fire the time- and change-driven triggers.
    ///
    /// Both are polled from `update()` rather than driven by their own threads,
    /// so a plugin can never be launched while the document is mid-edit — the
    /// app loop is the only place that knows the document is at rest.
    ///
    /// Nothing fires while Trellis is closed. That is a real limitation and it
    /// is stated in the Plugins window rather than hidden: a schedule people
    /// think is reliable, but silently isn't, is worse than no schedule.
    fn pump_plugin_triggers(&mut self) {
        use crate::plugins::Trigger;
        let now = Instant::now();
        // An idle window stops calling update(), which would silently stop the
        // clock for every timed trigger — the same trap the file-mirror poll hit.
        if self.plugins.iter().any(|p| {
            p.manifest.triggers.contains(&Trigger::Schedule)
                && self.is_approved(&p.manifest.name)
        }) {
            self.egui_ctx.request_repaint_after(Duration::from_secs(20));
        }

        // --- schedule ---
        let due: Vec<usize> = self
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.manifest.triggers.contains(&Trigger::Schedule)
                    && self.is_approved(&p.manifest.name)
                    && !self.plugin_running.contains(&p.manifest.name)
            })
            .filter(|(_, p)| {
                let every = Duration::from_secs(p.manifest.interval_mins.max(1) * 60);
                match self.plugin_last_run.get(&p.manifest.name) {
                    Some(t) => now.duration_since(*t) >= every,
                    // Not on launch: opening the app shouldn't kick off every
                    // scheduled plugin at once.
                    None => false,
                }
            })
            .map(|(i, _)| i)
            .collect();
        for i in due {
            let name = self.plugins[i].manifest.name.clone();
            self.plugin_last_run.insert(name, now);
            self.run_plugin(i, vec![("TRELLIS_TRIGGER".into(), "schedule".into())]);
        }
        // Seed the clock so the first interval is measured from launch.
        for p in &self.plugins {
            if p.manifest.triggers.contains(&Trigger::Schedule) {
                self.plugin_last_run.entry(p.manifest.name.clone()).or_insert(now);
            }
        }

        // --- on change ---
        let watchers: Vec<usize> = self
            .plugins
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.manifest.triggers.contains(&Trigger::OnChange)
                    && self.is_approved(&p.manifest.name)
                    && !self.plugin_running.contains(&p.manifest.name)
            })
            .map(|(i, _)| i)
            .collect();
        if watchers.is_empty() {
            return;
        }
        let rev = self.doc_revision.load(Ordering::Relaxed);
        match self.plugin_change_at {
            Some((seen, _)) if seen == rev => {}
            // The revision moved: restart the quiet period rather than firing,
            // so a burst of typing is one run at the end and not one per frame.
            _ => self.plugin_change_at = Some((rev, now)),
        }
        let Some((at_rev, since)) = self.plugin_change_at else { return };
        for i in watchers {
            let p = &self.plugins[i];
            let quiet = Duration::from_secs(p.manifest.debounce_secs.max(1));
            if now.duration_since(since) < quiet {
                // Make sure we wake to fire it; an idle window otherwise wouldn't.
                self.egui_ctx.request_repaint_after(quiet);
                continue;
            }
            let name = p.manifest.name.clone();
            let seen = self.plugin_seen_seq.get(&name).copied().unwrap_or(0);
            if seen >= at_rev {
                continue;
            }
            self.plugin_seen_seq.insert(name, at_rev);
            // The plugin is told where to resume from, so it reads exactly the
            // changes it hasn't seen out of /api/changes rather than guessing.
            self.run_plugin(
                i,
                vec![
                    ("TRELLIS_TRIGGER".into(), "change".into()),
                    ("TRELLIS_SINCE".into(), seen.to_string()),
                    ("TRELLIS_REV".into(), at_rev.to_string()),
                ],
            );
        }
    }

    fn pump_plugins(&mut self) {
        let done: Vec<_> = std::iter::from_fn(|| self.plugin_rx.try_recv().ok()).collect();
        for r in done {
            self.plugin_running.remove(&r.plugin);
            self.status = if r.ok { r.summary.clone() } else { format!("Plugin failed: {}", r.summary) };
            self.plugin_log.push(r);
            // Keep the pane bounded; a chatty plugin shouldn't grow forever.
            if self.plugin_log.len() > 50 {
                self.plugin_log.remove(0);
            }
        }
    }

    /// Plugins offering a given trigger, as (index, title).
    fn plugins_for(&self, t: crate::plugins::Trigger) -> Vec<(usize, String)> {
        self.plugins
            .iter()
            .enumerate()
            .filter(|(_, p)| p.manifest.triggers.contains(&t) && self.is_approved(&p.manifest.name))
            .map(|(i, p)| (i, p.manifest.title.clone()))
            .collect()
    }

    fn pump_api(&mut self) {
        let mut cmds = Vec::new();
        if let Some(rx) = &self.api_rx {
            while let Ok(cmd) = rx.try_recv() {
                cmds.push(cmd);
            }
        }
        for cmd in cmds {
            // Backup endpoints need app state (config + doc file), so answer them
            // here instead of in api::process (which only sees the Document).
            if let Some(resp) = self.handle_api_backup(&cmd.req) {
                let _ = cmd.resp.send(resp);
                continue;
            }
            if let Some(resp) = self.handle_api_templates(&cmd.req) {
                let _ = cmd.resp.send(resp);
                continue;
            }
            if let Some(resp) = self.handle_api_instance(&cmd.req) {
                let _ = cmd.resp.send(resp);
                continue;
            }
            // The other half of a plugin's scope. Confining a token to a subtree
            // needs the tree to resolve ancestry, which only exists here — the
            // API thread has no document. A request that names no node is
            // **refused** rather than waved through: a scope that quietly stops
            // applying at the edges is not a scope. The few instance-level reads
            // a plugin needs to orient itself are listed explicitly.
            if let Some(scope) = &cmd.scope {
                if let Some(root) = scope.subtree {
                    let allowed = match api::target_node(&cmd.req) {
                        Some(n) => self.node_is_within(n, root),
                        None => api::is_scope_neutral(&cmd.req),
                    };
                    if !allowed {
                        let _ = cmd.resp.send(api::ApiResponse::err(
                            403,
                            "outside the basket this plugin was given access to",
                        ));
                        continue;
                    }
                }
            }
            // `fit` in the request is applied by `process` from an estimate.
            // Note the target before the request is consumed, then re-measure
            // below with the real fonts — we're on the UI thread here.
            let fit_target = api::fit_request(&cmd.req);
            // Same reason as `fit_request`: the request is consumed below, and
            // reading the document *now* catches the pre-change state — a
            // deleted card's title can't be looked up after it's gone.
            // `source` is the one field an API request can use to reach outside
            // the document, so it is checked here rather than in `process`,
            // which has no access to the setting.
            if let Some(path) = api::source_request(&cmd.req) {
                if let Err(e) =
                    crate::model::mirror_allowed(&path, self.mirror_policy, &self.mirror_dirs)
                {
                    let _ = cmd.resp.send(api::ApiResponse::err(403, &e));
                    continue;
                }
            }
            let change = api::change_of(&cmd.req, &self.doc);
            let (changed, resp) = api::process(&mut self.doc, cmd.req);
            if let Some((node, card)) = fit_target {
                // On create the id only exists now, in the response.
                let card = card.or_else(|| {
                    serde_json::from_str::<serde_json::Value>(&resp.body)
                        .ok()
                        .and_then(|v| v["id"].as_u64())
                });
                if let Some(card) = card {
                    self.refit_card_precise(node, card);
                }
            }
            if changed {
                match change {
                    Some(mut c) => {
                        // A created entity had no id when the request was
                        // described; the response is where it first exists.
                        if c.id == 0 {
                            if let Some(id) = serde_json::from_str::<serde_json::Value>(&resp.body)
                                .ok()
                                .and_then(|v| v["id"].as_u64())
                            {
                                c.id = id;
                            }
                        }
                        self.note(c);
                    }
                    // `process` reports a change we couldn't describe. Record it
                    // as an undescribed document change rather than dropping it:
                    // a client that misses an edit entirely is far worse off than
                    // one told to re-read.
                    None => self.note(crate::changelog::Change::new(
                        crate::changelog::Actor::Api,
                        crate::changelog::Entity::Document,
                        crate::changelog::Op::Updated,
                        0,
                    )),
                }
                // A deleted node may have been the selection.
                if let Some(sel) = self.selected {
                    if !self.doc.nodes.contains_key(&sel) {
                        self.selected = self.doc.roots.first().copied();
                    }
                }
            }
            let _ = cmd.resp.send(resp);
        }
    }

    /// The root-level **Templates** basket, created if it isn't there yet. This
    /// is what makes a saved template something you can see and edit: every
    /// registered template keeps its master card here.
    ///
    /// Matches an existing root node by title (case-insensitively) so a Templates
    /// basket someone made by hand is adopted rather than duplicated.
    fn templates_node(&mut self) -> NodeId {
        let existing = self.doc.roots.iter().copied().find(|id| {
            self.doc
                .nodes
                .get(id)
                .is_some_and(|n| n.title.trim().eq_ignore_ascii_case(TEMPLATES_NODE_TITLE))
        });
        if let Some(id) = existing {
            return id;
        }
        let id = self.doc.add_node(None, TEMPLATES_NODE_TITLE.to_string());
        self.note_node(id, crate::changelog::Op::Created, "");
        id
    }

    /// Is this master reference still pointing at a card that exists?
    fn master_alive(&self, m: Option<MasterRef>) -> Option<MasterRef> {
        let m = m?;
        self.doc.card(m.node, m.card).is_some().then_some(m)
    }

    /// Stamp a template's master card into the Templates basket, laid out in a
    /// tidy grid so a library of them stays readable.
    fn stamp_master(&mut self, exp: &crate::model::CardExport) -> Option<MasterRef> {
        let node = self.templates_node();
        let n = self.doc.nodes.get(&node).map(|n| n.cards.len()).unwrap_or(0);
        let pos = egui::pos2(40.0 + (n % 4) as f32 * 340.0, 40.0 + (n / 4) as f32 * 260.0);
        let cid = self.doc.add_card_from_export(node, pos, exp.clone())?;
        self.note_card(node, cid, crate::changelog::Op::Created, "template.master");
        Some(MasterRef { node, card: cid })
    }

    /// Register a card as template `title`, stamping its master into the
    /// Templates basket. Shared by the UI action and the API so both behave the
    /// same. Returns `(index, name)`.
    ///
    /// If the source card already lives in the Templates basket it becomes the
    /// master as-is — registering from a master must not clone it.
    fn register_template(
        &mut self,
        node: NodeId,
        card: CardId,
        title: Option<&str>,
    ) -> Option<(usize, String)> {
        let json = self.doc.export_card_json(node, card)?;
        let mut exp = crate::model::parse_card_export(&json)?;
        if let Some(t) = title {
            if !t.trim().is_empty() {
                exp.title = t.to_string();
            }
        }
        let name = if exp.title.trim().is_empty() {
            exp.kind.label().to_string()
        } else {
            exp.title.clone()
        };
        let tnode = self.templates_node();
        let master = if node == tnode {
            Some(MasterRef { node, card })
        } else {
            self.stamp_master(&exp)
        };
        let index = self.templates.len();
        self.templates.push(Template { card: exp, master });
        Some((index, name))
    }

    /// Re-snapshot template `index` from a card, keeping the slot's index and
    /// (unless renamed) its name, then bring the master card in the Templates
    /// basket back in line so the basket always shows what inserts will stamp.
    fn update_template(
        &mut self,
        index: usize,
        node: NodeId,
        card: CardId,
        title: Option<&str>,
    ) -> Option<String> {
        if index >= self.templates.len() {
            return None;
        }
        let json = self.doc.export_card_json(node, card)?;
        let mut exp = crate::model::parse_card_export(&json)?;
        exp.title = match title {
            Some(t) if !t.trim().is_empty() => t.to_string(),
            _ => self.templates[index].card.title.clone(),
        };
        let name = exp.title.clone();
        let old = self.master_alive(self.templates[index].master);
        self.templates[index].card = exp.clone();

        // Updating *from* the master means it's already current — leave it be
        // (re-stamping would only churn its id). Otherwise replace it in place,
        // keeping its slot on the canvas.
        if old.is_some_and(|m| m.node == node && m.card == card) {
            return Some(name);
        }
        let keep_pos = old.and_then(|m| self.doc.card(m.node, m.card).map(|c| c.pos));
        if let Some(m) = old {
            self.doc.remove_card(m.node, m.card);
        }
        let master = match keep_pos {
            Some(p) => {
                let tnode = old.map(|m| m.node).unwrap_or_else(|| self.templates_node());
                self.doc
                    .add_card_from_export(tnode, p, exp)
                    .map(|cid| MasterRef { node: tnode, card: cid })
            }
            None => self.stamp_master(&exp),
        };
        self.templates[index].master = master;
        match master {
            Some(m) => self.note_card(m.node, m.card, crate::changelog::Op::Updated, "template.master"),
            None => self.mark_dirty(),
        }
        Some(name)
    }

    /// Remove template `index` and its master card. Returns the name.
    fn delete_template(&mut self, index: usize) -> Option<String> {
        if index >= self.templates.len() {
            return None;
        }
        let t = self.templates.remove(index);
        if let Some(m) = self.master_alive(t.master) {
            self.doc.remove_card(m.node, m.card);
            self.note_card(m.node, m.card, crate::changelog::Op::Deleted, "template.master");
        }
        Some(t.card.title)
    }

    /// Stamp a master card for every template that hasn't got a live one, so a
    /// library registered before the Templates basket existed becomes visible
    /// and editable. Returns `(templates node, stamped, already present)`.
    fn rebuild_templates_node(&mut self) -> (NodeId, usize, usize) {
        let node = self.templates_node();
        let (mut made, mut had) = (0, 0);
        for i in 0..self.templates.len() {
            if self.master_alive(self.templates[i].master).is_some() {
                had += 1;
                continue;
            }
            let exp = self.templates[i].card.clone();
            self.templates[i].master = self.stamp_master(&exp);
            if self.templates[i].master.is_some() {
                made += 1;
            }
        }
        (node, made, had)
    }

    /// Answer `GET /api/instance` — which document this instance has open, and
    /// on which port. With several instances running (one per document), an
    /// agent uses this to confirm it is driving the one it means to before it
    /// writes anything. Needs the doc path + server settings, so it's answered
    /// here rather than in `api::process`.
    fn handle_api_instance(&mut self, req: &api::ApiRequest) -> Option<api::ApiResponse> {
        match req {
            api::ApiRequest::Instance => Some(api::ApiResponse::ok(serde_json::json!({
                "app": "trellis",
                "version": env!("CARGO_PKG_VERSION"),
                "document": doc_display_name(self.doc_path.as_deref()),
                "path": self.doc_path.as_ref().map(|p| p.display().to_string()),
                "port": self.api_port,
                "lan": self.api_lan,
                "nodes": self.doc.nodes.len(),
                "unsaved_changes": self.dirty,
            }))),
            _ => None,
        }
    }

    /// Answer the backup API endpoints from app state. Returns `None` for any
    /// other request so the normal `api::process` path handles it.
    fn handle_api_backup(&mut self, req: &api::ApiRequest) -> Option<api::ApiResponse> {
        match req {
            api::ApiRequest::BackupStatus => {
                let dests: Vec<_> = self
                    .backup_cfg
                    .destinations
                    .iter()
                    .map(|d| {
                        serde_json::json!({
                            "kind": d.kind.label(), "name": d.name, "target": d.target, "enabled": d.enabled
                        })
                    })
                    .collect();
                Some(api::ApiResponse::ok(serde_json::json!({
                    "enabled": self.backup_cfg.enabled,
                    "interval_mins": self.backup_cfg.interval_mins,
                    "encrypt": self.backup_cfg.encrypt,
                    "running": self.backing_up,
                    "last_backup_secs_ago": self.last_backup.map(|t| t.elapsed().as_secs()),
                    "last_result": self.backup_status,
                    "destinations": dests,
                })))
            }
            api::ApiRequest::BackupRun => {
                if self.backup_cfg.destinations.iter().all(|d| !d.enabled) {
                    return Some(api::ApiResponse::err(400, "no enabled backup destinations configured"));
                }
                if self.backing_up {
                    return Some(api::ApiResponse::err(409, "a backup is already running"));
                }
                self.start_backup(true);
                Some(api::ApiResponse::ok(serde_json::json!({ "started": true })))
            }
            api::ApiRequest::HistoryList => {
                let snaps: Vec<_> = history_snapshots(&self.target_path())
                    .into_iter()
                    .map(|(p, name)| {
                        serde_json::json!({
                            "file": name,
                            "when": format_stamp(&name),
                            "bytes": p.metadata().map(|m| m.len()).unwrap_or(0),
                        })
                    })
                    .collect();
                // Retention is reported alongside the list, the same way
                // `GET /api/backup` reports its schedule: settings are written in
                // the app, but an agent should be able to see what governs the
                // snapshots it's looking at (and why an expected one is gone).
                Some(api::ApiResponse::ok(serde_json::json!({
                    "count": snaps.len(),
                    "keep": self.history_keep,
                    "min_gap_mins": self.history_gap_mins,
                    "snapshots": snaps,
                })))
            }
            api::ApiRequest::HistoryRestore(file) => {
                // Guard against path traversal: accept a bare snapshot filename only.
                if file.contains('/') || file.contains("..") {
                    return Some(api::ApiResponse::err(400, "file must be a snapshot filename"));
                }
                let Some(dir) = history_dir(&self.target_path()) else {
                    return Some(api::ApiResponse::err(404, "no history for this document"));
                };
                let path = dir.join(file);
                if !path.is_file() {
                    return Some(api::ApiResponse::err(404, "snapshot not found"));
                }
                match read_document(&path) {
                    Ok(doc) => {
                        self.reset_inline_images();
                        self.doc = doc;
                        self.selected = self.doc.roots.first().copied();
                        self.views.clear();
                        self.reset_history();
                        // Everything a client knew is now potentially wrong, and
                        // there is no per-entity way to say so.
                        self.note(
                            crate::changelog::Change::new(
                                crate::changelog::Actor::Api,
                                crate::changelog::Entity::Document,
                                crate::changelog::Op::Updated,
                                0,
                            )
                            .field("history.restore"),
                        );
                        Some(api::ApiResponse::ok(serde_json::json!({ "restored": true })))
                    }
                    Err(e) => Some(api::ApiResponse::err(500, &format!("restore failed: {e}"))),
                }
            }
            api::ApiRequest::OcrAll => {
                let n = self.ocr_all();
                Some(api::ApiResponse::ok(serde_json::json!({ "started": n > 0, "cards": n })))
            }
            _ => None,
        }
    }

    /// Answer the card-template API endpoints from app state. Templates are the
    /// same reusable card snapshots as the UI's Save as template / Insert template
    /// (persisted in app config, not the Document), so they can't live in
    /// `api::process`. Returns `None` for any other request.
    fn handle_api_templates(&mut self, req: &api::ApiRequest) -> Option<api::ApiResponse> {
        match req {
            api::ApiRequest::TemplateList => {
                let list: Vec<_> = self
                    .templates
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let m = self.master_alive(t.master);
                        serde_json::json!({
                            "index": i,
                            "title": t.card.title,
                            "kind": t.card.kind.label(),
                            "master_node": m.map(|m| m.node),
                            "master_card": m.map(|m| m.card),
                        })
                    })
                    .collect();
                Some(api::ApiResponse::ok(
                    serde_json::json!({ "count": self.templates.len(), "templates": list }),
                ))
            }
            // Snapshot an existing card into a reusable template (mirrors the
            // right-click "Save as template"). Build the card however you like
            // first (e.g. a table with headers + cell colors), then register it.
            api::ApiRequest::TemplateRegister { node, card, title } => {
                if self.doc.card(*node, *card).is_none() {
                    return Some(api::ApiResponse::err(404, "node or card not found"));
                }
                match self.register_template(*node, *card, title.as_deref()) {
                    Some((index, name)) => {
                        let m = self.templates[index].master;
                        Some(api::ApiResponse::ok(serde_json::json!({
                            "index": index,
                            "title": name,
                            "master_node": m.map(|m| m.node),
                            "master_card": m.map(|m| m.card),
                        })))
                    }
                    None => Some(api::ApiResponse::err(500, "could not build a template from that card")),
                }
            }
            // Give every template a master card in the Templates basket. For a
            // library registered before that basket existed, this is what makes
            // it visible and editable.
            api::ApiRequest::TemplateRebuild => {
                let (node, made, had) = self.rebuild_templates_node();
                Some(api::ApiResponse::ok(serde_json::json!({
                    "node": node,
                    "stamped": made,
                    "already_present": had,
                    "templates": self.templates.len(),
                })))
            }
            // Stamp a saved template into a basket as a new card (mirrors "Insert
            // template"). Returns the created card.
            api::ApiRequest::TemplateInsert { index, node, pos } => {
                let Some(exp) = self.templates.get(*index).map(|t| t.card.clone()) else {
                    return Some(api::ApiResponse::err(404, "no template at that index"));
                };
                if !self.doc.nodes.contains_key(node) {
                    return Some(api::ApiResponse::err(404, "node not found"));
                }
                let p = pos
                    .map(|[x, y]| egui::pos2(x, y))
                    .unwrap_or_else(|| egui::pos2(40.0, 40.0));
                match self.doc.add_card_from_export(*node, p, exp) {
                    Some(cid) => {
                        let title =
                            self.doc.card(*node, cid).map(|c| c.title.clone()).unwrap_or_default();
                        self.note(
                            crate::changelog::Change::new(
                                crate::changelog::Actor::Api,
                                crate::changelog::Entity::Card,
                                crate::changelog::Op::Created,
                                cid,
                            )
                            .in_node(*node)
                            .titled(title)
                            .field("template.insert"),
                        );
                        let card = self.doc.card(*node, cid).map(api::card_json);
                        Some(api::ApiResponse::ok(
                            serde_json::json!({ "node": node, "card": card }),
                        ))
                    }
                    None => Some(api::ApiResponse::err(500, "could not insert template")),
                }
            }
            // Re-snapshot an existing template slot from a card, keeping its index
            // (and its title unless a new one is given). This is what makes a
            // Templates-folder master editable: tweak the card, then update.
            api::ApiRequest::TemplateUpdate { index, node, card, title } => {
                if *index >= self.templates.len() {
                    return Some(api::ApiResponse::err(404, "no template at that index"));
                }
                if self.doc.card(*node, *card).is_none() {
                    return Some(api::ApiResponse::err(404, "node or card not found"));
                }
                match self.update_template(*index, *node, *card, title.as_deref()) {
                    Some(name) => {
                        let m = self.templates[*index].master;
                        Some(api::ApiResponse::ok(serde_json::json!({
                            "updated": index,
                            "title": name,
                            "master_node": m.map(|m| m.node),
                            "master_card": m.map(|m| m.card),
                        })))
                    }
                    None => Some(api::ApiResponse::err(500, "could not build a template from that card")),
                }
            }
            api::ApiRequest::TemplateDelete(index) => {
                match self.delete_template(*index) {
                    Some(title) => Some(api::ApiResponse::ok(
                        serde_json::json!({ "deleted": index, "title": title }),
                    )),
                    None => Some(api::ApiResponse::err(404, "no template at that index")),
                }
            }
            _ => None,
        }
    }

    /// A file dialog parented to the app window (falls back to unparented).
    fn file_dialog(&self) -> rfd::FileDialog {
        let d = rfd::FileDialog::new();
        match &self.dialog_parent {
            Some(p) => d.set_parent(p),
            None => d,
        }
    }

    /// A message dialog parented to the app window (falls back to unparented).
    fn message_dialog(&self) -> rfd::MessageDialog {
        let d = rfd::MessageDialog::new();
        match &self.dialog_parent {
            Some(p) => d.set_parent(p),
            None => d,
        }
    }

    // --- persistence --------------------------------------------------------

    fn target_path(&self) -> PathBuf {
        self.doc_path.clone().unwrap_or_else(|| self.autosave_path.clone())
    }

    /// Re-size a card with the real font metrics, the way the right-click
    /// "Fit to content" does.
    ///
    /// The API's `fit` is applied inside `api::process`, which can only estimate
    /// (no font context). For Text cards that estimate runs tall, so an
    /// API-created card carried a strip of blank card under its text that
    /// vanished the moment you used the menu action. Same measurement for both
    /// paths now.
    fn refit_card_precise(&mut self, node: NodeId, card: CardId) {
        let Some(c) = self.doc.card(node, card) else { return };
        let Some(size) = fit_card_size(&self.egui_ctx, c) else { return };
        if let Some(c) = self.doc.card_mut(node, card) {
            c.size = size;
        }
    }

    /// Take a version-history snapshot using this instance's retention
    /// settings. Thin wrapper so both save paths honour the same values.
    fn write_history_snapshot(&self, path: &std::path::Path) {
        write_history_snapshot(path, self.history_keep, self.history_gap_mins * 60);
    }

    /// Synchronous save — only for `on_exit`, where a background thread would be
    /// killed before it finished. Interactive/auto saves use `spawn_save` so the
    /// serialize + gzip + write never blocks the UI thread (they can take seconds
    /// on a large document).
    ///
    /// `snapshot = false` skips the version-history copy, and that is what exit
    /// passes. A snapshot costs a full read + write of the document *on top of*
    /// the save that just happened, and on a large document that is the whole
    /// reason closing the window appears to hang. It loses almost nothing: the
    /// document has just been written, and history exists to go *back*, so the
    /// state before this save is already in the previous snapshot.
    fn write_to(&mut self, path: PathBuf, snapshot: bool) {
        match serialize_and_write(&self.doc, &path) {
            Ok(_) => {
                if snapshot {
                    self.write_history_snapshot(&path);
                }
                self.dirty = false;
                self.last_change = None;
                self.status = format!("Saved → {}", path.display());
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    /// Save off the UI thread: clone the document (cheap — raw image bytes), then
    /// serialize + gzip + write it on a worker. The result is applied in
    /// `pump_save`. `dirty` clears only if nothing changed while we were saving.
    fn spawn_save(&mut self, path: PathBuf) {
        if self.saving {
            return; // one save at a time; a later change re-triggers autosave
        }
        self.saving = true;
        let snapshot = self.doc_revision.load(Ordering::Relaxed);
        let doc = self.doc.clone();
        let tx = self.save_tx.clone();
        let ctx = self.egui_ctx.clone();
        // Copied out before the move: the worker can't borrow `self`.
        let (keep, gap_secs) = (self.history_keep, self.history_gap_mins * 60);
        std::thread::spawn(move || {
            let res = serialize_and_write(&doc, &path);
            if res.is_ok() {
                write_history_snapshot(&path, keep, gap_secs);
            }
            let _ = tx.send((path, res, snapshot));
            ctx.request_repaint();
        });
    }

    /// Apply finished background saves.
    fn pump_save(&mut self) {
        let done: Vec<_> = std::iter::from_fn(|| self.save_rx.try_recv().ok()).collect();
        for (path, res, snapshot) in done {
            self.saving = false;
            match res {
                Ok(_) => {
                    // Clear dirty only if the document didn't change mid-save.
                    if self.doc_revision.load(Ordering::Relaxed) == snapshot {
                        self.dirty = false;
                        self.last_change = None;
                    }
                    self.status = format!("Saved → {}", path.display());
                }
                Err(e) => self.status = format!("Save failed: {e}"),
            }
        }
    }

    fn save(&mut self) {
        let path = self.target_path();
        self.spawn_save(path);
    }

    /// Fire scheduled backups and drain finished ones. Called each frame.
    fn pump_backup(&mut self) {
        // Apply finished backups (worker sends per-destination outcomes).
        let done: Vec<_> = std::iter::from_fn(|| self.backup_rx.try_recv().ok()).collect();
        for outcomes in done {
            self.backing_up = false;
            let failed: Vec<&crate::backup::DestOutcome> = outcomes.iter().filter(|o| !o.ok).collect();
            self.backup_status = if failed.is_empty() {
                format!("Backed up to {} destination(s) OK", outcomes.len())
            } else {
                let first = failed[0];
                format!("Backup: {}/{} failed — {}: {}", failed.len(), outcomes.len(), first.dest, first.detail)
            };
            self.status = self.backup_status.clone();
        }

        // Scheduled trigger: enabled, an interval set, and enough time elapsed.
        if self.backup_cfg.enabled && self.backup_cfg.interval_mins > 0 && !self.backing_up {
            let due = self
                .last_backup
                .map(|t| t.elapsed().as_secs() >= self.backup_cfg.interval_mins * 60)
                .unwrap_or(true);
            if due {
                self.start_backup(false);
            }
        }
    }

    /// Serialize the document and hand it to a worker thread that encrypts (if
    /// configured) and delivers it to every enabled destination. Off the UI
    /// thread — a large document plus a slow network target must not freeze the
    /// canvas. `manual` only affects the status wording.
    fn start_backup(&mut self, manual: bool) {
        if self.backing_up {
            self.status = "A backup is already running".to_string();
            return;
        }
        // Mark the attempt now so the interval timer doesn't re-fire while a slow
        // or failing backup is in flight.
        self.last_backup = Some(Instant::now());
        let bytes = match serialize_doc(&self.doc) {
            Ok(b) => b,
            Err(e) => {
                self.backup_status = format!("Backup failed: could not serialize document: {e}");
                self.status = self.backup_status.clone();
                return;
            }
        };
        self.backing_up = true;
        self.status = if manual { "Backing up now…".into() } else { "Running scheduled backup…".into() };
        let cfg = self.backup_cfg.clone();
        let tx = self.backup_tx.clone();
        let ctx = self.egui_ctx.clone();
        let stamp = crate::backup::stamp(std::time::SystemTime::now());
        std::thread::spawn(move || {
            let outcomes = crate::backup::run(&bytes, &stamp, &cfg);
            let _ = tx.send(outcomes);
            ctx.request_repaint();
        });
    }

    fn save_as(&mut self) {
        if let Some(path) = self.file_dialog()
            .add_filter("Trellis document", &["ron"])
            .set_file_name("untitled.ron")
            .save_file()
        {
            self.doc_path = Some(path.clone());
            self.spawn_save(path);
        }
    }

    fn confirm_discard(&self) -> bool {
        if !self.dirty {
            return true;
        }
        matches!(
            self.message_dialog()
                .set_title("Unsaved changes")
                .set_description("Discard the current document?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show(),
            rfd::MessageDialogResult::Yes
        )
    }

    /// Reset per-session inline-image registration when the document changes, so
    /// a freshly loaded document's images can't reuse the previous document's
    /// cached textures.
    fn reset_inline_images(&mut self) {
        self.inline_sent.clear();
        self.inline_epoch = self.inline_epoch.wrapping_add(1);
    }

    fn new_document(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        self.reset_inline_images();
        self.doc = Document::default();
        self.selected = self.doc.roots.first().copied();
        self.views.clear();
        self.reset_history();
        self.doc_path = None;
        self.dirty = false;
        self.status = "New document".to_string();
    }

    fn open_document(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(path) = self.file_dialog()
            .add_filter("Trellis document", &["ron"])
            .pick_file()
        {
            match read_document(&path) {
                Ok(doc) => {
                    self.reset_inline_images();
                    self.doc = doc;
                    self.selected = self.doc.roots.first().copied();
                    self.views.clear();
                    self.reset_history();
                    self.doc_path = Some(path.clone());
                    self.dirty = false;
                    self.status = format!("Opened {}", path.display());
                }
                Err(e) => self.status = format!("Open failed: {e}"),
            }
        }
    }

    fn import(&mut self, html: bool) {
        let (label, exts): (&str, &[&str]) = if html {
            ("HTML", &["html", "htm"])
        } else {
            ("Markdown", &["md", "markdown", "txt"])
        };
        if let Some(path) = self.file_dialog().add_filter(label, exts).pick_file() {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let title = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Imported".to_string());
                    let id = self.doc.import_as_node(title, &content, html);
                    self.selected = Some(id);
                    self.mark_dirty();
                    self.status = format!("Imported {} as a node", label);
                }
                Err(e) => self.status = format!("Read error: {e}"),
            }
        }
    }

    fn export_html(&mut self) {
        if let Some(path) = self.file_dialog()
            .add_filter("HTML", &["html"])
            .set_file_name("trellis-export.html")
            .save_file()
        {
            match std::fs::write(&path, self.doc.export_html()) {
                Ok(_) => self.status = format!("Exported HTML → {}", path.display()),
                Err(e) => self.status = format!("Export failed: {e}"),
            }
        }
    }

    fn export_json(&mut self) {
        if let Some(path) = self.file_dialog()
            .add_filter("JSON", &["json"])
            .set_file_name("trellis-export.json")
            .save_file()
        {
            match self.doc.export_json() {
                Ok(s) => match std::fs::write(&path, s) {
                    Ok(_) => self.status = format!("Exported JSON → {}", path.display()),
                    Err(e) => self.status = format!("Export failed: {e}"),
                },
                Err(e) => self.status = format!("Serialize failed: {e}"),
            }
        }
    }

    fn export_markdown(&mut self) {
        if let Some(path) = self.file_dialog()
            .add_filter("Markdown", &["md"])
            .set_file_name("trellis-export.md")
            .save_file()
        {
            match std::fs::write(&path, self.doc.export_markdown()) {
                Ok(_) => self.status = format!("Exported Markdown → {}", path.display()),
                Err(e) => self.status = format!("Export failed: {e}"),
            }
        }
    }

    fn export_pdf(&mut self) {
        if let Some(path) = self.file_dialog()
            .add_filter("PDF", &["pdf"])
            .set_file_name("trellis-export.pdf")
            .save_file()
        {
            match self.doc.export_pdf().and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())) {
                Ok(_) => self.status = format!("Exported PDF → {}", path.display()),
                Err(e) => self.status = format!("Export failed: {e}"),
            }
        }
    }

    fn export_image(&mut self, gif: bool) {
        let (label, ext, name) = if gif {
            ("GIF", "gif", "trellis-export.gif")
        } else {
            ("PNG", "png", "trellis-export.png")
        };
        if let Some(path) = self.file_dialog()
            .add_filter(label, &[ext])
            .set_file_name(name)
            .save_file()
        {
            match self.doc.export_image(gif).and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())) {
                Ok(_) => self.status = format!("Exported {label} → {}", path.display()),
                Err(e) => self.status = format!("Export failed: {e}"),
            }
        }
    }

    /// Load a JSON-exported document, replacing the current one. JSON isn't the
    /// native save format, so the result is treated as an unsaved document.
    fn import_json(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(path) = self.file_dialog().add_filter("JSON", &["json"]).pick_file() {
            match std::fs::read_to_string(&path).map(|s| serde_json::from_str::<Document>(&s)) {
                Ok(Ok(doc)) => {
                    self.reset_inline_images();
                    self.doc = doc;
                    self.selected = self.doc.roots.first().copied();
                    self.views.clear();
                    self.reset_history();
                    self.doc_path = None;
                    self.mark_dirty();
                    self.status = format!("Imported {}", path.display());
                }
                Ok(Err(e)) => self.status = format!("JSON parse error: {e}"),
                Err(e) => self.status = format!("Read error: {e}"),
            }
        }
    }

    // --- action application -------------------------------------------------

    /// Push described changes, filling in the ids of anything created.
    ///
    /// A create is described before it exists, so it arrives here with `id == 0`
    /// and takes the next id that appeared while the actions were applied.
    fn flush_notes(
        &mut self,
        pending: &mut Vec<crate::changelog::Change>,
        mut next_new_id: impl FnMut() -> Option<u64>,
    ) {
        for mut c in pending.drain(..) {
            if c.id == 0 {
                if let Some(id) = next_new_id() {
                    c.id = id;
                }
            }
            self.note(c);
        }
    }

    /// What a tree action is about to change, or `None` if it isn't a document
    /// edit at all.
    ///
    /// **`None` must mean exactly "does not dirty the document"** — this replaced
    /// a blanket `matches!` guard that decided the same thing, so a variant that
    /// wrongly returns `None` here stops its edit being saved, not merely logged.
    fn describe_tree(&self, a: &TreeAction) -> Option<crate::changelog::Change> {
        use crate::changelog::{Actor::Ui, Change, Entity::Node, Op};
        let title = |id: &NodeId| self.doc.nodes.get(id).map(|n| n.title.clone()).unwrap_or_default();
        let ch = |op, id| Change::new(Ui, Node, op, id);
        Some(match a {
            // View state, or a file dialog that records its own result.
            TreeAction::Select(_)
            | TreeAction::ToggleReorder
            | TreeAction::ExportBasket(..)
            | TreeAction::ExportBasketPdf(_)
            | TreeAction::ExportBasketPng(_)
            | TreeAction::ImportBasket(_)
            // Running a plugin changes nothing by itself; whatever it does over
            // the API is recorded there, under its own token.
            | TreeAction::RunPlugin(..) => return None,

            TreeAction::AddRoot => ch(Op::Created, 0),
            TreeAction::AddChild(p) => ch(Op::Created, 0).field(&format!("parent={p}")),
            TreeAction::AddSibling(s) => ch(Op::Created, 0).field(&format!("sibling={s}")),
            TreeAction::Remove(id) => ch(Op::Deleted, *id).titled(title(id)),
            TreeAction::Rename(id, t) => ch(Op::Updated, *id).titled(t.clone()).field("title"),
            TreeAction::ToggleExpand(id) => ch(Op::Updated, *id).titled(title(id)).field("expanded"),
            TreeAction::SetSubtreeExpanded(id, _) => {
                ch(Op::Updated, *id).titled(title(id)).field("expanded.subtree")
            }
            TreeAction::SetColor(id, _) => ch(Op::Updated, *id).titled(title(id)).field("color"),
            TreeAction::SetBg(id, _) => ch(Op::Updated, *id).titled(title(id)).field("bg"),
            TreeAction::MoveUp(id)
            | TreeAction::MoveDown(id)
            | TreeAction::MoveToTop(id)
            | TreeAction::MoveToBottom(id)
            | TreeAction::Indent(id)
            | TreeAction::Outdent(id) => ch(Op::Moved, *id).titled(title(id)),
            TreeAction::Reorder { moved, .. } => ch(Op::Moved, *moved).titled(title(moved)),
        })
    }

    /// What a canvas action is about to change. Same contract as
    /// [`Self::describe_tree`]: `None` means "not a document edit".
    fn describe_canvas(
        &self,
        a: &CanvasAction,
        node: NodeId,
    ) -> Option<crate::changelog::Change> {
        use crate::changelog::{Actor::Ui, Change, Entity, Op};
        let title = |id: &CardId| self.doc.card(node, *id).map(|c| c.title.clone()).unwrap_or_default();
        let card = |op, id: CardId| Change::new(Ui, Entity::Card, op, id).in_node(node);
        let group = |op, id: crate::model::GroupId| Change::new(Ui, Entity::Group, op, id).in_node(node);
        let upd = |id: &CardId, f: &str| card(Op::Updated, *id).titled(title(id)).field(f);
        Some(match a {
            // Pure view/clipboard/export, plus the template actions, which record
            // themselves where the library is actually touched.
            CanvasAction::ResetView
            | CanvasAction::CopyCard(_)
            | CanvasAction::SaveImage(..)
            | CanvasAction::SaveAllImages(_)
            | CanvasAction::ExportCardPng(_)
            | CanvasAction::ExportCardMarkdown(_)
            | CanvasAction::ExportCardPdf(_)
            | CanvasAction::ExportCardHtml(_)
            | CanvasAction::ExportCardText(_)
            | CanvasAction::ExportCardSvg(_)
            | CanvasAction::ExportCardJson(_)
            | CanvasAction::TableExportCsv(_)
            | CanvasAction::TableExportXlsx(_)
            | CanvasAction::ToggleSelect(_)
            | CanvasAction::ClearSelection
            | CanvasAction::ToggleDockMode
            | CanvasAction::ToggleSnapMode
            | CanvasAction::SaveAsTemplate(_)
            | CanvasAction::UpdateTemplate(..)
            | CanvasAction::DeleteTemplate(_) => return None,

            // Created — the id doesn't exist yet; `flush_notes` fills it in.
            CanvasAction::AddCard(kind, _) => card(Op::Created, 0).field(kind.label()),
            CanvasAction::PasteCard(_) => card(Op::Created, 0).field("paste"),
            CanvasAction::ImportCard(_) => card(Op::Created, 0).field("import"),
            CanvasAction::InsertTemplate(..) => card(Op::Created, 0).field("template.insert"),
            CanvasAction::Duplicate(c) => card(Op::Created, 0).field(&format!("duplicate={c}")),
            CanvasAction::DropFiles(..) => card(Op::Created, 0).field("drop"),

            CanvasAction::Remove(c) => card(Op::Deleted, *c).titled(title(c)),
            CanvasAction::MoveCard(c, _) => card(Op::Moved, *c).titled(title(c)).field("pos"),
            CanvasAction::RaiseCard(c) => card(Op::Moved, *c).titled(title(c)).field("order"),
            CanvasAction::ResizeCard(c, _) => upd(c, "size"),
            CanvasAction::FitCard(c) => upd(c, "size"),
            CanvasAction::SetTitle(c, _) => upd(c, "title"),
            CanvasAction::SetBody(c, _) => upd(c, "body"),
            CanvasAction::SetLang(c, _) => upd(c, "lang"),
            CanvasAction::SetColor(c, _) => upd(c, "color"),
            CanvasAction::SetFontScale(c, _) => upd(c, "font_scale"),
            CanvasAction::SetEditing(c, _) => upd(c, "editing"),
            CanvasAction::ChecklistToggle(c, _) => upd(c, "items.toggle"),
            CanvasAction::ChecklistSetText(c, ..) => upd(c, "items.text"),
            CanvasAction::ChecklistAdd(c) => upd(c, "items.add"),
            CanvasAction::ChecklistRemove(c, _) => upd(c, "items.remove"),
            CanvasAction::ChecklistMove(c, ..) => upd(c, "items.move"),
            CanvasAction::SketchAddStroke(c, _) => upd(c, "sketch.add_stroke"),
            CanvasAction::SketchUndo(c) => upd(c, "sketch.undo"),
            CanvasAction::SketchClear(c) => upd(c, "sketch.clear"),
            CanvasAction::LoadImage(c) => upd(c, "images.add"),
            CanvasAction::InsertInlineImage(c, _) => upd(c, "inline_images"),
            CanvasAction::RemoveImage(c, _) => upd(c, "images.remove"),
            CanvasAction::OcrCard(c) => upd(c, "ocr"),
            CanvasAction::OpenLightbox(c, _) => upd(c, "lightbox"),
            CanvasAction::TableSetCell(c, ..) => upd(c, "table.set_cell"),
            CanvasAction::TableSetBg(c, ..) => upd(c, "table.set_bg"),
            CanvasAction::TableSetFg(c, ..) => upd(c, "table.set_fg"),
            CanvasAction::TableInsertRow(c, _) => upd(c, "table.insert_row"),
            CanvasAction::TableRemoveRow(c, _) => upd(c, "table.remove_row"),
            CanvasAction::TableInsertCol(c, _) => upd(c, "table.insert_col"),
            CanvasAction::TableRemoveCol(c, _) => upd(c, "table.remove_col"),
            CanvasAction::TableSetColWidth(c, ..) => upd(c, "table.set_col_width"),
            CanvasAction::TableToggleHeader(c) => upd(c, "table.set_header"),
            CanvasAction::TableSetChart(c, spec) => {
                upd(c, if spec.is_some() { "chart" } else { "chart.clear" })
            }
            CanvasAction::TableImport(c) => upd(c, "rows"),
            CanvasAction::PickSource(c) => upd(c, "source"),
            CanvasAction::ClearSource(c) => upd(c, "source"),
            CanvasAction::DockCard(c, _) => upd(c, "dock"),
            CanvasAction::DetachCard(c) => upd(c, "dock"),

            CanvasAction::GroupSelected => group(Op::Created, 0).field("group"),
            CanvasAction::Ungroup(g) => group(Op::Deleted, *g),
            CanvasAction::RaiseGroup(g) => group(Op::Moved, *g).field("order"),
            CanvasAction::MoveGroup(g, _) => group(Op::Moved, *g).field("pos"),
            CanvasAction::SetGroupTitle(g, t) => group(Op::Updated, *g).titled(t.clone()).field("title"),
            CanvasAction::SetGroupColor(g, _) => group(Op::Updated, *g).field("color"),
        })
    }

    fn apply_tree(&mut self, actions: Vec<TreeAction>) {
        // Describe the edits *before* applying them, so a removed node's title is
        // still there to record, then push the entries afterwards so a client
        // never sees a change announced before it has happened.
        let mut pending: Vec<crate::changelog::Change> = actions
            .iter()
            .filter_map(|a| self.describe_tree(a))
            .collect();
        let known: std::collections::HashSet<NodeId> =
            if pending.iter().any(|c| c.id == 0) { self.doc.nodes.keys().copied().collect() }
            else { std::collections::HashSet::new() };
        for a in actions {
            match a {
                TreeAction::Select(id) => self.selected = Some(id),
                TreeAction::AddRoot => {
                    let id = self.doc.add_node(None, "Untitled".to_string());
                    self.selected = Some(id);
                    self.renaming = Some((id, "Untitled".to_string()));
                }
                TreeAction::AddChild(parent) => {
                    let id = self.doc.add_node(Some(parent), "Untitled".to_string());
                    if let Some(n) = self.doc.nodes.get_mut(&parent) {
                        n.expanded = true;
                    }
                    self.selected = Some(id);
                    self.renaming = Some((id, "Untitled".to_string()));
                }
                TreeAction::AddSibling(sib) => {
                    let id = self.doc.add_sibling(sib, "Untitled".to_string());
                    self.selected = Some(id);
                    self.renaming = Some((id, "Untitled".to_string()));
                }
                TreeAction::Remove(id) => {
                    self.doc.remove_node(id);
                    if self.selected == Some(id) {
                        self.selected = self.doc.roots.first().copied();
                    }
                }
                TreeAction::Rename(id, title) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.title = title;
                    }
                }
                TreeAction::ToggleExpand(id) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.expanded = !n.expanded;
                    }
                }
                TreeAction::SetSubtreeExpanded(id, expanded) => {
                    self.doc.set_subtree_expanded(id, expanded, true);
                }
                TreeAction::MoveUp(id) => self.doc.move_sibling(id, true),
                TreeAction::MoveDown(id) => self.doc.move_sibling(id, false),
                TreeAction::MoveToTop(id) => self.doc.move_to_edge(id, true),
                TreeAction::MoveToBottom(id) => self.doc.move_to_edge(id, false),
                TreeAction::Reorder { moved, target, before } => {
                    self.doc.reorder(moved, target, before);
                }
                TreeAction::ToggleReorder => self.reorder_mode = !self.reorder_mode,
                TreeAction::Indent(id) => self.doc.indent(id),
                TreeAction::Outdent(id) => self.doc.outdent(id),
                TreeAction::SetColor(id, col) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.color = col;
                    }
                }
                TreeAction::SetBg(id, bg) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.bg = bg;
                    }
                }
                TreeAction::ExportBasket(id, fmt, subs) => self.export_basket(id, fmt, subs),
                TreeAction::ExportBasketPdf(id) => self.begin_basket_shot(id, BasketFmt::Pdf),
                TreeAction::ExportBasketPng(id) => self.begin_basket_shot(id, BasketFmt::Png),
                TreeAction::ImportBasket(id) => self.import_basket(id),
                TreeAction::RunPlugin(id, idx) => {
                    // The node is handed over in the environment, so the plugin
                    // knows which basket it was invoked on.
                    let title =
                        self.doc.nodes.get(&id).map(|n| n.title.clone()).unwrap_or_default();
                    self.run_plugin(
                        idx,
                        vec![
                            ("TRELLIS_NODE".to_string(), id.to_string()),
                            ("TRELLIS_NODE_TITLE".to_string(), title),
                        ],
                    );
                }
            }
        }
        let mut fresh: Vec<NodeId> =
            self.doc.nodes.keys().copied().filter(|id| !known.contains(id)).collect();
        fresh.sort_unstable(); // ids ascend with creation order; HashMap order does not
        let mut fresh = fresh.into_iter();
        self.flush_notes(&mut pending, move || fresh.next());
    }

    /// A filesystem-safe base filename from a node's title.
    fn basket_basename(&self, node: NodeId) -> String {
        let raw = self.doc.nodes.get(&node).map(|n| n.title.trim().to_string()).unwrap_or_default();
        let cleaned: String = raw
            .chars()
            .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' })
            .collect();
        let cleaned = cleaned.trim().trim_matches('.').trim();
        if cleaned.is_empty() { "basket".to_string() } else { cleaned.to_string() }
    }

    /// Export one basket (node) to a text/data file (Markdown/HTML/JSON), with a
    /// save dialog pre-filled from the node title.
    fn export_basket(&mut self, node: NodeId, fmt: crate::tree::BasketFormat, subs: bool) {
        use crate::tree::BasketFormat;
        let base = self.basket_basename(node);
        let (ext, filter, label) = match fmt {
            BasketFormat::Markdown => ("md", "Markdown", "Markdown"),
            BasketFormat::Html => ("html", "HTML", "HTML"),
            BasketFormat::Json => ("json", "JSON", "JSON"),
        };
        let suffix = if subs { "-with-subnodes" } else { "" };
        let Some(path) = self
            .file_dialog()
            .add_filter(filter, &[ext])
            .set_file_name(format!("{base}{suffix}.{ext}"))
            .save_file()
        else {
            return;
        };
        let content = match fmt {
            BasketFormat::Markdown => self.doc.export_node_markdown(node, subs),
            BasketFormat::Html => self.doc.export_node_html_doc(node, subs),
            BasketFormat::Json => self.doc.export_node_json(node, subs),
        };
        match content {
            Some(s) => match std::fs::write(&path, s) {
                Ok(_) => self.status = format!("Exported basket {label} → {}", path.display()),
                Err(e) => self.status = format!("Export failed: {e}"),
            },
            None => self.status = "Export failed: node not found".to_string(),
        }
    }

    /// Import a basket JSON file as a child of `parent`, rebuilding its cards
    /// (and any subtree) with fresh ids.
    fn import_basket(&mut self, parent: NodeId) {
        let Some(path) =
            self.file_dialog().add_filter("Trellis basket (JSON)", &["json"]).pick_file()
        else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match crate::model::parse_node_export(&text) {
                Some(exp) => {
                    let title = exp.title.clone();
                    let new = self.doc.add_node_from_export(Some(parent), exp);
                    if let Some(n) = self.doc.nodes.get_mut(&parent) {
                        n.expanded = true;
                    }
                    self.selected = Some(new);
                    self.mark_dirty();
                    self.status = format!("Imported basket \"{title}\"");
                }
                None => self.status = "Import failed: not a Trellis basket file".to_string(),
            },
            Err(e) => self.status = format!("Import failed: {e}"),
        }
    }

    /// Turn OS-dropped files into cards: images → an image card, anything that
    /// decodes as UTF-8 text (txt/md/source/…) → a text card holding the file's
    /// contents. Cards fan out from the drop position; unknown binaries are
    /// skipped.
    /// The topmost Text card whose rect contains world-space `pos`, if any — so a
    /// dropped image can go inline into a note rather than spawn a new card.
    fn text_card_at(&self, node: NodeId, pos: egui::Pos2) -> Option<crate::model::CardId> {
        let n = self.doc.nodes.get(&node)?;
        n.cards
            .iter()
            .rev()
            .find(|c| {
                matches!(c.kind, CardKind::Text)
                    && egui::Rect::from_min_size(c.pos, c.size).contains(pos)
            })
            .map(|c| c.id)
    }

    fn drop_files(&mut self, node: NodeId, files: Vec<egui::DroppedFile>, pos: egui::Pos2) {
        let mut n = 0usize;
        for f in files {
            let bytes: Vec<u8> = match f.bytes.as_ref() {
                Some(b) => b.to_vec(),
                None => match f.path.as_ref().and_then(|p| std::fs::read(p).ok()) {
                    Some(b) => b,
                    None => continue,
                },
            };
            let name = f
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.name.clone());
            let ext = f
                .path
                .as_ref()
                .and_then(|p| p.extension())
                .map(|s| s.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let at = pos + egui::vec2(24.0, 24.0) * n as f32;
            if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp") {
                // Dropping an image onto a Text card embeds it inline in that
                // note; anywhere else it becomes a new image card.
                if let Some(tid) = self.text_card_at(node, pos) {
                    let alt = name.rsplit_once('.').map(|(a, _)| a).unwrap_or(&name).to_string();
                    if let Some(idx) = self.doc.add_inline_image(node, tid, bytes, name) {
                        if let Some(c) = self.doc.card_mut(node, tid) {
                            if !c.body.is_empty() && !c.body.ends_with('\n') {
                                c.body.push('\n');
                            }
                            c.body.push_str(&format!("![{alt}](trellis:{idx})\n"));
                        }
                        n += 1;
                    }
                } else {
                    let kind = CardKind::Image { data: Vec::new(), name: String::new(), extra: Vec::new(), ocr: String::new() };
                    if let Some(cid) = self.doc.add_card(node, at, kind) {
                        self.doc.add_image(node, cid, bytes, name);
                        n += 1;
                    }
                }
            } else if let Ok(text) = String::from_utf8(bytes) {
                // A dropped Trellis JSON card file becomes that exact card; any
                // other `.json` (or text) falls back to a text card.
                let imported = ext == "json"
                    && crate::model::parse_card_export(&text)
                        .and_then(|exp| self.doc.add_card_from_export(node, at, exp))
                        .is_some();
                if imported {
                    n += 1;
                } else if let Some(cid) = self.doc.add_card(node, at, CardKind::Text) {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.title = name;
                        c.body = text;
                        c.editing = false;
                    }
                    n += 1;
                }
            }
        }
        if n > 0 {
            self.mark_dirty();
            self.status = format!("Added {n} card{} from dropped files", if n == 1 { "" } else { "s" });
        }
    }

    fn apply_canvas(
        &mut self,
        ctx: &egui::Context,
        node: NodeId,
        actions: Vec<CanvasAction>,
        pointer_down: bool,
    ) {
        // Described before the loop (a removed card's title has to be read while
        // it still exists) and recorded after it (so nothing is announced before
        // it is true). Actions that only touch view state describe to `None` and
        // therefore don't dirty the document — which is what the old blanket
        // `matches!` guard was for.
        let mut pending: Vec<crate::changelog::Change> = actions
            .iter()
            .filter_map(|a| self.describe_canvas(a, node))
            .collect();
        // Creating actions can't know the new id yet, so note which cards exist
        // now and fill the blanks in from the difference afterwards. A basket
        // holds ~10 cards, so this is nothing.
        let known: std::collections::HashSet<CardId> = if pending.iter().any(|c| c.id == 0) {
            self.doc.nodes.get(&node).map(|n| n.cards.iter().map(|c| c.id).collect()).unwrap_or_default()
        } else {
            std::collections::HashSet::new()
        };
        // Undo: snapshot the node *before* mutating it. A discrete edit is its
        // own step; a held drag (same coalesce tag while the button is down)
        // collapses into one. Text/selection/view actions don't snapshot — egui
        // handles text-field undo itself.
        if !pointer_down {
            self.undo_coalesce = None;
        }
        let mut discrete = false;
        let mut cont: Option<&'static str> = None;
        for a in &actions {
            match undo_kind(a) {
                UndoKind::Discrete => discrete = true,
                UndoKind::Continuous(t) => cont = Some(t),
                UndoKind::None => {}
            }
        }
        if discrete {
            self.push_undo(node);
            self.undo_coalesce = None;
        } else if let Some(t) = cont {
            if self.undo_coalesce != Some(t) {
                self.push_undo(node);
                self.undo_coalesce = Some(t);
            }
        }
        for a in actions {
            match a {
                CanvasAction::AddCard(kind, pos) => {
                    self.doc.add_card(node, pos, kind);
                }
                CanvasAction::MoveCard(cid, delta) => {
                    // Moves the card plus anything docked to it.
                    self.doc.move_card_tree(node, cid, delta);
                }
                CanvasAction::ResizeCard(cid, delta) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.size = (c.size + delta).max(MIN_CARD);
                    }
                }
                CanvasAction::FitCard(cid) => {
                    // Measure Text height from the real galley (matches the render)
                    // rather than the off-thread estimate — see `fit_card_size`.
                    if let Some(sz) =
                        self.doc.card(node, cid).and_then(|c| fit_card_size(ctx, c))
                    {
                        if let Some(c) = self.doc.card_mut(node, cid) {
                            c.size = sz.max(MIN_CARD);
                        }
                    }
                }
                CanvasAction::SaveAsTemplate(cid) => {
                    if let Some((_, name)) = self.register_template(node, cid, None) {
                        self.status =
                            format!("Saved template \"{name}\" — master card in {TEMPLATES_NODE_TITLE}");
                    }
                }
                CanvasAction::InsertTemplate(idx, pos) => {
                    if let Some(exp) = self.templates.get(idx).map(|t| t.card.clone()) {
                        let name = exp.title.clone();
                        if self.doc.add_card_from_export(node, pos, exp).is_some() {
                            self.status = format!("Inserted template \"{name}\"");
                        }
                    }
                }
                CanvasAction::UpdateTemplate(idx, cid) => {
                    if let Some(name) = self.update_template(idx, node, cid, None) {
                        self.status = format!("Updated template \"{name}\"");
                    }
                }
                CanvasAction::DeleteTemplate(idx) => {
                    if let Some(name) = self.delete_template(idx) {
                        self.status = format!("Deleted template \"{name}\" and its master card");
                    }
                }
                CanvasAction::RaiseCard(cid) => self.doc.raise_card(node, cid),
                CanvasAction::SetTitle(cid, t) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.title = t;
                    }
                }
                CanvasAction::SetBody(cid, b) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.body = b;
                    }
                }
                CanvasAction::SetLang(cid, lang) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Code { lang: l } = &mut c.kind {
                            *l = lang;
                        }
                    }
                }
                CanvasAction::SetColor(cid, col) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.color = col;
                    }
                }
                CanvasAction::SetFontScale(cid, s) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.font_scale = s;
                    }
                }
                CanvasAction::DropFiles(files, pos) => {
                    self.drop_files(node, files, pos);
                }
                CanvasAction::SetEditing(cid, ed) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.editing = ed;
                    }
                }
                CanvasAction::Duplicate(cid) => {
                    self.doc.duplicate_card(node, cid);
                }
                CanvasAction::CopyCard(cid) => {
                    if let Some(n) = self.doc.nodes.get(&node) {
                        if let Some(c) = n.cards.iter().find(|c| c.id == cid) {
                            self.card_clipboard = Some(c.clone());
                            self.status = "Copied card".to_string();
                        }
                    }
                }
                CanvasAction::PasteCard(pos) => {
                    if let Some(tmpl) = self.card_clipboard.clone() {
                        self.doc.add_card_from(node, &tmpl, pos);
                        self.status = "Pasted card".to_string();
                    }
                }
                CanvasAction::Remove(cid) => {
                    self.doc.remove_card(node, cid);
                    self.tex_cache.forget(cid);
                }
                CanvasAction::ChecklistToggle(cid, i) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Checklist { items } = &mut c.kind {
                            if let Some(it) = items.get_mut(i) {
                                it.done = !it.done;
                            }
                        }
                    }
                }
                CanvasAction::ChecklistSetText(cid, i, text) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Checklist { items } = &mut c.kind {
                            if let Some(it) = items.get_mut(i) {
                                it.text = text;
                            }
                        }
                    }
                }
                CanvasAction::ChecklistAdd(cid) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Checklist { items } = &mut c.kind {
                            items.push(ChecklistItem { done: false, text: String::new() });
                        }
                    }
                }
                CanvasAction::ChecklistRemove(cid, i) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Checklist { items } = &mut c.kind {
                            if i < items.len() {
                                items.remove(i);
                            }
                        }
                    }
                }
                CanvasAction::ChecklistMove(cid, from, to) => {
                    self.doc.move_checklist_item(node, cid, from, to);
                }
                CanvasAction::SketchAddStroke(cid, stroke) => {
                    self.doc.sketch_add_stroke(node, cid, stroke);
                }
                CanvasAction::SketchUndo(cid) => {
                    self.doc.sketch_undo(node, cid);
                }
                CanvasAction::SketchClear(cid) => {
                    self.doc.sketch_clear(node, cid);
                }
                CanvasAction::LoadImage(cid) => self.load_image_into(node, cid),
                CanvasAction::InsertInlineImage(cid, at) => {
                    self.insert_inline_image_into(node, cid, at)
                }
                CanvasAction::TableSetCell(cid, r, c, text) => {
                    let _ = self.doc.table_set_cell(node, cid, r, c, text);
                }
                CanvasAction::TableSetBg(cid, r, c, bg) => {
                    let _ = self.doc.table_set_bg(node, cid, r, c, bg);
                }
                CanvasAction::TableSetFg(cid, r, c, fg) => {
                    let _ = self.doc.table_set_fg(node, cid, r, c, fg);
                }
                CanvasAction::TableInsertRow(cid, at) => {
                    let _ = self.doc.table_insert_row(node, cid, at);
                }
                CanvasAction::TableRemoveRow(cid, at) => {
                    let _ = self.doc.table_remove_row(node, cid, at);
                }
                CanvasAction::TableInsertCol(cid, at) => {
                    let _ = self.doc.table_insert_col(node, cid, at);
                }
                CanvasAction::TableRemoveCol(cid, at) => {
                    let _ = self.doc.table_remove_col(node, cid, at);
                }
                CanvasAction::TableSetColWidth(cid, c, w) => {
                    let _ = self.doc.table_set_col_width(node, cid, c, w);
                }
                CanvasAction::TableSetChart(cid, spec) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Table { table } = &mut c.kind {
                            table.chart = spec;
                        }
                    }
                }
                CanvasAction::TableToggleHeader(cid) => {
                    let _ = self.doc.table_toggle_header(node, cid);
                }
                CanvasAction::TableImport(cid) => self.table_import(node, cid),
                CanvasAction::TableExportCsv(cid) => self.table_export(node, cid, false),
                CanvasAction::TableExportXlsx(cid) => self.table_export(node, cid, true),
                CanvasAction::RemoveImage(cid, idx) => {
                    if self.doc.remove_image(node, cid, idx) {
                        self.tex_cache.forget(cid);
                    }
                }
                CanvasAction::OcrCard(cid) => {
                    let images: Vec<Vec<u8>> = self
                        .doc
                        .nodes
                        .get(&node)
                        .and_then(|n| n.cards.iter().find(|c| c.id == cid))
                        .map(|c| c.kind.images().iter().map(|(d, _)| d.to_vec()).collect())
                        .unwrap_or_default();
                    if images.is_empty() {
                        self.status = "Nothing to OCR (no image loaded)".into();
                    } else {
                        self.status = "OCR running…".into();
                        let tx = self.ocr_tx.clone();
                        let ctx = self.egui_ctx.clone();
                        std::thread::spawn(move || {
                            let res = ocr_images(&images);
                            let _ = tx.send((node, cid, res));
                            ctx.request_repaint();
                        });
                    }
                }
                CanvasAction::SaveImage(cid, idx) => self.save_card_image(node, cid, idx),
                CanvasAction::SaveAllImages(cid) => self.save_all_card_images(node, cid),
                CanvasAction::ExportCardPng(cid) => self.begin_card_shot(node, cid, false),
                CanvasAction::ExportCardMarkdown(cid) => {
                    let d = self.doc.export_card_markdown(node, cid).map(|s| s.into_bytes());
                    self.save_card_export(node, cid, "md", "Markdown", d.ok_or_else(card_gone));
                }
                CanvasAction::ExportCardPdf(cid) => self.begin_card_shot(node, cid, true),
                CanvasAction::ExportCardHtml(cid) => {
                    let d = self.doc.export_card_html(node, cid).map(|s| s.into_bytes());
                    self.save_card_export(node, cid, "html", "HTML", d.ok_or_else(card_gone));
                }
                CanvasAction::ExportCardText(cid) => {
                    let d = self.doc.export_card_text(node, cid).map(|s| s.into_bytes());
                    self.save_card_export(node, cid, "txt", "Text", d.ok_or_else(card_gone));
                }
                CanvasAction::ExportCardSvg(cid) => {
                    let d = self.doc.export_card_svg(node, cid).map(|s| s.into_bytes());
                    self.save_card_export(node, cid, "svg", "SVG", d.ok_or_else(card_gone));
                }
                CanvasAction::ExportCardJson(cid) => {
                    let d = self.doc.export_card_json(node, cid).map(|s| s.into_bytes());
                    self.save_card_export(node, cid, "json", "Trellis card (JSON)", d.ok_or_else(card_gone));
                }
                CanvasAction::ImportCard(pos) => self.import_card(node, pos),
                CanvasAction::OpenLightbox(cid, idx) => {
                    self.lightbox = Some(Lightbox {
                        node,
                        card: cid,
                        index: idx,
                        zoom: 1.0,
                        pan: egui::Vec2::ZERO,
                    });
                }
                CanvasAction::ToggleSelect(cid) => {
                    if !self.card_sel.insert(cid) {
                        self.card_sel.remove(&cid);
                    }
                }
                CanvasAction::ClearSelection => self.card_sel.clear(),
                CanvasAction::ToggleDockMode => self.dock_mode = !self.dock_mode,
                CanvasAction::ToggleSnapMode => self.snap_mode = !self.snap_mode,
                CanvasAction::GroupSelected => {
                    let ids: Vec<_> = self.card_sel.iter().copied().collect();
                    if self.doc.group_cards(node, &ids, "Group".to_string()).is_some() {
                        self.status = format!("Grouped {} cards", ids.len());
                    }
                    self.card_sel.clear();
                }
                CanvasAction::Ungroup(g) => self.doc.ungroup(node, g),
                CanvasAction::RaiseGroup(g) => self.doc.raise_group(node, g),
                CanvasAction::MoveGroup(g, delta) => self.doc.move_group(node, g, delta),
                CanvasAction::SetGroupTitle(g, t) => self.doc.set_group_title(node, g, t),
                CanvasAction::SetGroupColor(g, c) => self.doc.set_group_color(node, g, c),
                CanvasAction::PickSource(cid) => {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Mirror a file in this card")
                        .pick_file()
                    {
                        if let Some(c) = self.doc.card_mut(node, cid) {
                            c.source = Some(path.to_string_lossy().to_string());
                            c.source_mtime = None; // force the next poll to read it
                            c.source_error = None;
                            c.editing = false;
                        }
                        // Don't wait up to 3s for the timer to notice.
                        self.pump_sources(true);
                    }
                }
                CanvasAction::ClearSource(cid) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.source = None;
                        c.source_mtime = None;
                        c.source_error = None;
                    }
                }
                CanvasAction::DockCard(child, anchor) => self.doc.dock_card(node, child, anchor),
                CanvasAction::DetachCard(cid) => self.doc.detach_card(node, cid),
                CanvasAction::ResetView => {
                    self.views.insert(node, TSTransform::IDENTITY);
                }
            }
        }
        let mut fresh = self
            .doc
            .nodes
            .get(&node)
            .map(|n| n.cards.iter().map(|c| c.id).filter(|id| !known.contains(id)).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter();
        self.flush_notes(&mut pending, move || fresh.next());
    }

    /// Import a CSV/XLSX file into a table card (replaces its contents).
    fn table_import(&mut self, node: NodeId, card: crate::model::CardId) {
        let Some(path) = self
            .file_dialog()
            .add_filter("Table", &["csv", "xlsx"])
            .pick_file()
        else {
            return;
        };
        let is_xlsx = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("xlsx"))
            .unwrap_or(false);
        let values = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| {
            if is_xlsx {
                crate::model::xlsx_to_values(&bytes)
            } else {
                crate::model::csv_to_values(&bytes)
            }
        });
        match values {
            Ok(v) => {
                if self.doc.table_replace(node, card, v) {
                    self.mark_dirty();
                    self.status = format!("Imported {}", path.display());
                }
            }
            Err(e) => self.status = format!("Import failed: {e}"),
        }
    }

    /// Export a table card as CSV or XLSX (colors survive in XLSX).
    fn table_export(&mut self, node: NodeId, card: crate::model::CardId, xlsx: bool) {
        let Some(c) = self.doc.card_mut(node, card) else { return };
        let CardKind::Table { table } = c.kind.clone() else { return };
        let (label, ext, default) = if xlsx {
            ("Excel", "xlsx", "table.xlsx")
        } else {
            ("CSV", "csv", "table.csv")
        };
        let Some(path) = self
            .file_dialog()
            .add_filter(label, &[ext])
            .set_file_name(default)
            .save_file()
        else {
            return;
        };
        let data = if xlsx {
            table.to_xlsx()
        } else {
            Ok(table.to_csv().into_bytes())
        };
        match data.and_then(|d| std::fs::write(&path, d).map_err(|e| e.to_string())) {
            Ok(_) => self.status = format!("Exported → {}", path.display()),
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    /// Save a single image from an image card to a file the user picks. The
    /// dialog pre-fills the image's stored name so its original extension is kept.
    ///
    /// See [`download_image_name`] for the filename fallback.
    fn save_card_image(&mut self, node: NodeId, card: crate::model::CardId, idx: usize) {
        let Some(c) = self.doc.card_mut(node, card) else { return };
        let imgs = c.kind.images();
        let Some((bytes, name)) = imgs.get(idx) else { return };
        let bytes = bytes.to_vec();
        let name = download_image_name(name, idx);
        let ext = std::path::Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_string();
        let Some(path) = self
            .file_dialog()
            .add_filter("Image", &[ext.as_str()])
            .set_file_name(&name)
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, &bytes) {
            Ok(_) => self.status = format!("Saved image → {}", path.display()),
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    /// Save every image of an image card into a folder the user picks, each under
    /// its stored name (de-duplicated so nothing is silently overwritten).
    fn save_all_card_images(&mut self, node: NodeId, card: crate::model::CardId) {
        let Some(c) = self.doc.card_mut(node, card) else { return };
        let files: Vec<(String, Vec<u8>)> = c
            .kind
            .images()
            .iter()
            .enumerate()
            .map(|(i, (bytes, name))| (download_image_name(name, i), bytes.to_vec()))
            .collect();
        if files.is_empty() {
            return;
        }
        let Some(dir) = self.file_dialog().pick_folder() else { return };
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut saved = 0usize;
        let mut err = None;
        for (name, bytes) in files {
            let mut target = dir.join(&name);
            let mut n = 1;
            while used.contains(&target.to_string_lossy().to_string()) || target.exists() {
                let stem = std::path::Path::new(&name)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| name.clone());
                let ext = std::path::Path::new(&name)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                target = dir.join(format!("{stem}-{n}{ext}"));
                n += 1;
            }
            used.insert(target.to_string_lossy().to_string());
            match std::fs::write(&target, &bytes) {
                Ok(_) => saved += 1,
                Err(e) => {
                    err = Some(e.to_string());
                    break;
                }
            }
        }
        self.status = match err {
            Some(e) => format!("Saved {saved} image(s), then failed: {e}"),
            None => format!("Saved {saved} image(s) → {}", dir.display()),
        };
    }

    /// A default filename (no extension) for exporting a card: its title,
    /// sanitized to filesystem-safe characters, or `card` when it has no title.
    fn card_export_basename(&self, node: NodeId, card: crate::model::CardId) -> String {
        let raw = self
            .doc
            .card(node, card)
            .map(|c| c.title.trim().to_string())
            .unwrap_or_default();
        let cleaned: String = raw
            .chars()
            .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' })
            .collect();
        let cleaned = cleaned.trim().trim_matches('.').trim();
        if cleaned.is_empty() { "card".to_string() } else { cleaned.to_string() }
    }

    /// Shared tail for the per-card exporters: if the render succeeded, ask for a
    /// destination (pre-filled `<card title>.<ext>`) and write the bytes.
    fn save_card_export(
        &mut self,
        node: NodeId,
        card: crate::model::CardId,
        ext: &str,
        label: &str,
        data: Result<Vec<u8>, String>,
    ) {
        let bytes = match data {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Export failed: {e}");
                return;
            }
        };
        let base = self.card_export_basename(node, card);
        let Some(path) = self
            .file_dialog()
            .add_filter(label, &[ext])
            .set_file_name(format!("{base}.{ext}"))
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, &bytes) {
            Ok(_) => self.status = format!("Exported card → {}", path.display()),
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    /// Start a WYSIWYG single-card export: show the card's node and kick off the
    /// framing → screenshot state machine (driven in `update`). `pdf` picks PDF
    /// vs PNG.
    fn begin_card_shot(&mut self, node: NodeId, card: crate::model::CardId, pdf: bool) {
        self.selected = Some(node);
        let saved_view = self.views.get(&node).copied().unwrap_or_default();
        self.card_shot = Some(CardShot { node, card, pdf, saved_view, phase: ShotPhase::Framing });
        self.status = "Rendering card…".to_string();
    }

    /// Finish a single-card screenshot: restore the view, crop the framebuffer to
    /// the card's on-screen rect, and save it as PNG or (image-backed) PDF.
    fn finish_card_shot(&mut self, ctx: &egui::Context, image: &egui::ColorImage) {
        let Some(shot) = self.card_shot.take() else { return };
        self.views.insert(shot.node, shot.saved_view); // undo the temporary reframe
        let Some(rect) = self.card_rects.get(&shot.card).copied() else {
            self.status = "Export failed: card is not on screen".to_string();
            return;
        };
        let ppp = ctx.pixels_per_point();
        let [iw, ih] = image.size;
        let cx = |v: f32| (v.round() as i64).clamp(0, iw as i64) as usize;
        let cy = |v: f32| (v.round() as i64).clamp(0, ih as i64) as usize;
        let (x0, y0) = (cx(rect.min.x * ppp), cy(rect.min.y * ppp));
        let (x1, y1) = (cx(rect.max.x * ppp), cy(rect.max.y * ppp));
        if x1 <= x0 || y1 <= y0 {
            self.status = "Export failed: empty card region".to_string();
            return;
        }
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut rgba = Vec::with_capacity(cw * ch * 4);
        for y in y0..y1 {
            for x in x0..x1 {
                let px = image.pixels[y * iw + x];
                rgba.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
            }
        }
        let (ext, label, data) = if shot.pdf {
            ("pdf", "PDF", crate::model::image_rgba_to_pdf(&rgba, cw as u32, ch as u32))
        } else {
            ("png", "PNG", encode_png(&rgba, cw as u32, ch as u32))
        };
        self.save_card_export(shot.node, shot.card, ext, label, data);
    }

    /// Crop a full-window framebuffer to a card's on-screen rect (points) → RGBA.
    fn crop_shot(
        image: &egui::ColorImage,
        rect: egui::Rect,
        ppp: f32,
    ) -> Option<(Vec<u8>, u32, u32)> {
        let [iw, ih] = image.size;
        let cx = |v: f32| (v.round() as i64).clamp(0, iw as i64) as usize;
        let cy = |v: f32| (v.round() as i64).clamp(0, ih as i64) as usize;
        let (x0, y0) = (cx(rect.min.x * ppp), cy(rect.min.y * ppp));
        let (x1, y1) = (cx(rect.max.x * ppp), cy(rect.max.y * ppp));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut rgba = Vec::with_capacity(cw * ch * 4);
        for y in y0..y1 {
            for x in x0..x1 {
                let px = image.pixels[y * iw + x];
                rgba.extend_from_slice(&[px.r(), px.g(), px.b(), px.a()]);
            }
        }
        Some((rgba, cw as u32, ch as u32))
    }

    /// The card `pos`/`size` (canvas units) the current basket shot should frame,
    /// or `None` if no basket shot is framing this node.
    fn basket_frame_target(&self, sel: NodeId) -> Option<(egui::Pos2, egui::Vec2)> {
        let s = self.basket_shot.as_ref()?;
        // Frame during BOTH Framing and Requested: the framebuffer that egui
        // captures is the frame *after* the screenshot is requested, so the
        // reframe must still be in effect on that (Requested) frame or the shot
        // captures the un-reframed view.
        if s.node != sel {
            return None;
        }
        match s.queue.get(s.idx)? {
            ShotKind::Card(cid) => {
                let c = self.doc.card(sel, *cid)?;
                Some((c.pos, c.size))
            }
            ShotKind::Overview => self.basket_bbox(sel),
        }
    }

    /// Bounding box (pos, size) of all cards in a node's basket, in canvas units.
    fn basket_bbox(&self, node: NodeId) -> Option<(egui::Pos2, egui::Vec2)> {
        let n = self.doc.nodes.get(&node)?;
        if n.cards.is_empty() {
            return None;
        }
        let mut min = egui::pos2(f32::MAX, f32::MAX);
        let mut max = egui::pos2(f32::MIN, f32::MIN);
        for c in &n.cards {
            min.x = min.x.min(c.pos.x);
            min.y = min.y.min(c.pos.y);
            max.x = max.x.max(c.pos.x + c.size.x);
            max.y = max.y.max(c.pos.y + c.size.y);
        }
        Some((min, max - min))
    }

    /// Start a WYSIWYG basket export: select the node, queue an overview shot
    /// (plus a per-card shot for PDF), and let the screenshot loop drive it.
    fn begin_basket_shot(&mut self, node: NodeId, fmt: BasketFmt) {
        if self.card_shot.is_some() || self.basket_shot.is_some() {
            return;
        }
        let Some(n) = self.doc.nodes.get(&node) else { return };
        if n.cards.is_empty() {
            self.status = "Nothing to export: this basket is empty".to_string();
            return;
        }
        self.selected = Some(node);
        let saved_view = self.views.get(&node).copied().unwrap_or_default();
        let mut queue = vec![ShotKind::Overview];
        if matches!(fmt, BasketFmt::Pdf) {
            queue.extend(n.cards.iter().map(|c| ShotKind::Card(c.id)));
        }
        self.basket_shot =
            Some(BasketShot { node, fmt, saved_view, queue, idx: 0, captured: Vec::new(), phase: ShotPhase::Framing, framed: false });
        self.status = "Rendering basket…".to_string();
    }

    /// Capture the current basket shot from the framebuffer, then advance the
    /// queue (or finish and save when it's drained).
    fn capture_basket_shot(&mut self, ctx: &egui::Context, image: &egui::ColorImage) {
        let (node, kind, idx, qlen) = match self.basket_shot.as_ref() {
            Some(s) => match s.queue.get(s.idx) {
                Some(k) => (s.node, *k, s.idx, s.queue.len()),
                None => return,
            },
            None => return,
        };
        let ppp = ctx.pixels_per_point();
        // Crop rect: a card's rect, or the union of the basket's card rects.
        let rect = match kind {
            ShotKind::Card(cid) => self.card_rects.get(&cid).copied(),
            ShotKind::Overview => {
                let ids: Vec<_> = self
                    .doc
                    .nodes
                    .get(&node)
                    .map(|n| n.cards.iter().map(|c| c.id).collect())
                    .unwrap_or_default();
                let mut u: Option<egui::Rect> = None;
                for id in ids {
                    if let Some(r) = self.card_rects.get(&id) {
                        u = Some(u.map_or(*r, |acc| acc.union(*r)));
                    }
                }
                u
            }
        };
        let (rgba, w, h) = match rect.and_then(|r| Self::crop_shot(image, r, ppp)) {
            Some(v) => v,
            None => (Vec::new(), 0, 0),
        };
        let (title, text) = match kind {
            ShotKind::Overview => {
                let t = self.doc.nodes.get(&node).map(|n| n.title.clone()).unwrap_or_default();
                (format!("{t} — overview"), String::new())
            }
            ShotKind::Card(cid) => {
                let c = self.doc.card(node, cid);
                let title = c
                    .map(|c| {
                        if c.title.trim().is_empty() {
                            format!("({})", card_kind_label(&c.kind))
                        } else {
                            c.title.clone()
                        }
                    })
                    .unwrap_or_default();
                let text = self.doc.export_card_text(node, cid).unwrap_or_default();
                (title, text)
            }
        };
        if let Some(bs) = self.basket_shot.as_mut() {
            bs.captured.push(crate::model::ShotPage { rgba, w, h, title, text });
            bs.idx += 1;
            if bs.idx < bs.queue.len() {
                bs.phase = ShotPhase::Framing;
                bs.framed = false; // wait for the next shot's reframe to render
            }
        }
        if idx + 1 >= qlen {
            self.finish_basket_shot();
        }
    }

    /// Assemble the collected basket screenshots into a PNG or PDF and save it.
    fn finish_basket_shot(&mut self) {
        let Some(bs) = self.basket_shot.take() else { return };
        self.views.insert(bs.node, bs.saved_view); // undo the temporary reframe
        let base = self.basket_basename(bs.node);
        // Drop any shots that failed to crop (e.g. a card off-screen).
        let pages: Vec<_> = bs.captured.into_iter().filter(|p| p.w > 0 && p.h > 0).collect();
        if pages.is_empty() {
            self.status = "Export failed: nothing was captured".to_string();
            return;
        }
        match bs.fmt {
            BasketFmt::Png => {
                let ov = &pages[0];
                let data = encode_png(&ov.rgba, ov.w, ov.h);
                self.save_basket_visual("png", "PNG", data, &base);
            }
            BasketFmt::Pdf => {
                let data = crate::model::basket_pdf(&pages);
                self.save_basket_visual("pdf", "PDF", data, &base);
            }
        }
    }

    /// Save assembled basket-visual bytes via a save dialog (filename pre-filled).
    fn save_basket_visual(&mut self, ext: &str, label: &str, data: Result<Vec<u8>, String>, base: &str) {
        match data {
            Ok(bytes) => {
                if let Some(path) = self
                    .file_dialog()
                    .add_filter(label, &[ext])
                    .set_file_name(format!("{base}.{ext}"))
                    .save_file()
                {
                    match std::fs::write(&path, &bytes) {
                        Ok(_) => self.status = format!("Exported basket {label} → {}", path.display()),
                        Err(e) => self.status = format!("Export failed: {e}"),
                    }
                } else {
                    self.status = "Export cancelled".to_string();
                }
            }
            Err(e) => self.status = format!("Export failed: {e}"),
        }
    }

    /// Import a card from a JSON card file the user picks, placing it at `pos`.
    fn import_card(&mut self, node: NodeId, pos: egui::Pos2) {
        let Some(path) = self
            .file_dialog()
            .add_filter("Trellis card (JSON)", &["json"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match crate::model::parse_card_export(&text) {
                Some(exp) => {
                    let kind = card_kind_label(&exp.kind);
                    if self.doc.add_card_from_export(node, pos, exp).is_some() {
                        self.mark_dirty();
                        self.status = format!("Imported {kind} card");
                    } else {
                        self.status = "Import failed: could not create the card".to_string();
                    }
                }
                None => self.status = "Not a Trellis card file (wrong or missing format marker)".to_string(),
            },
            Err(e) => self.status = format!("Read error: {e}"),
        }
    }

    /// Pick an image file and embed it inline in a Text card's body, splicing a
    /// `![alt](trellis:N)` marker at char position `at` (the editor cursor).
    fn insert_inline_image_into(&mut self, node: NodeId, card: crate::model::CardId, at: usize) {
        let Some(path) = self
            .file_dialog()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp", "webp"])
            .pick_file()
        else {
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Image read error: {e}");
                return;
            }
        };
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let alt = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some(idx) = self.doc.add_inline_image(node, card, bytes, name) else {
            return;
        };
        let marker = format!("![{alt}](trellis:{idx})");
        if let Some(c) = self.doc.card_mut(node, card) {
            // Splice the marker at the cursor, padded with newlines so it lands on
            // its own line and renders as a block image.
            let at = at.min(c.body.chars().count());
            let byte = c.body.char_indices().nth(at).map(|(b, _)| b).unwrap_or(c.body.len());
            let mut ins = String::new();
            if byte > 0 && !c.body[..byte].ends_with('\n') {
                ins.push('\n');
            }
            ins.push_str(&marker);
            if byte < c.body.len() && !c.body[byte..].starts_with('\n') {
                ins.push('\n');
            }
            c.body.insert_str(byte, &ins);
        }
        self.mark_dirty();
        self.status = "Inserted image".to_string();
    }

    fn load_image_into(&mut self, node: NodeId, card: crate::model::CardId) {
        if let Some(path) = self.file_dialog()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp", "webp"])
            .pick_file()
        {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if self.doc.add_image(node, card, bytes, name) {
                        self.tex_cache.forget(card);
                        self.mark_dirty();
                        self.status = "Loaded image".to_string();
                    }
                }
                Err(e) => self.status = format!("Image read error: {e}"),
            }
        }
    }

    /// Full-screen image viewer: dark backdrop, fit-to-screen image, scroll or
    /// +/- to zoom, drag to pan, ←/→ (keys or buttons) to flip through the
    /// card's images, Esc / × / backdrop click to close.
    fn lightbox_ui(&mut self, ctx: &egui::Context) {
        let (node_id, card_id) = match &self.lightbox {
            Some(l) => (l.node, l.card),
            None => return,
        };
        let images: Vec<(&[u8], &str)> = match self
            .doc
            .nodes
            .get(&node_id)
            .and_then(|n| n.cards.iter().find(|c| c.id == card_id))
        {
            Some(c) => c.kind.images(),
            None => Vec::new(),
        };
        let n = images.len();
        if n == 0 {
            self.lightbox = None;
            return;
        }

        let (mut index, mut zoom, mut pan) = {
            let l = self.lightbox.as_ref().unwrap();
            (l.index.min(n - 1), l.zoom, l.pan)
        };
        let mut close = false;
        let mut step = 0isize;
        let screen_center = ctx.screen_rect().center();
        let mut scroll = 0.0;
        let mut pointer = None;
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Escape) {
                close = true;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                step = 1;
            }
            if i.key_pressed(egui::Key::ArrowLeft) {
                step = -1;
            }
            if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                zoom = (zoom * 1.25).min(10.0);
            }
            if i.key_pressed(egui::Key::Minus) {
                zoom = (zoom / 1.25).max(0.2);
            }
            scroll = i.raw_scroll_delta.y;
            pointer = i.pointer.hover_pos();
        });
        // Scroll zooms toward the pointer: keep the image point under the cursor
        // fixed by shifting the pan by the same ratio the zoom changed.
        if scroll != 0.0 {
            let old = zoom;
            zoom = (zoom * (1.0015f32).powf(scroll)).clamp(0.2, 10.0);
            if let Some(p) = pointer {
                let r = zoom / old;
                pan = (p - screen_center) * (1.0 - r) + pan * r;
            }
        }

        egui::Area::new(egui::Id::new("lightbox"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::Pos2::ZERO)
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                // Backdrop swallows canvas interactions; clicking it closes.
                let bg = ui.allocate_rect(screen, egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(235));
                if bg.clicked() {
                    close = true;
                }

                let (bytes, name) = images[index];
                let caption;
                if let Some(tex) = self.tex_cache.get(ctx, card_id, index, bytes) {
                    let img = tex.size_vec2();
                    let fit = (screen.width() * 0.94 / img.x)
                        .min(screen.height() * 0.88 / img.y)
                        .min(1.0);
                    let rect = egui::Rect::from_center_size(
                        screen.center() + pan,
                        img * fit * zoom,
                    );
                    let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                    if resp.dragged() {
                        pan += resp.drag_delta();
                    }
                    if resp.double_clicked() {
                        // Double-click toggles between fit and 2x.
                        zoom = if (zoom - 1.0).abs() < 0.01 { 2.0 } else { 1.0 };
                        pan = egui::Vec2::ZERO;
                    }
                    egui::Image::from_texture(egui::load::SizedTexture::from_handle(&tex))
                        .paint_at(ui, rect);
                    caption = format!(
                        "{} — {}/{} · {:.0}% · scroll or +/- to zoom · drag to pan · ←/→ next · Esc to close",
                        name,
                        index + 1,
                        n,
                        fit * zoom * 100.0
                    );
                } else {
                    caption = format!("{name} — unreadable image");
                }

                let fid = egui::FontId::proportional(14.0);
                ui.painter().text(
                    egui::pos2(screen.center().x, screen.bottom() - 14.0),
                    egui::Align2::CENTER_CENTER,
                    &caption,
                    fid,
                    egui::Color32::from_gray(0xd0),
                );

                // Controls on top of everything.
                let btn = |ui: &mut egui::Ui, r: egui::Rect, label: &str| {
                    ui.put(r, egui::Button::new(egui::RichText::new(label).size(20.0)))
                };
                let close_r = egui::Rect::from_min_size(
                    egui::pos2(screen.right() - 44.0, screen.top() + 8.0),
                    egui::vec2(36.0, 36.0),
                );
                if btn(ui, close_r, "×").clicked() {
                    close = true;
                }
                if n > 1 {
                    let side = egui::vec2(36.0, 72.0);
                    let prev_r = egui::Rect::from_center_size(
                        egui::pos2(screen.left() + 30.0, screen.center().y),
                        side,
                    );
                    let next_r = egui::Rect::from_center_size(
                        egui::pos2(screen.right() - 30.0, screen.center().y),
                        side,
                    );
                    if btn(ui, prev_r, "◀").clicked() {
                        step = -1;
                    }
                    if btn(ui, next_r, "▶").clicked() {
                        step = 1;
                    }
                }
            });

        if step != 0 {
            index = (index as isize + step).rem_euclid(n as isize) as usize;
            zoom = 1.0;
            pan = egui::Vec2::ZERO;
        }
        if close {
            self.lightbox = None;
        } else if let Some(l) = self.lightbox.as_mut() {
            l.index = index;
            l.zoom = zoom;
            l.pan = pan;
        }
    }

    // --- panels -------------------------------------------------------------

    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New").clicked() {
                        self.new_document();
                        ui.close_menu();
                    }
                    if ui.button("Open…").clicked() {
                        self.open_document();
                        ui.close_menu();
                    }
                    if ui.button("Save").clicked() {
                        self.save();
                        ui.close_menu();
                    }
                    if ui.button("Save As…").clicked() {
                        self.save_as();
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.menu_button("Import", |ui| {
                        if ui.button("Markdown…").clicked() {
                            self.import(false);
                            ui.close_menu();
                        }
                        if ui.button("HTML…").clicked() {
                            self.import(true);
                            ui.close_menu();
                        }
                        if ui.button("JSON…").clicked() {
                            self.import_json();
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Export", |ui| {
                        if ui.button("Markdown…").clicked() {
                            self.export_markdown();
                            ui.close_menu();
                        }
                        if ui.button("HTML…").clicked() {
                            self.export_html();
                            ui.close_menu();
                        }
                        if ui.button("JSON…").clicked() {
                            self.export_json();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("PDF…").clicked() {
                            self.export_pdf();
                            ui.close_menu();
                        }
                        if ui.button("PNG image…").clicked() {
                            self.export_image(false);
                            ui.close_menu();
                        }
                        if ui.button("GIF image…").clicked() {
                            self.export_image(true);
                            ui.close_menu();
                        }
                    });
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo"))
                        .on_hover_text("Ctrl+Z — undo card moves, autosort, add/remove, etc.")
                        .clicked()
                    {
                        self.undo();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo"))
                        .on_hover_text("Ctrl+Shift+Z / Ctrl+Y")
                        .clicked()
                    {
                        self.redo();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Add root node").clicked() {
                        self.apply_tree(vec![TreeAction::AddRoot]);
                        ui.close_menu();
                    }
                    if let Some(sel) = self.selected {
                        if ui.button("Add child to selected").clicked() {
                            self.apply_tree(vec![TreeAction::AddChild(sel)]);
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .button("Go to node…")
                        .on_hover_text("Ctrl+O — fuzzy-jump to any node by title or path")
                        .clicked()
                    {
                        self.switcher_open = true;
                        self.switcher_query.clear();
                        self.switcher_index = 0;
                        ui.close_menu();
                    }
                    if ui
                        .button("Search…")
                        .on_hover_text("Ctrl+F — full-text search across titles and cards")
                        .clicked()
                    {
                        self.search_open = true;
                        ui.close_menu();
                    }
                    if ui
                        .button("Tags…")
                        .on_hover_text("Browse #tags and the cards that use them")
                        .clicked()
                    {
                        self.tags_open = true;
                        ui.close_menu();
                    }
                    if ui
                        .button("Find cards…")
                        .on_hover_text("Filter cards across the tree by tag, property, or text")
                        .clicked()
                    {
                        self.find_open = true;
                        ui.close_menu();
                    }
                    if ui
                        .button("Agenda (tasks)…")
                        .on_hover_text("Open tasks with a due:: date, grouped by when they're due")
                        .clicked()
                    {
                        self.agenda_open = true;
                        ui.close_menu();
                    }
                    if ui
                        .button("Backlinks…")
                        .on_hover_text("Cards that [[link]] to the selected node")
                        .clicked()
                    {
                        self.backlinks_open = true;
                        ui.close_menu();
                    }
                    if ui
                        .button("Link graph…")
                        .on_hover_text("Visualize the [[wiki-link]] web across the tree")
                        .clicked()
                    {
                        self.graph_open = true;
                        self.graph_built = false;
                        ui.close_menu();
                    }
                    if ui
                        .button("Kanban board…")
                        .on_hover_text("Cards with a status:: property as columns; drag to change status")
                        .clicked()
                    {
                        self.kanban_open = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.menu_button("Themes", |ui| {
                        for (t, label) in Theme::ALL {
                            if ui.selectable_label(self.theme == t, label).clicked() {
                                self.theme = t;
                                ui.close_menu();
                            }
                        }
                    });
                    ui.separator();
                    let has_sel = self.selected.is_some();
                    if ui.add_enabled(has_sel, egui::Button::new("Zoom in")).clicked() {
                        self.zoom_selected(1.1);
                    }
                    if ui.add_enabled(has_sel, egui::Button::new("Zoom out")).clicked() {
                        self.zoom_selected(1.0 / 1.1);
                    }
                    if ui.add_enabled(has_sel, egui::Button::new("Reset zoom")).clicked() {
                        if let Some(sel) = self.selected {
                            self.views.insert(sel, TSTransform::IDENTITY);
                        }
                    }
                    ui.separator();
                    if ui.button("Find… (Ctrl+F)").clicked() {
                        self.search_open = !self.search_open;
                        ui.close_menu();
                    }
                });
                ui.menu_button("Tools", |ui| {
                    if ui
                        .add_enabled(self.selected.is_some(), egui::Button::new("Autosort cards"))
                        .on_hover_text("Arrange this basket's cards into a tidy, non-overlapping grid")
                        .clicked()
                    {
                        if let Some(sel) = self.selected {
                            self.push_undo(sel);
                            if self.doc.autosort(sel) {
                                self.mark_dirty();
                                self.status = "Autosorted cards into a grid".to_string();
                            } else {
                                self.undo.pop(); // nothing changed; drop the snapshot
                            }
                        }
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(self.selected.is_some(), egui::Button::new("Snip to card…"))
                        .on_hover_text("Capture a screen region into an image card in this basket")
                        .clicked()
                    {
                        self.start_snip();
                        ui.close_menu();
                    }
                    if ui
                        .button("OCR all images")
                        .on_hover_text("Extract text from every image card that doesn't have it yet")
                        .clicked()
                    {
                        self.ocr_all();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            !self.templates.is_empty(),
                            egui::Button::new("Rebuild Templates basket"),
                        )
                        .on_hover_text(
                            "Give every saved template a master card in the root-level Templates \
                             basket (creating it if needed), so you can see and edit them. Only \
                             stamps the ones that haven't got a master — safe to run twice.",
                        )
                        .clicked()
                    {
                        let (node, made, had) = self.rebuild_templates_node();
                        self.selected = Some(node);
                        self.status = if made == 0 {
                            format!("Templates basket already complete ({had} master cards)")
                        } else {
                            format!("Stamped {made} master card(s) into Templates ({had} already there)")
                        };
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Version history…").clicked() {
                        self.show_history = true;
                        ui.close_menu();
                    }
                    if ui.button("Backup…").clicked() {
                        self.show_backup = true;
                        ui.close_menu();
                    }
                    if ui.button("Plugins…").clicked() {
                        self.show_plugins = true;
                        ui.close_menu();
                    }
                    if ui.button("Requirements…").clicked() {
                        self.show_requirements = true;
                        ui.close_menu();
                    }
                    if ui.button("Settings…").clicked() {
                        self.show_settings = true;
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About Trellis").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    /// Re-probe PATH for every optional tool. Cheap enough to run on opening the
    /// window and on Re-check, far too expensive to run per frame.
    fn scan_requirements(&mut self) {
        let mgr = crate::deps::manager();
        self.req_scan = crate::deps::all()
            .into_iter()
            .map(|d| {
                let present = d.present();
                (
                    d.label.to_string(),
                    d.enables.to_string(),
                    d.url.to_string(),
                    present,
                    d.builtin_here,
                    d.install(mgr),
                )
            })
            .collect();
    }

    /// **Tools → Requirements…** — every optional external tool, whether it's
    /// here, what it buys, and a button that actually gets it.
    ///
    /// Trellis works with none of these installed, so this is a shopping list
    /// rather than a blocker. The point is that "install tesseract-ocr" is not
    /// an instruction a user can follow on Windows: the package name, the
    /// manager, and whether one exists at all are different everywhere, so the
    /// app works it out and either runs the install or hands over the exact
    /// command.
    /// **Tools → Plugins…** — what is installed, what each one is allowed to do,
    /// and the approval that grants it.
    ///
    /// The approval prompt states the scope as a sentence, because that is the
    /// entire basis on which someone decides whether to trust a plugin. A token
    /// is minted behind it and never shown: making people handle a credential per
    /// plugin would be worse than the single shared key this replaces.
    fn plugins_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_plugins;
        let doc_title = self
            .doc_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "document".into());
        let mut to_run: Option<usize> = None;
        let mut to_approve: Option<usize> = None;
        let mut to_revoke: Option<String> = None;
        let mut to_save: Option<usize> = None;
        let mut rescan = false;

        egui::Window::new("Plugins")
            .open(&mut open)
            .default_width(600.0)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.label(
                    "Plugins are separate programs that Trellis runs. They talk to \
                     it over the same API an agent uses — so a plugin that crashes \
                     cannot damage your document.",
                );
                if let Some(d) = crate::plugins::plugins_dir(self.startup_data_dir.as_deref()) {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Installed in:").weak());
                        ui.label(egui::RichText::new(d.display().to_string()).weak().monospace());
                    });
                }
                ui.separator();

                if self.plugins.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "No plugins installed. A plugin is a folder containing a \
                             plugin.json and something to run.",
                        )
                        .weak(),
                    );
                }

                for (i, p) in self.plugins.iter().enumerate() {
                    let approved = self.is_approved(&p.manifest.name);
                    let running = self.plugin_running.contains(&p.manifest.name);
                    ui.horizontal(|ui| {
                        ui.strong(&p.manifest.title);
                        if !p.manifest.version.is_empty() {
                            ui.label(egui::RichText::new(format!("v{}", p.manifest.version)).weak());
                        }
                        if running {
                            ui.spinner();
                            ui.label(egui::RichText::new("running…").weak());
                        }
                    });
                    ui.indent(&p.manifest.name, |ui| {
                        if !p.manifest.description.is_empty() {
                            ui.label(egui::RichText::new(&p.manifest.description).weak());
                        }
                        // The permission, as a sentence.
                        ui.label(
                            egui::RichText::new(format!(
                                "Wants to {}.",
                                p.manifest.scope.describe(&doc_title)
                            ))
                            .color(if p.manifest.scope.read_only {
                                ui.visuals().weak_text_color()
                            } else {
                                egui::Color32::from_rgb(230, 160, 60)
                            }),
                        );
                        let where_from: Vec<String> = p
                            .manifest
                            .triggers
                            .iter()
                            .map(|t| match t {
                                crate::plugins::Trigger::Schedule => {
                                    format!("every {} min", p.manifest.interval_mins.max(1))
                                }
                                crate::plugins::Trigger::OnChange => format!(
                                    "when the document changes (after {}s quiet)",
                                    p.manifest.debounce_secs.max(1)
                                ),
                                other => other.label().to_string(),
                            })
                            .collect();
                        ui.label(
                            egui::RichText::new(format!("Runs from: {}", where_from.join(", ")))
                                .weak()
                                .small(),
                        );
                        // Settings the plugin asked for. Rendered here because a
                        // config file in a directory nobody can find is not a
                        // setting anyone will ever change.
                        if !p.manifest.config.is_empty() {
                            let name = p.manifest.name.clone();
                            let values = self
                                .plugin_config
                                .entry(name.clone())
                                .or_insert_with(|| crate::plugins::read_config(&p.dir));
                            let mut changed = false;
                            egui::Grid::new(("plugcfg", &name))
                                .num_columns(2)
                                .spacing([8.0, 4.0])
                                .show(ui, |ui| {
                                    for f in &p.manifest.config {
                                        ui.label(&f.label).on_hover_text(&f.help);
                                        let v = values.entry(f.key.clone()).or_default();
                                        let w = egui::TextEdit::singleline(v)
                                            .desired_width(300.0)
                                            .password(f.secret)
                                            .hint_text(if f.secret { "paste it here" } else { "" });
                                        if ui.add(w).changed() {
                                            changed = true;
                                        }
                                        ui.end_row();
                                    }
                                });
                            let _ = changed;
                            if ui.button("Save settings").clicked() {
                                to_save = Some(i);
                            }
                            if p.manifest.config.iter().any(|f| f.required)
                                && !p.manifest.config.iter().any(|f| {
                                    f.required
                                        && values.get(&f.key).map(|v| !v.trim().is_empty()).unwrap_or(false)
                                })
                            {
                                ui.label(
                                    egui::RichText::new(
                                        "Needs at least one of the required settings above.",
                                    )
                                    .color(egui::Color32::from_rgb(230, 160, 60))
                                    .small(),
                                );
                            }
                        }
                        ui.horizontal(|ui| {
                            if approved {
                                ui.colored_label(
                                    egui::Color32::from_rgb(80, 190, 110),
                                    "✔ Approved",
                                );
                                if ui
                                    .add_enabled(!running, egui::Button::new("Run now"))
                                    .clicked()
                                {
                                    to_run = Some(i);
                                }
                                if ui
                                    .button("Revoke")
                                    .on_hover_text(
                                        "Delete its token. It stops working immediately.",
                                    )
                                    .clicked()
                                {
                                    to_revoke = Some(p.manifest.name.clone());
                                }
                            } else if ui
                                .button("Approve…")
                                .on_hover_text("Grant exactly the access described above")
                                .clicked()
                            {
                                to_approve = Some(i);
                            }
                        });
                    });
                    ui.add_space(8.0);
                }

                if !self.plugin_errors.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("Could not be loaded:").strong());
                    for e in &self.plugin_errors {
                        ui.label(egui::RichText::new(e).weak().small());
                    }
                }

                ui.separator();
                if self.plugins.iter().any(|p| {
                    p.manifest.triggers.contains(&crate::plugins::Trigger::Schedule)
                        || p.manifest.triggers.contains(&crate::plugins::Trigger::OnChange)
                }) {
                    ui.label(
                        egui::RichText::new(
                            "Scheduled and on-change plugins only run while Trellis is open — \
                             it is a desktop app, not a service.",
                        )
                        .weak()
                        .small(),
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("Rescan").clicked() {
                        rescan = true;
                    }
                });

                if !self.plugin_log.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("Recent runs").strong());
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for r in self.plugin_log.iter().rev() {
                            ui.horizontal(|ui| {
                                if r.ok {
                                    ui.colored_label(egui::Color32::from_rgb(80, 190, 110), "✔");
                                } else {
                                    ui.colored_label(egui::Color32::from_rgb(230, 100, 100), "✘");
                                }
                                ui.label(&r.summary);
                            });
                            if !r.output.trim().is_empty() {
                                ui.collapsing(format!("output — {}", r.plugin), |ui| {
                                    ui.code(&r.output);
                                });
                            }
                        }
                    });
                }
            });

        if let Some(i) = to_approve {
            if let Some(p) = self.plugins.get(i).cloned() {
                let t = self.grant_for(&p);
                self.status = if t.is_empty() {
                    format!("Could not approve {}", p.manifest.title)
                } else {
                    format!("{} approved — it can {}", p.manifest.title, p.manifest.scope.describe(&doc_title))
                };
            }
        }
        if let Some(i) = to_save {
            if let Some(p) = self.plugins.get(i) {
                let name = p.manifest.name.clone();
                let dir = p.dir.clone();
                let vals = self.plugin_config.get(&name).cloned().unwrap_or_default();
                self.status = match crate::plugins::write_config(&dir, &vals) {
                    Ok(()) => format!("Saved settings for {name}"),
                    Err(e) => format!("Could not save {name} settings: {e}"),
                };
            }
        }
        if let Some(name) = to_revoke {
            self.revoke(&name);
            self.status = format!("Revoked {name} — its token no longer works");
        }
        if let Some(i) = to_run {
            self.run_plugin(i, Vec::new());
        }
        if rescan {
            if let Some(d) = crate::plugins::plugins_dir(self.startup_data_dir.as_deref()) {
                let (p, e) = crate::plugins::scan(&d);
                self.plugins = p;
                self.plugin_errors = e;
                self.plugin_config.clear();
            }
        }
        self.show_plugins = open;
    }

    fn requirements_window(&mut self, ctx: &egui::Context) {
        if self.req_scan.is_empty() {
            self.scan_requirements();
        }
        let mgr = crate::deps::manager();
        let mut open = self.show_requirements;
        let mut rescan = false;
        egui::Window::new("Requirements")
            .open(&mut open)
            .default_width(560.0)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.label(
                    "Trellis runs without any of these. Each one switches on a \
                     single extra feature — a missing tool disables that feature \
                     and nothing else.",
                );
                match mgr {
                    crate::deps::Manager::None => {
                        ui.label(
                            egui::RichText::new(
                                "No package manager found, so these link to their download pages.",
                            )
                            .weak(),
                        );
                    }
                    m => {
                        ui.label(
                            egui::RichText::new(format!(
                                "Package manager: {}",
                                match m {
                                    crate::deps::Manager::Winget => "winget",
                                    crate::deps::Manager::Brew => "Homebrew",
                                    crate::deps::Manager::Apt => "apt",
                                    crate::deps::Manager::Dnf => "dnf",
                                    crate::deps::Manager::Pacman => "pacman",
                                    crate::deps::Manager::Zypper => "zypper",
                                    crate::deps::Manager::None => "",
                                }
                            ))
                            .weak(),
                        );
                    }
                }
                ui.separator();

                let mut note: Option<String> = None;
                for (label, enables, url, present, builtin, install) in &self.req_scan {
                    ui.horizontal(|ui| {
                        // Colour *and* a word: a tick alone is unreadable to
                        // anyone who can't separate the two greens.
                        if *present {
                            ui.colored_label(egui::Color32::from_rgb(80, 190, 110), "✔ Installed");
                        } else {
                            ui.colored_label(egui::Color32::from_rgb(230, 160, 60), "✘ Missing");
                        }
                        ui.strong(label);
                    });
                    ui.indent(label, |ui| {
                        ui.label(egui::RichText::new(enables).weak());
                        if !*present && *builtin {
                            // Worth saying out loud: this one normally ships
                            // with the OS, so its absence means it was removed
                            // or switched off rather than never installed —
                            // which points at a different fix.
                            ui.label(
                                egui::RichText::new(
                                    "Normally comes with the system — it may be an optional \
                                     component that isn't switched on.",
                                )
                                .weak()
                                .italics(),
                            );
                        }
                        if !*present {
                            ui.horizontal_wrapped(|ui| match install {
                                crate::deps::Install::Run { label, bin, args } => {
                                    if ui.button(label).clicked() {
                                        note = Some(match crate::deps::run_install(bin, args) {
                                            Ok(()) => format!(
                                                "Installing in a new window. Re-check when it \
                                                 finishes."
                                            ),
                                            Err(e) => e,
                                        });
                                    }
                                    if ui.button("Download page").clicked() {
                                        let _ = crate::deps::open_url(url);
                                    }
                                }
                                crate::deps::Install::Copy { label, cmd } => {
                                    ui.label(format!("{label}:"));
                                    ui.code(cmd);
                                    if ui.button("Copy").clicked() {
                                        ui.output_mut(|o| o.copied_text = cmd.clone());
                                        note = Some("Command copied to the clipboard".into());
                                    }
                                    if ui.button("Download page").clicked() {
                                        let _ = crate::deps::open_url(url);
                                    }
                                }
                                crate::deps::Install::Link => {
                                    if ui.button("Download page").clicked() {
                                        let _ = crate::deps::open_url(url);
                                    }
                                }
                            });
                        }
                    });
                    ui.add_space(6.0);
                }

                if let Some(n) = note {
                    self.req_note = n;
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .button("Re-check")
                        .on_hover_text("Probe PATH again — use this after installing something")
                        .clicked()
                    {
                        rescan = true;
                        self.req_note.clear();
                    }
                    if !self.req_note.is_empty() {
                        ui.label(egui::RichText::new(&self.req_note).weak());
                    }
                });
            });
        if rescan {
            self.scan_requirements();
        }
        self.show_requirements = open;
        if !open {
            // Drop the cache so reopening re-probes — the user has very likely
            // been off installing something in between.
            self.req_scan.clear();
            self.req_note.clear();
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.heading("Agent API");
                ui.label(
                    "An HTTP API for agents (and the Trellis mobile app) to add, query, \
                     edit and remove nodes and cards. Localhost-only by default; enable \
                     LAN access below to reach it from other devices on your network.",
                );
                ui.add_space(6.0);
                ui.label(egui::RichText::new(&self.api_status).weak());
                ui.add_space(6.0);

                egui::Grid::new("api_settings").num_columns(2).spacing([8.0, 8.0]).show(ui, |ui| {
                    ui.label("API key");
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.api_key)
                                    .desired_width(240.0)
                                    .hint_text("empty = API disabled"),
                            )
                            .changed()
                        {
                            self.sync_api_key();
                        }
                        if ui.button("Generate").clicked() {
                            self.api_key = generate_key();
                            self.sync_api_key();
                        }
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(self.api_key.clone());
                        }
                    });
                    ui.end_row();

                    ui.label("Files agents may mirror");
                    ui.vertical(|ui| {
                        use crate::model::MirrorPolicy as MP;
                        let mut p = self.mirror_policy;
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut p, MP::SafeDefault, "Anywhere but credentials");
                            ui.selectable_value(&mut p, MP::OnlyDirs, "Only these folders");
                            ui.selectable_value(&mut p, MP::Anywhere, "Anywhere");
                        });
                        if p != self.mirror_policy {
                            self.mirror_policy = p;
                        }
                        if self.mirror_policy == MP::OnlyDirs {
                            let mut text = self.mirror_dirs.join("\n");
                            if ui
                                .add(
                                    egui::TextEdit::multiline(&mut text)
                                        .desired_rows(3)
                                        .desired_width(320.0)
                                        .hint_text("/home/you/projects\n/srv/docs"),
                                )
                                .changed()
                            {
                                self.mirror_dirs = text
                                    .lines()
                                    .map(|l| l.trim().to_string())
                                    .filter(|l| !l.is_empty())
                                    .collect();
                            }
                        }
                        ui.label(
                            egui::RichText::new(
                                "Only limits the API. Your own File → Mirror a file… is never \
                                 restricted. Without a limit, anything holding the API key can \
                                 point a card at a file and read it back.",
                            )
                            .weak()
                            .small(),
                        );
                    });
                    ui.end_row();

                    ui.label("Port");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.api_port).range(1024..=65535));
                        ui.weak("(restart to apply — or launch with --port)")
                            .on_hover_text(
                                "One instance serves one document, so the port is how an agent \
                                 addresses this document. Launch with --port to pin it, and \
                                 --data-dir to give an instance its own key/port/settings so \
                                 several can run at once. GET /api/instance reports which \
                                 document a port is serving.",
                            );
                    });
                    ui.end_row();

                    ui.label("LAN access");
                    if ui
                        .checkbox(&mut self.api_lan, "Allow other devices on my network")
                        .on_hover_text(
                            "Binds the API to all network interfaces (0.0.0.0) so the \
                             mobile app and other devices can reach it. Still requires \
                             the API key. Only enable on trusted networks — never expose \
                             to the internet without a TLS proxy. Takes effect immediately.",
                        )
                        .changed()
                    {
                        self.restart_api(ctx);
                    }
                    ui.end_row();
                });

                ui.add_space(10.0);
                ui.heading("Document");
                ui.checkbox(&mut self.autosave, "Autosave changes")
                    .on_hover_text(
                        "Save changes to disk automatically a couple of seconds after \
                         you stop editing (like Google Docs). Written atomically. When \
                         off, save manually with Ctrl+S; changes are still saved on exit.",
                    );

                ui.add_space(10.0);
                ui.heading("Version history");
                ui.small(
                    egui::RichText::new(
                        "Each snapshot is a complete copy of the document, so these settings \
                         trade disk space and save time against how far back you can go. On a \
                         large document (lots of images) keep fewer, spaced further apart.",
                    )
                    .weak(),
                );
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.history_keep)
                            .range(HISTORY_KEEP_RANGE)
                            .speed(0.25),
                    );
                    ui.label("snapshots kept");
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.history_gap_mins)
                            .range(HISTORY_GAP_MINS_RANGE)
                            .speed(0.5),
                    );
                    ui.label("minutes between snapshots")
                        .on_hover_text(
                            "A burst of edits saves repeatedly; without a gap that would churn \
                             through the whole history in a minute.",
                        );
                });
                // Concrete numbers beat abstract settings: show what this costs
                // for THIS document, using the size it actually is on disk.
                if let Some(sz) = self
                    .doc_path
                    .as_ref()
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                {
                    ui.small(
                        egui::RichText::new(format!(
                            "This document is {:.1} MB, so history can reach about {:.1} MB.",
                            sz as f64 / 1e6,
                            (sz as f64 / 1e6) * self.history_keep as f64
                        ))
                        .weak(),
                    );
                }
                ui.small(
                    egui::RichText::new(
                        "No snapshot is taken when you close the app — the document is saved \
                         either way, and skipping it keeps closing quick on a big document.",
                    )
                    .weak(),
                );

                ui.add_space(10.0);
                ui.heading("Canvas");
                ui.checkbox(
                    &mut self.zoom_enabled,
                    "Zoom with Ctrl+scroll and Ctrl +/−",
                )
                .on_hover_text("Ctrl+0 and Reset view still reset zoom when this is off.");
                ui.checkbox(&mut self.minimap_enabled, "Minimap (overview + view reticle, bottom-right)")
                    .on_hover_text(
                        "A small map of the whole basket in the canvas corner, with a box showing \
                         your current view. Click or drag on it to jump the view. Spot cards that \
                         sit outside the main cluster without zooming out.",
                    );
                ui.checkbox(&mut self.dock_mode, "Dock mode (drag a card onto another to stick it)")
                    .on_hover_text(
                        "When on, dropping a card on another docks them so they move together; \
                         drag a docked card off to detach. Grouping works regardless.",
                    );
                ui.checkbox(&mut self.snap_mode, "Snap mode (align card edges while dragging)")
                    .on_hover_text("When on, a dragged card's edges snap to nearby cards' edges.");

                ui.add_space(8.0);
                ui.separator();
                ui.label("Authenticate with a header, then call the endpoints:");
                ui.add_space(4.0);
                let port = self.api_port;
                let host = if self.api_lan {
                    local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
                } else {
                    "127.0.0.1".to_string()
                };
                ui.code(format!(
                    "curl -H 'X-API-Key: {}' \\\n     http://{}:{}/api/tree",
                    if self.api_key.is_empty() { "<key>" } else { &self.api_key },
                    host,
                    port
                ));
                ui.add_space(4.0);
                // Copy-paste starters with this instance's real host/port/key —
                // the endpoint list says what exists, these say how to drive it.
                ui.collapsing("Examples", |ui| {
                    let k = if self.api_key.is_empty() { "<key>" } else { &self.api_key };
                    let a = format!("http://{host}:{port}/api");
                    for (what, cmd) in [
                        (
                            "Which document is this?",
                            format!("curl -H 'X-API-Key: {k}' {a}/instance"),
                        ),
                        (
                            "The tree, then one basket, then one card",
                            format!(
                                "curl -H 'X-API-Key: {k}' {a}/tree\n\
                                 curl -H 'X-API-Key: {k}' {a}/nodes/1\n\
                                 curl -H 'X-API-Key: {k}' {a}/nodes/1/cards/1"
                            ),
                        ),
                        (
                            "Add a card (fit sizes it to its content)",
                            format!(
                                "curl -H 'X-API-Key: {k}' -d '{{\"kind\":\"text\",\"title\":\"Note\",\
                                 \"body\":\"Hello\",\"fit\":true}}' \\\n     {a}/nodes/1/cards"
                            ),
                        ),
                        (
                            "A task — lands in the Agenda and Kanban",
                            format!(
                                "curl -H 'X-API-Key: {k}' -d '{{\"kind\":\"text\",\"title\":\"Ship it\",\
                                 \"body\":\"due:: 2026-08-15\\nstatus:: todo\",\"fit\":true}}' \\\n     {a}/nodes/1/cards"
                            ),
                        ),
                        (
                            "A table, populated in one call",
                            format!(
                                "curl -H 'X-API-Key: {k}' -d '{{\"kind\":\"table\",\"title\":\"Revenue\",\
                                 \"rows\":[[\"Quarter\",\"Revenue\"],[\"Q1\",\"1200\"],[\"Q2\",\"1850\"]]}}' \\\n     {a}/nodes/1/cards"
                            ),
                        ),
                        (
                            "Make that table readable (columns are 110px and don't wrap)",
                            format!(
                                "curl -H 'X-API-Key: {k}' -d '{{\"op\":\"autofit_cols\"}}' {a}/nodes/1/cards/1/table\n\
                                 curl -X PATCH -H 'X-API-Key: {k}' -d '{{\"fit\":true}}' {a}/nodes/1/cards/1   # then the frame"
                            ),
                        ),
                        (
                            "Chart that table (bar | line | scatter | pie)",
                            format!(
                                "curl -H 'X-API-Key: {k}' -d '{{\"kind\":\"bar\"}}' {a}/nodes/1/cards/1/chart\n\
                                 curl -X DELETE -H 'X-API-Key: {k}' {a}/nodes/1/cards/1/chart   # back to a grid"
                            ),
                        ),
                        (
                            "Just this project's tasks",
                            format!("curl -H 'X-API-Key: {k}' '{a}/tasks?project=1'"),
                        ),
                        (
                            "Wake the moment anything changes",
                            format!("curl -H 'X-API-Key: {k}' '{a}/wait?rev=0'"),
                        ),
                        (
                            "…then ask what actually changed (re-read only that)",
                            format!("curl -H 'X-API-Key: {k}' '{a}/changes?since=0'"),
                        ),
                        (
                            "Mirror a file in a card (read-only, tracks the file)",
                            format!(
                                "curl -X POST -H 'X-API-Key: {k}' -H 'Content-Type: application/json' \\\n  \
                                 -d '{{\"kind\":\"text\",\"title\":\"README\",\
                                 \"source\":\"/srv/app/README.md\",\"fit\":true}}' \\\n  \
                                 {a}/nodes/1/cards\n\
                                 # detach later (keeps the text):\n\
                                 curl -X PATCH -H 'X-API-Key: {k}' -H 'Content-Type: application/json' \\\n  \
                                 -d '{{\"source\":\"\"}}' {a}/nodes/1/cards/2"
                            ),
                        ),
                    ] {
                        ui.small(egui::RichText::new(what).strong());
                        ui.code(cmd);
                        ui.add_space(4.0);
                    }
                    ui.small(
                        egui::RichText::new("Node/card ids above are placeholders — right-click a node or card → Copy → id.")
                            .weak(),
                    );
                });
                ui.add_space(4.0);
                ui.collapsing("Endpoints", |ui| {
                    for line in [
                        "GET    /api/health                        (no auth)",
                        "GET    /api/instance   → which document this port serves",
                        "GET    /api/tree",
                        "GET    /api/nodes",
                        "POST   /api/nodes               {parent?, title}",
                        "GET    /api/nodes/{id}",
                        "PATCH  /api/nodes/{id}          {title?, color?, bg?}",
                        "DELETE /api/nodes/{id}",
                        "POST   /api/nodes/{id}/move     {before|after|index|to, parent?}",
                        "POST   /api/nodes/{id}/expand   {expanded, recursive?}",
                        "GET    /api/nodes/{id}/backlinks          (cards that [[link]] here)",
                        "GET    /api/graph                         (wiki-link nodes + edges)",
                        "GET    /api/nodes/{id}/cards",
                        "GET    /api/nodes/{id}/cards/{cid}        (one card, without the whole basket)",
                        "POST   /api/nodes/{id}/cards    {kind, title?, body?, lang?, items?, rows?, header?, pos?, size?, fit?, image_base64?, inline_images?, source?}",
                        "PATCH  /api/nodes/{id}/cards/{cid}       {title?, body?, kind?, color?, font_scale?, fit?, pos?, size?, items?, source?, …}",
                        "         source: mirror a file (body read-only, PATCH body → 409); source:\"\" detaches",
                        "DELETE /api/nodes/{id}/cards/{cid}",
                        "POST   /api/nodes/{id}/cards/{cid}/move  {before|after|index|to} (or {node,pos?} → another basket)",
                        "POST   /api/nodes/{id}/cards/{cid}/property {key, value}   (set key:: value)",
                        "POST   /api/nodes/{id}/cards/{cid}/dock  {anchor}          (unstick: DELETE …/dock)",
                        "POST   /api/nodes/{id}/cards/{cid}/group {group}           (remove: DELETE …/group)",
                        "POST   /api/nodes/{id}/cards/{cid}/table {op, …}           (set_cell / insert_row / set_col_width / autofit_cols {col?} …)",
                        "POST   /api/nodes/{id}/cards/{cid}/chart {kind, label_col?, value_cols?, show_table?}  (bar|line|scatter|pie; DELETE …/chart = plain grid)",
                        "POST   /api/nodes/{id}/cards/{cid}/sketch {op, …}          (add_stroke / undo / clear)",
                        "POST   /api/nodes/{id}/cards/{cid}/images {data_base64}    (GET / DELETE …/images/{idx})",
                        "GET    /api/nodes/{id}/groups             (POST create {cards,title?} / PATCH / DELETE {gid})",
                        "POST   /api/nodes/{id}/autosort",
                        "GET    /api/search?q=...                  (hits carry node + card)",
                        "GET    /api/tags[?name=<tag>]             (all tags / cards with a tag)",
                        "GET    /api/properties[?key=<k>&value=<v>]   (keys / matching cards)",
                        "GET    /api/query?tag=&key=&value=&text=  (combined card query)",
                        "GET    /api/tasks[?all=true][&project=<id>]  (due:: agenda, bucketed)",
                        "GET    /api/kanban[?project=<id>]         (cards grouped by status:: → columns)",
                        "POST   /api/ocr                           (OCR all un-OCR'd images)",
                        "GET    /api/export?format=markdown|html|json|pdf|png|gif",
                        "GET    /api/wait?rev=<n>                  (long-poll: that something changed, + epoch)",
                        "GET    /api/changes?since=<seq>[&limit=<n>]  (what changed: actor/entity/op/fields/property)",
                        "GET    /api/history                       (version snapshots + keep / min_gap_mins retention)",
                        "POST   /api/history/restore     {file}    (restore a snapshot)",
                        "GET    /api/backup                        (status)",
                        "POST   /api/backup/run                    (back up now)",
                        "GET    /api/templates                     (saved card templates)",
                        "POST   /api/templates          {node, card, title?}   (save a card as a template + master)",
                        "POST   /api/templates/{i}/insert {node, pos?}         (stamp it into a basket)",
                        "POST   /api/templates/{i}/update {node, card, title?} (re-snapshot in place from a card)",
                        "DELETE /api/templates/{i}                             (also deletes its master card)",
                        "POST   /api/templates/rebuild                         (give every template a master card)",
                        "",
                        "Full reference: API.md in the source repo.",
                    ] {
                        ui.monospace(line);
                    }
                });
            });
        self.show_settings = open;
    }

    fn backup_window(&mut self, ctx: &egui::Context) {
        use crate::backup::{BackupDest, DestKind};
        let mut open = self.show_backup;
        let mut do_backup = false;
        egui::Window::new("Backup")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(
                    "Full copies of your document to external locations — this is backup, \
                     not version history. Each run writes a complete, self-contained file \
                     (the same compressed format Trellis saves), optionally encrypted.",
                );
                ui.add_space(8.0);

                let cfg = &mut self.backup_cfg;
                ui.checkbox(&mut cfg.enabled, "Run scheduled backups automatically");
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        cfg.enabled,
                        egui::DragValue::new(&mut cfg.interval_mins).range(0..=100_000).suffix(" min"),
                    );
                    ui.label("between backups (0 = manual only)");
                });
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut cfg.retention).range(0..=10_000));
                    ui.label("keep newest N per disk destination (0 = keep all)");
                });

                ui.add_space(6.0);
                ui.checkbox(&mut cfg.encrypt, "Encrypt backups (gpg symmetric, AES-256)");
                if cfg.encrypt {
                    ui.horizontal(|ui| {
                        ui.label("Passphrase");
                        ui.add(
                            egui::TextEdit::singleline(&mut cfg.passphrase)
                                .password(true)
                                .desired_width(260.0)
                                .hint_text("required to encrypt / restore"),
                        );
                    });
                    ui.label(
                        egui::RichText::new(
                            "Restore with:  gpg -d file.ron.gz.gpg > file.ron.gz  — keep this passphrase safe; \
                             without it the backup can't be read.",
                        )
                        .weak()
                        .small(),
                    );
                }

                ui.add_space(10.0);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("Destinations");
                    ui.menu_button("+ Add", |ui| {
                        for kind in [DestKind::Disk, DestKind::Sftp, DestKind::Rclone] {
                            if ui.button(kind.label()).clicked() {
                                cfg.destinations.push(BackupDest::new(kind));
                                ui.close_menu();
                            }
                        }
                    });
                });

                let mut remove: Option<usize> = None;
                for (i, d) in cfg.destinations.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut d.enabled, "");
                                ui.strong(d.kind.label());
                                ui.add(
                                    egui::TextEdit::singleline(&mut d.name)
                                        .desired_width(120.0)
                                        .hint_text("label (optional)"),
                                );
                                if ui.button("×").on_hover_text("Remove this destination").clicked() {
                                    remove = Some(i);
                                }
                            });
                            let (label, hint) = match d.kind {
                                DestKind::Disk => ("Directory", "/mnt/usb/trellis-backups  (a local or mounted folder)"),
                                DestKind::Sftp => ("SSH target", "user@host:/backups/trellis  (uses your SSH keys via scp)"),
                                DestKind::Rclone => ("Rclone remote", "gdrive:trellis-backups  (configure the remote with `rclone config`)"),
                            };
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.add(
                                    egui::TextEdit::singleline(&mut d.target)
                                        .desired_width(340.0)
                                        .hint_text(hint),
                                );
                            });
                        });
                    });
                }
                if let Some(i) = remove {
                    cfg.destinations.remove(i);
                }
                if cfg.destinations.is_empty() {
                    ui.weak("No destinations yet — add a Disk, Network (SFTP), or Cloud (rclone) target.");
                }

                ui.add_space(10.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.backing_up, egui::Button::new("Back up now"))
                        .clicked()
                    {
                        do_backup = true;
                    }
                    if self.backing_up {
                        ui.spinner();
                        ui.label("Backing up…");
                    }
                });
                if let Some(t) = self.last_backup {
                    ui.weak(format!("Last run: {}s ago", t.elapsed().as_secs()));
                }
                if !self.backup_status.is_empty() {
                    ui.label(&self.backup_status);
                }
            });
        self.show_backup = open;
        if do_backup {
            self.start_backup(true);
        }
    }

    fn history_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_history;
        let path = self.target_path();
        let snaps = history_snapshots(&path);
        let mut restore: Option<PathBuf> = None;
        egui::Window::new("Version history")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(
                    "Automatic snapshots of this document, taken as you save (kept up to \
                     25, at least a few minutes apart). Restoring loads an older version \
                     as the current document — save to keep it.",
                );
                ui.add_space(6.0);
                if snaps.is_empty() {
                    ui.weak("No snapshots yet — they start accumulating after you save.");
                }
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, false]).show(ui, |ui| {
                    for (p, name) in &snaps {
                        ui.horizontal(|ui| {
                            let size = p.metadata().map(|m| m.len()).unwrap_or(0);
                            ui.label(format_stamp(name));
                            ui.weak(format!("{} KB", size / 1024));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Restore").clicked() {
                                    restore = Some(p.clone());
                                }
                            });
                        });
                        ui.separator();
                    }
                });
            });
        self.show_history = open;
        if let Some(p) = restore {
            if self.confirm_discard() {
                match read_document(&p) {
                    Ok(doc) => {
                        self.reset_inline_images();
                        self.doc = doc;
                        self.selected = self.doc.roots.first().copied();
                        self.views.clear();
                        self.reset_history();
                        self.mark_dirty(); // restored content isn't saved until the user saves
                        self.show_history = false;
                        self.status = format!("Restored snapshot {}", format_stamp(&p.file_name().unwrap_or_default().to_string_lossy()));
                    }
                    Err(e) => self.status = format!("Restore failed: {e}"),
                }
            }
        }
    }

    fn sync_api_key(&mut self) {
        if let Ok(mut k) = self.api_shared_key.lock() {
            *k = self.api_key.clone();
        }
    }

    fn search_panel(&mut self, ctx: &egui::Context) {
        let mut jump: Option<(NodeId, Option<CardId>)> = None;
        egui::SidePanel::right("search")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Search");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("×").clicked() {
                            self.search_open = false;
                        }
                    });
                });
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("find text…")
                        .desired_width(f32::INFINITY),
                );
                resp.request_focus();
                ui.separator();
                let hits = self.doc.search(&self.search_query);
                if self.search_query.is_empty() {
                    ui.weak("Type to search titles and card contents.");
                } else {
                    ui.weak(format!("{} match(es)", hits.len()));
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for hit in hits {
                        let frame = egui::Frame::none()
                            .inner_margin(egui::Margin::symmetric(6.0, 4.0));
                        frame.show(ui, |ui| {
                            if ui
                                .add(egui::Label::new(egui::RichText::new(&hit.node_title).strong())
                                    .sense(egui::Sense::click()))
                                .clicked()
                            {
                                jump = Some((hit.node, hit.card));
                            }
                            ui.small(hit.snippet);
                        });
                        ui.separator();
                    }
                });
            });
        if let Some((node, card)) = jump {
            self.reveal_hit(ctx, node, card);
        }
    }

    /// Ctrl+O: a centered palette to fuzzy-jump to any node by title or path.
    fn quick_switcher(&mut self, ctx: &egui::Context) {
        let q = self.switcher_query.to_lowercase();
        // (id, title, path, score) for every node that matches; best score first.
        let mut matches: Vec<(NodeId, String, String, i32)> = Vec::new();
        for (&id, n) in &self.doc.nodes {
            let path = crate::tree::node_path(&self.doc, id);
            let title_lc = n.title.to_lowercase();
            let hay = format!("{}\n{}", title_lc, path.to_lowercase());
            if let Some(score) = fuzzy_score(&q, &title_lc, &hay) {
                matches.push((id, n.title.clone(), path, score));
            }
        }
        matches.sort_by(|a, b| a.3.cmp(&b.3).then(a.2.len().cmp(&b.2.len())).then(a.1.cmp(&b.1)));
        matches.truncate(50);
        if matches.is_empty() {
            self.switcher_index = 0;
        } else if self.switcher_index >= matches.len() {
            self.switcher_index = matches.len() - 1;
        }

        // Read nav keys before the text field swallows them.
        let (down, up, enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if esc {
            self.switcher_open = false;
            return;
        }
        if down && !matches.is_empty() {
            self.switcher_index = (self.switcher_index + 1).min(matches.len() - 1);
        }
        if up {
            self.switcher_index = self.switcher_index.saturating_sub(1);
        }
        let mut jump: Option<NodeId> = None;
        if enter {
            if let Some(m) = matches.get(self.switcher_index) {
                jump = Some(m.0);
            }
        }

        let idx = self.switcher_index;
        egui::Window::new("Go to node")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .fixed_size(egui::vec2(480.0, 0.0))
            .show(ctx, |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.switcher_query)
                        .hint_text("Jump to a node…  ↑↓ move · Enter open · Esc close")
                        .desired_width(f32::INFINITY),
                );
                resp.request_focus();
                ui.separator();
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, false]).show(ui, |ui| {
                    for (i, (id, title, path, _)) in matches.iter().enumerate() {
                        let sel = i == idx;
                        let shown = if title.trim().is_empty() { "(untitled)".to_string() } else { title.clone() };
                        let r = ui.add(egui::SelectableLabel::new(sel, egui::RichText::new(shown).strong()));
                        ui.small(egui::RichText::new(path).weak());
                        if r.clicked() {
                            jump = Some(*id);
                        }
                        if sel {
                            r.scroll_to_me(Some(egui::Align::Center));
                        }
                    }
                    if matches.is_empty() && !self.switcher_query.is_empty() {
                        ui.weak("No matching nodes.");
                    }
                });
            });

        if let Some(id) = jump {
            self.jump_to_node(id);
        }
    }

    /// Right-side panel: browse every #tag, then the cards carrying one.
    fn tags_panel(&mut self, ctx: &egui::Context) {
        let mut jump: Option<(NodeId, Option<CardId>)> = None;
        egui::SidePanel::right("tags").resizable(true).default_width(260.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Tags");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("×").clicked() {
                        self.tags_open = false;
                    }
                });
            });
            ui.separator();
            match self.tag_selected.clone() {
                None => {
                    let tags = self.doc.tag_counts();
                    if tags.is_empty() {
                        ui.weak("No #tags yet. Write #tags in any card to group them across baskets.");
                    } else {
                        ui.weak(format!("{} tag(s)", tags.len()));
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            for (tag, count) in tags {
                                if ui
                                    .add(egui::Label::new(format!("#{tag}  ({count})")).sense(egui::Sense::click()))
                                    .clicked()
                                {
                                    self.tag_selected = Some(tag);
                                }
                            }
                        });
                    }
                }
                Some(tag) => {
                    if ui.button("← all tags").clicked() {
                        self.tag_selected = None;
                    }
                    ui.label(egui::RichText::new(format!("#{tag}")).strong());
                    let hits = self.doc.cards_with_tag(&tag);
                    ui.weak(format!("{} card(s)", hits.len()));
                    ui.separator();
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for hit in hits {
                            if ui
                                .add(egui::Label::new(egui::RichText::new(&hit.node_title).strong())
                                    .sense(egui::Sense::click()))
                                .clicked()
                            {
                                jump = Some((hit.node, hit.card));
                            }
                            ui.small(hit.snippet);
                            ui.separator();
                        }
                    });
                }
            }
        });
        if let Some((node, card)) = jump {
            self.reveal_hit(ctx, node, card);
        }
    }

    /// Right-side panel: filter cards across the whole tree by tag / property /
    /// text (all dropdown-driven, no syntax), with click-to-jump results.
    fn find_panel(&mut self, ctx: &egui::Context) {
        let mut jump: Option<(NodeId, Option<CardId>)> = None;
        let tags = self.doc.tag_counts();
        let keys = self.doc.property_keys();
        egui::SidePanel::right("find").resizable(true).default_width(280.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Find cards");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("×").clicked() {
                        self.find_open = false;
                    }
                });
            });
            ui.separator();

            egui::ComboBox::from_label("Tag")
                .selected_text(self.find_tag.clone().map(|t| format!("#{t}")).unwrap_or_else(|| "(any)".into()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.find_tag, None, "(any)");
                    for (t, c) in &tags {
                        ui.selectable_value(&mut self.find_tag, Some(t.clone()), format!("#{t} ({c})"));
                    }
                });
            egui::ComboBox::from_label("Property")
                .selected_text(self.find_key.clone().unwrap_or_else(|| "(any)".into()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.find_key, None, "(any)");
                    for (k, c) in &keys {
                        ui.selectable_value(&mut self.find_key, Some(k.clone()), format!("{k} ({c})"));
                    }
                });
            if self.find_key.is_some() {
                ui.horizontal(|ui| {
                    ui.label("= ");
                    ui.add(egui::TextEdit::singleline(&mut self.find_value).hint_text("any value").desired_width(150.0));
                });
            }
            ui.horizontal(|ui| {
                ui.label("Text");
                ui.add(egui::TextEdit::singleline(&mut self.find_text).hint_text("contains…").desired_width(150.0));
            });
            if ui.button("Clear filters").clicked() {
                self.find_tag = None;
                self.find_key = None;
                self.find_value.clear();
                self.find_text.clear();
            }
            ui.separator();

            let val = self.find_value.trim();
            let txt = self.find_text.trim();
            let hits = self.doc.query_cards(
                self.find_tag.as_deref(),
                self.find_key.as_deref(),
                if val.is_empty() { None } else { Some(val) },
                if txt.is_empty() { None } else { Some(txt) },
            );
            if self.find_tag.is_none() && self.find_key.is_none() && txt.is_empty() {
                ui.weak("Pick a tag or property, or type text, to list matching cards.");
            } else {
                ui.weak(format!("{} match(es)", hits.len()));
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for hit in hits {
                    if ui
                        .add(egui::Label::new(egui::RichText::new(&hit.node_title).strong()).sense(egui::Sense::click()))
                        .clicked()
                    {
                        jump = Some((hit.node, hit.card));
                    }
                    ui.small(hit.snippet);
                    ui.separator();
                }
            });
        });
        if let Some((node, card)) = jump {
            self.reveal_hit(ctx, node, card);
        }
    }

    /// Right-side panel: every open task (a card with a `due::` date) across the
    /// tree, grouped by when it's due. Click a task to jump to its basket.
    fn agenda_panel(&mut self, ctx: &egui::Context) {
        let today = crate::api::today_days();
        let mut tasks = self.doc.tasks();
        let mut jump: Option<(NodeId, CardId)> = None;
        egui::SidePanel::right("agenda").resizable(true).default_width(320.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Agenda");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("×").clicked() {
                        self.agenda_open = false;
                    }
                });
            });
            ui.checkbox(&mut self.agenda_show_done, "Show completed");

            // Filter to one project. Projects are the top-level nodes, so this
            // is "whose tasks am I looking at" — the thing a bare due-date list
            // can't tell you.
            let mut projects: Vec<(NodeId, String)> = Vec::new();
            for t in &tasks {
                if !projects.iter().any(|(id, _)| *id == t.root) {
                    projects.push((t.root, t.root_title.clone()));
                }
            }
            // Keep the tree's own order rather than first-seen, so the menu reads
            // the same way the left panel does.
            projects.sort_by_key(|(id, _)| {
                self.doc.roots.iter().position(|r| r == id).unwrap_or(usize::MAX)
            });
            // A project that no longer exists (deleted, or a different document)
            // must not silently hide every task.
            if self.agenda_project.is_some_and(|p| !projects.iter().any(|(id, _)| *id == p)) {
                self.agenda_project = None;
            }
            let current = self
                .agenda_project
                .and_then(|p| projects.iter().find(|(id, _)| *id == p))
                .map(|(_, t)| t.clone())
                .unwrap_or_else(|| "All projects".to_string());
            ui.horizontal(|ui| {
                ui.label("Project");
                egui::ComboBox::from_id_salt("agenda_project")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.agenda_project.is_none(), "All projects")
                            .clicked()
                        {
                            self.agenda_project = None;
                        }
                        for (id, title) in &projects {
                            let on = self.agenda_project == Some(*id);
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(10.0, 10.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(
                                    rect.center(),
                                    4.0,
                                    project_color(&self.doc, *id),
                                );
                                if ui.selectable_label(on, title).clicked() {
                                    self.agenda_project = Some(*id);
                                }
                            });
                        }
                    });
                if self.agenda_project.is_some() && ui.small_button("×").on_hover_text("Show every project").clicked() {
                    self.agenda_project = None;
                }
            });
            ui.separator();
            tasks.retain(|t| self.agenda_show_done || !t.done);
            if let Some(p) = self.agenda_project {
                tasks.retain(|t| t.root == p);
            }
            tasks.sort_by_key(|t| t.due_days.unwrap_or(i64::MAX));
            if tasks.is_empty() {
                ui.weak("No tasks yet. Add `due:: 2026-08-15` to any card to see it here.");
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (label, key) in [
                    ("Overdue", "overdue"),
                    ("Today", "today"),
                    ("This week", "week"),
                    ("Later", "later"),
                    ("No date", "nodate"),
                ] {
                    let group: Vec<&crate::model::TaskItem> = tasks
                        .iter()
                        .filter(|t| crate::api::task_bucket(t.due_days, today) == key)
                        .collect();
                    if group.is_empty() {
                        continue;
                    }
                    let color = match key {
                        "overdue" => egui::Color32::from_rgb(220, 90, 90),
                        "today" => egui::Color32::from_rgb(230, 170, 60),
                        _ => ui.visuals().weak_text_color(),
                    };
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(label).strong().color(color));
                    for t in group {
                        let title = if t.done {
                            egui::RichText::new(&t.title).strikethrough().weak()
                        } else {
                            egui::RichText::new(&t.title)
                        };
                        let pcolor = project_color(&self.doc, t.root);
                        let row = ui.horizontal(|ui| {
                            // A dot in the project's colour, so a glance down the
                            // list groups by project without reading a word.
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 4.0, pcolor);
                            ui.add(
                                egui::Label::new(format!("{}  ", t.due))
                                    .sense(egui::Sense::click()),
                            )
                        });
                        if ui.add(egui::Label::new(title).sense(egui::Sense::click())).clicked()
                            || row.inner.clicked()
                        {
                            jump = Some((t.node, t.card));
                        }
                        // Full breadcrumb, not just the parent: "Open Items"
                        // exists under more than one project, and the bare name
                        // has had agents attribute a task to the wrong one. The
                        // project half carries its colour so it reads at a glance.
                        let mut job = egui::text::LayoutJob::default();
                        let small = egui::TextStyle::Small.resolve(ui.style());
                        let (proj, rest) = match t.node_path.split_once(" › ") {
                            Some((a, b)) => (a.to_string(), format!(" › {b}")),
                            None => (t.node_path.clone(), String::new()),
                        };
                        job.append(
                            &proj,
                            0.0,
                            egui::TextFormat { font_id: small.clone(), color: pcolor, ..Default::default() },
                        );
                        if !rest.is_empty() {
                            job.append(
                                &rest,
                                0.0,
                                egui::TextFormat {
                                    font_id: small,
                                    color: ui.visuals().weak_text_color(),
                                    ..Default::default()
                                },
                            );
                        }
                        ui.label(job);
                    }
                }
            });
        });
        if let Some((node, card)) = jump {
            self.jump_to_card(ctx, node, card);
        }
    }

    /// Right-side panel: cards elsewhere that `[[link]]` to the selected node.
    fn backlinks_panel(&mut self, ctx: &egui::Context) {
        let mut jump: Option<(NodeId, Option<CardId>)> = None;
        egui::SidePanel::right("backlinks").resizable(true).default_width(260.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Backlinks");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("×").clicked() {
                        self.backlinks_open = false;
                    }
                });
            });
            ui.separator();
            match self.selected {
                None => {
                    ui.weak("Select a node to see what links to it.");
                }
                Some(sel) => {
                    let title = self.doc.nodes.get(&sel).map(|n| n.title.clone()).unwrap_or_default();
                    ui.label(egui::RichText::new(format!("Linked to: {title}")).strong());
                    ui.small(egui::RichText::new(format!("Use [[{title}]] in a card to link here.")).weak());
                    ui.separator();
                    let hits = self.doc.backlinks(sel);
                    if hits.is_empty() {
                        ui.weak("Nothing links here yet.");
                    } else {
                        ui.weak(format!("{} card(s) link here", hits.len()));
                    }
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for hit in hits {
                            if ui
                                .add(egui::Label::new(egui::RichText::new(&hit.node_title).strong()).sense(egui::Sense::click()))
                                .clicked()
                            {
                                jump = Some((hit.node, hit.card));
                            }
                            ui.small(hit.snippet);
                            ui.separator();
                        }
                    });
                }
            }
        });
        if let Some((node, card)) = jump {
            self.reveal_hit(ctx, node, card);
        }
    }

    /// Compute a force-directed layout of the wiki-link graph (once, when the
    /// window opens). Nodes start on a circle, then repel each other while edges
    /// pull linked nodes together — a few hundred cheap iterations settle it.
    fn build_graph(&mut self) {
        let (ids, edges) = self.doc.link_graph();
        self.graph_edges = edges;
        self.graph_layout.clear();
        let n = ids.len();
        if n == 0 {
            self.graph_built = true;
            return;
        }
        for (i, &id) in ids.iter().enumerate() {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            self.graph_layout.insert(id, egui::pos2(a.cos() * 200.0, a.sin() * 200.0));
        }
        let k = (250.0 / (n as f32).sqrt()).max(30.0); // ideal separation
        for _ in 0..300 {
            let mut disp: HashMap<NodeId, egui::Vec2> =
                ids.iter().map(|&id| (id, egui::Vec2::ZERO)).collect();
            for i in 0..ids.len() {
                for j in (i + 1)..ids.len() {
                    let mut d = self.graph_layout[&ids[i]] - self.graph_layout[&ids[j]];
                    let mut len = d.length();
                    if len < 0.01 {
                        d = egui::vec2(0.1 * (i as f32 + 1.0), 0.1);
                        len = d.length();
                    }
                    let f = k * k / len;
                    let dir = d / len;
                    *disp.get_mut(&ids[i]).unwrap() += dir * f;
                    *disp.get_mut(&ids[j]).unwrap() -= dir * f;
                }
            }
            for &(u, v) in &self.graph_edges {
                let d = self.graph_layout[&u] - self.graph_layout[&v];
                let len = d.length().max(0.01);
                let f = len * len / k;
                let dir = d / len;
                *disp.get_mut(&u).unwrap() -= dir * f;
                *disp.get_mut(&v).unwrap() += dir * f;
            }
            for &id in &ids {
                let mut dv = disp[&id];
                let l = dv.length();
                if l > 20.0 {
                    dv = dv / l * 20.0; // cap movement per step
                }
                *self.graph_layout.get_mut(&id).unwrap() += dv;
            }
        }
        self.graph_built = true;
    }

    /// The link-graph window: draws the force-directed layout, click a node to
    /// jump to it. Rebuilt each time it's opened so it reflects current links.
    fn graph_window(&mut self, ctx: &egui::Context) {
        if !self.graph_built {
            self.build_graph();
        }
        let mut open = self.graph_open;
        let mut jump: Option<NodeId> = None;
        egui::Window::new("Link graph")
            .open(&mut open)
            .default_size([620.0, 520.0])
            .resizable(true)
            .show(ctx, |ui| {
                if self.graph_layout.is_empty() {
                    ui.weak("No links yet. Write [[Node Title]] in a card to connect nodes here.");
                    return;
                }
                ui.small(format!("{} linked nodes · {} links · click a node to open it", self.graph_layout.len(), self.graph_edges.len()));
                let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
                let painter = ui.painter_at(rect);
                // Fit the layout's bounding box into the paint area.
                let mut min = egui::pos2(f32::MAX, f32::MAX);
                let mut max = egui::pos2(f32::MIN, f32::MIN);
                for p in self.graph_layout.values() {
                    min.x = min.x.min(p.x);
                    min.y = min.y.min(p.y);
                    max.x = max.x.max(p.x);
                    max.y = max.y.max(p.y);
                }
                let span = (max - min).max(egui::vec2(1.0, 1.0));
                let margin = 40.0;
                let scale = ((rect.width() - 2.0 * margin) / span.x)
                    .min((rect.height() - 2.0 * margin) / span.y)
                    .clamp(0.05, 2.5);
                let lcenter = egui::pos2((min.x + max.x) * 0.5, (min.y + max.y) * 0.5);
                let map = |p: egui::Pos2| rect.center() + (p - lcenter) * scale;

                let edge_stroke = egui::Stroke::new(1.0, ui.visuals().weak_text_color());
                for &(u, v) in &self.graph_edges {
                    if let (Some(a), Some(b)) = (self.graph_layout.get(&u), self.graph_layout.get(&v)) {
                        painter.line_segment([map(*a), map(*b)], edge_stroke);
                    }
                }
                let hover = resp.hover_pos();
                let mut hit: Option<NodeId> = None;
                for (&id, p) in &self.graph_layout {
                    let sp = map(*p);
                    let is_sel = self.selected == Some(id);
                    let near = hover.map_or(false, |h| h.distance(sp) <= 10.0);
                    if near {
                        hit = Some(id);
                    }
                    let col = match self.doc.nodes.get(&id).and_then(|n| n.color) {
                        Some(c) => egui::Color32::from_rgb(c[0], c[1], c[2]),
                        None => ui.visuals().selection.bg_fill,
                    };
                    painter.circle_filled(sp, if near || is_sel { 8.0 } else { 5.0 }, col);
                    let title = self.doc.nodes.get(&id).map(|n| n.title.clone()).unwrap_or_default();
                    let short: String = title.chars().take(24).collect();
                    painter.text(
                        sp + egui::vec2(8.0, -8.0),
                        egui::Align2::LEFT_BOTTOM,
                        short,
                        egui::FontId::proportional(11.0),
                        ui.visuals().text_color(),
                    );
                }
                if resp.clicked() {
                    if let Some(id) = hit {
                        jump = Some(id);
                    }
                }
            });
        self.graph_open = open;
        if !open {
            self.graph_built = false; // rebuild next open
        }
        if let Some(id) = jump {
            self.jump_to_node(id);
        }
    }

    /// Kanban board: cards that have a `status::` property, in columns by status.
    /// Drag a card to another column to rewrite its `status`. Click to jump.
    fn kanban_window(&mut self, ctx: &egui::Context) {
        let mut open = self.kanban_open;
        let mut board = self.doc.cards_by_status();
        // Projects present on the board, in tree order so the menu reads like
        // the left panel.
        let mut projects: Vec<(NodeId, String)> = Vec::new();
        for cards in board.values() {
            for c in cards {
                if !projects.iter().any(|(id, _)| *id == c.root) {
                    projects.push((c.root, c.root_title.clone()));
                }
            }
        }
        projects.sort_by_key(|(id, _)| {
            self.doc.roots.iter().position(|r| r == id).unwrap_or(usize::MAX)
        });
        // A stored project that's gone must not blank the board.
        if self.kanban_project.is_some_and(|p| !projects.iter().any(|(id, _)| *id == p)) {
            self.kanban_project = None;
        }
        if let Some(p) = self.kanban_project {
            for cards in board.values_mut() {
                cards.retain(|c| c.root == p);
            }
            board.retain(|_, cards| !cards.is_empty());
        }
        // Standard columns first, then any other statuses in use.
        let mut cols: Vec<String> = ["todo", "doing", "done"].iter().map(|s| s.to_string()).collect();
        for k in board.keys() {
            if !cols.contains(k) {
                cols.push(k.clone());
            }
        }
        if !self.kanban_show_done {
            cols.retain(|c| c != "done");
        }
        let today = crate::api::today_days();
        let mut show_done = self.kanban_show_done;
        let mut project_pick = self.kanban_project;
        // Colours resolved up front: the window closure can't borrow `self`.
        let pcolors: std::collections::HashMap<NodeId, egui::Color32> = projects
            .iter()
            .map(|(id, _)| (*id, project_color(&self.doc, *id)))
            .chain(
                board
                    .values()
                    .flatten()
                    .map(|c| (c.root, project_color(&self.doc, c.root))),
            )
            .collect();
        let mut jump: Option<(NodeId, CardId)> = None;
        let mut moves: Vec<(NodeId, CardId, String)> = Vec::new();
        egui::Window::new("Kanban board")
            .open(&mut open)
            .default_size([900.0, 560.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.small("Cards with a status:: property. Drag a card between columns to change its status.");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut show_done, "Show done");
                        ui.separator();
                        if project_pick.is_some() && ui.small_button("×").on_hover_text("Show every project").clicked() {
                            project_pick = None;
                        }
                        let current = project_pick
                            .and_then(|p| projects.iter().find(|(id, _)| *id == p))
                            .map(|(_, t)| t.clone())
                            .unwrap_or_else(|| "All projects".to_string());
                        egui::ComboBox::from_id_salt("kanban_project")
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(project_pick.is_none(), "All projects").clicked() {
                                    project_pick = None;
                                }
                                for (id, title) in &projects {
                                    let on = project_pick == Some(*id);
                                    ui.horizontal(|ui| {
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::vec2(10.0, 10.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().circle_filled(rect.center(), 4.0, pcolors[id]);
                                        if ui.selectable_label(on, title).clicked() {
                                            project_pick = Some(*id);
                                        }
                                    });
                                }
                            });
                        ui.label("Project");
                    });
                });
                if board.is_empty() {
                    ui.weak("No cards have a status:: property yet. Add `status:: todo` to a card.");
                }
                ui.separator();

                // Columns divide the window width so they fit without scrolling;
                // only scroll horizontally once there are more columns than fit
                // (each floored at 180px).
                let n = cols.len().max(1) as f32;
                let gap = ui.spacing().item_spacing.x;
                let col_w = (((ui.available_width() - gap * (n - 1.0)) / n) - 2.0).max(180.0);
                let col_h = ui.available_height().max(140.0);

                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let empty = Vec::new();
                        for col in &cols {
                            let cards = board.get(col).unwrap_or(&empty);
                            // top_down layout so cards stack vertically (the group
                            // would otherwise inherit this row's horizontal layout).
                            let resp = ui
                                .allocate_ui_with_layout(
                                    egui::vec2(col_w, col_h),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        egui::Frame::group(ui.style()).show(ui, |ui| {
                                            ui.set_width(col_w - 12.0);
                                            ui.set_min_height(col_h - 8.0);
                                            ui.strong(format!("{col}  ({})", cards.len()));
                                            ui.separator();
                                            // Each column scrolls its own cards, so a
                                            // tall column never overflows the board.
                                            egui::ScrollArea::vertical()
                                                .id_salt(("kbcol", col))
                                                .auto_shrink([false, false])
                                                .show(ui, |ui| {
                                                    for kc in cards {
                                                        let pc = pcolors.get(&kc.root).copied().unwrap_or(egui::Color32::GRAY);
                                                        kanban_card_ui(ui, kc, today, pc, &mut jump);
                                                    }
                                                });
                                        });
                                    },
                                )
                                .response;
                            if let Some(p) = resp.dnd_release_payload::<(NodeId, CardId)>() {
                                moves.push((p.0, p.1, col.clone()));
                            }
                        }
                    });
                });
            });
        self.kanban_open = open;
        self.kanban_show_done = show_done;
        self.kanban_project = project_pick;
        for (n, c, status) in moves {
            if self.doc.set_card_property(n, c, "status", &status) {
                // Carries the key and value, exactly as the API path does, so a
                // client can't tell whether a status move came from the board or
                // from an agent — and shouldn't have to.
                let title = self.doc.card(n, c).map(|k| k.title.clone()).unwrap_or_default();
                self.note(
                    crate::changelog::Change::new(
                        crate::changelog::Actor::Ui,
                        crate::changelog::Entity::Card,
                        crate::changelog::Op::Updated,
                        c,
                    )
                    .in_node(n)
                    .titled(title)
                    .field("property")
                    .property("status", status.clone()),
                );
            }
        }
        if let Some((node, card)) = jump {
            self.jump_to_card(ctx, node, card);
        }
    }

    /// Navigate a clicked `[[wiki-link]]` (its URL-encoded target) to the node
    /// it names, or report that no such node exists.
    fn follow_wikilink(&mut self, encoded: &str) {
        let target = crate::model::decode_link(encoded);
        match self.doc.resolve_link(&target) {
            Some(id) => self.jump_to_node(id),
            None => self.status = format!("No node named \"{target}\" to link to"),
        }
    }

    /// Select a node, open its ancestors so it's visible, and scroll to it.
    fn jump_to_node(&mut self, id: NodeId) {
        let mut cur = self.doc.nodes.get(&id).and_then(|n| n.parent);
        while let Some(pid) = cur {
            match self.doc.nodes.get_mut(&pid) {
                Some(p) => {
                    p.expanded = true;
                    cur = p.parent;
                }
                None => break,
            }
        }
        self.selected = Some(id);
        self.scroll_to = Some(id);
        self.switcher_open = false;
        self.mark_dirty(); // expanded flags are persisted
    }

    /// Like [`jump_to_node`], but also reveals a specific card: the canvas
    /// recenters on it and flashes a fading outline so the click clearly lands.
    /// Used by the agenda and Kanban rows (a task is one canonical card).
    fn jump_to_card(&mut self, ctx: &egui::Context, node: NodeId, card: CardId) {
        self.jump_to_node(node);
        self.focus_card = Some(card);
        self.highlight_card = Some(card);
        self.highlight_until = ctx.input(|i| i.time) + canvas::HIGHLIGHT_SECS;
    }

    /// Reveal a `SearchHit`: if it points at a specific card, jump to it
    /// (recenter + flash); otherwise (a node-title match) just navigate to the
    /// node. Used by the Search, Find, Tags, and Backlinks panels.
    fn reveal_hit(&mut self, ctx: &egui::Context, node: NodeId, card: Option<CardId>) {
        match card {
            Some(c) => self.jump_to_card(ctx, node, c),
            None => self.jump_to_node(node),
        }
    }
}

impl eframe::App for TrellisApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Capture the window handles so file/message dialogs can be parented
        // to the app window instead of opening behind it.
        if let (Ok(w), Ok(d)) = (frame.window_handle(), frame.display_handle()) {
            self.dialog_parent = Some(DialogParent { window: w.as_raw(), display: d.as_raw() });
        }

        // Keep the window title on the open document (New/Open/Save As change it).
        self.sync_window_title(ctx);

        // Apply any API requests from the server thread first.
        self.pump_api();
        // Apply any finished background OCR results.
        self.pump_ocr();
        // Turn finished region-snips into image cards.
        self.pump_plugins();
        self.pump_plugin_triggers();
        self.pump_sources(false);
        self.pump_snip();

        // Apply finished background saves.
        self.pump_save();

        // Fire scheduled backups and apply finished ones.
        self.pump_backup();

        // A requested single-card screenshot arrives as an input event one frame
        // after we ask for it; crop it to the card and save as PNG/PDF.
        if matches!(self.card_shot.as_ref().map(|s| &s.phase), Some(ShotPhase::Requested)) {
            let shot_img = ctx.input(|i| {
                i.events.iter().rev().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot_img {
                self.finish_card_shot(ctx, &img);
            }
        }

        // The same, one shot per frame, for a multi-shot basket export.
        if matches!(self.basket_shot.as_ref().map(|s| &s.phase), Some(ShotPhase::Requested)) {
            let shot_img = ctx.input(|i| {
                i.events.iter().rev().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot_img {
                self.capture_basket_shot(ctx, &img);
            }
        }

        // Autosave: once the document has been idle for AUTOSAVE_IDLE (no further
        // changes), write it to disk on a worker thread (never blocks the UI).
        // Debounced so continuous edits — dragging a card, typing — never save
        // mid-gesture. request_repaint_after wakes the loop at the deadline even
        // when the UI is otherwise idle.
        if self.autosave && self.dirty && !self.saving {
            match self.last_change {
                Some(t) if t.elapsed() >= AUTOSAVE_IDLE => {
                    let path = self.target_path();
                    self.spawn_save(path);
                }
                Some(t) => ctx.request_repaint_after(AUTOSAVE_IDLE.saturating_sub(t.elapsed())),
                None => self.last_change = Some(Instant::now()),
            }
        }

        // Keep the loop waking (~once a minute) so scheduled backups fire while
        // the UI is idle. Backup intervals are in minutes, so this is cheap.
        if self.backup_cfg.enabled && self.backup_cfg.interval_mins > 0 {
            ctx.request_repaint_after(std::time::Duration::from_secs(60));
        }

        // Zoom is per-canvas now, so keep the whole-UI zoom factor pinned at 1.0.
        // egui persists zoom_factor across runs, so an earlier build that scaled
        // the whole UI would otherwise leave the chrome stuck zoomed. Idempotent.
        if (ctx.zoom_factor() - 1.0).abs() > f32::EPSILON {
            ctx.set_zoom_factor(1.0);
        }

        // Theme / color scheme.
        ctx.set_visuals(self.theme.visuals());

        // Keyboard shortcuts (canvas zoom keys are handled in canvas::ui).
        let cmd = ctx.input(|i| i.modifiers.command);
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::S)) {
            self.save();
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::F)) {
            self.search_open = !self.search_open;
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::O)) {
            self.switcher_open = true;
            self.switcher_query.clear();
            self.switcher_index = 0;
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::N)) {
            self.new_document();
        }
        // Undo/redo — but not while a text field is capturing the keyboard, so
        // egui's built-in in-field text undo keeps Ctrl+Z while you type in a card.
        if !ctx.wants_keyboard_input() {
            let shift = ctx.input(|i| i.modifiers.shift);
            if cmd && !shift && ctx.input(|i| i.key_pressed(egui::Key::Z)) {
                self.undo();
            }
            if cmd && ((shift && ctx.input(|i| i.key_pressed(egui::Key::Z)))
                || ctx.input(|i| i.key_pressed(egui::Key::Y)))
            {
                self.redo();
            }
        }

        self.menu_bar(ctx);

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    self.save();
                }
                ui.separator();
                let title = self
                    .doc_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "untitled".to_string());
                let mark = if self.dirty { "●" } else { "" };
                ui.label(format!("{mark} {title}"));
                ui.separator();
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.0}%", self.current_zoom_pct()));
                });
            });
        });

        if self.search_open {
            self.search_panel(ctx);
        }
        if self.switcher_open {
            self.quick_switcher(ctx);
        }
        if self.tags_open {
            self.tags_panel(ctx);
        }
        if self.find_open {
            self.find_panel(ctx);
        }
        if self.agenda_open {
            self.agenda_panel(ctx);
        }
        if self.backlinks_open {
            self.backlinks_panel(ctx);
        }
        if self.graph_open {
            self.graph_window(ctx);
        }
        if self.kanban_open {
            self.kanban_window(ctx);
        }

        // Follow [[wiki-links]] (rendered as the `trellis:` URL scheme) by
        // navigating instead of letting eframe open a browser.
        let clicked = ctx.output(|o| o.open_url.as_ref().map(|u| u.url.clone()));
        if let Some(url) = clicked {
            if let Some(t) = url.strip_prefix("trellis:") {
                let target = t.to_string();
                ctx.output_mut(|o| o.open_url = None);
                self.follow_wikilink(&target);
            }
        }

        egui::SidePanel::left("tree")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                let scroll_to = self.scroll_to.take();
                let node_plugins = self.plugins_for(crate::plugins::Trigger::NodeMenu);
                let actions = tree::ui(
                    ui,
                    &self.doc,
                    self.selected,
                    &mut self.renaming,
                    self.reorder_mode,
                    scroll_to,
                    &node_plugins,
                );
                self.apply_tree(actions);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(sel) = self.selected {
                if self.doc.nodes.contains_key(&sel) {
                    // Card multi-selection is per-basket; reset it on node change.
                    if self.card_sel_node != Some(sel) {
                        self.card_sel.clear();
                        self.card_sel_node = Some(sel);
                    }
                    let mut view = self.views.get(&sel).copied().unwrap_or_default();
                    // If a WYSIWYG card screenshot is in its Framing phase for this
                    // node, reframe the view to fit that card so the whole card is
                    // captured unclipped. The reframe is temporary (not persisted).
                    let framing_card = match &self.card_shot {
                        Some(s) if s.node == sel && matches!(s.phase, ShotPhase::Framing) => {
                            Some(s.card)
                        }
                        _ => None,
                    };
                    if let Some(cid) = framing_card {
                        if let Some(c) = self.doc.card(sel, cid) {
                            view = framed_view(ui.available_rect_before_wrap(), c.pos, c.size);
                        }
                    }
                    // Likewise for a basket export: frame the current shot's target
                    // (the whole basket, or one card), temporarily and unpersisted.
                    let basket_target = self.basket_frame_target(sel);
                    if let Some((pos, size)) = basket_target {
                        view = framed_view(ui.available_rect_before_wrap(), pos, size);
                        // This frame renders the reframe, so a screenshot may now
                        // be requested (the first shot is otherwise un-reframed).
                        if let Some(bs) = self.basket_shot.as_mut() {
                            bs.framed = true;
                        }
                    }
                    let template_names: Vec<String> =
                        self.templates.iter().map(|t| t.card.title.clone()).collect();
                    let mut env = Env {
                        md: &mut self.md_cache,
                        tex: &mut self.tex_cache,
                        card_rects: &mut self.card_rects,
                        templates: &template_names,
                        inline_sent: &mut self.inline_sent,
                        inline_epoch: self.inline_epoch,
                        focus_card: self.focus_card,
                        highlight_card: self.highlight_card,
                        highlight_until: self.highlight_until,
                        minimap: self.minimap_enabled,
                        style: match self.theme {
                            Theme::StickyNotes => canvas::CardStyle::Sticky,
                            Theme::Futuristic => canvas::CardStyle::Futuristic,
                            _ => canvas::CardStyle::Normal,
                        },
                        glow: matches!(self.theme, Theme::Futuristic | Theme::SynthWave),
                    };
                    let can_paste = self.card_clipboard.is_some();
                    let node_path = crate::tree::node_path(&self.doc, sel);
                    let node = self.doc.nodes.get(&sel).unwrap();
                    let actions = canvas::ui(
                        ui,
                        node,
                        &node_path,
                        &mut view,
                        self.zoom_enabled,
                        can_paste,
                        self.dock_mode,
                        self.snap_mode,
                        &mut env,
                        &self.card_sel,
                    );
                    // The recenter is one-shot: the canvas consumed it this frame,
                    // so the user can pan freely afterward.
                    self.focus_card = None;
                    // Never let a temporary export reframe overwrite the real view.
                    if framing_card.is_none() && basket_target.is_none() {
                        self.views.insert(sel, view);
                    }
                    let pointer_down = ui.input(|i| i.pointer.any_down());
                    self.apply_canvas(ctx, sel, actions, pointer_down);
                } else {
                    self.selected = None;
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("No node selected. Add one on the left to start a basket.");
                });
            }
        });

        // Drive the single-card screenshot export. The canvas has just rendered
        // (framed onto the card, if Framing); request the framebuffer now — the
        // backend reads it at the end of this frame and delivers it next frame.
        if let Some(shot) = self.card_shot.as_mut() {
            if matches!(shot.phase, ShotPhase::Framing) {
                shot.phase = ShotPhase::Requested;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            ctx.request_repaint();
        }
        // Same driver for a basket export (one shot per frame across the queue).
        // Only request the shot once the reframe has actually rendered (`framed`),
        // so the overview/first page fits all cards instead of the starting view.
        if let Some(bs) = self.basket_shot.as_mut() {
            if matches!(bs.phase, ShotPhase::Framing) && bs.framed {
                bs.phase = ShotPhase::Requested;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            ctx.request_repaint();
        }

        if self.show_about {
            egui::Window::new("About Trellis")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.heading("Trellis");
                    ui.label(egui::RichText::new("The tree and the weave.").italics());
                    ui.add_space(4.0);
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.add_space(4.0);
                    ui.label("A hierarchical, spatial note-taking app.");
                    ui.add_space(4.0);
                    ui.label("A tree of nodes, where every node is a free-form basket of cards.");
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() {
                        self.show_about = false;
                    }
                });
        }

        if self.show_settings {
            self.settings_window(ctx);
        }
        if self.show_backup {
            self.backup_window(ctx);
        }
        if self.show_history {
            self.history_window(ctx);
        }
        if self.show_requirements {
            self.requirements_window(ctx);
        }
        if self.show_plugins {
            self.plugins_window(ctx);
        }

        self.lightbox_ui(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Remember which file to reopen next launch (untitled docs live in the
        // autosave slot and need no key).
        if let Some(p) = &self.doc_path {
            storage.set_string(LAST_DOC_KEY, p.display().to_string());
        }
        storage.set_string(API_KEY_KEY, self.api_key.clone());
        storage.set_string(API_PORT_KEY, self.api_port.to_string());
        storage.set_string(API_LAN_KEY, self.api_lan.to_string());
        storage.set_string(ZOOM_ENABLED_KEY, self.zoom_enabled.to_string());
        storage.set_string(DOCK_MODE_KEY, self.dock_mode.to_string());
        storage.set_string(SNAP_MODE_KEY, self.snap_mode.to_string());
        storage.set_string(MINIMAP_KEY, self.minimap_enabled.to_string());
        storage.set_string(THEME_KEY, self.theme.key().to_string());
        storage.set_string(AUTOSAVE_KEY, self.autosave.to_string());
        storage.set_string(
            AGENDA_PROJECT_KEY,
            self.agenda_project.map(|n| n.to_string()).unwrap_or_default(),
        );
        storage.set_string(
            KANBAN_PROJECT_KEY,
            self.kanban_project.map(|n| n.to_string()).unwrap_or_default(),
        );
        storage.set_string(BACKUP_KEY, self.backup_cfg.to_json());
        storage.set_string(HISTORY_KEEP_KEY, self.history_keep.to_string());
        storage.set_string(HISTORY_GAP_KEY, self.history_gap_mins.to_string());
        if let Ok(s) = serde_json::to_string(&self.templates) {
            storage.set_string(TEMPLATES_KEY, s);
        }
        storage.set_string(MIRROR_MODE_KEY, self.mirror_policy.key().to_string());
        storage.set_string(MIRROR_DIRS_KEY, self.mirror_dirs.join("\n"));
        if let Ok(g) = self.grants.lock() {
            if let Ok(s) = serde_json::to_string(&*g) {
                storage.set_string(GRANTS_KEY, s);
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Best-effort autosave to the working file (or the autosave slot).
        // No history snapshot here — see `write_to_inner`. This save blocks the
        // window from closing, so it stays as short as it can be.
        let path = self.target_path();
        self.write_to(path, false);
    }
}

/// Install DejaVu as the primary UI font. It carries the arrows, bullets,
/// dashes and box-drawing that egui's default fonts lack, so UI glyphs and the
/// wide Unicode common in dev/sysadmin notes render instead of showing tofu.
/// The egui defaults stay as fallback (emoji, Cyrillic, …).
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/DejaVuSans.ttf")),
    );
    fonts.font_data.insert(
        "dejavu-mono".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/DejaVuSansMono.ttf")),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "dejavu".to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "dejavu-mono".to_owned());
    ctx.set_fonts(fonts);
}

/// A random API key: 48 hex chars from the OS CSPRNG.
///
/// This used to read `/dev/urandom` directly, which does not exist on Windows or
/// on a sandboxed macOS process — the open failed and it fell back to a
/// `pid + nanoseconds` string. That is not a secret: both halves are guessable
/// to within a small range by anything that can see the process, and the key is
/// the only thing standing between a caller and the whole document. `getrandom`
/// asks each OS for its own CSPRNG (`getrandom(2)`, `BCryptGenRandom`,
/// `SecRandomCopyBytes`), so there is no weak path left to fall back to.
/// Capture a user-selected screen region to PNG bytes. An empty `Vec` means the
/// user cancelled the selection.
///
/// Every platform has a region-select capture; none of them agree on how to ask
/// for it. macOS and Windows both have one built in, so only Linux needs a tool
/// installed — see `deps::all()`.
///
/// **The result is judged by the file, not the exit code.** The capture tools
/// disagree about cancellation: maim and scrot exit non-zero, `screencapture`
/// exits *zero* and simply writes nothing, and the Windows path can't see the
/// child's outcome at all. A file that exists and is non-empty is a capture;
/// anything else after a clean run is a cancel.
fn capture_region() -> Result<Vec<u8>, String> {
    let out = std::env::temp_dir().join(format!("trellis-snip-{}.png", std::process::id()));
    // A leftover from a previous cancel would otherwise be returned as if it
    // were this capture — the same screenshot appearing twice.
    let _ = std::fs::remove_file(&out);

    let ran = capture_region_into(&out);
    let bytes = std::fs::read(&out).ok().filter(|b| !b.is_empty());
    let _ = std::fs::remove_file(&out);

    match (ran, bytes) {
        (_, Some(b)) => Ok(b),
        (Ok(()), None) => Ok(Vec::new()), // ran, no image — cancelled
        (Err(e), None) => Err(e),
    }
}

/// Drive the platform's interactive region capture, writing a PNG to `out`.
#[cfg(target_os = "linux")]
fn capture_region_into(out: &std::path::Path) -> Result<(), String> {
    let path = out.to_string_lossy().to_string();
    // (binary, args) — each does interactive region select and writes `path`.
    let candidates: [(&str, Vec<&str>); 5] = [
        ("spectacle", vec!["-b", "-n", "-r", "-o", &path]),
        ("gnome-screenshot", vec!["-a", "-f", &path]),
        ("maim", vec!["-s", &path]),
        ("scrot", vec!["-s", &path]),
        ("import", vec![&path]), // ImageMagick: click-drag a region
    ];
    for (bin, args) in candidates {
        if std::process::Command::new(bin).args(&args).status().is_ok() {
            return Ok(());
        }
    }
    Err(crate::deps::get("snip")
        .map(|d| crate::deps::missing_msg(&d))
        .unwrap_or_else(|| "no screenshot tool found".into()))
}

/// macOS ships this: `-i` is the familiar crosshair-drag, Esc cancels.
#[cfg(target_os = "macos")]
fn capture_region_into(out: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("screencapture")
        .arg("-i")
        .arg(out)
        .status()
        .map(|_| ())
        .map_err(|e| format!("could not run screencapture ({e})"))
}

/// Windows has the Snipping Tool overlay (the Win+Shift+S one) but it only ever
/// delivers to the clipboard — there is no "write a region to this file" flag.
/// So: open the overlay, wait for an image to land on the clipboard, and save
/// that. PowerShell does the whole thing because reading a clipboard *bitmap*
/// from Rust would mean a new dependency for this one feature.
///
/// The clipboard is cleared first so a picture that was already on it can't be
/// mistaken for the capture. That is not as destructive as it sounds: a
/// successful snip overwrites the clipboard anyway, so this only costs the user
/// their clipboard in the case where they cancel.
///
/// `powershell.exe` (5.1) rather than `pwsh`, because it is always present and
/// defaults to the single-threaded apartment the clipboard API requires; `-STA`
/// is passed anyway so it stays correct if the default ever changes.
#[cfg(target_os = "windows")]
fn capture_region_into(out: &std::path::Path) -> Result<(), String> {
    // PowerShell single-quoted strings escape a quote by doubling it.
    let path = out.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
         [Windows.Forms.Clipboard]::Clear(); \
         Start-Process 'ms-screenclip:'; \
         $deadline = (Get-Date).AddSeconds(120); \
         while ((Get-Date) -lt $deadline) {{ \
           Start-Sleep -Milliseconds 250; \
           $img = [Windows.Forms.Clipboard]::GetImage(); \
           if ($img) {{ \
             $img.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Png); \
             exit 0 \
           }} \
         }}; \
         exit 1"
    );
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-STA", "-WindowStyle", "Hidden", "-Command", &script])
        .status()
        .map(|_| ())
        .map_err(|e| {
            format!("could not start the Snipping Tool overlay via PowerShell ({e})")
        })
}

/// Run tesseract OCR over each image's bytes and return the combined text.
/// Writes each image to a temp file and shells out to the `tesseract` CLI
/// (a runtime dependency). Called on a background thread.
fn ocr_images(images: &[Vec<u8>]) -> Result<String, String> {
    let mut out = String::new();
    for (i, bytes) in images.iter().enumerate() {
        let path = std::env::temp_dir().join(format!("trellis-ocr-{}-{i}.img", std::process::id()));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let result = std::process::Command::new("tesseract")
            .arg(&path)
            .arg("stdout")
            .output();
        let _ = std::fs::remove_file(&path);
        match result {
            Ok(o) if o.status.success() => {
                out.push_str(String::from_utf8_lossy(&o.stdout).trim());
                out.push('\n');
            }
            Ok(o) => return Err(format!("tesseract error: {}", String::from_utf8_lossy(&o.stderr).trim())),
            // Name the tool *and* where to get it — "install tesseract-ocr" is
            // the package name on exactly one platform and helps nobody else.
            Err(_) => {
                return Err(crate::deps::get("tesseract")
                    .map(|d| crate::deps::missing_msg(&d))
                    .unwrap_or_else(|| "tesseract is not installed".into()))
            }
        }
    }
    Ok(out.trim().to_string())
}

/// Serialize the document to compact RON, gzip it, and write it atomically
/// (temp file + rename). Compact RON keeps the pre-compression text small;
/// embedded image bytes serialize as decimal arrays that pretty-printing bloated
/// ~32×, which gzip crushes back to near the raw image size. Pure and `Send`, so
/// it runs on a worker thread (see `spawn_save`).
/// Rank how well `query` (already lowercased) matches a node, lower = better.
/// Prefers a title substring, then a title subsequence, then a path hit. Empty
/// query matches everything (so Ctrl+O with no text lists nodes). `title` and
/// `hay` (title + "\n" + path) are lowercased by the caller.
fn fuzzy_score(query: &str, title: &str, hay: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(1000);
    }
    if let Some(pos) = title.find(query) {
        return Some(pos as i32); // earliest title substring wins
    }
    if let Some(s) = subseq_score(query, title) {
        return Some(100 + s);
    }
    if hay.contains(query) {
        return Some(400);
    }
    if let Some(s) = subseq_score(query, hay) {
        return Some(500 + s);
    }
    None
}

/// If every char of `q` appears in `text` in order, score by how early and how
/// tightly they matched (lower = better); else `None`.
fn subseq_score(q: &str, text: &str) -> Option<i32> {
    let mut chars = q.chars();
    let mut want = chars.next();
    let mut first: Option<usize> = None;
    let mut last = 0usize;
    for (i, c) in text.chars().enumerate() {
        if let Some(w) = want {
            if c == w {
                first.get_or_insert(i);
                last = i;
                want = chars.next();
            }
        }
    }
    if want.is_none() {
        let start = first.unwrap_or(0);
        Some(start as i32 + (last - start) as i32)
    } else {
        None
    }
}

/// Turn a snapshot filename `20260730-142530.ron.gz` into `2026-07-30 14:25:30`.
/// Falls back to the raw name if it isn't in the expected shape.
/// Snapshot/backup filenames are stamped in **UTC** (monotonic, and immune to
/// the hour that repeats at a DST fall-back), but a timestamp shown to a human
/// should be their own clock — otherwise a snapshot taken at 09:20 reads back as
/// 16:20. Convert for display only; the filename is untouched.
fn format_stamp(name: &str) -> String {
    let s = name.split('.').next().unwrap_or(name); // strip .ron.gz
    match chrono::NaiveDateTime::parse_from_str(s, "%Y%m%d-%H%M%S") {
        Ok(naive) => chrono::TimeZone::from_utc_datetime(&chrono::Utc, &naive)
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
        Err(_) => name.to_string(),
    }
}

fn serialize_doc(doc: &Document) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let s = ron::to_string(doc).map_err(|e| e.to_string())?;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(s.as_bytes()).map_err(|e| e.to_string())?;
    enc.finish().map_err(|e| e.to_string())
}

/// Defaults for how many version snapshots to keep, and the minimum gap between
/// them so a burst of autosaves doesn't churn through the whole history in a
/// minute. Both are settings (Tools → Settings → Version history) because the
/// right values depend on the document: a snapshot is a full copy, so a large
/// document wants fewer of them, spaced further apart.
const HISTORY_KEEP: usize = 25;
const HISTORY_MIN_GAP_SECS: u64 = 180;
/// Bounds for the settings, so a typo can't disable history or fill the disk.
const HISTORY_KEEP_RANGE: std::ops::RangeInclusive<usize> = 1..=100;
const HISTORY_GAP_MINS_RANGE: std::ops::RangeInclusive<u64> = 1..=1440;

/// The hidden sibling directory that holds a document's version snapshots, e.g.
/// `Notes.ron` → `.Notes.ron.history/`. `None` for a pathless document.
fn history_dir(doc_path: &std::path::Path) -> Option<PathBuf> {
    let name = doc_path.file_name()?.to_string_lossy();
    Some(doc_path.with_file_name(format!(".{name}.history")))
}

/// List a document's snapshots, newest first: `(path, filename)`.
fn history_snapshots(doc_path: &std::path::Path) -> Vec<(PathBuf, String)> {
    let Some(dir) = history_dir(doc_path) else { return Vec::new() };
    let mut v: Vec<(PathBuf, String)> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "gz"))
            .filter_map(|p| p.file_name().map(|n| (p.clone(), n.to_string_lossy().into_owned())))
            .collect(),
        Err(_) => return Vec::new(),
    };
    v.sort_by(|a, b| b.1.cmp(&a.1)); // timestamped names sort chronologically
    v
}

/// After a successful save, drop a timestamped snapshot into the history dir
/// (unless the newest one is younger than the min gap), then prune to the cap.
/// Runs on whatever thread saved; failures are best-effort and ignored.
fn write_history_snapshot(doc_path: &std::path::Path, keep: usize, min_gap_secs: u64) {
    let Some(dir) = history_dir(doc_path) else { return };
    let snaps = history_snapshots(doc_path);
    if let Some((newest, _)) = snaps.first() {
        if let Ok(age) = newest.metadata().and_then(|m| m.modified()).and_then(|t| t.elapsed().map_err(std::io::Error::other)) {
            if age.as_secs() < min_gap_secs {
                return;
            }
        }
    }
    let Ok(bytes) = std::fs::read(doc_path) else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = crate::backup::stamp(std::time::SystemTime::now());
    let _ = std::fs::write(dir.join(format!("{stamp}.ron.gz")), &bytes);
    // Prune oldest beyond the cap. `max(1)` so a bad setting can never delete
    // every snapshot including the one just written.
    let snaps = history_snapshots(doc_path);
    for (path, _) in snaps.into_iter().skip(keep.max(1)) {
        let _ = std::fs::remove_file(path);
    }
}

fn serialize_and_write(doc: &Document, path: &std::path::Path) -> Result<(), String> {
    let bytes = serialize_doc(doc)?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(format!(
        "{}.tmp",
        path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
    ));
    std::fs::write(&tmp, &bytes)
        .and_then(|_| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
}

/// Read a Trellis document from disk, transparently handling both current
/// **gzip-compressed** saves and older **plain-text RON** files (magic-byte sniff).
fn read_document(path: &std::path::Path) -> Result<Document, String> {
    use std::io::Read;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut s = String::new();
        flate2::read::GzDecoder::new(&bytes[..])
            .read_to_string(&mut s)
            .map_err(|e| e.to_string())?;
        s
    } else {
        String::from_utf8(bytes).map_err(|e| e.to_string())?
    };
    ron::from_str::<Document>(&text).map_err(|e| e.to_string())
}

/// The Settings status line describing where the API is listening.
fn api_status_line(lan: bool, port: u16) -> String {
    if lan {
        let host = local_ip().unwrap_or_else(|| "0.0.0.0".to_string());
        format!("Listening on http://{host}:{port}/api (LAN)")
    } else {
        format!("Listening on http://127.0.0.1:{port}/api")
    }
}

/// Best-effort local LAN IP, for showing the reachable API URL. Opens a UDP
/// socket "connected" to a public address (no packets are sent) and reads back
/// the local address the OS would route through. Returns `None` if unavailable.
fn local_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    Some(sock.local_addr().ok()?.ip().to_string())
}

fn generate_key() -> String {
    let mut buf = [0u8; 24];
    // Failing here means the OS has no entropy source at all. There is no
    // sensible weaker key to fall back to, so refuse rather than quietly hand
    // out a guessable one — the API is off until a key exists, which is the
    // safe direction to fail in.
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn default_autosave_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "Trellis", "Trellis")
        .map(|d| d.data_dir().join("autosave.ron"))
        .unwrap_or_else(|| PathBuf::from("trellis-autosave.ron"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gap under an API-created card: `Card::fit_size` has no font context so
    /// it *estimates* the wrapped height, and estimates tall. The right-click
    /// "Fit to content" measures the real galley and comes out shorter — which is
    /// why using the menu action on an API-created card visibly shrank it.
    ///
    /// `pump_api` now re-measures with `fit_card_size` after `api::process`, so
    /// both paths land on the same height. This pins the two facts that matter:
    /// the precise height is never taller than the estimate (a taller one would
    /// clip text), and for wrapping prose it is actually *shorter*, i.e. the gap
    /// this fixes was real.
    #[test]
    fn precise_fit_is_never_taller_than_the_estimate_and_closes_the_gap() {
        let ctx = egui::Context::default();
        // Fonts are lazily built on the first frame; lay out inside one.
        let _ = ctx.run(Default::default(), |_| {});

        let body = "This is a paragraph of ordinary prose that wraps across several \
                    lines inside the card.\n\n- a bullet\n- another bullet\n- a third one";
        let mut card = crate::model::Card::new(
            1,
            egui::pos2(0.0, 0.0),
            crate::model::CardKind::Text,
        );
        card.title = "Fit check".into();
        card.body = body.into();

        let estimate = card.fit_size().expect("text cards have an estimate");
        let precise = fit_card_size(&ctx, &card).expect("text cards can be measured");

        assert_eq!(precise.x, estimate.x, "width comes from the estimate either way");
        assert!(
            precise.y <= estimate.y,
            "precise {} must not exceed the estimate {} (taller would clip)",
            precise.y,
            estimate.y
        );
        assert!(
            precise.y < estimate.y,
            "wrapping prose should measure shorter than the estimate — otherwise \
             there was no gap to fix and this regression test proves nothing"
        );

        // A non-text card has no galley to measure; it keeps the estimate.
        let table = crate::model::Card::new(
            2,
            egui::pos2(0.0, 0.0),
            crate::model::CardKind::Table { table: crate::model::TableData::empty(2, 2) },
        );
        assert_eq!(fit_card_size(&ctx, &table), table.fit_size());
    }

    /// Headings render larger than body text, so a card full of them needs more
    /// height than the same words as prose. Measuring every line at one size is
    /// what left long notes clipped at the bottom.
    #[test]
    fn fit_counts_headings_as_taller_than_body_text() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |_| {});

        let mk = |body: &str| {
            let mut c = crate::model::Card::new(
                1,
                egui::pos2(0.0, 0.0),
                crate::model::CardKind::Text,
            );
            c.title = "T".into();
            c.body = body.into();
            c
        };
        // Same words, same line count — only the heading markers differ.
        let headed = mk("## Alpha section\n## Beta section\n## Gamma section");
        let plain = mk("Alpha section\nBeta section\nGamma section");

        let h_precise = fit_card_size(&ctx, &headed).unwrap().y;
        let p_precise = fit_card_size(&ctx, &plain).unwrap().y;
        assert!(
            h_precise > p_precise,
            "headings must measure taller: {h_precise} vs {p_precise}"
        );
        // The estimate has to agree, since it's the fallback with no font context.
        assert!(headed.fit_size().unwrap().y > plain.fit_size().unwrap().y);

        // `#tag` is not a heading — no space after the hashes. Tags are used
        // throughout these notes, so treating them as H1 would inflate every card.
        assert_eq!(crate::model::heading_level("#trellis #ops"), None);
        assert_eq!(crate::model::heading_level("## Real heading"), Some(2));
        assert_eq!(crate::model::heading_level("####### too many"), None);

        // Long notes fit instead of clipping: the old 1400 cap silently cut them.
        let long = mk(&"A line of prose in a long note.\n".repeat(120));
        assert!(
            fit_card_size(&ctx, &long).unwrap().y > 1400.0,
            "a long card must be allowed past the old cap rather than clipped"
        );
    }

    /// Templates saved before the Templates basket existed are stored as bare
    /// `CardExport` objects. They must keep loading — a format break here would
    /// silently empty someone's template library.
    #[test]
    fn old_config_templates_load_without_a_master() {
        let old = r#"[{"format":"trellis-card","version":1,"title":"Local/Prod verify grid",
                       "body":"","color":[68,68,68],"size":[400.0,240.0],"kind":"Text"}]"#;
        let ts: Vec<Template> = serde_json::from_str(old).expect("old config must still parse");
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].card.title, "Local/Prod verify grid");
        assert!(ts[0].master.is_none(), "no master until one is stamped");

        // And a round-trip with a master keeps both halves.
        let with = Template {
            card: ts[0].card.clone(),
            master: Some(MasterRef { node: 7, card: 12 }),
        };
        let json = serde_json::to_string(&vec![with]).unwrap();
        let back: Vec<Template> = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0].card.title, "Local/Prod verify grid");
        let m = back[0].master.expect("master survives a round-trip");
        assert_eq!((m.node, m.card), (7, 12));
        // The flattened card fields stay at the top level, so an old build reading
        // this config still finds a valid CardExport.
        assert!(json.contains("\"format\":\"trellis-card\""));
        assert!(crate::model::parse_card_export(&serde_json::to_string(&back[0]).unwrap()).is_some());
    }

    #[test]
    fn download_name_keeps_stored_name_and_extension() {
        assert_eq!(download_image_name("photo.jpg", 0), "photo.jpg");
        assert_eq!(download_image_name("scan.PNG", 3), "scan.PNG");
    }

    #[test]
    fn format_stamp_is_human_readable() {
        // Stamps are UTC on disk and shown in local time, so the expectation is
        // computed the same way rather than hard-coded to one machine's zone.
        let naive =
            chrono::NaiveDateTime::parse_from_str("20260730-142530", "%Y%m%d-%H%M%S").unwrap();
        let want = chrono::TimeZone::from_utc_datetime(&chrono::Utc, &naive)
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        assert_eq!(format_stamp("20260730-142530.ron.gz"), want);
        assert_eq!(format_stamp("weird-name"), "weird-name");
    }

    #[test]
    fn fuzzy_ranks_title_substring_over_subsequence_over_path() {
        // Empty query matches anything.
        assert!(fuzzy_score("", "anything", "anything\npath").is_some());
        // A title substring beats a subsequence match.
        let sub = fuzzy_score("mon", "monday 7/20", "monday 7/20\n2026 › july").unwrap();
        let seq = fuzzy_score("mnd", "monday 7/20", "monday 7/20\n2026 › july").unwrap();
        assert!(sub < seq, "substring {sub} should rank better than subsequence {seq}");
        // An earlier substring ranks better than a later one.
        let early = fuzzy_score("july", "july", "july\n2026").unwrap();
        let late = fuzzy_score("july", "week of july", "week of july\n2026").unwrap();
        assert!(early < late);
        // Matching only via the path still scores, but worse than a title hit.
        let path_only = fuzzy_score("2026", "monday", "monday\n2026 › july").unwrap();
        assert!(path_only > seq);
        // A query whose chars aren't all present doesn't match.
        assert!(fuzzy_score("xyz", "monday", "monday\njuly").is_none());
    }

    #[test]
    fn subseq_requires_in_order_chars() {
        assert!(subseq_score("abc", "aXbXc").is_some());
        assert!(subseq_score("cba", "aXbXc").is_none()); // wrong order
        assert!(subseq_score("abcd", "abc").is_none()); // 'd' missing
    }

    #[test]
    fn download_name_falls_back_for_nameless_images() {
        assert_eq!(download_image_name("", 0), "image-1.png");
        assert_eq!(download_image_name("   ", 4), "image-5.png");
    }
}
