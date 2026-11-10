//! Plain data shared by every source and by the GUI.

use serde::{Deserialize, Serialize};

/// One anime as shown in a grid / search result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimeCard {
    /// Id of the source this card came from (see `sources.toml`).
    pub source: String,
    /// Series page URL on that source.
    pub url: String,
    pub title: String,
    pub poster: Option<String>,
    /// Short badge such as "EP 12" / "ซับไทย" (whatever the listing shows).
    pub badge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub season: u32,
    pub num: f32,
    pub label: String,
    pub url: String,
}

impl Episode {
    /// Same episode on another source? (season is ignored when either side
    /// only has one season, because sites number seasons differently)
    pub fn same_as(&self, other: &Episode, both_single_season: bool) -> bool {
        (self.num - other.num).abs() < 0.01 && (both_single_season || self.season == other.season)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnimeDetail {
    pub source: String,
    pub url: String,
    pub title: String,
    pub poster: Option<String>,
    pub synopsis: String,
    pub tags: Vec<String>,
    pub episodes: Vec<Episode>,
}

impl AnimeDetail {
    pub fn single_season(&self) -> bool {
        self.episodes.windows(2).all(|w| w[0].season == w[1].season)
    }
    pub fn card(&self) -> AnimeCard {
        AnimeCard {
            source: self.source.clone(),
            url: self.url.clone(),
            title: self.title.clone(),
            poster: self.poster.clone(),
            badge: None,
        }
    }
}

/// A player/server option on an episode page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub kind: ServerKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerKind {
    /// An embed / stream URL already present on the page.
    Direct(String),
    /// DooPlay player that must be asked for its embed.
    DooPlay {
        post: String,
        typ: String,
        nume: String,
    },
}

/// One playable quality of a stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Variant {
    /// 1080, 720 … (None = unknown / adaptive)
    pub height: Option<u32>,
    pub url: String,
}

/// How good a resolved server is: lower is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StreamRank {
    /// Real .m3u8/.mp4 found.
    Direct = 0,
    /// A host yt-dlp understands (ok.ru, dailymotion …).
    YtDlp = 1,
    /// Unknown page, handed to the player as a last resort.
    Unknown = 2,
}

/// A server resolved as far as possible: what to hand to the player.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stream {
    pub source: String,
    pub server: String,
    /// Qualities, best first. Never empty.
    pub variants: Vec<Variant>,
    /// Page the stream was found on (used for Referer).
    pub page: String,
    pub rank: StreamRank,
}

impl Stream {
    pub fn direct(&self) -> bool {
        self.rank == StreamRank::Direct
    }
    pub fn heights(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.variants.iter().filter_map(|v| v.height).collect();
        v.dedup();
        v
    }
}
