//! Episode downloads: kept in `Downloads\<series>\` next to the program (or the folder set in Settings).
//!
//! Each series folder has `series.json` (the card + the episodes on disk) and one file per episode:
//! `E05 Episode 5.ts` (HLS: segments joined, decrypted when AES-128, fake image headers cut off — the same
//! clean-up the player's HLS proxy does) or `.mp4` (plain files, fMP4 HLS). A download writes `<name>.part`
//! and renames it when complete, so a half-finished file never looks like an episode.
//!
//! Jobs run one at a time (an episode's segments 4 in parallel); the link is found exactly as for playing
//! (servers best first, the next one when a download fails).

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use waku_core::player::Candidate;
use waku_core::{AnimeCard, Episode, SourceManager, StreamRank};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SeriesFile {
    pub card: AnimeCard,
    pub episodes: Vec<DlEpisode>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DlEpisode {
    pub num: f32,
    pub label: String,
    #[serde(default)]
    pub dub: bool,
    /// file name inside the series folder
    pub file: String,
    pub bytes: u64,
    /// the episode page it came from
    pub source_url: String,
}

/// Where downloads go: the folder from Settings, else `Downloads` next to the program.
pub fn root(setting: &str) -> PathBuf {
    if !setting.trim().is_empty() {
        return PathBuf::from(setting.trim());
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("Downloads")))
        .unwrap_or_else(|| PathBuf::from("Downloads"))
}

/// A name Windows accepts for a file or folder.
pub fn safe_name(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { ' ' } else { c })
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim_end_matches(['.', ' ']).chars().take(90).collect::<String>();
    if t.is_empty() { "anime".into() } else { t }
}

pub fn series_dir(root: &Path, card: &AnimeCard) -> PathBuf {
    root.join(safe_name(&card.title))
}

/// Every downloaded series (folders with a `series.json`), A–Z.
pub fn load_all(root: &Path) -> Vec<(PathBuf, SeriesFile)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let dir = e.path();
        if let Some(mut s) = load_series(&dir) {
            // forget episodes whose file was deleted by hand
            s.episodes.retain(|ep| dir.join(&ep.file).is_file());
            if !s.episodes.is_empty() {
                s.episodes.sort_by(|a, b| a.dub.cmp(&b.dub).then(a.num.partial_cmp(&b.num).unwrap_or(std::cmp::Ordering::Equal)));
                out.push((dir, s));
            }
        }
    }
    out.sort_by_key(|(_, s)| s.card.title.to_lowercase());
    out
}

pub fn load_series(dir: &Path) -> Option<SeriesFile> {
    serde_json::from_str(&std::fs::read_to_string(dir.join("series.json")).ok()?).ok()
}

fn save_series(dir: &Path, s: &SeriesFile) -> Result<()> {
    std::fs::write(dir.join("series.json"), serde_json::to_string_pretty(s)?)?;
    Ok(())
}

/// Remove one episode (file + entry); the folder goes when it was the last one.
pub fn delete_episode(dir: &Path, file: &str) {
    let _ = std::fs::remove_file(dir.join(file));
    if let Some(mut s) = load_series(dir) {
        s.episodes.retain(|e| e.file != file);
        if s.episodes.is_empty() {
            let _ = std::fs::remove_file(dir.join("series.json"));
            let _ = std::fs::remove_dir(dir);
        } else {
            let _ = save_series(dir, &s);
        }
    }
}

fn ep_file_stem(ep: &Episode) -> String {
    let n = if ep.num.fract() == 0.0 { format!("E{:02}", ep.num as i64) } else { format!("E{}", ep.num) };
    let dub = if ep.dub { " (Dub)" } else { "" };
    let label = ep.label.trim();
    let label = if label.is_empty() || label.chars().all(|c| c.is_ascii_digit()) { String::new() } else { format!(" {label}") };
    safe_name(&format!("{n}{dub}{label}"))
}

// ------------------------------------------------------------------ jobs

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Queued,
    Resolving,
    Downloading,
    Done,
    Failed(String),
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct JobState {
    pub status: Status,
    /// pieces (segments) or bytes done / total; total 0 = unknown
    pub done: u64,
    pub total: u64,
    pub bytes: u64,
    pub server: String,
}

pub struct Job {
    pub id: u64,
    pub card: AnimeCard,
    pub ep: Episode,
    pub st: Mutex<JobState>,
    pub cancel: AtomicBool,
}

