//! All sources together: health checks, search across sources, finding the
//! same anime on another source, resolving servers, and the source tester.

use crate::http::{Http, par_map};
use crate::model::*;
use crate::profile::{SiteProfile, load_sources, sources_path};
use crate::source::Source;
use crate::stream::Trace;
use crate::text::{fmt_num, title_similarity, truncate};
use crate::wp::WpSource;
use anyhow::{Result, anyhow};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Health {
    pub source: String,
    pub ok: bool,
    pub latency_ms: u128,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Alternative {
    pub card: AnimeCard,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct DiagStep {
    pub name: String,
    pub ok: bool,
    pub detail: String,
    pub trace: Vec<String>,
}

pub struct SourceManager {
    http: Http,
    sources: Vec<Arc<dyn Source>>,
    profiles: Vec<SiteProfile>,
    pub load_errors: Vec<String>,
}

impl SourceManager {
    /// Load `sources.toml` from the config folder (created on first run).
    pub fn load() -> Self {
        match load_sources(&sources_path()) {
            Ok(p) => Self::from_profiles(p),
            Err(e) => {
                let mut m = Self::from_profiles(
                    crate::profile::parse_sources(crate::profile::DEFAULT_SOURCES)
                        .unwrap_or_default(),
                );
                m.load_errors
                    .push(format!("{e:#} — using built-in defaults"));
                m
            }
        }
    }

    pub fn from_profiles(profiles: Vec<SiteProfile>) -> Self {
        let http = Http::new(25).expect("http client");
        let mut sources: Vec<Arc<dyn Source>> = Vec::new();
        let mut load_errors = Vec::new();
        for p in profiles.iter().filter(|p| p.enabled) {
            // Custom adapters for sites that are not WordPress-like go here:
            // match p.id.as_str() { "hareshi" => sources.push(Arc::new(HareshiSource::new(..))), _ => … }
            match WpSource::new(p.clone(), http.clone()) {
                Ok(s) => sources.push(Arc::new(s)),
                Err(e) => load_errors.push(format!("{e:#}")),
            }
        }
        Self {
            http,
            sources,
            profiles,
            load_errors,
        }
    }

    pub fn http(&self) -> &Http {
        &self.http
    }

    /// Enabled sources, in file order.
    pub fn sources(&self) -> &[Arc<dyn Source>] {
        &self.sources
    }

    pub fn profiles(&self) -> &[SiteProfile] {
        &self.profiles
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Source>> {
        self.sources.iter().find(|s| s.id() == id).cloned()
    }

    fn need(&self, id: &str) -> Result<Arc<dyn Source>> {
        self.get(id)
            .ok_or_else(|| anyhow!("source '{id}' is not enabled"))
    }

    pub fn health(&self) -> Vec<Health> {
        par_map(&self.sources, 8, |s| {
            let r = self.http.ping(&format!("{}/", s.base()));
            crate::srcstats::record(s.id(), "ping", *r.as_ref().unwrap_or(&0), r.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
            match r {
                Ok(ms) => Health {
                    source: s.id().into(),
                    ok: true,
                    latency_ms: ms,
                    error: None,
                },
                Err(e) => Health {
                    source: s.id().into(),
                    ok: false,
                    latency_ms: 0,
                    error: Some(format!("{e:#}")),
                },
            }
        })
    }

    pub fn latest(&self, id: &str, page: u32) -> Result<Vec<AnimeCard>> {
        let s = self.need(id)?;
        crate::srcstats::timed(id, "list", || s.latest(page))
    }

    /// Background upkeep of every source (title indexes …): call once after start-up.
    pub fn warm(&self) {
        for s in self.sources.iter().cloned() {
            std::thread::spawn(move || s.warm());
        }
    }

    pub fn search(&self, id: &str, q: &str) -> Result<Vec<AnimeCard>> {
        let s = self.need(id)?;
        crate::srcstats::timed(id, "search", || s.search(q))
    }

    /// Search every source at once: (source id, result).
    pub fn search_all(&self, q: &str) -> Vec<(String, Result<Vec<AnimeCard>, String>)> {
        par_map(&self.sources, 8, |s| {
            (
                s.id().to_string(),
                crate::srcstats::timed(s.id(), "search", || s.search(q)).map_err(|e| format!("{e:#}")),
            )
        })
    }

    pub fn detail(&self, id: &str, url: &str) -> Result<AnimeDetail> {
        let s = self.need(id)?;
        crate::srcstats::timed(id, "detail", || s.detail(url))
    }

    /// The same anime on the other sources, best match per source, best first.
    pub fn alternatives(&self, title: &str, exclude: &str) -> Vec<Alternative> {
        let others: Vec<&Arc<dyn Source>> =
            self.sources.iter().filter(|s| s.id() != exclude).collect();
        let mut out: Vec<Alternative> = par_map(&others, 8, |s| {
            let hits = s.search(title).ok()?;
            hits.into_iter()
                .map(|c| {
                    let sc = title_similarity(title, &c.title);
                    Alternative { card: c, score: sc }
                })
                .filter(|a| a.score >= 0.62)
                .max_by(|a, b| {
                    a.score
                        .partial_cmp(&b.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .into_iter()
        .flatten()
        .collect();
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// Episode in `other` that corresponds to `ep` of `from`.
    pub fn match_episode(from: &AnimeDetail, ep: &Episode, other: &AnimeDetail) -> Option<Episode> {
        let single = from.single_season() && other.single_season();
        other
            .episodes
            .iter()
            .find(|e| e.same_as(ep, single))
            .cloned()
    }

    /// Resolve every server of an episode (in parallel), best first.
    /// Errors of servers that failed are returned alongside.
    pub fn streams(&self, source: &str, episode_url: &str) -> Result<(Vec<Stream>, Vec<String>)> {
        if let Some(r) = take_prefetched(source, episode_url) {
            crate::http::note(&format!("resolve {source}: prefetched links used for {episode_url}"));
            return Ok(r);
        }
        self.streams_traced(source, episode_url, false)
    }

    /// Resolve an episode ahead of time (the next one, while the current plays to its end): the next
    /// `streams` call for it answers at once. Kept 10 minutes (some links expire); done in the background.
    pub fn prefetch(self: &Arc<Self>, source: &str, episode_url: &str) {
        let key = (source.to_string(), episode_url.to_string());
        {
            let mut m = PREFETCH.lock().unwrap();
            m.retain(|_, v| v.0.elapsed() < PREFETCH_KEEP);
            if m.contains_key(&key) {
                return;
            }
            m.insert(key.clone(), (std::time::Instant::now(), None)); // in flight
        }
        let me = self.clone();
        std::thread::spawn(move || {
            let t = std::time::Instant::now();
            let r = me.streams_traced(&key.0, &key.1, false).ok();
            crate::http::note(&format!("prefetch {} {} ms ok={} {}", key.0, t.elapsed().as_millis(), r.is_some(), key.1));
            let mut m = PREFETCH.lock().unwrap();
            match r {
                Some(r) => {
                    m.insert(key, (std::time::Instant::now(), Some(r)));
                }
                None => {
                    m.remove(&key);
                }
            }
        });
    }

    /// Like `streams`; with `trace` the resolver's steps are appended to the returned notes.
    pub fn streams_traced(&self, source: &str, episode_url: &str, trace: bool) -> Result<(Vec<Stream>, Vec<String>)> {
        let t = std::time::Instant::now();
        let r = self.streams_inner(source, episode_url, trace);
        let verdict = match &r {
            Ok((st, _)) if st.iter().any(|x| x.rank == crate::model::StreamRank::Direct) => Ok(()),
            Ok((st, errs)) if st.is_empty() => Err(errs.first().cloned().unwrap_or_else(|| "no servers".into())),
            Ok(_) => Err("no direct video link (only embeds)".into()),
            Err(e) => Err(format!("{e:#}")),
        };
        crate::srcstats::record(source, "resolve", t.elapsed().as_millis(), verdict);
        r
    }

    fn streams_inner(&self, source: &str, episode_url: &str, trace: bool) -> Result<(Vec<Stream>, Vec<String>)> {
        let s = self.need(source)?;
        let servers = s.servers(episode_url)?;
        let tr = Arc::new(if trace { Trace::enabled() } else { Trace::off() });
        // every server is resolved on its own thread; the answer does not wait for slow / hanging hosts once a
        // direct link is in hand (2.5 s grace for a better one), and never more than 30 s in all
        let (tx, rx) = std::sync::mpsc::channel::<(usize, Result<Stream>)>();
        for (i, sv) in servers.iter().cloned().enumerate() {
            let (s, tr, tx, ep) = (s.clone(), tr.clone(), tx.clone(), episode_url.to_string());
            std::thread::spawn(move || {
                // a bug in one resolver must not make the whole episode look empty
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.resolve(&sv, &ep, &tr)));
                let r = r.unwrap_or_else(|p| {
                    let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|x| x.to_string())).unwrap_or_default();
                    Err(anyhow!("internal error: {msg}"))
                });
                let _ = tx.send((i, r));
            });
        }
        drop(tx);
        let start = std::time::Instant::now();
        let mut deadline = start + std::time::Duration::from_secs(30);
        let mut results: Vec<(usize, Result<Stream>)> = Vec::new();
        while results.len() < servers.len() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok((i, r)) => {
                    crate::http::note(&format!(
                        "resolve {} '{}' {} ms: {}",
                        source,
                        servers[i].name,
                        start.elapsed().as_millis(),
                        match &r {
                            Ok(st) => format!("{:?} {}", st.rank, st.variants.first().map(|v| v.url.as_str()).unwrap_or("")),
                            Err(e) => format!("error {e:#}"),
                        }
                    ));
                    if r.as_ref().is_ok_and(|st| st.rank == crate::model::StreamRank::Direct) {
                        deadline = deadline.min(std::time::Instant::now() + std::time::Duration::from_millis(2500));
                    }
                    results.push((i, r));
                }
                Err(_) => break,
            }
        }
        let answered: Vec<usize> = results.iter().map(|r| r.0).collect();
        for (i, sv) in servers.iter().enumerate() {
            if !answered.contains(&i) {
                tr.log(format!("{}: still loading after {:.1} s, skipped", sv.name, start.elapsed().as_secs_f64()));
            }
        }
        let mut ok: Vec<(usize, Stream)> = Vec::new();
        let mut errs = Vec::new();
        for (i, r) in results {
            match r {
                Ok(st) => ok.push((i, st)),
                Err(e) => errs.push(format!("{}: {e:#}", servers[i].name)),
            }
        }
        // best first: direct links, then hosts that played well before, then the site's own order
        ok.sort_by(|(ia, a), (ib, b)| {
            let sa = a.variants.first().map_or(0.5, |v| crate::health::score(&v.url));
            let sb = b.variants.first().map_or(0.5, |v| crate::health::score(&v.url));
            a.rank.cmp(&b.rank).then(sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)).then(ia.cmp(ib))
        });
        if ok.is_empty() {
            return Err(anyhow!(
                "no server could be resolved:\n  {}",
                errs.join("\n  ")
            ));
        }
        errs.extend(tr.take());
        Ok((ok.into_iter().map(|x| x.1).collect(), errs))
    }

    /// Turn an unresolved player page (rank Unknown) into a direct stream by letting the browser helper play it
    /// and catching the video request. Slow (seconds), so it is done only for the server about to be played.
    pub fn deepen(&self, st: &Stream) -> Option<Stream> {
        if st.rank != crate::model::StreamRank::Unknown {
            return None;
        }
        let b = crate::http::browser()?;
        let embed = st.variants.first()?.url.clone();
        let mut found = b.sniff(&embed, &st.page).ok()?;
        // abyss serves its video as signed 2 MB chunks ("file.mp4#mp4/chunk/…"): the file itself answers 403
        found.urls.retain(|u| !u.contains("#mp4/chunk"));
        if found.urls.is_empty() {
            return None;
        }
        let hls = found.urls.iter().any(|u| crate::media::is_media_url(u) != Some(false));
        Some(Stream {
            variants: found
                .urls
                .iter()
                .take(4)
                .map(|u| crate::model::Variant { height: crate::media::height_of(u), url: u.split('#').next().unwrap_or(u).to_string() })
                .collect(),
            page: found.page,
            rank: crate::model::StreamRank::Direct,
            headers: found.headers,
            hls,
            ..st.clone()
        })
    }

    /// Source tester: latest -> detail -> servers -> resolve, step by step.
    pub fn diagnose(&self, id: &str) -> Vec<DiagStep> {
        let mut steps = Vec::new();
        let mut push = |ok: bool, name: &str, detail: String, trace: Vec<String>| {
            steps.push(DiagStep {
                name: name.into(),
                ok,
                detail,
                trace,
            });
        };
        let Some(s) = self.get(id) else {
            push(false, "source", format!("'{id}' not enabled"), vec![]);
            return steps;
        };
        match self.http.ping(&format!("{}/", s.base())) {
            Ok(ms) => push(
                true,
                "connect",
                format!("{} answered in {ms} ms", s.base()),
                vec![],
            ),
            Err(e) => {
                push(
                    false,
                    "connect",
                    format!("{e:#}"),
                    vec![
                        "site down / moved domain / Cloudflare — check base in sources.toml".into(),
                    ],
                );
                return steps;
            }
        }
        let cards = match s.latest(1) {
            Ok(c) => {
                let sample = c
                    .iter()
                    .take(5)
                    .map(|c| format!("{} — {}", truncate(&c.title, 50), c.url))
                    .collect();
                push(
                    !c.is_empty(),
                    "latest",
                    format!(
                        "{} titles ({} with poster)",
                        c.len(),
                        c.iter().filter(|x| x.poster.is_some()).count()
                    ),
                    sample,
                );
                c
            }
            Err(e) => {
                push(
                    false,
                    "latest",
                    format!("{e:#}"),
                    vec!["try card_selector / series_path in sources.toml".into()],
                );
                return steps;
            }
        };
        let Some(first) = cards.first() else {
            return steps;
        };
        let detail = match s.detail(&first.url) {
            Ok(d) => {
                let sample = d
                    .episodes
                    .iter()
                    .take(5)
                    .map(|e| format!("{} ({}) — {}", e.label, fmt_num(e.num), e.url))
                    .collect();
                let poster = if d.poster.is_some() { "✓" } else { "✗" };
                push(
                    !d.episodes.is_empty(),
                    "detail",
                    format!(
                        "\"{}\" — {} episodes, poster {poster}",
                        truncate(&d.title, 40),
                        d.episodes.len()
                    ),
                    sample,
                );
                d
            }
            Err(e) => {
                push(false, "detail", format!("{e:#}"), vec![]);
                return steps;
            }
        };
        let Some(ep) = detail.episodes.first() else {
            push(
                false,
                "episodes",
                "no episode links — set episode_path in sources.toml".into(),
                vec![],
            );
            return steps;
        };
        let servers = match s.servers(&ep.url) {
            Ok(v) => {
                push(
                    true,
                    "player",
                    format!("{} server", v.len()),
                    v.iter()
                        .map(|x| format!("{} — {:?}", x.name, x.kind))
                        .collect(),
                );
                v
            }
            Err(e) => {
                push(false, "player", format!("{e:#}"), vec![]);
                return steps;
            }
        };
        for sv in servers.iter().take(4) {
            let tr = Trace::enabled();
            match s.resolve(sv, &ep.url, &tr) {
                Ok(st) => {
                    let q: Vec<String> = st
                        .variants
                        .iter()
                        .map(|v| v.height.map_or("auto".into(), |h| format!("{h}p")))
                        .collect();
                    let mut t = tr.take();
                    t.push(format!("→ {}", st.variants[0].url));
                    push(
                        st.rank != StreamRank::Unknown,
                        &format!("stream: {}", sv.name),
                        format!("{:?} [{}]", st.rank, q.join(", ")),
                        t,
                    );
                }
                Err(e) => push(
                    false,
                    &format!("stream: {}", sv.name),
                    format!("{e:#}"),
                    tr.take(),
                ),
            }
        }
        steps
    }
}

type Prefetched = (Vec<Stream>, Vec<String>);
const PREFETCH_KEEP: std::time::Duration = std::time::Duration::from_secs(600);
static PREFETCH: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(String, String), (std::time::Instant, Option<Prefetched>)>>> =
    std::sync::LazyLock::new(Default::default);

/// Links resolved ahead of time for this episode (used once). A prefetch still in flight is waited for a little.
fn take_prefetched(source: &str, url: &str) -> Option<Prefetched> {
    let key = (source.to_string(), url.to_string());
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        {
            let mut m = PREFETCH.lock().unwrap();
            match m.get(&key) {
                None => return None,
                Some((t, _)) if t.elapsed() >= PREFETCH_KEEP => {
                    m.remove(&key);
                    return None;
                }
                Some((_, Some(_))) => return m.remove(&key).and_then(|v| v.1),
                Some((_, None)) => {}
            }
        }
        if std::time::Instant::now() >= until {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}
