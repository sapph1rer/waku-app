//! "Which anime is this?": search by a screenshot (trace.moe), in a dialog opened from the picture button of
//! the search bar. The picture comes from a file dropped on it, the clipboard (Ctrl+V), a file picker or a link; the hits say the show, the episode and the moment. Picking one searches
//! the sources for the show, and the series page then offers "Play EP n from mm:ss".

use crate::app::{Msg, WakuApp};
use crate::i18n::tr;
use crate::theme::*;
use crate::widgets;
use egui::{CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, vec2};

use egui_phosphor::regular as icon;
use waku_core::meta::tracemoe::SceneHit;

#[derive(Default)]
pub struct SceneSearch {
    pub open: bool,
    pub busy: bool,
    pub url: String,
    pub hits: Vec<SceneHit>,
    pub error: Option<String>,
    /// the picture being searched, shown beside the results
    pub preview: Option<egui::TextureHandle>,
    /// the hit picked: the series page offers to play from that moment
    pub jump: Option<SceneHit>,
}

pub fn mmss(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// PNG bytes of the image on the clipboard.
fn clipboard_png() -> Result<Vec<u8>, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let img = cb.get_image().map_err(|_| tr("There is no picture on the clipboard", "ไม่มีรูปในคลิปบอร์ด").to_string())?;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&img.bytes, img.width as u32, img.height as u32, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

use image::ImageEncoder;

impl WakuApp {
    /// Look a picture up (bytes of a jpg / png / webp / gif).
    pub fn scene_search(&mut self, bytes: Vec<u8>) {
        if bytes.len() > 25 << 20 {
            self.scene.error = Some(tr("The picture is too big (25 MB at most)", "รูปใหญ่เกินไป (ไม่เกิน 25 MB)").into());
            return;
        }
        self.scene.preview = image::load_from_memory(&bytes).ok().map(|i| {
            let i = i.thumbnail(640, 360).to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw());
            self.ctx.load_texture("scene-preview", ci, Default::default())
        });
        self.scene.busy = true;
        self.scene.error = None;
        self.scene.hits.clear();
        self.scene.open = true;
        self.spawn(move |m| Msg::Scene(waku_core::meta::tracemoe::search_image(m.http(), bytes).map_err(|e| format!("{e:#}"))));
    }

    fn scene_from_url(&mut self, url: String) {
        self.scene.busy = true;
        self.scene.error = None;
        self.spawn(move |m| match m.http().get_bytes(&url, None) {
            Ok(b) => Msg::SceneBytes(Ok(b)),
            Err(e) => Msg::SceneBytes(Err(format!("{e:#}"))),
        });
    }

    /// Files dropped on the window: while the dialog is up, or on the Search page (which opens it).
    pub fn scene_drop(&mut self, ctx: &egui::Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        let Some(f) = files.first() else { return };
        let bytes = f.bytes().ok().or_else(|| std::fs::read(f.path()).ok());
        self.scene.open = true;
        match bytes {
            Some(b) if image::guess_format(&b).is_ok() => self.scene_search(b),
            Some(_) => self.scene.error = Some(tr("That file is not a picture (jpg, png, webp, gif)", "ไฟล์นี้ไม่ใช่รูปภาพ (jpg, png, webp, gif)").into()),
            None => self.scene.error = Some(tr("Could not read the dropped file", "อ่านไฟล์ที่ลากมาไม่ได้").into()),
        }
    }

    /// Open the system's file picker (in the background; the answer comes back as `Msg::SceneBytes`).
    fn scene_pick_file(&mut self) {
        #[cfg(windows)]
        {
            let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
            std::thread::spawn(move || {
                let f = rfd::FileDialog::new()
                    .add_filter(tr("Pictures", "รูปภาพ"), &["png", "jpg", "jpeg", "webp", "gif", "bmp"])
                    .pick_file();
                if let Some(p) = f {
                    let _ = tx.send(Msg::SceneBytes(std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))));
                    ctx.request_repaint();
                }
            });
        }
    }

    /// Start over with another picture.
    fn scene_reset(&mut self) {
        self.scene.preview = None;
        self.scene.hits.clear();
        self.scene.error = None;
        self.scene.busy = false;
    }

    /// The "search by picture" dialog: drop a file on it, paste a picture (Ctrl+V), pick a file or give a link;
    /// the results (show, episode, moment) are listed under the picture.
    pub fn scene_dialog(&mut self, ctx: &egui::Context) {
        let on_search = self.page == crate::app::Page::Search;
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if !self.scene.open {
            // a picture dragged over the Search page opens the dialog to drop it on
            if on_search && hovering {
                self.scene.open = true;
                self.scene_reset();
            } else {
                if on_search {
                    self.scene_drop(ctx);
                }
                return;
            }
        }
        self.scene_drop(ctx);
        // Ctrl+V: a picture on the clipboard, or a link to one (unless the link field is being typed in)
        let link_id = egui::Id::new("scene-link");
        let typing = ctx.memory(|m| m.has_focus(link_id));
        if !typing {
            let (ctrl_v, pasted) = ctx.input(|i| {
                let v = i.modifiers.command && i.key_pressed(egui::Key::V);
                let t = i.events.iter().find_map(|e| if let egui::Event::Paste(t) = e { Some(t.clone()) } else { None });
                (v, t)
            });
            if ctrl_v || pasted.is_some() {
                match clipboard_png() {
                    Ok(b) => self.scene_search(b),
                    Err(e) => match pasted.map(|t| t.trim().to_string()).filter(|t| t.starts_with("http")) {
                        Some(u) => {
                            self.scene.url = u.clone();
                            self.scene_from_url(u);
                        }
                        None => self.scene.error = Some(e),
                    },
                }
            }
        }

        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("scene-dialog"))
            .frame(Frame::new().fill(PANEL).stroke(Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(16)).inner_margin(Margin::same(22)))
            .show(ctx, |ui| {
                let w = (ctx.content_rect().width() - 120.0).clamp(420.0, 760.0);
                ui.set_width(w);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {}", icon::IMAGE_SQUARE, tr("Search by picture", "ค้นหาด้วยภาพ"))).font(bold(19.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::ghost_button(ui, icon::X).clicked() {
                            close = true;
                        }
                    });
                });
                ui.label(
                    RichText::new(tr(
                        "Finds which anime, episode and moment a screenshot comes from (trace.moe).",
                        "บอกได้ว่าภาพหน้าจอมาจากอนิเมะเรื่องไหน ตอนไหน นาทีไหน (trace.moe)",
                    ))
                    .size(12.5)
                    .color(MUTED),
                );
                ui.add_space(14.0);

                if self.scene.preview.is_none() && !self.scene.busy {
                    // the drop zone
                    let (zone, resp) = ui.allocate_exact_size(vec2(w, 230.0), Sense::click());
                    let hv = ui.ctx().animate_bool_with_time(egui::Id::new("scene-zone"), hovering || resp.hovered(), 0.15);
                    let p = ui.painter();
                    p.rect_filled(zone, CornerRadius::same(12), widgets::lerp_color(BAR, ACCENT.gamma_multiply(0.10), hv));
                    let col = widgets::lerp_color(LINE, ACCENT, hv);
                    let r = zone.shrink(1.0);
                    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                    p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.5, col), 8.0, 6.0));
                    let c = zone.center();
                    p.text(c - vec2(0.0, 52.0), egui::Align2::CENTER_CENTER, icon::CLOUD_ARROW_UP, egui::FontId::proportional(46.0), widgets::lerp_color(MUTED, ACCENT_HI, hv));
                    let big = if hovering { tr("Drop it here", "ปล่อยรูปที่นี่ได้เลย") } else { tr("Drag a picture here", "ลากรูปมาวางที่นี่") };
                    p.text(c + vec2(0.0, 4.0), egui::Align2::CENTER_CENTER, big, bold(18.0), TEXT);
                    p.text(
                        c + vec2(0.0, 32.0),
                        egui::Align2::CENTER_CENTER,
                        tr("or press Ctrl+V to paste a copied picture", "หรือกด Ctrl+V เพื่อวางรูปที่คัดลอกไว้"),
                        egui::FontId::proportional(13.5),
                        MUTED,
                    );
                    if cfg!(windows) {
                        p.text(c + vec2(0.0, 62.0), egui::Align2::CENTER_CENTER, format!("{}  {}", icon::FOLDER_OPEN, tr("click to choose a file", "คลิกเพื่อเลือกไฟล์")), egui::FontId::proportional(13.0), widgets::lerp_color(DIM, ACCENT_HI, hv));
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.scene_pick_file();
                        }
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::LINK).size(16.0).color(MUTED));
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.scene.url)
                                .id(link_id)
                                .hint_text(tr("…or paste a link to a picture", "…หรือวางลิงก์รูป"))
                                .desired_width(w - 120.0),
                        );
                        let go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (go || widgets::ghost_button(ui, tr("Search", "ค้นหา")).clicked()) && self.scene.url.trim().starts_with("http") {
                            let u = self.scene.url.trim().to_string();
                            self.scene_from_url(u);
                        }
                    });
                }
                if let Some(e) = &self.scene.error {
                    ui.add_space(8.0);
                    ui.label(RichText::new(format!("{}  {e}", icon::WARNING_CIRCLE)).color(BAD));
                }
                if self.scene.busy && self.scene.preview.is_none() {
                    ui.add_space(8.0);
                    widgets::loading(ui, tr("Fetching the picture…", "กำลังโหลดรูป…"));
                }
                if let Some(t) = self.scene.preview.clone() {
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            let sz = t.size_vec2();
                            let pw = 220.0;
                            ui.add(egui::Image::new(&t).fit_to_exact_size(vec2(pw, pw * sz.y / sz.x.max(1.0))).corner_radius(CornerRadius::same(8)));
                            ui.add_space(8.0);
                            if !self.scene.busy && widgets::outline_button(ui, &format!("{}  {}", icon::ARROW_COUNTER_CLOCKWISE, tr("Another picture", "ใช้รูปอื่น"))).clicked() {
                                self.scene_reset();
                            }
                        });
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            if self.scene.busy {
                                widgets::loading(ui, tr("Looking it up…", "กำลังค้นหา…"));
                            } else if self.scene.hits.is_empty() && self.scene.error.is_none() {
                                ui.label(RichText::new(tr("No sure match. Try a frame without subtitles or menus.", "ไม่พบที่ตรงพอ ลองภาพที่ไม่มีซับหรือเมนูทับ")).color(MUTED));
                            }
                            if let Some(i) = self.scene_hits(ui) {
                                let h = self.scene.hits[i].clone();
                                let q = h.title_romaji.clone();
                                self.scene.jump = Some(h);
                                self.search_box = q.clone();
                                self.run_search(&q);
                                if self.page != crate::app::Page::Search {
                                    self.go(crate::app::Page::Search);
                                }
                                close = true;
                            }
                        });
                    });
                }
            });
        if close || modal.should_close() {
            self.scene.open = false;
        }
    }

    /// The matches, best first; returns the one clicked.
    fn scene_hits(&mut self, ui: &mut Ui) -> Option<usize> {
        let english = self.settings.language != "th";
        let mut pick = None;
        for (i, h) in self.scene.hits.iter().take(5).enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 74.0), Sense::click());
            let fill = if resp.hovered() { CARD_HI } else { CARD };
            ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
            let img_r = egui::Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(103.0, 58.0));
            if let Some(u) = &h.image
                && let Some(t) = self.images.get(u, "")
            {
                egui::Image::new(t).corner_radius(CornerRadius::same(6)).paint_at(ui, img_r);
            } else {
                ui.painter().rect_filled(img_r, CornerRadius::same(6), PANEL_HI);
            }
            let x = img_r.max.x + 12.0;
            let p = ui.painter();
            let tw = rect.max.x - x - 110.0;
            let g = p.layout(h.title(english), bold(15.0), TEXT, tw.max(60.0));
            let g = if g.rows.len() > 1 { p.layout_no_wrap(h.title(english).chars().take(40).collect::<String>() + "…", bold(15.0), TEXT) } else { g };
            p.galley(egui::pos2(x, rect.min.y + 10.0), g, TEXT);
            let ep = h.episode.map(|e| format!("EP {}", waku_core::text::fmt_num(e))).unwrap_or_else(|| tr("Movie / special", "มูฟวี่ / พิเศษ").into());
            p.text(
                egui::pos2(x, rect.min.y + 34.0),
                egui::Align2::LEFT_TOP,
                format!("{ep} · {} {}", tr("at", "นาที"), mmss(h.at)),
                egui::FontId::proportional(13.0),
                ACCENT_HI,
            );
            p.text(
                egui::pos2(x, rect.min.y + 52.0),
                egui::Align2::LEFT_TOP,
                format!("{:.0}% {}", h.similarity * 100.0, tr("match", "ตรงกัน")),
                egui::FontId::proportional(11.5),
                if h.similarity > 0.9 { GOOD } else { WARN },
            );
            p.text(rect.right_center() - vec2(14.0, 0.0), egui::Align2::RIGHT_CENTER, format!("{}  {}", tr("Find & play", "หาแล้วเล่น"), icon::ARROW_RIGHT), egui::FontId::proportional(13.0), if resp.hovered() { ACCENT_HI } else { MUTED });
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                pick = Some(i);
            }
            ui.add_space(6.0);
        }
        pick
    }

    /// The scene picked from an image search, when this series page is that show.
    pub fn scene_for(&self, meta: Option<&waku_core::meta::AnimeMeta>) -> Option<SceneHit> {
        let j = self.scene.jump.as_ref()?;
        (meta?.anilist_id? == j.anilist_id).then(|| j.clone())
    }
}
