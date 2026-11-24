//! Lutris: installed games from pga.db, covers and banners from its data
//! folder (native or Flatpak).

use std::path::{Path, PathBuf};

use crate::game::{Art, Arts, Game, Source};

pub fn roots() -> Vec<(PathBuf, bool)> {
    if let Some(r) = std::env::var_os("LENTA_LUTRIS_ROOT") {
        return vec![(PathBuf::from(r), false)];
    }
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (p, flatpak) in [
        (
            dirs::data_dir()
                .unwrap_or_else(|| home.join(".local/share"))
                .join("lutris"),
            false,
        ),
        (home.join(".var/app/net.lutris.Lutris/data/lutris"), true),
    ] {
        if p.join("pga.db").is_file() {
            out.push((p, flatpak));
        }
    }
    out
}

fn image(dirs: &[PathBuf], slug: &str) -> Option<Art> {
    dirs.iter()
        .flat_map(|d| ["jpg", "png", "jpeg", "webp"].map(|x| d.join(format!("{slug}.{x}"))))
        .find(|p| p.is_file())
        .map(Art::File)
}

fn cache_dir(root: &Path, flatpak: bool, sub: &str) -> Vec<PathBuf> {
    let mut v = vec![root.join(sub)];
    let cache = if flatpak {
        dirs::home_dir().map(|h| h.join(".var/app/net.lutris.Lutris/cache/lutris"))
    } else {
        dirs::cache_dir().map(|c| c.join("lutris"))
    };
    v.extend(cache.map(|c| c.join(sub)));
    v
}

/// Where a game's YAML config is (by its id in pga.db).
pub fn config_file(id: i64) -> Option<PathBuf> {
    for (root, flatpak) in roots() {
        let Ok(db) = rusqlite::Connection::open_with_flags(
            root.join("pga.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            continue;
        };
        let Ok(name) = db.query_row("SELECT configpath FROM games WHERE id = ?1", [id], |r| {
            r.get::<_, Option<String>>(0)
        }) else {
            continue;
        };
        let Some(name) = name else { continue };
        if let Some(p) = config_places(&root, flatpak, &name)
            .into_iter()
            .find(|p| p.is_file())
        {
            return Some(p);
        }
    }
    None
}

fn config_places(root: &Path, flatpak: bool, name: &str) -> Vec<PathBuf> {
    let mut places = vec![root.join(format!("games/{name}.yml"))];
    let conf = if flatpak {
        dirs::home_dir().map(|h| h.join(".var/app/net.lutris.Lutris/config/lutris"))
    } else {
        dirs::config_dir().map(|c| c.join("lutris"))
    };
    places.extend(conf.map(|c| c.join(format!("games/{name}.yml"))));
    places
}

/// The game's exe and Wine prefix from its YAML config (only the `game:`
/// section's two keys are needed, so no YAML parser).
fn game_config(root: &Path, flatpak: bool, name: &str) -> (Option<PathBuf>, Option<PathBuf>) {
    let places = config_places(root, flatpak, name);
    let Some(text) = places.iter().find_map(|p| std::fs::read_to_string(p).ok()) else {
        return (None, None);
    };
    let (mut exe, mut prefix) = (None, None);
    let mut in_game = false;
    for line in text.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            in_game = line.trim_end() == "game:";
            continue;
        }
        if !in_game {
            continue;
        }
        let Some((k, v)) = line.trim().split_once(':') else {
            continue;
        };
        let v = v.trim().trim_matches(|c| c == '\'' || c == '"');
        if v.is_empty() {
            continue;
        }
        let path = |v: &str| match v.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().map(|h| h.join(rest)),
            None => Some(PathBuf::from(v)),
        };
        match k.trim() {
            "exe" => exe = path(v),
            "prefix" => prefix = path(v),
            _ => {}
        }
    }
    // A relative exe sits in the game's folder; then it tells nothing.
    (exe.filter(|e| e.is_absolute()), prefix)
}

pub fn games() -> Vec<Game> {
    let mut out = Vec::new();
    for (root, flatpak) in roots() {
        let Ok(db) = rusqlite::Connection::open_with_flags(
            root.join("pga.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            continue;
        };
        let Ok(mut st) = db.prepare("SELECT id, name, slug, runner, directory, playtime, lastplayed, service, configpath FROM games WHERE installed = 1") else { continue };
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<f64>>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        });
        let Ok(rows) = rows else { continue };
        let covers = cache_dir(&root, flatpak, "coverart");
        let banners = cache_dir(&root, flatpak, "banners");
        let icons: Vec<PathBuf> = dirs::data_dir()
            .map(|d| d.join("icons/hicolor/128x128/apps"))
            .into_iter()
            .collect();
        for (id, name, slug, runner, dir, play, last, service, conf) in rows.flatten() {
            // Steam games Lutris only links to are listed from Steam itself.
            if service.as_deref() == Some("steam") || runner.as_deref() == Some("steam") {
                continue;
            }
            let slug = slug.unwrap_or_default();
            let url = format!("lutris:rungameid/{id}");
            let launch = if flatpak {
                vec![
                    vec![
                        "flatpak".into(),
                        "run".into(),
                        "net.lutris.Lutris".into(),
                        url.clone(),
                    ],
                    vec!["xdg-open".into(), url],
                ]
            } else {
                vec![
                    vec!["lutris".into(), url.clone()],
                    vec!["xdg-open".into(), url],
                ]
            };
            let banner = image(&banners, &slug);
            let (exe, prefix) = conf
                .as_deref()
                .map(|c| game_config(&root, flatpak, c))
                .unwrap_or_default();
            let prefix = prefix.or_else(|| {
                (runner.as_deref() == Some("wine"))
                    .then(|| dirs::home_dir().map(|h| h.join(".wine")))
                    .flatten()
            });
            let dir = dir
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
                .or_else(|| exe.as_ref().and_then(|e| e.parent()).map(Path::to_path_buf));
            let play = (play.unwrap_or(0.0) * 60.0).round() as u64;
            out.push(Game {
                dl: None,
                key: format!("lutris:{id}"),
                title: name.unwrap_or_else(|| slug.clone()),
                source: Source::Lutris,
                sub: runner,
                art: Arts {
                    landscape_alt: None,
                    portrait_alt: None,
                    portrait: image(&covers, &slug),
                    landscape: banner.clone(),
                    hero: banner,
                    logo: None,
                    icon: image(&icons, &format!("lutris_{slug}")),
                },
                playtime_min: (play > 0).then_some(play),
                last_played: last.filter(|&l| l > 0).map(|l| l as u64),
                steam_appid: None,
                store_appid: None,
                installed: true,
                compat_id: None,
                emu: None,
                prefix: prefix.filter(|p| p.is_dir()),
                install_dir: dir,
                launch,
            });
        }
    }
    out
}

/// Lutris's categories: (name, game keys). "favorite" is its favourites;
/// names starting with "." (".hidden") are its own.
pub fn categories() -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for (root, _) in roots() {
        let Ok(db) = rusqlite::Connection::open_with_flags(
            root.join("pga.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            continue;
        };
        let Ok(mut st) = db.prepare(
            "SELECT c.name, gc.game_id FROM categories c JOIN games_categories gc ON gc.category_id = c.id",
        ) else {
            continue;
        };
        let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        else {
            continue;
        };
        for (name, id) in rows.flatten() {
            let key = format!("lutris:{id}");
            match out.iter_mut().find(|(n, _)| *n == name) {
                Some((_, keys)) => keys.push(key),
                None => out.push((name, vec![key])),
            }
        }
    }
    out
}
