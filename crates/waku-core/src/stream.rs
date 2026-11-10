//! Turning an embed page into a playable stream (ported from Waku-cli):
//! follow nested iframes / JSON links, decode `atob()` blobs, find
//! `.m3u8` / `.mp4` / disguised `/m3u8/*.txt` playlists, probe sibling qualities.

use crate::http::{Http, par_map};
use crate::model::{StreamRank, Variant};
use crate::text::{abs, origin_of, truncate};
use regex::Regex;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Direct media: `*.m3u8` / `*.mp4`, or an HLS playlist served as `/m3u8/<id>.txt`.
pub const MEDIA_PAT: &str =
    r#"https?://[^"'\s<>\\]+?(?:\.(?:m3u8|mp4)|/m3u8/[^"'\s<>\\?]+\.txt)[^"'\s<>\\]*"#;

fn media_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(MEDIA_PAT).unwrap())
}

/// Collects step-by-step notes for the source tester (no-op when disabled).
#[derive(Default)]
pub struct Trace {
    on: bool,
    lines: Mutex<Vec<String>>,
}

impl Trace {
    pub fn enabled() -> Self {
        Self {
            on: true,
            lines: Mutex::new(Vec::new()),
        }
    }
    pub fn off() -> Self {
        Self::default()
    }
    pub fn log(&self, msg: impl Into<String>) {
        if self.on
            && let Ok(mut l) = self.lines.lock()
        {
            l.push(msg.into());
        }
    }
    pub fn take(&self) -> Vec<String> {
        self.lines
            .lock()
            .map(|mut l| std::mem::take(&mut *l))
            .unwrap_or_default()
    }
}

pub fn is_media_path(url: &str) -> bool {
    let p = url.to_lowercase();
    let p = p.split('?').next().unwrap_or("");
    p.ends_with(".m3u8") || p.ends_with(".mp4") || (p.contains("/m3u8/") && p.ends_with(".txt"))
}

/// Hosts mpv's yt-dlp hook understands: hand the page URL over as-is.
pub fn ytdlp_host(url: &str) -> bool {
    let u = url.to_lowercase();
    [
        "ok.ru",
        "youtube.com",
        "youtu.be",
        "dailymotion.com",
        "vimeo.com",
        "streamable.com",
        "drive.google.com",
        "bilibili.",
    ]
    .iter()
    .any(|h| u.contains(h))
}

fn followable(u: &str) -> bool {
    let low = u.to_lowercase();
    let path = low.split(['?', '#']).next().unwrap_or("");
    let assets = [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".ico", ".css", ".js", ".woff", ".woff2",
        ".ttf", ".vtt", ".srt",
    ];
    let junk = [
        "googletagmanager",
        "google-analytics",
        "gstatic.com",
        "googleapis.com",
        "facebook.",
        "histats",
        "doubleclick",
        "/cdn-cgi/",
        "twitter.com",
        "line.me",
    ];
    !assets.iter().any(|e| path.ends_with(e)) && !junk.iter().any(|j| low.contains(j))
}

