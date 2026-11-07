//! Which games are running and what Steam is doing with them (downloading,
//! updating), looked at every second and a half on a thread of its own.
//!
//! Running: a Steam game has Steam's `reaper SteamLaunch AppId=<id>` in
//! its process tree; another game, a process started from its folder (its
//! command line or working folder in it). Steam's work on a game is in its
//! appmanifest: StateFlags and the bytes done.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What Steam is doing with a game.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Work {
    /// Needs an update, not started (queued, or waiting to be played).
    Queued,
    /// Downloading or putting it in place: done of total bytes.
    Updating {
        done: u64,
        total: u64,
    },
    Paused {
        done: u64,
        total: u64,
    },
}

impl Work {
    pub fn frac(self) -> Option<f32> {
        match self {
            Work::Updating { done, total } | Work::Paused { done, total } if total > 0 => {
                Some((done as f32 / total as f32).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

#[derive(Default, Clone, PartialEq)]
pub struct Status {
    /// Game keys running now.
    pub running: HashSet<String>,
    /// Steam's work, by game key.
    pub work: HashMap<String, Work>,
}

/// What the watcher needs to know of a game.
#[derive(Clone)]
pub struct Watch {
    pub key: String,
    pub appid: Option<u32>,
    pub dir: Option<PathBuf>,
}

pub type Shared = Arc<Mutex<Status>>;

/// Look every 1.5 s at the games in `games` (kept fresh by the caller);
/// `changed` is called when anything changes.
pub fn spawn(games: Arc<Mutex<Vec<Watch>>>, changed: impl Fn() + Send + 'static) -> Shared {
    let shared: Shared = Default::default();
    let out = shared.clone();
    std::thread::Builder::new()
        .name("lenta-running".into())
        .spawn(move || {
            loop {
                let list = games.lock().map(|g| g.clone()).unwrap_or_default();
                let now = look(&list);
                let differs = shared.lock().map(|s| *s != now).unwrap_or(false);
                if differs {
                    if let Ok(mut s) = shared.lock() {
                        *s = now;
                    }
                    changed();
                }
                std::thread::sleep(Duration::from_millis(1500));
            }
        })
        .expect("spawn running watcher");
    out
}

/// Every process: its id, parent, name, command line and working folder.
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub comm: String,
    pub cmd: Vec<String>,
    /// The command line in one (inside Flatpak only this is exact).
    pub line: String,
    pub cwd: Option<PathBuf>,
}

/// The host's processes (inside Flatpak, asked of the host: the sandbox
/// sees only its own).
pub fn procs() -> Vec<Proc> {
    if crate::sandbox::flatpak() {
        return host_procs();
    }
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| {
            let pid: i32 = e.file_name().to_str()?.parse().ok()?;
            let raw = std::fs::read(e.path().join("cmdline")).ok()?;
            let cmd: Vec<String> = raw
                .split(|b| *b == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect();
            let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
            // "pid (comm) state ppid …": the comm may hold spaces.
            let (head, tail) = stat.rsplit_once(')')?;
            let comm = head.split_once('(')?.1.to_string();
            let ppid = tail.split_whitespace().nth(1)?.parse().ok()?;
            Some(Proc {
                pid,
                ppid,
                comm,
                line: cmd.join(" "),
                cwd: std::fs::read_link(e.path().join("cwd")).ok(),
                cmd,
            })
        })
        .collect()
}

/// ps and the working folders, through flatpak-spawn.
fn host_procs() -> Vec<Proc> {
    const SH: &str = "ps -eo pid=,ppid=,args=; echo @@; ps -eo pid=,comm=; echo @@; \
        find /proc -mindepth 2 -maxdepth 2 -name cwd -printf '%h\t%l\n' 2>/dev/null";
    let mut c = std::process::Command::new("sh");
    c.args(["-c", SH]);
    let Ok(out) = crate::sandbox::host(c)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    parse_ps(&String::from_utf8_lossy(&out.stdout))
}

fn parse_ps(text: &str) -> Vec<Proc> {
    let mut parts = text.split("@@\n");
    let (args, comms, cwds) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    let mut comm: HashMap<i32, String> = HashMap::new();
    for l in comms.lines() {
        let l = l.trim_start();
        if let Some((p, c)) = l.split_once(' ')
            && let Ok(p) = p.parse()
        {
            comm.insert(p, c.trim().to_string());
        }
    }
    let mut cwd: HashMap<i32, PathBuf> = HashMap::new();
    for l in cwds.lines() {
        if let Some((d, t)) = l.split_once('\t')
            && let Some(Ok(p)) = d.strip_prefix("/proc/").map(str::parse)
        {
            cwd.insert(p, t.into());
        }
    }
    args.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let pid: i32 = w.next()?.parse().ok()?;
            let ppid: i32 = w.next()?.parse().ok()?;
            let cmd: Vec<String> = w.map(String::from).collect();
            // What's after the two numbers, as it is.
            let mut rest = l.trim_start();
            for _ in 0..2 {
                rest = rest
                    .trim_start_matches(|c: char| !c.is_whitespace())
                    .trim_start();
            }
            let line = rest.to_string();
            Some(Proc {
                pid,
                ppid,
                comm: comm.remove(&pid).unwrap_or_default(),
                cwd: cwd.remove(&pid),
                cmd,
                line,
            })
        })
        .collect()
}

/// Steam's launch of this app: its reaper's command line.
fn is_steam_launch(cmd: &[String], appid: u32) -> bool {
    let tag = format!("AppId={appid}");
    cmd.iter().any(|a| a == "SteamLaunch") && cmd.contains(&tag)
}

/// Started from this folder.
fn from_dir(p: &Proc, dir: &std::path::Path) -> bool {
    let d = dir.to_string_lossy();
    if d.len() < 8 {
        return false;
    }
    p.cwd.as_deref().is_some_and(|c| c.starts_with(dir))
        || p.line.starts_with(d.as_ref())
        || p.cmd.iter().take(3).any(|a| a.starts_with(d.as_ref()))
}

pub fn look(games: &[Watch]) -> Status {
    let ps = procs();
    let mut st = Status::default();
    for g in games {
        let up = match g.appid {
            Some(id) if g.key.starts_with("steam:") => {
                ps.iter().any(|p| is_steam_launch(&p.cmd, id))
            }
            _ => g
                .dir
                .as_deref()
                .is_some_and(|d| ps.iter().any(|p| from_dir(p, d))),
        };
        if up {
            st.running.insert(g.key.clone());
        }
        if let (Some(id), Some(dir)) = (g.appid, g.dir.as_deref())
            && g.key.starts_with("steam:")
            && let Some(w) = manifest(dir, id).as_deref().and_then(work)
        {
            st.work.insert(g.key.clone(), w);
        }
    }
    st
}

/// The appmanifest beside a game's folder (steamapps/common/<dir>).
fn manifest(dir: &std::path::Path, appid: u32) -> Option<String> {
    let apps = dir.parent()?.parent()?;
    std::fs::read_to_string(apps.join(format!("appmanifest_{appid}.acf"))).ok()
}

fn work(text: &str) -> Option<Work> {
    let kv = crate::vdf::parse_text(text);
    let a = kv.get("AppState")?;
    let flags = a.get_int("StateFlags")? as u64;
    let n = |k: &str| a.get_int(k).unwrap_or(0).max(0) as u64;
    let (dl, dlt) = (n("BytesDownloaded"), n("BytesToDownload"));
    let (sg, sgt) = (n("BytesStaged"), n("BytesToStage"));
    // Downloading counts most; staging is the rest of the way.
    let (done, total) = if dlt > 0 && dl < dlt {
        (dl, dlt)
    } else {
        (sg, sgt)
    };
    const UPDATE_REQUIRED: u64 = 2;
    const UPDATE_RUNNING: u64 = 256;
    const UPDATE_PAUSED: u64 = 512;
    const UPDATE_STARTED: u64 = 1024;
    const BUSY: u64 = 1 << 17 | 1 << 18 | 1 << 19 | 1 << 20 | 1 << 21 | 1 << 22;
    if flags & UPDATE_PAUSED != 0 {
        return Some(Work::Paused { done, total });
    }
    if flags & (UPDATE_RUNNING | UPDATE_STARTED | BUSY) != 0 {
        return Some(Work::Updating { done, total });
    }
    if flags & UPDATE_REQUIRED != 0 {
        return Some(Work::Queued);
    }
    None
}

/// Close a game: its processes, children first; then, after a while,
/// whatever is left for good. Returns how many were asked to close.
pub fn stop(g: &Watch) -> usize {
    if std::env::var_os("LENTA_DRY_RUN").is_some() {
        return 1;
    }
    let ps = procs();
    let roots: Vec<i32> = ps
        .iter()
        .filter(|p| match g.appid {
            Some(id) if g.key.starts_with("steam:") => is_steam_launch(&p.cmd, id),
            _ => g.dir.as_deref().is_some_and(|d| from_dir(p, d)),
        })
        .map(|p| p.pid)
        .collect();
    // Everything under them (the reaper adopts what's left behind).
    let mut all: Vec<i32> = roots.clone();
    let mut i = 0;
    while i < all.len() {
        let parent = all[i];
        for p in &ps {
            if p.ppid == parent && !all.contains(&p.pid) {
                all.push(p.pid);
            }
        }
        i += 1;
    }
    let me = std::process::id() as i32;
    all.retain(|p| *p != me && *p > 1);
    signal(&all, libc::SIGTERM);
    let left = all.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(6));
        let alive: HashSet<i32> = procs().iter().map(|p| p.pid).collect();
        let left: Vec<i32> = left.into_iter().filter(|p| alive.contains(p)).collect();
        signal(&left, libc::SIGKILL);
    });
    all.len()
}

