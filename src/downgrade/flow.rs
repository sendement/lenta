//! The whole job, for the Versions tab and the command line: read a game's
//! versions; install one (download what's missing, close Steam, put the
//! files in, lock); lift the lock.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::assemble::Stats;
use super::download::{self, Event, Job, Login};
use super::game::{self, SteamGame};
use super::history::{History, Version, actual_depots};
use super::news::{self, Note};
use super::ops::{self, Staged};
use super::state::{Lock, State};
use crate::i18n::tr;
use crate::sources::steamplay::steam_running;

pub fn steam_root() -> Option<PathBuf> {
    crate::sources::steam::roots().into_iter().next()
}

/// A game's versions and what's on disk, read on a thread.
#[derive(Clone)]
pub struct Loaded {
    pub game: SteamGame,
    pub versions: Vec<Version>,
    pub actual: BTreeMap<String, String>,
    /// Downloaded depots and whether they are complete.
    pub staged: Vec<(Staged, bool)>,
    pub lock: Option<Lock>,
}

pub fn load(app: &str) -> Option<Loaded> {
    let root = steam_root()?;
    let game = SteamGame::find(&root, app)?;
    let info = game::appinfo(&root, app);
    let mut state = State::load();
    // Patch notes give builds their version numbers; kept 12 hours.
    let notes: Vec<Note> = match state.news(app, 12 * 3600) {
        Some(n) => n
            .into_iter()
            .map(|(time, version, title)| Note {
                time,
                version,
                title,
            })
            .collect(),
        None => match news::fetch(app, &game.name) {
            Ok(n) => {
                state.set_news(app, &n);
                if std::env::var_os("LENTA_DRY_RUN").is_none() {
                    let _ = state.save();
                }
                n.into_iter()
                    .map(|(time, version, title, _)| Note {
                        time,
                        version,
                        title,
                    })
                    .collect()
            }
            Err(_) => Vec::new(),
        },
    };
    let hist = History::new(&root, &state);
    let actual = actual_depots(&state, &game);
    let cands = hist.candidates(&game, info.as_ref(), &actual);
    let versions = hist.versions(&game, info.as_ref(), &cands, &notes);
    let staged = ops::staged(&root, &hist, app)
        .into_iter()
        .map(|s| {
            let ok = ops::staged_ok(&root, &s);
            (s, ok)
        })
        .collect();
    Some(Loaded {
        lock: state.lock(app),
        game,
        versions,
        actual,
        staged,
    })
}

/// Depots that differ from what's installed: depot → manifest.
pub fn changes(v: &Version, actual: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    v.depots
        .iter()
        .filter_map(|(d, m)| {
            let m = m.as_ref()?;
            (actual.get(d) != Some(m)).then(|| (d.clone(), m.clone()))
        })
        .collect()
}

impl Loaded {
    fn staged_for(&self, d: &str, m: &str) -> Option<&Staged> {
        self.staged
            .iter()
            .find(|(s, ok)| *ok && s.depot == d && s.manifest == m)
            .map(|(s, _)| s)
    }

    /// Everything the version needs is downloaded.
    pub fn ready(&self, target: &BTreeMap<String, String>) -> bool {
        !target.is_empty() && target.iter().all(|(d, m)| self.staged_for(d, m).is_some())
    }

    /// The depots still to download.
    pub fn jobs(&self, target: &BTreeMap<String, String>) -> Vec<Job> {
        let root = steam_root().unwrap_or_default();
        target
            .iter()
            .filter(|(d, m)| self.staged_for(d, m).is_none())
            .map(|(d, m)| Job {
                app: self.game.app.clone(),
                depot: d.clone(),
                manifest: m.clone(),
                branch: self.game.branch().to_string(),
                base_dir: Some(self.game.dir.clone()),
                base_manifest: self.actual.get(d).cloned(),
                steam_root: root.clone(),
            })
            .collect()
    }
}

/// What an install is doing, for a progress line.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    /// "dl_…" while downloading, "sd_installing" after.
    pub status: &'static str,
    pub depot: usize,
    pub depots: usize,
    pub stats: Stats,
    pub speed: u64,
}

/// Errors that mean "sign in again".
pub const NEED_LOGIN: &str = "auth";

