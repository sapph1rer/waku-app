//! Anime metadata (ported from AnimeShelf): AniList for the show (cover, banner, synopsis,
//! genres, score, schedule), Kitsu / Jikan for episode titles, thumbnails and air dates.
//!
//! Sites give Thai titles and their own posters; this layer matches each series page to an
//! AniList entry ("identify"), caches it in `meta.json`, and lets the user fix a wrong match
//! by hand (manual matches are never replaced automatically).

pub mod anilist;
pub mod anizip;
pub mod aniskip;
pub mod tracemoe;
pub mod jikan;
pub mod kitsu;

use crate::http::Http;
use crate::profile::config_dir;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ types

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct EpMeta {
    pub number: u32,
    pub title: String,
    pub thumbnail: Option<String>,
    /// "2023-09-29"
    pub aired: Option<String>,
    /// minutes
    pub length: Option<u32>,
    /// anilist | kitsu | jikan
    pub source: String,
    /// what happens in the episode (ani.zip / Kitsu, English)
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct AnimeMeta {
    /// false = nothing found online (cached so it is not asked again every time)
    pub matched: bool,
    /// picked by the user (Identify) — never replaced automatically
    pub manual: bool,
    pub anilist_id: Option<u64>,
    pub mal_id: Option<u64>,
    pub kitsu_id: Option<String>,
    /// episode titles / thumbnails were fetched
    pub enriched: bool,
    /// ani.zip was asked too (older cache entries were enriched without it)
    pub anizip: bool,
    /// version of the episode data (2 = with summaries); older entries are fetched again
    pub eps_rev: u32,
    pub title_romaji: Option<String>,
    pub title_english: Option<String>,
    pub title_native: Option<String>,
    pub synonyms: Vec<String>,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub banner_url: Option<String>,
    /// AniList's dominant cover color ("#e4a15d")
    pub color: Option<String>,
    pub genres: Vec<String>,
    pub episodes: Option<u32>,
    /// RELEASING | FINISHED | NOT_YET_RELEASED | CANCELLED | HIATUS
    pub status: Option<String>,
    /// next episode to air (number, unix time)
    pub next_airing: Option<(u32, u64)>,
    pub format: Option<String>,
    /// minutes per episode
    pub duration: Option<u32>,
    pub year: Option<u32>,
    /// first air date as yyyymmdd (missing parts 0), for ordering seasons / movies
    pub start: Option<u32>,
    /// WINTER | SPRING | SUMMER | FALL
    pub season: Option<String>,
    pub score: Option<u32>,
    pub studio: Option<String>,
    pub adult: bool,
    pub eps: Vec<EpMeta>,
    pub fetched_at: u64,
    /// problems with the episode providers
    pub notes: Vec<String>,
}

impl AnimeMeta {
    pub fn display_title(&self, prefer_english: bool) -> Option<String> {
        let (a, b) = if prefer_english {
            (&self.title_english, &self.title_romaji)
        } else {
            (&self.title_romaji, &self.title_english)
        };
        a.clone()
            .or_else(|| b.clone())
            .or_else(|| self.title_native.clone())
    }

    pub fn ep(&self, n: u32) -> Option<&EpMeta> {
        self.eps.iter().find(|e| e.number == n)
    }

    fn all_titles(&self) -> Vec<&str> {
        let mut v: Vec<&str> = [&self.title_romaji, &self.title_english, &self.title_native]
            .into_iter()
            .flatten()
            .map(|s| s.as_str())
            .collect();
        v.extend(self.synonyms.iter().map(|s| s.as_str()));
        v
    }

    /// Should the cached entry be fetched again?
    pub fn stale(&self, now: u64) -> bool {
        let age = now.saturating_sub(self.fetched_at);
        if self.manual && !self.matched {
            return false; // the user cleared the match: do not look it up again
        }
        if !self.matched {
            return age > 3 * 86400;
        }
        if self.status.as_deref() == Some("RELEASING") {
            return age > 12 * 3600;
        }
        age > 30 * 86400 || !self.enriched
    }
}

// ------------------------------------------------------------------ helpers

/// Provider base URL; `WAKU_ANILIST` / `WAKU_KITSU` / `WAKU_JIKAN` let a test server stand in.
pub fn base(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// Space requests to the same provider (rate limits).
pub fn throttle(host: &'static str, gap: Duration) {
    static LAST: OnceLock<Mutex<HashMap<&'static str, Instant>>> = OnceLock::new();
    let wait = {
        let mut m = LAST
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let next = m.get(host).map_or(now, |t| (*t + gap).max(now));
        m.insert(host, next);
        next.saturating_duration_since(now)
    };
    if !wait.is_zero() {
        std::thread::sleep(wait);
    }
}

pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let (mut in_tag, mut tag) = (false, String::new());
    for c in s.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                if tag
                    .trim_start_matches('/')
                    .to_ascii_lowercase()
                    .starts_with("br")
                {
                    out.push('\n');
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    let out = crate::text::unescape_html(&out);
    let mut res = String::new();
    let mut nl = 0;
    for c in out.chars() {
        if c == '\n' {
            nl += 1;
            if nl <= 2 {
                res.push(c);
            }
        } else {
            nl = 0;
            res.push(c);
        }
    }
    res.trim().to_string()
}

pub fn now() -> u64 {
    crate::store::now()
}

/// letters and digits only, lowercase ("Re:Zero 2nd" -> "rezero2nd")
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

// ------------------------------------------------------------------ matching

fn re(p: &str) -> regex::Regex {
    regex::Regex::new(p).unwrap()
}

/// Search strings for a site title: Thai text and fansub suffixes removed; the URL slug
/// (usually romaji, e.g. `/anime/sousou-no-frieren/`) as a second guess.
pub fn queries_for(site_title: &str, url: &str) -> Vec<String> {
    static JUNK: OnceLock<regex::Regex> = OnceLock::new();
    let junk = JUNK.get_or_init(|| re(r"(?i)\b(?:sub(?:bed)?|thai|th|dub|uncen(?:sored)?|bd|tv|ep(?:isode)?s?\s*\d+(?:\s*-\s*\d+)?|\d+\s*-\s*\d+|end|ซับไทย|พากย์ไทย)\b|[\[\](){}|]"));
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        let s = crate::text::clean(&s)
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '!' && c != '?')
            .to_string();
        if s.chars().filter(|c| c.is_ascii_alphanumeric()).count() >= 3
            && !out.iter().any(|x| norm(x) == norm(&s))
        {
            out.push(s);
        }
    };
    let no_thai: String = site_title
        .chars()
        .map(|c| {
            if ('\u{0E00}'..='\u{0E7F}').contains(&c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    push(junk.replace_all(&no_thai, " ").to_string());
    let slug = crate::text::percent_decode(
        &crate::text::path_segs(url)
            .last()
            .cloned()
            .unwrap_or_default(),
    );
    if slug.is_ascii() && !slug.chars().all(|c| c.is_ascii_digit()) {
        let s = slug.replace(['-', '_'], " ");
        push(junk.replace_all(&s, " ").to_string());
    }
    out
}

/// "Grand Blue Season 2" -> ["Grand Blue"]; "Title: Subtitle" -> ["Title"].
pub fn query_variants(q: &str) -> Vec<String> {
    static TAIL: OnceLock<regex::Regex> = OnceLock::new();
    let tail = TAIL.get_or_init(|| re(r"(?i)[\s:\-–]*(?:the\s+)?(?:final\s+season|season\s*\d+|\d+(?:st|nd|rd|th)\s+season|part\s*\d+|cour\s*\d+|s\d{1,2})\s*$"));
    let mut cur = q.trim().to_string();
    for _ in 0..3 {
        let next = tail.replace(&cur, "").trim().to_string();
        if next == cur {
            break;
        }
        cur = next;
    }
    let mut cands = vec![cur.clone()];
    if let Some((head, _)) = cur.split_once(':') {
        cands.push(head.trim().to_string());
    }
    let mut v = Vec::new();
    for c in cands {
        if c.chars().count() >= 3 && norm(&c) != norm(q) && !v.contains(&c) {
            v.push(c);
        }
    }
    v
}

/// Similarity 0..1 between a site title and a candidate; small bonus when the episode count fits.
pub fn match_score(query: &str, cand: &AnimeMeta, local_eps: u32) -> f64 {
    let q = norm(query);
    if q.is_empty() {
        return 0.0;
    }
    let mut best: f64 = 0.0;
    for t in cand.all_titles() {
        let t = norm(t);
        if t.is_empty() {
            continue;
        }
        let mut s = strsim::jaro_winkler(&q, &t);
        if t == q {
            s = 1.0;
        } else if t.contains(&q) || q.contains(&t) {
            let ratio = q.len().min(t.len()) as f64 / q.len().max(t.len()) as f64;
            s = s.max(0.7 + 0.3 * ratio);
        }
        best = best.max(s);
    }
    if let (Some(total), true) = (cand.episodes, local_eps > 0) {
        if total == local_eps {
            best += 0.04;
        } else if local_eps > total {
            best -= 0.05; // more episodes than the show has: probably another season
        }
    }
    // a page with episodes is not a show that has not started yet ("Frieren 3rd Season" vs season 1)
    if cand.status.as_deref() == Some("NOT_YET_RELEASED") {
        best -= if local_eps > 0 { 0.3 } else { 0.08 };
    }
    best
}

pub const MIN_SCORE: f64 = 0.72;

pub fn pick_best(query: &str, cands: Vec<AnimeMeta>, local_eps: u32) -> Option<AnimeMeta> {
    cands
        .into_iter()
        .map(|c| (match_score(query, &c, local_eps), c))
        .filter(|(s, _)| *s >= MIN_SCORE)
        // ties keep AniList's order (its first hit is the most relevant)
        .fold(None::<(f64, AnimeMeta)>, |best, x| match best {
            Some(b) if b.0 >= x.0 => Some(b),
            _ => Some(x),
        })
        .map(|(_, c)| c)
}

pub const EPS_REV: u32 = 2;

/// First non-empty title wins, in the order of `layers` (best first).
pub fn merge_eps(layers: Vec<Vec<EpMeta>>) -> Vec<EpMeta> {
    let mut map: BTreeMap<u32, EpMeta> = BTreeMap::new();
    for eps in layers {
        for e in eps {
            let slot = map.entry(e.number).or_insert_with(|| EpMeta {
                number: e.number,
                ..Default::default()
            });
            if slot.title.is_empty() && !e.title.is_empty() {
                slot.title = e.title.clone();
                slot.source = e.source.clone();
            }
            if slot.thumbnail.is_none() {
                slot.thumbnail = e.thumbnail.clone();
            }
            if slot.aired.is_none() {
                slot.aired = e.aired.clone();
            }
            if slot.length.is_none() {
                slot.length = e.length;
            }
            if slot.summary.is_none() {
                slot.summary = e.summary.clone();
            }
        }
    }
    map.into_values().collect()
}

// ------------------------------------------------------------------ pipeline

fn finish(mut m: AnimeMeta, manual: bool) -> AnimeMeta {
    m.matched = true;
    m.manual = manual;
    m.fetched_at = now();
    m
}

/// Find the AniList entry for a site series page. `Ok(meta with matched = false)` when nothing fits.
pub fn identify(http: &Http, site_title: &str, url: &str, local_eps: u32) -> Result<AnimeMeta> {
    let mut last_err = None;
    for q in queries_for(site_title, url) {
        let mut tries = vec![q.clone()];
        tries.extend(query_variants(&q));
        for t in tries {
            match anilist::search(http, &t) {
                Ok(c) => {
                    if let Some(m) = pick_best(&t, c, local_eps) {
                        return Ok(finish(m, false));
                    }
                }
                Err(e) => last_err = Some(e),
            }
        }
    }
    if let Some(e) = last_err {
        return Err(e);
    }
    Ok(AnimeMeta {
        matched: false,
        fetched_at: now(),
        ..Default::default()
    })
}

/// The user picked an entry (Identify): full data for that AniList id.
pub fn by_id(http: &Http, id: u64) -> Result<AnimeMeta> {
    Ok(finish(anilist::by_id(http, id)?, true))
}

/// "154587" / "https://anilist.co/anime/154587/Sousou-no-Frieren/" -> 154587
pub fn parse_anilist_id(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Ok(n) = s.parse() {
        return Some(n);
    }
    let i = s.find("anilist.co/anime/")? + "anilist.co/anime/".len();
    s[i..].split(['/', '?', '#']).next()?.parse().ok()
}

/// Episode titles, thumbnails and air dates: Kitsu (pictures), then Jikan where titles are missing,
/// then AniList's own streaming episodes. Failures are kept as notes; the show data stays.
pub fn enrich(http: &Http, mut m: AnimeMeta) -> AnimeMeta {
    // ani.zip and Kitsu side by side (each a few hundred ms); Jikan only when neither has titles, and not at all
    // once it has failed to answer in this session (on some networks it never connects: 10 s lost each time)
    let (anilist, mal, kitsu_known) = (m.anilist_id, m.mal_id, m.kitsu_id.clone());
    let (az, (kid, kitsu_eps, mut notes)) = std::thread::scope(|sc| {
        let az = sc.spawn(|| anizip::episodes(http, anilist, mal));
        let mut notes = vec![];
        let mut kid = kitsu_known;
        if kid.is_none() {
            match kitsu::find_id(http, anilist, mal) {
                Ok(id) => kid = id,
                Err(e) => notes.push(format!("Kitsu: {e:#}")),
            }
        }
        let mut keps = vec![];
        if let Some(id) = kid.clone() {
            match kitsu::episodes(http, &id) {
                Ok(v) => keps = v,
                Err(e) => notes.push(format!("Kitsu: {e:#}")),
            }
        }
        (az.join().unwrap_or_else(|_| Ok(Vec::new())), (kid, keps, notes))
    });
    m.kitsu_id = kid;
    let az_eps = match az {
        Ok(v) => v,
        Err(e) => {
            notes.push(format!("ani.zip: {e:#}"));
            Vec::new()
        }
    };
    static JIKAN_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let has_titles = |v: &[EpMeta]| !v.is_empty() && v.iter().all(|e| !e.title.is_empty());
    let mut jk = vec![];
    if !has_titles(&az_eps) && !has_titles(&kitsu_eps) && !JIKAN_DOWN.load(std::sync::atomic::Ordering::Relaxed)
        && let Some(mal) = m.mal_id
    {
        match jikan::episodes(http, mal) {
            Ok(v) => jk = v,
            Err(e) => {
                let msg = format!("{e:#}");
                if msg.contains("timed out") || msg.contains("Connect") {
                    JIKAN_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                notes.push(format!("Jikan: {msg}"));
            }
        }
    }
    let own = std::mem::take(&mut m.eps);
    // AniList streaming titles first (official English), then ani.zip (TVDB/TMDB titles, pictures), Kitsu, Jikan
    m.eps = merge_eps(vec![own, az_eps, kitsu_eps, jk]);
    m.enriched = notes.is_empty() || m.eps.iter().any(|e| !e.title.is_empty());
    m.anizip = true;
    m.eps_rev = EPS_REV;
    m.notes = notes;
    m
}

// ------------------------------------------------------------------ cache

/// Metadata cache keyed by series URL, with a title index so the same anime on another
/// source reuses the match (and a manual Identify applies everywhere).
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MetaStore {
    pub entries: HashMap<String, AnimeMeta>,
    /// norm(site title) -> series URL
    pub titles: HashMap<String, String>,
}

fn key(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn title_key(title: &str) -> String {
    // "Solo Leveling โซโล่เลเวลลิ่ง" and "Solo Leveling" on another site share a key
    let q = queries_for(title, "")
        .into_iter()
        .next()
        .unwrap_or_else(|| title.to_string());
    norm(&q)
}

impl MetaStore {
    fn path() -> std::path::PathBuf {
        config_dir().join("meta.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let dir = config_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(t) = serde_json::to_string(self) {
            let tmp = dir.join("meta.json.tmp");
            if std::fs::write(&tmp, t).is_ok() {
                let _ = std::fs::rename(&tmp, Self::path());
            }
        }
    }

    /// Entry for this page, or for the same title seen on another source.
    pub fn get(&self, url: &str, title: &str) -> Option<&AnimeMeta> {
        if let Some(m) = self.entries.get(&key(url)) {
            return Some(m);
        }
        let tk = title_key(title);
        if tk.is_empty() {
            return None;
        }
        self.titles
            .get(&tk)
            .and_then(|u| self.entries.get(u))
            .filter(|m| m.matched)
    }

    pub fn set(&mut self, url: &str, title: &str, m: AnimeMeta) {
        let k = key(url);
        let tk = title_key(title);
        if m.matched && !tk.is_empty() {
            // a manual match wins over automatic ones made for the same title elsewhere
            let keep_other = self
                .titles
                .get(&tk)
                .and_then(|u| self.entries.get(u))
                .is_some_and(|o| o.manual && !m.manual)
                && self.titles.get(&tk) != Some(&k);
            if !keep_other {
                self.titles.insert(tk, k.clone());
            }
        }
        self.entries.insert(k, m);
    }

    /// Forget the match for this page (and the title link pointing at it).
    pub fn clear(&mut self, url: &str) {
        let k = key(url);
        self.entries.remove(&k);
        self.titles.retain(|_, v| *v != k);
    }

    /// AniList ids of these series pages (for "my shows" in the schedule).
    pub fn anilist_ids<'a>(&self, pages: impl Iterator<Item = (&'a str, &'a str)>) -> Vec<u64> {
        let mut v: Vec<u64> = pages
            .filter_map(|(u, t)| self.get(u, t))
            .filter_map(|m| m.anilist_id)
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(romaji: &str, english: Option<&str>, eps: Option<u32>) -> AnimeMeta {
        AnimeMeta {
            title_romaji: Some(romaji.into()),
            title_english: english.map(String::from),
            episodes: eps,
            ..Default::default()
        }
    }

    #[test]
    fn queries_from_thai_titles() {
        assert_eq!(
            queries_for(
                "Sousou no Frieren คำอธิษฐานในวันที่จากลา",
                "https://x.com/anime/sousou-no-frieren/"
            ),
            vec!["Sousou no Frieren"]
        );
        assert_eq!(
            queries_for(
                "Kaiju No. 8 ไคจูหมายเลข 8 ซับไทย EP 1-12",
                "https://x.com/kaiju-8/"
            ),
            vec!["Kaiju No. 8 8", "kaiju 8"]
        );
        // pure Thai title: only the slug is usable
        assert_eq!(
            queries_for("มหาเวทย์ผนึกมาร", "https://x.com/anime/jujutsu-kaisen/"),
            vec!["jujutsu kaisen"]
        );
        assert!(queries_for("มหาเวทย์ผนึกมาร", "https://x.com/ep/123/").is_empty());
    }

    #[test]
    fn variants_and_matching() {
        assert_eq!(query_variants("Grand Blue Season 2"), vec!["Grand Blue"]);
        assert_eq!(
            query_variants("Shingeki no Kyojin: The Final Season"),
            vec!["Shingeki no Kyojin"]
        );
        assert!(query_variants("Re:Zero kara Hajimeru").is_empty()); // "Re" is too short to search
        let cands = vec![
            cand(
                "Sousou no Frieren",
                Some("Frieren: Beyond Journey's End"),
                Some(28),
            ),
            cand("Bleach", None, Some(366)),
        ];
        assert_eq!(
            pick_best("Sousou no Frieren", cands.clone(), 28)
                .unwrap()
                .title_romaji
                .as_deref(),
            Some("Sousou no Frieren")
        );
        assert!(pick_best("Naruto", cands, 0).is_none());
    }

    #[test]
    fn merge_prefers_first_title_keeps_thumbs() {
        let a = vec![EpMeta {
            number: 1,
            title: "Official".into(),
            source: "anilist".into(),
            ..Default::default()
        }];
        let b = vec![EpMeta {
            number: 1,
            title: "Other".into(),
            thumbnail: Some("t".into()),
            aired: Some("2024-01-01".into()),
            source: "kitsu".into(),
            ..Default::default()
        }];
        let m = merge_eps(vec![a, b]);
        assert_eq!(
            (
                m[0].title.as_str(),
                m[0].thumbnail.as_deref(),
                m[0].source.as_str()
            ),
            ("Official", Some("t"), "anilist")
        );
    }

    #[test]
    fn ids_and_store() {
        assert_eq!(
            parse_anilist_id("https://anilist.co/anime/154587/Sousou-no-Frieren/"),
            Some(154587)
        );
        assert_eq!(parse_anilist_id(" 21 "), Some(21));
        assert_eq!(parse_anilist_id("frieren"), None);

        let mut st = MetaStore::default();
        let m = AnimeMeta {
            matched: true,
            anilist_id: Some(1),
            ..Default::default()
        };
        st.set("https://a.com/anime/solo/", "Solo Leveling โซโล่เลเวลลิ่ง", m);
        // the same title on another source finds it
        assert_eq!(
            st.get("https://b.com/solo-leveling/", "Solo Leveling")
                .and_then(|m| m.anilist_id),
            Some(1)
        );
        // a manual match elsewhere wins the title link
        st.set(
            "https://b.com/solo-leveling/",
            "Solo Leveling",
            AnimeMeta {
                matched: true,
                manual: true,
                anilist_id: Some(2),
                ..Default::default()
            },
        );
        st.set(
            "https://c.com/solo/",
            "Solo Leveling",
            AnimeMeta {
                matched: true,
                anilist_id: Some(3),
                ..Default::default()
            },
        );
        assert_eq!(
            st.get("https://d.com/x/", "Solo Leveling")
                .and_then(|m| m.anilist_id),
            Some(2)
        );
        st.clear("https://b.com/solo-leveling");
        assert!(st.get("https://b.com/solo-leveling/", "zzz").is_none());
    }
}
