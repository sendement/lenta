//! Where games come from.

pub mod desktop;
pub mod emu;
pub mod heroic;
pub mod hydra;
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

/// Every game from the enabled sources: installed ones, and those the
/// launchers (Heroic, Hydra) have in their library but not installed.
pub fn scan(cfg: &Config) -> Vec<Game> {
    let (steam, heroic, lutris, hydra) = (cfg.steam, cfg.heroic, cfg.lutris, cfg.hydra);
    let s = std::thread::spawn(move || if steam { steam::games() } else { Vec::new() });
    let h = std::thread::spawn(move || if heroic { heroic::games() } else { Vec::new() });
    let l = std::thread::spawn(move || if lutris { lutris::games() } else { Vec::new() });
    let y = std::thread::spawn(move || if hydra { hydra::games() } else { Vec::new() });
    let own = cfg.own;
    let o = std::thread::spawn(move || {
        if own {
            crate::progs::games()
        } else {
            Vec::new()
        }
    });
    let mut out = Vec::new();
    for t in [s, h, l, y, o] {
        out.extend(t.join().unwrap_or_default());
    }
    // Games from elsewhere: a Steam emulator's files name the store id
    // and keep the achievements.
    // Own programs already looked (with the id the user gave).
    for g in out
        .iter_mut()
        .filter(|g| g.steam_appid.is_none() && g.source != crate::game::Source::Own)
    {
        let (id, save) = emu::resolve(g.prefix.as_deref(), g.install_dir.as_deref());
        g.store_appid = g.store_appid.or(id);
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
