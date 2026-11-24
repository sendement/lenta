//! Heroic Games Launcher: Epic (legendary), GOG, Amazon (nile) and
//! side-loaded games from its library caches, play time from its store.
//! Games owned but not installed are listed too; Play opens Heroic (its
//! install, asked for from outside, needs its own window open first).

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::game::{Art, Arts, Game, Source};

pub fn roots() -> Vec<(PathBuf, bool)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(r) = std::env::var_os("LENTA_HEROIC_ROOT") {
        return vec![(PathBuf::from(r), false)];
    }
    for (p, flatpak) in [
        (
            dirs::config_dir()
                .unwrap_or_else(|| home.join(".config"))
                .join("heroic"),
            false,
        ),
        (
            home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
            true,
        ),
    ] {
        if p.is_dir() {
            out.push((p, flatpak));
        }
    }
    out
}

fn read_json(p: PathBuf) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn art(v: &Value, key: &str) -> Option<Art> {
    let s = v.get(key)?.as_str()?.trim();
    if s.is_empty() {
        None
    } else if s.starts_with("http") {
        Some(Art::Url(s.to_string()))
    } else {
        let p = PathBuf::from(s.trim_start_matches("file://"));
        p.is_file().then_some(Art::File(p))
    }
}

pub fn games() -> Vec<Game> {
    let mut out = Vec::new();
    for (root, flatpak) in roots() {
        // appName → (minutes, last played).
        let mut times: HashMap<String, (u64, u64)> = HashMap::new();
        if let Some(Value::Object(m)) = read_json(root.join("store/timestamp.json")) {
            for (k, v) in m {
                let total = v
                    .get("totalPlayed")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
                    .max(0.0) as u64;
                let last = v
                    .get("lastPlayed")
                    .and_then(Value::as_str)
                    .and_then(crate::game::parse_iso)
                    .unwrap_or(0);
                times.insert(k, (total, last));
            }
        }
        // What Heroic's tools have installed (its library caches don't
        // always say): appName → install folder.
        let installed_at = installed_lists(&root);
        let libs = [
            ("store_cache/legendary_library.json", "legendary", "Epic"),
            ("store_cache/gog_library.json", "gog", "GOG"),
            ("store_cache/nile_library.json", "nile", "Amazon"),
            ("sideload_apps/library.json", "sideload", "Sideload"),
        ];
        for (file, runner, store) in libs {
            let Some(v) = read_json(root.join(file)) else {
                continue;
            };
            let list = v
                .get("library")
                .or_else(|| v.get("games"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for g in list {
                // DLCs and GOG's redistributables aren't games.
                if g.pointer("/install/is_dlc").and_then(Value::as_bool) == Some(true) {
                    continue;
                }
                let Some(app) = g.get("app_name").and_then(Value::as_str) else {
                    continue;
                };
                let at = installed_at.get(app).cloned();
                let installed = if runner == "sideload" {
                    g.get("is_installed").and_then(Value::as_bool) == Some(true)
                } else {
                    at.is_some()
                };
                let title = g
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(app)
                    .to_string();
                let runner = g
                    .get("runner")
                    .and_then(Value::as_str)
                    .unwrap_or(runner)
                    .to_string();
                let url = format!("heroic://launch?appName={app}&runner={runner}");
                // Not installed: Heroic's own window, to install it there.
                let mut launch = Vec::new();
                let mut base: Vec<String> = if flatpak {
                    ["flatpak", "run", "com.heroicgameslauncher.hgl"]
                        .map(String::from)
                        .to_vec()
                } else {
                    vec!["heroic".to_string()]
                };
                if installed {
                    base.push("--no-gui".into());
                    base.push(url.clone());
                    launch.push(base);
                    launch.push(vec!["xdg-open".into(), url]);
                } else {
                    launch.push(base);
                }
                let (play, last) = times.get(app).copied().unwrap_or_default();
                let cover = art(&g, "art_cover");
                out.push(Game {
                    dl: None,
                    key: format!("heroic:{runner}:{app}"),
                    title,
                    source: Source::Heroic,
                    sub: Some(store.into()),
                    art: Arts {
                        landscape_alt: None,
                        portrait_alt: None,
                        portrait: art(&g, "art_square"),
                        landscape: cover.clone(),
                        hero: art(&g, "art_background").or(cover),
                        logo: art(&g, "art_logo"),
                        icon: art(&g, "art_icon"),
                    },
                    playtime_min: (play > 0).then_some(play),
                    last_played: (last > 0).then_some(last),
                    steam_appid: None,
                    store_appid: None,
                    installed,
                    compat_id: None,
                    emu: None,
                    prefix: wine_prefix(&root, app),
                    install_dir: at.flatten().or_else(|| {
                        g.pointer("/install/install_path")
                            .and_then(Value::as_str)
                            .filter(|_| installed)
                            .map(PathBuf::from)
                    }),
                    launch,
                });
            }
        }
    }
    out
}

/// The games legendary, gogdl and nile have installed, with their folders.
pub fn installed_lists(root: &std::path::Path) -> HashMap<String, Option<PathBuf>> {
    let mut out = HashMap::new();
    let path = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(PathBuf::from);
    if let Some(Value::Object(m)) = read_json(root.join("legendaryConfig/legendary/installed.json"))
    {
        for (app, v) in m {
            out.insert(app, path(&v, "install_path"));
        }
    }
    if let Some(v) = read_json(root.join("gog_store/installed.json")) {
        for g in v
            .get("installed")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(app) = g.get("appName").and_then(Value::as_str) {
                out.insert(app.to_string(), path(g, "install_path"));
            }
        }
    }
    if let Some(Value::Array(list)) = read_json(root.join("nile_config/nile/installed.json")) {
        for g in list {
            if let Some(app) = g.get("id").and_then(Value::as_str) {
                out.insert(app.to_string(), path(&g, "path"));
            }
        }
    }
    out
}

/// The game's Wine prefix from Heroic's per-game settings.
fn wine_prefix(root: &std::path::Path, app: &str) -> Option<PathBuf> {
    let v = read_json(root.join(format!("GamesConfig/{app}.json")))?;
    let p = v.get(app)?.get("winePrefix")?.as_str()?;
    let p = match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()?.join(rest),
        None => PathBuf::from(p),
    };
    p.is_dir().then_some(p)
}

