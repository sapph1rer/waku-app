//! Reusable pieces: posters, backdrops with gradients, cards, buttons, tabs, sidebar items.

use crate::images::Images;
use crate::theme::*;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Painter, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, TextFormat, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;
use waku_core::AnimeCard;

pub const CARD_W: f32 = 168.0;
pub const POSTER_H: f32 = CARD_W * 1.5;
pub const CARD_H: f32 = POSTER_H + 58.0;

// ------------------------------------------------------------------ painting

/// UV rect that crops a `tex` image to fill `target` (CSS object-fit: cover).
/// `focus_y` 0 = keep the top, 0.5 = center.
pub fn cover_uv(tex: egui::Vec2, target: egui::Vec2, focus_y: f32) -> Rect {
    let tr = tex.x / tex.y.max(1.0);
    let r = target.x / target.y.max(1.0);
    if tr > r {
        let w = r / tr;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = tr / r;
        let y0 = (1.0 - h) * focus_y;
        Rect::from_min_max(pos2(0.0, y0), pos2(1.0, y0 + h))
    }
}

/// Linear gradient: horizontal (`vertical = false`, a→b left to right) or vertical (top to bottom).
pub fn gradient(p: &Painter, rect: Rect, a: Color32, b: Color32, vertical: bool) {
    let mut m = Mesh::default();
    let (tl, tr, bl, br) = if vertical { (a, a, b, b) } else { (a, b, a, b) };
    for (pos, c) in [
        (rect.left_top(), tl),
        (rect.right_top(), tr),
        (rect.left_bottom(), bl),
        (rect.right_bottom(), br),
    ] {
        m.vertices.push(Vertex {
            pos,
            uv: WHITE_UV,
            color: c,
        });
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(1, 3, 2);
    p.add(egui::Shape::mesh(m));
}

fn textured(
    p: &Painter,
    rect: Rect,
    radius: u8,
    tex: (egui::TextureId, egui::Vec2),
    tint: Color32,
    focus_y: f32,
) {
    p.add(egui::Shape::Rect(
        egui::epaint::RectShape::filled(rect, CornerRadius::same(radius), tint)
            .with_texture(tex.0, cover_uv(tex.1, rect.size(), focus_y)),
    ));
}

/// Image (cover-cropped) whose left `fade` fraction goes from transparent to opaque,
/// so it melts into whatever is underneath instead of leaving a seam.
fn faded_art(p: &Painter, rect: Rect, tex: (egui::TextureId, egui::Vec2), focus_y: f32, fade: f32) {
    let uv = cover_uv(tex.1, rect.size(), focus_y);
    let mut m = Mesh::with_texture(tex.0);
    let xs = [rect.min.x, rect.min.x + rect.width() * fade, rect.max.x];
    let us = [uv.min.x, uv.min.x + uv.width() * fade, uv.max.x];
    let alpha = [0u8, 255, 255];
    for i in 0..3 {
        let c = Color32::from_white_alpha(alpha[i]);
        m.vertices.push(Vertex {
            pos: pos2(xs[i], rect.min.y),
            uv: pos2(us[i], uv.min.y),
            color: c,
        });
        m.vertices.push(Vertex {
            pos: pos2(xs[i], rect.max.y),
            uv: pos2(us[i], uv.max.y),
            color: c,
        });
    }
    for i in 0..2u32 {
        let (a, b) = (i * 2, i * 2 + 2);
        m.add_triangle(a, b, a + 1);
        m.add_triangle(b, b + 1, a + 1);
    }
    p.add(egui::Shape::mesh(m));
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
        Some(t) => textured(p, rect, radius, t, Color32::WHITE, 0.5),
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
                regular(rect.height().min(rect.width()) * 0.2),
                DIM,
            );
        }
    }
}

/// Wide thumbnail cut from a poster (episode cards), darkened a little.
pub fn thumb(ui: &Ui, images: &mut Images, url: Option<&str>, referer: &str, rect: Rect) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(6), BAR);
    if let Some(t) = url.and_then(|u| images.get(u, referer).map(|t| (t.id(), t.size_vec2()))) {
        textured(p, rect, 6, t, Color32::from_gray(170), 0.22);
    }
}

