//! Generic, config-driven engine for WordPress-style anime sites
//! (DooPlay, AnimeStream, Elementor …). Anything a profile leaves empty is
//! guessed with heuristics, so a new site often works with only `base` set.

use crate::http::{Http, par_map};
use crate::model::*;
use crate::profile::{SiteProfile, cache_dir};
use crate::source::Source;
use crate::stream::{self, Trace, find_variants, rank_of_embed, url_from_value};
use crate::text::*;
use anyhow::{Result, anyhow, bail};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// First path segments that are never anime pages.
const IGNORED: &[&str] = &[
    "category",
    "tag",
    "tags",
    "page",
    "genre",
    "genres",
    "author",
    "wp-content",
    "wp-admin",
    "wp-json",
    "wp-login.php",
    "feed",
    "search",
    "contact",
    "contact-us",
    "about",
    "about-us",
    "dmca",
    "privacy-policy",
    "privacy",
    "terms",
    "login",
    "register",
    "account",
    "cart",
    "year",
    "years",
    "release",
    "studio",
    "studios",
    "network",
    "director",
    "cast",
    "type",
    "seasons",
    "schedule",
    "az-list",
    "a-z",
    "random",
    "request",
    "blog",
    "news",
    "trending",
];

const PLAYER_FRAG: &str = "#wk-player=";
/// Episode `n` of a single-page series whose episodes are `[data-ep]` buttons.
const EP_FRAG: &str = "#wk-ep=";

/// "1th - 3" (anime-waku dub list) -> (season 1, episode 3)
fn parse_dub_label(t: &str) -> Option<(Option<u32>, f32)> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let c = RE.get_or_init(|| Regex::new(r"^\s*(\d+)\s*th\s*-\s*(\d+(?:\.\d+)?)").unwrap()).captures(t)?;
    Some((c[1].parse().ok(), c[2].parse().ok()?))
}

fn sel(s: &str) -> Option<Selector> {
    if s.trim().is_empty() {
        None
    } else {
        Selector::parse(s).ok()
    }
}

fn sel_must(s: &str) -> Selector {
    Selector::parse(s).expect("bad built-in selector")
}

fn text_of(el: ElementRef) -> String {
    clean(&el.text().collect::<String>())
}

/// Text of a heading without badge chips inside it (alpha-hen: `<h2><span class="al-ez-index-tag">Hentai</span>Title</h2>`).
fn text_wo_tags(el: ElementRef) -> String {
    let is_tag = |e: ElementRef| {
        e.value().attr("class").is_some_and(|c| c.split_whitespace().any(|w| w.contains("-tag") || w == "tag" || w.ends_with("badge")))
    };
    let mut s = String::new();
    for n in el.descendants() {
        let Some(t) = n.value().as_text() else { continue };
        let in_tag = n.ancestors().take_while(|a| a.id() != el.id()).filter_map(ElementRef::wrap).any(is_tag);
        if !in_tag {
            s.push_str(t);
        }
    }
    clean(&s)
}

/// Best image URL of an `<img>` (lazy-load attributes first).
fn img_url(img: ElementRef, page: &str) -> Option<String> {
    let v = img.value();
    for a in [
        "data-src",
        "data-lazy-src",
        "data-original",
        "data-lazy",
        "src",
    ] {
        if let Some(s) = v.attr(a) {
            let s = s.trim();
            if !s.is_empty() && !s.starts_with("data:") {
                return abs(page, s);
            }
        }
    }
    for a in ["data-srcset", "srcset"] {
        if let Some(s) = v.attr(a)
            && let Some(first) = s
                .split(',')
                .next()
                .and_then(|x| x.split_whitespace().next())
        {
            return abs(page, first);
        }
    }
    None
}

/// Poster in a style attribute: `background-image: url(...)`.
fn bg_url(el: ElementRef, page: &str) -> Option<String> {
    let st = el.value().attr("style")?;
    let i = st.find("url(")? + 4;
    let rest = &st[i..];
    let j = rest.find(')')?;
    abs(page, rest[..j].trim_matches(['"', '\'', ' ']))
}

/// Episode is the Thai dub (พากย์ไทย) and not the subbed one?
pub fn is_dub(text: &str) -> bool {
    let low = text.to_lowercase();
    low.contains("พากย์") || low.contains("เสียงไทย") || low.contains("dub") || low.contains("-thai-") || low.contains("/thai-") || low.contains("-th-ep") || {
        static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        // anime-waku: "1th - 3" = season 1, Thai track, episode 3
        RE.get_or_init(|| Regex::new(r"^\s*\d+\s*th\s*-\s*\d").unwrap()).is_match(&low)
    }
}

/// A link text that describes a whole series ("EP.1-26", "ตอนที่ 1-12") rather than one episode.
fn is_range_label(text: &str) -> bool {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(?:ep\.?|ตอนที่|episode)\s*\d+(?:\.\d+)?\s*[-–~]\s*\d+").unwrap()).is_match(text)
}

/// Is a text a usable title (not a score "8.3", a quality "HD", a language tag …)?
fn title_like(t: &str) -> bool {
    static JUNK: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let t = t.trim();
    let n = t.chars().count();
    if !(2..=200).contains(&n) {
        return false;
    }
    let junk = JUNK.get_or_init(|| {
        Regex::new(r"(?i)^(?:[\d.,/\s%★☆]+|hd|fhd|sd|4k|uhd|ซับไทย|พากย์ไทย|sub|dub|thai|new|ใหม่|จบแล้ว|ยังไม่จบ|ep\.?\s*\d+|ตอนที่\s*\d+|[\d.]+\s*(?:hd|fhd|sd)|play|ดูเลย|watch)$").unwrap()
    });
    !junk.is_match(t)
}

/// Best title among the texts of a card: headings / title captions first (longest), then the link's
/// `title`, image alt and finally the link text (which glues scores, quality and language tags together).
fn best_title(card: ElementRef, a: ElementRef) -> String {
    let ok = |t: &String| title_like(t) && !t.eq_ignore_ascii_case("play") && !t.contains("play-button");
    let longest = |mut v: Vec<String>| {
        v.retain(|t| ok(t));
        v.sort_by_key(|t| std::cmp::Reverse(t.chars().count().min(160)));
        v.into_iter().next()
    };
    let heads: Vec<String> =
        card.select(&sel_must("h1, h2, h3, h4, h5, .title, .tt, [class*=title], [class*=name]")).map(text_wo_tags).collect();
    if let Some(t) = longest(heads) {
        return t;
    }
    let mut other: Vec<String> = Vec::new();
    if let Some(t) = a.value().attr("title") {
        other.push(clean(t));
    }
    for img in card.select(&sel_must("img[alt]")) {
        other.push(clean(img.value().attr("alt").unwrap_or("")));
    }
    if let Some(t) = longest(other) {
        return t;
    }
    longest(vec![text_of(a)]).unwrap_or_default()
}

