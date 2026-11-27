//! Lenta's own Windows programs and games ("Own" in the library): Wine
//! prefixes made here, a runner for each (a Proton through umu-run, or the
//! system's Wine), installers run in them, and the program to start found
//! from the shortcuts and files the installer left — or picked by hand.
//!
//! Kept in `~/.local/share/lenta/programs.json`; prefixes made by default
//! under `~/Games/Lenta/Prefixes`.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::game::{Arts, Game, Source};

// ------------------------------------------------------------------ store

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefix {
    pub name: String,
    pub path: PathBuf,
    /// "wine" (the system's), or a Proton's folder; empty: umu's own
    /// (UMU-Proton, fetched by umu).
    pub runner: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Program {
    pub id: u64,
    pub title: String,
    pub exe: PathBuf,
    pub args: String,
    /// Environment for it: `NAME=value` words (WINEDLLOVERRIDES, DXVK_…).
    pub env: String,
    /// The prefix's folder.
    pub prefix: PathBuf,
    /// Unix seconds.
    pub last_played: u64,
    pub added: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub prefixes: Vec<Prefix>,
    pub programs: Vec<Program>,
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("LENTA_DATA")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("lenta")))
        .unwrap_or_else(|| "/tmp/lenta-data".into())
}

fn store_path() -> PathBuf {
    data_dir().join("programs.json")
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Store {
    pub fn load() -> Store {
        std::fs::read_to_string(store_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = store_path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = p.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
    }

    pub fn prefix(&self, path: &Path) -> Option<&Prefix> {
        self.prefixes.iter().find(|p| p.path == path)
    }

    pub fn program(&self, id: u64) -> Option<&Program> {
        self.programs.iter().find(|p| p.id == id)
    }

    pub fn add_program(&mut self, mut p: Program) -> u64 {
        let mut id = now() * 1000;
        while self.programs.iter().any(|q| q.id == id) {
            id += 1;
        }
        p.id = id;
        p.added = now();
        self.programs.push(p);
        id
    }
}

/// Change the store on disk (read, change, written back).
pub fn edit(f: impl FnOnce(&mut Store)) -> Result<Store, String> {
    let mut s = Store::load();
    f(&mut s);
    s.save()?;
    Ok(s)
}

/// Where new prefixes go.
pub fn prefix_root(cfg_root: &str) -> PathBuf {
    let r = cfg_root.trim();
    if r.is_empty() {
        dirs::home_dir()
            .unwrap_or_default()
            .join("Games/Lenta/Prefixes")
    } else if let Some(rest) = r.strip_prefix("~/") {
        dirs::home_dir().unwrap_or_default().join(rest)
    } else {
        PathBuf::from(r)
    }
}

/// A folder name from what was typed.
pub fn folder_name(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let t = t.trim().trim_matches('.').to_string();
    if t.is_empty() { "prefix".into() } else { t }
}

// ----------------------------------------------------------------- runners

#[derive(Clone, Debug, PartialEq)]
pub struct Runner {
    /// What a prefix stores.
    pub id: String,
    pub name: String,
}

/// What can run things: the Protons found (custom and system ones, then
/// Steam's own), umu's own, and the system's Wine last.
pub fn runners() -> Vec<Runner> {
    let mut out: Vec<Runner> = Vec::new();
    let mut seen = BTreeSet::new();
    for t in crate::sources::steamplay::tools() {
        if let Some(d) = t.dir.filter(|d| d.join("proton").is_file())
            && seen.insert(d.clone())
        {
            out.push(Runner {
                id: d.display().to_string(),
                name: t.display.clone(),
            });
        }
    }
    // Steam's own Protons, in its libraries.
    if let Some(root) = crate::sources::steam::roots().into_iter().next() {
        let mut own = Vec::new();
        for lib in crate::sources::steam::library_dirs(&root) {
            let Ok(rd) = std::fs::read_dir(lib.join("steamapps/common")) else {
                continue;
            };
            for e in rd.flatten() {
                let d = e.path();
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with("Proton") && d.join("proton").is_file() && seen.insert(d.clone()) {
                    own.push(Runner {
                        id: d.display().to_string(),
                        name: n,
                    });
                }
            }
        }
        own.sort_by(|a, b| b.name.cmp(&a.name));
        out.extend(own);
    }
    if crate::sandbox::has_program("umu-run") {
        out.push(Runner {
            id: String::new(),
            name: crate::i18n::tr("rn_umu").to_string(),
        });
    }
    if crate::sandbox::has_program("wine") {
        out.push(Runner {
            id: "wine".into(),
            name: crate::i18n::tr("rn_wine").to_string(),
        });
    }
    out
}

pub fn runner_name(id: &str) -> String {
    match id {
        "wine" => crate::i18n::tr("rn_wine").to_string(),
        "" => crate::i18n::tr("rn_umu").to_string(),
        d => Path::new(d)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| d.to_string()),
    }
}

/// The command to run `exe` (with `args`) in a prefix: `env` sets the
/// folder it runs in and Wine's variables, then umu-run (with the Proton)
/// or wine. `exe` may be a program wine knows ("winecfg") or "" (only
/// make the prefix ready).
pub fn command(
    prefix: &Path,
    runner: &str,
    exe: &str,
    args: &[String],
    gameid: &str,
) -> Vec<String> {
    let dir = Path::new(exe)
        .parent()
        .filter(|d| d.is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| prefix.to_path_buf());
    let mut v: Vec<String> = vec![
        "env".into(),
        "-C".into(),
        dir.display().to_string(),
        format!("WINEPREFIX={}", prefix.display()),
    ];
    if runner == "wine" {
        v.push("wine".into());
        if exe.is_empty() {
            v.push("wineboot".into());
            v.push("-u".into());
        } else {
            if exe.to_lowercase().ends_with(".msi") {
                v.push("msiexec".into());
                v.push("/i".into());
            }
            v.push(exe.into());
        }
    } else {
        v.push(format!("GAMEID={gameid}"));
        if !runner.is_empty() {
            v.push(format!("PROTONPATH={runner}"));
        }
        v.push("umu-run".into());
        if exe.to_lowercase().ends_with(".msi") {
            v.push("msiexec".into());
            v.push("/i".into());
        }
        v.push(exe.into());
    }
    v.extend(args.iter().cloned());
    v
}

/// The program's environment put into its command (after WINEPREFIX), DLL
/// mods beside it let load (`WINEDLLOVERRIDES`, joined to one given).
pub fn with_env(mut v: Vec<String>, env: &str, mods: Option<String>) -> Vec<String> {
    if v.len() < 4 || v[0] != "env" {
        return v;
    }
    let mut extra: Vec<String> = split_args(env)
        .into_iter()
        .filter(|kv| {
            kv.split_once('=')
                .is_some_and(|(k, _)| !k.is_empty() && !k.contains(' '))
        })
        .collect();
    if let Some(m) = mods {
        match extra
            .iter_mut()
            .find(|kv| kv.starts_with("WINEDLLOVERRIDES="))
        {
            Some(kv) => {
                kv.push(';');
                kv.push_str(&m);
            }
            None => extra.push(format!("WINEDLLOVERRIDES={m}")),
        }
    }
    v.splice(4..4, extra);
    v
}

/// The same command with Wine drawing through Wayland itself (on a
/// Wayland session): an installer's empty main window (Inno Setup's,
/// white, behind its dialogs under X11) isn't shown then. Proton's builds
/// that have the driver (GE, CachyOS) take PROTON_ENABLE_WAYLAND; the
/// system's Wine picks it without DISPLAY. Kept for installers and tools:
/// games still go through X11 (the driver can't yet hold the pointer).
pub fn on_wayland(mut v: Vec<String>, runner: &str, session: bool) -> Vec<String> {
    if !session || v.len() < 3 || v[0] != "env" {
        return v;
    }
    let extra: Vec<String> = if runner == "wine" {
        vec!["-u".into(), "DISPLAY".into()]
    } else {
        vec!["PROTON_ENABLE_WAYLAND=1".into()]
    };
    v.splice(3..3, extra);
    v
}

/// Does this Proton have Wine's Wayland driver (UMU-Proton, like
/// Valve's, doesn't)?
pub fn proton_wayland(dir: &str) -> bool {
    Path::new(dir)
        .join("files/lib/wine/x86_64-unix/winewayland.so")
        .is_file()
}

/// Is this a Wayland session?
pub fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// winetricks with these verbs, in the prefix.
pub fn tricks(prefix: &Path, runner: &str, verbs: &[String]) -> Vec<String> {
    let mut v: Vec<String> = vec![
        "env".into(),
        "-C".into(),
        prefix.display().to_string(),
        format!("WINEPREFIX={}", prefix.display()),
    ];
    if runner == "wine" {
        v.push("winetricks".into());
    } else if let Some(bin) = runner.strip_prefix("wine:") {
        // Another Wine (Heroic's, Lutris's): winetricks told which.
        v.push(format!("WINE={bin}"));
        let server = Path::new(bin).with_file_name("wineserver");
        if server.is_file() {
            v.push(format!("WINESERVER={}", server.display()));
        }
        v.push("winetricks".into());
    } else {
        v.push("GAMEID=umu-default".into());
        if !runner.is_empty() {
            v.push(format!("PROTONPATH={runner}"));
        }
        v.push("umu-run".into());
        v.push("winetricks".into());
    }
    v.push("-q".into());
    v.extend(verbs.iter().cloned());
    v
}

/// The sets of libraries offered at one press: (i18n key, verbs).
pub const TRICKS: [(&str, &[&str]); 6] = [
    ("tk_base", &["vcrun2022", "d3dcompiler_47", "d3dx9", "xact"]),
    (
        "tk_vc_old",
        &[
            "vcrun2005",
            "vcrun2008",
            "vcrun2010",
            "vcrun2012",
            "vcrun2013",
        ],
    ),
    ("tk_dx", &["d3dx10", "d3dx11_43", "xinput", "xact_x64"]),
    ("tk_physx", &["physx"]),
    ("tk_dotnet", &["dotnet48"]),
    ("tk_fonts", &["corefonts"]),
];

/// Where libraries go: a prefix with what runs it, or a Steam game's
/// (through protontricks, which finds its Proton and prefix itself).
#[derive(Clone, Debug, PartialEq)]
pub enum Tricks {
    Prefix {
        name: String,
        path: PathBuf,
        runner: String,
    },
    Steam {
        name: String,
        appid: u32,
        prefix: PathBuf,
    },
}

impl Tricks {
    pub fn name(&self) -> &str {
        match self {
            Tricks::Prefix { name, .. } | Tricks::Steam { name, .. } => name,
        }
    }

    pub fn prefix(&self) -> &Path {
        match self {
            Tricks::Prefix { path, .. } => path,
            Tricks::Steam { prefix, .. } => prefix,
        }
    }

    /// What runs these verbs.
    pub fn argv(&self, verbs: &[String]) -> Vec<String> {
        match self {
            Tricks::Prefix { path, runner, .. } => tricks(path, runner, verbs),
            Tricks::Steam { appid, .. } => {
                let mut v = vec!["protontricks".to_string(), appid.to_string(), "-q".into()];
                v.extend(verbs.iter().cloned());
                v
            }
        }
    }

    /// The program it needs.
    pub fn tool(&self) -> &'static str {
        match self {
            Tricks::Steam { .. } => "protontricks",
            Tricks::Prefix { runner, .. } if runner == "wine" || runner.starts_with("wine:") => {
                "winetricks"
            }
            Tricks::Prefix { .. } => "umu-run",
        }
    }
}

