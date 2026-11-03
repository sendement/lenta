//! What touches a game's install: apply a downloaded version, lock it
//! against updates, keep the lock up to date, unlock.
//!
//! The lock: Steam updates a game when its appmanifest's build and depot
//! manifests differ from the branch's current ones in appinfo. After a
//! rollback the *latest* build and manifests are written into the
//! appmanifest (Steam thinks it's current), the real old ones are kept in
//! the shared state, AutoUpdateBehavior is set to "only on launch" and the
//! appmanifest made read-only. A strong lock also makes the install
//! read-only. When a newer build ships, `relock` bumps the spoofed build.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::assemble::{COMPLETE, FILES_LIST, read_files_list};
use super::game::{self, SteamGame};
use super::history::{History, actual_depots, depotcache_manifest};
use super::manifest::Manifest;
use super::state::{Lock, State, staging_dir};
use crate::vdf::{self, Kv};

fn steam_closed() -> Result<(), String> {
    // The fake-library tests run while the real Steam may be up.
    if cfg!(test) && std::env::var_os("LENTA_DRY_STEAM").is_some() {
        return Ok(());
    }
    if crate::sources::steamplay::steam_running() {
        return Err(crate::i18n::tr("sd_steam_running").into());
    }
    Ok(())
}

// --------------------------------------------------------------- staged

#[derive(Clone, Debug, PartialEq)]
pub struct Staged {
    pub depot: String,
    /// "" when unknown (download_depot still running).
    pub manifest: String,
    pub path: PathBuf,
    /// Ours (or steam-downgrader's) rather than Steam's download_depot.
    pub ours: bool,
    pub complete: bool,
}

/// Downloaded, not yet applied versions of a game's depots.
pub fn staged(root: &Path, hist: &History, app: &str) -> Vec<Staged> {
    let mut out = Vec::new();
    for content in [
        root.join("ubuntu12_32/steamapps/content"),
        root.join("steamapps/content"),
    ] {
        let Ok(rd) = std::fs::read_dir(content.join(format!("app_{app}"))) else {
            continue;
        };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let Some(depot) = n.strip_prefix("depot_") else {
                continue;
            };
            let manifest = hist
                .downloads(app, depot)
                .last()
                .map(|d| d.0.clone())
                .unwrap_or_default();
            out.push(Staged {
                depot: depot.to_string(),
                complete: !manifest.is_empty(),
                manifest,
                path: e.path(),
                ours: false,
            });
        }
    }
    if let Ok(rd) = std::fs::read_dir(staging_dir().join(app)) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some((d, m)) = n.split_once('_')
                && e.path().is_dir()
            {
                out.push(Staged {
                    depot: d.to_string(),
                    manifest: m.to_string(),
                    complete: e.path().join(COMPLETE).is_file(),
                    path: e.path(),
                    ours: true,
                });
            }
        }
    }
    out
}

/// (path, size) of every file the folder must provide (the unchanged ones
/// stay in the install and aren't listed); and whether the list is known
/// rather than read off the folder.
pub fn staged_files(root: &Path, st: &Staged) -> (Vec<(String, u64)>, bool) {
    if let Some((_, _, files)) = read_files_list(&st.path) {
        return (
            files
                .into_iter()
                .filter(|f| !f.2)
                .map(|f| (f.0, f.1))
                .collect(),
            true,
        );
    }
    if let Some(p) = depotcache_manifest(root, &st.depot, &st.manifest)
        && let Ok(m) = Manifest::read(&p, true)
        && !m.encrypted
    {
        return (
            m.files
                .iter()
                .filter(|f| !f.is_dir())
                .map(|f| (f.name.clone(), f.size))
                .collect(),
            true,
        );
    }
    let mut files = Vec::new();
    walk(&st.path, &st.path, &mut files);
    (files, false)
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, u64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let n = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if n != ".DepotDownloader" {
                walk(base, &p, out);
            }
        } else if n != COMPLETE && n != FILES_LIST {
            let rel = p
                .strip_prefix(base)
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned();
            out.push((rel, e.metadata().map(|m| m.len()).unwrap_or(0)));
        }
    }
}

/// Lower-cased paths of all files of the staged version, the unchanged too.
fn target_names(root: &Path, st: &Staged) -> BTreeSet<String> {
    if let Some((_, _, files)) = read_files_list(&st.path) {
        return files.into_iter().map(|f| f.0.to_lowercase()).collect();
    }
    staged_files(root, st)
        .0
        .into_iter()
        .map(|f| f.0.to_lowercase())
        .collect()
}

