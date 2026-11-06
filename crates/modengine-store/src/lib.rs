//! The asset store of Mod Engine: free assets from OpenGameArt.org.
//!
//! Everything that does not need a window lives here, shared by the editor's Tienda
//! (`modengine-editor/src/store.rs`) and the `modengine store` command line (which the
//! editor's coding agent runs):
//!
//! * [`oga`]: search URLs, licences and the HTML extractor for OGA pages, plus the robots.txt
//!   allow-list every request goes through.
//! * [`net`]: the honest, polite HTTP client (`curl`, our own User-Agent), the disk cache
//!   (24 h) and the rate limit (one request every 2 s) — shared between processes through a
//!   lock file in the cache folder, so the editor and the CLI together never go faster.
//! * [`zip`]: listing and extracting the .zip packs.
//! * [`credits`]: the CREDITS.md section of each asset.
//! * [`testing`]: fakes for tests (never the network).

pub mod credits;
pub mod net;
pub mod oga;
pub mod testing;
pub mod zip;