/// What winetricks has put in a prefix (its log; Proton's prefixes keep
/// it under pfx/).
pub fn installed_verbs(prefix: &Path) -> BTreeSet<String> {
    [
        prefix.join("winetricks.log"),
        prefix.join("pfx/winetricks.log"),
    ]
    .iter()
    .filter_map(|p| std::fs::read_to_string(p).ok())
    .flat_map(|s| {
        s.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('-'))
            .collect::<Vec<_>>()
    })
    .collect()
}

/// Where a game's libraries go, by its launcher: a Steam game through
/// protontricks; Heroic's with the Proton or Wine set for it there;
/// Lutris's with the system Wine; Hydra's with umu's Proton; own ones
/// with their prefix's. None without a prefix (a Steam game never run).
pub fn tricks_for(g: &Game) -> Option<Tricks> {
    let name = g.title.clone();
    match g.source {
        Source::Steam => {
            let appid = g.compat_id?;
            let prefix = crate::maint::prefix(g)?;
            Some(Tricks::Steam {
                name,
                appid,
                prefix,
            })
        }
        Source::Own => {
            let s = Store::load();
            let p = s.program(id_of(&g.key)?)?;
            let runner = s
                .prefix(&p.prefix)
                .map(|x| x.runner.clone())
                .unwrap_or_default();
            Some(Tricks::Prefix {
                name,
                path: p.prefix.clone(),
                runner,
            })
        }
        Source::Heroic => {
            let path = g.prefix.clone().filter(|p| p.is_dir())?;
            let app = g.key.rsplit(':').next()?;
            Some(Tricks::Prefix {
                name,
                path,
                runner: heroic_runner(app).unwrap_or_else(|| "wine".into()),
            })
        }
        Source::Lutris => Some(Tricks::Prefix {
            name,
            path: g.prefix.clone().filter(|p| p.is_dir())?,
            runner: "wine".into(),
        }),
        Source::Hydra => Some(Tricks::Prefix {
            name,
            path: g.prefix.clone().filter(|p| p.is_dir())?,
            runner: String::new(),
        }),
    }
}

