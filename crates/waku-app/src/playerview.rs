//! The built-in player screen (rmp, from the separate `rmp` workspace) wired to Waku:
//! episodes become the player's queue, the resolver in `app.rs` feeds it links one by one
//! (servers of the source, then other sources), and progress goes into the library for resume.

use crate::app::WakuApp;
use crate::i18n::{self, tr};
use rmp_egui::{EngineKind, Event, MpvConfig, OpenRequest, PlayerView, QueueItem, ViewSettings};
use std::sync::mpsc::Sender;
use std::time::Instant;
use waku_core::player::Candidate;
use waku_core::{AnimeDetail, Episode, Stream};

/// Resolver error text meaning "the user closed the player" (not a failure).
pub const CLOSED: &str = "\u{0}closed";

/// The player's answer to one candidate link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Opened,
    Failed,
    Closed,
}

pub struct ViewState {
    pub view: PlayerView,
    pub detail: AnimeDetail,
    /// queue index -> episode (same order as `detail.episodes`)
    pub eps: Vec<Episode>,
    /// resolver run feeding the current entry
    pub seq: u64,
    /// the candidate on screen waits for this answer
    pub reply: Option<Sender<Verdict>>,
    /// entries that played at least once (a later failure is a dropped stream, not a bad link)
    pub opened: std::collections::HashSet<usize>,
    /// automatic re-resolves of the current entry after a mid-stream failure
    pub recoveries: u32,
    pub last_save: Instant,
    /// a server picked by hand for the first entry (used by its first `NeedSource`)
    pub forced: Option<Stream>,
    /// the source really serving the link on screen when it is not `detail` (auto-failover)
    pub via: Option<(waku_core::AnimeCard, Episode)>,
    /// entry -> position of its first progress report (history is written once it advances past that)
    pub first_pos: std::collections::HashMap<usize, f64>,
    /// entries already written to the history
    pub recorded: std::collections::HashSet<usize>,
    /// the link on screen "opened" at this time but has not shown moving pictures yet: (when, position then)
    pub confirming: Option<(Instant, f64)>,
    /// entries whose opening / ending times were asked from AniSkip
    pub skip_asked: std::collections::HashSet<usize>,
}