/// The installed manifest a delta was built against, if any file relies on it.
fn base_manifest(st: &Staged) -> Option<String> {
    let (_, base, files) = read_files_list(&st.path)?;
    (files.iter().any(|f| f.2) && !base.is_empty()).then_some(base)
}

/// All present and the right size.
pub fn staged_ok(root: &Path, st: &Staged) -> bool {
    if !st.complete {
        return false;
    }
    let (files, known) = staged_files(root, st);
    files.iter().all(|(rel, size)| {
        std::fs::metadata(st.path.join(rel)).is_ok_and(|m| !known || m.len() == *size)
    })
}

/// Delete a downloaded version of one depot (only its own folder).
pub fn remove_staged(st: &Staged) {
    let _ = std::fs::remove_dir_all(&st.path);
    if let Some(parent) = st.path.parent()
        && std::fs::read_dir(parent).is_ok_and(|mut d| d.next().is_none())
    {
        let _ = std::fs::remove_dir(parent);
    }
}

// --------------------------------------------------------- permissions

fn set_tree_writable(root: &Path, writable: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mut paths = vec![root.to_path_buf()];
    let mut i = 0;
    while i < paths.len() {
        if let Ok(rd) = std::fs::read_dir(&paths[i])
            && !paths[i].is_symlink()
        {
            paths.extend(rd.flatten().map(|e| e.path()));
        }
        i += 1;
    }
    // Loosen parents first, tighten children first.
    let order: Box<dyn Iterator<Item = &PathBuf>> = if writable {
        Box::new(paths.iter())
    } else {
        Box::new(paths.iter().rev())
    };
    for p in order {
        let Ok(m) = p.symlink_metadata() else {
            continue;
        };
        if m.file_type().is_symlink() {
            continue;
        }
        let mode = m.permissions().mode();
        let new = if writable {
            mode | 0o200
        } else {
            mode & !0o222
        };
        let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(new & 0o7777));
    }
}

fn make_writable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(m) = p.symlink_metadata() {
        let _ = std::fs::set_permissions(
            p,
            std::fs::Permissions::from_mode((m.permissions().mode() | 0o200) & 0o7777),
        );
    }
}

// ---------------------------------------------------------------- apply

#[derive(Clone, Debug)]
pub struct Applied {
    pub moved: usize,
    pub deleted: usize,
}

/// Put one staged depot into the install (Steam closed). Lock afterwards.
pub fn apply_depot(
    root: &Path,
    state: &State,
    g: &SteamGame,
    st: &Staged,
    delete_extra: bool,
    progress: &dyn Fn(usize, usize),
) -> Result<Applied, String> {
    steam_closed()?;
    if st.manifest.is_empty() {
        return Err(format!("depot {}: download not finished", st.depot));
    }
    if !g.depots.contains_key(&st.depot) {
        return Err(format!("depot {} is not installed", st.depot));
    }
    if state.lock(&g.app).is_some_and(|l| l.strong) {
        set_tree_writable(&g.dir, true);
    }
    let (target, _) = staged_files(root, st);
    let names = target_names(root, st);
    let current = actual_depots(state, g).get(&st.depot).cloned();
    if let Some(base) = base_manifest(st)
        && Some(&base) != current.as_ref()
    {
        return Err(crate::i18n::tr("sd_base_changed").into());
    }
    // Files the installed build owns that the target doesn't: only ever
    // what the depot's manifest lists, so saves and configs survive.
    let mut deleted = 0;
    if delete_extra
        && let Some(cur) = current.as_ref().filter(|c| **c != st.manifest)
        && let Some(p) = depotcache_manifest(root, &st.depot, cur)
        && let Ok(m) = Manifest::read(&p, true)
        && !m.encrypted
    {
        for f in m
            .files
            .iter()
            .filter(|f| !f.is_dir() && !names.contains(&f.name.to_lowercase()))
        {
            let p = g.dir.join(&f.name);
            if p.is_file() || p.is_symlink() {
                if let Some(parent) = p.parent() {
                    make_writable(parent);
                }
                if std::fs::remove_file(&p).is_ok() {
                    deleted += 1;
                }
            }
        }
        let target_dirs: BTreeSet<String> = names
            .iter()
            .filter_map(|n| {
                Path::new(n)
                    .parent()
                    .map(|d| d.to_string_lossy().into_owned())
            })
            .collect();
        let mut dirs: Vec<&str> = m
            .files
            .iter()
            .filter(|f| f.is_dir())
            .map(|f| f.name.as_str())
            .collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.len()));
        for d in dirs {
            let p = g.dir.join(d);
            if !target_dirs.contains(&d.to_lowercase())
                && std::fs::read_dir(&p).is_ok_and(|mut r| r.next().is_none())
            {
                let _ = std::fs::remove_dir(&p);
            }
        }
    }
    let same_fs = {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(&st.path), std::fs::metadata(&g.dir)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev(),
            _ => false,
        }
    };
    let mut moved = 0;
    let mut missing = Vec::new();
    for (i, (rel, _)) in target.iter().enumerate() {
        progress(i, target.len());
        let src = st.path.join(rel);
        let dst = g.dir.join(rel);
        if !src.is_file() {
            missing.push(rel.clone());
            continue;
        }
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        if dst.exists() {
            make_writable(&dst);
        }
        if same_fs {
            std::fs::rename(&src, &dst).map_err(|e| format!("{rel}: {e}"))?;
        } else {
            let tmp = dst.with_file_name(format!(
                "{}.sdtmp",
                dst.file_name().unwrap_or_default().to_string_lossy()
            ));
            std::fs::copy(&src, &tmp).map_err(|e| format!("{rel}: {e}"))?;
            std::fs::rename(&tmp, &dst).map_err(|e| format!("{rel}: {e}"))?;
            let _ = std::fs::remove_file(&src);
        }
        moved += 1;
    }
    progress(target.len(), target.len());
    if let Some(first) = missing.first() {
        return Err(format!(
            "depot {}: {} files missing from the download (first: {first})",
            st.depot,
            missing.len()
        ));
    }
    remove_staged(st);
    Ok(Applied { moved, deleted })
}

