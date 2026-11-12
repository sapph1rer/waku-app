//! Shared blocking HTTP client (browser-like headers, cookie jar).

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue, REFERER, USER_AGENT};
use std::time::Duration;

pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/130.0 Safari/537.36";

/// What a real browser saw while playing a page: media requests it made.
#[derive(Debug, Clone, Default)]
pub struct Sniffed {
    /// media URLs (playlists / video files) in the order they were requested
    pub urls: Vec<String>,
    /// the frame URL that requested them (use as Referer)
    pub page: String,
    /// extra headers the stream needs ("Origin: …")
    pub headers: Vec<String>,
    /// what the browser saw when it found nothing (title, frames, errors), for the logs
    pub note: String,
}

/// A real browser engine (WebView2 helper in the app) used where plain HTTP is not enough:
/// Cloudflare "Just a moment…" challenges and players that build their URL in obfuscated JS.
pub trait Browser: Send + Sync {
    /// Page HTML after scripts/challenges ran.
    fn fetch(&self, url: &str, referer: Option<&str>) -> Result<String>;
    /// Open `url` (with `referer`) and report the media requests it makes.
    fn sniff(&self, url: &str, referer: &str) -> Result<Sniffed>;
    /// The browser's cookies for `url` ("a=b; c=d") and its User-Agent: after it passed a Cloudflare check,
    /// plain requests with these get through too (the clearance cookie is tied to the User-Agent).
    fn cookies(&self, _url: &str) -> Result<(String, String)> {
        bail!("not supported")
    }
}

/// Cloudflare clearance taken over from the browser helper, per host: (Cookie header, User-Agent).
static CF_PASS: std::sync::Mutex<Vec<(String, String, String)>> = std::sync::Mutex::new(Vec::new());

/// Timing notes for slow requests (`WAKU_HTTP_LOG=<file>`).
pub fn note(msg: &str) {
    if let Some(p) = std::env::var_os("WAKU_HTTP_LOG") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 1_000_000).unwrap_or(0);
            let _ = writeln!(f, "{t:>6} {msg}");
        }
    }
}

fn cf_pass(url: &str) -> Option<(String, String)> {
    load_cf_disk();
    let h = host_of(url);
    CF_PASS.lock().ok()?.iter().find(|x| x.0 == h).map(|x| (x.1.clone(), x.2.clone()))
}

/// Cloudflare passes survive a restart: `<cache>/cf_pass.json` holds each host's clearance cookie + the
/// browser's User-Agent (the cookie is bound to both), so the first pages after starting the app do not wait
/// for the browser helper. Passes older than a day are dropped; a refused one falls back to the browser.
fn cf_disk_path() -> std::path::PathBuf {
    crate::profile::cache_dir().join("cf_pass.json")
}

fn load_cf_disk() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Ok(t) = std::fs::read_to_string(cf_disk_path()) else { return };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { return };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut n = 0;
        for e in v.as_array().into_iter().flatten() {
            let s = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let at = e.get("at").and_then(|x| x.as_u64()).unwrap_or(0);
            let (h, c, ua) = (s("host"), s("cookie"), s("ua"));
            if h.is_empty() || c.is_empty() || ua.is_empty() || now.saturating_sub(at) > 86_400 {
                continue;
            }
            if let Ok(mut hs) = CF_HOSTS.lock()
                && !hs.contains(&h)
            {
                hs.push(h.clone());
            }
            if let Ok(mut p) = CF_PASS.lock()
                && !p.iter().any(|x| x.0 == h)
            {
                p.push((h, c, ua));
                n += 1;
            }
        }
        note(&format!("cf passes from disk: {n}"));
    });
}

fn save_cf_disk() {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let Ok(p) = CF_PASS.lock() else { return };
    let v: Vec<serde_json::Value> = p.iter().map(|(h, c, ua)| serde_json::json!({"host": h, "cookie": c, "ua": ua, "at": now})).collect();
    drop(p);
    let path = cf_disk_path();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, serde_json::to_string(&v).unwrap_or_default());
}

/// Forget a host's pass (Cloudflare stopped honouring it).
fn drop_cf_pass(url: &str) {
    let h = host_of(url);
    if let Ok(mut v) = CF_PASS.lock() {
        v.retain(|x| x.0 != h);
    }
    save_cf_disk();
}

fn learn_cf_pass(url: &str) {
    let Some(b) = browser() else { return };
    if let Ok((c, ua)) = b.cookies(url)
        && c.contains("cf_clearance")
        && !ua.is_empty()
        && let Ok(mut v) = CF_PASS.lock()
    {
        let h = host_of(url);
        v.retain(|x| x.0 != h);
        v.push((h, c, ua));
        drop(v);
        save_cf_disk();
    }
}

static BROWSER: std::sync::OnceLock<Box<dyn Browser>> = std::sync::OnceLock::new();
static CF_HOSTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Register the browser helper (once, at startup).
pub fn set_browser(b: Box<dyn Browser>) {
    let _ = BROWSER.set(b);
}

