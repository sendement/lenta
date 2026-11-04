//! The launcher's own collections, across all launchers: plain ones (a
//! list of games put there by hand) and dynamic ones (every game that fits
//! a rule, give or take games added or left out by hand).
//! Kept in ~/.local/share/lenta/collections.json.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::game::{Game, Source};

/// Yes, no, or either.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Tri {
    #[default]
    Any,
    Yes,
    No,
}

impl Tri {
    pub const ALL: [Tri; 3] = [Tri::Any, Tri::Yes, Tri::No];

    fn fits(self, v: bool) -> bool {
        match self {
            Tri::Any => true,
            Tri::Yes => v,
            Tri::No => !v,
        }
    }
}

/// When it was played last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Played {
    #[default]
    Any,
    Week,
    Month,
    Year,
    /// More than a year ago.
    Long,
    Never,
}

impl Played {
    pub const ALL: [Played; 6] = [
        Played::Any,
        Played::Week,
        Played::Month,
        Played::Year,
        Played::Long,
        Played::Never,
    ];
}

/// How long it was played, in all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Time {
    #[default]
    Any,
    /// Under an hour (never included).
    Little,
    /// Ten hours and more.
    Ten,
    /// Fifty hours and more.
    Fifty,
}

impl Time {
    pub const ALL: [Time; 4] = [Time::Any, Time::Little, Time::Ten, Time::Fifty];
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Rule {
    /// None: any launcher.
    pub source: Option<Source>,
    pub installed: Tri,
    pub favorite: Tri,
    /// Not from a launcher's own store: Non-Steam shortcuts, sideloaded.
    pub third_party: Tri,
    pub played: Played,
    pub time: Time,
    /// Words its name has (any case), empty for any.
    pub words: String,
}

/// What a rule needs to know about a game beyond the game itself.
pub struct Facts {
    pub favorite: bool,
    pub now: u64,
}

impl Rule {
    pub fn fits(&self, g: &Game, f: &Facts) -> bool {
        if self.source.is_some_and(|s| s != g.source) {
            return false;
        }
        let third = g.key.starts_with("steam-shortcut:") || g.sub.as_deref() == Some("Sideload");
        if !self.installed.fits(g.installed)
            || !self.favorite.fits(f.favorite)
            || !self.third_party.fits(third)
        {
            return false;
        }
        const DAY: u64 = 86_400;
        let ago = g
            .last_played
            .filter(|t| *t > 0)
            .map(|t| f.now.saturating_sub(t));
        let played = match self.played {
            Played::Any => true,
            Played::Week => ago.is_some_and(|a| a <= 7 * DAY),
            Played::Month => ago.is_some_and(|a| a <= 30 * DAY),
            Played::Year => ago.is_some_and(|a| a <= 365 * DAY),
            Played::Long => ago.is_some_and(|a| a > 365 * DAY),
            Played::Never => ago.is_none() && g.playtime_min.unwrap_or(0) == 0,
        };
        let min = g.playtime_min.unwrap_or(0);
        let time = match self.time {
            Time::Any => true,
            Time::Little => min < 60,
            Time::Ten => min >= 600,
            Time::Fifty => min >= 3000,
        };
        let title = g.title.to_lowercase();
        let words = self
            .words
            .split_whitespace()
            .all(|w| title.contains(&w.to_lowercase()));
        played && time && words
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Coll {
    pub id: u64,
    pub name: String,
    /// Games put in by hand (keys), in the order added.
    pub games: Vec<String>,
    /// Dynamic: also every game this fits.
    pub rule: Option<Rule>,
    /// Games taken out of a dynamic one although they fit.
    pub hide: Vec<String>,
}

impl Coll {
    pub fn dynamic(&self) -> bool {
        self.rule.is_some()
    }

    /// Is this game in it?
    pub fn has(&self, g: &Game, f: &Facts) -> bool {
        if self.games.contains(&g.key) {
            return true;
        }
        self.rule
            .as_ref()
            .is_some_and(|r| !self.hide.contains(&g.key) && r.fits(g, f))
    }

    pub fn add(&mut self, key: &str) {
        self.hide.retain(|k| k != key);
        if !self.games.iter().any(|k| k == key) {
            self.games.push(key.to_string());
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.games.retain(|k| k != key);
        if self.rule.is_some() && !self.hide.iter().any(|k| k == key) {
            self.hide.push(key.to_string());
        }
    }
}

#[derive(Default)]
pub struct Colls {
    pub list: Vec<Coll>,
    /// Where they're kept; none in tests (nothing is written).
    path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    collections: Vec<Coll>,
}

pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
    base.join("lenta/collections.json")
}

impl Colls {
    /// The user's, or nothing kept anywhere for tests (dry run).
    pub fn load() -> Colls {
        if std::env::var_os("LENTA_DRY_RUN").is_some() {
            return Colls::default();
        }
        Colls::load_from(default_path())
    }

    pub fn load_from(path: PathBuf) -> Colls {
        let list = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<File>(&b).ok())
            .map(|f| f.collections)
            .unwrap_or_default();
        Colls {
            list,
            path: Some(path),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let body = serde_json::to_vec_pretty(&File {
            collections: self.list.clone(),
        })
        .map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn get(&self, id: u64) -> Option<&Coll> {
        self.list.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Coll> {
        self.list.iter_mut().find(|c| c.id == id)
    }

    /// A new one (named, empty); its id.
    pub fn create(&mut self, name: String, rule: Option<Rule>) -> u64 {
        let id = self.list.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        self.list.push(Coll {
            id,
            name,
            rule,
            ..Default::default()
        });
        id
    }

    pub fn delete(&mut self, id: u64) {
        self.list.retain(|c| c.id != id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(key: &str, title: &str, installed: bool, last: Option<u64>, min: u64) -> Game {
        Game {
            key: key.into(),
            title: title.into(),
            source: Source::Steam,
            sub: None,
            art: Default::default(),
            playtime_min: Some(min),
            last_played: last,
            steam_appid: None,
            store_appid: None,
            emu: None,
            prefix: None,
            installed,
            compat_id: None,
            install_dir: None,
            launch: Vec::new(),
        }
    }

    #[test]
    fn rules_and_hand_picks() {
        let now = 1_000_000_000;
        let f = Facts {
            favorite: false,
            now,
        };
        let a = game(
            "steam:1",
            "Dark Souls III",
            true,
            Some(now - 3 * 86_400),
            900,
        );
        let b = game("steam:2", "Hades", false, None, 0);
        let rule = Rule {
            installed: Tri::Yes,
            played: Played::Week,
            time: Time::Ten,
            words: "souls dark".into(),
            ..Default::default()
        };
        assert!(rule.fits(&a, &f));
        assert!(!rule.fits(&b, &f));
        let never = Rule {
            played: Played::Never,
            ..Default::default()
        };
        assert!(never.fits(&b, &f) && !never.fits(&a, &f));

        let mut c = Coll {
            rule: Some(rule),
            ..Default::default()
        };
        assert!(c.has(&a, &f) && !c.has(&b, &f));
        c.add("steam:2");
        assert!(c.has(&b, &f));
        // Taken out of a dynamic one: stays out though it fits.
        c.remove("steam:1");
        assert!(!c.has(&a, &f));
        c.add("steam:1");
        assert!(c.has(&a, &f));
    }

    #[test]
    fn kept_on_disk() {
        let dir = std::env::temp_dir().join(format!("lenta-colls-{}", std::process::id()));
        let path = dir.join("collections.json");
        let mut cs = Colls::load_from(path.clone());
        let id = cs.create("Coop".into(), None);
        cs.get_mut(id).unwrap().add("lutris:3");
        cs.create("Recent".into(), Some(Rule::default()));
        cs.save().unwrap();
        let back = Colls::load_from(path);
        assert_eq!(back.list, cs.list);
        assert_eq!(back.list[0].games, vec!["lutris:3".to_string()]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