// ----------------------------------------------------------------- lock

/// The branch's latest build and depot manifests, per appinfo.
#[allow(clippy::type_complexity)]
fn spoof_target(
    root: &Path,
    g: &SteamGame,
) -> Result<(i64, BTreeMap<String, (String, u64)>), String> {
    let info =
        game::appinfo(root, &g.app).ok_or_else(|| crate::i18n::tr("sd_no_appinfo").to_string())?;
    let branch_name = if info.branches.contains_key(g.branch()) {
        g.branch()
    } else {
        "public"
    };
    let branch = info
        .branches
        .get(branch_name)
        .ok_or_else(|| crate::i18n::tr("sd_no_appinfo").to_string())?;
    let mut manifests = BTreeMap::new();
    for d in g.depots.keys() {
        if let Some(per) = info.depots.get(d)
            && let Some(m) = per.get(branch_name).or_else(|| per.get("public"))
        {
            manifests.insert(d.clone(), m.clone());
        }
    }
    Ok((branch.buildid, manifests))
}

fn read_acf(p: &Path) -> Result<Kv, String> {
    Ok(vdf::parse_text(
        &std::fs::read_to_string(p).map_err(|e| e.to_string())?,
    ))
}

/// Atomic write that keeps (and restores) a read-only appmanifest.
fn write_acf(p: &Path, kv: &Kv, read_only: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = p.with_file_name(format!(
        "{}.sdtmp",
        p.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, kv.to_text()).map_err(|e| e.to_string())?;
    make_writable(p);
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())?;
    let mode = if read_only { 0o444 } else { 0o644 };
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())
}

fn write_spoof(
    g: &SteamGame,
    buildid: i64,
    manifests: &BTreeMap<String, (String, u64)>,
) -> Result<bool, String> {
    let mut kv = read_acf(&g.acf)?;
    let before = kv.clone();
    let st = kv.map_mut("AppState").ok_or("appmanifest: no AppState")?;
    let s = |v: String| Kv::Str(v);
    st.set("buildid", s(buildid.to_string()));
    st.set("TargetBuildID", s("0".into()));
    st.set("StateFlags", s("4".into()));
    st.set("UpdateResult", s("0".into()));
    st.set("AutoUpdateBehavior", s("1".into()));
    st.set("ScheduledAutoUpdate", s("0".into()));
    for key in [
        "BytesToDownload",
        "BytesDownloaded",
        "BytesToStage",
        "BytesStaged",
    ] {
        if st.get(key).is_some() {
            st.set(key, s("0".into()));
        }
    }
    if let Some(deps) = st.map_mut("InstalledDepots") {
        for (d, (gid, size)) in manifests {
            if deps.get(d).is_some()
                && let Some(dep) = deps.map_mut(d)
            {
                dep.set("manifest", s(gid.clone()));
                if *size > 0 {
                    dep.set("size", s(size.to_string()));
                }
            }
        }
    }
    let changed = kv != before;
    if changed {
        write_acf(&g.acf, &kv, true)?;
    } else {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&g.acf, std::fs::Permissions::from_mode(0o444));
    }
    Ok(changed)
}

