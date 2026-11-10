//! Poster loader: background workers download (with Referer), cache on disk,
//! decode + downscale, then the UI thread turns them into textures.

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use waku_core::http::Http;
use waku_core::profile::cache_dir;

const WORKERS: usize = 6;
const MAX_H: u32 = 420;

enum Slot {
    Loading,
    Ready(TextureHandle),
    Failed,
}

pub struct Images {
    slots: HashMap<String, Slot>,
    req: Sender<(String, String)>,
    done: Receiver<(String, Result<ColorImage, String>)>,
}

fn disk_path(url: &str) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    cache_dir().join("img").join(format!("{:016x}", h.finish()))
}

const BLUR: &str = "blur::";

fn fetch(http: &Http, key: &str, referer: &str) -> Result<ColorImage, String> {
    let (blur, url) = match key.strip_prefix(BLUR) {
        Some(u) => (true, u),
        None => (false, key),
    };
    let path = disk_path(url);
    let bytes = match std::fs::read(&path) {
        Ok(b) if !b.is_empty() => b,
        _ => {
            let b = http
                .get_bytes(url, Some(referer))
                .map_err(|e| format!("{e:#}"))?;
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&path, &b);
            b
        }
    };
    let img = image::load_from_memory(&bytes).map_err(|e| {
        let _ = std::fs::remove_file(&path);
        e.to_string()
    })?;
    // hero backdrop: tiny + blurred, the GPU's linear upscale makes it soft and smooth
    let img = if blur {
        img.thumbnail(96, 96).blur(2.2)
    } else if img.height() > MAX_H {
        img.resize(MAX_H * 3, MAX_H, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

impl Images {
    pub fn new(ctx: &Context, http: Http) -> Self {
        let (req_tx, req_rx) = channel::<(String, String)>();
        let (done_tx, done_rx) = channel();
        let req_rx = Arc::new(Mutex::new(req_rx));
        for _ in 0..WORKERS {
            let rx = req_rx.clone();
            let tx = done_tx.clone();
            let http = http.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                loop {
                    let job = rx.lock().ok().and_then(|r| r.recv().ok());
                    let Some((url, referer)) = job else { break };
                    let res = fetch(&http, &url, &referer);
                    if tx.send((url, res)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Self {
            slots: HashMap::new(),
            req: req_tx,
            done: done_rx,
        }
    }

    /// Texture for `url`, starting a download the first time it is asked for.
    pub fn get(&mut self, url: &str, referer: &str) -> Option<&TextureHandle> {
        if !self.slots.contains_key(url) {
            self.slots.insert(url.to_string(), Slot::Loading);
            let _ = self.req.send((url.to_string(), referer.to_string()));
            return None;
        }
        match self.slots.get(url) {
            Some(Slot::Ready(t)) => Some(t),
            _ => None,
        }
    }

    /// Soft, blurred version of an image (for backdrops).
    pub fn get_blur(&mut self, url: &str, referer: &str) -> Option<&TextureHandle> {
        self.get(&format!("{BLUR}{url}"), referer)
    }

    pub fn failed(&self, url: &str) -> bool {
        matches!(self.slots.get(url), Some(Slot::Failed))
    }

    /// Move finished downloads into textures (call once per frame).
    pub fn pump(&mut self, ctx: &Context) {
        while let Ok((url, res)) = self.done.try_recv() {
            let slot = match res {
                Ok(img) => Slot::Ready(ctx.load_texture(url.clone(), img, TextureOptions::LINEAR)),
                Err(_) => Slot::Failed,
            };
            self.slots.insert(url, slot);
        }
    }
}
