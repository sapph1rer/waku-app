//! Players whose playlists answer only to the page's own script (aki-h.stream: `file/<id>/` says `null` to
//! anything but the player page itself). The hidden browser opens the player page and fetches the playlists
//! from inside it, as any visitor's browser does; they are then served from a loopback port to the player.
//! The video segments are on another host and open normally.

use crate::http::browser;
use crate::text::abs;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};

static STORE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static PORT: OnceLock<u16> = OnceLock::new();

/// `http://127.0.0.1:<port>/<key>.m3u8` serving `text` (until the app exits).
fn publish(key: &str, text: String) -> Option<String> {
    let port = *PORT.get_or_init(|| {
        let Ok(l) = TcpListener::bind("127.0.0.1:0") else { return 0 };
        let port = l.local_addr().map(|a| a.port()).unwrap_or(0);
        std::thread::Builder::new()
            .name("gated-playlists".into())
            .spawn(move || {
                for mut s in l.incoming().flatten() {
                    let mut buf = [0u8; 2048];
                    let n = s.read(&mut buf).unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let path = req.split_whitespace().nth(1).unwrap_or("").trim_start_matches('/').to_string();
                    let body = STORE.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&path).cloned()));
                    let _ = match body {
                        Some(b) => write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()),
                        None => write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    };
                }
            })
            .ok();
        port
    });
    if port == 0 {
        return None;
    }
    let name = format!("{key}.m3u8");
    STORE.lock().ok()?.get_or_insert_with(HashMap::new).insert(name.clone(), text);
    Some(format!("http://127.0.0.1:{port}/{name}"))
}

/// Make every relative URI of a playlist absolute.
fn absolutize(text: &str, base: &str) -> String {
    text.lines()
        .map(|l| {
            let t = l.trim();
            if t.is_empty() || t.starts_with('#') { l.to_string() } else { abs(base, t).unwrap_or_else(|| l.to_string()) }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `https://<host>/v/<id>` player page → (origin, id), for the players of this family.
fn parse(url: &str) -> Option<(String, String)> {
    let (origin, rest) = url.split_once("://").map(|(s, r)| (s, r))?;
    let (host, path) = rest.split_once('/')?;
    if !["aki-h.stream", "v.aki-h.com"].contains(&host) {
        return None;
    }
    let id = path.strip_prefix("v/").or_else(|| path.strip_prefix("v2/"))?;
    let id: String = id.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    (!id.is_empty()).then(|| (format!("{origin}://{host}"), id))
}

/// Playable local master playlist for a gated player page, or None (no browser helper, or it failed).
/// Returns (url, best height).
pub fn resolve(player_page: &str, referer: &str, log: &dyn Fn(String)) -> Option<(String, Option<u32>)> {
    let (origin, id) = parse(player_page)?;
    let b = browser()?;
    let page = format!("{origin}/v/{id}");
    // loads the page, so that the playlists can be asked for from inside it
    let _ = b.sniff(&page, referer);
    let master_url = format!("{origin}/file/{id}/");
    let master = b.fetch(&master_url, Some(&page)).ok()?;
    if !master.trim_start().starts_with("#EXTM3U") {
        log(format!("    gated {id}: master is not a playlist"));
        return None;
    }
    let mut out = Vec::new();
    let mut height = None;
    let mut want_variant = false;
    for (i, l) in master.lines().enumerate() {
        let t = l.trim();
        if t.starts_with("#EXT-X-STREAM-INF") {
            want_variant = true;
            if let Some(h) = t.split("RESOLUTION=").nth(1).and_then(|r| r.split(['x', ',']).nth(1)).and_then(|h| h.trim().parse::<u32>().ok()) {
                height = height.max(Some(h));
            }
            out.push(l.to_string());
        } else if want_variant && !t.is_empty() && !t.starts_with('#') {
            want_variant = false;
            let v_url = abs(&master_url, t)?;
            let text = b.fetch(&v_url, Some(&page)).ok().filter(|x| x.trim_start().starts_with("#EXTM3U"))?;
            let local = publish(&format!("{id}-{i}"), absolutize(&text, &v_url))?;
            out.push(local);
        } else {
            out.push(l.to_string());
        }
    }
    let url = publish(&format!("{id}-master"), out.join("\n"))?;
    log(format!("    gated {id}: playlists served locally, HLS {height:?}"));
    Some((url, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_and_playlists() {
        assert_eq!(parse("https://aki-h.stream/v/Adn6FcnS").unwrap().1, "Adn6FcnS");
        assert_eq!(parse("https://aki-h.stream/v2/Adn6FcnS?x=1").unwrap().1, "Adn6FcnS");
        assert!(parse("https://example.com/v/abc").is_none());
        let t = absolutize("#EXTM3U\n#EXTINF:2,\nv1/seg/1.webp\nhttps://x.y/2.webp", "https://aki-h.stream/quality/A/1080/");
        assert!(t.contains("https://aki-h.stream/quality/A/1080/v1/seg/1.webp") && t.ends_with("https://x.y/2.webp"));
        let u = publish("t", "#EXTM3U\n".into()).unwrap();
        let mut s = std::net::TcpStream::connect(u.trim_start_matches("http://").split('/').next().unwrap()).unwrap();
        write!(s, "GET /t.m3u8 HTTP/1.1\r\n\r\n").unwrap();
        let mut r = String::new();
        s.read_to_string(&mut r).unwrap();
        assert!(r.contains("200 OK") && r.ends_with("#EXTM3U\n"));
    }
}
