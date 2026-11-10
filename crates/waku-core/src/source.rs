//! The `Source` trait: one anime website.
//!
//! Most Thai sites are WordPress themes, so `WpSource` (config-driven) covers
//! them. A site that needs custom logic (SPA, JSON API …) can implement this
//! trait directly and be registered in `SourceManager::from_profiles`.

use crate::model::{AnimeCard, AnimeDetail, Server, Stream};
use crate::stream::Trace;
use anyhow::Result;

pub trait Source: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn base(&self) -> &str;
    fn note(&self) -> &str {
        ""
    }

    /// Newest / listed anime (page starts at 1).
    fn latest(&self, page: u32) -> Result<Vec<AnimeCard>>;
    /// Search; results are ranked best first.
    fn search(&self, query: &str) -> Result<Vec<AnimeCard>>;
    /// Series page: title, poster, synopsis, episodes.
    fn detail(&self, url: &str) -> Result<AnimeDetail>;
    /// Player options of an episode.
    fn servers(&self, episode_url: &str) -> Result<Vec<Server>>;
    /// Resolve one server into a playable stream.
    fn resolve(&self, server: &Server, episode_url: &str, tr: &Trace) -> Result<Stream>;
}
