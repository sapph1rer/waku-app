//! Finding the real video behind an embed page.
//!
//! What the Thai anime hosts do (studied on anime-waku, anifume, animeruka, anime-sugoi, fairyanime):
//!  * jQuery `$.ajax({url: "/k49…"})` that returns the `<iframe>` (anifume),
//!  * JW Player `sources: [{file, label}]` with MP4/HLS per quality (anifume, doodee-player),
//!  * Inertia/Vue apps with the video in an HTML-escaped JSON attribute `data-page="{…"url":"…"}"` (animemami),
//!  * an obfuscated bootstrap: reversed URL-safe base64 of URI-encoded JSON with `manifestUrl` (nya.animenani),
//!  * `atob("…")` blobs, nested iframes, JSON server lists (`"src": "https://…"`),
//!  * HLS playlists without an `.m3u8` name (`/hls/<id>.txt`, `/o/<id>/a`), served as JSON `{"p": base64}`,
//!    with segments disguised as `.webp` (rmp's HLS proxy plays those).
//!
//! `find_media` walks those layers (depth-limited) and returns the playable URLs with the HTTP context the
//! CDN expects (Referer / Origin of the page the player lives on). Unknown URLs found in player-ish JSON
//! fields are *probed* (first bytes): an `#EXTM3U` (or encoded one) means HLS, `ftyp`/`video/*` means MP4.

