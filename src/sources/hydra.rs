//! Hydra Launcher: the games in its library (LevelDB under
//! `~/.config/hydralauncher/hydra-db`), installed when Hydra knows their
//! program. Hydra keeps the base locked while it runs, so it's read from a
//! copy. Games are started through Hydra (`hydralauncher://run?…`), which
//! counts their time and uses its own Proton and prefix.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::game::{Art, Arts, Game, Source};

/// Hydra's data folders (where `hydra-db` and `wine-prefixes` are).
pub fn roots() -> Vec<PathBuf> {
    if let Some(r) = std::env::var_os("LENTA_HYDRA_ROOT") {
        return vec![PathBuf::from(r)];
    }
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let mut out = vec![
        dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("hydralauncher"),
    ];
    // The Flatpak's, whatever its id.
    if let Ok(rd) = std::fs::read_dir(home.join(".var/app")) {
        for e in rd.flatten() {
            out.push(e.path().join("config/hydralauncher"));
        }
    }
    out.retain(|r| r.join("hydra-db").is_dir());
    out
}

/// Every key and value of a LevelDB, read from a copy of it (`scratch`
/// emptied first) so a running Hydra's lock and writes don't matter.
pub fn read_db(db: &Path, scratch: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let _ = std::fs::remove_dir_all(scratch);
    std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(db).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name();
        if name == "LOCK" || !e.path().is_file() {
            continue;
        }
        std::fs::copy(e.path(), scratch.join(&name)).map_err(|e| e.to_string())?;
    }
    let out = dump(scratch);
    let _ = std::fs::remove_dir_all(scratch);
    out
}

fn dump(db: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    use rusty_leveldb::LdbIterator;
    let opt = rusty_leveldb::Options {
        create_if_missing: false,
        ..Default::default()
    };
    let mut db = rusty_leveldb::DB::open(db, opt).map_err(|e| e.to_string())?;
    let mut it = db.new_iter().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some((k, v)) = it.next() {
        out.push((String::from_utf8_lossy(&k).into_owned(), v.to_vec()));
    }
    Ok(out)
}

/// The records of one sublevel ("games", "downloads", "gameShopAssets"),
/// by their key within it ("steam:570940"), as JSON.
pub fn sublevel(all: &[(String, Vec<u8>)], name: &str) -> HashMap<String, Value> {
    let pre = format!("!{name}!");
    all.iter()
        .filter_map(|(k, v)| {
            let id = k.strip_prefix(&pre)?;
            Some((id.to_string(), serde_json::from_slice(v).ok()?))
        })
        .collect()
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k)?.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn art(v: Option<&Value>, keys: &[&str]) -> Option<Art> {
    let v = v?;
    keys.iter().find_map(|k| {
        let u = s(v, k)?;
        if u.starts_with("http") {
            Some(Art::Url(u.to_string()))
        } else {
            let p = PathBuf::from(u.trim_start_matches("file://").trim_start_matches("local:"));
            p.is_file().then_some(Art::File(p))
        }
    })
}

/// The deep link Hydra starts a game from (as its own shortcuts do).
pub fn run_link(shop: &str, id: &str) -> String {
    format!("hydralauncher://run?shop={shop}&objectId={id}")
}

/// The folder a game's program is in, up past the usual subfolders
/// (`bin`, `Binaries/Win64`…): the game's own folder.
pub fn game_dir(exe: &Path) -> Option<PathBuf> {
    const SUB: [&str; 12] = [
        "bin", "bin32", "bin64", "binaries", "win32", "win64", "x86", "x64", "x86_64", "retail",
        "release", "shipping",
    ];
    let mut d = exe.parent()?.to_path_buf();
    while d
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| SUB.contains(&n.to_lowercase().as_str()))
    {
        d = d.parent()?.to_path_buf();
    }
    Some(d)
}

pub fn games() -> Vec<Game> {
    let mut out = Vec::new();
    let scratch = crate::net::cache_dir().join("hydra-db-read");
    for root in roots() {
        let all = match read_db(&root.join("hydra-db"), &scratch) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("lenta: Hydra: {e}");
                continue;
            }
        };
        out.extend(from_db(&root, &all));
    }
    out
}

