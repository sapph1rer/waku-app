//! Series page (hero + episode list) and the server picker.

use crate::app::{Alts, WakuApp};
use crate::theme::*;
use crate::widgets::{self, chip, poster, tag};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use egui_phosphor::regular as icon;
use waku_core::text::truncate;
use waku_core::{AnimeCard, Episode, StreamRank};

enum Act {
    Play(Episode),
    Pick(Episode),
    Open(AnimeCard),
    Alts,
    ToggleList,
}

impl WakuApp {
    pub fn detail_page(&mut self, ui: &mut Ui) {
        if widgets::ghost_button(ui, &format!("{}  กลับ", icon::ARROW_LEFT)).clicked() {
            self.go_back();
            return;
        }
        ui.add_space(6.0);
        let Some(st) = &self.detail else {
            widgets::empty(ui, icon::TELEVISION, "ยังไม่ได้เลือกเรื่อง");
            return;
        };
        let card = st.card.clone();
        let data = match &st.data {
            None => {
                self.hero_skeleton(ui, &card);
                widgets::loading(
                    ui,
                    &format!("กำลังโหลดจาก {} …", self.source_name(&card.source)),
                );
                return;
            }
            Some(Err(e)) => {
                let e = e.clone();
                self.hero_skeleton(ui, &card);
                widgets::error_box(ui, &crate::app::first_line(&e));
                ui.add_space(8.0);
                if widgets::outline_button(ui, &format!("{}  หาเรื่องนี้จากแหล่งอื่น", icon::SWAP))
                    .clicked()
                {
                    let q = card.title.clone();
                    self.search_box = q.clone();
                    self.run_search(&q);
                }
                return;
            }
            Some(Ok(d)) => d.clone(),
        };

        let mut act: Option<Act> = None;
        let referer = self.referer(&data.source);
        let last = self.lib.last_watched(&data.url, &data.title).cloned();
        let in_list = self.lib.in_list(&data.url);

        // ---------------------------------------------------------- hero
        ui.horizontal_top(|ui| {
            let (prect, _) = ui.allocate_exact_size(vec2(230.0, 345.0), Sense::hover());
            poster(
                ui,
                &mut self.images,
                data.poster.as_deref().or(card.poster.as_deref()),
                &referer,
                prect,
                6,
            );
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.set_max_width((ui.available_width()).min(820.0));
                ui.label(RichText::new(&data.title).font(bold(30.0)).color(TEXT));
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("●").color(self.health_of(&data.source)));
                    ui.label(RichText::new(self.source_name(&data.source)).color(ACCENT));
                    ui.label(RichText::new(format!("·  {} ตอน", data.episodes.len())).color(MUTED));
                });
                ui.add_space(6.0);
                if !data.tags.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for t in &data.tags {
                            tag(ui, t);
                        }
                    });
                    ui.add_space(8.0);
                }
                if !data.synopsis.is_empty() {
                    let more = self.detail.as_ref().is_some_and(|d| d.more);
                    let long = data.synopsis.chars().count() > 320;
                    let text = if long && !more {
                        truncate(&data.synopsis, 320)
                    } else {
                        data.synopsis.clone()
                    };
                    ui.label(
                        RichText::new(text)
                            .color(Color32::from_rgb(0xd0, 0xd1, 0xd6))
                            .size(14.5),
                    );
                    if long
                        && widgets::ghost_button(
                            ui,
                            if more {
                                "ย่อ"
                            } else {
                                "อ่านเพิ่มเติม"
                            },
                        )
                        .clicked()
                    {
                        if let Some(d) = &mut self.detail {
                            d.more = !d.more;
                        }
                    }
                    ui.add_space(8.0);
                }

                ui.horizontal_wrapped(|ui| {
                    // continue = the episode after the last one watched (or that one if it is the last)
                    let resume = last.as_ref().and_then(|h| {
                        let i = data
                            .episodes
                            .iter()
                            .position(|e| (e.num - h.ep_num).abs() < 0.01 || e.url == h.ep_url)?;
                        data.episodes.get(i + 1).or(data.episodes.get(i)).cloned()
                    });
                    match (&resume, data.episodes.first()) {
                        (Some(e), _) => {
                            if widgets::primary_button(
                                ui,
                                &format!("{}  ดูต่อ: {}", icon::PLAY, e.label),
                            )
                            .clicked()
                            {
                                act = Some(Act::Play(e.clone()));
                            }
                        }
                        (None, Some(e)) => {
                            if widgets::primary_button(
                                ui,
                                &format!("{}  เริ่มดู {}", icon::PLAY, e.label),
                            )
                            .clicked()
                            {
                                act = Some(Act::Play(e.clone()));
                            }
                        }
                        (None, None) => {
                            ui.label(RichText::new("ยังไม่มีตอนให้ดูในแหล่งนี้").color(MUTED));
                        }
                    }
                    let list_label = if in_list {
                        format!("{}  อยู่ในรายการ", icon::CHECK)
                    } else {
                        format!("{}  รายการของฉัน", icon::PLUS)
                    };
                    if widgets::outline_button(ui, &list_label).clicked() {
                        act = Some(Act::ToggleList);
                    }
                    if widgets::outline_button(ui, &format!("{}  แหล่งอื่น", icon::SWAP))
                        .on_hover_text("หาเรื่องเดียวกันจากแหล่งอื่น")
                        .clicked()
                    {
                        act = Some(Act::Alts);
                    }
                });

                // alternatives
                if let Some(d) = &self.detail {
                    match &d.alts {
                        Alts::Idle => {}
                        Alts::Loading => widgets::loading(ui, "กำลังค้นหาในแหล่งอื่น…"),
                        Alts::Done(list) if list.is_empty() => {
                            ui.label(RichText::new("ไม่พบเรื่องนี้ในแหล่งอื่น").color(MUTED));
                        }
                        Alts::Done(list) => {
                            ui.add_space(4.0);
                            ui.label(RichText::new("มีในแหล่งอื่นด้วย:").size(13.0).color(MUTED));
                            let list = list.clone();
                            ui.horizontal_wrapped(|ui| {
                                for a in &list {
                                    let label = format!(
                                        "{} · {}%",
                                        self.source_name(&a.card.source),
                                        (a.score * 100.0).round()
                                    );
                                    let dot = self.health_of(&a.card.source);
                                    if chip(ui, &label, false, Some(dot))
                                        .on_hover_text(&a.card.title)
                                        .clicked()
                                    {
                                        act = Some(Act::Open(a.card.clone()));
                                    }
                                }
                            });
                        }
                    }
                }
            });
        });

        // ---------------------------------------------------------- episodes
        ui.add_space(26.0);
        let mut seasons: Vec<u32> = data.episodes.iter().map(|e| e.season).collect();
        seasons.dedup();
        seasons.sort();
        seasons.dedup();
        let (sel_season, newest_first) = self
            .detail
            .as_ref()
            .map(|d| (d.season, d.newest_first))
            .unwrap_or((None, false));
        let season = sel_season
            .filter(|s| seasons.contains(s))
            .or(seasons.first().copied());

        ui.horizontal(|ui| {
            ui.label(RichText::new("ตอนทั้งหมด").font(bold(21.0)));
            ui.add_space(10.0);
            if seasons.len() > 1 {
                for s in &seasons {
                    if chip(ui, &format!("ซีซั่น {s}"), Some(*s) == season, None).clicked()
                    {
                        if let Some(d) = &mut self.detail {
                            d.season = Some(*s);
                        }
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if newest_first {
                    "ล่าสุดก่อน"
                } else {
                    "ตอนแรกก่อน"
                };
                if widgets::ghost_button(ui, &format!("{}  {label}", icon::ARROWS_CLOCKWISE))
                    .clicked()
                {
                    if let Some(d) = &mut self.detail {
                        d.newest_first = !d.newest_first;
                    }
                }
            });
        });
        ui.add_space(8.0);

        let mut eps: Vec<&Episode> = data
            .episodes
            .iter()
            .filter(|e| season.is_none_or(|s| e.season == s))
            .collect();
        if newest_first {
            eps.reverse();
        }
        if eps.is_empty() {
            widgets::empty(
                ui,
                icon::LIST_NUMBERS,
                "ไม่พบรายชื่อตอน — ลอง “แหล่งอื่น” หรือกดทดสอบแหล่งในหน้าตั้งค่า",
            );
        }
        let watched_num = last.as_ref().map(|h| h.ep_num);
        let avail = ui.available_width();
        let gap = 12.0;
        let cols = ((avail + gap) / (300.0 + gap)).floor().max(1.0) as usize;
        let w = (avail - gap * (cols as f32 - 1.0)) / cols as f32;
        for row in eps.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for e in row {
                    if let Some(a) = episode_row(ui, e, w, watched_num) {
                        act = Some(a);
                    }
                }
            });
            ui.add_space(gap - 8.0);
        }

        // ---------------------------------------------------------- actions
        match act {
            Some(Act::Play(e)) => self.play(&data, &e, None),
            Some(Act::Pick(e)) => self.pick_server(&data, &e),
            Some(Act::Open(c)) => self.open(&c),
            Some(Act::Alts) => self.find_alts(),
            Some(Act::ToggleList) => {
                let mut c = data.card();
                c.poster = c.poster.or(card.poster);
                self.lib.toggle_list(&c);
            }
            None => {}
        }
    }

    fn hero_skeleton(&mut self, ui: &mut Ui, card: &AnimeCard) {
        let referer = self.referer(&card.source);
        ui.horizontal_top(|ui| {
            let (prect, _) = ui.allocate_exact_size(vec2(230.0, 345.0), Sense::hover());
            poster(
                ui,
                &mut self.images,
                card.poster.as_deref(),
                &referer,
                prect,
                6,
            );
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(&card.title).font(bold(30.0)));
                ui.label(RichText::new(self.source_name(&card.source)).color(ACCENT));
            });
        });
        ui.add_space(12.0);
    }

    pub fn server_modal(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.pick else { return };
        let mut close = false;
        let mut chosen = None;
        let title = format!("{} — {}", truncate(&p.detail.title, 50), p.ep.label);
        let source = self.source_name(&p.detail.source);
        let res = p.res.clone();
        let resp = egui::Modal::new(egui::Id::new("server_pick")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new("เลือกเซิร์ฟเวอร์").font(bold(20.0)));
            ui.label(RichText::new(format!("{title}  ·  {source}")).color(MUTED));
            ui.add_space(10.0);
            match &res {
                None => widgets::loading(ui, "กำลังตรวจทุกเซิร์ฟเวอร์…"),
                Some(Err(e)) => widgets::error_box(ui, &crate::app::first_line(e)),
                Some(Ok(list)) => {
                    for st in list {
                        let (kind, col) = match st.rank {
                            StreamRank::Direct => ("สตรีมตรง", GOOD),
                            StreamRank::YtDlp => ("ผ่าน yt-dlp", WARN),
                            StreamRank::Unknown => ("ไม่รู้จัก (ทางสุดท้าย)", BAD),
                        };
                        let qs: Vec<String> =
                            st.heights().iter().map(|h| format!("{h}p")).collect();
                        let label = format!(
                            "{}   {}{}",
                            st.server,
                            kind,
                            if qs.is_empty() {
                                String::new()
                            } else {
                                format!("  [{}]", qs.join(" "))
                            }
                        );
                        let r = ui.add_sized(
                            [ui.available_width(), 38.0],
                            egui::Button::new(RichText::new(label).color(TEXT)).fill(PANEL),
                        );
                        ui.painter().circle_filled(
                            pos2(r.rect.right() - 16.0, r.rect.center().y),
                            5.0,
                            col,
                        );
                        if r.clicked() {
                            chosen = Some(st.clone());
                        }
                    }
                }
            }
            ui.add_space(8.0);
            if widgets::ghost_button(ui, "ยกเลิก").clicked() {
                close = true;
            }
        });
        if resp.should_close() {
            close = true;
        }
        if let Some(st) = chosen {
            let (d, e) = (p.detail.clone(), p.ep.clone());
            self.pick = None;
            self.play(&d, &e, Some(st));
        } else if close {
            self.pick = None;
        }
    }
}

