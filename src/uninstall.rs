//! Uninstalling Heroic's, Lutris's and Hydra's games without their windows: the
//! game's folder deleted and the launcher's own record of it changed as
//! it would itself. Only folders the launcher installed into are deleted;
//! a game added by hand (Lutris without a game folder, Heroic's
//! side-loaded ones) is only taken off the launcher's list, its files left
//! where they are.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::game::{Game, Source};

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub key: String,
    /// Deleted.
    pub delete: Option<PathBuf>,
    /// Deleted too: what the game was installed from (Hydra's download).
    pub also: Vec<PathBuf>,
    pub bytes: u64,
    /// Left where they are (said in the question).
    pub kept: Vec<PathBuf>,
    /// The launcher has to be closed for it (Heroic writes its lists back,
    /// Hydra locks its base): its name and process.
    pub close: Option<(&'static str, &'static str)>,
}

/// A folder that may go: absolute, deep enough, not home or a system one.
pub fn deletable(p: &Path) -> bool {
    let home = dirs::home_dir().unwrap_or_default();
    p.is_absolute()
        && p.is_dir()
        && p.components().count() >= 4
        && p != home
        && !home.starts_with(p)
        && !["/usr", "/etc", "/bin", "/lib", "/opt", "/var", "/boot"]
            .iter()
            .any(|s| p.starts_with(s))
}

pub fn plan(g: &Game) -> Result<Plan, String> {
    match g.source {
        Source::Heroic => heroic_plan(g),
        Source::Lutris => lutris_plan(g),
        Source::Hydra => hydra_plan(g),
        _ => Err("not here".into()),
    }
}

