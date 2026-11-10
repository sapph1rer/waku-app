//! Generic, config-driven engine for WordPress-style anime sites
//! (DooPlay, AnimeStream, Elementor …). Anything a profile leaves empty is
//! guessed with heuristics, so a new site often works with only `base` set.

use crate::http::{Http, par_map};
use crate::model::*;
use crate::profile::{SiteProfile, cache_dir};
use crate::source::Source;
use crate::stream::{self, Trace, find_variants, rank_of_embed, sniff_stream, url_from_value};
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
        if let Some(s) = v.attr(a) {
            if let Some(first) = s
                .split(',')
                .next()
                .and_then(|x| x.split_whitespace().next())
            {
                return abs(page, first);
            }
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
        if let Some(re) = &self.episode_re {
            if re.is_match(&path) {
                return false;
            }
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
                let img = card.select(&img_sel).next();
                let title = card
                    .select(&sel_must("h2, h3, h4, .title, .tt"))
                    .next()
                    .map(text_of)
                    .filter(|t| !t.is_empty())
                    .or_else(|| a.value().attr("title").map(clean))
                    .or_else(|| img.and_then(|i| i.value().attr("alt")).map(clean))
                    .unwrap_or_else(|| text_of(a));
                let poster = img.and_then(|i| img_url(i, page));
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
        for a in doc.select(&a_sel) {
            let Some(url) = a.value().attr("href").and_then(|h| abs(page, h)) else {
                continue;
            };
            if !self.is_series_url(&url) {
                continue;
            }
            let mut poster = a
                .select(&img_sel)
                .next()
                .and_then(|i| img_url(i, page))
                .or_else(|| bg_url(a, page));
            let mut title = a.value().attr("title").map(clean).unwrap_or_default();
            if title.is_empty() {
                title = text_of(a);
            }
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
                    poster = e.select(&img_sel).next().and_then(|i| img_url(i, page));
                }
                if title.is_empty() || title.chars().count() < 3 {
                    if let Some(t) = e
                        .select(&sel_must("h2, h3, h4, .title, .tt"))
                        .next()
                        .map(text_of)
                    {
                        title = t;
                    }
                }
                if badge.is_none() {
                    badge = e
                        .select(&sel_must(".epx, .ep, .episode, .label, .badge"))
                        .next()
                        .map(text_of)
                        .filter(|b| !b.is_empty());
                }
            }
            if title.is_empty() {
                title = a
                    .select(&img_sel)
                    .next()
                    .and_then(|i| i.value().attr("alt"))
                    .map(clean)
                    .unwrap_or_default();
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
        if !force {
            if let Some(i) = self.index.lock().ok().and_then(|g| g.clone()) {
                return i;
            }
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
        let mut idx = if fresh && !force { read() } else { Vec::new() };
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
            let link_text = text_of(el);
            if full.trim_end_matches('/') == page_clean
                || !self.is_episode_url(&full, &link_text)
                || !seen.insert(full.clone())
            {
                continue;
            }
            let mut season = None;
            let mut num: Option<f32> = None;
            // "S - E" label in the nearest ancestor that holds only this link
            for anc in el.ancestors().take(4) {
                let Some(a) = ElementRef::wrap(anc) else {
                    continue;
                };
                if a.select(&a_sel)
                    .filter(|x| {
                        x.value()
                            .attr("href")
                            .and_then(|h| abs(page, h))
                            .is_some_and(|u| self.is_episode_url(&u, ""))
                    })
                    .count()
                    > 1
                {
                    break;
                }
                if let Some((s, n)) = parse_episode(&text_of(a)) {
                    season = s;
                    num = Some(n);
                    break;
                }
            }
            if num.is_none() {
                if let Some((s, n)) = parse_episode(&link_text)
                    .or_else(|| parse_episode(&url_path(&full).replace('-', " ")))
                {
                    season = s;
                    num = Some(n);
                }
            }
            eps.push(Episode {
                season: season.unwrap_or(1).max(1),
                num: num.unwrap_or(0.0),
                label: link_text,
                url: full,
            });
        }
        let id = |u: &str| {
            path_segs(u)
                .last()
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(0)
        };
        eps.sort_by(|a, b| {
            (a.season, a.num)
                .partial_cmp(&(b.season, b.num))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(id(&a.url).cmp(&id(&b.url)))
        });
        if eps.iter().all(|e| e.num == 0.0) {
            for (i, e) in eps.iter_mut().enumerate() {
                e.num = (i + 1) as f32;
            }
        }
        // the same number twice (sub + dub pages): keep both but number them apart in the label
        relabel(&mut eps);
        eps
    }

    // ------------------------------------------------------------ players

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

        // <select> / tab buttons whose value is a URL, an <iframe>, or base64 of one
        for el in doc.select(&sel_must("option[value], [data-src], [data-embed], [data-url], [data-video], [data-iframe], [data-link]")) {
            let v = el.value();
            if v.name() == "img" || v.name() == "iframe" || v.name() == "script" {
                continue;
            }
            for a in ["value", "data-embed", "data-video", "data-iframe", "data-src", "data-url", "data-link"] {
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
                if let Some(full) = abs(ep_url, &cap[1].replace("\\/", "/")) {
                    if !full.contains("googletagmanager") && !full.contains("youtube.com/embed") {
                        let n = out.len() + 1;
                        push(&mut out, format!("Embed {n}"), ServerKind::Direct(full));
                    }
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
                for u in [
                    format!("{b}/wp-json/dooplayer/v2/{post}/{typ}/{nume}"),
                    format!("{b}/wp-json/dooplayer/v1/post/{post}?type={typ}&source={nume}"),
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
                        if (200..300).contains(&st) {
                            if let Some(e) = stream::extract_embed(&body, ep_url) {
                                return Ok(e);
                            }
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
            ("ซับไทย", "ซับไทย"),
            ("พากย์ไทย", "พากย์ไทย"),
            ("ยังไม่จบ", "ยังไม่จบ"),
            ("จบแล้ว", "จบแล้ว"),
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

        let mut episodes = self.parse_episodes(&doc, url);
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
                    })
                    .collect();
                relabel(&mut episodes);
            } else if !servers.is_empty() {
                episodes.push(Episode {
                    season: 1,
                    num: 1.0,
                    label: "ดูเลย".into(),
                    url: url.to_string(),
                });
            }
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
            format!("S{} ตอนที่ {}", e.season, fmt_num(e.num))
        } else {
            format!("ตอนที่ {}", fmt_num(e.num))
        };
        let low = e.label.to_lowercase();
        let extra = if low.contains("พากย์") || low.contains("dub") {
            " (พากย์ไทย)"
        } else {
            ""
        };
        e.label = format!("{base}{extra}");
    }
}