/// The Wine or Proton Heroic runs a game with (its GamesConfig).
fn heroic_runner(app: &str) -> Option<String> {
    for (root, _) in crate::sources::heroic::roots() {
        let Ok(t) = std::fs::read_to_string(root.join(format!("GamesConfig/{app}.json"))) else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_str(&t).ok()?;
        let w = v.get(app)?.get("wineVersion")?;
        let bin = w.get("bin")?.as_str()?;
        return Some(match w.get("type").and_then(|t| t.as_str()) {
            Some("proton") => Path::new(bin).parent()?.display().to_string(),
            _ => format!("wine:{bin}"),
        });
    }
    None
}

/// Split typed arguments on spaces, keeping "quoted parts" together.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    let mut any = false;
    for c in s.chars().filter(|c| *c != '\0') {
        match c {
            '"' => {
                q = !q;
                any = true;
            }
            c if c.is_whitespace() && !q => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

/// Run it and wait (on the host under Flatpak); the job's way of running
/// installers and winetricks.
pub fn run_wait(argv: &[String]) -> Result<(), String> {
    if std::env::var_os("LENTA_DRY_RUN").is_some() {
        eprintln!("lenta: dry run, not running {argv:?}");
        return Ok(());
    }
    let (prog, args) = argv.split_first().ok_or("nothing to run")?;
    let mut c = Command::new(prog);
    c.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let st = crate::sandbox::host(c)
        .status()
        .map_err(|e| format!("{prog}: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("{prog}: {st}"))
    }
}

/// Make a prefix (or bring it up to date).
/// (umu-run "" makes it and then says it found nothing to run: what
/// counts is the prefix there afterwards.)
pub fn make_prefix(p: &Prefix) -> Result<(), String> {
    std::fs::create_dir_all(&p.path).map_err(|e| e.to_string())?;
    let r = run_wait(&command(&p.path, &p.runner, "", &[], "umu-default"));
    if p.path.join("drive_c/windows").is_dir() || std::env::var_os("LENTA_DRY_RUN").is_some() {
        Ok(())
    } else {
        r.and(Err(crate::i18n::tr("pg_prefix_failed").into()))
    }
}

// --------------------------------------------------------------- the games

pub fn game_key(id: u64) -> String {
    format!("own:{id}")
}

pub fn id_of(key: &str) -> Option<u64> {
    key.strip_prefix("own:")?.parse().ok()
}

/// The umu id that brings a game's fixes: its Steam id when known.
pub fn gameid(appid: Option<u32>) -> String {
    appid.map_or("umu-default".into(), |a| format!("umu-{a}"))
}

pub fn games() -> Vec<Game> {
    let s = Store::load();
    s.programs
        .iter()
        .map(|p| {
            let runner = s
                .prefix(&p.prefix)
                .map(|x| x.runner.clone())
                .unwrap_or_default();
            let installed = p.exe.is_file();
            Game {
                dl: None,
                key: game_key(p.id),
                title: p.title.clone(),
                source: Source::Own,
                sub: Some(runner_name(&runner)),
                art: Arts::default(),
                playtime_min: None,
                last_played: (p.last_played > 0).then_some(p.last_played),
                steam_appid: None,
                store_appid: None,
                emu: None,
                prefix: Some(p.prefix.clone()),
                installed,
                compat_id: None,
                install_dir: p.exe.parent().map(Path::to_path_buf),
                launch: vec![with_env(
                    command(
                        &p.prefix,
                        &runner,
                        &p.exe.display().to_string(),
                        &split_args(&p.args),
                        "umu-default",
                    ),
                    &p.env,
                    crate::dllmods::overrides(&crate::dllmods::find(&crate::dllmods::dirs_of(
                        None,
                        Some(&p.exe),
                    ))),
                )],
            }
        })
        .collect()
}

// ------------------------------------------------- what an installer left

/// Windows path → this machine's: C: is the prefix's drive_c, Z: the root,
/// others through the prefix's dosdevices.
pub fn unix_path(prefix: &Path, win: &str) -> Option<PathBuf> {
    let w = win.trim().trim_matches('"');
    let (drive, rest) = w.split_once(':')?;
    if drive.len() != 1 {
        return None;
    }
    let rest = rest.trim_start_matches(['\\', '/']).replace('\\', "/");
    let d = drive.to_ascii_lowercase();
    let base = match d.as_str() {
        "c" => prefix.join("drive_c"),
        "z" => PathBuf::from("/"),
        _ => std::fs::read_link(prefix.join(format!("dosdevices/{d}:"))).ok()?,
    };
    let base = if base.is_relative() {
        prefix.join("dosdevices").join(base)
    } else {
        base
    };
    Some(base.join(rest))
}

/// A Windows shortcut's target and arguments ([MS-SHLLINK]).
pub fn read_lnk(b: &[u8]) -> Option<(String, String)> {
    let u32_at = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize);
    let u16_at = |o: usize| Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?) as usize);
    if u32_at(0)? != 0x4C {
        return None;
    }
    let flags = u32_at(0x14)?;
    let unicode = flags & 0x80 != 0;
    let mut o = 0x4C;
    if flags & 1 != 0 {
        o += 2 + u16_at(o)?;
    }
    let mut target = String::new();
    if flags & 2 != 0 {
        let li = o;
        let size = u32_at(li)?;
        let hsize = u32_at(li + 4)?;
        let cstr = |at: usize| -> Option<String> {
            let s = b.get(at..)?;
            let end = s.iter().position(|&c| c == 0)?;
            Some(s[..end].iter().map(|&c| c as char).collect())
        };
        let wstr = |at: usize| -> Option<String> {
            let mut v = Vec::new();
            let mut i = at;
            loop {
                let c = u16_at(i)? as u16;
                if c == 0 {
                    break;
                }
                v.push(c);
                i += 2;
            }
            Some(String::from_utf16_lossy(&v))
        };
        let base = if hsize >= 0x24 && u32_at(li + 0x1C)? != 0 {
            wstr(li + u32_at(li + 0x1C)?)
        } else if u32_at(li + 0x10)? != 0 {
            cstr(li + u32_at(li + 0x10)?)
        } else {
            None
        };
        let suffix = if hsize >= 0x24 && u32_at(li + 0x20)? != 0 {
            wstr(li + u32_at(li + 0x20)?)
        } else if u32_at(li + 0x18)? != 0 {
            cstr(li + u32_at(li + 0x18)?)
        } else {
            None
        };
        target = format!("{}{}", base.unwrap_or_default(), suffix.unwrap_or_default());
        o = li + size;
    }
    // StringData: name, relative path, working dir, arguments, icon.
    let mut strings: [Option<String>; 5] = Default::default();
    for (i, bit) in [4u32, 8, 16, 32, 64].iter().enumerate() {
        if flags as u32 & bit != 0 {
            let n = u16_at(o)?;
            o += 2;
            let s = if unicode {
                let v: Vec<u16> = (0..n)
                    .map(|k| u16_at(o + k * 2).map(|c| c as u16))
                    .collect::<Option<_>>()?;
                o += n * 2;
                String::from_utf16_lossy(&v)
            } else {
                let s: String = b.get(o..o + n)?.iter().map(|&c| c as char).collect();
                o += n;
                s
            };
            // Some writers count the closing NUL in: not part of it.
            strings[i] = Some(
                s.trim_matches(|c: char| c == '\0' || c.is_whitespace())
                    .to_string(),
            );
        }
    }
    if target.is_empty() {
        // Only a relative path: kept as it is (resolved by the caller).
        target = strings[1].clone()?;
    }
    Some((target, strings[3].clone().unwrap_or_default()))
}

