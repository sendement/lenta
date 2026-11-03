//! An installed Steam game as the downgrader sees it: its appmanifest
//! (build, depots and their manifests), what appinfo.vdf says the branches
//! and depots look like now, and Steam's logs.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::vdf::{self, Kv};

#[derive(Clone, Debug, PartialEq)]
pub struct Depot {
    pub manifest: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct SteamGame {
    pub app: String,
    pub name: String,
    pub acf: PathBuf,
    pub dir: PathBuf,
    pub buildid: i64,
    /// "" is the public branch.
    pub beta: String,
    pub auto_update: i64,
    pub state_flags: i64,
    pub depots: BTreeMap<String, Depot>,
}

impl SteamGame {
    pub fn branch(&self) -> &str {
        if self.beta.is_empty() {
            "public"
        } else {
            &self.beta
        }
    }

    /// The installed game with this app id, from any library.
    pub fn find(root: &Path, app: &str) -> Option<SteamGame> {
        libraries(root).into_iter().find_map(|lib| {
            let acf = lib.join(format!("steamapps/appmanifest_{app}.acf"));
            acf.is_file().then(|| load(&acf, &lib)).flatten()
        })
    }
}

pub fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![root.to_path_buf()];
    if let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        let kv = vdf::parse_text(&text);
        for (_, e) in kv
            .get("libraryfolders")
            .map(Kv::entries)
            .unwrap_or_default()
        {
            if let Some(p) = e.get_str("path").map(PathBuf::from)
                && p.join("steamapps").is_dir()
                && !libs.iter().any(|l| same(l, &p))
            {
                libs.push(p);
            }
        }
    }
    libs
}