fn episode_row(ui: &mut Ui, e: &Episode, w: f32, watched: Option<f32>) -> Option<Act> {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 56.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let seen = watched.is_some_and(|n| e.num <= n + 0.001);
    let last = watched.is_some_and(|n| (e.num - n).abs() < 0.001);
    let hovered = resp.hovered();
    let p = ui.painter();
    p.rect_filled(
        rect,
        CornerRadius::same(4),
        if hovered { PANEL_HI } else { PANEL },
    );
    if last {
        p.rect_filled(
            Rect::from_min_size(rect.min, vec2(4.0, rect.height())),
            CornerRadius::ZERO,
            ACCENT,
        );
    }
    let c = pos2(rect.min.x + 30.0, rect.center().y);
    p.circle_filled(c, 15.0, if hovered { ACCENT } else { BAR });
    p.text(
        c + vec2(1.5, 0.0),
        Align2::CENTER_CENTER,
        icon::PLAY,
        regular(15.0),
        if hovered { Color32::BLACK } else { ACCENT },
    );
    let col = if seen && !hovered { MUTED } else { TEXT };
    let sub = if last {
        "ดูล่าสุด"
    } else if seen {
        "ดูแล้ว"
    } else {
        ""
    };
    let dy = if sub.is_empty() { 0.0 } else { -9.0 };
    p.text(
        pos2(rect.min.x + 56.0, rect.center().y + dy),
        Align2::LEFT_CENTER,
        &e.label,
        bold(15.0),
        col,
    );
    if !sub.is_empty() {
        p.text(
            pos2(rect.min.x + 56.0, rect.center().y + 10.0),
            Align2::LEFT_CENTER,
            sub,
            regular(12.0),
            if last { ACCENT } else { DIM },
        );
    }

    // server list button on the right
    let br = Rect::from_center_size(pos2(rect.max.x - 24.0, rect.center().y), vec2(32.0, 32.0));
    let b = ui
        .interact(br, ui.id().with(("srv", &e.url)), Sense::click())
        .on_hover_text("เลือกเซิร์ฟเวอร์เอง")
        .on_hover_cursor(CursorIcon::PointingHand);
    if b.hovered() {
        ui.painter().rect_stroke(
            br,
            CornerRadius::same(4),
            Stroke::new(1.0, LINE),
            StrokeKind::Inside,
        );
    }
    ui.painter().text(
        br.center(),
        Align2::CENTER_CENTER,
        icon::LIST_NUMBERS,
        regular(17.0),
        if b.hovered() { ACCENT } else { DIM },
    );
    if b.clicked() {
        return Some(Act::Pick(e.clone()));
    }
    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        return Some(Act::Play(e.clone()));
    }
    None
}
