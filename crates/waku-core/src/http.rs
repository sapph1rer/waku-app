//! Shared blocking HTTP client (browser-like headers, cookie jar).

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue, REFERER, USER_AGENT};
use std::time::Duration;

pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/130.0 Safari/537.36";

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
        let t = std::time::Instant::now();
        self.get(url, None)?;
        Ok(t.elapsed().as_millis())
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
