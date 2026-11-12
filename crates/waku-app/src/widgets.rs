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

/// Poster image painted into `rect`: a shimmer while it loads, then the picture fades in.
pub fn poster(
    ui: &Ui,
    images: &mut Images,
    url: Option<&str>,
    referer: &str,
    rect: Rect,
    radius: u8,
) {
    let tex = url.and_then(|u| images.get(u, referer).map(|t| (t.id(), t.size_vec2())));
    match tex {
        Some(t) => {
            let a = ui.ctx().animate_bool_with_time(egui::Id::new(("img_in", url)), true, 0.35);
            let p = ui.painter();
            p.rect_filled(rect, CornerRadius::same(radius), PANEL);
            textured(p, rect, radius, t, Color32::WHITE.gamma_multiply(a), 0.5);
        }
        None => {
            let failed = url.is_none_or(|u| images.failed(u));
            if failed {
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::same(radius), PANEL);
                p.text(rect.center(), Align2::CENTER_CENTER, icon::FILM_SLATE, regular(rect.height().min(rect.width()) * 0.2), DIM);
            } else {
                shimmer(ui, rect, radius);
            }
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

/// Poster card. `sub` is the gray line under the title. On hover the poster lifts a little,
/// darkens and a play button pops in; everything eases instead of snapping.
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
    let h = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.16);
    let lift = 4.0 * h;
    let prect = Rect::from_min_size(rect.min - vec2(0.0, lift), vec2(CARD_W, POSTER_H));
    if h > 0.0 {
        // soft shadow under the lifted poster
        ui.painter().rect_filled(prect.translate(vec2(0.0, 6.0)).expand(2.0 * h), CornerRadius::same(8), Color32::from_black_alpha((70.0 * h) as u8));
    }
    poster(ui, images, card.poster.as_deref(), referer, prect, 6);
    let p = ui.painter();
    if h > 0.0 {
        p.rect_filled(prect, CornerRadius::same(6), Color32::from_black_alpha((110.0 * h) as u8));
        let r = 18.0 + 8.0 * h;
        p.circle_filled(prect.center(), r, ACCENT.gamma_multiply(h));
        play_icon(p, prect.center(), 15.0 + 6.0 * h, Color32::WHITE.gamma_multiply(h));
        p.rect_stroke(prect, CornerRadius::same(6), Stroke::new(2.0, ACCENT.gamma_multiply(h)), StrokeKind::Inside);
    }
    if let Some(b) = card.badge.as_deref().filter(|b| !b.is_empty()) {
        let g = wrapped(ui, b, bold(11.5), Color32::WHITE, CARD_W - 20.0, 1);
        let br = Rect::from_min_size(prect.min + vec2(6.0, 6.0), g.size() + vec2(10.0, 4.0));
        p.rect_filled(br, CornerRadius::same(3), ACCENT);
        p.galley(br.min + vec2(5.0, 2.0), g, Color32::WHITE);
    }
    let title_col = lerp_color(TEXT, ACCENT_HI, h);
    let tg = wrapped(ui, &card.title, regular(14.5), title_col, CARD_W, 2);
    let th = tg.size().y;
    p.galley(pos2(rect.min.x, rect.min.y + POSTER_H + 6.0), tg, title_col);
    if !sub.is_empty() {
        let sg = wrapped(ui, sub, regular(12.0), MUTED, CARD_W, 1);
        p.galley(pos2(rect.min.x, rect.min.y + POSTER_H + 8.0 + th), sg, MUTED);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(&card.title)
}

/// Loading placeholder with the size of an `anime_card`.
pub fn card_skeleton(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(CARD_W, CARD_H), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    shimmer(ui, Rect::from_min_size(rect.min, vec2(CARD_W, POSTER_H)), 6);
    shimmer(ui, Rect::from_min_size(rect.min + vec2(0.0, POSTER_H + 8.0), vec2(CARD_W * 0.85, 12.0)), 4);
    shimmer(ui, Rect::from_min_size(rect.min + vec2(0.0, POSTER_H + 26.0), vec2(CARD_W * 0.5, 10.0)), 4);
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
    /// download state: None = not downloaded, Some(f) with f < 1 = downloading (f = done so far,
    /// negative = queued), Some(1.0) = on disk
    pub dl: Option<f32>,
    /// a downloaded episode: the ⋮ button deletes it instead of listing servers
    pub local: bool,
    /// hover text (the episode's summary when it is not shown in the row)
    pub tip: &'a str,
}

/// What was clicked on an episode row.
#[derive(Default)]
pub struct EpClick {
    pub play: bool,
    pub servers: bool,
    pub download: bool,
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
) -> EpClick {
    let w = ui.available_width();
    let h = if c.desc.is_empty() { 96.0 } else { 118.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if !ui.is_rect_visible(rect) {
        return EpClick::default();
    }
    let resp = if c.tip.is_empty() { resp } else { resp.on_hover_text(c.tip) };
    let hovered = resp.hovered();
    let ht = ui.ctx().animate_bool_with_time(resp.id.with("hover"), hovered, 0.14);
    let fill = if c.selected { CARD_HI } else { lerp_color(CARD, CARD_HI, ht) };
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
    let r = 17.0 + 3.0 * ht;
    p.circle_filled(tr.center(), r, lerp_color(Color32::from_black_alpha(150), ACCENT, ht));
    p.circle_stroke(tr.center(), r, Stroke::new(1.5, Color32::WHITE.gamma_multiply(1.0 - ht * 0.6)));
    play_icon(p, tr.center(), 14.0 + 2.0 * ht, Color32::WHITE);
    if let Some(f) = c.progress {
        let track = Rect::from_min_size(pos2(tr.min.x, tr.max.y - 4.0), vec2(tw, 4.0));
        p.rect_filled(track, CornerRadius::same(2), Color32::from_black_alpha(160));
        let bar = Rect::from_min_size(track.min, vec2(tw * f.clamp(0.0, 1.0), 4.0));
        p.rect_filled(bar, CornerRadius::same(2), ACCENT);
        if f >= 0.9 {
            // watched tick in the corner
            let c0 = pos2(tr.max.x - 13.0, tr.min.y + 13.0);
            p.circle_filled(c0, 10.0, ACCENT);
            p.text(c0, Align2::CENTER_CENTER, icon::CHECK, bold(12.0), Color32::WHITE);
        }
    }

    let x = tr.max.x + 16.0;
    let text_w = rect.max.x - x - 96.0;
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
        .on_hover_text(if c.local { crate::i18n::tr("Delete from this PC", "ลบออกจากเครื่อง") } else { crate::i18n::tr("Choose server", "เลือกเซิร์ฟเวอร์เอง") });
    if b.hovered() {
        ui.painter()
            .rect_filled(br, CornerRadius::same(6), PANEL_HI);
    }
    ui.painter().text(
        br.center(),
        Align2::CENTER_CENTER,
        if c.local { icon::TRASH } else { icon::DOTS_THREE_VERTICAL },
        regular(if c.local { 18.0 } else { 20.0 }),
        if b.hovered() && c.local { BAD } else if b.hovered() { TEXT } else { MUTED },
    );
    let srv = b.clicked();

    // download: arrow / ring with the progress / tick when on disk
    let dr = Rect::from_center_size(pos2(rect.max.x - 66.0, rect.center().y), vec2(34.0, 34.0));
    let tip = match c.dl {
        None => crate::i18n::tr("Download", "ดาวน์โหลด"),
        Some(f) if f >= 1.0 => crate::i18n::tr("Downloaded — in the Downloads tab", "ดาวน์โหลดแล้ว — อยู่ในแท็บดาวน์โหลด"),
        Some(_) => crate::i18n::tr("Downloading…", "กำลังดาวน์โหลด…"),
    };
    let d = ui
        .interact(dr, ui.id().with(("dl", id)), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tip);
    if d.hovered() {
        ui.painter().rect_filled(dr, CornerRadius::same(6), PANEL_HI);
    }
    let p = ui.painter();
    match c.dl {
        None => {
            p.text(dr.center(), Align2::CENTER_CENTER, icon::DOWNLOAD_SIMPLE, regular(19.0), if d.hovered() { TEXT } else { MUTED });
        }
        Some(f) if f >= 1.0 => {
            p.text(dr.center(), Align2::CENTER_CENTER, icon::CHECK_CIRCLE, regular(20.0), ACCENT_HI);
        }
        Some(f) => {
            let cen = dr.center();
            p.circle_stroke(cen, 11.0, Stroke::new(2.5, PANEL_HI));
            if f > 0.0 {
                let n = 40;
                let pts: Vec<egui::Pos2> = (0..=n)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_2 + f * std::f32::consts::TAU * i as f32 / n as f32;
                        cen + vec2(a.cos() * 11.0, a.sin() * 11.0)
                    })
                    .collect();
                p.add(egui::Shape::line(pts, Stroke::new(2.5, ACCENT)));
            } else {
                // queued / finding the link: a turning arc
                let t = ui.input(|i| i.time) as f32 * 4.0;
                let pts: Vec<egui::Pos2> = (0..=12).map(|i| t + i as f32 * 0.15).map(|a| cen + vec2(a.cos() * 11.0, a.sin() * 11.0)).collect();
                p.add(egui::Shape::line(pts, Stroke::new(2.5, ACCENT)));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
            }
            p.text(cen, Align2::CENTER_CENTER, icon::X, regular(10.0), MUTED);
        }
    }
    let download = d.clicked();
    let play = !srv && !download && resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
    EpClick { play, servers: srv, download }
}

