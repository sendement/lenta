//! What the Steam client keeps about the user's library besides the games:
//! collections (synced through Steam Cloud, mirrored locally), and which
//! compatibility tool (Proton) each game runs with. All local files.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::steam::{appinfo, roots, users};
use crate::vdf::{self, Kv};

/// The Steam install the launcher works with: the first one found.
pub fn root() -> Option<PathBuf> {
    roots().into_iter().next()
}

// ------------------------------------------------------------ collections

#[derive(Clone, Debug)]
pub struct Collection {
    /// "favorite", "hidden", or "uc-…" for the user's own.
    pub id: String,
    pub name: String,
    pub apps: Vec<u32>,
}

/// The user's collections: favourites first, the hidden ones last, the
/// rest by name. Dynamic ones (kept as filters, not lists) are left out.
pub fn collections() -> Vec<Collection> {
    let Some(root) = root() else {
        return Vec::new();
    };
    for (_, dir) in users(&root) {
        let f = dir.join("config/cloudstorage/cloud-storage-namespace-1.json");
        let Ok(b) = std::fs::read(&f) else { continue };
        let Some(mut list) = parse_collections(&b) else {
            continue;
        };
        let rank = |c: &Collection| match c.id.as_str() {
            "favorite" => 0,
            "hidden" => 2,
            _ => 1,
        };
        list.sort_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        return list;
    }
    Vec::new()
}

fn parse_collections(b: &[u8]) -> Option<Vec<Collection>> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let mut out = Vec::new();
    for e in v.as_array()? {
        let (Some(key), Some(meta)) = (e.get(0).and_then(Value::as_str), e.get(1)) else {
            continue;
        };
        if !key.starts_with("user-collections.")
            || meta.get("is_deleted").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(val) = meta
            .get("value")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
        else {
            continue;
        };
        if val.get("filterSpec").is_some_and(|f| !f.is_null()) {
            continue;
        }
        let removed: HashSet<u64> = val
            .get("removed")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_u64).collect())
            .unwrap_or_default();
        let apps: Vec<u32> = val
            .get("added")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_u64)
                    .filter(|id| !removed.contains(id))
                    .map(|id| id as u32)
                    .collect()
            })
            .unwrap_or_default();
        let id = val
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(key.trim_start_matches("user-collections."))
            .to_string();
        let name = val
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_string();
        out.push(Collection { id, name, apps });
    }
    Some(out)
}

// ------------------------------------------------------------- Steam Play

#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    /// What config.vdf stores: "proton_experimental", "GE-Proton11-7-x86_64".
    pub name: String,
    pub display: String,
    /// Steam's own (fetched on demand) rather than one put in by hand.
    pub official: bool,
    /// Steam's app id for an official one (installed and removed by Steam).
    pub appid: Option<u32>,
    /// On disk: an official one Steam has installed, or any other.
    pub installed: bool,
    /// Its folder (custom and system ones).
    pub dir: Option<PathBuf>,
    /// In the user's own compatibilitytools.d (system ones belong to
    /// packages).
    pub removable: bool,
}

