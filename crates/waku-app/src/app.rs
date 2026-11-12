//! App state, background jobs, playback with failover, and the frame layout
//! (left icon sidebar + content + now-playing bar).

use crate::i18n::{self, Lang, tr};
use crate::images::Images;
use crate::theme::*;
use crate::trf;
use crate::widgets;
use egui::{Color32, CornerRadius, Frame, Margin, RichText, Ui, vec2};
use egui_phosphor::regular as icon;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use waku_core::manager::{Alternative, DiagStep, Health};
use waku_core::meta::anilist::SchedItem;
use waku_core::meta::{AnimeMeta, MetaStore};
use waku_core::player::{self, PlayOutcome};
use waku_core::store::{Library, Settings};
use waku_core::stream::pick_variant;
use waku_core::{AnimeCard, AnimeDetail, Episode, SourceManager, Stream, StreamRank};

// ------------------------------------------------------------------ messages

pub enum Msg {
    Health(Vec<Health>),
    /// the libmpv download finished
    Libmpv(Result<(), String>),
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
    /// metadata for a series page (`enriched` = with episode titles)
    Meta {
        url: String,
        title: String,
        res: Result<AnimeMeta, String>,
    },
    /// AniList candidates for the Identify dialog
    IdentifyRes {
        seq: u64,
        res: Result<Vec<AnimeMeta>, String>,
    },
    /// one week of the airing schedule (key = local Monday 00:00)
    Schedule {
        week: u64,
        res: Result<Vec<SchedItem>, String>,
    },
    /// the newest GitHub release (manual = asked from Settings)
    Update {
        manual: bool,
        res: Result<crate::update::Release, String>,
    },
    /// a release was put in place: the new program
    Installed(Result<std::path::PathBuf, String>),
    /// newer sources rules in the repository: Some((text, revision))
    SourcesUpdate {
        manual: bool,
        res: Result<Option<(String, u32)>, String>,
    },
    /// a message from the watch party
    Party(crate::party::Wire),
    /// the watch party connection closed (guest)
    PartyClosed,
    /// the watch party is on another show: its page was loaded, play this episode from `pos`
    PartyOpen {
        detail: AnimeDetail,
        ep: Episode,
        pos: f64,
    },
    /// a franchise's watch order
    Order(u64, Result<Vec<waku_core::meta::anilist::OrderItem>, String>),
    /// a page of Browse › Discover
    Discover {
        seq: u64,
        res: Result<(Vec<waku_core::meta::AnimeMeta>, bool), String>,
    },
    /// trace.moe answered
    Scene(Result<Vec<waku_core::meta::tracemoe::SceneHit>, String>),
    /// the picture behind a link was downloaded (for the picture search)
    SceneBytes(Result<Vec<u8>, String>),
    /// a Source health test finished
    SourceTested(String),
    /// opening / ending times of an episode in the player (AniSkip)
    Skip {
        ep_url: String,
        ranges: Vec<waku_core::meta::aniskip::Skip>,
    },
    /// built-in player: the resolver found a link to try; answer through `reply`
    Candidate {
        seq: u64,
        ep_url: String,
        cand: waku_core::player::Candidate,
        /// the source really serving this link when it is not the page the user opened (auto-failover)
        via: Option<(AnimeCard, Episode)>,
        reply: Sender<crate::playerview::Verdict>,
    },
}

pub enum PlayEvt {
    Status(String),
    Started {
        card: AnimeCard,
        ep: Episode,
        line: String,
    },
    /// the external player really played this (history is written only now, never for links that failed)
    Played {
        card: AnimeCard,
        ep: Episode,
    },
    Finished,
    Failed(String),
}

// ------------------------------------------------------------------ state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Home,
    Search,
    Browse,
    Detail,
    MyList,
    History,
    OnAir,
    Downloads,
    Sources,
    Settings,
}

#[derive(Default)]
pub struct Listing {
    pub cards: Vec<AnimeCard>,
    /// cards came from the site in this session (not only from the start-up cache)
    pub fresh: bool,
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Episodes,
    Order,
    Details,
    Sources,
    More,
}

pub struct DetailState {
    pub card: AnimeCard,
    pub data: Option<Result<AnimeDetail, String>>,
    pub alts: Alts,
    pub tab: Tab,
    pub season: Option<u32>,
    pub newest_first: bool,
    /// Sub / Dub track picked in the episode list (None = follow what was watched last)
    pub dub: Option<bool>,
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
    pub tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub images: Images,

    pub page: Page,
    pub back: Vec<Page>,
    pub health: HashMap<String, Health>,
    pub health_loading: bool,
    /// libmpv is being downloaded (Settings › Playback)
    pub libmpv_loading: bool,

    pub home_source: String,
    pub listings: HashMap<String, Listing>,

    pub search_box: String,
    pub search_focus: bool,
    pub search_q: String,
    /// when the search box last changed (the search runs by itself shortly after typing stops)
    pub search_typed: Option<f64>,
    /// Home carousel: slide on screen, the one before it (for the cross-fade), when the current one started
    pub hero_idx: usize,
    pub hero_prev: usize,
    pub hero_t0: f64,
    /// height of the hero's text block last frame (it is laid out from the bottom up)
    pub hero_text_h: f32,
    /// the app logo (sidebar, loading screen)
    pub logo: Option<egui::TextureHandle>,
    /// start-up screen: when the app started, when it began to fade out (None = still showing)
    pub splash_t0: Option<f64>,
    pub splash_out: Option<f64>,
    /// test remote control (WAKU_DEV_DIR)
    pub dev: Option<crate::dev::Dev>,
    pub search_seq: u64,
    pub search: Vec<(String, Option<Result<Vec<AnimeCard>, String>>)>,

    pub detail: Option<DetailState>,
    pub play: Option<PlayState>,
    play_seq: u64,
    pub pick: Option<ServerPick>,
    pub diag: HashMap<String, Option<Vec<DiagStep>>>,
    pub player_ok: Option<bool>,
    pub toast: Option<(String, f64)>,

    // metadata (AniList / Kitsu / Jikan)
    pub meta: MetaStore,
    /// series URLs with a lookup in flight
    pub meta_pending: std::collections::HashSet<String>,
    /// background queue for listing cards (identify only, one request at a time)
    pub meta_queue: Sender<(String, String)>,
    pub identify: Option<crate::metaui::IdentifyState>,
    pub identify_seq: u64,