impl Job {
    pub fn state(&self) -> JobState {
        self.st.lock().unwrap().clone()
    }
    fn set(&self, f: impl FnOnce(&mut JobState)) {
        f(&mut self.st.lock().unwrap());
    }
    pub fn fraction(&self) -> f32 {
        let s = self.state();
        if s.total == 0 { 0.0 } else { (s.done as f32 / s.total as f32).clamp(0.0, 1.0) }
    }
}

#[derive(Default)]
pub struct Downloader {
    pub jobs: Mutex<Vec<Arc<Job>>>,
    running: AtomicBool,
    next_id: AtomicU64,
    /// bumped whenever a download finishes (the Downloads page reloads its list then)
    pub generation: AtomicU64,
}

impl Downloader {
    /// Queue an episode (ignored when it is already queued / downloading or on disk).
    pub fn enqueue(self: &Arc<Self>, mgr: Arc<SourceManager>, root: PathBuf, card: AnimeCard, ep: Episode, ctx: egui::Context) -> bool {
        {
            let jobs = self.jobs.lock().unwrap();
            if jobs.iter().any(|j| j.ep.url == ep.url && matches!(j.state().status, Status::Queued | Status::Resolving | Status::Downloading)) {
                return false;
            }
        }
        let dir = series_dir(&root, &card);
        if load_series(&dir).is_some_and(|s| s.episodes.iter().any(|e| e.source_url == ep.url && dir.join(&e.file).is_file())) {
            return false;
        }
        let job = Arc::new(Job {
            id: self.next_id.fetch_add(1, Ordering::Relaxed) + 1,
            card,
            ep,
            st: Mutex::new(JobState { status: Status::Queued, done: 0, total: 0, bytes: 0, server: String::new() }),
            cancel: AtomicBool::new(false),
        });
        self.jobs.lock().unwrap().push(job);
        if !self.running.swap(true, Ordering::SeqCst) {
            let me = self.clone();
            std::thread::Builder::new()
                .name("downloads".into())
                .spawn(move || me.worker(mgr, root, ctx))
                .ok();
        }
        true
    }

    /// Is this episode queued / downloading? (for the episode list)
    pub fn active_for(&self, ep_url: &str) -> Option<Arc<Job>> {
        self.jobs.lock().unwrap().iter().find(|j| j.ep.url == ep_url && matches!(j.state().status, Status::Queued | Status::Resolving | Status::Downloading)).cloned()
    }

    pub fn cancel(&self, id: u64) {
        if let Some(j) = self.jobs.lock().unwrap().iter().find(|j| j.id == id) {
            j.cancel.store(true, Ordering::Relaxed);
            if j.state().status == Status::Queued {
                j.set(|s| s.status = Status::Cancelled);
            }
        }
    }

    /// Drop finished / failed / cancelled jobs from the list.
    pub fn clear_finished(&self) {
        self.jobs.lock().unwrap().retain(|j| matches!(j.state().status, Status::Queued | Status::Resolving | Status::Downloading));
    }

    fn worker(self: Arc<Self>, mgr: Arc<SourceManager>, root: PathBuf, ctx: egui::Context) {
        loop {
            let next = self.jobs.lock().unwrap().iter().find(|j| j.state().status == Status::Queued).cloned();
            let Some(job) = next else {
                self.running.store(false, Ordering::SeqCst);
                // a job queued between the look and the flag: go on
                if self.jobs.lock().unwrap().iter().any(|j| j.state().status == Status::Queued) && !self.running.swap(true, Ordering::SeqCst) {
                    continue;
                }
                return;
            };
            let r = run_job(&mgr, &root, &job, &ctx);
            job.set(|s| {
                s.status = match &r {
                    Ok(()) => Status::Done,
                    Err(_) if job.cancel.load(Ordering::Relaxed) => Status::Cancelled,
                    Err(e) => Status::Failed(format!("{e:#}").lines().next().unwrap_or("").to_string()),
                }
            });
            waku_core::http::note(&format!("download {} {}: {:?}", job.card.title, job.ep.label, r.as_ref().err()));
            self.generation.fetch_add(1, Ordering::Relaxed);
            ctx.request_repaint();
        }
    }
}

