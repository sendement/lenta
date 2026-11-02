//! Lutris: installed games from pga.db, covers and banners from its data
//! folder (native or Flatpak).

use std::path::{Path, PathBuf};

use crate::game::{Art, Arts, Game, Source};

fn roots() -> Vec<(PathBuf, bool)> {
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

pub fn games() -> Vec<Game> {
    let mut out = Vec::new();
    for (root, flatpak) in roots() {
        let Ok(db) = rusqlite::Connection::open_with_flags(
            root.join("pga.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            continue;
        };
        let Ok(mut st) = db.prepare("SELECT id, name, slug, runner, directory, playtime, lastplayed, service FROM games WHERE installed = 1") else { continue };
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
            ))
        });
        let Ok(rows) = rows else { continue };
        let covers = cache_dir(&root, flatpak, "coverart");
        let banners = cache_dir(&root, flatpak, "banners");
        let icons: Vec<PathBuf> = dirs::data_dir()
            .map(|d| d.join("icons/hicolor/128x128/apps"))
            .into_iter()
            .collect();
        for (id, name, slug, runner, dir, play, last, service) in rows.flatten() {
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
            let play = (play.unwrap_or(0.0) * 60.0).round() as u64;
            out.push(Game {
                key: format!("lutris:{id}"),
                title: name.unwrap_or_else(|| slug.clone()),
                source: Source::Lutris,
                sub: runner,
                art: Arts {
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
                install_dir: dir.filter(|d| !d.is_empty()).map(PathBuf::from),
                launch,
            });
        }
    }
    out
}
