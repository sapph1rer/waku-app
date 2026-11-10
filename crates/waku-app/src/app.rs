//! App state, background jobs and the frame loop.

use crate::images::Images;
use crate::theme::*;
use crate::widgets;
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Ui, vec2};
use egui_phosphor::regular as icon;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use waku_core::manager::{Alternative, DiagStep, Health};
use waku_core::player::{self, PlayOutcome};
use waku_core::store::{Library, Settings};
use waku_core::stream::pick_variant;
use waku_core::{AnimeCard, AnimeDetail, Episode, SourceManager, Stream, StreamRank};

// ------------------------------------------------------------------ messages

pub enum Msg {
    Health(Vec<Health>),
    Latest {
        source: String,
        page: u32,
        res: Result<Vec<AnimeCard>, String>,
    },
    Search {
        seq: u64,
        source: String,
        res: Result<Vec<AnimeCard>, String>,
    },
    Detail {
        url: String,
        res: Result<AnimeDetail, String>,
    },
    Alts {
        url: String,
        res: Vec<Alternative>,
    },
    Play {
        seq: u64,
        evt: PlayEvt,
    },
    Streams {
        seq: u64,
        res: Result<Vec<Stream>, String>,
    },
    Diag {
        source: String,
        steps: Vec<DiagStep>,
    },
    PlayerCheck(bool),
}

pub enum PlayEvt {
    Status(String),
    Started {
        card: AnimeCard,
        ep: Episode,
        line: String,
    },
    Finished,
    Failed(String),
}

// ------------------------------------------------------------------ state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Home,
    Search,
    Detail,
    MyList,
    History,
    Settings,
}

#[derive(Default)]
pub struct Listing {
    pub cards: Vec<AnimeCard>,
    pub page: u32,
    pub loading: bool,
    pub error: Option<String>,
    pub end: bool,
}

pub enum Alts {
    Idle,
    Loading,
    Done(Vec<Alternative>),
}

pub struct DetailState {
    pub card: AnimeCard,
    pub data: Option<Result<AnimeDetail, String>>,
    pub alts: Alts,
    pub season: Option<u32>,
    pub newest_first: bool,
    pub more: bool,
}

pub struct PlayState {
    pub seq: u64,
    pub title: String,
    pub ep_label: String,
    pub lines: Vec<String>,
    pub running: bool,
    pub ok: Option<bool>,
    pub detail: AnimeDetail,
    pub ep: Episode,
}

pub struct ServerPick {
    pub seq: u64,
    pub detail: AnimeDetail,
    pub ep: Episode,
    pub res: Option<Result<Vec<Stream>, String>>,
}

pub struct WakuApp {
    pub ctx: egui::Context,
    pub mgr: Arc<SourceManager>,
    pub settings: Settings,
    pub lib: Library,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub images: Images,

    pub page: Page,
    pub back: Vec<Page>,
    pub health: HashMap<String, Health>,
    pub health_loading: bool,

    pub home_source: String,
    pub listings: HashMap<String, Listing>,

    pub search_box: String,
    pub search_q: String,
    pub search_seq: u64,
    pub search: Vec<(String, Option<Result<Vec<AnimeCard>, String>>)>,

    pub detail: Option<DetailState>,
    pub play: Option<PlayState>,
    play_seq: u64,
    pub pick: Option<ServerPick>,
    pub diag: HashMap<String, Option<Vec<DiagStep>>>,
    pub player_ok: Option<bool>,
    pub toast: Option<(String, f64)>,
}

