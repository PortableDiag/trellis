//! Central basket canvas: a pannable surface of draggable, resizable, editable
//! cards. Each card renders according to its `CardKind`.

use crate::images::TextureCache;
use crate::model::{Card, CardId, CardKind, ChecklistItem, Node};
use egui::text::{CCursor, CCursorRange};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

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
    Remove(CardId),
    ResetView,
    ChecklistToggle(CardId, usize),
    ChecklistSetText(CardId, usize, String),
    ChecklistAdd(CardId),
    ChecklistRemove(CardId, usize),
    LoadImage(CardId),
}

const TITLE_H: f32 = 24.0;

pub fn ui(ui: &mut egui::Ui, node: &Node, pan: &mut egui::Vec2, env: &mut Env) -> Vec<CanvasAction> {
    let mut actions = Vec::new();

    let (canvas_rect, canvas_resp) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    ui.set_clip_rect(canvas_rect);

    // Background + grid.
    let painter = ui.painter_at(canvas_rect);
    painter.rect_filled(canvas_rect, 0.0, ui.visuals().extreme_bg_color);
    draw_grid(&painter, canvas_rect, *pan, ui.visuals().weak_text_color());

    // Pan by dragging empty canvas.
    if canvas_resp.dragged_by(egui::PointerButton::Primary) {
        *pan += canvas_resp.drag_delta();
    }

    // Wheel scrolls the canvas when the pointer is over empty space; card
    // bodies keep their own inner scrolling when hovered directly.
    if canvas_resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        *pan += scroll;
    }

    // Double-click empty canvas → drop a text card there.
    if canvas_resp.double_clicked() {
        if let Some(p) = canvas_resp.interact_pointer_pos() {
            let cp = p - canvas_rect.min.to_vec2() - *pan;
            actions.push(CanvasAction::AddCard(CardKind::Text, cp));
        }
    }

    // Right-click empty canvas → choose a card kind to add.
    let menu_pos = canvas_resp.interact_pointer_pos();
    canvas_resp.context_menu(|ui| {
        ui.label("Add card");
        ui.separator();
        let cp = menu_pos
            .map(|p| p - canvas_rect.min.to_vec2() - *pan)
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
    });

    let origin = canvas_rect.min.to_vec2() + *pan;
    for card in &node.cards {
        card_ui(ui, card, origin, canvas_rect, env, &mut actions);
    }

    // Reset-view button (top-right) — snaps the pan back to the origin.
    let btn_rect = egui::Rect::from_min_size(
        egui::pos2(canvas_rect.right() - 104.0, canvas_rect.top() + 8.0),
        egui::vec2(96.0, 24.0),
    );
    let mut btn_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(btn_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    if btn_ui
        .button("Reset view")
        .on_hover_text("Recenter the canvas at the top-left")
        .clicked()
    {
        actions.push(CanvasAction::ResetView);
    }

    // Hint line.
    ui.painter().text(
        canvas_rect.left_bottom() + egui::vec2(8.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        "double-click: text card · right-click: any card · drag title: move · corner: resize · drag empty: pan",
        egui::FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
    );

    actions
}

fn card_ui(
    ui: &mut egui::Ui,
    card: &Card,
    origin: egui::Vec2,
    clip: egui::Rect,
    env: &mut Env,
    actions: &mut Vec<CanvasAction>,
) {
    let rect = egui::Rect::from_min_size(card.pos + origin, card.size);
    // Cull cards fully outside the viewport.
    if !clip.intersects(rect) {
        return;
    }

    let accent = egui::Color32::from_rgb(card.color[0], card.color[1], card.color[2]);
    let p = ui.painter_at(clip);
    p.rect_filled(rect, 6.0, ui.visuals().panel_fill);
    p.rect_stroke(rect, 6.0, egui::Stroke::new(1.0, accent));

    let title_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), TITLE_H));
    p.rect_filled(title_rect, 6.0, accent.gamma_multiply(0.35));

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
        actions.push(CanvasAction::MoveCard(card.id, handle.drag_delta()));
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
        title_rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title_text,
        egui::FontId::proportional(13.0),
        ui.visuals().strong_text_color(),
    );

    // Edit/view toggle button on the right of the title bar (for text/code).
    if supports_edit(&card.kind) {
        let btn_rect = egui::Rect::from_min_size(
            egui::pos2(title_rect.right() - 46.0, title_rect.top() + 2.0),
            egui::vec2(42.0, TITLE_H - 4.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect).layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        ));
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
    let body_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + 6.0, rect.min.y + TITLE_H + 4.0),
        rect.max - egui::vec2(6.0, 6.0),
    );
    if body_rect.height() > 6.0 {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body_rect));
        child.set_clip_rect(body_rect.intersect(clip));
        egui::ScrollArea::vertical()
            .id_salt(("card_body", card.id))
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                body_ui(ui, card, env, actions);
            });
    }

    // --- resize handle (bottom-right) --------------------------------------
    let grip = egui::Rect::from_min_size(rect.max - egui::vec2(14.0, 14.0), egui::vec2(14.0, 14.0));
    let grip_resp = ui.interact(
        grip,
        ui.id().with(("card_grip", card.id)),
        egui::Sense::drag(),
    );
    let gcol = if grip_resp.hovered() {
        accent
    } else {
        ui.visuals().weak_text_color()
    };
    for i in 1..=3 {
        let o = i as f32 * 3.5;
        p.line_segment(
            [
                egui::pos2(rect.max.x - o, rect.max.y - 2.0),
                egui::pos2(rect.max.x - 2.0, rect.max.y - o),
            ],
            egui::Stroke::new(1.2, gcol),
        );
    }
    if grip_resp.dragged() {
        actions.push(CanvasAction::ResizeCard(card.id, grip_resp.drag_delta()));
    }
}