fn run_job(mgr: &SourceManager, root: &Path, job: &Job, ctx: &egui::Context) -> Result<()> {
    job.set(|s| s.status = Status::Resolving);
    ctx.request_repaint();
    let (streams, _) = mgr.streams(&job.card.source, &job.ep.url).map_err(|e| anyhow!("{e:#}"))?;
    let dir = series_dir(root, &job.card);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let stem = ep_file_stem(&job.ep);
    let title = format!("{} — {}", job.card.title, job.ep.label);
    let mut last = anyhow!("no server gave a downloadable link");
    for mut st in streams {
        if job.cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if st.rank == StreamRank::Unknown {
            if waku_core::http::browser().is_none() {
                continue;
            }
            match mgr.deepen(&st) {
                Some(d) => st = d,
                None => continue,
            }
        }
        if st.rank != StreamRank::Direct {
            continue; // yt-dlp hosts: not downloaded here
        }
        let v = waku_core::stream::pick_variant(&st.variants, "best").clone();
        for c in waku_core::player::candidates(&st, &v, &title).into_iter().filter(|c| c.direct) {
            job.set(|s| {
                s.status = Status::Downloading;
                s.server = st.server.clone();
                s.done = 0;
                s.total = 0;
                s.bytes = 0;
            });
            ctx.request_repaint();
            match download(&c, &dir, &stem, job, ctx) {
                Ok((file, bytes)) => {
                    let mut s = load_series(&dir).unwrap_or(SeriesFile { card: job.card.clone(), episodes: vec![] });
                    s.card = job.card.clone();
                    s.episodes.retain(|e| !(e.source_url == job.ep.url || (e.num == job.ep.num && e.dub == job.ep.dub)) || e.file == file);
                    s.episodes.retain(|e| e.file != file);
                    s.episodes.push(DlEpisode { num: job.ep.num, label: job.ep.label.clone(), dub: job.ep.dub, file, bytes, source_url: job.ep.url.clone() });
                    save_series(&dir, &s)?;
                    return Ok(());
                }
                Err(e) if job.cancel.load(Ordering::Relaxed) => return Err(e),
                Err(e) => {
                    waku_core::http::note(&format!("download via {} failed: {e:#}", st.server));
                    last = e;
                }
            }
        }
    }
    Err(last)
}

// ------------------------------------------------------------------ HTTP

fn client() -> &'static reqwest::blocking::Client {
    static C: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(180))
            .gzip(true)
            .pool_max_idle_per_host(8)
            .build()
            .expect("http client")
    })
}

fn request(c: &Candidate, url: &str) -> reqwest::blocking::RequestBuilder {
    let mut r = client().get(url).header("User-Agent", &c.user_agent).header("Accept", "*/*");
    if !c.referrer.is_empty() {
        r = r.header("Referer", &c.referrer);
    }
    for h in &c.headers {
        if let Some((k, v)) = h.split_once(':')
            && !k.trim().eq_ignore_ascii_case("user-agent")
        {
            r = r.header(k.trim(), v.trim());
        }
    }
    r
}

fn get_bytes(c: &Candidate, url: &str) -> Result<(Vec<u8>, String)> {
    let mut last = anyhow!("request failed");
    for attempt in 0..3u64 {
        match request(c, url).send() {
            Ok(r) if r.status().is_success() => {
                let fin = r.url().to_string();
                return Ok((r.bytes()?.to_vec(), fin));
            }
            Ok(r) => last = anyhow!("HTTP {} for {url}", r.status()),
            Err(e) => last = anyhow!("{e}"),
        }
        std::thread::sleep(Duration::from_millis(400 * (attempt + 1)));
    }
    Err(last)
}

pub fn join(base: &str, rel: &str) -> String {
    let rel = rel.trim();
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return rel.to_string();
    }
    let (scheme, rest) = base.split_once("://").unwrap_or(("https", base));
    if let Some(r) = rel.strip_prefix("//") {
        return format!("{scheme}://{r}");
    }
    let host_end = rest.find('/').unwrap_or(rest.len());
    let host = &rest[..host_end];
    if rel.starts_with('/') {
        return format!("{scheme}://{host}{rel}");
    }
    let path = rest[host_end..].split(['?', '#']).next().unwrap_or("/");
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    format!("{scheme}://{host}{dir}{rel}")
}