// ------------------------------------------------------------------ controls

/// A texture cut to an exact circle (a triangle fan with matching UVs; rounded-rect images are off by a pixel
/// at small sizes and look lopsided).
pub fn circle_image(p: &egui::Painter, tex: egui::TextureId, center: egui::Pos2, r: f32, tint: Color32) {
    let mut m = Mesh::with_texture(tex);
    m.vertices.push(Vertex { pos: center, uv: egui::pos2(0.5, 0.5), color: tint });
    let n = 96;
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let (sn, cs) = a.sin_cos();
        m.vertices.push(Vertex { pos: center + vec2(cs * r, sn * r), uv: egui::pos2(0.5 + cs * 0.5, 0.5 + sn * 0.5), color: tint });
    }
    for i in 0..n as u32 {
        m.indices.extend_from_slice(&[0, i + 1, i + 2]);
    }
    p.add(egui::Shape::mesh(m));
}

/// A solid "play" triangle with softened corners, centred on its visual middle (the glyph from the icon font is
/// an outline and sits off-centre).
pub fn play_icon(p: &egui::Painter, center: egui::Pos2, size: f32, color: Color32) {
    let s = size;
    // centroid of the triangle below is at x = -0.05·s: shift it back onto the centre
    let o = center + vec2(0.06 * s, 0.0);
    let pts = vec![o + vec2(-0.36 * s, -0.48 * s), o + vec2(0.52 * s, 0.0), o + vec2(-0.36 * s, 0.48 * s)];
    p.add(egui::Shape::Path(egui::epaint::PathShape {
        points: pts,
        closed: true,
        fill: color,
        stroke: egui::epaint::PathStroke::new((0.14 * s).max(1.0), color),
    }));
}

