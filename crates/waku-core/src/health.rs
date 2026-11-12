//! Memory of which video hosts actually play, so the next episode tries working servers first.
//!
//! Every link the player opens (or fails to open) is recorded per host; streams are then ordered by
//! rank (direct > yt-dlp > unknown) and, within a rank, by how reliable their host has been lately.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Host {
    pub ok: u32,
    pub fail: u32,
    pub last_ok: u64,
    pub last_fail: u64,
}

static HOSTS: Mutex<Option<HashMap<String, Host>>> = Mutex::new(None);

fn host_of(url: &str) -> String {
    let h = url.split("://").nth(1).unwrap_or(url).split(['/', '?', '#', ':']).next().unwrap_or("").to_lowercase();
    // keep the registrable part: a1.rukoluo.com and a17.rukoluo.com are the same service
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() > 2 { parts[parts.len() - 2..].join(".") } else { h }
}

fn with<R>(f: impl FnOnce(&mut HashMap<String, Host>) -> R) -> R {
    let mut g = HOSTS.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(|| crate::store::load_json("hosts.json"));
    f(m)
}

/// Record that a link of this host opened (`ok`) or failed.
pub fn record(url: &str, ok: bool) {
    let now = crate::store::now();
    let snapshot = with(|m| {
        let h = m.entry(host_of(url)).or_default();
        if ok {
            h.ok = h.ok.saturating_add(1);
            h.last_ok = now;
        } else {
            h.fail = h.fail.saturating_add(1);
            h.last_fail = now;
        }
        m.clone()
    });
    crate::store::save_json("hosts.json", &snapshot);
}

/// Reliability 0..1 (0.5 = unknown). Recent results weigh more than old ones.
pub fn score(url: &str) -> f32 {
    let now = crate::store::now();
    with(|m| match m.get(&host_of(url)) {
        None => 0.5,
        Some(h) => {
            let mut s = (h.ok as f32 + 1.0) / (h.ok as f32 + h.fail as f32 + 2.0);
            // failed within the last 10 minutes and not working since: push it back
            if h.last_fail > h.last_ok && now.saturating_sub(h.last_fail) < 600 {
                s *= 0.5;
            }
            s
        }
    })
}

/// Record that a whole source played an episode (`ok`) or had no working server for it.
pub fn record_source(id: &str, ok: bool) {
    record(&format!("source://{id}"), ok);
}

/// Reliability 0..1 of a source as a whole (0.5 = unknown).
pub fn source_score(id: &str) -> f32 {
    score(&format!("source://{id}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn hosts_group_subdomains() {
        assert_eq!(super::host_of("https://a17.rukoluo.com/f/1/x.mp4"), "rukoluo.com");
        assert_eq!(super::host_of("https://vdohls.com/x/playlist.m3u8"), "vdohls.com");
    }
}