fn attr(attrs: &str, key: &str) -> Option<String> {
    // KEY=value / KEY="value", comma separated outside quotes
    let mut rest = attrs;
    while !rest.is_empty() {
        let (k, after) = rest.split_once('=')?;
        let k = k.trim().trim_start_matches(',').trim();
        let (v, tail) = if let Some(q) = after.strip_prefix('"') {
            let end = q.find('"')?;
            (&q[..end], q[end + 1..].trim_start_matches(','))
        } else {
            let end = after.find(',').unwrap_or(after.len());
            (&after[..end], after[end..].trim_start_matches(','))
        };
        if k.eq_ignore_ascii_case(key) {
            return Some(v.to_string());
        }
        rest = tail;
    }
    None
}

// ------------------------------------------------------------------ download one link

/// Download `c` into `dir/stem.(ts|mp4)`; returns (file name, bytes).
fn download(c: &Candidate, dir: &Path, stem: &str, job: &Job, ctx: &egui::Context) -> Result<(String, u64)> {
    let lower = c.url.to_lowercase();
    let path = lower.split('?').next().unwrap_or("");
    if c.force_hls || path.ends_with(".m3u8") {
        let (body, fin) = get_bytes(c, &c.url)?;
        let pl = rmp_core::hlsproxy::decode_playlist(&body).ok_or_else(|| anyhow!("the link is not a playlist"))?;
        return download_hls(c, &pl, &fin, dir, stem, job, ctx);
    }
    // a plain file — or a playlist without the extension
    let mut resp = request(c, &c.url).send()?;
    if !resp.status().is_success() {
        bail!("HTTP {} for {}", resp.status(), c.url);
    }
    let fin = resp.url().to_string();
    let total = resp.content_length().unwrap_or(0);
    let mut head = vec![0u8; 0];
    let mut buf = vec![0u8; 256 * 1024];
    // look at the start: a playlist is small text
    while head.len() < 8192 {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    if total < 4 << 20 && head.iter().take(512).all(|b| *b != 0) {
        let mut all = head.clone();
        resp.read_to_end(&mut all)?;
        if let Some(pl) = rmp_core::hlsproxy::decode_playlist(&all) {
            return download_hls(c, &pl, &fin, dir, stem, job, ctx);
        }
        head = all;
    }
    let ext = if lower.contains(".mkv") { "mkv" } else if lower.contains(".webm") { "webm" } else { "mp4" };
    let name = format!("{stem}.{ext}");
    let part = dir.join(format!("{name}.part"));
    let mut f = std::fs::File::create(&part)?;
    f.write_all(&head)?;
    let mut done = head.len() as u64;
    job.set(|s| {
        s.total = total;
        s.done = done;
        s.bytes = done;
    });
    loop {
        if job.cancel.load(Ordering::Relaxed) {
            drop(f);
            let _ = std::fs::remove_file(&part);
            bail!("cancelled");
        }
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n])?;
        done += n as u64;
        job.set(|s| {
            s.done = done;
            s.bytes = done;
        });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    drop(f);
    if total > 0 && done < total {
        let _ = std::fs::remove_file(&part);
        bail!("the download stopped early ({done} of {total} bytes)");
    }
    std::fs::rename(&part, dir.join(&name))?;
    Ok((name, done))
}

struct Seg {
    url: String,
    /// AES-128 key URL + IV (None = from the sequence number)
    key: Option<(String, Option<[u8; 16]>)>,
    seq: u64,
}