/// Poster-sized image of a card: lazy attributes first, skipping icons / play buttons / placeholders.
fn best_poster(card: ElementRef, page: &str, styles: &std::collections::HashMap<String, String>) -> Option<String> {
    let mut best: Option<(i64, String)> = None;
    for img in card.select(&sel_must("img")) {
        let Some(u) = img_url(img, page) else { continue };
        let low = u.to_lowercase();
        if low.ends_with(".svg") || ["play-button", "play.png", "icon", "logo", "placeholder", "thumbnail.jpg", "loading", "blank.gif", "spacer"].iter().any(|k| low.contains(k)) {
            continue;
        }
        let w: i64 = img.value().attr("width").and_then(|x| x.parse().ok()).unwrap_or(150);
        if w < 60 {
            continue;
        }
        if best.as_ref().is_none_or(|b| w > b.0) {
            best = Some((w, u));
        }
    }
    if let Some((_, u)) = best {
        return Some(u);
    }
    // background-image on the card or one of its children
    for el in std::iter::once(card).chain(card.descendants().filter_map(ElementRef::wrap)) {
        if let Some(u) = bg_url(el, page) {
            return Some(u);
        }
    }
    // Elementor loop items: background images live in a <style> block keyed by "e-loop-item-<id>"
    let class = card.value().attr("class").unwrap_or("");
    for c in class.split_whitespace() {
        if let Some(u) = styles.get(c) {
            return abs(page, u);
        }
    }
    for anc in card.ancestors().take(3).filter_map(ElementRef::wrap) {
        for c in anc.value().attr("class").unwrap_or("").split_whitespace() {
            if let Some(u) = styles.get(c) {
                return abs(page, u);
            }
        }
    }
    None
}

/// `.e-loop-item-123 … {background-image:url("…")}` rules of a page: class -> image url.
fn style_backgrounds(html: &str) -> std::collections::HashMap<String, String> {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#"\.(e-loop-item-\d+|post-\d+)[^{]{0,300}\{[^}]*background-image\s*:\s*url\(\s*["']?([^"')]+)"#).unwrap());
    let mut m = std::collections::HashMap::new();
    for c in re.captures_iter(html) {
        m.entry(c[1].to_string()).or_insert_with(|| c[2].to_string());
    }
    m
}

pub struct WpSource {
    p: SiteProfile,
    http: Http,
    series_re: Option<Regex>,
    episode_re: Option<Regex>,
    strip_re: Option<Regex>,
    index: Mutex<Option<Vec<(String, String)>>>,
}

