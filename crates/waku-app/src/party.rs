//! Watch party: watch the same episode together, in sync.
//!
//! One person hosts (a TCP port, 47800 by default), the others join by its address. Only small control messages
//! travel — what is playing, play / pause, the position, chat — every player finds and streams the video
//! itself. Anyone may pause, seek or change the episode; the host passes every message on to the others and
//! sends its position every 3 s so everyone stays within ~2 s. Lines of JSON over TCP.
//!
//! Over the internet the host's port must be reachable: same LAN, a VPN such as Tailscale / ZeroTier /
//! Radmin VPN, or a forwarded port.

use crate::app::{Msg, WakuApp};
use crate::i18n::tr;
use crate::theme::*;
use crate::widgets;
use egui::{Align2, CornerRadius, Frame, Margin, RichText, vec2};
use egui_phosphor::regular as icon;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use waku_core::{AnimeDetail, Episode};

pub const PORT: u16 = 47800;

/// What is playing, enough for another Waku to open the same episode.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Item {
    pub source: String,
    pub series_url: String,
    pub title: String,
    pub poster: Option<String>,
    pub ep: Episode,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Wire {
    Hello { name: String },
    Members { names: Vec<String> },
    /// `at`: when `pos` was read, in the room's clock (the host's; ms since 1970) — 0 from older versions
    Load {
        item: Item,
        pos: f64,
        paused: bool,
        #[serde(default)]
        at: u64,
    },
    State {
        ep_url: String,
        pos: f64,
        paused: bool,
        #[serde(default)]
        at: u64,
    },
    /// clock probe from a guest (`id` = theirs); the host answers with `Pong` (NTP style: received / sent)
    Ping { id: u64, t0: u64 },
    Pong { id: u64, t0: u64, hr: u64, hs: u64 },
    Chat { name: String, text: String },
    /// someone left (internet rooms: there is no connection to see dropping)
    Bye { name: String },
}

enum Role {
    Host { clients: Arc<Mutex<Vec<(u32, String, TcpStream)>>>, stop: Arc<AtomicBool>, current: Arc<Mutex<Option<Wire>>> },
    Guest { stream: TcpStream },
    /// through ntfy.sh, end-to-end encrypted (`relay.rs`); `host` = created the room (answers newcomers)
    Relay { relay: crate::relay::Relay, host: bool, me: String },
}

pub struct Party {
    role: Role,
    pub address: String,
    pub members: Vec<String>,
    pub chat: Vec<(String, String)>,
    pub draft: String,
    /// last thing seen of the local player: (ep url, position, paused, when)
    last: Option<(String, f64, bool, Instant)>,
    /// after following a remote command, local changes are not echoed back for a moment
    quiet_until: Instant,
    last_beat: Instant,
    /// the local player was opening a file on the last frame
    loading: bool,
    pub collapsed: bool,
    /// keeping in step with the room
    sync: Sync,
}

/// What everyone should be seeing: the last position someone in the room told (or this app told), and when.
#[derive(Clone, Debug)]
struct Reference {
    ep_url: String,
    pos: f64,
    /// room clock (ms)
    at: u64,
    paused: bool,
    /// it came from here: this app is what the others follow
    mine: bool,
}

/// Clock and position keeping. The room's clock is the host's: a guest measures how far its own clock is from
/// it with a few pings, so "position P at time T" from anyone means the same moment everywhere, and a guest
/// knows where the room is *now* (P + time since T) instead of where it was when the message left.
struct Sync {
    /// add to the local clock to get the room's (ms)
    offset: i64,
    /// round trip of the probe `offset` came from (the shortest one wins)
    best_rtt: u64,
    id: u64,
    pings: u32,
    next_ping: Instant,
    reference: Option<Reference>,
    /// last correction (seek / speed change) — a seek needs a moment to land
    last_fix: Instant,
    last_seek: Instant,
}

impl Default for Sync {
    fn default() -> Self {
        let mut b = [0u8; 8];
        let _ = getrandom::getrandom(&mut b);
        Sync {
            offset: 0,
            best_rtt: u64::MAX,
            id: u64::from_le_bytes(b),
            pings: 0,
            next_ping: Instant::now() + Duration::from_millis(600),
            reference: None,
            last_fix: Instant::now(),
            last_seek: Instant::now() - Duration::from_secs(10),
        }
    }
}

impl Sync {
    fn room_now(&self) -> u64 {
        (crate::relay::now_ms() as i64 + self.offset).max(0) as u64
    }

    /// Where the room is now, by the reference.
    fn expected(&self) -> Option<(String, f64, bool)> {
        let r = self.reference.as_ref()?;
        if r.paused {
            return Some((r.ep_url.clone(), r.pos, true));
        }
        let gone = self.room_now().saturating_sub(r.at).min(6 * 3600 * 1000) as f64 / 1000.0;
        Some((r.ep_url.clone(), r.pos + gone, false))
    }

    /// A clock probe came back: NTP's offset = ((hr - t0) + (hs - t1)) / 2, kept when the round trip is the
    /// shortest yet (the least room for an uneven trip).
    fn on_pong(&mut self, t0: u64, hr: u64, hs: u64, t1: u64) {
        let rtt = (t1.saturating_sub(t0)).saturating_sub(hs.saturating_sub(hr));
        if rtt <= self.best_rtt {
            self.best_rtt = rtt;
            self.offset = ((hr as i64 - t0 as i64) + (hs as i64 - t1 as i64)) / 2;
        }
    }
}

