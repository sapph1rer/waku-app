//! Discord Rich Presence: "Watching <anime> · EP 5" with the poster and the time left, through Discord's local
//! IPC pipe (`\\.\pipe\discord-ipc-N` on Windows, a unix socket elsewhere). Nothing is sent anywhere else.
//!
//! Needs a Discord application id (Settings): its name is what Discord shows ("Watching Waku"). A worker
//! thread keeps the connection, reconnects when Discord starts later, and only sends when the activity changed.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub details: String,
    pub state: String,
    pub image: Option<String>,
    pub image_text: String,
    /// unix seconds (start, end): the time bar; None while paused
    pub times: Option<(u64, u64)>,
}

enum Cmd {
    Set(Option<Activity>),
    ClientId(String),
}

pub struct Presence {
    tx: Sender<Cmd>,
    last: Option<Option<Activity>>,
    client_id: String,
}

impl Presence {
    pub fn start(client_id: &str) -> Presence {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("discord".into()).spawn(move || worker(rx)).ok();
        let _ = tx.send(Cmd::ClientId(client_id.to_string()));
        Presence { tx, last: None, client_id: client_id.to_string() }
    }

    pub fn set_client_id(&mut self, id: &str) {
        if id != self.client_id {
            self.client_id = id.to_string();
            self.last = None;
            let _ = self.tx.send(Cmd::ClientId(id.to_string()));
        }
    }

    /// What to show (None = clear). Cheap to call every frame: only changes go out.
    pub fn set(&mut self, a: Option<Activity>) {
        // the time bar moves on its own: a few seconds of drift is not a change
        let same = match (&self.last, &a) {
            (Some(Some(x)), Some(y)) => {
                x.details == y.details
                    && x.state == y.state
                    && x.image == y.image
                    && match (x.times, y.times) {
                        (Some(p), Some(q)) => p.0.abs_diff(q.0) < 5 && p.1.abs_diff(q.1) < 5,
                        (None, None) => true,
                        _ => false,
                    }
            }
            (Some(None), None) => true,
            _ => false,
        };
        if !same {
            self.last = Some(a.clone());
            let _ = self.tx.send(Cmd::Set(a));
        }
    }
}

// ------------------------------------------------------------------ IPC

trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

fn connect() -> Option<Box<dyn Pipe>> {
    for i in 0..10 {
        #[cfg(windows)]
        {
            if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")) {
                return Some(Box::new(f));
            }
        }
        #[cfg(unix)]
        {
            let dirs = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"].iter().filter_map(|k| std::env::var(k).ok()).chain(["/tmp".to_string()]);
            for d in dirs {
                for sub in ["", "app/com.discordapp.Discord/", "snap.discord/"] {
                    if let Ok(s) = std::os::unix::net::UnixStream::connect(format!("{d}/{sub}discord-ipc-{i}")) {
                        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                        return Some(Box::new(s));
                    }
                }
            }
        }
    }
    None
}

fn send(p: &mut dyn Pipe, op: u32, v: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(v)?;
    let mut buf = Vec::with_capacity(8 + body.len());
    buf.extend_from_slice(&op.to_le_bytes());
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(&body);
    p.write_all(&buf)?;
    p.flush()
}

fn recv(p: &mut dyn Pipe) -> std::io::Result<(u32, Value)> {
    let mut h = [0u8; 8];
    p.read_exact(&mut h)?;
    let op = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
    let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
    if len > 1 << 20 {
        return Err(std::io::Error::other("frame too big"));
    }
    let mut b = vec![0u8; len];
    p.read_exact(&mut b)?;
    Ok((op, serde_json::from_slice(&b).unwrap_or(Value::Null)))
}

pub fn activity_json(a: &Activity) -> Value {
    let cut = |s: &str, n: usize| -> String {
        let t: String = s.chars().take(n).collect();
        if t.chars().count() < 2 { format!("{t}  ") } else { t }
    };
    let mut act = json!({
        "type": 3, // "Watching"
        "details": cut(&a.details, 120),
        "state": cut(&a.state, 120),
    });
    if let Some((s, e)) = a.times {
        act["timestamps"] = json!({ "start": s, "end": e });
    }
    let mut assets = serde_json::Map::new();
    if let Some(img) = a.image.as_ref().filter(|u| u.starts_with("https://")) {
        assets.insert("large_image".into(), json!(img));
        assets.insert("large_text".into(), json!(cut(&a.image_text, 120)));
    }
    if !assets.is_empty() {
        act["assets"] = Value::Object(assets);
    }
    act
}

