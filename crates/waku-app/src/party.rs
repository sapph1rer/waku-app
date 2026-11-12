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
    Load { item: Item, pos: f64, paused: bool },
    State { ep_url: String, pos: f64, paused: bool },
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

    /// Every frame: tell the room about what the viewer did here; the host's heartbeat.
    pub fn party_tick(&mut self) {
        let now_state = self.party_now();
        let Some(p) = self.party.as_mut() else { return };
        let Some((item, pos, paused, loaded)) = now_state else { return };
        if !loaded {
            // finding / opening the link: neither "paused" nor a position yet — say nothing
            p.loading = true;
            return;
        }
        if let Role::Host { current, .. } = &p.role {
            *current.lock().unwrap() = Some(Wire::Load { item: item.clone(), pos, paused });
        }
        let quiet = Instant::now() < p.quiet_until;
        let mut out: Option<Wire> = None;
        if std::mem::take(&mut p.loading) || p.last.is_none() {
            // just started playing: announce it only when it is an episode the room is not on
            let other = p.last.as_ref().is_none_or(|l| l.0 != item.ep.url);
            if other && !quiet && (p.last.is_some() || p.is_host()) {
                out = Some(Wire::Load { item: item.clone(), pos, paused });
            }
        } else if let Some((ep_url, last_pos, last_paused, at)) = &p.last
            && !quiet
        {
            if *ep_url != item.ep.url {
                out = Some(Wire::Load { item: item.clone(), pos, paused });
            } else if *last_paused != paused {
                out = Some(Wire::State { ep_url: item.ep.url.clone(), pos, paused });
            } else {
                let expect = if paused { *last_pos } else { last_pos + at.elapsed().as_secs_f64() };
                if (pos - expect).abs() > 3.0 {
                    out = Some(Wire::State { ep_url: item.ep.url.clone(), pos, paused }); // a seek here
                }
            }
        }
        // the host's position check: every 3 s on a LAN, every 2 minutes through ntfy.sh (its daily limit)
        let beat = if p.is_relay() { Duration::from_secs(120) } else { Duration::from_secs(3) };
        if out.is_none() && p.is_host() && p.last_beat.elapsed() > beat {
            out = Some(Wire::State { ep_url: item.ep.url.clone(), pos, paused });
        }
        if let Some(w) = out {
            if matches!(w, Wire::State { .. }) {
                p.last_beat = Instant::now();
            }
            p.send(&w);
        }
        p.last = Some((item.ep.url.clone(), pos, paused, Instant::now()));
        self.ctx.request_repaint_after(Duration::from_millis(500));
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
            Wire::State { ep_url, pos, paused } => {
                p.quiet_until = Instant::now() + Duration::from_millis(1500);
                let host_beat = !p.is_host();
                if let Some(v) = self.view.as_mut()
                    && v.eps.get(v.view.index()).is_some_and(|e| e.url == ep_url)
                {
                    v.view.set_paused(paused);
                    // ~0.2 s of travel; small drift is left alone (seeking stutters)
                    let target = if paused { pos } else { pos + 0.2 };
                    if (v.view.position() - target).abs() > if host_beat { 2.0 } else { 1.0 } {
                        v.view.seek_to(target);
                    }
                    p.last = Some((ep_url, target, paused, Instant::now()));
                }
            }
            Wire::Load { item, pos, paused } => {
                p.quiet_until = Instant::now() + Duration::from_millis(4000);
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
                        p.send(&Wire::Load { item, pos, paused });
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
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
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
        let Some(p) = self.party.as_mut() else { return };
        egui::Area::new("party-panel".into()).anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -96.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            Frame::new().fill(BAR.gamma_multiply(0.94)).stroke(egui::Stroke::new(1.0, LINE)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(300.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {}", icon::USERS_THREE, tr("Watch party", "ดูด้วยกัน"))).font(bold(15.0)).color(ACCENT_HI));
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
                    let r = ui.add(egui::TextEdit::singleline(&mut p.draft).hint_text(tr("Say something…", "พิมพ์ข้อความ…")).desired_width(230.0));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (enter || widgets::ghost_button(ui, icon::PAPER_PLANE_RIGHT).clicked()) && !p.draft.trim().is_empty() {
                        send = Some(std::mem::take(&mut p.draft).trim().chars().take(300).collect());
                        r.request_focus();
                    }
                });
            });
        });
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
    fn wire_roundtrip() {
        let w = Wire::State { ep_url: "u".into(), pos: 12.5, paused: true };
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
        h.send(&Wire::State { ep_url: "e".into(), pos: 5.0, paused: false });
        assert!(wait(&grx, &|w| matches!(w, Wire::State { pos, .. } if *pos == 5.0)));
        h.leave();
        g.leave();
    }
}