/// Hero backdrop: blurred art that fades into the background on the left and bottom
/// (the readable side for title + buttons), sharp poster art on the right.
pub fn backdrop(ui: &Ui, images: &mut Images, url: Option<&str>, referer: &str, rect: Rect) {
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, CornerRadius::ZERO, BG);
    if let Some(u) = url {
        if let Some(t) = images.get_blur(u, referer).map(|t| (t.id(), t.size_vec2())) {
            textured(&p, rect, 0, t, Color32::from_gray(150), 0.3);
        }
        // sharp art block on the right
        let art_w = (rect.width() * 0.48).min(rect.height() * 1.6);
        let art = Rect::from_min_max(pos2(rect.max.x - art_w, rect.min.y), rect.max);
        if let Some(t) = images.get(u, referer).map(|t| (t.id(), t.size_vec2())) {
            faded_art(&p, art, t, 0.25, 0.5);
        }
    }
    let w = rect.width();
    // left: solid → transparent, so text sits on near-black
    gradient(
        &p,
        Rect::from_min_size(rect.min, vec2(w * 0.30, rect.height())),
        BG,
        BG,
        false,
    );
    gradient(
        &p,
        Rect::from_min_max(
            pos2(rect.min.x + w * 0.30, rect.min.y),
            pos2(rect.min.x + w * 0.62, rect.max.y),
        ),
        BG,
        Color32::TRANSPARENT,
        false,
    );
    // bottom fade into the page
    let h = rect.height();
    gradient(
        &p,
        Rect::from_min_max(pos2(rect.min.x, rect.max.y - h * 0.35), rect.max),
        Color32::TRANSPARENT,
        BG,
        true,
    );
    // a touch of darkening at the top
    gradient(
        &p,
        Rect::from_min_size(rect.min, vec2(w, h * 0.15)),
        Color32::from_black_alpha(140),
        Color32::TRANSPARENT,
        true,
    );
}

pub fn wrapped(
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

/// Text block limited to `rows` lines that takes layout space.
pub fn clamp_label(
    ui: &mut Ui,
    text: &str,
    font: egui::FontId,
    color: Color32,
    rows: usize,
) -> Response {
    let g = wrapped(ui, text, font, color, ui.available_width(), rows);
    let (rect, resp) = ui.allocate_exact_size(g.size(), Sense::hover());
    ui.painter().galley(rect.min, g, color);
    resp
}

// ------------------------------------------------------------------ cards

/// Poster card. `sub` is the gray line under the title.
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
        p.rect_filled(prect, CornerRadius::same(6), Color32::from_black_alpha(110));
        p.circle_filled(prect.center(), 26.0, ACCENT);
        p.text(
            prect.center() + vec2(2.0, 0.0),
            Align2::CENTER_CENTER,
            icon::PLAY,
            regular(24.0),
            Color32::WHITE,
        );
        p.rect_stroke(
            prect,
            CornerRadius::same(6),
            Stroke::new(2.0, ACCENT),
            StrokeKind::Inside,
        );
    }
    if let Some(b) = card.badge.as_deref().filter(|b| !b.is_empty()) {
        let g = wrapped(ui, b, bold(11.5), Color32::WHITE, CARD_W - 20.0, 1);
        let br = Rect::from_min_size(prect.min + vec2(6.0, 6.0), g.size() + vec2(10.0, 4.0));
        p.rect_filled(br, CornerRadius::same(3), ACCENT);
        p.galley(br.min + vec2(5.0, 2.0), g, Color32::WHITE);
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

pub struct EpCard<'a> {
    pub title: &'a str,
    pub meta: &'a str,
    pub desc: &'a str,
    /// 0..=1 watched progress bar under the thumbnail (None = none)
    pub progress: Option<f32>,
    pub selected: bool,
}

