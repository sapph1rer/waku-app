//! trace.moe: which anime / episode / moment a screenshot comes from (free tier: a few searches a minute).

use crate::http::Http;
use anyhow::{Result, anyhow};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct SceneHit {
    pub anilist_id: u64,
    pub mal_id: Option<u64>,
    pub title_romaji: String,
    pub title_english: Option<String>,
    pub title_native: Option<String>,
    pub adult: bool,
    /// None for movies / when trace.moe does not know
    pub episode: Option<f32>,
    /// seconds into the episode
    pub at: f64,
    /// 0..1
    pub similarity: f32,
    /// a still of the matched moment
    pub image: Option<String>,
}

impl SceneHit {
    pub fn title(&self, english: bool) -> String {
        if english
            && let Some(t) = &self.title_english
        {
            return t.clone();
        }
        self.title_romaji.clone()
    }
}

/// Look `image` up (jpg / png / webp / gif bytes). Best match first; weak matches (< 80 %) dropped.
pub fn search_image(http: &Http, image: Vec<u8>) -> Result<Vec<SceneHit>> {
    let ct = sniff(&image);
    let v = http.post_bytes_json("https://api.trace.moe/search?anilistInfo&cutBorders", ct, image)?;
    if let Some(e) = v.get("error").and_then(|x| x.as_str()).filter(|e| !e.is_empty()) {
        return Err(anyhow!("trace.moe: {e}"));
    }
    Ok(parse(&v))
}

fn sniff(b: &[u8]) -> &'static str {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if b.starts_with(b"GIF8") {
        "image/gif"
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/jpeg"
    }
}

pub fn parse(v: &Value) -> Vec<SceneHit> {
    let mut out: Vec<SceneHit> = Vec::new();
    for r in v.get("result").and_then(|x| x.as_array()).into_iter().flatten() {
        let a = &r["anilist"];
        let Some(id) = a["id"].as_u64().or_else(|| a.as_u64()) else { continue };
        let sim = r["similarity"].as_f64().unwrap_or(0.0) as f32;
        if sim < 0.8 {
            continue;
        }
        let episode = r["episode"].as_f64().map(|e| e as f32).or_else(|| r["episode"].as_str().and_then(|s| s.parse().ok()));
        let hit = SceneHit {
            anilist_id: id,
            mal_id: a["idMal"].as_u64(),
            title_romaji: a.pointer("/title/romaji").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            title_english: a.pointer("/title/english").and_then(|x| x.as_str()).map(String::from),
            title_native: a.pointer("/title/native").and_then(|x| x.as_str()).map(String::from),
            adult: a["isAdult"].as_bool() == Some(true),
            episode,
            at: r["from"].as_f64().unwrap_or(0.0).max(0.0),
            similarity: sim,
            image: r["image"].as_str().map(String::from),
        };
        // the same show + episode once (best first already)
        if !out.iter().any(|h| h.anilist_id == hit.anilist_id && h.episode == hit.episode) {
            out.push(hit);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        let v: serde_json::Value = serde_json::from_str(r#"{"frameCount":1,"error":"","result":[
          {"anilist":{"id":154587,"idMal":52991,"title":{"native":"葬送のフリーレン","romaji":"Sousou no Frieren","english":"Frieren: Beyond Journey's End"},"synonyms":[],"isAdult":false},
           "filename":"x.mp4","episode":3,"from":663.5,"to":665.1,"similarity":0.96,"video":"v","image":"https://img"},
          {"anilist":{"id":154587,"idMal":52991,"title":{"romaji":"Sousou no Frieren"},"isAdult":false},"episode":3,"from":10,"similarity":0.9},
          {"anilist":{"id":1,"title":{"romaji":"x"}},"episode":1,"from":1,"similarity":0.5}]}"#).unwrap();
        let h = super::parse(&v);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].episode, Some(3.0));
        assert_eq!(h[0].at, 663.5);
        assert_eq!(h[0].title(true), "Frieren: Beyond Journey's End");
    }
}