/// Accent button with a drawn play icon ("▶ Start Watching").
pub fn play_button(ui: &mut Ui, text: &str) -> Response {
    let g = ui.painter().layout_no_wrap(text.to_string(), bold(15.0), Color32::WHITE);
    let size = vec2(18.0 + 14.0 + 11.0 + g.size().x + 20.0, 42.0);
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let hv = ui.ctx().animate_bool_with_time(r.id, r.hovered(), 0.12);
    let press = if r.is_pointer_button_down_on() { 1.0 } else { 0.0 };
    let rect = rect.shrink(press);
    let p = ui.painter();
    p.rect_filled(rect.translate(vec2(0.0, 3.0)), CornerRadius::same(8), Color32::from_black_alpha((60.0 + 40.0 * hv) as u8));
    p.rect_filled(rect, CornerRadius::same(8), lerp_color(ACCENT, ACCENT_HI, hv * 0.6));
    let ic = pos2(rect.min.x + 18.0 + 7.0, rect.center().y);
    play_icon(p, ic, 14.0 + hv, Color32::WHITE);
    p.galley(pos2(rect.min.x + 18.0 + 14.0 + 11.0, rect.center().y - g.size().y / 2.0), g, Color32::WHITE);
    r.on_hover_cursor(CursorIcon::PointingHand)
}

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

