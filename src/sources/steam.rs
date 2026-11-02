//! Steam: installed games from every library, non-Steam shortcuts, their
//! art from the library cache (or the user's custom grid art), play time
//! from localconfig.vdf and achievements from appcache/stats — all local.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::game::{Art, Arts, Game, Source};
use crate::vdf::{self, Kv};

/// Steam installs found (native, ~/.steam links, Flatpak), each once.
pub fn roots() -> Vec<PathBuf> {
    if let Some(r) = std::env::var_os("LENTA_STEAM_ROOT") {
        let r = PathBuf::from(r);
        return if r.join("steamapps").is_dir() {
            vec![r]
        } else {
            Vec::new()
        };
    }
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let candidates = [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
    ];
    let mut out: Vec<PathBuf> = Vec::new();
    for c in candidates {
        let Ok(real) = c.canonicalize() else { continue };
        if real.join("steamapps").is_dir() && !out.contains(&real) {
            out.push(real);
        }
    }
    out
}

fn flatpak_root(root: &Path) -> bool {
    root.to_string_lossy().contains("com.valvesoftware.Steam")
}

fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![root.to_path_buf()];
    if let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        let kv = vdf::parse_text(&text);
        if let Some(folders) = kv.get("libraryfolders") {
            for (_, f) in folders.entries() {
                if let Some(p) = f.get_str("path") {
                    libs.push(PathBuf::from(p));
                }
            }
        }
    }
    let mut out = Vec::new();
    for l in libs {
        let l = l.canonicalize().unwrap_or(l);
        if l.join("steamapps").is_dir() && !out.contains(&l) {
            out.push(l);
        }
    }
    out
}

/// userdata/<account id> folders, the most recently logged in first.
pub fn users(root: &Path) -> Vec<(u32, PathBuf)> {
    let mut out: Vec<(u32, PathBuf)> = std::fs::read_dir(root.join("userdata"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            Some((
                e.file_name()
                    .to_str()?
                    .parse::<u32>()
                    .ok()
                    .filter(|&id| id != 0)?,
                e.path(),
            ))
        })
        .collect();
    let recent = std::fs::read_to_string(root.join("config/loginusers.vdf"))
        .ok()
        .and_then(|t| {
            let kv = vdf::parse_text(&t);
            kv.get("users")?
                .entries()
                .iter()
                .find(|(_, u)| u.get_str("MostRecent") == Some("1"))
                .and_then(|(id, _)| id.parse::<u64>().ok())
                .map(|id64| (id64 & 0xffff_ffff) as u32)
        });
    out.sort_by_key(|(id, _)| (Some(*id) != recent, *id));
    out
}

/// What appinfo.vdf says of these apps: their type ("game", "tool"…).
fn app_types(root: &Path, wanted: &HashSet<u32>) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    let Ok(b) = std::fs::read(root.join("appcache/appinfo.vdf")) else {
        return out;
    };
    let mut r = vdf::Reader { b: &b, at: 0 };
    let Some(magic) = r.u32() else { return out };
    let _universe = r.u32();
    let v29 = magic == 0x0756_4429;
    if !v29 && magic != 0x0756_4428 {
        return out;
    }
    let mut table: Vec<String> = Vec::new();
    if v29 {
        let Some(off) = b
            .get(8..16)
            .map(|s| u64::from_le_bytes(s.try_into().unwrap()) as usize)
        else {
            return out;
        };
        r.at = 16;
        let mut t = vdf::Reader { b: &b, at: off };
        let n = t.u32().unwrap_or(0);
        for _ in 0..n {
            match t.cstr() {
                Some(s) => table.push(s.to_string()),
                None => break,
            }
        }
    }
    loop {
        let Some(appid) = r.u32() else { break };
        if appid == 0 {
            break;
        }
        let Some(size) = r.u32() else { break };
        let end = r.at + size as usize;
        if wanted.contains(&appid) {
            // State, update time, token, sha1, change number, binary sha1.
            let header = 4 + 4 + 8 + 20 + 4 + 20;
            let mut kv = vdf::Reader {
                b: b.get(..end.min(b.len())).unwrap_or(&[]),
                at: r.at + header,
            };
            if let Some(m) = kv.map(v29.then_some(table.as_slice())) {
                let kv = Kv::Map(m);
                if let Some(t) = kv.path(&["appinfo", "common", "type"]).and_then(Kv::str) {
                    out.insert(appid, t.to_lowercase());
                }
            }
        }
        r.at = end;
    }
    out
}

