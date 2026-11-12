//! waku-browser — a hidden WebView2 window Waku drives over stdin/stdout (one JSON object per line).
//!
//! Requests:
//!   {"id": 1, "op": "fetch", "url": "...", "referer": "..."}   -> page HTML after scripts / Cloudflare check
//!   {"id": 2, "op": "sniff", "url": "...", "referer": "..."}   -> video URLs the page's player requests
//!   {"id": 3, "op": "show"} / {"op": "hide"} / {"op": "quit"}
//! Replies:
//!   {"id": 1, "ok": true, "status": 200, "url": "...", "html": "..."}
//!   {"id": 2, "ok": true, "urls": ["..."], "page": "...", "pages": ["..."]}
//!   {"id": n, "ok": false, "error": "..."}
//!
//! Why: some sites sit behind Cloudflare's JavaScript check (plain HTTP gets 403), and some players build the
//! stream URL in heavily obfuscated JavaScript. A real browser engine handles both. The window stays hidden;
//! it is shown only when a check needs a click (Turnstile), and hidden again afterwards.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("waku-browser needs Windows (WebView2)");
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    if let Err(e) = win::run() {
        let _ = std::fs::write(std::env::temp_dir().join("waku-browser-error.txt"), format!("{e:#?}"));
        println!("{}", serde_json::json!({"id": 0, "ok": false, "fatal": true, "error": format!("{e}")}));
        std::process::exit(1);
    }
}