fn parse_iv(s: &str) -> Option<[u8; 16]> {
    let h = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if h.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn download_hls(c: &Candidate, pl: &str, base: &str, dir: &Path, stem: &str, job: &Job, ctx: &egui::Context) -> Result<(String, u64)> {
    let (mut pl, mut base) = (pl.to_string(), base.to_string());
    // master playlist: the sharpest variant
    if pl.contains("#EXT-X-STREAM-INF") {
        let mut best: Option<(u64, String)> = None;
        let mut attrs: Option<String> = None;
        for line in pl.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some(a) = line.strip_prefix("#EXT-X-STREAM-INF:") {
                attrs = Some(a.to_string());
            } else if !line.starts_with('#')
                && let Some(a) = attrs.take()
            {
                let h = attr(&a, "RESOLUTION").and_then(|r| r.split(['x', 'X']).nth(1).and_then(|h| h.parse::<u64>().ok())).unwrap_or(0);
                let bw = attr(&a, "BANDWIDTH").and_then(|b| b.parse::<u64>().ok()).unwrap_or(0);
                let score = h * 1_000_000_000 + bw;
                if best.as_ref().is_none_or(|b| score > b.0) {
                    best = Some((score, join(&base, line)));
                }
            }
        }
        let (_, u) = best.ok_or_else(|| anyhow!("empty master playlist"))?;
        let (body, fin) = get_bytes(c, &u)?;
        pl = rmp_core::hlsproxy::decode_playlist(&body).ok_or_else(|| anyhow!("variant is not a playlist"))?;
        base = fin;
    }
    let mut segs: Vec<Seg> = Vec::new();
    let mut key: Option<(String, Option<[u8; 16]>)> = None;
    let mut init: Option<String> = None;
    let mut seq: u64 = 0;
    for line in pl.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            seq = v.trim().parse().unwrap_or(0);
        } else if let Some(a) = line.strip_prefix("#EXT-X-KEY:") {
            let method = attr(a, "METHOD").unwrap_or_default();
            key = match method.as_str() {
                "NONE" | "" => None,
                "AES-128" => Some((join(&base, &attr(a, "URI").ok_or_else(|| anyhow!("key without URI"))?), attr(a, "IV").and_then(|v| parse_iv(&v)))),
                m => bail!("encrypted stream ({m}) cannot be downloaded"),
            };
        } else if let Some(a) = line.strip_prefix("#EXT-X-MAP:") {
            init = attr(a, "URI").map(|u| join(&base, &u));
        } else if !line.starts_with('#') {
            segs.push(Seg { url: join(&base, line), key: key.clone(), seq });
            seq += 1;
        }
    }
    if segs.is_empty() {
        bail!("the playlist has no segments");
    }
    let ext = if init.is_some() { "mp4" } else { "ts" };
    let name = format!("{stem}.{ext}");
    let part = dir.join(format!("{name}.part"));
    let mut f = std::io::BufWriter::new(std::fs::File::create(&part)?);
    let mut written: u64 = 0;
    if let Some(u) = &init {
        let (b, _) = get_bytes(c, u)?;
        f.write_all(&b)?;
        written += b.len() as u64;
    }
    let total = segs.len();
    job.set(|s| {
        s.total = total as u64;
        s.done = 0;
    });

    // 4 downloaders, one writer (in order); at most 16 finished pieces wait in memory
    let segs = Arc::new(segs);
    let next = Arc::new(AtomicUsize::new(0));
    let ready: Arc<(Mutex<BTreeMap<usize, Result<Vec<u8>, String>>>, Condvar)> = Arc::new((Mutex::new(BTreeMap::new()), Condvar::new()));
    let written_upto = Arc::new(AtomicUsize::new(0));
    let keys: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let fmp4 = init.is_some();
    let mut workers = Vec::new();
    for _ in 0..4 {
        let (segs, next, ready, written_upto, keys, stop, c) = (segs.clone(), next.clone(), ready.clone(), written_upto.clone(), keys.clone(), stop.clone(), c.clone());
        workers.push(std::thread::spawn(move || {
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                // do not run too far ahead of the writer
                if next.load(Ordering::Relaxed) >= written_upto.load(Ordering::Relaxed) + 16 {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(s) = segs.get(i) else { return };
                let r = fetch_segment(&c, s, &keys, fmp4).map_err(|e| format!("piece {}: {e:#}", i + 1));
                let (m, cv) = &*ready;
                m.lock().unwrap().insert(i, r);
                cv.notify_all();
            }
        }));
    }
    let mut res: Result<()> = Ok(());
    for i in 0..total {
        let data = {
            let (m, cv) = &*ready;
            let mut g = m.lock().unwrap();
            loop {
                if let Some(d) = g.remove(&i) {
                    break Some(d);
                }
                if job.cancel.load(Ordering::Relaxed) {
                    break None;
                }
                g = cv.wait_timeout(g, Duration::from_millis(300)).unwrap().0;
            }
        };
        let Some(data) = data else {
            res = Err(anyhow!("cancelled"));
            break;
        };
        match data {
            Ok(d) => {
                if let Err(e) = f.write_all(&d) {
                    res = Err(e.into());
                    break;
                }
                written += d.len() as u64;
            }
            Err(e) => {
                res = Err(anyhow!(e));
                break;
            }
        }
        written_upto.store(i + 1, Ordering::Relaxed);
        job.set(|s| {
            s.done = i as u64 + 1;
            s.bytes = written;
        });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    stop.store(true, Ordering::Relaxed);
    for w in workers {
        let _ = w.join();
    }
    let flushed = f.flush();
    drop(f);
    if let Err(e) = res.and(flushed.map_err(Into::into)) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, dir.join(&name))?;
    Ok((name, written))
}