#[derive(Default)]
pub struct PartyUi {
    pub open: bool,
    pub join_addr: String,
    pub join_code: String,
    pub error: Option<String>,
    /// the LAN (IP address) way instead of a room code
    pub lan: bool,
}

fn send_line(s: &mut TcpStream, w: &Wire) -> bool {
    match serde_json::to_string(w) {
        Ok(mut t) => {
            t.push('\n');
            s.write_all(t.as_bytes()).is_ok()
        }
        Err(_) => false,
    }
}

/// This PC's address on the network (no packet is sent).
pub fn local_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

fn names(clients: &[(u32, String, TcpStream)], host: &str) -> Vec<String> {
    std::iter::once(format!("{host} ★")).chain(clients.iter().filter(|c| !c.1.is_empty()).map(|c| c.1.clone())).collect()
}

impl Party {
    fn send(&mut self, w: &Wire) {
        match &mut self.role {
            Role::Host { clients, .. } => {
                let mut c = clients.lock().unwrap();
                c.retain_mut(|(_, _, s)| send_line(s, w));
            }
            Role::Guest { stream } => {
                send_line(stream, w);
            }
            Role::Relay { relay, .. } => relay.send(w.clone()),
        }
    }

    pub fn is_host(&self) -> bool {
        matches!(self.role, Role::Host { .. } | Role::Relay { host: true, .. })
    }

    /// An internet room (code) rather than a LAN one (address).
    pub fn is_relay(&self) -> bool {
        matches!(self.role, Role::Relay { .. })
    }

    fn leave(&mut self) {
        match &mut self.role {
            Role::Host { clients, stop, .. } => {
                stop.store(true, Ordering::Relaxed);
                for (_, _, s) in clients.lock().unwrap().iter() {
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
                // wake the accept loop
                let _ = TcpStream::connect_timeout(&format!("127.0.0.1:{PORT}").parse().unwrap(), Duration::from_millis(300));
            }
            Role::Guest { stream } => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            Role::Relay { relay, me, .. } => {
                relay.send(Wire::Bye { name: me.clone() });
                relay.close();
            }
        }
    }
}

/// An internet room: `code` = None creates one.
fn relay_party(server: &str, code: Option<&str>, name: String, tx: std::sync::mpsc::Sender<Msg>, ctx: egui::Context) -> Result<Party, String> {
    let host = code.is_none();
    let code = match code {
        Some(c) => crate::relay::normalize(c).ok_or_else(|| tr("A room code has 8 letters / digits, like K7F3-9QMX", "รหัสห้องมี 8 ตัว เช่น K7F3-9QMX").to_string())?,
        None => crate::relay::new_code(),
    };
    let relay = crate::relay::start(server, &code, tx, ctx);
    let address = relay.code.clone();
    if !host {
        relay.send(Wire::Hello { name: name.clone() });
    }
    Ok(Party {
        role: Role::Relay { relay, host, me: name.clone() },
        address,
        members: if host { vec![format!("{name} ★")] } else { Vec::new() },
        chat: Vec::new(),
        draft: String::new(),
        last: None,
        quiet_until: Instant::now(),
        last_beat: Instant::now(),
        loading: false,
        collapsed: false,
        sync: Sync::default(),
    })
}

/// Host: accept guests, pass their messages to the app and to the other guests.
fn host(port: u16, name: String, tx: std::sync::mpsc::Sender<Msg>, ctx: egui::Context) -> std::io::Result<Party> {
    let l = TcpListener::bind(("0.0.0.0", port))?;
    let clients: Arc<Mutex<Vec<(u32, String, TcpStream)>>> = Arc::default();
    let stop = Arc::new(AtomicBool::new(false));
    let current: Arc<Mutex<Option<Wire>>> = Arc::default();
    let next_id = Arc::new(AtomicU32::new(1));
    {
        let (clients, stop, current, tx, ctx, name) = (clients.clone(), stop.clone(), current.clone(), tx.clone(), ctx.clone(), name.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(s) = s else { continue };
                let _ = s.set_nodelay(true);
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let Ok(w) = s.try_clone() else { continue };
                clients.lock().unwrap().push((id, String::new(), w));
                let (clients, stop, current, tx, ctx, name) = (clients.clone(), stop.clone(), current.clone(), tx.clone(), ctx.clone(), name.clone());
                std::thread::spawn(move || {
                    let reader = BufReader::new(s);
                    for line in reader.lines() {
                        let Ok(line) = line else { break };
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        let Ok(w) = serde_json::from_str::<Wire>(&line) else { continue };
                        if let Wire::Hello { name: who } = &w {
                            let mut c = clients.lock().unwrap();
                            if let Some(x) = c.iter_mut().find(|x| x.0 == id) {
                                x.1 = who.chars().take(32).collect();
                            }
                            let list = Wire::Members { names: names(&c, &name) };
                            let now = current.lock().unwrap().clone();
                            for x in c.iter_mut() {
                                send_line(&mut x.2, &list);
                            }
                            // the newcomer starts where everyone is
                            if let Some(now) = now
                                && let Some(x) = c.iter_mut().find(|x| x.0 == id)
                            {
                                send_line(&mut x.2, &now);
                            }
                            drop(c);
                            let _ = tx.send(Msg::Party(list));
                            let _ = tx.send(Msg::Party(Wire::Chat { name: String::new(), text: format!("{who} {}", tr("joined", "เข้าร่วมแล้ว")) }));
                            ctx.request_repaint();
                            continue;
                        }
                        // pass it on to the other guests, and to this app
                        {
                            let mut c = clients.lock().unwrap();
                            for x in c.iter_mut().filter(|x| x.0 != id) {
                                send_line(&mut x.2, &w);
                            }
                        }
                        let _ = tx.send(Msg::Party(w));
                        ctx.request_repaint();
                    }
                    // gone
                    let mut c = clients.lock().unwrap();
                    let who = c.iter().find(|x| x.0 == id).map(|x| x.1.clone()).unwrap_or_default();
                    c.retain(|x| x.0 != id);
                    let list = Wire::Members { names: names(&c, &name) };
                    for x in c.iter_mut() {
                        send_line(&mut x.2, &list);
                    }
                    drop(c);
                    if !stop.load(Ordering::Relaxed) {
                        let _ = tx.send(Msg::Party(list));
                        if !who.is_empty() {
                            let _ = tx.send(Msg::Party(Wire::Chat { name: String::new(), text: format!("{who} {}", tr("left", "ออกไปแล้ว")) }));
                        }
                        ctx.request_repaint();
                    }
                });
            }
        });
    }
    Ok(Party {
        role: Role::Host { clients, stop, current },
        address: format!("{}:{port}", local_ip()),
        members: vec![format!("{name} ★")],
        chat: Vec::new(),
        draft: String::new(),
        last: None,
        quiet_until: Instant::now(),
        last_beat: Instant::now(),
        loading: false,
        collapsed: false,
        sync: Sync::default(),
    })
}