fn worker(rx: Receiver<Cmd>) {
    let mut client_id = String::new();
    let mut want: Option<Activity> = None;
    let mut pipe: Option<Box<dyn Pipe>> = None;
    let mut dirty = false;
    let mut nonce = 0u64;
    loop {
        // wait for a change (or retry a lost connection every 15 s)
        let cmd = rx.recv_timeout(Duration::from_secs(15));
        match cmd {
            Ok(Cmd::Set(a)) => {
                want = a;
                dirty = true;
            }
            Ok(Cmd::ClientId(id)) => {
                if id != client_id {
                    client_id = id;
                    pipe = None; // handshake again under the new application
                    dirty = true;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return,
        }
        // take the newest of a burst
        while let Ok(c) = rx.try_recv() {
            match c {
                Cmd::Set(a) => want = a,
                Cmd::ClientId(id) => {
                    if id != client_id {
                        client_id = id;
                        pipe = None;
                    }
                }
            }
            dirty = true;
        }
        if client_id.trim().is_empty() || !client_id.chars().all(|c| c.is_ascii_digit()) {
            pipe = None;
            continue;
        }
        if !dirty && pipe.is_some() {
            continue;
        }
        if pipe.is_none() {
            if want.is_none() {
                continue; // nothing to show: no need to talk to Discord
            }
            let Some(mut p) = connect() else { continue };
            if send(&mut *p, 0, &json!({ "v": 1, "client_id": client_id })).is_err() || recv(&mut *p).map(|(op, _)| op != 1).unwrap_or(true) {
                continue; // Discord answered with an error / closed: the application id may be wrong
            }
            pipe = Some(p);
        }
        let Some(p) = pipe.as_mut() else { continue };
        nonce += 1;
        let args = match &want {
            Some(a) => json!({ "pid": std::process::id(), "activity": activity_json(a) }),
            None => json!({ "pid": std::process::id() }),
        };
        let msg = json!({ "cmd": "SET_ACTIVITY", "args": args, "nonce": format!("waku-{nonce}") });
        if send(&mut **p, 1, &msg).is_err() || recv(&mut **p).is_err() {
            pipe = None; // Discord closed: try again on the next change / tick
            continue;
        }
        dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_shape() {
        let a = Activity { details: "Frieren".into(), state: "EP 3".into(), image: Some("https://x/y.jpg".into()), image_text: "Frieren".into(), times: Some((10, 20)) };
        let v = activity_json(&a);
        assert_eq!(v["type"], 3);
        assert_eq!(v["timestamps"]["end"], 20);
        assert_eq!(v["assets"]["large_image"], "https://x/y.jpg");
        // Discord wants at least 2 characters
        let b = Activity { details: "X".into(), state: String::new(), image: None, image_text: String::new(), times: None };
        assert_eq!(activity_json(&b)["details"].as_str().unwrap().chars().count(), 3);
    }

    /// A fake Discord: handshake + one SET_ACTIVITY arrive in the right framing.
    #[cfg(unix)]
    #[test]
    fn talks_ipc() {
        let dir = std::env::temp_dir().join(format!("waku-dc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("discord-ipc-0");
        let _ = std::fs::remove_file(&sock);
        let l = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        // SAFETY: test-only, single-threaded at this point
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };
        let server = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let (op, hello) = recv(&mut s).unwrap();
            assert_eq!(op, 0);
            assert_eq!(hello["client_id"], "123");
            send(&mut s, 1, &json!({"evt": "READY"})).unwrap();
            let (op, set) = recv(&mut s).unwrap();
            send(&mut s, 1, &json!({"evt": null})).unwrap();
            (op, set)
        });
        let mut p = Presence::start("123");
        p.set(Some(Activity { details: "Frieren".into(), state: "EP 3".into(), image: None, image_text: String::new(), times: None }));
        let (op, set) = server.join().unwrap();
        assert_eq!(op, 1);
        assert_eq!(set["cmd"], "SET_ACTIVITY");
        assert_eq!(set["args"]["activity"]["details"], "Frieren");
    }
}

impl crate::app::WakuApp {
    /// Keep Discord's "Watching …" in step with the built-in player (called every frame).
    pub fn discord_tick(&mut self) {
        if !self.settings.discord_presence {
            self.presence.set(None);
            return;
        }
        self.presence.set_client_id(&self.settings.discord_app_id.clone());
        let act = self.view.as_ref().and_then(|v| {
            let st = v.view.state();
            let e = v.eps.get(v.view.index())?;
            let m = self.meta.get(&v.detail.url, &v.detail.title);
            let title = self.title_for(m, &v.detail.title);
            let ep = crate::i18n::ep_name(e);
            let src = if v.detail.source == crate::dlpage::LOCAL { crate::i18n::tr("downloaded", "ไฟล์ในเครื่อง").to_string() } else { self.source_name(&v.detail.source) };
            let paused = st.paused || !st.loaded;
            let state = if paused { format!("{ep} · {}", crate::i18n::tr("paused", "หยุดไว้")) } else { format!("{ep} · {src}") };
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let times = match (paused, st.duration) {
                (false, Some(d)) if d > 0.0 => {
                    let start = now.saturating_sub(st.position as u64);
                    Some((start, start + d as u64))
                }
                _ => None,
            };
            Some(Activity {
                details: title.clone(),
                state,
                image: m.and_then(|m| m.cover_url.clone()).or(v.detail.poster.clone()),
                image_text: title,
                times,
            })
        });
        self.presence.set(act);
    }
}