pub fn lang_list(s: &str) -> Vec<String> {
    s.split([',', ';']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

fn view_settings(s: &waku_core::store::Settings) -> ViewSettings {
    let mpv = MpvConfig {
        hwdec: s.hwdec.clone(),
        quality: s.video_quality.clone(),
        deband: s.deband,
        audio_channels: s.audio_channels.clone(),
        audio_passthrough: s.audio_passthrough.clone(),
        sub_scale: s.sub_scale,
        ytdl_path: {
            let p = waku_core::player::find_tool("yt-dlp");
            if p.is_file() { p.to_string_lossy().to_string() } else { String::new() }
        },
        ..Default::default()
    };
    ViewSettings {
        engine: EngineKind::parse(&s.engine),
        mpv,
        thai: s.language == "th",
        autoplay_next: s.autoplay_next,
        volume: s.volume,
        muted: s.muted,
        sub_delay: s.sub_delay,
        sub_pos: s.sub_pos,
        sub_file_style: s.sub_file_style,
        auto_skip: if s.auto_skip { vec!["op".into(), "ed".into()] } else { vec![] },
        audio_langs: lang_list(&s.audio_langs),
        sub_langs: lang_list(&s.sub_langs),
        ..Default::default()
    }
}

fn to_request(c: &Candidate, start: f64) -> OpenRequest {
    OpenRequest {
        url: c.url.clone(),
        title: Some(c.title.clone()),
        start,
        referrer: Some(c.referrer.clone()),
        user_agent: Some(c.user_agent.clone()),
        force_format: c.force_hls.then(|| "hls".to_string()),
        headers: c.headers.clone(),
        // the loopback proxy also cleans segments disguised as images, which plain .m3u8 links use too
        hls_proxy: Some(c.direct && (c.force_hls || c.url.to_lowercase().split('?').next().unwrap_or("").ends_with(".m3u8"))),
        volume: 100.0,
        ..Default::default()
    }
}

impl WakuApp {
    /// Open the player screen for `ep` and start finding a link.
    pub fn play_builtin(&mut self, detail: &AnimeDetail, ep: &Episode, forced: Option<Stream>) {
        if let Some(mut old) = self.view.take() {
            if let Some(r) = old.reply.take() {
                let _ = r.send(Verdict::Closed);
            }
        }
        let meta = self.meta_of(&detail.url, &detail.title).cloned();
        let eps = detail.episodes.clone();
        let idx = eps.iter().position(|e| e.url == ep.url).unwrap_or(0);
        let (eps, idx) = if eps.is_empty() { (vec![ep.clone()], 0) } else { (eps, idx) };
        let queue: Vec<QueueItem> = eps
            .iter()
            .map(|e| {
                // "Episode 3 · The Marble Moon in the Ramune Bottle" when the episode's title is known
                let t = meta.as_ref().filter(|_| e.num.fract() == 0.0).and_then(|m| m.ep(e.num as u32)).map(|x| x.title.clone()).filter(|t| !t.is_empty());
                let subtitle = match t {
                    Some(t) => format!("{} · {t}", i18n::ep_name(e)),
                    None => i18n::ep_name(e),
                };
                QueueItem { title: detail.title.clone(), subtitle, req: None, skip: vec![] }
            })
            .collect();
        // the view asks for the link of entry `idx` right away (Event::NeedSource) and the resolver starts then
        let view = PlayerView::new(&self.ctx, view_settings(&self.settings), queue, idx);
        self.view = Some(ViewState {
            view,
            detail: detail.clone(),
            eps,
            seq: 0,
            reply: None,
            opened: Default::default(),
            recoveries: 0,
            last_save: Instant::now(),
            forced,
            via: None,
            first_pos: Default::default(),
            recorded: Default::default(),
            confirming: None,
            skip_asked: Default::default(),
        });
    }

    /// A link from the resolver: show it in the player.
    pub fn on_candidate(&mut self, seq: u64, ep_url: String, cand: Candidate, via: Option<(waku_core::AnimeCard, Episode)>, reply: Sender<Verdict>) {
        let resume = self.settings.resume;
        let start_for = |lib: &waku_core::store::Library| if resume { lib.resume_point(&ep_url) } else { 0.0 };
        let start = match self.start_once.take() {
            Some((u, at)) if u == ep_url => at,
            other => {
                self.start_once = other;
                start_for(&self.lib)
            }
        };
        let src_name = self.source_name(&cand.source);
        let Some(v) = self.view.as_mut() else {
            let _ = reply.send(Verdict::Closed);
            return;
        };
        if v.seq != seq {
            let _ = reply.send(Verdict::Closed); // an old run (episode changed meanwhile)
            return;
        }
        let Some(idx) = v.eps.iter().position(|e| e.url == ep_url) else {
            let _ = reply.send(Verdict::Closed);
            return;
        };
        if let Some(old) = v.reply.replace(reply) {
            let _ = old.send(Verdict::Failed);
        }
        v.via = via;
        v.confirming = None;
        // after a dropped stream the view keeps its own position (start 0 = "where I was")
        let start = if v.opened.contains(&idx) { 0.0 } else { start };
        let req = to_request(&cand, start);
        // the player's Info shows where this comes from
        let host = cand.url.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("").to_string();
        let mut info = vec![
            format!("{}: {src_name}", tr("Website", "เว็บ")),
            format!("{}: {}", tr("Server", "เซิร์ฟเวอร์"), cand.server),
            format!("{}: {host}", tr("Video host", "โฮสต์วิดีโอ")),
        ];
        if let Some(h) = cand.height {
            info.push(format!("{}: {h}p", tr("Quality offered", "ความชัดที่เว็บให้")));
        }
        let page = match &v.via {
            Some((c, _)) if cand.source == crate::dlpage::LOCAL => c.url.clone(),
            Some((c, _)) => {
                info.push(tr("(switched here automatically: the first website did not play)", "(สลับมาเว็บนี้อัตโนมัติ เพราะเว็บแรกเล่นไม่ได้)").to_string());
                c.url.clone()
            }
            None => v.detail.url.clone(),
        };
        info.push(format!("{}: {page}", tr("Page", "หน้าเว็บ")));
        v.view.set_source_info(Some(info.join("\n")));
        // the site's other qualities of this stream (e.g. 720p / 360p files), sharpest first
        let mut qs: Vec<(u32, String)> = cand.variants.iter().filter_map(|(h, u)| h.map(|h| (h, u.clone()))).collect();
        qs.sort_by(|a, b| b.0.cmp(&a.0));
        qs.dedup_by_key(|x| x.0);
        let cur = qs.iter().position(|q| q.1 == cand.url).unwrap_or(0);
        let list: Vec<(String, OpenRequest)> = qs
            .iter()
            .map(|(h, u)| {
                let c = Candidate { url: u.clone(), ..cand.clone() };
                (format!("{h}p"), to_request(&c, 0.0))
            })
            .collect();
        v.view.set_qualities(if list.len() > 1 { list } else { Vec::new() }, cur);
        if v.view.index() == idx && v.view.queue()[idx].req.is_none() {
            v.view.provide(idx, req);
        } else if v.view.index() == idx {
            v.view.retry(req);
        }
    }

    /// The resolver gave up (every server / source failed) while the player waits.
    pub fn view_resolver_failed(&mut self, seq: u64, err: &str) {
        if let Some(v) = self.view.as_mut() {
            if v.seq == seq {
                v.reply = None;
                let msg = format!("{}\n{}", tr("No working link was found for this episode.", "หาลิงก์ที่เล่นได้ของตอนนี้ไม่เจอ"), crate::app::first_line(err));
                v.view.show_error(msg);
            }
        }
    }

    /// Draw the player (whole window) and act on its events.
    pub fn player_screen(&mut self, ui: &mut egui::Ui) {
        let Some(v) = self.view.as_mut() else { return };
        let events = v.view.ui(ui);
        // opened but nothing moves: give up on this link (pausing stops the clock)
        if let Some((t0, _)) = v.confirming {
            if v.view.state().paused {
                v.confirming = Some((Instant::now(), f64::NAN));
            } else if t0.elapsed().as_secs() >= 15 {
                v.confirming = None;
                if let Some(r) = v.reply.take() {
                    let _ = r.send(Verdict::Failed);
                    v.view.searching();
                }
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        for ev in events {
            self.on_view_event(ev);
        }
    }

    fn on_view_event(&mut self, ev: Event) {
        let Some(v) = self.view.as_mut() else { return };
        match ev {
            Event::Opened { index, engine: _ } => {
                // A link that opens can still show nothing (dead stream behind a valid playlist, a format the
                // engine cannot decode). The resolver is told "it plays" only once the position moves; if it
                // does not within a few seconds the link counts as failed and the next one is tried.
                let _ = index;
                if v.reply.is_some() {
                    v.confirming = Some((Instant::now(), f64::NAN));
                }
                // (not written to the history yet: a link can "open" and still show nothing — see Progress)
            }
            Event::Failed { index, error } => {
                v.confirming = None;
                if let Some(r) = v.reply.take() {
                    // the resolver tries the next link; meanwhile the screen says "Finding a source"
                    let _ = r.send(Verdict::Failed);
                    v.view.searching();
                } else if v.opened.contains(&index) && v.recoveries < 3 {
                    // the stream broke after playing: find a fresh link and continue where it stopped
                    v.recoveries += 1;
                    let (d, e) = (v.detail.clone(), v.eps[index].clone());
                    let seq = self.start_play(&d, &e, None, true);
                    if let Some(v) = self.view.as_mut() {
                        v.seq = seq;
                    }
                    self.notify(tr("Stream interrupted — reconnecting…", "สตรีมหลุด — กำลังเชื่อมต่อใหม่…"));
                } else {
                    let _ = error;
                }
            }
            Event::NeedSource { index } => {
                if let Some(r) = v.reply.take() {
                    let _ = r.send(Verdict::Closed);
                }
                v.recoveries = 0;
                let (d, e) = (v.detail.clone(), v.eps[index].clone());
                if d.source == crate::dlpage::LOCAL {
                    // a downloaded file: no resolving; progress / history go to the website's episode
                    v.seq += 1;
                    let seq = v.seq;
                    let cand = Candidate {
                        url: e.url.clone(),
                        referrer: String::new(),
                        user_agent: String::new(),
                        force_hls: false,
                        headers: vec![],
                        direct: true,
                        title: format!("{} — {}", d.title, i18n::ep_name(&e)),
                        source: crate::dlpage::LOCAL.into(),
                        server: tr("file on this PC", "ไฟล์ในเครื่อง").into(),
                        height: None,
                        variants: vec![],
                    };
                    let via = self.local_origin(&e.url);
                    let (tx, _rx) = std::sync::mpsc::channel();
                    self.on_candidate(seq, e.url.clone(), cand, via, tx);
                    return;
                }
                let forced = v.forced.take();
                let seq = self.start_play(&d, &e, forced, true);
                if let Some(v) = self.view.as_mut() {
                    v.seq = seq;
                }
            }
            Event::IndexChanged(_) => {}
            Event::Progress { index, pos, dur, ended } => {
                // opening / ending times (AniSkip) once the length is known
                if dur > 60.0 && !v.skip_asked.contains(&index)
                    && let Some(e) = v.eps.get(index).filter(|e| e.num > 0.0 && e.num.fract() == 0.0)
                {
                    v.skip_asked.insert(index);
                    let mal = self.meta.get(&v.detail.url, &v.detail.title).filter(|m| m.matched).and_then(|m| m.mal_id);
                    if let Some(mal) = mal {
                        let (http, tx, ctx, url, n) = (self.mgr.http().clone(), self.tx.clone(), self.ctx.clone(), e.url.clone(), e.num as u32);
                        std::thread::spawn(move || {
                            let r = waku_core::meta::aniskip::skip_times(&http, mal, n, dur);
                            waku_core::http::note(&format!("aniskip mal={mal} ep={n} len={dur:.0}: {r:?}"));
                            if let Ok(list) = r
                                && !list.is_empty()
                            {
                                let _ = tx.send(crate::app::Msg::Skip { ep_url: url, ranges: list });
                                ctx.request_repaint();
                            }
                        });
                    }
                }
                // near the end: find the next episode's link now, so "Next" starts at once
                if dur > 120.0 && pos >= dur - 150.0 && !ended && v.opened.contains(&index) && v.detail.source != crate::dlpage::LOCAL
                    && let Some(n) = v.eps.get(index + 1)
                {
                    self.mgr.prefetch(&v.detail.source, &n.url);
                }
                if let Some((_, p0)) = v.confirming.as_mut() {
                    if p0.is_nan() {
                        *p0 = pos;
                    } else if pos - *p0 >= 1.0 || ended {
                        v.confirming = None;
                        v.opened.insert(index);
                        if let Some(r) = v.reply.take() {
                            let _ = r.send(Verdict::Opened);
                        }
                    }
                }
                if let Some(e) = v.eps.get(index) {
                    let url = e.url.clone();
                    let mut save = ended || v.last_save.elapsed().as_secs() >= 10;
                    // Continue Watching gets the show only once the picture has really been running for a few
                    // seconds, and under the source that is actually playing it (after a failover that is not
                    // the page the user opened) — so a source that could not play never lands there.
                    let first = *v.first_pos.entry(index).or_insert(pos);
                    if !v.recorded.contains(&index) && (pos - first >= 4.0 || ended) {
                        v.recorded.insert(index);
                        let (card, we) = v.via.clone().unwrap_or_else(|| (v.detail.card(), e.clone()));
                        waku_core::health::record_source(&card.source, true);
                        self.lib.watched(&card, we.num, &we.label, &we.url, we.dub);
                        save = true;
                    }
                    if let Some((_, we)) = v.via.as_ref().filter(|(_, we)| we.url != url && v.recorded.contains(&index)) {
                        // keep the position under the playing source's episode too (Continue resumes there)
                        self.lib.set_progress(&we.url, pos, dur, ended);
                    }
                    if save {
                        v.last_save = Instant::now();
                    }
                    self.lib.set_progress(&url, pos, dur, ended);
                    if save {
                        self.lib.save();
                    }
                }
            }
            Event::TracksPicked { audio, sub } => {
                let put_first = |list: &mut String, val: String| {
                    let mut l = lang_list(list);
                    l.retain(|x| !x.eq_ignore_ascii_case(&val) && !x.eq_ignore_ascii_case("off"));
                    l.insert(0, val);
                    *list = l.join(", ");
                };
                if let Some(a) = audio {
                    put_first(&mut self.settings.audio_langs, a);
                }
                if let Some(sl) = sub {
                    put_first(&mut self.settings.sub_langs, sl);
                }
                self.settings.save();
            }
            Event::SettingsChanged => {
                let vs = v.view.settings().clone();
                let s = &mut self.settings;
                s.volume = vs.volume;
                s.muted = vs.muted;
                s.sub_delay = vs.sub_delay;
                s.sub_scale = vs.mpv.sub_scale;
                s.sub_pos = vs.sub_pos;
                s.sub_file_style = vs.sub_file_style;
                s.deband = vs.mpv.deband;
                s.audio_channels = vs.mpv.audio_channels.clone();
                s.video_quality = vs.mpv.quality.clone();
                s.save();
            }
            Event::Closed => {
                if let Some(r) = v.reply.take() {
                    let _ = r.send(Verdict::Closed);
                }
                self.view = None;
                self.lib.save();
            }
        }
    }
}

impl WakuApp {
    /// AniSkip answered for an episode of the open player.
    pub fn on_skip_times(&mut self, ep_url: &str, ranges: Vec<waku_core::meta::aniskip::Skip>) {
        let Some(v) = self.view.as_mut() else { return };
        let Some(idx) = v.eps.iter().position(|e| e.url == ep_url) else { return };
        let r = ranges.into_iter().map(|s| rmp_egui::SkipRange { kind: s.kind, start: s.start, end: s.end }).collect();
        v.view.set_skip(idx, r);
    }
}
