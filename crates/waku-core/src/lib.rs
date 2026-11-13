//! waku-core — multi-source Thai anime scraping engine used by the Waku app.

pub mod gated;
pub mod srcstats;
pub mod http;
pub mod manager;
pub mod media;
pub mod browser;
pub mod health;
pub mod meta;
pub mod model;
pub mod player;
pub mod profile;
pub mod source;
pub mod store;
pub mod stream;
pub mod text;
pub mod wp;

pub use manager::SourceManager;
pub use model::*;
pub use source::Source;
