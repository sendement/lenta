//! Heroic Games Launcher: installed Epic (legendary), GOG, Amazon (nile)
//! and side-loaded games from its library caches, play time from its store.

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::game::{Art, Arts, Game, Source};

fn roots() -> Vec<(PathBuf, bool)> {
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
                if g.get("is_installed").and_then(Value::as_bool) != Some(true) {
                    continue;
                }
                let Some(app) = g.get("app_name").and_then(Value::as_str) else {
                    continue;
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
                let mut launch = Vec::new();
                if flatpak {
                    launch.push(vec![
                        "flatpak".into(),
                        "run".into(),
                        "com.heroicgameslauncher.hgl".into(),
                        "--no-gui".into(),
                        url.clone(),
                    ]);
                } else {
                    launch.push(vec!["heroic".into(), "--no-gui".into(), url.clone()]);
                }
                launch.push(vec!["xdg-open".into(), url]);
                let (play, last) = times.get(app).copied().unwrap_or_default();
                let cover = art(&g, "art_cover");
                out.push(Game {
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
                    installed: true,
                    compat_id: None,
                    emu: None,
                    prefix: wine_prefix(&root, app),
                    install_dir: g
                        .pointer("/install/install_path")
                        .and_then(Value::as_str)
                        .map(PathBuf::from),
                    launch,
                });
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
