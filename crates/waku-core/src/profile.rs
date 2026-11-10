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
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SourcesFile {
    #[serde(default)]
    pub source: Vec<SiteProfile>,
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
    }
    let text = std::fs::read_to_string(path)?;
    let mut list = parse_sources(&text)?;
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
    fn defaults_parse() {
        let v = parse_sources(DEFAULT_SOURCES).unwrap();
        assert_eq!(v.len(), 6);
        assert_eq!(v[0].id, "waku");
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
