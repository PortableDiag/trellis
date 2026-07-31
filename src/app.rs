//! Application state and the eframe update loop that stitches the panels
//! together: menu bar, tree, basket canvas, search, and all file operations.

use crate::canvas::{self, CanvasAction, Env};
use crate::images::TextureCache;
use crate::model::{CardId, CardKind, ChecklistItem, Document, NodeId};
use crate::tree::{self, TreeAction};
use crate::api::{self, ApiCommand};
use egui_commonmark::CommonMarkCache;
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
const DEFAULT_API_PORT: u16 = 7373;
const ZOOM_ENABLED_KEY: &str = "zoom_enabled";
const DOCK_MODE_KEY: &str = "dock_mode";
const SNAP_MODE_KEY: &str = "snap_mode";
const MINIMAP_KEY: &str = "minimap";
const THEME_KEY: &str = "theme";
const AUTOSAVE_KEY: &str = "autosave";
const BACKUP_KEY: &str = "backup";
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
}

impl Theme {
    const ALL: [(Theme, &'static str); 3] = [
        (Theme::Trellis, "Trellis"),
        (Theme::Light, "Light"),
        (Theme::TerminalGreen, "Terminal Green"),
    ];

    fn from_key(s: &str) -> Theme {
        match s {
            "Light" => Theme::Light,
            "TerminalGreen" => Theme::TerminalGreen,
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
        }
    }

    fn visuals(self) -> egui::Visuals {
        match self {
            Theme::Light => egui::Visuals::light(),
            Theme::Trellis => egui::Visuals::dark(),
            Theme::TerminalGreen => terminal_green_visuals(),
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
    const MAX_H: f32 = 1400.0;
    let fs = if c.font_scale > 0.0 { c.font_scale } else { 1.0 };
    let w = base.x; // keep the estimate's width; only the height was wrong
    let wrap_w = (w - PAD * 2.0).max(1.0);
    // Same text the CommonMark view shows: image markers → alt text, zero-width
    // markup (`*`, `` ` ``) dropped. Single newlines already break lines in a
    // galley, matching the card's hard-wrap render.
    let text = crate::model::strip_size_markup(&crate::model::strip_inline_markers(&c.body));
    let font = egui::FontId::proportional(14.0 * fs);
    let galley = ctx.fonts(|f| f.layout(text, font, egui::Color32::WHITE, wrap_w));
    let mut content_h = galley.size().y;
    for (_iw, ih) in c.inline_image_sizes(wrap_w) {
        content_h += ih + 6.0; // inline images stack under the text
    }
    let h = (TITLE_H + PAD * 2.0 + content_h).clamp(MIN_H, MAX_H);
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
    dialog_parent: Option<DialogParent>,
    /// Full-screen image viewer, opened by double-clicking an image card image.
    lightbox: Option<Lightbox>,
    dirty: bool,
    /// Autosave: when on, changes are written to disk shortly after you pause.
    autosave: bool,
    /// When the document last changed, for the autosave idle-debounce.
    last_change: Option<Instant>,
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
    /// Backlinks panel: cards that `[[link]]` to the selected node.
    backlinks_open: bool,
    /// Kanban board window: cards grouped by `status::`, drag between columns.
    kanban_open: bool,
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
    templates: Vec<crate::model::CardExport>,
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

    /// Per-node undo/redo history. Each entry snapshots one node before a canvas
    /// edit (moves, autosort, add/remove, etc.); a whole drag coalesces into one.
    undo: Vec<(NodeId, crate::model::Node)>,
    redo: Vec<(NodeId, crate::model::Node)>,
    /// Coalesce key for the in-progress gesture, so a drag is one undo step.
    undo_coalesce: Option<&'static str>,
}

impl TrellisApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        setup_fonts(&cc.egui_ctx);
        let autosave_path = default_autosave_path();

        // Reopen the document from the last session if possible; otherwise fall
        // back to the autosave slot, then to a fresh welcome document.
        let last_path = cc
            .storage
            .and_then(|s| s.get_string(LAST_DOC_KEY))
            .map(PathBuf::from);
        let mut doc_path: Option<PathBuf> = None;
        let mut doc: Option<Document> = None;
        if let Some(p) = &last_path {
            if let Ok(d) = read_document(p) {
                doc = Some(d);
                doc_path = Some(p.clone());
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

        // Agent API: load config, then start the localhost server. It binds
        // regardless of key so toggling the key in Settings works live; requests
        // are rejected while the key is empty.
        let api_key = cc
            .storage
            .and_then(|s| s.get_string(API_KEY_KEY))
            .unwrap_or_default();
        let api_port = cc
            .storage
            .and_then(|s| s.get_string(API_PORT_KEY))
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_API_PORT);
        let api_lan = cc
            .storage
            .and_then(|s| s.get_string(API_LAN_KEY))
            .map(|s| s == "true")
            .unwrap_or(false);
        let api_shared_key = Arc::new(Mutex::new(api_key.clone()));
        let (api_tx, api_rx) = std::sync::mpsc::channel::<ApiCommand>();
        let doc_revision = Arc::new(AtomicU64::new(0));
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
        ) {
            Ok(server) => (Some(server), api_status_line(api_lan, api_port)),
            Err(e) => (None, format!("Failed to start on port {api_port}: {e}")),
        };

        Self {
            doc,
            selected,
            views: HashMap::new(),
            md_cache: CommonMarkCache::default(),
            tex_cache: TextureCache::default(),
            renaming: None,
            doc_path,
            autosave_path,
            dialog_parent: None,
            lightbox: None,
            dirty: false,
            autosave,
            last_change: None,
            saving: false,
            save_tx,
            save_rx,
            status: "Ready".to_string(),
            backup_cfg,
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
            backlinks_open: false,
            kanban_open: false,
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
                self.mark_dirty();
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
                self.mark_dirty();
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
                        self.mark_dirty();
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
                        self.mark_dirty();
                        self.status = "Snip added as an image card".into();
                    }
                }
                Ok(_) => self.status = "Snip cancelled".into(),
                Err(e) => self.status = format!("Snip failed: {e}"),
            }
        }
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
            let (changed, resp) = api::process(&mut self.doc, cmd.req);
            if changed {
                self.mark_dirty();
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
                Some(api::ApiResponse::ok(serde_json::json!({ "count": snaps.len(), "snapshots": snaps })))
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
                        self.mark_dirty();
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
                        serde_json::json!({ "index": i, "title": t.title, "kind": t.kind.label() })
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
                let Some(json) = self.doc.export_card_json(*node, *card) else {
                    return Some(api::ApiResponse::err(404, "node or card not found"));
                };
                let Some(mut exp) = crate::model::parse_card_export(&json) else {
                    return Some(api::ApiResponse::err(500, "could not build a template from that card"));
                };
                if let Some(t) = title {
                    if !t.trim().is_empty() {
                        exp.title = t.clone();
                    }
                }
                let name = if exp.title.trim().is_empty() {
                    exp.kind.label().to_string()
                } else {
                    exp.title.clone()
                };
                let index = self.templates.len();
                self.templates.push(exp);
                Some(api::ApiResponse::ok(serde_json::json!({ "index": index, "title": name })))
            }
            // Stamp a saved template into a basket as a new card (mirrors "Insert
            // template"). Returns the created card.
            api::ApiRequest::TemplateInsert { index, node, pos } => {
                let Some(exp) = self.templates.get(*index).cloned() else {
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
                        self.mark_dirty();
                        let card = self.doc.card(*node, cid).map(api::card_json);
                        Some(api::ApiResponse::ok(
                            serde_json::json!({ "node": node, "card": card }),
                        ))
                    }
                    None => Some(api::ApiResponse::err(500, "could not insert template")),
                }
            }
            api::ApiRequest::TemplateDelete(index) => {
                if *index >= self.templates.len() {
                    return Some(api::ApiResponse::err(404, "no template at that index"));
                }
                let t = self.templates.remove(*index);
                Some(api::ApiResponse::ok(
                    serde_json::json!({ "deleted": index, "title": t.title }),
                ))
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

    /// Synchronous save — only for `on_exit`, where a background thread would be
    /// killed before it finished. Interactive/auto saves use `spawn_save` so the
    /// serialize + gzip + write never blocks the UI thread (they can take seconds
    /// on a large document).
    fn write_to(&mut self, path: PathBuf) {
        match serialize_and_write(&self.doc, &path) {
            Ok(_) => {
                write_history_snapshot(&path);
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
        std::thread::spawn(move || {
            let res = serialize_and_write(&doc, &path);
            if res.is_ok() {
                write_history_snapshot(&path);
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

    fn apply_tree(&mut self, actions: Vec<TreeAction>) {
        // Selection and the reorder-mode toggle aren't document edits.
        if actions.iter().any(|a| {
            !matches!(
                a,
                TreeAction::Select(_)
                    | TreeAction::ToggleReorder
                    | TreeAction::ExportBasket(..)
                    | TreeAction::ExportBasketPdf(_)
                    | TreeAction::ExportBasketPng(_)
                    | TreeAction::ImportBasket(_)
            )
        }) {
            self.mark_dirty();
        }
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
            }
        }
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
        // ResetView only nudges the (unsaved) pan, so it must not dirty the doc.
        if actions.iter().any(|a| {
            !matches!(
                a,
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
                    | CanvasAction::DeleteTemplate(_)
            )
        }) {
            self.mark_dirty();
        }
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
                    if let Some(json) = self.doc.export_card_json(node, cid) {
                        if let Some(exp) = crate::model::parse_card_export(&json) {
                            let name = if exp.title.trim().is_empty() {
                                exp.kind.label().to_string()
                            } else {
                                exp.title.clone()
                            };
                            self.templates.push(exp);
                            self.status = format!("Saved template \"{name}\"");
                        }
                    }
                }
                CanvasAction::InsertTemplate(idx, pos) => {
                    if let Some(exp) = self.templates.get(idx).cloned() {
                        let name = exp.title.clone();
                        if self.doc.add_card_from_export(node, pos, exp).is_some() {
                            self.status = format!("Inserted template \"{name}\"");
                        }
                    }
                }
                CanvasAction::DeleteTemplate(idx) => {
                    if idx < self.templates.len() {
                        let t = self.templates.remove(idx);
                        self.status = format!("Deleted template \"{}\"", t.title);
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
                    if self.doc.table_set_cell(node, cid, r, c, text) {
                        self.mark_dirty();
                    }
                }
                CanvasAction::TableSetBg(cid, r, c, bg) => {
                    if self.doc.table_set_bg(node, cid, r, c, bg) {
                        self.mark_dirty();
                    }
                }
                CanvasAction::TableSetFg(cid, r, c, fg) => {
                    if self.doc.table_set_fg(node, cid, r, c, fg) {
                        self.mark_dirty();
                    }
                }
                CanvasAction::TableInsertRow(cid, at) => {
                    if self.doc.table_insert_row(node, cid, at) {
                        self.mark_dirty();
                    }
                }
                CanvasAction::TableRemoveRow(cid, at) => {
                    if self.doc.table_remove_row(node, cid, at) {
                        self.mark_dirty();
                    }
                }
                CanvasAction::TableInsertCol(cid, at) => {
                    if self.doc.table_insert_col(node, cid, at) {
                        self.mark_dirty();
                    }
                }
                CanvasAction::TableRemoveCol(cid, at) => {
                    if self.doc.table_remove_col(node, cid, at) {
                        self.mark_dirty();
                    }
                }
                CanvasAction::TableSetColWidth(cid, c, w) => {
                    if self.doc.table_set_col_width(node, cid, c, w) {
                        self.mark_dirty();
                    }
                }
                CanvasAction::TableToggleHeader(cid) => {
                    if self.doc.table_toggle_header(node, cid) {
                        self.mark_dirty();
                    }
                }
                CanvasAction::TableImport(cid) => self.table_import(node, cid),
                CanvasAction::TableExportCsv(cid) => self.table_export(node, cid, false),
                CanvasAction::TableExportXlsx(cid) => self.table_export(node, cid, true),
                CanvasAction::RemoveImage(cid, idx) => {
                    if self.doc.remove_image(node, cid, idx) {
                        self.tex_cache.forget(cid);
                        self.mark_dirty();
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
                CanvasAction::DockCard(child, anchor) => self.doc.dock_card(node, child, anchor),
                CanvasAction::DetachCard(cid) => self.doc.detach_card(node, cid),
                CanvasAction::ResetView => {
                    self.views.insert(node, TSTransform::IDENTITY);
                }
            }
        }
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
                    if ui.button("Version history…").clicked() {
                        self.show_history = true;
                        ui.close_menu();
                    }
                    if ui.button("Backup…").clicked() {
                        self.show_backup = true;
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

                    ui.label("Port");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.api_port).range(1024..=65535));
                        ui.weak("(restart to apply a port change)");
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
                ui.collapsing("Endpoints", |ui| {
                    for line in [
                        "GET    /api/health                        (no auth)",
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
                        "POST   /api/nodes/{id}/cards    {kind, title?, body?, lang?, items?, pos?, size?, fit?, image_base64?, inline_images?}",
                        "PATCH  /api/nodes/{id}/cards/{cid}       {title?, body?, kind?, color?, font_scale?, fit?, pos?, size?, items?, …}",
                        "DELETE /api/nodes/{id}/cards/{cid}",
                        "POST   /api/nodes/{id}/cards/{cid}/move  {before|after|index|to}",
                        "POST   /api/nodes/{id}/cards/{cid}/property {key, value}   (set key:: value)",
                        "POST   /api/nodes/{id}/cards/{cid}/dock  {anchor}          (unstick: DELETE …/dock)",
                        "POST   /api/nodes/{id}/cards/{cid}/group {group}           (remove: DELETE …/group)",
                        "POST   /api/nodes/{id}/cards/{cid}/table {op, …}           (set_cell / insert_row / …)",
                        "POST   /api/nodes/{id}/cards/{cid}/sketch {op, …}          (add_stroke / undo / clear)",
                        "POST   /api/nodes/{id}/cards/{cid}/images {data_base64}    (GET / DELETE …/images/{idx})",
                        "GET    /api/nodes/{id}/groups             (POST create {cards,title?} / PATCH / DELETE {gid})",
                        "POST   /api/nodes/{id}/autosort",
                        "GET    /api/search?q=...                  (hits carry node + card)",
                        "GET    /api/tags[?name=<tag>]             (all tags / cards with a tag)",
                        "GET    /api/properties[?key=<k>&value=<v>]   (keys / matching cards)",
                        "GET    /api/query?tag=&key=&value=&text=  (combined card query)",
                        "GET    /api/tasks[?all=true]              (due:: agenda, bucketed)",
                        "POST   /api/ocr                           (OCR all un-OCR'd images)",
                        "GET    /api/export?format=markdown|html|json|pdf|png|gif",
                        "GET    /api/wait?rev=<n>                  (long-poll for changes)",
                        "GET    /api/history                       (version snapshots)",
                        "POST   /api/history/restore     {file}    (restore a snapshot)",
                        "GET    /api/backup                        (status)",
                        "POST   /api/backup/run                    (back up now)",
                        "GET    /api/templates                     (saved card templates)",
                        "POST   /api/templates          {node, card, title?}   (save a card as a template)",
                        "POST   /api/templates/{i}/insert {node, pos?}         (stamp it into a basket)",
                        "DELETE /api/templates/{i}",
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
            ui.separator();
            tasks.retain(|t| self.agenda_show_done || !t.done);
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
                        let row = ui.add(
                            egui::Label::new(format!("  {}  ", t.due))
                                .sense(egui::Sense::click()),
                        );
                        if ui.add(egui::Label::new(title).sense(egui::Sense::click())).clicked() || row.clicked() {
                            jump = Some((t.node, t.card));
                        }
                        ui.small(egui::RichText::new(&t.node_title).weak());
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
        let board = self.doc.cards_by_status();
        // Standard columns first, then any other statuses in use.
        let mut cols: Vec<String> = ["todo", "doing", "done"].iter().map(|s| s.to_string()).collect();
        for k in board.keys() {
            if !cols.contains(k) {
                cols.push(k.clone());
            }
        }
        let mut jump: Option<(NodeId, CardId)> = None;
        let mut moves: Vec<(NodeId, CardId, String)> = Vec::new();
        egui::Window::new("Kanban board")
            .open(&mut open)
            .default_size([760.0, 480.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.small("Cards with a status:: property. Drag a card to another column to change its status.");
                if board.is_empty() {
                    ui.weak("No cards have a status:: property yet. Add `status:: todo` to a card.");
                }
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let empty = Vec::new();
                        for col in &cols {
                            let cards = board.get(col).unwrap_or(&empty);
                            let resp = ui
                                .allocate_ui(egui::vec2(210.0, ui.available_height().max(120.0)), |ui| {
                                    ui.group(|ui| {
                                        ui.set_min_width(196.0);
                                        ui.set_min_height(110.0);
                                        ui.strong(format!("{col}  ({})", cards.len()));
                                        ui.separator();
                                        for &(node, card, ref title, ref nt) in cards {
                                            let src = ui.dnd_drag_source(
                                                egui::Id::new(("kb", node, card)),
                                                (node, card),
                                                |ui| {
                                                    ui.group(|ui| {
                                                        ui.set_min_width(176.0);
                                                        ui.label(title);
                                                        ui.small(egui::RichText::new(nt).weak());
                                                    });
                                                },
                                            );
                                            if src.response.clicked() {
                                                jump = Some((node, card));
                                            }
                                        }
                                    })
                                    .response
                                })
                                .response;
                            if let Some(p) = resp.dnd_release_payload::<(NodeId, CardId)>() {
                                moves.push((p.0, p.1, col.clone()));
                            }
                        }
                    });
                });
            });
        self.kanban_open = open;
        for (n, c, status) in moves {
            if self.doc.set_card_property(n, c, "status", &status) {
                self.mark_dirty();
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

        // Apply any API requests from the server thread first.
        self.pump_api();
        // Apply any finished background OCR results.
        self.pump_ocr();
        // Turn finished region-snips into image cards.
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
                let actions = tree::ui(
                    ui,
                    &self.doc,
                    self.selected,
                    &mut self.renaming,
                    self.reorder_mode,
                    scroll_to,
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
                        self.templates.iter().map(|t| t.title.clone()).collect();
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
        storage.set_string(BACKUP_KEY, self.backup_cfg.to_json());
        if let Ok(s) = serde_json::to_string(&self.templates) {
            storage.set_string(TEMPLATES_KEY, s);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Best-effort autosave to the working file (or the autosave slot).
        let path = self.target_path();
        self.write_to(path);
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

/// A random API key (48 hex chars from the OS RNG, falling back to a weak
/// time/pid mix if `/dev/urandom` is unavailable).
/// Capture a user-selected screen region to PNG bytes, trying the region-select
/// screenshot tools commonly present on Linux desktops in turn. An empty `Vec`
/// means the user cancelled the selection.
fn capture_region() -> Result<Vec<u8>, String> {
    let out = std::env::temp_dir().join(format!("trellis-snip-{}.png", std::process::id()));
    let path = out.to_string_lossy().to_string();
    // (binary, args) — each does interactive region select and writes `path`.
    let candidates: [(&str, Vec<&str>); 5] = [
        ("spectacle", vec!["-b", "-n", "-r", "-o", &path]),
        ("gnome-screenshot", vec!["-a", "-f", &path]),
        ("maim", vec!["-s", &path]),
        ("scrot", vec!["-s", &path]),
        ("import", vec![&path]), // ImageMagick: click-drag a region
    ];
    let mut tried = Vec::new();
    for (bin, args) in candidates {
        match std::process::Command::new(bin).args(&args).status() {
            Ok(status) => {
                if !status.success() {
                    // Non-zero usually means the user pressed Esc to cancel.
                    let _ = std::fs::remove_file(&out);
                    return Ok(Vec::new());
                }
                let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(&out);
                return Ok(bytes);
            }
            Err(_) => tried.push(bin),
        }
    }
    Err(format!("no screenshot tool found (tried {}); install one, e.g. maim or scrot", tried.join(", ")))
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
            Err(e) => return Err(format!("tesseract not found ({e}); install tesseract-ocr")),
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
fn format_stamp(name: &str) -> String {
    let s = name.split('.').next().unwrap_or(name); // strip .ron.gz
    let b = s.as_bytes();
    if b.len() == 15 && b[8] == b'-' && s.is_char_boundary(15) {
        format!(
            "{}-{}-{} {}:{}:{}",
            &s[0..4], &s[4..6], &s[6..8], &s[9..11], &s[11..13], &s[13..15]
        )
    } else {
        name.to_string()
    }
}

fn serialize_doc(doc: &Document) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let s = ron::to_string(doc).map_err(|e| e.to_string())?;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(s.as_bytes()).map_err(|e| e.to_string())?;
    enc.finish().map_err(|e| e.to_string())
}

/// How many version snapshots to keep, and the minimum gap between them so a
/// burst of autosaves doesn't churn through the whole history in a minute.
const HISTORY_KEEP: usize = 25;
const HISTORY_MIN_GAP_SECS: u64 = 180;

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
fn write_history_snapshot(doc_path: &std::path::Path) {
    let Some(dir) = history_dir(doc_path) else { return };
    let snaps = history_snapshots(doc_path);
    if let Some((newest, _)) = snaps.first() {
        if let Ok(age) = newest.metadata().and_then(|m| m.modified()).and_then(|t| t.elapsed().map_err(std::io::Error::other)) {
            if age.as_secs() < HISTORY_MIN_GAP_SECS {
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
    // Prune oldest beyond the cap.
    let snaps = history_snapshots(doc_path);
    for (path, _) in snaps.into_iter().skip(HISTORY_KEEP) {
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
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf))
        .is_ok();
    if ok {
        buf.iter().map(|b| format!("{b:02x}")).collect()
    } else {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("trellis-{}-{:x}", std::process::id(), t)
    }
}

fn default_autosave_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "Trellis", "Trellis")
        .map(|d| d.data_dir().join("autosave.ron"))
        .unwrap_or_else(|| PathBuf::from("trellis-autosave.ron"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_name_keeps_stored_name_and_extension() {
        assert_eq!(download_image_name("photo.jpg", 0), "photo.jpg");
        assert_eq!(download_image_name("scan.PNG", 3), "scan.PNG");
    }

    #[test]
    fn format_stamp_is_human_readable() {
        assert_eq!(format_stamp("20260730-142530.ron.gz"), "2026-07-30 14:25:30");
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
