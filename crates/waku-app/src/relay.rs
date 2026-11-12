//! Watch party across the internet without a server of our own: the messages go through ntfy.sh (a public
//! publish / subscribe service), end-to-end encrypted.
//!
//! * A room is a short code, `K7F3-9QMX` (8 characters of Crockford base32, ~40 bits).
//! * PBKDF2-HMAC-SHA256 (210 000 rounds) turns the code into 64 bytes: the first 32 are the AES-256-GCM key,
//!   the rest names the ntfy topic. The topic therefore says nothing about the code, and guessing codes costs
//!   the slow derivation each time — ntfy.sh (who sees the topic) cannot cheaply work back to the key.
//! * Every message: random 96-bit nonce + AES-256-GCM of `{from, seq, at, w}` → base64. Wrong key / tampered
//!   text fails to open and is dropped; `from` + increasing `seq` + a time window drop replays.
//! * ntfy.sh allows ~250 messages a day per IP (bursts of 60, then 1 every 5 s): only real events are sent
//!   (play / pause / seek / episode / chat), a burst is merged (only the newest position goes out), at most
//!   one message a second, and the host's position check is every 2 minutes instead of every 3 s.

use crate::app::Msg;
use crate::party::Wire;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_SERVER: &str = "https://ntfy.sh";
const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const ROUNDS: u32 = 210_000;

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("system random numbers");
    b
}

/// A new room code, `XXXX-XXXX`.
pub fn new_code() -> String {
    let r: [u8; 8] = random();
    let s: String = r.iter().map(|b| ALPHABET[(*b & 31) as usize] as char).collect();
    format!("{}-{}", &s[..4], &s[4..])
}

