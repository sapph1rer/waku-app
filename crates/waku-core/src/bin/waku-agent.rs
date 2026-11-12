//! waku-agent — development helper: fetches pages from *this* PC's network on request.
//!
//!   waku-agent <dir>
//!
//! Watches `<dir>/req/*.json` and writes `<dir>/resp/<name>.txt` (first line = JSON meta, then the body).
//! Request: {"url": "...", "method": "GET|POST", "referer": "...", "headers": {"k":"v"},
//!           "form": {"k":"v"}, "client": "waku|cli|h1|browser", "ua": "..."}
//!   or     {"cmd": "diagnose", "source": "<id>"} / {"cmd": "search", "q": "..."} /
//!          {"cmd": "detail", "source": "<id>", "url": "..."} / {"cmd": "streams", "source": "<id>", "url": "..."} /
//!          {"cmd": "latest", "source": "<id>"} / {"cmd": "quit"} /
//!          {"cmd": "exec", "line": "cargo build ...", "cwd": "C:\\..."} (runs `cmd /C line` in the background,
//!          answer = exit code + output when it ends) / {"cmd": "restart"} (exit code 3: 4-agent.cmd rebuilds and
//!          starts the agent again).
//! Used to study the anime sites (Cloudflare, markup, players) from the real network Waku runs on.

use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use waku_core::SourceManager;

const UA_CLI: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

fn client(kind: &str, ua: Option<&str>) -> Client {
    let mut b = Client::builder().timeout(Duration::from_secs(30)).cookie_store(true).gzip(true).brotli(true);
    let mut h = reqwest::header::HeaderMap::new();
    let ua = ua.unwrap_or(match kind {
        "waku" => waku_core::http::UA,
        _ => UA_CLI,
    });
    h.insert("user-agent", ua.parse().unwrap());
    match kind {
        "browser" => {
            h.insert("accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8".parse().unwrap());
            h.insert("accept-language", "th-TH,th;q=0.9,en;q=0.8".parse().unwrap());
            h.insert("sec-fetch-dest", "document".parse().unwrap());
            h.insert("sec-fetch-mode", "navigate".parse().unwrap());
            h.insert("sec-fetch-site", "none".parse().unwrap());
            h.insert("upgrade-insecure-requests", "1".parse().unwrap());
        }
        "waku" => {
            h.insert("accept-language", "th,en;q=0.8".parse().unwrap());
            h.insert("accept", "text/html,application/xhtml+xml,application/json;q=0.9,*/*;q=0.8".parse().unwrap());
        }
        _ => {
            h.insert("accept-language", "th,en;q=0.8".parse().unwrap());
        }
    }
    b = b.default_headers(h);
    if kind == "h1" || kind == "cli" {
        b = b.http1_only();
    }
    b.build().unwrap()
}

fn fetch(r: &Value) -> String {
    let s = |k: &str| r.get(k).and_then(|v| v.as_str());
    let url = s("url").unwrap_or("");
    let c = client(s("client").unwrap_or("waku"), s("ua"));
    let t = Instant::now();
    let mut req = if s("method") == Some("POST") { c.post(url) } else { c.get(url) };
    if let Some(rf) = s("referer") {
        req = req.header("referer", rf);
    }
    if let Some(hs) = r.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in hs {
            req = req.header(k.as_str(), v.as_str().unwrap_or(""));
        }
    }
    if let Some(f) = r.get("form").and_then(|v| v.as_object()) {
        let pairs: Vec<(String, String)> = f.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect();
        req = req.form(&pairs);
    }
    match req.send() {
        Ok(resp) => {
            let st = resp.status().as_u16();
            let fin = resp.url().to_string();
            let hs: serde_json::Map<String, Value> = resp.headers().iter().map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap_or("")))).collect();
            let bytes = resp.bytes().map(|b| b.to_vec()).unwrap_or_default();
            let body = if r.get("hex").and_then(|v| v.as_bool()) == Some(true) {
                bytes.iter().take(1024).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
            } else {
                String::from_utf8_lossy(&bytes).to_string()
            };
            format!("{}\n{}", json!({"status": st, "url": fin, "ms": t.elapsed().as_millis() as u64, "len": bytes.len(), "headers": hs}), body)
        }
        Err(e) => format!("{}\n", json!({"error": format!("{e:#}"), "ms": t.elapsed().as_millis() as u64})),
    }
}

