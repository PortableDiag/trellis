//! Central basket canvas: a pannable surface of draggable, resizable, editable
//! cards. Each card renders according to its `CardKind`.

use crate::images::TextureCache;
use crate::model::{Card, CardGroup, CardId, CardKind, ChecklistItem, GroupId, Node};
use std::collections::{HashMap, HashSet};
use egui::text::{CCursor, CCursorRange};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use emath::TSTransform;

/// Zoom bounds for the canvas view.
pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 3.0;

/// How long (seconds) the reveal-a-card flash outline lingers, fading out.
pub const HIGHLIGHT_SECS: f64 = 1.4;

/// Shared, frame-persistent caches the canvas needs.
pub struct Env<'a> {
    pub md: &'a mut CommonMarkCache,
    pub tex: &'a mut TextureCache,
    /// Filled each frame with every drawn card's on-screen rect (points), so the
    /// app can crop a framebuffer screenshot to a single card for WYSIWYG export.
    pub card_rects: &'a mut HashMap<CardId, egui::Rect>,
    /// Names of the user's saved card templates, for the "Insert template" menu.
    pub templates: &'a [String],
    /// Approved plugins offering the `card-menu` trigger, as (index, title).
    pub card_plugins: &'a [(usize, String)],
    /// `bytes://` URIs already registered with egui this session, so a text
    /// card's inline images are uploaded once instead of every frame.
    pub inline_sent: &'a mut std::collections::HashSet<String>,
    /// Document generation, mixed into inline-image URIs so a reloaded document
    /// can't collide with the previous one's cached image textures.
    pub inline_epoch: u64,
    /// One-shot request to recenter the view on this card (agenda/Kanban row
    /// click). Consumed by the app after the call.
    pub focus_card: Option<CardId>,
    /// Card to flash-highlight, and the `ui.input().time` at which the flash ends.
    pub highlight_card: Option<CardId>,
    pub highlight_until: f64,
    /// Draw the bottom-right minimap (overview of the basket + a view reticle).
    pub minimap: bool,
    /// Theme-driven card look (Normal / Sticky / Futuristic).
    pub style: CardStyle,
    /// Draw a soft accent-colored glow behind each card (the radiant neon
    /// themes: Futuristic, SynthWave).
    pub glow: bool,
}

/// How cards are painted, chosen by the active theme. `Normal` is the default
/// look (panel-fill body + accent title bar). `Sticky` paints the whole card one
/// solid paper color — header and body the same, like a real sticky note.
/// `Futuristic` gives it a beveled tech-panel frame.
#[derive(Clone, Copy, PartialEq)]
pub enum CardStyle {
    Normal,
    Sticky,
    Futuristic,
}

/// The default accent a fresh card is created with (`model::Card::new`). In the
/// Sticky theme, a card still on this default is drawn as classic yellow paper;
/// once recolored, the whole note takes the chosen color.
const DEFAULT_CARD_COLOR: [u8; 3] = [0x3b, 0x82, 0xf6];

/// Darken an opaque color by scaling its RGB toward black (keeps full alpha,
/// unlike `gamma_multiply`, which also fades the alpha).
fn darken(c: egui::Color32, f: f32) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
    )
}

/// Linear-ish mix of two opaque colors (`t` = weight of `b`), for approximating
/// what a translucent title tint looks like once painted over the body.
fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t) as u8;
    egui::Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Perceived luminance (0=black, 1=white) of an opaque color.
