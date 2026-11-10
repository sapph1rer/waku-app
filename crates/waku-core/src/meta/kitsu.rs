//! Kitsu (https://kitsu.io/api/edge): episode titles, air dates, runtime and episode thumbnails.
//! Found through Kitsu's id mappings (AniList id first, then MyAnimeList). No key needed. Ported from AnimeShelf.

use super::{EpMeta, base, throttle};
use crate::http::Http;
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::time::Duration;

const GAP: Duration = Duration::from_millis(250);
const HDR: &[(&str, &str)] = &[("Accept", "application/vnd.api+json")];

fn root() -> String {
    base("WAKU_KITSU", "https://kitsu.io/api/edge")
}

pub fn find_id(
    http: &Http,
    anilist_id: Option<u64>,
    mal_id: Option<u64>,
) -> Result<Option<String>> {
    let mut last_err = None;
    for (site, id) in [
        ("anilist%2Fanime", anilist_id),
        ("myanimelist%2Fanime", mal_id),
    ] {
        let Some(id) = id.filter(|i| *i > 0) else {
            continue;
        };
        let url = format!(
            "{}/mappings?filter%5BexternalSite%5D={site}&filter%5BexternalId%5D={id}&include=item",
            root()
        );
        throttle("kitsu", GAP);
        match http.get_json(&url, HDR) {
            Ok(v) => {
                if let Some(k) = parse_mapping(&v) {
                    return Ok(Some(k));
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    match last_err {
        Some(e) => Err(e),
        None => Ok(None),
    }
}

pub fn parse_mapping(v: &Value) -> Option<String> {
    let is_anime = |x: &Value| x.get("type").and_then(|t| t.as_str()) == Some("anime");
    if let Some(inc) = v.get("included").and_then(|i| i.as_array())
        && let Some(a) = inc.iter().find(|x| is_anime(x))
    {
        return a.get("id").and_then(|i| i.as_str()).map(String::from);
    }
    v.pointer("/data/0/relationships/item/data")
        .filter(|x| is_anime(x))
        .and_then(|x| x.get("id"))
        .and_then(|i| i.as_str())
        .map(String::from)
}

pub fn episodes(http: &Http, kitsu_id: &str) -> Result<Vec<EpMeta>> {
    let mut all = vec![];
    for page in 0..40 {
        let url = format!(
            "{}/anime/{kitsu_id}/episodes?page%5Blimit%5D=20&page%5Boffset%5D={}&sort=number",
            root(),
            page * 20
        );
        throttle("kitsu", GAP);
        let (eps, more) = parse_episodes(&http.get_json(&url, HDR)?)?;
        all.extend(eps);
        if !more {
            break;
        }
    }
    Ok(all)
}

fn generic(title: &str, n: u32) -> bool {
    let t = title.trim().to_lowercase();
    t.is_empty()
        || t == format!("episode {n}")
        || t == format!("ep {n}")
        || t == format!("ep. {n}")
        || t == format!("{n}")
}

pub fn parse_episodes(v: &Value) -> Result<(Vec<EpMeta>, bool)> {
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow!("Kitsu: unexpected response"))?;
    let mut out = vec![];
    for e in data {
        let Some(a) = e.get("attributes") else {
            continue;
        };
        let Some(number) = a.get("number").and_then(|n| n.as_u64()) else {
            continue;
        };
        let number = number as u32;
        let pick = |p: &str| {
            a.pointer(p)
                .and_then(|x| x.as_str())
                .map(|x| x.trim().to_string())
                .filter(|x| !generic(x, number))
        };
        let title = pick("/titles/en_us")
            .or_else(|| pick("/titles/en"))
            .or_else(|| pick("/canonicalTitle"))
            .or_else(|| pick("/titles/en_jp"))
            .unwrap_or_default();
        let thumb = [
            "/thumbnail/large",
            "/thumbnail/original",
            "/thumbnail/medium",
            "/thumbnail/small",
        ]
        .iter()
        .find_map(|p| {
            a.pointer(p)
                .and_then(|x| x.as_str())
                .filter(|x| !x.is_empty())
                .map(String::from)
        });
        let aired = a
            .get("airdate")
            .and_then(|x| x.as_str())
            .map(|d| d.chars().take(10).collect::<String>())
            .filter(|d| !d.is_empty());
        let length = a
            .get("length")
            .and_then(|x| x.as_u64())
            .map(|x| x as u32)
            .filter(|x| *x > 0);
        out.push(EpMeta {
            number,
            title,
            thumbnail: thumb,
            aired,
            length,
            source: "kitsu".into(),
        });
    }
    let more = v
        .pointer("/links/next")
        .is_some_and(|n| n.as_str().is_some_and(|s| !s.is_empty()))
        && !data.is_empty();
    Ok((out, more))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        let v: Value = serde_json::from_str(r#"{"data":[{"id":"1","type":"mappings","relationships":{"item":{"data":{"type":"anime","id":"42422"}}}}],"included":[{"id":"42422","type":"anime","attributes":{}}]}"#).unwrap();
        assert_eq!(parse_mapping(&v).as_deref(), Some("42422"));
        assert_eq!(
            parse_mapping(&serde_json::from_str(r#"{"data":[]}"#).unwrap()),
            None
        );
    }

    #[test]
    fn episode_page() {
        let v: Value = serde_json::from_str(
            r#"{"data":[
              {"id":"1","attributes":{"number":1,"canonicalTitle":"Episode 1","titles":{"en_jp":"Bouken no Owari","en_us":"The Journey's End"},"airdate":"2023-09-29","length":24,
               "thumbnail":{"original":"https://media.kitsu.io/o.jpg","large":"https://media.kitsu.io/l.jpg"}}},
              {"id":"2","attributes":{"number":2,"canonicalTitle":"Episode 2","titles":{},"thumbnail":null}}],
             "links":{"next":"https://kitsu.io/api/edge/anime/1/episodes?page[offset]=20"}}"#,
        )
        .unwrap();
        let (e, more) = parse_episodes(&v).unwrap();
        assert!(more);
        assert_eq!(e[0].title, "The Journey's End");
        assert_eq!(
            e[0].thumbnail.as_deref(),
            Some("https://media.kitsu.io/l.jpg")
        );
        assert_eq!(e[0].length, Some(24));
        assert_eq!(e[1].title, "");
    }
}
