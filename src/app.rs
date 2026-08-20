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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
/// Desktop-mode window placements, `card id -> [x, y]` in screen pixels.
///
/// **App config, not the document.** A desktop position is screen geometry for
/// one machine; a document opened on another box, or read by the Android app,
/// must not carry it. Same rule that puts templates and the backup schedule
/// here rather than in the `.ron`.
const DESKTOP_CARDS_KEY: &str = "desktop_cards";
const GRANTS_KEY: &str = "plugin_grants";
const MIRROR_MODE_KEY: &str = "mirror_policy";
const MIRROR_DIRS_KEY: &str = "mirror_dirs";
pub(crate) const DEFAULT_API_PORT: u16 = 7373;
const ZOOM_ENABLED_KEY: &str = "zoom_enabled";
const DOCK_MODE_KEY: &str = "dock_mode";
const SNAP_MODE_KEY: &str = "snap_mode";
const GRID_MODE_KEY: &str = "grid_mode";
/// Depth (true Z) and Time (a card with extent in days). Both are **view** modes
/// and both default off: `z` and the span stay on the card whatever these say, so
/// turning one off flattens what you see rather than discarding what someone
/// arranged. A new user meets the canvas they have always had.
/// Which binary path we last registered as the `trellis://` handler.
///
/// Stored rather than a bare "done" flag so a **new install, or the same install
/// moved**, re-registers itself: a handler pointing at a binary that is no longer
/// there fails silently, and a link that does nothing is indistinguishable from
/// the feature not existing.
const URL_SCHEME_REGISTERED_KEY: &str = "url_scheme_registered";
/// Panel state the user expects to survive a restart.
///
/// Whether a panel is open is a *setting*, not a transient: the Agenda is easy to
/// forget exists when it closes itself every launch, and re-ticking "Show
/// completed" every time is the kind of small friction that makes a view feel
/// unreliable.
const AGENDA_OPEN_KEY: &str = "agenda_open";
const AGENDA_DONE_KEY: &str = "agenda_show_done";
const AGENDA_PLACE_KEY: &str = "agenda_placement";
const KANBAN_OPEN_KEY: &str = "kanban_open";
const KANBAN_DONE_KEY: &str = "kanban_show_done";
const KANBAN_PLACE_KEY: &str = "kanban_placement";
/// Do detached panels follow the main window when it moves?
const STICK_WINDOWS_KEY: &str = "stick_windows";
const TAGS_OPEN_KEY: &str = "tags_open";
const FIND_OPEN_KEY: &str = "find_open";
const BACKLINKS_OPEN_KEY: &str = "backlinks_open";
const CLAIMS_OPEN_KEY: &str = "claims_open";
const DEPTH_MODE_KEY: &str = "depth_mode";
const TIME_MODE_KEY: &str = "time_mode";
const MINIMAP_KEY: &str = "minimap";
/// Desktop notifications. Off by default: an app that starts popping up
/// system-wide the first time you run it has taken a decision that was not its
/// to take.
const NOTIFY_DIGEST_KEY: &str = "notify_digest";
const NOTIFY_AGENT_KEY: &str = "notify_agent";
/// How the root nodes are ordered in the tree. Persisted, because a sort you
/// have to re-pick every launch is not an ordering, it is a chore.
const TREE_SORT_KEY: &str = "tree_sort";
/// Journal root for daily notes. Absent = the feature is off, which is the
/// default and the whole point: a dated node must never appear unasked.
const DAILY_ROOT_KEY: &str = "daily_root";
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
    Blueprint,
    Silkscreen,
    Phosphor,
}

impl Theme {
    const ALL: [(Theme, &'static str); 9] = [
        (Theme::Trellis, "Trellis"),
        (Theme::Light, "Light"),
        (Theme::TerminalGreen, "Terminal Green"),
        (Theme::StickyNotes, "Sticky Notes"),
        (Theme::Futuristic, "Futuristic"),
        (Theme::SynthWave, "SynthWave"),
        (Theme::Blueprint, "Blueprint"),
        (Theme::Silkscreen, "Silkscreen"),
        (Theme::Phosphor, "Phosphor"),
    ];

    fn from_key(s: &str) -> Theme {
        match s {
            "Light" => Theme::Light,
            "TerminalGreen" => Theme::TerminalGreen,
            "StickyNotes" => Theme::StickyNotes,
            "Futuristic" => Theme::Futuristic,
            "SynthWave" => Theme::SynthWave,
            "Blueprint" => Theme::Blueprint,
            "Silkscreen" => Theme::Silkscreen,
            "Phosphor" => Theme::Phosphor,
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
            Theme::Blueprint => "Blueprint",
            Theme::Silkscreen => "Silkscreen",
            Theme::Phosphor => "Phosphor",
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
            Theme::Blueprint => blueprint_visuals(),
            Theme::Silkscreen => silkscreen_visuals(),
            Theme::Phosphor => phosphor_visuals(),
        }
    }
}

/// Drafting board: cyan linework on Prussian blue.
///
/// The canvas is already a board you arrange things on, so the metaphor is not
/// borrowed — it is what the app is. Everything is line, not fill: a drawing
/// reads by its edges, which is also why this stays legible for long reading
/// where the neon themes tire.
fn blueprint_visuals() -> egui::Visuals {
    use egui::{Color32, Stroke};
    let ink = Color32::from_rgb(0xdc, 0xef, 0xff); // near-white linework
    let line = Color32::from_rgb(0x7d, 0xc3, 0xf0); // cyan rule
    let dim = Color32::from_rgb(0x4a, 0x86, 0xb4);
    let board = Color32::from_rgb(0x0b, 0x2a, 0x43); // Prussian blue ground
    let sheet = Color32::from_rgb(0x11, 0x38, 0x57); // the drawing sheet

    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(ink);
    v.hyperlink_color = Color32::from_rgb(0x9f, 0xdc, 0xff);
    v.panel_fill = sheet;
    v.window_fill = sheet;
    v.extreme_bg_color = board;
    v.faint_bg_color = Color32::from_rgb(0x15, 0x42, 0x66);
    v.code_bg_color = Color32::from_rgb(0x09, 0x24, 0x3a);
    v.window_stroke = Stroke::new(1.0, line);
    v.selection.bg_fill = line.gamma_multiply(0.28);
    v.selection.stroke = Stroke::new(1.0, line);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = sheet;
    w.noninteractive.weak_bg_fill = sheet;
    w.noninteractive.fg_stroke = Stroke::new(1.0, dim);
    w.inactive.bg_fill = Color32::from_rgb(0x15, 0x42, 0x66);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x15, 0x42, 0x66);
    w.inactive.fg_stroke = Stroke::new(1.0, ink);
    w.hovered.bg_fill = Color32::from_rgb(0x1b, 0x51, 0x7c);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x1b, 0x51, 0x7c);
    w.hovered.fg_stroke = Stroke::new(1.5, ink);
    w.hovered.bg_stroke = Stroke::new(1.0, line);
    w.active.bg_fill = Color32::from_rgb(0x22, 0x63, 0x96);
    w.active.weak_bg_fill = Color32::from_rgb(0x22, 0x63, 0x96);
    w.active.fg_stroke = Stroke::new(1.5, ink);
    w.active.bg_stroke = Stroke::new(1.0, line);
    w.open.fg_stroke = Stroke::new(1.0, ink);
    v
}

/// Solder mask and silkscreen: gold on dark board green.
///
/// The one theme where the *structure* is visible in the look — a docked or
/// grouped card is a part with a trace running to it, which is exactly what
/// docking means.
fn silkscreen_visuals() -> egui::Visuals {
    use egui::{Color32, Stroke};
    let silk = Color32::from_rgb(0xf2, 0xf4, 0xef); // white silkscreen legend
    let gold = Color32::from_rgb(0xd9, 0xa8, 0x2b); // ENIG pad / trace
    let dim = Color32::from_rgb(0x8f, 0x6f, 0x22);
    let mask = Color32::from_rgb(0x0a, 0x2e, 0x1e); // solder mask green
    let board = Color32::from_rgb(0x06, 0x1f, 0x15);

    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(silk);
    v.hyperlink_color = Color32::from_rgb(0xf0, 0xc8, 0x62);
    v.panel_fill = mask;
    v.window_fill = mask;
    v.extreme_bg_color = board;
    v.faint_bg_color = Color32::from_rgb(0x0e, 0x3a, 0x27);
    v.code_bg_color = board;
    v.window_stroke = Stroke::new(1.0, dim);
    v.selection.bg_fill = gold.gamma_multiply(0.24);
    v.selection.stroke = Stroke::new(1.0, gold);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = mask;
    w.noninteractive.weak_bg_fill = mask;
    w.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0xb9, 0xc6, 0xbd));
    w.inactive.bg_fill = Color32::from_rgb(0x0e, 0x3a, 0x27);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x0e, 0x3a, 0x27);
    w.inactive.fg_stroke = Stroke::new(1.0, silk);
    w.hovered.bg_fill = Color32::from_rgb(0x14, 0x4a, 0x33);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x14, 0x4a, 0x33);
    w.hovered.fg_stroke = Stroke::new(1.5, silk);
    w.hovered.bg_stroke = Stroke::new(1.0, gold);
    w.active.bg_fill = Color32::from_rgb(0x1a, 0x5c, 0x40);
    w.active.weak_bg_fill = Color32::from_rgb(0x1a, 0x5c, 0x40);
    w.active.fg_stroke = Stroke::new(1.5, silk);
    w.active.bg_stroke = Stroke::new(1.0, gold);
    w.open.fg_stroke = Stroke::new(1.0, silk);
    v
}

/// A storage oscilloscope: P31 blue-green traces on a graticule.
///
/// Distinct from Terminal Green on purpose — that is a *console*, this is an
/// *instrument*: no fill anywhere, everything drawn as a trace, and the grid
/// read as a graticule rather than as a background texture.
fn phosphor_visuals() -> egui::Visuals {
    use egui::{Color32, Stroke};
    let trace = Color32::from_rgb(0x7a, 0xf7, 0xd4); // P31 blue-green
    let dim = Color32::from_rgb(0x3f, 0xa8, 0x92);
    let bg = Color32::from_rgb(0x02, 0x0a, 0x09);
    let panel = Color32::from_rgb(0x05, 0x14, 0x11);

    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(trace);
    v.hyperlink_color = Color32::from_rgb(0xa8, 0xff, 0xe6);
    v.panel_fill = panel;
    v.window_fill = panel;
    v.extreme_bg_color = bg;
    v.faint_bg_color = Color32::from_rgb(0x07, 0x1c, 0x18);
    v.code_bg_color = bg;
    v.window_stroke = Stroke::new(1.0, dim);
    v.selection.bg_fill = trace.gamma_multiply(0.20);
    v.selection.stroke = Stroke::new(1.0, trace);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = panel;
    w.noninteractive.weak_bg_fill = panel;
    w.noninteractive.fg_stroke = Stroke::new(1.0, dim);
    w.inactive.bg_fill = Color32::from_rgb(0x08, 0x20, 0x1b);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x08, 0x20, 0x1b);
    w.inactive.fg_stroke = Stroke::new(1.0, trace);
    w.hovered.bg_fill = Color32::from_rgb(0x0b, 0x2c, 0x25);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x0b, 0x2c, 0x25);
    w.hovered.fg_stroke = Stroke::new(1.5, trace);
    w.hovered.bg_stroke = Stroke::new(1.0, dim);
    w.active.bg_fill = Color32::from_rgb(0x0f, 0x38, 0x2f);
    w.active.weak_bg_fill = Color32::from_rgb(0x0f, 0x38, 0x2f);
    w.active.fg_stroke = Stroke::new(1.5, trace);
    w.active.bg_stroke = Stroke::new(1.0, trace);
    w.open.fg_stroke = Stroke::new(1.0, trace);
    v
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
        A::PlaceCards(..) => UndoKind::Discrete,
        A::ExtractSelection(..) => UndoKind::Discrete,
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
                    // Same reason as the agenda: a board column is narrow and a
                    // task can carry its context in its text.
                    ui.label(egui::RichText::new(elide(&kc.title, 70)).strong())
                        .on_hover_text(&kc.title);
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

/// The binary to relaunch on **File → Restart**, which is only ever used
/// *because* the binary has been replaced.
///
/// `current_exe()` alone gets this exactly backwards. On Linux it reads
/// `/proc/self/exe`, which follows the **inode** the process is running, not the
/// path — and installing a new build unlinks that inode, so the link reads
/// `…/trellis (deleted)`. Restart then either fails with `No such file or
/// directory` or relaunches the old build, which is how "Restart only works if
/// the version hasn't changed" was reported: the one case it exists for was the
/// one case it could not do.
///
/// So the deleted marker is stripped and the path taken only if something is
/// there now — that file *is* the new build. `argv[0]` is the fallback, since a
/// desktop entry and both launch scripts pass an absolute path.
fn exe_for_restart() -> Option<std::path::PathBuf> {
    pick_exe(
        std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string()),
        std::env::args().next(),
        |p| p.is_file(),
    )
}

/// The choosing half of [`exe_for_restart`], with the filesystem passed in so
/// the ordering and the `(deleted)` stripping can be tested.
fn pick_exe(
    current: Option<String>, argv0: Option<String>, exists: impl Fn(&std::path::Path) -> bool,
) -> Option<std::path::PathBuf> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(s) = current {
        candidates.push(std::path::PathBuf::from(
            s.strip_suffix(" (deleted)").unwrap_or(&s).to_string(),
        ));
    }
    if let Some(a0) = argv0.filter(|a| a.contains('/')) {
        candidates.push(std::path::PathBuf::from(a0));
    }
    candidates.into_iter().find(|p| exists(p))
}

/// What a template's *content* is, for telling an edited master from an
/// untouched one.
///
/// Size and depth are excluded on purpose: dragging a master, or letting Fit
/// resize it, is layout rather than a change to the template. Image bytes are
/// excluded because comparing megabytes of base64 on every frame to learn what
/// the file name already says is not worth it — so swapping in a different
/// picture under the same name is the one edit this will not notice.
fn template_key(e: &crate::model::CardExport) -> String {
    let mut e = e.clone();
    e.size = [0.0, 0.0];
    e.z = 0.0;
    for img in &mut e.inline_images {
        img.data.clear();
    }
    if let crate::model::CardKind::Image { data, extra, .. } = &mut e.kind {
        data.clear();
        for x in extra.iter_mut() {
            x.data.clear();
        }
    }
    serde_json::to_string(&e).unwrap_or_default()
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

/// What a new agent token is scoped to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TokenTarget {
    /// Make a root basket named after the agent and confine it there. The
    /// default, because "this agent gets its own workspace" is the case worth
    /// making easy — the alternative is issuing a whole-document token because
    /// creating a basket first was a separate chore.
    NewBasket,
    /// Confine it to a basket that already exists.
    Existing(NodeId),
    /// No confinement. Deliberately the awkward option.
    WholeDocument,
}