fn luminance(c: egui::Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

/// A readable card-title color for a title bar of the given background: the
/// theme's bright text on a dark bar, near-black on a light one. This decouples
/// the title from the theme's *accent* (egui's `strong_text_color` is the loud
/// active-widget color), which otherwise put e.g. SynthWave's pink text on a
/// blue card — a contrast no-no.
fn title_text_color(v: &egui::Visuals, bar_bg: egui::Color32) -> egui::Color32 {
    if luminance(bar_bg) < 0.5 {
        v.override_text_color.unwrap_or(egui::Color32::from_gray(0xEC))
    } else {
        egui::Color32::from_gray(0x18)
    }
}

/// A rectangle with its top-right and bottom-left corners cut at 45° (a diagonal
/// bevel) — the tech-panel look for the Futuristic theme. `c` is the cut size.
fn bevel_diag(rect: egui::Rect, c: f32) -> Vec<egui::Pos2> {
    let c = c.min(rect.width() * 0.5).min(rect.height() * 0.5);
    vec![
        rect.left_top(),
        egui::pos2(rect.right() - c, rect.top()),
        egui::pos2(rect.right(), rect.top() + c),
        rect.right_bottom(),
        egui::pos2(rect.left() + c, rect.bottom()),
        egui::pos2(rect.left(), rect.bottom() - c),
    ]
}

/// The title strip with only its top-right corner cut, matching the top of a
/// `bevel_diag` card.
fn bevel_title(title: egui::Rect, c: f32) -> Vec<egui::Pos2> {
    let c = c.min(title.width() * 0.5);
    vec![
        title.left_top(),
        egui::pos2(title.right() - c, title.top()),
        egui::pos2(title.right(), title.top() + c),
        title.right_bottom(),
        title.left_bottom(),
    ]
}

/// Actions requested by the canvas, applied by the app afterwards.
pub enum CanvasAction {
    AddCard(CardKind, egui::Pos2),
    MoveCard(CardId, egui::Vec2),
    ResizeCard(CardId, egui::Vec2),
    /// Resize a card to fit its content (right-click → Fit to content).
    FitCard(CardId),
    RaiseCard(CardId),
    SetTitle(CardId, String),
    SetBody(CardId, String),
    SetLang(CardId, String),
    SetColor(CardId, [u8; 3]),
    SetFontScale(CardId, f32),
    SetEditing(CardId, bool),
    Duplicate(CardId),
    CopyCard(CardId),
    PasteCard(egui::Pos2),
    Remove(CardId),
    ResetView,
    /// Files dropped onto the canvas, to become cards at the given world pos.
    DropFiles(Vec<egui::DroppedFile>, egui::Pos2),
    ChecklistToggle(CardId, usize),
    ChecklistSetText(CardId, usize, String),
    ChecklistAdd(CardId),
    ChecklistRemove(CardId, usize),
    /// Reorder a checklist item from index `from` to before index `to`.
    ChecklistMove(CardId, usize, usize),
    // Sketch (freehand draw) cards.
    SketchAddStroke(CardId, crate::model::Stroke),
    SketchUndo(CardId),
    SketchClear(CardId),
    LoadImage(CardId),
    /// Pick an image file and embed it inline in a Text card's body, splicing a
    /// `![](trellis:N)` marker at the given cursor char position (toolbar button).
    InsertInlineImage(CardId, usize),
    RemoveImage(CardId, usize),
    /// Run OCR over an image card's images and store the extracted text.
    OcrCard(CardId),
    /// Run an approved `card-menu` plugin against this card (index into the
    /// app's plugin list).
    RunCardPlugin(CardId, usize),
    /// Save one of an image card's images to a file (index into `kind.images()`).
    SaveImage(CardId, usize),
    /// Save all of an image card's images into a chosen folder.
    SaveAllImages(CardId),
    /// Export a single card to a shareable file (one variant per format).
    ExportCardPng(CardId),
    ExportCardMarkdown(CardId),
    ExportCardPdf(CardId),
    ExportCardHtml(CardId),
    ExportCardText(CardId),
    ExportCardSvg(CardId),
    ExportCardJson(CardId),
    /// Import a card from a JSON file (opens a picker), placed at the world pos.
    ImportCard(egui::Pos2),
    /// Save a card as a reusable template (stored in app config).
    SaveAsTemplate(CardId),
    /// Overwrite the template at this index from this card (re-snapshot in place),
    /// so an edited Templates-folder master updates the stored template.
    UpdateTemplate(usize, CardId),
    /// Insert the template at this index as a new card at the world pos.
    InsertTemplate(usize, egui::Pos2),
    /// Delete the saved template at this index.
    DeleteTemplate(usize),
    // Table (spreadsheet) cards.
    TableSetCell(CardId, usize, usize, String),
    TableSetBg(CardId, usize, usize, Option<[u8; 3]>),
    TableSetFg(CardId, usize, usize, Option<[u8; 3]>),
    TableInsertRow(CardId, usize),
    TableRemoveRow(CardId, usize),
    TableInsertCol(CardId, usize),
    TableRemoveCol(CardId, usize),
    TableSetColWidth(CardId, usize, f32),
    TableToggleHeader(CardId),
    /// Draw this table as a chart (`None` = back to a plain grid).
    TableSetChart(CardId, Option<crate::model::ChartSpec>),
    TableImport(CardId),
    TableExportCsv(CardId),
    TableExportXlsx(CardId),
    /// Open the full-screen image viewer at the given image of a card.
    OpenLightbox(CardId, usize),
    // Multi-select (runtime only; used to build a group).
    ToggleSelect(CardId),
    ClearSelection,
    // Grouping.
    GroupSelected,
    Ungroup(GroupId),
    RaiseGroup(GroupId),
    MoveGroup(GroupId, egui::Vec2),
    SetGroupTitle(GroupId, String),
    SetGroupColor(GroupId, [u8; 3]),
    // Docking (stick a card onto another).
    /// Choose a file for this card to mirror (opens a file dialog).
    PickSource(CardId),
    /// Stop mirroring: keep the text, drop the link.
    ClearSource(CardId),
    DockCard(CardId, CardId),
    DetachCard(CardId),
    ToggleDockMode,
    ToggleSnapMode,
}

const TITLE_H: f32 = 24.0;
/// How close (world units) a dragged edge must be to snap to another card's edge.
const SNAP_DIST: f32 = 8.0;

/// The canvas view: `view.translation` is the pan (screen px, relative to the
/// canvas top-left) and `view.scaling` is the zoom. Cards live in "world"
/// coordinates (`card.pos`); the layer transform below maps world → screen so
/// that only the cards zoom — the surrounding chrome never does.
pub fn ui(
    ui: &mut egui::Ui,
    node: &Node,
    node_path: &str,
    view: &mut TSTransform,
    zoom_enabled: bool,
    can_paste: bool,
    dock_mode: bool,
    snap_mode: bool,
    env: &mut Env,
    selection: &HashSet<CardId>,
) -> Vec<CanvasAction> {
    let mut actions = Vec::new();

    let (canvas_rect, canvas_resp) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    ui.set_clip_rect(canvas_rect);

    // Background + grid. A per-node basket color overrides the theme default
    // (the black grid canvas); the grid is still drawn on top of it.
    let painter = ui.painter_at(canvas_rect);
    let bg = node
        .bg
        .map(|c| egui::Color32::from_rgb(c[0], c[1], c[2]))
        .unwrap_or_else(|| ui.visuals().extreme_bg_color);
    painter.rect_filled(canvas_rect, 0.0, bg);
    draw_grid(&painter, canvas_rect, *view, ui.visuals().weak_text_color());

    // Minimap interaction — resolved from raw pointer input *before* the canvas
    // pan, so a press that starts on the map claims the view (recenter on the
    // pointed-at spot) and the empty-canvas pan is suppressed. The drag latches
    // via a memory flag so it keeps tracking even if the pointer leaves the box.
    // The map's *visuals* are painted later, on top of the cards.
    let minimap_geom = if env.minimap { minimap_geometry(canvas_rect, &node.cards) } else { None };
    let mut minimap_active = false; // a minimap drag owns the view this frame
    let mut minimap_over = false; // pointer is over the map (suppress canvas gestures)
    if let Some(g) = &minimap_geom {
        let drag_id = ui.id().with("minimap_drag");
        let mut dragging = ui.memory(|m| m.data.get_temp::<bool>(drag_id).unwrap_or(false));
        let (pressed, down, pos) = ui.input(|i| {
            (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.latest_pos())
        });
        if let Some(p) = pos {
            minimap_over = g.outer.contains(p);
            // Only claim the view if the press *began* on the map. A canvas drag
            // that merely passes over the minimap must NOT grab the reticle and
            // teleport the view — you have to press down inside the map first.
            if pressed && minimap_over {
                dragging = true;
            }
        }
        // Recenter on the pointed-at spot while a latched minimap drag is held. A
        // plain click lands here too: the press over the map recenters at once —
        // computed *before* clearing the latch so a fast click (press+release in
        // one frame, `down` already false) still recenters.
        let target = if dragging { pos } else { None };
        if !down {
            dragging = false;
        }
        if let Some(p) = target {
            // Clamp to the map, map back to world, recenter the view there.
            let cp = egui::pos2(
                p.x.clamp(g.inner.min.x, g.inner.max.x),
                p.y.clamp(g.inner.min.y, g.inner.max.y),
            );
            let w = g.world_min + (cp - g.inner.min) / g.scale;
            view.translation =
                (canvas_rect.center() - canvas_rect.min) - view.scaling * w.to_vec2();
            minimap_active = true;
            ui.ctx().request_repaint();
        }
        ui.memory_mut(|m| m.data.insert_temp(drag_id, dragging));
    }

    // Pan by dragging empty canvas (screen-space delta) — unless the minimap has
    // claimed this drag.
    if canvas_resp.dragged_by(egui::PointerButton::Primary) && !minimap_active {
        view.translation += canvas_resp.drag_delta();
    }

    // Wheel over empty canvas pans; Ctrl+wheel (and pinch) zoom instead — egui
    // routes Ctrl+scroll into zoom_delta and out of smooth_scroll_delta.
    if canvas_resp.hovered() && !minimap_over {
        view.translation += ui.input(|i| i.smooth_scroll_delta);
        if zoom_enabled {
            let zd = ui.input(|i| i.zoom_delta());
            if (zd - 1.0).abs() > f32::EPSILON {
                if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
                    zoom_at(view, canvas_rect, ptr, zd);
                }
            }
        }
    }

    // Keyboard zoom (canvas-only): +/- around the canvas centre, Ctrl+0 resets.
    let cmd = ui.input(|i| i.modifiers.command);
    if zoom_enabled && cmd {
        if ui.input(|i| i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)) {
            zoom_at(view, canvas_rect, canvas_rect.center(), 1.1);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Minus)) {
            zoom_at(view, canvas_rect, canvas_rect.center(), 1.0 / 1.1);
        }
    }
    if cmd && ui.input(|i| i.key_pressed(egui::Key::Num0)) {
        *view = TSTransform::IDENTITY; // reset works even if zoom is disabled
    }

    // One-shot: recenter the view on a card the app asked us to reveal (agenda /
    // Kanban row click). Keep the current zoom; just pan so the card's center
    // lands in the middle of the canvas.
    if let Some(fid) = env.focus_card {
        if let Some(c) = node.cards.iter().find(|c| c.id == fid) {
            let world_center = c.pos + c.size * 0.5;
            view.translation = (canvas_rect.center() - canvas_rect.min)
                - view.scaling * world_center.to_vec2();
        }
    }

    // world → screen for this canvas.
    let to_screen = TSTransform::from_translation(canvas_rect.min.to_vec2()) * *view;

    // Double-click empty canvas → drop a text card at that world position.
    if canvas_resp.double_clicked() && !minimap_over {
        if let Some(p) = canvas_resp.interact_pointer_pos() {
            actions.push(CanvasAction::AddCard(CardKind::Text, to_screen.inverse() * p));
        }
    }

    // Drag & drop files from the OS: text/markdown → text card, image → image
    // card, dropped at the pointer. A hint overlay shows while files hover.
    if ui.input(|i| !i.raw.hovered_files.is_empty()) {
        let p = ui.painter_at(canvas_rect);
        p.rect_stroke(
            canvas_rect.shrink(4.0),
            8.0,
            egui::Stroke::new(2.0, egui::Color32::from_rgb(0x4a, 0xde, 0x80)),
        );
        p.text(
            canvas_rect.center(),
            egui::Align2::CENTER_CENTER,
            "Drop files to add cards",
            egui::FontId::proportional(18.0),
            egui::Color32::from_rgb(0x4a, 0xde, 0x80),
        );
    }
    let dropped = ui.input(|i| i.raw.dropped_files.clone());
    if !dropped.is_empty() {
        let screen = ui
            .input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()))
            .unwrap_or_else(|| canvas_rect.center());
        actions.push(CanvasAction::DropFiles(dropped, to_screen.inverse() * screen));
    }

    // Clicking empty canvas clears any card multi-selection.
    if canvas_resp.clicked() {
        actions.push(CanvasAction::ClearSelection);
    }

    // Right-click empty canvas → choose a card kind to add, at the click spot.
    // The click's world position is captured when the menu opens: on the later
    // frame where a menu item is actually clicked, the pointer is on the menu,
    // not the canvas, so reading interact_pointer_pos() then would yield None
    // (which used to drop new cards at world (40,40) — the "top area" bug).
    let menu_world_key = ui.id().with("canvas_menu_world_pos");
    if canvas_resp.secondary_clicked() {
        if let Some(p) = canvas_resp.interact_pointer_pos() {
            ui.memory_mut(|m| m.data.insert_temp(menu_world_key, to_screen.inverse() * p));
        }
    }
    canvas_resp.context_menu(|ui| {
        ui.label("Add card");
        ui.separator();
        let cp = ui
            .memory(|m| m.data.get_temp::<egui::Pos2>(menu_world_key))
            .unwrap_or_else(|| to_screen.inverse() * canvas_rect.center());
        if ui.button("Text").clicked() {
            actions.push(CanvasAction::AddCard(CardKind::Text, cp));
            ui.close_menu();
        }
        if ui.button("Code").clicked() {
            actions.push(CanvasAction::AddCard(CardKind::Code { lang: "rust".into() }, cp));
            ui.close_menu();
        }
        if ui.button("Checklist").clicked() {
            actions.push(CanvasAction::AddCard(
                CardKind::Checklist {
                    items: vec![ChecklistItem { done: false, text: String::new() }],
                },
                cp,
            ));
            ui.close_menu();
        }
        if ui.button("Table").clicked() {
            actions.push(CanvasAction::AddCard(
                CardKind::Table { table: crate::model::TableData::empty(3, 3) },
                cp,
            ));
            ui.close_menu();
        }
        if ui.button("Image").clicked() {
            actions.push(CanvasAction::AddCard(
                CardKind::Image { data: Vec::new(), name: String::new(), extra: Vec::new(), ocr: String::new() },
                cp,
            ));
            ui.close_menu();
        }
        if ui.button("Sketch").clicked() {
            actions.push(CanvasAction::AddCard(CardKind::Sketch { strokes: Vec::new() }, cp));
            ui.close_menu();
        }
        ui.separator();
        if ui
            .button("Import card…")
            .on_hover_text("Add a card from a Trellis JSON card file")
            .clicked()
        {
            actions.push(CanvasAction::ImportCard(cp));
            ui.close_menu();
        }
        if ui
            .add_enabled(can_paste, egui::Button::new("Paste card"))
            .clicked()
        {
            actions.push(CanvasAction::PasteCard(cp));
            ui.close_menu();
        }
        ui.menu_button("Insert template", |ui| {
            if env.templates.is_empty() {
                ui.add_enabled(false, egui::Button::new("No templates yet"));
                ui.label("Right-click a card → Save as template");
            }
            for (i, name) in env.templates.iter().enumerate() {
                ui.horizontal(|ui| {
                    let label = if name.trim().is_empty() { "(untitled)" } else { name.as_str() };
                    if ui.button(label).clicked() {
                        actions.push(CanvasAction::InsertTemplate(i, cp));
                        ui.close_menu();
                    }
                    if ui.small_button("✕").on_hover_text("Delete this template").clicked() {
                        actions.push(CanvasAction::DeleteTemplate(i));
                        ui.close_menu();
                    }
                });
            }
        });
    });

    let zoom = to_screen.scaling;
    let world_rect = |c: &Card| egui::Rect::from_min_size(c.pos, c.size);
    let screen_rect = |c: &Card| to_screen.mul_rect(world_rect(c));

    // --- group containers, drawn behind their member cards ------------------
    let mut gbounds: HashMap<GroupId, egui::Rect> = HashMap::new();
    for card in &node.cards {
        if let Some(g) = card.group {
            let wr = world_rect(card);
            gbounds.entry(g).and_modify(|r| *r = r.union(wr)).or_insert(wr);
        }
    }
    let bg = ui.painter_at(canvas_rect);
    // The header strip draws behind the cards (bg painter) and its interaction is
    // registered here, *before* the cards — so where a card overlaps the header,
    // the card wins and the buried part of the header neither responds nor bleeds
    // through on hover. Only the visible part is clickable. A header being dragged
    // is repainted on top after the cards so you can see it while you move it.
    let mut dragging_header: Option<(GroupId, egui::Rect)> = None;
    for group in &node.groups {
        let Some(wb) = gbounds.get(&group.id) else { continue };
        let srect = to_screen.mul_rect(wb.expand(10.0));
        let gcol = egui::Color32::from_rgb(group.color[0], group.color[1], group.color[2]);
        bg.rect(
            srect,
            6.0 * zoom,
            gcol.gamma_multiply(0.06),
            egui::Stroke::new(1.5, gcol.gamma_multiply(0.75)),
        );
        // Header strip above the box: click to raise, drag to move, RMB for menu.
        let hh = 18.0 * zoom;
        let header = egui::Rect::from_min_size(
            egui::pos2(srect.min.x, srect.min.y - hh - 3.0 * zoom),
            egui::vec2(srect.width(), hh),
        );
        bg.rect_filled(header, 4.0 * zoom, gcol.gamma_multiply(0.9));
        let label = if group.title.is_empty() { "Group" } else { group.title.as_str() };
        bg.text(
            header.left_center() + egui::vec2(6.0 * zoom, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(11.0 * zoom),
            egui::Color32::from_gray(240),
        );
        let hresp =
            ui.interact(header, ui.id().with(("group_hdr", group.id)), egui::Sense::click_and_drag());
        // Clicking a visible part of the header brings the whole group to the top.
        if hresp.clicked() {
            actions.push(CanvasAction::RaiseGroup(group.id));
        }
        if hresp.dragged() {
            actions.push(CanvasAction::MoveGroup(group.id, hresp.drag_delta() / zoom));
            dragging_header = Some((group.id, header));
        }
        hresp.context_menu(|ui| group_menu(ui, group, &mut actions));
    }

    // --- dock connectors: faint links between stuck cards -------------------
    for card in &node.cards {
        if let Some(anchor_id) = card.docked_to {
            if let Some(anchor) = node.cards.iter().find(|c| c.id == anchor_id) {
                bg.line_segment(
                    [to_screen * world_rect(card).center(), to_screen * world_rect(anchor).center()],
                    egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
                );
            }
        }
    }

    // --- docking: detach on drag start, dock on drop, highlight the target --
    // `card + its dock subtree` — excluded from being its own drop target.
    let subtree = |root: CardId| -> Vec<CardId> {
        let mut ids = vec![root];
        loop {
            let mut added = false;
            for c in &node.cards {
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
        ids
    };
    let card_at = |pt: egui::Pos2, exclude: &[CardId]| -> Option<CardId> {
        node.cards
            .iter()
            .rev()
            .find(|c| !exclude.contains(&c.id) && screen_rect(c).contains(pt))
            .map(|c| c.id)
    };
    let mut dragging: Option<CardId> = None;
    for card in &node.cards {
        if ui.ctx().is_being_dragged(ui.id().with(("card_handle", card.id))) {
            dragging = Some(card.id);
        }
    }
    let mut dock_highlight: Option<egui::Rect> = None;
    if dock_mode {
        let mem_key = ui.id().with("canvas_dragging_card");
        let prev: Option<CardId> =
            ui.memory(|m| m.data.get_temp::<Option<CardId>>(mem_key)).flatten();
        ui.memory_mut(|m| m.data.insert_temp(mem_key, dragging));
        if let (Some(cur), None) = (dragging, prev) {
            // Drag just started: pop the card out of its current dock.
            actions.push(CanvasAction::DetachCard(cur));
        }
        if let Some(cur) = dragging {
            if let Some(pt) = ui.input(|i| i.pointer.hover_pos()) {
                if let Some(target) = card_at(pt, &subtree(cur)) {
                    if let Some(t) = node.cards.iter().find(|c| c.id == target) {
                        dock_highlight = Some(screen_rect(t));
                    }
                }
            }
        }
        if let (None, Some(pc)) = (dragging, prev) {
            // Drag just ended: dock onto whatever card is under the drop point.
            if let Some(pt) = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos())) {
                if let Some(target) = card_at(pt, &subtree(pc)) {
                    actions.push(CanvasAction::DockCard(pc, target));
                }
            }
        }
    }

    // Cards are drawn directly at their zoomed screen rects (see card_ui), which
    // keeps text selection/editing working (transformed layers broke it).
    env.card_rects.clear();
    for card in &node.cards {
        env.card_rects.insert(card.id, screen_rect(card));
        card_ui(
            ui,
            card,
            node_path,
            to_screen,
            canvas_rect,
            env,
            selection.contains(&card.id),
            snap_mode.then_some(&node.cards[..]),
            &mut actions,
        );
    }

    // While a header is being dragged, repaint it on top of the cards so you can
    // see the handle you grabbed as the group moves.
    if let Some((gid, header)) = dragging_header {
        if let Some(group) = node.groups.iter().find(|g| g.id == gid) {
            let top = ui.painter_at(canvas_rect);
            let gcol = egui::Color32::from_rgb(group.color[0], group.color[1], group.color[2]);
            top.rect_filled(header, 4.0 * zoom, gcol);
            let label = if group.title.is_empty() { "Group" } else { group.title.as_str() };
            top.text(
                header.left_center() + egui::vec2(6.0 * zoom, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(11.0 * zoom),
                egui::Color32::WHITE,
            );
        }
    }

    // Drop-target highlight, painted on top of the cards.
    if let Some(hr) = dock_highlight {
        ui.painter_at(canvas_rect).rect_stroke(
            hr.expand(2.0 * zoom),
            6.0 * zoom,
            egui::Stroke::new(2.5, egui::Color32::from_rgb(0x4a, 0xde, 0x80)),
        );
    }

    // Reveal flash: a card the app asked us to jump to (agenda/Kanban) gets a
    // bright outline that fades over HIGHLIGHT_SECS, so a click clearly lands.
    if let Some(hid) = env.highlight_card {
        let now = ui.input(|i| i.time);
        if now < env.highlight_until {
            if let Some(c) = node.cards.iter().find(|c| c.id == hid) {
                let frac = ((env.highlight_until - now) / HIGHLIGHT_SECS).clamp(0.0, 1.0) as f32;
                let alpha = (frac * 255.0) as u8;
                ui.painter_at(canvas_rect).rect_stroke(
                    screen_rect(c).expand(4.0 * zoom),
                    6.0 * zoom,
                    egui::Stroke::new(
                        3.0,
                        egui::Color32::from_rgba_unmultiplied(0xff, 0xd1, 0x66, alpha),
                    ),
                );
                ui.ctx().request_repaint(); // keep animating the fade
            }
        }
    }

    // Minimap visuals — painted on top of the cards. A small overview of the
    // whole basket in the bottom-right, with an amber reticle for the current
    // view, so you can spot cards far from the main cluster (and click/drag to jump
    // there — handled at the top of this function). Toggle in Settings → Canvas.
    if let Some(g) = &minimap_geom {
        let paint = ui.painter_at(canvas_rect);
        paint.rect_filled(g.outer, 4.0, ui.visuals().panel_fill.gamma_multiply(0.92));
        let border = if minimap_over {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        paint.rect_stroke(g.outer, 4.0, egui::Stroke::new(1.0, border));
        let w2m = |w: egui::Pos2| g.inner.min + (w - g.world_min) * g.scale;
        for c in &node.cards {
            let r = egui::Rect::from_min_max(w2m(c.pos), w2m(c.pos + c.size));
            let col = egui::Color32::from_rgb(c.color[0], c.color[1], c.color[2]);
            paint.rect_filled(r, 0.0, col.gamma_multiply(0.85));
        }
        // Reticle: the current viewport (screen → world → minimap).
        let inv = |sp: egui::Pos2| {
            egui::pos2(
                (sp.x - canvas_rect.min.x - view.translation.x) / view.scaling,
                (sp.y - canvas_rect.min.y - view.translation.y) / view.scaling,
            )
        };
        let reticle = egui::Rect::from_min_max(w2m(inv(canvas_rect.min)), w2m(inv(canvas_rect.max)))
            .intersect(g.inner);
        paint.rect_stroke(
            reticle,
            0.0,
            egui::Stroke::new(1.5, egui::Color32::from_rgb(0xff, 0xd1, 0x66)),
        );
    }

    // Reset-view button — in a foreground layer, untransformed, so it stays put
    // and clickable above the cards.
    let btn_pos = egui::pos2(canvas_rect.right() - 104.0, canvas_rect.top() + 8.0);
    // `Middle`, not `Foreground`: the cards are painted in the panel's
    // background layer, so Middle is already above them — while Foreground is
    // above *windows* too, which had these toggles painting straight through
    // Settings, Kanban and every other window, and taking the clicks with them.
    egui::Area::new(ui.id().with("reset_view"))
        .order(egui::Order::Middle)
        .fixed_pos(btn_pos)
        .show(ui.ctx(), |ui| {
            // Keep the label on one line — the Area would otherwise size narrow
            // and wrap "Reset view" onto two lines.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if ui
                .button("Reset view")
                .on_hover_text("Reset zoom to 100% and recenter the canvas")
                .clicked()
            {
                actions.push(CanvasAction::ResetView);
            }
        });

    // Card tools (top-left): Dock-mode toggle and, when 2+ cards are selected,
    // a Group button.
    egui::Area::new(ui.id().with("card_tools"))
        .order(egui::Order::Middle)
        .fixed_pos(egui::pos2(canvas_rect.left() + 8.0, canvas_rect.top() + 8.0))
        .show(ui.ctx(), |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(dock_mode, "Dock")
                    .on_hover_text("Dock mode: drag a card onto another to stick them together")
                    .clicked()
                {
                    actions.push(CanvasAction::ToggleDockMode);
                }
                if ui
                    .selectable_label(snap_mode, "Snap")
                    .on_hover_text("Snap mode: a dragged card's edges align to nearby cards")
                    .clicked()
                {
                    actions.push(CanvasAction::ToggleSnapMode);
                }
                if selection.len() >= 2
                    && ui
                        .button(format!("Group {} cards", selection.len()))
                        .on_hover_text("Wrap the selected cards in a container")
                        .clicked()
                {
                    actions.push(CanvasAction::GroupSelected);
                }
            });
        });

    // Hint line (screen space).
    ui.painter().text(
        canvas_rect.left_bottom() + egui::vec2(8.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        "double-click: text card · right-click: any card · drag title: move · ctrl+click: select · drag group header: move group · ctrl+scroll: zoom",
        egui::FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
    );

    actions
}

