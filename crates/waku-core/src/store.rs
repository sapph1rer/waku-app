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

pub(crate) fn load_json<T: DeserializeOwned + Default>(name: &str) -> T {
    load(name)
}

pub(crate) fn save_json<T: Serialize>(name: &str, v: &T) {
    save(name, v)
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
    /// Jump over openings / endings by themselves (AniSkip times); the "Skip" button shows either way.
    pub auto_skip: bool,
    /// Where downloads go ("" = `Downloads` next to the program).
    pub download_dir: String,
    /// Discord Rich Presence ("Watching …"), with the Discord application id whose name it shows
    pub discord_presence: bool,
    pub discord_app_id: String,
    /// name shown to the others in a watch party
    pub party_name: String,
    /// ntfy server for internet watch parties ("" = https://ntfy.sh; a self-hosted ntfy works too)
    pub party_server: String,
    /// GitHub repository the app updates and the sources' rules come from ("owner/repo")
    pub update_repo: String,
    /// look for a new version of the app on start
    pub check_updates: bool,
    /// 18+ (hentai) sources: off unless the user turned them on (and said they are 18 or older)
    pub adult_sources: bool,
    /// take new sources rules from the repository by themselves (when sources.toml was not edited by hand)
    pub auto_update_sources: bool,
    /// Resume episodes from where they were left.
    pub resume: bool,
    /// Latest searches, newest first (shown as shortcuts on the Search page).
    pub recent_searches: Vec<String>,
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
            auto_skip: true,
            download_dir: String::new(),
            discord_presence: false,
            discord_app_id: String::new(),
            party_name: String::new(),
            party_server: String::new(),
            update_repo: "sapph1rer/waku-app".into(),
            check_updates: true,
            adult_sources: false,
            auto_update_sources: true,
            resume: true,
            recent_searches: Vec::new(),
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
    /// the Thai-dub track (not the subbed one)
    #[serde(default)]
    pub dub: bool,
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
        let mut l: Library = load("library.json");
        let n = l.history.len();
        l.dedupe_history();
        if l.history.len() != n {
            l.save();
        }
        l
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

    /// A series page brought its picture: cards saved without one (history, My List) get it.
    pub fn fill_poster(&mut self, url: &str, poster: &str) -> bool {
        let mut changed = false;
        for c in self.history.iter_mut().map(|h| &mut h.card).chain(self.my_list.iter_mut()) {
            if c.url == url && c.poster.as_deref() != Some(poster) && c.poster.is_none() {
                c.poster = Some(poster.to_string());
                changed = true;
            }
        }
        if changed {
            self.save();
        }
        changed
    }

    /// Record a watched episode (one entry per anime title, newest first).
    pub fn watched(&mut self, card: &AnimeCard, ep_num: f32, ep_label: &str, ep_url: &str, dub: bool) {
        // one entry per show, whichever site it was watched on; a card without a picture keeps the one it had
        let mut card = card.clone();
        if card.poster.is_none() {
            card.poster = self
                .history
                .iter()
                .find(|h| h.card.url == card.url)
                .and_then(|h| h.card.poster.clone())
                .or_else(|| self.my_list.iter().find(|c| c.url == card.url).and_then(|c| c.poster.clone()));
        }
        self.history.retain(|h| h.card.url != card.url && !crate::text::same_show(&h.card.title, &card.title));
        self.history.insert(
            0,
            HistoryEntry {
                card,
                ep_num,
                ep_label: ep_label.into(),
                ep_url: ep_url.into(),
                when: now(),
                dub,
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

    /// How much of an episode was watched (0..=1), from the player's saved position.
    pub fn fraction(&self, ep_url: &str) -> Option<f32> {
        self.progress.get(ep_url).filter(|p| p.dur > 0.0).map(|p| (p.pos / p.dur).clamp(0.0, 1.0) as f32)
    }

    pub fn last_watched(&self, url: &str, title: &str) -> Option<&HistoryEntry> {
        self.history.iter().find(|h| h.card.url == url).or_else(|| self.history.iter().find(|h| crate::text::same_show(&h.card.title, title)))
    }

    /// Forget a show's history entry (it was recorded but never really played).
    pub fn unwatch(&mut self, ep_url: &str) {
        let n = self.history.len();
        self.history.retain(|h| h.ep_url != ep_url);
        if self.history.len() != n {
            self.save();
        }
    }

    /// Clean-up at start: older versions recorded an entry per site (also for sites that failed to play), so the
    /// same show could sit in Continue Watching several times. Keep the newest entry of each show; of entries
    /// equally new, prefer one with a saved position.
    pub fn dedupe_history(&mut self) {
        let mut kept: Vec<HistoryEntry> = Vec::new();
        let mut all = std::mem::take(&mut self.history);
        all.sort_by(|a, b| b.when.cmp(&a.when));
        for h in all {
            match kept.iter_mut().find(|k| crate::text::same_show(&k.card.title, &h.card.title)) {
                Some(k) => {
                    // the newer one stays unless it never played and the older one did
                    let played = |e: &HistoryEntry, lib: &std::collections::HashMap<String, Progress>| lib.get(&e.ep_url).is_some_and(|p| p.pos > 5.0);
                    if !played(k, &self.progress) && played(&h, &self.progress) {
                        *k = h;
                    }
                }
                None => kept.push(h),
            }
        }
        kept.sort_by(|a, b| b.when.cmp(&a.when));
        self.history = kept;
    }
}
