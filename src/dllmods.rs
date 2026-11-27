//! Mods that come as a DLL put beside the game (UE4SS's dwmapi, ReShade's
//! dxgi, OptiScaler, Special K, ASI Loader, dgVoodoo…). Windows loads a
//! DLL from the game's folder first; Wine takes its own built-in one for
//! these names unless told otherwise, so the mod never runs. Told:
//!
//! - for own programs, `WINEDLLOVERRIDES` at their start;
//! - for any other launcher's game, the prefix's registry, for that game's
//!   programs only (`Software\Wine\AppDefaults\<exe>\DllOverrides`), so it
//!   holds whoever starts the game and nothing else in the prefix changes.

use std::path::{Path, PathBuf};

/// DLLs Wine has built in that mods stand in for.
const PROXIES: [&str; 22] = [
    "dwmapi",
    "dxgi",
    "d3d8",
    "d3d9",
    "d3d10",
    "d3d10_1",
    "d3d11",
    "d3d12",
    "ddraw",
    "dinput8",
    "dinput",
    "winmm",
    "version",
    "xinput1_3",
    "xinput1_4",
    "xinput9_1_0",
    "winhttp",
    "wininet",
    "dbghelp",
    "dsound",
    "opengl32",
    "msacm32",
];

/// A folder with such DLLs: which, what they look like, its programs.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub dir: PathBuf,
    pub dlls: Vec<String>,
    pub kinds: Vec<&'static str>,
    pub exes: Vec<String>,
}

fn names(dir: &Path) -> Vec<(String, bool)> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| {
                    (
                        e.file_name().to_string_lossy().to_string(),
                        e.path().is_dir(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What mod it is, by the files beside it.
fn kinds(entries: &[(String, bool)]) -> Vec<&'static str> {
    let has =
        |f: &dyn Fn(&str, bool) -> bool| entries.iter().any(|(n, d)| f(&n.to_lowercase(), *d));
    let mut out = Vec::new();
    if has(&|n, d| n == "ue4ss.dll" || n == "ue4ss-settings.ini" || (d && n == "ue4ss")) {
        out.push("UE4SS");
    }
    if has(&|n, d| n == "reshade.ini" || (d && n == "reshade-shaders") || n.starts_with("reshade"))
    {
        out.push("ReShade");
    }
    if has(&|n, _| n.starts_with("optiscaler")) {
        out.push("OptiScaler");
    }
    if has(&|n, _| n.starts_with("specialk")) {
        out.push("Special K");
    }
    if has(&|n, _| n.ends_with(".asi")) {
        out.push("ASI Loader");
    }
    if has(&|n, _| n.starts_with("dgvoodoo")) {
        out.push("dgVoodoo");
    }
    if has(&|n, d| n == "doorstop_config.ini" || (d && n == "bepinex")) {
        out.push("BepInEx");
    }
    if has(&|n, d| d && n == "melonloader") {
        out.push("MelonLoader");
    }
    out
}

/// One folder's mod DLLs (none: None).
pub fn in_dir(dir: &Path) -> Option<Found> {
    let entries = names(dir);
    let mut dlls: Vec<String> = entries
        .iter()
        .filter(|(_, d)| !d)
        .filter_map(|(n, _)| {
            let low = n.to_lowercase();
            let stem = low.strip_suffix(".dll")?;
            PROXIES.contains(&stem).then(|| stem.to_string())
        })
        .collect();
    if dlls.is_empty() {
        return None;
    }
    dlls.sort();
    dlls.dedup();
    let mut exes: Vec<String> = entries
        .iter()
        .filter(|(n, d)| !d && n.to_lowercase().ends_with(".exe"))
        .map(|(n, _)| n.clone())
        .collect();
    exes.sort();
    Some(Found {
        dir: dir.to_path_buf(),
        dlls,
        kinds: kinds(&entries),
        exes,
    })
}

/// Where a game's mods would be: its program's folder, its install folder,
/// and an Unreal game's `<Project>/Binaries/Win64` (where the real program
/// is, beside a small launcher at the top).
pub fn dirs_of(install: Option<&Path>, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };
    if let Some(d) = exe.and_then(Path::parent) {
        add(d.to_path_buf());
    }
    let roots: Vec<PathBuf> = install
        .into_iter()
        .map(Path::to_path_buf)
        .chain(exe.and_then(Path::parent).map(Path::to_path_buf))
        .collect();
    for r in roots {
        add(r.clone());
        for (n, d) in names(&r) {
            if d && n != "Engine" {
                for bin in ["Binaries/Win64", "Binaries/WinGDK", "Binaries/Win32"] {
                    add(r.join(&n).join(bin));
                }
            }
        }
    }
    out
}

/// Are there any (looked at once in a while: menus ask every frame).
pub fn any(dirs: Vec<PathBuf>) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    type Seen = HashMap<Vec<PathBuf>, (Instant, bool)>;
    static SEEN: Mutex<Option<Seen>> = Mutex::new(None);
    let mut g = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if let Some((t, v)) = m.get(&dirs)
        && t.elapsed() < Duration::from_secs(5)
    {
        return *v;
    }
    let v = !find(&dirs).is_empty();
    m.insert(dirs, (Instant::now(), v));
    v
}