/// Guest: connect to a host.
fn join(addr: &str, name: String, tx: std::sync::mpsc::Sender<Msg>, ctx: egui::Context) -> Result<Party, String> {
    let addr = addr.trim();
    let addr = if addr.contains(':') { addr.to_string() } else { format!("{addr}:{PORT}") };
    use std::net::ToSocketAddrs;
    let sa = addr.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or_else(|| tr("Unknown address", "ไม่รู้จักที่อยู่นี้").to_string())?;
    let s = TcpStream::connect_timeout(&sa, Duration::from_secs(6)).map_err(|e| format!("{}: {e}", tr("Could not reach the host", "ต่อไปหาผู้จัดไม่ได้")))?;
    let _ = s.set_nodelay(true);
    let mut w = s.try_clone().map_err(|e| e.to_string())?;
    send_line(&mut w, &Wire::Hello { name });
    std::thread::spawn(move || {
        for line in BufReader::new(s).lines() {
            let Ok(line) = line else { break };
            if let Ok(w) = serde_json::from_str::<Wire>(&line) {
                let _ = tx.send(Msg::Party(w));
                ctx.request_repaint();
            }
        }
        let _ = tx.send(Msg::PartyClosed);
        ctx.request_repaint();
    });
    Ok(Party {
        role: Role::Guest { stream: w },
        address: addr,
        members: Vec::new(),
        chat: Vec::new(),
        draft: String::new(),
        last: None,
        quiet_until: Instant::now(),
        last_beat: Instant::now(),
        loading: false,
        collapsed: false,
        sync: Sync::default(),
    })
}

impl WakuApp {
    fn party_name(&self) -> String {
        let n = self.settings.party_name.trim();
        if n.is_empty() { tr("Viewer", "ผู้ชม").into() } else { n.chars().take(32).collect() }
    }

    /// What the local player shows now, as the room sees it: (item, position, paused, playing a file yet).
    fn party_now(&self) -> Option<(Item, f64, bool, bool)> {
        let v = self.view.as_ref()?;
        if v.detail.source == crate::dlpage::LOCAL {
            return None; // a file on this PC: the others cannot open it
        }
        let ep = v.eps.get(v.view.index())?.clone();
        let st = v.view.state();
        let item = Item { source: v.detail.source.clone(), series_url: v.detail.url.clone(), title: v.detail.title.clone(), poster: v.detail.poster.clone(), ep };
        Some((item, v.view.position(), st.paused, st.loaded && st.error.is_none()))
    }