/// What a prefix holds: its programs (outside `windows`) and shortcuts.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub exes: BTreeSet<PathBuf>,
    pub lnks: BTreeSet<PathBuf>,
}

fn walk(dir: &Path, depth: usize, out: &mut Snapshot) {
    if depth > 9 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let p = e.path();
        if t.is_dir() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if depth == 0 && n == "windows" {
                continue;
            }
            walk(&p, depth + 1, out);
        } else if t.is_file() {
            let n = e.file_name().to_string_lossy().to_lowercase();
            if n.ends_with(".exe") {
                out.exes.insert(p);
            } else if n.ends_with(".lnk") {
                out.lnks.insert(p);
            }
        }
    }
}

pub fn snapshot(prefix: &Path) -> Snapshot {
    let mut s = Snapshot::default();
    walk(&prefix.join("drive_c"), 0, &mut s);
    s
}

/// Not the program itself: uninstallers, setups, redistributables…
fn helper(name: &str) -> bool {
    let n = name.to_lowercase();
    [
        "unins",
        "uninst",
        "setup",
        "install",
        "redist",
        "vcredist",
        "vc_redist",
        "dxsetup",
        "dxwebsetup",
        "directx",
        "dotnet",
        "crash",
        "report",
        "helper",
        "update",
        "patch",
        "physx",
        "oalinst",
        "ue4prereq",
        "prereq",
        "easyanticheat",
        "battleye",
        "cefprocess",
        "unity crash",
        "notification",
    ]
    .iter()
    .any(|w| n.contains(w))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub title: String,
    pub exe: PathBuf,
    pub args: String,
}

