//! Metadata in the UI: automatic matching, cover/title overrides for cards, the right-click
//! menu, and the Jellyfin-style Identify dialog for fixing a wrong match.

use crate::app::{Msg, WakuApp};
use crate::i18n::tr;
use crate::theme::*;
use crate::trf;
use crate::widgets;
use egui::{Color32, CornerRadius, CursorIcon, Rect, Response, RichText, Sense, Ui, pos2, vec2};
use egui_phosphor::regular as icon;
use std::sync::mpsc::{Sender, channel};
use waku_core::meta::{self, AnimeMeta};
use waku_core::{AnimeCard, AnimeDetail};

pub struct IdentifyState {
    pub card: AnimeCard,
    pub local_eps: u32,
    pub query: String,
    pub id_text: String,
    pub results: Option<Result<Vec<AnimeMeta>, String>>,
}

/// Actions from a card's right-click menu.
pub enum CardAct {
    Open(AnimeCard),
    Identify(AnimeCard),
    Refresh(AnimeCard),
    Clear(AnimeCard),
    ToggleList(AnimeCard),
    /// take a show out of Continue Watching (by its episode URL)
    Unwatch(String),
}

/// One background thread identifies listing cards, one at a time, so AniList's rate limit
/// is respected while the detail page (which uses its own thread) stays fast.
pub fn start_meta_worker(ctx: &egui::Context, tx: Sender<Msg>) -> Sender<(String, String)> {
    let (qtx, qrx) = channel::<(String, String)>();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let Ok(http) = waku_core::http::Http::new(20) else {
            return;
        };
        while let Ok((url, title)) = qrx.recv() {
            let res = meta::identify(&http, &title, &url, 0).map_err(|e| format!("{e:#}"));
            if tx.send(Msg::Meta { url, title, res }).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    });
    qtx
}

/// The context menu shown on any anime card.
pub fn card_menu(
    resp: &Response,
    card: &AnimeCard,
    in_list: bool,
    identified: bool,
    act: &mut Option<CardAct>,
) {
    card_menu_ext(resp, card, in_list, identified, None, act)
}

/// `card_menu` with "Remove from Continue Watching" (for history cards: their episode URL).
pub fn card_menu_ext(
    resp: &Response,
    card: &AnimeCard,
    in_list: bool,
    identified: bool,
    unwatch: Option<&str>,
    act: &mut Option<CardAct>,
) {
    resp.context_menu(|ui| {
        ui.set_min_width(210.0);
        if let Some(ep) = unwatch {
            if ui.button(format!("{}  {}", icon::X_CIRCLE, tr("Remove from Continue Watching", "เอาออกจากดูต่อ"))).clicked() {
                *act = Some(CardAct::Unwatch(ep.to_string()));
                ui.close();
            }
            ui.separator();
        }
        if ui
            .button(format!("{}  {}", icon::PLAY, tr("Open", "เปิด")))
            .clicked()
        {
            *act = Some(CardAct::Open(card.clone()));
            ui.close();
        }
        let l = if in_list {
            tr("Remove from My List", "เอาออกจากรายการ")
        } else {
            tr("Add to My List", "เพิ่มในรายการ")
        };
        if ui
            .button(format!("{}  {l}", icon::BOOKMARK_SIMPLE))
            .clicked()
        {
            *act = Some(CardAct::ToggleList(card.clone()));
            ui.close();
        }
        ui.separator();
        if ui
            .button(format!(
                "{}  {}",
                icon::MAGNIFYING_GLASS,
                tr("Identify…", "ระบุเรื่องใหม่…")
            ))
            .clicked()
        {
            *act = Some(CardAct::Identify(card.clone()));
            ui.close();
        }
        if ui
            .button(format!(
                "{}  {}",
                icon::ARROWS_CLOCKWISE,
                tr("Refresh metadata", "รีเฟรชข้อมูล")
            ))
            .clicked()
        {
            *act = Some(CardAct::Refresh(card.clone()));
            ui.close();
        }
        if identified
            && ui
                .button(format!(
                    "{}  {}",
                    icon::X_CIRCLE,
                    tr("Clear identification", "ล้างการระบุเรื่อง")
                ))
                .clicked()
        {
            *act = Some(CardAct::Clear(card.clone()));
            ui.close();
        }
    });
}

impl WakuApp {
    /// Matched metadata for a page (or the same title seen on another source).
    pub fn meta_of(&self, url: &str, title: &str) -> Option<&AnimeMeta> {
        self.meta.get(url, title).filter(|m| m.matched)
    }