/// Heroic's favourites and own categories, as game-key patterns
/// ("heroic:*:<appName>" when the runner isn't recorded).
pub fn categories() -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for (root, _) in roots() {
        let Some(v) = read_json(root.join("store/config.json")) else {
            continue;
        };
        let pick = |paths: &[&str]| paths.iter().find_map(|p| v.pointer(p)).cloned();
        // Favourites: [{appName, …}].
        if let Some(list) = pick(&[
            "/favouriteGames/list",
            "/games/favouriteGames/list",
            "/favourites",
        ])
        .and_then(|l| l.as_array().cloned())
        {
            let keys: Vec<String> = list
                .iter()
                .filter_map(|f| {
                    let app = f
                        .get("appName")
                        .and_then(Value::as_str)
                        .or_else(|| f.as_str())?;
                    Some(format!("heroic:*:{app}"))
                })
                .collect();
            if !keys.is_empty() {
                out.push(("favorite".into(), keys));
            }
        }
        // Own categories: {name: ["<appName>_<runner>", …]}.
        if let Some(cats) = pick(&["/customCategories", "/games/customCategories"])
            .and_then(|c| c.as_object().cloned())
        {
            for (name, list) in cats {
                let keys: Vec<String> = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|e| match e.rsplit_once('_') {
                        Some((app, runner))
                            if ["legendary", "gog", "nile", "sideload"].contains(&runner) =>
                        {
                            format!("heroic:{runner}:{app}")
                        }
                        _ => format!("heroic:*:{e}"),
                    })
                    .collect();
                out.push((name, keys));
            }
        }
    }
    out
}

/// Does a game key fit a category's pattern?
pub fn key_matches(pattern: &str, key: &str) -> bool {
    match pattern.strip_prefix("heroic:*:") {
        Some(app) => key.starts_with("heroic:") && key.rsplit(':').next() == Some(app),
        None => pattern == key,
    }
}
