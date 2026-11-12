//! Browse › Discover: every anime (AniList's catalogue) narrowed down by genre, year, season, format, status,
//! score and sorted as wanted. Picking one searches all the sources for it.

use crate::app::{Msg, WakuApp};
use crate::i18n::tr;
use crate::theme::*;
use crate::widgets;
use egui::{RichText, Ui, vec2};
use egui_phosphor::regular as icon;
use waku_core::AnimeCard;
use waku_core::meta::AnimeMeta;
use waku_core::meta::anilist::{Discover, GENRES};

#[derive(Default)]
pub struct DiscoverState {
    pub filter: Discover,
    /// the filter the list below belongs to
    pub shown: Option<Discover>,
    pub items: Vec<AnimeMeta>,
    pub page: u32,
    pub more: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// bumped per new query; stale answers are dropped
    pub seq: u64,
    /// text typed in the title box (applied after a pause)
    pub typed_at: Option<f64>,
}

fn season_name(s: &str) -> &'static str {
    match s {
        "WINTER" => tr("Winter", "ฤดูหนาว"),
        "SPRING" => tr("Spring", "ฤดูใบไม้ผลิ"),
        "SUMMER" => tr("Summer", "ฤดูร้อน"),
        "FALL" => tr("Fall", "ฤดูใบไม้ร่วง"),
        _ => tr("Any season", "ทุกฤดู"),
    }
}

fn format_name(f: &str) -> &'static str {
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

fn status_name(s: &str) -> &'static str {
    match s {
        "RELEASING" => tr("Airing", "กำลังฉาย"),
        "FINISHED" => tr("Finished", "จบแล้ว"),
        "NOT_YET_RELEASED" => tr("Upcoming", "เร็วๆ นี้"),
        _ => tr("Any status", "ทุกสถานะ"),
    }
}

fn sort_name(s: &str) -> &'static str {
    match s {
        "TRENDING_DESC" => tr("Trending", "กำลังมาแรง"),
        "SCORE_DESC" => tr("Highest score", "คะแนนสูงสุด"),
        "START_DATE_DESC" => tr("Newest", "ใหม่ล่าสุด"),
        "FAVOURITES_DESC" => tr("Most favourited", "คนชอบมากสุด"),
        _ => tr("Most popular", "ยอดนิยม"),
    }
}

fn genre_name(g: &str) -> String {
    let th = match g {
        "Action" => "แอ็กชัน",
        "Adventure" => "ผจญภัย",
        "Comedy" => "ตลก",
        "Drama" => "ดราม่า",
        "Ecchi" => "เอจจิ",
        "Fantasy" => "แฟนตาซี",
        "Horror" => "สยองขวัญ",
        "Mahou Shoujo" => "สาวน้อยเวทมนตร์",
        "Mecha" => "หุ่นยนต์",
        "Music" => "ดนตรี",
        "Mystery" => "ลึกลับ",
        "Psychological" => "จิตวิทยา",
        "Romance" => "โรแมนติก",
        "Sci-Fi" => "ไซไฟ",
        "Slice of Life" => "ชีวิตประจำวัน",
        "Sports" => "กีฬา",
        "Supernatural" => "เหนือธรรมชาติ",
        "Thriller" => "ระทึกขวัญ",
        _ => g,
    };
    tr(g, th).to_string()
}

/// The card shown for an AniList entry.
pub fn meta_card(m: &AnimeMeta, english: bool) -> AnimeCard {
    let title = if english { m.title_english.clone().or(m.title_romaji.clone()) } else { m.title_romaji.clone().or(m.title_english.clone()) }.unwrap_or_default();
    let badge = [m.year.map(|y| y.to_string()), m.format.as_deref().map(|f| format_name(f).to_string())].into_iter().flatten().collect::<Vec<_>>().join(" · ");
    AnimeCard {
        source: "anilist".into(),
        url: format!("https://anilist.co/anime/{}", m.anilist_id.unwrap_or(0)),
        title,
        poster: m.cover_url.clone(),
        badge: (!badge.is_empty()).then_some(badge),
    }
}