/// Protect the install as it is now (after apply: `real` holds what's
/// really on disk) from Steam's updates.
pub fn lock(
    root: &Path,
    state: &mut State,
    g: &SteamGame,
    real: &BTreeMap<String, String>,
    real_buildid: Option<i64>,
    strong: bool,
) -> Result<(), String> {
    steam_closed()?;
    let prev = state.lock(&g.app);
    let mut actual = actual_depots(state, g);
    actual.extend(real.clone());
    let (buildid, manifests) = spoof_target(root, g)?;
    let installed: BTreeMap<String, String> = g
        .depots
        .iter()
        .map(|(d, x)| (d.clone(), x.manifest.clone()))
        .collect();
    // Locking an up-to-date game freezes its current build.
    let real_buildid =
        real_buildid.or_else(|| (prev.is_none() && actual == installed).then_some(g.buildid));
    write_spoof(g, buildid, &manifests)?;
    if strong {
        set_tree_writable(&g.dir, false);
    } else if prev.as_ref().is_some_and(|l| l.strong) {
        set_tree_writable(&g.dir, true);
    }
    let l = Lock {
        actual_depots: actual,
        actual_buildid: real_buildid.or(prev.as_ref().and_then(|p| p.actual_buildid)),
        orig_auto_update: prev.as_ref().map_or(g.auto_update, |p| p.orig_auto_update),
        strong,
        spoofed_buildid: buildid,
        locked_at: prev.as_ref().map_or(0, |p| p.locked_at),
    };
    state.set_lock(&g.app, &g.name, &l);
    state.save()
}

/// Bring every lock's spoofed build up to the latest Steam knows. Safe with
/// Steam running (it reads appmanifests at start and can't write the
/// read-only file). Returns (name, old, new) per change.
pub fn relock(root: &Path) -> Vec<(String, i64, i64)> {
    let mut state = State::load();
    let mut changes = Vec::new();
    for app in state.locked_apps() {
        let Some(g) = SteamGame::find(root, &app) else {
            continue;
        };
        let Ok((buildid, manifests)) = spoof_target(root, &g) else {
            continue;
        };
        let old = state.lock(&app).map_or(0, |l| l.spoofed_buildid);
        if write_spoof(&g, buildid, &manifests).unwrap_or(false) {
            changes.push((g.name.clone(), old, buildid));
            state.set_spoofed(&app, buildid);
        }
        if state.lock(&app).is_some_and(|l| l.strong) {
            set_tree_writable(&g.dir, false);
        }
    }
    if !changes.is_empty() {
        let _ = state.save();
    }
    changes
}

/// Drop the protection and tell Steam the truth, so its next update is a
/// normal one from the real old build.
pub fn unlock(state: &mut State, g: &SteamGame) -> Result<(), String> {
    steam_closed()?;
    let l = state.lock(&g.app);
    if l.as_ref().is_some_and(|l| l.strong) {
        set_tree_writable(&g.dir, true);
    }
    make_writable(&g.acf);
    if let Some(l) = l {
        let mut kv = read_acf(&g.acf)?;
        let st = kv.map_mut("AppState").ok_or("appmanifest: no AppState")?;
        if let Some(deps) = st.map_mut("InstalledDepots") {
            for (d, real) in &l.actual_depots {
                if deps.get(d).is_some()
                    && let Some(dep) = deps.map_mut(d)
                {
                    dep.set("manifest", Kv::Str(real.clone()));
                }
            }
        }
        if let Some(b) = l.actual_buildid {
            st.set("buildid", Kv::Str(b.to_string()));
        }
        st.set(
            "AutoUpdateBehavior",
            Kv::Str(l.orig_auto_update.to_string()),
        );
        write_acf(&g.acf, &kv, false)?;
        state.remove_lock(&g.app);
        state.save()?;
    }
    Ok(())
}

