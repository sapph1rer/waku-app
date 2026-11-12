//! How each source has been doing: timings and failures of every list / search / detail / resolve / play,
//! kept across restarts (`<cache>/waku/srcstats.json`, the last 60 tries per operation).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{LazyLock, Mutex};

pub const OPS: [&str; 6] = ["ping", "list", "search", "detail", "resolve", "play"];
const KEEP: usize = 60;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Sample {
    /// unix seconds
    pub at: u64,
    pub ms: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<String>,
}

#[derive(Default, Clone, Debug)]
pub struct OpSummary {
    pub tries: usize,
    pub ok: usize,
    /// median of the successful tries
    pub median_ms: Option<u64>,
    pub last: Option<Sample>,
    pub last_err: Option<Sample>,
}

impl OpSummary {
    pub fn rate(&self) -> Option<f32> {
        (self.tries > 0).then(|| self.ok as f32 / self.tries as f32)
    }
}

type Store = BTreeMap<String, BTreeMap<String, VecDeque<Sample>>>;

static DATA: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(load()));

fn path() -> std::path::PathBuf {
    crate::profile::cache_dir().join("srcstats.json")
}

fn load() -> Store {
    std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Note one try of `op` on `source`.
pub fn record(source: &str, op: &str, ms: u128, result: Result<(), String>) {
    if source.is_empty() || source == "local" {
        return;
    }
    let s = Sample { at: now(), ms: ms as u64, ok: result.is_ok(), err: result.err().map(|e| e.lines().next().unwrap_or("").chars().take(200).collect()) };
    let snapshot = {
        let mut d = DATA.lock().unwrap();
        let q = d.entry(source.to_string()).or_default().entry(op.to_string()).or_default();
        q.push_back(s);
        while q.len() > KEEP {
            q.pop_front();
        }
        serde_json::to_string(&*d).ok()
    };
    if let Some(t) = snapshot {
        // written off the calling thread: the UI never waits on the disk
        std::thread::spawn(move || {
            let p = path();
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, t).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        });
    }
}

/// Time `f` and record it (ok = `Ok`).
pub fn timed<T, E: std::fmt::Display>(source: &str, op: &str, f: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let t = std::time::Instant::now();
    let r = f();
    record(source, op, t.elapsed().as_millis(), r.as_ref().map(|_| ()).map_err(|e| format!("{e:#}").replace("\n", " ")));
    r
}

pub fn summary(source: &str, op: &str) -> OpSummary {
    let d = DATA.lock().unwrap();
    let Some(q) = d.get(source).and_then(|m| m.get(op)) else { return OpSummary::default() };
    summarize(q)
}

pub fn summarize(q: &VecDeque<Sample>) -> OpSummary {
    let mut good: Vec<u64> = q.iter().filter(|s| s.ok).map(|s| s.ms).collect();
    good.sort_unstable();
    OpSummary {
        tries: q.len(),
        ok: good.len(),
        median_ms: good.get(good.len() / 2).copied(),
        last: q.back().cloned(),
        last_err: q.iter().rev().find(|s| !s.ok).cloned(),
    }
}

/// Recent tries of one operation (oldest first), for the little bar chart.
pub fn samples(source: &str, op: &str) -> Vec<Sample> {
    DATA.lock().unwrap().get(source).and_then(|m| m.get(op)).map(|q| q.iter().cloned().collect()).unwrap_or_default()
}

pub fn clear() {
    DATA.lock().unwrap().clear();
    let _ = std::fs::remove_file(path());
}

#[cfg(test)]
mod tests {
    #[test]
    fn summary_counts() {
        let q: std::collections::VecDeque<super::Sample> = [(100, true), (300, true), (50, false), (200, true)]
            .into_iter()
            .map(|(ms, ok)| super::Sample { at: 0, ms, ok, err: (!ok).then(|| "x".into()) })
            .collect();
        let s = super::summarize(&q);
        assert_eq!((s.tries, s.ok, s.median_ms), (4, 3, Some(200)));
        assert_eq!(s.last_err.as_ref().unwrap().ms, 50);
        assert!((s.rate().unwrap() - 0.75).abs() < 1e-6);
    }
}
