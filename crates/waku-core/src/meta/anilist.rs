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
    nextAiringEpisode { episode airingAt } startDate { year month day } season averageScore genres coverImage { extraLarge large color } bannerImage \
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

/// What to list on Browse › Discover.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Discover {
    pub search: String,
    pub genres: Vec<String>,
    pub year: Option<u32>,
    /// WINTER | SPRING | SUMMER | FALL
    pub season: Option<String>,
    /// TV | MOVIE | OVA | ONA | SPECIAL | TV_SHORT
    pub formats: Vec<String>,
    /// RELEASING | FINISHED | NOT_YET_RELEASED
    pub status: Option<String>,
    /// POPULARITY_DESC | TRENDING_DESC | SCORE_DESC | START_DATE_DESC | FAVOURITES_DESC
    pub sort: String,
    pub min_score: Option<u32>,
}

pub const GENRES: [&str; 18] = [
    "Action", "Adventure", "Comedy", "Drama", "Ecchi", "Fantasy", "Horror", "Mahou Shoujo", "Mecha", "Music", "Mystery",
    "Psychological", "Romance", "Sci-Fi", "Slice of Life", "Sports", "Supernatural", "Thriller",
];

/// One page (30) of AniList anime matching `f`; second value: there are more pages.
pub fn discover(http: &Http, f: &Discover, page: u32) -> Result<(Vec<AnimeMeta>, bool)> {
    let q = format!(
        "query ($p: Int, $s: String, $g: [String], $y: Int, $se: MediaSeason, $f: [MediaFormat], $st: MediaStatus, $so: [MediaSort], $sc: Int) {{ \
         Page(page: $p, perPage: 30) {{ pageInfo {{ hasNextPage }} media(type: ANIME, isAdult: false, search: $s, genre_in: $g, seasonYear: $y, \
         season: $se, format_in: $f, status: $st, sort: $so, averageScore_greater: $sc) {{ {FIELDS} }} }} }}"
    );
    let mut vars = serde_json::Map::new();
    vars.insert("p".into(), json!(page.max(1)));
    if !f.search.trim().is_empty() {
        vars.insert("s".into(), json!(f.search.trim()));
    }
    if !f.genres.is_empty() {
        vars.insert("g".into(), json!(f.genres));
    }
    if let Some(y) = f.year {
        vars.insert("y".into(), json!(y));
    }
    if let Some(se) = &f.season {
        vars.insert("se".into(), json!(se));
    }
    if !f.formats.is_empty() {
        vars.insert("f".into(), json!(f.formats));
    }
    if let Some(st) = &f.status {
        vars.insert("st".into(), json!(st));
    }
    let sort = if f.sort.is_empty() { "POPULARITY_DESC" } else { f.sort.as_str() };
    // a text search is best ranked by relevance
    let sort = if f.search.trim().is_empty() { vec![sort] } else { vec!["SEARCH_MATCH", sort] };
    vars.insert("so".into(), json!(sort));
    if let Some(sc) = f.min_score {
        vars.insert("sc".into(), json!(sc));
    }
    let v = post(http, &q, Value::Object(vars))?;
    let more = v.pointer("/data/Page/pageInfo/hasNextPage").and_then(|x| x.as_bool()).unwrap_or(false);
    Ok((parse_page(&v)?, more))
}

/// One entry of a franchise's watch order.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct OrderItem {
    pub meta: AnimeMeta,
    /// how AniList relates it to the show it was reached from (SEQUEL, SIDE_STORY …)
    pub relation: String,
    /// part of the main story (TV / movie / ONA sequels and prequels), not a side story or recap
    pub main: bool,
}

const REL_FIELDS: &str = "relations { edges { relationType node { id type format startDate { year month day } } } }";

/// The whole franchise around `id` (prequels, sequels, movies, OVAs, side stories), in air-date order.
pub fn watch_order(http: &Http, id: u64) -> Result<Vec<OrderItem>> {
    use std::collections::{HashMap, VecDeque};
    let q = format!("query ($id: Int) {{ Media(id: $id, type: ANIME) {{ {FIELDS} {REL_FIELDS} }} }}");
    let mut seen: HashMap<u64, (AnimeMeta, String, bool)> = HashMap::new();
    let mut queue: VecDeque<(u64, String, bool)> = VecDeque::from([(id, "SOURCE".to_string(), true)]);
    let mut asked = 0;
    while let Some((cur, rel, main)) = queue.pop_front() {
        if seen.contains_key(&cur) {
            continue;
        }
        if asked >= 16 {
            break; // long franchises: enough for a list (AniList allows ~90 requests a minute)
        }
        asked += 1;
        let v = post(http, &q, json!({ "id": cur }))?;
        let Some(m) = v.pointer("/data/Media") else { continue };
        let meta = parse_media(m);
        seen.insert(cur, (meta, rel.clone(), main));
        for e in m.pointer("/relations/edges").and_then(|x| x.as_array()).into_iter().flatten() {
            let rt = e["relationType"].as_str().unwrap_or("");
            let node = &e["node"];
            if node["type"].as_str() != Some("ANIME") {
                continue;
            }
            let Some(nid) = node["id"].as_u64() else { continue };
            if seen.contains_key(&nid) {
                continue;
            }
            let fmt = node["format"].as_str().unwrap_or("");
            // follow the story: prequels / sequels everywhere, side stories and the parent from the main line
            let (follow, is_main) = match rt {
                "PREQUEL" | "SEQUEL" => (true, main && matches!(fmt, "TV" | "TV_SHORT" | "MOVIE" | "ONA")),
                "PARENT" | "SIDE_STORY" | "ALTERNATIVE" | "SPIN_OFF" if main => (rt != "SPIN_OFF" || fmt == "MOVIE", false),
                _ => (false, false),
            };
            if follow && fmt != "MUSIC" {
                queue.push_back((nid, rt.to_string(), is_main));
            }
        }
    }
    let mut out: Vec<OrderItem> = seen.into_values().map(|(meta, relation, main)| OrderItem { meta, relation, main }).collect();
    out.sort_by_key(|x| (x.meta.start.unwrap_or(u32::MAX), x.meta.anilist_id.unwrap_or(0)));
    Ok(out)
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
        start: n(m, "/startDate/year").map(|y| y as u32 * 10_000 + n(m, "/startDate/month").unwrap_or(0) as u32 * 100 + n(m, "/startDate/day").unwrap_or(0) as u32),
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
                summary: None,
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