/// The tools that run Windows games: Steam's Protons (newest first) and
/// the ones in compatibilitytools.d.
pub fn tools() -> Vec<Tool> {
    let Some(root) = root() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // Steam Play's own list lives in the appinfo of app 891390.
    let info = appinfo(&root, &[891390].into_iter().collect());
    if let Some(list) = info
        .get(&891390)
        .and_then(|kv| kv.path(&["appinfo", "extended", "compat_tools"]))
    {
        let installed_apps: HashSet<u32> = super::steam::installed_app_ids(&root);
        for (name, t) in list.entries() {
            if t.get_str("from_oslist") != Some("windows") {
                continue;
            }
            let appid = t.get_int("appid").map(|a| a as u32);
            out.push(Tool {
                name: name.clone(),
                display: t.get_str("display_name").unwrap_or(name).to_string(),
                official: true,
                installed: appid.is_some_and(|a| installed_apps.contains(&a)),
                appid,
                dir: None,
                removable: false,
            });
        }
    }
    let mut dirs = vec![root.join("compatibilitytools.d")];
    if let Some(h) = dirs::home_dir() {
        dirs.push(h.join(".steam/root/compatibilitytools.d"));
    }
    dirs.push(crate::sandbox::system(
        "/usr/share/steam/compatibilitytools.d",
    ));
    dirs.push(crate::sandbox::system(
        "/usr/local/share/steam/compatibilitytools.d",
    ));
    let mut custom: Vec<Tool> = Vec::new();
    let own = root.join("compatibilitytools.d").canonicalize().ok();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let manifest = if p.is_dir() {
                p.join("compatibilitytool.vdf")
            } else if p.extension().is_some_and(|x| x == "vdf") {
                p
            } else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            let kv = vdf::parse_text(&text);
            let Some(list) = kv.path(&["compatibilitytools", "compat_tools"]) else {
                continue;
            };
            for (name, t) in list.entries() {
                if custom.iter().any(|c| c.name == *name) {
                    continue;
                }
                let dir = if e.path().is_dir() {
                    Some(e.path())
                } else {
                    None
                };
                let removable = dir
                    .as_ref()
                    .and_then(|d| d.canonicalize().ok())
                    .is_some_and(|d| d.parent().map(Path::to_path_buf) == own);
                custom.push(Tool {
                    name: name.clone(),
                    display: t.get_str("display_name").unwrap_or(name).to_string(),
                    official: false,
                    appid: None,
                    installed: true,
                    dir,
                    removable,
                });
            }
        }
    }
    // Newest versions first (names hold the numbers).
    custom.sort_by(|a, b| natural(&b.display, &a.display));
    out.extend(custom);
    out
}

/// Compare with digit runs as numbers ("GE-Proton11-7" after "…11-10" is
/// wrong, so 7 < 10).
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> Vec<Result<u64, String>> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut digits = false;
        for c in s.chars() {
            if c.is_ascii_digit() != digits && !cur.is_empty() {
                out.push(if digits {
                    Ok(cur.parse().unwrap_or(0))
                } else {
                    Err(cur.to_lowercase())
                });
                cur.clear();
            }
            digits = c.is_ascii_digit();
            cur.push(c);
        }
        if !cur.is_empty() {
            out.push(if digits {
                Ok(cur.parse().unwrap_or(0))
            } else {
                Err(cur.to_lowercase())
            });
        }
        out
    };
    split(a).cmp(&split(b))
}

fn config_path(root: &Path) -> PathBuf {
    root.join("config/config.vdf")
}

/// Each game's chosen tool; the entry for 0 is the default for all.
pub fn mapping() -> HashMap<u32, String> {
    let Some(root) = root() else {
        return HashMap::new();
    };
    let Ok(text) = std::fs::read_to_string(config_path(&root)) else {
        return HashMap::new();
    };
    let kv = vdf::parse_text(&text);
    let Some(m) = kv.path(&[
        "InstallConfigStore",
        "Software",
        "Valve",
        "Steam",
        "CompatToolMapping",
    ]) else {
        return HashMap::new();
    };
    m.entries()
        .iter()
        .filter_map(|(id, v)| {
            let name = v.get_str("name").filter(|n| !n.is_empty())?;
            Some((id.parse().ok()?, name.to_string()))
        })
        .collect()
}

/// Is the Steam client running? (It rewrites config.vdf when it quits, so
/// changes made meanwhile would be lost.)
pub fn steam_running() -> bool {
    crate::running::named("steam")
}