/// Minimal base64 decoder (standard + url-safe alphabets).
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in s.bytes() {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// A value that may be a URL, an `<iframe>` snippet, or base64 of either.
pub fn url_from_value(v: &str, page: &str) -> Option<String> {
    let v = v.trim();
    if v.len() < 8 {
        return None;
    }
    if v.starts_with("http") || v.starts_with("//") {
        return abs(page, v).filter(|u| followable(u));
    }
    if let Some(e) = extract_embed(v, page) {
        return Some(e);
    }
    if v.len() >= 16 && !v.contains(' ') {
        let dec = b64_decode(v).and_then(|b| String::from_utf8(b).ok())?;
        return extract_embed(&dec, page);
    }
    None
}

/// Pull an embed / stream URL out of a player API response (JSON, an
/// `<iframe>` snippet, or a bare URL).
pub fn extract_embed(body: &str, page: &str) -> Option<String> {
    static IFRAME: OnceLock<Regex> = OnceLock::new();
    let t = body.trim();
    let mut text = t.to_string();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(t) {
        for k in ["embed_url", "url", "file", "src", "embed", "iframe", "link"] {
            if let Some(x) = v.get(k).and_then(|x| x.as_str()) {
                text = x.to_string();
                break;
            }
        }
    }
    let text = text.replace("\\/", "/").replace("&amp;", "&");
    let iframe =
        IFRAME.get_or_init(|| Regex::new(r#"(?i)<iframe[^>]+src=["']([^"']+)["']"#).unwrap());
    if let Some(c) = iframe.captures(&text) {
        return abs(page, &c[1]);
    }
    let one = text.trim();
    if (one.starts_with("http") || one.starts_with("//"))
        && !one.contains(char::is_whitespace)
        && !one.contains('<')
    {
        return abs(page, one);
    }
    media_re().find(&text).map(|m| m.as_str().to_string())
}

/// Other pages worth opening from this one: iframes, plus JSON/JS fields such as
/// `"src": "https://…"`, `file: '…'`, `embed_url` …
fn candidates(txt: &str, page: &str) -> Vec<String> {
    static IFR: OnceLock<Regex> = OnceLock::new();
    static KV: OnceLock<Regex> = OnceLock::new();
    let mut out: Vec<String> = Vec::new();
    let mut push = |raw: &str| {
        if let Some(full) = abs(page, raw)
            && followable(&full)
            && full != page
            && !out.contains(&full)
        {
            out.push(full);
        }
    };
    let ifr = IFR.get_or_init(|| Regex::new(r#"(?i)<iframe[^>]+?\ssrc=["']([^"']+)["']"#).unwrap());
    for c in ifr.captures_iter(txt) {
        push(&c[1]);
    }
    let kv = KV.get_or_init(|| {
        Regex::new(r#"(?i)["']?(?:src|file|url|embed|embed_url|iframe|link|source|player)["']?\s*[:=]\s*["'](https?://[^"'\s<>]+)["']"#).unwrap()
    });
    for c in kv.captures_iter(txt) {
        push(&c[1]);
    }
    out
}

/// Look for a direct stream behind an embed. Returns (stream urls, page they were on).
pub fn sniff_stream(
    http: &Http,
    url: &str,
    referer: &str,
    tr: &Trace,
) -> Option<(Vec<String>, String)> {
    let mut seen = HashSet::new();
    sniff(http, url, referer, 5, &mut seen, tr)
}

fn sniff(
    http: &Http,
    url: &str,
    referer: &str,
    depth: u8,
    seen: &mut HashSet<String>,
    tr: &Trace,
) -> Option<(Vec<String>, String)> {
    if is_media_path(url) {
        return Some((vec![url.to_string()], referer.to_string()));
    }
    if depth == 0 || ytdlp_host(url) || !seen.insert(url.to_string()) {
        return None;
    }
    tr.log(format!("  hop GET {}", truncate(url, 140)));
    let html = match http.get(url, Some(referer)) {
        Ok(h) => h,
        Err(e) => {
            tr.log(format!("    failed: {e:#}"));
            return None;
        }
    };
    let txt = html
        .replace("\\/", "/")
        .replace("\\u0026", "&")
        .replace("&amp;", "&");
    let mut found: Vec<String> = Vec::new();
    for m in media_re().find_iter(&txt) {
        let u = m.as_str().to_string();
        if !found.contains(&u) {
            found.push(u);
        }
    }
    if !found.is_empty() {
        tr.log(format!("    found {} stream url(s)", found.len()));
        return Some((found, url.to_string()));
    }
    static ATOB: OnceLock<Regex> = OnceLock::new();
    let atob = ATOB
        .get_or_init(|| Regex::new(r#"atob\(\s*["']([A-Za-z0-9+/=_-]{16,})["']\s*\)"#).unwrap());
    for cap in atob.captures_iter(&txt) {
        if let Some(dec) = b64_decode(&cap[1]).and_then(|b| String::from_utf8(b).ok()) {
            let dec = dec.replace("\\/", "/");
            if let Some(m) = media_re().find(&dec) {
                tr.log("    found stream in atob() blob");
                return Some((vec![m.as_str().to_string()], url.to_string()));
            }
        }
    }
    let next: Vec<String> = candidates(&txt, url).into_iter().take(8).collect();
    for u in next {
        if let Some(found) = sniff(http, &u, url, depth - 1, seen, tr) {
            return Some(found);
        }
    }
    None
}

// ------------------------------------------------------------------ quality

/// Height encoded in a playlist URL: `…-1080.txt`, `…_720p.m3u8`, `…/480.mp4`
pub fn quality_of(url: &str) -> Option<u32> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re =
        RE.get_or_init(|| Regex::new(r"(?i)[-_/.](\d{3,4})p?\.(?:txt|m3u8|mp4)(?:\?|$)").unwrap());
    re.captures(url)
        .and_then(|c| c[1].parse::<u32>().ok())
        .filter(|h| (144..=4320).contains(h))
}

/// Same playlist URL with another height (`…-1080.txt` -> `…-720.txt`).
pub fn quality_sibling(url: &str, h: u32) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(.*[-_/.])(\d{3,4})(p?)(\.(?:txt|m3u8)(?:\?.*)?)$").unwrap()
    });
    let c = re.captures(url)?;
    Some(format!("{}{}{}{}", &c[1], h, &c[3], &c[4]))
}

fn probe_playlist(http: &Http, url: &str, referer: &str) -> bool {
    http.get(url, Some(referer))
        .map(|b| b.trim_start().starts_with("#EXTM3U"))
        .unwrap_or(false)
}

/// Every quality we can offer: the URLs found, plus sibling heights that answer.
pub fn find_variants(http: &Http, urls: &[String], page: &str) -> Vec<Variant> {
    let mut v: Vec<Variant> = Vec::new();
    for u in urls {
        if !v.iter().any(|x| x.url == *u) {
            v.push(Variant {
                height: quality_of(u),
                url: u.clone(),
            });
        }
    }
    if let Some(base) = urls
        .iter()
        .find(|u| quality_of(u).is_some() && quality_sibling(u, 720).is_some())
    {
        let referer = origin_of(page);
        let wanted: Vec<u32> = [1080u32, 720, 480, 360]
            .into_iter()
            .filter(|h| !v.iter().any(|x| x.height == Some(*h)))
            .collect();
        let extra: Vec<Variant> = par_map(&wanted, 4, |&h| {
            quality_sibling(base, h)
                .filter(|u| probe_playlist(http, u, &referer))
                .map(|u| Variant {
                    height: Some(h),
                    url: u,
                })
        })
        .into_iter()
        .flatten()
        .collect();
        v.extend(extra);
    }
    v.sort_by(|a, b| b.height.cmp(&a.height));
    v
}

/// "best" | "worst" | "1080" | "720p" … -> closest available variant.
pub fn pick_variant<'a>(vs: &'a [Variant], want: &str) -> &'a Variant {
    let w = want.trim().to_lowercase();
    let w = w.trim_end_matches('p');
    let known = || vs.iter().filter(|v| v.height.is_some());
    match w {
        "" | "best" | "auto" | "max" => &vs[0],
        "worst" | "min" | "low" => known().next_back().unwrap_or(&vs[0]),
        n => match n.parse::<u32>() {
            Ok(h) => known()
                .find(|v| v.height.is_some_and(|x| x <= h))
                .or_else(|| known().next_back())
                .unwrap_or(&vs[0]),
            Err(_) => &vs[0],
        },
    }
}

pub fn rank_of_embed(embed: &str) -> StreamRank {
    if is_media_path(embed) {
        StreamRank::Direct
    } else if ytdlp_host(embed) {
        StreamRank::YtDlp
    } else {
        StreamRank::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vs(hs: &[Option<u32>]) -> Vec<Variant> {
        hs.iter()
            .map(|h| Variant {
                height: *h,
                url: format!("u{h:?}"),
            })
            .collect()
    }

    #[test]
    fn quality_from_url() {
        let u = "https://player.doodee-player.com/m3u8/e74ffcac-480.txt";
        assert_eq!(quality_of(u), Some(480));
        assert_eq!(quality_of("https://x.com/a/720p.m3u8?t=1"), Some(720));
        assert_eq!(quality_of("https://x.com/a/master.m3u8"), None);
        assert_eq!(
            quality_sibling(u, 1080).as_deref(),
            Some("https://player.doodee-player.com/m3u8/e74ffcac-1080.txt")
        );
        assert_eq!(quality_sibling("https://x.com/a/clip.mp4", 360), None);
    }

    #[test]
    fn picks_closest_quality() {
        let v = vs(&[Some(1080), Some(720), Some(480), Some(360)]);
        assert_eq!(pick_variant(&v, "best").height, Some(1080));
        assert_eq!(pick_variant(&v, "worst").height, Some(360));
        assert_eq!(pick_variant(&v, "720p").height, Some(720));
        assert_eq!(pick_variant(&v, "600").height, Some(480));
        assert_eq!(pick_variant(&v, "144").height, Some(360));
        assert_eq!(pick_variant(&vs(&[None]), "720").height, None);
    }

    #[test]
    fn embeds() {
        let page = "https://site.com/ep/1/";
        assert_eq!(
            extract_embed(
                r#"{"embed_url":"<iframe src=\"https:\/\/h.com\/e\/1\"></iframe>"}"#,
                page
            )
            .as_deref(),
            Some("https://h.com/e/1")
        );
        // base64 of <iframe src="https://h.com/v/9">
        let b = "PGlmcmFtZSBzcmM9Imh0dHBzOi8vaC5jb20vdi85Ij4=";
        assert_eq!(
            url_from_value(b, page).as_deref(),
            Some("https://h.com/v/9")
        );
        let c = candidates(
            r#"{"src": "https://p.doodee-player.com/?token=ab", "img": "https://a.com/l.png"}"#,
            page,
        );
        assert_eq!(c, vec!["https://p.doodee-player.com/?token=ab"]);
    }
}