/// Minutes played and last played, per app, from every user's localconfig.
fn playtimes(root: &Path) -> HashMap<u32, (u64, u64)> {
    let mut out: HashMap<u32, (u64, u64)> = HashMap::new();
    for (_, dir) in users(root) {
        let Ok(t) = std::fs::read_to_string(dir.join("config/localconfig.vdf")) else {
            continue;
        };
        let kv = vdf::parse_text(&t);
        let Some(apps) = kv.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"])
        else {
            continue;
        };
        for (id, a) in apps.entries() {
            let Ok(id) = id.parse::<u32>() else { continue };
            let play = a.get_int("Playtime").unwrap_or(0).max(0) as u64;
            let last = a.get_int("LastPlayed").unwrap_or(0).max(0) as u64;
            let e = out.entry(id).or_default();
            e.0 = e.0.max(play);
            e.1 = e.1.max(last);
        }
    }
    out
}

fn first_file(cands: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    cands.into_iter().find(|p| p.is_file())
}

/// A file by name in librarycache/<appid>/ or one folder deeper (Steam
/// keeps some art in folders named by hashes).
fn find_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

fn cdn(appid: u32, file: &str) -> Art {
    Art::Url(format!(
        "https://shared.steamstatic.com/store_item_assets/steam/apps/{appid}/{file}"
    ))
}

/// Custom grid art for an id (shortcuts and replaced covers).
fn grid(users: &[(u32, PathBuf)], stem: &str) -> Option<PathBuf> {
    users.iter().find_map(|(_, u)| {
        first_file(
            ["png", "jpg", "jpeg", "webp"].map(|x| u.join(format!("config/grid/{stem}.{x}"))),
        )
    })
}

fn steam_art(root: &Path, users: &[(u32, PathBuf)], appid: u32) -> Arts {
    let cache = root.join("appcache/librarycache");
    let dir = cache.join(appid.to_string());
    // The art in the interface's language first, then the usual.
    let lang = crate::i18n::steam_lang();
    let local = |names: &[&str]| -> Option<PathBuf> {
        for n in names {
            let (stem, ext) = n.rsplit_once('.').unwrap_or((n, ""));
            if let Some(p) = find_in(&dir, &format!("{stem}_{lang}.{ext}")) {
                return Some(p);
            }
        }
        names.iter().find_map(|n| {
            find_in(&dir, n).or_else(|| first_file([cache.join(format!("{appid}_{n}"))]))
        })
    };
    let icon = std::fs::read_dir(&dir).ok().and_then(|rd| {
        rd.flatten().map(|e| e.path()).find(|p| {
            p.extension().is_some_and(|x| x == "jpg")
                && p.file_stem().is_some_and(|s| {
                    s.len() == 40 && s.to_string_lossy().chars().all(|c| c.is_ascii_hexdigit())
                })
        })
    });
    let pick = |custom: Option<PathBuf>, local: Option<PathBuf>, file: &str| {
        Some(
            custom
                .or(local)
                .map(Art::File)
                .unwrap_or_else(|| cdn(appid, file)),
        )
    };
    let custom_portrait = grid(users, &format!("{appid}p"));
    let big_portrait = local(&["library_600x900.jpg"]);
    // Newer Steam keeps only a half-size cover; the full one comes from
    // the store's CDN, the small one stays as the fallback.
    let small_portrait = local(&["library_capsule.jpg"]).map(Art::File);
    Arts {
        portrait_alt: if custom_portrait.is_none() && big_portrait.is_none() {
            small_portrait
        } else {
            None
        },
        portrait: pick(custom_portrait, big_portrait, "library_600x900.jpg"),
        landscape: pick(
            grid(users, &appid.to_string()),
            local(&["header.jpg", "library_header.jpg"]),
            "header.jpg",
        ),
        hero: pick(
            grid(users, &format!("{appid}_hero")),
            local(&["library_hero.jpg"]),
            "library_hero.jpg",
        ),
        logo: pick(
            grid(users, &format!("{appid}_logo")),
            local(&["logo.png"]),
            "logo.png",
        ),
        icon: icon
            .or_else(|| first_file([cache.join(format!("{appid}_icon.jpg"))]))
            .map(Art::File),
    }
}

