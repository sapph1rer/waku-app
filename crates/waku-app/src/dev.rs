//! Remote control for testing (development only): with `WAKU_DEV_DIR=<folder>` set, the app reads
//! `<folder>/cmd.txt` (one command per line), runs the commands one after another and renames the file to
//! `cmd.done`. Screenshots go to `<folder>/<name>.png`, a log of what ran to `<folder>/dev.log`.
//!
//! Commands: `wait SECS` · `move X Y` · `click X Y` · `rclick X Y` · `scroll DY` · `key NAME` (Space, Escape,
//! ArrowLeft …) · `text STRING` · `shot NAME` · `page Home|Search|Browse|MyList|History|OnAir|Settings` ·
//! `search QUERY` · `quit`. Coordinates are in UI points (the screenshot's pixels divided by the scale).

use egui::{Event, Key, PointerButton, Pos2, RawInput};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub struct Dev {
    dir: PathBuf,
    queue: VecDeque<String>,
    wait_until: Option<Instant>,
    last_poll: Instant,
    pending_shot: Option<String>,
    /// a command for the app itself (page / search), taken by `app.rs`
    pub app_cmd: Option<String>,
}

impl Dev {
    pub fn from_env() -> Option<Dev> {
        let dir = PathBuf::from(std::env::var_os("WAKU_DEV_DIR")?);
        let _ = std::fs::create_dir_all(&dir);
        Some(Dev { dir, queue: VecDeque::new(), wait_until: None, last_poll: Instant::now(), pending_shot: None, app_cmd: None })
    }

    fn log(&self, s: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("dev.log")) {
            let _ = writeln!(f, "{s}");
        }
    }

    /// Called with every frame's input: feeds the next command's events.
    pub fn input(&mut self, ctx: &egui::Context, raw: &mut RawInput) {
        ctx.request_repaint_after(Duration::from_millis(200));
        // finished screenshots arrive as events
        for e in &raw.events {
            if let Event::Screenshot { image, .. } = e
                && let Some(name) = self.pending_shot.take()
            {
                let path = self.dir.join(format!("{name}.png"));
                let ok = image::save_buffer(&path, image.as_raw(), image.width() as u32, image.height() as u32, image::ExtendedColorType::Rgba8).is_ok();
                self.log(&format!("shot {name}: {ok}"));
            }
        }
        if self.last_poll.elapsed() > Duration::from_millis(300) {
            self.last_poll = Instant::now();
            let f = self.dir.join("cmd.txt");
            if let Ok(t) = std::fs::read_to_string(&f) {
                let _ = std::fs::rename(&f, self.dir.join("cmd.done"));
                self.queue.extend(t.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')));
            }
        }
        if self.wait_until.is_some_and(|t| Instant::now() < t) || self.pending_shot.is_some() || self.app_cmd.is_some() {
            return;
        }
        self.wait_until = None;
        let Some(line) = self.queue.pop_front() else { return };
        self.log(&format!("> {line}"));
        let mut it = line.splitn(2, ' ');
        let cmd = it.next().unwrap_or("");
        let arg = it.next().unwrap_or("").trim().to_string();
        let xy = || -> Option<Pos2> {
            let mut p = arg.split_whitespace().filter_map(|v| v.parse::<f32>().ok());
            Some(Pos2::new(p.next()?, p.next()?))
        };
        let click = |raw: &mut RawInput, p: Pos2, b: PointerButton| {
            let m = Default::default();
            raw.events.push(Event::PointerMoved(p));
            raw.events.push(Event::PointerButton { pos: p, button: b, pressed: true, modifiers: m });
            raw.events.push(Event::PointerButton { pos: p, button: b, pressed: false, modifiers: m });
        };
        match cmd {
            "wait" => self.wait_until = Some(Instant::now() + Duration::from_secs_f64(arg.parse().unwrap_or(1.0))),
            "move" => {
                if let Some(p) = xy() {
                    raw.events.push(Event::PointerMoved(p));
                }
            }
            "click" => {
                if let Some(p) = xy() {
                    click(raw, p, PointerButton::Primary);
                }
            }
            "rclick" => {
                if let Some(p) = xy() {
                    click(raw, p, PointerButton::Secondary);
                }
            }
            "scroll" => {
                let dy: f32 = arg.parse().unwrap_or(-200.0);
                raw.events.push(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, dy), modifiers: Default::default(), phase: egui::TouchPhase::Move });
            }
            "key" => {
                if let Some(k) = Key::from_name(&arg) {
                    for pressed in [true, false] {
                        raw.events.push(Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: Default::default() });
                    }
                }
            }
            "text" => raw.events.push(Event::Text(arg)),
            "shot" => {
                self.pending_shot = Some(if arg.is_empty() { "shot".into() } else { arg });
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            "quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            _ => self.app_cmd = Some(line),
        }
    }
}