/// What a plugin worker thread sends back.
///
/// Progress and completion share one channel so they can't arrive out of order —
/// a late progress line landing after the "done" would otherwise leave a
/// finished plugin showing a half-full bar forever.
pub enum PluginEvent {
    Progress(crate::plugins::Progress),
    Done(crate::plugins::RunResult),
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

/// The tallest a window's scrolling body may be.
///
/// The screen, less room for the title bar, the frame and a margin at each end.
/// A window whose content simply grows is fine until the content is a list that
/// grows too: the Settings window is anchored to the centre and not resizable, so
/// an expanded `Endpoints` section ran off both ends of the display with no way to
/// reach either. Capping the body and letting it scroll is what makes a long
/// section safe to add to.
fn window_body_max_height(ctx: &egui::Context) -> f32 {
    (ctx.screen_rect().height() - 120.0).max(200.0)
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
    /// Camera orbit per basket, in screen pixels (Alt+drag). Per basket for the
    /// same reason the pan/zoom is: an angle you chose to read one arrangement
    /// should not follow you into another.
    eyes: HashMap<NodeId, egui::Vec2>,
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
    /// Colour emoji overlay — see [`crate::emoji`]. Holds the font bytes and one
    /// texture per character actually seen.
    emoji: crate::emoji::Emoji,
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
    /// A `trellis://` link landed on the API thread, which has no frame context;
    /// the reveal (and the window focus) happen on the next frame, where one
    /// exists. The highlight clock is frame time, not wall clock.
    pending_reveal: Option<(NodeId, CardId)>,
    /// The same deferred reveal for a group link.
    pending_reveal_group: Option<(NodeId, crate::model::GroupId)>,
    focus_window: bool,
    /// The card to flash-highlight on the canvas, and the `ctx` time the flash ends.
    highlight_card: Option<CardId>,
    highlight_until: f64,
    /// Same pair for a whole group (`[[#g…]]`, a Ctrl+O group row, or a
    /// `trellis://…/group/…` link). Shares `highlight_until`: only one reveal is
    /// ever in flight, so a second clock would just be a way to disagree.
    focus_group: Option<crate::model::GroupId>,
    highlight_group: Option<crate::model::GroupId>,
    /// Cards currently living on the desktop as their own OS windows, and the
    /// position each window was **created** at. Linux/X11 only.
    ///
    /// **This is never updated while a window is open**, and that is the whole
    /// point. `show_viewport_*` diffs the builder each frame and commands the
    /// window whenever a field changes, so feeding the window's own reported
    /// position back into the builder makes it fight the window manager — the
    /// window flashes and jumps, which is a delta chased against a lagging
    /// reading, exactly the v0.99.1 bug. Where the window actually *is* lives in
    /// `desktop_live` and only ever leaves via `save`.
    desktop_cards: std::collections::HashMap<CardId, [f32; 2]>,
    /// Last observed position of each open desktop window. Read-only feedback:
    /// it is persisted, and never fed back into a `ViewportBuilder`.
    desktop_live: std::collections::HashMap<CardId, [f32; 2]>,
    /// The most recent frame's context, so an API request answered in the pump
    /// loop can reach the window's own position — Desktop mode places card
    /// windows relative to it. Cheap to hold: `egui::Context` is an `Arc`.
    last_ctx: Option<egui::Context>,
    /// The basket currently *in* Desktop mode — the whole thing out on the
    /// desktop at once, the way VMware's Unity puts a guest's windows on the
    /// host. `None` means no basket is out as a whole (individual cards sent
    /// from their own menu are still tracked in `desktop_cards`).
    desktop_mode: Option<NodeId>,
    /// Claims panel: cards that assert state (`verify::`), worst first, so a
    /// workspace that has gone out of date says so instead of being believed.
    claims_open: bool,
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
    agenda_placement: Placement,
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
    kanban_placement: Placement,
    /// Detached Agenda / Kanban windows follow the main window when it moves,
    /// keeping the offset you put them at. On by default: a board you detached
    /// to sit beside the canvas is no use if it stays behind when the canvas
    /// moves, and the relative move keeps working across monitors.
    stick_windows: bool,
    /// The main window's outer position last frame, and how far it moved this
    /// one. A detached window is nudged by the same delta.
    last_main_pos: Option<egui::Pos2>,
    main_move_delta: egui::Vec2,
    /// Per-panel follow state; see [`StickState`].
    agenda_stick: StickState,
    kanban_stick: StickState,
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
    /// When on, a dragged or resized card is quantised to the canvas grid.
    /// Independent of `snap_mode`, which wins on any axis it claims.
    grid_mode: bool,
    depth_mode: bool,
    time_mode: bool,
    /// Path this build registered as the link handler, if any — shown in
    /// Settings so "why doesn't my link open?" has an answer on screen.
    url_scheme_registered: Option<String>,
    /// When on, a small overview map in the canvas's bottom-right shows the whole
    /// basket and a reticle of the current view (Settings; on by default).
    minimap_enabled: bool,
    /// Journal root for daily notes, or `None` when the feature is off.
    /// Per instance (it lives in this instance's config), so a work document can
    /// keep a journal while a personal one never grows one.
    daily_root: Option<NodeId>,
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
    /// Notify on startup with what is due, and when an agent edits.
    notify_digest: bool,
    notify_agent: bool,
    /// Root ordering — a view over `doc.roots`, never a rewrite of it.
    tree_sort: crate::tree::TreeSort,
    /// The change-log seq already reported, so one edit is announced once.
    notified_seq: crate::changelog::Seq,
    /// Set once the startup digest has been considered — it fires once per run,
    /// not once per frame.
    digest_done: bool,
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
    plugin_rx: Receiver<PluginEvent>,
    plugin_tx: Sender<PluginEvent>,
    /// Plugins currently running, so the UI can say so and not start a second.
    plugin_running: std::collections::HashSet<String>,
    /// The latest progress line per running plugin, for the window and the
    /// status bar.
    plugin_progress: std::collections::HashMap<String, crate::plugins::Progress>,
    /// The cancel flag handed to each running plugin. Setting it kills the
    /// child; the entry goes when the run reports back, so a plugin ignoring
    /// SIGKILL (it can't) would still be tracked honestly.
    plugin_cancel: std::collections::HashMap<String, Arc<AtomicBool>>,
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
    /// The "issue a token to an agent" form in Settings → Agent API.
    new_token_label: String,
    new_token_target: TokenTarget,
    new_token_read_only: bool,
    /// The token just minted, kept so it can be copied. Cleared when the form is
    /// reused, so the panel doesn't keep offering a stale one.
    new_token_minted: Option<(String, String)>,
    new_token_error: String,

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
        let grid_mode = cc
            .storage
            .and_then(|s| s.get_string(GRID_MODE_KEY))
            .map(|s| s == "true")
            .unwrap_or(false);
        let depth_mode = cc
            .storage
            .and_then(|s| s.get_string(DEPTH_MODE_KEY))
            .map(|s| s == "true")
            .unwrap_or(false);
        let time_mode = cc
            .storage
            .and_then(|s| s.get_string(TIME_MODE_KEY))
            .map(|s| s == "true")
            .unwrap_or(false);
        // Register the link scheme on a fresh install, and again if the binary
        // has moved. Silent and best-effort: it is a convenience, and a desktop
        // that has no `xdg-mime` must not produce an error dialog on first run.
        // The `http://127.0.0.1:<port>/open/...` form works regardless.
        let exe_now = std::env::current_exe().ok().map(|p| p.display().to_string());
        let registered_for = cc.storage.and_then(|s| s.get_string(URL_SCHEME_REGISTERED_KEY));
        // **Never clobber a working registration.** Three or more instances is the
        // normal case here — two documents plus a dev build — and they share one
        // desktop-wide handler. If a scratch or development binary silently
        // re-pointed it at itself, every link would break the moment that binary
        // was rebuilt or deleted. So the automatic path only acts when there is
        // no handler, or the one there points at a binary that no longer exists.
        // Settings → Register now is the deliberate override.
        let url_scheme_registered = match (&exe_now, &registered_for) {
            (Some(exe), Some(done)) if exe == done && scheme_handler_healthy() => {
                registered_for.clone()
            }
            (Some(_), _) if scheme_handler_healthy() => registered_for.clone(),
            (Some(_), _) => register_url_scheme().ok().and(exe_now.clone()),
            _ => registered_for.clone(),
        };
        let minimap_enabled = cc
            .storage
            .and_then(|s| s.get_string(MINIMAP_KEY))
            .map(|s| s != "false")
            .unwrap_or(true);
        // Both default to OFF. Notifications are a decision about the whole
        // desktop, not about this window, so they are opted into.
        let notify_digest = cc
            .storage
            .and_then(|s| s.get_string(NOTIFY_DIGEST_KEY))
            .map(|s| s == "true")
            .unwrap_or(false);
        let notify_agent = cc
            .storage
            .and_then(|s| s.get_string(NOTIFY_AGENT_KEY))
            .map(|s| s == "true")
            .unwrap_or(false);
        let tree_sort = cc
            .storage
            .and_then(|s| s.get_string(TREE_SORT_KEY))
            .map(|s| crate::tree::TreeSort::from_key(&s))
            .unwrap_or_default();
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
            eyes: HashMap::new(),
            md_cache: CommonMarkCache::default(),
            tex_cache: TextureCache::default(),
            renaming: None,
            doc_path,
            autosave_path,
            // Empty so the first frame always pushes the real title.
            window_title: String::new(),
            dialog_parent: None,
            emoji: crate::emoji::Emoji::new(),
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
            pending_reveal: None,
            pending_reveal_group: None,
            focus_window: false,
            highlight_card: None,
            highlight_until: 0.0,
            focus_group: None,
            highlight_group: None,
            desktop_cards: cc
                .storage
                .and_then(|s| s.get_string(DESKTOP_CARDS_KEY))
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            desktop_live: Default::default(),
            desktop_mode: None,
            last_ctx: None,
            claims_open: cc
                .storage
                .and_then(|s| s.get_string(CLAIMS_OPEN_KEY))
                .map(|s| s == "true")
                .unwrap_or(false),
            tags_open: cc
                .storage
                .and_then(|s| s.get_string(TAGS_OPEN_KEY))
                .map(|s| s == "true")
                .unwrap_or(false),
            tag_selected: None,
            find_open: cc
                .storage
                .and_then(|s| s.get_string(FIND_OPEN_KEY))
                .map(|s| s == "true")
                .unwrap_or(false),
            find_tag: None,
            find_key: None,
            find_value: String::new(),
            find_text: String::new(),
            agenda_open: cc
                .storage
                .and_then(|s| s.get_string(AGENDA_OPEN_KEY))
                .map(|s| s == "true")
                .unwrap_or(false),
            agenda_show_done: cc
                .storage
                .and_then(|s| s.get_string(AGENDA_DONE_KEY))
                .map(|s| s == "true")
                .unwrap_or(false),
            agenda_placement: cc
                .storage
                .and_then(|s| s.get_string(AGENDA_PLACE_KEY))
                .map(|s| Placement::from_str(&s))
                .unwrap_or(Placement::Docked),
            stick_windows: cc
                .storage
                .and_then(|s| s.get_string(STICK_WINDOWS_KEY))
                .map(|s| s == "true")
                .unwrap_or(true),
            last_main_pos: None,
            main_move_delta: egui::Vec2::ZERO,
            agenda_stick: StickState::default(),
            kanban_stick: StickState::default(),
            agenda_project: cc
                .storage
                .and_then(|st| st.get_string(AGENDA_PROJECT_KEY))
                .and_then(|v| v.parse().ok()),
            kanban_project: cc
                .storage
                .and_then(|st| st.get_string(KANBAN_PROJECT_KEY))
                .and_then(|v| v.parse().ok()),
            backlinks_open: cc
                .storage
                .and_then(|s| s.get_string(BACKLINKS_OPEN_KEY))
                .map(|s| s == "true")
                .unwrap_or(false),
            kanban_open: cc
                .storage
                .and_then(|s| s.get_string(KANBAN_OPEN_KEY))
                .map(|s| s == "true")
                .unwrap_or(false),
            kanban_show_done: cc
                .storage
                .and_then(|s| s.get_string(KANBAN_DONE_KEY))
                .map(|s| s == "true")
                .unwrap_or(true),
            kanban_placement: cc
                .storage
                .and_then(|s| s.get_string(KANBAN_PLACE_KEY))
                .map(|s| Placement::from_str(&s))
                .unwrap_or(Placement::Docked),
            graph_open: false,
            graph_built: false,
            graph_layout: HashMap::new(),
            graph_edges: Vec::new(),
            show_about: false,
            theme,
            zoom_enabled,
            minimap_enabled,
            notify_digest,
            notify_agent,
            tree_sort,
            notified_seq: 0,
            digest_done: false,
            daily_root: cc
                .storage
                .and_then(|st| st.get_string(DAILY_ROOT_KEY))
                .and_then(|v| v.trim().parse::<NodeId>().ok()),
            reorder_mode: false,
            dock_mode,
            snap_mode,
            grid_mode,
            depth_mode,
            time_mode,
            url_scheme_registered,
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
            plugin_progress: std::collections::HashMap::new(),
            plugin_cancel: std::collections::HashMap::new(),
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
            new_token_label: String::new(),
            new_token_target: TokenTarget::NewBasket,
            new_token_read_only: false,
            new_token_minted: None,
            new_token_error: String::new(),
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
    /// Relaunch this instance exactly as it was started.
    ///
    /// The whole point is *exactly*: same document, same `--port`, same
    /// `--data-dir`. An instance is defined by those — the port **is** the
    /// document's address — so a restart that dropped an argument would quietly
    /// open a different instance, or a second one on the default port.
    ///
    /// The new process waits before binding. The old one is still holding the
    /// API port for the moment it takes this one to exit, and a failed bind does
    /// not stop Trellis starting — it starts without an API, which looks healthy
    /// and answers nothing. That failure already has a status-bar warning
    /// because two instances on one port produced it once.
    fn restart(&mut self) {
        // **Synchronously.** `save()` spawns a background thread, so the child
        // used to be launched while the save was still running — it then opened
        // the file as it stood *before* this process finished writing, and any
        // edit in the new window would write that stale copy back over the good
        // one. Observed: the child started 19 seconds before the parent's final
        // write landed.
        let path = self.target_path();
        self.write_to(path, false);
        let exe = match exe_for_restart() {
            Some(e) => e,
            None => {
                self.status = "Restart failed: cannot find the Trellis binary".into();
                return;
            }
        };
        let args: Vec<String> = std::env::args().skip(1).collect();
        // A deadline, not a guess. The old value was a flat 1.5s sleep, which is
        // fine until the exit save is slow — on a large document over a slow
        // volume it took **20 seconds**, so the child bound the port, lost, and
        // came up silently API-less. The child now waits for the port to be free
        // and only gives up at this deadline.
        match std::process::Command::new(exe)
            .args(&args)
            .env("TRELLIS_RESTART_WAIT_SECS", "90")
            .spawn()
        {
            Ok(_) => self.status = "Restarting…".into(),
            Err(e) => self.status = format!("Restart failed: {e}"),
        }
    }

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
        /// A tailed file is a log: three seconds is a long time to watch nothing
        /// happen. Only a `stat` runs at this rate — the read is still gated on
        /// the mtime actually moving.
        const TAIL_POLL: Duration = Duration::from_millis(600);

        // Ask for a wake-up while any card mirrors a file. egui only calls
        // `update` when something requests a repaint, so on an idle window this
        // poll would otherwise never run — the file changed on disk and the card
        // sat there stale until the user happened to move the mouse. Requested
        // before the timer check, or the first early return silences it forever.
        let any_source =
            self.doc.nodes.values().any(|n| n.cards.iter().any(|c| c.source.is_some()));
        // A tail polls faster than a plain mirror, so the whole cadence drops to
        // the faster one as soon as any card is tailing.
        let any_tail = self
            .doc
            .nodes
            .values()
            .any(|n| n.cards.iter().any(|c| c.source.is_some() && c.source_tail.is_some()));
        let poll = if any_tail { TAIL_POLL } else { POLL };
        if any_source {
            self.egui_ctx.request_repaint_after(poll);
        }

        if !force {
            match self.last_source_poll {
                Some(t) if t.elapsed() < poll => return,
                _ => {}
            }
        }
        self.last_source_poll = Some(Instant::now());
        if !any_source {
            return;
        }

        // Collect first: reading files while holding a borrow on the document
        // would mean re-borrowing it mutably per card.
        let mut stale: Vec<(NodeId, CardId, String, Option<u32>)> = Vec::new();
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
                    stale.push((*nid, card.id, path.clone(), card.source_tail));
                }
            }
        }

        for (nid, cid, path, tail) in stale {
            // A tail seeks from the end, so the size cap does not apply to it —
            // which is the whole point: a growing log is the file the cap was
            // locking out.
            let result = match tail {
                Some(n) => crate::model::read_source_tail(&path, n),
                None => crate::model::read_source(&path),
            };
            let Some(card) = self.doc.card_mut(nid, cid) else { continue };
            let before = source_signature(card);
            match result {
                Ok((text, mtime)) => {
                    // A table card mirrors *data*, not prose: parse the file
                    // into cells. Filling rather than rebuilding is essential —
                    // rebuilding drops column widths, and on a 3-second poll
                    // that would re-flatten the columns continuously while
                    // someone was reading them.
                    let is_table = matches!(card.kind, CardKind::Table { .. });
                    if is_table {
                        match crate::model::delimited_to_values(&path, &text) {
                            Ok(values) => {
                                if let CardKind::Table { table } = &mut card.kind {
                                    table.fill_values(values);
                                }
                                card.source_mtime = Some(mtime);
                                card.source_error = None;
                            }
                            Err(e) => card.source_error = Some(e),
                        }
                    } else {
                        card.body = text;
                        card.source_mtime = Some(mtime);
                        card.source_error = None;
                    }
                }
                Err(e) => {
                    // Keep the last good text: a mirror that empties itself
                    // because a disk was unmounted is worse than a stale one.
                    card.source_error = Some(e);
                }
            }
            // Compare a signature, not just the body: a table's refresh moves
            // cells while `body` never changes, so a body-only comparison would
            // report no change and never wake a client.
            if Some(before) != self.doc.card(nid, cid).map(source_signature) {
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
        // `!standalone` throughout: a plugin must never pick up a token the user
        // issued to an agent that happens to share its name.
        if let Some(existing) = g.iter_mut().find(|g| g.plugin == name && !g.standalone) {
            if existing.scope == want {
                return existing.token.clone();
            }
            // The manifest asks for something different than was approved.
            existing.scope = want;
            existing.token = crate::plugins::mint_token();
            return existing.token.clone();
        }
        let token = crate::plugins::mint_token();
        g.push(crate::plugins::Grant {
            plugin: name,
            token: token.clone(),
            scope: want,
            standalone: false,
        });
        token
    }

    fn is_approved(&self, name: &str) -> bool {
        self.grants
            .lock()
            .map(|g| g.iter().any(|g| g.plugin == name && !g.standalone))
            .unwrap_or(false)
    }

    fn revoke(&mut self, name: &str) {
        if let Ok(mut g) = self.grants.lock() {
            g.retain(|g| !(g.plugin == name && !g.standalone));
        }
    }

    /// Every token issued to an agent, as (label, scope, token).
    fn agent_tokens(&self) -> Vec<(String, crate::plugins::Scope, String)> {
        self.grants
            .lock()
            .map(|g| {
                g.iter()
                    .filter(|g| g.standalone)
                    .map(|g| (g.plugin.clone(), g.scope.clone(), g.token.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Issue a token to an agent. The label is the agent's own name, so a token
    /// found in a config file elsewhere can be matched back to who holds it.
    /// Returns an error rather than silently replacing an existing one — two
    /// agents sharing a label would make revocation ambiguous.
    fn mint_agent_token(
        &mut self,
        label: &str,
        scope: crate::plugins::Scope,
    ) -> Result<String, String> {
        let label = label.trim().to_string();
        if label.is_empty() {
            return Err("Give the token a name — the agent's own name.".into());
        }
        let Ok(mut g) = self.grants.lock() else {
            return Err("could not read the token list".into());
        };
        if g.iter().any(|g| g.standalone && g.plugin.eq_ignore_ascii_case(&label)) {
            return Err(format!("There is already a token named {label}. Revoke it first."));
        }
        let token = crate::plugins::mint_agent_token();
        g.push(crate::plugins::Grant {
            plugin: label,
            token: token.clone(),
            scope,
            standalone: true,
        });
        Ok(token)
    }

    fn revoke_agent_token(&mut self, label: &str) {
        if let Ok(mut g) = self.grants.lock() {
            g.retain(|g| !(g.standalone && g.plugin == label));
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
        let cancel = Arc::new(AtomicBool::new(false));
        self.plugin_cancel.insert(p.manifest.name.clone(), Arc::clone(&cancel));
        self.plugin_progress.remove(&p.manifest.name);
        let tx = self.plugin_tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let ping = egui_ctx.clone();
            let ptx = tx.clone();
            let r = crate::plugins::run(&p, &token, &base, &ctx, &cancel, &|pr| {
                let _ = ptx.send(PluginEvent::Progress(pr));
                // Each line has to wake the UI itself: an idle window stops
                // calling update(), and progress nobody repaints is no progress.
                ping.request_repaint();
            });
            let _ = tx.send(PluginEvent::Done(r));
            egui_ctx.request_repaint();
        });
    }

    /// Ask a running plugin to stop. The flag is read by the runner's watcher
    /// thread, which kills the child; the run then reports back as cancelled
    /// like any other finish, so there is one path out of "running".
    fn cancel_plugin(&mut self, name: &str) {
        if let Some(flag) = self.plugin_cancel.get(name) {
            flag.store(true, Ordering::Relaxed);
            self.status = format!("Stopping {name}…");
        }
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
        let events: Vec<_> = std::iter::from_fn(|| self.plugin_rx.try_recv().ok()).collect();
        for ev in events {
            match ev {
                PluginEvent::Progress(pr) => {
                    if !pr.message.is_empty() {
                        self.status = pr.message.clone();
                    }
                    self.plugin_progress.insert(pr.plugin.clone(), pr);
                }
                PluginEvent::Done(r) => {
                    self.plugin_running.remove(&r.plugin);
                    self.plugin_cancel.remove(&r.plugin);
                    self.plugin_progress.remove(&r.plugin);
                    self.status = if r.ok || r.cancelled {
                        r.summary.clone()
                    } else {
                        format!("Plugin failed: {}", r.summary)
                    };
                    self.plugin_log.push(r);
                    // Keep the pane bounded; a chatty plugin shouldn't grow forever.
                    if self.plugin_log.len() > 50 {
                        self.plugin_log.remove(0);
                    }
                }
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

    /// Desktop notifications: the startup digest, and agent edits as they land.
    ///
    /// **Only while the window is unfocused.** If you are looking at Trellis, an
    /// agent's edit is already on the canvas — a popup would be telling you what
    /// you can see, which is how a notifier teaches you to ignore it.
    ///
    /// **And only while Trellis is running.** A desktop app is not a service.
    /// That is the honest limit of this channel and it is stated in Settings
    /// rather than left to be discovered; a Telegram message is the answer when
    /// something has to reach you with the app closed.
    fn pump_notifications(&mut self, ctx: &egui::Context) {
        if !self.notify_digest && !self.notify_agent {
            return;
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));

        // The digest, once per run. Deliberately at startup rather than on a
        // timer: a desktop app can only tell you things while it is open, so the
        // moment it opens is the one moment it is certain to be able to.
        if self.notify_digest && !self.digest_done {
            self.digest_done = true;
            if let Some((summary, body)) =
                crate::notify::digest(&self.doc, crate::api::today_days())
            {
                crate::notify::send(&summary, &body);
            }
        }

        if !self.notify_agent || focused {
            // Keep the cursor moving while focused, or every edit made while you
            // were looking would be announced the moment you switched away.
            if focused {
                if let Ok(log) = self.changes.lock() {
                    self.notified_seq = log.newest();
                }
            }
            return;
        }

        let fresh: Vec<(String, String)> = match self.changes.lock() {
            Ok(log) => {
                let (entries, _) = log.since(self.notified_seq, 64);
                self.notified_seq = log.newest();
                entries
                    .iter()
                    // `api` and not `ui`: the point is work that arrived while
                    // you were elsewhere. Your own edits are not news.
                    .filter(|c| c.actor == crate::changelog::Actor::Api)
                    .map(|c| {
                        (
                            format!("{:?} {:?}", c.entity, c.op),
                            c.title.clone().unwrap_or_default(),
                        )
                    })
                    .collect()
            }
            Err(_) => return,
        };
        if fresh.is_empty() {
            return;
        }
        // One notification for the batch, not one per change: an agent writing a
        // basket makes twenty entries, and twenty popups is an attack.
        let summary = if fresh.len() == 1 {
            "Trellis — an agent changed a card".to_string()
        } else {
            format!("Trellis — an agent made {} changes", fresh.len())
        };
        let body = crate::notify::elide(
            &fresh
                .iter()
                .map(|(_, t)| t.as_str())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
            120,
        );
        crate::notify::send(&summary, &body);
    }

    fn pump_api(&mut self) {
        let mut cmds = Vec::new();
        if let Some(rx) = &self.api_rx {
            while let Ok(cmd) = rx.try_recv() {
                cmds.push(cmd);
            }
        }
        for mut cmd in cmds {
            // A card-addressed write (`PATCH /api/cards/{cid}`, …) names no
            // basket. Resolve it into the ordinary node-addressed request FIRST,
            // before the scope check below, so the rest of this loop — the token
            // check, the mirror check, the change log, `process` — is the one that
            // already exists. A second set of write paths that each had to
            // remember to check a scope is how the v0.111.0 escape happened.
            if let api::ApiRequest::ByCard { card, .. } = &cmd.req {
                let card = *card;
                match self.doc.locate_card(card) {
                    Some(node) => {
                        let api::ApiRequest::ByCard { op, .. } =
                            std::mem::replace(&mut cmd.req, api::ApiRequest::Health)
                        else {
                            unreachable!("matched ByCard immediately above")
                        };
                        cmd.req = api::resolve_by_card(node, card, op);
                    }
                    None => {
                        // A confined token gets the same answer for "no such card"
                        // as for "a card you cannot see": telling the two apart
                        // turns this route into a way to probe the rest of the
                        // document one id at a time. Same reasoning as the 403 the
                        // scope check gives instead of a 404.
                        let confined = cmd.scope.as_ref().and_then(|s| s.subtree).is_some();
                        let _ = cmd.resp.send(if confined {
                            api::ApiResponse::err(
                                403,
                                "outside the basket this token was given access to",
                            )
                        } else {
                            api::ApiResponse::err(404, "no card with that id in this document")
                        });
                        continue;
                    }
                }
            }
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
            if let Some(resp) = self.handle_api_daily(&cmd.req) {
                let _ = cmd.resp.send(resp);
                continue;
            }
            if let Some(resp) = self.handle_api_open(&cmd.req) {
                let _ = cmd.resp.send(resp);
                continue;
            }
            if let Some(resp) = self.handle_api_go(&cmd.req) {
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
                        // `GET /api/cards/{id}` names a card, not a basket, so it
                        // has no target until the document resolves one. Do that
                        // here — where the tree exists — and check the basket it
                        // actually lands in, so a confined token can resolve the
                        // ids it is quoted for its own cards without the route
                        // becoming a way to read the rest of the document. An id
                        // that resolves to nothing is refused, not waved through.
                        None => match &cmd.req {
                            api::ApiRequest::LocateCard(cid) => self
                                .doc
                                .locate_card(*cid)
                                .is_some_and(|n| self.node_is_within(n, root)),
                            // Same resolve-then-check for a group id: an id
                            // alone names no basket until the tree says so.
                            api::ApiRequest::LocateGroup(gid)
                            | api::ApiRequest::GroupBacklinks(gid) => self
                                .doc
                                .locate_group(*gid)
                                .is_some_and(|n| self.node_is_within(n, root)),
                            _ => api::is_scope_neutral(&cmd.req),
                        },
                    };
                    // A move is checked at BOTH ends. `target_node` names where
                    // the thing is coming *from*, so on its own it lets a
                    // confined token carry its own card or basket out into the
                    // rest of the document — a write outside the scope, made by
                    // relocating something inside it.
                    let allowed = allowed && self.move_dest_within(&cmd.req, root);
                    if !allowed {
                        let _ = cmd.resp.send(api::ApiResponse::err(
                            403,
                            // "token", not "plugin": the same scope check now
                            // answers agents holding a token of their own, and
                            // an error naming the wrong kind of caller sends
                            // whoever reads it looking in the wrong list.
                            "outside the basket this token was given access to",
                        ));
                        continue;
                    }
                }
            }
            // `fit` in the request is applied by `process` from an estimate.
            // Note the target before the request is consumed, then re-measure
            // below with the real fonts — we're on the UI thread here.
            // An API-created checklist arrives with `id: 0` items (the API
            // thread has no counter). Backfill after the command applies, so an
            // item is never visible to anything without an identity.
            let needs_item_ids = matches!(
                cmd.req,
                api::ApiRequest::AddCard { .. }
                    | api::ApiRequest::UpdateCard { .. }
                    | api::ApiRequest::AddCards { .. }
            );
            let fit_target = api::fit_request(&cmd.req);
            // Which entries of a BATCH create asked to be fitted. Paired with the
            // ids below, because a created card has no id until it exists — the
            // same reason the single-card path reads its id from the response.
            let fit_batch = api::fit_batch(&cmd.req);
            // A batch EDIT names its cards, so the ids need no pairing — but the
            // re-measure still has to happen here, or `fit` would mean something
            // slightly different depending on whether you sent one card or ten.
            let fit_updates = api::fit_updates(&cmd.req);
            let fit_batch_node = match &cmd.req {
                api::ApiRequest::AddCards { node, .. } => Some(*node),
                _ => None,
            };
            // Same reason as `fit_request`: the request is consumed below, and
            // reading the document *now* catches the pre-change state — a
            // deleted card's title can't be looked up after it's gone.
            // `source` is the one field an API request can use to reach outside
            // the document, so it is checked here rather than in `process`,
            // which has no access to the setting.
            for path in api::source_requests(&cmd.req) {
                // A token confined to a basket may not mirror a file at all,
                // whatever the global policy allows.
                //
                // Without this the confinement leaks completely: the token
                // creates a card **inside its own basket** pointing at any file
                // the policy permits, then reads the body back. Verified before
                // the fix — a file outside the basket came back in full, and
                // under the default policy that includes another document on
                // disk. The scope says "this basket and nothing else"; reading
                // the filesystem is not in that.
                if cmd.scope.as_ref().and_then(|s| s.subtree).is_some() {
                    let _ = cmd.resp.send(api::ApiResponse::err(
                        403,
                        "a token confined to a basket cannot mirror files",
                    ));
                    continue;
                }
                if let Err(e) =
                    crate::model::mirror_allowed(&path, self.mirror_policy, &self.mirror_dirs)
                {
                    let _ = cmd.resp.send(api::ApiResponse::err(403, &e));
                    continue;
                }
            }
            let change = api::change_of(&cmd.req, &self.doc);
            let (changed, resp) = api::process(&mut self.doc, cmd.req);
            if needs_item_ids && changed {
                self.doc.ensure_item_ids();
            }
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
            if let Some((node, cards)) = fit_updates {
                for cid in cards {
                    self.refit_card_precise(node, cid);
                }
            }
            if let Some(node) = fit_batch_node {
                if !fit_batch.is_empty() {
                    let ids: Vec<u64> = serde_json::from_str::<serde_json::Value>(&resp.body)
                        .ok()
                        .and_then(|v| {
                            v["ids"].as_array().map(|a| {
                                a.iter().filter_map(|x| x.as_u64()).collect::<Vec<_>>()
                            })
                        })
                        .unwrap_or_default();
                    for i in fit_batch {
                        if let Some(&cid) = ids.get(i) {
                            self.refit_card_precise(node, cid);
                        }
                    }
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
    /// The template master cards living in `node`, as
    /// `card → (template index, name, edited?)`.
    ///
    /// **Why "edited" has to be shown.** The stored snapshot is the authority —
    /// inserting stamps *it*, never the master — which is deliberate, so that a
    /// stray edit cannot silently change every future insert. But nothing said
    /// the two had diverged, so editing a master looked like editing the
    /// template and wasn't: the insert kept producing the old content, with no
    /// error and nothing to notice. Reported from use.
    ///
    /// Only masters in the basket being drawn are considered, so this costs
    /// nothing anywhere except inside the Templates basket itself.
    fn master_states(&self, node: NodeId) -> HashMap<CardId, (usize, String, bool)> {
        let mut out = HashMap::new();
        for (i, t) in self.templates.iter().enumerate() {
            let Some(m) = &t.master else { continue };
            if m.node != node || self.doc.card(m.node, m.card).is_none() {
                continue;
            }
            let name = if t.card.title.trim().is_empty() {
                t.card.kind.label().to_string()
            } else {
                t.card.title.clone()
            };
            let edited = match self
                .doc
                .export_card_json(m.node, m.card)
                .and_then(|j| crate::model::parse_card_export(&j))
            {
                Some(now) => template_key(&now) != template_key(&t.card),
                // A master that cannot be exported cannot be compared; claiming
                // it is edited would put a warning on a card nobody can fix.
                None => false,
            };
            out.insert(m.card, (i, name, edited));
        }
        out
    }

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
    /// Open today's journal node, creating it (and any missing month) on the way.
    ///
    /// Returns `None` when daily notes are switched off, which is the default.
    /// **This is the only thing that ever creates a dated node** — nothing on the
    /// ordinary node-creation path knows about journals, so a document whose
    /// owner never asked for one can never grow one.
    fn go_to_today(&mut self) -> Option<crate::model::DailyNode> {
        self.go_to_day(api::today_daily_date())
    }

    fn go_to_day(&mut self, date: crate::model::DailyDate) -> Option<crate::model::DailyNode> {
        let root = self.daily_root?;
        if !self.doc.nodes.contains_key(&root) {
            // The journal root was deleted. Switch the feature off rather than
            // silently rebuilding a tree somewhere else.
            self.daily_root = None;
            self.status = "Daily notes: the journal root is gone — set it again in Settings".into();
            return None;
        }
        let res = self.doc.ensure_daily(root, date)?;
        if res.root != root {
            // A new year. Follow it, so next January doesn't keep reaching for
            // the old one.
            self.daily_root = Some(res.root);
        }
        if res.created {
            self.mark_dirty();
        }
        self.jump_to_node(res.node);
        Some(res)
    }

    fn handle_api_daily(&mut self, req: &api::ApiRequest) -> Option<api::ApiResponse> {
        match req {
            // Read the setting — the same two facts Settings shows.
            api::ApiRequest::DailyConfig => Some(api::ApiResponse::ok(serde_json::json!({
                "enabled": self.daily_root.is_some(),
                "root": self.daily_root,
                "root_title": self.daily_root.and_then(|id| self.doc.nodes.get(&id).map(|n| n.title.clone())),
                "root_path": self.daily_root.map(|id| self.doc.node_path(id)),
            }))),
            // Set or clear it — the two buttons in Settings.
            api::ApiRequest::SetDailyRoot(root) => Some(match root {
                Some(id) if !self.doc.nodes.contains_key(id) => {
                    api::ApiResponse::err(404, "node not found")
                }
                Some(id) => {
                    self.daily_root = Some(*id);
                    api::ApiResponse::ok(serde_json::json!({
                        "enabled": true,
                        "root": id,
                        "root_path": self.doc.node_path(*id),
                    }))
                }
                None => {
                    self.daily_root = None;
                    api::ApiResponse::ok(serde_json::json!({ "enabled": false, "root": null }))
                }
            }),
            api::ApiRequest::DailyNote { date } => {
                let d = match date {
                    Some(s) => match api::daily_date_from(s) {
                        Some(d) => d,
                        None => {
                            return Some(api::ApiResponse::err(
                                400,
                                "date must be YYYY-MM-DD and a real calendar day",
                            ))
                        }
                    },
                    None => api::today_daily_date(),
                };
                Some(match self.go_to_day(d) {
                Some(res) => api::ApiResponse::ok(serde_json::json!({
                    "node": res.node,
                    "created": res.created,
                    "title": self.doc.nodes.get(&res.node).map(|n| n.title.clone()),
                    "path": self.doc.node_path(res.node),
                })),
                None => api::ApiResponse::err(
                    404,
                    "daily notes are off for this instance — Tools → Settings → Daily notes",
                ),
                })
            }
            _ => None,
        }
    }

    /// `GET /open/...` — a `trellis://` link landing in this instance.
    ///
    /// Navigation only. It answers whether the target exists and moves the
    /// window there; it never returns document content, because this is the one
    /// route with no key and a page that could read cards by walking ids is the
    /// same hole that was closed in v0.85.1 for mirrored files.
    fn handle_api_open(&mut self, req: &api::ApiRequest) -> Option<api::ApiResponse> {
        // Minting a link needs the port and the document name, which are app
        // state rather than document state — hence answering it here.
        if let api::ApiRequest::CardLink(cid) = req {
            let cid = *cid as crate::model::CardId;
            let Some(node) = self.doc.locate_card(cid) else {
                return Some(api::ApiResponse::err(404, "no card with that id"));
            };
            let doc = self
                .doc_path
                .as_ref()
                .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
                .unwrap_or_default();
            let scheme = crate::URL_SCHEME;
            let port = self.api_port;
            return Some(api::ApiResponse::ok(serde_json::json!({
                "card": cid,
                "node": node,
                "node_path": self.doc.node_path(node),
                "document": doc,
                // The bare form is what you paste. `verified` adds the check that
                // turns "a different document is on this port" from a silent
                // landing on the wrong real card into an error.
                // `127.0.0.1:` is not decoration. Without a host the port sits
                // in the URL's HOST position, where a bare integer is a legal
                // IPv4 address — KDE's parser rewrote `7374` to `0.0.28.206`
                // and the link arrived unusable. With the port in the port
                // position there is nothing left to normalise.
                "link": format!("{scheme}://127.0.0.1:{port}/card/{cid}"),
                "link_verified": format!("{scheme}://127.0.0.1:{port}/card/{cid}?doc={doc}"),
                // Works with nothing registered — a browser or a terminal can
                // follow it today.
                "http": format!("http://127.0.0.1:{port}/open/card/{cid}"),
            })));
        }
        // Desktop-mode placement is app config, so it is answered here rather
        // than in `process`, the same as templates and the backup schedule.
        if let api::ApiRequest::ListCardDesktop = req {
            let cards: Vec<serde_json::Value> = self
                .desktop_cards
                .iter()
                .map(|(cid, p)| serde_json::json!({ "card": cid, "pos": p }))
                .collect();
            return Some(api::ApiResponse::ok(serde_json::json!({
                "supported": cfg!(target_os = "linux"),
                "cards": cards,
            })));
        }
        if let api::ApiRequest::SetNodeDesktop { node, on } = req {
            if !cfg!(target_os = "linux") {
                return Some(api::ApiResponse::err(
                    501,
                    "desktop mode needs a window manager that lets an application \
                     position its own windows — Linux/X11 only for now",
                ));
            }
            if !self.doc.nodes.contains_key(node) {
                return Some(api::ApiResponse::err(404, "node not found"));
            }
            #[cfg(target_os = "linux")]
            {
                let n = *node;
                if *on {
                    if let Some(prev) = self.desktop_mode {
                        if prev != n {
                            self.recall_basket_from_desktop(prev);
                        }
                    }
                    let ctx = self.last_ctx.clone();
                    match ctx {
                        Some(c) => self.send_basket_to_desktop(&c, n),
                        // No frame has run yet, so there are no screen rects to
                        // place windows by. Refusing beats opening every card in
                        // one corner of the display.
                        None => {
                            return Some(api::ApiResponse::err(
                                503,
                                "the window has not drawn yet — try again in a moment",
                            ))
                        }
                    }
                } else {
                    self.recall_basket_from_desktop(n);
                }
                let out: Vec<u64> = self.desktop_cards.keys().copied().collect();
                return Some(api::ApiResponse::ok(serde_json::json!({
                    "node": n,
                    "desktop": *on,
                    "cards": out,
                })));
            }
            #[cfg(not(target_os = "linux"))]
            return Some(api::ApiResponse::err(501, "Linux/X11 only"));
        }
        if let api::ApiRequest::SetCardDesktop { card, pos, on } = req {
            let cid = *card as crate::model::CardId;
            if !cfg!(target_os = "linux") {
                return Some(api::ApiResponse::err(
                    501,
                    "desktop mode needs a window manager that lets an application                      position its own windows — Linux/X11 only for now",
                ));
            }
            if self.doc.locate_card(cid).is_none() {
                return Some(api::ApiResponse::err(404, "no card with that id"));
            }
            if *on {
                let p = pos.unwrap_or_else(|| {
                    self.card_rects
                        .get(&cid)
                        .map(|r| [r.min.x + 60.0, r.min.y + 60.0])
                        .unwrap_or([200.0, 200.0])
                });
                self.desktop_cards.insert(cid, p);
                return Some(api::ApiResponse::ok(serde_json::json!({
                    "card": cid, "desktop": true, "pos": p,
                })));
            }
            let was = self.desktop_cards.remove(&cid).is_some();
            return Some(api::ApiResponse::ok(serde_json::json!({
                "card": cid, "desktop": false, "was_on_desktop": was,
            })));
        }
        if let api::ApiRequest::GroupLink(gid) = req {
            let gid = *gid as crate::model::GroupId;
            let Some(node) = self.doc.locate_group(gid) else {
                return Some(api::ApiResponse::err(404, "no group with that id"));
            };
            let doc = self
                .doc_path
                .as_ref()
                .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
                .unwrap_or_default();
            let scheme = crate::URL_SCHEME;
            let port = self.api_port;
            let title = self
                .doc
                .nodes
                .get(&node)
                .and_then(|n| n.groups.iter().find(|g| g.id == gid))
                .map(|g| g.title.clone())
                .unwrap_or_default();
            return Some(api::ApiResponse::ok(serde_json::json!({
                "group": gid,
                "title": title,
                "node": node,
                "node_path": self.doc.node_path(node),
                "document": doc,
                "link": format!("{scheme}://127.0.0.1:{port}/group/{gid}"),
                "link_verified": format!("{scheme}://127.0.0.1:{port}/group/{gid}?doc={doc}"),
                "http": format!("http://127.0.0.1:{port}/open/group/{gid}"),
                // The in-document form, which is what you actually paste into a
                // card. A `trellis://` link is for leaving the app.
                "wikilink": format!("[[#g{gid}]]"),
            })));
        }
        let api::ApiRequest::Open { kind, id, doc } = req else {
            return None;
        };
        // `doc` is optional — the port is the address (the operator's ruling) —
        // but when it is given it is checked, so the one real failure mode (a
        // different document started on that port) is an error rather than a
        // silent landing on a real card that is not the one meant.
        if let Some(want) = doc {
            let have = self
                .doc_path
                .as_ref()
                .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
                .unwrap_or_default();
            if !want.is_empty() && !have.eq_ignore_ascii_case(want) {
                return Some(api::ApiResponse::err(
                    409,
                    &format!("this port is serving {have}, not {want}"),
                ));
            }
        }
        let found = match kind {
            api::OpenKind::Node => {
                let nid = *id as crate::model::NodeId;
                self.doc.nodes.contains_key(&nid).then(|| {
                    self.jump_to_node(nid);
                    format!("node {nid}")
                })
            }
            api::OpenKind::Card => {
                let cid = *id as crate::model::CardId;
                self.doc.locate_card(cid).map(|n| {
                    // Same reveal the Agenda and a [[#id]] link use — land *on*
                    // the card, not merely in its basket — deferred to the next
                    // frame.
                    self.pending_reveal = Some((n, cid));
                    format!("card {cid}")
                })
            }
            api::OpenKind::Group => {
                let gid = *id as crate::model::GroupId;
                self.doc.locate_group(gid).map(|n| {
                    self.pending_reveal_group = Some((n, gid));
                    format!("group {gid}")
                })
            }
        };
        Some(match found {
            Some(what) => {
                // Ask for focus on the next frame — a link that navigates a
                // window you cannot see has not taken you anywhere. Done via a
                // flag because this runs on the pump, not inside a frame.
                self.focus_window = true;
                self.status = format!("Opened {what} from a link");
                api::ApiResponse::ok(serde_json::json!({ "opened": what }))
            }
            None => api::ApiResponse::err(404, "no such target in this document"),
        })
    }

    /// `GET /go/{kind}/{id}` — the page that hands a phone off to `trellis://`.
    ///
    /// **This window does not move.** That is the whole difference from `/open/`:
    /// the reader is on another device, and focusing the desktop because someone
    /// tapped a notification on the sofa is a jump nobody asked for.
    ///
    /// The `trellis://` URL is assembled **in the page**, from `location.host`,
    /// because only the reader's browser knows how it reached us — a link minted
    /// here would say `127.0.0.1`, which on a phone is the phone. The document
    /// name is baked in, because that is what lets the app pick the right
    /// workstation when two of them serve the same port.
    ///
    /// It is a **link, not a redirect**. An automatic `location =` to a custom
    /// scheme is what in-app browsers block; a link the reader taps is a user
    /// gesture, which is the case they allow.
    fn handle_api_go(&mut self, req: &api::ApiRequest) -> Option<api::ApiResponse> {
        let api::ApiRequest::Go { kind, id } = req else {
            return None;
        };
        let doc_name = self
            .doc_path
            .as_ref()
            .and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_string()))
            .unwrap_or_default();
        // Resolve for real, so a stale link says so instead of handing the phone
        // a URL that will fail again at the other end.
        let (path, what) = match kind {
            api::OpenKind::Node => {
                let nid = *id as crate::model::NodeId;
                match self.doc.nodes.get(&nid) {
                    Some(n) => (format!("node/{nid}"), n.title.clone()),
                    None => return Some(api::ApiResponse::err(404, "no such basket")),
                }
            }
            api::OpenKind::Card => {
                let cid = *id as crate::model::CardId;
                match self.doc.locate_card(cid) {
                    Some(n) => {
                        let title = self
                            .doc
                            .card(n, cid)
                            .map(card_label)
                            .unwrap_or_else(|| format!("card {cid}"));
                        (format!("card/{cid}"), title)
                    }
                    None => return Some(api::ApiResponse::err(404, "no such card")),
                }
            }
            api::OpenKind::Group => {
                let gid = *id as crate::model::GroupId;
                match self.doc.locate_group(gid) {
                    Some(n) => {
                        let title = self
                            .doc
                            .nodes
                            .get(&n)
                            .and_then(|node| node.groups.iter().find(|g| g.id == gid))
                            .map(|g| g.title.clone())
                            .unwrap_or_else(|| format!("group {gid}"));
                        (format!("group/{gid}"), title)
                    }
                    None => return Some(api::ApiResponse::err(404, "no such group")),
                }
            }
        };
        let where_ = match kind {
            api::OpenKind::Node => String::new(),
            _ => self
                .doc
                .locate_card(*id as crate::model::CardId)
                .or_else(|| self.doc.locate_group(*id as crate::model::GroupId))
                .map(|n| self.doc.node_path(n))
                .unwrap_or_default(),
        };
        Some(api::ApiResponse::html(200, go_page(&path, &doc_name, &what, &where_)))
    }

    /// The app-level settings, as the API sees them.
    ///
    /// **Why these are an endpoint at all.** Everything a person can do in this
    /// app, an agent can do — that is the rule the whole API is built on, and it
    /// had quietly stopped being true: the theme, the canvas toggles and (as of
    /// this session) notifications and project sort were reachable only by
    /// clicking. An agent setting up a machine, or restoring one, could not put
    /// it back the way it was.
    ///
    /// **These are instance settings, not document settings.** They live in the
    /// config beside the key and the port, so they are per `--data-dir`: work and
    /// personal can differ, and neither is written into the `.ron` file.
    fn settings_json(&self) -> serde_json::Value {
        serde_json::json!({
            "theme": self.theme.key(),
            "tree_sort": self.tree_sort.key(),
            "minimap": self.minimap_enabled,
            "dock_mode": self.dock_mode,
            "snap_mode": self.snap_mode,
            "grid_mode": self.grid_mode,
            "depth_mode": self.depth_mode,
            "time_mode": self.time_mode,
            "notify_digest": self.notify_digest,
            "notify_agent": self.notify_agent,
            "zoom_enabled": self.zoom_enabled,
            "autosave": self.autosave,
            "stick_windows": self.stick_windows,
            "agenda_open": self.agenda_open,
            "agenda_show_done": self.agenda_show_done,
            "agenda_project": self.agenda_project,
            "kanban_open": self.kanban_open,
            "kanban_show_done": self.kanban_show_done,
            "kanban_project": self.kanban_project,
            "tags_open": self.tags_open,
            "claims_open": self.claims_open,
            "find_open": self.find_open,
            "backlinks_open": self.backlinks_open,
            "history_keep": self.history_keep,
            "history_gap_mins": self.history_gap_mins,
        })
    }

    /// Apply a settings patch, refusing anything it does not know.
    ///
    /// An unknown key is a **400 naming it**, like every other input since
    /// v0.86.0 — a typo that silently does nothing is the failure this API spent
    /// a release removing everywhere else. A known key with the wrong type is
    /// refused the same way rather than coerced.
    fn apply_settings(&mut self, patch: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
        // Validate the whole patch before applying any of it, so a bad third key
        // cannot leave the first two applied — the rule table ops learned in
        // v0.102.0.
        for (k, v) in patch {
            let ok = match k.as_str() {
                "theme" => v.as_str().map(|s| Theme::from_key(s).key() == s).unwrap_or(false),
                "tree_sort" => v
                    .as_str()
                    .map(|s| crate::tree::TreeSort::from_key(s).key() == s)
                    .unwrap_or(false),
                "minimap" | "dock_mode" | "snap_mode" | "grid_mode" | "depth_mode"
                | "time_mode"
                | "notify_digest" | "notify_agent" | "zoom_enabled" | "autosave"
                | "stick_windows" | "agenda_open" | "agenda_show_done" | "kanban_open"
                | "kanban_show_done" | "tags_open" | "claims_open" | "find_open"
                | "backlinks_open" => {
                    v.is_boolean()
                }
                // A project filter is a node id, or null for "all projects".
                "agenda_project" | "kanban_project" => {
                    v.is_null() || v.as_u64().is_some()
                }
                // Clamped rather than refused: the UI clamps these too, and the
                // retention rules are the app's to enforce, not the caller's to
                // get exactly right.
                "history_keep" | "history_gap_mins" => v.as_u64().is_some(),
                _ => {
                    return Err(format!(
                        "unknown setting {k:?}. Settable: theme, tree_sort, minimap, \
                         dock_mode, snap_mode, grid_mode, depth_mode, time_mode, \
                         notify_digest, \
                         notify_agent, zoom_enabled, autosave, stick_windows, agenda_open, \
                         agenda_show_done, agenda_project, kanban_open, kanban_show_done, \
                         kanban_project, tags_open, claims_open, find_open, backlinks_open, \
                         history_keep, \
                         history_gap_mins. Deliberately not settable over the API: the API \
                         key, port and LAN flag (a caller must not be able to widen its own \
                         reach), and the file-mirroring policy (same reason). Change those \
                         in Tools -> Settings."
                    ))
                }
            };
            if !ok {
                return Err(format!("setting {k:?}: {v} is not a value it accepts"));
            }
        }
        for (k, v) in patch {
            match k.as_str() {
                "theme" => self.theme = Theme::from_key(v.as_str().unwrap_or("")),
                "tree_sort" => {
                    self.tree_sort = crate::tree::TreeSort::from_key(v.as_str().unwrap_or(""))
                }
                "minimap" => self.minimap_enabled = v.as_bool().unwrap_or(false),
                "dock_mode" => self.dock_mode = v.as_bool().unwrap_or(false),
                "snap_mode" => self.snap_mode = v.as_bool().unwrap_or(false),
                "grid_mode" => self.grid_mode = v.as_bool().unwrap_or(false),
                "depth_mode" => self.depth_mode = v.as_bool().unwrap_or(false),
                "time_mode" => self.time_mode = v.as_bool().unwrap_or(false),
                "notify_digest" => self.notify_digest = v.as_bool().unwrap_or(false),
                "notify_agent" => self.notify_agent = v.as_bool().unwrap_or(false),
                "zoom_enabled" => self.zoom_enabled = v.as_bool().unwrap_or(false),
                "autosave" => self.autosave = v.as_bool().unwrap_or(false),
                "stick_windows" => self.stick_windows = v.as_bool().unwrap_or(false),
                "agenda_open" => self.agenda_open = v.as_bool().unwrap_or(false),
                "agenda_show_done" => self.agenda_show_done = v.as_bool().unwrap_or(false),
                "kanban_open" => self.kanban_open = v.as_bool().unwrap_or(false),
                "kanban_show_done" => self.kanban_show_done = v.as_bool().unwrap_or(false),
                "tags_open" => self.tags_open = v.as_bool().unwrap_or(false),
                "claims_open" => self.claims_open = v.as_bool().unwrap_or(false),
                "find_open" => self.find_open = v.as_bool().unwrap_or(false),
                "backlinks_open" => self.backlinks_open = v.as_bool().unwrap_or(false),
                "agenda_project" => self.agenda_project = v.as_u64(),
                "kanban_project" => self.kanban_project = v.as_u64(),
                // Same clamps the Settings panel applies, and `keep.max(1)` so a
                // zero cannot prune away the snapshot just written.
                "history_keep" => {
                    self.history_keep = (v.as_u64().unwrap_or(1) as usize).clamp(1, 200)
                }
                "history_gap_mins" => self.history_gap_mins = v.as_u64().unwrap_or(0).min(1440),
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_api_instance(&mut self, req: &api::ApiRequest) -> Option<api::ApiResponse> {
        match req {
            // Answered here rather than in `process` because it reads the
            // filesystem and moves the selection and status line — none of which
            // the pure-over-Document path has.
            api::ApiRequest::ImportVault { parent, path } => {
                let dir = std::path::PathBuf::from(path);
                if !dir.is_dir() {
                    return Some(api::ApiResponse::err(400, "path is not a directory"));
                }
                if let Some(p) = parent {
                    if !self.doc.nodes.contains_key(p) {
                        return Some(api::ApiResponse::err(404, "parent node not found"));
                    }
                }
                let name = dir
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Vault".to_string());
                let files = match crate::vault::read_vault(&dir) {
                    Ok(f) => f,
                    Err(e) => return Some(api::ApiResponse::err(400, &e.to_string())),
                };
                if files.is_empty() {
                    return Some(api::ApiResponse::err(400, "the folder holds no files to import"));
                }
                let r = crate::vault::import_vault(&mut self.doc, *parent, &name, files);
                if let Some(n) = self.doc.nodes.get_mut(&r.root) {
                    n.expanded = true;
                }
                self.mark_dirty();
                self.status = crate::vault::describe(&r, &name);
                Some(api::ApiResponse::ok(
                    serde_json::to_value(&r).unwrap_or(serde_json::Value::Null),
                ))
            }
            api::ApiRequest::SettingsGet => Some(api::ApiResponse::ok(self.settings_json())),
            api::ApiRequest::SettingsSet(patch) => Some(match self.apply_settings(patch) {
                // A theme change repaints and a sort reorders the tree, so the
                // caller gets the settings back as they now are rather than as
                // they asked for them.
                Ok(()) => api::ApiResponse::ok(self.settings_json()),
                Err(e) => api::ApiResponse::err(400, &e),
            }),
            api::ApiRequest::Instance => Some(api::ApiResponse::ok(serde_json::json!({
                "app": "trellis",
                "version": env!("CARGO_PKG_VERSION"),
                "document": doc_display_name(self.doc_path.as_deref()),
                "path": self.doc_path.as_ref().map(|p| p.display().to_string()),
                "port": self.api_port,
                "lan": self.api_lan,
                // The address another device on the network can reach this
                // instance on, when LAN access is on. `port` alone is not enough
                // to build a link for a phone: everything this app mints says
                // `127.0.0.1`, which on the phone is the phone. Null when LAN
                // access is off, because then there is honestly no such address.
                "lan_host": self.api_lan
                    .then(lan_addresses)
                    .and_then(|v| v.first().cloned()),
                // Every candidate, because the first is a heuristic: a machine on
                // two LANs plus a VPN has three, and only the reader knows which
                // network their phone is on.
                "lan_hosts": self.api_lan.then(lan_addresses).unwrap_or_default(),
                "nodes": self.doc.nodes.len(),
                // What the embedded files cost. The document is written whole on
                // every save, so this number is paid on each autosave and copied
                // into every snapshot and backup — and it is otherwise invisible
                // until a backup gets slow.
                "attachment_bytes": self.doc.attachment_bytes(),
                "unsaved_changes": self.dirty,
                // How many cards assert state that is past its check date.
                // It rides on `/api/instance` because that is the call every
                // agent already makes first — a workspace that has gone stale
                // should say so before it is read, not after it is believed.
                "stale_claims": api::stale_claim_count(&self.doc),
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
            // One entry for the whole tree, not one per node.
            TreeAction::SetAllExpanded(_) => ch(Op::Updated, 0).field("expanded.all"),
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
            CanvasAction::ToggleDesktopMode => {
                Change::new(Ui, Entity::Node, Op::Updated, node).field("desktop_mode")
            }
            CanvasAction::SendToDesktop(c) => {
                card(Op::Updated, *c).titled(title(c)).field("desktop")
            }
            CanvasAction::RecallFromDesktop(c) => {
                card(Op::Updated, *c).titled(title(c)).field("desktop.recall")
            }
            // Pure view/clipboard/export, plus the template actions, which record
            // themselves where the library is actually touched.
            // Selecting changes nothing in the document.
            CanvasAction::SelectCards(_)
            | CanvasAction::ResetView
            | CanvasAction::CopyCard(_)
            // Launching a plugin changes nothing by itself. Anything it does to
            // the document arrives over the API and is logged there, as `api`
            // rather than `ui` — which is the honest actor.
            | CanvasAction::RunCardPlugin(..)
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
            | CanvasAction::ToggleGridMode
            | CanvasAction::ToggleDepthMode
            | CanvasAction::ToggleTimeMode
            | CanvasAction::FollowLink(_)
            | CanvasAction::RevealElsewhere(..)
            | CanvasAction::SaveAsTemplate(_)
            | CanvasAction::UpdateTemplate(..)
            | CanvasAction::DeleteTemplate(_)
            // Writing a copy of an attachment out to disk changes no document.
            | CanvasAction::SaveAttachment(..) => return None,

            // Detaching one does: the bytes leave the document with it.
            CanvasAction::RemoveAttachment(c, _) => {
                card(Op::Updated, *c).titled(title(c)).field("attachments")
            }

            // Created — the id doesn't exist yet; `flush_notes` fills it in.
            CanvasAction::AddCard(kind, _) => card(Op::Created, 0).field(kind.label()),
            CanvasAction::PasteCard(_) => card(Op::Created, 0).field("paste"),
            CanvasAction::ImportCard(_) => card(Op::Created, 0).field("import"),
            CanvasAction::InsertTemplate(..) => card(Op::Created, 0).field("template.insert"),
            CanvasAction::Duplicate(c) => card(Op::Created, 0).field(&format!("duplicate={c}")),
            CanvasAction::DropFiles(..) => card(Op::Created, 0).field("drop"),

            CanvasAction::Remove(c) => card(Op::Deleted, *c).titled(title(c)),
            CanvasAction::MoveCard(c, _) => card(Op::Moved, *c).titled(title(c)).field("pos"),
            CanvasAction::SetZ(c, _) => card(Op::Moved, *c).titled(title(c)).field("z"),
            CanvasAction::SetEmphasis(c, _) => upd(c, "emphasis"),
            CanvasAction::RaiseCard(c) => card(Op::Moved, *c).titled(title(c)).field("order"),
            CanvasAction::ResizeCard(c, _) => upd(c, "size"),
            // One entry for the whole arrangement, naming how many moved: N
            // separate "moved" rows for one menu click is noise, and collapsing
            // them afterwards would lose the count.
            CanvasAction::PlaceCards(m) => card(Op::Moved, m.first().map_or(0, |(c, _)| *c))
                .field(&format!("arrange:{}", m.len())),
            CanvasAction::ExtractSelection(c, _) => card(Op::Created, 0).field(&format!("extract={c}")),
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
            CanvasAction::SetSourceTail(c, _) => upd(c, "source_tail"),
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

    /// Does this action reorder a **root**? Sorting only governs roots, so a
    /// move inside a project must not switch it off.
    fn reorders_a_root(&self, a: &TreeAction) -> bool {
        let is_root = |id: &NodeId| {
            self.doc.nodes.get(id).map(|n| n.parent.is_none()).unwrap_or(false)
        };
        match a {
            TreeAction::MoveUp(id)
            | TreeAction::MoveDown(id)
            | TreeAction::MoveToTop(id)
            | TreeAction::MoveToBottom(id) => is_root(id),
            TreeAction::Reorder { moved, .. } => is_root(moved),
            _ => false,
        }
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
                TreeAction::SetAllExpanded(expanded) => {
                    self.doc.set_all_expanded(expanded);
                }
                // Reordering by hand while a sort is on would look like it did
                // nothing — the view would re-sort it straight back. The move is
                // what was asked for, so the sort steps aside rather than
                // silently winning.
                TreeAction::MoveUp(_)
                | TreeAction::MoveDown(_)
                | TreeAction::MoveToTop(_)
                | TreeAction::MoveToBottom(_)
                | TreeAction::Reorder { .. }
                    if self.tree_sort != crate::tree::TreeSort::Manual
                        && self.reorders_a_root(&a) =>
                {
                    self.tree_sort = crate::tree::TreeSort::Manual;
                    self.status =
                        "Sorting off — projects are back in the order you arrange".to_string();
                    self.apply_tree(vec![a]);
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

    /// Import an **Obsidian vault** the user picks, as a new root-level project.
    ///
    /// A root rather than a child of the selection: a vault is somebody's whole
    /// notes, and burying it inside whatever basket happened to be selected is
    /// the wrong default. Moving a basket afterwards is one drag; digging one out
    /// is not.
    fn import_vault(&mut self) {
        let Some(dir) = self.file_dialog().pick_folder() else { return };
        let name = dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Vault".to_string());
        match crate::vault::read_vault(&dir) {
            Ok(files) if files.is_empty() => {
                self.status = format!("Nothing to import: {} holds no files", dir.display());
            }
            Ok(files) => {
                let r = crate::vault::import_vault(&mut self.doc, None, &name, files);
                self.selected = Some(r.root);
                if let Some(n) = self.doc.nodes.get_mut(&r.root) {
                    n.expanded = true;
                }
                self.mark_dirty();
                self.status = crate::vault::describe(&r, &name);
            }
            Err(e) => self.status = format!("Import failed: {e}"),
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

    /// The card under `pos`, whatever kind it is — an attachment can ride on any
    /// card, so unlike [`text_card_at`] this does not care.
    fn any_card_at(&self, node: NodeId, pos: egui::Pos2) -> Option<crate::model::CardId> {
        let n = self.doc.nodes.get(&node)?;
        n.cards
            .iter()
            .rev()
            .find(|c| egui::Rect::from_min_size(c.pos, c.size).contains(pos))
            .map(|c| c.id)
    }

    /// Bytes above which a dropped file asks first. **A warning, not a refusal:**
    /// the operator's call, and the reason it is asked at all is below.
    const ATTACH_WARN_BYTES: usize = 10 * 1024 * 1024;

    /// Attach a dropped file to the card under the cursor, or to a new card named
    /// after it. `false` if the operator declined.
    ///
    /// **Why size is worth a prompt.** The document is one gzip-compressed RON
    /// file written *whole*, atomically, on every save — so an embedded file is
    /// re-serialised on every autosave, and copied into every version-history
    /// snapshot and every backup archive. That is a real cost and it is invisible
    /// at the moment of the drop, which is exactly when someone can still decide
    /// against it.
    fn attach_dropped(
        &mut self,
        node: NodeId,
        bytes: Vec<u8>,
        name: String,
        pos: egui::Pos2,
        at: egui::Pos2,
    ) -> bool {
        if bytes.len() > Self::ATTACH_WARN_BYTES {
            let mb = bytes.len() as f64 / (1024.0 * 1024.0);
            let ok = matches!(
                self.message_dialog()
                    .set_title("Large attachment")
                    .set_description(&format!(
                        "{name} is {mb:.1} MB.\n\nThe bytes are stored inside the \
                         document, which is written whole on every save — so this is \
                         re-written on each autosave and copied into every snapshot \
                         and backup.\n\nAttach it anyway?"
                    ))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show(),
                rfd::MessageDialogResult::Yes
            );
            if !ok {
                self.status = format!("Did not attach {name}");
                return false;
            }
        }
        // Onto the card you dropped it on, if there is one — "the spec belongs to
        // this task" is the case worth serving. Otherwise a card of its own.
        let target = self.any_card_at(node, pos).or_else(|| {
            let cid = self.doc.add_card(node, at, CardKind::Text)?;
            if let Some(c) = self.doc.card_mut(node, cid) {
                c.title = name.clone();
                c.editing = false;
            }
            Some(cid)
        });
        let Some(cid) = target else { return false };
        self.doc.add_attachment(node, cid, bytes, name).is_some()
    }

    /// Write one attachment back out to a file the user picks.
    ///
    /// The whole point of storing bytes rather than a path: the file can be taken
    /// back out anywhere the document goes, long after whatever produced it is
    /// gone from this disk.
    fn save_attachment(&mut self, node: NodeId, card: crate::model::CardId, idx: usize) {
        let Some(att) = self.doc.attachment(node, card, idx) else { return };
        let (name, data) = (att.name.clone(), att.data.clone());
        let Some(path) = self.file_dialog().set_file_name(&name).save_file() else { return };
        match std::fs::write(&path, &data) {
            Ok(()) => self.status = format!("Saved {} \u{2192} {}", name, path.display()),
            Err(e) => self.status = format!("Could not save {name}: {e}"),
        }
    }

    fn drop_files(&mut self, node: NodeId, files: Vec<egui::DroppedFile>, pos: egui::Pos2) {
        let mut n = 0usize;
        // A vault import writes its own status line, which says far more than a
        // card count; don't let a file dropped alongside it overwrite that.
        let mut vault_status = false;
        for f in files {
            // A dropped **folder** is a vault, not a file: it has no bytes to
            // read, so before this it fell through the whole chain and did
            // nothing at all. Dragging a vault in is the gesture people try
            // first, and it is the same import the File menu offers.
            if let Some(dir) = f.path.as_ref().filter(|p| p.is_dir()) {
                let name = dir
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Vault".to_string());
                match crate::vault::read_vault(dir) {
                    Ok(vf) if !vf.is_empty() => {
                        let r = crate::vault::import_vault(&mut self.doc, None, &name, vf);
                        if let Some(nd) = self.doc.nodes.get_mut(&r.root) {
                            nd.expanded = true;
                        }
                        self.mark_dirty();
                        self.status = crate::vault::describe(&r, &name);
                    }
                    Ok(_) => self.status = format!("Nothing to import: {name} holds no files"),
                    Err(e) => self.status = format!("Import failed: {e}"),
                }
                vault_status = true;
                continue;
            }
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
            } else if let Ok(text) = String::from_utf8(bytes.clone()) {
                // A dropped Trellis JSON card file becomes that exact card; any
                // other `.json` (or text) falls back to a text card.
                let imported = ext == "json"
                    && crate::model::parse_card_export(&text)
                        .and_then(|exp| self.doc.add_card_from_export(node, at, exp))
                        .is_some();
                if imported {
                    n += 1;
                } else if let Some(cid) = self.doc.add_card(node, at, CardKind::Text) {
                    // A note from Obsidian (or Jekyll, or Hugo) arrives with its
                    // metadata in a YAML frontmatter block, which Trellis cannot
                    // read: the property parser needs `::` and YAML uses a single
                    // colon, so `due: 2026-09-01` would land as inert prose. Turn
                    // it into the lines this app actually reads, and let the rest
                    // of the file be the body.
                    let (fields, rest) = crate::model::split_frontmatter(&text);
                    // `title:` becomes the card's title below, so it must not also
                    // become a `title::` property — the round trip would grow one
                    // copy per export/import cycle.
                    let carried: Vec<(String, String)> = fields
                        .iter()
                        .filter(|(k, _)| !k.eq_ignore_ascii_case("title"))
                        .cloned()
                        .collect();
                    let front = crate::model::frontmatter_to_trellis(&carried);
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        // A `title:` field is the note's name, so it becomes the
                        // card's rather than being repeated as a property.
                        let titled = fields
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("title"))
                            .map(|(_, v)| v.clone());
                        // The note's **name** is the file name without its
                        // extension — Obsidian's own identity rule, and what the
                        // vault importer uses. A card called "Glossary.md" is
                        // the extension leaking into the title.
                        c.title = titled.unwrap_or_else(|| {
                            match name.rsplit_once('.') {
                                Some((stem, _)) if !stem.is_empty() => stem.to_string(),
                                _ => name.clone(),
                            }
                        });
                        c.body = if front.is_empty() {
                            rest.to_string()
                        } else {
                            format!("{front}\n{rest}")
                        };
                        c.editing = false;
                    }
                    n += 1;
                }
            } else if self.attach_dropped(node, bytes, name, pos, at) {
                // Anything else — a PDF, a .docx, a .zip, an .mp3. Before this it
                // fell off the end of the chain: no card, no error, no status
                // line, which is the one answer worse than a refusal.
                n += 1;
            }
        }
        if n > 0 {
            self.mark_dirty();
            if !vault_status {
                self.status =
                    format!("Added {n} card{} from dropped files", if n == 1 { "" } else { "s" });
            }
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
                CanvasAction::SaveAttachment(cid, idx) => {
                    self.save_attachment(node, cid, idx);
                }
                CanvasAction::RemoveAttachment(cid, idx) => {
                    let name = self
                        .doc
                        .attachment(node, cid, idx)
                        .map(|a| a.name.clone())
                        .unwrap_or_default();
                    if self.doc.remove_attachment(node, cid, idx) {
                        self.dirty = true;
                        self.status = format!("Removed {name}");
                    }
                }
                // Desktop MODE: the whole basket at once, which is what "the
                // cards of a workspace are on the screen" means — a per-card
                // action is the exception, not the feature.
                #[cfg(not(target_os = "linux"))]
                CanvasAction::ToggleDesktopMode => {
                    self.status =
                        "Desktop mode needs a window manager that lets an application place                          its own windows — Linux/X11 only for now".into();
                }
                #[cfg(target_os = "linux")]
                CanvasAction::ToggleDesktopMode => {
                    if self.desktop_mode == Some(node) {
                        self.recall_basket_from_desktop(node);
                    } else {
                        // Only one basket is out at a time: two baskets of
                        // windows on one desktop is a pile with no way to tell
                        // which document you are looking at.
                        if let Some(prev) = self.desktop_mode {
                            self.recall_basket_from_desktop(prev);
                        }
                        self.send_basket_to_desktop(ctx, node);
                    }
                }
                // Placement is app config, so this touches no document state and
                // must not mark the document dirty.
                CanvasAction::SendToDesktop(cid) => {
                    // Open near where the card sits on screen rather than at a
                    // fixed corner, so it lands roughly where you were looking.
                    let pos = self
                        .card_rects
                        .get(&cid)
                        .map(|r| [r.min.x + 60.0, r.min.y + 60.0])
                        .unwrap_or([200.0, 200.0]);
                    self.desktop_cards.insert(cid, pos);
                    self.status = format!("Card #{cid} sent to the desktop");
                }
                CanvasAction::RecallFromDesktop(cid) => {
                    self.desktop_cards.remove(&cid);
                    self.status = format!("Card #{cid} recalled from the desktop");
                }
                CanvasAction::AddCard(kind, pos) => {
                    self.doc.add_card(node, pos, kind);
                }
                CanvasAction::MoveCard(cid, delta) => {
                    // Dragging one card of a selection moves the whole
                    // selection — which is the point of drawing a box round
                    // them. Anything docked to each still travels with it.
                    if self.card_sel.len() > 1
                        && self.card_sel_node == Some(node)
                        && self.card_sel.contains(&cid)
                    {
                        let ids: Vec<_> = self.card_sel.iter().copied().collect();
                        for id in ids {
                            self.doc.move_card_tree(node, id, delta);
                        }
                    } else {
                        // Moves the card plus anything docked to it.
                        self.doc.move_card_tree(node, cid, delta);
                    }
                }
                CanvasAction::ExtractSelection(cid, (from, to)) => {
                    self.extract_selection(ctx, node, cid, from, to);
                }
                CanvasAction::PlaceCards(moves) => {
                    // Absolute, and each card independently: an arrangement is
                    // exactly the case where the selection must NOT travel as one.
                    for (cid, pos) in moves {
                        if let Some(c) = self.doc.card_mut(node, cid) {
                            c.pos = pos;
                        }
                    }
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
                    // Mint before borrowing the card: an item without an id is a
                    // position, not a task, and everything downstream depends on
                    // it having one from the moment it exists.
                    let id = self.doc.mint_item_id();
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        if let CardKind::Checklist { items } = &mut c.kind {
                            items.push(ChecklistItem { id, done: false, text: String::new() });
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
                CanvasAction::RunCardPlugin(cid, idx) => {
                    // The plugin is told which card, not what is in it. It reads
                    // that over the API under its own scope — so a read-only
                    // plugin invoked from a card menu is still read-only.
                    let node_title =
                        self.doc.nodes.get(&node).map(|n| n.title.clone()).unwrap_or_default();
                    let card_title =
                        self.doc.card(node, cid).map(|c| c.title.clone()).unwrap_or_default();
                    self.run_plugin(
                        idx,
                        vec![
                            ("TRELLIS_TRIGGER".into(), "card-menu".into()),
                            ("TRELLIS_NODE".into(), node.to_string()),
                            ("TRELLIS_NODE_TITLE".into(), node_title),
                            ("TRELLIS_CARD".into(), cid.to_string()),
                            ("TRELLIS_CARD_TITLE".into(), card_title),
                        ],
                    );
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
                CanvasAction::SelectCards(ids) => {
                    // Replace rather than add: a marquee is a statement about
                    // what you want selected, not an increment. Shift is already
                    // spent on drawing the box, and Ctrl+click still adds one.
                    self.card_sel = ids.into_iter().collect();
                    self.card_sel_node = Some(node);
                    self.status = match self.card_sel.len() {
                        0 => "Nothing in the box".to_string(),
                        1 => "1 card selected".to_string(),
                        n => format!("{n} cards selected — drag one to move them all"),
                    };
                }
                CanvasAction::ToggleDockMode => self.dock_mode = !self.dock_mode,
                CanvasAction::ToggleSnapMode => self.snap_mode = !self.snap_mode,
                CanvasAction::ToggleGridMode => self.grid_mode = !self.grid_mode,
                CanvasAction::ToggleDepthMode => self.depth_mode = !self.depth_mode,
                CanvasAction::ToggleTimeMode => self.time_mode = !self.time_mode,

                CanvasAction::FollowLink(target) => {
                    // Same resolution as a link in a text card: `#id` is a card,
                    // an integer is a node, then a title match.
                    self.follow_link_target(ctx, &target);
                }
                CanvasAction::RevealElsewhere(home, cid) => {
                    // Go to where the card actually lives and reveal it there —
                    // the same path the Agenda and a [[#id]] link already use.
                    self.jump_to_card(ctx, home, cid);
                }
                CanvasAction::SetZ(cid, z) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.z = z;
                    }
                }
                CanvasAction::SetEmphasis(cid, e) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.emphasis = e;
                        // Set by hand, so it never lapses. The expiry exists to
                        // stop *agents* accumulating permanent noise; a person
                        // turning one on has just said what they meant.
                        c.emphasis_until = None;
                    }
                }
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
                CanvasAction::SetSourceTail(cid, n) => {
                    if let Some(c) = self.doc.card_mut(node, cid) {
                        c.source_tail = n;
                        // Force the next poll to re-read: the mtime has not moved,
                        // but what we want out of the file has.
                        c.source_mtime = None;
                    }
                    self.pump_sources(true);
                    self.status = match n {
                        Some(n) => format!("Tailing the last {n} lines"),
                        None => "Showing the whole file".into(),
                    };
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
                    // Reset view means *straight on* as well as unzoomed —
                    // otherwise an orbit you cannot undo is one click away.
                    self.eyes.insert(node, egui::Vec2::ZERO);
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
                        ui.separator();
                        if ui
                            .button("Obsidian vault…")
                            .on_hover_text(
                                "A folder of Markdown notes becomes a tree of baskets: folder \
                                 → basket, note → card, frontmatter → key:: value, and \
                                 ![[file.pdf]] → an attachment on the card that names it. \
                                 [[Note]] links are rewritten to card links so they resolve.",
                            )
                            .clicked()
                        {
                            self.import_vault();
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
                    if ui
                        .button("Restart")
                        .on_hover_text(
                            "Save and start this same instance again — same document, same \
                             port, same data directory. What you need after installing a new \
                             build.",
                        )
                        .clicked()
                    {
                        self.restart();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close_menu();
                    }
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
                    // Only shown when a journal root has been chosen. An action
                    // that silently does nothing is worse than an absent one.
                    if self.daily_root.is_some() {
                        if ui
                            .button("Today's note")
                            .on_hover_text("Ctrl+T — open today's journal node, creating it if needed")
                            .clicked()
                        {
                            self.go_to_today();
                            ui.close_menu();
                        }
                        ui.separator();
                    }
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
                        .button("Random card")
                        .on_hover_text(
                            "Open a card at random, anywhere in the document — for \
                             rediscovering what you wrote and forgot",
                        )
                        .clicked()
                    {
                        self.go_to_random_card(ui.ctx());
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
                    {
                        // Count it up front so the menu itself reports the
                        // problem: a currency panel nobody opens is no better
                        // than the stale card it was built to catch.
                        let stale = api::stale_claim_count(&self.doc);
                        let label = if stale > 0 {
                            format!("Claims… ({stale} to re-check)")
                        } else {
                            "Claims…".to_string()
                        };
                        if ui
                            .button(label)
                            .on_hover_text(
                                "Cards that assert state and say when to re-check it (verify::)",
                            )
                            .clicked()
                        {
                            self.claims_open = true;
                            ui.close_menu();
                        }
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
                    ui.menu_button("Sort projects", |ui| {
                        ui.label(
                            egui::RichText::new(
                                "Orders the top-level projects only. Sub-nodes keep the order \
                                 you gave them — inside a project, order is usually meaning.",
                            )
                            .weak()
                            .small(),
                        );
                        ui.separator();
                        for (opt, label) in crate::tree::TreeSort::ALL {
                            if ui
                                .selectable_label(self.tree_sort == opt, label)
                                .clicked()
                            {
                                self.tree_sort = opt;
                                self.status = if opt == crate::tree::TreeSort::Manual {
                                    "Projects back in the document's own order".to_string()
                                } else {
                                    format!("Projects sorted by {}", label.to_lowercase())
                                };
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        ui.label(
                            egui::RichText::new(
                                "This is a view: the document keeps its own order, so a new \
                                 project simply appears in the right place instead of at the \
                                 bottom waiting to be dragged.",
                            )
                            .weak()
                            .small(),
                        );
                    });
                    ui.separator();
                    // Whole-tree folding: a menu item, not a header button. It
                    // moves every node in the document, which is not something a
                    // stray click on the way to a menu should be able to do.
                    if ui
                        .button("Collapse the whole tree")
                        .on_hover_text("Fold every root and everything under it")
                        .clicked()
                    {
                        self.apply_tree(vec![TreeAction::SetAllExpanded(false)]);
                        ui.close_menu();
                    }
                    if ui
                        .button("Expand the whole tree")
                        .on_hover_text("Open every root and everything under it")
                        .clicked()
                    {
                        self.apply_tree(vec![TreeAction::SetAllExpanded(true)]);
                        ui.close_menu();
                    }
                    ui.separator();
                    // The two axes live together here as well as on the canvas —
                    // named, so the pair has a reason to be a pair.
                    ui.menu_button("Hypercube", |ui| {
                        ui.label(
                            egui::RichText::new("A basket is x and y. These add z and time.")
                                .weak()
                                .small(),
                        );
                        ui.separator();
                        ui.checkbox(&mut self.depth_mode, "Depth (z) — a basket is a volume")
                            .on_hover_text(
                                "Shift+scroll over a card slides it in z; Alt+drag looks \
                                 around. Off, z is the stacking order and nothing is lost.",
                            );
                        ui.checkbox(&mut self.time_mode, "Time — a task spans the days it covers")
                            .on_hover_text(
                                "A card with start:: and due:: appears in every day between \
                                 them — the same card, not a copy.",
                            );
                        ui.separator();
                        let both = self.depth_mode && self.time_mode;
                        if ui
                            .button(if both { "Turn both off" } else { "Turn both on" })
                            .clicked()
                        {
                            self.depth_mode = !both;
                            self.time_mode = !both;
                            ui.close_menu();
                        }
                    });
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
                        .button(format!("Register {}:// links…", crate::URL_SCHEME))
                        .on_hover_text(
                            "Teach this desktop to open a link like trellis://127.0.0.1:7374/card/1391 in \
                             Trellis. Without it the links still work from a terminal, and the \
                             http://127.0.0.1 form works anywhere — this is what makes them \
                             clickable in a browser or a chat window.",
                        )
                        .clicked()
                    {
                        self.status = match register_url_scheme() {
                            Ok(path) => {
                                self.url_scheme_registered =
                                    std::env::current_exe().ok().map(|p| p.display().to_string());
                                format!("Registered {}:// — {path}", crate::URL_SCHEME)
                            }
                            Err(e) => format!("Could not register the link scheme: {e}"),
                        };
                        ui.close_menu();
                    }
                    ui.separator();
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
                        .add_enabled(self.selected.is_some(), egui::Button::new("Fix overlapping cards"))
                        .on_hover_text(
                            "Push overlapping cards down until nothing is covered, keeping the \
                             layout — every card's left edge stays put. Use this instead of \
                             Autosort on a basket you arranged yourself.",
                        )
                        .clicked()
                    {
                        if let Some(sel) = self.selected {
                            self.push_undo(sel);
                            let moved = self.doc.resolve_overlaps(sel);
                            if moved > 0 {
                                self.mark_dirty();
                                self.status = format!("Moved {moved} card(s) clear");
                            } else {
                                self.undo.pop(); // nothing changed; drop the snapshot
                                self.status = "Nothing overlaps in this basket".to_string();
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
        let mut to_cancel: Option<String> = None;
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
                            let prog = self.plugin_progress.get(&p.manifest.name);
                            // A bar only when the plugin reports a percentage.
                            // A bar that isn't measuring anything is a lie about
                            // how far along the run is.
                            match prog.and_then(|pr| pr.percent) {
                                Some(pct) => {
                                    ui.add(
                                        egui::ProgressBar::new(pct / 100.0)
                                            .desired_width(140.0)
                                            .show_percentage(),
                                    );
                                }
                                None => {
                                    ui.label(egui::RichText::new("running…").weak());
                                }
                            }
                            if let Some(msg) = prog.map(|pr| pr.message.as_str()).filter(|m| !m.is_empty()) {
                                ui.label(egui::RichText::new(msg).weak().small());
                            }
                            if ui
                                .button("Cancel")
                                .on_hover_text("Stop the plugin. Anything it already wrote stays.")
                                .clicked()
                            {
                                to_cancel = Some(p.manifest.name.clone());
                            }
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
                                if r.cancelled {
                                    // Neither tick nor cross: you stopped it, so
                                    // it neither succeeded nor went wrong.
                                    ui.colored_label(egui::Color32::from_rgb(180, 180, 180), "■");
                                } else if r.ok {
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
        if let Some(name) = to_cancel {
            self.cancel_plugin(&name);
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
                // Scrolls and is capped to the screen: this window holds a
                // list that grows, and a window that simply grows with it runs
                // off the display.
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height(window_body_max_height(ctx))
                    .show(ui, |ui| {
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

    /// Issue and revoke the tokens held by agents elsewhere — one per agent,
    /// named after it, each confined to its own basket.
    ///
    /// This exists because the alternative people actually reach for is handing
    /// out the instance key, which is unrestricted and can only be revoked by
    /// regenerating it and breaking every other client at once.
    fn agent_tokens_ui(&mut self, ui: &mut egui::Ui, doc_title: &str) {
        let existing = self.agent_tokens();
        if existing.is_empty() {
            ui.label(
                egui::RichText::new(
                    "None. An agent on your network needs a token of its own — the API \
                     key above is unrestricted and revoking it breaks every client.",
                )
                .weak()
                .small(),
            );
        }
        let mut revoke: Option<String> = None;
        for (label, scope, token) in &existing {
            let basket = scope.subtree.and_then(|n| self.doc.nodes.get(&n)).map(|n| n.title.clone());
            ui.horizontal(|ui| {
                ui.strong(label);
                let sentence = scope.describe_named(doc_title, basket.as_deref());
                ui.label(
                    egui::RichText::new(format!("can {sentence}")).color(if scope.subtree.is_none()
                    {
                        egui::Color32::from_rgb(230, 160, 60)
                    } else {
                        ui.visuals().weak_text_color()
                    }),
                );
                if ui
                    .small_button("Copy")
                    .on_hover_text("Copy this token, to paste into the agent's configuration")
                    .clicked()
                {
                    ui.ctx().copy_text(token.clone());
                }
                if ui
                    .small_button("Revoke")
                    .on_hover_text("This token stops working immediately. Others are unaffected.")
                    .clicked()
                {
                    revoke = Some(label.clone());
                }
            });
            // A basket that has been deleted since would silently widen nothing
            // — the scope still refuses — but the row would read as if the token
            // were fine, so say so.
            if scope.subtree.is_some() && basket.is_none() {
                ui.label(
                    egui::RichText::new(
                        "  ↳ its basket no longer exists; every request is refused",
                    )
                    .color(egui::Color32::from_rgb(230, 100, 100))
                    .small(),
                );
            }
        }
        if let Some(label) = revoke {
            self.revoke_agent_token(&label);
            if self.new_token_minted.as_ref().map(|(l, _)| l == &label).unwrap_or(false) {
                self.new_token_minted = None;
            }
            self.status = format!("Revoked {label}'s token");
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_token_label)
                    .desired_width(140.0)
                    .hint_text("agent's name"),
            );
            let label = self.new_token_label.trim().to_string();
            // Path, not just title: two baskets called "Notes" under different
            // projects are indistinguishable otherwise, and picking the wrong
            // one here is a permission mistake.
            let current = match self.new_token_target {
                TokenTarget::NewBasket => {
                    if label.is_empty() {
                        "a new basket".to_string()
                    } else {
                        format!("a new basket “{label}”")
                    }
                }
                TokenTarget::Existing(id) => crate::tree::node_path(&self.doc, id),
                TokenTarget::WholeDocument => "the whole document".to_string(),
            };
            egui::ComboBox::from_id_salt("agent_token_target")
                .selected_text(current)
                .width(240.0)
                .show_ui(ui, |ui| {
                    let new_label = if label.is_empty() {
                        "a new basket".to_string()
                    } else {
                        format!("a new basket “{label}”")
                    };
                    ui.selectable_value(&mut self.new_token_target, TokenTarget::NewBasket, new_label);
                    let mut ids: Vec<NodeId> = self.doc.nodes.keys().copied().collect();
                    ids.sort_unstable();
                    for id in ids {
                        let path = crate::tree::node_path(&self.doc, id);
                        ui.selectable_value(&mut self.new_token_target, TokenTarget::Existing(id), path);
                    }
                    ui.separator();
                    ui.selectable_value(
                        &mut self.new_token_target,
                        TokenTarget::WholeDocument,
                        "the whole document (no limit)",
                    );
                });
            ui.checkbox(&mut self.new_token_read_only, "read-only");
            if ui.button("Issue token").clicked() {
                self.issue_agent_token();
            }
        });
        if !self.new_token_error.is_empty() {
            ui.label(
                egui::RichText::new(&self.new_token_error)
                    .color(egui::Color32::from_rgb(230, 100, 100))
                    .small(),
            );
        }
        if let Some((label, token)) = self.new_token_minted.clone() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{label}:")).strong());
                ui.add(
                    egui::TextEdit::singleline(&mut token.clone())
                        .desired_width(300.0)
                        .font(egui::TextStyle::Monospace),
                );
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(token.clone());
                }
            });
            ui.label(
                egui::RichText::new(
                    "Send as X-API-Key (or Authorization: Bearer). It stays listed above, \
                     so you can copy it again — it is stored in this instance's settings, \
                     and pretending otherwise would just make you mint a second one.",
                )
                .weak()
                .small(),
            );
        }
        if self.api_lan {
            ui.label(
                egui::RichText::new(
                    "The API is plain HTTP: a token crossing your network is readable by \
                     anything on it.",
                )
                .weak()
                .small(),
            );
        }
    }

    /// Mint the token the form describes, creating its basket if that's what was
    /// asked for.
    fn issue_agent_token(&mut self) {
        self.new_token_error.clear();
        let label = self.new_token_label.trim().to_string();
        if label.is_empty() {
            self.new_token_error = "Give the token a name — the agent's own name.".into();
            return;
        }
        let subtree = match self.new_token_target {
            TokenTarget::WholeDocument => None,
            TokenTarget::Existing(id) => {
                if !self.doc.nodes.contains_key(&id) {
                    self.new_token_error = "That basket no longer exists.".into();
                    return;
                }
                Some(id)
            }
            TokenTarget::NewBasket => {
                // Named after the agent, so the basket and the token that can
                // reach it carry the same name in both places.
                let id = self.doc.add_node(None, label.clone());
                self.note(
                    crate::changelog::Change::new(
                        crate::changelog::Actor::Ui,
                        crate::changelog::Entity::Node,
                        crate::changelog::Op::Created,
                        id,
                    )
                    .titled(label.clone()),
                );
                Some(id)
            }
        };
        let scope = crate::plugins::Scope { read_only: self.new_token_read_only, subtree };
        match self.mint_agent_token(&label, scope) {
            Ok(_token) => {
                let token = self
                    .agent_tokens()
                    .into_iter()
                    .find(|(l, _, _)| l == &label)
                    .map(|(_, _, t)| t)
                    .unwrap_or_default();
                self.new_token_minted = Some((label.clone(), token));
                self.status = format!("Issued a token to {label}");
                self.new_token_label.clear();
                self.new_token_target = TokenTarget::NewBasket;
            }
            Err(e) => self.new_token_error = e,
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let doc_title = doc_display_name(self.doc_path.as_deref());
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                // The body scrolls, and is capped to the screen. Without this the
                // window grew to whatever its content needed — and since it is
                // anchored to the centre and not resizable, expanding `Endpoints`
                // pushed both ends of the list off-screen with no way to reach
                // them. A settings window you cannot read the bottom of is the
                // same defect as a doc surface nobody updated.
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height(window_body_max_height(ctx))
                    .show(ui, |ui| {
                    ui.heading("Agent API");
                    // Links are how an agent hands you a place, so this belongs
                    // beside the key and the endpoint list rather than in Canvas.
                    ui.horizontal(|ui| {
                        let scheme = crate::URL_SCHEME;
                        match &self.url_scheme_registered {
                            Some(p) if !p.is_empty() => {
                                ui.weak(format!("{scheme}:// links open this build"))
                                    .on_hover_text(format!("Registered for {p}"));
                            }
                            _ => {
                                ui.weak(format!("{scheme}:// links are not registered on this desktop"));
                            }
                        }
                        if ui
                            .button("Register now")
                            .on_hover_text(
                                "Done automatically on a new install and whenever the binary moves. \
                                 Use this if a link stopped opening — or after installing Trellis \
                                 somewhere new. The http://127.0.0.1:<port>/open/… form needs no \
                                 registration and works anywhere.",
                            )
                            .clicked()
                        {
                            match register_url_scheme() {
                                Ok(path) => {
                                    self.url_scheme_registered =
                                        std::env::current_exe().ok().map(|p| p.display().to_string());
                                    self.status = format!("Registered {scheme}:// — {path}");
                                }
                                Err(e) => self.status = format!("Could not register: {e}"),
                            }
                        }
                    });
                    ui.add_space(4.0);
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

                        ui.label("Agent tokens");
                        ui.vertical(|ui| {
                            self.agent_tokens_ui(ui, &doc_title);
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
                    ui.heading("Daily notes");
                    ui.small(
                        egui::RichText::new(
                            "Off unless you choose a journal root. Trellis then keeps \
                             <root> → <month> → <day> under it, and Ctrl+T opens today's node, \
                             creating it only if it isn't there. Nothing dated is ever created \
                             any other way. This setting belongs to this instance, so one \
                             document can keep a journal while another never grows one.",
                        )
                        .weak(),
                    );
                    let current = self
                        .daily_root
                        .and_then(|id| self.doc.nodes.get(&id).map(|n| (id, n.title.clone())));
                    ui.horizontal(|ui| {
                        ui.label("Journal root:");
                        match &current {
                            Some((id, title)) => {
                                ui.label(egui::RichText::new(format!("{title}  (#{id})")).strong());
                            }
                            None => {
                                ui.label(egui::RichText::new("none — daily notes are off").weak());
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        let sel = self.selected.and_then(|id| {
                            self.doc.nodes.get(&id).map(|n| (id, n.title.clone()))
                        });
                        let label = match &sel {
                            Some((_, t)) => format!("Use selected node: {t}"),
                            None => "Select a node in the tree first".to_string(),
                        };
                        if ui
                            .add_enabled(sel.is_some(), egui::Button::new(label))
                            .on_hover_text(
                                "Point it at the node holding your journal — for a year-per-root \
                                 tree, the year itself. When the year turns over, Trellis moves to \
                                 that year's sibling rather than nesting the new year inside the old.",
                            )
                            .clicked()
                        {
                            if let Some((id, _)) = sel {
                                self.daily_root = Some(id);
                            }
                        }
                        if ui
                            .add_enabled(self.daily_root.is_some(), egui::Button::new("Turn off"))
                            .on_hover_text("Stops Ctrl+T and POST /api/daily. Nothing is deleted.")
                            .clicked()
                        {
                            self.daily_root = None;
                        }
                    });

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
                    ui.separator();
                    ui.label(egui::RichText::new("Notifications").strong());
                    ui.label(
                        egui::RichText::new(
                            "Desktop notifications, from this instance, about this document. \
                             They only fire while Trellis is running — a desktop app is not a \
                             service — and never while this window has focus, because an edit \
                             you can see does not need announcing. For something that reaches \
                             you with Trellis closed, and that waits to be dealt with rather \
                             than being swiped away, use the Telegram plugin.",
                        )
                        .weak()
                        .small(),
                    );
                    ui.checkbox(&mut self.notify_digest, "On startup: what is overdue or due today")
                        .on_hover_text(
                            "Sent once when the document opens, and only when there is something \
                             to say. A notifier that reports \"nothing due\" is one you learn to \
                             ignore.",
                        );
                    ui.checkbox(&mut self.notify_agent, "When an agent changes something")
                        .on_hover_text(
                            "Changes that arrive over the API while you are in another window. \
                             One notification per batch, not one per card.",
                        );
                    if !cfg!(target_os = "linux") || crate::deps::which("notify-send").is_some() {
                        // Nothing to say: the tool is there.
                    } else {
                        ui.label(
                            egui::RichText::new(
                                "notify-send is not installed, so nothing can be delivered — \
                                 Tools → Requirements… installs it.",
                            )
                            .weak()
                            .small(),
                        );
                    }
                    ui.separator();
                    ui.checkbox(&mut self.dock_mode, "Dock mode (drag a card onto another to stick it)")
                        .on_hover_text(
                            "When on, dropping a card on another docks them so they move together; \
                             drag a docked card off to detach. Grouping works regardless.",
                        );
                    ui.checkbox(&mut self.grid_mode, "Grid mode (quantise a card to the canvas grid)")
                        .on_hover_text(
                            "Snap still wins where it applies: an axis aligned to \
                             another card's edge is left alone, and only an axis no \
                             card claimed is quantised.",
                        );
                    ui.checkbox(&mut self.snap_mode, "Snap mode (align card edges while dragging)")
                        .on_hover_text("When on, a dragged card's edges snap to nearby cards' edges.");
                    if cfg!(target_os = "linux") {
                        let sel = self.selected;
                        let on = sel.is_some() && self.desktop_mode == sel;
                        let mut want = on;
                        let r = ui.checkbox(
                            &mut want,
                            "Desktop mode (this basket's cards become windows on your desktop)",
                        );
                        r.on_hover_text(
                            // Continued with `\` on every line: written as a plain
                            // multi-line literal, the source indentation became part
                            // of the tooltip, which showed a 26-space gap mid-sentence.
                            "Every card in the open basket becomes its own borderless \
                             window, keeping the arrangement it has here — move the \
                             Trellis window away and the cards stay on your desktop \
                             among your other applications. Turn it off to bring them \
                             all back.\n\nLinux/X11 only: elsewhere an application may \
                             not place its own windows.",
                        );
                        if want != on {
                            if let Some(n) = sel {
                                #[cfg(target_os = "linux")]
                                if want {
                                    if let Some(prev) = self.desktop_mode {
                                        self.recall_basket_from_desktop(prev);
                                    }
                                    let c = ctx.clone();
                                    self.send_basket_to_desktop(&c, n);
                                } else {
                                    self.recall_basket_from_desktop(n);
                                }
                            }
                        }
                    }
                    ui.label(
                        egui::RichText::new(
                            "Hypercube — a basket is x and y; these two add z and time.",
                        )
                        .strong(),
                    );
                    ui.checkbox(&mut self.depth_mode, "Depth (z) — the basket is a volume")
                        .on_hover_text(
                            "Cards get a real depth instead of a stacking order: near ones are \
                             larger and cover far ones, and Shift+scroll over a card slides it \
                             toward or away from you. Off is exactly the flat canvas — a card's \
                             depth is kept either way, so turning this off never loses an \
                             arrangement, and with it off the depth is simply the stacking order.",
                        );
                    ui.checkbox(&mut self.time_mode, "Time — a task is present on every day it spans")
                        .on_hover_text(
                            "A card carrying start:: and due:: is shown in every day between them, \
                             as the same card — one id, one truth, edited in any of them. Off, a \
                             day shows only the cards that live in it, exactly as now.",
                        );
                    // Colour emoji come from a font on the machine, not from the
                    // app: say which one, because "still grey" otherwise looks like
                    // a bug rather than a missing font.
                    ui.add_space(4.0);
                    ui.weak(self.emoji.status())
                        .on_hover_text(
                            "Emoji are drawn from a colour font's bitmaps, painted over the text. \
                             Windows' Segoe UI Emoji stores its colour as vector layers rather than \
                             bitmaps, so it can't be used this way and emoji stay monochrome there.",
                        );

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
                                "A link that opens a card on your PHONE",
                                format!(
                                    "# lan_host is an address the phone can reach this machine on.\n\
                                     curl -s -H 'X-API-Key: {k}' {a}/instance   # read lan_host\n\
                                     # then the link is simply:\n\
                                     http://<lan_host>:{port}/go/card/1391\n\
                                     # Telegram strips a trellis:// link silently, so a message needs\n\
                                     # this http hop; the page it serves opens the app on the phone."
                                ),
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
                            (
                                "Act on a card by its id alone — no basket lookup",
                                format!(
                                    "curl -H 'X-API-Key: {k}' -d '{{\"key\":\"status\",\"value\":\"done\"}}' \\\n  \
                                     {a}/cards/1391/property
                                     # and take its date off, so it LEAVES the agenda:
                                     curl -X DELETE -H 'X-API-Key: {k}' '{a}/cards/1391/property?key=due'"
                                ),
                            ),
                            (
                                "Add to a card we both write to (no read-modify-write)",
                                format!(
                                    "curl -H 'X-API-Key: {k}' -H 'Content-Type: application/json' \\\n  \
                                     -d '{{\"text\":\"**note** — appended on the server\"}}' \\\n  \
                                     {a}/cards/1391/append"
                                ),
                            ),
                            (
                                "A whole list at once — any baskets (what /tasks hands you)",
                                format!(
                                    "curl -H 'X-API-Key: {k}' '{a}/cards?ids=1391,1392'
                                     curl -H 'X-API-Key: {k}' -H 'Content-Type: application/json' \\\n  \
                                     -d '{{\"cards\":[1391,1392],\"key\":\"status\",\"value\":\"done\"}}' \\\n  \
                                     {a}/cards/property"
                                ),
                            ),
                            (
                                "Archive a basket's finished cards in one call (ids survive)",
                                format!(
                                    "curl -H 'X-API-Key: {k}' -H 'Content-Type: application/json' \\\n  \
                                     -d '{{\"cards\":[1836,1837],\"node\":378,\"pos\":[40,40],\"gap\":20}}' \\\n  \
                                     {a}/nodes/1/cards/move"
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
                        "GET    /api/docs[?section=examples]   → THIS build's API.md, compiled in (any scope; ~100KB whole)",
                            "GET    /api/settings   → theme, canvas toggles, panels, notifications, retention",
                            "POST   /api/settings   {theme?, tree_sort?, minimap?, snap_mode?, grid_mode?, notify_digest?, …}",
                            "GET    /api/tree",
                            "GET    /api/nodes",
                            "POST   /api/nodes               {parent?, title}",
                            "GET    /api/nodes/{id}",
                            "PATCH  /api/nodes/{id}          {title?, color?, bg?}",
                            "DELETE /api/nodes/{id}",
                            "POST   /api/nodes/{id}/move     {before|after|index|to, parent?}",
                            "POST   /api/nodes/{id}/expand   {expanded, recursive?}",
                            "POST   /api/expand              {expanded}   (the whole tree)",
                            "GET    /api/nodes/{id}/backlinks          (cards that [[link]] here)",
                            "GET    /api/graph                         (wiki-link nodes + edges)",
                            "GET    /api/nodes/{id}/cards",
                            "GET    /api/nodes/{id}/cards/{cid}        (one card, without the whole basket)",
                            "GET    /api/cards/{cid}                   (find a card from its id alone → {node, node_path, card})",
                            "GET    /api/cards/{cid}/link              (canonical trellis:// link for this card)",
                            "GET    /open/card/{cid} · /open/node/{id} · /open/group/{gid}  (no key — what a trellis:// link opens)",
                            "GET    /go/card/{cid} · /go/node/{id} · /go/group/{gid}   (no key — a PAGE that opens the card on the PHONE that loaded it; Telegram strips trellis:// links, so a notification needs this http hop. Build it with lan_host from /api/instance)",
                            "GET    /api/cards/{cid}/backlinks         (cards whose [[#id]] links point at this card)",
                        "         every card carries `empty` — a checklist/table has NO body, so never read body alone to decide a card is blank",
                        "         a bare [[Title]] resolves to the linking card's own project first, then the lowest node id (stable across runs)",
                            "GET    /api/cards?ids=1391,1392           (read a LIST of cards, any baskets; 'missing' names the ids that are gone)",
                            "POST   /api/cards/property {cards:[ids], key, value}  ·  DELETE …/property {cards, key}",
                            "         one property, cards ANYWHERE — for the id lists /api/tasks and /api/claims hand back (whole-document: no confined tokens)",
                            "PATCH  /api/cards/{cid}   ·  DELETE /api/cards/{cid}          (a card id is a complete address for WRITES too)",
                            "POST   /api/cards/{cid}/property {key,value}  ·  DELETE …/property?key=due",
                            "POST   /api/cards/{cid}/move {node,pos?} | {before|after|index|to}",
                            "POST   /api/cards/{cid}/items/{item}/done {done}  ·  …/items/{item}/property (POST/DELETE)",
                            "POST   /api/cards/{cid}/append {text, at?, separator?}        (add to a shared card without sending the body back)",
                            "POST   /api/cards/{cid}/items  {text, done?, at?}  ·  DELETE …/items/{item}   (one line; ids of the rest stay put)",
                            "         same operations as the /nodes/{id}/cards/{cid}/… twins — no need to look the basket up first",
                            "GET    /api/groups/{gid}                  (find a group from its id alone → {node, node_path, group})",
                            "GET    /api/groups/{gid}/link             (canonical trellis:// link + the [[#g…]] form)",
                            "GET    /api/groups/{gid}/backlinks        (cards whose [[#g…]] links point at this group)",
                            "POST   /api/nodes/{id}/cards/{cid}/items/{item}/property {key, value}   (one checklist LINE)",
                            "DELETE /api/nodes/{id}/cards/{cid}/items/{item}/property?key=due",
                            "POST   /api/nodes/{id}/cards/{cid}/items/{item}/done     {done}   (tick a line)",
                            "POST   /api/daily  {date?}                (a day's journal node, created on demand; opt-in per instance)",
                            "GET    /api/daily                         (is it on, and which node is the journal root)",
                            "POST   /api/daily/root {node}   /   DELETE /api/daily/root   (turn it on / off)",
                            "POST   /api/nodes/{id}/cards    {kind, title?, body?, lang?, items?, rows?, header?, pos?, z?, size?, fit?, image_base64?, inline_images?, source?}",
                            "PATCH  /api/nodes/{id}/cards/{cid}       {title?, body?, kind?, color?, font_scale?, fit?, pos?, z?, size?, items?, source?, emphasis?, emphasis_intensity?, emphasis_minutes?, …}",
                            "         source: mirror a file — text/code fill the body, TABLE cards fill cells from CSV/TSV; source:\"\" detaches",
                            "DELETE /api/nodes/{id}/cards/{cid}",
                            "POST   /api/nodes/{id}/cards/{cid}/move  {before|after|index|to} (or {node,pos?} → another basket)",
                            "POST   /api/nodes/{id}/cards    [ {…}, {…} ]      (an ARRAY creates a batch; ids come back in order)",
                            "POST   /api/nodes/{id}/cards/move        {cards:[ids], node, pos?, gap?}  (batch; whole list validated first)",
                            "POST   /api/nodes/{id}/cards/property    {cards:[ids], key, value}        (one property, many cards)",
                            "DELETE /api/nodes/{id}/cards/property    {cards:[ids], key}               (take it back off them; key in the BODY here)",
                            "PATCH  /api/nodes/{id}/cards             {cards:[ids], color?, size?, fit?, font_scale?, z?, emphasis?…}",
                            "         presentation only — title/body/items/rows/kind/lang/header/source are refused BY NAME (one card at a time)",
                            "DELETE /api/nodes/{id}/cards             {cards:[ids]}                    (validated in full first; no 'all' form)",
                            "POST   /api/nodes/{id}/desktop           (DESKTOP MODE — the whole basket becomes windows; DELETE brings it back)",
                            "GET    /api/desktop                      (cards out on the desktop as their own windows; Linux/X11)",
                            "POST   /api/cards/{cid}/desktop {pos?}   (send a card to the desktop; DELETE recalls it)",
                            "POST   /api/nodes/{id}/cards/{cid}/property {key, value}   (set key:: value)",
                            "DELETE /api/nodes/{id}/cards/{cid}/property?key=due        (remove the line; not the same as value:\"\")",
                            "POST   /api/nodes/{id}/cards/{cid}/dock  {anchor}          (unstick: DELETE …/dock)",
                            "POST   /api/nodes/{id}/cards/{cid}/group {group}           (remove: DELETE …/group)",
                            "POST   /api/nodes/{id}/cards/{cid}/table {op, …}           (set_cell / insert_row / set_col_width / autofit_cols {col?} …)",
                            "         …or send an ARRAY of ops, applied in order; a failure names which one",
                            "         set_rules {rules:[{col?,when,value,bg?,fg?}]}  colour cells by value (gt/lt/ge/le/eq/ne/contains/empty)",
                            "POST   /api/nodes/{id}/cards/{cid}/chart {kind, label_col?, value_cols?, show_table?}  (bar|line|scatter|pie; DELETE …/chart = plain grid)",
                            "POST   /api/nodes/{id}/cards/{cid}/sketch {op, …}          (add_stroke / undo / clear)",
                            "POST   /api/nodes/{id}/cards/{cid}/images {data_base64}    (GET / DELETE …/images/{idx})",
                            "  NOTE  `body` is REFUSED (400) on checklist/table/image/sketch — their items/rows/bytes are the content, and text in their body is never read as a property. Send `items` or `rows`. Judged on the kind the card WILL be, so {kind:\"text\", body:…} still converts.",
                            "GET    /api/nodes/{id}/cards/{cid}/attachments             (files carried by ANY card — the bytes, not a path; names + sizes only)",
                            "POST   /api/nodes/{id}/cards/{cid}/attachments {name, data_base64}  ·  GET / DELETE …/attachments/{idx}   (the document is written WHOLE on every save, so size costs every autosave, snapshot and backup — attachment_bytes on /api/instance is the running total)",
                            "GET    /api/nodes/{id}/groups             (POST create {cards,title?} / PATCH / DELETE {gid})",
                            "POST   /api/nodes/{id}/groups/{gid}/move  {node, pos?}     (the whole group — container, members and id)",
                            "POST   /api/nodes/{id}/autosort",
                            "GET    /api/nodes/{id}/overlaps           (which cards cover each other)",
                            "POST   /api/nodes/{id}/overlaps           (push them clear, keeping x)",
                            "GET    /api/search?q=...                  (hits carry node + card)",
                            "GET    /api/tags[?name=<tag>]             (all tags / cards with a tag)",
                            "GET    /api/properties[?key=<k>&value=<v>]   (keys / matching cards)",
                            "GET    /api/properties/problems           (due::/start::/verify:: values that will not parse)",
                            "GET    /api/query?tag=&key=&value=&text=  (combined card query)",
                            "GET    /api/cards/{cid}/run               (a saved view card's rows; set one with PATCH {view:{…}})",
                            "GET    /api/tasks[?all=true][&project=<id>]  (due:: agenda, bucketed)",
                            "GET    /api/kanban[?project=<id>]         (cards grouped by status:: → columns)",
                            "GET    /api/claims[?expired=true][&project=<id>]  (verify:: — which stated facts are out of date)",
                            "POST   /api/ocr                           (OCR all un-OCR'd images)",
                            "GET    /api/export?format=markdown|html|json|pdf|png|gif",
                            "POST   /api/import/vault       {path, parent?}  (an Obsidian vault → baskets; .canvas → a basket)",
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
                // Scrolls and is capped to the screen: this window holds a
                // list that grows, and a window that simply grows with it runs
                // off the display.
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .max_height(window_body_max_height(ctx))
                    .show(ui, |ui| {
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

    /// Ctrl+O: a centered palette to fuzzy-jump to any node by title or path —
    /// or straight to a node **or card** by its id.
    fn quick_switcher(&mut self, ctx: &egui::Context) {
        let q = self.switcher_query.to_lowercase();
        // A bare id jumps straight to what it names. Ids are what the API,
        // `/api/tree` and every error message talk in, so typing one you just
        // read somewhere is the obvious thing to try — and it used to find
        // nothing at all unless the number happened to appear in a title.
        // `#12` works too, since that's how ids are written in the docs.
        //
        // **Cards are searched as well as nodes**, because a card id is what an
        // agent quotes most: they run into the thousands while node ids stop in
        // the hundreds, so nearly every id an operator was handed resolved to
        // nothing here. One number can name both a node and a card — they are
        // separate id spaces — so both rows are offered rather than guessing.
        let typed = queried_node_id(&q);
        let by_id = typed.filter(|id| self.doc.nodes.contains_key(id));
        let card_by_id = typed.and_then(|id| self.doc.locate_card(id).map(|node| (node, id)));
        let mut matches: Vec<SwitcherHit> = Vec::new();
        if let Some((node, card)) = card_by_id {
            let path = crate::tree::node_path(&self.doc, node);
            let label = self
                .doc
                .card(node, card)
                .map(card_label)
                .unwrap_or_else(|| "(untitled card)".to_string());
            // Just above a node id match, so that when a number is both, the
            // node — the coarser, more likely target — still leads.
            matches.push(SwitcherHit { node, card: Some(card), group: None, id: card, label, path, score: i32::MIN + 1 });
        }
        // A group id is typed `g146`, so it can never collide with the node/card
        // rows above — nothing else in the palette parses a leading letter.
        if let Some(gid) = queried_group_id(&q) {
            if let Some(node) = self.doc.locate_group(gid) {
                let path = crate::tree::node_path(&self.doc, node);
                let label = self
                    .doc
                    .nodes
                    .get(&node)
                    .and_then(|n| n.groups.iter().find(|g| g.id == gid))
                    .map(|g| {
                        if g.title.trim().is_empty() {
                            "(untitled group)".to_string()
                        } else {
                            g.title.clone()
                        }
                    })
                    .unwrap_or_else(|| "(untitled group)".to_string());
                matches.push(SwitcherHit {
                    node,
                    card: None,
                    group: Some(gid),
                    id: gid,
                    label,
                    path,
                    score: i32::MIN + 1,
                });
            }
        }
        // **Card titles, below every basket.** The palette could resolve a card by
        // its *id* since v0.87.0 but never by its name, so the one thing you
        // actually remember about a card was the one thing you could not type.
        //
        // Three rules keep it reach rather than discovery, which is Ctrl+F's job:
        // only the **title** is matched (never the body); a card with **no title**
        // is skipped entirely, because matching its body-derived label would make
        // the palette answer with rows nobody can predict; and nothing is offered
        // for an **empty query**, where `fuzzy_score` matches everything and the
        // list would become every card in the document.
        //
        // `CARD_SCORE_BASE` puts every card after every basket rather than
        // interleaving them by score. That is the same call the id rows already
        // make — the basket is the coarser, likelier target — and it means the
        // palette's first screen never changes shape because a card happened to
        // score well.
        const CARD_SCORE_BASE: i32 = 10_000;
        if !q.is_empty() {
            for (&nid, n) in &self.doc.nodes {
                let mut path: Option<String> = None;
                for c in &n.cards {
                    if c.title.trim().is_empty() || Some(c.id) == card_by_id.map(|(_, c)| c) {
                        continue;
                    }
                    let title_lc = c.title.to_lowercase();
                    let p = path
                        .get_or_insert_with(|| crate::tree::node_path(&self.doc, nid))
                        .clone();
                    let hay = format!("{}\n{}", title_lc, p.to_lowercase());
                    let Some(score) = fuzzy_score(&q, &title_lc, &hay) else { continue };
                    matches.push(SwitcherHit {
                        node: nid,
                        card: Some(c.id),
                        group: None,
                        id: c.id,
                        label: c.title.clone(),
                        path: p,
                        score: score.saturating_add(CARD_SCORE_BASE),
                    });
                }
            }
        }
        for (&id, n) in &self.doc.nodes {
            let path = crate::tree::node_path(&self.doc, id);
            let title_lc = n.title.to_lowercase();
            let hay = format!("{}\n{}", title_lc, path.to_lowercase());
            // Below every real score, so an exact id is always the first row —
            // and never listed twice when the title matches the digits as well.
            let score = if Some(id) == by_id {
                i32::MIN
            } else {
                match fuzzy_score(&q, &title_lc, &hay) {
                    Some(s) => s,
                    None => continue,
                }
            };
            matches.push(SwitcherHit { node: id, card: None, group: None, id, label: n.title.clone(), path, score });
        }
        matches.sort_by(|a, b| {
            a.score.cmp(&b.score).then(a.path.len().cmp(&b.path.len())).then(a.label.cmp(&b.label))
        });
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
        let mut jump: Option<(NodeId, Option<CardId>, Option<crate::model::GroupId>)> = None;
        if enter {
            if let Some(m) = matches.get(self.switcher_index) {
                jump = Some((m.node, m.card, m.group));
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
                        .hint_text("Jump to a node or card…  title, path, or an id")
                        .desired_width(f32::INFINITY),
                );
                resp.request_focus();
                ui.separator();
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, false]).show(ui, |ui| {
                    for (i, m) in matches.iter().enumerate() {
                        let sel = i == idx;
                        let shown =
                            if m.label.trim().is_empty() { "(untitled)".to_string() } else { m.label.clone() };
                        let r = ui.add(egui::SelectableLabel::new(sel, egui::RichText::new(shown).strong()));
                        // The id is shown, not just accepted. Until now it lived
                        // only behind right-click → Copy, so an operator handed a
                        // number had no way to see one anywhere in the app and no
                        // way to check they'd landed on the right thing.
                        ui.horizontal(|ui| {
                            ui.small(
                                egui::RichText::new(if m.card.is_some() {
                                    format!("card #{}", m.id)
                                } else if m.group.is_some() {
                                    format!("group #g{}", m.id)
                                } else {
                                    format!("node #{}", m.id)
                                })
                                .weak(),
                            );
                            ui.small(egui::RichText::new(&m.path).weak());
                        });
                        if r.clicked() {
                            jump = Some((m.node, m.card, m.group));
                        }
                        if sel {
                            r.scroll_to_me(Some(egui::Align::Center));
                        }
                    }
                    if matches.is_empty() && !self.switcher_query.is_empty() {
                        ui.weak("No matching nodes or cards.");
                    }
                });
            });

        // A card hit reveals the card itself — recenter and flash — rather than
        // just opening the basket and leaving you to find it. `reveal_hit` is the
        // same path the Agenda, Kanban, Find, Tags and Backlinks rows already use.
        if let Some((node, card, group)) = jump {
            self.switcher_open = false;
            match group {
                Some(g) => self.jump_to_group(ctx, node, g),
                None => self.reveal_hit(ctx, node, card),
            }
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

    /// Right-side panel: every card that **asserts state**, worst first.
    ///
    /// This is the human half of `verify::`. The agent half is
    /// `GET /api/claims` and the `stale_claims` count on `/api/instance`; this
    /// panel exists so the person keeping the workspace can see the same thing
    /// without being told by an agent that just wasted a session on it.
    fn claims_panel(&mut self, ctx: &egui::Context) {
        let mut jump: Option<(NodeId, Option<CardId>)> = None;
        let today = api::today_days();
        let mut claims = self.doc.claims();
        let rank = |b: &str| match b {
            "expired" => 0,
            "unparsed" => 1,
            "today" => 2,
            "soon" => 3,
            _ => 4,
        };
        claims.sort_by_key(|c| (rank(api::claim_bucket(c.verify_days, today)), c.verify_days));
        egui::SidePanel::right("claims").resizable(true).default_width(300.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Claims");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("×").clicked() {
                        self.claims_open = false;
                    }
                });
            });
            ui.separator();
            if claims.is_empty() {
                ui.weak(
                    "No claims yet.\n\nA card that states how something IS — a version, a \
                     count, what someone owes you — should say when that should be checked \
                     again:\n\nverify:: 2026-09-01\ncheck:: GET /api/instance\n\nThey are \
                     listed here, worst first, and the count rides on /api/instance so an \
                     agent is warned before it believes the card.",
                );
                return;
            }
            let stale = claims
                .iter()
                .filter(|c| {
                    matches!(api::claim_bucket(c.verify_days, today), "expired" | "unparsed")
                })
                .count();
            if stale > 0 {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 90, 90),
                    format!("{stale} of {} need re-checking", claims.len()),
                );
            } else {
                ui.weak(format!("{} claim(s), all current", claims.len()));
            }
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for c in &claims {
                    let bucket = api::claim_bucket(c.verify_days, today);
                    let (mark, color) = match bucket {
                        "expired" => ("expired", egui::Color32::from_rgb(220, 90, 90)),
                        "unparsed" => ("unreadable date", egui::Color32::from_rgb(220, 150, 60)),
                        "today" => ("due today", egui::Color32::from_rgb(220, 180, 60)),
                        "soon" => ("this week", egui::Color32::from_rgb(150, 170, 90)),
                        _ => ("ok", ui.visuals().weak_text_color()),
                    };
                    if ui
                        .add(
                            egui::Label::new(egui::RichText::new(&c.title).strong())
                                .sense(egui::Sense::click()),
                        )
                        .clicked()
                    {
                        jump = Some((c.node, Some(c.card)));
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(color, format!("verify:: {} — {mark}", c.verify));
                    });
                    ui.small(&c.node_path);
                    // The command that settles it, when the card said. Shown
                    // rather than hidden behind a hover: the whole point is that
                    // re-checking should be cheaper than doubting.
                    if let Some(check) = &c.check {
                        ui.small(egui::RichText::new(format!("check:: {check}")).italics());
                    }
                    ui.separator();
                }
            });
        });
        if let Some((node, card)) = jump {
            self.reveal_hit(ctx, node, card);
        }
    }

    /// Right-side panel: filter cards across the whole tree by tag / property /
    /// text (all dropdown-driven, no syntax), with click-to-jump results.
    fn find_panel(&mut self, ctx: &egui::Context) {
        let mut jump: Option<(NodeId, Option<CardId>)> = None;
        let mut save_view = false;
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
            ui.horizontal(|ui| {
                if ui.button("Clear filters").clicked() {
                    self.find_tag = None;
                    self.find_key = None;
                    self.find_value.clear();
                    self.find_text.clear();
                }
                // **The on-ramp.** This panel has already built the query; a
                // saved view only adds keeping it. Building one by hand means
                // writing a `view` field over the API, which nobody discovers.
                let any = self.find_tag.is_some()
                    || self.find_key.is_some()
                    || !self.find_text.trim().is_empty();
                if ui
                    .add_enabled(any, egui::Button::new("Save as view card"))
                    .on_hover_text(
                        "Put these filters on a new card in the selected basket. It shows \
                         the cards they match, recomputed every time you look — it stores \
                         the question, never the answer.",
                    )
                    .clicked()
                {
                    save_view = true;
                }
            });
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
        if save_view {
            self.save_find_as_view();
        }
        if let Some((node, card)) = jump {
            self.reveal_hit(ctx, node, card);
        }
    }

    /// A spot in `node` no existing card occupies: below everything, at the left
    /// margin. Deliberately not "the middle of the view" — a card dropped into
    /// the arrangement someone made is a card they have to move.
    fn free_spot(&self, node: NodeId) -> egui::Pos2 {
        let bottom = self
            .doc
            .nodes
            .get(&node)
            .map(|n| n.cards.iter().map(|c| c.pos.y + c.size.y).fold(0.0_f32, f32::max))
            .unwrap_or(0.0);
        egui::pos2(40.0, bottom + 40.0)
    }

    /// Turn the Find panel's current filters into a saved-view card.
    ///
    /// Lands in the **selected basket**, so it appears where the person was
    /// already looking. Columns default to the property being filtered on: a view
    /// of "cards where `status:: blocked`" that does not show `status` is a list
    /// you then have to open one by one.
    fn save_find_as_view(&mut self) {
        use crate::model::{CardKind, ViewFilter, ViewOp, ViewSpec};
        let Some(node) = self.selected else {
            self.status = "Select a basket to put the view card in".to_string();
            return;
        };
        let mut filters = Vec::new();
        let mut columns: Vec<String> = Vec::new();
        if let Some(t) = self.find_tag.clone() {
            filters.push(ViewFilter { key: "tag".into(), op: ViewOp::Eq, value: t });
        }
        if let Some(k) = self.find_key.clone() {
            let v = self.find_value.trim().to_string();
            // A key with no value means "has this property at all", which is what
            // the panel means by picking a key and leaving the value blank.
            let op = if v.is_empty() { ViewOp::Exists } else { ViewOp::Eq };
            filters.push(ViewFilter { key: k.clone(), op, value: v });
            columns.push(k);
        }
        let txt = self.find_text.trim().to_string();
        if !txt.is_empty() {
            filters.push(ViewFilter { key: "text".into(), op: ViewOp::Contains, value: txt });
        }
        if filters.is_empty() {
            self.status = "Nothing to save — pick a tag, a property, or some text".to_string();
            return;
        }
        // `due` earns a column on every view: it is what this document is
        // organised around and what people sort by.
        if !columns.iter().any(|c| c == "due") {
            columns.push("due".into());
        }
        let title = describe_find(
            self.find_tag.as_deref(),
            self.find_key.as_deref(),
            &self.find_value,
            &self.find_text,
        );
        let pos = self.free_spot(node);
        let Some(cid) = self.doc.add_card(node, pos, CardKind::Text) else {
            self.status = "Could not create the view card".to_string();
            return;
        };
        if let Some(c) = self.doc.card_mut(node, cid) {
            c.title = title.clone();
            c.size = egui::vec2(420.0, 260.0);
            c.editing = false;
            c.view = Some(ViewSpec { filters, columns, ..Default::default() });
        }
        self.mark_dirty();
        self.focus_card = Some(cid);
        self.status = format!("Saved view \"{title}\"");
    }

    /// Right-side panel: every open task (a card with a `due::` date) across the
    /// tree, grouped by when it's due. Click a task to jump to its basket.
    /// Every open task across the tree, grouped by when it is due.
    ///
    /// Rendered into whatever container the placement asks for — the right-hand
    /// panel, or a window of its own — so the two cannot drift apart in what
    /// they show.
    fn agenda_panel(&mut self, ctx: &egui::Context) {
        if self.agenda_placement == Placement::Window {
            let vid = egui::ViewportId::from_hash_of("agenda-window");
            let builder = egui::ViewportBuilder::default()
                .with_title("Trellis — Agenda")
                .with_inner_size([420.0, 700.0]);
            let mut closed = false;
            let stick = self.stick_windows;
            let delta = self.main_move_delta;
            ctx.show_viewport_immediate(vid, builder, |vctx, _| {
                egui::CentralPanel::default().show(vctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.agenda_body(ui));
                });
                follow_main_window(vctx, stick, delta, &mut self.agenda_stick);
                // Closing the OS window closes the panel; it must not linger as an
                // invisible open panel that the View menu then refuses to reopen.
                if vctx.input(|i| i.viewport().close_requested()) {
                    closed = true;
                }
            });
            if closed {
                self.agenda_open = false;
            }
            return;
        }
        // **A capped width.** A panel sized by its content is a panel that one bad
        // value can stretch across the whole window — which is what a `due::` that
        // had swallowed a sentence did. The parser no longer produces one; this
        // makes it unable to matter again.
        egui::SidePanel::right("agenda")
            .resizable(true)
            .default_width(320.0)
            .max_width(560.0)
            .show(ctx, |ui| self.agenda_body(ui));
    }

    /// The Agenda itself, container-agnostic.
    fn agenda_body(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let today = crate::api::today_days();
        let mut tasks = self.doc.tasks();
        let mut jump: Option<(NodeId, CardId)> = None;
        // (node, card, new due) — `None` clears the date. Applied after the
        // panel closes, since the panel borrows the document to draw itself.
        let mut reschedule: Option<(NodeId, CardId, Option<crate::model::ItemId>, Option<String>)> = None;
        ui.horizontal(|ui| {
            ui.heading("Agenda");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("×").clicked() {
                    self.agenda_open = false;
                }
                if ui
                    .button(self.agenda_placement.label())
                    .on_hover_text(
                        "Move the Agenda between the side panel and a window of its own — which \
                         can go on another monitor. Remembered next launch.",
                    )
                    .clicked()
                {
                    self.agenda_placement = self.agenda_placement.toggled();
                }
                // Only meaningful once it is a window of its own.
                if self.agenda_placement == Placement::Window {
                    stick_toggle(ui, &mut self.stick_windows);
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
                    .filter(|t| crate::api::task_bucket_spanning(t, today) == key)
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
                    // A task's text can be long — a checklist line often
                    // carries its context with it, and a 300-character row
                    // turns the agenda into a wall. Show the first line's
                    // worth and keep the whole thing on hover; the card
                    // itself is where the full text belongs.
                    let shown = elide(&t.title, 80);
                    let title = if t.done {
                        egui::RichText::new(&shown).strikethrough().weak()
                    } else {
                        egui::RichText::new(&shown)
                    };
                    let pcolor = project_color(&self.doc, t.root);
                    let row = ui.horizontal(|ui| {
                        // A dot in the project's colour, so a glance down the
                        // list groups by project without reading a word.
                        let (rect, _) = ui
                            .allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, pcolor);
                        // Elided like the title. A date is ten characters;
                        // anything longer is a value that has gone wrong, and the
                        // panel must not be the thing that reports it — by
                        // growing to the width of the window.
                        ui.add(
                            egui::Label::new(format!("{}  ", elide(&t.due, 12)))
                                .sense(egui::Sense::click()),
                        )
                    });
                    let mut label =
                        ui.add(egui::Label::new(title).sense(egui::Sense::click()));
                    if shown.len() < t.title.len() {
                        label = label.on_hover_text(&t.title);
                    }
                    if label.clicked() || row.inner.clicked() {
                        jump = Some((t.node, t.card));
                    }
                    // Move a task without leaving the list. Editing the
                    // `due::` line by hand was the only way, and that
                    // friction is exactly what makes people copy a task card
                    // to the next day instead — which silently creates a
                    // second task.
                    label.context_menu(|ui| {
                        ui.label(egui::RichText::new(&t.title).strong());
                        ui.small(egui::RichText::new(format!("due {}", t.due)).weak());
                        ui.separator();
                        for (text, days, months) in [
                            ("Today", 0i64, 0u32),
                            ("Tomorrow", 1, 0),
                            ("In 3 days", 3, 0),
                            ("Next week", 7, 0),
                            ("Next month", 0, 1),
                        ] {
                            let when = crate::api::date_from_today(days, months);
                            if ui.button(format!("{text}  ({when})")).clicked() {
                                reschedule = Some((t.node, t.card, t.item, Some(when)));
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        if ui
                            .button("Clear date")
                            .on_hover_text(
                                // This said the task "moves to No date rather than
                                // leaving the agenda". It does not: the button
                                // clears the property, and `tasks()` skips a card
                                // with no `due` at all, so the row goes. "No date"
                                // is where a due:: that will not PARSE lands.
                                "Removes the due:: line, so this leaves the agenda \
                                 entirely. (A due:: whose value is not a date — \
                                 `due:: soon` — is what sits under \"No date\".)",
                            )
                            .clicked()
                        {
                            reschedule = Some((t.node, t.card, t.item, None));
                            ui.close_menu();
                        }
                        if ui.button("Open the card").clicked() {
                            jump = Some((t.node, t.card));
                            ui.close_menu();
                        }
                    });
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
        if let Some((node, card, item, due)) = reschedule {
            // A checklist line carries its own date, so the edit has to land on
            // the line — writing to the card would move every task in the list.
            let changed = match (item, &due) {
                (Some(i), Some(d)) => self.doc.set_item_property(node, card, i, "due", d),
                (Some(i), None) => self.doc.clear_item_property(node, card, i, "due"),
                (None, Some(d)) => self.doc.set_card_property(node, card, "due", d),
                (None, None) => self.doc.clear_card_property(node, card, "due"),
            };
            if changed {
                // note_card stamps `touched` and marks the document dirty; doing
                // either here as well would be a second, divergent code path.
                self.note_card(node, card, crate::changelog::Op::Updated, "due");
                self.status = match due {
                    Some(d) => format!("Task due {d}"),
                    None => "Task due date cleared".to_string(),
                };
            }
        }
        if let Some((node, card)) = jump {
            self.jump_to_card(&ctx, node, card);
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
    /// The Kanban board, in whichever container the placement asks for.
    ///
    /// A window *inside* the app window cannot be moved to another monitor or
    /// left beside the canvas, which is most of what a board is for — so it can
    /// be detached into a real OS window.
    fn kanban_window(&mut self, ctx: &egui::Context) {
        if self.kanban_placement == Placement::Window {
            let vid = egui::ViewportId::from_hash_of("kanban-window");
            let builder = egui::ViewportBuilder::default()
                .with_title("Trellis — Kanban")
                .with_inner_size([1000.0, 620.0]);
            let mut closed = false;
            let stick = self.stick_windows;
            let delta = self.main_move_delta;
            ctx.show_viewport_immediate(vid, builder, |vctx, _| {
                egui::CentralPanel::default().show(vctx, |ui| self.kanban_body(ui));
                follow_main_window(vctx, stick, delta, &mut self.kanban_stick);
                if vctx.input(|i| i.viewport().close_requested()) {
                    closed = true;
                }
            });
            if closed {
                self.kanban_open = false;
            }
            return;
        }
        let mut open = self.kanban_open;
        egui::Window::new("Kanban board")
            .open(&mut open)
            .default_size([900.0, 560.0])
            .resizable(true)
            .show(ctx, |ui| self.kanban_body(ui));
        self.kanban_open = open;
    }

    /// The board itself, container-agnostic.
    fn kanban_body(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // (the container owns whether the board is open)
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
            ui.horizontal(|ui| {
                ui.small("Cards with a status:: property. Drag a card between columns to change its status.");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(self.kanban_placement.label())
                        .on_hover_text(
                            "Move the board between a window inside Trellis and one of its own — \
                             which can go on another monitor. Remembered next launch.",
                        )
                        .clicked()
                    {
                        self.kanban_placement = self.kanban_placement.toggled();
                    }
                    // Only meaningful once it is a window of its own.
                    if self.kanban_placement == Placement::Window {
                        stick_toggle(ui, &mut self.stick_windows);
                    }
                    ui.separator();
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
        // whether the board is open is the container's business, not the body's
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
            self.jump_to_card(&ctx, node, card);
        }
    }

    /// Navigate a clicked `[[wiki-link]]` (its URL-encoded target) to the node
    /// it names, or report that no such node exists.
    fn follow_wikilink(&mut self, ctx: &egui::Context, encoded: &str) {
        let target = crate::model::decode_link(encoded);
        self.follow_link_target(ctx, &target);
    }

    /// Go where a `[[link]]` points, given the **raw** target.
    ///
    /// Split out because a table cell already has the target in hand — it never
    /// went through a URL — so encoding it just to decode it again would be a
    /// round trip that could only introduce a difference between the two paths.
    fn follow_link_target(&mut self, ctx: &egui::Context, target: &str) {
        let target = target.to_string();
        // From the basket the link was clicked in, so a bare [[Archive]] means
        // this project's Archive rather than whichever one hashed first.
        let resolved = match self.selected {
            Some(here) => self.doc.resolve_link_target_from(&target, here),
            None => self.doc.resolve_link_target(&target),
        };
        match resolved {
            Some(crate::model::LinkTarget::Node(id)) => self.jump_to_node(id),
            // A card link lands *on the card* — recentre and flash — not merely
            // in its basket. In a journal-shaped document the basket is a day
            // holding twenty other cards, so "opened the right basket" is not
            // an answer.
            Some(crate::model::LinkTarget::Card { node, card }) => {
                self.jump_to_card(ctx, node, card)
            }
            // A group link lands on the group box, for the same reason a card
            // link lands on the card: the basket is not the thing that was named.
            Some(crate::model::LinkTarget::Group { node, group }) => {
                self.jump_to_group(ctx, node, group)
            }
            None => {
                let t = target.trim_start_matches('#');
                self.status = if t.starts_with(['g', 'G']) && t[1..].parse::<u64>().is_ok() {
                    format!("No group {target} in this document")
                } else if target.starts_with('#') {
                    format!("No card {target} in this document")
                } else {
                    format!("No node named \"{target}\" to link to")
                }
            }
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

    /// Move a card's selected text into a card of its own, leaving `![[#id]]`
    /// where it was.
    ///
    /// **This is the answer to "one task is one card, never copied", applied to
    /// prose.** Before `![[#id]]` (v0.125.0) the only way to split a card was to
    /// copy text into a new one and leave the original behind, which is two
    /// sources of truth from the moment you finish. Extract *moves* the text and
    /// leaves a **view** of it, so there is exactly one copy and the card reads
    /// exactly as it did.
    ///
    /// The new card is placed to the right of the source at the canvas grid step,
    /// so it lands somewhere deliberate rather than on top of whatever is there.
    fn extract_selection(
        &mut self,
        ctx: &egui::Context,
        node: NodeId,
        cid: CardId,
        from: usize,
        to: usize,
    ) {
        let Some(src) = self.doc.card(node, cid) else { return };
        let chars: Vec<char> = src.body.chars().collect();
        let (from, to) = (from.min(chars.len()), to.min(chars.len()));
        if from >= to {
            return;
        }
        let Some((taken, rewritten_head, rewritten_tail)) = split_for_extract(&chars, from, to)
        else {
            self.status = "Nothing to extract \u{2014} the selection is blank".into();
            return;
        };
        if taken.trim().is_empty() {
            self.status = "Nothing to extract \u{2014} the selection is blank".into();
            return;
        }
        let (pos, size) = (src.pos, src.size);
        let at = egui::pos2(pos.x + size.x + canvas::GRID_STEP, pos.y);
        let Some(new_id) = self.doc.add_card(node, at, crate::model::CardKind::Text) else {
            return;
        };
        // A title from the first non-blank line, so the new card is findable in
        // the Ctrl+O palette rather than being one of many "(untitled card)".
        let title = taken
            .lines()
            .map(|l| l.trim_start_matches(['#', '-', '*', '>', ' ']).trim())
            .find(|l| !l.is_empty())
            .map(|l| l.chars().take(60).collect::<String>())
            .unwrap_or_default();
        if let Some(c) = self.doc.card_mut(node, new_id) {
            c.title = title;
            c.body = taken.trim().to_string();
            c.size = size;
            // `add_card` opens a new card for typing, which is right when it is
            // blank and wrong here: the text is already written, and edit mode
            // would show it — and the embed left behind — as raw Markdown.
            c.editing = false;
        }
        // Fitted through the same path as Fit to content, so an extracted card
        // opens readable rather than inheriting a size chosen for other text.
        if let Some(sz) = self.doc.card(node, new_id).and_then(|c| fit_card_size(ctx, c)) {
            if let Some(c) = self.doc.card_mut(node, new_id) {
                c.size = sz.max(MIN_CARD);
            }
        }
        if let Some(c) = self.doc.card_mut(node, cid) {
            c.body = format!("{rewritten_head}![[#{new_id}]]{rewritten_tail}");
        }
        self.mark_dirty();
        self.focus_card = Some(new_id);
        self.status = format!("Extracted to card #{new_id}, embedded here");
    }

    /// Jump to a uniformly random card anywhere in the document.
    ///
    /// **Uniform over cards, not over baskets.** Picking a basket and then a card
    /// inside it would make a card in a two-card basket far likelier than one in
    /// a basket of fifty, which for rediscovery is exactly backwards: the crowded
    /// baskets are where the forgotten things are. So the index is drawn across
    /// the flattened list.
    ///
    /// Randomness comes from the OS CSPRNG that already mints the API key rather
    /// than a new dependency — overkill for this, but it is one call and there is
    /// no weaker source already in the tree.
    fn go_to_random_card(&mut self, ctx: &egui::Context) {
        let all: Vec<(NodeId, CardId)> = self
            .doc
            .nodes
            .iter()
            .flat_map(|(nid, n)| n.cards.iter().map(move |c| (*nid, c.id)))
            .collect();
        if all.is_empty() {
            self.status = "No cards to pick from".into();
            return;
        }
        let mut b = [0u8; 8];
        if getrandom::fill(&mut b).is_err() {
            self.status = "Could not draw a random number".into();
            return;
        }
        // Modulo bias over a 64-bit draw is far below anything observable here.
        let (node, card) = all[(u64::from_le_bytes(b) % all.len() as u64) as usize];
        self.jump_to_card(ctx, node, card);
        let title = self
            .doc
            .card(node, card)
            .map(|c| c.title.clone())
            .unwrap_or_default();
        self.status = if title.is_empty() {
            format!("Random card #{card} of {}", all.len())
        } else {
            format!("Random: {title} (#{card} of {})", all.len())
        };
    }

    /// Take a whole basket onto the desktop — Desktop mode proper.
    ///
    /// **The arrangement is preserved.** Each card's window opens exactly where
    /// the card appears on screen right now, so the layout you built in the
    /// basket is the layout you get on the desktop; move or minimise the Trellis
    /// window and the cards are simply there. That is what makes it Unity rather
    /// than "a pile of windows".
    ///
    /// Cards are read from `card_rects`, which the canvas fills each frame with
    /// the on-screen rectangle it actually drew — so zoom, pan and depth are all
    /// already accounted for, rather than recomputed here and allowed to drift.
    #[cfg(target_os = "linux")]
    fn send_basket_to_desktop(&mut self, ctx: &egui::Context, node: NodeId) {
        // egui reports positions inside the window; a window manager wants them
        // on the screen. Without the window's own origin every card would open
        // in the top-left corner of the display.
        let cards: Vec<(CardId, egui::Pos2, egui::Vec2)> = match self.doc.nodes.get(&node) {
            Some(n) => n.cards.iter().map(|c| (c.id, c.pos, c.size)).collect(),
            None => return,
        };
        if cards.is_empty() {
            self.status = "Nothing in this basket to send to the desktop".into();
            return;
        }

        // The basket's own bounding box, in world coordinates. Positions come
        // from the DOCUMENT, not from the drawn screen rects: a card scrolled
        // out of the viewport has a screen rect off the display, and placing a
        // window there just makes the window manager clamp it to the edge —
        // measured, and it flattened the arrangement into a row along the
        // bottom of the screen.
        let mut min = egui::pos2(f32::MAX, f32::MAX);
        let mut max = egui::pos2(f32::MIN, f32::MIN);
        let mut biggest = egui::vec2(0.0, 0.0);
        for (_, pos, size) in &cards {
            min.x = min.x.min(pos.x);
            min.y = min.y.min(pos.y);
            max.x = max.x.max(pos.x + size.x);
            max.y = max.y.max(pos.y + size.y);
            biggest = biggest.max(*size);
        }

        // Fit that box to the screen, so the whole basket is reachable. Windows
        // keep their real size — scaling a card's *window* would shrink its text
        // — so only the spacing between them is compressed.
        let screen = ctx
            .input(|i| i.viewport().monitor_size)
            .unwrap_or(egui::vec2(1920.0, 1080.0));
        const MARGIN: f32 = 40.0;
        let room = (screen - egui::vec2(MARGIN * 2.0, MARGIN * 2.0) - biggest).max(egui::vec2(1.0, 1.0));
        let span = (max - min).max(egui::vec2(1.0, 1.0));
        let scale = (room.x / span.x).min(room.y / span.y).min(1.0).max(0.05);

        let mut sent = 0usize;
        for (cid, pos, size) in &cards {
            let x = MARGIN + (pos.x - min.x) * scale;
            let y = MARGIN + (pos.y - min.y) * scale;
            // Clamp so no window is placed where the WM would have to move it —
            // a moved window is one whose position we did not choose.
            let x = x.clamp(0.0, (screen.x - size.x).max(0.0));
            let y = y.clamp(0.0, (screen.y - size.y).max(0.0));
            self.desktop_cards.insert(*cid, [x, y]);
            sent += 1;
        }
        self.desktop_mode = Some(node);
        self.status = format!("{sent} card(s) on the desktop — click Desktop again to bring them back");
    }

    /// Bring a whole basket back off the desktop.
    #[cfg(target_os = "linux")]
    fn recall_basket_from_desktop(&mut self, node: NodeId) {
        let ids: Vec<CardId> = match self.doc.nodes.get(&node) {
            Some(n) => n.cards.iter().map(|c| c.id).collect(),
            None => Vec::new(),
        };
        for cid in ids {
            self.desktop_cards.remove(&cid);
            self.desktop_live.remove(&cid);
        }
        if self.desktop_mode == Some(node) {
            self.desktop_mode = None;
        }
        self.status = "Cards recalled from the desktop".into();
    }

    /// Desktop mode: draw every card that has been sent out as its own
    /// borderless OS window, so it interleaves with other applications in the
    /// window manager's z-order.
    ///
    /// **One real window per card, not one transparent overlay.** An overlay is
    /// a single window and therefore sits entirely above or entirely below every
    /// other application — a card could never be behind a browser and in front of
    /// a terminal, which is the whole point. Only genuine top-level windows take
    /// part in the WM's stacking.
    ///
    /// **Not always-on-top.** A card that can never go behind anything is a HUD,
    /// not part of the desktop.
    ///
    /// Linux/X11 only: an application may position its own windows there. Wayland
    /// has no protocol for it at all, and macOS/Windows need their own pass.
    #[cfg(target_os = "linux")]
    fn desktop_windows(&mut self, ctx: &egui::Context) {
        if self.desktop_cards.is_empty() {
            return;
        }
        // Resolve each card once, up front: a card can be deleted or moved while
        // its window is open, and a stale id must close the window rather than
        // panic or draw nothing forever.
        let open: Vec<(CardId, [f32; 2], NodeId)> = self
            .desktop_cards
            .iter()
            .filter_map(|(&cid, &pos)| self.doc.locate_card(cid).map(|n| (cid, pos, n)))
            .collect();
        let gone: Vec<CardId> = self
            .desktop_cards
            .keys()
            .copied()
            .filter(|c| self.doc.locate_card(*c).is_none())
            .collect();
        for c in gone {
            self.desktop_cards.remove(&c);
        }

        let mut recall: Vec<CardId> = Vec::new();
        let mut moved: Vec<(CardId, [f32; 2])> = Vec::new();
        let mut resized: Vec<(CardId, egui::Vec2)> = Vec::new();
        let mut actions: Vec<canvas::CanvasAction> = Vec::new();
        for (cid, pos, node) in open {
            let Some(card) = self.doc.card(node, cid).cloned() else { continue };
            let node_path = crate::tree::node_path(&self.doc, node);
            let size = card.size;
            let vid = egui::ViewportId::from_hash_of(("desktop-card", cid));
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("Trellis card #{cid}"))
                .with_decorations(false)
                .with_transparent(true)
                .with_taskbar(false)
                .with_position(pos)
                .with_inner_size([size.x, size.y]);

            let template_names: Vec<String> =
                self.templates.iter().map(|t| t.card.title.clone()).collect();
            let card_plugins = self.plugins_for(crate::plugins::Trigger::CardMenu);
            let masters = self.master_states(node);
            // The REAL set here, so the right-click menu inside the window offers
            // *Recall* rather than *Send to desktop* on a card that is plainly
            // already out. The placeholder is suppressed by `as_window` instead —
            // an earlier fix passed an empty set for both and produced exactly
            // that wrong menu.
            let out_now: std::collections::HashSet<CardId> =
                self.desktop_cards.keys().copied().collect();
            let theme = self.theme;
            let inline_epoch = self.inline_epoch;
            let minimap = self.minimap_enabled;
            let md = &mut self.md_cache;
            let tex = &mut self.tex_cache;
            let rects = &mut self.card_rects;
            let sent = &mut self.inline_sent;
            // Bound out here with the other field borrows: a desktop card is a
            // real window drawn from the same document, and an `![[#id]]` in it
            // must resolve against the whole document like anywhere else.
            let doc = &self.doc;

            ctx.show_viewport_immediate(vid, builder, |vctx, _| {
                let mut env = Env {
                    doc,
                    node,
                    md, tex, card_rects: rects,
                    templates: &template_names,
                    masters: &masters,
                    card_plugins: &card_plugins,
                    inline_sent: sent,
                    inline_epoch,
                    focus_card: None,
                    highlight_card: None,
                    highlight_until: 0.0,
                    focus_group: None,
                    highlight_group: None,
                    minimap,
                    style: match theme {
                        Theme::StickyNotes => canvas::CardStyle::Sticky,
                        Theme::Futuristic => canvas::CardStyle::Futuristic,
                        Theme::Blueprint => canvas::CardStyle::Blueprint,
                        Theme::Silkscreen => canvas::CardStyle::Silkscreen,
                        Theme::Phosphor => canvas::CardStyle::Phosphor,
                        _ => canvas::CardStyle::Normal,
                    },
                    glow: matches!(theme, Theme::Futuristic | Theme::SynthWave | Theme::Phosphor),
                    on_desktop: &out_now,
                    as_window: true,
                };
                // Transparent frame: the window is the card, so its rounded
                // corners must show the desktop rather than a grey rectangle.
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(egui::Color32::TRANSPARENT))
                    .show(vctx, |ui| {
                        // `StartDrag` hands the whole move to the window manager.
                        // Chasing the pointer delta ourselves cannot converge —
                        // the delta is measured inside a window that is itself
                        // moving, which is the v0.99.1 bug.
                        if canvas::desktop_card_ui(ui, &card, &node_path, &mut env, &mut actions) {
                            vctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                        }
                        // Recall button, top-right, where a close box belongs.
                        let bx = egui::Rect::from_min_size(
                            egui::pos2(ui.max_rect().max.x - 24.0, ui.max_rect().min.y + 2.0),
                            egui::vec2(20.0, 20.0),
                        );
                        if ui
                            .put(bx, egui::Button::new("⤓").small().frame(false))
                            .on_hover_text("Recall to its basket")
                            .clicked()
                        {
                            recall.push(cid);
                        }
                    });

                // Note where the window manager actually put it, for persistence
                // only. **Never** fed back into the builder: that is what made
                // the window fight the WM and flash.
                if let Some(r) = vctx.input(|i| i.viewport().outer_rect) {
                    moved.push((cid, [r.min.x, r.min.y]));
                }
                // A desktop window IS the card, so resizing the window resizes
                // the card — otherwise the new size is lost the moment it is
                // recalled, which reads as the resize having been ignored.
                //
                // The epsilon matters: writing back a value that differs by a
                // fraction of a pixel would change `card.size`, which changes the
                // builder, which commands the window — the flashing loop again,
                // just with size instead of position.
                if let Some(r) = vctx.input(|i| i.viewport().inner_rect) {
                    let now = r.size();
                    if (now.x - size.x).abs() > 1.0 || (now.y - size.y).abs() > 1.0 {
                        resized.push((cid, now));
                    }
                }
                // Closing the OS window recalls the card, rather than leaving an
                // invisible entry that the menu then refuses to reopen.
                if vctx.input(|i| i.viewport().close_requested()) {
                    recall.push(cid);
                }
            });
        }
        for (cid, p) in moved {
            self.desktop_live.insert(cid, p);
        }
        for (cid, sz) in resized {
            if let Some(node) = self.doc.locate_card(cid) {
                if let Some(c) = self.doc.card_mut(node, cid) {
                    c.size = sz;
                }
                self.note(
                    crate::changelog::Change::new(
                        crate::changelog::Actor::Ui,
                        crate::changelog::Entity::Card,
                        crate::changelog::Op::Updated,
                        cid,
                    )
                    .in_node(node)
                    .field("size"),
                );
            }
        }
        for cid in recall {
            self.desktop_cards.remove(&cid);
            self.desktop_live.remove(&cid);
        }
        if !actions.is_empty() {
            // Card edits made in a desktop window go through the same path as
            // edits made on the canvas — one code path, so they cannot drift.
            let sel = self.selected;
            if let Some(node) = sel {
                self.apply_canvas(ctx, node, actions, false);
            }
        }
    }

    /// Whether a move request's *destination* is inside a scoped token's
    /// subtree. Requests that cannot relocate anything answer `true`.
    fn move_dest_within(&self, req: &api::ApiRequest, root: NodeId) -> bool {
        match api::move_destination(req) {
            None => true,
            Some(api::MoveDest::Basket(n)) => self.node_is_within(n, root),
            Some(api::MoveDest::Parent(Some(p))) => self.node_is_within(p, root),
            // The top level is outside every subtree by definition.
            Some(api::MoveDest::Parent(None)) => false,
            Some(api::MoveDest::Sibling(s)) => self
                .doc
                .nodes
                .get(&s)
                .and_then(|n| n.parent)
                .is_some_and(|p| self.node_is_within(p, root)),
        }
    }

    /// Like [`jump_to_card`], but reveals a whole group: the canvas centres on
    /// the members' bounding box and flashes the container.
    fn jump_to_group(
        &mut self,
        ctx: &egui::Context,
        node: NodeId,
        group: crate::model::GroupId,
    ) {
        self.jump_to_node(node);
        self.focus_group = Some(group);
        self.highlight_group = Some(group);
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
    /// Clear to fully transparent so a Desktop-mode card window shows the desktop
    /// through its rounded corners. The main window is unaffected in appearance:
    /// its side, top and central panels paint opaque fills over this.
    fn clear_color(&self, _v: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.last_ctx = Some(ctx.clone());
        // Capture the window handles so file/message dialogs can be parented
        // to the app window instead of opening behind it.
        if let (Ok(w), Ok(d)) = (frame.window_handle(), frame.display_handle()) {
            self.dialog_parent = Some(DialogParent { window: w.as_raw(), display: d.as_raw() });
        }

        // Keep the window title on the open document (New/Open/Save As change it).
        self.sync_window_title(ctx);

        // How far the main window moved since the last frame, so a stuck
        // detached panel can be nudged by the same amount. Read before anything
        // draws, because the viewport closures that use it run during the frame.
        let main_pos = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min));
        self.main_move_delta = match (self.last_main_pos, main_pos) {
            (Some(prev), Some(now)) => now - prev,
            _ => egui::Vec2::ZERO,
        };
        self.last_main_pos = main_pos;

        // Apply any API requests from the server thread first.
        self.pump_api();
        // Apply any finished background OCR results.
        self.pump_ocr();
        // Turn finished region-snips into image cards.
        self.pump_plugins();
        self.pump_plugin_triggers();
        self.pump_sources(false);
        self.pump_snip();

        self.pump_notifications(ctx);

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
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::T)) {
            // Silent when daily notes are off — there is no journal to open, and
            // a hotkey must not be the thing that creates one.
            self.go_to_today();
        }
        if cmd && ctx.input(|i| i.key_pressed(egui::Key::N)) {
            self.new_document();
        }
        // Escape clears a card selection — the way out of a marquee, asked for
        // by name. Only when there is one to clear, and never while a text field
        // has the keyboard, where Escape means "stop editing this".
        if !self.card_sel.is_empty()
            && !ctx.wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.card_sel.clear();
            self.status = "Selection cleared".to_string();
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
        if self.claims_open {
            self.claims_panel(ctx);
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
                    self.tree_sort,
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
                    let card_plugins = self.plugins_for(crate::plugins::Trigger::CardMenu);
                    let masters = self.master_states(sel);
                    let desktop_ids: std::collections::HashSet<CardId> =
                        self.desktop_cards.keys().copied().collect();
                    let mut env = Env {
                        doc: &self.doc,
                        node: sel,
                        md: &mut self.md_cache,
                        tex: &mut self.tex_cache,
                        card_rects: &mut self.card_rects,
                        templates: &template_names,
                        masters: &masters,
                        card_plugins: &card_plugins,
                        inline_sent: &mut self.inline_sent,
                        inline_epoch: self.inline_epoch,
                        focus_card: self.focus_card,
                        highlight_card: self.highlight_card,
                        highlight_until: self.highlight_until,
                        focus_group: self.focus_group,
                        highlight_group: self.highlight_group,
                        on_desktop: &desktop_ids,
                        as_window: false,
                        minimap: self.minimap_enabled,
                        style: match self.theme {
                            Theme::StickyNotes => canvas::CardStyle::Sticky,
                            Theme::Futuristic => canvas::CardStyle::Futuristic,
                            Theme::Blueprint => canvas::CardStyle::Blueprint,
                            Theme::Silkscreen => canvas::CardStyle::Silkscreen,
                            Theme::Phosphor => canvas::CardStyle::Phosphor,
                            _ => canvas::CardStyle::Normal,
                        },
                        glow: matches!(self.theme, Theme::Futuristic | Theme::SynthWave | Theme::Phosphor),
                    };
                    let can_paste = self.card_clipboard.is_some();
                    let node_path = crate::tree::node_path(&self.doc, sel);
                    // Time mode: if this basket is a journal day, gather the cards
                    // whose span covers it. Cloned because the canvas borrows the
                    // document immutably for the node it is drawing, and these
                    // live in other baskets — one clone per projected card per
                    // frame, bounded by what is live on a single day.
                    let projected: Vec<(NodeId, String, crate::model::Card)> = if self.time_mode {
                        self.doc
                            .nodes
                            .get(&sel)
                            .and_then(|n| crate::model::parse_daily_title(&n.title))
                            // Through the same parser `due::` goes through, so a
                            // day node and a due date cannot disagree about what
                            // a calendar day is — the bug `today_days` exists to
                            // prevent, one level up.
                            .and_then(|(y, m, d)| {
                                crate::model::parse_ymd(&format!("{y:04}-{m:02}-{d:02}"))
                            })
                            .map(|day| {
                                self.doc
                                    .cards_live_on(day, sel)
                                    .into_iter()
                                    .filter_map(|(home, cid)| {
                                        let c = self.doc.card(home, cid)?.clone();
                                        Some((home, crate::tree::node_path(&self.doc, home), c))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let mut eye = self.eyes.get(&sel).copied().unwrap_or_default();
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
                        self.grid_mode,
                        self.desktop_mode == Some(sel),
                        self.depth_mode,
                        &mut eye,
                        self.time_mode,
                        &projected,
                        &mut env,
                        &self.card_sel,
                    );
                    // The recenter is one-shot: the canvas consumed it this frame,
                    // so the user can pan freely afterward.
                    self.focus_card = None;
                    self.focus_group = None;
                    // Never let a temporary export reframe overwrite the real view.
                    if framing_card.is_none() && basket_target.is_none() {
                        self.views.insert(sel, view);
                        self.eyes.insert(sel, eye);
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

        // Follow [[wiki-links]] (rendered as the `trellis:` URL scheme) by
        // navigating instead of letting eframe open a browser.
        //
        // This must be the *last* thing in `update()`: `open_url` is set while a
        // widget is rendered and cleared by the backend at the end of the frame,
        // so anything drawn after this check gets its click read a frame too
        // late — by which point eframe has already opened a browser. Links in a
        // card were doing exactly that, because the canvas renders below.
        let clicked = ctx.output(|o| o.open_url.as_ref().map(|u| u.url.clone()));
        if let Some(url) = clicked {
            if let Some(t) = url.strip_prefix("trellis:") {
                let target = t.to_string();
                ctx.output_mut(|o| o.open_url = None);
                self.follow_wikilink(ctx, &target);
            }
        }

        // Desktop-mode windows, drawn after the canvas so a card sent out this
        // frame already has a screen rect to open near.
        #[cfg(target_os = "linux")]
        self.desktop_windows(ctx);

        // A `trellis://` link that arrived on the API thread: reveal it here,
        // where there is a frame (the highlight fade is measured in frame time)
        // and where the viewport can be asked for focus.
        if let Some((node, card)) = self.pending_reveal.take() {
            self.jump_to_card(ctx, node, card);
        }
        if let Some((node, group)) = self.pending_reveal_group.take() {
            self.jump_to_group(ctx, node, group);
        }
        if std::mem::take(&mut self.focus_window) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            // **A raise can be refused, and usually is.** Focus-stealing
            // prevention (KWin's is on by default) ignores a raise from an app
            // the user was not just interacting with — which is exactly this
            // case, since the click was in a terminal. The window then jumps to
            // the card silently and the link looks like it did nothing.
            //
            // Asking for *attention* is the sanctioned way to say so: whatever
            // the policy, the taskbar entry lights up. Window managers clear it
            // as soon as the window is focused, so it costs nothing when the
            // raise does go through.
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Critical,
            ));
            ctx.request_repaint();
        }

        // Colour emoji, painted over the glyphs laid out above. Last, and after
        // the wiki-link check, because it reads back what the frame drew —
        // anything added to a paint list after this point is not covered.
        self.emoji.overlay(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Desktop-mode placements: app config, so they live here rather than in
        // the document — screen geometry belongs to this machine.
        let placements: std::collections::HashMap<CardId, [f32; 2]> = self
            .desktop_cards
            .iter()
            .map(|(&c, &p)| (c, self.desktop_live.get(&c).copied().unwrap_or(p)))
            .collect();
        if let Ok(j) = serde_json::to_string(&placements) {
            storage.set_string(DESKTOP_CARDS_KEY, j);
        }
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
        storage.set_string(GRID_MODE_KEY, self.grid_mode.to_string());
        storage.set_string(DEPTH_MODE_KEY, self.depth_mode.to_string());
        storage.set_string(TIME_MODE_KEY, self.time_mode.to_string());
        storage.set_string(AGENDA_OPEN_KEY, self.agenda_open.to_string());
        storage.set_string(AGENDA_DONE_KEY, self.agenda_show_done.to_string());
        storage.set_string(AGENDA_PLACE_KEY, self.agenda_placement.as_str().to_string());
        storage.set_string(KANBAN_OPEN_KEY, self.kanban_open.to_string());
        storage.set_string(KANBAN_DONE_KEY, self.kanban_show_done.to_string());
        storage.set_string(KANBAN_PLACE_KEY, self.kanban_placement.as_str().to_string());
        storage.set_string(STICK_WINDOWS_KEY, self.stick_windows.to_string());
        storage.set_string(TAGS_OPEN_KEY, self.tags_open.to_string());
        storage.set_string(CLAIMS_OPEN_KEY, self.claims_open.to_string());
        storage.set_string(FIND_OPEN_KEY, self.find_open.to_string());
        storage.set_string(BACKLINKS_OPEN_KEY, self.backlinks_open.to_string());
        storage.set_string(
            URL_SCHEME_REGISTERED_KEY,
            self.url_scheme_registered.clone().unwrap_or_default(),
        );
        storage.set_string(MINIMAP_KEY, self.minimap_enabled.to_string());
        storage.set_string(NOTIFY_DIGEST_KEY, self.notify_digest.to_string());
        storage.set_string(NOTIFY_AGENT_KEY, self.notify_agent.to_string());
        storage.set_string(TREE_SORT_KEY, self.tree_sort.key().to_string());
        // Absent rather than "0" when off: a stored root that points at nothing
        // would be indistinguishable from a deleted journal.
        storage.set_string(
            DAILY_ROOT_KEY,
            self.daily_root.map(|id| id.to_string()).unwrap_or_default(),
        );
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
/// Is there already a link handler pointing at a binary that still exists?
///
/// The question is not "did *we* register" — several instances share one
/// desktop-wide handler, and any Trellis binary can forward a link, because a
/// link names a **port** and the handler is a one-shot forwarder that exits. It
/// only needs replacing when it points at something that has gone.
fn scheme_handler_healthy() -> bool {
    let Ok(home) = std::env::var("HOME") else { return false };
    let file = std::path::Path::new(&home)
        .join(".local/share/applications")
        .join(format!("{}-url.desktop", crate::URL_SCHEME));
    let Ok(text) = std::fs::read_to_string(&file) else { return false };
    text.lines()
        .find_map(|l| l.strip_prefix("Exec="))
        .map(|exec| {
            // `Exec=/path/to/trellis %u` — the path is everything before ` %`.
            let path = exec.split(" %").next().unwrap_or("").trim();
            !path.is_empty() && std::path::Path::new(path).exists()
        })
        .unwrap_or(false)
}

/// Register this binary as the handler for `trellis://` links.
///
/// Linux only for now: a `.desktop` file plus `xdg-mime`. macOS wants the scheme
/// in the bundle's `Info.plist` (so it belongs in the packaging step, not at
/// runtime) and Windows wants registry keys under `HKCU\Software\Classes`.
/// Both are deliberately left undone rather than half-done — the `http://`
/// form works on every platform today, which is why that form exists.
fn register_url_scheme() -> Result<String, String> {
    if !cfg!(target_os = "linux") {
        return Err(
            "only Linux is wired up; use the http://127.0.0.1:<port>/open/... form, which needs \
             no registration"
                .into(),
        );
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // No dependency for this: it is one environment variable, and taking a
    // crate for it would put it in every build.
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let dir = std::path::Path::new(&home).join(".local/share/applications");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let scheme = crate::URL_SCHEME;
    let file = dir.join(format!("{scheme}-url.desktop"));
    // `%u` hands the URL through as the first argument, which is exactly what
    // `follow_link` expects. NoDisplay keeps it out of the app menu: this is a
    // handler, not a second launcher for Trellis.
    let desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Trellis link handler\n\
         Exec={} %u\n\
         NoDisplay=true\n\
         Terminal=false\n\
         MimeType=x-scheme-handler/{scheme};\n",
        exe.display()
    );
    std::fs::write(&file, desktop).map_err(|e| e.to_string())?;
    // Best-effort: the file alone is enough on a desktop that rescans, and both
    // of these are absent on minimal systems.
    let _ = std::process::Command::new("update-desktop-database").arg(&dir).status();
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", &format!("{scheme}-url.desktop"), &format!("x-scheme-handler/{scheme}")])
        .status();
    Ok(file.display().to_string())
}

/// The Stick toggle, drawn in a detached panel's header. One setting covers
/// both panels — someone who wants their windows to travel with the app wants
/// it for the Agenda and the board alike, and two switches for one idea is two
/// things to find.
fn stick_toggle(ui: &mut egui::Ui, stick: &mut bool) {
    if ui
        .selectable_label(*stick, "📌")
        .on_hover_text(
            "Stick to the main window: detached panels move with Trellis, keeping \
             the offset you put them at. Applies to the Agenda and the Kanban board.",
        )
        .clicked()
    {
        *stick = !*stick;
    }
}

/// Where a stuck window is *meant* to be, so it can be told absolutely instead
/// of nudged.
///
/// **Why this is not a delta.** The first version added the main window's frame
/// delta to the panel's own reported position. Both readings lag the window
/// manager by a frame or more, and `OuterPosition` is answered with a position
/// that differs from what was asked by the window's decoration inset — so every
/// move left a small residue, the next move added to it, and the panel walked
/// off the side of the screen. Chasing a moving target with a measurement of
/// where you already are cannot converge.
///
/// Holding a *target* fixes it: the target moves with the main window, and the
/// command is sent **once per target**. A stale or offset reading can no longer
/// feed back into it, because nothing is ever measured to produce it.
#[derive(Default)]
struct StickState {
    /// Where this panel should sit.
    target: Option<egui::Pos2>,
    /// The last target actually sent, so an identical one is never re-sent —
    /// that is what stops a per-frame tug-of-war with the window manager.
    sent: Option<egui::Pos2>,
    /// Where the panel was last seen, to notice the user dragging it.
    seen: Option<egui::Pos2>,
    /// How long to keep ignoring movement after commanding one, while the
    /// window manager catches up — otherwise the panel moving *because Trellis
    /// moved it* reads as the user moving it.
    ///
    /// **Wall-clock, not a frame count.** Counting frames looks equivalent and
    /// is not: egui only repaints when something happens, so an idle app draws
    /// almost none, and a counter set to 8 was still armed the next time the
    /// window was touched — minutes later. It ate exactly the event it was
    /// meant to let through.
    settle_until: Option<std::time::Instant>,
}

/// Keep a detached panel at its offset from the main window.
///
/// **Relative, not anchored.** Dragging the app across the desk brings its
/// board along; a board parked on a second monitor keeps the offset you gave
/// it rather than being pulled into a fixed slot. Dragging the panel itself
/// re-teaches the offset, which is what the two guards below protect.
fn follow_main_window(
    vctx: &egui::Context, stick: bool, delta: egui::Vec2, st: &mut StickState,
) {
    let Some(rect) = vctx.input(|i| i.viewport().outer_rect) else { return };
    let measured = rect.min;
    if !stick {
        // Forget everything, so switching it back on adopts wherever the panel
        // is now instead of teleporting it to a stale target.
        *st = StickState::default();
        return;
    }
    // Is the panel simply sitting where we last put it? Then this position is
    // ours, not a move to learn from. This is the guard that matters; the timer
    // only covers the moment between asking and the window manager obeying.
    let ours = st.sent.is_some_and(|s| (measured - s).length() <= 2.0);
    let settling = st.settle_until.is_some_and(|t| std::time::Instant::now() < t);
    if !ours && !settling {
        if let Some(prev) = st.seen {
            // The user dragged this window: that is the new offset to keep.
            if (measured - prev).length() > 1.0 {
                st.target = Some(measured);
                st.sent = None;
            }
        }
    }
    st.seen = Some(measured);
    let target = st.target.get_or_insert(measured);
    if delta != egui::Vec2::ZERO {
        *target += delta;
    }
    let target = *target;
    if st.sent == Some(target) {
        return;
    }
    // **No clamp to the monitor.** The obvious backstop — keep the panel on
    // screen — was measured doing the opposite: `monitor_size` describes one
    // monitor while window positions are in whole-desktop coordinates, so on a
    // multi-monitor desk it pinned the panel to a box near the origin and the
    // follow visibly stopped part-way. egui exposes no origin for a monitor, so
    // the check cannot be written correctly, and a wrong guard is worse than
    // none: the runaway it was insuring against is fixed at its cause.
    st.target = Some(target);
    st.sent = Some(target);
    st.settle_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(400));
    vctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target));
}

/// Where a panel lives: inside the main window, or in one of its own.
///
/// A window inside a window cannot be moved to a second monitor, put beside the
/// canvas, or left open while you work — which is the whole point of a board you
/// glance at. egui's viewports make it a real OS window.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    Docked,
    Window,
}

impl Placement {
    fn from_str(s: &str) -> Self {
        if s == "window" { Self::Window } else { Self::Docked }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::Docked => "docked",
        }
    }
    fn label(self) -> &'static str {
        match self {
            // Says what clicking it does, not what state you are in.
            Self::Docked => "⧉ Detach",
            Self::Window => "⧉ Dock",
        }
    }
    fn toggled(self) -> Self {
        match self {
            Self::Docked => Self::Window,
            Self::Window => Self::Docked,
        }
    }
}

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
    // Emoji. egui bundles a *subset* of Noto Emoji that predates Unicode 12, so
    // anything newer drew tofu — U+1F534 🔴 was there and U+1F7E2 🟢 was not,
    // which is exactly the pair someone reaches for as a status indicator.
    //
    // This is the **outline** (monochrome) Noto Emoji, not NotoColorEmoji: that
    // one is CBDT/CBLC, a colour *bitmap* format with no glyph outlines, and
    // epaint rasterizes outlines — adding it renders nothing, silently. So
    // emoji are shape-only here by construction, and two coloured circles still
    // look identical. For status an agent should colour a table cell or use an
    // inline `<span style="color:…">`; see API.md.
    fonts.font_data.insert(
        "noto-emoji".to_owned(),
        egui::FontData::from_static(include_bytes!("../assets/NotoEmoji.ttf")),
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
    // Ahead of egui's own emoji fonts in the fallback chain, behind the text
    // fonts: it must win over the stale subset for a glyph both define, and
    // never over DejaVu for the arrows and dashes DejaVu draws better.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let chain = fonts.families.entry(family).or_default();
        let at = if chain.is_empty() { 0 } else { 1 };
        chain.insert(at, "noto-emoji".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// What a mirrored card looks like right now, for spotting a real change.
///
/// Cheap and comparable: the body for a text/code card, the cell text for a
/// table, plus the error either way. `CardKind` has no `PartialEq` — deriving one
/// across the whole enum (image bytes included) to answer this would be far more
/// than the question needs.
fn source_signature(c: &crate::model::Card) -> (String, Option<String>) {
    let content = match &c.kind {
        CardKind::Table { table } => table
            .rows
            .iter()
            .map(|r| r.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\u{1f}"))
            .collect::<Vec<_>>()
            .join("\u{1e}"),
        _ => c.body.clone(),
    };
    (content, c.source_error.clone())
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
/// The node id a Go-to-node query names, if it is one.
///
/// Accepts `12` and `#12`, with surrounding space. Nothing else: `12 notes` is a
/// search for a title, not an id, and treating it as one would hijack a perfectly
/// good text query.
/// Shorten `text` to `max` characters on a word boundary, with an ellipsis.
///
/// By characters, never bytes — a task line full of em-dashes and arrows would
/// panic on a byte slice. Breaks at whitespace so a row never ends mid-word.
fn elide(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    let cut = head.rfind(char::is_whitespace).unwrap_or(head.len());
    format!("{}…", head[..cut].trim_end())
}

/// One row in the Ctrl+O palette: a node, or a card inside one.
struct SwitcherHit {
    /// The basket to open. For a card or group hit this is its basket.
    node: NodeId,
    /// Set when this row is a card, so Enter reveals the card and not just its
    /// basket.
    card: Option<CardId>,
    /// Set when this row is a group, so Enter reveals the container.
    group: Option<crate::model::GroupId>,
    /// The id shown on the row — the node's, the card's, or the group's.
    id: u64,
    label: String,
    path: String,
    score: i32,
}

/// What to call a card in the palette. Cards very often have no title, so fall
/// back to the first non-empty line of the body: a row reading "(untitled card)"
/// tells you nothing about whether you found the right one.
/// Split a body for [`App::extract_selection`]: the text to move out, and the
/// head and tail the `![[#id]]` embed goes between.
///
/// **The embed has to land on its own line.** An embed is a block — it renders
/// the whole target card — so left inline it would be swallowed into the
/// surrounding paragraph and the card would read as one run of prose with a card
/// jammed into the middle of a sentence. Newlines are added only where there is
/// not one already, so extracting a whole paragraph does not leave blank lines
/// piling up behind it.
///
/// Returns `None` for an empty range, so the caller never creates a card for
/// nothing.
fn split_for_extract(chars: &[char], from: usize, to: usize) -> Option<(String, String, String)> {
    if from >= to || to > chars.len() {
        return None;
    }
    let taken: String = chars[from..to].iter().collect();
    let mut head: String = chars[..from].iter().collect();
    let mut tail: String = chars[to..].iter().collect();
    if !head.is_empty() && !head.ends_with('\n') {
        head.push('\n');
    }
    if !tail.is_empty() && !tail.starts_with('\n') {
        tail.insert(0, '\n');
    }
    Some((taken, head, tail))
}

fn card_label(c: &crate::model::Card) -> String {
    if !c.title.trim().is_empty() {
        return c.title.clone();
    }
    let line = c
        .body
        .lines()
        .map(|l| l.trim_start_matches(['#', '-', '*', '>', ' ']).trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.is_empty() {
        "(untitled card)".to_string()
    } else if line.chars().count() > 60 {
        format!("{}…", line.chars().take(60).collect::<String>())
    } else {
        line.to_string()
    }
}

/// A typed group id: `g146` or `#g146`. Kept separate from
/// [`queried_node_id`] because the `g` is the only thing distinguishing a group
/// id from a card id — they come from different counters and would otherwise
/// both match a bare number.
fn queried_group_id(query: &str) -> Option<crate::model::GroupId> {
    let t = query.trim();
    let rest = t.strip_prefix('#').unwrap_or(t);
    let digits = rest.strip_prefix(['g', 'G'])?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<crate::model::GroupId>().ok()
}

fn queried_node_id(query: &str) -> Option<NodeId> {
    let t = query.trim();
    let digits = t.strip_prefix('#').unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<NodeId>().ok()
}

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
            // A `.part` is an unfinished write and is excluded by the extension
            // check above. This drops anything that is not a readable gzip at
            // all — a snapshot that cannot be decompressed must never be offered
            // for restore, because the moment you need one is the worst possible
            // moment to discover it is empty.
            .filter(|p| is_readable_gzip(p))
            .filter_map(|p| p.file_name().map(|n| (p.clone(), n.to_string_lossy().into_owned())))
            .collect(),
        Err(_) => return Vec::new(),
    };
    v.sort_by(|a, b| b.1.cmp(&a.1)); // timestamped names sort chronologically
    v
}

/// Whether a snapshot file is a gzip that actually decompresses.
///
/// Cheap: it reads the header and pulls a few bytes through the decoder rather
/// than inflating the whole document, so listing history stays fast.
fn is_readable_gzip(p: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(p) else { return false };
    let mut d = flate2::read::GzDecoder::new(std::io::BufReader::new(f));
    let mut buf = [0u8; 64];
    matches!(d.read(&mut buf), Ok(n) if n > 0)
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
    // Temp-then-rename, like the document save itself. Writing straight to the
    // final name leaves a TRUNCATED `.gz` behind if the write does not finish —
    // and it looks like a perfectly good snapshot, with a valid timestamped
    // name, until the day you try to restore it. Observed: a 55 KB entry beside
    // 12.8 MB ones, written while the process was leaving.
    let tmp = dir.join(format!("{stamp}.ron.gz.part"));
    let final_path = dir.join(format!("{stamp}.ron.gz"));
    if std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, &final_path)).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
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
    let mut doc = ron::from_str::<Document>(&text).map_err(|e| e.to_string())?;
    // Every checklist written before items had ids arrives with `id: 0`. Assign
    // them here, once, so the rest of the app can assume an item *is* something
    // rather than a position. Idempotent, and it leaves the file alone until the
    // document is next saved for its own reasons.
    doc.ensure_item_ids();
    Ok(doc)
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

    /// **An embed is a block**, so it has to land on its own line — left inline it
    /// is swallowed into the surrounding paragraph and the card reads as prose
    /// with a whole card jammed mid-sentence. Newlines are added only where there
    /// is not one already, so extracting a whole paragraph does not pile blank
    /// lines up behind it.
    #[test]
    fn extract_puts_the_embed_on_its_own_line_without_piling_up_newlines() {
        let body: Vec<char> = "before SELECTED after".chars().collect();
        let (taken, head, tail) = split_for_extract(&body, 7, 15).unwrap();
        assert_eq!(taken, "SELECTED");
        assert_eq!(head, "before \n");
        assert_eq!(tail, "\n after");

        // Already on its own line: nothing is added.
        let para: Vec<char> = "one\nTWO\nthree".chars().collect();
        let (taken, head, tail) = split_for_extract(&para, 4, 7).unwrap();
        assert_eq!(taken, "TWO");
        assert_eq!(head, "one\n");
        assert_eq!(tail, "\nthree");

        // Selecting the whole body leaves nothing either side, and no stray
        // newline where there is no text to separate from.
        let all: Vec<char> = "everything".chars().collect();
        let (taken, head, tail) = split_for_extract(&all, 0, all.len()).unwrap();
        assert_eq!((taken.as_str(), head.as_str(), tail.as_str()), ("everything", "", ""));

        // An empty or backwards range creates nothing at all.
        assert!(split_for_extract(&all, 3, 3).is_none());
        assert!(split_for_extract(&all, 5, 2).is_none());
        assert!(split_for_extract(&all, 0, 999).is_none());
    }

    /// Typing an id has to find that node and nothing else. The `#` form is
    /// accepted because that is how ids are written in the docs and in the tree.
    #[test]
    fn go_to_node_accepts_a_node_id() {
        assert_eq!(queried_node_id("12"), Some(12));
        assert_eq!(queried_node_id("#12"), Some(12));
        assert_eq!(queried_node_id("  7 "), Some(7));
        assert_eq!(queried_node_id("0"), Some(0));
    }

    /// A query that merely *contains* digits is still a title search — hijacking
    /// it would break looking up "Q4 2026" or "v2".
    #[test]
    fn go_to_node_leaves_text_queries_alone() {
        assert_eq!(queried_node_id(""), None);
        assert_eq!(queried_node_id("#"), None);
        assert_eq!(queried_node_id("12 notes"), None);
        assert_eq!(queried_node_id("v2"), None);
        assert_eq!(queried_node_id("Q4 2026"), None);
        assert_eq!(queried_node_id("-3"), None);
    }

    /// A long task line has to shorten without panicking on a multi-byte char
    /// and without ending mid-word — agenda rows are 300+ characters now that a
    /// checklist item can carry its own context.
    #[test]
    fn elide_shortens_on_a_word_boundary_and_counts_characters() {
        assert_eq!(elide("short", 80), "short");
        assert_eq!(elide("one two three four", 11), "one two…");
        // Never slices a multi-byte character in half.
        let emdash = "→ ".repeat(200);
        let out = elide(&emdash, 40);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 41);
        // Exactly at the limit is left alone.
        let exact: String = "a".repeat(80);
        assert_eq!(elide(&exact, 80), exact);
        assert!(elide(&"a".repeat(81), 80).ends_with('…'));
    }

    /// An exact id must outrank every fuzzy hit, or typing "2" in a document
    /// full of "2026" cards would bury the node you asked for.
    #[test]
    fn an_id_match_sorts_above_any_fuzzy_score() {
        let best_fuzzy = fuzzy_score("2", "2026 plans", "2026 plans").unwrap();
        assert!(i32::MIN < best_fuzzy);
        // A card hit sits just above a node hit, so when one number names both,
        // the node still leads — but both are still ahead of every fuzzy row.
        assert!(i32::MIN < i32::MIN + 1 && i32::MIN + 1 < best_fuzzy);
    }

    /// Most cards have no title. A palette row reading "(untitled card)" would
    /// not tell you whether you'd found the right one, which is the only
    /// question the row exists to answer.
    #[test]
    fn a_card_row_is_labelled_by_its_title_then_its_first_real_line() {
        use crate::model::{Card, CardKind};
        let mut c = Card::new(1, egui::pos2(0.0, 0.0), CardKind::Text);
        c.title = "Has a title".into();
        c.body = "body text".into();
        assert_eq!(card_label(&c), "Has a title");

        // No title: the first non-empty line, with markdown ornament stripped so
        // a heading doesn't read as "### Heading".
        c.title = String::new();
        c.body = "\n\n### The real first line\nmore".into();
        assert_eq!(card_label(&c), "The real first line");

        c.body = "- a bullet".into();
        assert_eq!(card_label(&c), "a bullet");

        // Nothing at all is honest rather than blank.
        c.body = "   \n\n".into();
        assert_eq!(card_label(&c), "(untitled card)");

        // Long lines are truncated by *characters*, not bytes — slicing a
        // multi-byte char in half would panic on a card full of em-dashes.
        c.body = "é".repeat(200);
        let label = card_label(&c);
        assert!(label.ends_with('…'));
        assert_eq!(label.chars().count(), 61);
    }

    fn test_app_grants() -> Arc<Mutex<Vec<crate::plugins::Grant>>> {
        Arc::new(Mutex::new(Vec::new()))
    }

    /// The whole point of a per-agent token: revoking one must not disturb the
    /// others, and a plugin must never inherit one.
    #[test]
    fn agent_tokens_are_independent_of_each_other_and_of_plugins() {
        use crate::plugins::{Grant, Scope};
        let grants = test_app_grants();
        {
            let mut g = grants.lock().unwrap();
            g.push(Grant {
                plugin: "SCOUT".into(),
                token: "agent_a".into(),
                scope: Scope { read_only: false, subtree: Some(7) },
                standalone: true,
            });
            g.push(Grant {
                plugin: "BOB".into(),
                token: "agent_b".into(),
                scope: Scope { read_only: true, subtree: Some(9) },
                standalone: true,
            });
            // A *plugin* that happens to share a name with an agent.
            g.push(Grant {
                plugin: "SCOUT".into(),
                token: "plug_x".into(),
                scope: Scope::default(),
                standalone: false,
            });
        }
        // Revoking the agent leaves the other agent and the plugin alone.
        grants.lock().unwrap().retain(|g| !(g.standalone && g.plugin == "SCOUT"));
        let left = grants.lock().unwrap();
        let tokens: Vec<&str> = left.iter().map(|g| g.token.as_str()).collect();
        assert!(!tokens.contains(&"agent_a"), "the agent's token is gone");
        assert!(tokens.contains(&"agent_b"), "the other agent is untouched");
        assert!(tokens.contains(&"plug_x"), "the plugin's grant is untouched");
    }

    /// The sentence a token is checked against has to name the basket — "one
    /// basket" is not something anyone can audit.
    #[test]
    fn a_scoped_token_describes_itself_by_basket_name() {
        use crate::plugins::Scope;
        let s = Scope { read_only: false, subtree: Some(7) };
        assert_eq!(s.describe_named("Personal.ron", Some("SCOUT")), "read and change SCOUT and everything under it");
        let ro = Scope { read_only: true, subtree: Some(7) };
        assert_eq!(ro.describe_named("Personal.ron", Some("SCOUT")), "read SCOUT and everything under it");
        let whole = Scope::default();
        assert_eq!(
            whole.describe_named("Personal.ron", None),
            "read and change your whole Personal.ron document"
        );
    }

    /// Agent tokens are tellable from plugin tokens at a glance — they end up in
    /// config files elsewhere, and "which list do I revoke this from" has to be
    /// answerable from the string itself.
    #[test]
    fn agent_and_plugin_tokens_are_distinguishable() {
        let a = crate::plugins::mint_agent_token();
        let p = crate::plugins::mint_token();
        assert!(a.starts_with("agent_"));
        assert!(p.starts_with("plug_"));
        assert_ne!(a[6..], p[5..]);
        assert!(a.len() > 40);
    }

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

    /// Restart exists for one reason — the binary was replaced — and that is
    /// exactly when `/proc/self/exe` reads `… (deleted)`, because it follows the
    /// inode the process is running rather than the path. Taking it literally
    /// made Restart fail with `No such file or directory` on the only occasion
    /// it was needed.
    #[test]
    fn restart_finds_the_replacement_binary_not_the_deleted_inode() {
        use std::path::{Path, PathBuf};
        let live = Path::new("/opt/trellis/trellis");
        let here = |p: &Path| p == live;

        // The reported case: upgraded under a running process.
        assert_eq!(
            pick_exe(Some("/opt/trellis/trellis (deleted)".into()), None, here),
            Some(PathBuf::from("/opt/trellis/trellis")),
            "the new build at the same path was not found"
        );
        // A path that genuinely ends in that text is not mangled into nothing:
        // strip, find nothing there, fall back to argv[0].
        assert_eq!(
            pick_exe(
                Some("/gone/trellis (deleted)".into()),
                Some("/opt/trellis/trellis".into()),
                here,
            ),
            Some(PathBuf::from("/opt/trellis/trellis"))
        );
        // Ordinary case: current_exe wins and argv[0] is never consulted.
        assert_eq!(
            pick_exe(Some("/opt/trellis/trellis".into()), Some("nonsense".into()), here),
            Some(PathBuf::from("/opt/trellis/trellis"))
        );
        // A bare argv[0] is a PATH lookup, not a file — never spawn it blind.
        assert_eq!(pick_exe(None, Some("trellis".into()), here), None);
        assert_eq!(pick_exe(None, None, here), None);
    }
}

/// The `/go/…` hand-off page.
///
/// Deliberately tiny and self-contained: no network, no fonts, no images. It is
/// served over plain LAN HTTP to whatever browser a notification opened, so the
/// fewer assumptions it makes the more devices it works on.
///
/// **A link, not a redirect.** An automatic jump to a custom scheme is the thing
/// in-app browsers refuse; a tap is a user gesture, which they allow. The link is
/// also shown as text, because the one failure this cannot prevent is the app not
/// being installed — and then the reader needs to see what it was trying to open
/// rather than watch nothing happen.
fn go_page(path: &str, doc: &str, what: &str, where_: &str) -> String {
    // `path` and `doc` are ours (a kind plus an integer, and a file name), but the
    // title is the operator's text and goes into the document, so it is escaped.
    let title = crate::model::escape_html_pub(what);
    let place = crate::model::escape_html_pub(where_);
    let doc_js = doc.replace('\\', "\\\\").replace('"', "\\\"");
    let subtitle = if place.is_empty() { String::new() } else { format!("<p class=\"where\">{place}</p>") };
    format!(
        r##"<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Open in Trellis</title>
<style>
  :root {{ color-scheme: light dark; }}
  body {{ margin: 0; min-height: 100vh; display: flex; align-items: center;
         justify-content: center; font: 16px/1.5 system-ui, sans-serif;
         background: #faf9f7; color: #1c1917; padding: 24px; }}
  main {{ max-width: 30rem; width: 100%; text-align: center; }}
  h1 {{ font-size: 1.05rem; font-weight: 600; margin: 0 0 .25rem; }}
  .where {{ margin: 0 0 1.5rem; font-size: .85rem; opacity: .65; }}
  a.go {{ display: block; padding: .9rem 1.25rem; border-radius: .6rem;
          background: #4f46e5; color: #fff; text-decoration: none;
          font-weight: 600; }}
  code {{ display: block; margin-top: 1.5rem; font-size: .72rem; opacity: .55;
          word-break: break-all; }}
  @media (prefers-color-scheme: dark) {{
    body {{ background: #1c1917; color: #f5f5f4; }}
  }}
</style>
</head><body><main>
<h1>{title}</h1>
{subtitle}
<a class="go" id="go" href="#">Open in Trellis</a>
<code id="url"></code>
<script>
  // location.host is the only address known to be reachable from THIS device:
  // the desktop would have written 127.0.0.1, which on a phone is the phone.
  var url = "trellis://" + location.host + "/{path}?doc=" + encodeURIComponent("{doc_js}");
  document.getElementById("go").href = url;
  document.getElementById("url").textContent = url;
</script>
</main></body></html>
"##
    )
}

/// This machine's addresses another device could reach it on, best first.
///
/// Found by asking the **routing table** rather than enumerating interfaces, which
/// needs no dependency: a UDP socket is `connect`ed to an address and its own
/// local address read back. **No packet is sent** — `connect` on a UDP socket only
/// fixes the peer and selects a route.
///
/// One probe is not enough, and this machine is why. Its default route is a **VPN**
/// (`tun0`, a 100.64/10 carrier-grade-NAT address) while the phone is on one of two
/// ordinary LANs — so the single "route off this machine" answer was confidently
/// the one address a phone cannot use. Probing each private range as well finds the
/// on-link address for each, and **RFC 1918 is preferred over CGNAT** because a
/// 100.64 address is nearly always the VPN rather than the network the reader is on.
///
/// Still a guess, so it is offered as a hint and every consumer can override it.
fn lan_addresses() -> Vec<String> {
    // 192.0.2.1 is TEST-NET-1 (reserved, never routed to) and stands for "off this
    // machine"; the rest each stand for their own private range.
    // `192.168.1.x` earns its own probe rather than being covered by
    // `192.168.0.x`: they are separate /24s, and this machine is on both (wired
    // and wireless), so probing only one found one of its two LANs.
    const PROBES: [&str; 5] =
        ["192.0.2.1:9", "192.168.0.1:9", "192.168.1.1:9", "10.0.0.1:9", "172.16.0.1:9"];
    let mut found: Vec<std::net::Ipv4Addr> = Vec::new();
    for probe in PROBES {
        let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") else { continue };
        if sock.connect(probe).is_err() {
            continue;
        }
        if let Ok(std::net::SocketAddr::V4(addr)) = sock.local_addr() {
            let ip = *addr.ip();
            if !ip.is_loopback() && !ip.is_unspecified() && !found.contains(&ip) {
                found.push(ip);
            }
        }
    }
    found.sort_by_key(|ip| !ip.is_private()); // private first, stable otherwise
    found.into_iter().map(|ip| ip.to_string()).collect()
}

/// A readable name for a view built from the Find panel's filters, so the card
/// says what it shows without anyone opening it.
fn describe_find(tag: Option<&str>, key: Option<&str>, value: &str, text: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = tag {
        parts.push(format!("#{t}"));
    }
    if let Some(k) = key {
        let v = value.trim();
        parts.push(if v.is_empty() { format!("has {k}::") } else { format!("{k}:: {v}") });
    }
    let t = text.trim();
    if !t.is_empty() {
        parts.push(format!("\u{201c}{t}\u{201d}"));
    }
    if parts.is_empty() {
        "Saved view".to_string()
    } else {
        parts.join(" \u{b7} ")
    }
}