fn fetch_segment(c: &Candidate, s: &Seg, keys: &Mutex<HashMap<String, Vec<u8>>>, fmp4: bool) -> Result<Vec<u8>> {
    let (mut d, _) = get_bytes(c, &s.url)?;
    if let Some((ku, iv)) = &s.key {
        let k = {
            let cached = keys.lock().unwrap().get(ku).cloned();
            match cached {
                Some(k) => k,
                None => {
                    let (k, _) = get_bytes(c, ku)?;
                    keys.lock().unwrap().insert(ku.clone(), k.clone());
                    k
                }
            }
        };
        if k.len() != 16 {
            bail!("bad key ({} bytes)", k.len());
        }
        let iv = iv.unwrap_or_else(|| {
            let mut b = [0u8; 16];
            b[8..].copy_from_slice(&s.seq.to_be_bytes());
            b
        });
        use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
        let dec = cbc::Decryptor::<aes::Aes128>::new_from_slices(&k, &iv).map_err(|e| anyhow!("{e}"))?;
        let n = dec.decrypt_padded_mut::<Pkcs7>(&mut d).map_err(|_| anyhow!("could not decrypt a piece"))?.len();
        d.truncate(n);
    }
    if !fmp4 {
        // fake image header in front of the MPEG-TS data
        let off = rmp_core::hlsproxy::ts_start(&d);
        if off > 0 {
            d.drain(..off);
        }
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_join() {
        assert_eq!(safe_name("Re:Zero / S2?"), "Re Zero S2");
        assert_eq!(join("https://h.com/a/b.m3u8?x=1", "s/1.ts"), "https://h.com/a/s/1.ts");
        assert_eq!(join("https://h.com/a/b.m3u8", "/o/1.ts"), "https://h.com/o/1.ts");
        assert_eq!(attr(r#"METHOD=AES-128,URI="k.bin",IV=0x01"#, "URI").as_deref(), Some("k.bin"));
        let e = Episode { season: 1, num: 5.0, label: "Episode 5".into(), url: String::new(), dub: false };
        assert_eq!(ep_file_stem(&e), "E05 Episode 5");
        assert_eq!(parse_iv("0x000102030405060708090a0b0c0d0e0f").unwrap()[15], 15);
    }
}

#[cfg(test)]
mod live {
    /// Needs a local server: `WAKU_DL_TEST=http://localhost:8765/master.m3u8` (AES-128 playlist, see README).
    #[test]
    fn hls_from_env() {
        let Ok(url) = std::env::var("WAKU_DL_TEST") else { return };
        let c = waku_core::player::Candidate {
            url,
            referrer: String::new(),
            user_agent: "test".into(),
            force_hls: false,
            headers: vec![],
            direct: true,
            title: String::new(),
            source: String::new(),
            server: String::new(),
            height: None,
            variants: vec![],
        };
        let job = super::Job {
            id: 1,
            card: waku_core::AnimeCard { source: String::new(), url: String::new(), title: "t".into(), poster: None, badge: None },
            ep: waku_core::Episode { season: 1, num: 1.0, label: String::new(), url: String::new(), dub: false },
            st: std::sync::Mutex::new(super::JobState { status: super::Status::Downloading, done: 0, total: 0, bytes: 0, server: String::new() }),
            cancel: Default::default(),
        };
        let dir = std::env::temp_dir().join("waku-dl-test");
        let _ = std::fs::create_dir_all(&dir);
        let (name, bytes) = super::download(&c, &dir, "E01", &job, &egui::Context::default()).unwrap();
        let d = std::fs::read(dir.join(&name)).unwrap();
        assert_eq!(d.len() as u64, bytes);
        assert_eq!(d.len(), 5 * 1880);
        assert!(d.chunks(188).enumerate().all(|(i, p)| p[0] == 0x47 && p[1] == (i / 10) as u8), "decrypted in order");
        eprintln!("ok {name} {bytes}");
    }
}
