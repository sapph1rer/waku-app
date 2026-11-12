//! Watch order tab: the whole franchise (seasons, movies, OVAs, side stories) in air-date order, from AniList's
//! relations; kept on disk a week (`<cache>/waku/order/<id>.json`). Picking an entry searches the sources.

use crate::app::{Msg, WakuApp};
use crate::i18n::tr;
use crate::theme::*;
use crate::widgets;
use egui::{CornerRadius, RichText, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as icon;
use waku_core::meta::AnimeMeta;
use waku_core::meta::anilist::OrderItem;

const KEEP_SECS: u64 = 7 * 86_400;

fn path(id: u64) -> std::path::PathBuf {
    waku_core::profile::cache_dir().join("order").join(format!("{id}.json"))
}

pub fn save(id: u64, v: &[OrderItem]) {
    let p = path(id);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(t) = serde_json::to_string(v) {
        let _ = std::fs::write(p, t);
    }
}

fn load(id: u64) -> Option<Vec<OrderItem>> {
    let p = path(id);
    let age = p.metadata().ok()?.modified().ok()?.elapsed().ok()?.as_secs();
    if age > KEEP_SECS {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn relation_name(r: &str) -> &'static str {
    match r {
        "SOURCE" => tr("this show", "เรื่องนี้"),
        "PREQUEL" => tr("prequel", "ภาคก่อน"),
        "SEQUEL" => tr("sequel", "ภาคต่อ"),
        "SIDE_STORY" => tr("side story", "เรื่องเสริม"),
        "PARENT" => tr("main story", "เรื่องหลัก"),
        "ALTERNATIVE" => tr("alternative version", "อีกเวอร์ชัน"),
        "SPIN_OFF" => tr("spin-off", "ภาคแยก"),
        _ => "",
    }
}

fn format_label(f: &str) -> &'static str {
    match f {
        "TV" => "TV",
        "TV_SHORT" => tr("TV short", "TV สั้น"),
        "MOVIE" => tr("Movie", "มูฟวี่"),
        "OVA" => "OVA",
        "ONA" => "ONA",
        "SPECIAL" => tr("Special", "พิเศษ"),
        _ => "",
    }
}

impl WakuApp {
    /// The furthest episode watched of this AniList show (on any source), if any.
    fn watched_of(&self, anilist_id: u64) -> Option<f32> {
        self.lib
            .history
            .iter()
            .filter(|h| self.meta.get(&h.card.url, &h.card.title).and_then(|m| m.anilist_id) == Some(anilist_id))
            .map(|h| h.ep_num)
            .fold(None, |a: Option<f32>, n| Some(a.map_or(n, |a| a.max(n))))
    }

    /// Start loading the franchise of an AniList show (from disk, else AniList) unless it is known.
    pub fn ensure_order(&mut self, id: u64) {
        if !self.orders.contains_key(&id) {
            match load(id) {
                Some(v) => {
                    self.orders.insert(id, Some(Ok(v)));
                }
                None => {
                    self.orders.insert(id, None);
                    self.spawn(move |mgr| Msg::Order(id, waku_core::meta::anilist::watch_order(mgr.http(), id).map_err(|e| format!("{e:#}"))));
                }
            }
        }
    }

    /// Names for a page's seasons (1, 2, …): the TV seasons of the franchise in air order, when AniList knows
    /// as many as the page has (a page with one season is the show itself).
    pub fn season_names(&mut self, m: Option<&AnimeMeta>, seasons: &[u32]) -> std::collections::HashMap<u32, String> {
        let mut out = std::collections::HashMap::new();
        let Some(m) = m.filter(|m| m.matched) else { return out };
        let english = self.settings.language != "th";
        let name = |mm: &AnimeMeta| if english { mm.title_english.clone().or(mm.title_romaji.clone()) } else { mm.title_romaji.clone().or(mm.title_english.clone()) };
        if seasons.len() <= 1 {
            if let (Some(s), Some(n)) = (seasons.first(), name(m)) {
                out.insert(*s, n);
            }
            return out;
        }
        let Some(id) = m.anilist_id else { return out };
        self.ensure_order(id);
        let Some(Some(Ok(items))) = self.orders.get(&id) else { return out };
        let tv: Vec<&AnimeMeta> = items
            .iter()
            .filter(|x| x.main && matches!(x.meta.format.as_deref(), Some("TV") | Some("TV_SHORT") | Some("ONA")))
            .map(|x| &x.meta)
            .collect();
        // season n is the n-th TV entry; a page numbering more seasons than AniList has gets no names
        if seasons.iter().any(|s| *s == 0 || *s as usize > tv.len()) {
            return out;
        }
        for s in seasons {
            if let Some(n) = name(tv[*s as usize - 1]) {
                out.insert(*s, n);
            }
        }
        out
    }

    pub fn order_tab(&mut self, ui: &mut Ui, m: Option<&AnimeMeta>) {
        let Some(id) = m.and_then(|m| m.anilist_id) else {
            widgets::empty(
                ui,
                icon::TREE_STRUCTURE,
                tr(
                    "The watch order needs this show matched to AniList first (Details › Identify).",
                    "ต้องจับคู่เรื่องนี้กับ AniList ก่อน (แท็บรายละเอียด › ระบุเรื่อง)",
                ),
            );
            return;
        };
        self.ensure_order(id);
        let items = match self.orders.get(&id) {
            Some(Some(Ok(v))) => v.clone(),
            Some(Some(Err(e))) => {
                widgets::error_box(ui, &crate::app::first_line(e).to_string());
                if widgets::outline_button(ui, tr("Retry", "ลองใหม่")).clicked() {
                    self.orders.remove(&id);
                }
                return;
            }
            _ => {
                widgets::loading(ui, tr("Following the seasons, movies and OVAs…", "กำลังไล่ดูซีซั่น มูฟวี่ และ OVA…"));
                return;
            }
        };
        let n_main = items.iter().filter(|x| x.main).count();
        ui.horizontal(|ui| {
            let total = items.len();
            ui.label(RichText::new(crate::trf!("{total} in this franchise · {n_main} in the main story", "ทั้งหมด {total} ภาค · เนื้อเรื่องหลัก {n_main}")).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::chip(ui, tr("Main story only", "เฉพาะเนื้อเรื่องหลัก"), self.order_main_only, None).clicked() {
                    self.order_main_only = !self.order_main_only;
                }
            });
        });
        ui.add_space(10.0);
        let english = self.settings.language != "th";
        let mut pick: Option<AnimeMeta> = None;
        let mut step = 0;
        for it in items.iter().filter(|x| x.main || !self.order_main_only) {
            step += 1;
            let mm = &it.meta;
            let here = mm.anilist_id == Some(id);
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 92.0), Sense::click());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let fill = if here { SELECT } else if resp.hovered() { CARD_HI } else { CARD };
            ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
            if here {
                ui.painter().rect_stroke(rect, CornerRadius::same(10), egui::Stroke::new(1.5, ACCENT), egui::StrokeKind::Inside);
            }
            // step number + a line joining the main story
            let num_c = pos2(rect.min.x + 28.0, rect.center().y);
            ui.painter().circle_filled(num_c, 15.0, if it.main { ACCENT } else { PANEL_HI });
            ui.painter().text(num_c, egui::Align2::CENTER_CENTER, step.to_string(), bold(14.0), TEXT);
            let img = egui::Rect::from_min_size(pos2(rect.min.x + 56.0, rect.min.y + 8.0), vec2(54.0, 76.0));
            match mm.cover_url.as_deref().and_then(|u| self.images.get(u, "")) {
                Some(t) => {
                    egui::Image::new(t).corner_radius(CornerRadius::same(6)).paint_at(ui, img);
                }
                None => {
                    ui.painter().rect_filled(img, CornerRadius::same(6), PANEL);
                }
            }
            let x = img.max.x + 14.0;
            let title = if english { mm.title_english.clone().or(mm.title_romaji.clone()) } else { mm.title_romaji.clone().or(mm.title_english.clone()) }.unwrap_or_default();
            let p = ui.painter();
            let galley = p.layout(title, bold(16.0), TEXT, rect.max.x - x - 200.0);
            p.galley(pos2(x, rect.min.y + 14.0), galley, TEXT);
            let line = [
                mm.format.as_deref().map(|f| format_label(f).to_string()),
                mm.year.map(|y| y.to_string()),
                mm.episodes.map(|e| crate::trf!("{e} eps", "{e} ตอน")),
                mm.score.map(|s| format!("★ {:.1}", s as f32 / 10.0)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            p.text(pos2(x, rect.min.y + 58.0), egui::Align2::LEFT_TOP, line, egui::FontId::proportional(13.0), MUTED);
            // right side: relation, watched, here
            let rx = rect.max.x - 16.0;
            let tag = if here { tr("you are here", "คุณอยู่ตรงนี้").to_string() } else { relation_name(&it.relation).to_string() };
            p.text(pos2(rx, rect.min.y + 18.0), egui::Align2::RIGHT_TOP, tag, egui::FontId::proportional(12.5), if here { ACCENT_HI } else if it.main { MUTED } else { DIM });
            if let Some(id2) = mm.anilist_id
                && let Some(n) = self.watched_of(id2)
            {
                let total = mm.episodes.unwrap_or(0) as f32;
                let ep = waku_core::text::fmt_num(n);
                let txt = if total > 0.0 && n >= total { tr("watched", "ดูจบแล้ว").to_string() } else { crate::trf!("watched up to EP {ep}", "ดูถึงตอน {ep}") };
                ui.painter().text(pos2(rx, rect.min.y + 40.0), egui::Align2::RIGHT_TOP, format!("{}  {txt}", icon::CHECK_CIRCLE), egui::FontId::proportional(12.5), GOOD);
            }
            if !here && mm.status.as_deref() == Some("NOT_YET_RELEASED") {
                ui.painter().text(pos2(rx, rect.max.y - 16.0), egui::Align2::RIGHT_BOTTOM, tr("not out yet", "ยังไม่ฉาย"), egui::FontId::proportional(12.0), WARN);
            } else if !here {
                ui.painter().text(
                    pos2(rx, rect.max.y - 16.0),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("{}  {}", tr("Find on sources", "หาในแหล่ง"), icon::ARROW_RIGHT),
                    egui::FontId::proportional(12.5),
                    if resp.hovered() { ACCENT_HI } else { DIM },
                );
            }
            if !here && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                pick = Some(mm.clone());
            }
            ui.add_space(8.0);
        }
        if let Some(m) = pick {
            let q = m.title_romaji.clone().or(m.title_english.clone()).unwrap_or_default();
            self.search_box = q.clone();
            self.remember_search(&q);
            self.run_search(&q);
            self.go(crate::app::Page::Search);
        }
    }
}
