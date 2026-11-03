//! Rolling a Steam game back to an older build and keeping it there: a
//! port of steam-downgrader. Versions are gathered from what the machine
//! knows; old depots are fetched over Steam's own protocol (CM + CDN) and
//! applied over the install; the appmanifest is then made to look current
//! so Steam leaves the game alone. State and downloads are shared with
//! steam-downgrader (~/.local/share/steam-downgrader).

pub mod assemble;
pub mod cdn;
pub mod cm;
pub mod crypto;
pub mod download;
pub mod flow;
pub mod game;
pub mod history;
pub mod manifest;
pub mod news;
pub mod ops;
pub mod pb;
pub mod state;