fn same(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

fn load(acf: &Path, lib: &Path) -> Option<SteamGame> {
    let kv = vdf::parse_text(&std::fs::read_to_string(acf).ok()?);
    let st = kv.get("AppState")?;
    let app = st.get_str("appid")?.to_string();
    let name = st.get_str("name")?.to_string();
    let installdir = st.get_str("installdir")?;
    let mut depots = BTreeMap::new();
    for (id, d) in st
        .get("InstalledDepots")
        .map(Kv::entries)
        .unwrap_or_default()
    {
        depots.insert(
            id.clone(),
            Depot {
                manifest: d.get_str("manifest").unwrap_or_default().to_string(),
                size: d.get_int("size").unwrap_or(0).max(0) as u64,
            },
        );
    }
    Some(SteamGame {
        app,
        name,
        acf: acf.to_path_buf(),
        dir: lib.join("steamapps/common").join(installdir),
        buildid: st.get_int("buildid").unwrap_or(0),
        beta: st
            .path(&["UserConfig", "BetaKey"])
            .and_then(Kv::str)
            .unwrap_or_default()
            .to_string(),
        auto_update: st.get_int("AutoUpdateBehavior").unwrap_or(0),
        state_flags: st.get_int("StateFlags").unwrap_or(0),
        depots,
    })
}

// ------------------------------------------------------------------ appinfo

#[derive(Clone, Debug, Default)]
pub struct Branch {
    pub buildid: i64,
    pub time_updated: i64,
    pub description: String,
}

#[derive(Clone, Debug, Default)]
pub struct AppInfo {
    pub branches: BTreeMap<String, Branch>,
    /// depot → branch → (gid, size).
    pub depots: BTreeMap<String, BTreeMap<String, (String, u64)>>,
}

/// What appinfo.vdf knows of this app now.
pub fn appinfo(root: &Path, app: &str) -> Option<AppInfo> {
    let id: u32 = app.parse().ok()?;
    let rec = crate::sources::steam::appinfo(root, &[id].into_iter().collect());
    let kv = rec.get(&id)?;
    let depots = kv.path(&["appinfo", "depots"])?;
    let mut info = AppInfo::default();
    for (name, b) in depots.get("branches").map(Kv::entries).unwrap_or_default() {
        info.branches.insert(
            name.clone(),
            Branch {
                buildid: b.get_int("buildid").unwrap_or(0),
                time_updated: b.get_int("timeupdated").unwrap_or(0),
                description: b.get_str("description").unwrap_or_default().to_string(),
            },
        );
    }
    for (did, d) in depots.entries() {
        if !did.bytes().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let mut per = BTreeMap::new();
        for (branch, m) in d.get("manifests").map(Kv::entries).unwrap_or_default() {
            // Newer appinfo: {gid, size, download}; older: the gid itself.
            let gid = m.get_str("gid").map(str::to_string).or_else(|| match m {
                Kv::Str(s) => Some(s.clone()),
                Kv::Int(i) => Some(i.to_string()),
                _ => None,
            });
            let gid = gid.or_else(|| m.get_int("gid").map(|g| (g as u64).to_string()));
            if let Some(g) = gid.filter(|g| !g.is_empty()) {
                per.insert(
                    branch.clone(),
                    (g, m.get_int("size").unwrap_or(0).max(0) as u64),
                );
            }
        }
        info.depots.insert(did.clone(), per);
    }
    Some(info)
}

// --------------------------------------------------------------------- logs

/// Older rotation first, so later lines win.
pub fn log_lines(root: &Path, stem: &str) -> Vec<String> {
    let mut out = Vec::new();
    for f in [format!("{stem}.previous.txt"), format!("{stem}.txt")] {
        if let Ok(b) = std::fs::read(root.join("logs").join(f)) {
            out.extend(String::from_utf8_lossy(&b).lines().map(str::to_string));
        }
    }
    out
}

/// "[2026-09-28 02:29:59] …" → local unix time.
pub fn log_time(line: &str) -> Option<i64> {
    let s = line.strip_prefix('[')?.get(..19)?;
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    local_to_unix(
        n(0, 4)?,
        n(5, 7)?,
        n(8, 10)?,
        n(11, 13)?,
        n(14, 16)?,
        n(17, 19)?,
    )
}

fn local_to_unix(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> Option<i64> {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = (y - 1900) as i32;
    tm.tm_mon = (mo - 1) as i32;
    tm.tm_mday = d as i32;
    tm.tm_hour = h as i32;
    tm.tm_min = mi as i32;
    tm.tm_sec = s as i32;
    tm.tm_isdst = -1;
    let t = unsafe { libc::mktime(&mut tm) };
    (t != -1).then_some(t as i64)
}

/// A build this client installed: (time, buildid, depot → manifest).
pub type Logged = (i64, i64, BTreeMap<String, String>);

/// "AppID X finished update, n mounted depots (BuildID B) : d1 (m1), …"
/// per app.
pub fn finished_updates(root: &Path) -> HashMap<String, Vec<Logged>> {
    let mut out: HashMap<String, Vec<_>> = HashMap::new();
    for line in log_lines(root, "content_log") {
        let Some(i) = line.find("] AppID ") else {
            continue;
        };
        let rest = &line[i + 8..];
        let Some((app, rest)) = rest.split_once(" finished update, ") else {
            continue;
        };
        let Some(b) = rest.find("(BuildID ") else {
            continue;
        };
        let after = &rest[b + 9..];
        let Some((bid, tail)) = after.split_once(") : ") else {
            continue;
        };
        let (Ok(bid), Some(t)) = (bid.parse::<i64>(), log_time(&line)) else {
            continue;
        };
        let mut depots = BTreeMap::new();
        // "d1 (m1),d2 (m2)," — commas, with or without spaces.
        for pair in tail.split(',') {
            if let Some((d, m)) = pair.trim().split_once(" (")
                && let Some(m) = m.strip_suffix(')')
                && d.bytes().all(|c| c.is_ascii_digit())
            {
                depots.insert(d.to_string(), m.to_string());
            }
        }
        out.entry(app.to_string())
            .or_default()
            .push((t, bid, depots));
    }
    out
}

/// `download_depot` results: (app, depot) → [(manifest, time)].
pub fn depot_downloads(root: &Path) -> HashMap<(String, String), Vec<(String, i64)>> {
    let mut out: HashMap<(String, String), Vec<(String, i64)>> = HashMap::new();
    for line in log_lines(root, "console_log") {
        let Some(i) = line.find("Depot download complete : \"") else {
            continue;
        };
        let rest = &line[i + 27..];
        let Some((path, tail)) = rest.split_once("\" (manifest ") else {
            continue;
        };
        let Some(m) = tail.split(')').next() else {
            continue;
        };
        let path = path.replace('\\', "/");
        let (Some(a), Some(d)) = (path.find("app_"), path.find("depot_")) else {
            continue;
        };
        let num = |s: &str| {
            s.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        };
        let (app, depot) = (num(&path[a + 4..]), num(&path[d + 6..]));
        if app.is_empty() || depot.is_empty() {
            continue;
        }
        out.entry((app, depot))
            .or_default()
            .push((m.to_string(), log_time(&line).unwrap_or(0)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_parsing() {
        let t = log_time("[2026-09-28 02:29:59] x").unwrap();
        assert!(t > 1_700_000_000);
    }

    #[test]
    fn finished_update_line() {
        let dir = std::env::temp_dir().join(format!("lenta-log-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::write(
            dir.join("logs/content_log.txt"),
            "[2026-09-15 00:02:59] AppID 553850 finished update, 4 mounted depots (BuildID 24826606) : 553851 (5937460960958490081),553853 (9020691380707780829),\n",
        )
        .unwrap();
        let u = finished_updates(&dir);
        let (_, bid, depots) = &u["553850"][0];
        assert_eq!(*bid, 24826606);
        assert_eq!(depots.len(), 2);
        assert_eq!(depots["553853"], "9020691380707780829");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real installs and logs read without trouble (read only).
    #[test]
    fn real_steam() {
        for root in crate::sources::steam::roots() {
            let ups = finished_updates(&root);
            let n: usize = ups.values().map(Vec::len).sum();
            eprintln!("finished updates: {n} in {} apps", ups.len());
            for (app, list) in ups.iter().take(3) {
                if let Some(g) = SteamGame::find(&root, app) {
                    let info = appinfo(&root, app);
                    eprintln!(
                        "{} build {} depots {:?} branches {:?} logged {}",
                        g.name,
                        g.buildid,
                        g.depots.keys().collect::<Vec<_>>(),
                        info.map(|i| i.branches.keys().cloned().collect::<Vec<_>>()),
                        list.len()
                    );
                }
            }
        }
    }
}
