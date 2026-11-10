//! Jikan (unofficial MyAnimeList API, https://api.jikan.moe/v4): episode titles + air dates. Ported from AnimeShelf.

use super::{EpMeta, base, throttle};
use crate::http::Http;
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::time::Duration;

const GAP: Duration = Duration::from_millis(450); // 3 requests / second

pub fn episodes(http: &Http, mal_id: u64) -> Result<Vec<EpMeta>> {
    let mut all = vec![];
    for page in 1..=15 {
        let url = format!(
            "{}/anime/{mal_id}/episodes?page={page}",
            base("WAKU_JIKAN", "https://api.jikan.moe/v4")
        );
        throttle("jikan", GAP);
        let (eps, more) = parse_page(&http.get_json(&url, &[])?)?;
        all.extend(eps);
        if !more {
            break;
        }
    }
    Ok(all)
}

pub fn parse_page(v: &Value) -> Result<(Vec<EpMeta>, bool)> {
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow!("Jikan: unexpected response"))?;
    let eps = data
        .iter()
        .filter_map(|e| {
            let number = e.get("mal_id")?.as_u64()? as u32;
            let title = e
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let aired = e
                .get("aired")
                .and_then(|t| t.as_str())
                .map(|d| d.chars().take(10).collect());
            Some(EpMeta {
                number,
                title,
                thumbnail: None,
                aired,
                length: None,
                source: "jikan".into(),
            })
        })
        .collect();
    let more = v
        .pointer("/pagination/has_next_page")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    Ok((eps, more))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses() {
        let v: Value = serde_json::from_str(r#"{"pagination":{"has_next_page":false},"data":[{"mal_id":1,"title":"The Journey's End","aired":"2023-09-29T00:00:00+00:00"},{"mal_id":2,"title":"It Didn't Have to Be Magic"}]}"#).unwrap();
        let (e, more) = parse_page(&v).unwrap();
        assert!(!more);
        assert_eq!(e[0].aired.as_deref(), Some("2023-09-29"));
        assert_eq!(e[1].number, 2);
    }
}
