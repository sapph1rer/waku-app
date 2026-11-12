//! AniSkip (api.aniskip.com): community-timed openings / endings by MyAnimeList id + episode number.

use crate::http::Http;
use anyhow::Result;
use serde_json::Value;

/// One range to jump over: kind "op" / "ed", start, end (seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct Skip {
    pub kind: String,
    pub start: f64,
    pub end: f64,
}

/// Opening / ending of episode `ep`. `len` = the video's length in seconds when known (0 = any): the times
/// were taken on some release of the episode, a length far off means a different cut.
pub fn skip_times(http: &Http, mal_id: u64, ep: u32, len: f64) -> Result<Vec<Skip>> {
    let url = format!(
        "https://api.aniskip.com/v2/skip-times/{mal_id}/{ep}?types[]=op&types[]=ed&types[]=mixed-op&types[]=mixed-ed&episodeLength={}",
        len.max(0.0).round() as u64
    );
    let t = http.get_plain(&url, None)?;
    Ok(parse(&serde_json::from_str(&t)?, len))
}

pub fn parse(v: &Value, len: f64) -> Vec<Skip> {
    let mut out: Vec<Skip> = Vec::new();
    for r in v.get("results").and_then(|x| x.as_array()).into_iter().flatten() {
        let iv = &r["interval"];
        let (Some(a), Some(b)) = (iv["startTime"].as_f64(), iv["endTime"].as_f64()) else { continue };
        let kind = match r["skipType"].as_str().unwrap_or("") {
            "op" | "mixed-op" => "op",
            "ed" | "mixed-ed" => "ed",
            _ => continue,
        };
        let el = r["episodeLength"].as_f64().unwrap_or(0.0);
        if len > 0.0 && el > 0.0 && (el - len).abs() > 25.0 {
            continue; // timed on a different cut
        }
        if b <= a || b - a > 240.0 || out.iter().any(|x| x.kind == kind) {
            continue;
        }
        out.push(Skip { kind: kind.into(), start: a, end: b });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        let v: serde_json::Value = serde_json::from_str(r#"{"found":true,"results":[
            {"interval":{"startTime":88.2,"endTime":178.1},"skipType":"op","skipId":"x","episodeLength":1420.0},
            {"interval":{"startTime":1300.0,"endTime":1390.0},"skipType":"ed","skipId":"y","episodeLength":1500.0},
            {"interval":{"startTime":1310.0,"endTime":1400.0},"skipType":"ed","skipId":"z","episodeLength":1421.0}]}"#).unwrap();
        let s = super::parse(&v, 1420.0);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].kind, "op");
        assert_eq!(s[1].start, 1310.0);
    }
}
