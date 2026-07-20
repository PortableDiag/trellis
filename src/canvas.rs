//! Central basket canvas: a pannable surface of draggable, editable cards.

use crate::model::{CardId, Node};

/// Actions requested by the canvas, applied after rendering.
pub enum CanvasAction {
    AddCard(egui::Pos2),
    MoveCard(CardId, egui::Vec2),
    EditCard(CardId, String),
    RemoveCard(CardId),
}

pub fn ui(ui: &mut egui::Ui, node: &Node, pan: &mut egui::Vec2) -> Vec<CanvasAction> {
    let mut actions = Vec::new();

    let (canvas_rect, canvas_resp) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());

    // Background.
    let painter = ui.painter_at(canvas_rect);
    painter.rect_filled(canvas_rect, 0.0, ui.visuals().extreme_bg_color);
    draw_grid(&painter, canvas_rect, *pan);

    // Pan with a background drag (not on a card).
    if canvas_resp.dragged_by(egui::PointerButton::Primary) {
        *pan += canvas_resp.drag_delta();
    }

    // Double-click empty canvas → drop a new card at that spot.
    if canvas_resp.double_clicked() {
        if let Some(p) = canvas_resp.interact_pointer_pos() {
            let canvas_pos = p - canvas_rect.min.to_vec2() - *pan;
            actions.push(CanvasAction::AddCard(canvas_pos));
        }
    }

    let origin = canvas_rect.min.to_vec2() + *pan;
    for card in &node.cards {
        let screen_rect =
            egui::Rect::from_min_size(card.pos + origin, card.size);

        // Card body.
        let p = ui.painter_at(canvas_rect);
        let accent = egui::Color32::from_rgb(card.color[0], card.color[1], card.color[2]);
        p.rect_filled(screen_rect, 6.0, ui.visuals().panel_fill);
        p.rect_stroke(screen_rect, 6.0, egui::Stroke::new(1.0, accent));
        let title_rect = egui::Rect::from_min_size(
            screen_rect.min,
            egui::vec2(screen_rect.width(), 22.0),
        );
        p.rect_filled(title_rect, 6.0, accent.gamma_multiply(0.35));

        // Drag handle = title bar.
        let handle_id = ui.id().with(("card_handle", card.id));
        let handle = ui.interact(title_rect, handle_id, egui::Sense::click_and_drag());
        if handle.dragged() {
            actions.push(CanvasAction::MoveCard(card.id, handle.drag_delta()));
        }
        if handle.clicked_by(egui::PointerButton::Secondary) {
            actions.push(CanvasAction::RemoveCard(card.id));
        }

        // Editable text body.
        let body_rect = egui::Rect::from_min_max(
            egui::pos2(screen_rect.min.x + 6.0, screen_rect.min.y + 26.0),
            screen_rect.max - egui::vec2(6.0, 6.0),
        );
        let mut text = card.text.clone();
        let edit = ui.put(
            body_rect,
            egui::TextEdit::multiline(&mut text)
                .frame(false)
                .desired_width(body_rect.width()),
        );
        if edit.changed() {
            actions.push(CanvasAction::EditCard(card.id, text));
        }
    }

    ui.painter().text(
        canvas_rect.left_bottom() + egui::vec2(8.0, -8.0),
        egui::Align2::LEFT_BOTTOM,
        "double-click: new card   ·   drag title: move   ·   right-click title: delete   ·   drag empty: pan",
        egui::FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
    );

    actions
}

fn draw_grid(painter: &egui::Painter, rect: egui::Rect, pan: egui::Vec2) {
    let step = 32.0;
    let color = egui::Color32::from_gray(60).gamma_multiply(0.4);
    let stroke = egui::Stroke::new(1.0, color);

    let start_x = rect.min.x + pan.x.rem_euclid(step);
    let mut x = start_x;
    while x < rect.max.x {
        painter.line_segment([egui::pos2(x, rect.min.y), egui::pos2(x, rect.max.y)], stroke);
        x += step;
    }
    let start_y = rect.min.y + pan.y.rem_euclid(step);
    let mut y = start_y;
    while y < rect.max.y {
        painter.line_segment([egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y)], stroke);
        y += step;
    }
}