    // on air
    pub sched: HashMap<u64, Option<Result<Vec<SchedItem>, String>>>,
    pub sched_day: u64,
    pub sched_filter: u8,
    pub sched_mine: bool,

    // built-in player (rmp)
    pub view: Option<crate::playerview::ViewState>,

    // downloads
    pub dl: Arc<crate::download::Downloader>,
    /// series on disk (reloaded when a download finishes / the page opens), the one opened
    pub dl_list: Vec<(std::path::PathBuf, crate::download::SeriesFile)>,
    pub dl_gen: u64,
    pub dl_open: Option<std::path::PathBuf>,
    /// sources being tested on the Source health page
    pub src_testing: std::collections::HashSet<String>,
    /// search by picture
    pub scene: crate::scene::SceneSearch,
    /// start the next play of this episode here (seconds), once
    pub start_once: Option<(String, f64)>,
    /// Browse › Discover (AniList catalogue with filters); false = the sources' latest lists
    pub discover: crate::discover::DiscoverState,
    pub browse_discover: bool,
    /// franchise watch orders by AniList id (None = loading)
    pub orders: HashMap<u64, Option<Result<Vec<waku_core::meta::anilist::OrderItem>, String>>>,
    pub order_main_only: bool,
    pub presence: crate::discord::Presence,
    pub party: Option<crate::party::Party>,
    pub party_ui: crate::party::PartyUi,
    /// app / sources rules updates from GitHub
    pub upd: crate::update::UpdateState,
}