/// The code as typed → canonical 8 characters (spaces / dashes ignored; O→0, I/L→1 like Crockford), or None.
pub fn normalize(code: &str) -> Option<String> {
    let mut out = String::new();
    for c in code.trim().trim_start_matches("WAKU").trim_start_matches("waku").chars() {
        let c = match c.to_ascii_uppercase() {
            ' ' | '-' | '_' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !ALPHABET.contains(&(c as u8)) {
            return None;
        }
        out.push(c);
    }
    (out.len() == 8).then_some(out)
}

pub fn pretty(code: &str) -> String {
    normalize(code).map(|c| format!("{}-{}", &c[..4], &c[4..])).unwrap_or_else(|| code.to_string())
}

/// (AES key, ntfy topic) of a room. Slow on purpose (~0.1–0.3 s).
pub fn derive(code: &str) -> ([u8; 32], String) {
    let c = normalize(code).unwrap_or_default();
    let mut out = [0u8; 64];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(c.as_bytes(), b"waku-party-v1", ROUNDS, &mut out);
    let mut key = [0u8; 32];
    key.copy_from_slice(&out[..32]);
    let topic: String = out[32..52].iter().map(|b| format!("{b:02x}")).collect();
    (key, format!("waku-{topic}"))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Envelope {
    pub from: u64,
    pub seq: u64,
    /// unix ms when sent
    pub at: u64,
    pub w: Wire,
}

pub fn seal(key: &[u8; 32], e: &Envelope) -> String {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let nonce: [u8; 12] = random();
    let plain = serde_json::to_vec(e).unwrap_or_default();
    let ct = cipher.encrypt(Nonce::from_slice(&nonce), plain.as_ref()).expect("encrypt");
    let mut all = nonce.to_vec();
    all.extend_from_slice(&ct);
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(all)
}

pub fn open(key: &[u8; 32], text: &str) -> Option<Envelope> {
    let all = base64::engine::general_purpose::STANDARD_NO_PAD.decode(text.trim().trim_end_matches('=')).ok()?;
    if all.len() < 12 + 16 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let plain = cipher.decrypt(Nonce::from_slice(&all[..12]), &all[12..]).ok()?;
    serde_json::from_slice(&plain).ok()
}

/// Accepts each sender's messages once, in order, and only fresh ones (±10 minutes).
#[derive(Default)]
pub struct ReplayGuard {
    last: HashMap<u64, u64>,
}

impl ReplayGuard {
    pub fn accept(&mut self, e: &Envelope, now: u64) -> bool {
        if e.at + 600_000 < now || e.at > now + 600_000 {
            return false;
        }
        let l = self.last.entry(e.from).or_insert(0);
        if e.seq <= *l {
            return false;
        }
        *l = e.seq;
        true
    }
}

/// Keep what must arrive, merge the rest: of several position updates only the newest goes out, and a new
/// episode makes earlier position updates moot.
pub fn coalesce(q: Vec<Wire>) -> Vec<Wire> {
    let last_load = q.iter().rposition(|w| matches!(w, Wire::Load { .. }));
    let last_state = q.iter().rposition(|w| matches!(w, Wire::State { .. }));
    q.into_iter()
        .enumerate()
        .filter(|(i, w)| match w {
            Wire::Load { .. } => Some(*i) == last_load,
            Wire::State { .. } => Some(*i) == last_state && last_load.is_none_or(|l| *i > l),
            _ => true,
        })
        .map(|(_, w)| w)
        .collect()
}

pub struct Relay {
    pub code: String,
    out: Sender<Wire>,
    stop: Arc<AtomicBool>,
}

impl Relay {
    pub fn send(&self, w: Wire) {
        let _ = self.out.send(w);
    }

    pub fn close(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn client(read_timeout: Option<Duration>) -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(read_timeout)
        .user_agent(concat!("waku/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

/// Open a room (`code`) on `server`: messages for this app come back as `Msg::Party`.
pub fn start(server: &str, code: &str, tx: Sender<Msg>, ctx: egui::Context) -> Relay {
    let (key, topic) = derive(code);
    let me = u64::from_le_bytes(random());
    let stop = Arc::new(AtomicBool::new(false));
    let server = server.trim_end_matches('/').to_string();
    let url = format!("{server}/{topic}");
    let (out_tx, out_rx) = channel::<Wire>();
    let seq = Arc::new(AtomicU64::new(0));
    let errors: Arc<Mutex<Option<Instant>>> = Arc::default();

    // receive: a long-lived JSON stream; ntfy sends a keepalive every ~45 s, so 100 s of silence = dead link
    {
        let (url, stop, tx, ctx) = (url.clone(), stop.clone(), tx.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("party-recv".into())
            .spawn(move || {
                let c = client(Some(Duration::from_secs(100)));
                let mut guard = ReplayGuard::default();
                let mut since: Option<String> = None;
                let mut fails = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let u = match &since {
                        Some(id) => format!("{url}/json?since={id}"),
                        None => format!("{url}/json"),
                    };
                    match c.get(&u).send() {
                        Ok(r) if r.status().is_success() => {
                            fails = 0;
                            for line in std::io::BufReader::new(r).lines() {
                                if stop.load(Ordering::Relaxed) {
                                    return;
                                }
                                let Ok(line) = line else { break };
                                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                                if v["event"] != "message" {
                                    continue;
                                }
                                if let Some(id) = v["id"].as_str() {
                                    since = Some(id.to_string());
                                }
                                let Some(e) = v["message"].as_str().and_then(|m| open(&key, m)) else { continue };
                                if e.from == me || !guard.accept(&e, now_ms()) {
                                    continue;
                                }
                                let _ = tx.send(Msg::Party(e.w));
                                ctx.request_repaint();
                            }
                        }
                        Ok(r) => {
                            fails += 1;
                            waku_core::http::note(&format!("party: subscribe HTTP {}", r.status()));
                        }
                        Err(e) => {
                            fails += 1;
                            waku_core::http::note(&format!("party: subscribe {e}"));
                        }
                    }
                    if fails == 3 {
                        let _ = tx.send(Msg::Party(Wire::Chat {
                            name: String::new(),
                            text: crate::i18n::tr("Connection to the room keeps dropping — retrying…", "การเชื่อมต่อห้องหลุดบ่อย — กำลังต่อใหม่…").into(),
                        }));
                        ctx.request_repaint();
                    }
                    std::thread::sleep(Duration::from_secs((1 << fails.min(4)).min(15)));
                }
            })
            .ok();
    }

    // send: one at a time, a second apart, bursts merged
    {
        let (stop, seq, errors) = (stop.clone(), seq.clone(), errors.clone());
        std::thread::Builder::new()
            .name("party-send".into())
            .spawn(move || send_loop(out_rx, url, key, me, seq, stop, errors, tx, ctx))
            .ok();
    }
    Relay { code: pretty(code), out: out_tx, stop }
}

#[allow(clippy::too_many_arguments)]
fn send_loop(rx: Receiver<Wire>, url: String, key: [u8; 32], me: u64, seq: Arc<AtomicU64>, stop: Arc<AtomicBool>, errors: Arc<Mutex<Option<Instant>>>, tx: Sender<Msg>, ctx: egui::Context) {
    let c = client(Some(Duration::from_secs(15)));
    let mut last_send = Instant::now() - Duration::from_secs(5);
    loop {
        let Ok(first) = rx.recv() else { return };
        if stop.load(Ordering::Relaxed) && !matches!(first, Wire::Bye { .. }) {
            return;
        }
        // wait out the one-second spacing, collecting whatever else arrives meanwhile
        let gap = Duration::from_millis(1000).saturating_sub(last_send.elapsed());
        std::thread::sleep(gap);
        let mut batch = vec![first];
        while let Ok(w) = rx.try_recv() {
            batch.push(w);
        }
        for w in coalesce(batch) {
            let bye = matches!(w, Wire::Bye { .. });
            let e = Envelope { from: me, seq: seq.fetch_add(1, Ordering::Relaxed) + 1, at: now_ms(), w };
            let body = seal(&key, &e);
            let mut tries = 0;
            loop {
                tries += 1;
                let r = c.post(&url).body(body.clone()).send();
                last_send = Instant::now();
                match r {
                    Ok(r) if r.status().is_success() => break,
                    Ok(r) if r.status().as_u16() == 429 && tries < 3 => {
                        // over ntfy.sh's limit: tell once a minute, wait, try again
                        let mut g = errors.lock().unwrap();
                        if g.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
                            *g = Some(Instant::now());
                            let _ = tx.send(Msg::Party(Wire::Chat {
                                name: String::new(),
                                text: crate::i18n::tr(
                                    "ntfy.sh is limiting messages from this connection (about 250 a day) — retrying",
                                    "ntfy.sh จำกัดจำนวนข้อความจากเน็ตนี้ (ประมาณ 250 ข้อความ/วัน) — กำลังลองใหม่",
                                )
                                .into(),
                            }));
                            ctx.request_repaint();
                        }
                        drop(g);
                        std::thread::sleep(Duration::from_secs(6));
                    }
                    Ok(r) => {
                        waku_core::http::note(&format!("party: publish HTTP {}", r.status()));
                        break;
                    }
                    Err(e) if tries < 3 => {
                        waku_core::http::note(&format!("party: publish {e}"));
                        std::thread::sleep(Duration::from_secs(2));
                    }
                    Err(_) => break,
                }
            }
            if bye {
                return;
            }
            std::thread::sleep(Duration::from_millis(1000));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        let c = new_code();
        assert_eq!(c.len(), 9);
        assert!(normalize(&c).is_some());
        assert_eq!(normalize("k7f3 9qmx").as_deref(), Some("K7F39QMX"));
        assert_eq!(normalize("K7F3-9QMO").as_deref(), Some("K7F39QM0")); // O read as 0
        assert_eq!(normalize("WAKU-K7F3-9QMX").as_deref(), Some("K7F39QMX"));
        assert!(normalize("K7F3-9QM").is_none());
        assert!(normalize("K7F3-9QM!").is_none());
        assert_eq!(pretty("k7f39qmx"), "K7F3-9QMX");
    }

    #[test]
    fn crypto_roundtrip_and_tamper() {
        let (k, topic) = derive("K7F3-9QMX");
        let (k2, topic2) = derive("k7f3 9qmx");
        assert_eq!((k, &topic), (k2, &topic2), "same room however it is typed");
        assert!(topic.starts_with("waku-") && topic.len() == 45);
        let (other, other_topic) = derive("K7F3-9QMY");
        assert_ne!(topic, other_topic);
        let e = Envelope { from: 7, seq: 1, at: 5, w: Wire::Chat { name: "a".into(), text: "สวัสดี".into() } };
        let s = seal(&k, &e);
        assert!(!s.contains("สวัสดี"));
        let back = open(&k, &s).unwrap();
        assert!(matches!(back.w, Wire::Chat { ref text, .. } if text == "สวัสดี"));
        assert!(open(&other, &s).is_none(), "another room's key cannot read it");
        // flip one character: authentication fails
        let mut t: Vec<char> = s.chars().collect();
        let i = t.len() / 2;
        t[i] = if t[i] == 'A' { 'B' } else { 'A' };
        assert!(open(&k, &t.into_iter().collect::<String>()).is_none());
        // two seals of the same message differ (fresh nonce)
        assert_ne!(seal(&k, &e), s);
    }

    #[test]
    fn replay_guard() {
        let mut g = ReplayGuard::default();
        let now = 1_000_000_000;
        let e = |seq, at| Envelope { from: 1, seq, at, w: Wire::Bye { name: String::new() } };
        assert!(g.accept(&e(1, now), now));
        assert!(!g.accept(&e(1, now), now), "same message twice");
        assert!(!g.accept(&e(0, now), now), "older");
        assert!(g.accept(&e(2, now), now));
        assert!(!g.accept(&e(3, now - 3_600_000), now), "an hour old");
    }

    #[test]
    fn merging() {
        let st = |p: f64| Wire::State { ep_url: "e".into(), pos: p, paused: false };
        let chat = Wire::Chat { name: "a".into(), text: "x".into() };
        let out = coalesce(vec![st(1.0), chat.clone(), st(2.0), st(3.0)]);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[1], Wire::State { pos, .. } if pos == 3.0));
        // a load after the positions makes them moot
        let ld = Wire::Load { item: crate::party::tests::item(), pos: 0.0, paused: false };
        let out = coalesce(vec![st(1.0), ld, chat]);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], Wire::Load { .. }));
    }

    /// Two members through a fake ntfy server (publish = POST, subscribe = JSON stream).
    #[test]
    fn through_fake_ntfy() {
        let srv = fake_ntfy::start();
        let ctx = egui::Context::default();
        let (atx, arx) = channel();
        let (btx, brx) = channel();
        let a = start(&srv, "K7F3-9QMX", atx, ctx.clone());
        let b = start(&srv, "k7f3 9qmx", btx, ctx.clone());
        let c_ = start(&srv, "AAAA-BBBB", channel().0, ctx);
        std::thread::sleep(Duration::from_millis(400)); // subscriptions up
        a.send(Wire::Chat { name: "A".into(), text: "hi".into() });
        c_.send(Wire::Chat { name: "C".into(), text: "other room".into() });
        let got = brx.recv_timeout(Duration::from_secs(5)).expect("b hears a");
        assert!(matches!(got, Msg::Party(Wire::Chat { ref text, .. }) if text == "hi"));
        // a does not hear itself, and nobody hears the other room
        assert!(arx.recv_timeout(Duration::from_millis(1500)).is_err());
        assert!(brx.recv_timeout(Duration::from_millis(1500)).is_err());
        // the server only ever saw ciphertext
        assert!(!fake_ntfy::seen().contains("hi\""));
        a.close();
        b.close();
    }

    /// A tiny stand-in for ntfy: POST /topic stores, GET /topic/json streams new messages of that topic.
    mod fake_ntfy {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::sync::{Arc, Mutex};

        type Subs = Arc<Mutex<Vec<(String, TcpStream)>>>;
        static SEEN: Mutex<String> = Mutex::new(String::new());

        pub fn seen() -> String {
            SEEN.lock().unwrap().clone()
        }

        pub fn start() -> String {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            let subs: Subs = Arc::default();
            std::thread::spawn(move || {
                let mut n = 0;
                for s in l.incoming().flatten() {
                    let subs = subs.clone();
                    n += 1;
                    let id = n;
                    std::thread::spawn(move || handle(s, subs, id));
                }
            });
            format!("http://127.0.0.1:{port}")
        }

        fn handle(s: TcpStream, subs: Subs, id: u32) {
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut first = String::new();
            r.read_line(&mut first).unwrap();
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                if h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let parts: Vec<&str> = first.split_whitespace().collect();
            let path = parts[1].split('?').next().unwrap().to_string();
            let mut out = s;
            if parts[0] == "POST" {
                let mut body = vec![0u8; len];
                r.read_exact(&mut body).unwrap();
                let msg = String::from_utf8(body).unwrap();
                SEEN.lock().unwrap().push_str(&msg);
                let topic = path.trim_start_matches('/').to_string();
                let line = serde_json::json!({"id": format!("m{id}"), "event": "message", "topic": topic, "message": msg}).to_string() + "\n";
                for (t, sub) in subs.lock().unwrap().iter_mut() {
                    if *t == topic {
                        let _ = write!(sub, "{:x}\r\n{}\r\n", line.len(), line);
                        let _ = sub.flush();
                    }
                }
                let _ = out.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
            } else {
                let topic = path.trim_start_matches('/').trim_end_matches("/json").to_string();
                let _ = out.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\n\r\n");
                let open = serde_json::json!({"id": "o", "event": "open", "topic": topic}).to_string() + "\n";
                let _ = write!(out, "{:x}\r\n{}\r\n", open.len(), open);
                subs.lock().unwrap().push((topic, out));
            }
        }
    }
}
