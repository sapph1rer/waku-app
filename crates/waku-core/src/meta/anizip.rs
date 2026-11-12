//! ani.zip (https://api.ani.zip/mappings): episode titles (English / romaji / Japanese), thumbnails, air dates and
//! runtime, from TheTVDB / TMDB, looked up by AniList (or MyAnimeList) id. No key needed; answers in a few hundred
//! ms and knows new seasons that Kitsu / Jikan have no titles for yet.

use super::{EpMeta, base};
use crate::http::Http;
use anyhow::Result;
use serde_json::Value;

pub fn episodes(http: &Http, anilist_id: Option<u64>, mal_id: Option<u64>) -> Result<Vec<EpMeta>> {
    let q = match (anilist_id, mal_id) {
        (Some(a), _) => format!("anilist_id={a}"),
        (None, Some(m)) => format!("mal_id={m}"),
        _ => return Ok(Vec::new()),
    };
    let url = format!("{}/mappings?{q}", base("WAKU_ANIZIP", "https://api.ani.zip"));
    Ok(parse(&http.get_json(&url, &[])?))
}

pub fn parse(v: &Value) -> Vec<EpMeta> {
    let Some(eps) = v.get("episodes").and_then(|x| x.as_object()) else { return Vec::new() };
    let mut out: Vec<EpMeta> = eps
        .iter()
        .filter_map(|(k, e)| {
            // regular episodes are keyed "1", "2" …; specials ("S1") are left out
            let number: u32 = k.parse().ok().or_else(|| e.get("episodeNumber").and_then(|x| x.as_u64()).map(|x| x as u32))?;
            let t = e.get("title");
            let pick = |lang: &str| t.and_then(|t| t.get(lang)).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
            let title = pick("en").or_else(|| pick("x-jat")).or_else(|| pick("ja")).unwrap_or_default();
            // "Episode 3" style placeholders say nothing
            let title = if title.to_lowercase().trim_start_matches("episode").trim().parse::<u32>().is_ok() { String::new() } else { title };
            Some(EpMeta {
                number,
                title,
                thumbnail: e.get("image").and_then(|x| x.as_str()).filter(|s| s.starts_with("http")).map(str::to_string),
                aired: e.get("airDate").or_else(|| e.get("airdate")).and_then(|x| x.as_str()).map(|s| s.chars().take(10).collect()),
                length: e.get("length").or_else(|| e.get("runtime")).and_then(|x| x.as_u64()).map(|x| x as u32),
                source: "anizip".into(),
                summary: e.get("overview").or_else(|| e.get("summary")).and_then(|x| x.as_str()).map(|t| crate::text::strip_tags(t).trim().to_string()).filter(|t| t.len() > 10),
            })
        })
        .collect();
    out.sort_by_key(|e| e.number);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        let v: serde_json::Value = serde_json::from_str(r#"{"episodes":{"1":{"title":{"ja":"春","en":"The Hazy Spring Moon Above","x-jat":"Haru"},"image":"https://a/1.jpg","airDate":"2025-10-07","length":24},"2":{"title":{"en":"Episode 2","x-jat":"Sougo"}},"S1":{"title":{"en":"Special"}}}}"#).unwrap();
        let e = super::parse(&v);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].title, "The Hazy Spring Moon Above");
        assert_eq!(e[0].thumbnail.as_deref(), Some("https://a/1.jpg"));
        assert_eq!(e[1].title, ""); // "Episode 2" is no title; x-jat is not used when en exists
    }
}
