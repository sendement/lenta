//! Work that outlasts a question (lenta waits 20 s for an answer):
//! unpacking and putting in a mod, downloading from Nexus. Each job is a
//! file in `jobs/`; the program runs again on its own (`lenta-mods work
//! <id>`) to do it, and the pages read how far it got.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::layout::{Choices, Layout, Plan};
use crate::store::{self, Game, Nexus, Rec, data, now};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub id: String,
    /// "install" or "download".
    pub kind: String,
    pub game: String,
    pub root: PathBuf,
    pub archive: PathBuf,
    /// "running", "choose" (a variant to pick), "done", "error".
    pub status: String,
    pub progress: f32,
    pub message: String,
    pub name: String,
    pub version: String,
    pub nexus: Option<Nexus>,
    pub nexus_id: Option<u64>,
    /// Variants to choose from, and the one chosen.
    pub options: Vec<String>,
    pub chosen: Option<usize>,
    /// A download's link (nxm://…).
    pub link: String,
    pub started: u64,
}

fn dir() -> PathBuf {
    data().join("jobs")
}

impl Job {
    pub fn path(&self) -> PathBuf {
        dir().join(format!("{}.json", self.id))
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(dir());
        let tmp = self.path().with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, self.path());
        }
    }

    pub fn load(id: &str) -> Option<Job> {
        serde_json::from_str(&std::fs::read_to_string(dir().join(format!("{id}.json"))).ok()?).ok()
    }

    pub fn all() -> Vec<Job> {
        let mut v: Vec<Job> = std::fs::read_dir(dir())
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                    .filter_map(|e| {
                        serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by_key(|j| j.started);
        v
    }

    pub fn forget(&self) {
        let _ = std::fs::remove_file(self.path());
        let _ = std::fs::remove_dir_all(tmp_dir(&self.id));
    }

    pub fn new(kind: &str) -> Job {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        Job {
            id: format!("{kind}-{t}"),
            kind: kind.into(),
            status: "running".into(),
            started: now(),
            ..Default::default()
        }
    }

    pub fn fail(&mut self, e: impl Into<String>) {
        self.status = "error".into();
        self.message = e.into();
        self.save();
    }
}

fn tmp_dir(id: &str) -> PathBuf {
    data().join("tmp").join(id)
}

/// Run `lenta-mods work <id>` on its own (not tied to lenta's pipes).
pub fn spawn(id: &str) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .args(["work", id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Unpack with 7-Zip (zip, 7z, rar…), else bsdtar; the percent as it goes.
pub fn unpack(archive: &Path, to: &Path, mut progress: impl FnMut(f32)) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(to);
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let seven = ["7z", "7zz", "7za"]
        .into_iter()
        .find(|p| std::process::Command::new(p).arg("i").output().is_ok());
    if let Some(p) = seven {
        let mut ch = std::process::Command::new(p)
            .arg("x")
            .arg("-y")
            .arg("-bsp1")
            .arg("-bso0")
            .arg(format!("-o{}", to.display()))
            .arg(archive)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut out) = ch.stdout.take() {
            let mut buf = [0u8; 512];
            let mut word = String::new();
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                for &b in &buf[..n] {
                    let c = b as char;
                    if c == '%' {
                        if let Ok(v) = word.trim().parse::<f32>() {
                            progress(v / 100.0);
                        }
                        word.clear();
                    } else if c.is_ascii_digit() || c == ' ' {
                        word.push(c);
                        if word.len() > 8 {
                            word = word.split_off(word.len() - 4);
                        }
                    } else {
                        word.clear();
                    }
                }
            }
        }
        let mut err = String::new();
        if let Some(mut e) = ch.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        let st = ch.wait().map_err(|e| e.to_string())?;
        if !st.success() {
            return Err(err
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("7-Zip failed")
                .to_string());
        }
        return Ok(());
    }
    let st = std::process::Command::new("bsdtar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(to)
        .status()
        .map_err(|e| format!("no 7-Zip or bsdtar: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err("bsdtar failed".into())
    }
}

/// Start putting an archive in (the job runs on its own).
pub fn start_install(
    game: &str,
    root: &Path,
    archive: &Path,
    nexus: Option<Nexus>,
    name: Option<(String, String)>,
) -> Result<Job, String> {
    let mut j = Job::new("install");
    let file = archive
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let (n, v, nid) = crate::layout::parse_name(&file);
    let (n, v) = name.unwrap_or((n, v));
    j.game = game.into();
    j.root = root.into();
    j.archive = archive.into();
    j.name = n;
    j.version = v;
    j.nexus_id = nid.or(nexus.as_ref().map(|x| x.mod_id));
    j.nexus = nexus;
    j.message = file;
    j.save();
    spawn(&j.id)?;
    Ok(j)
}

/// The job's work, in the program run on its own.
pub fn run(id: &str) {
    let Some(mut j) = Job::load(id) else { return };
    match j.kind.as_str() {
        "install" => install(&mut j),
        "download" => download(&mut j),
        _ => {}
    }
}

fn install(j: &mut Job) {
    let tmp = tmp_dir(&j.id);
    let l = Layout::detect(&j.root);
    // First pass: unpack and see how it goes in.
    if j.chosen.is_none() {
        let id = j.id.clone();
        let r = unpack(&j.archive, &tmp, |f| {
            if let Some(mut k) = Job::load(&id) {
                k.progress = f * 0.8;
                k.save();
            }
        });
        if let Err(e) = r {
            return j.fail(e);
        }
    }
    let choices: Choices = match crate::layout::plan(&tmp, &l, &j.name) {
        Ok(c) => c,
        Err(e) => return j.fail(e),
    };
    let pick = match (choices.options.len(), j.chosen) {
        (1, _) => 0,
        (_, Some(i)) if i < choices.options.len() => i,
        _ => {
            j.options = choices.options.iter().map(|o| o.0.clone()).collect();
            j.status = "choose".into();
            j.progress = 0.8;
            j.save();
            return;
        }
    };
    let (variant, plan) = choices.options[pick].clone();
    match commit(j, &plan, &variant) {
        Ok(msg) => {
            j.status = "done".into();
            j.progress = 1.0;
            j.message = msg;
            j.save();
            let _ = std::fs::remove_dir_all(&tmp);
        }
        Err(e) => j.fail(e),
    }
}

/// One job at a time changes a game's list (two installs at once).
struct Lock(PathBuf);

impl Lock {
    fn take(game: &str) -> Lock {
        let p = store::game_dir(game).join("lock");
        let _ = std::fs::create_dir_all(store::game_dir(game));
        for _ in 0..600 {
            if std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&p)
                .is_ok()
            {
                break;
            }
            // A lock older than ten minutes is a crashed job's.
            if std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|e| e.as_secs() > 600)
            {
                let _ = std::fs::remove_file(&p);
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Lock(p)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Into the store, then into the game.
fn commit(j: &Job, plan: &Plan, variant: &str) -> Result<String, String> {
    let _lock = Lock::take(&j.game);
    let mut g = Game::load(&j.game, &j.root);
    let name = if variant.is_empty() {
        j.name.clone()
    } else {
        format!("{} ({variant})", j.name)
    };
    // The same mod again (a newer version): the old one goes first.
    let old: Vec<String> = g
        .mods
        .iter()
        .filter(|m| m.name == name)
        .map(|m| m.id.clone())
        .collect();
    for o in old {
        g.remove(&o)?;
    }
    let id = g.new_id(&name);
    let store = g.store(&id);
    let mut files = Vec::new();
    for (from, to) in &plan.moves {
        let dst = store.join(to);
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::rename(from, &dst)
            .or_else(|_| std::fs::copy(from, &dst).map(|_| ()))
            .map_err(|e| format!("{}: {e}", from.display()))?;
        files.push(to.to_string_lossy().to_string());
    }
    for m in &plan.made {
        let dst = store.join(m);
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        std::fs::write(&dst, b"").map_err(|e| e.to_string())?;
        files.push(m.to_string_lossy().to_string());
    }
    if let Some(n) = &j.nexus
        && g.domain.is_empty()
    {
        g.domain = n.domain.clone();
    }
    g.add(Rec {
        id,
        name: name.clone(),
        version: j.version.clone(),
        kind: plan.kind.clone(),
        enabled: true,
        files,
        archive: j.message.clone(),
        nexus: j.nexus.clone(),
        nexus_id: j.nexus_id,
        installed: now(),
        update: None,
    })?;
    Ok(name)
}

/// An nxm link: ask Nexus for the file's address, fetch it.
pub fn start_download(link: &str) -> Result<Job, String> {
    let l = crate::nexus::parse_link(link).ok_or("not an nxm:// link")?;
    let mut j = Job::new("download");
    j.link = link.to_string();
    j.nexus = Some(Nexus {
        domain: l.domain.clone(),
        mod_id: l.mod_id,
        file_id: l.file_id,
    });
    j.nexus_id = Some(l.mod_id);
    j.name = format!("{} #{}", l.domain, l.mod_id);
    j.save();
    Ok(j)
}

fn download(j: &mut Job) {
    let cfg = crate::nexus::Config::load();
    let Some(l) = crate::nexus::parse_link(&j.link) else {
        return j.fail("bad link");
    };
    if let Ok((name, version)) = crate::nexus::mod_info(&cfg.apikey, &l.domain, l.mod_id) {
        j.name = name;
        j.version = version;
        j.save();
    }
    let (url, file) = match crate::nexus::download_url(&cfg.apikey, &l) {
        Ok(x) => x,
        Err(e) => return j.fail(e),
    };
    let to = crate::nexus::downloads().join(&file);
    j.message = file;
    j.save();
    if store::dry() {
        j.status = "done".into();
        j.save();
        return;
    }
    let id = j.id.clone();
    let r = crate::nexus::fetch(&url, &to, |got, total| {
        if let Some(mut k) = Job::load(&id) {
            k.progress = if total > 0 {
                got as f32 / total as f32
            } else {
                0.0
            };
            k.save();
        }
    });
    match r {
        Ok(()) => {
            j.archive = to;
            j.status = "done".into();
            j.progress = 1.0;
            j.save();
        }
        Err(e) => j.fail(e),
    }
}