impl WakuApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup(&cc.egui_ctx);
        let settings = Settings::load();
        cc.egui_ctx
            .set_zoom_factor(settings.ui_scale.clamp(0.7, 2.0));
        let mgr = Arc::new(SourceManager::load());
        let (tx, rx) = channel();
        let images = Images::new(&cc.egui_ctx, mgr.http().clone());
        let home_source = if mgr.get(&settings.home_source).is_some() {
            settings.home_source.clone()
        } else {
            mgr.sources()
                .first()
                .map(|s| s.id().to_string())
                .unwrap_or_default()
        };
        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            mgr,
            settings,
            lib: Library::load(),
            tx,
            rx,
            images,
            page: Page::Home,
            back: Vec::new(),
            health: HashMap::new(),
            health_loading: false,
            home_source,
            listings: HashMap::new(),
            search_box: String::new(),
            search_q: String::new(),
            search_seq: 0,
            search: Vec::new(),
            detail: None,
            play: None,
            play_seq: 0,
            pick: None,
            diag: HashMap::new(),
            player_ok: None,
            toast: None,
        };
        for e in app.mgr.load_errors.clone() {
            app.notify(e);
        }
        app.refresh_health();
        app.check_player();
        app
    }

    // ------------------------------------------------------------ jobs

    pub fn spawn(&self, f: impl FnOnce(&SourceManager) -> Msg + Send + 'static) {
        let mgr = self.mgr.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let m = f(&mgr);
            let _ = tx.send(m);
            ctx.request_repaint();
        });
    }

    pub fn notify(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), self.ctx.input(|i| i.time) + 5.0));
    }

    pub fn source_name(&self, id: &str) -> String {
        self.mgr
            .get(id)
            .map(|s| s.name().to_string())
            .unwrap_or_else(|| id.to_string())
    }

    pub fn referer(&self, id: &str) -> String {
        self.mgr
            .get(id)
            .map(|s| format!("{}/", s.base()))
            .unwrap_or_default()
    }

    pub fn health_of(&self, id: &str) -> Color32 {
        match self.health.get(id) {
            Some(h) => widgets::health_color(Some(h.ok), h.latency_ms),
            None => widgets::health_color(None, 0),
        }
    }

    pub fn refresh_health(&mut self) {
        self.health_loading = true;
        self.spawn(|m| Msg::Health(m.health()));
    }

    pub fn check_player(&mut self) {
        self.player_ok = None;
        let cmd = self.settings.player_cmd.clone();
        self.spawn(move |_| Msg::PlayerCheck(player::player_available(&cmd)));
    }

    pub fn reload_sources(&mut self) {
        self.mgr = Arc::new(SourceManager::load());
        self.listings.clear();
        self.health.clear();
        self.diag.clear();
        if self.mgr.get(&self.home_source).is_none() {
            self.home_source = self
                .mgr
                .sources()
                .first()
                .map(|s| s.id().to_string())
                .unwrap_or_default();
        }
        for e in self.mgr.load_errors.clone() {
            self.notify(e);
        }
        self.refresh_health();
        self.notify(format!(
            "โหลดแหล่งใหม่แล้ว ({} แหล่ง)",
            self.mgr.sources().len()
        ));
    }

    pub fn ensure_latest(&mut self, source: &str) {
        if source.is_empty() || self.listings.contains_key(source) {
            return;
        }
        self.listings.insert(
            source.to_string(),
            Listing {
                loading: true,
                ..Default::default()
            },
        );
        let s = source.to_string();
        self.spawn(move |m| Msg::Latest {
            source: s.clone(),
            page: 1,
            res: m.latest(&s, 1).map_err(|e| format!("{e:#}")),
        });
    }

    pub fn retry_latest(&mut self, source: &str) {
        self.listings.remove(source);
        self.ensure_latest(source);
    }

    pub fn more_latest(&mut self, source: &str) {
        let Some(l) = self.listings.get_mut(source) else {
            return;
        };
        if l.loading || l.end {
            return;
        }
        l.loading = true;
        let page = l.page + 1;
        let s = source.to_string();
        self.spawn(move |m| Msg::Latest {
            source: s.clone(),
            page,
            res: m.latest(&s, page).map_err(|e| format!("{e:#}")),
        });
    }

    pub fn run_search(&mut self, q: &str) {
        let q = q.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.search_seq += 1;
        self.search_q = q.clone();
        self.search = self
            .mgr
            .sources()
            .iter()
            .map(|s| (s.id().to_string(), None))
            .collect();
        let seq = self.search_seq;
        for (id, _) in self.search.clone() {
            let q = q.clone();
            self.spawn(move |m| Msg::Search {
                seq,
                source: id.clone(),
                res: m.search(&id, &q).map_err(|e| format!("{e:#}")),
            });
        }
        self.go(Page::Search);
    }

    pub fn open(&mut self, card: &AnimeCard) {
        self.detail = Some(DetailState {
            card: card.clone(),
            data: None,
            alts: Alts::Idle,
            season: None,
            newest_first: false,
            more: false,
        });
        let (src, url) = (card.source.clone(), card.url.clone());
        self.spawn(move |m| Msg::Detail {
            url: url.clone(),
            res: m.detail(&src, &url).map_err(|e| format!("{e:#}")),
        });
        self.go(Page::Detail);
    }

    pub fn find_alts(&mut self) {
        let Some(d) = &mut self.detail else { return };
        let Some(Ok(data)) = &d.data else { return };
        d.alts = Alts::Loading;
        let (title, src, url) = (data.title.clone(), data.source.clone(), data.url.clone());
        self.spawn(move |m| Msg::Alts {
            url,
            res: m.alternatives(&title, &src),
        });
    }

    pub fn go(&mut self, p: Page) {
        if self.page != p {
            self.back.push(self.page);
            if self.back.len() > 30 {
                self.back.remove(0);
            }
        }
        self.page = p;
    }

    pub fn go_back(&mut self) {
        self.page = self.back.pop().unwrap_or(Page::Home);
    }

    pub fn run_diag(&mut self, id: &str) {
        self.diag.insert(id.to_string(), None);
        let s = id.to_string();
        self.spawn(move |m| Msg::Diag {
            source: s.clone(),
            steps: m.diagnose(&s),
        });
    }

    pub fn pick_server(&mut self, detail: &AnimeDetail, ep: &Episode) {
        self.play_seq += 1;
        let seq = self.play_seq;
        self.pick = Some(ServerPick {
            seq,
            detail: detail.clone(),
            ep: ep.clone(),
            res: None,
        });
        let (src, url) = (detail.source.clone(), ep.url.clone());
        self.spawn(move |m| Msg::Streams {
            seq,
            res: m
                .streams(&src, &url)
                .map(|x| x.0)
                .map_err(|e| format!("{e:#}")),
        });
    }

    // ------------------------------------------------------------ playback

    /// Play an episode in a background thread. Order: every server of the
    /// current source (best first), then — when auto failover is on — the same
    /// episode on the other sources. `forced` = play exactly this stream.
    pub fn play(&mut self, detail: &AnimeDetail, ep: &Episode, forced: Option<Stream>) {
        if self.play.as_ref().is_some_and(|p| p.running) {
            self.notify("กำลังเปิดตอนอื่นอยู่ — ปิด player ก่อน");
            return;
        }
        self.play_seq += 1;
        let seq = self.play_seq;
        self.play = Some(PlayState {
            seq,
            title: detail.title.clone(),
            ep_label: ep.label.clone(),
            lines: vec![format!("กำลังเตรียม {} …", ep.label)],
            running: true,
            ok: None,
            detail: detail.clone(),
            ep: ep.clone(),
        });

        let mgr = self.mgr.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let s = self.settings.clone();
        let (detail, ep) = (detail.clone(), ep.clone());
        std::thread::spawn(move || {
            let send = |evt: PlayEvt| {
                let _ = tx.send(Msg::Play { seq, evt });
                ctx.request_repaint();
            };
            let name = |id: &str| {
                mgr.get(id)
                    .map(|x| x.name().to_string())
                    .unwrap_or_else(|| id.into())
            };
            let title = |d: &AnimeDetail, e: &Episode| format!("{} — {}", d.title, e.label);

            // Ok(true) = played, Ok(false) = try elsewhere, Err = stop (player missing …)
            let try_streams =
                |d: &AnimeDetail, e: &Episode, streams: Vec<Stream>| -> Result<bool, String> {
                    for st in streams {
                        let v = pick_variant(&st.variants, &s.quality).clone();
                        let q = v
                            .height
                            .map(|h| format!("{h}p"))
                            .unwrap_or_else(|| "auto".into());
                        let how = match st.rank {
                            StreamRank::Direct => "สตรีมตรง",
                            StreamRank::YtDlp => "yt-dlp",
                            StreamRank::Unknown => "ทางสุดท้าย",
                        };
                        let line = format!("▶ {} · {} · {q} ({how})", name(&st.source), st.server);
                        send(PlayEvt::Started {
                            card: d.card(),
                            ep: e.clone(),
                            line,
                        });
                        match player::play(&s.player_cmd, &st, &v, &title(d, e)) {
                            Ok(PlayOutcome::Played) => return Ok(true),
                            Ok(PlayOutcome::LoadFailed) => send(PlayEvt::Status(format!(
                                "↻ {} เปิดไม่ได้ ลองตัวถัดไป",
                                st.server
                            ))),
                            Err(err) => return Err(format!("{err:#}")),
                        }
                    }
                    Ok(false)
                };
            let try_source = |d: &AnimeDetail, e: &Episode| -> Result<bool, String> {
                send(PlayEvt::Status(format!(
                    "[{}] กำลังหาเซิร์ฟเวอร์ของ {} …",
                    name(&d.source),
                    e.label
                )));
                match mgr.streams(&d.source, &e.url) {
                    Ok((streams, _)) => try_streams(d, e, streams),
                    Err(err) => {
                        send(PlayEvt::Status(format!(
                            "✗ [{}] {}",
                            name(&d.source),
                            first_line(&format!("{err:#}"))
                        )));
                        Ok(false)
                    }
                }
            };

            let result = (|| -> Result<bool, String> {
                if let Some(st) = forced {
                    return try_streams(&detail, &ep, vec![st]);
                }
                if try_source(&detail, &ep)? {
                    return Ok(true);
                }
                if !s.auto_failover {
                    return Ok(false);
                }
                send(PlayEvt::Status("⇄ กำลังหาเรื่องเดียวกันจากแหล่งอื่น …".into()));
                let mut alts = mgr.alternatives(&detail.title, &detail.source);
                let rank = |id: &str| {
                    s.source_order
                        .iter()
                        .position(|x| x == id)
                        .unwrap_or(usize::MAX)
                };
                alts.sort_by(|a, b| {
                    rank(&a.card.source).cmp(&rank(&b.card.source)).then(
                        b.score
                            .partial_cmp(&a.score)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
                });
                for alt in alts {
                    let Ok(other) = mgr.detail(&alt.card.source, &alt.card.url) else {
                        continue;
                    };
                    let Some(e2) = SourceManager::match_episode(&detail, &ep, &other) else {
                        send(PlayEvt::Status(format!(
                            "– [{}] ไม่มี {}",
                            name(&other.source),
                            ep.label
                        )));
                        continue;
                    };
                    if try_source(&other, &e2)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            })();
            match result {
                Ok(true) => send(PlayEvt::Finished),
                Ok(false) => send(PlayEvt::Failed(
                    "ทุกเซิร์ฟเวอร์/ทุกแหล่งเปิดไม่ได้ ลองเลือกเซิร์ฟเวอร์เองหรือดูแหล่งอื่น".into(),
                )),
                Err(e) => send(PlayEvt::Failed(e)),
            }
        });
    }

    pub fn next_episode(&self) -> Option<(AnimeDetail, Episode)> {
        let p = self.play.as_ref()?;
        let i = p.detail.episodes.iter().position(|e| e.url == p.ep.url)?;
        p.detail
            .episodes
            .get(i + 1)
            .map(|e| (p.detail.clone(), e.clone()))
    }

    // ------------------------------------------------------------ messages

    fn drain(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Health(v) => {
                    self.health_loading = false;
                    self.health = v.into_iter().map(|h| (h.source.clone(), h)).collect();
                }
                Msg::Latest { source, page, res } => {
                    let l = self.listings.entry(source).or_default();
                    l.loading = false;
                    match res {
                        Ok(cards) => {
                            let before = l.cards.len();
                            for c in cards {
                                if !l.cards.iter().any(|x| x.url == c.url) {
                                    l.cards.push(c);
                                }
                            }
                            l.page = page;
                            l.end = l.cards.len() == before;
                            l.error = None;
                        }
                        Err(e) if page == 1 => l.error = Some(e),
                        Err(_) => l.end = true,
                    }
                }
                Msg::Search { seq, source, res } => {
                    if seq == self.search_seq {
                        if let Some(slot) = self.search.iter_mut().find(|x| x.0 == source) {
                            slot.1 = Some(res);
                        }
                    }
                }
                Msg::Detail { url, res } => {
                    if let Some(d) = &mut self.detail {
                        if d.card.url == url {
                            if let Ok(data) = &res {
                                if d.card.poster.is_none() {
                                    d.card.poster = data.poster.clone();
                                }
                            }
                            d.data = Some(res);
                        }
                    }
                }
                Msg::Alts { url, res } => {
                    if let Some(d) = &mut self.detail {
                        if matches!(&d.data, Some(Ok(x)) if x.url == url) {
                            d.alts = Alts::Done(res);
                        }
                    }
                }
                Msg::Play { seq, evt } => self.on_play(seq, evt),
                Msg::Streams { seq, res } => {
                    if let Some(p) = &mut self.pick {
                        if p.seq == seq {
                            p.res = Some(res);
                        }
                    }
                }
                Msg::Diag { source, steps } => {
                    self.diag.insert(source, Some(steps));
                }
                Msg::PlayerCheck(ok) => self.player_ok = Some(ok),
            }
        }
    }

    fn on_play(&mut self, seq: u64, evt: PlayEvt) {
        let Some(p) = &mut self.play else { return };
        if p.seq != seq {
            return;
        }
        match evt {
            PlayEvt::Status(s) => p.lines.push(s),
            PlayEvt::Started { card, ep, line } => {
                p.lines.push(line);
                p.ep_label = ep.label.clone();
                self.lib.watched(&card, ep.num, &ep.label, &ep.url);
            }
            PlayEvt::Finished => {
                p.running = false;
                p.ok = Some(true);
                p.lines.push("✓ ปิด player แล้ว".into());
            }
            PlayEvt::Failed(e) => {
                p.running = false;
                p.ok = Some(false);
                p.lines.push(format!("✗ {e}"));
            }
        }
        if p.lines.len() > 60 {
            p.lines.drain(..20);
        }
    }

    // ------------------------------------------------------------ chrome

    fn top_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let logo = ui
                .add(
                    egui::Label::new(
                        RichText::new(format!("{} waku", icon::PLAY_CIRCLE))
                            .font(bold(24.0))
                            .color(ACCENT),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if logo.clicked() {
                self.go(Page::Home);
            }
            ui.add_space(18.0);
            for (p, label) in [
                (Page::Home, "หน้าแรก"),
                (Page::MyList, "รายการของฉัน"),
                (Page::History, "ประวัติ"),
            ] {
                let active = self.page == p;
                let r = ui.add(
                    egui::Button::new(RichText::new(label).font(bold(15.0)).color(if active {
                        TEXT
                    } else {
                        MUTED
                    }))
                    .fill(Color32::TRANSPARENT)
                    .frame(false)
                    .min_size(vec2(0.0, 36.0)),
                );
                if active {
                    let rr = r.rect;
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rr.min.x, rr.max.y + 6.0),
                            egui::pos2(rr.max.x, rr.max.y + 9.0),
                        ),
                        CornerRadius::ZERO,
                        ACCENT,
                    );
                }
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.go(p);
                }
                ui.add_space(10.0);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let gear = ui.add(
                    egui::Button::new(RichText::new(icon::GEAR_SIX).size(20.0).color(
                        if self.page == Page::Settings {
                            ACCENT
                        } else {
                            MUTED
                        },
                    ))
                    .frame(false),
                );
                if gear.on_hover_text("ตั้งค่า / แหล่งข้อมูล").clicked()
                {
                    self.go(Page::Settings);
                }
                // source health summary
                let ok = self.health.values().filter(|h| h.ok).count();
                let total = self.mgr.sources().len();
                let col = if self.health.is_empty() {
                    DIM
                } else if ok == total {
                    GOOD
                } else if ok > 0 {
                    WARN
                } else {
                    BAD
                };
                let r = ui.add(
                    egui::Label::new(
                        RichText::new(format!("{}  {ok}/{total}", icon::BROADCAST)).color(col),
                    )
                    .sense(egui::Sense::click()),
                );
                if r.on_hover_text("แหล่งที่ออนไลน์ — คลิกเพื่อตรวจใหม่").clicked()
                {
                    self.refresh_health();
                }
                ui.add_space(8.0);
                let te = egui::TextEdit::singleline(&mut self.search_box)
                    .hint_text(format!("{}  ค้นหาอนิเมะ (ทุกแหล่ง)", icon::MAGNIFYING_GLASS))
                    .desired_width(300.0)
                    .background_color(PANEL)
                    .margin(Margin::symmetric(10, 7));
                let r = ui.add(te);
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let q = self.search_box.clone();
                    self.run_search(&q);
                }
            });
        });
    }

    fn now_playing(&mut self, ui: &mut Ui) {
        let mut close = false;
        let mut next: Option<(AnimeDetail, Episode)> = None;
        let mut pick: Option<(AnimeDetail, Episode)> = None;
        let nxt = self.next_episode();
        let Some(p) = &self.play else { return };
        ui.horizontal(|ui| {
            let (g, c) = match p.ok {
                None => (icon::SPINNER, ACCENT),
                Some(true) => (icon::CHECK_CIRCLE, GOOD),
                Some(false) => (icon::X_CIRCLE, BAD),
            };
            ui.label(RichText::new(g).size(26.0).color(c));
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} — {}",
                        waku_core::text::truncate(&p.title, 60),
                        p.ep_label
                    ))
                    .font(bold(15.0)),
                );
                // once finished, keep showing which source/server actually played
                let last = p.lines.last().cloned().unwrap_or_default();
                let started = p.lines.iter().rev().find(|l| l.starts_with('▶')).cloned();
                let line = match (p.running, started) {
                    (false, Some(s)) if p.ok == Some(true) => format!("{s}   ·   {last}"),
                    _ => last,
                };
                ui.label(RichText::new(line).size(13.0).color(MUTED))
                    .on_hover_text(p.lines.join("\n"));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(RichText::new(icon::X).size(18.0)).frame(false))
                    .on_hover_text("ปิดแถบนี้")
                    .clicked()
                {
                    close = true;
                }
                if !p.running {
                    if let Some(n) = &nxt {
                        if widgets::primary_button(
                            ui,
                            &format!("{}  ตอนถัดไป ({})", icon::SKIP_FORWARD, n.1.label),
                        )
                        .clicked()
                        {
                            next = nxt.clone();
                        }
                    }
                    if p.ok == Some(false)
                        && widgets::outline_button(
                            ui,
                            &format!("{}  เลือกเซิร์ฟเวอร์เอง", icon::LIST_NUMBERS),
                        )
                        .clicked()
                    {
                        pick = Some((p.detail.clone(), p.ep.clone()));
                    }
                }
            });
        });
        if close {
            self.play = None;
        }
        if let Some((d, e)) = next {
            self.play(&d, &e, None);
        }
        if let Some((d, e)) = pick {
            self.pick_server(&d, &e);
        }
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let Some((msg, until)) = self.toast.clone() else {
            return;
        };
        if ctx.input(|i| i.time) > until {
            self.toast = None;
            return;
        }
        egui::Area::new(egui::Id::new("toast"))
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -90.0))
            .show(ctx, |ui| {
                Frame::new()
                    .fill(PANEL_HI)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.label(RichText::new(msg).color(TEXT));
                    });
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