    /// Title to show for an identified anime, following the "Titles" setting.
    pub fn title_for(&self, m: Option<&AnimeMeta>, site: &str) -> String {
        match (m, self.settings.title_lang.as_str()) {
            (Some(m), "english") => m.display_title(true).unwrap_or_else(|| site.to_string()),
            (Some(m), "romaji") => m.display_title(false).unwrap_or_else(|| site.to_string()),
            _ => site.to_string(),
        }
    }

    /// A card as it should be drawn: AniList cover + title when known. Unknown cards are queued
    /// for a background lookup.
    pub fn card_view(&mut self, c: &AnimeCard) -> AnimeCard {
        let mut out = c.clone();
        match self.meta.get(&c.url, &c.title) {
            Some(m) if m.matched => {
                if m.cover_url.is_some() {
                    out.poster = m.cover_url.clone();
                }
                out.title = self.title_for(Some(m), &c.title);
            }
            Some(_) => {}
            None => {
                if self.settings.fetch_meta && self.meta_pending.insert(c.url.clone()) {
                    let _ = self.meta_queue.send((c.url.clone(), c.title.clone()));
                }
            }
        }
        out
    }

    /// Poster card with metadata applied, right-click menu, and click-to-open.
    pub fn draw_card(&mut self, ui: &mut Ui, c: &AnimeCard, sub: &str, act: &mut Option<CardAct>) {
        let view = self.card_view(c);
        let referer = self.referer(&c.source);
        let resp = widgets::anime_card(ui, &mut self.images, &view, &referer, sub);
        let in_list = self.lib.in_list(&c.url);
        let identified = self.meta_of(&c.url, &c.title).is_some();
        card_menu(&resp, c, in_list, identified, act);
        if resp.clicked() {
            *act = Some(CardAct::Open(c.clone()));
        }
    }