/// Apply a multiplicative zoom `factor` anchored at `screen_pt`, clamped so the
/// resulting scale stays within [`MIN_ZOOM`, `MAX_ZOOM`].
fn zoom_at(view: &mut TSTransform, canvas_rect: egui::Rect, screen_pt: egui::Pos2, factor: f32) {
    let target = (view.scaling * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let factor = target / view.scaling;
    if (factor - 1.0).abs() < f32::EPSILON {
        return;
    }
    let to_screen = TSTransform::from_translation(canvas_rect.min.to_vec2()) * *view;
    let anchor = (to_screen.inverse() * screen_pt).to_vec2();
    *view = *view
        * TSTransform::from_translation(anchor)
        * TSTransform::from_scaling(factor)
        * TSTransform::from_translation(-anchor);
}

/// Scale a child ui's fonts/spacing by `zoom` so card text zooms with the
/// canvas while still being drawn directly (which keeps text selection working).
fn scale_fonts(ui: &mut egui::Ui, zoom: f32) {
    if (zoom - 1.0).abs() < 1e-3 {
        return;
    }
    let mut style: egui::Style = (**ui.style()).clone();
    for (_, font) in style.text_styles.iter_mut() {
        font.size *= zoom;
    }
    let sp = &mut style.spacing;
    sp.item_spacing *= zoom;
    sp.button_padding *= zoom;
    sp.interact_size *= zoom;
    sp.icon_width *= zoom;
    sp.icon_width_inner *= zoom;
    sp.icon_spacing *= zoom;
    ui.set_style(style);
}

fn card_ui(
    ui: &mut egui::Ui,
    card: &Card,
    node_path: &str,
    to_screen: TSTransform,
    clip: egui::Rect,
    env: &mut Env,
    selected: bool,
    // `Some(all cards)` when snap mode is on: the dragged card's edges snap to
    // these. `None` = snapping off.
    snap_others: Option<&[Card]>,
    actions: &mut Vec<CanvasAction>,
) {
    let zoom = to_screen.scaling;
    // Draw the card directly at its zoomed screen rect. (An earlier version put
    // each card in a transformed layer, which broke text selection.)
    let rect = to_screen.mul_rect(egui::Rect::from_min_size(card.pos, card.size));
    if !clip.intersects(rect) {
        return;
    }
    let r = 6.0 * zoom;
    let title_h = TITLE_H * zoom;

    let accent = egui::Color32::from_rgb(card.color[0], card.color[1], card.color[2]);
    let p = ui.painter_at(clip);
    let panel = ui.visuals().panel_fill;
    let title_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), title_h));
    let bevel_c = 18.0 * zoom; // Futuristic corner-cut (bigger = more skewed)

    // Radiant glow behind the frame — concentric accent rings, brightest at the
    // edge, fading outward (the neon themes: Futuristic / SynthWave).
    if env.glow {
        let bevel = env.style == CardStyle::Futuristic;
        for i in (1..=5).rev() {
            let grow = i as f32 * 2.2 * zoom;
            let a = 0.05 + 0.035 * (5 - i) as f32; // inner rings brighter
            let col = accent.gamma_multiply(a);
            let stroke = egui::Stroke::new(2.2 * zoom, col);
            if bevel {
                p.add(egui::Shape::convex_polygon(
                    bevel_diag(rect.expand(grow), bevel_c + grow),
                    egui::Color32::TRANSPARENT,
                    stroke,
                ));
            } else {
                p.rect_stroke(rect.expand(grow), r + grow, stroke);
            }
        }
    }

    // The effective title-bar background (for picking a readable title color).
    let title_bg;
    // --- card frame, per theme ---------------------------------------------
    match env.style {
        CardStyle::Sticky => {
            // One solid paper color for the whole note — header and body the
            // same, like a real sticky. A default (uncolored) card is yellow;
            // recoloring paints the entire note.
            let paper = if card.color == DEFAULT_CARD_COLOR {
                egui::Color32::from_rgb(0xff, 0xe9, 0x6b)
            } else {
                accent
            };
            title_bg = paper;
            p.rect_filled(rect, r, paper);
            p.rect_stroke(rect, r, egui::Stroke::new(1.0, darken(paper, 0.72)));
            // A faint divider under the title keeps it readable without a
            // differently-colored header bar.
            p.line_segment(
                [
                    egui::pos2(rect.left() + 4.0 * zoom, title_rect.bottom()),
                    egui::pos2(rect.right() - 4.0 * zoom, title_rect.bottom()),
                ],
                egui::Stroke::new(1.0, darken(paper, 0.78)),
            );
        }
        CardStyle::Futuristic => {
            // Angular tech panel: sharp frame with the top-right and bottom-left
            // corners beveled, and a bright accent edge. A brighter diagonal on
            // the top-right cut plays up the skew (content stays axis-aligned).
            title_bg = mix(panel, accent, 0.30);
            p.add(egui::Shape::convex_polygon(
                bevel_diag(rect, bevel_c),
                panel,
                egui::Stroke::new(1.6, accent),
            ));
            p.add(egui::Shape::convex_polygon(
                bevel_title(title_rect, bevel_c),
                accent.gamma_multiply(0.30),
                egui::Stroke::NONE,
            ));
            p.line_segment(
                [
                    egui::pos2(rect.right() - bevel_c, rect.top()),
                    egui::pos2(rect.right(), rect.top() + bevel_c),
                ],
                egui::Stroke::new(2.2 * zoom, accent),
            );
        }
        CardStyle::Normal => {
            title_bg = mix(panel, accent, 0.35);
            p.rect_filled(rect, r, panel);
            p.rect_stroke(rect, r, egui::Stroke::new(1.0, accent));
            p.rect_filled(title_rect, r, accent.gamma_multiply(0.35));
        }
    }
    // Multi-select outline (Ctrl+click builds a selection to group).
    if selected {
        p.rect_stroke(
            rect.expand(2.5 * zoom),
            r + 2.0 * zoom,
            egui::Stroke::new((2.0 * zoom).max(1.5), egui::Color32::from_rgb(0xff, 0xd1, 0x66)),
        );
    }
    // Small marker on a docked card's title bar.
    if card.docked_to.is_some() {
        p.circle_filled(
            title_rect.right_center() - egui::vec2(74.0 * zoom, 0.0),
            2.5 * zoom,
            ui.visuals().strong_text_color(),
        );
    }

    // --- title bar: drag to move, double-click to toggle edit, menu on RMB ---
    let handle = ui.interact(
        title_rect,
        ui.id().with(("card_handle", card.id)),
        egui::Sense::click_and_drag(),
    );
    let cmd = ui.input(|i| i.modifiers.command);
    let grab_key = ui.id().with(("card_grab", card.id));
    if handle.drag_started() {
        actions.push(CanvasAction::RaiseCard(card.id));
        // Remember where on the card we grabbed (world units), so snapping can
        // track the pointer's intended position without drift.
        if let Some(pp) = handle.interact_pointer_pos() {
            let grab = (to_screen.inverse() * pp) - card.pos;
            ui.memory_mut(|m| m.data.insert_temp(grab_key, grab));
        }
    }
    if handle.clicked() {
        if cmd {
            // Ctrl/Cmd+click toggles the card in the group selection.
            actions.push(CanvasAction::ToggleSelect(card.id));
        } else {
            actions.push(CanvasAction::RaiseCard(card.id));
            actions.push(CanvasAction::ClearSelection);
        }
    }
    if handle.dragged() {
        let grab = ui.memory(|m| m.data.get_temp::<egui::Vec2>(grab_key));
        match (snap_others, handle.interact_pointer_pos(), grab) {
            (Some(others), Some(pp), Some(grab)) => {
                // Snap the pointer-intended top-left to nearby card edges.
                let intended = (to_screen.inverse() * pp) - grab;
                let (snapped, gx, gy) =
                    snap_position(intended, card.size, others, card.id, SNAP_DIST);
                actions.push(CanvasAction::MoveCard(card.id, snapped - card.pos));
                // Guide lines at the snapped edges.
                let guide = egui::Stroke::new(1.0, egui::Color32::from_rgb(0xff, 0xd1, 0x66));
                if let Some(x) = gx {
                    let sx = (to_screen * egui::pos2(x, 0.0)).x;
                    p.line_segment([egui::pos2(sx, clip.top()), egui::pos2(sx, clip.bottom())], guide);
                }
                if let Some(y) = gy {
                    let sy = (to_screen * egui::pos2(0.0, y)).y;
                    p.line_segment([egui::pos2(clip.left(), sy), egui::pos2(clip.right(), sy)], guide);
                }
            }
            _ => actions.push(CanvasAction::MoveCard(card.id, handle.drag_delta() / zoom)),
        }
    }
    if handle.double_clicked() && supports_edit(&card.kind) {
        actions.push(CanvasAction::SetEditing(card.id, !card.editing));
    }
    handle.context_menu(|ui| {
        card_menu(ui, card, node_path, env.templates, env.card_plugins, actions)
    });

    // Title label.
    let title_text = if card.title.is_empty() {
        card.kind.label().to_string()
    } else {
        card.title.clone()
    };
    p.text(
        title_rect.left_center() + egui::vec2(8.0 * zoom, 0.0),
        egui::Align2::LEFT_CENTER,
        title_text,
        egui::FontId::proportional(13.0 * zoom),
        title_text_color(ui.visuals(), title_bg),
    );

    // Edit/view toggle button on the right of the title bar (for text/code).
    if supports_edit(&card.kind) {
        let btn_rect = egui::Rect::from_min_size(
            egui::pos2(title_rect.right() - 46.0 * zoom, title_rect.top() + 2.0 * zoom),
            egui::vec2(42.0 * zoom, title_h - 4.0 * zoom),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect).layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        ));
        scale_fonts(&mut child, zoom);
        if card.source.is_some() {
            // A mirrored body is the file's, so there is nothing to edit here.
            // Say which file rather than just disabling the button — an unlabelled
            // missing control is a puzzle.
            let err = card.source_error.is_some();
            child.add(
                egui::Label::new(
                    egui::RichText::new(if err { "⚠ file" } else { "🔗 file" })
                        .small()
                        .color(if err {
                            egui::Color32::from_rgb(230, 160, 60)
                        } else {
                            child.visuals().weak_text_color()
                        }),
                )
                .selectable(false),
            )
            .on_hover_text(match &card.source_error {
                Some(e) => format!("Mirroring {}\n\n{e}", card.source.as_deref().unwrap_or("")),
                None => format!(
                    "Mirroring {}\nRead-only — edit the file instead.",
                    card.source.as_deref().unwrap_or("")
                ),
            });
        } else {
            let label = if card.editing { "view" } else { "edit" };
            if child
                .add(egui::Button::new(label).frame(false).small())
                .on_hover_text(if card.editing { "Preview" } else { "Edit" })
                .clicked()
            {
                actions.push(CanvasAction::SetEditing(card.id, !card.editing));
            }
        }
    }

    // Copy button (left of edit/view): card text to both clipboards.
    if let Some(text) = copyable_text(card) {
        let from_right = if supports_edit(&card.kind) { 66.0 } else { 24.0 };
        let btn_rect = egui::Rect::from_min_size(
            egui::pos2(
                title_rect.right() - from_right * zoom,
                title_rect.top() + 2.0 * zoom,
            ),
            egui::vec2(18.0 * zoom, title_h - 4.0 * zoom),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect).layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        ));
        scale_fonts(&mut child, zoom);
        if child
            .add(egui::Button::new("🗐").frame(false).small())
            .on_hover_text("Copy text (clipboard + primary selection)")
            .clicked()
        {
            copy_both(&child, &text);
        }
    }

    // --- body ---------------------------------------------------------------
    let pad = 6.0 * zoom;
    let body_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + pad, rect.min.y + title_h + 4.0 * zoom),
        rect.max - egui::vec2(pad, pad),
    );
    if body_rect.height() > 6.0 {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body_rect));
        child.set_clip_rect(body_rect.intersect(clip));
        scale_fonts(&mut child, zoom);
        egui::ScrollArea::vertical()
            .id_salt(("card_body", card.id))
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                body_ui(ui, card, env, zoom, actions);
            });
    }

    // --- resize handle (bottom-right) --------------------------------------
    let g = 14.0 * zoom;
    let grip = egui::Rect::from_min_size(rect.max - egui::vec2(g, g), egui::vec2(g, g));
    let grip_resp = ui.interact(grip, ui.id().with(("card_grip", card.id)), egui::Sense::drag());
    let gcol = if grip_resp.hovered() {
        accent
    } else {
        ui.visuals().weak_text_color()
    };
    for i in 1..=3 {
        let o = i as f32 * 3.5 * zoom;
        p.line_segment(
            [
                egui::pos2(rect.max.x - o, rect.max.y - 2.0 * zoom),
                egui::pos2(rect.max.x - 2.0 * zoom, rect.max.y - o),
            ],
            egui::Stroke::new(1.2, gcol),
        );
    }
    if grip_resp.dragged() {
        actions.push(CanvasAction::ResizeCard(card.id, grip_resp.drag_delta() / zoom));
    }
}

/// Rewrite a Text card's `![alt](trellis:N)` markers to `bytes://…` URIs,
/// registering each referenced image's bytes with egui once (tracked in `sent`).
/// egui's image loaders (installed at startup) decode and cache them, and the
/// CommonMark viewer then shows them inline via `Image::from_uri`. A marker whose
/// index has no image collapses to its alt text.
fn resolve_inline_images(
    ctx: &egui::Context,
    card: &Card,
    epoch: u64,
    sent: &mut std::collections::HashSet<String>,
) -> String {
    crate::model::map_inline_images(&card.body, |alt, n| match card.inline_images.get(n) {
        Some(e) => {
            let uri = format!("bytes://trellis-{}-{}-{}-{}", epoch, card.id, n, e.data.len());
            if sent.insert(uri.clone()) {
                ctx.include_bytes(uri.clone(), e.data.clone());
            }
            format!("![{alt}]({uri})")
        }
        None => alt.to_string(),
    })
}