/// Text tabs with an accent underline that slides to the active tab. Returns the new index.
pub fn tabs(ui: &mut Ui, labels: &[String], current: usize) -> usize {
    let mut out = current;
    let id = ui.id().with("tabs_ul");
    let start_x = ui.cursor().min.x;
    let mut active_rect: Option<Rect> = None;
    let resp = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 34.0;
        for (i, l) in labels.iter().enumerate() {
            let active = i == current;
            let r = ui.add(egui::Label::new(RichText::new(l).font(bold(16.0)).color(Color32::TRANSPARENT)).sense(Sense::click()));
            // colour fades between muted and white
            let t = ui.ctx().animate_bool_with_time(r.id.with("on"), active || r.hovered(), 0.15);
            let col = lerp_color(MUTED, TEXT, if active { 1.0 } else { t * 0.7 });
            ui.painter().text(r.rect.left_center(), Align2::LEFT_CENTER, l, bold(16.0), col);
            if active {
                active_rect = Some(r.rect);
            }
            if r.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                out = i;
            }
        }
    });
    if let Some(rr) = active_rect {
        let ctx = ui.ctx();
        let x0 = ctx.animate_value_with_time(id.with("x0"), rr.min.x - 10.0, 0.22);
        let x1 = ctx.animate_value_with_time(id.with("x1"), rr.max.x + 10.0, 0.22);
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(x0, rr.max.y + 9.0), pos2(x1, rr.max.y + 12.0)),
            CornerRadius::same(2),
            ACCENT,
        );
    }
    let line_y = resp.response.rect.max.y + 12.0;
    ui.painter().hline(start_x..=ui.max_rect().max.x, line_y, Stroke::new(1.0, LINE));
    ui.add_space(14.0);
    out
}

/// Two-to-four way switch with a sliding thumb ("Sub | Dub"). Returns the new index.
pub fn segmented(ui: &mut Ui, labels: &[String], current: usize) -> usize {
    let font = bold(13.0);
    let widths: Vec<f32> = labels.iter().map(|l| ui.painter().layout_no_wrap(l.clone(), font.clone(), TEXT).size().x + 28.0).collect();
    let seg_w = widths.iter().cloned().fold(0.0, f32::max);
    let (rect, _) = ui.allocate_exact_size(vec2(seg_w * labels.len() as f32 + 6.0, 34.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(255), PANEL);
    let id = ui.id().with(("seg", labels.len()));
    let x = ui.ctx().animate_value_with_time(id, rect.min.x + 3.0 + seg_w * current as f32, 0.2);
    let thumb = Rect::from_min_size(pos2(x, rect.min.y + 3.0), vec2(seg_w, rect.height() - 6.0));
    p.rect_filled(thumb, CornerRadius::same(255), ACCENT);
    let mut out = current;
    for (i, l) in labels.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.min.x + 3.0 + seg_w * i as f32, rect.min.y), vec2(seg_w, rect.height()));
        let resp = ui.interact(r, id.with(i), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let col = if i == current { Color32::WHITE } else if resp.hovered() { TEXT } else { MUTED };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, l, font.clone(), col);
        if resp.clicked() {
            out = i;
        }
    }
    out
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

