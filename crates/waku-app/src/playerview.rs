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
        let eps = detail.episodes.clone();
        let idx = eps.iter().position(|e| e.url == ep.url).unwrap_or(0);
        let (eps, idx) = if eps.is_empty() { (vec![ep.clone()], 0) } else { (eps, idx) };
        let queue: Vec<QueueItem> = eps
            .iter()
            .map(|e| QueueItem { title: detail.title.clone(), subtitle: i18n::ep_name(e), req: None, skip: vec![] })
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
        });
    }

    /// A link from the resolver: show it in the player.
    pub fn on_candidate(&mut self, seq: u64, ep_url: String, cand: Candidate, reply: Sender<Verdict>) {
        let resume = self.settings.resume;
        let start_for = |lib: &waku_core::store::Library| if resume { lib.resume_point(&ep_url) } else { 0.0 };
        let start = start_for(&self.lib);
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
        // after a dropped stream the view keeps its own position (start 0 = "where I was")
        let start = if v.opened.contains(&idx) { 0.0 } else { start };
        let req = to_request(&cand, start);
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
        for ev in events {
            self.on_view_event(ev);
        }
    }

    fn on_view_event(&mut self, ev: Event) {
        let Some(v) = self.view.as_mut() else { return };
        match ev {
            Event::Opened { index, engine: _ } => {
                v.opened.insert(index);
                if let Some(r) = v.reply.take() {
                    let _ = r.send(Verdict::Opened);
                }
                let card = v.detail.card();
                if let Some(e) = v.eps.get(index).cloned() {
                    self.lib.watched(&card, e.num, &e.label, &e.url);
                }
            }
            Event::Failed { index, error } => {
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
                let forced = v.forced.take();
                let seq = self.start_play(&d, &e, forced, true);
                if let Some(v) = self.view.as_mut() {
                    v.seq = seq;
                }
            }
            Event::IndexChanged(_) => {}
            Event::Progress { index, pos, dur, ended } => {
                if let Some(e) = v.eps.get(index) {
                    let url = e.url.clone();
                    let save = ended || v.last_save.elapsed().as_secs() >= 10;
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