/// Injected into every document and frame before the page's own scripts.
pub const HOOK_JS: &str = r#"
(function () {
  if (window.__wk) return; window.__wk = 1;
  var RX = /\.m3u8|\.mp4|\.mpd|\.mkv|\.webm|\/hls\/|\/m3u8\/|playlist|manifest|master|\.txt(\?|$)/i;
  var BAD = /\.(js|css|png|jpe?g|gif|svg|woff2?|ttf|ico|webp|json)(\?|$)|googlesyndication|google-analytics|googletagmanager|histats|doubleclick|amung\.us|cloudflareinsights|\/cdn-cgi\/|imasdk|\/vast|preroll|pre-roll|adserver|popads|\/ads?\/|cdend\.com|\/ufa[a-z0-9]*\.mp4|advert/i;
  var top_ = (function () { try { return window.top === window; } catch (e) { return false; } })();
  function send(m) {
    try {
      if (top_) { if (window.ipc) window.ipc.postMessage(JSON.stringify(m)); }
      else window.top.postMessage({ __wk: m }, '*');
    } catch (e) {}
  }
  if (top_) {
    window.addEventListener('message', function (e) {
      if (e.data && e.data.__wk && window.ipc) window.ipc.postMessage(JSON.stringify(e.data.__wk));
    });
  }
  function rep(u, how, force) {
    if (!u || typeof u !== 'string') return;
    try { u = new URL(u, location.href).href; } catch (e) { return; }
    if (!/^https?:/.test(u)) return;
    if (BAD.test(u)) return;
    if (!force && !RX.test(u)) return;
    window.__wkN = (window.__wkN || 0) + 1;
    send({ k: 'media', url: u, page: location.href, how: how });
  }
  window.__wkRep = rep;
  window.addEventListener('error', function (e) { (window.__wkErr = window.__wkErr || []).push(String(e.message).slice(0, 120)); });
  try {
    var of = window.fetch;
    if (of) window.fetch = function (i) {
      try { rep(typeof i === 'string' ? i : (i && i.url), 'fetch'); } catch (e) {}
      return of.apply(this, arguments);
    };
  } catch (e) {}
  try {
    var oo = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function (m, u) {
      try { rep(String(u), 'xhr'); } catch (e) {}
      return oo.apply(this, arguments);
    };
  } catch (e) {}
  try {
    var d = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
    if (d && d.set) Object.defineProperty(HTMLMediaElement.prototype, 'src', {
      get: d.get, configurable: true,
      set: function (v) { rep(String(v), 'video', true); return d.set.call(this, v); }
    });
    var sa = Element.prototype.setAttribute;
    Element.prototype.setAttribute = function (n, v) {
      try { if (n === 'src' && (this instanceof HTMLMediaElement || this instanceof HTMLSourceElement)) rep(String(v), 'video', true); } catch (e) {}
      return sa.apply(this, arguments);
    };
  } catch (e) {}
  try {
    new PerformanceObserver(function (l) { l.getEntries().forEach(function (e) { rep(e.name, 'res'); }); })
      .observe({ type: 'resource', buffered: true });
  } catch (e) {}
  // jwplayer("x").setup({...}): read the sources out of the config
  function scan(c, depth, key) {
    if (c == null || depth > 7) return;
    if (typeof c === 'string') {
      if (/^(https?:)?\/\//.test(c) || /^\//.test(c)) rep(c, 'jw', /^(file|src|url|hls|dash|playlist|source)$/i.test(key || ''));
      return;
    }
    if (typeof c === 'object') { for (var k in c) { try { scan(c[k], depth + 1, k); } catch (e) {} } }
  }
  try {
    var real;
    Object.defineProperty(window, 'jwplayer', {
      configurable: true,
      get: function () { return real; },
      set: function (v) {
        if (typeof v !== 'function') { real = v; return; }
        real = new Proxy(v, {
          apply: function (t, self, args) {
            var p = Reflect.apply(t, self, args);
            try {
              if (p && p.setup && !p.__wk) {
                var s = p.setup; p.__wk = 1;
                p.setup = function (c) { try { scan(c, 0, ''); } catch (e) {} return s.apply(this, arguments); };
              }
            } catch (e) {}
            return p;
          }
        });
      }
    });
  } catch (e) {}
  // never make a sound
  try {
    var op = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function () { try { this.muted = true; } catch (e) {} return op.apply(this, arguments); };
  } catch (e) {}
  // try to start the player (Rust calls __wkKick in the top frame; it forwards to sub-frames)
  function kick() {
    try { if (real && typeof real === 'function') { var p = real(); if (p && p.play) p.play(); } } catch (e) {}
    try { document.querySelectorAll('video').forEach(function (v) { v.muted = true; var r = v.play(); if (r && r.catch) r.catch(function () {}); }); } catch (e) {}
    try {
      var b = document.querySelector('.jw-icon-display, .vjs-big-play-button, .plyr__control--overlaid, [class*=play-button], [class*=btn-play], .play-btn, #play');
      if (b) b.click();
    } catch (e) {}
    try { for (var i = 0; i < window.frames.length; i++) window.frames[i].postMessage({ __wkkick: 1 }, '*'); } catch (e) {}
  }
  window.__wkKick = kick;
  window.addEventListener('message', function (e) { if (e.data && e.data.__wkkick) kick(); });
  window.open = function () { return null; };
})();
"#;

#[cfg(windows)]
mod win {
    use serde_json::{Value, json};
    use std::io::{BufRead, Write};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
    use tao::window::WindowBuilder;
    use wry::http::HeaderMap;
    use wry::{WebContext, WebViewBuilder};

    #[derive(Debug)]
    enum Ev {
        Cmd(Value),
        StdinClosed,
        /// result of a polling script: [readyState, title, href, challenged]
        Poll(u64, String),
        Html(u64, String),
        Ipc(String),
    }

    enum Kind {
        Fetch,
        Sniff,
    }

    struct Job {
        id: Value,
        kind: Kind,
        token: u64,
        url: String,
        started: Instant,
        deadline: Instant,
        loaded_at: Option<Instant>,
        first_media: Option<Instant>,
        last_kick: Instant,
        urls: Vec<String>,
        pages: Vec<String>,
        shown: bool,
        tried_inpage: bool,
        diag: Value,
        html_pending: bool,
        /// the page that was open when the job started: a poll still seeing it means the navigation has not
        /// replaced it yet (its HTML must not be taken for the new page's)
        start_href: String,
    }

    /// Debug trace (`WAKU_HTTP_LOG=<file>`, shared with waku-core's notes).
    fn trace(msg: &str) {
        if let Some(p) = std::env::var_os("WAKU_HTTP_LOG") {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 1_000_000).unwrap_or(0);
                let _ = writeln!(f, "{t:>6} [browser] {msg}");
            }
        }
    }

    fn reply(v: Value) {
        let s = v.to_string();
        trace(&format!("reply {}", s.chars().take(160).collect::<String>()));
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{v}");
        let _ = out.flush();
    }

    fn origin(u: &str) -> String {
        let (scheme, rest) = u.split_once("://").unwrap_or(("https", u));
        format!("{scheme}://{}", rest.split(['/', '?', '#']).next().unwrap_or(""))
    }

    const POLL_JS: &str = r#"(function(){var t=document.title||'';var b=document.body?document.body.innerText.slice(0,400):'';
