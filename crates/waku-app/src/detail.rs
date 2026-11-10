//! Title page in the style of the reference: full-bleed hero with backdrop art,
//! tabs, a season column on the left and episode cards on the right.

use crate::app::{Alts, Tab, WakuApp};
use crate::i18n::{self, tr};
use crate::theme::*;
use crate::trf;
use crate::widgets::{self, EpCard, backdrop, badge, episode_card};
use egui::{
    Color32, CornerRadius, CursorIcon, Frame, Margin, Rect, RichText, Sense, Stroke, Ui, pos2, vec2,
};
use egui_phosphor::regular as icon;
use waku_core::text::{fmt_num, truncate};
use waku_core::{AnimeCard, AnimeDetail, Episode, StreamRank};

const PAD: f32 = 36.0;
const HERO_H: f32 = 440.0;

enum Act {
    Play(Episode),
    Pick(Episode),
    Open(AnimeCard),
    ToggleList,
    Tab(Tab),
    Season(u32),
    Sort,
    Back,
    SearchElsewhere,
}

impl WakuApp {
    pub fn detail_page(&mut self, ui: &mut Ui) {
        let Some(st) = &self.detail else {
            Frame::new()
                .inner_margin(Margin::same(PAD as i8))
                .show(ui, |ui| {
                    widgets::empty(
                        ui,
                        icon::TELEVISION,
                        tr("No title selected", "ยังไม่ได้เลือกเรื่อง"),
                    )
                });
            return;
        };
        let card = st.card.clone();
        let data: Option<Result<AnimeDetail, String>> = st.data.clone();
        let (tab, sel_season, newest_first) = (st.tab, st.season, st.newest_first);
        let mut act: Option<Act> = None;

        let referer = self.referer(&card.source);
        let art = data
            .as_ref()
            .and_then(|d| d.as_ref().ok())
            .and_then(|d| d.poster.clone())
            .or(card.poster.clone());
        let last = data
            .as_ref()
            .and_then(|d| d.as_ref().ok())
            .and_then(|d| self.lib.last_watched(&d.url, &d.title).cloned());

        // ---------------------------------------------------------- hero
        let w = ui.available_width();
        let (hero, _) = ui.allocate_exact_size(vec2(w, HERO_H), Sense::hover());
        backdrop(ui, &mut self.images, art.as_deref(), &referer, hero);

        // back button (top-left over the art)
        let br = Rect::from_min_size(hero.min + vec2(PAD - 6.0, 18.0), vec2(40.0, 40.0));
        let b = ui
            .interact(br, ui.id().with("back"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        ui.painter().circle_filled(
            br.center(),
            18.0,
            if b.hovered() {
                Color32::from_black_alpha(220)
            } else {
                Color32::from_black_alpha(140)
            },
        );
        ui.painter().text(
            br.center(),
            egui::Align2::CENTER_CENTER,
            icon::ARROW_LEFT,
            regular(18.0),
            TEXT,
        );
        if b.clicked() {
            act = Some(Act::Back);
        }

        let text_w = (w * 0.46).clamp(380.0, 640.0);
        let mut h = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(
            pos2(hero.min.x + PAD, hero.min.y + 84.0),
            vec2(text_w, HERO_H - 100.0),
        )));
        let title = data
            .as_ref()
            .and_then(|d| d.as_ref().ok())
            .map(|d| d.title.clone())
            .unwrap_or(card.title.clone());
        widgets::clamp_label(&mut h, &title, bold(38.0), TEXT, 2);
        h.add_space(10.0);

        match &data {
            None => {
                let sn = self.source_name(&card.source);
                widgets::loading(&mut h, &trf!("Loading from {sn} …", "กำลังโหลดจาก {sn} …"));
            }
            Some(Err(e)) => {
                widgets::error_box(&mut h, &crate::app::first_line(e));
                h.add_space(8.0);
                if widgets::outline_button(
                    &mut h,
                    &format!(
                        "{}  {}",
                        icon::SWAP,
                        tr("Find on other sources", "หาเรื่องนี้จากแหล่งอื่น")
                    ),
                )
                .clicked()
                {
                    act = Some(Act::SearchElsewhere);
                }
            }
            Some(Ok(d)) => {
                let seasons = seasons_of(d);
                // badges row
                h.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    let mut first = true;
                    for t in d
                        .tags
                        .iter()
                        .filter(|t| ["Sub", "Dub", "Ongoing", "Completed"].contains(&t.as_str()))
                    {
                        badge(ui, &i18n::tag(t), first);
                        first = false;
                    }
                    badge(ui, tr("Series", "ซีรีส์"), first);
                    let (s, n) = (seasons.len(), d.episodes.len());
                    let info = if s > 1 {
                        trf!("{s} Seasons · {n} Episodes", "{s} ซีซั่น · {n} ตอน")
                    } else {
                        trf!("{n} Episodes", "{n} ตอน")
                    };
                    badge(ui, &info, false);
                });
                h.add_space(12.0);
                if !d.synopsis.is_empty() {
                    widgets::clamp_label(
                        &mut h,
                        &d.synopsis,
                        regular(15.0),
                        Color32::from_gray(222),
                        3,
                    );
                    h.add_space(10.0);
                }
                // source + genres line
                h.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(
                        RichText::new("●")
                            .color(self.health_of(&d.source))
                            .size(12.0),
                    );
                    ui.label(
                        RichText::new(self.source_name(&d.source))
                            .size(13.5)
                            .color(Color32::from_gray(200)),
                    );
                    for g in d
                        .tags
                        .iter()
                        .filter(|t| !["Sub", "Dub", "Ongoing", "Completed"].contains(&t.as_str()))
                    {
                        ui.label(RichText::new("·").color(DIM));
                        ui.label(RichText::new(g).size(13.5).color(MUTED));
                    }
                });
                h.add_space(18.0);

                // primary actions
                let resume = last.as_ref().and_then(|hst| {
                    let i = d
                        .episodes
                        .iter()
                        .position(|e| e.url == hst.ep_url || (e.num - hst.ep_num).abs() < 0.01)?;
                    d.episodes.get(i + 1).or(d.episodes.get(i)).cloned()
                });
                let in_list = self.lib.in_list(&d.url);
                h.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    match (&resume, d.episodes.first()) {
                        (Some(e), _) => {
                            let s = i18n::ep_short(e);
                            if widgets::primary_button(
                                ui,
                                &format!("{}  {}", icon::PLAY, trf!("Continue {s}", "ดูต่อ {s}")),
                            )
                            .clicked()
                            {
                                act = Some(Act::Play(e.clone()));
                            }
                        }
                        (None, Some(e)) => {
                            let s = i18n::ep_short(e);
                            if widgets::primary_button(
                                ui,
                                &format!(
                                    "{}  {}",
                                    icon::PLAY,
                                    trf!("Start Watching {s}", "เริ่มดู {s}")
                                ),
                            )
                            .clicked()
                            {
                                act = Some(Act::Play(e.clone()));
                            }
                        }
                        (None, None) => {
                            ui.label(
                                RichText::new(tr(
                                    "No episodes on this source yet",
                                    "ยังไม่มีตอนในแหล่งนี้",
                                ))
                                .color(MUTED),
                            );
                        }
                    }
                    let label = if in_list {
                        format!("{}  {}", icon::CHECK, tr("In My List", "อยู่ในรายการ"))
                    } else {
                        format!("{}  {}", icon::PLUS, tr("My List", "รายการของฉัน"))
                    };
                    if widgets::secondary_button(ui, &label).clicked() {
                        act = Some(Act::ToggleList);
                    }
                });
            }
        }

        // ---------------------------------------------------------- tabs + body
        if let Some(Ok(d)) = &data {
            Frame::new()
                .inner_margin(Margin {
                    left: PAD as i8,
                    right: PAD as i8,
                    top: 4,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    let tabs = [Tab::Episodes, Tab::Details, Tab::Sources, Tab::More];
                    let labels: Vec<String> = vec![
                        tr("Episodes", "ตอน").into(),
                        tr("Details", "รายละเอียด").into(),
                        tr("Other Sources", "แหล่งอื่น").into(),
                        tr("More Like This", "เรื่องอื่นๆ").into(),
                    ];
                    let cur = tabs.iter().position(|t| *t == tab).unwrap_or(0);
                    let new = widgets::tabs(ui, &labels, cur);
                    if new != cur {
                        act = Some(Act::Tab(tabs[new]));
                    }
                    match tab {
                        Tab::Episodes => self.episodes_tab(
                            ui,
                            d,
                            &referer,
                            art.as_deref(),
                            sel_season,
                            newest_first,
                            last.as_ref().map(|h| h.ep_num),
                            &mut act,
                        ),
                        Tab::Details => self.details_tab(ui, d),
                        Tab::Sources => self.sources_tab(ui, &mut act),
                        Tab::More => self.more_tab(ui, d, &mut act),
                    }
                });
        }

        // ---------------------------------------------------------- actions
        match act {
            Some(Act::Play(e)) => {
                if let Some(Ok(d)) = &data {
                    self.play(d, &e, None)
                }
            }
            Some(Act::Pick(e)) => {
                if let Some(Ok(d)) = &data {
                    self.pick_server(d, &e)
                }
            }
            Some(Act::Open(c)) => self.open(&c),
            Some(Act::ToggleList) => {
                if let Some(Ok(d)) = &data {
                    let mut c = d.card();
                    c.poster = c.poster.or(card.poster.clone());
                    self.lib.toggle_list(&c);
                }
            }
            Some(Act::Tab(t)) => {
                if let Some(s) = &mut self.detail {
                    s.tab = t;
                }
                if t == Tab::Sources {
                    self.find_alts();
                }
            }
            Some(Act::Season(n)) => {
                if let Some(s) = &mut self.detail {
                    s.season = Some(n);
                }
            }
            Some(Act::Sort) => {
                if let Some(s) = &mut self.detail {
                    s.newest_first = !s.newest_first;
                }
            }
            Some(Act::Back) => self.go_back(),
            Some(Act::SearchElsewhere) => {
                let q = card.title.clone();
                self.search_box = q.clone();
                self.run_search(&q);
            }
            None => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn episodes_tab(
        &mut self,
        ui: &mut Ui,
        d: &AnimeDetail,
        referer: &str,
        art: Option<&str>,
        sel: Option<u32>,
        newest_first: bool,
        watched: Option<f32>,
        act: &mut Option<Act>,
    ) {
        let seasons = seasons_of(d);
        if d.episodes.is_empty() {
            widgets::empty(
                ui,
                icon::LIST_NUMBERS,
                tr(
                    "No episode list found — try Other Sources, or test this source in Settings",
                    "ไม่พบรายชื่อตอน — ลองแท็บแหล่งอื่น หรือทดสอบแหล่งในหน้าตั้งค่า",
                ),
            );
            return;
        }
        let season = sel
            .filter(|s| seasons.iter().any(|x| x.0 == *s))
            .unwrap_or(seasons[0].0);
        let src_name = self.source_name(&d.source);
        let sub = if d.tags.iter().any(|t| t == "Dub") {
            i18n::tag("Dub")
        } else {
            i18n::tag("Sub")
        };

        ui.horizontal_top(|ui| {
            // season column
            let col_w = 220.0;
            ui.allocate_ui_with_layout(
                vec2(col_w, 10.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(col_w);
                    Frame::new()
                        .fill(BAR)
                        .corner_radius(CornerRadius::same(8))
                        .show(ui, |ui| {
                            ui.set_width(col_w);
                            for (i, (s, n)) in seasons.iter().enumerate() {
                                let (r, resp) =
                                    ui.allocate_exact_size(vec2(col_w, 62.0), Sense::click());
                                let active = *s == season;
                                let p = ui.painter();
                                if active {
                                    p.rect_filled(r, CornerRadius::same(8), SELECT);
                                    p.rect_filled(
                                        Rect::from_min_size(r.min, vec2(3.0, r.height())),
                                        CornerRadius::ZERO,
                                        ACCENT,
                                    );
                                } else if resp.hovered() {
                                    p.rect_filled(r, CornerRadius::same(8), PANEL);
                                }
                                let sn = trf!("Season {s}", "ซีซั่น {s}");
                                let en = trf!("{n} Episodes", "{n} ตอน");
                                p.text(
                                    r.min + vec2(18.0, 20.0),
                                    egui::Align2::LEFT_CENTER,
                                    sn,
                                    bold(15.0),
                                    TEXT,
                                );
                                p.text(
                                    r.min + vec2(18.0, 42.0),
                                    egui::Align2::LEFT_CENTER,
                                    en,
                                    regular(12.5),
                                    MUTED,
                                );
                                if i + 1 < seasons.len() && !active {
                                    p.hline(
                                        r.min.x + 12.0..=r.max.x - 12.0,
                                        r.max.y,
                                        Stroke::new(1.0, LINE),
                                    );
                                }
                                if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    *act = Some(Act::Season(*s));
                                }
                            }
                        });
                },
            );
            ui.add_space(18.0);

            // episode cards
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let label = if newest_first {
                        tr("Newest first", "ล่าสุดก่อน")
                    } else {
                        tr("Oldest first", "ตอนแรกก่อน")
                    };
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::ghost_button(ui, &format!("{}  {label}", icon::SORT_ASCENDING))
                            .clicked()
                        {
                            *act = Some(Act::Sort);
                        }
                    });
                });
                let mut eps: Vec<&Episode> =
                    d.episodes.iter().filter(|e| e.season == season).collect();
                if newest_first {
                    eps.reverse();
                }
                ui.spacing_mut().item_spacing.y = 10.0;
                for e in eps {
                    let seen = watched.is_some_and(|n| e.num <= n + 0.001);
                    let is_last = watched.is_some_and(|n| (e.num - n).abs() < 0.001);
                    let n = fmt_num(e.num);
                    let name = i18n::ep_name(e);
                    let title = format!("{n}. {name}");
                    let meta = if is_last {
                        trf!(
                            "{sub} · {src_name} · last watched",
                            "{sub} · {src_name} · ดูล่าสุด"
                        )
                    } else if seen {
                        trf!("{sub} · {src_name} · watched", "{sub} · {src_name} · ดูแล้ว")
                    } else {
                        format!("{sub} · {src_name}")
                    };
                    let c = EpCard {
                        title: &title,
                        meta: &meta,
                        desc: "",
                        progress: seen.then_some(1.0),
                        selected: is_last,
                    };
                    let (play, srv) = episode_card(ui, &mut self.images, art, referer, &c, &e.url);
                    if play {
                        *act = Some(Act::Play(e.clone()));
                    }
                    if srv {
                        *act = Some(Act::Pick(e.clone()));
                    }
                }
            });
        });
    }

    fn details_tab(&mut self, ui: &mut Ui, d: &AnimeDetail) {
        if !d.synopsis.is_empty() {
            ui.label(RichText::new(tr("Synopsis", "เรื่องย่อ")).font(bold(17.0)));
            ui.add_space(4.0);
            ui.label(RichText::new(&d.synopsis).color(Color32::from_gray(215)));
            ui.add_space(16.0);
        }
        egui::Grid::new("details_grid")
            .num_columns(2)
            .spacing([30.0, 10.0])
            .show(ui, |ui| {
                let row = |ui: &mut Ui, k: &str, v: &str| {
                    ui.label(RichText::new(k).color(MUTED));
                    ui.label(v);
                    ui.end_row();
                };
                row(ui, tr("Source", "แหล่ง"), &self.source_name(&d.source));
                row(
                    ui,
                    tr("Episodes", "จำนวนตอน"),
                    &d.episodes.len().to_string(),
                );
                row(ui, tr("Seasons", "ซีซั่น"), &seasons_of(d).len().to_string());
                let tags: Vec<String> = d.tags.iter().map(|t| i18n::tag(t)).collect();
                row(ui, tr("Tags", "แท็ก"), &tags.join(", "));
                ui.label(RichText::new(tr("Page", "หน้าเว็บ")).color(MUTED));
                ui.hyperlink_to(truncate(&d.url, 80), &d.url);
                ui.end_row();
            });
    }

    fn sources_tab(&mut self, ui: &mut Ui, act: &mut Option<Act>) {
        let Some(st) = &self.detail else { return };
        match &st.alts {
            Alts::Idle | Alts::Loading => {
                widgets::loading(ui, tr("Searching other sources…", "กำลังค้นหาในแหล่งอื่น…"))
            }
            Alts::Done(list) if list.is_empty() => widgets::empty(
                ui,
                icon::SWAP,
                tr(
                    "This title was not found on other sources",
                    "ไม่พบเรื่องนี้ในแหล่งอื่น",
                ),
            ),
            Alts::Done(list) => {
                let list = list.clone();
                ui.spacing_mut().item_spacing.y = 10.0;
                for a in &list {
                    let (r, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 84.0), Sense::click());
                    let p = ui.painter();
                    p.rect_filled(
                        r,
                        CornerRadius::same(8),
                        if resp.hovered() { CARD_HI } else { CARD },
                    );
                    let pr = Rect::from_min_size(r.min + vec2(12.0, 8.0), vec2(45.0, 68.0));
                    let referer = self.referer(&a.card.source);
                    widgets::poster(
                        ui,
                        &mut self.images,
                        a.card.poster.as_deref(),
                        &referer,
                        pr,
                        4,
                    );
                    let p = ui.painter();
                    p.circle_filled(
                        pos2(pr.max.x + 22.0, r.min.y + 26.0),
                        4.0,
                        self.health_of(&a.card.source),
                    );
                    p.text(
                        pos2(pr.max.x + 34.0, r.min.y + 26.0),
                        egui::Align2::LEFT_CENTER,
                        self.source_name(&a.card.source),
                        bold(15.0),
                        TEXT,
                    );
                    p.text(
                        pos2(pr.max.x + 18.0, r.min.y + 54.0),
                        egui::Align2::LEFT_CENTER,
                        truncate(&a.card.title, 90),
                        regular(13.5),
                        MUTED,
                    );
                    let pct = (a.score * 100.0).round();
                    p.text(
                        pos2(r.max.x - 20.0, r.center().y),
                        egui::Align2::RIGHT_CENTER,
                        trf!("{pct}% match", "ตรง {pct}%"),
                        bold(14.0),
                        if pct >= 85.0 { GOOD } else { WARN },
                    );
                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                        *act = Some(Act::Open(a.card.clone()));
                    }
                }
            }
        }
    }

    fn more_tab(&mut self, ui: &mut Ui, d: &AnimeDetail, act: &mut Option<Act>) {
        let cards: Vec<AnimeCard> = self
            .listings
            .get(&d.source)
            .map(|l| {
                l.cards
                    .iter()
                    .filter(|c| c.url != d.url)
                    .take(18)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if cards.is_empty() {
            widgets::loading(ui, tr("Loading…", "กำลังโหลด…"));
            return;
        }
        widgets::card_grid(ui, &cards, |ui, c| {
            let referer = self.referer(&c.source);
            if widgets::anime_card(ui, &mut self.images, c, &referer, "").clicked() {
                *act = Some(Act::Open(c.clone()));
            }
        });
    }

    pub fn server_modal(&mut self, ctx: &egui::Context) {
        let Some(p) = &self.pick else { return };
        let mut close = false;
        let mut chosen = None;
        let title = format!(
            "{} — {}",
            truncate(&p.detail.title, 50),
            i18n::ep_name(&p.ep)
        );
        let source = self.source_name(&p.detail.source);
        let res = p.res.clone();
        let resp = egui::Modal::new(egui::Id::new("server_pick")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(RichText::new(tr("Choose a server", "เลือกเซิร์ฟเวอร์")).font(bold(20.0)));
            ui.label(RichText::new(format!("{title}  ·  {source}")).color(MUTED));
            ui.add_space(10.0);
            match &res {
                None => widgets::loading(ui, tr("Checking every server…", "กำลังตรวจทุกเซิร์ฟเวอร์…")),
                Some(Err(e)) => widgets::error_box(ui, &crate::app::first_line(e)),
                Some(Ok(list)) => {
                    for st in list {
                        let (kind, col) = match st.rank {
                            StreamRank::Direct => (tr("direct stream", "สตรีมตรง"), GOOD),
                            StreamRank::YtDlp => (tr("via yt-dlp", "ผ่าน yt-dlp"), WARN),
                            StreamRank::Unknown => {
                                (tr("unknown (last resort)", "ไม่รู้จัก (ทางสุดท้าย)"), BAD)
                            }
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
            if widgets::ghost_button(ui, tr("Cancel", "ยกเลิก")).clicked() {
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

/// (season number, episode count), in order.
fn seasons_of(d: &AnimeDetail) -> Vec<(u32, usize)> {
    let mut v: Vec<(u32, usize)> = Vec::new();
    for e in &d.episodes {
        match v.iter_mut().find(|x| x.0 == e.season) {
            Some(x) => x.1 += 1,
            None => v.push((e.season, 1)),
        }
    }
    v.sort_by_key(|x| x.0);
    if v.is_empty() {
        v.push((1, 0));
    }
    v
}
