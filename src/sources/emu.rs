//! Achievements kept by the Steam emulators cracked games ship with
//! (CODEX, RUNE, EMPRESS, Goldberg/GSE, OnlineFix…), in the game's Wine
//! prefix. Their folders are named by the Steam app id, which also tells
//! which game it is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where an emulator keeps one game's achievements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    pub appid: u32,
    pub file: PathBuf,
}

fn subdirs(p: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// `<dir>/<appid>/<rest>` for every numeric child of `dir`.
fn by_appid(dir: &Path, rest: &[&str], out: &mut Vec<Save>) {
    for d in subdirs(dir) {
        let Some(appid) = d
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|&n| n > 0)
        else {
            continue;
        };
        for r in rest {
            let file = d.join(r.replace("{id}", &appid.to_string()));
            if file.is_file() {
                out.push(Save { appid, file });
                break;
            }
        }
    }
}

/// Every emulator save in a Wine prefix.
pub fn saves(prefix: &Path) -> Vec<Save> {
    let c = prefix.join("drive_c");
    let mut out = Vec::new();
    let public = c.join("users/Public/Documents");
    // CODEX, RUNE and friends: Public/Documents/Steam/<group>/<appid>.
    for group in subdirs(&public.join("Steam")) {
        by_appid(&group, &["achievements.ini"], &mut out);
    }
    by_appid(
        &public.join("OnlineFix"),
        &["Stats/Achievements.ini", "Stats/achievements.ini"],
        &mut out,
    );
    for user in subdirs(&c.join("users")) {
        let roaming = user.join("AppData/Roaming");
        for d in ["Goldberg SteamEmu Saves", "GSE Saves"] {
            by_appid(&roaming.join(d), &["achievements.json"], &mut out);
        }
        by_appid(
            &roaming.join("EMPRESS"),
            &["remote/{id}/achievements.json", "achievements.json"],
            &mut out,
        );
        by_appid(
            &roaming.join("Steam/CODEX"),
            &["achievements.ini"],
            &mut out,
        );
        by_appid(
            &user.join("Documents/Steam/CODEX"),
            &["achievements.ini"],
            &mut out,
        );
    }
    // RLD!, DODI and others under ProgramData/Steam/<group>/<appid>/stats.
    for group in subdirs(&c.join("ProgramData/Steam")) {
        by_appid(&group, &["stats/achievements.ini"], &mut out);
    }
    out
}

/// The Steam app id a game folder names: steam_appid.txt, or an
/// emulator's settings (AppId=…). Looks a few levels down.
pub fn appid_in(dir: &Path) -> Option<u32> {
    let mut queue = vec![(dir.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((d, depth)) = queue.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            seen += 1;
            if seen > 4000 {
                return None;
            }
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_lowercase();
            if p.is_dir() {
                if depth < 3 && !name.starts_with('.') {
                    queue.push((p, depth + 1));
                }
                continue;
            }
            let id = match name.as_str() {
                "steam_appid.txt" => std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|s| s.trim().parse().ok()),
                "steam_emu.ini" | "codex.ini" | "rune.ini" | "steam_api.ini" | "onlinefix.ini"
                | "cream_api.ini" | "ds.ini" => ini_appid(&p),
                _ => None,
            };
            if let Some(id) = id.filter(|&i: &u32| i > 0) {
                return Some(id);
            }
        }
    }
    None
}

fn ini_appid(p: &Path) -> Option<u32> {
    let s = std::fs::read(p).ok()?;
    let s = String::from_utf8_lossy(&s);
    s.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        let k = k.trim().to_lowercase();
        (k == "appid" || k == "realappid")
            .then(|| v.trim().parse().ok())
            .flatten()
    })
}

/// This game's app id and emulator save, from its prefix and folder. A
/// prefix shared by several games needs the folder's id to pick one.
pub fn resolve(prefix: Option<&Path>, dir: Option<&Path>) -> (Option<u32>, Option<Save>) {
    let from_dir = dir.and_then(appid_in);
    let all = prefix.map(saves).unwrap_or_default();
    let save = match from_dir {
        Some(id) => all.into_iter().find(|s| s.appid == id),
        None => {
            let mut ids: Vec<u32> = all.iter().map(|s| s.appid).collect();
            ids.sort_unstable();
            ids.dedup();
            if ids.len() == 1 {
                all.into_iter().next()
            } else {
                None
            }
        }
    };
    (from_dir.or(save.as_ref().map(|s| s.appid)), save)
}

