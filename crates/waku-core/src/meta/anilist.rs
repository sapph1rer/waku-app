//! AniList GraphQL (https://graphql.anilist.co): cover, banner, description, genres, score,
//! episode titles + thumbnails (streamingEpisodes) and the public airing schedule.
//! Ported from AnimeShelf.

use super::{AnimeMeta, EpMeta, base, strip_html, throttle};
use crate::http::Http;
use anyhow::{Result, anyhow};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::OnceLock;
use std::time::Duration;

fn url() -> String {
    base("WAKU_ANILIST", "https://graphql.anilist.co")
}
/// AniList allows 90 requests / minute (sometimes 30 when degraded).
const GAP: Duration = Duration::from_millis(700);

const FIELDS: &str = "id idMal title { romaji english native } synonyms description(asHtml: false) format duration status episodes \
    nextAiringEpisode { episode airingAt } startDate { year } season averageScore genres coverImage { extraLarge large color } bannerImage \
    studios(isMain: true) { nodes { name } } streamingEpisodes { title thumbnail } isAdult";

fn post(http: &Http, q: &str, vars: Value) -> Result<Value> {
    throttle("anilist", GAP);
    let v = http.post_json(&url(), &json!({ "query": q, "variables": vars }))?;
    check_errors(&v)?;
    Ok(v)
}

pub fn search(http: &Http, query: &str) -> Result<Vec<AnimeMeta>> {
    let q = format!(
        "query ($s: String) {{ Page(perPage: 10) {{ media(search: $s, type: ANIME) {{ {FIELDS} }} }} }}"
    );
    parse_page(&post(http, &q, json!({ "s": query }))?)
}

pub fn by_id(http: &Http, id: u64) -> Result<AnimeMeta> {
    let q = format!("query ($id: Int) {{ Media(id: $id, type: ANIME) {{ {FIELDS} }} }}");
    let v = post(http, &q, json!({ "id": id }))?;
    v.pointer("/data/Media")
        .filter(|m| m.is_object())
        .map(parse_media)
        .ok_or_else(|| anyhow!("AniList: id {id} not found"))
}

/// One airing slot of the public schedule.
#[derive(Clone, Debug, PartialEq)]
pub struct SchedItem {
    pub id: u64,
    pub title: String,
    pub title_romaji: Option<String>,
    pub cover: Option<String>,
    pub format: Option<String>,
    pub duration: Option<u32>,
    /// AniList average score, 0-100
    pub score: Option<u32>,
    pub total: Option<u32>,
    pub episode: u32,
    pub at: u64,
    pub adult: bool,
    pub country: Option<String>,
}

/// Everything that airs in [from, to) (unix seconds), earliest first.
pub fn schedule(http: &Http, from: u64, to: u64) -> Result<Vec<SchedItem>> {
    let q = "query ($a: Int, $b: Int, $p: Int) { Page(page: $p, perPage: 50) { pageInfo { hasNextPage } \
        airingSchedules(airingAt_greater: $a, airingAt_lesser: $b, sort: TIME) { episode airingAt \
        media { id title { romaji english } coverImage { large } format duration episodes averageScore isAdult countryOfOrigin } } } }";
    let mut out = vec![];
    for page in 1..=10 {
        let v = post(
            http,
            q,
            json!({ "a": from.saturating_sub(1), "b": to, "p": page }),
        )?;
        out.extend(parse_schedule(&v)?);
        if v.pointer("/data/Page/pageInfo/hasNextPage")
            .and_then(|x| x.as_bool())
            != Some(true)
        {
            break;
        }
    }
    Ok(out)
}

pub fn parse_schedule(v: &Value) -> Result<Vec<SchedItem>> {
    check_errors(v)?;
    let arr = v
        .pointer("/data/Page/airingSchedules")
        .and_then(|m| m.as_array())
        .ok_or_else(|| anyhow!("AniList: unexpected response"))?;
    Ok(arr
        .iter()
        .filter_map(|e| {
            let m = e.get("media")?;
            Some(SchedItem {
                id: n(m, "/id")?,
                title: s(m, "/title/english").or_else(|| s(m, "/title/romaji"))?,
                title_romaji: s(m, "/title/romaji"),
                cover: s(m, "/coverImage/large"),
                format: s(m, "/format"),
                duration: n(m, "/duration").map(|x| x as u32),
                score: n(m, "/averageScore").map(|x| x as u32),
                total: n(m, "/episodes").map(|x| x as u32),
                episode: n(e, "/episode")? as u32,
                at: n(e, "/airingAt")?,
                adult: m.get("isAdult").and_then(|x| x.as_bool()) == Some(true),
                country: s(m, "/countryOfOrigin"),
            })
        })
        .collect())
}

fn check_errors(v: &Value) -> Result<()> {
    if let Some(e) = v.get("errors").and_then(|e| e.get(0)) {
        return Err(anyhow!(
            "AniList: {}",
            e.get("message").and_then(|m| m.as_str()).unwrap_or("error")
        ));
    }
    Ok(())
}

pub fn parse_page(v: &Value) -> Result<Vec<AnimeMeta>> {
    check_errors(v)?;
    let arr = v
        .pointer("/data/Page/media")
        .and_then(|m| m.as_array())
        .ok_or_else(|| anyhow!("AniList: unexpected response"))?;
    Ok(arr.iter().map(parse_media).collect())
}

fn s(v: &Value, p: &str) -> Option<String> {
    v.pointer(p)
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
}
fn n(v: &Value, p: &str) -> Option<u64> {
    v.pointer(p).and_then(|x| x.as_u64())
}