pub fn first_line(s: &str) -> String {
    waku_core::text::truncate(s.lines().next().unwrap_or(s), 140)
}

impl eframe::App for WakuApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.images.pump(&ctx);
        self.drain();

        egui::Panel::top("top")
            .frame(Frame::new().fill(BAR).inner_margin(Margin {
                left: 28,
                right: 28,
                top: 10,
                bottom: 10,
            }))
            .show(ui, |ui| self.top_bar(ui));

        if self.play.is_some() {
            egui::Panel::bottom("now_playing")
                .frame(
                    Frame::new()
                        .fill(BAR)
                        .inner_margin(Margin::symmetric(28, 10))
                        .stroke(egui::Stroke::new(1.0, LINE)),
                )
                .show(ui, |ui| self.now_playing(ui));
        }

        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG))
            .show(ui, |ui| {
                let salt = match self.page {
                    Page::Detail => format!(
                        "detail-{}",
                        self.detail
                            .as_ref()
                            .map(|d| d.card.url.as_str())
                            .unwrap_or("")
                    ),
                    p => format!("{p:?}"),
                };
                egui::ScrollArea::vertical()
                    .id_salt(salt)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin {
                                left: 32,
                                right: 32,
                                top: 22,
                                bottom: 48,
                            })
                            .show(ui, |ui| match self.page {
                                Page::Home => self.home_page(ui),
                                Page::Search => self.search_page(ui),
                                Page::Detail => self.detail_page(ui),
                                Page::MyList => self.my_list_page(ui),
                                Page::History => self.history_page(ui),
                                Page::Settings => self.settings_page(ui),
                            });
                    });
            });

        self.server_modal(&ctx);
        self.toast(&ctx);

        // mouse "back" button
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) {
            self.go_back();
        }
    }
}