/// Download the depots (in turn), reporting into `report`.
pub fn download_all(
    jobs: &[Job],
    account: &crate::account::Account,
    cancel: &AtomicBool,
    report: &(dyn Fn(Progress) + Sync),
) -> Result<(), String> {
    for (i, job) in jobs.iter().enumerate() {
        let base = Progress {
            status: "dl_signing_in",
            depot: i,
            depots: jobs.len(),
            ..Default::default()
        };
        report(base.clone());
        let last = std::sync::Mutex::new(base);
        let emit = |e: Event| {
            if let Ok(mut p) = last.lock() {
                match e {
                    Event::Status(s) => p.status = s,
                    Event::Progress(s, sp) => {
                        p.stats = s;
                        p.speed = sp;
                    }
                }
                report(p.clone());
            }
        };
        match download::run(job, Login::Account(account), cancel, &emit) {
            Ok(_) => {}
            Err(download::Error::Auth) => return Err(NEED_LOGIN.into()),
            Err(download::Error::Cancelled) => return Err(tr("sd_cancelled").into()),
            Err(download::Error::Other(e)) => return Err(e),
        }
    }
    Ok(())
}

/// Close Steam if it runs; true if it was running.
pub fn close_steam() -> Result<bool, String> {
    if !steam_running() {
        return Ok(false);
    }
    crate::game::run_first(&crate::sources::steam::steam_run(&["-shutdown"]))?;
    let t = Instant::now();
    while steam_running() {
        if t.elapsed() > Duration::from_secs(60) {
            return Err(tr("sd_steam_wont_close").into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    // Let it finish writing its files.
    std::thread::sleep(Duration::from_secs(2));
    Ok(true)
}

pub fn start_steam() {
    let _ = crate::game::run_first(&crate::sources::steam::steam_run(&["-silent"]));
}

/// Put a downloaded version in and lock the game. Steam is closed for
/// it and started again if it was running.
pub fn install(
    app: &str,
    target: &BTreeMap<String, String>,
    buildid: Option<i64>,
) -> Result<String, String> {
    let root = steam_root().ok_or("Steam not found")?;
    let loaded = load(app).ok_or("game not found")?;
    let stageds: Vec<Staged> = target
        .iter()
        .map(|(d, m)| {
            loaded
                .staged_for(d, m)
                .cloned()
                .ok_or_else(|| format!("depot {d} not downloaded"))
        })
        .collect::<Result<_, _>>()?;
    let was_running = close_steam()?;
    let result = (|| {
        let (mut written, mut removed) = (0, 0);
        for st in &stageds {
            let state = State::load();
            let g = SteamGame::find(&root, app).ok_or("game gone")?;
            let done = ops::apply_depot(&root, &state, &g, st, true, &|_, _| {})?;
            written += done.moved;
            removed += done.deleted;
            // Lock after every depot: if a later one fails, the state still
            // knows what's really on disk.
            let mut state = State::load();
            let g = SteamGame::find(&root, app).ok_or("game gone")?;
            let one: BTreeMap<String, String> = [(st.depot.clone(), st.manifest.clone())].into();
            ops::lock(&root, &mut state, &g, &one, None, false)?;
        }
        let mut state = State::load();
        let g = SteamGame::find(&root, app).ok_or("game gone")?;
        ops::lock(&root, &mut state, &g, target, buildid, false)?;
        Ok::<_, String>(format!(
            "{} ({written} {}, {removed} {})",
            tr("sd_rolled_back"),
            tr("sd_files"),
            tr("sd_removed")
        ))
    })();
    if was_running {
        start_steam();
    }
    result
}

/// Lift the lock; `validate` has Steam check the files (back to latest).
pub fn unlock(app: &str, validate: bool) -> Result<String, String> {
    let root = steam_root().ok_or("Steam not found")?;
    let was_running = close_steam()?;
    let result = (|| {
        let mut state = State::load();
        let g = SteamGame::find(&root, app).ok_or("game gone")?;
        ops::unlock(&mut state, &g)
    })();
    if result.is_ok() && validate {
        crate::game::run_first(&crate::sources::steam::steam_run(&[&format!(
            "steam://validate/{app}"
        )]))?;
    } else if was_running {
        start_steam();
    }
    result.map(|_| tr("sd_unlocked").to_string())
}
