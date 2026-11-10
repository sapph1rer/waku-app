//! Settings, "My List" and watch history, saved as JSON in the config folder.

use crate::model::AnimeCard;
use crate::profile::config_dir;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn load<T: DeserializeOwned + Default>(name: &str) -> T {
    std::fs::read_to_string(config_dir().join(name))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save<T: Serialize>(name: &str, v: &T) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(t) = serde_json::to_string_pretty(v) {
        let tmp: PathBuf = dir.join(format!("{name}.tmp"));
        if std::fs::write(&tmp, t).is_ok() {
            let _ = std::fs::rename(&tmp, dir.join(name));
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// best | worst | 1080p | 720p …
    pub quality: String,
    /// mpv (default), a full path, or another player such as vlc.
    pub player_cmd: String,
    /// Source shown on the home page.
    pub home_source: String,
    /// When every server of a source fails, try the same episode on other sources.
    pub auto_failover: bool,
    /// Preferred order for failover (source ids). Missing ids go last.
    pub source_order: Vec<String>,
    pub ui_scale: f32,
    /// UI language: "en" (default) or "th".
    pub language: String,
    /// Which title to show when metadata is known: "english" (default) | "romaji" | "site".
    pub title_lang: String,
    /// Look titles up on AniList / Kitsu / Jikan (covers, banners, episode titles).
    pub fetch_meta: bool,
    /// Show 18+ titles in On Air.
    pub show_adult: bool,
    /// "builtin" (rmp inside the window, default) or "external" (`player_cmd`).
    pub player: String,
    /// Built-in player: "auto" (libmpv, else the native engine) | "mpv" | "native".
    pub engine: String,
    /// Built-in player: hardware decoding ("auto-safe", "d3d11va", "nvdec", "no" …).
    pub hwdec: String,
    pub volume: f64,
    pub muted: bool,
    pub sub_delay: f64,
    pub sub_scale: f64,
    pub sub_pos: f64,
    pub sub_file_style: bool,
    pub deband: bool,
    /// "auto" (speaker layout of the device) or "stereo"
    pub audio_channels: String,
    /// bitstream to an AV receiver, e.g. "ac3,eac3,truehd,dts-hd" ("" = off)
    pub audio_passthrough: String,
    /// "fast" | "balanced" | "high"
    pub video_quality: String,
    /// preferred audio / subtitle languages ("Japanese", "Thai" …; sub "off" = start without)
    pub audio_langs: String,
    pub sub_langs: String,
    pub autoplay_next: bool,
    /// Resume episodes from where they were left.
    pub resume: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: "best".into(),
            player_cmd: "mpv".into(),
            home_source: "waku".into(),
            auto_failover: true,
            source_order: Vec::new(),
            ui_scale: 1.0,
            language: "en".into(),
            title_lang: "english".into(),
            fetch_meta: true,
            show_adult: false,
            player: "builtin".into(),
            engine: "auto".into(),
            hwdec: "auto-safe".into(),
            volume: 100.0,
            muted: false,
            sub_delay: 0.0,
            sub_scale: 1.0,
            sub_pos: 100.0,
            sub_file_style: true,
            deband: true,
            audio_channels: "auto".into(),
            audio_passthrough: String::new(),
            video_quality: "balanced".into(),
            audio_langs: String::new(),
            sub_langs: "Thai, English".into(),
            autoplay_next: true,
            resume: true,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        load("settings.json")
    }
    pub fn save(&self) {
        save("settings.json", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub card: AnimeCard,
    pub ep_num: f32,
    pub ep_label: String,
    pub ep_url: String,
    pub when: u64,
    /// where the built-in player stopped (seconds) and the episode length (0 = unknown)
    #[serde(default)]
    pub pos: f64,
    #[serde(default)]
    pub dur: f64,
}

/// Resume points per episode URL (built-in player).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Progress {
    pub pos: f64,
    pub dur: f64,
    pub when: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub my_list: Vec<AnimeCard>,
    pub history: Vec<HistoryEntry>,
    /// episode url -> how far it was watched
    pub progress: std::collections::HashMap<String, Progress>,
}

impl Library {
    pub fn load() -> Self {
        load("library.json")
    }
    pub fn save(&self) {
        save("library.json", self)
    }

    pub fn in_list(&self, url: &str) -> bool {
        self.my_list.iter().any(|c| c.url == url)
    }

    pub fn toggle_list(&mut self, card: &AnimeCard) {
        if let Some(i) = self.my_list.iter().position(|c| c.url == card.url) {
            self.my_list.remove(i);
        } else {
            self.my_list.insert(0, card.clone());
        }
        self.save();
    }

    /// Record a watched episode (one entry per anime title, newest first).
    pub fn watched(&mut self, card: &AnimeCard, ep_num: f32, ep_label: &str, ep_url: &str) {
        let key = crate::text::norm(&card.title);
        self.history
            .retain(|h| h.card.url != card.url && crate::text::norm(&h.card.title) != key);
        self.history.insert(
            0,
            HistoryEntry {
                card: card.clone(),
                ep_num,
                ep_label: ep_label.into(),
                ep_url: ep_url.into(),
                when: now(),
                pos: 0.0,
                dur: 0.0,
            },
        );
        self.history.truncate(200);
        self.save();
    }

    /// Remember the position of an episode (saved by the caller with `save`, it changes often).
    pub fn set_progress(&mut self, ep_url: &str, pos: f64, dur: f64, ended: bool) {
        let pos = if ended || (dur > 0.0 && dur - pos < 30.0) { dur } else { pos };
        self.progress.insert(ep_url.to_string(), Progress { pos, dur, when: now() });
        if let Some(h) = self.history.iter_mut().find(|h| h.ep_url == ep_url) {
            h.pos = pos;
            h.dur = dur;
        }
        // keep the file small: newest 2000 episodes
        if self.progress.len() > 2000 {
            let mut v: Vec<(String, u64)> = self.progress.iter().map(|(k, p)| (k.clone(), p.when)).collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1));
            for (k, _) in v.into_iter().skip(2000) {
                self.progress.remove(&k);
            }
        }
    }

    /// Where to start an episode: the saved position unless it was (almost) finished.
    pub fn resume_point(&self, ep_url: &str) -> f64 {
        match self.progress.get(ep_url) {
            Some(p) if p.pos > 15.0 && (p.dur <= 0.0 || p.dur - p.pos > 30.0) => p.pos - 3.0,
            _ => 0.0,
        }
    }

    pub fn last_watched(&self, url: &str, title: &str) -> Option<&HistoryEntry> {
        let key = crate::text::norm(title);
        self.history
            .iter()
            .find(|h| h.card.url == url || crate::text::norm(&h.card.title) == key)
    }
}
