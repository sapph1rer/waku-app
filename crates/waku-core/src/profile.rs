//! Site profiles: everything the generic engine needs to know about one site.
//! Loaded from `sources.toml` so selectors / domains can be fixed without a rebuild.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_SOURCES: &str = include_str!("../sources.default.toml");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteProfile {
    pub id: String,
    pub name: String,
    pub base: String,
    pub enabled: bool,
    pub note: String,
    /// Listing pages for "latest" (page N = `<path>page/N/`).
    pub latest_paths: Vec<String>,
    /// `{q}` is replaced by the url-encoded query.
    pub search_path: String,
    /// Regex on the decoded URL path of a series page ("" = heuristic).
    pub series_path: String,
    /// Regex on the decoded URL path of an episode page ("" = heuristic).
    pub episode_path: String,
    /// Element wrapping one anime card in listings ("" = any link with a poster).
    pub card_selector: String,
    pub title_selector: String,
    pub poster_selector: String,
    pub synopsis_selector: String,
    pub genre_selector: String,
    /// Regex removing suffixes such as "ตอนที่ 1-12 ซับไทย" from titles.
    pub title_strip: String,
    /// Build a local title index from the sitemap (better search).
    pub sitemap_index: bool,
    /// Extra first path segments that are never anime pages.
    pub ignore_paths: Vec<String>,
    /// 18+ (hentai): loaded only when "Adult sources" is on in Settings.
    pub adult: bool,
}

impl Default for SiteProfile {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            base: String::new(),
            enabled: true,
            note: String::new(),
            latest_paths: vec!["/".into()],
            search_path: "/?s={q}".into(),
            series_path: String::new(),
            episode_path: String::new(),
            card_selector: String::new(),
            title_selector: String::new(),
            poster_selector: String::new(),
            synopsis_selector: String::new(),
            genre_selector: String::new(),
            title_strip: String::new(),
            sitemap_index: false,
            ignore_paths: Vec::new(),
            adult: false,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SourcesFile {
    /// bumped whenever the rules in the repository change (the app takes a higher one)
    #[serde(default)]
    pub revision: u32,
    #[serde(default)]
    pub source: Vec<SiteProfile>,
}

/// The `revision` of a sources file (0 = none / unreadable).
pub fn sources_revision(text: &str) -> u32 {
    toml::from_str::<SourcesFile>(text).map(|f| f.revision).unwrap_or(0)
}

fn applied_path() -> PathBuf {
    config_dir().join("sources.applied")
}

fn text_hash(t: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(t.replace("\r\n", "\n").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Remember that the app itself wrote this text (so a later update may replace it without asking).
fn mark_applied(text: &str) {
    let _ = std::fs::write(applied_path(), text_hash(text));
}

/// The user's sources.toml: (revision, edited by hand since the app last wrote it).
pub fn local_sources_state() -> (u32, bool) {
    let Ok(t) = std::fs::read_to_string(sources_path()) else { return (0, false) };
    let applied = std::fs::read_to_string(applied_path()).unwrap_or_default();
    // copies written before update tracking: unedited when they are exactly a built-in default
    let pristine = applied.trim() == text_hash(&t) || t.replace("\r\n", "\n") == DEFAULT_SOURCES.replace("\r\n", "\n");
    (sources_revision(&t), !pristine)
}

/// The rules file as published in the repository (`owner/repo`, branch main).
pub fn sources_url(repo: &str) -> String {
    format!("https://raw.githubusercontent.com/{}/main/crates/waku-core/sources.default.toml", repo.trim().trim_matches('/'))
}

/// Newer rules in the repository than on this PC: Some((text, revision)).
pub fn check_remote_sources(http: &crate::http::Http, repo: &str) -> Result<Option<(String, u32)>> {
    let text = http.get_plain(&sources_url(repo), None)?;
    let list = parse_sources(&text)?;
    if list.is_empty() {
        anyhow::bail!("the repository's sources file has no sources");
    }
    let remote = sources_revision(&text);
    let (local, _) = local_sources_state();
    Ok((remote > local).then_some((text, remote)))
}

/// Write new rules as the user's sources.toml; `backup` keeps the old file as `sources.toml.bak-<time>`.
pub fn apply_sources(text: &str, backup: bool) -> Result<Option<PathBuf>> {
    parse_sources(text)?;
    let p = sources_path();
    let mut saved = None;
    if backup && p.exists() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let b = p.with_extension(format!("toml.bak-{t}"));
        std::fs::copy(&p, &b)?;
        saved = Some(b);
    }
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&p, text)?;
    mark_applied(text);
    Ok(saved)
}

pub fn parse_sources(text: &str) -> Result<Vec<SiteProfile>> {
    let f: SourcesFile = toml::from_str(text).context("sources.toml: invalid TOML")?;
    let mut out = Vec::new();
    for mut p in f.source {
        p.base = p.base.trim_end_matches('/').to_string();
        if p.id.is_empty() || p.base.is_empty() {
            continue;
        }
        if p.name.is_empty() {
            p.name = p.id.clone();
        }
        out.push(p);
    }
    Ok(out)
}

/// Sources that no longer serve videos: (id, reason shown in Settings).
pub const RETIRED: &[(&str, &str)] =
    &[("hareshi", "turned off: hareshi.net is now a directory of licensed streaming services (no videos to play)")];

/// `%APPDATA%\waku` on Windows, `~/.config/waku` elsewhere.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("waku")
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("waku")
}

pub fn sources_path() -> PathBuf {
    config_dir().join("sources.toml")
}

/// Read the user's `sources.toml`, creating it from the defaults on first run.
/// Sources added to the defaults later are appended (by id) so updates reach users.
pub fn load_sources(path: &Path) -> Result<Vec<SiteProfile>> {
    if !path.exists() {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, DEFAULT_SOURCES)?;
        if path == sources_path() {
            mark_applied(DEFAULT_SOURCES);
        }
    }
    let text = std::fs::read_to_string(path)?;
    let mut list = parse_sources(&text)?;
    // sites that stopped hosting videos are switched off even in older copies of sources.toml
    for p in list.iter_mut() {
        if let Some((_, why)) = RETIRED.iter().find(|(id, _)| *id == p.id) {
            p.enabled = false;
            p.note = why.to_string();
        }
    }
    for d in parse_sources(DEFAULT_SOURCES)? {
        if !list.iter().any(|p| p.id == d.id) {
            list.push(d);
        }
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions() {
        assert_eq!(sources_revision(DEFAULT_SOURCES), 2);
        assert_eq!(sources_revision("[[source]]\nid='x'\nbase='https://x'"), 0);
        assert_eq!(text_hash("a\r\nb"), text_hash("a\nb"), "line endings do not count as an edit");
        assert!(sources_url("sapph1rer/waku-app/").ends_with("/sapph1rer/waku-app/main/crates/waku-core/sources.default.toml"));
    }

    #[test]
    fn defaults_parse() {
        let v = parse_sources(DEFAULT_SOURCES).unwrap();
        assert_eq!(v.len(), 9);
        assert_eq!(v[0].id, "waku");
        // the 18+ ones are marked (they load only when turned on)
        assert_eq!(v.iter().filter(|p| p.adult).map(|p| p.id.as_str()).collect::<Vec<_>>(), ["akih", "alphahen", "animeh"]);
        assert!(v.iter().all(|p| !p.base.ends_with('/')));
        for p in &v {
            for re in [&p.series_path, &p.episode_path, &p.title_strip] {
                if !re.is_empty() {
                    regex::Regex::new(re).unwrap();
                }
            }
        }
    }
}