fn stream_title() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(r"(?i)^\s*(?:episode|ep\.?|e)\s*(\d{1,4})\s*(?:[-:–—.]\s*(.*))?$").unwrap()
    })
}

pub fn parse_media(m: &Value) -> AnimeMeta {
    let mut a = AnimeMeta {
        anilist_id: n(m, "/id"),
        mal_id: n(m, "/idMal"),
        title_romaji: s(m, "/title/romaji"),
        title_english: s(m, "/title/english"),
        title_native: s(m, "/title/native"),
        synonyms: m
            .get("synonyms")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        description: s(m, "/description")
            .map(|d| strip_html(&d))
            .filter(|d| !d.is_empty()),
        cover_url: s(m, "/coverImage/extraLarge").or_else(|| s(m, "/coverImage/large")),
        banner_url: s(m, "/bannerImage"),
        color: s(m, "/coverImage/color"),
        genres: m
            .get("genres")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        episodes: n(m, "/episodes").map(|x| x as u32),
        status: s(m, "/status"),
        next_airing: n(m, "/nextAiringEpisode/episode")
            .zip(n(m, "/nextAiringEpisode/airingAt"))
            .map(|(e, t)| (e as u32, t)),
        format: s(m, "/format"),
        duration: n(m, "/duration").map(|x| x as u32),
        year: n(m, "/startDate/year").map(|x| x as u32),
        season: s(m, "/season"),
        score: n(m, "/averageScore").map(|x| x as u32),
        studio: m
            .pointer("/studios/nodes/0/name")
            .and_then(|x| x.as_str())
            .map(String::from),
        adult: m.get("isAdult").and_then(|x| x.as_bool()) == Some(true),
        ..Default::default()
    };
    if let Some(list) = m.get("streamingEpisodes").and_then(|x| x.as_array()) {
        for (i, e) in list.iter().enumerate() {
            let raw = e.get("title").and_then(|t| t.as_str()).unwrap_or("");
            let (number, title) = match stream_title().captures(raw) {
                Some(c) => (
                    c[1].parse().unwrap_or(i as u32 + 1),
                    c.get(2)
                        .map(|t| t.as_str().trim().to_string())
                        .unwrap_or_default(),
                ),
                None => (i as u32 + 1, raw.trim().to_string()),
            };
            a.eps.push(EpMeta {
                number,
                title,
                thumbnail: s(e, "/thumbnail"),
                aired: None,
                length: None,
                source: "anilist".into(),
            });
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const SAMPLE: &str = r##"{"data":{"Page":{"media":[{"id":154587,"idMal":52991,
      "title":{"romaji":"Sousou no Frieren","english":"Frieren: Beyond Journey's End","native":"葬送のフリーレン"},
      "synonyms":["Frieren at the Funeral"],"description":"After the party<br><br>defeated the <i>Demon King</i>...",
      "format":"TV","status":"FINISHED","episodes":28,"startDate":{"year":2023},"season":"FALL","averageScore":91,"genres":["Adventure","Drama"],
      "coverImage":{"extraLarge":"https://img/xl.jpg","large":"https://img/l.jpg","color":"#e4a15d"},"bannerImage":null,
      "studios":{"nodes":[{"name":"MADHOUSE"}]},"isAdult":false,
      "streamingEpisodes":[{"title":"Episode 1 - The Journey's End","thumbnail":"https://t/1.jpg"},{"title":"Episode 2","thumbnail":null},{"title":"Special","thumbnail":null}]}]}}}"##;

    #[test]
    fn parses_sample() {
        let v: Value = serde_json::from_str(SAMPLE).unwrap();
        let a = &parse_page(&v).unwrap()[0];
        assert_eq!(
            (a.anilist_id, a.mal_id, a.year),
            (Some(154587), Some(52991), Some(2023))
        );
        assert_eq!(a.studio.as_deref(), Some("MADHOUSE"));
        assert_eq!(a.cover_url.as_deref(), Some("https://img/xl.jpg"));
        assert_eq!(
            a.description.as_deref(),
            Some("After the party\n\ndefeated the Demon King...")
        );
        assert_eq!(
            (a.eps[0].number, a.eps[0].title.as_str()),
            (1, "The Journey's End")
        );
        assert_eq!(a.eps[1].title, "");
        assert_eq!(a.eps[2].number, 3);
    }

    #[test]
    fn parses_schedule() {
        let v: Value = serde_json::from_str(r#"{"data":{"Page":{"pageInfo":{"hasNextPage":false},"airingSchedules":[
          {"episode":4,"airingAt":1792800000,"media":{"id":1,"title":{"romaji":"Tokyo Revengers","english":null},"coverImage":{"large":"u"},"format":"TV","duration":24,"episodes":12,"averageScore":76,"isAdult":false,"countryOfOrigin":"JP"}},
          {"episode":1,"airingAt":1792800100,"media":{"id":2,"title":{"romaji":"X","english":"X"},"format":"TV","isAdult":true}}]}}}"#).unwrap();
        let r = parse_schedule(&v).unwrap();
        assert_eq!(r.len(), 2);
        assert!(!r[0].adult && r[1].adult);
        assert_eq!(
            (
                r[0].id,
                r[0].episode,
                r[0].score,
                r[0].total,
                r[0].title.as_str()
            ),
            (1, 4, Some(76), Some(12), "Tokyo Revengers")
        );
    }

    #[test]
    fn graphql_error() {
        let v: Value =
            serde_json::from_str(r#"{"errors":[{"message":"Too Many Requests."}]}"#).unwrap();
        assert!(parse_page(&v).is_err());
    }
}
