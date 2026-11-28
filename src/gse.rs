//! Goldberg / GSE (gbe_fork) only keeps the achievements its
//! `steam_settings/achievements.json` lists: without that file every
//! unlock the game asks for is dropped. Some releases ship without it;
//! this writes one from Steam's list (names in English and Russian,
//! hidden flags, icons), the way GSE's own generate_emu_config does.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// A GSE `steam_settings` folder that lacks the achievements list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    pub dir: PathBuf,
}

fn is_steam_api(name: &str) -> bool {
    let n = name.to_lowercase();
    n == "steam_api64.dll" || n == "steam_api.dll"
}

/// Is `d` (holding steam_api*.dll) set up for GSE: a steam_settings
/// folder beside it, with GSE's own files in.
fn gse_settings(d: &Path) -> Option<PathBuf> {
    let s = d.join("steam_settings");
    if !s.is_dir() {
        return None;
    }
    let ours = [
        "configs.app.ini",
        "configs.user.ini",
        "configs.main.ini",
        "steam_appid.txt",
    ];
    ours.iter().any(|f| s.join(f).is_file()).then_some(s)
}

/// The GSE settings folders in a game's folder (a few levels down) that
/// have no achievements list.
pub fn missing(dir: &Path) -> Vec<Missing> {
    let mut out = Vec::new();
    let mut queue = vec![(dir.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((d, depth)) = queue.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        let mut has_api = false;
        for e in rd.flatten() {
            seen += 1;
            if seen > 4000 {
                return out;
            }
            let name = e.file_name().to_string_lossy().to_string();
            let p = e.path();
            if p.is_dir() {
                if depth < 4 && !name.starts_with('.') && name != "steam_settings" {
                    queue.push((p, depth + 1));
                }
            } else if is_steam_api(&name) {
                has_api = true;
            }
        }
        if has_api
            && let Some(s) = gse_settings(&d)
            && !s.join("achievements.json").is_file()
        {
            out.push(Missing { dir: s });
        }
    }
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

/// Where to look for a game: its folder, or above Binaries/Win64.
pub fn missing_for(install_dir: Option<&Path>) -> Vec<Missing> {
    let Some(d) = install_dir else {
        return Vec::new();
    };
    let mut root = d;
    while crate::sources::emu::is_bin_dir(root) {
        match root.parent() {
            Some(p) => root = p,
            None => break,
        }
    }
    missing(root)
}

fn icon_name(url: &str) -> String {
    let base = url.rsplit('/').next().unwrap_or(url);
    let base = base.split('?').next().unwrap_or(base);
    if base.is_empty() {
        format!("{:016x}.jpg", crate::net::hash(url))
    } else {
        base.to_string()
    }
}

/// The list GSE reads, from Steam's lists in each language.
fn build(
    en: &[crate::sources::steam::Achievement],
    ru: &[crate::sources::steam::Achievement],
    icons: &dyn Fn(&crate::game::Art) -> Option<String>,
) -> Value {
    let arr: Vec<Value> = en
        .iter()
        .map(|a| {
            let r = ru.iter().find(|x| x.id == a.id);
            let mut name = json!({ "english": a.name });
            let mut desc = json!({ "english": a.desc });
            if let Some(r) = r {
                name["russian"] = json!(r.name);
                desc["russian"] = json!(r.desc);
            }
            let mut v = json!({
                "name": a.id,
                "displayName": name,
                "description": desc,
                "hidden": if a.hidden { 1 } else { 0 },
            });
            if let Some(i) = a.icon.as_ref().and_then(icons) {
                v["icon"] = json!(i);
            }
            if let Some(i) = a.icon_gray.as_ref().and_then(icons) {
                v["icongray"] = json!(i);
            }
            v
        })
        .collect();
    Value::Array(arr)
}

/// Write the list (and icons) into `m.dir`; how many achievements. Never
/// replaces a list already there.
pub fn write(m: &Missing, appid: u32) -> Result<usize, String> {
    let target = m.dir.join("achievements.json");
    if target.exists() {
        return Err("achievements.json is already there".into());
    }
    let en = crate::sources::web::achievement_list(appid, "english")
        .ok_or("Steam gave no achievements list")?;
    if en.is_empty() {
        return Err("Steam lists no achievements for this game".into());
    }
    let ru = crate::sources::web::achievement_list(appid, "russian").unwrap_or_default();
    let img = m.dir.join("achievement_images");
    let _ = std::fs::create_dir_all(&img);
    let icons = |a: &crate::game::Art| -> Option<String> {
        let crate::game::Art::Url(u) = a else {
            return None;
        };
        let name = icon_name(u);
        let f = img.join(&name);
        if !f.is_file() {
            let b = crate::net::get(u).ok()?;
            std::fs::write(&f, b).ok()?;
        }
        Some(format!("achievement_images/{name}"))
    };
    let v = build(&en, &ru, &icons);
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    let tmp = m.dir.join("achievements.json.lenta-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    Ok(en.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::steam::Achievement;

    fn ach(id: &str, name: &str, hidden: bool) -> Achievement {
        Achievement {
            id: id.into(),
            name: name.into(),
            desc: format!("{name}!"),
            icon: Some(crate::game::Art::Url(format!("https://x/{id}.jpg"))),
            icon_gray: None,
            hidden,
            unlocked: None,
            percent: None,
        }
    }

    #[test]
    fn finds_gse_without_list() {
        let t = std::env::temp_dir().join(format!("lenta-gse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let bin = t.join("Game/Binaries/Win64");
        std::fs::create_dir_all(bin.join("steam_settings")).unwrap();
        std::fs::write(bin.join("steam_api64.dll"), "").unwrap();
        std::fs::write(bin.join("steam_settings/configs.user.ini"), "").unwrap();
        // Another steam_api without GSE settings: not ours to touch.
        let other = t.join("Engine/Bin");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("steam_api.dll"), "").unwrap();
        let m = missing_for(Some(&bin));
        assert_eq!(
            m,
            vec![Missing {
                dir: bin.join("steam_settings")
            }]
        );
        std::fs::write(bin.join("steam_settings/achievements.json"), "[]").unwrap();
        assert!(missing_for(Some(&bin)).is_empty());
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn list_as_gse_reads_it() {
        let en = [ach("ACH_1", "One", false), ach("ACH_2", "Two", true)];
        let ru = [ach("ACH_1", "Один", false)];
        let v = build(&en, &ru, &|a| match a {
            crate::game::Art::Url(u) => Some(format!("achievement_images/{}", icon_name(u))),
            _ => None,
        });
        assert_eq!(v[0]["name"], "ACH_1");
        assert_eq!(v[0]["displayName"]["russian"], "Один");
        assert_eq!(v[0]["hidden"], 0);
        assert_eq!(v[1]["hidden"], 1);
        assert_eq!(v[0]["icon"], "achievement_images/ACH_1.jpg");
        assert!(v[1]["displayName"].get("russian").is_none());
    }
}

/// Writes the list into a real game (run by hand):
/// `LENTA_GSE_GAME=<exe folder> LENTA_GSE_APPID=<id> cargo test gse::live -- --ignored`.
#[cfg(test)]
mod live {
    #[test]
    #[ignore]
    fn real_game() {
        let dir = std::path::PathBuf::from(std::env::var("LENTA_GSE_GAME").unwrap());
        let id: u32 = std::env::var("LENTA_GSE_APPID").unwrap().parse().unwrap();
        let m = super::missing_for(Some(&dir));
        println!("{m:?}");
        for x in &m {
            println!("{} → {:?}", x.dir.display(), super::write(x, id));
        }
    }
}
