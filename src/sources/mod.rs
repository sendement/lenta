//! Where games come from.

pub mod desktop;
pub mod heroic;
pub mod lutris;
pub mod steam;

use crate::config::{Config, Sort};
use crate::game::Game;

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