impl WpSource {
    pub fn new(p: SiteProfile, http: Http) -> Result<Self> {
        let re = |s: &str, what: &str| -> Result<Option<Regex>> {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                Regex::new(s)
                    .map(Some)
                    .map_err(|e| anyhow!("[{}] bad {what} regex: {e}", p.id))
            }
        };
        Ok(Self {
            series_re: re(&p.series_path, "series_path")?,
            episode_re: re(&p.episode_path, "episode_path")?,
            strip_re: re(&p.title_strip, "title_strip")?,
            p,
            http,
            index: Mutex::new(None),
        })
    }

    pub fn profile(&self) -> &SiteProfile {
        &self.p
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http") {
            path.to_string()
        } else {
            format!("{}{}", self.p.base, path)
        }
    }

    fn get(&self, url: &str) -> Result<String> {
        self.http.get(url, Some(&format!("{}/", self.p.base)))
    }

    fn ignored(&self, segs: &[String]) -> bool {
        let Some(first) = segs.first() else {
            return true;
        };
        let f = first.to_lowercase();
        f.starts_with("wp-")
            || f.ends_with(".php")
            || f.ends_with(".xml")
            || IGNORED.contains(&f.as_str())
            || self
                .p
                .ignore_paths
                .iter()
                .any(|x| x.trim_matches('/').eq_ignore_ascii_case(&f))
    }

    fn episode_path_hint(path: &str) -> bool {
        let p = path.to_lowercase();
        p.starts_with("/ep/")
            || p.contains("/episode")
            || p.contains("-episode-")
            || p.contains("ตอนที่")
            || p.contains("-ep-")
    }

    pub fn is_episode_url(&self, url: &str, link_text: &str) -> bool {
        if !same_site(url, &self.p.base) {
            return false;
        }
        let path = url_path(url);
        match &self.episode_re {
            Some(re) => re.is_match(&path),
            None => {
                let segs = path_segs(url);
                !self.ignored(&segs)
                    && (Self::episode_path_hint(&path)
                        || looks_like_episode(link_text)
                        || looks_like_episode(&path))
            }
        }
    }

    pub fn is_series_url(&self, url: &str) -> bool {
        if !same_site(url, &self.p.base) {
            return false;
        }
        let segs = path_segs(url);
        if segs.is_empty() || self.ignored(&segs) {
            return false;
        }
        let path = url_path(url);
        if let Some(re) = &self.episode_re
            && re.is_match(&path)
        {
            return false;
        }
        match &self.series_re {
            Some(re) => re.is_match(&path),
            None => segs.len() <= 2 && !Self::episode_path_hint(&path),
        }
    }

    fn strip_title(&self, t: &str) -> String {
        let t = clean(t);
        let out = match &self.strip_re {
            Some(re) => re.replace(&t, "").trim().to_string(),
            None => t.clone(),
        };
        if out.is_empty() { t } else { out }
    }

    // ------------------------------------------------------------ listings

    /// Anime cards in any listing HTML (latest, search, archive).
    pub fn extract_cards(&self, html: &str, page: &str, need_poster: bool) -> Vec<AnimeCard> {
        let doc = Html::parse_document(html);
        let a_sel = sel_must("a[href]");
        let img_sel = sel_must("img");
        let mut out: Vec<AnimeCard> = Vec::new();

        let add = |out: &mut Vec<AnimeCard>,
                   url: String,
                   title: String,
                   poster: Option<String>,
                   badge: Option<String>| {
            let title = self.strip_title(&title);
            match out.iter_mut().find(|c| c.url == url) {
                Some(c) => {
                    if c.title.chars().count() < title.chars().count() {
                        c.title = title;
                    }
                    if c.poster.is_none() {
                        c.poster = poster;
                    }
                    if c.badge.is_none() {
                        c.badge = badge;
                    }
                }
                None => out.push(AnimeCard {
                    source: self.p.id.clone(),
                    url,
                    title,
                    poster,
                    badge,
                }),
            }
        };

        // explicit card containers
        if let Some(card_sel) = sel(&self.p.card_selector) {
            for card in doc.select(&card_sel) {
                let Some(a) = card.select(&a_sel).find(|a| {
                    a.value()
                        .attr("href")
                        .and_then(|h| abs(page, h))
                        .is_some_and(|u| self.is_series_url(&u) || self.is_episode_url(&u, ""))
                }) else {
                    continue;
                };
                let url = abs(page, a.value().attr("href").unwrap_or_default()).unwrap_or_default();
                let styles = style_backgrounds(html);
                let title = best_title(card, a);
                let poster = best_poster(card, page, &styles);
                let badge = card
                    .select(&sel_must(".epx, .ep, .episode, .quality, .label, .badge"))
                    .next()
                    .map(text_of);
                if !title.is_empty() {
                    add(
                        &mut out,
                        url,
                        title,
                        poster,
                        badge.filter(|b| !b.is_empty()),
                    );
                }
            }
            if !out.is_empty() {
                return out;
            }
        }

        // generic: every series link, poster from the link or its nearest single-link ancestor
        let styles = style_backgrounds(html);
        for a in doc.select(&a_sel) {
            let Some(url) = a.value().attr("href").and_then(|h| abs(page, h)) else {
                continue;
            };
            if !self.is_series_url(&url) {
                continue;
            }
            let mut poster = best_poster(a, page, &styles);
            let mut title = best_title(a, a);
            let mut badge = None;
            for anc in a.ancestors().take(4) {
                let Some(e) = ElementRef::wrap(anc) else {
                    continue;
                };
                let n_links = e
                    .select(&a_sel)
                    .filter(|x| {
                        x.value().attr("href").and_then(|h| abs(page, h)).as_deref()
                            == Some(url.as_str())
                            || x.select(&img_sel).next().is_some()
                    })
                    .count();
                let other_series = e
                    .select(&a_sel)
                    .filter_map(|x| x.value().attr("href").and_then(|h| abs(page, h)))
                    .filter(|u| *u != url && self.is_series_url(u))
                    .count();
                if other_series > 0 || n_links > 3 {
                    break;
                }
                if poster.is_none() {
                    poster = best_poster(e, page, &styles);
                }
                let t = best_title(e, a);
                if t.chars().count() > title.chars().count() && t.chars().count() < 200 {
                    title = t;
                }
                if badge.is_none() {
                    badge = e
                        .select(&sel_must(".epx, .ep, .episode, .label, .badge"))
                        .next()
                        .map(text_of)
                        .filter(|b| !b.is_empty());
                }
            }
            if !title_like(&title) {
                continue;
            }
            let heuristic = self.series_re.is_none();
            if title.is_empty()
                || (heuristic && poster.is_none())
                || (need_poster && poster.is_none())
            {
                continue;
            }
            add(&mut out, url, title, poster, badge);
        }
        out
    }

    // ------------------------------------------------------------ index

    fn index_file(&self) -> std::path::PathBuf {
        cache_dir().join(format!("index-{}.tsv", self.p.id))
    }

    /// (url, title-ish) of every series page, from the sitemap. Cached 12h.
    pub fn title_index(&self, force: bool) -> Vec<(String, String)> {
        if !self.p.sitemap_index {
            return Vec::new();
        }
        if !force && let Some(i) = self.index.lock().ok().and_then(|g| g.clone()) {
            return i;
        }
        let path = self.index_file();
        let read = || -> Vec<(String, String)> {
            std::fs::read_to_string(&path)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| {
                    l.split_once('\t')
                        .map(|(u, t)| (u.to_string(), t.to_string()))
                })
                .collect()
        };
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < Duration::from_secs(12 * 3600));
        if !force {
            // never built on the search path (sitemaps can take many seconds, through the browser for Cloudflare
            // sites): whatever is on disk, even old; `warm` rebuilds it in the background
            let idx = read();
            if let Ok(mut g) = self.index.lock() {
                *g = Some(idx.clone());
            }
            let _ = fresh;
            return idx;
        }
        let mut idx = Vec::new();
        if idx.is_empty() {
            idx = self.index_from_sitemaps();
            if idx.is_empty() {
                idx = read(); // stale is better than nothing
            } else {
                let _ = std::fs::create_dir_all(cache_dir());
                let body: String = idx
                    .iter()
                    .map(|(u, t)| format!("{u}\t{}\n", t.replace(['\t', '\n', '\r'], " ")))
                    .collect();
                let _ = std::fs::write(&path, body);
            }
        }
        if let Ok(mut g) = self.index.lock() {
            *g = Some(idx.clone());
        }
        idx
    }

    fn index_from_sitemaps(&self) -> Vec<(String, String)> {
        let roots = [
            "/wp-sitemap.xml",
            "/sitemap_index.xml",
            "/sitemap.xml",
            "/post-sitemap.xml",
            "/anime-sitemap.xml",
        ];
        let loc = Regex::new(r"<loc>\s*(?:<!\[CDATA\[)?\s*([^<\s\]]+)").unwrap();
        let mut entries: Vec<(String, String)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut queue: Vec<String> = roots.iter().rev().map(|r| self.url(r)).collect();
        while let Some(u) = queue.pop() {
            if seen.len() > 60 || !seen.insert(u.clone()) {
                continue;
            }
            let Ok(xml) = self.get(&u) else { continue };
            for cap in loc.captures_iter(&xml) {
                let l = unescape_html(&cap[1]);
                if l.contains(".xml") {
                    let low = l.to_lowercase();
                    let skip = [
                        "episode",
                        "-ep-",
                        "taxonomy",
                        "category",
                        "tag",
                        "user",
                        "author",
                        "page-sitemap",
                    ]
                    .iter()
                    .any(|k| low.contains(k));
                    if !skip {
                        queue.push(l);
                    }
                } else if self.is_series_url(&l) && !entries.iter().any(|e| e.0 == l) {
                    let slug = path_segs(&l).last().cloned().unwrap_or_default();
                    entries.push((l, percent_decode(&slug).replace('-', " ")));
                }
            }
        }
        entries
    }

    // ------------------------------------------------------------ episodes

    fn parse_episodes(&self, doc: &Html, page: &str) -> Vec<Episode> {
        let a_sel = sel_must("a[href]");
        let mut seen = HashSet::new();
        let mut eps: Vec<Episode> = Vec::new();
        let page_clean = page
            .split('#')
            .next()
            .unwrap_or(page)
            .trim_end_matches('/')
            .to_string();
        for el in doc.select(&a_sel) {
            let Some(full) = el.value().attr("href").and_then(|h| abs(page, h)) else {
                continue;
            };
            let full = full.split('#').next().unwrap_or(&full).to_string();
            let mut link_text = text_of(el);
            if link_text.is_empty() {
                // thumbnail link (aki-h): the caption is only in `title` / the image alt
                let alt = el.select(&sel_must("img[alt]")).next().and_then(|i| i.value().attr("alt")).unwrap_or("");
                link_text = clean(el.value().attr("title").filter(|t| !t.trim().is_empty()).unwrap_or(alt));
            }
            if full.trim_end_matches('/') == page_clean
                || !self.is_episode_url(&full, &link_text)
                || is_range_label(&link_text)
                || is_range_label(el.value().attr("title").unwrap_or(""))
                || !seen.insert(full.clone())
            {
                continue;
            }
            let mut season = None;
            let mut num: Option<f32> = None;
            let mut dub = is_dub(&link_text) || is_dub(&percent_decode(&url_path(&full)));
            // "S - E" label in the nearest ancestor that holds only this link
            for anc in el.ancestors().take(4) {
                let Some(a) = ElementRef::wrap(anc) else {
                    continue;
                };
                if a.select(&a_sel)
                    .filter_map(|x| x.value().attr("href").and_then(|h| abs(page, h)))
                    .filter(|u| self.is_episode_url(u, ""))
                    .collect::<HashSet<_>>()
                    .len()
                    > 1
                {
                    break;
                }
                let t = text_of(a);
                if let Some((s, n)) = parse_episode(&t).or_else(|| parse_dub_label(&t)) {
                    season = s;
                    num = Some(n);
                    dub |= is_dub(&t);
                    break;
                }
            }
            if num.is_none()
                && let Some((s, n)) = parse_episode(&link_text)
                    .or_else(|| parse_episode(&url_path(&full).replace('-', " ")))
            {
                season = s;
                num = Some(n);
            }
            eps.push(Episode {
                season: season.unwrap_or(1).max(1),
                num: num.unwrap_or(0.0),
                label: link_text,
                url: full,
                dub,
            });
        }
        // aki-h lists "ตอนที่ N ซับไทย" next to "Vol N Sub-Eng": the English-sub copies would clash on numbers
        let eng = |e: &Episode| {
            let t = format!("{} {}", e.label, percent_decode(&url_path(&e.url))).to_lowercase().replace('-', " ");
            t.contains("sub eng")
        };
        if eps.iter().any(|e| !eng(e)) && eps.iter().any(eng) {
            eps.retain(|e| !eng(e));
        }
        // a page linking to other series' episodes (sidebars, "related") must not add those: when most links
        // share a series key (first path segment / slug words), drop the outliers
        if eps.len() >= 3 {
            let key = |u: &str| {
                let segs = path_segs(u);
                if segs.len() >= 2 { segs[0].clone() } else { String::new() }
            };
            let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for e in &eps {
                *counts.entry(key(&e.url)).or_default() += 1;
            }
            if let Some((top, n)) = counts.iter().max_by_key(|x| x.1).map(|(k, n)| (k.clone(), *n))
                && !top.is_empty()
                && n * 2 > eps.len()
                && n < eps.len()
            {
                eps.retain(|e| key(&e.url) == top);
            }
        }
        let id = |u: &str| {
            path_segs(u)
                .last()
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(0)
        };
        eps.sort_by(|a, b| {
            (a.dub, a.season, a.num)
                .partial_cmp(&(b.dub, b.season, b.num))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(id(&a.url).cmp(&id(&b.url)))
        });
        if eps.iter().all(|e| e.num == 0.0) {
            for (i, e) in eps.iter_mut().enumerate() {
                e.num = (i + 1) as f32;
            }
        }
        // one entry per (track, season, number): mirrors of the same episode are servers, not episodes
        let mut keys = HashSet::new();
        eps.retain(|e| keys.insert((e.dub, e.season, (e.num * 100.0) as i64)));
        relabel(&mut eps);
        eps
    }

    /// Single-page sites (anime-sugoi): `<button data-ep="3" data-src="embed" data-src2="backup">`.
    fn data_ep_episodes(&self, doc: &Html, page: &str) -> Vec<Episode> {
        let mut eps: Vec<Episode> = Vec::new();
        for el in doc.select(&sel_must("[data-ep]")) {
            let v = el.value();
            let has_src = ["data-src", "data-src2", "data-url", "data-embed", "data-video"].iter().any(|a| v.attr(a).is_some_and(|x| !x.trim().is_empty()));
            if !has_src {
                continue;
            }
            let label = text_of(el);
            let Some(n) = v.attr("data-ep").and_then(|x| x.trim().parse::<f32>().ok()).or_else(|| parse_episode(&label).map(|x| x.1)) else { continue };
            let dub = is_dub(&label);
            if eps.iter().any(|e| e.num == n && e.dub == dub) {
                continue;
            }
            eps.push(Episode { season: 1, num: n, label, url: format!("{}{EP_FRAG}{}", page.split('#').next().unwrap_or(page), fmt_num(n)), dub });
        }
        eps.sort_by(|a, b| (a.dub, a.num).partial_cmp(&(b.dub, b.num)).unwrap_or(std::cmp::Ordering::Equal));
        relabel(&mut eps);
        eps
    }

    // ------------------------------------------------------------ players

    /// fairyanime: `<script src="/base/<id>/">` defines `var host = 'https://…/'` and builds player URLs as
    /// `host + "playback/f/<id>/"` per server tab.
    fn servers_from_loader_scripts(&self, html: &str, page: &str) -> Vec<Server> {
        static SRC: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        static VAR: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
        let src_re = SRC.get_or_init(|| Regex::new(r#"<script[^>]+src=["']([^"']+)["']"#).unwrap());
        let var_re = VAR.get_or_init(|| Regex::new(r#"var\s+(\w+)\s*=\s*['"](https?://[^'"]+)['"]"#).unwrap());
        let mut out: Vec<Server> = Vec::new();
        for c in src_re.captures_iter(html) {
            let Some(u) = abs(page, &c[1]) else { continue };
            let path = url_path(&u).to_lowercase();
            // dynamic same-site scripts only (not libraries)
            if !crate::text::same_site(&u, page) || path.ends_with(".js") || path.contains("/wp-") || path.contains("cdn-cgi") {
                continue;
            }
            let Ok(js) = self.http.get(&u, Some(page)) else { continue };
            for v in var_re.captures_iter(&js) {
                let (name, base) = (v[1].to_string(), v[2].to_string());
                let cat = Regex::new(&format!(r#"\b{}\s*\+\s*["']([^"']+)["']"#, regex::escape(&name))).unwrap();
                for m in cat.captures_iter(&js) {
                    let url = format!("{base}{}", &m[1]);
                    if out.iter().any(|s| s.kind == ServerKind::Direct(url.clone())) {
                        continue;
                    }
                    let name = if out.is_empty() { "Main".to_string() } else { format!("Backup {}", out.len()) };
                    out.push(Server { name, kind: ServerKind::Direct(url) });
                }
            }
        }
        out
    }

    fn servers_from_html(&self, html: &str, ep_url: &str) -> Vec<Server> {
        let doc = Html::parse_document(html);
        let mut out: Vec<Server> = Vec::new();
        let push = |out: &mut Vec<Server>, name: String, kind: ServerKind| {
            if !out.iter().any(|s| s.kind == kind) {
                let name = if name.is_empty() {
                    format!("Server {}", out.len() + 1)
                } else {
                    truncate(&name, 40)
                };
                out.push(Server { name, kind });
            }
        };

        // DooPlay: <li class="dooplay_player_option" data-post data-type data-nume>
        for li in doc.select(&sel_must(
            "li.dooplay_player_option, #playeroptionsul li, [data-post][data-nume]",
        )) {
            let v = li.value();
            let (Some(post), Some(nume)) = (v.attr("data-post"), v.attr("data-nume")) else {
                continue;
            };
            if nume == "trailer" {
                continue;
            }
            push(
                &mut out,
                text_of(li),
                ServerKind::DooPlay {
                    post: post.into(),
                    typ: v.attr("data-type").unwrap_or("tv").into(),
                    nume: nume.into(),
                },
            );
        }

        // aki-h: `window.displayvideo(0, 40819)` fills an iframe with `<web_uri>video/40819/`
        if let Ok(re) = Regex::new(r"displayvideo\(\s*\d+\s*,\s*(\d+)\s*\)")
            && let Some(c) = re.captures(html)
            && let Some(u) = abs(ep_url, &format!("/video/{}/", &c[1]))
        {
            push(&mut out, "Main".into(), ServerKind::Direct(u));
        }

        // <select> / tab buttons whose value is a URL, an <iframe>, or base64 of one
        // (alpha-hen: `<div class="mp-s-sl" data-id="https://…">Main Player</div>`)
        for el in doc.select(&sel_must("option[value], [data-src], [data-embed], [data-url], [data-video], [data-iframe], [data-link], .mp-s-sl[data-id]")) {
            let v = el.value();
            if v.name() == "img" || v.name() == "iframe" || v.name() == "script" {
                continue;
            }
            for a in ["value", "data-embed", "data-video", "data-iframe", "data-src", "data-url", "data-link", "data-id"] {
                if let Some(u) = v.attr(a).and_then(|x| url_from_value(x, ep_url)) {
                    if self.embed_candidate(&u, &text_of(el)) {
                        push(&mut out, text_of(el), ServerKind::Direct(u));
                    }
                    break;
                }
            }
        }

        // plain iframes
        for f in doc.select(&sel_must("iframe")) {
            let v = f.value();
            let src = v
                .attr("src")
                .filter(|s| !s.is_empty() && !s.starts_with("about:"))
                .or_else(|| v.attr("data-src"))
                .or_else(|| v.attr("data-lazy-src"));
            if let Some(s) = src.and_then(|s| abs(ep_url, s)) {
                let low = s.to_lowercase();
                if [
                    "googletagmanager",
                    "facebook.com",
                    "disqus",
                    "youtube.com/embed",
                ]
                .iter()
                .any(|j| low.contains(j))
                    && !low.contains("youtube.com/embed")
                {
                    continue;
                }
                if low.contains("youtube.com/embed") {
                    continue; // trailers
                }
                let n = out.len() + 1;
                push(&mut out, format!("Embed {n}"), ServerKind::Direct(s));
            }
        }

        // iframes written inside scripts / JSON (escaped quotes)
        if let Ok(re) = Regex::new(r#"(?i)<iframe[^>]+?src=\\?["']([^"'\\]+)"#) {
            for cap in re.captures_iter(html) {
                if let Some(full) = abs(ep_url, &cap[1].replace("\\/", "/"))
                    && !full.contains("googletagmanager")
                    && !full.contains("youtube.com/embed")
                {
                    let n = out.len() + 1;
                    push(&mut out, format!("Embed {n}"), ServerKind::Direct(full));
                }
            }
        }

        // last resort: raw media URLs anywhere in the page
        if out.is_empty() {
            let re = Regex::new(stream::MEDIA_PAT).unwrap();
            let txt = html.replace("\\/", "/").replace("&amp;", "&");
            for m in re.find_iter(&txt) {
                let n = out.len() + 1;
                push(
                    &mut out,
                    format!("Stream {n}"),
                    ServerKind::Direct(m.as_str().to_string()),
                );
            }
        }
        out
    }

    /// Is `u` (found in a player option) an embed rather than a link to another page of the site?
    fn embed_candidate(&self, u: &str, text: &str) -> bool {
        if !same_site(u, &self.p.base) || stream::is_media_path(u) {
            return true;
        }
        let p = url_path(u).to_lowercase();
        let playerish = [
            "player", "embed", "/v/", "/e/", "video", "stream", "iframe", "/play/", "watch",
        ]
        .iter()
        .any(|k| p.contains(k));
        playerish || !(self.is_series_url(u) || self.is_episode_url(u, text))
    }

    fn resolve_embed(&self, server: &Server, ep_url: &str, tr: &Trace) -> Result<String> {
        match &server.kind {
            ServerKind::Direct(u) => Ok(u.clone()),
            ServerKind::DooPlay { post, typ, nume } => {
                let b = &self.p.base;
                let mut errs: Vec<String> = Vec::new();
                // v1 first: what anime-waku's own player script calls (v2 is not served there, and every miss
                // costs a slow round trip through Cloudflare)
                for u in [
                    format!("{b}/wp-json/dooplayer/v1/post/{post}?type={typ}&source={nume}"),
                    format!("{b}/wp-json/dooplayer/v2/{post}/{typ}/{nume}"),
                ] {
                    tr.log(format!("  DooPlay GET {u}"));
                    match self.http.get(&u, Some(ep_url)) {
                        Ok(body) => match stream::extract_embed(&body, ep_url) {
                            Some(e) => return Ok(e),
                            None => errs.push(format!(
                                "{u}: no embed in response ({})",
                                truncate(&body, 80)
                            )),
                        },
                        Err(e) => errs.push(format!("{e:#}")),
                    }
                }
                let ajax = format!("{b}/wp-admin/admin-ajax.php");
                tr.log(format!("  DooPlay POST {ajax}"));
                match self.http.post_form(
                    &ajax,
                    ep_url,
                    &[
                        ("action", "doo_player_ajax"),
                        ("post", post),
                        ("nume", nume),
                        ("type", typ),
                    ],
                ) {
                    Ok((st, body)) => {
                        if (200..300).contains(&st)
                            && let Some(e) = stream::extract_embed(&body, ep_url)
                        {
                            return Ok(e);
                        }
                        errs.push(format!("POST {ajax} -> HTTP {st}: {}", truncate(&body, 80)));
                    }
                    Err(e) => errs.push(format!("{e:#}")),
                }
                Err(anyhow!(
                    "could not resolve player (post {post}, nume {nume}): {}",
                    errs.join(" | ")
                ))
            }
        }
    }
}

impl WpSource {
    /// Series page HTML -> detail (no network).
    pub fn parse_detail(&self, html: &str, url: &str) -> AnimeDetail {
        let doc = Html::parse_document(html);
        let meta = |prop: &str| -> Option<String> {
            doc.select(&sel_must(&format!(
                r#"meta[property="{prop}"], meta[name="{prop}"]"#
            )))
            .next()
            .and_then(|m| m.value().attr("content"))
            .map(|s| clean(&unescape_html(s)))
            .filter(|s| !s.is_empty())
        };

        let raw_title = sel(&self.p.title_selector)
            .and_then(|s| doc.select(&s).next().map(text_of))
            .filter(|t| !t.is_empty())
            .or_else(|| {
                doc.select(&sel_must("h1"))
                    .next()
                    .map(text_of)
                    .filter(|t| !t.is_empty())
            })
            .or_else(|| meta("og:title"))
            .unwrap_or_else(|| {
                percent_decode(&path_segs(url).last().cloned().unwrap_or_default())
                    .replace('-', " ")
            });

        let mut tags = Vec::new();
        for (th, label) in [
            ("ซับไทย", "Sub"),
            ("พากย์ไทย", "Dub"),
            ("ยังไม่จบ", "Ongoing"),
            ("จบแล้ว", "Completed"),
        ] {
            if raw_title.contains(th) {
                tags.push(label.to_string());
            }
        }
        let title = self.strip_title(&raw_title);

        let poster = sel(&self.p.poster_selector)
            .and_then(|s| doc.select(&s).next().and_then(|i| img_url(i, url)))
            .or_else(|| meta("og:image"))
            .or_else(|| {
                doc.select(&sel_must(
                    ".poster img, .thumb img, .entry-content img, article img",
                ))
                .next()
                .and_then(|i| img_url(i, url))
            });

        let synopsis = sel(&self.p.synopsis_selector)
            .and_then(|s| {
                // nested matches (".x p" and ".x") return the same text twice: keep the longest unique pieces
                let mut parts: Vec<String> = Vec::new();
                for t in doc
                    .select(&s)
                    .map(text_of)
                    .filter(|t| t.chars().count() > 20)
                {
                    if parts.iter().any(|p| p.contains(&t)) {
                        continue;
                    }
                    parts.retain(|p| !t.contains(p.as_str()));
                    parts.push(t);
                }
                let t = parts.join("\n\n");
                (!t.is_empty()).then_some(t)
            })
            .or_else(|| meta("og:description"))
            .or_else(|| meta("description"))
            .map(|t| crate::text::strip_tags(&t))
            .unwrap_or_default();

        let genre_sel = sel(&self.p.genre_selector).unwrap_or_else(|| {
            sel_must(r#".sgeneros a, .genres a, .genxed a, a[rel="tag"], a[href*="/genre/"]"#)
        });
        for g in doc
            .select(&genre_sel)
            .map(text_of)
            .filter(|g| !g.is_empty() && g.chars().count() < 25)
        {
            if !tags.contains(&g) && tags.len() < 10 {
                tags.push(g);
            }
        }

        let mut episodes = self.data_ep_episodes(&doc, url);
        if episodes.is_empty() {
            episodes = self.parse_episodes(&doc, url);
        }
        if episodes.is_empty() {
            // single-page sites: players live on the series page itself
            let servers = self.servers_from_html(html, url);
            let episodic: Vec<(usize, f32)> = servers
                .iter()
                .enumerate()
                .filter_map(|(i, s)| parse_episode(&s.name).map(|(_, n)| (i, n)))
                .collect();
            if episodic.len() >= 2 {
                episodes = episodic
                    .into_iter()
                    .map(|(i, n)| Episode {
                        season: 1,
                        num: n,
                        label: String::new(),
                        url: format!("{url}{PLAYER_FRAG}{i}"),
                        dub: false,
                    })
                    .collect();
                relabel(&mut episodes);
            } else if !servers.is_empty() {
                episodes.push(Episode {
                    season: 1,
                    num: 1.0,
                    label: "Watch".into(),
                    url: url.to_string(),
                    dub: false,
                });
            }
        }

        // a page that is the Thai-dub edition of a show (fairyanime "… เสียงไทย"): all its episodes are dubbed
        if !episodes.iter().any(|e| e.dub) && is_dub(&title) && !title.contains("ซับ") {
            for e in &mut episodes {
                e.dub = true;
            }
        }
        if episodes.iter().any(|e| e.dub) && !tags.iter().any(|t| t == "Dub") {
            tags.insert(0, "Dub".into());
        }
        if episodes.iter().any(|e| !e.dub) && !tags.iter().any(|t| t == "Sub") {
            tags.insert(0, "Sub".into());
        }
        AnimeDetail {
            source: self.p.id.clone(),
            url: url.to_string(),
            title,
            poster,
            synopsis,
            tags,
            episodes,
        }
    }
}

fn relabel(eps: &mut [Episode]) {
    let multi = eps.windows(2).any(|w| w[0].season != w[1].season);
    for e in eps.iter_mut() {
        let base = if multi {
            format!("S{} Episode {}", e.season, fmt_num(e.num))
        } else {
            format!("Episode {}", fmt_num(e.num))
        };
        e.label = base;
    }
}

impl Source for WpSource {
    fn warm(&self) {
        if !self.p.sitemap_index {
            return;
        }
        let stale = std::fs::metadata(self.index_file())
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_none_or(|age| age > Duration::from_secs(12 * 3600));
        if stale {
            let _ = self.title_index(true);
        }
    }

    fn id(&self) -> &str {
        &self.p.id
    }
    fn name(&self) -> &str {
        &self.p.name
    }
    fn base(&self) -> &str {
        &self.p.base
    }
    fn note(&self) -> &str {
        &self.p.note
    }

    fn latest(&self, page: u32) -> Result<Vec<AnimeCard>> {
        let mut last_err = None;
        for path in &self.p.latest_paths {
            let mut u = self.url(path);
            if page > 1 {
                if !u.ends_with('/') {
                    u.push('/');
                }
                u = format!("{u}page/{page}/");
            }
            match self.get(&u) {
                Ok(html) => {
                    let cards = self.extract_cards(&html, &u, false);
                    if !cards.is_empty() {
                        return Ok(cards);
                    }
                    last_err = Some(anyhow!("{u}: no anime cards recognised"));
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("no latest_paths configured")))
    }

    fn search(&self, query: &str) -> Result<Vec<AnimeCard>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let enc: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
        let mut cards: Vec<AnimeCard> = Vec::new();
        let mut ep_hits: Vec<String> = Vec::new();
        let mut remote_err = None;
        for page in 1..=2u32 {
            let mut u = self.url(&self.p.search_path.replace("{q}", &enc));
            if page > 1 {
                u = u.replacen("/?", &format!("/page/{page}/?"), 1);
            }
            let html = match self.get(&u) {
                Ok(h) => h,
                Err(e) => {
                    if page == 1 {
                        remote_err = Some(e);
                    }
                    break;
                }
            };
            let before = cards.len() + ep_hits.len();
            for c in self.extract_cards(&html, &u, false) {
                if !cards.iter().any(|x| x.url == c.url) {
                    cards.push(c);
                }
            }
            if self.episode_re.is_some() {
                let doc = Html::parse_document(&html);
                for a in doc.select(&sel_must("a[href]")) {
                    if let Some(full) = a.value().attr("href").and_then(|h| abs(&u, h))
                        && self.is_episode_url(&full, "")
                        && !ep_hits.contains(&full)
                    {
                        ep_hits.push(full);
                    }
                }
            }
            if cards.len() + ep_hits.len() == before || cards.len() - before.min(cards.len()) < 12 {
                break; // a second page only when the first one was full
            }
        }

        // episode hits -> their series page (only when the search found few series pages itself)
        ep_hits.truncate(if cards.len() < 3 { 6 } else { 0 });
        let from_eps: Vec<Vec<AnimeCard>> = par_map(&ep_hits, 6, |e| {
            self.get(e)
                .map(|h| {
                    self.extract_cards(&h, e, false)
                        .into_iter()
                        .take(1)
                        .collect()
                })
                .unwrap_or_default()
        });
        for c in from_eps.into_iter().flatten() {
            if !cards.iter().any(|x| x.url == c.url) {
                cards.push(c);
            }
        }

        // local sitemap index (fuzzy, offline)
        let idx = self.title_index(false);
        if !idx.is_empty() {
            let mut local: Vec<(f32, &(String, String))> = idx
                .iter()
                .map(|e| (score(q, &e.1), e))
                .filter(|x| x.0 >= 0.55)
                .collect();
            local.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            let missing: Vec<&(String, String)> = local
                .iter()
                .map(|x| x.1)
                .filter(|e| !cards.iter().any(|c| c.url == e.0))
                .take(10)
                .collect();
            // straight from the index (no page loads: the poster comes from the metadata / when opened)
            for e in missing {
                let title = self.strip_title(&e.1);
                cards.push(AnimeCard { source: self.p.id.clone(), url: e.0.clone(), title, poster: None, badge: None });
            }
        }

        if cards.is_empty()
            && let Some(e) = remote_err
        {
            return Err(e);
        }
        let slug = |u: &str| {
            percent_decode(&path_segs(u).last().cloned().unwrap_or_default()).replace('-', " ")
        };
        let mut scored: Vec<(f32, AnimeCard)> = cards
            .into_iter()
            .map(|c| (score(q, &format!("{} {}", c.title, slug(&c.url))), c))
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.title.len().cmp(&b.1.title.len()))
        });
        let top = scored.first().map(|x| x.0).unwrap_or(0.0);
        let cutoff = (top - 0.4).max(0.3);
        Ok(scored
            .into_iter()
            .filter(|x| x.0 >= cutoff)
            .map(|x| x.1)
            .collect())
    }

    fn detail(&self, url: &str) -> Result<AnimeDetail> {
        let html = self.get(url)?;
        // an episode page was opened: jump to its series page when one is linked
        if self.is_episode_url(url, "") && self.series_re.is_some() {
            let doc = Html::parse_document(&html);
            let series = doc
                .select(&sel_must(".breadcrumb a[href], #breadcrumbs a[href], .ep_infos a[href], .all-episodes a[href], a[href]"))
                .filter_map(|a| a.value().attr("href").and_then(|h| abs(url, h)))
                .find(|u| self.is_series_url(u));
            if let Some(s) = series {
                return self.detail(&s);
            }
        }
        Ok(self.parse_detail(&html, url))
    }

    fn servers(&self, episode_url: &str) -> Result<Vec<Server>> {
        if let Some((page, n)) = episode_url.split_once(EP_FRAG) {
            let html = self.get(page)?;
            let doc = Html::parse_document(&html);
            let want: f32 = n.parse().unwrap_or(-1.0);
            let mut list: Vec<Server> = Vec::new();
            for el in doc.select(&sel_must("[data-ep]")) {
                let v = el.value();
                let num = v.attr("data-ep").and_then(|x| x.trim().parse::<f32>().ok()).or_else(|| parse_episode(&text_of(el)).map(|x| x.1));
                if num.is_none_or(|x| (x - want).abs() > 0.01) {
                    continue;
                }
                for (i, a) in ["data-src", "data-src2", "data-src3", "data-url", "data-embed", "data-video"].iter().enumerate() {
                    if let Some(u) = v.attr(a).and_then(|x| url_from_value(x, page))
                        && !list.iter().any(|s| s.kind == ServerKind::Direct(u.clone()))
                    {
                        let name = if i == 0 { "Main".to_string() } else { format!("Backup {i}") };
                        list.push(Server { name, kind: ServerKind::Direct(u) });
                    }
                }
            }
            if list.is_empty() {
                bail!("episode {n} not found on {page}");
            }
            return Ok(list);
        }
        let (page, only) = match episode_url.split_once(PLAYER_FRAG) {
            Some((p, i)) => (p, i.parse::<usize>().ok()),
            None => (episode_url, None),
        };
        let html = self.get(page)?;
        let mut list = self.servers_from_html(&html, page);
        if let Some(i) = only {
            list = list.into_iter().nth(i).into_iter().collect();
        }
        if list.is_empty() {
            list = self.servers_from_loader_scripts(&html, page);
        }
        if list.is_empty() {
            // players loaded by script (anifume: `$.ajax({url: …})` fills the player box): resolve the page itself
            let txt = crate::media::normalize(&html);
            if txt.contains("$.ajax") || txt.contains("fetch(") || txt.contains("jwplayer") || txt.contains("sources") {
                list.push(Server { name: "Main".into(), kind: ServerKind::Direct(page.to_string()) });
            }
        }
        if list.is_empty() {
            bail!("no player found on {page} (markup unknown — adjust sources.toml)");
        }
        Ok(list)
    }

    fn resolve(&self, server: &Server, episode_url: &str, tr: &Trace) -> Result<Stream> {
        let page = episode_url.split(PLAYER_FRAG).next().unwrap_or(episode_url);
        let page = page.split(EP_FRAG).next().unwrap_or(page);
        let embed = self.resolve_embed(server, page, tr)?;
        tr.log(format!("  embed: {}", truncate(&embed, 140)));
        let base = Stream {
            source: self.p.id.clone(),
            server: server.name.clone(),
            variants: Vec::new(),
            page: page.to_string(),
            rank: StreamRank::Unknown,
            headers: Vec::new(),
            hls: false,
        };
        let referer = if embed == page { format!("{}/", self.p.base) } else { page.to_string() };
        if let Some(m) = crate::media::find_media(&self.http, &embed, &referer, tr) {
            let hls = m.urls.iter().any(|u| u.hls);
            let urls: Vec<String> = m.urls.iter().map(|u| u.url.clone()).collect();
            let mut variants: Vec<Variant> = m.urls.iter().map(|u| Variant { height: u.height, url: u.url.clone() }).collect();
            // sibling qualities (`…-1080.txt` -> `…-720.txt`) only for plain named playlists
            if variants.len() == 1 && variants[0].height.is_some() {
                variants = find_variants(&self.http, &urls, &m.page);
            }
            return Ok(Stream { variants, page: m.page, rank: StreamRank::Direct, headers: m.headers, hls, ..base });
        }
        Ok(Stream {
            variants: vec![Variant { height: None, url: embed.clone() }],
            rank: rank_of_embed(&embed),
            ..base
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::parse_sources;

    fn src(id: &str) -> WpSource {
        let p = parse_sources(crate::profile::DEFAULT_SOURCES)
            .unwrap()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap();
        WpSource::new(p, Http::new(5).unwrap()).unwrap()
    }

    const DOOPLAY_SERIES: &str = r#"
    <html><head><meta property="og:image" content="https://anime-waku.com/wp-content/uploads/f.jpg"></head><body>
      <h1>Sousou no Frieren คำอธิษฐานในวันที่จากลา ตอนที่ 1-28 ซับไทย จบแล้ว</h1>
      <div class="sgeneros"><a href="/genre/fantasy/">Fantasy</a></div>
      <div class="wp-content"><p>เรื่องราวของเอลฟ์จอมเวทหลังจากปราบจอมมารสำเร็จแล้ว การเดินทางครั้งใหม่ก็เริ่มต้นขึ้น</p></div>
      <ul class="episodios">
        <li><div class="numerando">1 - 2</div><div class="episodiotitle"><a href="https://anime-waku.com/ep/102/">ตอนที่ 2</a></div></li>
        <li><div class="numerando">1 - 1</div><div class="episodiotitle"><a href="https://anime-waku.com/ep/101/">ตอนที่ 1</a></div></li>
      </ul>
      <a href="https://anime-waku.com/anime/other-show/"><img src="x.jpg" alt="Other"></a>
    </body></html>"#;

    #[test]
    fn dooplay_series_page() {
        let s = src("waku");
        let d = s.parse_detail(DOOPLAY_SERIES, "https://anime-waku.com/anime/frieren/");
        assert_eq!(d.title, "Sousou no Frieren คำอธิษฐานในวันที่จากลา");
        assert_eq!(d.episodes.len(), 2);
        assert_eq!(d.episodes[0].num, 1.0);
        assert_eq!(d.episodes[0].label, "Episode 1");
        assert!(
            d.tags.contains(&"Completed".to_string()) && d.tags.contains(&"Fantasy".to_string())
        );
        assert_eq!(
            d.poster.as_deref(),
            Some("https://anime-waku.com/wp-content/uploads/f.jpg")
        );
        assert!(d.synopsis.starts_with("เรื่องราว"));
    }

    #[test]
    fn adult_sites_titles_and_episodes() {
        // alpha-hen: a category chip sits inside the card heading
        let s = src("alphahen");
        let cards = s.extract_cards(
            r#"<article><a href="https://www.alpha-hen.com/hitori-tsuma/"><img src="https://www.alpha-hen.com/a.jpg" width="200">
               <h2 class="entry-title ez-card-title"><span class="al-ez-index-tag">Hentai</span>Hitori Tsuma</h2></a></article>"#,
            "https://www.alpha-hen.com/",
            false,
        );
        assert_eq!(cards[0].title, "Hitori Tsuma");
        // aki-h: thumbnail link (no text) comes before the caption link; English-sub copies are dropped
        let s = src("akih");
        let ep = |id: &str, t: &str| {
            format!(r#"<div class="ak-ep"><a class="ak-ep__thumb" href="https://aki-h.com/watch/{id}/" title="{t}"><img alt="{t}"></a><h3><a href="https://aki-h.com/watch/{id}/">{t}</a></h3></div>"#)
        };
        let html = format!(
            "<h1>Enjo Kouhai</h1>{}{}{}",
            ep("aaa", "Enjo Kouhai ตอนที่ 1 ซับไทย"),
            ep("bbb", "Enjo Kouhai ตอนที่ 2 ซับไทย"),
            ep("ccc", "Enjo Kouhai Vol 2 Sub-Eng")
        );
        let d = s.parse_detail(&html, "https://aki-h.com/enjo-kouhai/");
        assert_eq!(d.episodes.iter().map(|e| e.num).collect::<Vec<_>>(), vec![1.0, 2.0]);
    }

    #[test]
    fn dooplay_players() {
        let s = src("waku");
        let html = r#"<ul id="playeroptionsul">
          <li class="dooplay_player_option" data-type="tv" data-post="555" data-nume="1"><span class="title">Main</span></li>
          <li class="dooplay_player_option" data-type="tv" data-post="555" data-nume="trailer"><span>Trailer</span></li>
        </ul>"#;
        let v = s.servers_from_html(html, "https://anime-waku.com/ep/101/");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Main");
        assert!(matches!(&v[0].kind, ServerKind::DooPlay { nume, .. } if nume == "1"));
    }

    #[test]
    fn heuristic_cards_and_single_page_players() {
        let s = src("anifume");
        let home = r#"<div class="grid">
            <article><a href="https://anifume.com/kaiju-no-8/"><img data-src="https://anifume.com/i/k8.jpg"></a><h2><a href="https://anifume.com/kaiju-no-8/">Kaiju No. 8 ซับไทย</a></h2><span class="epx">EP 12</span></article>
            <article><a href="https://anifume.com/dandadan/"><img src="https://anifume.com/i/dd.jpg" alt="Dandadan"></a></article>
            <a href="https://anifume.com/category/action/">Action</a>
            <a href="https://anifume.com/contact/"><img src="c.png"></a>
          </div>"#;
        let cards = s.extract_cards(home, "https://anifume.com/", false);
        let titles: Vec<&str> = cards.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["Kaiju No. 8", "Dandadan"]);
        assert_eq!(cards[0].badge.as_deref(), Some("EP 12"));

        let page = r#"<h1>Show</h1><select id="eps">
            <option value="https://host.com/e/1">ตอนที่ 1</option>
            <option value="PGlmcmFtZSBzcmM9Imh0dHBzOi8vaG9zdC5jb20vZS8yIj4=">ตอนที่ 2</option></select>"#;
        let d = s.parse_detail(page, "https://anifume.com/show/");
        assert_eq!(d.episodes.len(), 2);
        assert_eq!(d.episodes[1].url, "https://anifume.com/show/#wk-player=1");
    }

    #[test]
    fn url_classes() {
        let s = src("waku");
        assert!(s.is_series_url("https://anime-waku.com/anime/frieren/"));
        assert!(!s.is_series_url("https://anime-waku.com/ep/123/"));
        assert!(s.is_episode_url("https://anime-waku.com/ep/123/", ""));
        let h = src("hareshi");
        assert!(h.is_series_url("https://hareshi.net/one-piece/"));
        assert!(!h.is_series_url("https://www.hareshi.net/category/x/"));
        assert!(!h.is_series_url("https://other.com/one-piece/"));
        assert!(h.is_episode_url("https://www.hareshi.net/one-piece-ep-5/", ""));
    }
}

#[cfg(test)]
mod card_tests {
    use super::*;

    #[test]
    fn junk_titles_rejected() {
        assert!(!title_like("8.3 HD"));
        assert!(!title_like("8.3"));
        assert!(title_like("Black Clover Season 2"));
        let html = r#"<a href="https://x.net/cyber-2026/"><div class="elementor-heading-title">8.3</div><div class="elementor-icon"><svg></svg></div><div class="elementor-heading-title">HD</div></a>"#;
        let doc = Html::parse_fragment(html);
        let a = doc.select(&sel_must("a")).next().unwrap();
        assert_eq!(best_title(a, a), "");
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::profile::parse_sources;

    fn src(id: &str) -> WpSource {
        let p = parse_sources(crate::profile::DEFAULT_SOURCES).unwrap().into_iter().find(|p| p.id == id).unwrap();
        WpSource::new(p, Http::new(5).unwrap()).unwrap()
    }

    /// Real pages saved by the dev agent (skipped when the fixture is missing).
    #[test]
    fn sugoi_home_cards() {
        let Ok(html) = std::fs::read_to_string("/tmp/claude-0/fx/sugoi.html") else { return };
        for c in src("sugoi").extract_cards(&html, "https://anime-sugoi.net/", false).iter().take(40) {
            println!("{} | {} | {:?}", c.title, c.url, c.poster);
        }
    }
}