fn title_of(exe: &Path) -> String {
    crate::sources::hydra::game_dir(exe)
        .and_then(|d| d.file_name().map(|n| n.to_string_lossy().to_string()))
        .filter(|n| {
            !["drive_c", "Program Files", "Program Files (x86)", "Games"].contains(&n.as_str())
        })
        .unwrap_or_else(|| {
            exe.file_stem()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        })
}

/// The programs an install added, the likeliest first: what its new
/// shortcuts point at, else its new programs (not helpers; shallower and
/// bigger first).
pub fn found(prefix: &Path, before: &Snapshot, after: &Snapshot) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    for l in after.lnks.difference(&before.lnks) {
        let Ok(b) = std::fs::read(l) else { continue };
        let Some((target, args)) = read_lnk(&b) else {
            continue;
        };
        let Some(exe) = unix_path(prefix, &target) else {
            continue;
        };
        let stem = l
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // GOG's "Launch <game>".
        let name = stem
            .strip_prefix("Launch ")
            .map(str::to_string)
            .unwrap_or(stem);
        if !exe.is_file()
            || !exe.to_string_lossy().to_lowercase().ends_with(".exe")
            || helper(&name)
            || helper(&exe.file_name().unwrap_or_default().to_string_lossy())
            || out.iter().any(|f| f.exe == exe)
        {
            continue;
        }
        out.push(Found {
            title: name,
            exe,
            args,
        });
    }
    if !out.is_empty() {
        return out;
    }
    let new: Vec<&PathBuf> = after
        .exes
        .difference(&before.exes)
        .filter(|e| !helper(&e.file_name().unwrap_or_default().to_string_lossy()))
        .collect();
    // The top of what was installed: the shortest common folder.
    let depth = |p: &Path| p.components().count();
    let mut scored: Vec<(usize, std::cmp::Reverse<u64>, &PathBuf)> = new
        .iter()
        .map(|e| {
            let size = std::fs::metadata(e).map_or(0, |m| m.len());
            (depth(e), std::cmp::Reverse(size), *e)
        })
        .collect();
    scored.sort();
    for (_, _, e) in scored.into_iter().take(12) {
        out.push(Found {
            title: title_of(e),
            exe: e.clone(),
            args: String::new(),
        });
    }
    out
}

