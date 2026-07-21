//! Central basket canvas: a pannable surface of draggable, resizable, editable
//! cards. Each card renders according to its `CardKind`.

use crate::images::TextureCache;
use crate::model::{Card, CardId, CardKind, ChecklistItem, Node};
use egui::text::{CCursor, CCursorRange};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use emath::TSTransform;

/// Zoom bounds for the canvas view.
pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 3.0;

/// Shared, frame-persistent caches the canvas needs.
pub struct Env<'a> {
    pub md: &'a mut CommonMarkCache,
    pub tex: &'a mut TextureCache,
}

/// Actions requested by the canvas, applied by the app afterwards.
pub enum CanvasAction {
    AddCard(CardKind, egui::Pos2),
    MoveCard(CardId, egui::Vec2),
    ResizeCard(CardId, egui::Vec2),
    RaiseCard(CardId),
    SetTitle(CardId, String),
    SetBody(CardId, String),
    SetLang(CardId, String),
    SetColor(CardId, [u8; 3]),
    SetEditing(CardId, bool),
    Duplicate(CardId),
    CopyCard(CardId),
    PasteCard(egui::Pos2),
    Remove(CardId),
    ResetView,
    ChecklistToggle(CardId, usize),
    ChecklistSetText(CardId, usize, String),
    ChecklistAdd(CardId),
    ChecklistRemove(CardId, usize),
    LoadImage(CardId),
}

const TITLE_H: f32 = 24.0;

/// The canvas view: `view.translation` is the pan (screen px, relative to the
/// canvas top-left) and `view.scaling` is the zoom. Cards live in "world"
/// coordinates (`card.pos`); the layer transform below maps world → screen so
/// that only the cards zoom — the surrounding chrome never does.
pub fn ui(
    ui: &mut egui::Ui,
    node: &Node,
    view: &mut TSTransform,
    zoom_enabled: bool,
    can_paste: bool,
    env: &mut Env,
) -> Vec<CanvasAction> {
    let mut actions = Vec::new();

    let (canvas_rect, canvas_resp) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    ui.set_clip_rect(canvas_rect);

    // Background + grid.
    let painter = ui.painter_at(canvas_rect);
    painter.rect_filled(canvas_rect, 0.0, ui.visuals().extreme_bg_color);
    draw_grid(&painter, canvas_rect, *view, ui.visuals().weak_text_color());

    // Pan by dragging empty canvas (screen-space delta).
    if canvas_resp.dragged_by(egui::PointerButton::Primary) {
        view.translation += canvas_resp.drag_delta();
    }

    // Wheel over empty canvas pans; Ctrl+wheel (and pinch) zoom instead — egui
    // routes Ctrl+scroll into zoom_delta and out of smooth_scroll_delta.
    if canvas_resp.hovered() {
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

    // world → screen for this canvas.
    let to_screen = TSTransform::from_translation(canvas_rect.min.to_vec2()) * *view;

    // Double-click empty canvas → drop a text card at that world position.
    if canvas_resp.double_clicked() {
        if let Some(p) = canvas_resp.interact_pointer_pos() {
            actions.push(CanvasAction::AddCard(CardKind::Text, to_screen.inverse() * p));
        }
    }

    // Right-click empty canvas → choose a card kind to add.
    let menu_pos = canvas_resp.interact_pointer_pos();
    canvas_resp.context_menu(|ui| {
        ui.label("Add card");
        ui.separator();
        let cp = menu_pos
            .map(|p| to_screen.inverse() * p)
            .unwrap_or(egui::pos2(40.0, 40.0));
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
        if ui.button("Image").clicked() {
            actions.push(CanvasAction::AddCard(
                CardKind::Image { data: Vec::new(), name: String::new() },
                cp,
            ));
            ui.close_menu();
        }
        ui.separator();
        if ui
            .add_enabled(can_paste, egui::Button::new("Paste card"))
            .clicked()
        {
            actions.push(CanvasAction::PasteCard(cp));
            ui.close_menu();
        }
    });

    // Cards are drawn directly at their zoomed screen rects (see card_ui), which
    // keeps text selection/editing working (transformed layers broke it).
    for card in &node.cards {
        card_ui(ui, card, to_screen, canvas_rect, env, &mut actions);
    }

    // Reset-view button — in a foreground layer, untransformed, so it stays put
    // and clickable above the cards.
    let btn_pos = egui::pos2(canvas_rect.right() - 104.0, canvas_rect.top() + 8.0);
    egui::Area::new(ui.id().with("reset_view"))
        .order(egui::Order::Foreground)
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

    // Hint line (screen space).
    ui.painter().text(
        canvas_rect.left_bottom() + egui::vec2(8.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        "double-click: text card · right-click: any card · drag title: move · corner: resize · drag empty: pan · ctrl+scroll: zoom",
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
    to_screen: TSTransform,
    clip: egui::Rect,
    env: &mut Env,
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
    p.rect_filled(rect, r, ui.visuals().panel_fill);
    p.rect_stroke(rect, r, egui::Stroke::new(1.0, accent));

    let title_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), title_h));
    p.rect_filled(title_rect, r, accent.gamma_multiply(0.35));

    // --- title bar: drag to move, double-click to toggle edit, menu on RMB ---
    let handle = ui.interact(
        title_rect,
        ui.id().with(("card_handle", card.id)),
        egui::Sense::click_and_drag(),
    );
    if handle.drag_started() || handle.clicked() {
        actions.push(CanvasAction::RaiseCard(card.id));
    }
    if handle.dragged() {
        // Screen delta → world delta.
        actions.push(CanvasAction::MoveCard(card.id, handle.drag_delta() / zoom));
    }
    if handle.double_clicked() && supports_edit(&card.kind) {
        actions.push(CanvasAction::SetEditing(card.id, !card.editing));
    }
    handle.context_menu(|ui| card_menu(ui, card, actions));

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
        ui.visuals().strong_text_color(),
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
        let label = if card.editing { "view" } else { "edit" };
        if child
            .add(egui::Button::new(label).frame(false).small())
            .on_hover_text(if card.editing { "Preview" } else { "Edit" })
            .clicked()
        {
            actions.push(CanvasAction::SetEditing(card.id, !card.editing));
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
                body_ui(ui, card, env, actions);
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

fn body_ui(ui: &mut egui::Ui, card: &Card, env: &mut Env, actions: &mut Vec<CanvasAction>) {
    ui.set_width(ui.available_width());
    match &card.kind {
        CardKind::Text => {
            if card.editing {
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
                        edited = Some(line_prefix(&card.body, sel, "1. "));
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
                });

                let mut body = card.body.clone();
                let out = egui::TextEdit::multiline(&mut body)
                    .id(edit_id)
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
                // Render single newlines as line breaks (see hard_wrap).
                CommonMarkViewer::new().show(ui, env.md, &crate::model::hard_wrap(&card.body));
            }
        }
        CardKind::Code { lang } => {
            if card.editing {
                ui.horizontal(|ui| {
                    ui.label("lang:");
                    let lang_id = ui.make_persistent_id(("card_lang_edit", card.id));
                    let (l, l_changed, _) =
                        singleline_primary(ui, lang_id, lang, |te| te.desired_width(90.0));
                    if l_changed {
                        actions.push(CanvasAction::SetLang(card.id, l));
                    }
                });
                let code_id = ui.make_persistent_id(("card_code_edit", card.id));
                let mut body = card.body.clone();
                let out = egui::TextEdit::multiline(&mut body)
                    .id(code_id)
                    .font(egui::TextStyle::Monospace)
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
                CommonMarkViewer::new().show(ui, env.md, &fenced);
            }
        }
        CardKind::Checklist { items } => {
            for (i, item) in items.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut done = item.done;
                    if ui.checkbox(&mut done, "").changed() {
                        actions.push(CanvasAction::ChecklistToggle(card.id, i));
                    }
                    let item_id = ui.make_persistent_id(("card_check_edit", card.id, i));
                    let (text, changed, _) =
                        singleline_primary(ui, item_id, &item.text, |te| {
                            te.desired_width(f32::INFINITY).hint_text("item")
                        });
                    if changed {
                        actions.push(CanvasAction::ChecklistSetText(card.id, i, text));
                    }
                    if ui.add(egui::Button::new("×").frame(false).small()).clicked() {
                        actions.push(CanvasAction::ChecklistRemove(card.id, i));
                    }
                });
            }
            if ui.button("+ item").clicked() {
                actions.push(CanvasAction::ChecklistAdd(card.id));
            }
        }
        CardKind::Image { data, name } => {
            // Editing an image card just means naming it, so you can tell a few
            // apart. The image itself always shows.
            if card.editing {
                title_field(ui, card, actions);
            }
            if data.is_empty() {
                if ui.button("Load image…").clicked() {
                    actions.push(CanvasAction::LoadImage(card.id));
                }
            } else if let Some(tex) = env.tex.get(ui.ctx(), card.id, data) {
                let avail = ui.available_width().max(32.0);
                let img_size = tex.size_vec2();
                let scale = (avail / img_size.x).min(1.0);
                let src = egui::load::SizedTexture::from_handle(&tex);
                ui.add(egui::Image::from_texture(src).fit_to_exact_size(img_size * scale));
                ui.horizontal(|ui| {
                    ui.weak(name.as_str());
                    if ui.small_button("replace").clicked() {
                        actions.push(CanvasAction::LoadImage(card.id));
                    }
                });
            } else {
                ui.colored_label(egui::Color32::from_rgb(0xef, 0x44, 0x44), "unreadable image");
                if ui.small_button("load another").clicked() {
                    actions.push(CanvasAction::LoadImage(card.id));
                }
            }
        }
    }
}