fn body_ui(ui: &mut egui::Ui, card: &Card, env: &mut Env, zoom: f32, actions: &mut Vec<CanvasAction>) {
    ui.set_width(ui.available_width());
    match &card.kind {
        CardKind::Text => {
            // `source.is_none()`: a mirrored body belongs to the file, so the
            // editor must not open even if `editing` was set before the file was
            // attached — otherwise typing goes straight into the next refresh.
            if card.editing && card.source.is_none() {
                let edit_id = ui.make_persistent_id(("card_md_edit", card.id));

                let title_resp = title_field(ui, card, actions);
                // Tab from the title jumps straight to the body editor, so a card
                // can be filled out title-then-body without hitting the toolbar.
                let tab_to_body = title_resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Tab) && !i.modifiers.shift);

                // Last-known selection (from the previous frame) drives the
                // toolbar; default to the end of the text if nothing's selected.
                let sel = egui::widgets::text_edit::TextEditState::load(ui.ctx(), edit_id)
                    .and_then(|s| s.cursor.char_range())
                    .map(|r| {
                        let (p, s) = (r.primary.index, r.secondary.index);
                        (p.min(s), p.max(s))
                    })
                    .unwrap_or_else(|| {
                        let n = card.body.chars().count();
                        (n, n)
                    });

                let mut edited: Option<(String, CCursorRange)> = None;
                ui.horizontal_wrapped(|ui| {
                    if fmt_btn(ui, "B", "Bold") {
                        edited = Some(wrap_inline(&card.body, sel, "**"));
                    }
                    if fmt_btn(ui, "I", "Italic") {
                        edited = Some(wrap_inline(&card.body, sel, "*"));
                    }
                    if fmt_btn(ui, "S", "Strikethrough") {
                        edited = Some(wrap_inline(&card.body, sel, "~~"));
                    }
                    if fmt_btn(ui, "<>", "Inline code") {
                        edited = Some(wrap_inline(&card.body, sel, "`"));
                    }
                    ui.separator();
                    if fmt_btn(ui, "H1", "Heading 1") {
                        edited = Some(line_prefix(&card.body, sel, "# "));
                    }
                    if fmt_btn(ui, "H2", "Heading 2") {
                        edited = Some(line_prefix(&card.body, sel, "## "));
                    }
                    if fmt_btn(ui, "•", "Bullet list") {
                        edited = Some(line_prefix(&card.body, sel, "- "));
                    }
                    if fmt_btn(ui, "1.", "Numbered list") {
                        edited = Some(numbered_prefix(&card.body, sel));
                    }
                    if fmt_btn(ui, "\u{201C}\u{201D}", "Quote") {
                        edited = Some(line_prefix(&card.body, sel, "> "));
                    }
                    if fmt_btn(ui, "[ ]", "Task item") {
                        edited = Some(line_prefix(&card.body, sel, "- [ ] "));
                    }
                    ui.separator();
                    if fmt_btn(ui, "{ }", "Code block") {
                        edited = Some(wrap_block(&card.body, sel));
                    }
                    if fmt_btn(ui, "link", "Link") {
                        edited = Some(make_link(&card.body, sel));
                    }
                    if fmt_btn(ui, "img", "Insert image (embed a picture in this note)") {
                        actions.push(CanvasAction::InsertInlineImage(card.id, sel.1));
                    }
                    if fmt_btn(ui, "\u{2014}", "Horizontal rule") {
                        edited = Some(insert_hr(&card.body, sel));
                    }
                    ui.separator();
                    // Text color: pick a color, then apply it to the selection.
                    // Wraps the text in an inline HTML span, which renders colored
                    // in the HTML export. (The in-app CommonMark preview drops raw
                    // HTML, so the color only shows once exported.)
                    let ckey = egui::Id::new("trellis_text_color");
                    let mut rgb =
                        ui.data(|d| d.get_temp::<[u8; 3]>(ckey)).unwrap_or([0xef, 0x44, 0x44]);
                    if ui
                        .color_edit_button_srgb(&mut rgb)
                        .on_hover_text("Pick text color")
                        .changed()
                    {
                        ui.data_mut(|d| d.insert_temp(ckey, rgb));
                    }
                    let swatch = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                    if ui
                        .add(egui::Button::new(egui::RichText::new("A").color(swatch)).small())
                        .on_hover_text("Color selected text (shows in HTML export)")
                        .clicked()
                    {
                        edited = Some(wrap_color(&card.body, sel, rgb));
                    }
                    ui.separator();
                    font_scale_menu(ui, card, actions);
                });

                // Auto-continue Markdown lists: Enter on a list line inserts the
                // next marker; Enter on an empty item ends the list. Done before
                // the editor shows so we can swallow the newline it would insert.
                if edited.is_none()
                    && ui.memory(|m| m.has_focus(edit_id))
                    && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.any())
                {
                    if let Some(range) =
                        egui::widgets::text_edit::TextEditState::load(ui.ctx(), edit_id)
                            .and_then(|s| s.cursor.char_range())
                    {
                        if range.primary.index == range.secondary.index {
                            let at = range.primary.index;
                            let start = line_start(&card.body, at);
                            let line: String =
                                card.body.chars().skip(start).take(at - start).collect();
                            match list_enter(&line) {
                                Some(ListEnter::Continue(marker)) => {
                                    edited = Some(replace_range(&card.body, (at, at), &marker));
                                }
                                Some(ListEnter::Exit) => {
                                    edited = Some(replace_range(&card.body, (start, at), ""));
                                }
                                None => {}
                            }
                            if edited.is_some() {
                                ui.input_mut(|i| {
                                    i.events.retain(|e| {
                                        !matches!(
                                            e,
                                            egui::Event::Key {
                                                key: egui::Key::Enter,
                                                pressed: true,
                                                ..
                                            }
                                        )
                                    })
                                });
                            }
                        }
                    }
                }

                let mut body = card.body.clone();
                let out = egui::TextEdit::multiline(&mut body)
                    .id(edit_id)
                    .font(scaled_font(ui, egui::TextStyle::Body, card.font_scale))
                    .hint_text("Markdown… (select text, then a button wraps it)")
                    .desired_width(f32::INFINITY)
                    .desired_rows(6)
                    .show(ui);

                // Make the selection middle-click-pasteable elsewhere.
                mirror_selection_to_primary(ui, &out, &body);

                // Middle-click pastes the primary selection at the text cursor.
                if edited.is_none() && out.response.middle_clicked() {
                    if let Some(paste) = take_primary_selection() {
                        let at = out.state.cursor.char_range().map(sorted).unwrap_or_else(|| {
                            let n = card.body.chars().count();
                            (n, n)
                        });
                        edited = Some(replace_range(&card.body, at, &paste));
                    }
                }

                if let Some((text, range)) = edited {
                    // A toolbar op or paste ran: apply it and place the selection
                    // over the result. (The editor itself didn't change this frame.)
                    actions.push(CanvasAction::SetBody(card.id, text));
                    let mut state = out.state;
                    state.cursor.set_char_range(Some(range));
                    state.store(ui.ctx(), edit_id);
                    out.response.request_focus();
                } else if out.response.changed() {
                    actions.push(CanvasAction::SetBody(card.id, body));
                }

                if tab_to_body {
                    ui.memory_mut(|m| m.request_focus(edit_id));
                }
            } else if card.body.trim().is_empty() {
                ui.weak("(empty — double-click title to edit)");
            } else {
                // Resolve inline-image markers to registered `bytes://` URIs, then
                // render single newlines as line breaks (see hard_wrap).
                let resolved =
                    resolve_inline_images(ui.ctx(), card, env.inline_epoch, env.inline_sent);
                let linked = crate::model::wikilinks_to_md(&resolved);
                scale_text(ui, card.font_scale, |ui| {
                    CommonMarkViewer::new().show(ui, env.md, &crate::model::hard_wrap(&linked));
                });
            }
        }
        CardKind::Code { lang } => {
            if card.editing && card.source.is_none() {
                ui.horizontal(|ui| {
                    ui.label("lang:");
                    let lang_id = ui.make_persistent_id(("card_lang_edit", card.id));
                    let (l, l_changed, _) =
                        singleline_primary(ui, lang_id, lang, |te| te.desired_width(90.0));
                    if l_changed {
                        actions.push(CanvasAction::SetLang(card.id, l));
                    }
                    ui.separator();
                    font_scale_menu(ui, card, actions);
                });
                let code_id = ui.make_persistent_id(("card_code_edit", card.id));
                let mut body = card.body.clone();
                let out = egui::TextEdit::multiline(&mut body)
                    .id(code_id)
                    .font(scaled_font(ui, egui::TextStyle::Monospace, card.font_scale))
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(6)
                    .show(ui);
                mirror_selection_to_primary(ui, &out, &body);
                if out.response.middle_clicked() {
                    if let Some(paste) = take_primary_selection() {
                        let at = out.state.cursor.char_range().map(sorted).unwrap_or_else(|| {
                            let n = card.body.chars().count();
                            (n, n)
                        });
                        let (text, range) = replace_range(&card.body, at, &paste);
                        actions.push(CanvasAction::SetBody(card.id, text));
                        let mut state = out.state;
                        state.cursor.set_char_range(Some(range));
                        state.store(ui.ctx(), code_id);
                        out.response.request_focus();
                    }
                } else if out.response.changed() {
                    actions.push(CanvasAction::SetBody(card.id, body));
                }
            } else {
                let fenced = format!("```{}\n{}\n```", lang, card.body);
                scale_text(ui, card.font_scale, |ui| {
                    CommonMarkViewer::new().show(ui, env.md, &fenced);
                });
            }
        }
        CardKind::Checklist { items } => {
            let editing = card.editing;
            if editing {
                title_field(ui, card, actions);
            }
            for (i, item) in items.iter().enumerate() {
                ui.horizontal(|ui| {
                    // Reorder (drag grip) and delete (×) are structural edits, so
                    // they only appear in edit mode — otherwise a stray drag or
                    // click in view mode could move or delete an item.
                    if editing {
                        // Drag grip: reorder items by dragging this handle onto
                        // another row. Payload is (card, index) so drags stay
                        // within one card.
                        let egui::InnerResponse { response: drag, .. } = ui.dnd_drag_source(
                            ui.make_persistent_id(("cl_drag", card.id, i)),
                            (card.id, i),
                            |ui| {
                                ui.add(egui::Label::new("\u{2807}").sense(egui::Sense::drag()))
                                    .on_hover_text("Drag to reorder")
                            },
                        );
                        if let Some(payload) = drag.dnd_hover_payload::<(CardId, usize)>() {
                            if payload.0 == card.id {
                                let rect = drag.rect;
                                let before = ui
                                    .input(|inp| inp.pointer.hover_pos())
                                    .map_or(true, |p| p.y < rect.center().y);
                                let y = if before { rect.top() } else { rect.bottom() };
                                ui.painter().hline(
                                    rect.x_range(),
                                    y,
                                    egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                                );
                                if let Some(p) = drag.dnd_release_payload::<(CardId, usize)>() {
                                    let to = if before { i } else { i + 1 };
                                    actions.push(CanvasAction::ChecklistMove(card.id, p.1, to));
                                }
                            }
                        }
                    }
                    let mut done = item.done;
                    if ui.checkbox(&mut done, "").changed() {
                        actions.push(CanvasAction::ChecklistToggle(card.id, i));
                    }
                    if editing {
                        let item_id = ui.make_persistent_id(("card_check_edit", card.id, i));
                        // Leave room for the × delete button; an infinite-width
                        // field would push it outside the card and make it
                        // unclickable.
                        let text_w = (ui.available_width() - 26.0).max(24.0);
                        let (text, changed, _) =
                            singleline_primary(ui, item_id, &item.text, |te| {
                                te.desired_width(text_w).hint_text("item")
                            });
                        if changed {
                            actions.push(CanvasAction::ChecklistSetText(card.id, i, text));
                        }
                        if ui
                            .add(egui::Button::new("\u{00d7}").frame(false).small())
                            .on_hover_text("Delete item")
                            .clicked()
                        {
                            actions.push(CanvasAction::ChecklistRemove(card.id, i));
                        }
                    } else {
                        // View mode: read-only item text (wraps within the card).
                        ui.label(&item.text);
                    }
                });
            }
            if editing && ui.button("+ item").clicked() {
                actions.push(CanvasAction::ChecklistAdd(card.id));
            }
        }
        CardKind::Table { table } => {
            if card.editing {
                title_field(ui, card, actions);
            }
            // A chart is a *view* of the table, not a separate card kind: the
            // cells stay the data, so editing one redraws the chart.
            if let Some(spec) = table.chart.clone() {
                chart_ui(ui, card, table, &spec, zoom);
                if !spec.show_table {
                    return;
                }
                ui.add_space(4.0 * zoom);
            }
            egui::ScrollArea::horizontal()
                .id_salt(("table_h", card.id))
                .show(ui, |ui| {
                    table_ui(ui, card, table, zoom, actions);
                });
        }
        k @ CardKind::Image { .. } => {
            // Editing an image card just means naming it, so you can tell a few
            // apart. The images themselves always show.
            if card.editing {
                title_field(ui, card, actions);
            }
            let images = k.images();
            if images.is_empty() {
                if ui.button("Load image…").clicked() {
                    actions.push(CanvasAction::LoadImage(card.id));
                }
            } else {
                // Grid of images: chunked rows, each image fit to its cell
                // width. Double-click opens the full-screen viewer.
                let cols = grid_cols(images.len());
                let spacing = ui.spacing().item_spacing.x;
                let cell_w =
                    ((ui.available_width() - spacing * (cols as f32 - 1.0)) / cols as f32).max(32.0);
                for (row_i, row) in images.chunks(cols).enumerate() {
                    ui.horizontal(|ui| {
                        for (col_i, (data, name)) in row.iter().enumerate() {
                            let idx = row_i * cols + col_i;
                            match env.tex.get(ui.ctx(), card.id, idx, data) {
                                Some(tex) => {
                                    let img_size = tex.size_vec2();
                                    let scale = (cell_w / img_size.x).min(1.0);
                                    let src = egui::load::SizedTexture::from_handle(&tex);
                                    let resp = ui
                                        .add(
                                            egui::Image::from_texture(src)
                                                .fit_to_exact_size(img_size * scale)
                                                .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text(format!("{name} — double-click to view"));
                                    if resp.double_clicked() {
                                        actions.push(CanvasAction::OpenLightbox(card.id, idx));
                                    }
                                    resp.context_menu(|ui| {
                                        if ui.button("View").clicked() {
                                            actions.push(CanvasAction::OpenLightbox(card.id, idx));
                                            ui.close_menu();
                                        }
                                        if ui.button("Remove image").clicked() {
                                            actions.push(CanvasAction::RemoveImage(card.id, idx));
                                            ui.close_menu();
                                        }
                                    });
                                }
                                None => {
                                    let resp = ui.colored_label(
                                        egui::Color32::from_rgb(0xef, 0x44, 0x44),
                                        format!("unreadable: {name}"),
                                    );
                                    resp.context_menu(|ui| {
                                        if ui.button("Remove image").clicked() {
                                            actions.push(CanvasAction::RemoveImage(card.id, idx));
                                            ui.close_menu();
                                        }
                                    });
                                }
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if images.len() == 1 {
                        ui.weak(images[0].1);
                    } else {
                        ui.weak(format!("{} images", images.len()));
                    }
                    if ui.small_button("add image").clicked() {
                        actions.push(CanvasAction::LoadImage(card.id));
                    }
                });
            }
        }
        CardKind::Sketch { strokes } => {
            sketch_ui(ui, card, strokes, zoom, actions);
        }
    }
}

/// Freehand draw surface. Edit mode: a toolbar (color, brush size, undo, clear)
/// plus a canvas that captures drag gestures into strokes. View mode: the same
/// strokes, read-only. Points are stored in the card's local logical space
/// (zoom-independent); we map to/from screen with the drawing area origin + zoom.
fn sketch_ui(
    ui: &mut egui::Ui,
    card: &Card,
    strokes: &[crate::model::Stroke],
    zoom: f32,
    actions: &mut Vec<CanvasAction>,
) {
    let color_key = egui::Id::new(("sketch_color", card.id));
    let width_key = egui::Id::new(("sketch_width", card.id));
    let mut rgb = ui.data(|d| d.get_temp::<[u8; 3]>(color_key)).unwrap_or([0xef, 0x44, 0x44]);
    let mut width = ui.data(|d| d.get_temp::<f32>(width_key)).unwrap_or(3.0);

    if card.editing {
        ui.horizontal_wrapped(|ui| {
            if ui.color_edit_button_srgb(&mut rgb).on_hover_text("Brush color").changed() {
                ui.data_mut(|d| d.insert_temp(color_key, rgb));
            }
            ui.add(egui::Slider::new(&mut width, 1.0..=24.0).show_value(false))
                .on_hover_text("Brush size");
            ui.data_mut(|d| d.insert_temp(width_key, width));
            if ui.button("Undo stroke").clicked() {
                actions.push(CanvasAction::SketchUndo(card.id));
            }
            if ui.button("Clear").clicked() {
                actions.push(CanvasAction::SketchClear(card.id));
            }
        });
    }

    // Drawing surface fills the remaining space.
    let size = egui::vec2(ui.available_width(), ui.available_height().max(40.0));
    let sense = if card.editing { egui::Sense::drag() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0 * zoom, ui.visuals().extreme_bg_color);
    let origin = rect.min;
    let to_screen = |p: &[f32; 2]| origin + egui::vec2(p[0] * zoom, p[1] * zoom);
    let paint_stroke = |painter: &egui::Painter, color: [u8; 3], w: f32, pts: &[egui::Pos2]| {
        let col = egui::Color32::from_rgb(color[0], color[1], color[2]);
        match pts {
            [] => {}
            [p] => {
                painter.circle_filled(*p, (w * zoom * 0.5).max(0.5), col);
            }
            _ => {
                painter.add(egui::Shape::line(pts.to_vec(), egui::Stroke::new(w * zoom, col)));
            }
        }
    };

    // Committed strokes.
    for st in strokes {
        let pts: Vec<egui::Pos2> = st.points.iter().map(&to_screen).collect();
        paint_stroke(&painter, st.color, st.width, &pts);
    }

    // In-progress stroke (edit mode): accumulate local points across the drag.
    if card.editing {
        let buf_key = egui::Id::new(("sketch_buf", card.id));
        let mut buf: Vec<[f32; 2]> = ui.data(|d| d.get_temp(buf_key)).unwrap_or_default();
        if let Some(p) = resp.interact_pointer_pos() {
            if resp.dragged() || resp.drag_started() {
                let local = [(p.x - origin.x) / zoom, (p.y - origin.y) / zoom];
                // Skip near-duplicate points to keep strokes compact.
                if buf.last().map_or(true, |l| {
                    (l[0] - local[0]).abs() > 0.5 || (l[1] - local[1]).abs() > 0.5
                }) {
                    buf.push(local);
                }
            }
        }
        // Live preview of the current stroke.
        let preview: Vec<egui::Pos2> = buf.iter().map(&to_screen).collect();
        paint_stroke(&painter, rgb, width, &preview);
        if resp.drag_stopped() {
            if !buf.is_empty() {
                actions.push(CanvasAction::SketchAddStroke(
                    card.id,
                    crate::model::Stroke { color: rgb, width, points: std::mem::take(&mut buf) },
                ));
            }
            ui.data_mut(|d| d.remove::<Vec<[f32; 2]>>(buf_key));
        } else {
            ui.data_mut(|d| d.insert_temp(buf_key, buf));
        }
    }
}

/// Snap a card's would-be top-left `pos` to nearby card edges. Each axis snaps
/// independently to the closest edge (left/right, top/bottom) of another card
/// within `threshold`. Returns the adjusted position plus the world x/y of any
/// snapped edge (for guide lines). `self_id` is excluded from the candidates.
fn snap_position(
    pos: egui::Pos2,
    size: egui::Vec2,
    others: &[Card],
    self_id: CardId,
    threshold: f32,
) -> (egui::Pos2, Option<f32>, Option<f32>) {
    let (l, r) = (pos.x, pos.x + size.x);
    let (t, b) = (pos.y, pos.y + size.y);
    // (distance, adjust, guide-line world coord)
    let mut best_x: Option<(f32, f32, f32)> = None;
    let mut best_y: Option<(f32, f32, f32)> = None;
    for o in others {
        if o.id == self_id {
            continue;
        }
        let orect = egui::Rect::from_min_size(o.pos, o.size);
        for (mine, theirs) in
            [(l, orect.left()), (l, orect.right()), (r, orect.left()), (r, orect.right())]
        {
            let d = theirs - mine;
            if d.abs() <= threshold && best_x.map_or(true, |(bd, _, _)| d.abs() < bd) {
                best_x = Some((d.abs(), d, theirs));
            }
        }
        for (mine, theirs) in
            [(t, orect.top()), (t, orect.bottom()), (b, orect.top()), (b, orect.bottom())]
        {
            let d = theirs - mine;
            if d.abs() <= threshold && best_y.map_or(true, |(bd, _, _)| d.abs() < bd) {
                best_y = Some((d.abs(), d, theirs));
            }
        }
    }
    let dx = best_x.map_or(0.0, |(_, d, _)| d);
    let dy = best_y.map_or(0.0, |(_, d, _)| d);
    (egui::pos2(pos.x + dx, pos.y + dy), best_x.map(|(_, _, g)| g), best_y.map(|(_, _, g)| g))
}

/// Render the shared accent palette as a wrapped grid of swatch buttons.
/// Returns the picked color, or `None` if nothing was clicked this frame.
/// Shared by the card, group and (via `pub(crate)`) tree-node color menus.
pub(crate) fn swatch_grid(ui: &mut egui::Ui) -> Option<[u8; 3]> {
    let mut picked = None;
    ui.set_max_width(8.0 * 22.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for (name, col) in crate::model::SWATCHES {
            let color = egui::Color32::from_rgb(col[0], col[1], col[2]);
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 3.0, color);
            ui.painter()
                .rect_stroke(rect, 3.0, egui::Stroke::new(1.0, egui::Color32::from_gray(90)));
            if resp.on_hover_text(*name).clicked() {
                picked = Some(*col);
            }
        }
    });
    picked
}