impl Source for WpSource {
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
                    if let Some(full) = a.value().attr("href").and_then(|h| abs(&u, h)) {
                        if self.is_episode_url(&full, "") && !ep_hits.contains(&full) {
                            ep_hits.push(full);
                        }
                    }
                }
            }
            if cards.len() + ep_hits.len() == before {
                break;
            }
        }

        // episode hits -> their series page
        ep_hits.truncate(6);
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
            let loaded: Vec<Option<AnimeCard>> =
                par_map(&missing, 8, |e| self.detail(&e.0).ok().map(|d| d.card()));
            cards.extend(loaded.into_iter().flatten());
        }

        if cards.is_empty() {
            if let Some(e) = remote_err {
                return Err(e);
            }
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
            bail!("no player found on {page} (markup unknown — adjust sources.toml)");
        }
        Ok(list)
    }

    fn resolve(&self, server: &Server, episode_url: &str, tr: &Trace) -> Result<Stream> {
        let page = episode_url.split(PLAYER_FRAG).next().unwrap_or(episode_url);
        let embed = self.resolve_embed(server, page, tr)?;
        tr.log(format!("  embed: {}", truncate(&embed, 140)));
        let base = Stream {
            source: self.p.id.clone(),
            server: server.name.clone(),
            variants: Vec::new(),
            page: page.to_string(),
            rank: StreamRank::Unknown,
        };
        Ok(match sniff_stream(&self.http, &embed, page, tr) {
            Some((urls, found_on)) => Stream {
                variants: find_variants(&self.http, &urls, &found_on),
                page: found_on,
                rank: StreamRank::Direct,
                ..base
            },
            None => Stream {
                variants: vec![Variant {
                    height: None,
                    url: embed.clone(),
                }],
                rank: rank_of_embed(&embed),
                ..base
            },
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
        assert_eq!(d.episodes[0].label, "ตอนที่ 1");
        assert!(d.tags.contains(&"จบแล้ว".to_string()) && d.tags.contains(&"Fantasy".to_string()));
        assert_eq!(
            d.poster.as_deref(),
            Some("https://anime-waku.com/wp-content/uploads/f.jpg")
        );
        assert!(d.synopsis.starts_with("เรื่องราว"));
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