impl WakuApp {
    fn discover_load(&mut self, more: bool) {
        let d = &mut self.discover;
        if more {
            d.page += 1;
        } else {
            d.page = 1;
            d.items.clear();
            d.seq += 1;
        }
        d.loading = true;
        d.error = None;
        d.shown = Some(d.filter.clone());
        let (f, page, seq) = (d.filter.clone(), d.page, d.seq);
        self.spawn(move |m| Msg::Discover { seq, res: waku_core::meta::anilist::discover(m.http(), &f, page).map_err(|e| format!("{e:#}")) });
    }

    pub fn on_discover(&mut self, seq: u64, res: Result<(Vec<AnimeMeta>, bool), String>) {
        if seq != self.discover.seq {
            return;
        }
        let d = &mut self.discover;
        d.loading = false;
        match res {
            Ok((items, more)) => {
                for m in items {
                    if !d.items.iter().any(|x| x.anilist_id == m.anilist_id) {
                        d.items.push(m);
                    }
                }
                d.more = more;
            }
            Err(e) => d.error = Some(crate::app::first_line(&e).to_string()),
        }
    }

    pub fn discover_page(&mut self, ui: &mut Ui) {
        let now = ui.input(|i| i.time);
        let f = &mut self.discover.filter;

        // row 1: title, sort
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut f.search).hint_text(tr("Title (optional)", "ชื่อเรื่อง (ไม่ใส่ก็ได้)")).desired_width(260.0));
            if r.changed() {
                self.discover.typed_at = Some(now);
            }
            ui.add_space(8.0);
            egui::ComboBox::from_id_salt("d-sort").selected_text(format!("{}  {}", icon::SORT_DESCENDING, sort_name(&f.sort))).width(170.0).show_ui(ui, |ui| {
                for s in ["POPULARITY_DESC", "TRENDING_DESC", "SCORE_DESC", "START_DATE_DESC", "FAVOURITES_DESC"] {
                    let cur = if f.sort.is_empty() { "POPULARITY_DESC" } else { f.sort.as_str() };
                    if ui.selectable_label(cur == s, sort_name(s)).clicked() {
                        f.sort = s.into();
                    }
                }
            });
            egui::ComboBox::from_id_salt("d-year").selected_text(f.year.map(|y| y.to_string()).unwrap_or_else(|| tr("Any year", "ทุกปี").into())).width(100.0).show_ui(ui, |ui| {
                if ui.selectable_label(f.year.is_none(), tr("Any year", "ทุกปี")).clicked() {
                    f.year = None;
                }
                for y in (1970..=2027).rev() {
                    if ui.selectable_label(f.year == Some(y), y.to_string()).clicked() {
                        f.year = Some(y);
                    }
                }
            });
            egui::ComboBox::from_id_salt("d-season").selected_text(season_name(f.season.as_deref().unwrap_or(""))).width(120.0).show_ui(ui, |ui| {
                if ui.selectable_label(f.season.is_none(), season_name("")).clicked() {
                    f.season = None;
                }
                for s in ["WINTER", "SPRING", "SUMMER", "FALL"] {
                    if ui.selectable_label(f.season.as_deref() == Some(s), season_name(s)).clicked() {
                        f.season = Some(s.into());
                    }
                }
            });
            egui::ComboBox::from_id_salt("d-status").selected_text(status_name(f.status.as_deref().unwrap_or(""))).width(120.0).show_ui(ui, |ui| {
                if ui.selectable_label(f.status.is_none(), status_name("")).clicked() {
                    f.status = None;
                }
                for s in ["RELEASING", "FINISHED", "NOT_YET_RELEASED"] {
                    if ui.selectable_label(f.status.as_deref() == Some(s), status_name(s)).clicked() {
                        f.status = Some(s.into());
                    }
                }
            });
            let score_txt = f.min_score.map(|s| format!("★ {}+", s / 10)).unwrap_or_else(|| tr("Any score", "ทุกคะแนน").into());
            egui::ComboBox::from_id_salt("d-score").selected_text(score_txt).width(110.0).show_ui(ui, |ui| {
                if ui.selectable_label(f.min_score.is_none(), tr("Any score", "ทุกคะแนน")).clicked() {
                    f.min_score = None;
                }
                for s in [60, 70, 75, 80, 85] {
                    if ui.selectable_label(f.min_score == Some(s - 1), format!("★ {}+", s as f32 / 10.0)).clicked() {
                        f.min_score = Some(s - 1);
                    }
                }
            });
        });
        ui.add_space(8.0);
        // row 2: formats, then genres (several may be picked)
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for fm in ["TV", "MOVIE", "OVA", "ONA", "SPECIAL", "TV_SHORT"] {
                let on = f.formats.iter().any(|x| x == fm);
                if widgets::chip(ui, format_name(fm), on, None).clicked() {
                    if on {
                        f.formats.retain(|x| x != fm);
                    } else {
                        f.formats.push(fm.into());
                    }
                }
            }
            ui.add_space(10.0);
            ui.label(RichText::new("|").color(LINE));
            ui.add_space(10.0);
            for g in GENRES {
                let on = f.genres.iter().any(|x| x == g);
                if widgets::chip(ui, &genre_name(g), on, None).clicked() {
                    if on {
                        f.genres.retain(|x| x != g);
                    } else {
                        f.genres.push(g.into());
                    }
                }
            }
            let any = f != &Discover::default();
            if any && widgets::ghost_button(ui, &format!("{}  {}", icon::X, tr("Clear filters", "ล้างตัวกรอง"))).clicked() {
                *f = Discover::default();
            }
        });
        ui.add_space(14.0);

        // run the query: right away for clicks, after a pause for typing
        let typing = self.discover.typed_at.is_some_and(|t| now - t < 0.6);
        if typing {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        }
        if self.discover.shown.as_ref() != Some(&self.discover.filter) && !typing {
            self.discover.typed_at = None;
            self.discover_load(false);
        }

        if let Some(e) = self.discover.error.clone() {
            widgets::error_box(ui, &e);
            ui.add_space(8.0);
        }
        let english = self.settings.language != "th";
        let items = self.discover.items.clone();
        if items.is_empty() && self.discover.loading {
            widgets::card_grid(ui, &[(); 12], |ui, _| widgets::card_skeleton(ui));
            return;
        }
        if items.is_empty() && !self.discover.loading && self.discover.error.is_none() {
            widgets::empty(ui, icon::FUNNEL, tr("Nothing matches these filters", "ไม่มีเรื่องที่ตรงกับตัวกรองนี้"));
            return;
        }
        let mut pick: Option<AnimeMeta> = None;
        widgets::card_grid(ui, &items, |ui, m| {
            let card = meta_card(m, english);
            let sub = [
                m.score.map(|s| format!("★ {:.1}", s as f32 / 10.0)),
                m.episodes.map(|e| crate::trf!("{e} eps", "{e} ตอน")),
                m.genres.first().map(|g| genre_name(g)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let r = widgets::anime_card(ui, &mut self.images, &card, "", &sub);
            let r = r.on_hover_text(m.description.clone().unwrap_or_default().chars().take(400).collect::<String>());
            if r.clicked() {
                pick = Some(m.clone());
            }
        });
        ui.add_space(14.0);
        if self.discover.loading {
            widgets::loading(ui, tr("Loading…", "กำลังโหลด…"));
        } else if self.discover.more {
            ui.vertical_centered(|ui| {
                if widgets::outline_button(ui, tr("Load more", "โหลดเพิ่ม")).clicked() {
                    self.discover_load(true);
                }
            });
        }
        if let Some(m) = pick {
            // find it on the sources (the romaji title matches Thai sites best)
            let q = m.title_romaji.clone().or(m.title_english.clone()).unwrap_or_default();
            self.search_box = q.clone();
            self.remember_search(&q);
            self.run_search(&q);
            self.go(crate::app::Page::Search);
        }
    }
}