    /// Every frame: tell the room about what the viewer did here; the host's heartbeat; a guest's clock probes;
    /// keeping up with the room.
    pub fn party_tick(&mut self) {
        let now_state = self.party_now();
        let Some(p) = self.party.as_mut() else { return };
        // a guest measures the room's clock: 3 probes when joining, then now and then (internet rooms have
        // ntfy.sh's daily limit: every 20 min there, every minute on a LAN)
        if !p.is_host() && Instant::now() >= p.sync.next_ping {
            p.sync.pings += 1;
            let gap = match p.sync.pings {
                1 => 2,
                2 => 5,
                _ if p.is_relay() => 1200,
                _ => 60,
            };
            p.sync.next_ping = Instant::now() + Duration::from_secs(gap);
            let w = Wire::Ping { id: p.sync.id, t0: crate::relay::now_ms() };
            p.send(&w);
        }
        let Some((item, pos, paused, loaded)) = now_state else { return };
        if !loaded {
            // finding / opening the link: neither "paused" nor a position yet — say nothing
            p.loading = true;
            return;
        }
        let at = p.sync.room_now();
        if let Role::Host { current, .. } = &p.role {
            *current.lock().unwrap() = Some(Wire::Load { item: item.clone(), pos, paused, at });
        }
        let quiet = Instant::now() < p.quiet_until;
        let mut out: Option<Wire> = None;
        if std::mem::take(&mut p.loading) || p.last.is_none() {
            // just started playing: announce it only when it is an episode the room is not on
            let other = p.last.as_ref().is_none_or(|l| l.0 != item.ep.url);
            if other && !quiet && (p.last.is_some() || p.is_host()) {
                out = Some(Wire::Load { item: item.clone(), pos, paused, at });
            }
        } else if let Some((ep_url, last_pos, last_paused, when)) = &p.last
            && !quiet
        {
            if *ep_url != item.ep.url {
                out = Some(Wire::Load { item: item.clone(), pos, paused, at });
            } else if *last_paused != paused {
                out = Some(Wire::State { ep_url: item.ep.url.clone(), pos, paused, at });
            } else {
                let expect = if paused { *last_pos } else { last_pos + when.elapsed().as_secs_f64() * 1.1 };
                let low = if paused { *last_pos } else { last_pos + when.elapsed().as_secs_f64() * 0.9 };
                if pos > expect + 3.0 || pos < low - 3.0 {
                    out = Some(Wire::State { ep_url: item.ep.url.clone(), pos, paused, at }); // a seek here
                }
            }
        }
        // the host's position check: every 3 s on a LAN, every 2 minutes through ntfy.sh (its daily limit)
        let beat = if p.is_relay() { Duration::from_secs(120) } else { Duration::from_secs(3) };
        if out.is_none() && p.is_host() && p.last_beat.elapsed() > beat {
            out = Some(Wire::State { ep_url: item.ep.url.clone(), pos, paused, at });
        }
        if let Some(w) = out {
            if matches!(w, Wire::State { .. }) {
                p.last_beat = Instant::now();
            }
            // what this app says is what the room follows now
            p.sync.reference = Some(Reference { ep_url: item.ep.url.clone(), pos, at, paused, mine: true });
            p.send(&w);
        }
        p.last = Some((item.ep.url.clone(), pos, paused, Instant::now()));
        let catch_up = self.party_catch_up(&item.ep.url, pos, paused);
        self.ctx.request_repaint_after(Duration::from_millis(if catch_up { 100 } else { 500 }));
    }

    /// Keep this player on the room's moment: a little faster / slower for small gaps (no stutter), a seek
    /// for big ones. Returns true while catching up.
    fn party_catch_up(&mut self, ep_url: &str, pos: f64, paused: bool) -> bool {
        let Some(p) = self.party.as_mut() else { return false };
        let Some(v) = self.view.as_mut() else { return false };
        let following = p.sync.reference.as_ref().is_some_and(|r| !r.mine && r.ep_url == ep_url);
        let Some((_, want, want_paused)) = p.sync.expected().filter(|_| following) else {
            v.view.set_sync_rate(1.0);
            return false;
        };
        if p.sync.last_fix.elapsed() < Duration::from_millis(200) || p.sync.last_seek.elapsed() < Duration::from_millis(2500) {
            return true;
        }
        p.sync.last_fix = Instant::now();
        if want_paused != paused {
            v.view.set_paused(want_paused);
        }
        let diff = want - pos;
        if want_paused {
            v.view.set_sync_rate(1.0);
            if diff.abs() > 0.3 {
                v.view.seek_to(want);
                p.sync.last_seek = Instant::now();
                p.quiet_until = Instant::now() + Duration::from_millis(1500);
                p.last = Some((ep_url.to_string(), want, true, Instant::now()));
            }
            return false;
        }
        if diff.abs() > 2.0 {
            // far off (just opened the episode, a stall): jump, a little ahead for the time a seek takes
            v.view.set_sync_rate(1.0);
            let to = want + 0.5;
            v.view.seek_to(to);
            p.sync.last_seek = Instant::now();
            p.quiet_until = Instant::now() + Duration::from_millis(1500);
            p.last = Some((ep_url.to_string(), to, false, Instant::now()));
            true
        } else if diff.abs() > 0.08 {
            // up to ±10 %: half a second is made up in about 5 s, without a sound glitch
            v.view.set_sync_rate(1.0 + (diff * 0.5).clamp(-0.1, 0.1));
            true
        } else {
            v.view.set_sync_rate(1.0);
            false
        }
    }

    /// How far this player is from the room (seconds; + = behind), for the panel.
    fn party_gap(&self) -> Option<f64> {
        let p = self.party.as_ref()?;
        let r = p.sync.reference.as_ref().filter(|r| !r.mine)?;
        let v = self.view.as_ref()?;
        if v.eps.get(v.view.index()).is_none_or(|e| e.url != r.ep_url) || r.paused {
            return None;
        }
        let (_, want, _) = p.sync.expected()?;
        Some(want - v.view.position())
    }

