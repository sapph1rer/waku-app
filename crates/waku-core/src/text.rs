//! String / URL helpers and the fuzzy title matcher (from Waku-cli).

use regex::Regex;
use std::sync::OnceLock;
use url::Url;

pub fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(n.saturating_sub(1)).collect::<String>()
        )
    }
}

pub fn fmt_num(n: f32) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as u32)
    } else {
        format!("{n}")
    }
}

/// Resolve `href` against `base_url` ("//host/x" -> https).
pub fn abs(base_url: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty()
        || href.starts_with('#')
        || href.starts_with("javascript:")
        || href.starts_with("data:")
    {
        return None;
    }
    if href.starts_with("//") {
        return Some(format!("https:{href}"));
    }
    Url::parse(base_url)
        .ok()?
        .join(href)
        .ok()
        .map(|u| u.to_string())
}

pub fn path_segs(url: &str) -> Vec<String> {
    Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .map(|s| s.filter(|x| !x.is_empty()).map(String::from).collect())
        })
        .unwrap_or_default()
}

/// Decoded path, e.g. "/anime/frieren/" (percent-decoding keeps Thai readable).
pub fn url_path(url: &str) -> String {
    let p = Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    percent_decode(&p)
}

pub fn host(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
        .unwrap_or_default()
}

pub fn same_site(a: &str, b: &str) -> bool {
    let (ha, hb) = (host(a), host(b));
    !ha.is_empty()
        && (ha == hb || ha.ends_with(&format!(".{hb}")) || hb.ends_with(&format!(".{ha}")))
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `https://host[:port]/` — what a browser sends as Referer across origins.
pub fn origin_of(url: &str) -> String {
    match Url::parse(url) {
        Ok(u) => match (u.host_str(), u.port()) {
            (Some(h), Some(p)) => format!("{}://{h}:{p}/", u.scheme()),
            (Some(h), None) => format!("{}://{h}/", u.scheme()),
            _ => url.to_string(),
        },
        Err(_) => url.to_string(),
    }
}

pub fn unescape_html(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&#038;", "&")
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&#8211;", "–")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

// ------------------------------------------------------------------ episodes

/// (season, episode) from a label such as "1 - 5", "ตอนที่ 12", "EP.3", "Episode 7.5".
pub fn parse_episode(text: &str) -> Option<(Option<u32>, f32)> {
    static SE: OnceLock<Regex> = OnceLock::new();
    static EP: OnceLock<Regex> = OnceLock::new();
    let se = SE.get_or_init(|| Regex::new(r"^\s*(\d+)\s*-\s*(\d+(?:\.\d+)?)\b").unwrap());
    let ep = EP.get_or_init(|| {
        Regex::new(r"(?i)(?:ตอนที่|ตอน|episode|ep\.?|อีพี)\s*[:#.]?\s*(\d+(?:\.\d+)?)").unwrap()
    });
    if let Some(m) = se.captures(text) {
        return Some((m[1].parse().ok(), m[2].parse().ok()?));
    }
    ep.captures(text)
        .and_then(|m| m[1].parse().ok())
        .map(|n| (None, n))
}

pub fn looks_like_episode(text: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(ตอนที่|ตอน\s*\d|\bep\.?\s*\d|episode\s*\d|อีพี)").unwrap())
        .is_match(text)
}

// ------------------------------------------------------------------ ranking

pub fn norm(s: &str) -> String {
    let t: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    clean(&t)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn word_sim(q: &str, h: &str) -> f32 {
    if q == h {
        return 1.0;
    }
    if !q.is_ascii() {
        // Thai etc. has no word breaks: plain substring match
        return if h.contains(q) { 0.9 } else { 0.0 };
    }
    let ql = q.chars().count();
    let hl = h.chars().count();
    if ql >= 3 && h.starts_with(q) {
        return 0.65;
    }
    if ql == 2 && h.starts_with(q) {
        return 0.3;
    }
    let maxd = if ql >= 8 {
        2
    } else if ql >= 4 {
        1
    } else {
        0
    };
    if maxd > 0 && hl.abs_diff(ql) <= maxd {
        let d = levenshtein(q, h);
        if d <= maxd {
            return 0.85 - 0.1 * (d as f32 - 1.0).max(0.0);
        }
    }
    0.0
}

/// 0.0 ..= 1.0 — how well `hay` matches `query`.
pub fn score(query: &str, hay: &str) -> f32 {
    let q = norm(query);
    let h = norm(hay);
    let qw: Vec<&str> = q.split(' ').filter(|x| !x.is_empty()).collect();
    let hw: Vec<&str> = h.split(' ').filter(|x| !x.is_empty()).collect();
    if qw.is_empty() || hw.is_empty() {
        return 0.0;
    }
    let sum: f32 = qw
        .iter()
        .map(|t| hw.iter().map(|w| word_sim(t, w)).fold(0.0, f32::max))
        .sum();
    let mut s = sum / qw.len() as f32;
    if format!(" {h} ").contains(&format!(" {q} ")) {
        s += 0.25;
    } else if qw.len() > 1 {
        let mut pos = 0;
        let mut ok = true;
        for t in &qw {
            match hw[pos..].iter().position(|w| word_sim(t, w) >= 0.65) {
                Some(i) => pos += i + 1,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            s += 0.1;
        }
    }
    let qc = qw.join("");
    let hc = hw.join("");
    if qc.chars().count() >= 4 && hc.contains(&qc) {
        s = s.max(0.9);
    }
    s.min(1.0)
}

/// Symmetric title similarity for matching the same anime across sources.
/// Season markers must agree ("season 2" vs nothing = penalised).
pub fn title_similarity(a: &str, b: &str) -> f32 {
    let s = score(a, b).min(score(b, a).max(score(a, b) * 0.85));
    let sa = season_of(a);
    let sb = season_of(b);
    if sa.unwrap_or(1) != sb.unwrap_or(1) {
        s * 0.5
    } else {
        s
    }
}

pub fn season_of(title: &str) -> Option<u32> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)(?:season|ภาค|ซีซั่น|\bs)\s*(\d{1,2})\b|(\d)(?:st|nd|rd|th)\s+season")
            .unwrap()
    });
    let c = re.captures(title)?;
    c.get(1).or(c.get(2))?.as_str().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episodes() {
        assert_eq!(parse_episode("1 - 5"), Some((Some(1), 5.0)));
        assert_eq!(parse_episode("Frieren ตอนที่ 12 ซับไทย"), Some((None, 12.0)));
        assert_eq!(parse_episode("EP.3"), Some((None, 3.0)));
        assert_eq!(parse_episode("Episode 7.5"), Some((None, 7.5)));
        assert_eq!(parse_episode("ดูอนิเมะ"), None);
    }

    #[test]
    fn fuzzy() {
        assert!(score("re zero", "Re:Zero kara Hajimeru") > 0.8);
        assert!(score("rezero", "Re Zero") >= 0.9);
        assert!(score("frieren", "Sousou no Frieren") > 0.8);
        assert!(score("naruto", "Bleach") < 0.3);
        assert!(title_similarity("Grand Blue Season 3", "Grand Blue") < 0.6);
        assert!(title_similarity("Sousou no Frieren", "Sousou no Frieren ซับไทย") > 0.8);
    }

    #[test]
    fn urls() {
        assert_eq!(
            url_path("https://a.com/%E0%B8%95%E0%B8%AD%E0%B8%99/"),
            "/ตอน/"
        );
        assert!(same_site("https://www.a.com/x", "https://a.com/"));
        assert_eq!(
            abs("https://a.com/x/", "//cdn.b.com/i.jpg").unwrap(),
            "https://cdn.b.com/i.jpg"
        );
    }
}
