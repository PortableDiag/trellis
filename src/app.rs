//! Application state and the eframe update loop that stitches the panels
//! together: menu bar, tree, basket canvas, search, and all file operations.

use crate::canvas::{self, CanvasAction, Env};
use crate::images::TextureCache;
use crate::model::{CardKind, ChecklistItem, Document, NodeId};
use crate::tree::{self, TreeAction};
use crate::api::{self, ApiCommand};
use egui_commonmark::CommonMarkCache;
use std::collections::HashMap;
use std::path::PathBuf;
use raw_window_handle::{HasDisplayHandle as _, HasWindowHandle as _};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

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
const DEFAULT_API_PORT: u16 = 7373;
const ZOOM_ENABLED_KEY: &str = "zoom_enabled";
const DOCK_MODE_KEY: &str = "dock_mode";
const SNAP_MODE_KEY: &str = "snap_mode";
const THEME_KEY: &str = "theme";

/// Selectable color schemes. Dark/Light are egui's built-ins; add new variants
/// here (and to `ALL`) to grow the list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Theme {
    Dark,
    Light,
    TerminalGreen,
}

impl Theme {
    const ALL: [(Theme, &'static str); 3] = [
        (Theme::Dark, "Dark"),
        (Theme::Light, "Light"),
        (Theme::TerminalGreen, "Terminal Green"),
    ];

    fn from_key(s: &str) -> Theme {
        match s {
            "Light" => Theme::Light,
            "TerminalGreen" => Theme::TerminalGreen,
            _ => Theme::Dark,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::TerminalGreen => "TerminalGreen",
        }
    }

    fn visuals(self) -> egui::Visuals {
        match self {
            Theme::Light => egui::Visuals::light(),
            Theme::Dark => egui::Visuals::dark(),
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
    status: String,

    search_open: bool,
    search_query: String,
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
    /// A copied card, ready to paste into any basket.
    card_clipboard: Option<crate::model::Card>,
    /// Runtime multi-selection of cards in the current basket, used to build a
    /// group. Cleared when the selected node changes. Never persisted.
    card_sel: std::collections::HashSet<crate::model::CardId>,
    /// Which node `card_sel` belongs to, so it resets when the basket changes.
    card_sel_node: Option<NodeId>,

    // Agent HTTP API.
    api_rx: Option<Receiver<ApiCommand>>,
    /// Shared with the server thread so key edits take effect without a restart.
    api_shared_key: Arc<Mutex<String>>,
    api_key: String,
    api_port: u16,
    api_status: String,
    show_settings: bool,
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
            if let Ok(d) = std::fs::read_to_string(p)
                .map_err(|_| ())
                .and_then(|s| ron::from_str::<Document>(&s).map_err(|_| ()))
            {
                doc = Some(d);
                doc_path = Some(p.clone());
            }
        }
        let doc = doc
            .or_else(|| {
                std::fs::read_to_string(&autosave_path)
                    .ok()
                    .and_then(|s| ron::from_str::<Document>(&s).ok())
            })
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
        let theme = cc
            .storage
            .and_then(|s| s.get_string(THEME_KEY))
            .map(|s| Theme::from_key(&s))
            .unwrap_or(Theme::Dark);

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
        let api_shared_key = Arc::new(Mutex::new(api_key.clone()));
        let (api_tx, api_rx) = std::sync::mpsc::channel::<ApiCommand>();
        let api_status = match api::serve(
            api_port,
            cc.egui_ctx.clone(),
            api_tx,
            Arc::clone(&api_shared_key),
        ) {
            Ok(()) => format!("Listening on http://127.0.0.1:{api_port}/api"),
            Err(e) => format!("Failed to start on port {api_port}: {e}"),
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
            status: "Ready".to_string(),
            search_open: false,
            search_query: String::new(),
            show_about: false,
            theme,
            zoom_enabled,
            reorder_mode: false,
            dock_mode,
            snap_mode,
            card_clipboard: None,
            card_sel: std::collections::HashSet::new(),
            card_sel_node: None,
            api_rx: Some(api_rx),
            api_shared_key,
            api_key,
            api_port,
            api_status,
            show_settings: false,
        }
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

    /// Drain and apply any pending API commands from the server thread.
    fn pump_api(&mut self) {
        let mut cmds = Vec::new();
        if let Some(rx) = &self.api_rx {
            while let Ok(cmd) = rx.try_recv() {
                cmds.push(cmd);
            }
        }
        for cmd in cmds {
            let (changed, resp) = api::process(&mut self.doc, cmd.req);
            if changed {
                self.dirty = true;
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

    fn write_to(&mut self, path: PathBuf) {
        match ron::ser::to_string_pretty(&self.doc, ron::ser::PrettyConfig::default()) {
            Ok(s) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                match std::fs::write(&path, s) {
                    Ok(_) => {
                        self.dirty = false;
                        self.status = format!("Saved → {}", path.display());
                    }
                    Err(e) => self.status = format!("Save failed: {e}"),
                }
            }
            Err(e) => self.status = format!("Serialize failed: {e}"),
        }
    }

    fn save(&mut self) {
        let path = self.target_path();
        self.write_to(path);
    }

    fn save_as(&mut self) {
        if let Some(path) = self.file_dialog()
            .add_filter("Trellis document", &["ron"])
            .set_file_name("untitled.ron")
            .save_file()
        {
            self.doc_path = Some(path.clone());
            self.write_to(path);
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

    fn new_document(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        self.doc = Document::default();
        self.selected = self.doc.roots.first().copied();
        self.views.clear();
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
            match std::fs::read_to_string(&path).map(|s| ron::from_str::<Document>(&s)) {
                Ok(Ok(doc)) => {
                    self.doc = doc;
                    self.selected = self.doc.roots.first().copied();
                    self.views.clear();
                    self.doc_path = Some(path.clone());
                    self.dirty = false;
                    self.status = format!("Opened {}", path.display());
                }
                Ok(Err(e)) => self.status = format!("Parse error: {e}"),
                Err(e) => self.status = format!("Read error: {e}"),
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
                    self.dirty = true;
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
                    self.doc = doc;
                    self.selected = self.doc.roots.first().copied();
                    self.views.clear();
                    self.doc_path = None;
                    self.dirty = true;
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
        if actions
            .iter()
            .any(|a| !matches!(a, TreeAction::Select(_) | TreeAction::ToggleReorder))
        {
            self.dirty = true;
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
                TreeAction::MoveUp(id) => self.doc.move_sibling(id, true),
                TreeAction::MoveDown(id) => self.doc.move_sibling(id, false),
                TreeAction::MoveToTop(id) => self.doc.move_to_edge(id, true),
                TreeAction::MoveToBottom(id) => self.doc.move_to_edge(id, false),
                TreeAction::Reorder { moved, target, before } => {
                    self.doc.reorder(moved, target, before)
                }
                TreeAction::ToggleReorder => self.reorder_mode = !self.reorder_mode,
                TreeAction::Indent(id) => self.doc.indent(id),
                TreeAction::Outdent(id) => self.doc.outdent(id),
                TreeAction::SetColor(id, col) => {
                    if let Some(n) = self.doc.nodes.get_mut(&id) {
                        n.color = col;
                    }
                }
            }
        }
    }

    /// Turn OS-dropped files into cards: images → an image card, anything that
    /// decodes as UTF-8 text (txt/md/source/…) → a text card holding the file's
    /// contents. Cards fan out from the drop position; unknown binaries are
    /// skipped.
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
                let kind = CardKind::Image { data: Vec::new(), name: String::new(), extra: Vec::new() };
                if let Some(cid) = self.doc.add_card(node, at, kind) {
                    self.doc.add_image(node, cid, bytes, name);
                    n += 1;
                }
            } else if let Ok(text) = String::from_utf8(bytes) {
                if let Some(cid) = self.doc.add_card(node, at, CardKind::Text) {
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
            self.dirty = true;
            self.status = format!("Added {n} card{} from dropped files", if n == 1 { "" } else { "s" });
        }
    }

    fn apply_canvas(&mut self, node: NodeId, actions: Vec<CanvasAction>) {
        // ResetView only nudges the (unsaved) pan, so it must not dirty the doc.
        if actions.iter().any(|a| {
            !matches!(
                a,
                CanvasAction::ResetView
                    | CanvasAction::CopyCard(_)
                    | CanvasAction::ToggleSelect(_)
                    | CanvasAction::ClearSelection
                    | CanvasAction::ToggleDockMode
                    | CanvasAction::ToggleSnapMode
            )
        }) {
            self.dirty = true;
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
                CanvasAction::LoadImage(cid) => self.load_image_into(node, cid),
                CanvasAction::TableSetCell(cid, r, c, text) => {
                    if self.doc.table_set_cell(node, cid, r, c, text) {
                        self.dirty = true;
                    }
                }
                CanvasAction::TableSetBg(cid, r, c, bg) => {
                    if self.doc.table_set_bg(node, cid, r, c, bg) {
                        self.dirty = true;
                    }
                }
                CanvasAction::TableSetFg(cid, r, c, fg) => {
                    if self.doc.table_set_fg(node, cid, r, c, fg) {
                        self.dirty = true;
                    }
                }
                CanvasAction::TableInsertRow(cid, at) => {
                    if self.doc.table_insert_row(node, cid, at) {
                        self.dirty = true;
                    }
                }
                CanvasAction::TableRemoveRow(cid, at) => {
                    if self.doc.table_remove_row(node, cid, at) {
                        self.dirty = true;
                    }
                }
                CanvasAction::TableInsertCol(cid, at) => {
                    if self.doc.table_insert_col(node, cid, at) {
                        self.dirty = true;
                    }
                }
                CanvasAction::TableRemoveCol(cid, at) => {
                    if self.doc.table_remove_col(node, cid, at) {
                        self.dirty = true;
                    }
                }
                CanvasAction::TableSetColWidth(cid, c, w) => {
                    if self.doc.table_set_col_width(node, cid, c, w) {
                        self.dirty = true;
                    }
                }
                CanvasAction::TableToggleHeader(cid) => {
                    if self.doc.table_toggle_header(node, cid) {
                        self.dirty = true;
                    }
                }
                CanvasAction::TableImport(cid) => self.table_import(node, cid),
                CanvasAction::TableExportCsv(cid) => self.table_export(node, cid, false),
                CanvasAction::TableExportXlsx(cid) => self.table_export(node, cid, true),
                CanvasAction::RemoveImage(cid, idx) => {
                    if self.doc.remove_image(node, cid, idx) {
                        self.tex_cache.forget(cid);
                        self.dirty = true;
                    }
                }
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
                    self.dirty = true;
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
                        self.dirty = true;
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
                    ui.menu_button("Theme", |ui| {
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
                            if self.doc.autosort(sel) {
                                self.dirty = true;
                                self.status = "Autosorted cards into a grid".to_string();
                            }
                        }
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
                    "A localhost HTTP API for agents to add, query, edit and remove \
                     nodes and cards. Bound to 127.0.0.1 only.",
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
                });

                ui.add_space(10.0);
                ui.heading("Canvas");
                ui.checkbox(
                    &mut self.zoom_enabled,
                    "Zoom with Ctrl+scroll and Ctrl +/−",
                )
                .on_hover_text("Ctrl+0 and Reset view still reset zoom when this is off.");
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
                ui.code(format!(
                    "curl -H 'X-API-Key: {}' \\\n     http://127.0.0.1:{}/api/tree",
                    if self.api_key.is_empty() { "<key>" } else { &self.api_key },
                    port
                ));
                ui.add_space(4.0);
                ui.collapsing("Endpoints", |ui| {
                    for line in [
                        "GET    /api/health                     (no auth)",
                        "GET    /api/tree",
                        "GET    /api/nodes",
                        "POST   /api/nodes            {parent?, title}",
                        "GET    /api/nodes/{id}",
                        "PATCH  /api/nodes/{id}       {title?, color?}",
                        "DELETE /api/nodes/{id}",
                        "GET    /api/nodes/{id}/cards",
                        "POST   /api/nodes/{id}/cards {kind, title?, body?, lang?, items?, pos?}",
                        "PATCH  /api/nodes/{id}/cards/{cid}  {title?, body?}",
                        "DELETE /api/nodes/{id}/cards/{cid}",
                        "GET    /api/search?q=...",
                    ] {
                        ui.monospace(line);
                    }
                });
            });
        self.show_settings = open;
    }

    fn sync_api_key(&mut self) {
        if let Ok(mut k) = self.api_shared_key.lock() {
            *k = self.api_key.clone();
        }
    }

    fn search_panel(&mut self, ctx: &egui::Context) {
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
                                self.selected = Some(hit.node);
                            }
                            ui.small(hit.snippet);
                        });
                        ui.separator();
                    }
                });
            });
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
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::N)) {
            self.new_document();
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

        egui::SidePanel::left("tree")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                let actions =
                    tree::ui(ui, &self.doc, self.selected, &mut self.renaming, self.reorder_mode);
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
                    let node = self.doc.nodes.get(&sel).unwrap();
                    let mut env = Env {
                        md: &mut self.md_cache,
                        tex: &mut self.tex_cache,
                    };
                    let can_paste = self.card_clipboard.is_some();
                    let actions = canvas::ui(
                        ui,
                        node,
                        &mut view,
                        self.zoom_enabled,
                        can_paste,
                        self.dock_mode,
                        self.snap_mode,
                        &mut env,
                        &self.card_sel,
                    );
                    self.views.insert(sel, view);
                    self.apply_canvas(sel, actions);
                } else {
                    self.selected = None;
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("No node selected. Add one on the left to start a basket.");
                });
            }
        });

        if self.show_about {
            egui::Window::new("About Trellis")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.heading("Trellis");
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
        storage.set_string(ZOOM_ENABLED_KEY, self.zoom_enabled.to_string());
        storage.set_string(DOCK_MODE_KEY, self.dock_mode.to_string());
        storage.set_string(SNAP_MODE_KEY, self.snap_mode.to_string());
        storage.set_string(THEME_KEY, self.theme.key().to_string());
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