    /// A message from the room.
    pub fn on_party(&mut self, w: Wire) {
        let Some(p) = self.party.as_mut() else { return };
        match w {
            Wire::Members { names } => p.members = names,
            Wire::Chat { name, text } => {
                let line = if name.is_empty() { text.clone() } else { format!("{name}: {text}") };
                p.chat.push((name, text));
                if p.chat.len() > 200 {
                    p.chat.remove(0);
                }
                if let Some(v) = self.view.as_mut() {
                    v.view.notice(line);
                }
            }
            Wire::Ping { id, t0 } => {
                if p.is_host() {
                    let now = crate::relay::now_ms();
                    p.send(&Wire::Pong { id, t0, hr: now, hs: now });
                }
            }
            Wire::Pong { id, t0, hr, hs } => {
                if id == p.sync.id {
                    p.sync.on_pong(t0, hr, hs, crate::relay::now_ms());
                }
            }
            Wire::State { ep_url, pos, paused, at } => {
                p.quiet_until = Instant::now() + Duration::from_millis(1500);
                // (an older version sends no time: count ~0.3 s of travel)
                let at = if at > 0 { at.min(p.sync.room_now()) } else { p.sync.room_now().saturating_sub(300) };
                p.sync.reference = Some(Reference { ep_url: ep_url.clone(), pos, at, paused, mine: false });
                let (_, target, _) = p.sync.expected().unwrap_or((ep_url.clone(), pos, paused));
                if let Some(v) = self.view.as_mut()
                    && v.eps.get(v.view.index()).is_some_and(|e| e.url == ep_url)
                {
                    v.view.set_paused(paused);
                    // a big jump at once; small gaps are closed gently by `party_catch_up`
                    if (v.view.position() - target).abs() > 2.0 {
                        let to = if paused { target } else { target + 0.5 };
                        v.view.seek_to(to);
                        p.sync.last_seek = Instant::now();
                    }
                    p.last = Some((ep_url, target, paused, Instant::now()));
                }
            }
            Wire::Load { item, pos, paused, at } => {
                p.quiet_until = Instant::now() + Duration::from_millis(4000);
                let at = if at > 0 { at.min(p.sync.room_now()) } else { p.sync.room_now().saturating_sub(300) };
                p.sync.reference = Some(Reference { ep_url: item.ep.url.clone(), pos, at, paused, mine: false });
                // open it where the room is by now (and catch up once it plays)
                let pos = p.sync.expected().map_or(pos, |e| e.1);
                p.last = Some((item.ep.url.clone(), pos, paused, Instant::now()));
                // same series open: just switch the episode
                if let Some(v) = self.view.as_mut()
                    && v.detail.url == item.series_url
                    && let Some(i) = v.eps.iter().position(|e| e.url == item.ep.url)
                {
                    if i != v.view.index() {
                        self.start_once = Some((item.ep.url.clone(), pos));
                        v.view.go_to(i);
                    } else if (v.view.position() - pos).abs() > 2.0 {
                        v.view.seek_to(pos);
                    }
                    return;
                }
                let it = item.clone();
                self.spawn(move |m| match m.detail(&it.source, &it.series_url) {
                    Ok(d) => Msg::PartyOpen { detail: d, ep: it.ep, pos },
                    Err(e) => Msg::Party(Wire::Chat { name: String::new(), text: format!("{} {}: {}", tr("Could not open", "เปิดไม่ได้"), it.title, crate::app::first_line(&format!("{e:#}"))) }),
                });
            }
            Wire::Hello { name } => {
                // internet room: the host keeps the list and tells the newcomer what is on
                if p.is_relay() && p.is_host() {
                    let name: String = name.chars().take(32).collect();
                    if !p.members.contains(&name) {
                        p.members.push(name.clone());
                    }
                    p.chat.push((String::new(), format!("{name} {}", tr("joined", "เข้าร่วมแล้ว"))));
                    let list = Wire::Members { names: p.members.clone() };
                    p.send(&list);
                    let now = self.party_now();
                    if let Some(p) = self.party.as_mut()
                        && let Some((item, pos, paused, _)) = now
                    {
                        let at = p.sync.room_now();
                        p.send(&Wire::Load { item, pos, paused, at });
                    }
                }
            }
            Wire::Bye { name } => {
                p.chat.push((String::new(), format!("{name} {}", tr("left", "ออกไปแล้ว"))));
                if p.is_relay() && p.is_host() {
                    p.members.retain(|m| *m != name);
                    let list = Wire::Members { names: p.members.clone() };
                    p.send(&list);
                }
            }
        }
    }

    pub fn party_open(&mut self, detail: AnimeDetail, ep: Episode, pos: f64) {
        let ep = detail.episodes.iter().find(|e| e.url == ep.url).cloned().unwrap_or(ep);
        self.start_once = Some((ep.url.clone(), pos));
        if let Some(p) = self.party.as_mut() {
            p.quiet_until = Instant::now() + Duration::from_secs(6);
        }
        self.play_builtin(&detail, &ep, None);
    }

    pub fn party_closed(&mut self) {
        if self.party.as_ref().is_some_and(|p| !p.is_host()) {
            self.party = None;
            self.notify(tr("The watch party ended (the host left or the connection dropped)", "ปาร์ตี้จบแล้ว (ผู้จัดออกหรือการเชื่อมต่อหลุด)"));
        }
    }