use crate::http::Http;
use crate::stream::{Trace, b64_decode, ytdlp_host};
use crate::text::{abs, origin_of, percent_decode, truncate};
use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaUrl {
    pub url: String,
    /// 1080 / 720 … from a label or the URL
    pub height: Option<u32>,
    /// HLS (possibly disguised) — the player must treat it as HLS
    pub hls: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub urls: Vec<MediaUrl>,
    /// page the player lives on (Referer)
    pub page: String,
    /// extra headers the CDN wants ("Origin: …")
    pub headers: Vec<String>,
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

/// HTML entities + JS escapes that hide URLs from simple regexes.
pub fn normalize(html: &str) -> String {
    let mut s = html
        .replace("\\/", "/")
        .replace("\\u0026", "&")
        .replace("\\u002F", "/")
        .replace("\\u002f", "/")
        .replace("\\u003A", ":")
        .replace("\\u003a", ":")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#039;", "'")
        .replace("&#39;", "'")
        .replace("&#x2F;", "/")
        .replace("&#47;", "/");
    s = s.replace("&amp;", "&");
    s
}

pub fn is_media_url(u: &str) -> Option<bool> {
    let p = u.to_lowercase();
    let p = p.split(['?', '#']).next().unwrap_or("");
    if p.ends_with(".m3u8") || (p.contains("/m3u8/") && p.ends_with(".txt")) {
        Some(true)
    } else if p.ends_with(".mp4") || p.ends_with(".mkv") || p.ends_with(".webm") {
        Some(false)
    } else {
        None
    }
}

pub fn height_of(s: &str) -> Option<u32> {
    static H: OnceLock<Regex> = OnceLock::new();
    re(&H, r"(?i)(?:^|[^0-9])(2160|1440|1080|720|576|480|360|240)(?:p|[^0-9]|$)")
        .captures(s)
        .and_then(|c| c[1].parse().ok())
}

fn asset(u: &str) -> bool {
    let low = u.to_lowercase();
    let path = low.split(['?', '#']).next().unwrap_or("");
    [".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".css", ".js", ".woff", ".woff2", ".ttf", ".vtt", ".srt", ".json", ".xml"]
        .iter()
        .any(|e| path.ends_with(e))
        || [
            "googletagmanager", "google-analytics", "gstatic.com", "googleapis.com", "facebook.", "histats", "doubleclick",
            "/cdn-cgi/", "twitter.com", "line.me", "cloudflareinsights", "jwpcdn.com", "jsdelivr.net", "cdnjs.", "w3.org",
            "schema.org", "youtube.com/embed", "disqus", "player24k.com/api/ads", "displayvertising",
        ]
        .iter()
        .any(|j| low.contains(j))
}

/// Decoded strings hidden in the page: `atob("…")`, reversed base64 bootstraps, percent-encoded JSON.
fn hidden_strings(txt: &str) -> Vec<String> {
    static ATOB: OnceLock<Regex> = OnceLock::new();
    static LIT: OnceLock<Regex> = OnceLock::new();
    let mut out = Vec::new();
    let mut try_b64 = |s: &str| {
        for cand in [s.to_string(), s.chars().rev().collect::<String>()] {
            let c = cand.replace('-', "+").replace('_', "/");
            if let Some(b) = b64_decode(&c) {
                if let Ok(t) = String::from_utf8(b) {
                    let t = if t.contains('%') { percent_decode(&t) } else { t };
                    if t.contains("http") || t.contains("#EXTM3U") || t.contains("\"url\"") {
                        out.push(normalize(&t));
                    }
                }
            }
        }
    };
    for c in re(&ATOB, r#"atob\(\s*["']([A-Za-z0-9+/=_-]{16,})["']\s*\)"#).captures_iter(txt) {
        try_b64(&c[1]);
    }
    // long base64-looking string literals (bootstraps such as `const s="EdTJ…"`)
    for c in re(&LIT, r#"["']([A-Za-z0-9+/=_-]{120,})["']"#).captures_iter(txt).take(12) {
        try_b64(&c[1]);
    }
    out
}

/// URLs in player-ish fields (`file`, `src`, `url`, `manifestUrl` …), absolute or page-relative.
fn field_urls(txt: &str, page: &str) -> Vec<(String, Option<String>)> {
    static KV: OnceLock<Regex> = OnceLock::new();
    let kv = re(
        &KV,
        r#"(?i)["']?(file|src|source|url|manifestUrl|manifest|hls|hlsUrl|playlist|video_url|videoUrl|stream|streamUrl|embed_url|link|iframe|embed|player)["']?\s*[:=]\s*["']([^"'\s<>]{6,})["']"#,
    );
    let mut out = Vec::new();
    for c in kv.captures_iter(txt) {
        let raw = &c[2];
        if raw.starts_with("data:") || raw.starts_with("javascript") || raw.starts_with('#') || raw.starts_with("blob:") {
            continue;
        }
        let looks_url = raw.starts_with("http") || raw.starts_with("//") || raw.starts_with('/') || raw.starts_with("./");
        if !looks_url {
            continue;
        }
        if let Some(u) = abs(page, raw) {
            if !asset(&u) && !out.iter().any(|(x, _): &(String, Option<String>)| *x == u) {
                out.push((u, Some(c[1].to_lowercase())));
            }
        }
    }
    out
}

/// JW Player style `{file: "…", label: "720p"}` pairs.
fn jw_sources(txt: &str, page: &str) -> Vec<MediaUrl> {
    static JW: OnceLock<Regex> = OnceLock::new();
    let r = re(&JW, r#"(?is)["']?file["']?\s*:\s*["']([^"']+)["']\s*,?\s*(?:[^{}]{0,200}?["']?label["']?\s*:\s*["']([^"']*)["'])?"#);
    let mut out: Vec<MediaUrl> = Vec::new();
    for c in r.captures_iter(txt) {
        let Some(u) = abs(page, &c[1]) else { continue };
        if asset(&u) {
            continue;
        }
        let label = c.get(2).map(|m| m.as_str()).unwrap_or("");
        let kind = is_media_url(&u);
        out.push(MediaUrl { height: height_of(label).or_else(|| height_of(&u)), hls: kind.unwrap_or(false), url: u });
    }
    out
}

/// Script-driven loaders: `$.ajax({url: "/x"})`, `fetch("/x")`, `$.get("/x")`, `.load("/x")`.
fn ajax_urls(txt: &str, page: &str) -> Vec<String> {
    static AJ: OnceLock<Regex> = OnceLock::new();
    let r = re(&AJ, r#"(?is)(?:\$\.ajax\(\s*\{[^}]{0,200}?url\s*:|\$\.(?:get|post)\(|fetch\(|\.load\()\s*["']([^"']+)["']"#);
    r.captures_iter(txt).filter_map(|c| abs(page, &c[1])).filter(|u| !asset(u)).collect()
}

fn iframes(txt: &str, page: &str) -> Vec<String> {
    static IF: OnceLock<Regex> = OnceLock::new();
    let r = re(&IF, r#"(?i)<iframe[^>]+?\s(?:data-src|src)=["']([^"']+)["']"#);
    r.captures_iter(txt).filter_map(|c| abs(page, &c[1])).filter(|u| !u.starts_with("about:") && !asset(u)).collect()
}

/// What a URL serves, judged from its first bytes. Some(true) = HLS, Some(false) = video file.
pub fn probe(http: &Http, url: &str, referer: &str, headers: &[String], tr: &Trace) -> Option<bool> {
    match http.peek(url, referer, headers, 64 * 1024) {
        Ok((status, ctype, head)) => {
            let ct = ctype.to_lowercase();
            let verdict = if decode_playlist_head(&head) {
                Some(true)
            } else if ct.starts_with("video/") || ct.contains("mp4") || head.get(4..8) == Some(b"ftyp") || head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
                Some(false)
            } else {
                None
            };
            tr.log(format!("    probe {} -> {status} {ct} {}", truncate(url, 90), match verdict {
                Some(true) => "HLS",
                Some(false) => "video",
                None => "not media",
            }));
            if (200..300).contains(&status) { verdict } else { None }
        }
        Err(e) => {
            tr.log(format!("    probe {} failed: {e:#}", truncate(url, 90)));
            None
        }
    }
}

/// Plain / JSON-wrapped / base64 HLS playlist?
pub fn decode_playlist_head(body: &[u8]) -> bool {
    let t = String::from_utf8_lossy(body);
    let t = t.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("#EXTM3U") {
        return true;
    }
    if t.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(t.trim()) {
            if let Some(o) = v.as_object() {
                return o.values().filter_map(|x| x.as_str()).any(|s| decode_playlist_head(s.as_bytes()));
            }
        }
        return false;
    }
    let compact: String = t.chars().take(4096).filter(|c| !c.is_whitespace()).collect();
    // (binary answers — an image, a video segment — decode to non-ASCII text: not base64)
    compact.len() >= 12
        && compact.is_ascii()
        && b64_decode(&compact[..compact.len() / 4 * 4]).is_some_and(|b| String::from_utf8_lossy(&b).trim_start().starts_with("#EXTM3U"))
}

/// Walk an embed page down to the video. `fetch` loads a page (plain HTTP, or through the browser helper for
/// Cloudflare-protected pages).
pub fn find_media(http: &Http, url: &str, referer: &str, tr: &Trace) -> Option<Media> {
    let mut seen = HashSet::new();
    walk(http, url, referer, 5, &mut seen, false, tr)
}

fn ctx_for(page: &str) -> (String, Vec<String>) {
    let o = origin_of(page);
    let origin = o.trim_end_matches('/').to_string();
    (o, vec![format!("Origin: {origin}")])
}

/// FairyAnime's main player (`anime.tonytonychopper.net/v2/<id>`): the page script (several layers of packers)
/// only sets `hls = "…/file2/<id>/"`; the playlists there answer only requests that look like the player's own
/// XHR (Referer = the player page, Origin, `Sec-Fetch-*` of a same-origin fetch) and say `null` otherwise. The
/// segments (TS, named `.webp`) are on another host and open without any of that.
fn tonytonychopper(http: &Http, url: &str, tr: &Trace) -> Option<Media> {
    let rest = url.split_once("://anime.tonytonychopper.net/v2/")?.1;
    let id: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    if id.is_empty() {
        return None;
    }
    let page = format!("https://anime.tonytonychopper.net/v2/{id}");
    let headers: Vec<String> = ["Origin: https://anime.tonytonychopper.net", "Sec-Fetch-Site: same-origin", "Sec-Fetch-Mode: cors", "Sec-Fetch-Dest: empty"].iter().map(|s| s.to_string()).collect();
    let master = format!("https://anime.tonytonychopper.net/file2/{id}/");
    let body = http.peek(&master, &page, &headers, 8192).ok().map(|(_, _, b)| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    if !body.trim_start().starts_with("#EXTM3U") {
        tr.log(format!("    tonytonychopper {id}: no playlist ({})", truncate(body.trim(), 40)));
        return None;
    }
    // some videos list a quality whose playlist is empty (the site then falls back to its backup player)
    let variant = body.lines().map(str::trim).find(|l| l.starts_with("http"));
    if let Some(v) = variant {
        let ok = http.peek(v, &page, &headers, 2048).ok().is_some_and(|(_, _, b)| String::from_utf8_lossy(&b).trim_start().starts_with("#EXTM3U"));
        if !ok {
            tr.log(format!("    tonytonychopper {id}: the video playlist is empty"));
            return None;
        }
    }
    let height = body.lines().find_map(|l| l.split("RESOLUTION=").nth(1)).and_then(|r| r.split(['x', ',']).nth(1)).and_then(|h| h.trim().parse().ok());
    tr.log(format!("    tonytonychopper {id}: HLS {height:?}"));
    Some(Media { urls: vec![MediaUrl { url: master, height, hls: true }], page, headers })
}

fn walk(http: &Http, url: &str, referer: &str, depth: u8, seen: &mut HashSet<String>, xhr: bool, tr: &Trace) -> Option<Media> {
    if let Some(hls) = is_media_url(url) {
        let (_, headers) = ctx_for(referer);
        return Some(Media { urls: vec![MediaUrl { url: url.into(), height: height_of(url), hls }], page: referer.into(), headers });
    }
    if depth == 0 || ytdlp_host(url) || !seen.insert(url.split('#').next().unwrap_or(url).to_string()) {
        return None;
    }
    // streaming.aki.today: `playback/f/<id>/` only builds its iframe with script, the page it points at is `playback/v/<id>/`
    if let Some((host, id)) = url.split_once("/playback/f/").map(|(h, r)| (h, r.trim_end_matches('/')))
        && host.ends_with("streaming.aki.today")
        && !id.is_empty()
    {
        return walk(http, &format!("{host}/playback/v/{id}/"), referer, depth, seen, xhr, tr);
    }
    tr.log(format!("  open {}", truncate(url, 140)));
    if let Some((local, height)) = crate::gated::resolve(url, referer, &|s| tr.log(s)) {
        return Some(Media { urls: vec![MediaUrl { url: local, height, hls: true }], page: url.into(), headers: vec![] });
    }
    if let Some(m) = tonytonychopper(http, url, tr) {
        return Some(m);
    }
    // `$.ajax` loaders answer only to XHR requests (anifume returns an empty body otherwise)
    let fetched = if xhr {
        http.get_with(url, referer, &["X-Requested-With: XMLHttpRequest".to_string()])
    } else {
        http.get(url, Some(referer))
    };
    let html = match fetched {
        Ok(h) if h.trim().is_empty() && !xhr => {
            tr.log("    empty page");
            return None;
        }
        Ok(h) => h,
        Err(e) => {
            tr.log(format!("    failed: {e:#}"));
            return None;
        }
    };
    let mut txt = normalize(&html);
    for h in hidden_strings(&txt) {
        txt.push('\n');
        txt.push_str(&h);
    }
    let (page_ref, headers) = ctx_for(url);

    // 1. JW / sources lists and plain media links
    let mut urls: Vec<MediaUrl> = Vec::new();
    let add = |m: MediaUrl, urls: &mut Vec<MediaUrl>| {
        if !urls.iter().any(|x| x.url == m.url) {
            urls.push(m);
        }
    };
    for m in jw_sources(&txt, url) {
        if is_media_url(&m.url).is_some() {
            add(m, &mut urls);
        }
    }
    // vdohide players (playhdhls …): `PLAYER_CONFIG = {"vdoId": "…", "node": {"playlist": "vdohls.com"}}`
    static VDO: OnceLock<Regex> = OnceLock::new();
    static NODE: OnceLock<Regex> = OnceLock::new();
    if let Some(id) = re(&VDO, r#""vdoId"\s*:\s*"([A-Za-z0-9_-]+)""#).captures(&txt).map(|c| c[1].to_string()) {
        let node = re(&NODE, r#""playlist"\s*:\s*"([A-Za-z0-9.-]+)""#).captures(&txt).map(|c| c[1].to_string()).unwrap_or_else(|| "vdohls.com".into());
        add(MediaUrl { url: format!("https://{node}/{id}/playlist.m3u8"), height: None, hls: true }, &mut urls);
    }
    static MEDIA: OnceLock<Regex> = OnceLock::new();
    for m in re(&MEDIA, r#"https?://[^"'\s<>\\]+?(?:\.(?:m3u8|mp4)|/m3u8/[^"'\s<>\\?]+\.txt)(?:\?[^"'\s<>\\]*)?"#).find_iter(&txt) {
        let u = m.as_str().to_string();
        if !asset(&u) {
            add(MediaUrl { height: height_of(&u), hls: is_media_url(&u).unwrap_or(false), url: u }, &mut urls);
        }
    }
    // 2. player fields with unknown types (`/hls/<id>.txt`, `/o/<id>/a`, JW file without extension): probe them
    let fields = field_urls(&txt, url);
    let probe_list: Vec<(String, Option<u32>)> = fields
        .iter()
        .filter(|(u, k)| {
            is_media_url(u).is_none()
                && k.as_deref().is_some_and(|k| ["file", "url", "manifesturl", "manifest", "hls", "hlsurl", "playlist", "video_url", "videourl", "stream", "streamurl", "source"].contains(&k))
                && !crate::text::same_site(u, url) | u.contains("/hls/") | u.contains("manifest") | u.contains("/m3u8")
        })
        .map(|(u, _)| (u.clone(), height_of(u)))
        .take(6)
        .collect();
    for (u, h) in probe_list {
        if urls.len() >= 6 {
            break;
        }
        if let Some(hls) = probe(http, &u, &page_ref, &headers, tr) {
            add(MediaUrl { url: u, height: h, hls }, &mut urls);
        }
    }
    if !urls.is_empty() {
        tr.log(format!("    found {} video url(s) on {}", urls.len(), truncate(url, 80)));
        urls.sort_by(|a, b| b.height.cmp(&a.height));
        return Some(Media { urls, page: page_ref, headers });
    }

    // 3. go deeper: iframes, script loaders, embed-ish fields
    let mut next: Vec<String> = Vec::new();
    let ajax = ajax_urls(&txt, url);
    for u in iframes(&txt, url).into_iter().chain(ajax.iter().cloned()).chain(
        fields
            .iter()
            .filter(|(_, k)| k.as_deref().is_some_and(|k| ["src", "embed_url", "embed", "iframe", "link", "player", "url"].contains(&k)))
            .map(|(u, _)| u.clone()),
    ) {
        if !next.contains(&u) && u != url {
            next.push(u);
        }
    }
    for u in next.into_iter().take(10) {
        let x = ajax.contains(&u);
        if let Some(m) = walk(http, &u, url, depth - 1, seen, x, tr) {
            return Some(m);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vdohide_config() {
        let t = r#"window.PLAYER_CONFIG = {"vdoId":"L-4cj_Mtt0j50","node":{"static":"static.vdohls.com","playlist":"vdohls.com"}};"#;
        let re = Regex::new(r#""vdoId"\s*:\s*"([A-Za-z0-9_-]+)""#).unwrap();
        assert_eq!(&re.captures(t).unwrap()[1], "L-4cj_Mtt0j50");
    }

    #[test]
    fn decodes_nya_bootstrap() {
        let s = "EdTJlNHbhZWQzUiMyUSZk9WT0lGZ1FmMyUyQyUCbsVnbBNTJyITJwJDcyITJDJTJwATMBNTJyITJ05WZjJXZQN3clJ3ZvJHcyITJDJTJyITJ0RnducXZpZXZyBnRyUCdGJTJtEVdGVXUlRHUaZEZClkRyUybGJTJt92Yukmbh5WZtlmbh5CdhNmRyUiRyUSQzUycwRHdoJjMlE0MlIjMlwmcVRHdWdXZpZXZyBnMyUyQyUiMyUCcGJTJtEVdGVXUlRHUaZEZClkRyUybGJTJt92Yukmbh5WZtlmbh5CdhNmRyUiRyUSQzUycwRHdoJjMlE0MlIjMlwmcVJXZ0N3bwJjMlMkMlIjMlEmRyUSLRVnR1FVZ0BlWGRmQJZkMl8mRyUSbvNmLp5WYuVWbp5WYuQXYjZkMlYkMlE0MlMHc0RHayITJBNTJyITJsJXV0NXZmlmbh1mMyUyQyUiMyUSekFWZyJjMlE0MlIjMlMXd0FGdzJjMlMkMlIjMlgFWOdUa2R0NDlVZYNVOfZ3btJjMlE0MlIjMlQWSj9GZyITJCdTJ";
        let page = format!(r#"<script>(()=>{{const s="{s}";}})();</script>"#);
        let hidden = hidden_strings(&page).join("\n");
        assert!(hidden.contains("https://cat.animenani.com/o/IBdFZPteQuFuQ-/a"), "{hidden}");
        let f = field_urls(&hidden, "https://nya.animenani.com/embed/x");
        assert!(f.iter().any(|(u, k)| u.ends_with("/o/IBdFZPteQuFuQ-/a") && k.as_deref() == Some("manifesturl")));
    }

    #[test]
    fn jw_and_inertia() {
        let jw = r#"jwplayer("vpl").setup({"playlist": [{ "sources": [{ "file": "https://a16.x.com/f/1/A-01.mp4?m=1&e=2", "label": "720p" }, { "file": "https://a16.x.com/f/1/A-01-q360.mp4?m=3", "label": "360p", "default": "true" }] }]});"#;
        let v = jw_sources(jw, "https://anifume.com/player/x");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].height, Some(720));
        assert_eq!(v[1].height, Some(360));
        let inertia = r#"<div id="app" data-page="{&quot;video&quot;:{&quot;url&quot;:&quot;https:\/\/cdn2.m.com\/hls\/abc==.txt&quot;,&quot;type&quot;:&quot;direct&quot;}}"></div>"#;
        let n = normalize(inertia);
        let f = field_urls(&n, "https://animemami.xyz/v/1");
        assert_eq!(f[0].0, "https://cdn2.m.com/hls/abc==.txt");
    }

    #[test]
    fn ajax_loader() {
        let page = r##"<script>function mainv(){ $.ajax({ url: "/k49dF65?u=hj%2Fx", success: function(r){ $("#vpfi").html(r); } }); }</script>"##;
        assert_eq!(ajax_urls(page, "https://anifume.com/1/2"), vec!["https://anifume.com/k49dF65?u=hj%2Fx".to_string()]);
    }

    #[test]
    fn playlist_heads() {
        assert!(decode_playlist_head(b"#EXTM3U\n"));
        assert!(decode_playlist_head(br#"{"p":"I0VYVE0zVQojRVhULVgtVkVSU0lPTjozCg=="}"#));
        assert!(!decode_playlist_head(b"<html>"));
    }
}