/// The library from a dump of Hydra's base.
pub fn from_db(root: &Path, all: &[(String, Vec<u8>)]) -> Vec<Game> {
    let assets = sublevel(all, "gameShopAssets");
    let downloads = sublevel(all, "downloads");
    let mut out = Vec::new();
    for (id, g) in sublevel(all, "games") {
        if g.get("isDeleted").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some((shop, object)) = id.split_once(':') else {
            continue;
        };
        let a = assets.get(&id);
        let title = s(&g, "title")
            .or_else(|| a.and_then(|a| s(a, "title")))
            .unwrap_or(object)
            .to_string();
        let exe = s(&g, "executablePath").map(PathBuf::from);
        let installed = exe.as_ref().is_some_and(|e| e.is_file());
        let link = run_link(shop, object);
        let both = [Some(&g), a];
        let pick = |keys: &[&str]| both.iter().find_map(|v| art(*v, keys));
        let prefix = s(&g, "winePrefixPath")
            .map(PathBuf::from)
            .or_else(|| Some(root.join("wine-prefixes").join(object)))
            .filter(|p| p.is_dir());
        let ms = g
            .get("playTimeInMilliseconds")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        out.push(Game {
            dl: downloads
                .get(&id)
                .filter(|_| !installed)
                .filter(|d| s(d, "status") != Some("removed"))
                .map(|d| {
                    if matches!(s(d, "status"), Some("complete" | "seeding")) {
                        1.0
                    } else {
                        d.get("progress")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0)
                            .min(0.999) as f32
                    }
                }),
            key: format!("hydra:{id}"),
            title,
            source: Source::Hydra,
            sub: match shop {
                "steam" => None,
                "custom" => Some("Custom".into()),
                o => Some(o.to_string()),
            },
            art: Arts {
                portrait: pick(&["customCoverImageUrl", "libraryImageUrl", "coverImageUrl"]),
                portrait_alt: None,
                landscape: None,
                landscape_alt: None,
                hero: pick(&["customHeroImageUrl", "libraryHeroImageUrl"]),
                logo: pick(&["customLogoImageUrl", "logoImageUrl"]),
                icon: pick(&["customIconUrl", "iconUrl"]),
            },
            playtime_min: Some((ms / 60_000.0) as u64).filter(|&m| m > 0),
            last_played: s(&g, "lastTimePlayed").and_then(crate::game::parse_iso),
            steam_appid: None,
            store_appid: (shop == "steam").then(|| object.parse().ok()).flatten(),
            emu: None,
            prefix,
            installed,
            compat_id: None,
            install_dir: exe.as_deref().filter(|_| installed).and_then(game_dir),
            launch: vec![
                vec!["hydralauncher".into(), link.clone()],
                vec!["xdg-open".into(), link],
            ],
        });
    }
    out
}

/// Write to Hydra's base (Hydra closed): `put` records as JSON, `del` keys.
pub fn write_db(db: &Path, put: &[(String, Value)], del: &[String]) -> Result<(), String> {
    let opt = rusty_leveldb::Options {
        create_if_missing: false,
        ..Default::default()
    };
    let mut d = rusty_leveldb::DB::open(db, opt).map_err(|e| e.to_string())?;
    for (k, v) in put {
        let bytes = serde_json::to_vec(v).map_err(|e| e.to_string())?;
        d.put(k.as_bytes(), &bytes).map_err(|e| e.to_string())?;
    }
    for k in del {
        d.delete(k.as_bytes()).map_err(|e| e.to_string())?;
    }
    d.flush().map_err(|e| e.to_string())?;
    d.close().map_err(|e| e.to_string())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A Hydra data folder with a base holding these records.
    pub fn fake(dir: &Path, recs: &[(&str, Value)]) {
        let db = dir.join("hydra-db");
        let opt = rusty_leveldb::Options {
            create_if_missing: true,
            ..Default::default()
        };
        let mut d = rusty_leveldb::DB::open(&db, opt).unwrap();
        for (k, v) in recs {
            d.put(k.as_bytes(), &serde_json::to_vec(v).unwrap())
                .unwrap();
        }
        d.flush().unwrap();
        d.close().unwrap();
    }

    #[test]
    fn library_read() {
        let t = std::env::temp_dir().join(format!("lenta-hydra-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let exe = t.join("Games/DSR/bin/DSR.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"MZ").unwrap();
        std::fs::create_dir_all(t.join("wine-prefixes/570940/drive_c")).unwrap();
        fake(
            &t,
            &[
                (
                    "!games!steam:570940",
                    serde_json::json!({"title": "DARK SOULS", "isDeleted": false,
                        "playTimeInMilliseconds": 140826.4, "lastTimePlayed": "2026-11-22T22:23:11.571Z",
                        "executablePath": exe}),
                ),
                (
                    "!games!steam:4115450",
                    serde_json::json!({"title": "Phantom Blade Zero", "isDeleted": false,
                        "executablePath": null}),
                ),
                (
                    "!games!steam:2358720",
                    serde_json::json!({"title": "Gone", "isDeleted": true}),
                ),
                (
                    "!downloads!steam:4115450",
                    serde_json::json!({"status": "paused", "progress": 0.96}),
                ),
                (
                    "!gameShopAssets!steam:570940",
                    serde_json::json!({"libraryImageUrl": "https://x/cover.jpg"}),
                ),
                ("language", serde_json::json!("ru")),
            ],
        );
        let all = read_db(&t.join("hydra-db"), &t.join("scratch")).unwrap();
        let mut g = from_db(&t, &all);
        g.sort_by_key(|g| g.title.clone());
        assert_eq!(g.len(), 2);
        let ds = &g[0];
        assert_eq!(ds.key, "hydra:steam:570940");
        assert!(ds.installed);
        assert_eq!(ds.store_appid, Some(570940));
        assert_eq!(ds.playtime_min, Some(2));
        assert_eq!(
            ds.install_dir.as_deref(),
            Some(t.join("Games/DSR").as_path())
        );
        assert_eq!(
            ds.prefix.as_deref(),
            Some(t.join("wine-prefixes/570940").as_path())
        );
        assert_eq!(
            ds.art.portrait,
            Some(Art::Url("https://x/cover.jpg".into()))
        );
        assert_eq!(
            ds.launch[0][1],
            "hydralauncher://run?shop=steam&objectId=570940"
        );
        assert!(!g[1].installed);
        assert_eq!(g[1].dl, Some(0.96));
        assert_eq!(ds.dl, None);
        assert!(!t.join("scratch").exists());
        let _ = std::fs::remove_dir_all(&t);
    }
}