fn body_ui(ui: &mut egui::Ui, card: &Card, env: &mut Env, actions: &mut Vec<CanvasAction>) {
    ui.set_width(ui.available_width());
    match &card.kind {
        CardKind::Text => {
            if card.editing {
                let edit_id = ui.make_persistent_id(("card_md_edit", card.id));

                let mut title = card.title.clone();
                let title_resp = ui.add(
                    egui::TextEdit::singleline(&mut title)
                        .hint_text("card title")
                        .desired_width(f32::INFINITY),
                );
                if title_resp.changed() {
                    actions.push(CanvasAction::SetTitle(card.id, title));
                }
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
                });

                let mut body = card.body.clone();
                let out = egui::TextEdit::multiline(&mut body)
                    .id(edit_id)
                    .hint_text("Markdown… (select text, then a button wraps it)")
                    .desired_width(f32::INFINITY)
                    .desired_rows(6)
                    .show(ui);

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
                CommonMarkViewer::new().show(ui, env.md, &card.body);
            }
        }
        CardKind::Code { lang } => {
            if card.editing {
                ui.horizontal(|ui| {
                    ui.label("lang:");
                    let mut l = lang.clone();
                    if ui.add(egui::TextEdit::singleline(&mut l).desired_width(90.0)).changed() {
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
                    let mut text = item.text.clone();
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(f32::INFINITY)
                            .hint_text("item"),
                    );
                    if resp.changed() {
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

fn supports_edit(kind: &CardKind) -> bool {
    matches!(kind, CardKind::Text | CardKind::Code { .. })
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
fn wrap_inline(text: &str, sel: (usize, usize), marker: &str) -> (String, CCursorRange) {
    let (a, b) = sel;
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

/// The X11/Wayland PRIMARY selection — the source for middle-click paste.
/// Empty or unavailable selections yield `None`.
#[cfg(target_os = "linux")]
fn take_primary_selection() -> Option<String> {
    use arboard::{Clipboard, GetExtLinux, LinuxClipboardKind};
    let mut cb = Clipboard::new().ok()?;
    let text = cb.get().clipboard(LinuxClipboardKind::Primary).text().ok()?;
    (!text.is_empty()).then_some(text)
}

#[cfg(not(target_os = "linux"))]
fn take_primary_selection() -> Option<String> {
    None
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect, pan: egui::Vec2, color: egui::Color32) {
    let step = 32.0;
    let stroke = egui::Stroke::new(1.0, color.gamma_multiply(0.25));
    let mut x = rect.min.x + pan.x.rem_euclid(step);
    while x < rect.max.x {
        painter.line_segment([egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)], stroke);
        x += step;
    }
    let mut y = rect.min.y + pan.y.rem_euclid(step);
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
    fn bold_with_empty_selection_puts_cursor_between_markers() {
        let (out, sel) = wrap_inline("", (0, 0), "**");
        assert_eq!(out, "****");
        assert_eq!(range(&sel), (2, 2));
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