/// Animated loading placeholder (a soft light sweeping across), for cards and text lines.
pub fn shimmer(ui: &Ui, rect: Rect, radius: u8) {
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(radius), PANEL);
    let t = ui.input(|i| i.time) as f32;
    let w = rect.width().max(1.0);
    let band = (w * 0.45).max(60.0);
    let x = rect.min.x - band + ((t * 0.9) % 1.6) / 1.6 * (w + band * 2.0);
    let clip = p.with_clip_rect(rect);
    let hi = Color32::from_rgba_unmultiplied(255, 255, 255, 14);
    gradient(&clip, Rect::from_min_max(pos2(x, rect.min.y), pos2(x + band / 2.0, rect.max.y)), Color32::TRANSPARENT, hi, false);
    gradient(&clip, Rect::from_min_max(pos2(x + band / 2.0, rect.min.y), pos2(x + band, rect.max.y)), hi, Color32::TRANSPARENT, false);
    ui.ctx().request_repaint();
}

/// One sidebar entry: icon above a small label; the highlight fades in on hover / when active.
pub fn sidebar_item(ui: &mut Ui, glyph: &str, label: &str, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(64.0, 58.0), Sense::click());
    let ctx = ui.ctx();
    let a = ctx.animate_bool_with_time(resp.id.with("act"), active, 0.18);
    let h = ctx.animate_bool_with_time(resp.id.with("hov"), resp.hovered(), 0.12);
    let p = ui.painter();
    if a > 0.0 {
        p.rect_filled(rect.shrink(4.0 * (1.0 - a)), CornerRadius::same(10), SELECT.gamma_multiply(a));
    }
    if h > 0.0 && !active {
        p.rect_filled(rect, CornerRadius::same(10), PANEL.gamma_multiply(h));
    }
    if a > 0.0 {
        // accent bar on the left edge
        let bar_h = 22.0 * a;
        p.rect_filled(
            Rect::from_center_size(pos2(rect.min.x + 1.5, rect.center().y), vec2(3.0, bar_h)),
            CornerRadius::same(2),
            ACCENT,
        );
    }
    let col = lerp_color(lerp_color(MUTED, TEXT, h), ACCENT_HI, a);
    let lift = -2.0 * h;
    p.text(rect.center() - vec2(0.0, 8.0 - lift), Align2::CENTER_CENTER, glyph, regular(21.0), col);
    p.text(rect.center() + vec2(0.0, 15.0), Align2::CENTER_CENTER, label, regular(11.0), col);
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

pub struct SearchBar {
    /// Enter was pressed
    pub submitted: bool,
    /// the text changed this frame
    pub changed: bool,
}