/// Steam's own runtimes and tools, when appinfo can't tell.
fn looks_like_tool(name: &str) -> bool {
    let n = name.to_lowercase();
    [
        "proton",
        "steam linux runtime",
        "steamworks",
        "steamvr",
        "redistributable",
    ]
    .iter()
    .any(|k| n.contains(k))
}

fn steam_cmd(root: &Path, url: String) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    if flatpak_root(root) {
        out.push(vec![
            "flatpak".into(),
            "run".into(),
            "com.valvesoftware.Steam".into(),
            url.clone(),
        ]);
    }
    out.push(vec!["steam".into(), url.clone()]);
    out.push(vec!["xdg-open".into(), url]);
    out
}

pub fn games() -> Vec<Game> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for root in roots() {
        let users = users(&root);
        let mut found = Vec::new();
        for lib in libraries(&root) {
            let Ok(rd) = std::fs::read_dir(lib.join("steamapps")) else {
                continue;
            };
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if !(n.starts_with("appmanifest_") && n.ends_with(".acf")) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(e.path()) else {
                    continue;
                };
                let kv = vdf::parse_text(&text);
                let Some(st) = kv.get("AppState") else {
                    continue;
                };
                let Some(appid) = st.get_int("appid").map(|i| i as u32) else {
                    continue;
                };
                // Bit 4: fully installed (not just queued for download).
                if st.get_int("StateFlags").is_some_and(|f| f & 4 == 0) {
                    continue;
                }
                let name = st.get_str("name").unwrap_or_default().to_string();
                let dir = st
                    .get_str("installdir")
                    .map(|d| lib.join("steamapps/common").join(d));
                found.push((appid, name, dir));
            }
        }
        let ids: HashSet<u32> = found.iter().map(|f| f.0).collect();
        let types = app_types(&root, &ids);
        let times = playtimes(&root);
        for (appid, name, dir) in found {
            let ok = match types.get(&appid).map(String::as_str) {
                Some(t) => matches!(t, "game" | "demo" | "mod" | "beta"),
                None => !looks_like_tool(&name) && appid != 228980,
            };
            if !ok || !seen.insert(appid) {
                continue;
            }
            let (play, last) = times.get(&appid).copied().unwrap_or_default();
            out.push(Game {
                key: format!("steam:{appid}"),
                title: if name.is_empty() {
                    format!("App {appid}")
                } else {
                    name
                },
                source: Source::Steam,
                sub: None,
                art: steam_art(&root, &users, appid),
                playtime_min: (play > 0).then_some(play),
                last_played: (last > 0).then_some(last),
                steam_appid: Some(appid),
                install_dir: dir,
                launch: steam_cmd(&root, format!("steam://rungameid/{appid}")),
            });
        }
        out.extend(shortcuts(&root, &users));
    }
    out
}

