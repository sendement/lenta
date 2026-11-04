//! Where games come from.

pub mod desktop;
pub mod emu;
pub mod heroic;
pub mod lutris;
pub mod steam;
pub mod steamplay;
pub mod web;

use crate::config::{Config, Sort};
use crate::game::Game;

/// What the library shelves beyond the games: the launchers' own
/// categories (favourites included) and Steam's installed soundtracks.
#[derive(Clone, Default)]
pub struct Shelves {
    pub lutris: Vec<(String, Vec<String>)>,
    pub heroic: Vec<(String, Vec<String>)>,
    pub soundtracks: Vec<Game>,
}

pub fn shelves(cfg: &Config) -> Shelves {
    Shelves {
        lutris: if cfg.lutris {
            lutris::categories()
        } else {
            Vec::new()
        },
        heroic: if cfg.heroic {
            heroic::categories()
        } else {
            Vec::new()
        },
        soundtracks: if cfg.steam {
            steam::soundtracks()
        } else {
            Vec::new()
        },
    }
}

/// Every installed game from the enabled sources.
pub fn scan(cfg: &Config) -> Vec<Game> {
    let (steam, heroic, lutris) = (cfg.steam, cfg.heroic, cfg.lutris);
    let s = std::thread::spawn(move || if steam { steam::games() } else { Vec::new() });
    let h = std::thread::spawn(move || if heroic { heroic::games() } else { Vec::new() });
    let l = std::thread::spawn(move || if lutris { lutris::games() } else { Vec::new() });
    let mut out = Vec::new();
    for t in [s, h, l] {
        out.extend(t.join().unwrap_or_default());
    }
    // Games from elsewhere: a Steam emulator's files name the store id
    // and keep the achievements.
    for g in out.iter_mut().filter(|g| g.steam_appid.is_none()) {
        let (id, save) = emu::resolve(g.prefix.as_deref(), g.install_dir.as_deref());
        g.store_appid = id;
        g.emu = save;
    }
    out
}

pub fn sort(games: &mut [Game], how: Sort) {
    let name = |g: &Game| g.title.to_lowercase();
    match how {
        Sort::Name => games.sort_by_key(name),
        Sort::Recent => games.sort_by(|a, b| {
            b.last_played
                .unwrap_or(0)
                .cmp(&a.last_played.unwrap_or(0))
                .then_with(|| name(a).cmp(&name(b)))
        }),
        Sort::Playtime => games.sort_by(|a, b| {
            b.playtime_min
                .unwrap_or(0)
                .cmp(&a.playtime_min.unwrap_or(0))
                .then_with(|| name(a).cmp(&name(b)))
        }),
    }
}