fn command(mgr: &SourceManager, r: &Value) -> String {
    let s = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    match s("cmd").as_str() {
        "diagnose" => {
            let mut out = String::new();
            for d in mgr.diagnose(&s("source")) {
                out += &format!("[{}] {:<14} {}\n", if d.ok { "ok" } else { "FAIL" }, d.name, d.detail);
                for t in d.trace {
                    out += &format!("      {t}\n");
                }
            }
            out
        }
        "search" => {
            // every source on its own thread, each timed
            let q = s("q");
            let ids: Vec<String> = mgr.sources().iter().map(|x| x.id().to_string()).collect();
            let res: Vec<String> = std::thread::scope(|sc| {
                let hs: Vec<_> = ids.iter().map(|id| {
                    let q = q.clone();
                    sc.spawn(move || {
                        let t = Instant::now();
                        let r = mgr.search(id, &q);
                        let ms = t.elapsed().as_millis();
                        match r {
                            Ok(v) => {
                                let mut o = format!("{id}: {} result(s) in {ms} ms\n", v.len());
                                for c in v.iter().take(5) {
                                    o += &format!("  {} | {} | poster={}\n", c.title, c.url, c.poster.is_some());
                                }
                                o
                            }
                            Err(e) => format!("{id}: error in {ms} ms: {e:#}\n"),
                        }
                    })
                }).collect();
                hs.into_iter().map(|h| h.join().unwrap_or_default()).collect()
            });
            res.concat()
        }
        "latest" => match mgr.latest(&s("source"), 1) {
            Ok(v) => v.iter().map(|c| format!("{} | {} | poster={} | badge={:?}\n", c.title, c.url, c.poster.as_deref().unwrap_or("-"), c.badge)).collect(),
            Err(e) => format!("error: {e:#}\n"),
        },
        "detail" => match mgr.detail(&s("source"), &s("url")) {
            Ok(d) => {
                let mut out = format!("title={} eps={}\n", d.title, d.episodes.len());
                for e in &d.episodes {
                    out += &format!("  s={} num={} dub={} label={} url={}\n", e.season, e.num, e.dub, e.label, e.url);
                }
                out
            }
            Err(e) => format!("error: {e:#}\n"),
        },
        "streams" => match mgr.streams_traced(&s("source"), &s("url"), true) {
            Ok((st, trace)) => {
                let mut out = format!("{st:#?}\n--- trace\n{}\n", trace.join("\n"));
                if r.get("deep").and_then(|v| v.as_bool()) == Some(true) {
                    for x in st.iter().filter(|x| x.rank == waku_core::StreamRank::Unknown).take(3) {
                        out += &format!("--- deepen {} {}\n", x.server, x.variants[0].url);
                        match mgr.deepen(x) {
                            Some(d) => out += &format!("{d:#?}\n"),
                            None => {
                                let note = waku_core::http::browser()
                                    .and_then(|b| b.sniff(&x.variants[0].url, &x.page).ok())
                                    .map(|f| f.note)
                                    .unwrap_or_default();
                                out += &format!("nothing found {note}\n");
                            }
                        }
                    }
                }
                out
            }
            Err(e) => format!("error: {e:#}\n"),
        },
        // {"cmd":"survey","source":"waku","n":8}: latest N titles -> newest + first episode -> streams, one line each
        "survey" => {
            let n = r.get("n").and_then(|v| v.as_u64()).unwrap_or(6) as usize;
            let skip = r.get("skip").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let src = s("source");
            let mut out = String::new();
            let cards = match mgr.latest(&src, 1) {
                Ok(c) => c,
                Err(e) => return format!("latest: {e:#}\n"),
            };
            for c in cards.iter().skip(skip).take(n) {
                let t = Instant::now();
                let d = match mgr.detail(&src, &c.url) {
                    Ok(d) => d,
                    Err(e) => {
                        out += &format!("## {} : detail error {e:#}\n", c.title);
                        continue;
                    }
                };
                out += &format!("## {} ({} eps, detail {} ms)\n", c.title, d.episodes.len(), t.elapsed().as_millis());
                let mut eps: Vec<&waku_core::Episode> = d.episodes.iter().rev().take(1).collect();
                if let Some(f) = d.episodes.first() {
                    if d.episodes.len() > 1 {
                        eps.push(f);
                    }
                }
                for e in eps {
                    let t = Instant::now();
                    match mgr.streams_traced(&src, &e.url, true) {
                        Ok((st, tr)) => {
                            out += &format!("  ep {} {} ({} ms) {}\n", e.label, e.url, t.elapsed().as_millis(), if e.dub { "DUB" } else { "" });
                            for x in &st {
                                out += &format!("    [{:?}] {} hls={} h={:?} {}\n", x.rank, x.server, x.hls, x.variants.iter().map(|v| v.height).collect::<Vec<_>>(), x.variants.first().map(|v| v.url.as_str()).unwrap_or(""));
                            }
                            for l in tr.iter().filter(|l| r.get("trace").and_then(|v| v.as_bool()) == Some(true) || l.contains("embed") || !l.starts_with(' ')) {
                                out += &format!("      | {l}\n");
                            }
                        }
                        Err(err) => out += &format!("  ep {} {} FAILED: {err:#}\n", e.label, e.url),
                    }
                }
            }
            out
        }
        // {"cmd":"meta","title":"…"}: identify + enrich, timed, with episode titles
        "meta" => {
            let t = Instant::now();
            let http = mgr.http();
            let eps: u32 = s("eps").parse().unwrap_or(0);
            match waku_core::meta::identify(http, &s("title"), &s("url"), eps) {
                Ok(m) => {
                    let t1 = t.elapsed().as_millis();
                    let m = waku_core::meta::enrich(http, m);
                    let mut out = format!("identify {t1} ms, enrich {} ms: anilist={:?} mal={:?} kitsu={:?} title={:?} eps={} notes={:?}\n", t.elapsed().as_millis(), m.anilist_id, m.mal_id, m.kitsu_id, m.title_english.as_ref().or(m.title_romaji.as_ref()), m.eps.len(), m.notes);
                    for e in m.eps.iter().take(30) {
                        out += &format!("  {} {:?} thumb={}\n", e.number, e.title, e.thumbnail.is_some());
                    }
                    out
                }
                Err(e) => format!("error {} ms: {e:#}\n", t.elapsed().as_millis()),
            }
        }
        // the app's own GET (Cloudflare fast path / browser fallback), timed
        "get" => {
            let t = Instant::now();
            let r = mgr.http().get(&s("url"), Some(&s("referer")).filter(|x| !x.is_empty()).map(|x| x.as_str()));
            match r {
                Ok(h) => format!("ok {} ms len={} cf={}\n{}", t.elapsed().as_millis(), h.len(), waku_core::http::is_cf_host(&s("url")), h.chars().take(300).collect::<String>()),
                Err(e) => format!("error {} ms: {e:#}\n", t.elapsed().as_millis()),
            }
        }
        "sniff" => match waku_core::http::browser() {
            Some(b) => match b.sniff(&s("url"), &s("referer")) {
                Ok(f) => format!("{f:#?}\n{}\n", f.note),
                Err(e) => format!("error: {e:#}\n"),
            },
            None => "no browser helper\n".into(),
        },
        "bfetch" => match waku_core::http::browser() {
            Some(b) => match b.fetch(&s("url"), Some(&s("referer")).filter(|r| !r.is_empty()).map(|x| x.as_str())) {
                Ok(h) => format!("len={}\n{h}", h.len()),
                Err(e) => format!("error: {e:#}\n"),
            },
            None => "no browser helper\n".into(),
        },
        other => format!("unknown cmd {other}\n"),
    }
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let (rq, rs) = (dir.join("req"), dir.join("resp"));
    let _ = std::fs::create_dir_all(&rq);
    let _ = std::fs::create_dir_all(&rs);
    let _ = std::fs::write(dir.join("agent.alive"), format!("{:?}", std::time::SystemTime::now()));
    if std::env::var_os("WAKU_HTTP_LOG").is_none() {
        // SAFETY: set before any other thread starts
        unsafe { std::env::set_var("WAKU_HTTP_LOG", dir.join("http.log")) };
    }
    let browser = waku_core::browser::install();
    let mut mgr = SourceManager::load();
    println!("waku-agent watching {} (browser helper: {browser})", rq.display());
    loop {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&rq).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        files.sort();
        for f in files {
            if f.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let name = f.file_stem().unwrap().to_string_lossy().to_string();
            let txt = std::fs::read_to_string(&f).unwrap_or_default();
            let _ = std::fs::remove_file(&f);
            let r: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
            if r.get("cmd").and_then(|v| v.as_str()) == Some("quit") {
                return;
            }
            if r.get("cmd").and_then(|v| v.as_str()) == Some("restart") {
                write(&rs, &name, "restarting\n");
                std::process::exit(3);
            }
            if r.get("cmd").and_then(|v| v.as_str()) == Some("exec") {
                let line = r.get("line").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let cwd = r.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from).unwrap_or_else(|| dir.clone());
                let rs2 = rs.clone();
                println!("-> {name}: {line}");
                std::thread::spawn(move || {
                    let t = Instant::now();
                    let mut c = if cfg!(windows) { std::process::Command::new("cmd") } else { std::process::Command::new("sh") };
                    c.arg(if cfg!(windows) { "/C" } else { "-c" }).arg(&line).current_dir(&cwd);
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        c.creation_flags(0x0800_0000);
                    }
                    let out = match c.output() {
                        Ok(o) => format!(
                            "exit={} secs={:.1}\n--- stdout\n{}\n--- stderr\n{}",
                            o.status.code().unwrap_or(-1),
                            t.elapsed().as_secs_f64(),
                            String::from_utf8_lossy(&o.stdout),
                            String::from_utf8_lossy(&o.stderr)
                        ),
                        Err(e) => format!("spawn error: {e}\n"),
                    };
                    write(&rs2, &name, &out);
                });
                continue;
            }
            if r.get("cmd").and_then(|v| v.as_str()) == Some("reload") {
                mgr = SourceManager::load();
                write(&rs, &name, "reloaded\n");
                continue;
            }
            println!("-> {name}");
            let out = if r.get("cmd").is_some() { command(&mgr, &r) } else { fetch(&r) };
            write(&rs, &name, &out);
        }
        let _ = std::fs::write(dir.join("agent.alive"), format!("{:?}", std::time::SystemTime::now()));
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn write(dir: &Path, name: &str, s: &str) {
    let tmp = dir.join(format!("{name}.part"));
    let _ = std::fs::write(&tmp, s);
    let _ = std::fs::rename(&tmp, dir.join(format!("{name}.txt")));
}