    /// A Continue Watching card: like `draw_card`, with a "remove" button on the poster while hovered.
    pub fn draw_history_card(&mut self, ui: &mut Ui, c: &AnimeCard, ep_url: &str, sub: &str, act: &mut Option<CardAct>) {
        let view = self.card_view(c);
        let referer = self.referer(&c.source);
        let resp = widgets::anime_card(ui, &mut self.images, &view, &referer, sub);
        let in_list = self.lib.in_list(&c.url);
        let identified = self.meta_of(&c.url, &c.title).is_some();
        card_menu_ext(&resp, c, in_list, identified, Some(ep_url), act);
        // a small "remove" button, always there (dim), bright on hover; its own hit area on top of the card
        let id = egui::Id::new(("unwatch", ep_url));
        let close_c = resp.rect.min + egui::vec2(widgets::CARD_W - 16.0, 16.0);
        let close_r = egui::Rect::from_center_size(close_c, egui::vec2(30.0, 30.0));
        let hit = ui.interact(close_r, id, egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let card_hv = ui.ctx().animate_bool_with_time(id.with("card"), resp.hovered() || hit.hovered(), 0.15);
        let hv = ui.ctx().animate_bool_with_time(id.with("hv"), hit.hovered(), 0.1);
        let p = ui.painter();
        let alpha = 0.55 + 0.45 * card_hv;
        p.circle_filled(close_c, 12.0, widgets::lerp_color(egui::Color32::from_black_alpha(200), egui::Color32::from_rgb(0xd9, 0x3a, 0x3a), hv).gamma_multiply(alpha));
        p.circle_stroke(close_c, 12.0, egui::Stroke::new(1.0, egui::Color32::from_white_alpha((60.0 * alpha) as u8)));
        let k = 4.2;
        let st = egui::Stroke::new(1.8, egui::Color32::WHITE.gamma_multiply(alpha));
        p.line_segment([close_c + egui::vec2(-k, -k), close_c + egui::vec2(k, k)], st);
        p.line_segment([close_c + egui::vec2(-k, k), close_c + egui::vec2(k, -k)], st);
        let hit = hit.on_hover_text(tr("Remove from Continue Watching", "เอาออกจากดูต่อ"));
        if hit.clicked() {
            *act = Some(CardAct::Unwatch(ep_url.to_string()));
        } else if resp.clicked() && !hit.hovered() {
            *act = Some(CardAct::Open(c.clone()));
        }
    }

    /// Full metadata (with episode titles) for an opened series page.
    pub fn ensure_meta_detail(&mut self, d: &AnimeDetail) {
        if !self.settings.fetch_meta || self.meta_pending.contains(&d.url) {
            return;
        }
        let existing = self.meta.get(&d.url, &d.title).cloned();
        // an automatic match to a show that has not aired, for a page that has episodes, is wrong: match again
        let existing = existing.filter(|m| m.manual || !m.matched || d.episodes.is_empty() || m.status.as_deref() != Some("NOT_YET_RELEASED"));
        if let Some(m) = &existing
            && !m.stale(meta::now())
            && ((m.enriched && m.anizip && m.eps_rev >= meta::EPS_REV) || !m.matched)
        {
            return;
        }
        self.meta_pending.insert(d.url.clone());
        let (url, title, eps) = (d.url.clone(), d.title.clone(), d.episodes.len() as u32);
        self.spawn(move |mgr| {
            let http = mgr.http();
            let res = (|| -> anyhow::Result<AnimeMeta> {
                let m = match existing {
                    // keep a hand-picked entry; just refresh it
                    Some(m) if m.manual && m.matched => {
                        meta::by_id(http, m.anilist_id.unwrap_or(0))?
                    }
                    Some(m) if m.matched && m.anilist_id.is_some() => {
                        let mut f =
                            waku_core::meta::anilist::by_id(http, m.anilist_id.unwrap_or(0))?;
                        f.matched = true;
                        f.fetched_at = meta::now();
                        f
                    }
                    _ => meta::identify(http, &title, &url, eps)?,
                };
                Ok(if m.matched { meta::enrich(http, m) } else { m })
            })();
            Msg::Meta {
                url,
                title,
                res: res.map_err(|e| format!("{e:#}")),
            }
        });
    }

    pub fn on_meta(&mut self, url: String, title: String, res: Result<AnimeMeta, String>) {
        self.meta_pending.remove(&url);
        match res {
            Ok(m) => {
                // an automatic answer arriving late must not replace a hand-picked match
                if let Some(old) = self.meta.get(&url, &title)
                    && old.manual
                    && !m.manual
                {
                    return;
                }
                self.meta.set(&url, &title, m);
                self.meta.save();
            }
            Err(e) => {
                if self.identify.is_none()
                    && self.detail.as_ref().is_some_and(|d| d.card.url == url)
                {
                    let e = crate::app::first_line(&e);
                    self.notify(trf!(
                        "Metadata lookup failed: {e}",
                        "ดึงข้อมูลเรื่องไม่สำเร็จ: {e}"
                    ));
                }
            }
        }
    }

    pub fn apply_card_act(&mut self, a: CardAct) {
        match a {
            CardAct::Open(c) => self.open(&c),
            CardAct::ToggleList(c) => self.lib.toggle_list(&c),
            CardAct::Unwatch(ep) => {
                self.lib.unwatch(&ep);
                self.notify(tr("Removed from Continue Watching", "เอาออกจากดูต่อแล้ว"));
            }
            CardAct::Identify(c) => self.open_identify(&c, 0),
            CardAct::Refresh(c) => self.refresh_meta(&c),
            CardAct::Clear(c) => {
                self.meta.clear(&c.url);
                // remember "no match" so it is not looked up again automatically
                let none = AnimeMeta {
                    matched: false,
                    manual: true,
                    fetched_at: meta::now(),
                    ..Default::default()
                };
                self.meta.set(&c.url, &c.title, none);
                self.meta.save();
                self.notify(tr("Identification cleared", "ล้างการระบุเรื่องแล้ว"));
            }
        }
    }

    pub fn refresh_meta(&mut self, c: &AnimeCard) {
        let keep = self
            .meta
            .get(&c.url, &c.title)
            .filter(|m| m.manual && m.matched)
            .and_then(|m| m.anilist_id);
        self.meta_pending.insert(c.url.clone());
        let (url, title) = (c.url.clone(), c.title.clone());
        let eps = self
            .detail
            .as_ref()
            .and_then(|d| d.data.as_ref())
            .and_then(|r| r.as_ref().ok())
            .filter(|d| d.url == url)
            .map_or(0, |d| d.episodes.len() as u32);
        self.spawn(move |mgr| {
            let http = mgr.http();
            let res = (|| -> anyhow::Result<AnimeMeta> {
                let m = match keep {
                    Some(id) => meta::by_id(http, id)?,
                    None => meta::identify(http, &title, &url, eps)?,
                };
                Ok(if m.matched { meta::enrich(http, m) } else { m })
            })();
            Msg::Meta {
                url,
                title,
                res: res.map_err(|e| format!("{e:#}")),
            }
        });
        if keep.is_none() {
            self.meta.clear(&c.url);
        }
        self.notify(tr("Refreshing metadata…", "กำลังรีเฟรชข้อมูล…"));
    }

    // ------------------------------------------------------------ identify dialog

    pub fn open_identify(&mut self, c: &AnimeCard, local_eps: u32) {
        let query = meta::queries_for(&c.title, &c.url)
            .into_iter()
            .next()
            .unwrap_or_else(|| c.title.clone());
        self.identify = Some(IdentifyState {
            card: c.clone(),
            local_eps,
            query,
            id_text: String::new(),
            results: None,
        });
        self.identify_search();
    }

    fn identify_search(&mut self) {
        let Some(st) = &mut self.identify else { return };
        let q = st.query.trim().to_string();
        if q.is_empty() {
            return;
        }
        st.results = None;
        self.identify_seq += 1;
        let seq = self.identify_seq;
        let local_eps = st.local_eps;
        self.spawn(move |mgr| {
            let res = meta::anilist::search(mgr.http(), &q).map(|mut v| {
                // best guess first, like the automatic matcher would rank them
                v.sort_by(|a, b| {
                    meta::match_score(&q, b, local_eps)
                        .partial_cmp(&meta::match_score(&q, a, local_eps))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                v
            });
            Msg::IdentifyRes {
                seq,
                res: res.map_err(|e| format!("{e:#}")),
            }
        });
    }

    fn identify_apply(&mut self, id: u64) {
        let Some(st) = self.identify.take() else {
            return;
        };
        let (url, title) = (st.card.url.clone(), st.card.title.clone());
        self.meta_pending.insert(url.clone());
        self.spawn(move |mgr| {
            let http = mgr.http();
            let res = meta::by_id(http, id).map(|m| meta::enrich(http, m));
            Msg::Meta {
                url,
                title,
                res: res.map_err(|e| format!("{e:#}")),
            }
        });
        self.notify(tr("Applying the new match…", "กำลังใช้ข้อมูลเรื่องที่เลือก…"));
    }

    pub fn identify_modal(&mut self, ctx: &egui::Context) {
        let Some(st) = &self.identify else { return };
        let card = st.card.clone();
        let results = st.results.clone();
        let current = self
            .meta
            .get(&card.url, &card.title)
            .filter(|m| m.matched)
            .cloned();
        let mut search = false;
        let mut apply: Option<u64> = None;
        let mut close = false;
        let mut bad_id = false;

        let resp = egui::Modal::new(egui::Id::new("identify")).show(ctx, |ui| {
            ui.set_width(660.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icon::MAGNIFYING_GLASS)
                        .size(22.0)
                        .color(ACCENT),
                );
                ui.label(RichText::new(tr("Identify", "ระบุเรื่อง")).font(bold(22.0)));
            });
            let src = self.source_name(&card.source);
            ui.label(RichText::new(format!("{}  ·  {src}", card.title)).color(MUTED));
            if let Some(m) = &current {
                let t = m.display_title(true).unwrap_or_default();
                let how = if m.manual {
                    tr("chosen by you", "เลือกเอง")
                } else {
                    tr("automatic", "อัตโนมัติ")
                };
                ui.label(
                    RichText::new(trf!("Current match: {t} ({how})", "ตอนนี้จับคู่กับ: {t} ({how})"))
                        .size(13.0)
                        .color(Color32::from_gray(190)),
                );
            }
            ui.add_space(10.0);

            let Some(st) = &mut self.identify else { return };
            egui::Grid::new("identify_fields")
                .num_columns(3)
                .spacing([10.0, 10.0])
                .show(ui, |ui| {
                    ui.label(tr("Name", "ชื่อเรื่อง"));
                    let r = ui.add_sized(
                        [420.0, 34.0],
                        egui::TextEdit::singleline(&mut st.query)
                            .background_color(PANEL)
                            .margin(egui::Margin::symmetric(8, 6)),
                    );
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        search = true;
                    }
                    if widgets::primary_button(
                        ui,
                        &format!("{}  {}", icon::MAGNIFYING_GLASS, tr("Search", "ค้นหา")),
                    )
                    .clicked()
                    {
                        search = true;
                    }
                    ui.end_row();
                    ui.label(tr("AniList ID / URL", "AniList ID / ลิงก์"));
                    ui.add_sized(
                        [420.0, 34.0],
                        egui::TextEdit::singleline(&mut st.id_text)
                            .background_color(PANEL)
                            .hint_text("154587  /  https://anilist.co/anime/154587")
                            .margin(egui::Margin::symmetric(8, 6)),
                    );
                    if widgets::outline_button(ui, tr("Use ID", "ใช้ ID นี้")).clicked()
                    {
                        match meta::parse_anilist_id(&st.id_text) {
                            Some(id) => apply = Some(id),
                            None => bad_id = true,
                        }
                    }
                    ui.end_row();
                });
            ui.add_space(10.0);

            match &results {
                None => widgets::loading(ui, tr("Searching AniList…", "กำลังค้นหาใน AniList…")),
                Some(Err(e)) => widgets::error_box(ui, &crate::app::first_line(e)),
                Some(Ok(list)) if list.is_empty() => {
                    ui.label(
                        RichText::new(tr(
                            "No results — try the English or romaji name",
                            "ไม่พบ — ลองชื่ออังกฤษหรือโรมาจิ",
                        ))
                        .color(MUTED),
                    );
                }
                Some(Ok(list)) => {
                    egui::ScrollArea::vertical()
                        .max_height(430.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 6.0;
                            for m in list {
                                if self.identify_row(
                                    ui,
                                    m,
                                    current.as_ref().and_then(|c| c.anilist_id) == m.anilist_id,
                                ) {
                                    apply = m.anilist_id;
                                }
                            }
                        });
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if widgets::ghost_button(ui, tr("Cancel", "ยกเลิก")).clicked() {
                    close = true;
                }
                ui.label(
                    RichText::new(tr(
                        "Tip: right-click any title to identify it again",
                        "เคล็ดลับ: คลิกขวาที่เรื่องไหนก็ได้เพื่อระบุเรื่องใหม่",
                    ))
                    .size(12.0)
                    .color(DIM),
                );
            });
        });
        if resp.should_close() {
            close = true;
        }
        if bad_id {
            self.notify(tr(
                "That is not an AniList id or link",
                "ไม่ใช่ AniList ID หรือลิงก์",
            ));
        }
        if search {
            self.identify_search();
        } else if let Some(id) = apply {
            self.identify_apply(id);
        } else if close {
            self.identify = None;
        }
    }

    /// One AniList candidate. Returns true when clicked.
    fn identify_row(&mut self, ui: &mut Ui, m: &AnimeMeta, is_current: bool) -> bool {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 92.0), Sense::click());
        let hovered = resp.hovered();
        ui.painter().rect_filled(
            r,
            CornerRadius::same(8),
            if hovered { CARD_HI } else { CARD },
        );
        if is_current {
            ui.painter().rect_stroke(
                r,
                CornerRadius::same(8),
                egui::Stroke::new(1.5, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let pr = Rect::from_min_size(r.min + vec2(10.0, 8.0), vec2(53.0, 76.0));
        widgets::poster(
            ui,
            &mut self.images,
            m.cover_url.as_deref(),
            "https://anilist.co/",
            pr,
            4,
        );
        let p = ui.painter();
        let x = pr.max.x + 14.0;
        let w = r.max.x - x - 70.0;
        let title = m.display_title(true).unwrap_or_default();
        let g = widgets::wrapped(ui, &title, bold(15.5), TEXT, w, 1);
        p.galley(pos2(x, r.min.y + 10.0), g, TEXT);
        let alt = [&m.title_romaji, &m.title_native]
            .into_iter()
            .flatten()
            .filter(|t| **t != title)
            .cloned()
            .collect::<Vec<_>>()
            .join("  ·  ");
        let g = widgets::wrapped(ui, &alt, regular(12.5), MUTED, w, 1);
        p.galley(pos2(x, r.min.y + 34.0), g, MUTED);
        let info = crate::i18n::meta_line(m);
        let g = widgets::wrapped(ui, &info, regular(12.5), Color32::from_gray(200), w, 1);
        p.galley(pos2(x, r.min.y + 58.0), g, Color32::from_gray(200));
        if let Some(sc) = m.score {
            p.text(
                pos2(r.max.x - 16.0, r.center().y),
                egui::Align2::RIGHT_CENTER,
                format!("{} {sc}%", icon::STAR),
                bold(14.0),
                if sc >= 75 { GOOD } else { WARN },
            );
        }
        resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
    }
}
