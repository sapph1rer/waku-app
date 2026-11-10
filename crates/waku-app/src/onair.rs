//! "On Air": AniList's public airing schedule by day (local time), with live / upcoming /
//! aired state and a countdown. Clicking a show searches every source for it.
//! Schedule logic ported from AnimeShelf's `onair.rs`.

use crate::app::WakuApp;
use crate::i18n::{self, tr};
use crate::theme::*;
use crate::trf;
use crate::widgets;
use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use egui::{
    Color32, CornerRadius, CursorIcon, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::collections::HashSet;
use waku_core::meta::anilist::SchedItem;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AirState {
    Unaired,
    OnAir,
    Aired,
}

/// Where an episode airing at `at` stands at `now` (it counts as on air for its running time).
pub fn state_of(at: u64, duration_min: Option<u32>, now: u64) -> AirState {
    if at > now {
        AirState::Unaired
    } else if now < at + duration_min.unwrap_or(24).max(1) as u64 * 60 {
        AirState::OnAir
    } else {
        AirState::Aired
    }
}

fn date_of(ts: u64) -> NaiveDate {
    Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .map(|d| d.date_naive())
        .unwrap_or_default()
}

fn midnight(d: NaiveDate) -> u64 {
    d.and_hms_opt(0, 0, 0)
        .and_then(|t| Local.from_local_datetime(&t).earliest())
        .map_or(0, |t| t.timestamp().max(0) as u64)
}

/// Local midnight of the day containing `ts`.
pub fn day_start(ts: u64) -> u64 {
    midnight(date_of(ts))
}

pub fn shift_day(day: u64, n: i64) -> u64 {
    midnight(date_of(day) + chrono::Duration::days(n))
}

/// Monday 00:00 (local) of the week containing `day`.
pub fn week_start(day: u64) -> u64 {
    let d = date_of(day);
    midnight(d - chrono::Duration::days(d.weekday().num_days_from_monday() as i64))
}

fn weekday(day: u64) -> u32 {
    date_of(day).weekday().num_days_from_monday()
}

fn hhmm(ts: u64) -> String {
    Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .map_or(String::new(), |t| {
            format!("{:02}:{:02}", t.hour(), t.minute())
        })
}

fn utc_offset() -> String {
    let secs = Local::now().offset().local_minus_utc();
    let (h, m) = (secs / 3600, (secs.abs() / 60) % 60);
    if m == 0 {
        format!("UTC{h:+}")
    } else {
        format!("UTC{h:+}:{m:02}")
    }
}

enum RowAct {
    /// (AniList id, title to search for)
    Find(u64, String),
    Open(u64),
}

impl WakuApp {
    fn sched_load(&mut self, week: u64) {
        if self.sched.contains_key(&week) {
            return;
        }
        self.sched.insert(week, None);
        let to = shift_day(week, 7);
        self.spawn(move |m| crate::app::Msg::Schedule {
            week,
            res: waku_core::meta::anilist::schedule(m.http(), week, to)
                .map_err(|e| format!("{e:#}")),
        });
    }

    /// A known series page (history, my list, loaded listings) matched to this AniList id.
    fn card_with_anilist_id(&self, id: u64) -> Option<waku_core::AnimeCard> {
        let hist = self.lib.history.iter().map(|h| &h.card);
        let lists = self.listings.values().flat_map(|l| l.cards.iter());
        hist.chain(self.lib.my_list.iter())
            .chain(lists)
            .find(|c| self.meta_of(&c.url, &c.title).and_then(|m| m.anilist_id) == Some(id))
            .cloned()
    }

    /// AniList ids of the shows in My List and History (for "My shows").
    fn my_anilist_ids(&self) -> HashSet<u64> {
        let pages = self
            .lib
            .my_list
            .iter()
            .map(|c| (c.url.as_str(), c.title.as_str()))
            .chain(
                self.lib
                    .history
                    .iter()
                    .map(|h| (h.card.url.as_str(), h.card.title.as_str())),
            );
        self.meta.anilist_ids(pages).into_iter().collect()
    }

    pub fn onair_page(&mut self, ui: &mut Ui) {
        let now = waku_core::store::now();
        let today = day_start(now);
        let day = self.sched_day;
        let week = week_start(day);
        self.sched_load(week);
        // keep the next week warm so "tomorrow" on a Sunday is instant
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(30));

        // keyboard: ← → change day
        if !ui.ctx().egui_wants_keyboard_input() {
            let (l, r) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowLeft),
                    i.key_pressed(egui::Key::ArrowRight),
                )
            });
            if l {
                self.sched_day = shift_day(day, -1);
            }
            if r {
                self.sched_day = shift_day(day, 1);
            }
        }

        // ---------------------------------------------------------- header
        let d = date_of(day);
        let when = if day == today {
            tr("Today", "วันนี้").to_string()
        } else {
            i18n::weekday_long(weekday(day)).to_string()
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::CALENDAR_DOTS).size(28.0).color(ACCENT));
            ui.label(RichText::new(tr("On Air", "ตารางฉาย")).font(bold(28.0)));
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "{when} · {:02}/{:02}/{}",
                    d.day(),
                    d.month(),
                    d.year()
                ))
                .size(16.0)
                .color(MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::ghost_button(
                    ui,
                    &format!("{}  {}", tr("Next week", "สัปดาห์หน้า"), icon::CARET_RIGHT),
                )
                .clicked()
                {
                    self.sched_day = shift_day(day, 7);
                }
                if day != today && widgets::outline_button(ui, tr("Today", "วันนี้")).clicked()
                {
                    self.sched_day = today;
                }
                if widgets::ghost_button(
                    ui,
                    &format!("{}  {}", icon::CARET_LEFT, tr("Previous week", "สัปดาห์ก่อน")),
                )
                .clicked()
                {
                    self.sched_day = shift_day(day, -7);
                }
            });
        });
        ui.add_space(12.0);

        let week_items: Vec<SchedItem> = match self.sched.get(&week) {
            Some(Some(Ok(v))) => v
                .iter()
                .filter(|it| self.settings.show_adult || !it.adult)
                .cloned()
                .collect(),
            _ => Vec::new(),
        };

        // ---------------------------------------------------------- day strip
        let gap = 10.0;
        let tile_w = ((ui.available_width() - gap * 6.0) / 7.0).max(70.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for i in 0..7 {
                let dd = shift_day(week, i);
                let n = week_items
                    .iter()
                    .filter(|it| day_start(it.at) == dd)
                    .count();
                let (r, resp) = ui.allocate_exact_size(vec2(tile_w, 66.0), Sense::click());
                let sel = dd == day;
                let p = ui.painter();
                p.rect_filled(
                    r,
                    CornerRadius::same(10),
                    if sel {
                        SELECT
                    } else if resp.hovered() {
                        PANEL
                    } else {
                        BAR
                    },
                );
                if sel {
                    p.rect_stroke(
                        r,
                        CornerRadius::same(10),
                        Stroke::new(1.5, ACCENT),
                        StrokeKind::Inside,
                    );
                }
                let dt = date_of(dd);
                let col = if sel { ACCENT_HI } else { TEXT };
                p.text(
                    pos2(r.min.x + 14.0, r.min.y + 20.0),
                    egui::Align2::LEFT_CENTER,
                    i18n::weekday_short(i as u32),
                    bold(14.0),
                    col,
                );
                p.text(
                    pos2(r.min.x + 14.0, r.min.y + 45.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{}/{}", dt.day(), dt.month()),
                    regular(13.0),
                    MUTED,
                );
                if n > 0 {
                    p.text(
                        pos2(r.max.x - 12.0, r.min.y + 45.0),
                        egui::Align2::RIGHT_CENTER,
                        n.to_string(),
                        regular(12.0),
                        DIM,
                    );
                }
                if dd == today {
                    p.circle_filled(pos2(r.max.x - 14.0, r.min.y + 18.0), 4.0, ACCENT);
                }
                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    self.sched_day = dd;
                }
            }
        });
        ui.add_space(12.0);

        // ---------------------------------------------------------- filters
        let mine = self.my_anilist_ids();
        let day_items: Vec<&SchedItem> = week_items
            .iter()
            .filter(|it| day_start(it.at) == day)
            .collect();
        let flt = self.sched_filter;
        let shown: Vec<&SchedItem> = day_items
            .iter()
            .copied()
            .filter(|it| {
                let st = state_of(it.at, it.duration, now);
                (flt == 0
                    || (flt == 1 && st == AirState::OnAir)
                    || (flt == 2 && st == AirState::Unaired)
                    || (flt == 3 && st == AirState::Aired))
                    && (!self.sched_mine || mine.contains(&it.id))
            })
            .collect();
        ui.horizontal_wrapped(|ui| {
            for (i, l) in [
                tr("All", "ทั้งหมด"),
                tr("Airing now", "กำลังฉาย"),
                tr("Upcoming", "ยังไม่ฉาย"),
                tr("Aired", "ฉายแล้ว"),
            ]
            .into_iter()
            .enumerate()
            {
                if widgets::chip(ui, l, flt == i as u8, None).clicked() {
                    self.sched_filter = i as u8;
                }
            }
            ui.add_space(8.0);
            let l = format!("{}  {}", icon::BOOKMARK_SIMPLE, tr("My shows", "เรื่องของฉัน"));
            if widgets::chip(ui, &l, self.sched_mine, None)
                .on_hover_text(tr("Shows in My List or History", "เรื่องที่อยู่ในรายการหรือประวัติ"))
                .clicked()
            {
                self.sched_mine = !self.sched_mine;
            }
            ui.add_space(8.0);
            let n = shown.len();
            let tz = utc_offset();
            ui.label(
                RichText::new(trf!(
                    "{n} shows · times in your time zone ({tz})",
                    "{n} เรื่อง · เวลาตามเครื่องของคุณ ({tz})"
                ))
                .size(12.5)
                .color(DIM),
            );
        });
        ui.add_space(14.0);

        // ---------------------------------------------------------- list
        match self.sched.get(&week) {
            None | Some(None) => {
                widgets::loading(
                    ui,
                    tr(
                        "Loading the schedule from AniList…",
                        "กำลังโหลดตารางฉายจาก AniList…",
                    ),
                );
                return;
            }
            Some(Some(Err(e))) => {
                let e = crate::app::first_line(e);
                widgets::error_box(ui, &e);
                if widgets::outline_button(
                    ui,
                    &format!("{}  {}", icon::ARROWS_CLOCKWISE, tr("Retry", "ลองใหม่")),
                )
                .clicked()
                {
                    self.sched.remove(&week);
                }
                return;
            }
            _ => {}
        }
        if shown.is_empty() {
            widgets::empty(
                ui,
                icon::CALENDAR_BLANK,
                tr(
                    "Nothing airs here with these filters",
                    "ไม่มีเรื่องที่ฉายตามตัวกรองนี้",
                ),
            );
            return;
        }

        let mut act: Option<RowAct> = None;
        let mut now_drawn = day != today;
        ui.spacing_mut().item_spacing.y = 10.0;
        for it in shown {
            if !now_drawn && it.at > now {
                now_drawn = true;
                now_line(ui, now);
            }
            if let Some(a) = self.sched_row(ui, it, now, mine.contains(&it.id)) {
                act = Some(a);
            }
        }
        if !now_drawn {
            now_line(ui, now);
        }

        match act {
            Some(RowAct::Find(id, q)) => {
                // already matched in the app (history, my list, a listing)? open it directly
                match self.card_with_anilist_id(id) {
                    Some(c) => self.open(&c),
                    None => {
                        self.search_box = q.clone();
                        self.run_search(&q);
                    }
                }
            }
            Some(RowAct::Open(id)) => ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                "https://anilist.co/anime/{id}"
            ))),
            None => {}
        }
    }

    fn sched_row(&mut self, ui: &mut Ui, it: &SchedItem, now: u64, mine: bool) -> Option<RowAct> {
        let w = ui.available_width();
        let (r, resp) = ui.allocate_exact_size(vec2(w, 104.0), Sense::click());
        if !ui.is_rect_visible(r) {
            return None;
        }
        let st = state_of(it.at, it.duration, now);
        let hovered = resp.hovered();

        // time column
        let p = ui.painter();
        let tc = if st == AirState::Aired { MUTED } else { TEXT };
        p.text(
            pos2(r.min.x, r.min.y + 30.0),
            egui::Align2::LEFT_CENTER,
            hhmm(it.at),
            bold(22.0),
            tc,
        );
        let (sl, sc) = match st {
            AirState::OnAir => (format!("● {}", tr("ON AIR", "กำลังฉาย")), ACCENT_HI),
            AirState::Unaired => {
                let eta = i18n::fmt_eta(it.at as i64 - now as i64);
                (trf!("in {eta}", "อีก {eta}"), Color32::from_gray(200))
            }
            AirState::Aired => (tr("Aired", "ฉายแล้ว").to_string(), DIM),
        };
        p.text(
            pos2(r.min.x, r.min.y + 58.0),
            egui::Align2::LEFT_CENTER,
            sl,
            regular(12.5),
            sc,
        );

        // card
        let card = Rect::from_min_max(pos2(r.min.x + 96.0, r.min.y), r.max);
        p.rect_filled(
            card,
            CornerRadius::same(10),
            if hovered { CARD_HI } else { CARD },
        );
        if st == AirState::OnAir {
            p.rect_stroke(
                card,
                CornerRadius::same(10),
                Stroke::new(1.5, ACCENT),
                StrokeKind::Inside,
            );
        }
        if mine {
            p.rect_filled(
                Rect::from_min_size(card.min, vec2(4.0, card.height())),
                CornerRadius::same(2),
                ACCENT,
            );
        }
        let cr = Rect::from_min_size(card.min + vec2(12.0, 9.0), vec2(60.0, 86.0));
        widgets::poster(
            ui,
            &mut self.images,
            it.cover.as_deref(),
            "https://anilist.co/",
            cr,
            6,
        );
        let p = ui.painter();
        let x = cr.max.x + 16.0;
        let tw = card.max.x - x - 150.0;
        let g = widgets::wrapped(ui, &it.title, bold(16.0), TEXT, tw, 1);
        p.galley(pos2(x, card.min.y + 14.0), g, TEXT);
        if let Some(rj) = it.title_romaji.as_deref().filter(|t| *t != it.title) {
            let g = widgets::wrapped(ui, rj, regular(12.5), MUTED, tw, 1);
            p.galley(pos2(x, card.min.y + 39.0), g, MUTED);
        }
        let mut info = vec![match it.total {
            Some(t) => {
                let (e, t) = (it.episode, t);
                trf!("Episode {e} / {t}", "ตอนที่ {e} / {t}")
            }
            None => {
                let e = it.episode;
                trf!("Episode {e}", "ตอนที่ {e}")
            }
        }];
        if let Some(f) = &it.format {
            info.push(i18n::format_name(f));
        }
        if let Some(d) = it.duration {
            info.push(trf!("{d} min", "{d} นาที"));
        }
        if let Some(s) = it.score {
            info.push(format!("★ {s}%"));
        }
        if it.adult {
            info.push("18+".into());
        }
        let g = widgets::wrapped(
            ui,
            &info.join("  ·  "),
            regular(13.0),
            Color32::from_gray(205),
            tw,
            1,
        );
        p.galley(pos2(x, card.min.y + 64.0), g, Color32::from_gray(205));

        // find button
        let br =
            Rect::from_center_size(pos2(card.max.x - 70.0, card.center().y), vec2(112.0, 36.0));
        let b = ui
            .interact(
                br,
                ui.id().with(("find", it.id, it.episode)),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        let p = ui.painter();
        p.rect_filled(
            br,
            CornerRadius::same(6),
            if b.hovered() { ACCENT } else { PANEL_HI },
        );
        p.text(
            br.center(),
            egui::Align2::CENTER_CENTER,
            format!("{}  {}", icon::MAGNIFYING_GLASS, tr("Find", "หาดู")),
            bold(13.5),
            TEXT,
        );

        // Thai sites mostly list the English name (it.title is English when AniList has one)
        let query = it.title.clone();
        let mut act = None;
        resp.context_menu(|ui| {
            if ui
                .button(format!(
                    "{}  {}",
                    icon::MAGNIFYING_GLASS,
                    tr("Search sources", "ค้นหาในทุกแหล่ง")
                ))
                .clicked()
            {
                act = Some(RowAct::Find(it.id, query.clone()));
                ui.close();
            }
            if ui
                .button(format!(
                    "{}  {}",
                    icon::ARROW_SQUARE_OUT,
                    tr("Open on AniList", "เปิดใน AniList")
                ))
                .clicked()
            {
                act = Some(RowAct::Open(it.id));
                ui.close();
            }
        });
        if b.clicked()
            || resp
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(tr(
                    "Search every source for this show",
                    "ค้นหาเรื่องนี้จากทุกแหล่ง",
                ))
                .clicked()
        {
            act = Some(RowAct::Find(it.id, query));
        }
        act
    }
}

fn now_line(ui: &mut Ui, now: u64) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let p = ui.painter();
    let y = r.center().y;
    p.text(
        pos2(r.min.x, y),
        egui::Align2::LEFT_CENTER,
        format!("{} {}", tr("Now", "ตอนนี้"), hhmm(now)),
        bold(12.5),
        ACCENT_HI,
    );
    p.hline(r.min.x + 80.0..=r.max.x, y, Stroke::new(1.5, ACCENT));
    p.circle_filled(pos2(r.min.x + 80.0, y), 4.0, ACCENT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states() {
        assert_eq!(state_of(100, Some(24), 50), AirState::Unaired);
        assert_eq!(state_of(100, Some(24), 100 + 60), AirState::OnAir);
        assert_eq!(state_of(100, Some(24), 100 + 25 * 60), AirState::Aired);
    }

    #[test]
    fn weeks_start_on_monday() {
        let now = waku_core::store::now();
        let w = week_start(now);
        assert_eq!(weekday(w), 0);
        assert!(w <= now && now < shift_day(w, 7));
        assert_eq!(week_start(shift_day(w, 6)), w);
    }
}