/// Episode row like the reference: thumbnail with a play button, title, meta, description.
/// Returns (play clicked, server-list clicked).
pub fn episode_card(
    ui: &mut Ui,
    images: &mut Images,
    art: Option<&str>,
    referer: &str,
    c: &EpCard,
    id: &str,
) -> (bool, bool) {
    let w = ui.available_width();
    let h = if c.desc.is_empty() { 96.0 } else { 112.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if !ui.is_rect_visible(rect) {
        return (false, false);
    }
    let hovered = resp.hovered();
    let fill = if c.selected || hovered { CARD_HI } else { CARD };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    if c.selected {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.5, ACCENT),
            StrokeKind::Inside,
        );
    }
    let pad = 12.0;
    let tw = (h - pad * 2.0) * 16.0 / 9.0;
    let tr = Rect::from_min_size(rect.min + vec2(pad, pad), vec2(tw, h - pad * 2.0));
    thumb(ui, images, art, referer, tr);
    let p = ui.painter();
    p.circle_filled(tr.center(), 17.0, Color32::from_black_alpha(150));
    p.circle_stroke(tr.center(), 17.0, Stroke::new(1.5, Color32::WHITE));
    p.text(
        tr.center() + vec2(1.5, 0.0),
        Align2::CENTER_CENTER,
        icon::PLAY,
        regular(15.0),
        if hovered { ACCENT_HI } else { Color32::WHITE },
    );
    if let Some(f) = c.progress {
        let bar = Rect::from_min_size(
            pos2(tr.min.x, tr.max.y - 4.0),
            vec2(tw * f.clamp(0.0, 1.0), 4.0),
        );
        p.rect_filled(bar, CornerRadius::same(2), ACCENT);
    }

    let x = tr.max.x + 16.0;
    let text_w = rect.max.x - x - 56.0;
    let tg = wrapped(ui, c.title, bold(16.0), TEXT, text_w, 1);
    let ty = tg.size().y;
    p.galley(pos2(x, rect.min.y + pad + 2.0), tg, TEXT);
    let mg = wrapped(ui, c.meta, regular(12.5), MUTED, text_w, 1);
    let my = mg.size().y;
    p.galley(pos2(x, rect.min.y + pad + 6.0 + ty), mg, MUTED);
    if !c.desc.is_empty() {
        let dg = wrapped(
            ui,
            c.desc,
            regular(13.0),
            Color32::from_gray(200),
            text_w,
            2,
        );
        p.galley(
            pos2(x, rect.min.y + pad + 14.0 + ty + my),
            dg,
            Color32::from_gray(200),
        );
    }

    let br = Rect::from_center_size(pos2(rect.max.x - 28.0, rect.center().y), vec2(34.0, 34.0));
    let b = ui
        .interact(br, ui.id().with(("srv", id)), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(crate::i18n::tr("Choose server", "เลือกเซิร์ฟเวอร์เอง"));
    if b.hovered() {
        ui.painter()
            .rect_filled(br, CornerRadius::same(6), PANEL_HI);
    }
    ui.painter().text(
        br.center(),
        Align2::CENTER_CENTER,
        icon::DOTS_THREE_VERTICAL,
        regular(20.0),
        if b.hovered() { TEXT } else { MUTED },
    );
    let srv = b.clicked();
    let play = !srv && resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
    (play, srv)
}

// ------------------------------------------------------------------ controls

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).font(bold(15.0)).color(Color32::WHITE))
        .fill(ACCENT)
        .corner_radius(CornerRadius::same(6))
        .min_size(vec2(0.0, 42.0));
    let r = ui.add(b);
    if r.hovered() {
        ui.painter()
            .rect_filled(r.rect, CornerRadius::same(6), Color32::from_white_alpha(20));
    }
    r.on_hover_cursor(CursorIcon::PointingHand)
}