/// Remember the builds of installs seen, so they stay rollback targets.
pub fn snapshot_installed(root: &Path) {
    let mut state = State::load();
    let mut dirty = false;
    for lib in game::libraries(root) {
        let Ok(rd) = std::fs::read_dir(lib.join("steamapps")) else {
            continue;
        };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let Some(app) = n
                .strip_prefix("appmanifest_")
                .and_then(|s| s.strip_suffix(".acf"))
            else {
                continue;
            };
            let Some(g) = SteamGame::find(root, app) else {
                continue;
            };
            if g.state_flags == 4 && state.lock(app).is_none() && !g.depots.is_empty() {
                let d: BTreeMap<String, String> = g
                    .depots
                    .iter()
                    .map(|(k, v)| (k.clone(), v.manifest.clone()))
                    .collect();
                dirty |= state.record_snapshot(app, g.buildid, &d);
            }
        }
    }
    if dirty {
        let _ = state.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake library and game in a temp folder: apply a staged depot, lock
    /// (appmanifest spoofed and read-only, real manifests in state), unlock.
    /// Never touches the real Steam.
    #[test]
    fn apply_lock_unlock_on_fake_game() {
        let tmp = std::env::temp_dir().join(format!("lenta-ops-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("Steam");
        let sd = tmp.join("sd");
        unsafe { std::env::set_var("LENTA_SD_DIR", &sd) };
        unsafe { std::env::set_var("LENTA_DRY_STEAM", "1") };
        std::fs::create_dir_all(root.join("steamapps/common/Game")).unwrap();
        std::fs::write(root.join("steamapps/common/Game/data.bin"), b"new").unwrap();
        std::fs::write(root.join("steamapps/common/Game/save.cfg"), b"mine").unwrap();
        std::fs::write(
            root.join("steamapps/appmanifest_999.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"999\"\n\t\"name\"\t\t\"Fake\"\n\t\"installdir\"\t\t\"Game\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"buildid\"\t\t\"200\"\n\t\"AutoUpdateBehavior\"\t\t\"0\"\n\t\"InstalledDepots\"\n\t{\n\t\t\"1000\"\n\t\t{\n\t\t\t\"manifest\"\t\t\"222\"\n\t\t\t\"size\"\t\t\"3\"\n\t\t}\n\t}\n}\n",
        )
        .unwrap();
        // A finished download of manifest 111.
        let stage = sd.join("staging/999/1000_111");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join("data.bin"), b"old!").unwrap();
        std::fs::write(
            stage.join(FILES_LIST),
            r#"{"manifest":"111","base_manifest":"","files":[["data.bin",4,false]]}"#,
        )
        .unwrap();
        std::fs::write(stage.join(COMPLETE), b"").unwrap();
        let state0 = State::load();
        let hist = History::new(&root, &state0);
        let st = staged(&root, &hist, "999");
        assert_eq!(st.len(), 1);
        assert!(staged_ok(&root, &st[0]));
        let g = SteamGame::find(&root, "999").unwrap();
        apply_depot(&root, &state0, &g, &st[0], true, &|_, _| {}).unwrap();
        assert_eq!(std::fs::read(g.dir.join("data.bin")).unwrap(), b"old!");
        assert_eq!(std::fs::read(g.dir.join("save.cfg")).unwrap(), b"mine");
        assert!(!stage.exists());
        // No appinfo for a fake app: lock must say so, not half-work.
        let mut state = State::load();
        let real: BTreeMap<String, String> = [("1000".to_string(), "111".to_string())].into();
        assert!(lock(&root, &mut state, &g, &real, Some(100), false).is_err());
        // Spoof by hand as lock would, then unlock restores the truth.
        write_spoof(
            &g,
            300,
            &[("1000".to_string(), ("333".to_string(), 3))].into(),
        )
        .unwrap();
        state.set_lock(
            "999",
            "Fake",
            &Lock {
                actual_depots: real,
                actual_buildid: Some(100),
                orig_auto_update: 0,
                strong: false,
                spoofed_buildid: 300,
                locked_at: 0,
            },
        );
        state.save().unwrap();
        let g = SteamGame::find(&root, "999").unwrap();
        assert_eq!(g.buildid, 300);
        assert_eq!(g.depots["1000"].manifest, "333");
        assert_eq!(g.auto_update, 1);
        let mut state = State::load();
        assert_eq!(actual_depots(&state, &g)["1000"], "111");
        unlock(&mut state, &g).unwrap();
        let g = SteamGame::find(&root, "999").unwrap();
        assert_eq!(g.buildid, 100);
        assert_eq!(g.depots["1000"].manifest, "111");
        assert_eq!(g.auto_update, 0);
        assert!(State::load().lock("999").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