/// Non-Steam games added to Steam.
fn shortcuts(root: &Path, users: &[(u32, PathBuf)]) -> Vec<Game> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (_, u) in users {
        let Ok(b) = std::fs::read(u.join("config/shortcuts.vdf")) else {
            continue;
        };
        let Some(kv) = vdf::parse_binary(&b, None) else {
            continue;
        };
        let Some(list) = kv.get("shortcuts") else {
            continue;
        };
        for (_, s) in list.entries() {
            let Some(id) = s.get_int("appid").map(|i| i as u32) else {
                continue;
            };
            let Some(name) = s.get_str("AppName").filter(|n| !n.is_empty()) else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            let file = |stem: String| grid(users, &stem).map(Art::File);
            let icon = s
                .get_str("icon")
                .map(|i| PathBuf::from(i.trim_matches('"')))
                .filter(|p| p.is_file())
                .map(Art::File);
            let long_id = ((id as u64) << 32) | 0x0200_0000;
            let last = s.get_int("LastPlayTime").unwrap_or(0).max(0) as u64;
            out.push(Game {
                key: format!("steam-shortcut:{id}"),
                title: name.to_string(),
                source: Source::Steam,
                sub: Some("Non-Steam".into()),
                art: Arts {
                    portrait_alt: None,
                    portrait: file(format!("{id}p")),
                    landscape: file(id.to_string()),
                    hero: file(format!("{id}_hero")),
                    logo: file(format!("{id}_logo")),
                    icon,
                },
                playtime_min: None,
                last_played: (last > 0).then_some(last),
                steam_appid: None,
                install_dir: s
                    .get_str("StartDir")
                    .map(|d| PathBuf::from(d.trim_matches('"'))),
                launch: steam_cmd(root, format!("steam://rungameid/{long_id}")),
            });
        }
    }
    out
}

// ------------------------------------------------------------ achievements

#[derive(Clone, Debug)]
pub struct Achievement {
    pub name: String,
    pub desc: String,
    pub icon: Option<Art>,
    pub icon_gray: Option<Art>,
    pub hidden: bool,
    /// Unlocked, and when (0 when Steam kept no time).
    pub unlocked: Option<u64>,
}

fn localized(kv: Option<&Kv>, lang: &str) -> String {
    match kv {
        Some(Kv::Str(s)) => s.clone(),
        Some(m @ Kv::Map(_)) => m
            .get_str(lang)
            .or_else(|| m.get_str("english"))
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

/// The game's achievements and which of them the user has, or None when
/// it has none. `lang` is Steam's name for the language ("russian").
pub fn achievements(appid: u32, lang: &str) -> Option<Vec<Achievement>> {
    for root in roots() {
        let stats = root.join("appcache/stats");
        let Ok(b) = std::fs::read(stats.join(format!("UserGameStatsSchema_{appid}.bin"))) else {
            continue;
        };
        let schema = vdf::parse_binary(&b, None)?;
        let game = schema
            .get(&appid.to_string())
            .or_else(|| schema.entries().first().map(|(_, v)| v))?;
        let user = users(&root).into_iter().find_map(|(id, _)| {
            let b = std::fs::read(stats.join(format!("UserGameStats_{id}_{appid}.bin"))).ok()?;
            vdf::parse_binary(&b, None)
        });
        let ucache = user.as_ref().and_then(|u| u.get("cache"));
        let mut out = Vec::new();
        for (sid, stat) in game.get("stats").map(Kv::entries).unwrap_or_default() {
            let Some(bits) = stat.get("bits") else {
                continue;
            };
            let ustat = ucache.and_then(|c| c.get(sid));
            let data = ustat.and_then(|s| s.get_int("data")).unwrap_or(0) as u32;
            let times = ustat.and_then(|s| s.get("AchievementTimes"));
            for (bit, a) in bits.entries() {
                let Ok(bit) = bit.parse::<u32>() else {
                    continue;
                };
                let d = a.get("display");
                let icon = |k: &str| {
                    d.and_then(|d| d.get_str(k)).filter(|s| !s.is_empty()).map(|f| {
                        Art::Url(format!("https://shared.fastly.steamstatic.com/community_assets/images/apps/{appid}/{f}"))
                    })
                };
                let got = bit < 32 && data & (1 << bit) != 0;
                out.push(Achievement {
                    name: localized(d.and_then(|d| d.get("name")), lang),
                    desc: localized(d.and_then(|d| d.get("desc")), lang),
                    icon: icon("icon"),
                    icon_gray: icon("icon_gray"),
                    hidden: d.and_then(|d| d.get_int("hidden")).unwrap_or(0) != 0,
                    unlocked: got.then(|| {
                        times
                            .and_then(|t| t.get_int(&bit.to_string()))
                            .unwrap_or(0)
                            .max(0) as u64
                    }),
                });
            }
        }
        return (!out.is_empty()).then_some(out);
    }
    None
}