/// Dark translucent button with a light outline ("+ My List").
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).font(bold(15.0)).color(Color32::WHITE))
        .fill(Color32::from_black_alpha(140))
        .stroke(Stroke::new(1.2, Color32::from_gray(200)))
        .corner_radius(CornerRadius::same(6))
        .min_size(vec2(0.0, 42.0));
    let r = ui.add(b);
    if r.hovered() {
        ui.painter()
            .rect_filled(r.rect, CornerRadius::same(6), Color32::from_white_alpha(14));
    }
    r.on_hover_cursor(CursorIcon::PointingHand)
}

pub fn outline_button(ui: &mut Ui, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).font(bold(14.5)).color(ACCENT))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.5, ACCENT))
        .corner_radius(CornerRadius::same(6))
        .min_size(vec2(0.0, 38.0));
    ui.add(b).on_hover_cursor(CursorIcon::PointingHand)
}

pub fn ghost_button(ui: &mut Ui, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).color(MUTED))
        .fill(Color32::TRANSPARENT)
        .frame(false);
    ui.add(b).on_hover_cursor(CursorIcon::PointingHand)
}

/// Pill used for source tabs. `dot` = optional status color.
pub fn chip(ui: &mut Ui, text: &str, selected: bool, dot: Option<Color32>) -> Response {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), regular(13.5), TEXT);
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
        p.circle_filled(
            pos2(x + 4.0, rect.center().y),
            4.0,
            if selected { Color32::WHITE } else { c },
        );
        x += dot_w;
    }
    p.galley(pos2(x, rect.min.y + pad.y), g, TEXT);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Small boxed badge ("Sub", "2025", "Series"); `hot` = accent-tinted.
pub fn badge(ui: &mut Ui, text: &str, hot: bool) {
    let (fill, stroke, col) = if hot {
        (
            Color32::from_rgb(0x4a, 0x1d, 0x12),
            Color32::from_rgb(0x9a, 0x3a, 0x22),
            Color32::from_rgb(0xff, 0xb3, 0x99),
        )
    } else {
        (
            Color32::from_rgba_unmultiplied(40, 41, 46, 220),
            LINE,
            Color32::from_gray(210),
        )
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.5).color(col));
        });
}

/// Text tabs with an accent underline. Returns the new index.
pub fn tabs(ui: &mut Ui, labels: &[String], current: usize) -> usize {
    let mut out = current;

    let start_x = ui.cursor().min.x;
    let resp = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 34.0;
        for (i, l) in labels.iter().enumerate() {
            let active = i == current;
            let r = ui.add(
                egui::Label::new(
                    RichText::new(l)
                        .font(if active { bold(16.0) } else { regular(16.0) })
                        .color(if active { TEXT } else { MUTED }),
                )
                .sense(Sense::click()),
            );
            if active {
                let rr = r.rect;
                ui.painter().rect_filled(
                    Rect::from_min_max(
                        pos2(rr.min.x - 10.0, rr.max.y + 9.0),
                        pos2(rr.max.x + 10.0, rr.max.y + 12.0),
                    ),
                    CornerRadius::same(1),
                    ACCENT,
                );
            }
            if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                out = i;
            }
        }
    });
    let line_y = resp.response.rect.max.y + 12.0;
    ui.painter().hline(
        start_x..=ui.max_rect().max.x,
        line_y,
        Stroke::new(1.0, LINE),
    );
    ui.add_space(14.0);
    out
}

/// One sidebar entry: icon above a small label, tinted when active.
pub fn sidebar_item(ui: &mut Ui, glyph: &str, label: &str, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(64.0, 58.0), Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, CornerRadius::same(10), SELECT);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(10), PANEL);
    }
    let col = if active {
        ACCENT_HI
    } else if resp.hovered() {
        TEXT
    } else {
        MUTED
    };
    p.text(
        rect.center() - vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        glyph,
        regular(21.0),
        col,
    );
    p.text(
        rect.center() + vec2(0.0, 15.0),
        Align2::CENTER_CENTER,
        label,
        regular(11.0),
        col,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
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
        .corner_radius(CornerRadius::same(6))
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