pub fn find(dirs: &[PathBuf]) -> Vec<Found> {
    dirs.iter().filter_map(|d| in_dir(d)).collect()
}

/// `dwmapi,dxgi=n,b` for these, or nothing.
pub fn overrides(found: &[Found]) -> Option<String> {
    let mut all: Vec<&str> = found
        .iter()
        .flat_map(|f| f.dlls.iter().map(String::as_str))
        .collect();
    all.sort();
    all.dedup();
    (!all.is_empty()).then(|| format!("{}=n,b", all.join(",")))
}

// ------------------------------------------------------------- registry

/// The prefix's user registry (Proton's prefixes keep it under pfx/).
pub fn user_reg(prefix: &Path) -> Option<PathBuf> {
    [prefix.join("pfx/user.reg"), prefix.join("user.reg")]
        .into_iter()
        .find(|p| p.is_file())
}

fn section(exe: &str) -> String {
    format!("[Software\\\\Wine\\\\AppDefaults\\\\{exe}\\\\DllOverrides]")
}

fn same_header(line: &str, head: &str) -> bool {
    line.split(']')
        .next()
        .is_some_and(|l| format!("{l}]").eq_ignore_ascii_case(head))
}

/// Is every DLL told to load from the game for every one of its programs?
pub fn allowed(reg: &str, f: &Found) -> bool {
    f.exes.iter().all(|exe| {
        let head = section(exe);
        let mut lines = reg.lines().skip_while(|l| !same_header(l, &head));
        if lines.next().is_none() {
            return false;
        }
        let body: Vec<&str> = lines.take_while(|l| !l.starts_with('[')).collect();
        f.dlls.iter().all(|d| {
            body.iter()
                .any(|l| l.to_lowercase().starts_with(&format!("\"{d}\"=\"native")))
        })
    })
}