/// Children first (the list goes parents first).
fn signal(pids: &[i32], sig: i32) {
    if pids.is_empty() {
        return;
    }
    if crate::sandbox::flatpak() {
        let mut c = std::process::Command::new("kill");
        c.arg(format!("-{sig}"))
            .args(pids.iter().rev().map(|p| p.to_string()));
        let _ = crate::sandbox::host(c).status();
        return;
    }
    for p in pids.iter().rev() {
        unsafe {
            libc::kill(*p, sig);
        }
    }
}

/// Is a process with this name running? (Inside Flatpak asked of the host,
/// at most once a second.)
pub fn named(name: &str) -> bool {
    if crate::sandbox::flatpak() {
        type Seen = Option<(std::time::Instant, Vec<String>)>;
        static LAST: Mutex<Seen> = Mutex::new(None);
        let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if g.as_ref()
            .is_none_or(|(t, _)| t.elapsed() > Duration::from_secs(1))
        {
            *g = Some((
                std::time::Instant::now(),
                procs().into_iter().map(|p| p.comm).collect(),
            ));
        }
        return g.as_ref().is_some_and(|(_, v)| v.iter().any(|c| c == name));
    }
    procs().iter().any(|p| p.comm == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_states() {
        let m = |flags: u64, extra: &str| {
            work(&format!(
                "\"AppState\"\n{{\n\"appid\" \"1\"\n\"StateFlags\" \"{flags}\"\n{extra}\n}}\n"
            ))
        };
        assert_eq!(m(4, ""), None);
        assert_eq!(m(6, ""), Some(Work::Queued));
        let w = m(
            1026,
            "\"BytesToDownload\" \"200\"\n\"BytesDownloaded\" \"50\"",
        )
        .unwrap();
        assert_eq!(
            w,
            Work::Updating {
                done: 50,
                total: 200
            }
        );
        assert_eq!(w.frac(), Some(0.25));
        assert!(matches!(m(4 | 2 | 512, ""), Some(Work::Paused { .. })));
    }

    #[test]
    fn steam_launch_found() {
        let c = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert!(is_steam_launch(
            &c("/x/reaper SteamLaunch AppId=1245620 -- /x/proton waitforexitandrun g.exe"),
            1245620
        ));
        assert!(!is_steam_launch(
            &c("/x/reaper SteamLaunch AppId=12456 --"),
            1245620
        ));
    }

    #[test]
    fn host_ps_read() {
        let t = "    1     0 /sbin/init splash\n  420     1 /x/reaper SteamLaunch AppId=7 -- /g/My Game/a.exe\n@@\n    1 systemd\n  420 reaper\n@@\n/proc/420\t/g/My Game\n";
        let ps = parse_ps(t);
        assert_eq!(ps.len(), 2);
        let r = &ps[1];
        assert_eq!((r.pid, r.ppid, r.comm.as_str()), (420, 1, "reaper"));
        assert!(is_steam_launch(&r.cmd, 7));
        assert_eq!(r.line, "/x/reaper SteamLaunch AppId=7 -- /g/My Game/a.exe");
        assert_eq!(r.cwd.as_deref(), Some(std::path::Path::new("/g/My Game")));
        assert!(from_dir(r, std::path::Path::new("/g/My Game")));
    }

    /// This test's own process counts as "started from" its folder.
    #[test]
    fn found_by_folder() {
        let dir = std::env::current_dir().unwrap();
        let st = look(&[Watch {
            key: "lutris:1".into(),
            appid: None,
            dir: Some(dir),
        }]);
        assert!(st.running.contains("lutris:1"));
    }
}
