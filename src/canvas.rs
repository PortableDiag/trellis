//! Central basket canvas: a pannable surface of draggable, resizable, editable
//! cards. Each card renders according to its `CardKind`.

use crate::images::TextureCache;
use crate::model::{Card, CardId, CardKind, ChecklistItem, Node};
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
        if ui.button("📝  Text").clicked() {
            actions.push(CanvasAction::AddCard(CardKind::Text, cp));
            ui.close_menu();
        }
        if ui.button("💻  Code").clicked() {
            actions.push(CanvasAction::AddCard(CardKind::Code { lang: "rust".into() }, cp));
            ui.close_menu();
        }
        if ui.button("☑  Checklist").clicked() {
            actions.push(CanvasAction::AddCard(
                CardKind::Checklist {
                    items: vec![ChecklistItem { done: false, text: String::new() }],
                },
                cp,
            ));
            ui.close_menu();
        }
        if ui.button("🖼  Image").clicked() {
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
            egui::pos2(title_rect.right() - 26.0, title_rect.top() + 2.0),
            egui::vec2(22.0, TITLE_H - 4.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect).layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        ));
        let glyph = if card.editing { "👁" } else { "✏" };
        if child
            .add(egui::Button::new(glyph).frame(false).small())
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
                let mut body = card.body.clone();
                let mut title = card.title.clone();
                if ui
                    .add(egui::TextEdit::singleline(&mut title).hint_text("card title").desired_width(f32::INFINITY))
                    .changed()
                {
                    actions.push(CanvasAction::SetTitle(card.id, title));
                }
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut body)
                            .hint_text("Markdown…")
                            .desired_width(f32::INFINITY)
                            .desired_rows(6),
                    )
                    .changed()
                {
                    actions.push(CanvasAction::SetBody(card.id, body));
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
                let mut body = card.body.clone();
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut body)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(6),
                    )
                    .changed()
                {
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
                    if ui.add(egui::Button::new("✕").frame(false).small()).clicked() {
                        actions.push(CanvasAction::ChecklistRemove(card.id, i));
                    }
                });
            }
            if ui.button("＋ item").clicked() {
                actions.push(CanvasAction::ChecklistAdd(card.id));
            }
        }
        CardKind::Image { data, name } => {
            if data.is_empty() {
                if ui.button("🖼  Load image…").clicked() {
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
        let (glyph, label) = if card.editing { ("👁", "Preview") } else { ("✏", "Edit") };
        if ui.button(format!("{glyph}  {label}")).clicked() {
            actions.push(CanvasAction::SetEditing(card.id, !card.editing));
            ui.close_menu();
        }
    }
    if ui.button("⧉  Duplicate").clicked() {
        actions.push(CanvasAction::Duplicate(card.id));
        ui.close_menu();
    }
    ui.menu_button("🎨  Colour", |ui| {
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
    if ui.button("🗑  Delete card").clicked() {
        actions.push(CanvasAction::Remove(card.id));
        ui.close_menu();
    }
}

fn supports_edit(kind: &CardKind) -> bool {
    matches!(kind, CardKind::Text | CardKind::Code { .. })
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