/// The big search field: a centred pill with the magnifier inside, a clear button, and an accent ring that
/// fades in while it has the keyboard.
pub fn search_bar(ui: &mut Ui, text: &mut String, hint: &str, focus: bool) -> SearchBar {
    let w = ui.available_width().min(860.0);
    let h = 54.0;
    let pad = (ui.available_width() - w) / 2.0;
    let (outer, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let rect = Rect::from_min_size(pos2(outer.min.x + pad, outer.min.y), vec2(w, h));
    let id = ui.id().with("search_bar");
    let edit_id = id.with("edit");
    let has_focus = ui.memory(|m| m.has_focus(edit_id));
    let f = ui.ctx().animate_bool_with_time(id.with("focus"), has_focus, 0.18);
    let p = ui.painter();
    // soft glow + pill
    if f > 0.0 {
        p.rect_filled(rect.expand(4.0 * f), CornerRadius::same(255), ACCENT.gamma_multiply(0.16 * f));
    }
    p.rect_filled(rect, CornerRadius::same(255), lerp_color(PANEL, PANEL_HI, f));
    p.rect_stroke(rect, CornerRadius::same(255), Stroke::new(1.0 + f, lerp_color(LINE, ACCENT, f)), StrokeKind::Inside);
    p.text(pos2(rect.min.x + 26.0, rect.center().y), Align2::CENTER_CENTER, icon::MAGNIFYING_GLASS, regular(21.0), lerp_color(MUTED, ACCENT_HI, f));
    let clear_w = if text.is_empty() { 16.0 } else { 46.0 };
    let inner = Rect::from_min_max(pos2(rect.min.x + 50.0, rect.min.y + 6.0), pos2(rect.max.x - clear_w, rect.max.y - 6.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let r = child.add(
        egui::TextEdit::singleline(text)
            .id(edit_id)
            .hint_text(RichText::new(hint).color(DIM))
            .font(regular(19.0))
            .text_color(TEXT)
            .frame(egui::Frame::NONE)
            .desired_width(inner.width()),
    );
    if focus {
        r.request_focus();
    }
    let submitted = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let mut changed = r.changed();
    if !text.is_empty() {
        let c = pos2(rect.max.x - 28.0, rect.center().y);
        let cr = ui.interact(Rect::from_center_size(c, vec2(32.0, 32.0)), id.with("clear"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let hv = ui.ctx().animate_bool_with_time(id.with("clear_h"), cr.hovered(), 0.12);
        ui.painter().circle_filled(c, 14.0, CARD_HI.gamma_multiply(hv));
        ui.painter().text(c, Align2::CENTER_CENTER, icon::X, regular(15.0), lerp_color(MUTED, TEXT, hv));
        if cr.clicked() {
            text.clear();
            changed = true;
            ui.memory_mut(|m| m.request_focus(edit_id));
        }
    }
    SearchBar { submitted, changed }
}

/// Small rounded shortcut button with an icon ("recent search" chips).
pub fn shortcut_chip(ui: &mut Ui, glyph: &str, label: &str) -> Response {
    let font = regular(14.0);
    let shown: String = if label.chars().count() > 42 { label.chars().take(40).collect::<String>() + "…" } else { label.to_string() };
    let g = ui.painter().layout_no_wrap(shown, font, TEXT);
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 46.0, 34.0), Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id.with("h"), resp.hovered(), 0.12);
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(255), lerp_color(PANEL, CARD_HI, h));
    p.rect_stroke(rect, CornerRadius::same(255), Stroke::new(1.0, lerp_color(LINE, ACCENT, h)), StrokeKind::Inside);
    p.text(pos2(rect.min.x + 18.0, rect.center().y), Align2::CENTER_CENTER, glyph, regular(14.0), lerp_color(MUTED, ACCENT_HI, h));
    p.galley(pos2(rect.min.x + 32.0, rect.center().y - g.size().y / 2.0), g, TEXT);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Error card: a friendly sentence, the technical text folded under "Details".
pub fn problem_card(ui: &mut Ui, friendly: &str, raw: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(0x2a, 0x16, 0x18, 235))
        .stroke(Stroke::new(1.0, BAD.gamma_multiply(0.7)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(icon::WARNING_CIRCLE).size(22.0).color(Color32::from_rgb(0xff, 0x8a, 0x80)));
                ui.vertical(|ui| {
                    ui.label(RichText::new(friendly).size(14.5).color(Color32::from_rgb(0xff, 0xd6, 0xd2)));
                    if raw.trim() != friendly.trim() {
                        egui::CollapsingHeader::new(RichText::new(crate::i18n::tr("Details", "รายละเอียด")).size(12.0).color(MUTED))
                            .id_salt(("problem", raw.len()))
                            .show(ui, |ui| {
                                ui.label(RichText::new(raw).size(11.5).color(DIM).monospace());
                            });
                    }
                });
            });
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