var ch=/just a moment|attention required|checking your browser|verify you are human|โปรดรอสักครู่|security check/i.test(t+' '+b)
 || !!document.querySelector('#challenge-form,#cf-challenge-running,.cf-turnstile,#turnstile-wrapper,[name=cf-turnstile-response]');
var dg={jw:typeof window.jwplayer,ipc:!!window.ipc,ifr:document.querySelectorAll('iframe').length,vid:document.querySelectorAll('video').length,
 rep:window.__wkN||0,vsrc:Array.prototype.map.call(document.querySelectorAll('video'),function(v){return v.currentSrc||v.src}).join(' '),
 ifs:Array.prototype.map.call(document.querySelectorAll('iframe'),function(f){return f.src}).join(' ').slice(0,300),err:(window.__wkErr||[]).slice(-3).join(' | ')};
return JSON.stringify([document.readyState,t,location.href,ch,dg]);})()"#;

    pub fn run() -> anyhow_lite::Result<()> {
        let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
        let proxy: EventLoopProxy<Ev> = event_loop.create_proxy();

        // stdin reader
        {
            let p = proxy.clone();
            std::thread::spawn(move || {
                let stdin = std::io::stdin();
                for line in stdin.lock().lines() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<Value>(&line) {
                        Ok(v) => {
                            let _ = p.send_event(Ev::Cmd(v));
                        }
                        Err(e) => reply(json!({"id": 0, "ok": false, "error": format!("bad request: {e}")})),
                    }
                }
                let _ = p.send_event(Ev::StdinClosed);
            });
        }

        let window = WindowBuilder::new()
            .with_title("Waku — browser check")
            .with_visible(false)
            .with_inner_size(LogicalSize::new(1000.0, 720.0))
            .build(&event_loop)?;

        let data_dir = std::env::var_os("WAKU_BROWSER_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Waku").join("browser")
        });
        let _ = std::fs::create_dir_all(&data_dir);
        let mut ctx = WebContext::new(Some(data_dir));

        // only the job's own navigation (and redirects on its site) may change the top page: ad scripts like to
        // send the window elsewhere
        let nav_host: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let ipc_proxy = proxy.clone();
        let nh = nav_host.clone();
        let webview = WebViewBuilder::new_with_web_context(&mut ctx)
            .with_initialization_script(crate::HOOK_JS)
            .with_ipc_handler(move |req| {
                let _ = ipc_proxy.send_event(Ev::Ipc(req.body().clone()));
            })
            .with_new_window_req_handler(|_url, _features| wry::NewWindowResponse::Deny)
            .with_navigation_handler(move |url: String| {
                let allowed = nh.lock().ok().and_then(|g| g.clone());
                match allowed {
                    None => true,
                    Some(o) => url.starts_with("about:") || origin(&url) == o || url.contains("challenges.cloudflare.com"),
                }
            })
            .with_autoplay(true)
            .with_url("about:blank")
            .build(&window)?;

        let mut job: Option<Job> = None;
        let mut queue: std::collections::VecDeque<Value> = Default::default();
        let mut token: u64 = 0;
        // the engine's own User-Agent (cookies some hosts set are checked against it)
        let ua: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let mut current_origin = String::new();
        let mut current_href = String::new();
        let mut challenged_origin: Option<String> = None;

        {
            let ua = ua.clone();
            let _ = webview.evaluate_script_with_callback("navigator.userAgent", move |s| {
                if let Ok(mut g) = ua.lock() {
                    *g = serde_json::from_str::<String>(&s).unwrap_or_default();
                }
            });
        }
        reply(json!({"id": 0, "ok": true, "ready": true}));

        event_loop.run(move |event, _, control_flow| {
            let _keep_context_alive = &ctx;
            *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(400));
            let mut finish: Option<Value> = None;
            match event {
                Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                    // the user closed the check window: hide it, fail the job
                    window.set_visible(false);
                    if let Some(j) = &job {
                        finish = Some(json!({"id": j.id, "ok": false, "error": "browser window closed"}));
                    }
                }
                Event::UserEvent(Ev::StdinClosed) => *control_flow = ControlFlow::Exit,
                Event::UserEvent(Ev::Cmd(v)) => match v.get("op").and_then(|x| x.as_str()).unwrap_or("") {
                    "quit" => *control_flow = ControlFlow::Exit,
                    "show" => window.set_visible(true),
                    "hide" => window.set_visible(false),
                    "ping" => reply(json!({"id": v.get("id"), "ok": true})),
                    "cookies" => {
                        let u = v.get("url").and_then(|x| x.as_str()).unwrap_or("");
                        let c = webview
                            .cookies_for_url(u)
                            .map(|v| v.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; "))
                            .unwrap_or_default();
                        let ua = ua.lock().map(|g| g.clone()).unwrap_or_default();
                        reply(json!({"id": v.get("id"), "ok": true, "cookies": c, "ua": ua}));
                    }
                    _ => queue.push_back(v),
                },
                Event::UserEvent(Ev::Ipc(body)) => {
                    if let (Some(j), Ok(m)) = (job.as_mut(), serde_json::from_str::<Value>(&body)) {
                        match m.get("k").and_then(|x| x.as_str()) {
                            Some("media") if matches!(j.kind, Kind::Sniff) => {
                                let u = m.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                let p = m.get("page").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                if !u.is_empty() && !j.urls.contains(&u) {
                                    j.urls.push(u);
                                    j.pages.push(p);
                                    j.first_media.get_or_insert_with(Instant::now);
                                }
                            }
                            Some("fetch") if matches!(j.kind, Kind::Fetch) && m.get("t").and_then(|x| x.as_u64()) == Some(j.token) => {
                                let status = m.get("status").and_then(|x| x.as_u64()).unwrap_or(0);
                                let body = m.get("body").and_then(|x| x.as_str()).unwrap_or("");
                                let challenged = (status == 403 || status == 503) && body.contains("challenge");
                                if m.get("err").is_none() && status > 0 && !challenged {
                                    finish = Some(json!({"id": j.id, "ok": true, "status": status, "url": j.url, "html": body}));
                                } else {
                                    // fall back to a real navigation
                                    navigate(&webview, &nav_host, &j.url, None);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::UserEvent(Ev::Poll(t, s)) => {
                    if let Some(j) = job.as_mut().filter(|j| j.token == t) {
                        let v: Value = serde_json::from_str(&s).ok().and_then(|x: Value| x.as_str().and_then(|s| serde_json::from_str(s).ok())).unwrap_or(Value::Null);
                        if v.get(0).and_then(|x| x.as_str()) == Some("__wkhtml__") {
                            return;
                        }
                        let ready = v.get(0).and_then(|x| x.as_str()) == Some("complete");
                        let href = v.get(2).and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let ch = v.get(3).and_then(|x| x.as_bool()).unwrap_or(false);
                        j.diag = json!({"title": v.get(1), "href": href, "challenged": ch, "page": v.get(4)});
                        trace(&format!("poll t={} ready={ready} ch={ch} href={href} start={} job={}", j.token, j.start_href, j.url));
                        if !href.is_empty() && href != "about:blank" {
                            current_origin = origin(&href);
                            current_href = href.clone();
                        }
                        let strip = |u: &str| u.split('#').next().unwrap_or(u).trim_end_matches('/').to_string();
                        let stale = matches!(j.kind, Kind::Fetch)
                            && strip(&href) == strip(&j.start_href)
                            && strip(&href) != strip(&j.url)
                            && j.started.elapsed() < Duration::from_secs(20);
                        if stale {
                            return; // the old page is still up: wait for the navigation
                        }
                        if ch {
                            challenged_origin = Some(origin(&j.url));
                            // a check that does not pass on its own needs the user: show the window
                            if !j.shown && j.started.elapsed() > Duration::from_secs(8) {
                                j.shown = true;
                                window.set_visible(true);
                                window.set_focus();
                            }
                        } else if ready && href != "about:blank" {
                            if j.loaded_at.is_none() {
                                j.loaded_at = Some(Instant::now());
                            }
                            if matches!(j.kind, Kind::Fetch) && !j.html_pending {
                                j.html_pending = true;
                                let p = proxy.clone();
                                let tk = j.token;
                                let _ = webview.evaluate_script_with_callback("JSON.stringify(['__wkhtml__', location.href, document.documentElement.outerHTML])", move |h| {
                                    let _ = p.send_event(Ev::Html(tk, h));
                                });
                            }
                        }
                    }
                }
                Event::UserEvent(Ev::Html(t, h)) => {
                    if let Some(j) = job.as_mut().filter(|j| j.token == t && matches!(j.kind, Kind::Fetch)) {
                        // the callback returns a JSON string holding ["__wkhtml__", href, html]
                        let inner: String = serde_json::from_str(&h).unwrap_or_default();
                        let arr: Value = serde_json::from_str(&inner).unwrap_or(Value::Null);
                        let ok = arr.get(0).and_then(|x| x.as_str()) == Some("__wkhtml__");
                        let html = arr.get(2).and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let got = arr.get(1).and_then(|x| x.as_str()).unwrap_or("");
                        let strip = |u: &str| u.split('#').next().unwrap_or(u).trim_end_matches('/').to_string();
                        let old_page = strip(got) == strip(&j.start_href) && strip(got) != strip(&j.url) && j.started.elapsed() < Duration::from_secs(20);
                        if !ok || html.len() < 64 || old_page {
                            // evaluated while the document was being replaced (or a stray answer): try again
                            j.html_pending = false;
                            return;
                        }
                        challenged_origin = challenged_origin.take().filter(|o| *o != origin(&j.url));
                        finish = Some(json!({"id": j.id, "ok": true, "status": 200, "url": j.url, "html": html}));
                    }
                }
                _ => {}
            }

            // start the next job
            if job.is_none()
                && finish.is_none()
                && let Some(v) = queue.pop_front()
            {
                token += 1;
                let url = v.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let referer = v.get("referer").and_then(|x| x.as_str()).map(str::to_string);
                let kind = if v.get("op").and_then(|x| x.as_str()) == Some("sniff") { Kind::Sniff } else { Kind::Fetch };
                let secs = v.get("timeout").and_then(|x| x.as_u64()).unwrap_or(match kind {
                    Kind::Fetch => 60,
                    Kind::Sniff => 25,
                });
                let mut j = Job {
                    id: v.get("id").cloned().unwrap_or(Value::Null),
                    kind,
                    token,
                    url: url.clone(),
                    started: Instant::now(),
                    deadline: Instant::now() + Duration::from_secs(secs),
                    loaded_at: None,
                    first_media: None,
                    last_kick: Instant::now(),
                    urls: Vec::new(),
                    pages: Vec::new(),
                    shown: false,
                    tried_inpage: false,
                    diag: Value::Null,
                    html_pending: false,
                    start_href: current_href.clone(),
                };
                trace(&format!("job {token} {} {url} (current {current_href})", v.get("op").and_then(|x| x.as_str()).unwrap_or("")));
                // same site already open and past its check: fetch from inside the page (fast, keeps cookies)
                let same = origin(&url) == current_origin && challenged_origin.as_deref() != Some(current_origin.as_str());
                if matches!(j.kind, Kind::Fetch) && same {
                    j.tried_inpage = true;
                    let js = format!(
                        "fetch({u},{{credentials:'include'}}).then(function(r){{return r.text().then(function(t){{window.ipc.postMessage(JSON.stringify({{k:'fetch',t:{tk},status:r.status,body:t}}))}})}}).catch(function(e){{window.ipc.postMessage(JSON.stringify({{k:'fetch',t:{tk},err:String(e)}}))}})",
                        u = serde_json::to_string(&url).unwrap(),
                        tk = token
                    );
                    let _ = webview.evaluate_script(&js);
                } else {
                    navigate(&webview, &nav_host, &url, referer.as_deref());
                }
                job = Some(j);
            }

            // poll the running job
            if finish.is_none()
                && let Some(j) = job.as_mut()
            {
                let now = Instant::now();
                if now > j.deadline {
                    finish = Some(match j.kind {
                        Kind::Sniff if !j.urls.is_empty() => sniff_reply(j, &webview, &ua.lock().map(|g| g.clone()).unwrap_or_default()),
                        Kind::Sniff => json!({"id": j.id, "ok": true, "urls": [], "page": j.url, "pages": [], "diag": j.diag}),
                        Kind::Fetch => json!({"id": j.id, "ok": false, "error": format!("timed out waiting for the page (browser check not passed?) {}", j.diag)}),
                    });
                } else if matches!(j.kind, Kind::Sniff)
                    && j.first_media.is_some_and(|t| now.duration_since(t) > Duration::from_millis(2500))
                {
                    finish = Some(sniff_reply(j, &webview, &ua.lock().map(|g| g.clone()).unwrap_or_default()));
                } else if !j.html_pending && !(matches!(j.kind, Kind::Fetch) && j.tried_inpage && j.loaded_at.is_none() && now.duration_since(j.started) < Duration::from_secs(4)) {
                    let p = proxy.clone();
                    let tk = j.token;
                    let _ = webview.evaluate_script_with_callback(POLL_JS, move |s| {
                        let _ = p.send_event(Ev::Poll(tk, s));
                    });
                    if matches!(j.kind, Kind::Sniff) && j.loaded_at.is_some() && now.duration_since(j.last_kick) > Duration::from_millis(1500) {
                        j.last_kick = now;
                        let _ = webview.evaluate_script("window.__wkKick&&window.__wkKick()");
                    }
                }
            }

            if let Some(r) = finish {
                if job.as_ref().is_some_and(|j| j.shown) {
                    window.set_visible(false);
                }
                reply(r);
                job = None;
                if let Ok(mut g) = nav_host.lock() {
                    *g = None;
                }
                // sniffing leaves a playing page behind: stop it
                let _ = webview.evaluate_script("try{document.querySelectorAll('video').forEach(function(v){v.pause()})}catch(e){}");
                *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(10));
            }
        });
    }

    fn sniff_reply(j: &Job, webview: &wry::WebView, ua: &str) -> Value {
        // playlists first, then files; the frame that asked for the first one is the Referer to use
        let mut order: Vec<usize> = (0..j.urls.len()).collect();
        let rank = |u: &str| {
            let l = u.to_lowercase();
            if l.contains(".m3u8") || l.contains("/hls/") || l.contains("/m3u8/") || l.contains("master") || l.contains("playlist") {
                0
            } else if l.contains(".mp4") || l.contains(".mkv") || l.contains(".webm") {
                1
            } else {
                2
            }
        };
        order.sort_by_key(|&i| rank(&j.urls[i]));
        let urls: Vec<&String> = order.iter().map(|&i| &j.urls[i]).collect();
        let pages: Vec<&String> = order.iter().map(|&i| &j.pages[i]).collect();
        // some hosts hand out links that only work with the cookies their page script set (fairyanime's
        // tonytonychopper): pass them on, with the browser's User-Agent they belong to
        let cookies = urls
            .first()
            .and_then(|u| webview.cookies_for_url(u).ok())
            .map(|v| v.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; "))
            .unwrap_or_default();
        json!({"id": j.id, "ok": true, "urls": urls, "page": pages.first().map(|s| s.as_str()).unwrap_or(&j.url), "pages": pages, "cookies": cookies, "ua": ua})
    }

    fn navigate(webview: &wry::WebView, nav_host: &Arc<Mutex<Option<String>>>, url: &str, referer: Option<&str>) {
        if let Ok(mut g) = nav_host.lock() {
            *g = Some(origin(url));
        }
        let mut h = HeaderMap::new();
        if let Some(r) = referer
            && let Ok(v) = r.parse()
        {
            h.insert("referer", v);
        }
        let _ = webview.load_url_with_headers(url, h);
    }

    /// Minimal error plumbing without pulling anyhow into the helper.
    pub mod anyhow_lite {
        pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    }
}