/// The achievements a save marks as earned (API name → unix time, 0 when
/// none was kept).
pub fn unlocked(file: &Path) -> HashMap<String, u64> {
    let Ok(b) = std::fs::read(file) else {
        return HashMap::new();
    };
    let is_json = file
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("json"));
    if is_json {
        parse_json(&b)
    } else {
        parse_ini(&String::from_utf8_lossy(&b))
    }
}

fn parse_json(b: &[u8]) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    let Ok(serde_json::Value::Object(m)) = serde_json::from_slice::<serde_json::Value>(b) else {
        return out;
    };
    for (k, v) in m {
        let earned = v
            .get("earned")
            .or_else(|| v.get("achieved"))
            .is_some_and(|e| e.as_bool() == Some(true) || e.as_i64() == Some(1));
        if earned {
            let t = ["earned_time", "unlock_time", "time"]
                .iter()
                .find_map(|f| v.get(*f).and_then(|t| t.as_u64()))
                .unwrap_or(0);
            out.insert(k, t);
        }
    }
    out
}

fn truthy(v: &str) -> bool {
    let v = v.trim().to_lowercase();
    v == "1" || v == "true"
}

fn parse_ini(s: &str) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    let mut section: Option<String> = None;
    let mut got = false;
    let mut time = 0u64;
    let mut flush = |name: &Option<String>, got: bool, time: u64| {
        if let Some(n) = name
            && got
            && !n.eq_ignore_ascii_case("SteamAchievements")
        {
            out.insert(n.clone(), time);
        }
    };
    for line in s.lines() {
        let line = line.trim();
        if let Some(n) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            flush(&section, got, time);
            section = Some(n.to_string());
            got = false;
            time = 0;
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim().to_lowercase().as_str() {
            "achieved" | "unlocked" | "haveachieved" | "state" => got = truthy(v),
            "unlocktime" | "unlock_time" | "timestamp" | "haveachievedtime" | "time" => {
                time = v.trim().parse().unwrap_or(0)
            }
            _ => {}
        }
    }
    flush(&section, got, time);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_ini() {
        let m = parse_ini(
            "[ACH_1]\nAchieved=1\nCurProgress=0\nMaxProgress=0\nUnlockTime=1700000000\n\
             [ACH_2]\nAchieved=0\nUnlockTime=0\n\
             [SteamAchievements]\nCount=1\n00000=ACH_1\n",
        );
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("ACH_1"), Some(&1700000000));
    }

    #[test]
    fn onlinefix_ini() {
        let m = parse_ini("[NEW_ACH]\nachieved=true\ntimestamp=1600000000\n");
        assert_eq!(m.get("NEW_ACH"), Some(&1600000000));
    }

    #[test]
    fn goldberg_json() {
        let m = parse_json(
            br#"{"A":{"earned":true,"earned_time":5},"B":{"earned":false,"earned_time":0}}"#,
        );
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("A"), Some(&5));
    }

    #[test]
    fn finds_save_in_prefix() {
        let root = std::env::temp_dir().join(format!("lenta-emu-{}", std::process::id()));
        let pfx = root.join("pfx");
        let rune = pfx.join("drive_c/users/Public/Documents/Steam/RUNE/1636440");
        std::fs::create_dir_all(&rune).unwrap();
        std::fs::write(
            rune.join("achievements.ini"),
            "[ACH_1]\nAchieved=1\nUnlockTime=42\n[SteamAchievements]\nCount=1\n",
        )
        .unwrap();
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        let (id, save) = resolve(Some(&pfx), Some(&game));
        assert_eq!(id, Some(1636440));
        let save = save.unwrap();
        assert_eq!(unlocked(&save.file).get("ACH_1"), Some(&42));
        // A shared prefix: the game's own steam_appid.txt picks the save.
        let other = pfx.join("drive_c/users/Public/Documents/Steam/CODEX/570");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("achievements.ini"), "").unwrap();
        assert_eq!(resolve(Some(&pfx), Some(&game)).1, None);
        std::fs::write(game.join("steam_appid.txt"), "570\n").unwrap();
        assert_eq!(
            resolve(Some(&pfx), Some(&game)).1.map(|s| s.appid),
            Some(570)
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
