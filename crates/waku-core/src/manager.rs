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
            match self.http.ping(&format!("{}/", s.base())) {
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
        self.need(id)?.latest(page)
    }

    pub fn search(&self, id: &str, q: &str) -> Result<Vec<AnimeCard>> {
        self.need(id)?.search(q)
    }

    /// Search every source at once: (source id, result).
    pub fn search_all(&self, q: &str) -> Vec<(String, Result<Vec<AnimeCard>, String>)> {
        par_map(&self.sources, 8, |s| {
            (
                s.id().to_string(),
                s.search(q).map_err(|e| format!("{e:#}")),
            )
        })
    }

    pub fn detail(&self, id: &str, url: &str) -> Result<AnimeDetail> {
        self.need(id)?.detail(url)
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
        let s = self.need(source)?;
        let servers = s.servers(episode_url)?;
        let tr = Trace::off();
        let results: Vec<(usize, Result<Stream>)> = par_map(
            &servers.iter().enumerate().collect::<Vec<_>>(),
            8,
            |(i, sv)| (*i, s.resolve(sv, episode_url, &tr)),
        );
        let mut ok: Vec<(usize, Stream)> = Vec::new();
        let mut errs = Vec::new();
        for (i, r) in results {
            match r {
                Ok(st) => ok.push((i, st)),
                Err(e) => errs.push(format!("{}: {e:#}", servers[i].name)),
            }
        }
        ok.sort_by_key(|(i, st)| (st.rank, *i));
        if ok.is_empty() {
            return Err(anyhow!(
                "no server could be resolved:\n  {}",
                errs.join("\n  ")
            ));
        }
        Ok((ok.into_iter().map(|x| x.1).collect(), errs))
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
                "เชื่อมต่อ",
                format!("{} ตอบใน {ms} ms", s.base()),
                vec![],
            ),
            Err(e) => {
                push(
                    false,
                    "เชื่อมต่อ",
                    format!("{e:#}"),
                    vec!["เว็บล่ม / เปลี่ยนโดเมน / ติด Cloudflare — แก้ base ใน sources.toml".into()],
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
                    "หน้าล่าสุด",
                    format!(
                        "พบ {} เรื่อง (มี poster {})",
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
                    "หน้าล่าสุด",
                    format!("{e:#}"),
                    vec!["ลองตั้ง card_selector / series_path ใน sources.toml".into()],
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
                    "หน้าเรื่อง",
                    format!(
                        "\"{}\" — {} ตอน, poster {poster}",
                        truncate(&d.title, 40),
                        d.episodes.len()
                    ),
                    sample,
                );
                d
            }
            Err(e) => {
                push(false, "หน้าเรื่อง", format!("{e:#}"), vec![]);
                return steps;
            }
        };
        let Some(ep) = detail.episodes.first() else {
            push(
                false,
                "ตอน",
                "ไม่พบลิงก์ตอน — ตั้ง episode_path ใน sources.toml".into(),
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