/// Set (or take away) native-first loading of the DLLs for the folder's
/// programs, in the registry's text.
pub fn edit(reg: &str, f: &Found, allow: bool, now: u64) -> String {
    let mut lines: Vec<String> = reg.lines().map(str::to_string).collect();
    for exe in &f.exes {
        let head = section(exe);
        let start = lines.iter().position(|l| same_header(l, &head));
        match start {
            Some(s) => {
                let end = lines[s + 1..]
                    .iter()
                    .position(|l| l.starts_with('['))
                    .map_or(lines.len(), |e| s + 1 + e);
                // Drop the old values for these DLLs, then add.
                let mut body: Vec<String> = lines[s + 1..end]
                    .iter()
                    .filter(|l| {
                        let low = l.to_lowercase();
                        !f.dlls.iter().any(|d| low.starts_with(&format!("\"{d}\"=")))
                    })
                    .cloned()
                    .collect();
                // Keep the blank line that ends the section last.
                let blank = body.last().is_some_and(|l| l.trim().is_empty());
                if blank {
                    body.pop();
                }
                if allow {
                    body.extend(f.dlls.iter().map(|d| format!("\"{d}\"=\"native,builtin\"")));
                }
                if blank {
                    body.push(String::new());
                }
                lines.splice(s + 1..end, body);
            }
            None if allow => {
                if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!("{head} {now}"));
                lines.extend(f.dlls.iter().map(|d| format!("\"{d}\"=\"native,builtin\"")));
                lines.push(String::new());
            }
            None => {}
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Is Wine running in this prefix (its registry would be written over)?
pub fn prefix_busy(prefix: &Path) -> bool {
    let want: Vec<String> = [prefix.to_path_buf(), prefix.join("pfx")]
        .iter()
        .map(|p| format!("WINEPREFIX={}", p.display()))
        .collect();
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return false;
    };
    rd.flatten().any(|e| {
        let p = e.path();
        std::fs::read_to_string(p.join("comm")).is_ok_and(|c| c.trim() == "wineserver")
            && std::fs::read(p.join("environ")).is_ok_and(|env| {
                env.split(|b| *b == 0).any(|kv| {
                    let s = String::from_utf8_lossy(kv);
                    want.iter()
                        .any(|w| s.trim_end_matches('/') == w.trim_end_matches('/'))
                })
            })
    })
}

/// Write it to the prefix (a copy of the old one kept beside it once).
pub fn apply(prefix: &Path, found: &[Found], allow: bool) -> Result<(), String> {
    if std::env::var_os("LENTA_DRY_RUN").is_some() {
        return Ok(());
    }
    let reg = user_reg(prefix).ok_or("no user.reg")?;
    if prefix_busy(prefix) {
        return Err(crate::i18n::tr("dm_busy").into());
    }
    let text = std::fs::read_to_string(&reg).map_err(|e| e.to_string())?;
    let backup = reg.with_extension("reg.lenta-backup");
    if !backup.exists() {
        let _ = std::fs::write(&backup, &text);
    }
    let now = crate::progs::now();
    let mut out = text;
    for f in found {
        out = edit(&out, f, allow, now);
    }
    let tmp = reg.with_extension("reg.lenta-tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &reg).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(n: &str) -> PathBuf {
        let t = std::env::temp_dir().join(format!("lenta-dll-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        t
    }

    #[test]
    fn mods_found() {
        let t = tmp("find");
        // An Unreal game: a launcher on top, the program and UE4SS deeper.
        let win64 = t.join("Fox/Binaries/Win64");
        std::fs::create_dir_all(win64.join("ue4ss")).unwrap();
        for f in ["Fox-Win64-Shipping.exe", "dwmapi.dll", "UE4SS.dll"] {
            std::fs::write(win64.join(f), b"x").unwrap();
        }
        std::fs::write(t.join("Fox.exe"), b"x").unwrap();
        // ReShade beside the launcher too.
        std::fs::write(t.join("dxgi.dll"), b"x").unwrap();
        std::fs::write(t.join("ReShade.ini"), b"x").unwrap();
        std::fs::write(t.join("steam_api64.dll"), b"x").unwrap();
        let found = find(&dirs_of(Some(&t), Some(&t.join("Fox.exe"))));
        assert_eq!(found.len(), 2, "{found:?}");
        let top = found.iter().find(|f| f.dir == t).unwrap();
        assert_eq!(
            (top.dlls.clone(), top.kinds.clone()),
            (vec!["dxgi".to_string()], vec!["ReShade"])
        );
        let ue = found.iter().find(|f| f.dir == win64).unwrap();
        assert_eq!(ue.dlls, ["dwmapi"]);
        assert_eq!(ue.kinds, ["UE4SS"]);
        assert_eq!(ue.exes, ["Fox-Win64-Shipping.exe"]);
        assert_eq!(overrides(&found).as_deref(), Some("dwmapi,dxgi=n,b"));
        let _ = std::fs::remove_dir_all(t);
    }

    #[test]
    fn registry_edited() {
        let reg = "WINE REGISTRY Version 2\n;; All keys relative to \\\\User\n\n[Software\\\\Wine] 1\n\"Version\"=\"win10\"\n\n";
        let f = Found {
            dir: "/g".into(),
            dlls: vec!["dwmapi".into(), "dxgi".into()],
            kinds: vec![],
            exes: vec!["Fox.exe".into()],
        };
        assert!(!allowed(reg, &f));
        let on = edit(reg, &f, true, 7);
        assert!(on.contains("[Software\\\\Wine\\\\AppDefaults\\\\Fox.exe\\\\DllOverrides] 7\n\"dwmapi\"=\"native,builtin\"\n\"dxgi\"=\"native,builtin\"\n"));
        assert!(allowed(&on, &f));
        assert!(on.contains("\"Version\"=\"win10\""), "the rest kept");
        // Again: no doubles.
        let twice = edit(&on, &f, true, 8);
        assert_eq!(twice.matches("\"dwmapi\"").count(), 1);
        let off = edit(&twice, &f, false, 9);
        assert!(!allowed(&off, &f));
        assert!(!off.contains("\"dwmapi\""));
        assert!(off.contains("\"Version\"=\"win10\""));
    }

    /// What this machine's games have beside them (read only):
    /// `cargo test real_games -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_games() {
        let cfg = crate::config::Config::load();
        for g in crate::sources::scan(&cfg) {
            let exe = crate::progs::id_of(&g.key).and_then(|id| {
                crate::progs::Store::load()
                    .program(id)
                    .map(|p| p.exe.clone())
            });
            let f = find(&dirs_of(g.install_dir.as_deref(), exe.as_deref()));
            if !f.is_empty() {
                eprintln!("{} [{:?}]", g.title, g.source);
                for x in f {
                    eprintln!("   {:?} {:?} {:?} {:?}", x.kinds, x.dlls, x.exes, x.dir);
                }
            }
        }
    }

    /// Written as Wine reads it: a real prefix (GE-Proton through umu), the
    /// overrides put in, then asked of Wine's own `reg query`.
    #[test]
    #[ignore]
    fn real_registry() {
        use crate::progs;
        let proton = progs::runners()
            .into_iter()
            .find(|r| r.name.contains("GE-Proton"))
            .expect("a GE-Proton");
        let t = std::env::temp_dir().join(format!("lenta-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let p = progs::Prefix {
            name: "r".into(),
            path: t.clone(),
            runner: proton.id.clone(),
        };
        progs::make_prefix(&p).unwrap();
        // Wine's server stays a moment after the prefix is made.
        for _ in 0..40 {
            if !prefix_busy(&t) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let f = Found {
            dir: t.clone(),
            dlls: vec!["dwmapi".into(), "dxgi".into()],
            kinds: vec![],
            exes: vec!["Fox-Win64-Shipping.exe".into()],
        };
        apply(&t, std::slice::from_ref(&f), true).unwrap();
        let mut c = std::process::Command::new("env");
        c.arg("-C").arg(&t);
        for a in progs::command(
            &t,
            &p.runner,
            "reg",
            &[
                "query".into(),
                "HKCU\\Software\\Wine\\AppDefaults\\Fox-Win64-Shipping.exe\\DllOverrides".into(),
            ],
            "umu-default",
        )
        .into_iter()
        .skip(3)
        {
            c.arg(a);
        }
        let out = c.output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        eprintln!("{text}");
        assert!(
            text.contains("dwmapi") && text.contains("native,builtin"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&t);
    }
}
