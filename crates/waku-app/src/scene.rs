//! "Which anime is this?": search by a screenshot (trace.moe). The picture comes from the clipboard, a file
//! dropped on the window or a link; the hits say the show, the episode and the moment. Picking one searches
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

    /// Files dropped on the window while the Search page is open.
    pub fn scene_drop(&mut self, ctx: &egui::Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(f) = files.first() {
            let bytes = std::fs::read(f.path()).ok();
            match bytes {
                Some(b) => self.scene_search(b),
                None => self.scene.error = Some(tr("Could not read the dropped file", "อ่านไฟล์ที่ลากมาไม่ได้").into()),
            }
        }
    }

    /// The image-search panel on the Search page.
    pub fn scene_panel(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.scene_drop(&ctx);
        // Ctrl+V with no text field taking it: a picture from the clipboard
        let paste = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V)) && !ctx.egui_wants_keyboard_input();
        if paste && let Ok(b) = clipboard_png() {
            self.scene_search(b);
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if !self.scene.open && !hovering {
            return;
        }
        let stroke = if hovering { Stroke::new(2.0, ACCENT) } else { Stroke::new(1.0, LINE) };
        Frame::new().fill(BAR).stroke(stroke).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}  {}", icon::IMAGE_SQUARE, tr("Which anime is this?", "ภาพนี้มาจากเรื่องอะไร?"))).font(bold(17.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::ghost_button(ui, icon::X).clicked() {
                        self.scene.open = false;
                    }
                });
            });
            ui.label(
                RichText::new(tr(
                    "Drop a screenshot here, press Ctrl+V with a picture copied, or paste a link to an image. Finds the show, the episode and the moment (trace.moe).",
                    "ลากภาพหน้าจอมาวางที่นี่ กด Ctrl+V เมื่อคัดลอกรูปไว้ หรือวางลิงก์รูป — บอกได้ว่าเรื่องอะไร ตอนไหน นาทีไหน (trace.moe)",
                ))
                .size(12.5)
                .color(MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if widgets::outline_button(ui, &format!("{}  {}", icon::CLIPBOARD_TEXT, tr("Paste picture", "วางรูปจากคลิปบอร์ด"))).clicked() {
                    match clipboard_png() {
                        Ok(b) => self.scene_search(b),
                        Err(e) => self.scene.error = Some(e),
                    }
                }
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.scene.url)
                        .hint_text(tr("…or a link to a picture", "…หรือลิงก์รูป"))
                        .desired_width(320.0),
                );
                let go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (go || widgets::ghost_button(ui, tr("Search", "ค้นหา")).clicked()) && self.scene.url.trim().starts_with("http") {
                    let u = self.scene.url.trim().to_string();
                    self.scene_from_url(u);
                }
            });
            if let Some(e) = &self.scene.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(BAD));
            }
            if self.scene.busy || !self.scene.hits.is_empty() || self.scene.preview.is_some() {
                ui.add_space(12.0);
            }
            ui.horizontal_top(|ui| {
                if let Some(t) = &self.scene.preview {
                    let sz = t.size_vec2();
                    let w = 220.0;
                    ui.add(egui::Image::new(t).fit_to_exact_size(vec2(w, w * sz.y / sz.x.max(1.0))).corner_radius(CornerRadius::same(8)));
                    ui.add_space(12.0);
                }
                ui.vertical(|ui| {
                    if self.scene.busy {
                        widgets::loading(ui, tr("Looking it up…", "กำลังค้นหา…"));
                    } else if self.scene.preview.is_some() && self.scene.hits.is_empty() && self.scene.error.is_none() {
                        ui.label(RichText::new(tr("No sure match. Try a frame without subtitles or menus.", "ไม่พบที่ตรงพอ ลองภาพที่ไม่มีซับหรือเมนูทับ")).color(MUTED));
                    }
                    let english = self.settings.language != "th";
                    let mut pick = None;
                    for (i, h) in self.scene.hits.iter().take(5).enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().min(620.0), 74.0), Sense::click());
                        let fill = if resp.hovered() { CARD_HI } else { CARD };
                        ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
                        let img_r = egui::Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(103.0, 58.0));
                        if let Some(u) = &h.image
                            && let Some(t) = self.images.get(u, "")
                        {
                            egui::Image::new(t).corner_radius(CornerRadius::same(6)).paint_at(ui, img_r);
                        } else {
                            ui.painter().rect_filled(img_r, CornerRadius::same(6), PANEL);
                        }
                        let x = img_r.max.x + 12.0;
                        let p = ui.painter();
                        p.text(egui::pos2(x, rect.min.y + 12.0), egui::Align2::LEFT_TOP, h.title(english), bold(15.0), TEXT);
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
                    if let Some(i) = pick {
                        let h = self.scene.hits[i].clone();
                        let q = h.title_romaji.clone();
                        self.scene.jump = Some(h);
                        self.search_box = q.clone();
                        self.run_search(&q);
                    }
                });
            });
        });
        ui.add_space(14.0);
    }

    /// The scene picked from an image search, when this series page is that show.
    pub fn scene_for(&self, meta: Option<&waku_core::meta::AnimeMeta>) -> Option<SceneHit> {
        let j = self.scene.jump.as_ref()?;
        (meta?.anilist_id? == j.anilist_id).then(|| j.clone())
    }
}