/// Set (Some) or clear (None) the tool of these games in config.vdf.
/// Steam must not be running; a copy of the old file is kept beside it.
pub fn set_tools(changes: &[(u32, Option<String>)]) -> Result<(), String> {
    if std::env::var_os("LENTA_DRY_RUN").is_some() {
        return Ok(());
    }
    let root = root().ok_or("Steam not found")?;
    if steam_running() {
        return Err("Steam is running".into());
    }
    let path = config_path(&root);
    let _ = std::fs::copy(&path, root.join("config/config.vdf.lenta-backup"));
    write_tools(&path, changes)
}

/// The edit itself, on any config.vdf.
fn write_tools(path: &Path, changes: &[(u32, Option<String>)]) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut kv = vdf::parse_text(&text);
    let map = kv
        .path_mut(&[
            "InstallConfigStore",
            "Software",
            "Valve",
            "Steam",
            "CompatToolMapping",
        ])
        .ok_or("config.vdf: unexpected layout")?;
    for (id, tool) in changes {
        let id = id.to_string();
        match tool {
            Some(name) => {
                let e = map.map_mut(&id).ok_or("config.vdf: unexpected layout")?;
                e.set("name", Kv::Str(name.clone()));
                e.set("config", Kv::Str(String::new()));
                e.set("priority", Kv::Str("250".into()));
            }
            None => map.remove(&id),
        }
    }
    let out = kv.to_text();
    // Never write something that doesn't read back the same.
    if vdf::parse_text(&out) != kv {
        return Err("config.vdf: could not be written safely".into());
    }
    let tmp = path.with_extension("vdf.lenta-tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(natural("GE-Proton11-10", "GE-Proton11-7").is_gt());
        assert!(natural("GE-Proton9-27", "GE-Proton10-1").is_lt());
    }

    /// On a copy of the real config.vdf (or a small one): set, change,
    /// clear, and everything else stays byte for byte.
    #[test]
    fn writes_tool_mapping() {
        let dir = std::env::temp_dir().join(format!("lenta-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("config.vdf");
        let real = roots()
            .into_iter()
            .find_map(|r| std::fs::read_to_string(r.join("config/config.vdf")).ok());
        let src = real.unwrap_or_else(|| {
            "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"x\"\t\t\"1\"\n\t\t\t}\n\t\t}\n\t}\n}\n".into()
        });
        std::fs::write(&f, &src).unwrap();
        let id = 4_000_000_001u32;
        let get = |f: &Path| {
            let kv = vdf::parse_text(&std::fs::read_to_string(f).unwrap());
            kv.path(&[
                "InstallConfigStore",
                "Software",
                "Valve",
                "Steam",
                "CompatToolMapping",
            ])
            .and_then(|m| m.get(&id.to_string()))
            .and_then(|e| e.get_str("name"))
            .map(str::to_string)
        };
        write_tools(&f, &[(id, Some("proton_9".into()))]).unwrap();
        assert_eq!(get(&f).as_deref(), Some("proton_9"));
        write_tools(&f, &[(id, Some("GE-Proton11-7-x86_64".into()))]).unwrap();
        assert_eq!(get(&f).as_deref(), Some("GE-Proton11-7-x86_64"));
        write_tools(&f, &[(id, None)]).unwrap();
        assert_eq!(get(&f), None);
        // With the test entry gone, the file reads as it was.
        assert_eq!(
            vdf::parse_text(&std::fs::read_to_string(&f).unwrap()),
            vdf::parse_text(&src)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collections_json() {
        let b = br#"[
          ["user-collections.favorite",{"key":"user-collections.favorite","value":"{\"id\":\"favorite\",\"name\":\"Fav\",\"added\":[10,20],\"removed\":[20]}"}],
          ["user-collections.uc-x",{"key":"user-collections.uc-x","is_deleted":true}],
          ["user-collections.uc-d",{"key":"user-collections.uc-d","value":"{\"id\":\"uc-d\",\"name\":\"Dyn\",\"filterSpec\":{\"a\":1}}"}],
          ["other",{"key":"other","value":"1"}]
        ]"#;
        let c = parse_collections(b).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "Fav");
        assert_eq!(c[0].apps, vec![10]);
    }
}