/// Breadcrumb identifying a card: its node's path plus the card's title
/// (or `card #id` when it has no title), e.g. `HOUSE › ATTIC › Shopping list`.
fn card_path(card: &Card, node_path: &str) -> String {
    let label = if card.title.trim().is_empty() {
        format!("card #{}", card.id)
    } else {
        card.title.clone()
    };
    if node_path.is_empty() {
        label
    } else {
        format!("{node_path} › {label}")
    }
}

fn card_menu(
    ui: &mut egui::Ui,
    card: &Card,
    node_path: &str,
    templates: &[String],
    card_plugins: &[(usize, String)],
    actions: &mut Vec<CanvasAction>,
) {
    if supports_edit(&card.kind) && card.source.is_none() {
        let label = if card.editing { "Preview" } else { "Edit" };
        if ui.button(label).clicked() {
            actions.push(CanvasAction::SetEditing(card.id, !card.editing));
            ui.close_menu();
        }
    }
    if ui.button("Duplicate").clicked() {
        actions.push(CanvasAction::Duplicate(card.id));
        ui.close_menu();
    }
    if !matches!(card.kind, CardKind::Image { .. })
        && ui
            .button("Fit to content")
            .on_hover_text("Resize this card so its content is fully readable")
            .clicked()
    {
        actions.push(CanvasAction::FitCard(card.id));
        ui.close_menu();
    }
    if matches!(card.kind, CardKind::Text | CardKind::Code { .. }) {
        match card.source.is_some() {
            false => {
                if ui
                    .button("Mirror a file…")
                    .on_hover_text(
                        "Show a file's contents in this card, kept up to date while \
                         the document is open. The card becomes read-only — edit the \
                         file itself.",
                    )
                    .clicked()
                {
                    actions.push(CanvasAction::PickSource(card.id));
                    ui.close_menu();
                }
            }
            true => {
                if ui
                    .button("Stop mirroring")
                    .on_hover_text("Keep the text that's here and make the card editable again")
                    .clicked()
                {
                    actions.push(CanvasAction::ClearSource(card.id));
                    ui.close_menu();
                }
            }
        }
    }
    if ui
        .button("Save as template")
        .on_hover_text("Reuse this card later via right-click canvas → Insert template")
        .clicked()
    {
        actions.push(CanvasAction::SaveAsTemplate(card.id));
        ui.close_menu();
    }
    // Overwrite an existing template from this (edited) card — the "template
    // editor" flow: keep a master in a Templates node, tweak it, then update.
    if !templates.is_empty() {
        ui.menu_button("Update template", |ui| {
            ui.label("Replace which template with this card?");
            for (i, name) in templates.iter().enumerate() {
                let label = if name.trim().is_empty() { "(untitled)" } else { name.as_str() };
                if ui.button(label).clicked() {
                    actions.push(CanvasAction::UpdateTemplate(i, card.id));
                    ui.close_menu();
                }
            }
        });
    }
    if ui.button("Copy card").clicked() {
        actions.push(CanvasAction::CopyCard(card.id));
        ui.close_menu();
    }
    // Approved plugins that asked for the card menu. They receive the card's id
    // and its basket's, not its contents — a plugin reads what it needs over the
    // API under the scope it was approved for, so this trigger grants nothing
    // new.
    if !card_plugins.is_empty() {
        ui.separator();
        for (idx, title) in card_plugins {
            if ui.button(title).clicked() {
                actions.push(CanvasAction::RunCardPlugin(card.id, *idx));
                ui.close_menu();
            }
        }
    }
    if matches!(card.kind, CardKind::Image { .. }) {
        if ui
            .button("Extract text (OCR)")
            .on_hover_text("Read text from the image(s) with OCR so this card is searchable")
            .clicked()
        {
            actions.push(CanvasAction::OcrCard(card.id));
            ui.close_menu();
        }
    }
    // Download the image(s) to disk so they can be shared/re-used outside Trellis.
    if matches!(card.kind, CardKind::Image { .. }) {
        let imgs = card.kind.images();
        match imgs.len() {
            0 => {}
            1 => {
                if ui.button("Download image…").clicked() {
                    actions.push(CanvasAction::SaveImage(card.id, 0));
                    ui.close_menu();
                }
            }
            n => {
                ui.menu_button("Download", |ui| {
                    for (idx, (_, name)) in imgs.iter().enumerate() {
                        let label = if name.is_empty() {
                            format!("{}. image", idx + 1)
                        } else {
                            format!("{}. {}", idx + 1, name)
                        };
                        if ui.button(label).clicked() {
                            actions.push(CanvasAction::SaveImage(card.id, idx));
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button(format!("All {n} images…")).clicked() {
                        actions.push(CanvasAction::SaveAllImages(card.id));
                        ui.close_menu();
                    }
                });
            }
        }
    }
    // Copy the card's id or its breadcrumb path so you can point an agent at
    // this exact card (`/api/nodes/{node}/cards/{id}`).
    ui.menu_button("Copy", |ui| {
        if ui.button("Card id").clicked() {
            copy_both(ui, &card.id.to_string());
            ui.close_menu();
        }
        if ui.button("Card path").clicked() {
            copy_both(ui, &card_path(card, node_path));
            ui.close_menu();
        }
    });
    // Export just this card to a shareable file — no need to export the whole
    // workspace and crop. Common formats for every kind, plus kind-specific ones.
    ui.menu_button("Export Card", |ui| {
        if ui.button("PNG image").clicked() {
            actions.push(CanvasAction::ExportCardPng(card.id));
            ui.close_menu();
        }
        if ui.button("Markdown (.md)").clicked() {
            actions.push(CanvasAction::ExportCardMarkdown(card.id));
            ui.close_menu();
        }
        if ui.button("PDF").clicked() {
            actions.push(CanvasAction::ExportCardPdf(card.id));
            ui.close_menu();
        }
        if ui.button("HTML").clicked() {
            actions.push(CanvasAction::ExportCardHtml(card.id));
            ui.close_menu();
        }
        if ui.button("Plain text (.txt)").clicked() {
            actions.push(CanvasAction::ExportCardText(card.id));
            ui.close_menu();
        }
        if ui
            .button("JSON (card file)")
            .on_hover_text("A portable card file — import it into any Trellis workspace")
            .clicked()
        {
            actions.push(CanvasAction::ExportCardJson(card.id));
            ui.close_menu();
        }
        match &card.kind {
            CardKind::Table { .. } => {
                ui.separator();
                if ui.button("CSV").clicked() {
                    actions.push(CanvasAction::TableExportCsv(card.id));
                    ui.close_menu();
                }
                if ui.button("Excel (.xlsx)").clicked() {
                    actions.push(CanvasAction::TableExportXlsx(card.id));
                    ui.close_menu();
                }
            }
            CardKind::Sketch { .. } => {
                ui.separator();
                if ui.button("SVG (vector)").clicked() {
                    actions.push(CanvasAction::ExportCardSvg(card.id));
                    ui.close_menu();
                }
            }
            _ => {}
        }
    });
    ui.menu_button("Color", |ui| {
        if let Some(col) = swatch_grid(ui) {
            actions.push(CanvasAction::SetColor(card.id, col));
            ui.close_menu();
        }
    });
    if card.docked_to.is_some() && ui.button("Detach from dock").clicked() {
        actions.push(CanvasAction::DetachCard(card.id));
        ui.close_menu();
    }
    if let Some(g) = card.group {
        if ui.button("Ungroup").clicked() {
            actions.push(CanvasAction::Ungroup(g));
            ui.close_menu();
        }
    }
    ui.separator();
    if ui.button("Delete card").clicked() {
        actions.push(CanvasAction::Remove(card.id));
        ui.close_menu();
    }
}

/// Context menu for a group's header: rename, recolor, or ungroup.
fn group_menu(ui: &mut egui::Ui, group: &CardGroup, actions: &mut Vec<CanvasAction>) {
    ui.horizontal(|ui| {
        ui.label("Name:");
        let mut title = group.title.clone();
        if ui.text_edit_singleline(&mut title).changed() {
            actions.push(CanvasAction::SetGroupTitle(group.id, title));
        }
    });
    ui.menu_button("Color", |ui| {
        if let Some(col) = swatch_grid(ui) {
            actions.push(CanvasAction::SetGroupColor(group.id, col));
            ui.close_menu();
        }
    });
    ui.separator();
    if ui.button("Ungroup").clicked() {
        actions.push(CanvasAction::Ungroup(group.id));
        ui.close_menu();
    }
}

/// Render the card's title editor (a singleline field with primary-selection
/// support) and push a `SetTitle` action when it changes. Returns the field
/// response so callers can react to focus (e.g. Tab-to-body). Shared by text
/// and image cards.
fn title_field(ui: &mut egui::Ui, card: &Card, actions: &mut Vec<CanvasAction>) -> egui::Response {
    let title_id = ui.make_persistent_id(("card_title_edit", card.id));
    let (title, changed, resp) = singleline_primary(ui, title_id, &card.title, |te| {
        te.hint_text("card title").desired_width(f32::INFINITY)
    });
    if changed {
        actions.push(CanvasAction::SetTitle(card.id, title));
    }
    resp
}

/// Grid columns for an image card: single image full-width, up to four in two
/// columns, then three.
fn grid_cols(n: usize) -> usize {
    match n {
        0 | 1 => 1,
        2..=4 => 2,
        _ => 3,
    }
}

fn supports_edit(kind: &CardKind) -> bool {
    matches!(
        kind,
        CardKind::Text
            | CardKind::Code { .. }
            | CardKind::Image { .. }
            | CardKind::Table { .. }
            | CardKind::Checklist { .. }
            | CardKind::Sketch { .. }
    )
}

const TABLE_ROW_H: f32 = 24.0;
const TABLE_HANDLE_W: f32 = 20.0;

/// The spreadsheet card body. Edit mode shows a toolbar (rows/cols, colors,
/// import/export), row/column handles with insert/delete menus, draggable
/// column-resize grips, and a TextEdit per cell. View mode renders the same
/// grid read-only with cell colors.
/// Series colors, picked to stay distinguishable on both the light and dark
/// themes rather than following the card accent (which varies per card).
const SERIES_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0x4d, 0x9d, 0xe0), // blue
    egui::Color32::from_rgb(0xe1, 0x5f, 0x5f), // red
    egui::Color32::from_rgb(0x3b, 0xb2, 0x73), // green
    egui::Color32::from_rgb(0xe1, 0xa3, 0x3f), // amber
    egui::Color32::from_rgb(0x9b, 0x7e, 0xdb), // violet
    egui::Color32::from_rgb(0x37, 0xb5, 0xb5), // teal
    egui::Color32::from_rgb(0xd9, 0x6d, 0xb0), // pink
    egui::Color32::from_rgb(0x8a, 0x9a, 0xa8), // slate
];

