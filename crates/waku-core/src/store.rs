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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub my_list: Vec<AnimeCard>,
    pub history: Vec<HistoryEntry>,
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
            },
        );
        self.history.truncate(200);
        self.save();
    }

    pub fn last_watched(&self, url: &str, title: &str) -> Option<&HistoryEntry> {
        let key = crate::text::norm(title);
        self.history
            .iter()
            .find(|h| h.card.url == url || crate::text::norm(&h.card.title) == key)
    }
}