impl WakuApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup(&cc.egui_ctx);
        rmp_egui::install(cc.get_proc_address.clone());
        let settings = Settings::load();
        let presence = crate::discord::Presence::start(&settings.discord_app_id);
        i18n::set(Lang::from_code(&settings.language));
        cc.egui_ctx
            .set_zoom_factor(settings.ui_scale.clamp(0.7, 2.0));
        let mgr = Arc::new(SourceManager::load());
        {
            // title indexes (search) are rebuilt in the background, after the home page had its turn
            let m = mgr.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(20));
                m.warm();
            });
        }
        let (tx, rx) = channel();
        {
            // GitHub: a newer app / newer sources rules — after start-up has settled; old files of an update go
            crate::update::clean_old();
            let (m, tx, ctx) = (mgr.clone(), tx.clone(), cc.egui_ctx.clone());
            let (repo, app) = (settings.update_repo.clone(), settings.check_updates);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(15));
                if !repo.trim().is_empty() {
                    if app {
                        let _ = tx.send(Msg::Update { manual: false, res: crate::update::latest_release(m.http(), &repo).map_err(|e| format!("{e:#}")) });
                    }
                    // rules are always checked; without auto-update they wait in Settings
                    let _ = tx.send(Msg::SourcesUpdate { manual: false, res: waku_core::profile::check_remote_sources(m.http(), &repo).map_err(|e| format!("{e:#}")) });
                    ctx.request_repaint();
                }
            });
        }
        let tx_meta = tx.clone();
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
            // WAKU_PAGE=search|settings|… opens that page first (handy for screenshots / testing)
            page: match std::env::var("WAKU_PAGE").unwrap_or_default().as_str() {
                "search" => Page::Search,
                "settings" => Page::Settings,
                "browse" => Page::Browse,
                "history" => Page::History,
                "mylist" => Page::MyList,
                "onair" => Page::OnAir,
                "downloads" => Page::Downloads,
                _ => Page::Home,
            },
            back: Vec::new(),
            health: HashMap::new(),
            health_loading: false,
            libmpv_loading: false,
            home_source,
            listings: HashMap::new(),
            search_box: String::new(),
            search_focus: false,
            search_q: String::new(),
            search_typed: None,
            hero_idx: 0,
            hero_prev: 0,
            hero_t0: 0.0,
            hero_text_h: 300.0,
            logo: None,
            splash_t0: None,
            splash_out: None,
            dev: crate::dev::Dev::from_env(),
            search_seq: 0,
            search: Vec::new(),
            detail: None,
            play: None,
            play_seq: 0,
            pick: None,
            diag: HashMap::new(),
            player_ok: None,
            toast: None,
            meta: MetaStore::load(),
            meta_pending: Default::default(),
            meta_queue: crate::metaui::start_meta_worker(&cc.egui_ctx, tx_meta),
            identify: None,
            identify_seq: 0,
            sched: HashMap::new(),
            sched_day: crate::onair::day_start(waku_core::store::now()),
            sched_filter: 0,
            sched_mine: false,
            view: None,
            dl: Default::default(),
            dl_list: Vec::new(),
            dl_gen: u64::MAX,
            dl_open: None,
            src_testing: Default::default(),
            scene: Default::default(),
            start_once: None,
            discover: Default::default(),
            browse_discover: false,
            orders: HashMap::new(),
            order_main_only: false,
            presence,
            party: None,
            party_ui: Default::default(),
            upd: Default::default(),
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

    /// Download the libmpv build rmp was tested with and put `libmpv-2.dll` next to the program
    /// (the same thing `rmp/scripts/get-libmpv.ps1` does). Windows only.
    pub fn download_libmpv(&mut self) {
        if self.libmpv_loading {
            return;
        }
        self.libmpv_loading = true;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let res = fetch_libmpv().map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Libmpv(res));
            ctx.request_repaint();
        });
    }

    /// Remember a search the user committed to (Enter, or opening one of its results).
    pub fn remember_search(&mut self, q: &str) {
        let q = q.trim();
        if q.chars().count() < 2 {
            return;
        }
        let r = &mut self.settings.recent_searches;
        r.retain(|x| !x.eq_ignore_ascii_case(q));
        r.insert(0, q.to_string());
        r.truncate(10);
        self.settings.save();
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

    pub fn set_language(&mut self, l: Lang) {
        i18n::set(l);
        self.settings.language = l.code().into();
        self.settings.save();
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
        let n = self.mgr.sources().len();
        self.notify(trf!("Sources reloaded ({n})", "โหลดแหล่งใหม่แล้ว ({n} แหล่ง)"));
    }

    pub fn ensure_latest(&mut self, source: &str) {
        if source.is_empty() || self.listings.contains_key(source) {
            return;
        }
        // last session's list is shown at once (start-up is instant); the fresh one replaces it when it arrives
        let cached = latest_cache_load(source);
        self.listings.insert(
            source.to_string(),
            Listing {
                loading: true,
                page: if cached.is_empty() { 0 } else { 1 },
                cards: cached,
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
        if self.search_q == q && self.page == Page::Search && !self.search.is_empty() {
            return; // already showing exactly this
        }
        self.search_seq += 1;
        self.search_q = q.clone();
        self.search_typed = None;
        self.search = self
            .mgr
            .sources()
            .iter()
            .map(|s| {
                // the same search done before shows at once (and is refreshed)
                let cached = search_cache_load(s.id(), &q);
                (s.id().to_string(), cached.map(Ok))
            })
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
        // the page as it was last time shows at once; the fresh one replaces it when it arrives
        let cached = detail_cache_load(&card.url);
        if let Some(c) = &cached {
            self.ensure_meta_detail(c);
        }
        self.detail = Some(DetailState {
            card: card.clone(),
            data: cached.map(Ok),
            alts: Alts::Idle,
            tab: Tab::Episodes,
            season: None,
            newest_first: false,
            dub: None,
        });
        let (src, url) = (card.source.clone(), card.url.clone());
        self.spawn(move |m| Msg::Detail {
            url: url.clone(),
            res: m.detail(&src, &url).map_err(|e| format!("{e:#}")),
        });
        self.ensure_latest(&card.source); // for "More like this"
        self.go(Page::Detail);
    }

    pub fn find_alts(&mut self) {
        let Some(d) = &mut self.detail else { return };
        let Some(Ok(data)) = &d.data else { return };
        if !matches!(d.alts, Alts::Idle) {
            return;
        }
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

    /// Play in a background thread: every server of the current source (best
    /// first), then — with auto failover — the same episode on other sources.
    /// `forced` = play exactly this stream.
    pub fn play(&mut self, detail: &AnimeDetail, ep: &Episode, forced: Option<Stream>) {
        if self.settings.player != "external" {
            self.play_builtin(detail, ep, forced);
            return;
        }
        self.start_play(detail, ep, forced, false);
    }

    /// The resolver: find streams (current source, then other sources) and hand them to the player
    /// one by one until one opens. `builtin` = the rmp view inside the window, else the external player.
    pub fn start_play(&mut self, detail: &AnimeDetail, ep: &Episode, forced: Option<Stream>, builtin: bool) -> u64 {
        if !builtin && self.play.as_ref().is_some_and(|p| p.running) {
            self.notify(tr(
                "Another episode is opening — close the player first",
                "กำลังเปิดตอนอื่นอยู่ — ปิด player ก่อน",
            ));
            return 0;
        }
        self.play_seq += 1;
        let seq = self.play_seq;
        let name = i18n::ep_name(ep);
        self.play = Some(PlayState {
            seq,
            title: detail.title.clone(),
            ep_label: name.clone(),
            lines: vec![trf!("Preparing {name} …", "กำลังเตรียม {name} …")],
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
            let view_ep_url = ep.url.clone();
            let view_source = detail.source.clone();
            // hand one stream to the player; Err = stop everything (player missing / closed)
            let attempt = |d: &AnimeDetail, e: &Episode, st: &Stream, v: &waku_core::Variant| -> Result<PlayOutcome, String> {
                let t = format!("{} — {}", d.title, i18n::ep_name(e));
                if !builtin {
                    let r = player::play(&s.player_cmd, st, v, &t).map_err(|e| format!("{e:#}"));
                    if matches!(r, Ok(PlayOutcome::Played)) {
                        waku_core::health::record_source(&d.source, true);
                        send(PlayEvt::Played { card: d.card(), ep: e.clone() });
                    }
                    return r;
                }
                let via = (d.source != view_source || e.url != view_ep_url).then(|| (d.card(), e.clone()));
                for cand in player::candidates(st, v, &t) {
                    let url = cand.url.clone();
                    let (rtx, rrx) = channel();
                    if tx.send(Msg::Candidate { seq, ep_url: view_ep_url.clone(), cand, via: via.clone(), reply: rtx }).is_err() {
                        return Err(crate::playerview::CLOSED.into());
                    }
                    ctx.request_repaint();
                    let t0 = std::time::Instant::now();
                    waku_core::http::note(&format!("player: open {url}"));
                    let verdict = rrx.recv();
                    waku_core::http::note(&format!("player: {:?} after {} ms", verdict.as_ref().ok(), t0.elapsed().as_millis()));
                    match verdict {
                        Ok(crate::playerview::Verdict::Opened) => {
                            waku_core::health::record(&url, true);
                            waku_core::srcstats::record(&d.source, "play", t0.elapsed().as_millis(), Ok(()));
                            return Ok(PlayOutcome::Played);
                        }
                        Ok(crate::playerview::Verdict::Failed) => {
                            waku_core::health::record(&url, false);
                            waku_core::srcstats::record(&d.source, "play", t0.elapsed().as_millis(), Err(format!("the player could not open {}", url.split('/').nth(2).unwrap_or(""))));
                            continue;
                        }
                        Ok(crate::playerview::Verdict::Closed) | Err(_) => return Err(crate::playerview::CLOSED.into()),
                    }
                }
                Ok(PlayOutcome::LoadFailed)
            };
            let sname = |id: &str| {
                mgr.get(id)
                    .map(|x| x.name().to_string())
                    .unwrap_or_else(|| id.into())
            };

            // Ok(true) played, Ok(false) try elsewhere, Err stop (player missing …)
            let try_streams =
                |d: &AnimeDetail, e: &Episode, streams: Vec<Stream>| -> Result<bool, String> {
                    for mut st in streams {
                        // a player page nobody could read: let the browser helper play it and catch the link
                        if builtin && st.rank == StreamRank::Unknown && waku_core::http::browser().is_some() {
                            let sv = st.server.clone();
                            send(PlayEvt::Status(trf!(
                                "{sv}: reading the player page …",
                                "{sv}: กำลังอ่านหน้า player …"
                            )));
                            match mgr.deepen(&st) {
                                Some(d) => st = d,
                                None => {
                                    send(PlayEvt::Status(trf!("{sv}: no video found", "{sv}: ไม่พบวิดีโอ")));
                                    continue;
                                }
                            }
                        }
                        let v = pick_variant(&st.variants, &s.quality).clone();
                        let q = v
                            .height
                            .map(|h| format!("{h}p"))
                            .unwrap_or_else(|| "auto".into());
                        let how = match st.rank {
                            StreamRank::Direct => tr("direct", "สตรีมตรง"),
                            StreamRank::YtDlp => "yt-dlp",
                            StreamRank::Unknown => tr("last resort", "ทางสุดท้าย"),
                        };
                        let line = format!("{} · {} · {q} ({how})", sname(&st.source), st.server);
                        send(PlayEvt::Started {
                            card: d.card(),
                            ep: e.clone(),
                            line,
                        });
                        match attempt(d, e, &st, &v) {
                            Ok(PlayOutcome::Played) => return Ok(true),
                            Ok(PlayOutcome::LoadFailed) => {
                                let sv = &st.server;
                                send(PlayEvt::Status(trf!(
                                    "{sv} failed to load, trying the next one",
                                    "{sv} เปิดไม่ได้ ลองตัวถัดไป"
                                )))
                            }
                            Err(err) => return Err(format!("{err:#}")),
                        }
                    }
                    Ok(false)
                };
            let try_source = |d: &AnimeDetail, e: &Episode| -> Result<bool, String> {
                let (src, en) = (sname(&d.source), i18n::ep_name(e));
                send(PlayEvt::Status(trf!(
                    "[{src}] finding servers for {en} …",
                    "[{src}] กำลังหาเซิร์ฟเวอร์ของ {en} …"
                )));
                match mgr.streams(&d.source, &e.url) {
                    Ok((streams, _)) => try_streams(d, e, streams),
                    Err(err) => {
                        send(PlayEvt::Status(format!(
                            "[{src}] {}",
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
                waku_core::health::record_source(&detail.source, false);
                if !s.auto_failover {
                    return Ok(false);
                }
                send(PlayEvt::Status(
                    tr(
                        "Looking for the same title on other sources …",
                        "กำลังหาเรื่องเดียวกันจากแหล่งอื่น …",
                    )
                    .into(),
                ));
                let mut alts = mgr.alternatives(&detail.title, &detail.source);
                let rank = |id: &str| {
                    s.source_order
                        .iter()
                        .position(|x| x == id)
                        .unwrap_or(usize::MAX)
                };
                // sources that have been playing well go first, then the order from Settings
                let health = |id: &str| (waku_core::health::source_score(id) * 4.0).round() as i32;
                alts.sort_by(|a, b| {
                    health(&b.card.source).cmp(&health(&a.card.source)).then(rank(&a.card.source).cmp(&rank(&b.card.source))).then(
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
                        let (src, en) = (sname(&other.source), i18n::ep_name(&ep));
                        send(PlayEvt::Status(trf!(
                            "– [{src}] has no {en}",
                            "– [{src}] ไม่มี {en}"
                        )));
                        continue;
                    };
                    if try_source(&other, &e2)? {
                        return Ok(true);
                    }
                    waku_core::health::record_source(&other.source, false);
                }
                Ok(false)
            })();
            match result {
                Ok(true) => send(PlayEvt::Finished),
                Ok(false) => send(PlayEvt::Failed(
                    tr(
                        "No server on any source could be opened. Try choosing a server or another source.",
                        "ทุกเซิร์ฟเวอร์/ทุกแหล่งเปิดไม่ได้ ลองเลือกเซิร์ฟเวอร์เองหรือดูแหล่งอื่น",
                    )
                    .into(),
                )),
                Err(e) => send(PlayEvt::Failed(e)),
            }
        });
        seq
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
        self.dl_refresh();
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Libmpv(res) => {
                    self.libmpv_loading = false;
                    match res {
                        Ok(()) if rmp_egui::reload_engine() => self.notify(tr("libmpv installed — the built-in player now uses it", "ติดตั้ง libmpv แล้ว — player ในตัวจะใช้ทันที")),
                        Ok(()) => self.notify(tr("libmpv was downloaded but could not be loaded", "ดาวน์โหลด libmpv แล้วแต่โหลดไม่สำเร็จ")),
                        Err(e) => self.notify(format!("{}: {}", tr("libmpv download failed", "ดาวน์โหลด libmpv ไม่สำเร็จ"), first_line(&e))),
                    }
                }
                Msg::Health(v) => {
                    self.health_loading = false;
                    self.health = v.into_iter().map(|h| (h.source.clone(), h)).collect();
                }
                Msg::Latest { source, page, res } => {
                    let l = self.listings.entry(source.clone()).or_default();
                    l.loading = false;
                    match res {
                        Ok(cards) if page == 1 => {
                            latest_cache_save(&source, &cards);
                            l.end = cards.is_empty();
                            l.cards = cards;
                            l.page = 1;
                            l.error = None;
                            l.fresh = true;
                        }
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
                        // an old list from the cache stays up when the site cannot be reached now
                        Err(e) if page == 1 && l.cards.is_empty() => l.error = Some(e),
                        Err(_) if page == 1 => {}
                        Err(_) => l.end = true,
                    }
                }
                Msg::Search { seq, source, res } => {
                    if seq == self.search_seq
                        && let Some(slot) = self.search.iter_mut().find(|x| x.0 == source)
                    {
                        match (&res, &slot.1) {
                            (Err(_), Some(Ok(_))) => {} // keep the earlier answer when the site does not answer now
                            _ => {
                                if let Ok(v) = &res {
                                    search_cache_save(&source, &self.search_q, v);
                                }
                                slot.1 = Some(res);
                            }
                        }
                    }
                }
                Msg::Detail { url, res } => {
                    if let Some(d) = &mut self.detail
                        && d.card.url == url
                    {
                        if let Ok(data) = &res
                            && d.card.poster.is_none()
                        {
                            d.card.poster = data.poster.clone();
                        }
                        let prev = d.data.as_ref().and_then(|r| r.as_ref().ok()).cloned();
                        let res = match (res, prev) {
                            // a failed or empty refresh does not replace a good page from the cache
                            (Err(_), Some(p)) => Ok(p),
                            (Ok(n), Some(p)) if n.episodes.is_empty() && !p.episodes.is_empty() => Ok(p),
                            (r, _) => r,
                        };
                        if let Ok(x) = &res
                            && !x.episodes.is_empty()
                        {
                            detail_cache_save(&url, x);
                        }
                        let found = res.as_ref().ok().cloned();
                        d.data = Some(res);
                        if let Some(data) = found {
                            self.ensure_meta_detail(&data);
                        }
                    }
                }
                Msg::Alts { url, res } => {
                    if let Some(d) = &mut self.detail
                        && matches!(&d.data, Some(Ok(x)) if x.url == url)
                    {
                        d.alts = Alts::Done(res);
                    }
                }
                Msg::Play { seq, evt } => self.on_play(seq, evt),
                Msg::Streams { seq, res } => {
                    if let Some(p) = &mut self.pick
                        && p.seq == seq
                    {
                        p.res = Some(res);
                    }
                }
                Msg::Diag { source, steps } => {
                    self.diag.insert(source, Some(steps));
                }
                Msg::PlayerCheck(ok) => self.player_ok = Some(ok),
                Msg::Meta { url, title, res } => self.on_meta(url, title, res),
                Msg::IdentifyRes { seq, res } => {
                    if let Some(st) = &mut self.identify
                        && self.identify_seq == seq
                    {
                        st.results = Some(res);
                    }
                }
                Msg::Candidate { seq, ep_url, cand, via, reply } => self.on_candidate(seq, ep_url, cand, via, reply),
                Msg::Skip { ep_url, ranges } => self.on_skip_times(&ep_url, ranges),
                Msg::Party(w) => self.on_party(w),
                Msg::Update { manual, res } => self.on_update(manual, res),
                Msg::Installed(res) => self.on_installed(res),
                Msg::SourcesUpdate { manual, res } => self.on_sources_update(manual, res),
                Msg::PartyClosed => self.party_closed(),
                Msg::PartyOpen { detail, ep, pos } => self.party_open(detail, ep, pos),
                Msg::Order(id, r) => {
                    if let Ok(v) = &r {
                        crate::order::save(id, v);
                    }
                    self.orders.insert(id, Some(r));
                }
                Msg::Discover { seq, res } => self.on_discover(seq, res),
                Msg::Scene(r) => {
                    self.scene.busy = false;
                    match r {
                        Ok(h) => self.scene.hits = h.into_iter().filter(|h| !h.adult).collect(),
                        Err(e) => self.scene.error = Some(first_line(&e).to_string()),
                    }
                }
                Msg::SceneBytes(r) => match r {
                    Ok(b) => self.scene_search(b),
                    Err(e) => {
                        self.scene.busy = false;
                        self.scene.error = Some(first_line(&e).to_string());
                    }
                },
                Msg::SourceTested(id) => {
                    self.src_testing.remove(&id);
                }
                Msg::Schedule { week, res } => {
                    self.sched.insert(week, Some(res));
                }
            }
        }
    }

    fn on_play(&mut self, seq: u64, evt: PlayEvt) {
        let mut failed: Option<String> = None;
        let Some(p) = &mut self.play else { return };
        if p.seq != seq {
            return;
        }
        match evt {
            PlayEvt::Status(s) => p.lines.push(s),
            PlayEvt::Started { card, ep, line } => {
                p.lines.push(line);
                p.ep_label = i18n::ep_name(&ep);
                let _ = card; // history is written once the episode really plays (PlayEvt::Played / player progress)
            }
            PlayEvt::Played { card, ep } => {
                self.lib.watched(&card, ep.num, &ep.label, &ep.url, ep.dub);
            }
            PlayEvt::Finished => {
                p.running = false;
                p.ok = Some(true);
                p.lines
                    .push(tr("Player closed", "ปิด player แล้ว").into());
            }
            PlayEvt::Failed(e) if e == crate::playerview::CLOSED => {
                p.running = false;
                p.ok = Some(true);
                p.lines.push(tr("Player closed", "ปิด player แล้ว").into());
            }
            PlayEvt::Failed(e) => {
                p.running = false;
                p.ok = Some(false);
                p.lines.push(format!("{e}"));
                failed = Some(e);
            }
        }
        if p.lines.len() > 60 {
            p.lines.drain(..20);
        }
        let last = p.lines.last().cloned();
        if let Some(v) = self.view.as_mut() {
            if v.seq == seq {
                v.view.set_status(last);
            }
        }
        if let Some(e) = failed {
            self.view_resolver_failed(seq, &e);
        }
    }

    // ------------------------------------------------------------ chrome

    pub fn logo_texture(&mut self) -> &egui::TextureHandle {
        if self.logo.is_none() {
            let img = image::load_from_memory(include_bytes!("../assets/logo.png")).map(|i| i.to_rgba8()).ok();
            let ci = match img {
                Some(i) => egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw()),
                None => egui::ColorImage::new([1, 1], vec![ACCENT]),
            };
            self.logo = Some(self.ctx.load_texture("waku-logo", ci, egui::TextureOptions::LINEAR));
        }
        self.logo.as_ref().unwrap()
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            // app logo (round, with an accent ring); click = home
            let (r, resp) = ui.allocate_exact_size(vec2(46.0, 46.0), egui::Sense::click());
            let tex = self.logo_texture().id();
            let hv = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.15);
            // centred on whole pixels, so the ring is even all round
            let ppp = ui.ctx().pixels_per_point();
            let c = egui::pos2((r.center().x * ppp).round() / ppp, (r.center().y * ppp).round() / ppp);
            let rad = 21.0 + hv;
            ui.painter().circle_filled(c, rad + 2.5, ACCENT.gamma_multiply(0.85 + 0.15 * hv));
            widgets::circle_image(ui.painter(), tex, c, rad, Color32::WHITE);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.page = Page::Home;
            }
            ui.add_space(42.0);

            let items = [
                (Page::Search, icon::MAGNIFYING_GLASS, tr("Search", "ค้นหา")),
                (Page::Home, icon::HOUSE, tr("Home", "หน้าแรก")),
                (Page::Browse, icon::SQUARES_FOUR, tr("Browse", "เรียกดู")),
                (Page::MyList, icon::BOOKMARK_SIMPLE, tr("My List", "รายการ")),
                (
                    Page::History,
                    icon::CLOCK_COUNTER_CLOCKWISE,
                    tr("History", "ประวัติ"),
                ),
                (Page::OnAir, icon::CALENDAR_DOTS, tr("On Air", "ตารางฉาย")),
                (Page::Downloads, icon::DOWNLOAD_SIMPLE, tr("Downloads", "ดาวน์โหลด")),
            ];
            for (p, g, label) in items {
                let active = self.page == p;
                if widgets::sidebar_item(ui, g, label, active).clicked() {
                    if p == Page::Search {
                        self.search_focus = true;
                    }
                    self.go(p);
                }
                ui.add_space(4.0);
            }
        });
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(14.0);
            let r = widgets::sidebar_item(
                ui,
                icon::GEAR_SIX,
                tr("Settings", "ตั้งค่า"),
                self.page == Page::Settings,
            );
            if r.clicked() {
                self.go(Page::Settings);
            }
            ui.add_space(4.0);
            let in_party = self.party.is_some();
            if widgets::sidebar_item(ui, icon::USERS_THREE, tr("Party", "ดูด้วยกัน"), in_party || self.party_ui.open).clicked() && !in_party {
                self.party_ui.open = !self.party_ui.open;
            }
            // tiny online indicator
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
            let hr = ui
                .add(
                    egui::Label::new(
                        RichText::new(format!("{} {ok}/{total}", icon::BROADCAST))
                            .size(11.5)
                            .color(col),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_text(tr(
                    "Sources online — click for the Source health page",
                    "แหล่งที่ออนไลน์ — คลิกเพื่อดูสถานะแหล่ง",
                ));
            if hr.clicked() {
                self.refresh_health();
                self.go(Page::Sources);
            }
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
                    .on_hover_text(tr("Close", "ปิด"))
                    .clicked()
                {
                    close = true;
                }
                if !p.running {
                    if let Some(n) = &nxt {
                        let en = i18n::ep_name(&n.1);
                        if widgets::primary_button(
                            ui,
                            &format!(
                                "{}  {}",
                                icon::SKIP_FORWARD,
                                trf!("Next: {en}", "ถัดไป: {en}")
                            ),
                        )
                        .clicked()
                        {
                            next = nxt.clone();
                        }
                    }
                    if p.ok == Some(false)
                        && widgets::outline_button(
                            ui,
                            &format!(
                                "{}  {}",
                                icon::LIST_NUMBERS,
                                tr("Choose server", "เลือกเซิร์ฟเวอร์เอง")
                            ),
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
        let now = ctx.input(|i| i.time);
        if now > until {
            self.toast = None;
            return;
        }
        // slide up + fade in, fade out during the last 0.3 s
        let id = egui::Id::new("toast");
        let shown = ctx.animate_bool_with_time(id.with(&msg), true, 0.2);
        let out = ((until - now) / 0.3).clamp(0.0, 1.0) as f32;
        let a = shown.min(out);
        egui::Area::new(id)
            .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -70.0 - 20.0 * a))
            .interactable(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(a);
                Frame::new()
                    .fill(PANEL_HI)
                    .stroke(egui::Stroke::new(1.0, LINE))
                    .corner_radius(CornerRadius::same(10))
                    .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(120) })
                    .inner_margin(Margin::symmetric(18, 11))
                    .show(ui, |ui| {
                        ui.label(RichText::new(msg).color(TEXT));
                    });
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
}

/// The mpv development build rmp is tested against (shinchiro/mpv-winbuild-cmake, about 30 MB).
const LIBMPV_URL: &str = "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20261019/mpv-dev-x86_64-20261019-git-6a262d11f6.7z";

fn fetch_libmpv() -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    if !cfg!(windows) {
        bail!("only needed on Windows (install libmpv with your package manager)");
    }
    let dir = std::env::current_exe()?.parent().context("no program folder")?.to_path_buf();
    let http = waku_core::http::Http::new(900)?;
    let bytes = http.get_bytes(LIBMPV_URL, None).context("downloading")?;
    if bytes.len() < 5_000_000 || !bytes.starts_with(b"7z") {
        bail!("unexpected download ({} bytes)", bytes.len());
    }
    let tmp = std::env::temp_dir().join("waku-libmpv");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let archive = tmp.join("mpv-dev.7z");
    std::fs::write(&archive, &bytes)?;
    // tar.exe ships with Windows 10 / 11 (bsdtar) and reads .7z
    let mut cmd = std::process::Command::new("tar");
    cmd.arg("-xf").arg(&archive).arg("-C").arg(&tmp).arg("libmpv-2.dll");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window
    }
    let out = cmd.output().context("running tar.exe to unpack the archive")?;
    let dll = tmp.join("libmpv-2.dll");
    if !out.status.success() || !dll.exists() {
        bail!("could not unpack: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    std::fs::copy(&dll, dir.join("libmpv-2.dll")).with_context(|| format!("copying into {}", dir.display()))?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

/// A short, human explanation of an error from the sources (the raw text goes under "Details").
pub fn friendly_error(e: &str) -> String {
    let low = e.to_lowercase();
    if low.contains("cloudflare") {
        if waku_core::http::browser().is_some() {
            tr(
                "This site is behind a Cloudflare check that could not be passed right now. Try again in a moment.",
                "เว็บนี้มีด่านตรวจของ Cloudflare ที่ยังผ่านไม่ได้ตอนนี้ ลองใหม่อีกครั้งในอีกสักครู่",
            )
            .into()
        } else {
            tr(
                "This site is behind a Cloudflare check. Waku needs its browser helper (waku-browser.exe next to waku.exe) to open it.",
                "เว็บนี้มีด่านตรวจของ Cloudflare ต้องมีตัวช่วยเบราว์เซอร์ (waku-browser.exe ข้าง waku.exe) จึงจะเปิดได้",
            )
            .into()
        }
    } else if low.contains("404") {
        tr("This page no longer exists on the site.", "หน้านี้ไม่มีอยู่บนเว็บแล้ว").into()
    } else if low.contains("timed out") || low.contains("timeout") || low.contains("connect") || low.contains("dns") {
        tr("The site did not answer. Check your connection or try again.", "เว็บไม่ตอบสนอง ตรวจสอบอินเทอร์เน็ตหรือลองใหม่").into()
    } else if low.contains("no player") || low.contains("no server") {
        tr("No playable server was found for this episode on this site.", "ไม่พบเซิร์ฟเวอร์ที่เล่นได้ของตอนนี้ในเว็บนี้").into()
    } else if low.contains("http 5") {
        tr("The site is having problems right now (server error).", "เว็บมีปัญหาอยู่ในขณะนี้ (server error)").into()
    } else {
        first_line(e)
    }
}

pub fn first_line(s: &str) -> String {
    waku_core::text::truncate(s.lines().next().unwrap_or(s), 140)
}

impl WakuApp {
    /// App-level commands of the test remote (see dev.rs).
    fn dev_command(&mut self) {
        if self.dev.is_some() {
            self.ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        let Some(line) = self.dev.as_mut().and_then(|d| d.app_cmd.take()) else { return };
        let (cmd, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let mut out = String::new();
        match cmd {
            "page" => {
                self.page = match arg {
                    "Search" => Page::Search,
                    "Browse" => Page::Browse,
                    "MyList" => Page::MyList,
                    "History" => Page::History,
                    "OnAir" => Page::OnAir,
                    "Downloads" => Page::Downloads,
                    "Sources" => Page::Sources,
                    "Settings" => Page::Settings,
                    _ => Page::Home,
                }
            }
            "search" => {
                self.search_box = arg.to_string();
                self.run_search(arg);
                self.page = Page::Search;
            }
            // `open SOURCE URL`: a series page; `play N`: episode N (1-based, site order) of the open page
            "open" => {
                if let Some((src, url)) = arg.split_once(' ') {
                    let card = self.listings.get(src).and_then(|l| l.cards.iter().find(|c| c.url == url).cloned()).unwrap_or_else(|| AnimeCard {
                        source: src.to_string(),
                        url: url.to_string(),
                        title: url.to_string(),
                        poster: None,
                        badge: None,
                    });
                    self.open(&card);
                } else if let Some(c) = self.listings.get(arg).and_then(|l| l.cards.first().cloned()) {
                    self.open(&c);
                }
            }
            "play" => {
                let n: usize = arg.parse().unwrap_or(1);
                let d = self.detail.as_ref().and_then(|d| d.data.as_ref()).and_then(|r| r.as_ref().ok()).cloned();
                match d.as_ref().and_then(|d| d.episodes.get(n.saturating_sub(1)).cloned().map(|e| (d, e))) {
                    Some((d, e)) => self.play(d, &e, None),
                    None => out += "play: no episode list open\n",
                }
            }
            "state" => {
                out += &format!("page={:?} view={} listings={:?}\n", self.page, self.view.is_some(), self.listings.iter().map(|(k, l)| (k.clone(), l.cards.len(), l.loading, l.error.is_some())).collect::<Vec<_>>());
                if let Some(v) = &self.view {
                    let st = v.view.state();
                    out += &format!(
                        "player: engine={} loaded={} pos={:.1}/{:?} paused={} buffering={} err={:?} {}x{} {} hwdec={} fps={:.2} drops={}/{} title={}\n",
                        v.view.engine_name(), st.loaded, st.position, st.duration, st.paused, st.buffering, st.error, st.width, st.height, st.vcodec, st.hwdec, st.fps, st.dropped_frames, st.decoder_drops, st.title
                    );
                }
                out += &self.dev_party("");
            }
            // watch party: `party host`, `party join CODE`, `party chat TEXT`, `party pause|play`, `party leave`
            "party" => out += &self.dev_party(&arg),
            _ => out += &format!("unknown command {line}\n"),
        }
        if !out.is_empty()
            && let Ok(dir) = std::env::var("WAKU_DEV_DIR")
        {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::path::Path::new(&dir).join("dev.log")) {
                let _ = f.write_all(out.as_bytes());
            }
        }
    }

    /// Start-up screen: the logo, a spinner and what is being loaded, until the home page has something to show
    /// (at most 10 s), then it fades away. Clicks do not reach the page underneath while it is up.
    fn splash(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let t0 = *self.splash_t0.get_or_insert(now);
        // every source has answered (fresh list or an error), at most 20 s
        let ids: Vec<String> = self.mgr.sources().iter().map(|s| s.id().to_string()).collect();
        for id in &ids {
            self.ensure_latest(id);
        }
        let answered = |l: Option<&Listing>| l.is_some_and(|l| l.fresh || l.error.is_some() || (!l.loading && !l.cards.is_empty()));
        let done = ids.iter().filter(|id| answered(self.listings.get(*id))).count();
        let total = ids.len();
        if self.splash_out.is_none() {
            // (WAKU_SPLASH_SECS keeps it up longer, for looking at it)
            let min = std::env::var("WAKU_SPLASH_SECS").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.6);
            if (done == ids.len() && now - t0 > min) || now - t0 > 20.0f64.max(min) {
                self.splash_out = Some(now);
            }
        }
        let fade = self.splash_out.map_or(1.0, |t| 1.0 - ((now - t) / 0.45) as f32).clamp(0.0, 1.0);
        if fade <= 0.0 {
            return;
        }
        ctx.request_repaint();
        let screen = ctx.content_rect();
        let tex = self.logo_texture().id();
        let ids: Vec<(String, String)> = self.mgr.sources().iter().map(|s| (s.id().to_string(), self.source_name(s.id()))).collect();
        egui::Area::new(egui::Id::new("splash"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::ZERO, BG.gamma_multiply(fade));
                let in_k = (((now - t0) / 0.5) as f32).clamp(0.0, 1.0);
                let ease = 1.0 - (1.0 - in_k).powi(3);
                let a = fade * ease;
                let c = rect.center() - vec2(0.0, 40.0 + 12.0 * (1.0 - ease));
                // logo with a soft glow ring that breathes
                let pulse = 0.5 + 0.5 * ((now * 2.2).sin() as f32);
                let r = 54.0;
                p.circle_filled(c, r + 10.0 + 4.0 * pulse, ACCENT.gamma_multiply(0.10 * a));
                p.circle_filled(c, r + 3.0, ACCENT.gamma_multiply(a));
                widgets::circle_image(ui.painter(), tex, c, r, Color32::WHITE.gamma_multiply(a));
                let p = ui.painter();
                p.text(c + vec2(0.0, r + 34.0), egui::Align2::CENTER_CENTER, "Waku", bold(30.0), TEXT.gamma_multiply(a));
                p.text(c + vec2(0.0, r + 104.0), egui::Align2::CENTER_CENTER, trf!("Connecting to sources  {done}/{total}", "กำลังเชื่อมต่อแหล่ง  {done}/{total}"), regular(13.0), MUTED.gamma_multiply(a));
                // spinner: an arc that runs round
                let sc = c + vec2(0.0, r + 78.0);
                let start = (now * 4.0) as f32;
                let pts: Vec<egui::Pos2> = (0..=24).map(|i| {
                    let t = start + i as f32 / 24.0 * 4.2;
                    sc + vec2(t.cos(), t.sin()) * 11.0
                }).collect();
                p.add(egui::Shape::line(pts, egui::Stroke::new(2.5, ACCENT_HI.gamma_multiply(a))));
                // what is loading
                let mut y = sc.y + 58.0;
                for (id, name) in &ids {
                    let (txt, col) = match self.listings.get(id) {
                        Some(l) if l.error.is_some() && l.cards.is_empty() => (format!("{}  {name}", icon::X), MUTED),
                        Some(l) if answered(Some(l)) => (format!("{}  {name}", icon::CHECK), TEXT),
                        _ => (format!("{}  {name}", icon::DOTS_THREE), DIM),
                    };
                    p.text(egui::pos2(rect.center().x, y), egui::Align2::CENTER_CENTER, txt, regular(13.5), col.gamma_multiply(a));
                    y += 21.0;
                }
            });
    }
}

fn search_cache_path(source: &str, q: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (source, q.trim().to_lowercase()).hash(&mut h);
    waku_core::profile::cache_dir().join("search").join(format!("{:016x}.json", h.finish()))
}

/// Search results younger than a day.
fn search_cache_load(source: &str, q: &str) -> Option<Vec<AnimeCard>> {
    let p = search_cache_path(source, q);
    let age = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?.elapsed().ok()?;
    if age > std::time::Duration::from_secs(86400) {
        return None;
    }
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn search_cache_save(source: &str, q: &str, v: &[AnimeCard]) {
    let p = search_cache_path(source, q);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(b) = serde_json::to_vec(v) {
        let _ = std::fs::write(p, b);
    }
}

fn detail_cache_path(url: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.trim_end_matches('/').hash(&mut h);
    waku_core::profile::cache_dir().join("detail").join(format!("{:016x}.json", h.finish()))
}

fn detail_cache_load(url: &str) -> Option<waku_core::AnimeDetail> {
    std::fs::read(detail_cache_path(url)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn detail_cache_save(key: &str, d: &waku_core::AnimeDetail) {
    let p = detail_cache_path(key);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(b) = serde_json::to_vec(d) {
        let _ = std::fs::write(p, b);
    }
}

fn latest_cache_path(source: &str) -> std::path::PathBuf {
    waku_core::profile::cache_dir().join("latest").join(format!("{source}.json"))
}

fn latest_cache_load(source: &str) -> Vec<AnimeCard> {
    std::fs::read(latest_cache_path(source)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn latest_cache_save(source: &str, cards: &[AnimeCard]) {
    let p = latest_cache_path(source);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec(cards) {
        let _ = std::fs::write(p, b);
    }
}

impl eframe::App for WakuApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(d) = &mut self.dev {
            d.input(ctx, raw);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.dev_command();
        self.images.pump(&ctx);
        self.drain();
        self.discord_tick();
        let splash_ctx = ctx.clone();

        // the built-in player takes the whole window while it is open
        if self.view.is_some() {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(Color32::BLACK))
                .show(ui, |ui| self.player_screen(ui));
            // the watch party follows the player (and its chat floats over the video)
            self.party_ui(&ctx);
            self.toast(&ctx);
            return;
        }

        egui::Panel::left("nav")
            .exact_size(84.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(BAR)
                    .inner_margin(Margin::symmetric(10, 0))
                    .stroke(egui::Stroke::new(1.0, Color32::from_rgb(0x1d, 0x1d, 0x20))),
            )
            .show(ui, |ui| self.sidebar(ui));

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
                // fade + slide the page in when it changes
                let now = ui.input(|i| i.time);
                let key = egui::Id::new("page_anim");
                let (prev, t0): (String, f64) = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_default();
                let t0 = if prev != salt {
                    ui.ctx().data_mut(|d| d.insert_temp(key, (salt.clone(), now)));
                    now
                } else {
                    t0
                };
                let k = (((now - t0) / 0.28) as f32).clamp(0.0, 1.0);
                let ease = 1.0 - (1.0 - k).powi(3);
                if k < 1.0 {
                    ui.ctx().request_repaint();
                }
                ui.multiply_opacity(ease);
                egui::ScrollArea::vertical()
                    .id_salt(salt)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // pages with a full-bleed hero draw their own margins
                        let full_bleed = matches!(self.page, Page::Detail | Page::Home);
                        let m = if full_bleed {
                            Margin {
                                left: 0,
                                right: 0,
                                top: 0,
                                bottom: 48,
                            }
                        } else {
                            Margin {
                                left: 36,
                                right: 36,
                                top: 28,
                                bottom: 48,
                            }
                        };
                        let mut m = m;
                        m.top += (14.0 * (1.0 - ease)) as i8;
                        Frame::new().inner_margin(m).show(ui, |ui| match self.page {
                            Page::Home => self.home_page(ui),
                            Page::Search => self.search_page(ui),
                            Page::Browse => self.browse_page(ui),
                            Page::Detail => self.detail_page(ui),
                            Page::MyList => self.my_list_page(ui),
                            Page::History => self.history_page(ui),
                            Page::OnAir => self.onair_page(ui),
                            Page::Downloads => self.downloads_page(ui),
                            Page::Sources => self.sources_page(ui),
                            Page::Settings => self.settings_page(ui),
                        });
                    });
            });

        self.server_modal(&ctx);
        self.party_ui(&ctx);
        self.identify_modal(&ctx);
        self.toast(&ctx);
        self.splash(&splash_ctx);

        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) {
            self.go_back();
        }
    }
}