    /// Host / join dialog, and the floating room panel while in a party.
    pub fn party_ui(&mut self, ctx: &egui::Context) {
        self.party_tick();
        if self.party.is_some() {
            self.party_panel(ctx);
            return;
        }
        if !self.party_ui.open {
            return;
        }
        let mut open = true;
        egui::Window::new(format!("{}  {}", icon::USERS_THREE, tr("Watch party", "ดูด้วยกัน")))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(tr(
                        "Watch the same episode together, in sync: play, pause, seek and episode changes follow everyone, plus chat. Each player streams the video itself.",
                        "ดูตอนเดียวกันพร้อมกัน: เล่น หยุด เลื่อน และเปลี่ยนตอนตามกันทุกคน มีแชท — แต่ละเครื่องโหลดวิดีโอเอง",
                    ))
                    .color(MUTED),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(tr("My name", "ชื่อของฉัน"));
                    if ui.add(egui::TextEdit::singleline(&mut self.settings.party_name).hint_text(tr("Viewer", "ผู้ชม")).desired_width(180.0)).lost_focus() {
                        self.settings.save();
                    }
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if widgets::chip(ui, &format!("{}  {}", icon::GLOBE, tr("Over the internet (room code)", "ผ่านอินเทอร์เน็ต (รหัสห้อง)")), !self.party_ui.lan, None).clicked() {
                        self.party_ui.lan = false;
                        self.party_ui.error = None;
                    }
                    if widgets::chip(ui, &format!("{}  {}", icon::WIFI_HIGH, tr("Same network (IP)", "วงเดียวกัน (IP)")), self.party_ui.lan, None).clicked() {
                        self.party_ui.lan = true;
                        self.party_ui.error = None;
                    }
                });
                ui.add_space(12.0);
                let server = if self.settings.party_server.trim().is_empty() { crate::relay::DEFAULT_SERVER.to_string() } else { self.settings.party_server.trim().to_string() };
                if !self.party_ui.lan {
                    if widgets::play_button(ui, tr("Create a room", "สร้างห้อง")).clicked() {
                        self.settings.save();
                        match relay_party(&server, None, self.party_name(), self.tx.clone(), self.ctx.clone()) {
                            Ok(p) => {
                                self.party = Some(p);
                                self.party_ui.open = false;
                                self.party_ui.error = None;
                            }
                            Err(e) => self.party_ui.error = Some(e),
                        }
                    }
                    ui.add_space(14.0);
                    ui.label(RichText::new(tr("…or join with a friend's room code", "…หรือเข้าห้องด้วยรหัสของเพื่อน")).color(MUTED));
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut self.party_ui.join_code).hint_text("K7F3-9QMX").font(egui::TextStyle::Monospace).desired_width(160.0));
                        let go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (widgets::outline_button(ui, tr("Join", "เข้าร่วม")).clicked() || go) && !self.party_ui.join_code.trim().is_empty() {
                            self.settings.save();
                            match relay_party(&server, Some(&self.party_ui.join_code.clone()), self.party_name(), self.tx.clone(), self.ctx.clone()) {
                                Ok(p) => {
                                    self.party = Some(p);
                                    self.party_ui.open = false;
                                    self.party_ui.error = None;
                                }
                                Err(e) => self.party_ui.error = Some(e),
                            }
                        }
                    });
                } else {
                    if widgets::play_button(ui, tr("Host on this network", "เปิดห้องในวงนี้")).clicked() {
                        self.settings.save();
                        match host(PORT, self.party_name(), self.tx.clone(), self.ctx.clone()) {
                            Ok(p) => {
                                self.party = Some(p);
                                self.party_ui.open = false;
                                self.party_ui.error = None;
                            }
                            Err(e) => self.party_ui.error = Some(format!("{} {PORT}: {e}", tr("Could not open port", "เปิดพอร์ตไม่ได้"))),
                        }
                    }
                    ui.add_space(14.0);
                    ui.label(RichText::new(tr("…or join a friend's party", "…หรือเข้าห้องของเพื่อน")).color(MUTED));
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut self.party_ui.join_addr).hint_text("192.168.1.20:47800").desired_width(220.0));
                        let go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (widgets::outline_button(ui, tr("Join", "เข้าร่วม")).clicked() || go) && !self.party_ui.join_addr.trim().is_empty() {
                            self.settings.save();
                            match join(&self.party_ui.join_addr.clone(), self.party_name(), self.tx.clone(), self.ctx.clone()) {
                                Ok(p) => {
                                    self.party = Some(p);
                                    self.party_ui.open = false;
                                    self.party_ui.error = None;
                                }
                                Err(e) => self.party_ui.error = Some(e),
                            }
                        }
                    });
                }
                if let Some(e) = &self.party_ui.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).color(BAD));
                }
                ui.add_space(10.0);
                let note = if self.party_ui.lan {
                    tr(
                        "Everyone on the same Wi-Fi / LAN (or the same VPN such as Tailscale). Fastest: the position is checked every 3 seconds.",
                        "ทุกคนอยู่ Wi-Fi / LAN เดียวกัน (หรือ VPN เดียวกันอย่าง Tailscale) — เร็วสุด ตรวจตำแหน่งทุก 3 วินาที",
                    )
                } else {
                    tr(
                        "Works from anywhere, no setup. Messages pass through ntfy.sh encrypted with the room code (AES-256): ntfy.sh and anyone without the code see only scrambled text. Give the code only to friends. ntfy.sh allows ~250 messages a day per connection, so only play / pause / seek / episode / chat are sent.",
                        "ใช้ได้จากทุกที่ ไม่ต้องตั้งค่า — ข้อความผ่าน ntfy.sh โดยเข้ารหัสด้วยรหัสห้อง (AES-256): ntfy.sh และคนที่ไม่มีรหัสเห็นแค่ข้อความที่อ่านไม่ออก ให้รหัสเฉพาะเพื่อน — ntfy.sh จำกัดประมาณ 250 ข้อความ/วัน/เน็ต จึงส่งเฉพาะ เล่น/หยุด/เลื่อน/เปลี่ยนตอน/แชท",
                    )
                };
                ui.label(RichText::new(note).size(11.5).color(DIM));
            });
        if !open {
            self.party_ui.open = false;
        }
    }

    fn party_panel(&mut self, ctx: &egui::Context) {
        let mut leave = false;
        let mut send: Option<String> = None;
        let gap = self.party_gap();
        let Some(p) = self.party.as_mut() else { return };
        // faded until the pointer comes over it (or a message is being typed), so it does not cover the video
        let id = egui::Id::new("party-panel");
        let prev: Option<egui::Rect> = ctx.data(|d| d.get_temp(id.with("rect")));
        let over = ctx.pointer_hover_pos().zip(prev).is_some_and(|(pt, r)| r.expand(8.0).contains(pt));
        let typing = ctx.memory(|m| m.has_focus(id.with("draft")));
        let alpha = ctx.animate_bool_with_time(id.with("show"), over || typing || ctx.is_being_dragged(id), 0.2);
        // draggable anywhere (the position is remembered by egui and kept on screen); starts bottom right
        let start = ctx.content_rect().right_bottom() - vec2(16.0 + 324.0, 96.0 + 120.0);
        let area = egui::Area::new(id).default_pos(start).movable(true).constrain(true).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.multiply_opacity(0.35 + 0.65 * alpha);
            Frame::new().fill(BAR.gamma_multiply(0.94)).stroke(egui::Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(300.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {}", icon::USERS_THREE, tr("Watch party", "ดูด้วยกัน"))).font(bold(15.0)).color(ACCENT_HI)).on_hover_text(tr("Drag anywhere to move", "ลากเพื่อย้าย"));
                    ui.label(RichText::new(format!("· {}", p.members.len().max(1))).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::ghost_button(ui, icon::SIGN_OUT).on_hover_text(tr("Leave", "ออกจากห้อง")).clicked() {
                            leave = true;
                        }
                        let g = if p.collapsed { icon::CARET_UP } else { icon::CARET_DOWN };
                        if widgets::ghost_button(ui, g).clicked() {
                            p.collapsed = !p.collapsed;
                        }
                    });
                });
                if p.collapsed {
                    return;
                }
                if p.is_host() || p.is_relay() {
                    ui.horizontal(|ui| {
                        let label = if p.is_relay() { tr("Room code:", "รหัสห้อง:") } else { tr("Room address:", "ที่อยู่ห้อง:") };
                        ui.label(RichText::new(label).size(12.0).color(MUTED));
                        let size = if p.is_relay() { 17.0 } else { 13.0 };
                        let r = ui.add(egui::Label::new(RichText::new(&p.address).monospace().size(size).color(TEXT)).sense(egui::Sense::click())).on_hover_text(tr("Click to copy", "คลิกเพื่อคัดลอก"));
                        if r.clicked() {
                            ui.ctx().copy_text(p.address.clone());
                        }
                    });
                }
                ui.label(RichText::new(p.members.join(", ")).size(12.0).color(DIM));
                // in step with the room?
                if let Some(g) = gap {
                    let (txt, col) = if g.abs() <= 0.15 {
                        (tr("In sync with the room", "ตรงกับห้องแล้ว").to_string(), GOOD)
                    } else {
                        let s = format!("{:.1}", g.abs());
                        let t = if g > 0.0 { crate::trf!("Catching up — {s} s behind", "กำลังตามห้อง — ช้ากว่า {s} วิ") } else { crate::trf!("Easing back — {s} s ahead", "กำลังรอห้อง — เร็วกว่า {s} วิ") };
                        (t, WARN)
                    };
                    ui.label(RichText::new(format!("{}  {txt}", icon::CLOCK_CLOCKWISE)).size(12.0).color(col));
                }
                ui.separator();
                egui::ScrollArea::vertical().max_height(160.0).stick_to_bottom(true).show(ui, |ui| {
                    for (who, text) in &p.chat {
                        if who.is_empty() {
                            ui.label(RichText::new(text).size(12.0).italics().color(DIM));
                        } else {
                            ui.label(RichText::new(format!("{who}: {text}")).size(13.0));
                        }
                    }
                });
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut p.draft).id(id.with("draft")).hint_text(tr("Say something…", "พิมพ์ข้อความ…")).desired_width(230.0));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (enter || widgets::ghost_button(ui, icon::PAPER_PLANE_RIGHT).clicked()) && !p.draft.trim().is_empty() {
                        send = Some(std::mem::take(&mut p.draft).trim().chars().take(300).collect());
                        r.request_focus();
                    }
                });
            });
        });
        ctx.data_mut(|d| d.insert_temp(id.with("rect"), area.response.rect));
        if alpha > 0.0 && alpha < 1.0 {
            ctx.request_repaint();
        }
        if let Some(text) = send {
            let name = self.party_name();
            if let Some(p) = self.party.as_mut() {
                p.send(&Wire::Chat { name: name.clone(), text: text.clone() });
            }
            self.on_party(Wire::Chat { name, text });
        }
        if leave && let Some(mut p) = self.party.take() {
            p.leave();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn item() -> Item {
        Item { source: "s".into(), series_url: "u".into(), title: "t".into(), poster: None, ep: Episode { season: 1, num: 1.0, label: "1".into(), url: "e".into(), dub: false } }
    }
    #[test]
    fn clock_offset_and_room_position() {
        let mut s = Sync::default();
        // guest clock 5000 ms behind the host; 200 ms each way; host held it 300 ms before answering
        let (t0, hr, hs, t1) = (1_000_000u64, 1_005_200, 1_005_500, 1_000_700);
        s.on_pong(t0, hr, hs, t1);
        assert_eq!(s.offset, 5000);
        assert_eq!(s.best_rtt, 400);
        // a slower probe does not replace it
        s.on_pong(t0, hr + 900, hs + 900, t1 + 2000);
        assert_eq!(s.offset, 5000);
        // the host said "100 s" 1.5 s ago (room clock): the room is at 101.5 s now
        let at = s.room_now() - 1500;
        s.reference = Some(Reference { ep_url: "e".into(), pos: 100.0, at, paused: false, mine: false });
        let (_, want, _) = s.expected().unwrap();
        assert!((want - 101.5).abs() < 0.05, "{want}");
        s.reference.as_mut().unwrap().paused = true;
        assert_eq!(s.expected().unwrap().1, 100.0);
    }

    #[test]
    fn old_peers_still_understood() {
        // v0.1.1 sends no `at` and knows no Ping
        let w: Wire = serde_json::from_str(r#"{"t":"state","ep_url":"u","pos":3.0,"paused":false}"#).unwrap();
        assert!(matches!(w, Wire::State { at: 0, .. }));
    }

    #[test]
    fn wire_roundtrip() {
        let w = Wire::State { ep_url: "u".into(), pos: 12.5, paused: true, at: 7 };
        let t = serde_json::to_string(&w).unwrap();
        assert!(t.contains(r#""t":"state""#));
        match serde_json::from_str::<Wire>(&t).unwrap() {
            Wire::State { pos, paused, .. } => assert!(paused && pos == 12.5),
            _ => panic!(),
        }
    }

    /// host + one guest on localhost: hello → members, chat relayed to the host app, host's message to the guest
    #[test]
    fn host_and_guest() {
        let ctx = egui::Context::default();
        let (htx, hrx) = std::sync::mpsc::channel();
        let (gtx, grx) = std::sync::mpsc::channel();
        let mut h = host(47899, "Host".into(), htx, ctx.clone()).unwrap();
        let mut g = join("127.0.0.1:47899", "Guest".into(), gtx, ctx).unwrap();
        let wait = |rx: &std::sync::mpsc::Receiver<Msg>, f: &dyn Fn(&Wire) -> bool| {
            let until = Instant::now() + Duration::from_secs(5);
            while Instant::now() < until {
                if let Ok(Msg::Party(w)) = rx.recv_timeout(Duration::from_millis(200))
                    && f(&w)
                {
                    return true;
                }
            }
            false
        };
        assert!(wait(&grx, &|w| matches!(w, Wire::Members { names } if names.len() == 2)));
        g.send(&Wire::Chat { name: "Guest".into(), text: "hi".into() });
        assert!(wait(&hrx, &|w| matches!(w, Wire::Chat { text, .. } if text == "hi")));
        h.send(&Wire::State { ep_url: "e".into(), pos: 5.0, paused: false, at: 0 });
        assert!(wait(&grx, &|w| matches!(w, Wire::State { pos, .. } if *pos == 5.0)));
        h.leave();
        g.leave();
    }
}

impl WakuApp {
    /// Development remote control (`WAKU_DEV_DIR`): drive a party without clicking.
    pub fn dev_party(&mut self, arg: &str) -> String {
        let (cmd, rest) = arg.split_once(' ').unwrap_or((arg, ""));
        let server = if self.settings.party_server.trim().is_empty() { crate::relay::DEFAULT_SERVER.to_string() } else { self.settings.party_server.clone() };
        match cmd {
            "host" | "join" => {
                if let Some(mut p) = self.party.take() {
                    p.leave();
                }
                let name = if rest.trim().is_empty() || cmd == "join" { self.party_name() } else { rest.trim().to_string() };
                let code = (cmd == "join").then(|| rest.split_whitespace().next().unwrap_or("").to_string());
                match relay_party(&server, code.as_deref(), name, self.tx.clone(), self.ctx.clone()) {
                    Ok(p) => self.party = Some(p),
                    Err(e) => return format!("party error: {e}\n"),
                }
            }
            "chat" => {
                let name = self.party_name();
                if let Some(p) = self.party.as_mut() {
                    p.send(&Wire::Chat { name: name.clone(), text: rest.to_string() });
                }
                self.on_party(Wire::Chat { name, text: rest.to_string() });
            }
            "pause" | "play" => {
                if let Some(v) = self.view.as_mut() {
                    v.view.set_paused(cmd == "pause");
                }
            }
            "seek" => {
                if let (Some(v), Ok(t)) = (self.view.as_mut(), rest.trim().parse::<f64>()) {
                    v.view.seek_to(t);
                }
            }
            "leave" => {
                if let Some(mut p) = self.party.take() {
                    p.leave();
                }
            }
            // (not party, but handy here) search by a picture file: `party scene C:\path\shot.png`
            "scene" => match std::fs::read(rest.trim()) {
                Ok(b) => self.scene_search(b),
                Err(e) => return format!("scene: {e}\n"),
            },
            _ => {}
        }
        match &self.party {
            _ if cmd == "scene" || !self.scene.hits.is_empty() || self.scene.busy => format!(
                "scene: busy={} err={:?} hits={:?}\n",
                self.scene.busy,
                self.scene.error,
                self.scene.hits.iter().map(|h| format!("{} ep={:?} at={:.0} sim={:.2}", h.title_romaji, h.episode, h.at, h.similarity)).collect::<Vec<_>>()
            ),
            Some(p) => format!(
                "party: {} host={} members={:?} chat={:?}\n",
                p.address,
                p.is_host(),
                p.members,
                p.chat.iter().rev().take(4).rev().map(|(n, t)| format!("{n}:{t}")).collect::<Vec<_>>()
            ),
            None => "party: none\n".into(),
        }
    }
}
