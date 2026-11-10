//! Reusable pieces: poster cards, pill chips, buttons, section headers.

use crate::images::Images;
use crate::theme::*;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    TextFormat, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;
use waku_core::AnimeCard;

pub const CARD_W: f32 = 168.0;
pub const POSTER_H: f32 = CARD_W * 1.5;
pub const CARD_H: f32 = POSTER_H + 58.0;

/// UV rect that crops a `tex` image to fill `target` (like CSS object-fit: cover).
pub fn cover_uv(tex: egui::Vec2, target: egui::Vec2) -> Rect {
    let tr = tex.x / tex.y.max(1.0);
    let r = target.x / target.y.max(1.0);
    if tr > r {
        let w = r / tr;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = tr / r;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

/// Poster image (or placeholder) painted into `rect`.
pub fn poster(
    ui: &Ui,
    images: &mut Images,
    url: Option<&str>,
    referer: &str,
    rect: Rect,
    radius: u8,
) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(radius), PANEL);
    let tex = url.and_then(|u| images.get(u, referer).map(|t| (t.id(), t.size_vec2())));
    match tex {
        Some((id, size)) => {
            // rounded image: paint, then mask the corners with the background
            p.add(egui::Shape::Rect(
                egui::epaint::RectShape::filled(rect, CornerRadius::same(radius), Color32::WHITE)
                    .with_texture(id, cover_uv(size, rect.size())),
            ));
        }
        None => {
            let failed = url.is_none_or(|u| images.failed(u));
            let glyph = if failed {
                icon::TELEVISION
            } else {
                icon::SPINNER
            };
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                glyph,
                regular(34.0),
                DIM,
            );
        }
    }
}

fn wrapped(
    ui: &Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
    width: f32,
    rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::single_section(text.to_string(), TextFormat::simple(font, color));
    job.wrap = TextWrapping {
        max_width: width,
        max_rows: rows,
        break_anywhere: rows == 1,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

/// Crunchyroll-style poster card. `sub` is the gray line under the title.
pub fn anime_card(
    ui: &mut Ui,
    images: &mut Images,
    card: &AnimeCard,
    referer: &str,
    sub: &str,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(CARD_W, CARD_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let prect = Rect::from_min_size(rect.min, vec2(CARD_W, POSTER_H));
    poster(ui, images, card.poster.as_deref(), referer, prect, 6);
    let p = ui.painter();
    let hovered = resp.hovered();
    if hovered {
        p.rect_filled(prect, CornerRadius::same(6), Color32::from_black_alpha(120));
        p.circle_filled(prect.center(), 28.0, ACCENT);
        p.text(
            prect.center() + vec2(2.0, 0.0),
            Align2::CENTER_CENTER,
            icon::PLAY,
            regular(26.0),
            Color32::BLACK,
        );
        p.rect_stroke(
            prect,
            CornerRadius::same(6),
            Stroke::new(2.0, ACCENT),
            StrokeKind::Inside,
        );
    }
    if let Some(b) = card.badge.as_deref().filter(|b| !b.is_empty()) {
        let g = wrapped(ui, b, bold(11.5), Color32::BLACK, CARD_W - 20.0, 1);
        let br = Rect::from_min_size(prect.min + vec2(6.0, 6.0), g.size() + vec2(10.0, 4.0));
        p.rect_filled(br, CornerRadius::same(3), ACCENT);
        p.galley(br.min + vec2(5.0, 2.0), g, Color32::BLACK);
    }
    let title_col = if hovered { ACCENT_HI } else { TEXT };
    let tg = wrapped(ui, &card.title, regular(14.5), title_col, CARD_W, 2);
    let th = tg.size().y;
    p.galley(pos2(rect.min.x, prect.max.y + 6.0), tg, title_col);
    if !sub.is_empty() {
        let sg = wrapped(ui, sub, regular(12.0), MUTED, CARD_W, 1);
        p.galley(pos2(rect.min.x, prect.max.y + 8.0 + th), sg, MUTED);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(&card.title)
}

/// Wrap cards to the available width.
pub fn card_grid<T>(ui: &mut Ui, items: &[T], mut each: impl FnMut(&mut Ui, &T)) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = vec2(18.0, 22.0);
        ui.horizontal_wrapped(|ui| {
            for it in items {
                each(ui, it);
            }
        });
    });
}

/// Horizontally scrolling row of cards.
pub fn card_row<T>(ui: &mut Ui, id: &str, items: &[T], mut each: impl FnMut(&mut Ui, &T)) {
    egui::ScrollArea::horizontal()
        .id_salt(id)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 18.0;
                for it in items {
                    each(ui, it);
                }
            });
        });
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).font(bold(15.0)).color(Color32::BLACK))
        .fill(ACCENT)
        .corner_radius(CornerRadius::same(3))
        .min_size(vec2(0.0, 40.0));
    let r = ui.add(b);
    if r.hovered() {
        ui.painter()
            .rect_filled(r.rect, CornerRadius::same(3), Color32::from_white_alpha(18));
    }
    r.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn outline_button(ui: &mut Ui, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).font(bold(15.0)).color(ACCENT))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(2.0, ACCENT))
        .corner_radius(CornerRadius::same(3))
        .min_size(vec2(0.0, 40.0));
    ui.add(b).on_hover_cursor(CursorIcon::PointingHand)
}

pub fn ghost_button(ui: &mut Ui, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).color(MUTED))
        .fill(Color32::TRANSPARENT)
        .frame(false);
    ui.add(b).on_hover_cursor(CursorIcon::PointingHand)
}

/// Pill used for source tabs / tags. `dot` = optional status color.
pub fn chip(ui: &mut Ui, text: &str, selected: bool, dot: Option<Color32>) -> Response {
    let font = regular(13.5);
    let g = ui.painter().layout_no_wrap(
        text.to_string(),
        font,
        if selected { Color32::BLACK } else { TEXT },
    );
    let pad = vec2(14.0, 7.0);
    let dot_w = if dot.is_some() { 14.0 } else { 0.0 };
    let size = vec2(g.size().x + pad.x * 2.0 + dot_w, g.size().y + pad.y * 2.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if selected {
        ACCENT
    } else if resp.hovered() {
        PANEL_HI
    } else {
        PANEL
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(255), fill);
    let mut x = rect.min.x + pad.x;
    if let Some(c) = dot {
        p.circle_filled(pos2(x + 4.0, rect.center().y), 4.0, c);
        x += dot_w;
    }
    p.galley(pos2(x, rect.min.y + pad.y), g, TEXT);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Small static tag (genres etc.).
pub fn tag(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(PANEL)
        .corner_radius(CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.5).color(MUTED));
        });
}

pub fn section_title(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).font(bold(21.0)).color(TEXT));
    ui.add_space(4.0);
}

pub fn loading(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().color(ACCENT).size(18.0));
        ui.label(RichText::new(text).color(MUTED));
    });
}

pub fn error_box(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(0x2a, 0x16, 0x18))
        .stroke(Stroke::new(1.0, BAD))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("{}  {text}", icon::WARNING))
                    .color(Color32::from_rgb(0xff, 0xb4, 0xb4)),
            );
        });
}

pub fn empty(ui: &mut Ui, glyph: &str, text: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(glyph).size(48.0).color(DIM));
        ui.label(RichText::new(text).color(MUTED));
    });
}

pub fn health_color(ok: Option<bool>, ms: u128) -> Color32 {
    match ok {
        None => DIM,
        Some(false) => BAD,
        Some(true) if ms > 2500 => WARN,
        Some(true) => GOOD,
    }
}
