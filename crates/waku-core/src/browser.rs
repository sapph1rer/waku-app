//! Client for the `waku-browser` helper (hidden WebView2 window, see crates/waku-browser).
//!
//! The helper is started on first use and kept running; requests are serialised (one page at a time).
//! It answers one JSON object per line on stdout.

use crate::http::{Browser, Sniffed};
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

struct Proc {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next: u64,
}

pub struct BrowserProcess {
    exe: PathBuf,
    p: Mutex<Option<Proc>>,
    /// disabled after the helper failed to start (no WebView2 runtime …)
    broken: Mutex<Option<String>>,
}

impl BrowserProcess {
    /// The helper next to the running executable (`waku-browser.exe`), if it exists.
    pub fn beside_current_exe() -> Option<Self> {
        let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        let exe = dir.join(if cfg!(windows) { "waku-browser.exe" } else { "waku-browser" });
        (cfg!(windows) && exe.exists()).then(|| Self { exe, p: Mutex::new(None), broken: Mutex::new(None) })
    }

    fn start(&self) -> Result<Proc> {
        let mut cmd = Command::new(&self.exe);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().with_context(|| format!("starting {}", self.exe.display()))?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && tx.send(v).is_err()
                {
                    break;
                }
            }
        });
        // wait for {"ready": true}
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(v) if v.get("ready").and_then(|x| x.as_bool()) == Some(true) => {}
            Ok(v) => bail!("browser helper failed: {}", v.get("error").and_then(|x| x.as_str()).unwrap_or("?")),
            Err(_) => {
                let _ = child.kill();
                bail!("browser helper did not start (is the WebView2 runtime installed?)")
            }
        }
        Ok(Proc { child, stdin, rx, next: 1 })
    }

    fn call(&self, mut req: Value, timeout: Duration) -> Result<Value> {
        if let Some(e) = self.broken.lock().ok().and_then(|g| g.clone()) {
            bail!("browser helper unavailable: {e}");
        }
        let mut g = self.p.lock().map_err(|_| anyhow!("browser lock poisoned"))?;
        let alive = g.as_mut().is_some_and(|p| p.child.try_wait().ok().flatten().is_none());
        if !alive {
            match self.start() {
                Ok(p) => *g = Some(p),
                Err(e) => {
                    if let Ok(mut b) = self.broken.lock() {
                        *b = Some(format!("{e:#}"));
                    }
                    return Err(e);
                }
            }
        }
        let p = g.as_mut().unwrap();
        let id = p.next;
        p.next += 1;
        req["id"] = json!(id);
        writeln!(p.stdin, "{req}").context("writing to browser helper")?;
        p.stdin.flush()?;
        loop {
            match p.rx.recv_timeout(timeout) {
                Ok(v) if v.get("id").and_then(|x| x.as_u64()) == Some(id) => {
                    if v.get("ok").and_then(|x| x.as_bool()) == Some(true) {
                        return Ok(v);
                    }
                    bail!("{}", v.get("error").and_then(|x| x.as_str()).unwrap_or("browser error"));
                }
                Ok(_) => continue, // a late answer to an earlier, timed-out request
                Err(_) => {
                    // stuck: restart next time
                    let _ = p.child.kill();
                    *g = None;
                    bail!("browser helper timed out");
                }
            }
        }
    }
}

impl Drop for BrowserProcess {
    fn drop(&mut self) {
        if let Ok(mut g) = self.p.lock()
            && let Some(p) = g.as_mut()
        {
            let _ = writeln!(p.stdin, "{}", json!({"op": "quit"}));
        }
    }
}

impl Browser for BrowserProcess {
    fn cookies(&self, url: &str) -> Result<(String, String)> {
        let v = self.call(json!({"op": "cookies", "url": url}), Duration::from_secs(10))?;
        let c = v.get("cookies").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let ua = v.get("ua").and_then(|x| x.as_str()).unwrap_or("").to_string();
        Ok((c, ua))
    }

    fn fetch(&self, url: &str, referer: Option<&str>) -> Result<String> {
        let v = self.call(json!({"op": "fetch", "url": url, "referer": referer}), Duration::from_secs(95))?;
        let status = v.get("status").and_then(|x| x.as_u64()).unwrap_or(200);
        if status >= 400 {
            bail!("GET {url} (browser) -> HTTP {status}");
        }
        Ok(v.get("html").and_then(|x| x.as_str()).unwrap_or("").to_string())
    }

    fn sniff(&self, url: &str, referer: &str) -> Result<Sniffed> {
        let v = self.call(json!({"op": "sniff", "url": url, "referer": referer, "timeout": 25}), Duration::from_secs(45))?;
        let urls: Vec<String> = v.get("urls").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let page = v.get("page").and_then(|x| x.as_str()).unwrap_or(url).to_string();
        let o = crate::text::origin_of(&page);
        let note = v.get("diag").filter(|d| !d.is_null()).map(|d| d.to_string()).unwrap_or_default();
        let mut headers = vec![format!("Origin: {}", o.trim_end_matches('/'))];
        if let Some(c) = v.get("cookies").and_then(|x| x.as_str()).filter(|c| !c.is_empty()) {
            headers.push(format!("Cookie: {c}"));
            if let Some(ua) = v.get("ua").and_then(|x| x.as_str()).filter(|u| !u.is_empty()) {
                headers.push(format!("User-Agent: {ua}"));
            }
        }
        Ok(Sniffed { urls, headers, page, note })
    }
}

/// Register the helper for the whole process when it is installed next to the executable.
pub fn install() -> bool {
    match BrowserProcess::beside_current_exe() {
        Some(b) => {
            crate::http::set_browser(Box::new(b));
            true
        }
        None => false,
    }
}