fn card_menu(ui: &mut egui::Ui, card: &Card, actions: &mut Vec<CanvasAction>) {
    if supports_edit(&card.kind) {
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
    if ui.button("Copy card").clicked() {
        actions.push(CanvasAction::CopyCard(card.id));
        ui.close_menu();
    }
    ui.menu_button("Color", |ui| {
        let swatches: [(&str, [u8; 3]); 6] = [
            ("Blue", [0x3b, 0x82, 0xf6]),
            ("Green", [0x22, 0xc5, 0x5e]),
            ("Amber", [0xf5, 0x9e, 0x0b]),
            ("Red", [0xef, 0x44, 0x44]),
            ("Violet", [0x8b, 0x5c, 0xf6]),
            ("Slate", [0x64, 0x74, 0x8b]),
        ];
        for (name, col) in swatches {
            if ui.button(name).clicked() {
                actions.push(CanvasAction::SetColor(card.id, col));
                ui.close_menu();
            }
        }
    });
    ui.separator();
    if ui.button("Delete card").clicked() {
        actions.push(CanvasAction::Remove(card.id));
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

fn supports_edit(kind: &CardKind) -> bool {
    matches!(kind, CardKind::Text | CardKind::Code { .. } | CardKind::Image { .. })
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

/// Own the X11 PRIMARY selection with `text` via xclip/xsel (they daemonize to
/// serve it to other apps — arboard/egui can't). Only runs when the selection
/// changed, on a detached thread so writing can't stall the UI.
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
                let _ = child.wait(); // xclip/xsel fork to a daemon, so this returns
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