/// One file: an installer to run, or the program itself? (by its name)
pub fn looks_like_installer(p: &Path) -> bool {
    let n = p
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    n.ends_with(".msi")
        || ["setup", "install", "installer", "_inst"]
            .iter()
            .any(|w| n.contains(w))
}

/// A name for a program from its file (for a portable one).
pub fn title_for(p: &Path) -> String {
    title_of(p)
}

/// Prefixes made elsewhere, by programs' counts: for showing what's used.
pub fn counts(s: &Store) -> HashMap<PathBuf, usize> {
    let mut m = HashMap::new();
    for p in &s.programs {
        *m.entry(p.prefix.clone()).or_insert(0) += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shortcut as Wine writes one: link info with a local path, then
    /// unicode string data (relative path, working dir, arguments).
    fn lnk(target: &str, args: &str) -> Vec<u8> {
        let mut b = vec![0u8; 0x4C];
        b[0] = 0x4C;
        let flags: u32 = 2 | 8 | 16 | 32 | 0x80;
        b[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
        // LinkInfo: header 0x1C, local base path right after it.
        let path = target.as_bytes();
        let size = 0x1C + path.len() + 1 + 1;
        let mut li = Vec::new();
        li.extend((size as u32).to_le_bytes());
        li.extend(0x1Cu32.to_le_bytes());
        li.extend(1u32.to_le_bytes());
        li.extend(0u32.to_le_bytes()); // volume id
        li.extend(0x1Cu32.to_le_bytes()); // local base path
        li.extend(0u32.to_le_bytes());
        li.extend(((0x1C + path.len() + 1) as u32).to_le_bytes()); // suffix
        li.extend(path);
        li.push(0);
        li.push(0);
        b.extend(li);
        for s in ["..\\x.exe", "C:\\Games", args] {
            let w: Vec<u16> = s.encode_utf16().collect();
            b.extend((w.len() as u16).to_le_bytes());
            for c in w {
                b.extend(c.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn shortcuts_read() {
        let (_, a) = read_lnk(&lnk("C:\\Games\\Fox\\Fox.exe", "\0")).unwrap();
        assert_eq!(a, "", "GOG's shortcuts count a NUL in");
        let (t, a) = read_lnk(&lnk("C:\\Games\\Fox\\Fox.exe", "-windowed")).unwrap();
        assert_eq!(t, "C:\\Games\\Fox\\Fox.exe");
        assert_eq!(a, "-windowed");
        assert!(read_lnk(b"nope").is_none());
    }

    #[test]
    fn windows_paths() {
        let p = Path::new("/pfx");
        assert_eq!(
            unix_path(p, "C:\\Program Files\\Fox\\fox.exe"),
            Some(PathBuf::from("/pfx/drive_c/Program Files/Fox/fox.exe"))
        );
        assert_eq!(
            unix_path(p, "Z:\\home\\a\\b.exe"),
            Some(PathBuf::from("/home/a/b.exe"))
        );
        assert_eq!(unix_path(p, "relative.exe"), None);
    }

    #[test]
    fn what_an_install_left() {
        let t = std::env::temp_dir().join(format!("lenta-progs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let c = t.join("drive_c");
        let old = c.join("Program Files/Old/old.exe");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, b"MZ").unwrap();
        let before = snapshot(&t);
        // The install: a game, its uninstaller, a redistributable, a
        // helper deep inside.
        let game = c.join("Games/Fox Game/Fox.exe");
        std::fs::create_dir_all(game.parent().unwrap()).unwrap();
        std::fs::write(&game, vec![0u8; 5000]).unwrap();
        std::fs::write(c.join("Games/Fox Game/unins000.exe"), b"MZ").unwrap();
        std::fs::create_dir_all(c.join("Games/Fox Game/_redist")).unwrap();
        std::fs::write(c.join("Games/Fox Game/_redist/vc_redist.x64.exe"), b"MZ").unwrap();
        std::fs::create_dir_all(c.join("Games/Fox Game/bin/tools")).unwrap();
        std::fs::write(c.join("Games/Fox Game/bin/tools/edit.exe"), b"MZ").unwrap();
        std::fs::create_dir_all(c.join("windows/system32")).unwrap();
        std::fs::write(c.join("windows/system32/new.exe"), b"MZ").unwrap();
        let after = snapshot(&t);
        let f = found(&t, &before, &after);
        assert_eq!(f[0].exe, game);
        assert_eq!(f[0].title, "Fox Game");
        assert_eq!(f.len(), 2, "{f:?}");
        // With a shortcut, that's the one.
        let menu = c.join("ProgramData/Microsoft/Windows/Start Menu/Programs");
        std::fs::create_dir_all(&menu).unwrap();
        std::fs::write(
            menu.join("Fox Editor.lnk"),
            lnk("C:\\Games\\Fox Game\\bin\\tools\\edit.exe", ""),
        )
        .unwrap();
        std::fs::write(
            menu.join("Uninstall Fox.lnk"),
            lnk("C:\\Games\\Fox Game\\unins000.exe", ""),
        )
        .unwrap();
        let after = snapshot(&t);
        let f = found(&t, &before, &after);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].title, "Fox Editor");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn tricks_targets() {
        let s = Tricks::Steam {
            name: "x".into(),
            appid: 570,
            prefix: "/c/570".into(),
        };
        assert_eq!(
            s.argv(&["vcrun2022".into()]),
            ["protontricks", "570", "-q", "vcrun2022"]
        );
        assert_eq!(s.tool(), "protontricks");
        let h = Tricks::Prefix {
            name: "h".into(),
            path: "/p".into(),
            runner: "wine:/opt/w/bin/wine".into(),
        };
        let v = h.argv(&["d3dx9".into()]);
        assert!(v.contains(&"WINE=/opt/w/bin/wine".to_string()));
        assert_eq!(&v[v.len() - 3..], ["winetricks", "-q", "d3dx9"]);
        assert_eq!(h.tool(), "winetricks");
        let t = std::env::temp_dir().join(format!("lenta-tlog-{}", std::process::id()));
        std::fs::create_dir_all(t.join("pfx")).unwrap();
        std::fs::write(t.join("pfx/winetricks.log"), "vcrun2022\nd3dx9\n").unwrap();
        let have = installed_verbs(&t);
        assert!(have.contains("vcrun2022") && have.contains("d3dx9") && !have.contains("xact"));
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn commands() {
        let c = command(
            Path::new("/p"),
            "/t/GE",
            "/g/x.exe",
            &["-a".into()],
            "umu-570",
        );
        assert_eq!(
            c,
            [
                "env",
                "-C",
                "/p",
                "WINEPREFIX=/p",
                "GAMEID=umu-570",
                "PROTONPATH=/t/GE",
                "umu-run",
                "/g/x.exe",
                "-a"
            ]
        );
        let w = command(Path::new("/p"), "wine", "/p/setup.msi", &[], "");
        assert_eq!(&w[4..], ["wine", "msiexec", "/i", "/p/setup.msi"]);
        assert_eq!(split_args("-a \"b c\" d"), ["-a", "b c", "d"]);
        let p = on_wayland(c.clone(), "/t/GE", true);
        assert_eq!(
            &p[..5],
            [
                "env",
                "-C",
                "/p",
                "PROTON_ENABLE_WAYLAND=1",
                "WINEPREFIX=/p"
            ]
        );
        let w2 = on_wayland(w.clone(), "wine", true);
        assert_eq!(&w2[3..6], ["-u", "DISPLAY", "WINEPREFIX=/p"]);
        assert_eq!(on_wayland(w.clone(), "wine", false), w);
        assert!(split_args("\0").is_empty());
        let e = with_env(
            c.clone(),
            "DXVK_HUD=fps WINEDLLOVERRIDES=\"winmm=n,b\" junk",
            Some("dwmapi=n,b".into()),
        );
        assert_eq!(
            &e[3..7],
            [
                "WINEPREFIX=/p",
                "DXVK_HUD=fps",
                "WINEDLLOVERRIDES=winmm=n,b;dwmapi=n,b",
                "GAMEID=umu-570"
            ]
        );
        assert!(looks_like_installer(Path::new("/d/setup_fox_1.2.exe")));
        assert!(!looks_like_installer(Path::new("/d/Fox.exe")));
    }
}

#[cfg(test)]
mod real {
    use super::*;

    /// A real prefix through umu and a Proton here, an "installer" (cmd
    /// copying a program into C:\Games), and what it left found. Slow;
    /// needs umu-run and GE-Proton: `cargo test real_install -- --ignored`.
    #[test]
    #[ignore]
    fn real_install() {
        let proton = runners()
            .into_iter()
            .find(|r| r.name.contains("GE-Proton"))
            .expect("a GE-Proton");
        let t = std::env::temp_dir().join(format!("lenta-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let p = Prefix {
            name: "t".into(),
            path: t.clone(),
            runner: proton.id.clone(),
        };
        make_prefix(&p).unwrap();
        assert!(t.join("drive_c/windows/system32").is_dir());
        let before = snapshot(&t);
        let args: Vec<String> = [
            "/c",
            "mkdir C:\\Games\\Fox && copy C:\\windows\\system32\\notepad.exe C:\\Games\\Fox\\Fox.exe",
        ]
        .map(String::from)
        .to_vec();
        run_wait(&command(&t, &p.runner, "cmd", &args, "umu-default")).unwrap();
        let after = snapshot(&t);
        let f = found(&t, &before, &after);
        eprintln!("{f:?}");
        assert_eq!(
            f.first().map(|f| f.exe.clone()),
            Some(t.join("drive_c/Games/Fox/Fox.exe"))
        );
        assert_eq!(f[0].title, "Fox");
        let _ = std::fs::remove_dir_all(&t);
    }
}
