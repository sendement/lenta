//! Download one depot version into the staging folder: sign in to the CM,
//! get the depot key and the manifests, plan from what's on disk, fetch
//! the rest from the CDN on a few threads.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use super::assemble::{Assembler, Stats};
use super::cdn::Cdn;
use super::cm::{Cm, CmError};
use super::manifest::{File, Manifest};
use super::state::staging_dir;

#[derive(Clone, Debug)]
pub struct Job {
    pub app: String,
    pub depot: String,
    pub manifest: String,
    pub branch: String,
    /// The install, to take unchanged files and chunks from.
    pub base_dir: Option<PathBuf>,
    /// The manifest installed there now.
    pub base_manifest: Option<String>,
    pub steam_root: PathBuf,
}

impl Job {
    pub fn out_dir(&self) -> PathBuf {
        staging_dir()
            .join(&self.app)
            .join(format!("{}_{}", self.depot, self.manifest))
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Status(&'static str),
    /// (stats, bytes a second while downloading).
    Progress(Stats, u64),
}

#[derive(Debug)]
pub enum Error {
    /// No usable sign-in: sign in again (as the Steam client).
    Auth,
    Cancelled,
    Other(String),
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Other(s)
    }
}

pub enum Login<'a> {
    Account(&'a crate::account::Account),
    #[allow(dead_code)]
    Anonymous,
}

fn base_from_depotcache(root: &Path, depot: &str, gid: &str) -> Option<Vec<File>> {
    let p = root.join(format!("depotcache/{depot}_{gid}.manifest"));
    let m = Manifest::read(&p, true).ok()?;
    (!m.encrypted).then_some(m.files)
}

pub fn run(
    job: &Job,
    login: Login,
    cancel: &AtomicBool,
    emit: &(dyn Fn(Event) + Sync),
) -> Result<Stats, Error> {
    let app: u32 = job.app.parse().map_err(|_| "bad app id".to_string())?;
    let depot: u32 = job.depot.parse().map_err(|_| "bad depot id".to_string())?;
    let gid: u64 = job
        .manifest
        .parse()
        .map_err(|_| "bad manifest id".to_string())?;
    emit(Event::Status("dl_signing_in"));
    let cm = match login {
        Login::Account(a) => {
            if !a.is_client() {
                return Err(Error::Auth);
            }
            Cm::logon(&a.name, a.refresh_token())
        }
        Login::Anonymous => Cm::anonymous(),
    };
    let cm = match cm {
        Ok(c) => c,
        Err(CmError::Auth(_)) => return Err(Error::Auth),
        Err(CmError::Other(e)) => return Err(Error::Other(e)),
    };
    let cell = cm.cell_id;
    let cm = Mutex::new(cm);
    let key = cm.lock().unwrap().depot_key(app, depot)?;
    let cdn = Cdn::new(cell)?;
    emit(Event::Status("dl_manifest"));
    let target = cdn.manifest(&cm, app, depot, gid, &key, &job.branch)?;
    let base = match (&job.base_dir, &job.base_manifest) {
        (Some(_), Some(b)) if *b != job.manifest => {
            base_from_depotcache(&job.steam_root, &job.depot, b).or_else(|| {
                emit(Event::Status("dl_base_manifest"));
                let g: u64 = b.parse().ok()?;
                cdn.manifest(&cm, app, depot, g, &key, &job.branch)
                    .ok()
                    .map(|m| m.files)
            })
        }
        (Some(_), Some(_)) => Some(target.files.clone()),
        _ => None,
    };
    let base_dir = base.as_ref().and(job.base_dir.as_deref());
    let out = job.out_dir();
    let mut asm = Assembler::new(
        target.files.clone(),
        &out,
        base.unwrap_or_default(),
        base_dir,
    );
    emit(Event::Status(if base_dir.is_some() {
        "dl_checking"
    } else {
        "dl_preparing"
    }));
    let mut last = Instant::now();
    let todo = asm.plan(&mut |s: &Stats| {
        if last.elapsed().as_millis() > 300 {
            last = Instant::now();
            emit(Event::Progress(*s, 0));
        }
    })?;
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    emit(Event::Status("dl_downloading"));
    let stats = Mutex::new(asm.stats);
    let queue = Mutex::new(todo.into_iter());
    let failed: Mutex<Option<String>> = Mutex::new(None);
    let started = Instant::now();
    let fetched = AtomicU64::new(0);
    let last = Mutex::new(Instant::now());
    let files: &[File] = &asm.files;
    let out_dir = asm.out().to_path_buf();
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) || failed.lock().unwrap().is_some() {
                        return;
                    }
                    let Some((i, c)) = queue.lock().unwrap().next() else {
                        return;
                    };
                    let f = &files[i];
                    let mut err = String::new();
                    let mut ok = false;
                    for attempt in 0..5 {
                        match cdn
                            .chunk(Some(&cm), app, depot, &c.sha, &key)
                            .and_then(|d| Assembler::store(&out_dir, f, &c, &d))
                        {
                            Ok(()) => {
                                ok = true;
                                break;
                            }
                            Err(e) => {
                                err = e;
                                std::thread::sleep(std::time::Duration::from_secs(1 + attempt));
                            }
                        }
                    }
                    if !ok {
                        *failed.lock().unwrap() = Some(format!("{}: {err}", f.name));
                        return;
                    }
                    let n = fetched.fetch_add(c.size as u64, Ordering::Relaxed) + c.size as u64;
                    let snapshot = {
                        let mut st = stats.lock().unwrap();
                        st.downloaded += c.size as u64;
                        *st
                    };
                    let mut l = last.lock().unwrap();
                    if l.elapsed().as_millis() > 300 {
                        *l = Instant::now();
                        let speed = n * 1000 / (started.elapsed().as_millis().max(1) as u64);
                        emit(Event::Progress(snapshot, speed));
                    }
                }
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    if let Some(e) = failed.into_inner().unwrap() {
        return Err(Error::Other(e));
    }
    asm.stats = stats.into_inner().unwrap();
    asm.finish(
        &job.manifest,
        job.base_manifest.as_deref().unwrap_or_default(),
    )?;
    if let Ok(cm) = cm.into_inner() {
        cm.close();
    }
    Ok(asm.stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole small public depot, anonymously (13.7 MB from Steam):
    /// `cargo test anonymous_depot -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn anonymous_depot() {
        let tmp = std::env::temp_dir().join(format!("lenta-dl-{}", std::process::id()));
        unsafe { std::env::set_var("LENTA_SD_DIR", &tmp) };
        let root = crate::sources::steam::roots().into_iter().next().unwrap();
        let job = Job {
            app: "228980".into(),
            depot: "228984".into(),
            manifest: "2547553897526095397".into(),
            branch: "public".into(),
            base_dir: None,
            base_manifest: None,
            steam_root: root,
        };
        let cancel = AtomicBool::new(false);
        let stats =
            run(&job, Login::Anonymous, &cancel, &|e| eprintln!("{e:?}")).expect("download");
        eprintln!("{stats:?}");
        assert_eq!(stats.downloaded, stats.total);
        let out = job.out_dir();
        assert!(out.join(super::super::assemble::COMPLETE).is_file());
        // Every file hashes to what the manifest says.
        let (_, _, files) = super::super::assemble::read_files_list(&out).unwrap();
        assert!(!files.is_empty());
        // Byte for byte what Steam installed from the same manifest.
        let installed = job.steam_root.join("steamapps/common/Steamworks Shared");
        let mut same = 0;
        for (name, _, _) in &files {
            let a = std::fs::read(out.join(name)).unwrap();
            if let Ok(b) = std::fs::read(installed.join(name)) {
                assert!(a == b, "{name} differs from the install");
                same += 1;
            }
        }
        eprintln!("{same}/{} files equal to the install", files.len());
        assert!(same > 0);
        // From the install as base: nothing to fetch.
        let _ = std::fs::remove_dir_all(&tmp);
        let delta = Job {
            base_dir: Some(installed),
            base_manifest: Some(job.manifest.clone()),
            ..job.clone()
        };
        let stats = run(&delta, Login::Anonymous, &cancel, &|_| {}).expect("delta");
        eprintln!("delta: {stats:?}");
        assert_eq!(stats.downloaded, 0);
        assert_eq!(stats.unchanged, stats.total);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