/// Draw a table card as a **pie**: proportions of a whole.
///
/// egui_plot has no pie, so this is painted by hand. A slice wider than a
/// half-turn isn't convex, and egui fills assume convexity, so each slice is
/// filled as a fan of sub-wedges (≤ 60° each) and the outline is stroked
/// separately — otherwise a single dominant slice tessellates inside out.
///
/// Only the **first** series is used: a pie shows one set of parts. Non-positive
/// and missing values are skipped — a negative has no meaningful arc, and
/// silently folding it in as its absolute value would misstate every percentage.
fn pie_ui(
    ui: &mut egui::Ui,
    table: &crate::model::TableData,
    spec: &crate::model::ChartSpec,
    zoom: f32,
) {
    use std::f32::consts::TAU;

    let (labels, series) = table.chart_data(spec);
    let Some((series_name, vals)) = series.first() else {
        ui.small(
            egui::RichText::new("No numeric column to chart.")
                .color(ui.visuals().weak_text_color()),
        );
        return;
    };

    let slices: Vec<(String, f64)> = vals
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.filter(|v| *v > 0.0).map(|v| (i, v)))
        .map(|(i, v)| (labels.get(i).cloned().unwrap_or_default(), v))
        .collect();
    let total: f64 = slices.iter().map(|(_, v)| *v).sum();
    if slices.is_empty() || total <= 0.0 {
        ui.small(
            egui::RichText::new(format!(
                "\"{series_name}\" has no positive values to divide into a pie."
            ))
            .color(ui.visuals().weak_text_color()),
        );
        return;
    }

    let avail = ui.available_size();
    let h = if spec.show_table {
        (avail.y * 0.55).max(80.0 * zoom)
    } else {
        avail.y.max(80.0 * zoom)
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail.x, h), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    // Legend on the right when there's room for it, else all pie.
    let legend_w = if rect.width() > 260.0 * zoom {
        (rect.width() * 0.34).min(190.0 * zoom)
    } else {
        0.0
    };
    let pie_rect = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width() - legend_w, rect.height()),
    );
    let center = pie_rect.center();
    let radius = (pie_rect.width().min(pie_rect.height()) / 2.0 - 4.0 * zoom).max(6.0 * zoom);

    // Which slice is the pointer over? Cheap, and it makes small slices readable.
    let hovered = resp.hover_pos().and_then(|p| {
        let d = p - center;
        if d.length() > radius {
            return None;
        }
        // Angles run clockwise from 12 o'clock, matching the draw order.
        let mut a = (d.x).atan2(-d.y);
        if a < 0.0 {
            a += TAU;
        }
        let mut acc = 0.0;
        for (i, (_, v)) in slices.iter().enumerate() {
            let sweep = (*v / total) as f32 * TAU;
            if a >= acc && a < acc + sweep {
                return Some(i);
            }
            acc += sweep;
        }
        None
    });

    let mut start = 0.0f32;
    let mut bounds: Vec<f32> = Vec::with_capacity(slices.len() + 1);
    for (i, (label, v)) in slices.iter().enumerate() {
        let frac = (*v / total) as f32;
        let sweep = frac * TAU;
        let base = SERIES_COLORS[i % SERIES_COLORS.len()];
        let color = if hovered == Some(i) { mix(base, egui::Color32::WHITE, 0.25) } else { base };
        let r = if hovered == Some(i) { radius + 2.0 * zoom } else { radius };
        bounds.push(start);

        // One polygon per slice: egui fills a polygon as a fan from its first
        // vertex, and every point on the arc is visible from the centre, so this
        // tessellates correctly even past a half-turn. Splitting it into
        // sub-wedges instead leaves anti-aliased seams across the slice.
        let segs = ((sweep / 0.12).ceil() as usize).max(2);
        let mut pts = Vec::with_capacity(segs + 2);
        pts.push(center);
        for sgi in 0..=segs {
            let a = start + sweep * sgi as f32 / segs as f32;
            pts.push(center + egui::vec2(a.sin(), -a.cos()) * r);
        }
        painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));

        // Percentage inside the slice, when it's big enough to hold text.
        if frac >= 0.06 {
            let mid = start + sweep / 2.0;
            let at = center + egui::vec2(mid.sin(), -mid.cos()) * (r * 0.62);
            painter.text(
                at,
                egui::Align2::CENTER_CENTER,
                format!("{:.0}%", frac * 100.0),
                egui::FontId::proportional(11.0 * zoom),
                title_text_color(ui.visuals(), color),
            );
        }
        let _ = label;
        start += sweep;
    }
    bounds.push(start);

    // Separators + rim, drawn over the fills so no sub-wedge seams show.
    let edge = egui::Stroke::new(1.0 * zoom, ui.visuals().panel_fill);
    for a in &bounds {
        painter.line_segment(
            [center, center + egui::vec2(a.sin(), -a.cos()) * radius],
            edge,
        );
    }

    if legend_w > 0.0 {
        let x = pie_rect.right() + 8.0 * zoom;
        let line_h = 15.0 * zoom;
        let max_rows = ((rect.height() / line_h).floor() as usize).max(1);
        let mut y = rect.top() + 4.0 * zoom;
        for (i, (label, v)) in slices.iter().take(max_rows).enumerate() {
            let color = SERIES_COLORS[i % SERIES_COLORS.len()];
            let sw = 8.0 * zoom;
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(x, y + (line_h - sw) / 2.0),
                    egui::vec2(sw, sw),
                ),
                1.0 * zoom,
                color,
            );
            let pct = (*v / total) * 100.0;
            painter.text(
                egui::pos2(x + sw + 5.0 * zoom, y + line_h / 2.0),
                egui::Align2::LEFT_CENTER,
                format!("{label}  {pct:.0}%"),
                egui::FontId::proportional(11.0 * zoom),
                ui.visuals().text_color(),
            );
            y += line_h;
        }
        if slices.len() > max_rows {
            painter.text(
                egui::pos2(x, y + line_h / 2.0),
                egui::Align2::LEFT_CENTER,
                format!("+{} more", slices.len() - max_rows),
                egui::FontId::proportional(11.0 * zoom),
                ui.visuals().weak_text_color(),
            );
        }
    }

    // Exact value on hover — the percentage alone loses the underlying number,
    // and small slices have no room for a label at all.
    if let Some(i) = hovered {
        let (label, v) = &slices[i];
        resp.on_hover_text(format!(
            "{label}: {v} ({:.1}%)",
            (*v / total) * 100.0
        ));
    }
}

/// Draw a table card as a chart. **Every pixel dimension here is scaled by
/// `zoom`** — the canvas paints cards at their screen rect with no transform
/// layer, so anything left unscaled keeps its size while the card shrinks.
fn chart_ui(
    ui: &mut egui::Ui,
    card: &Card,
    table: &crate::model::TableData,
    spec: &crate::model::ChartSpec,
    zoom: f32,
) {
    use crate::model::ChartKind;
    use egui_plot::{Bar, BarChart, Legend, Line, Plot, Points, PlotPoints};

    // A pie is proportions of one series, not an x/y plot — its own painter.
    if spec.kind == ChartKind::Pie {
        pie_ui(ui, table, spec, zoom);
        return;
    }

    let (labels, series) = table.chart_data(spec);
    if labels.is_empty() || series.is_empty() {
        ui.small(
            egui::RichText::new("No numeric columns to chart — add numbers, or pick columns.")
                .color(ui.visuals().weak_text_color()),
        );
        return;
    }

    // Fill the card, leaving room for the grid underneath when it's shown.
    let avail = ui.available_size();
    let h = if spec.show_table { (avail.y * 0.55).max(80.0 * zoom) } else { avail.y.max(60.0 * zoom) };

    let label_for = {
        let labels = labels.clone();
        move |i: f64| -> String {
            let idx = i.round() as isize;
            if idx >= 0 && (idx as usize) < labels.len() {
                labels[idx as usize].clone()
            } else {
                String::new()
            }
        }
    };

    let mut plot = Plot::new(("chart", card.id))
        .height(h)
        .width(avail.x.max(60.0 * zoom))
        .allow_scroll(false) // the canvas owns scroll/zoom; a plot stealing it fights the card
        .allow_drag(false)
        .allow_zoom(false)
        .allow_boxed_zoom(false)
        .show_axes([true, true])
        .x_axis_formatter(move |m, _| label_for(m.value))
        .label_formatter(|name, v| {
            if name.is_empty() {
                format!("{:.4}", v.y)
            } else {
                format!("{name}: {:.4}", v.y)
            }
        });
    if series.len() > 1 {
        plot = plot.legend(Legend::default());
    }

    plot.show(ui, |plot_ui| {
        let n = series.len().max(1);
        for (si, (name, vals)) in series.iter().enumerate() {
            let color = SERIES_COLORS[si % SERIES_COLORS.len()];
            match spec.kind {
                ChartKind::Bar => {
                    // Group the series side by side within each label slot.
                    let w = 0.8 / n as f64;
                    let bars: Vec<Bar> = vals
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| v.map(|v| (i, v)))
                        .map(|(i, v)| {
                            let off = (si as f64 - (n as f64 - 1.0) / 2.0) * w;
                            Bar::new(i as f64 + off, v).width(w * 0.9)
                        })
                        .collect();
                    plot_ui.bar_chart(BarChart::new(bars).color(color).name(name));
                }
                ChartKind::Line => {
                    // Gaps split the line rather than being bridged, so a
                    // missing reading doesn't look like a measured one.
                    let mut run: Vec<[f64; 2]> = Vec::new();
                    let mut first = true;
                    // Flush a finished run: two or more samples draw a segment, a
                    // lone one draws a dot. A single reading between two gaps has
                    // no segment to belong to, and dropping it would hide real
                    // data — it must still show.
                    let flush = |plot_ui: &mut egui_plot::PlotUi,
                                     run: &mut Vec<[f64; 2]>,
                                     first: &mut bool| {
                        if run.len() > 1 {
                            let l = Line::new(PlotPoints::from(std::mem::take(run))).color(color);
                            plot_ui.line(if *first { l.name(name) } else { l });
                            *first = false;
                        } else if !run.is_empty() {
                            let p = Points::new(std::mem::take(run))
                                .color(color)
                                .radius(3.0 * zoom);
                            plot_ui.points(if *first { p.name(name) } else { p });
                            *first = false;
                        }
                    };
                    for (i, v) in vals.iter().enumerate() {
                        match v {
                            Some(v) => run.push([i as f64, *v]),
                            None => flush(plot_ui, &mut run, &mut first),
                        }
                    }
                    flush(plot_ui, &mut run, &mut first);
                }
                ChartKind::Scatter => {
                    let pts: Vec<[f64; 2]> = vals
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| v.map(|v| [i as f64, v]))
                        .collect();
                    plot_ui.points(
                        Points::new(pts).color(color).radius(3.5 * zoom).name(name),
                    );
                }
                // Handled by pie_ui before we ever build a Plot.
                ChartKind::Pie => {}
            }
        }
    });
}

fn table_ui(
    ui: &mut egui::Ui,
    card: &Card,
    table: &crate::model::TableData,
    zoom: f32,
    actions: &mut Vec<CanvasAction>,
) {
    let id = card.id;
    let cols = table.n_cols();
    let focus_key = ui.id().with(("table_focus", id));
    // Every pixel dimension below is multiplied by `zoom` so the grid scales
    // uniformly with the card (the cell *text* is already scaled by the body's
    // font scaler; the cell rects/handles/spacing were not, which left the grid
    // full-size inside a shrinking frame). `cw(c)` is a column's on-screen width.
    let row_h = TABLE_ROW_H * zoom;
    let handle_w = TABLE_HANDLE_W * zoom;
    let cw = |c: usize| table.col_width(c) * zoom;

    if card.editing {
        // --- toolbar ------------------------------------------------------
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("+ row").clicked() {
                actions.push(CanvasAction::TableInsertRow(id, table.rows.len()));
            }
            if ui.small_button("+ col").clicked() {
                actions.push(CanvasAction::TableInsertCol(id, cols));
            }
            let mut header = table.header;
            if ui.checkbox(&mut header, "header").changed() {
                actions.push(CanvasAction::TableToggleHeader(id));
            }
            ui.separator();
            // Chart: a view of this same table, so the cells stay the data.
            let cur = table.chart.clone();
            let cur_label = cur.as_ref().map_or("Chart: off", |c| match c.kind {
                crate::model::ChartKind::Bar => "Chart: bar",
                crate::model::ChartKind::Line => "Chart: line",
                crate::model::ChartKind::Scatter => "Chart: scatter",
                crate::model::ChartKind::Pie => "Chart: pie",
            });
            egui::ComboBox::from_id_salt(("chart_kind", id))
                .selected_text(cur_label)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(cur.is_none(), "Off (plain table)").clicked() {
                        actions.push(CanvasAction::TableSetChart(id, None));
                    }
                    for k in crate::model::ChartKind::ALL {
                        let on = cur.as_ref().is_some_and(|c| c.kind == k);
                        if ui.selectable_label(on, k.label()).clicked() {
                            let mut spec = cur.clone().unwrap_or_default();
                            spec.kind = k;
                            actions.push(CanvasAction::TableSetChart(id, Some(spec)));
                        }
                    }
                });
            if let Some(c) = &cur {
                let mut show = c.show_table;
                if ui
                    .checkbox(&mut show, "grid")
                    .on_hover_text("Show the source table under the chart")
                    .changed()
                {
                    let mut spec = c.clone();
                    spec.show_table = show;
                    actions.push(CanvasAction::TableSetChart(id, Some(spec)));
                }
            }
            ui.separator();
            if ui.small_button("Import…").on_hover_text("Load a CSV or XLSX file").clicked() {
                actions.push(CanvasAction::TableImport(id));
            }
            if ui.small_button("CSV…").on_hover_text("Export as CSV").clicked() {
                actions.push(CanvasAction::TableExportCsv(id));
            }
            if ui.small_button("XLSX…").on_hover_text("Export as Excel (keeps colors)").clicked() {
                actions.push(CanvasAction::TableExportXlsx(id));
            }
        });
        // Cell colors: pick, then apply to the focused cell.
        let focus = ui.data(|d| d.get_temp::<(usize, usize)>(focus_key));
        ui.horizontal(|ui| {
            let bkey = egui::Id::new("trellis_table_bg");
            let fkey = egui::Id::new("trellis_table_fg");
            let mut bg = ui.data(|d| d.get_temp::<[u8; 3]>(bkey)).unwrap_or([0xfd, 0xe6, 0x8a]);
            let mut fg = ui.data(|d| d.get_temp::<[u8; 3]>(fkey)).unwrap_or([0xef, 0x44, 0x44]);
            if ui.color_edit_button_srgb(&mut bg).on_hover_text("Cell background color").changed() {
                ui.data_mut(|d| d.insert_temp(bkey, bg));
            }
            if ui.small_button("fill").on_hover_text("Apply background to the selected cell").clicked() {
                if let Some((r, c)) = focus {
                    actions.push(CanvasAction::TableSetBg(id, r, c, Some(bg)));
                }
            }
            ui.separator();
            if ui.color_edit_button_srgb(&mut fg).on_hover_text("Cell font color").changed() {
                ui.data_mut(|d| d.insert_temp(fkey, fg));
            }
            if ui.small_button("A").on_hover_text("Apply font color to the selected cell").clicked() {
                if let Some((r, c)) = focus {
                    actions.push(CanvasAction::TableSetFg(id, r, c, Some(fg)));
                }
            }
            ui.separator();
            if ui.small_button("clear").on_hover_text("Remove colors from the selected cell").clicked() {
                if let Some((r, c)) = focus {
                    actions.push(CanvasAction::TableSetBg(id, r, c, None));
                    actions.push(CanvasAction::TableSetFg(id, r, c, None));
                }
            }
            match focus {
                Some((r, c)) => ui.weak(format!("cell {}{}", col_letter(c), r + 1)),
                None => ui.weak("click a cell first"),
            };
        });

        // --- column header strip (letters + resize grips) -----------------
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0 * zoom;
            ui.add_space(handle_w + 2.0 * zoom);
            for c in 0..cols {
                let w = cw(c);
                let btn = ui.add_sized(
                    [(w - 10.0 * zoom).max(20.0 * zoom), 16.0 * zoom],
                    egui::Button::new(egui::RichText::new(col_letter(c)).size(10.0 * zoom)).small(),
                );
                btn.context_menu(|ui| {
                    if ui.button("Insert column left").clicked() {
                        actions.push(CanvasAction::TableInsertCol(id, c));
                        ui.close_menu();
                    }
                    if ui.button("Insert column right").clicked() {
                        actions.push(CanvasAction::TableInsertCol(id, c + 1));
                        ui.close_menu();
                    }
                    if ui.button("Delete column").clicked() {
                        actions.push(CanvasAction::TableRemoveCol(id, c));
                        ui.close_menu();
                    }
                });
                // Resize grip.
                let (grip, gresp) =
                    ui.allocate_exact_size(egui::vec2(8.0 * zoom, 16.0 * zoom), egui::Sense::drag());
                let gcol = if gresp.hovered() || gresp.dragged() {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.painter().line_segment(
                    [grip.center_top() + egui::vec2(0.0, 2.0), grip.center_bottom() - egui::vec2(0.0, 2.0)],
                    egui::Stroke::new(2.0, gcol),
                );
                if gresp.dragged() && gresp.drag_delta().x != 0.0 {
                    // Drag is in screen px; store the world-space column width.
                    let world_w = table.col_width(c) + gresp.drag_delta().x / zoom;
                    actions.push(CanvasAction::TableSetColWidth(id, c, world_w));
                }
            }
        });
    }

    // --- the grid ---------------------------------------------------------
    let header_bg = ui.visuals().faint_bg_color;
    for (r, row) in table.rows.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0 * zoom;
            if card.editing {
                let rh = ui.add_sized(
                    [handle_w, row_h],
                    egui::Button::new(egui::RichText::new(format!("{}", r + 1)).size(10.0 * zoom)).small(),
                );
                rh.context_menu(|ui| {
                    if ui.button("Insert row above").clicked() {
                        actions.push(CanvasAction::TableInsertRow(id, r));
                        ui.close_menu();
                    }
                    if ui.button("Insert row below").clicked() {
                        actions.push(CanvasAction::TableInsertRow(id, r + 1));
                        ui.close_menu();
                    }
                    if ui.button("Delete row").clicked() {
                        actions.push(CanvasAction::TableRemoveRow(id, r));
                        ui.close_menu();
                    }
                });
            }
            for (c, cell) in row.iter().enumerate() {
                let w = cw(c);
                // The cell senses clicks when the card *isn't* being edited, so
                // right-click can offer to copy it. Sensing them in edit mode
                // instead would steal them from the text editor that sits here.
                let (rect, cell_resp) = ui.allocate_exact_size(
                    egui::vec2(w, row_h),
                    if card.editing { egui::Sense::hover() } else { egui::Sense::click() },
                );
                // Cell background: explicit color, else header shading, else a
                // faint outline so the grid reads as a grid.
                if let Some([rr, gg, bb]) = cell.bg {
                    ui.painter()
                        .rect_filled(rect, 2.0, egui::Color32::from_rgb(rr, gg, bb));
                } else if table.header && r == 0 {
                    ui.painter().rect_filled(rect, 2.0, header_bg);
                }
                ui.painter().rect_stroke(
                    rect,
                    2.0,
                    egui::Stroke::new(0.5, ui.visuals().weak_text_color().gamma_multiply(0.5)),
                );
                let fg = cell.fg.map(|[rr, gg, bb]| egui::Color32::from_rgb(rr, gg, bb));
                if card.editing {
                    let text = cell.text.clone();
                    // Wired for the X11 primary selection exactly like the body
                    // editor: selecting in a cell offers the text to other apps,
                    // and middle-click pastes it back. Cells used a bare
                    // `TextEdit` before, which is why select-and-middle-click —
                    // the ordinary way to move text around on X11 — did nothing
                    // to or from a table.
                    let cell_id = ui.make_persistent_id(("table_cell", id, r, c));
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let (new_text, changed, resp) =
                        singleline_primary(&mut child, cell_id, &text, |te| {
                            let te = te
                                .frame(false)
                                .margin(egui::vec2(4.0 * zoom, 3.0 * zoom))
                                .desired_width(w - 8.0 * zoom);
                            match fg {
                                Some(fg) => te.text_color(fg),
                                None => te,
                            }
                        });
                    if resp.has_focus() || resp.gained_focus() {
                        ui.data_mut(|d| d.insert_temp(focus_key, (r, c)));
                    }
                    if changed {
                        actions.push(CanvasAction::TableSetCell(id, r, c, new_text));
                    }
                } else {
                    let clipped = ui.painter_at(rect.shrink2(egui::vec2(4.0 * zoom, 0.0)));
                    let galley = ui.fonts(|f| {
                        f.layout_no_wrap(
                            cell.text.clone(),
                            egui::TextStyle::Body.resolve(ui.style()),
                            fg.unwrap_or_else(|| {
                                if table.header && r == 0 {
                                    ui.visuals().strong_text_color()
                                } else {
                                    ui.visuals().text_color()
                                }
                            }),
                        )
                    });
                    clipped.galley(
                        egui::pos2(
                            rect.left() + 4.0 * zoom,
                            rect.center().y - galley.size().y / 2.0,
                        ),
                        galley,
                        ui.visuals().text_color(),
                    );
                    // A card that isn't in edit mode paints its cells, so there
                    // is no text to select and no way to get a value out.
                    // Right-click gives one — cell, row or column, to both the
                    // clipboard and the primary selection, so it can be pasted
                    // anywhere by either means. Rows and columns go out as TSV,
                    // which is what a spreadsheet expects on paste.
                    cell_resp.context_menu(|ui| {
                        if ui.button("Copy cell").clicked() {
                            copy_both(ui, &cell.text);
                            ui.close_menu();
                        }
                        if ui.button("Copy row").clicked() {
                            let row: Vec<String> = table
                                .rows
                                .get(r)
                                .map(|row| row.iter().map(|c| c.text.clone()).collect())
                                .unwrap_or_default();
                            copy_both(ui, &row.join("\t"));
                            ui.close_menu();
                        }
                        if ui.button("Copy column").clicked() {
                            let col: Vec<String> = table
                                .rows
                                .iter()
                                .filter_map(|row| row.get(c).map(|c| c.text.clone()))
                                .collect();
                            copy_both(ui, &col.join("\n"));
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.label(
                            egui::RichText::new("Edit the card to select text in a cell")
                                .small()
                                .weak(),
                        );
                    });
                }
            }
        });
    }
}