/// Do it; the bytes freed.
pub fn run(g: &Game, plan: &Plan) -> Result<u64, String> {
    if std::env::var_os("LENTA_DRY_RUN").is_some() {
        return Ok(0);
    }
    match g.source {
        Source::Heroic => {
            let (runner, app) = heroic_ids(&g.key).ok_or("bad key")?;
            for (root, _) in crate::sources::heroic::roots() {
                heroic_forget(&root, runner, app)?;
            }
        }
        Source::Lutris => {
            let id = lutris_id(&g.key).ok_or("bad key")?;
            for (root, _) in crate::sources::lutris::roots() {
                lutris_forget(&root.join("pga.db"), id)?;
            }
        }
        Source::Hydra => {
            let (root, id) = hydra_ids(g)?;
            hydra_forget(&root, &id, !plan.also.is_empty())?;
        }
        _ => return Err("not here".into()),
    }
    for d in plan.delete.iter().chain(&plan.also) {
        std::fs::remove_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    Ok(plan.bytes)
}

// ------------------------------------------------------------------ Heroic

fn heroic_ids(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix("heroic:")?;
    rest.split_once(':')
}

fn read(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn write(p: &Path, v: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.lenta-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

fn heroic_plan(g: &Game) -> Result<Plan, String> {
    let (runner, _) = heroic_ids(&g.key).ok_or("bad key")?;
    let dir = g.install_dir.clone().filter(|d| deletable(d));
    let mut kept: Vec<PathBuf> = g.prefix.iter().cloned().collect();
    let delete = if runner == "sideload" {
        kept.extend(g.install_dir.iter().cloned());
        None
    } else {
        dir
    };
    Ok(Plan {
        key: g.key.clone(),
        bytes: delete.as_deref().map_or(0, crate::steamlib::tree_size),
        delete,
        also: Vec::new(),
        kept,
        close: Some(("Heroic", "heroic")),
    })
}

/// Take the game off Heroic's lists of installed games (its own, and the
/// one of the tool behind it: legendary, gogdl, nile).
pub fn heroic_forget(root: &Path, runner: &str, app: &str) -> Result<(), String> {
    let edit = |p: PathBuf, f: &dyn Fn(&mut Value) -> bool| -> Result<(), String> {
        let Some(mut v) = read(&p) else { return Ok(()) };
        if f(&mut v) {
            write(&p, &v)?;
        }
        Ok(())
    };
    match runner {
        "legendary" => edit(
            root.join("legendaryConfig/legendary/installed.json"),
            &|v| v.as_object_mut().and_then(|m| m.remove(app)).is_some(),
        )?,
        "gog" => edit(root.join("gog_store/installed.json"), &|v| {
            let Some(list) = v.get_mut("installed").and_then(Value::as_array_mut) else {
                return false;
            };
            let n = list.len();
            list.retain(|g| g.get("appName").and_then(Value::as_str) != Some(app));
            list.len() != n
        })?,
        "nile" => edit(root.join("nile_config/nile/installed.json"), &|v| {
            let Some(list) = v.as_array_mut() else {
                return false;
            };
            let n = list.len();
            list.retain(|g| g.get("id").and_then(Value::as_str) != Some(app));
            list.len() != n
        })?,
        _ => {}
    }
    // Heroic's library caches (what lenta and Heroic list from).
    let cache = match runner {
        "legendary" => "store_cache/legendary_library.json",
        "gog" => "store_cache/gog_library.json",
        "nile" => "store_cache/nile_library.json",
        _ => "sideload_apps/library.json",
    };
    edit(root.join(cache), &|v| {
        let field = if v.get("library").is_some() {
            "library"
        } else {
            "games"
        };
        let Some(list) = v.get_mut(field).and_then(Value::as_array_mut) else {
            return false;
        };
        let n = list.len();
        if runner == "sideload" {
            list.retain(|g| g.get("app_name").and_then(Value::as_str) != Some(app));
            return list.len() != n;
        }
        let mut changed = false;
        for g in list.iter_mut() {
            if g.get("app_name").and_then(Value::as_str) == Some(app)
                && let Some(o) = g.as_object_mut()
            {
                o.insert("is_installed".into(), Value::Bool(false));
                o.insert("install".into(), Value::Object(Default::default()));
                changed = true;
            }
        }
        changed
    })
}

// ------------------------------------------------------------------ Lutris

fn lutris_id(key: &str) -> Option<i64> {
    key.strip_prefix("lutris:")?.parse().ok()
}

/// The folder Lutris installed the game in ("" for games added by hand).
fn lutris_dir(db: &Path, id: i64) -> Option<PathBuf> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .ok()?;
    let d: Option<String> = c
        .query_row("SELECT directory FROM games WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .ok()?;
    d.filter(|d| !d.trim().is_empty()).map(PathBuf::from)
}

fn lutris_plan(g: &Game) -> Result<Plan, String> {
    let id = lutris_id(&g.key).ok_or("bad key")?;
    let own = crate::sources::lutris::roots()
        .into_iter()
        .find_map(|(r, _)| lutris_dir(&r.join("pga.db"), id));
    let delete = own.filter(|d| deletable(d));
    let mut kept = Vec::new();
    if delete.is_none()
        && let Some(d) = &g.install_dir
    {
        kept.push(d.clone());
    }
    if let Some(p) = &g.prefix
        && !delete.as_ref().is_some_and(|d| p.starts_with(d))
    {
        kept.push(p.clone());
    }
    Ok(Plan {
        key: g.key.clone(),
        bytes: delete.as_deref().map_or(0, crate::steamlib::tree_size),
        delete,
        also: Vec::new(),
        kept,
        close: None,
    })
}

/// Not installed any more, as Lutris marks it (the game stays in its
/// library, to be installed again).
pub fn lutris_forget(db: &Path, id: i64) -> Result<(), String> {
    let c = rusqlite::Connection::open(db).map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE games SET installed = 0, installed_at = NULL WHERE id = ?1",
        [id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ------------------------------------------------------------------ Hydra

/// The Hydra folder the game is from and its id there ("steam:570940").
fn hydra_ids(g: &Game) -> Result<(PathBuf, String), String> {
    let id = g.key.strip_prefix("hydra:").ok_or("bad key")?.to_string();
    let roots = crate::sources::hydra::roots();
    let root = roots
        .iter()
        .find(|r| g.prefix.as_ref().is_some_and(|p| p.starts_with(r)))
        .or(roots.first())
        .cloned()
        .ok_or("no Hydra")?;
    Ok((root, id))
}

fn hydra_records(root: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let scratch = crate::net::cache_dir().join("hydra-db-plan");
    crate::sources::hydra::read_db(&root.join("hydra-db"), &scratch)
}

/// What Hydra itself deletes ("Remove files": its download of the game)
/// and the folder the game was installed into, when neither holds
/// another game of Hydra's or is a shared one (`~/Games`, Downloads).
pub fn hydra_dirs(
    all: &[(String, Vec<u8>)],
    id: &str,
) -> (Option<PathBuf>, Option<PathBuf>, Vec<PathBuf>) {
    use crate::sources::hydra::{game_dir, sublevel};
    let games = sublevel(all, "games");
    let downloads = sublevel(all, "downloads");
    let exe_of = |v: &Value| {
        v.get("executablePath")
            .and_then(Value::as_str)
            .map(PathBuf::from)
    };
    let dl_of = |v: &Value| {
        let base = v.get("downloadPath")?.as_str()?;
        let name = v.get("folderName")?.as_str()?.trim();
        (!name.is_empty() && !name.contains('/')).then(|| Path::new(base).join(name))
    };
    // Everything the other games are in.
    let mut others: Vec<PathBuf> = Vec::new();
    for (k, v) in &games {
        if k != id {
            others.extend(exe_of(v));
        }
    }
    for (k, v) in &downloads {
        if k != id {
            others.extend(dl_of(v));
        }
    }
    let shared = |d: &Path| {
        let n = d
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        ["games", "downloads", "загрузки", "steamlibrary", "common", "heroic", "hydra"]
            .contains(&n.as_str())
            || dirs::download_dir().is_some_and(|x| x == d)
    };
    let ok = |d: &Path| deletable(d) && !shared(d) && !others.iter().any(|o| o.starts_with(d));
    let exe = games.get(id).and_then(exe_of);
    let game = exe.as_deref().and_then(game_dir);
    let download = downloads.get(id).and_then(dl_of);
    let mut kept = Vec::new();
    let game_del = match game {
        Some(d) if ok(&d) => Some(d),
        Some(d) => {
            kept.push(d);
            None
        }
        None => None,
    };
    let dl_del = download
        .filter(|d| ok(d))
        .filter(|d| !game_del.as_ref().is_some_and(|g| d.starts_with(g)));
    (game_del, dl_del, kept)
}

fn hydra_plan(g: &Game) -> Result<Plan, String> {
    let (root, id) = hydra_ids(g)?;
    let all = hydra_records(&root)?;
    let (delete, also, mut kept) = hydra_dirs(&all, &id);
    kept.extend(g.prefix.iter().cloned());
    let also: Vec<PathBuf> = also.into_iter().collect();
    let bytes = delete
        .iter()
        .chain(&also)
        .map(|d| crate::steamlib::tree_size(d))
        .sum();
    Ok(Plan {
        key: g.key.clone(),
        delete,
        also,
        bytes,
        kept,
        close: Some(("Hydra", "hydralauncher")),
    })
}

/// Not installed any more, as Hydra marks it when its files are removed
/// (the game stays in its library). Its download record goes too when
/// the download was deleted. The base is copied to `hydra-db.lenta-backup`
/// first.
pub fn hydra_forget(root: &Path, id: &str, download_gone: bool) -> Result<(), String> {
    let db = root.join("hydra-db");
    let all = hydra_records(root)?;
    let games = crate::sources::hydra::sublevel(&all, "games");
    let Some(mut g) = games.get(id).cloned() else {
        return Ok(());
    };
    let backup = root.join("hydra-db.lenta-backup");
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
    for e in std::fs::read_dir(&db).map_err(|e| e.to_string())?.flatten() {
        if e.file_name() != "LOCK" && e.path().is_file() {
            std::fs::copy(e.path(), backup.join(e.file_name())).map_err(|e| e.to_string())?;
        }
    }
    if let Some(o) = g.as_object_mut() {
        for k in ["executablePath", "installedSizeInBytes", "installerSizeInBytes"] {
            o.insert(k.into(), Value::Null);
        }
        o.insert("automaticCloudSync".into(), Value::Bool(false));
    }
    let del = if download_gone {
        vec![format!("!downloads!{id}")]
    } else {
        Vec::new()
    };
    crate::sources::hydra::write_db(&db, &[(format!("!games!{id}"), g)], &del)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let t = std::env::temp_dir().join(format!("lenta-uninst-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        t
    }

    #[test]
    fn heroic_lists_changed() {
        let r = tmp("heroic");
        std::fs::create_dir_all(r.join("legendaryConfig/legendary")).unwrap();
        std::fs::create_dir_all(r.join("gog_store")).unwrap();
        std::fs::create_dir_all(r.join("store_cache")).unwrap();
        std::fs::write(
            r.join("legendaryConfig/legendary/installed.json"),
            r#"{"Fox": {"install_path": "/g/Fox"}, "Owl": {"install_path": "/g/Owl"}}"#,
        )
        .unwrap();
        std::fs::write(
            r.join("store_cache/legendary_library.json"),
            r#"{"library": [{"app_name": "Fox", "is_installed": true, "install": {"install_path": "/g/Fox"}}]}"#,
        )
        .unwrap();
        std::fs::write(
            r.join("gog_store/installed.json"),
            r#"{"installed": [{"appName": "123", "install_path": "/g/Gog"}, {"appName": "9"}]}"#,
        )
        .unwrap();
        heroic_forget(&r, "legendary", "Fox").unwrap();
        heroic_forget(&r, "gog", "123").unwrap();
        let inst = read(&r.join("legendaryConfig/legendary/installed.json")).unwrap();
        assert!(inst.get("Fox").is_none() && inst.get("Owl").is_some());
        let lib = read(&r.join("store_cache/legendary_library.json")).unwrap();
        assert_eq!(lib["library"][0]["is_installed"], Value::Bool(false));
        let gog = read(&r.join("gog_store/installed.json")).unwrap();
        assert_eq!(gog["installed"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(r);
    }

    #[test]
    fn lutris_marked_not_installed() {
        let r = tmp("lutris");
        let db = r.join("pga.db");
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE games (id INTEGER PRIMARY KEY, slug TEXT, directory TEXT, installed INTEGER, installed_at INTEGER);
             INSERT INTO games VALUES (1, 'a', '', 1, 5), (2, 'b', '/home/x/Games/b', 1, 6);",
        )
        .unwrap();
        assert_eq!(
            lutris_dir(&db, 1),
            None,
            "added by hand: no folder of its own"
        );
        assert_eq!(lutris_dir(&db, 2), Some(PathBuf::from("/home/x/Games/b")));
        lutris_forget(&db, 2).unwrap();
        let (i, at): (i64, Option<i64>) = c
            .query_row(
                "SELECT installed, installed_at FROM games WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((i, at), (0, None));
        let _ = std::fs::remove_dir_all(r);
    }

    #[test]
    fn hydra_game_and_download_go() {
        use serde_json::json;
        let t = tmp("hydra");
        let home = t.join("h");
        let dsr = home.join("Games/Dark Souls/DSR.exe");
        let pbz = home.join("Games/PBZ/Win64/PBZ.exe");
        let lone = home.join("Games/lone.exe");
        for e in [&dsr, &pbz, &lone] {
            std::fs::create_dir_all(e.parent().unwrap()).unwrap();
            std::fs::write(e, b"MZ").unwrap();
        }
        let dl = home.join("Загрузки");
        std::fs::create_dir_all(dl.join("[RG] Dark Souls")).unwrap();
        std::fs::create_dir_all(dl.join("PBZ-Repack")).unwrap();
        crate::sources::hydra::tests::fake(
            &t,
            &[
                ("!games!steam:1", json!({"title": "DSR", "executablePath": dsr})),
                ("!games!steam:2", json!({"title": "PBZ", "executablePath": pbz})),
                ("!games!steam:3", json!({"title": "Lone", "executablePath": lone})),
                ("!downloads!steam:1", json!({"downloadPath": dl, "folderName": "[RG] Dark Souls"})),
                ("!downloads!steam:2", json!({"downloadPath": dl, "folderName": "PBZ-Repack"})),
            ],
        );
        let all = hydra_records(&t).unwrap();
        let (g, d, kept) = hydra_dirs(&all, "steam:1");
        assert_eq!(g, Some(home.join("Games/Dark Souls")));
        assert_eq!(d, Some(dl.join("[RG] Dark Souls")));
        assert!(kept.is_empty());
        // Up past Win64.
        assert_eq!(hydra_dirs(&all, "steam:2").0, Some(home.join("Games/PBZ")));
        // A program right in ~/Games: that folder is everyone's.
        let (g, _, kept) = hydra_dirs(&all, "steam:3");
        assert_eq!((g, kept), (None, vec![home.join("Games")]));

        hydra_forget(&t, "steam:1", true).unwrap();
        let all = hydra_records(&t).unwrap();
        let games = crate::sources::hydra::sublevel(&all, "games");
        assert_eq!(games["steam:1"]["executablePath"], Value::Null);
        assert_eq!(games["steam:1"]["title"], "DSR");
        assert!(games["steam:2"]["executablePath"].is_string());
        let dls = crate::sources::hydra::sublevel(&all, "downloads");
        assert!(!dls.contains_key("steam:1") && dls.contains_key("steam:2"));
        assert!(t.join("hydra-db.lenta-backup/CURRENT").is_file());
        let _ = std::fs::remove_dir_all(t);
    }

    #[test]
    fn only_safe_folders_go() {
        let home = dirs::home_dir().unwrap();
        assert!(!deletable(&home));
        assert!(!deletable(Path::new("/usr/share/games/x")));
        assert!(!deletable(Path::new("/home")));
        let t = tmp("safe");
        let g = t.join("Games/Game");
        std::fs::create_dir_all(&g).unwrap();
        assert!(deletable(&g));
        let _ = std::fs::remove_dir_all(t);
    }
}