pub fn browser() -> Option<&'static dyn Browser> {
    BROWSER.get().map(|b| b.as_ref())
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("").to_lowercase()
}

/// Was this host seen behind a Cloudflare challenge?
pub fn is_cf_host(url: &str) -> bool {
    load_cf_disk();
    let h = host_of(url);
    CF_HOSTS.lock().map(|v| v.contains(&h)).unwrap_or(false)
}

#[derive(Clone)]
pub struct Http {
    c: Client,
}

impl Http {
    pub fn new(timeout_secs: u64) -> Result<Self> {
        let mut h = HeaderMap::new();
        h.insert(USER_AGENT, HeaderValue::from_static(UA));
        h.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("th,en;q=0.8"));
        h.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8",
            ),
        );
        let c = Client::builder()
            .default_headers(h)
            .cookie_store(true)
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self { c })
    }

    pub fn get(&self, url: &str, referer: Option<&str>) -> Result<String> {
        if is_cf_host(url)
            && let Some(b) = browser()
        {
            // fast path: plain HTTP with the browser's clearance cookie (a page through the browser takes seconds)
            let t0 = std::time::Instant::now();
            static FAST_OFF_UNTIL: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
            let fast_off = FAST_OFF_UNTIL.lock().ok().and_then(|g| *g).is_some_and(|t| std::time::Instant::now() < t);
            if let Some((cookie, ua)) = cf_pass(url).filter(|_| !fast_off) {
                // short leash: when Cloudflare stops honouring the cookie it tends to stall the connection rather
                // than refuse it; after a miss the fast path rests for 10 minutes
                let off = || {
                    if let Ok(mut g) = FAST_OFF_UNTIL.lock() {
                        *g = Some(std::time::Instant::now() + Duration::from_secs(600));
                    }
                };
                let mut req = self.c.get(url).header("Cookie", cookie).header(USER_AGENT, ua).timeout(Duration::from_secs(6));
                if let Some(r) = referer {
                    req = req.header(REFERER, r);
                }
                match req.send() {
                    Ok(resp) if resp.status().is_success() => {
                        if let Ok(t) = resp.text() {
                            note(&format!("fast {} ms {url}", t0.elapsed().as_millis()));
                            return Ok(t);
                        }
                    }
                    Ok(resp) if matches!(resp.status().as_u16(), 404 | 410) => {
                        // the page does not exist: not Cloudflare's doing, the pass is still good
                        note(&format!("fast {} ms: HTTP {} {url}", t0.elapsed().as_millis(), resp.status()));
                        anyhow::bail!("HTTP {} for {url}", resp.status());
                    }
                    Ok(resp) => {
                        off();
                        drop_cf_pass(url);
                        note(&format!("fast path refused: HTTP {} {url}", resp.status()))
                    }
                    Err(e) => {
                        off();
                        note(&format!("fast path error {e} {url}"))
                    }
                }
            } else {
                note(&format!("no clearance cookie yet for {url}"));
            }
            let t1 = std::time::Instant::now();
            let r = b.fetch(url, referer);
            note(&format!("browser fetch {} ms ok={} {url}", t1.elapsed().as_millis(), r.is_ok()));
            if r.is_ok() {
                let t2 = std::time::Instant::now();
                learn_cf_pass(url);
                note(&format!("cookies {} ms, have={}", t2.elapsed().as_millis(), cf_pass(url).is_some()));
            }
            return r;
        }
        match self.get_plain(url, referer) {
            Err(e) if format!("{e}").contains("Cloudflare challenge") => {
                if let Ok(mut v) = CF_HOSTS.lock() {
                    let h = host_of(url);
                    if !v.contains(&h) {
                        v.push(h);
                    }
                }
                match browser() {
                    Some(b) => {
                        note(&format!("challenge -> browser {url}"));
                        let t1 = std::time::Instant::now();
                        let r = b.fetch(url, referer).with_context(|| format!("{e:#}; browser fallback failed"));
                        note(&format!("browser fetch {} ms ok={}", t1.elapsed().as_millis(), r.is_ok()));
                        if r.is_ok() {
                            learn_cf_pass(url);
                        }
                        r
                    }
                    None => Err(e),
                }
            }
            r => r,
        }
    }

    /// GET with extra headers ("Name: value"), e.g. `X-Requested-With: XMLHttpRequest` for script loaders.
    pub fn get_with(&self, url: &str, referer: &str, headers: &[String]) -> Result<String> {
        if headers.is_empty() {
            return self.get(url, Some(referer));
        }
        let mut req = self.c.get(url).header(REFERER, referer);
        for h in headers {
            if let Some((k, v)) = h.split_once(':') {
                req = req.header(k.trim(), v.trim());
            }
        }
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        let st = resp.status();
        if !st.is_success() {
            bail!("GET {url} -> HTTP {st}");
        }
        Ok(resp.text()?)
    }

    /// Plain HTTP GET without the browser fallback.
    pub fn get_plain(&self, url: &str, referer: Option<&str>) -> Result<String> {
        let mut req = self.c.get(url);
        if let Some(r) = referer {
            req = req.header(REFERER, r);
        }
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        let st = resp.status();
        if !st.is_success() {
            let cf =
                resp.headers().get("server").and_then(|v| v.to_str().ok()) == Some("cloudflare");
            if cf && (st.as_u16() == 403 || st.as_u16() == 503) {
                bail!("GET {url} -> HTTP {st} (Cloudflare challenge)");
            }
            bail!("GET {url} -> HTTP {st}");
        }
        Ok(resp.text()?)
    }

    pub fn get_bytes(&self, url: &str, referer: Option<&str>) -> Result<Vec<u8>> {
        let mut req = self.c.get(url);
        if let Some(r) = referer {
            req = req.header(REFERER, r);
        }
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            bail!("GET {url} -> HTTP {}", resp.status());
        }
        Ok(resp.bytes()?.to_vec())
    }

    /// First bytes of a URL (status, content-type, up to `max` bytes) — used to tell what a link serves.
    pub fn peek(&self, url: &str, referer: &str, headers: &[String], max: usize) -> Result<(u16, String, Vec<u8>)> {
        use std::io::Read;
        let mut req = self.c.get(url).header(REFERER, referer).header(ACCEPT, "*/*");
        for h in headers {
            if let Some((k, v)) = h.split_once(':') {
                req = req.header(k.trim(), v.trim());
            }
        }
        let resp = req.send().with_context(|| format!("GET {url}"))?;
        let st = resp.status().as_u16();
        let ct = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let mut buf = Vec::new();
        resp.take(max as u64).read_to_end(&mut buf)?;
        Ok((st, ct, buf))
    }

    /// POST form (used by old DooPlay `admin-ajax.php`). Returns (status, body).
    pub fn post_form(
        &self,
        url: &str,
        referer: &str,
        form: &[(&str, &str)],
    ) -> Result<(u16, String)> {
        let resp = self
            .c
            .post(url)
            .header(REFERER, referer)
            .header("X-Requested-With", "XMLHttpRequest")
            .form(form)
            .send()
            .with_context(|| format!("POST {url}"))?;
        let st = resp.status().as_u16();
        Ok((st, resp.text().unwrap_or_default()))
    }

    /// Time a GET of `url` (used for source health). Returns milliseconds.
    pub fn ping(&self, url: &str) -> Result<u128> {
        // plain HTTP only: a Cloudflare check answering means the site is up (going through the browser helper
        // here would hold its one-page-at-a-time queue for many seconds at start-up)
        let t = std::time::Instant::now();
        match self.get_plain(url, None) {
            Ok(_) => Ok(t.elapsed().as_millis()),
            Err(e) if format!("{e}").contains("Cloudflare challenge") => Ok(t.elapsed().as_millis()),
            Err(e) => Err(e),
        }
    }

    /// JSON API GET (metadata providers). 429 is retried after the server's Retry-After.
    pub fn get_json(&self, url: &str, headers: &[(&str, &str)]) -> Result<serde_json::Value> {
        self.json_call(url, |c| {
            let mut r = c.get(url);
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r
        })
    }

    /// JSON API POST (AniList GraphQL).
    pub fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        self.json_call(url, |c| {
            c.post(url).header("Accept", "application/json").json(body)
        })
    }

    /// POST raw bytes (an image …) and read JSON back.
    pub fn post_bytes_json(&self, url: &str, content_type: &str, body: Vec<u8>) -> Result<serde_json::Value> {
        self.json_call(url, |c| {
            c.post(url).header("Accept", "application/json").header("Content-Type", content_type).timeout(Duration::from_secs(60)).body(body.clone())
        })
    }

    fn json_call(
        &self,
        url: &str,
        build: impl Fn(&Client) -> reqwest::blocking::RequestBuilder,
    ) -> Result<serde_json::Value> {
        for attempt in 0..3u64 {
            let resp = build(&self.c)
                .send()
                .with_context(|| format!("request {url}"))?;
            let st = resp.status();
            if st.as_u16() == 429 {
                let wait = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .unwrap_or(4 + attempt * 4)
                    .min(30);
                std::thread::sleep(Duration::from_secs(wait));
                continue;
            }
            let text = resp.text()?;
            if !st.is_success() && !text.trim_start().starts_with('{') {
                bail!("{url} -> HTTP {st}");
            }
            return serde_json::from_str(&text).with_context(|| format!("{url}: bad JSON"));
        }
        bail!("{url}: rate limited (HTTP 429)")
    }
}

/// Run `f` over `items` on up to `width` threads at a time (order preserved).
pub fn par_map<T: Sync, R: Send>(items: &[T], width: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let mut out = Vec::with_capacity(items.len());
    for chunk in items.chunks(width.max(1)) {
        let res: Vec<Option<R>> = std::thread::scope(|s| {
            let f = &f;
            let hs: Vec<_> = chunk.iter().map(|it| s.spawn(move || f(it))).collect();
            hs.into_iter().map(|h| h.join().ok()).collect()
        });
        out.extend(res.into_iter().flatten());
    }
    out
}