/// Spreadsheet-style column label: A, B, …, Z, AA, AB, …
fn col_letter(mut c: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (c % 26) as u8) as char);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s
}

/// The card's plain-text content for the title-bar copy button, if it has any.
/// Checklists render as Markdown task lines; tables as CSV.
fn copyable_text(card: &Card) -> Option<String> {
    match &card.kind {
        CardKind::Text | CardKind::Code { .. } => Some(card.body.clone()),
        CardKind::Table { table } => Some(table.to_csv()),
        CardKind::Checklist { items } => Some(
            items
                .iter()
                .map(|it| format!("- [{}] {}", if it.done { 'x' } else { ' ' }, it.text))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        CardKind::Image { .. } | CardKind::Sketch { .. } => None,
    }
}

/// Copy `text` to both the system clipboard and the X11 PRIMARY selection.
pub(crate) fn copy_both(ui: &egui::Ui, text: &str) {
    ui.ctx().copy_text(text.to_string());
    // Drop the dedup key so the PRIMARY write happens even if we wrote this
    // same text before — another app may have overwritten PRIMARY since.
    ui.memory_mut(|m| m.data.remove::<String>(egui::Id::new("trellis_primary_sel")));
    set_primary_selection(ui, text);
}

// --- font size ---------------------------------------------------------------

/// A `FontId` from a base text style, scaled by `mult` (per-card font size).
fn scaled_font(ui: &egui::Ui, style: egui::TextStyle, mult: f32) -> egui::FontId {
    let mut f = style.resolve(ui.style());
    f.size *= mult;
    f
}

/// Run `body` with all of the ui's text styles scaled by `mult`, isolated to a
/// child scope so the rest of the canvas keeps the default sizes. Used to size
/// the rendered (view-mode) card text.
fn scale_text(ui: &mut egui::Ui, mult: f32, body: impl FnOnce(&mut egui::Ui)) {
    if (mult - 1.0).abs() < f32::EPSILON {
        body(ui);
        return;
    }
    ui.scope(|ui| {
        for f in ui.style_mut().text_styles.values_mut() {
            f.size *= mult;
        }
        body(ui);
    });
}

/// Toolbar control: pick the card's body font size (a multiplier). Presets keep
/// it simple; the label shows the current percentage.
fn font_scale_menu(ui: &mut egui::Ui, card: &Card, actions: &mut Vec<CanvasAction>) {
    let cur = card.font_scale;
    ui.menu_button(format!("A {:.0}%", cur * 100.0), |ui| {
        for (name, s) in
            [("75%", 0.75f32), ("90%", 0.9), ("100%", 1.0), ("125%", 1.25), ("150%", 1.5), ("200%", 2.0)]
        {
            if ui.selectable_label((cur - s).abs() < 0.001, name).clicked() {
                actions.push(CanvasAction::SetFontScale(card.id, s));
                ui.close_menu();
            }
        }
    })
    .response
    .on_hover_text("Body font size");
}

// --- Markdown formatting toolbar helpers ------------------------------------
//
// All operate on char indices (egui cursors are char-based) and return the new
// body text plus the selection to place over the formatted region.

fn fmt_btn(ui: &mut egui::Ui, label: &str, tip: &str) -> bool {
    ui.add(egui::Button::new(label).small())
        .on_hover_text(tip)
        .clicked()
}

fn ccrange(min: usize, max: usize) -> CCursorRange {
    CCursorRange::two(
        CCursor { index: min, prefer_next_row: false },
        CCursor { index: max, prefer_next_row: false },
    )
}

/// Byte offset of the `n`th char (or the string length if out of range).
fn byte_of(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(s.len())
}

/// Wrap the selection in a symmetric marker (`**`, `*`, `~~`, `` ` ``). With no
/// selection, inserts the markers and drops the cursor between them.
///
/// Leading/trailing whitespace in the selection is left *outside* the markers,
/// because Markdown emphasis needs the markers to hug the text: `**bold**`, not
/// `** bold **` (the latter renders as literal asterisks).
fn wrap_inline(text: &str, sel: (usize, usize), marker: &str) -> (String, CCursorRange) {
    let (mut a, mut b) = sel;
    let chars: Vec<char> = text.chars().collect();
    while a < b && chars.get(a).is_some_and(|c| c.is_whitespace()) {
        a += 1;
    }
    while b > a && chars.get(b - 1).is_some_and(|c| c.is_whitespace()) {
        b -= 1;
    }
    let (ba, bb) = (byte_of(text, a), byte_of(text, b));
    let ml = marker.chars().count();
    let mut out = String::with_capacity(text.len() + ml * 2);
    out.push_str(&text[..ba]);
    out.push_str(marker);
    out.push_str(&text[ba..bb]);
    out.push_str(marker);
    out.push_str(&text[bb..]);
    (out, ccrange(a + ml, b + ml))
}

/// Wrap the selection in an inline HTML color span (`<span style="color:#rrggbb">
/// …</span>`). Renders colored in the HTML export; the in-app CommonMark viewer
/// drops raw HTML, so the color only appears once exported. Whitespace is kept
/// outside the span, like [`wrap_inline`]. With no selection, inserts an empty
/// span and drops the cursor inside it.
fn wrap_color(text: &str, sel: (usize, usize), rgb: [u8; 3]) -> (String, CCursorRange) {
    let (mut a, mut b) = sel;
    let chars: Vec<char> = text.chars().collect();
    while a < b && chars.get(a).is_some_and(|c| c.is_whitespace()) {
        a += 1;
    }
    while b > a && chars.get(b - 1).is_some_and(|c| c.is_whitespace()) {
        b -= 1;
    }
    let open = format!("<span style=\"color:#{:02x}{:02x}{:02x}\">", rgb[0], rgb[1], rgb[2]);
    let close = "</span>";
    let (ba, bb) = (byte_of(text, a), byte_of(text, b));
    let mut out = String::with_capacity(text.len() + open.len() + close.len());
    out.push_str(&text[..ba]);
    out.push_str(&open);
    out.push_str(&text[ba..bb]);
    out.push_str(close);
    out.push_str(&text[bb..]);
    let ol = open.chars().count();
    (out, ccrange(a + ol, b + ol))
}

/// What pressing Enter on a Markdown list line should do.
enum ListEnter {
    /// Insert this text (a newline plus the next marker) to continue the list.
    Continue(String),
    /// The current item is empty — clear its marker and leave the list.
    Exit,
}

/// Char index of the start of the line containing char index `at`.
fn line_start(text: &str, at: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut i = at.min(chars.len());
    while i > 0 && chars[i - 1] != '\n' {
        i -= 1;
    }
    i
}

/// Given the current line up to the cursor, decide how Enter continues a list:
/// bullets (`-`/`*`/`+`), task items (`- [ ]`), and numbered (`1.`/`1)`), with
/// indentation preserved. An empty item ends the list. `None` = not a list line.
fn list_enter(line: &str) -> Option<ListEnter> {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent_len);
    // Task list items first (they start with a bullet too).
    for pfx in ["- [ ] ", "- [x] ", "- [X] "] {
        if let Some(after) = rest.strip_prefix(pfx) {
            return Some(if after.trim().is_empty() {
                ListEnter::Exit
            } else {
                ListEnter::Continue(format!("\n{indent}- [ ] "))
            });
        }
    }
    // Plain bullets.
    for m in ['-', '*', '+'] {
        let pfx = format!("{m} ");
        if let Some(after) = rest.strip_prefix(pfx.as_str()) {
            return Some(if after.trim().is_empty() {
                ListEnter::Exit
            } else {
                ListEnter::Continue(format!("\n{indent}{m} "))
            });
        }
    }
    // Numbered: digits then ". " or ") ".
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let after_num = &rest[digits.len()..];
        for sep in [". ", ") "] {
            if let Some(after) = after_num.strip_prefix(sep) {
                return Some(if after.trim().is_empty() {
                    ListEnter::Exit
                } else {
                    let n: u64 = digits.parse().unwrap_or(0);
                    let sep_ch = sep.chars().next().unwrap();
                    ListEnter::Continue(format!("\n{indent}{}{sep_ch} ", n + 1))
                });
            }
        }
    }
    None
}

/// Prepend `prefix` to every line the selection touches (headings, lists, quote).
fn line_prefix(text: &str, sel: (usize, usize), prefix: &str) -> (String, CCursorRange) {
    let chars: Vec<char> = text.chars().collect();
    let (a, b) = sel;
    // Start of the line containing `a`.
    let mut start = a.min(chars.len());
    while start > 0 && chars[start - 1] != '\n' {
        start -= 1;
    }
    let mut points = vec![start];
    let mut i = start;
    while i < b.min(chars.len()) {
        if chars[i] == '\n' {
            points.push(i + 1);
        }
        i += 1;
    }
    let pchars: Vec<char> = prefix.chars().collect();
    let pset: std::collections::HashSet<usize> = points.iter().copied().collect();
    let mut newv: Vec<char> = Vec::with_capacity(chars.len() + pchars.len() * points.len());
    for (idx, c) in chars.iter().enumerate() {
        if pset.contains(&idx) {
            newv.extend(pchars.iter().copied());
        }
        newv.push(*c);
    }
    if pset.contains(&chars.len()) {
        newv.extend(pchars.iter().copied());
    }
    let added = pchars.len() * points.len();
    (newv.into_iter().collect(), ccrange(a + pchars.len(), b + added))
}

/// Like [`line_prefix`] but numbers each touched line `1. `, `2. `, `3. `…
/// (the fixed-prefix version would make every line `1.`).
fn numbered_prefix(text: &str, sel: (usize, usize)) -> (String, CCursorRange) {
    let chars: Vec<char> = text.chars().collect();
    let (a, b) = sel;
    let mut start = a.min(chars.len());
    while start > 0 && chars[start - 1] != '\n' {
        start -= 1;
    }
    let mut points = vec![start];
    let mut i = start;
    while i < b.min(chars.len()) {
        if chars[i] == '\n' {
            points.push(i + 1);
        }
        i += 1;
    }
    // Each line start → its numbered marker.
    let markers: std::collections::HashMap<usize, Vec<char>> = points
        .iter()
        .enumerate()
        .map(|(k, &p)| (p, format!("{}. ", k + 1).chars().collect()))
        .collect();
    let mut newv: Vec<char> = Vec::with_capacity(chars.len() + points.len() * 3);
    let mut before_a = 0usize;
    let mut total = 0usize;
    for (idx, c) in chars.iter().enumerate() {
        if let Some(m) = markers.get(&idx) {
            newv.extend(m.iter().copied());
            total += m.len();
            if idx <= a {
                before_a += m.len();
            }
        }
        newv.push(*c);
    }
    if let Some(m) = markers.get(&chars.len()) {
        newv.extend(m.iter().copied());
        total += m.len();
        if chars.len() <= a {
            before_a += m.len();
        }
    }
    (newv.into_iter().collect(), ccrange(a + before_a, b + total))
}

/// Wrap the selection in a fenced ``` code block on its own lines.
fn wrap_block(text: &str, sel: (usize, usize)) -> (String, CCursorRange) {
    let (a, b) = sel;
    let (ba, bb) = (byte_of(text, a), byte_of(text, b));
    let inner = &text[ba..bb];
    let nl_before = ba > 0 && !text[..ba].ends_with('\n');
    let nl_after = bb < text.len() && !text[bb..].starts_with('\n');
    let mut out = String::new();
    out.push_str(&text[..ba]);
    if nl_before {
        out.push('\n');
    }
    out.push_str("```\n");
    out.push_str(inner);
    out.push_str("\n```");
    if nl_after {
        out.push('\n');
    }
    out.push_str(&text[bb..]);
    // Cursor after the opening fence line, spanning the inner text.
    let pos = a + if nl_before { 1 } else { 0 } + 4; // "```\n"
    (out, ccrange(pos, pos + inner.chars().count()))
}

/// Turn the selection into a `[label](url)` link, selecting the `url` placeholder.
fn make_link(text: &str, sel: (usize, usize)) -> (String, CCursorRange) {
    let (a, b) = sel;
    let (ba, bb) = (byte_of(text, a), byte_of(text, b));
    let label = &text[ba..bb];
    let label_len = label.chars().count();
    let mut out = String::new();
    out.push_str(&text[..ba]);
    out.push('[');
    out.push_str(label);
    out.push_str("](url)");
    out.push_str(&text[bb..]);
    let url_start = a + 1 + label_len + 2; // '[' + label + ']('
    (out, ccrange(url_start, url_start + 3))
}

/// Insert a `---` horizontal rule on its own line at the cursor.
fn insert_hr(text: &str, sel: (usize, usize)) -> (String, CCursorRange) {
    let a = sel.0;
    let ba = byte_of(text, a);
    let mut ins = String::new();
    if ba > 0 && !text[..ba].ends_with('\n') {
        ins.push('\n');
    }
    ins.push_str("---\n");
    let mut out = String::new();
    out.push_str(&text[..ba]);
    out.push_str(&ins);
    out.push_str(&text[ba..]);
    let pos = a + ins.chars().count();
    (out, ccrange(pos, pos))
}

/// (min, max) char indices of a selection range.
fn sorted(r: CCursorRange) -> (usize, usize) {
    let (p, s) = (r.primary.index, r.secondary.index);
    (p.min(s), p.max(s))
}

/// Replace the `[a, b)` char range with `insert`; the cursor lands after it.
fn replace_range(text: &str, sel: (usize, usize), insert: &str) -> (String, CCursorRange) {
    let (a, b) = sel;
    let (ba, bb) = (byte_of(text, a), byte_of(text, b));
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(&text[..ba]);
    out.push_str(insert);
    out.push_str(&text[bb..]);
    let pos = a + insert.chars().count();
    (out, ccrange(pos, pos))
}

/// Read the X11 PRIMARY selection (the middle-click paste source) via xclip or
/// xsel. arboard can't reliably serve/read the primary selection across apps.
#[cfg(target_os = "linux")]
fn take_primary_selection() -> Option<String> {
    for (cmd, args) in [
        ("xclip", &["-selection", "primary", "-o"][..]),
        ("xsel", &["--primary", "--output"][..]),
    ] {
        if let Ok(out) = std::process::Command::new(cmd).args(args).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).into_owned();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn take_primary_selection() -> Option<String> {
    None
}

/// The resident `xclip`/`xsel` currently owning PRIMARY on our behalf.
///
/// There must be **at most one**. These tools daemonize and stay alive to serve
/// the selection, so spawning one per change piles up resident processes that
/// fight each other for ownership — which breaks the clipboard for the whole
/// desktop, not just for Trellis.
#[cfg(target_os = "linux")]
static PRIMARY_OWNER: std::sync::Mutex<Option<std::process::Child>> = std::sync::Mutex::new(None);

/// Own the X11 PRIMARY selection with `text` via xclip/xsel (they daemonize to
/// serve it to other apps — arboard/egui can't).
///
/// **Kills the previous owner before spawning a new one.** Without that, every
/// change spawned another resident process: dragging across a card title made
/// one per character, and the survivors competed for selection ownership until
/// the clipboard stopped working system-wide. The "did the text change" guard
/// below is not enough on its own — a drag changes the text on every frame, so
/// it lets nearly all of them through.
///
/// Runs on a detached thread so neither the kill nor the write can stall the UI.
#[cfg(target_os = "linux")]
fn set_primary_selection(ui: &egui::Ui, text: &str) {
    let key = egui::Id::new("trellis_primary_sel");
    if ui.memory(|m| m.data.get_temp::<String>(key)).as_deref() == Some(text) {
        return;
    }
    ui.memory_mut(|m| m.data.insert_temp(key, text.to_string()));
    let text = text.to_string();
    std::thread::spawn(move || {
        use std::io::Write;
        use std::process::{Command, Stdio};
        // Retire the previous owner first. Held across the spawn so two rapid
        // selections can't both get past the check and leave one orphaned.
        let Ok(mut owner) = PRIMARY_OWNER.lock() else { return };
        if let Some(mut old) = owner.take() {
            let _ = old.kill();
            let _ = old.wait(); // reap it, or we trade daemons for zombies
        }
        for (cmd, args) in [
            ("xclip", &["-selection", "primary"][..]),
            ("xsel", &["--primary", "--input"][..]),
        ] {
            if let Ok(mut child) = Command::new(cmd)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(mut si) = child.stdin.take() {
                    let _ = si.write_all(text.as_bytes());
                }
                // Do NOT wait: the process stays alive to serve the selection.
                // Keep the handle so the next change can retire exactly this one.
                *owner = Some(child);
                return;
            }
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn set_primary_selection(_ui: &egui::Ui, _text: &str) {}

/// Copy the editor's current selection (if any) to the primary selection.
fn mirror_selection_to_primary(
    ui: &egui::Ui,
    out: &egui::widgets::text_edit::TextEditOutput,
    text: &str,
) {
    if let Some(range) = out.state.cursor.char_range() {
        let (a, b) = sorted(range);
        if a != b {
            let sel: String = text.chars().skip(a).take(b - a).collect();
            set_primary_selection(ui, &sel);
        }
    }
}

/// A singleline editor wired for the X11 primary selection like the body editor:
/// its selection mirrors to primary, and middle-click pastes primary at the
/// cursor. `build` customises the `TextEdit` (hint, width, …). Returns the
/// (possibly edited) text, whether it changed, and the response.
fn singleline_primary(
    ui: &mut egui::Ui,
    id: egui::Id,
    initial: &str,
    build: impl FnOnce(egui::TextEdit<'_>) -> egui::TextEdit<'_>,
) -> (String, bool, egui::Response) {
    let mut text = initial.to_string();
    let out = build(egui::TextEdit::singleline(&mut text).id(id)).show(ui);
    mirror_selection_to_primary(ui, &out, &text);
    let mut changed = out.response.changed();
    if out.response.middle_clicked() {
        if let Some(paste) = take_primary_selection() {
            let at = out.state.cursor.char_range().map(sorted).unwrap_or_else(|| {
                let n = text.chars().count();
                (n, n)
            });
            let (new_text, range) = replace_range(&text, at, &paste);
            text = new_text;
            let mut state = out.state.clone();
            state.cursor.set_char_range(Some(range));
            state.store(ui.ctx(), id);
            out.response.request_focus();
            changed = true;
        }
    }
    (text, changed, out.response)
}

/// Placement of the bottom-right minimap: its outer box, the inner drawing area,
/// and the world→minimap mapping (`inner.min + (world - world_min) * scale`).
struct MinimapGeom {
    outer: egui::Rect,
    inner: egui::Rect,
    world_min: egui::Pos2,
    scale: f32,
}

/// Compute the minimap box for the current basket: the world bounding box of all
/// cards, fit (preserving aspect) into a small box tucked into the canvas's
/// bottom-right corner. `None` if the basket is empty.
fn minimap_geometry(canvas_rect: egui::Rect, cards: &[Card]) -> Option<MinimapGeom> {
    if cards.is_empty() {
        return None;
    }
    let mut min = egui::pos2(f32::MAX, f32::MAX);
    let mut max = egui::pos2(f32::MIN, f32::MIN);
    for c in cards {
        min.x = min.x.min(c.pos.x);
        min.y = min.y.min(c.pos.y);
        max.x = max.x.max(c.pos.x + c.size.x);
        max.y = max.y.max(c.pos.y + c.size.y);
    }
    // Guard a degenerate span (a single tiny card) against divide-by-zero.
    let world_size = egui::vec2((max.x - min.x).max(1.0), (max.y - min.y).max(1.0));
    // Fit the content aspect into a max box, but never bigger than a fraction of
    // the canvas (so a tiny window still leaves room to work).
    let max_w = (canvas_rect.width() * 0.28).clamp(80.0, 200.0);
    let max_h = (canvas_rect.height() * 0.28).clamp(60.0, 150.0);
    let scale = (max_w / world_size.x).min(max_h / world_size.y);
    let inner_size = world_size * scale;
    let pad = 6.0;
    let margin = 12.0;
    let outer_size = inner_size + egui::vec2(pad * 2.0, pad * 2.0);
    let outer = egui::Rect::from_min_size(
        egui::pos2(
            canvas_rect.right() - margin - outer_size.x,
            canvas_rect.bottom() - margin - outer_size.y,
        ),
        outer_size,
    );
    let inner = egui::Rect::from_min_size(outer.min + egui::vec2(pad, pad), inner_size);
    Some(MinimapGeom { outer, inner, world_min: min, scale })
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect, view: TSTransform, color: egui::Color32) {
    let step = 32.0 * view.scaling;
    if step < 6.0 {
        return; // too dense to be useful when zoomed far out
    }
    let stroke = egui::Stroke::new(1.0, color.gamma_multiply(0.25));
    let mut x = rect.min.x + view.translation.x.rem_euclid(step);
    while x < rect.max.x {
        painter.line_segment([egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)], stroke);
        x += step;
    }
    let mut y = rect.min.y + view.translation.y.rem_euclid(step);
    while y < rect.max.y {
        painter.line_segment([egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)], stroke);
        y += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(r: &CCursorRange) -> (usize, usize) {
        (r.secondary.index, r.primary.index) // (min, max) as built by ccrange
    }

    #[test]
    fn copyable_text_covers_body_and_checklist_but_not_images() {
        use crate::model::ChecklistItem;
        let mut card = Card::new(1, egui::pos2(0.0, 0.0), CardKind::Text);
        card.body = "hello **world**".into();
        assert_eq!(copyable_text(&card).as_deref(), Some("hello **world**"));

        let items = vec![
            ChecklistItem { done: true, text: "done item".into() },
            ChecklistItem { done: false, text: "todo item".into() },
        ];
        let cl = Card::new(2, egui::pos2(0.0, 0.0), CardKind::Checklist { items });
        assert_eq!(
            copyable_text(&cl).as_deref(),
            Some("- [x] done item\n- [ ] todo item")
        );

        let img = Card::new(3, egui::pos2(0.0, 0.0), CardKind::Image {
            data: vec![],
            name: "pic".into(),
            extra: vec![],
            ocr: String::new(),
        });
        assert_eq!(copyable_text(&img), None);
    }

    #[test]
    fn card_path_uses_title_or_id_and_prepends_node_path() {
        let mut card = Card::new(7, egui::pos2(0.0, 0.0), CardKind::Text);
        card.title = "Shopping list".into();
        assert_eq!(card_path(&card, "HOUSE › KITCHEN"), "HOUSE › KITCHEN › Shopping list");
        // No title falls back to the card id.
        card.title = "   ".into();
        assert_eq!(card_path(&card, "HOUSE"), "HOUSE › card #7");
        // Empty node path yields just the label.
        card.title = "Notes".into();
        assert_eq!(card_path(&card, ""), "Notes");
    }

    #[test]
    fn table_copy_button_yields_csv_and_col_letters_extend() {
        use crate::model::{TableCell, TableData};
        let mut t = TableData::empty(2, 2);
        t.rows[0][0] = TableCell::new("a");
        t.rows[0][1] = TableCell::new("b,x");
        t.rows[1][0] = TableCell::new("c");
        let mut card = Card::new(1, egui::pos2(0.0, 0.0), CardKind::Table { table: t });
        card.editing = false;
        let csv = copyable_text(&card).unwrap();
        assert_eq!(csv.trim(), "a,\"b,x\"\nc,");

        assert_eq!(col_letter(0), "A");
        assert_eq!(col_letter(25), "Z");
        assert_eq!(col_letter(26), "AA");
        assert_eq!(col_letter(27), "AB");
    }

    #[test]
    fn grid_cols_scales_with_image_count() {
        assert_eq!(grid_cols(1), 1);
        assert_eq!(grid_cols(2), 2);
        assert_eq!(grid_cols(4), 2);
        assert_eq!(grid_cols(5), 3);
        assert_eq!(grid_cols(9), 3);
    }

    #[test]
    fn bold_wraps_selection_and_reselects_inner() {
        // "hello world", select "world" (chars 6..11).
        let (out, sel) = wrap_inline("hello world", (6, 11), "**");
        assert_eq!(out, "hello **world**");
        assert_eq!(range(&sel), (8, 13)); // selection still spans "world"
    }

    #[test]
    fn bold_keeps_markers_inside_surrounding_spaces() {
        // Selecting " added fuzzy search. " (with spaces) must bold the words,
        // not produce invalid "** added fuzzy search. **".
        let text = "x added fuzzy search. y";
        let (out, _) = wrap_inline(text, (1, 22), "**");
        assert_eq!(out, "x **added fuzzy search.** y");
    }

    #[test]
    fn bold_with_empty_selection_puts_cursor_between_markers() {
        let (out, sel) = wrap_inline("", (0, 0), "**");
        assert_eq!(out, "****");
        assert_eq!(range(&sel), (2, 2));
    }

    #[test]
    fn numbered_prefix_increments_each_line() {
        let text = "one\ntwo\nthree";
        let (out, _) = numbered_prefix(text, (0, text.chars().count()));
        assert_eq!(out, "1. one\n2. two\n3. three");
        // A single line just gets "1. ".
        let (one, _) = numbered_prefix("solo", (0, 4));
        assert_eq!(one, "1. solo");
    }

    #[test]
    fn list_enter_continues_and_exits() {
        // Numbered: next number, indentation kept.
        assert!(matches!(
            list_enter("1. first"),
            Some(ListEnter::Continue(s)) if s == "\n2. "
        ));
        assert!(matches!(
            list_enter("   3. nested"),
            Some(ListEnter::Continue(s)) if s == "\n   4. "
        ));
        // Bullets and tasks.
        assert!(matches!(list_enter("- item"), Some(ListEnter::Continue(s)) if s == "\n- "));
        assert!(matches!(
            list_enter("- [ ] todo"),
            Some(ListEnter::Continue(s)) if s == "\n- [ ] "
        ));
        assert!(matches!(
            list_enter("- [x] done"),
            Some(ListEnter::Continue(s)) if s == "\n- [ ] "
        ));
        // Empty item ends the list.
        assert!(matches!(list_enter("1. "), Some(ListEnter::Exit)));
        assert!(matches!(list_enter("- "), Some(ListEnter::Exit)));
        // Not a list.
        assert!(list_enter("just text").is_none());
        assert!(list_enter("").is_none());
    }

    #[test]
    fn snap_aligns_edge_to_edge_and_ignores_far_cards() {
        // Anchor at (100,100), default size 240x160 → right edge x=340.
        let anchor = Card::new(1, egui::pos2(100.0, 100.0), CardKind::Text);
        let others = [anchor];
        // Dragged card's left edge at 344 is 4px from the anchor's right (340) and
        // its top (100) already lines up → both axes snap.
        let (snapped, gx, gy) =
            snap_position(egui::pos2(344.0, 100.0), egui::vec2(240.0, 160.0), &others, 2, 8.0);
        assert_eq!(snapped, egui::pos2(340.0, 100.0));
        assert_eq!(gx, Some(340.0));
        assert_eq!(gy, Some(100.0));
        // Far away → no snap, position unchanged.
        let (far, fx, fy) =
            snap_position(egui::pos2(900.0, 900.0), egui::vec2(240.0, 160.0), &others, 2, 8.0);
        assert_eq!(far, egui::pos2(900.0, 900.0));
        assert!(fx.is_none() && fy.is_none());
    }

    #[test]
    fn color_wraps_selection_in_html_span_and_reselects_inner() {
        // "hello world", select "world" (chars 6..11), red.
        let (out, sel) = wrap_color("hello world", (6, 11), [0xef, 0x44, 0x44]);
        assert_eq!(out, "hello <span style=\"color:#ef4444\">world</span>");
        // Selection still spans "world": starts after the 30-char opening span.
        let ol = "<span style=\"color:#ef4444\">".chars().count();
        assert_eq!(range(&sel), (6 + ol, 11 + ol));
    }

    #[test]
    fn color_keeps_span_inside_surrounding_spaces() {
        let (out, _) = wrap_color("x hi y", (1, 5), [0x00, 0xff, 0x00]);
        assert_eq!(out, "x <span style=\"color:#00ff00\">hi</span> y");
    }

    #[test]
    fn inline_code_handles_multibyte_offsets() {
        // "café x" — 'é' is 2 bytes; select "x" (char index 5..6).
        let (out, _sel) = wrap_inline("café x", (5, 6), "`");
        assert_eq!(out, "café `x`");
    }

    #[test]
    fn heading_prefixes_single_line() {
        let (out, _) = line_prefix("title", (0, 0), "# ");
        assert_eq!(out, "# title");
    }

    #[test]
    fn bullet_prefixes_each_selected_line() {
        let (out, _) = line_prefix("a\nb\nc", (0, 5), "- ");
        assert_eq!(out, "- a\n- b\n- c");
    }

    #[test]
    fn code_block_wraps_on_own_lines() {
        let (out, _) = wrap_block("x", (0, 1));
        assert_eq!(out, "```\nx\n```");
    }

    #[test]
    fn link_selects_url_placeholder() {
        let (out, sel) = make_link("site", (0, 4));
        assert_eq!(out, "[site](url)");
        assert_eq!(range(&sel), (7, 10)); // "url"
    }
}
