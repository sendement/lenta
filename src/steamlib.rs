//! Steam's libraries without Steam's windows: where games live, a new
//! library on another drive, installing a game into the library chosen
//! (an appmanifest Steam picks up and downloads by itself), moving a game
//! between libraries (its files, its Proton prefix, shader cache and
//! Workshop items) and uninstalling one.
//!
//! Steam rewrites libraryfolders.vdf when it quits and doesn't look for
//! new appmanifests while it runs, so the last step of each is done with
//! Steam closed; the caller closes it and starts it again (in the tray).

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::vdf::{self, Kv};

/// A library: its folder (the one holding steamapps/), its drive's space,
/// how many games are in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub path: PathBuf,
    pub free: u64,
    pub total: u64,
    pub games: usize,
}

/// A drive with no library yet, where one could go.
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub mount: PathBuf,
    pub fs: String,
    pub free: u64,
    pub total: u64,
    /// The folder the library would be (mount/SteamLibrary).
    pub folder: PathBuf,
}

/// Where a library goes: one there is, or a new one on a drive.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    Lib(PathBuf),
    New(PathBuf),
}

impl Place {
    pub fn path(&self) -> &Path {
        match self {
            Place::Lib(p) | Place::New(p) => p,
        }
    }
}

pub fn root() -> Option<PathBuf> {
    crate::sources::steam::roots().into_iter().next()
}

/// (free, total) bytes on the drive holding `p`.
pub fn space(p: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let c = CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let f = s.f_frsize as u64;
    Some((s.f_bavail as u64 * f, s.f_blocks as u64 * f))
}

fn dev(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok().map(|m| m.dev())
}

pub fn libraries(root: &Path) -> Vec<Library> {
    crate::sources::steam::library_dirs(root)
        .into_iter()
        .map(|path| {
            let (free, total) = space(&path).unwrap_or((0, 0));
            let games = std::fs::read_dir(path.join("steamapps"))
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| {
                            let n = e.file_name();
                            let n = n.to_string_lossy();
                            n.starts_with("appmanifest_") && n.ends_with(".acf")
                        })
                        .count()
                })
                .unwrap_or(0);
            Library {
                path,
                free,
                total,
                games,
            }
        })
        .collect()
}

/// The library holding this app, and its appmanifest.
pub fn library_of(root: &Path, appid: u32) -> Option<(PathBuf, PathBuf)> {
    crate::sources::steam::library_dirs(root)
        .into_iter()
        .find_map(|l| {
            let acf = l.join(format!("steamapps/appmanifest_{appid}.acf"));
            acf.is_file().then_some((l, acf))
        })
}

/// File systems a Steam library works on (Proton needs real Unix ones;
/// NTFS mostly works, FAT doesn't).
const GOOD_FS: [&str; 8] = [
    "ext4", "btrfs", "xfs", "f2fs", "bcachefs", "zfs", "ntfs3", "fuseblk",
];

/// Drives (mounted, writable, a usable file system) with no library on
/// them yet.
pub fn drives(libs: &[Library]) -> Vec<Drive> {
    let text = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    drives_in(&text, libs)
}

/// (device, mount point, file system) of each mount.
fn mount_list(mounts: &str) -> Vec<(String, PathBuf, String)> {
    mounts
        .lines()
        .filter_map(|line| {
            let mut w = line.split_whitespace();
            let (src, mount, fs) = (w.next()?, w.next()?, w.next()?);
            // Spaces in mount points come as \040.
            Some((
                src.to_string(),
                PathBuf::from(mount.replace("\\040", " ")),
                fs.to_string(),
            ))
        })
        .collect()
}

/// The device a path is on: the deepest mount above it (btrfs subvolumes
/// of one disk share it, though their st_dev differ).
fn device_of(list: &[(String, PathBuf, String)], p: &Path) -> Option<String> {
    list.iter()
        .filter(|(_, m, _)| p.starts_with(m))
        .max_by_key(|(_, m, _)| m.as_os_str().len())
        .map(|(s, _, _)| s.clone())
}

/// The file system a path is on ("ext4", "exfat"…).
pub fn fs_of(p: &Path) -> Option<String> {
    let text = std::fs::read_to_string("/proc/self/mounts").ok()?;
    let list = mount_list(&text);
    list.iter()
        .filter(|(_, m, _)| p.starts_with(m))
        .max_by_key(|(_, m, _)| m.as_os_str().len())
        .map(|(_, _, f)| f.clone())
}

/// No Unix links or modes there: Proton prefixes can't live on it, and a
/// locked appmanifest can't be read-only.
pub fn plain_fs(fs: &str) -> bool {
    matches!(fs, "exfat" | "vfat" | "msdos" | "fat")
}

fn drives_in(mounts: &str, libs: &[Library]) -> Vec<Drive> {
    let list = mount_list(mounts);
    let taken: Vec<String> = libs
        .iter()
        .filter_map(|l| device_of(&list, &l.path))
        .collect();
    let home = dirs::home_dir().unwrap_or_default();
    let home_dev = device_of(&list, &home);
    let mut seen: Vec<String> = Vec::new();
    let mut out: Vec<Drive> = Vec::new();
    for (src, mount, fs) in &list {
        if !GOOD_FS.contains(&fs.as_str()) || taken.contains(src) || seen.contains(src) {
            continue;
        }
        let m = mount.to_string_lossy();
        let user_drive = m.starts_with("/run/media/")
            || m.starts_with("/media/")
            || m.starts_with("/mnt/")
            || mount == Path::new("/")
            || home.starts_with(mount);
        if !user_drive {
            continue;
        }
        // The system drive: a library in the home folder.
        let base = if home_dev.as_deref() == Some(src.as_str()) {
            home.clone()
        } else {
            mount.clone()
        };
        if !writable(&base) {
            continue;
        }
        seen.push(src.clone());
        let (free, total) = space(&base).unwrap_or((0, 0));
        out.push(Drive {
            folder: base.join("SteamLibrary"),
            mount: mount.clone(),
            fs: fs.clone(),
            free,
            total,
        });
    }
    out
}

fn writable(p: &Path) -> bool {
    use std::ffi::CString;
    CString::new(p.as_os_str().as_encoded_bytes())
        .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
}

// ---------------------------------------------------------- libraryfolders

/// Both copies Steam keeps.
fn folders_files(root: &Path) -> Vec<PathBuf> {
    [
        root.join("steamapps/libraryfolders.vdf"),
        root.join("config/libraryfolders.vdf"),
    ]
    .into_iter()
    .filter(|p| p.is_file())
    .collect()
}

/// Read, change, check it reads back the same, write (a copy of the old
/// one kept beside it).
fn edit_vdf(path: &Path, f: impl FnOnce(&mut Kv) -> Result<(), String>) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut kv = vdf::parse_text(&text);
    f(&mut kv)?;
    let out = kv.to_text();
    if vdf::parse_text(&out) != kv {
        return Err(format!("{}: could not be written safely", path.display()));
    }
    let _ = std::fs::copy(path, path.with_extension("vdf.lenta-backup"));
    let tmp = path.with_extension("vdf.lenta-tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn content_id() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Steam's are 64-bit numbers; any unique one will do.
    format!(
        "{}",
        (t as u64 ^ (std::process::id() as u64) << 40) & 0x7fff_ffff_ffff_ffff
    )
}

/// Make `dir` a library and add it to Steam's list. Steam must be closed.
pub fn add_library(root: &Path, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir.join("steamapps")).map_err(|e| e.to_string())?;
    let id = content_id();
    let own = dir.join("libraryfolder.vdf");
    if !own.is_file() {
        std::fs::write(
            &own,
            format!(
                "\"libraryfolder\"\n{{\n\t\"contentid\"\t\t\"{id}\"\n\t\"label\"\t\t\"\"\n}}\n"
            ),
        )
        .map_err(|e| e.to_string())?;
    }
    let path = dir.to_string_lossy().into_owned();
    for f in folders_files(root) {
        edit_vdf(&f, |kv| {
            let folders = kv
                .map_mut("libraryfolders")
                .ok_or("libraryfolders.vdf: unexpected layout")?;
            if folders
                .entries()
                .iter()
                .any(|(_, v)| v.get_str("path") == Some(path.as_str()))
            {
                return Ok(());
            }
            let n = folders
                .entries()
                .iter()
                .filter_map(|(k, _)| k.parse::<u32>().ok())
                .max()
                .map_or(0, |m| m + 1);
            folders.set(
                &n.to_string(),
                Kv::Map(vec![
                    ("path".into(), Kv::Str(path.clone())),
                    ("label".into(), Kv::Str(String::new())),
                    ("contentid".into(), Kv::Str(id.clone())),
                    ("totalsize".into(), Kv::Str("0".into())),
                    ("update_clean_bytes_tally".into(), Kv::Str("0".into())),
                    ("time_last_update_verified".into(), Kv::Str("0".into())),
                    ("apps".into(), Kv::Map(Vec::new())),
                ]),
            );
            Ok(())
        })?;
    }
    Ok(())
}

/// Note in libraryfolders.vdf which library holds the app (Steam keeps a
/// list per library; `to` None takes it off).
fn set_app_library(root: &Path, appid: u32, to: Option<&Path>, size: u64) -> Result<(), String> {
    let id = appid.to_string();
    let to = to.map(|p| p.canonicalize().unwrap_or(p.to_path_buf()));
    for f in folders_files(root) {
        edit_vdf(&f, |kv| {
            let Some(Kv::Map(folders)) = kv.map_mut("libraryfolders") else {
                return Err("libraryfolders.vdf: unexpected layout".into());
            };
            for (_, lib) in folders.iter_mut() {
                let here = lib.get_str("path").map(|p| {
                    let p = PathBuf::from(p);
                    p.canonicalize().unwrap_or(p)
                });
                let Some(apps) = lib.map_mut("apps") else {
                    continue;
                };
                apps.remove(&id);
                if here.is_some() && here == to {
                    apps.set(&id, Kv::Str(size.to_string()));
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

// ------------------------------------------------------------------ install

/// The folder Steam would install the app in (from appinfo), else one
/// made from its name.
pub fn install_dir_name(root: &Path, appid: u32, name: &str) -> String {
    let info = crate::sources::steam::appinfo(root, &[appid].into_iter().collect());
    let from_info = info.get(&appid).and_then(|kv| {
        kv.path(&["appinfo", "config", "installdir"])
            .or_else(|| kv.path(&["config", "installdir"]))
            .and_then(Kv::str)
            .map(str::to_string)
    });
    from_info
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            name.chars()
                .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
                .collect::<String>()
                .trim()
                .to_string()
        })
}

/// The account Steam signed in last, as a SteamID64.
fn owner(root: &Path) -> Option<u64> {
    crate::sources::steam::users(root)
        .first()
        .map(|(id, _)| 76561197960265728 + *id as u64)
}

/// The appmanifest that has Steam download the app into this library the
/// next time it starts: "update required", nothing there yet.
pub fn install_manifest(appid: u32, name: &str, installdir: &str, owner: Option<u64>) -> String {
    let kv = Kv::Map(vec![(
        "AppState".into(),
        Kv::Map(vec![
            ("appid".into(), Kv::Str(appid.to_string())),
            ("Universe".into(), Kv::Str("1".into())),
            ("name".into(), Kv::Str(name.into())),
            ("StateFlags".into(), Kv::Str("1026".into())),
            ("installdir".into(), Kv::Str(installdir.into())),
            ("LastUpdated".into(), Kv::Str("0".into())),
            ("SizeOnDisk".into(), Kv::Str("0".into())),
            ("StagingSize".into(), Kv::Str("0".into())),
            ("buildid".into(), Kv::Str("0".into())),
            (
                "LastOwner".into(),
                Kv::Str(owner.map_or("0".into(), |o| o.to_string())),
            ),
            ("UpdateResult".into(), Kv::Str("0".into())),
            ("BytesToDownload".into(), Kv::Str("0".into())),
            ("BytesDownloaded".into(), Kv::Str("0".into())),
            ("BytesToStage".into(), Kv::Str("0".into())),
            ("BytesStaged".into(), Kv::Str("0".into())),
            ("TargetBuildID".into(), Kv::Str("0".into())),
            ("AutoUpdateBehavior".into(), Kv::Str("0".into())),
            (
                "AllowOtherDownloadsWhileRunning".into(),
                Kv::Str("0".into()),
            ),
            ("ScheduledAutoUpdate".into(), Kv::Str("0".into())),
            ("UserConfig".into(), Kv::Map(Vec::new())),
            ("MountedConfig".into(), Kv::Map(Vec::new())),
        ]),
    )]);
    kv.to_text()
}

/// Put the game in for Steam to download (Steam closed): the library made
/// if it's a new one, its appmanifest written.
pub fn install(root: &Path, place: &Place, appid: u32, name: &str) -> Result<(), String> {
    if let Some((lib, _)) = library_of(root, appid) {
        return Err(format!("already in {}", lib.display()));
    }
    if let Place::New(dir) = place {
        add_library(root, dir)?;
    }
    let lib = place.path();
    let dir = install_dir_name(root, appid, name);
    let acf = lib.join(format!("steamapps/appmanifest_{appid}.acf"));
    std::fs::create_dir_all(lib.join("steamapps")).map_err(|e| e.to_string())?;
    std::fs::write(&acf, install_manifest(appid, name, &dir, owner(root)))
        .map_err(|e| e.to_string())?;
    set_app_library(root, appid, Some(lib), 0)
}

// --------------------------------------------------------------- the files

/// A game's things in a library: (where, what it is). The appmanifest is
/// kept apart: it goes last.
pub fn game_parts(lib: &Path, appid: u32, installdir: &str) -> Vec<PathBuf> {
    let a = lib.join("steamapps");
    [
        a.join("common").join(installdir),
        a.join("compatdata").join(appid.to_string()),
        a.join("shadercache").join(appid.to_string()),
        a.join("workshop/content").join(appid.to_string()),
        a.join(format!("workshop/appworkshop_{appid}.acf")),
    ]
    .into_iter()
    .filter(|p| p.symlink_metadata().is_ok())
    .collect()
}

pub fn tree_size(p: &Path) -> u64 {
    let Ok(m) = p.symlink_metadata() else {
        return 0;
    };
    if m.is_dir() {
        std::fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum())
            .unwrap_or(0)
    } else if m.is_file() {
        m.len()
    } else {
        0
    }
}

/// How far a copy has got.
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    /// What's being done (an i18n key).
    pub stage: &'static str,
}

pub const CANCELLED: &str = "cancelled";
/// A Proton game onto exFAT/FAT.
pub const NO_LINKS: &str = "no-links";

/// Copy one file in pieces (in the kernel when it can), reporting.
fn copy_file(
    src: &Path,
    dst: &Path,
    report: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let from = std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let meta = from.metadata().map_err(|e| e.to_string())?;
    let to = std::fs::File::create(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut left = meta.len();
    let mut kernel = true;
    let mut buf = Vec::new();
    while left > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.into());
        }
        let chunk = left.min(64 << 20) as usize;
        let n = if kernel {
            let r = unsafe {
                libc::copy_file_range(
                    from.as_raw_fd(),
                    std::ptr::null_mut(),
                    to.as_raw_fd(),
                    std::ptr::null_mut(),
                    chunk,
                    0,
                )
            };
            if r < 0 {
                let e = std::io::Error::last_os_error();
                match e.raw_os_error() {
                    Some(libc::EXDEV)
                    | Some(libc::ENOSYS)
                    | Some(libc::EINVAL)
                    | Some(libc::EOPNOTSUPP) => {
                        kernel = false;
                        continue;
                    }
                    _ => return Err(format!("{}: {e}", src.display())),
                }
            }
            r as usize
        } else {
            use std::io::{Read, Write};
            if buf.is_empty() {
                buf = vec![0u8; 8 << 20];
            }
            let want = chunk.min(buf.len());
            let n = (&from).read(&mut buf[..want]).map_err(|e| e.to_string())?;
            (&to).write_all(&buf[..n]).map_err(|e| e.to_string())?;
            n
        };
        if n == 0 {
            break;
        }
        left -= n as u64;
        report(n as u64);
    }
    to.sync_all().map_err(|e| e.to_string())?;
    std::fs::set_permissions(dst, std::fs::Permissions::from_mode(meta.mode() & 0o7777))
        .map_err(|e| e.to_string())
}

/// Copy a tree as it is: folders, files with their modes, links as links
/// (Proton prefixes are full of them).
pub fn copy_tree(
    src: &Path,
    dst: &Path,
    report: &mut dyn FnMut(u64),
    cancel: &AtomicBool,
) -> Result<(), String> {
    let m = src
        .symlink_metadata()
        .map_err(|e| format!("{}: {e}", src.display()))?;
    if m.file_type().is_symlink() {
        let t = std::fs::read_link(src).map_err(|e| e.to_string())?;
        std::os::unix::fs::symlink(&t, dst).map_err(|e| format!("{}: {e}", dst.display()))
    } else if m.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        let mut entries: Vec<_> = std::fs::read_dir(src)
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            copy_tree(&e.path(), &dst.join(e.file_name()), report, cancel)?;
        }
        std::fs::set_permissions(dst, std::fs::Permissions::from_mode(m.mode() & 0o7777))
            .map_err(|e| e.to_string())
    } else if m.is_file() {
        copy_file(src, dst, report, cancel)
    } else {
        // Sockets, fifos: nothing a game needs moved.
        Ok(())
    }
}

fn remove_any(p: &Path) -> Result<(), String> {
    let m = match p.symlink_metadata() {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };
    let r = if m.is_dir() {
        // Read-only folders inside can't be emptied: open them up first.
        let _ = make_writable(p);
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    };
    r.map_err(|e| format!("{}: {e}", p.display()))
}

fn make_writable(p: &Path) -> std::io::Result<()> {
    let m = p.symlink_metadata()?;
    if m.is_dir() {
        if m.mode() & 0o200 == 0 {
            std::fs::set_permissions(
                p,
                std::fs::Permissions::from_mode((m.mode() | 0o700) & 0o7777),
            )?;
        }
        for e in std::fs::read_dir(p)?.flatten() {
            make_writable(&e.path())?;
        }
    }
    Ok(())
}

/// A game's move, worked out before anything is touched.
#[derive(Clone, Debug)]
pub struct Move {
    pub appid: u32,
    pub from: PathBuf,
    pub to: PathBuf,
    pub acf: PathBuf,
    pub parts: Vec<PathBuf>,
    pub bytes: u64,
    /// Same drive: renamed, nothing copied.
    pub same_drive: bool,
}

pub fn plan_move(root: &Path, appid: u32, to: &Path) -> Result<Move, String> {
    let (from, acf) = library_of(root, appid).ok_or("not installed")?;
    let text = std::fs::read_to_string(&acf).map_err(|e| e.to_string())?;
    let kv = vdf::parse_text(&text);
    let st = kv.get("AppState").ok_or("appmanifest: unexpected layout")?;
    let installdir = st
        .get_str("installdir")
        .filter(|d| !d.is_empty() && !d.contains('/') && *d != "..")
        .ok_or("appmanifest: no installdir")?
        .to_string();
    // Only a game at rest: Steam not downloading or checking it.
    let flags = st.get_int("StateFlags").unwrap_or(0);
    if flags != 4 && flags & 4 == 0 {
        return Err("busy".into());
    }
    if flags & (2 | 256 | 512 | 1024) != 0 {
        return Err("busy".into());
    }
    let to_c = to.canonicalize().unwrap_or(to.to_path_buf());
    if from == to_c {
        return Err("same library".into());
    }
    let parts = game_parts(&from, appid, &installdir);
    let has_prefix = parts
        .iter()
        .any(|p| p.to_string_lossy().contains("/steamapps/compatdata/"));
    let mut probe = to.to_path_buf();
    while !probe.exists() && probe.pop() {}
    if has_prefix && fs_of(&probe).is_some_and(|f| plain_fs(&f)) {
        return Err(NO_LINKS.into());
    }
    for p in &parts {
        let rel = p.strip_prefix(&from).unwrap_or(p);
        if to.join(rel).symlink_metadata().is_ok() {
            return Err(format!("already there: {}", to.join(rel).display()));
        }
    }
    let bytes = parts.iter().map(|p| tree_size(p)).sum();
    let parent = |p: &Path| {
        let mut q = p.to_path_buf();
        while !q.exists() {
            if !q.pop() {
                break;
            }
        }
        q
    };
    let same_drive =
        dev(&from.join("steamapps")).is_some() && dev(&from.join("steamapps")) == dev(&parent(to));
    if !same_drive
        && let Some((free, _)) = space(&parent(to))
        && free < bytes + (1 << 30)
    {
        return Err("no room".into());
    }
    Ok(Move {
        appid,
        from,
        to: to.to_path_buf(),
        acf,
        parts,
        bytes,
        same_drive,
    })
}

impl Move {
    fn dst(&self, p: &Path) -> PathBuf {
        self.to.join(p.strip_prefix(&self.from).unwrap_or(p))
    }

    /// The long part, with Steam running: copy everything over (another
    /// drive). On failure or cancel what was copied is taken away again.
    pub fn copy(&self, report: &mut dyn FnMut(u64), cancel: &AtomicBool) -> Result<(), String> {
        if self.same_drive {
            return Ok(());
        }
        let mut made: Vec<PathBuf> = Vec::new();
        let r = (|| {
            for p in &self.parts {
                let d = self.dst(p);
                if let Some(parent) = d.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                made.push(d.clone());
                copy_tree(p, &d, report, cancel)?;
            }
            Ok(())
        })();
        if r.is_err() {
            for d in made.iter().rev() {
                let _ = remove_any(d);
            }
        }
        r
    }

    /// A copy made but not to be used (another game's move failed): gone.
    pub fn discard(&self) {
        if self.same_drive {
            return;
        }
        for p in self.parts.iter().rev() {
            let _ = remove_any(&self.dst(p));
        }
    }

    /// The last part, Steam closed: the appmanifest over, Steam's list
    /// changed, the old copy gone (renamed over on the same drive).
    pub fn commit(&self, root: &Path) -> Result<(), String> {
        let new_acf = self
            .to
            .join(format!("steamapps/appmanifest_{}.acf", self.appid));
        if self.same_drive {
            for p in &self.parts {
                let d = self.dst(p);
                if let Some(parent) = d.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::rename(p, &d).map_err(|e| format!("{}: {e}", p.display()))?;
            }
        }
        std::fs::create_dir_all(self.to.join("steamapps")).map_err(|e| e.to_string())?;
        // Keeps its mode: steam-downgrader's lock makes it read-only.
        let mode = std::fs::metadata(&self.acf).map(|m| m.mode()).ok();
        std::fs::copy(&self.acf, &new_acf).map_err(|e| e.to_string())?;
        if let Some(m) = mode {
            let _ = std::fs::set_permissions(&new_acf, std::fs::Permissions::from_mode(m & 0o7777));
        }
        // Locked by steam-downgrader: read-only again (a copy from exFAT
        // comes without its mode).
        if crate::downgrade::state::State::load()
            .lock(&self.appid.to_string())
            .is_some()
        {
            let _ = std::fs::set_permissions(&new_acf, std::fs::Permissions::from_mode(0o444));
        }
        std::fs::remove_file(&self.acf).map_err(|e| e.to_string())?;
        set_app_library(root, self.appid, Some(&self.to), self.bytes)?;
        if !self.same_drive {
            for p in &self.parts {
                remove_any(p)?;
            }
        }
        Ok(())
    }
}

/// What the appmanifest says the game takes.
pub fn size_on_disk(acf: &Path) -> u64 {
    std::fs::read_to_string(acf)
        .ok()
        .and_then(|t| {
            vdf::parse_text(&t)
                .path(&["AppState", "SizeOnDisk"])
                .and_then(Kv::int)
        })
        .unwrap_or(0)
        .max(0) as u64
}

// ------------------------------------------------------- with Steam closed

/// Run `f` with Steam closed: closed first if it runs, started again
/// after (in the tray) if it ran, or if `start` (an install: Steam then
/// downloads it).
pub fn steam_closed<T>(start: bool, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    use crate::sources::steamplay::steam_running;
    let was = steam_running();
    if was {
        crate::game::run_first(&crate::sources::steam::steam_run(&["-shutdown"]))?;
        let t = std::time::Instant::now();
        while steam_running() {
            if t.elapsed() > std::time::Duration::from_secs(90) {
                return Err(crate::i18n::tr("mg_steam_stuck").into());
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        // Let it finish writing its files.
        std::thread::sleep(std::time::Duration::from_millis(1500));
    }
    let r = f();
    if was || start {
        let _ = crate::game::run_first(&crate::sources::steam::steam_run(&["-silent"]));
    }
    r
}

/// Move these games to `to`: all copied first with Steam running, then
/// Steam closed a moment to switch them over. (moved, skipped as busy.)
pub fn move_games(
    root: &Path,
    apps: &[u32],
    to: &Place,
    report: &dyn Fn(Progress),
    cancel: &AtomicBool,
) -> Result<(usize, usize), String> {
    let dir = to.path().to_path_buf();
    let mut moves = Vec::new();
    let mut skipped = 0;
    for id in apps {
        match plan_move(root, *id, &dir) {
            Ok(m) => moves.push(m),
            Err(_) if apps.len() > 1 => skipped += 1,
            Err(e) => return Err(e),
        }
    }
    let total: u64 = moves
        .iter()
        .filter(|m| !m.same_drive)
        .map(|m| m.bytes)
        .sum();
    let mut done = 0u64;
    for (i, m) in moves.iter().enumerate() {
        let r = m.copy(
            &mut |n| {
                done += n;
                report(Progress {
                    done,
                    total,
                    stage: "mg_copying",
                });
            },
            cancel,
        );
        if let Err(e) = r {
            for c in &moves[..i] {
                c.discard();
            }
            return Err(e);
        }
    }
    report(Progress {
        done: total,
        total,
        stage: "mg_switching",
    });
    steam_closed(false, || {
        if let Place::New(d) = to {
            add_library(root, d)?;
        }
        for m in &moves {
            m.commit(root)?;
        }
        Ok(())
    })?;
    Ok((moves.len(), skipped))
}

// ---------------------------------------------------------------- uninstall

/// What uninstalling takes away (the Proton prefix stays: saves live in
/// it) and how much that frees.
pub fn uninstall_plan(root: &Path, appid: u32) -> Result<(PathBuf, Vec<PathBuf>, u64), String> {
    let (lib, acf) = library_of(root, appid).ok_or("not installed")?;
    let text = std::fs::read_to_string(&acf).map_err(|e| e.to_string())?;
    let kv = vdf::parse_text(&text);
    let installdir = kv
        .path(&["AppState", "installdir"])
        .and_then(Kv::str)
        .filter(|d| !d.is_empty() && !d.contains('/') && *d != "..")
        .ok_or("appmanifest: no installdir")?
        .to_string();
    let parts: Vec<PathBuf> = game_parts(&lib, appid, &installdir)
        .into_iter()
        .filter(|p| !p.to_string_lossy().contains("/steamapps/compatdata/"))
        .collect();
    // The game's own size from the appmanifest (walking a big game's
    // files takes a while), the rest counted.
    let common = lib.join("steamapps/common").join(&installdir);
    let bytes = parts
        .iter()
        .map(|p| {
            if *p == common {
                size_on_disk(&acf).max(1)
            } else {
                tree_size(p)
            }
        })
        .sum();
    Ok((acf, parts, bytes))
}

/// Steam closed: the appmanifest first (so Steam never sees a half game),
/// then the files.
pub fn uninstall(root: &Path, appid: u32) -> Result<u64, String> {
    let (acf, parts, bytes) = uninstall_plan(root, appid)?;
    std::fs::remove_file(&acf).map_err(|e| e.to_string())?;
    set_app_library(root, appid, None, 0)?;
    for p in &parts {
        remove_any(p)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Steam with two libraries and one game with a prefix, in a temp
    /// folder.
    fn fake() -> (PathBuf, PathBuf, PathBuf) {
        let t = std::env::temp_dir().join(format!(
            "lenta-steamlib-{}-{}",
            std::process::id(),
            content_id()
        ));
        let root = t.join("Steam");
        let other = t.join("Drive/SteamLibrary");
        let a = root.join("steamapps");
        std::fs::create_dir_all(a.join("common/Game Dir/bin")).unwrap();
        std::fs::create_dir_all(a.join("compatdata/70/pfx/drive_c")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::create_dir_all(other.join("steamapps")).unwrap();
        std::fs::write(a.join("common/Game Dir/bin/game.exe"), vec![7u8; 300_000]).unwrap();
        std::os::unix::fs::symlink("bin/game.exe", a.join("common/Game Dir/link")).unwrap();
        std::fs::write(a.join("compatdata/70/pfx/drive_c/save.dat"), b"save").unwrap();
        std::fs::write(
            a.join("appmanifest_70.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"70\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"Game Dir\"\n}\n",
        )
        .unwrap();
        std::fs::set_permissions(
            a.join("appmanifest_70.acf"),
            std::fs::Permissions::from_mode(0o444),
        )
        .unwrap();
        let folders = format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"70\"\t\t\"300000\"\n\t\t}}\n\t}}\n}}\n",
            root.display()
        );
        std::fs::write(a.join("libraryfolders.vdf"), &folders).unwrap();
        std::fs::write(root.join("config/libraryfolders.vdf"), &folders).unwrap();
        (t, root, other)
    }

    #[test]
    fn library_added_moved_to_and_uninstalled() {
        let (t, root, other) = fake();
        add_library(&root, &other).unwrap();
        add_library(&root, &other).unwrap();
        let libs = libraries(&root);
        assert_eq!(libs.len(), 2, "{libs:?}");
        assert_eq!(libs[0].games, 1);
        let other = other.canonicalize().unwrap();
        let m = plan_move(&root, 70, &other).unwrap();
        assert_eq!(m.parts.len(), 2, "game and prefix");
        assert!(m.bytes >= 300_004);
        let mut n = 0;
        // Copy as if to another drive.
        let m = Move {
            same_drive: false,
            ..m
        };
        m.copy(&mut |b| n += b, &AtomicBool::new(false)).unwrap();
        assert_eq!(n, m.bytes);
        m.commit(&root).unwrap();
        let g = other.join("steamapps/common/Game Dir");
        assert_eq!(
            std::fs::read(g.join("bin/game.exe")).unwrap().len(),
            300_000
        );
        assert_eq!(
            std::fs::read_link(g.join("link")).unwrap(),
            Path::new("bin/game.exe")
        );
        assert!(
            other
                .join("steamapps/compatdata/70/pfx/drive_c/save.dat")
                .is_file()
        );
        let acf = other.join("steamapps/appmanifest_70.acf");
        assert_eq!(
            std::fs::metadata(&acf).unwrap().mode() & 0o777,
            0o444,
            "lock kept"
        );
        assert!(!root.join("steamapps/common/Game Dir").exists());
        assert!(!root.join("steamapps/appmanifest_70.acf").exists());
        let kv = vdf::parse_text(
            &std::fs::read_to_string(root.join("config/libraryfolders.vdf")).unwrap(),
        );
        assert!(kv.path(&["libraryfolders", "0", "apps", "70"]).is_none());
        assert!(kv.path(&["libraryfolders", "1", "apps", "70"]).is_some());
        assert_eq!(library_of(&root, 70).unwrap().0, other);

        // Back on the same drive: renamed.
        let back = plan_move(&root, 70, &root).unwrap();
        assert!(back.same_drive);
        back.copy(&mut |_| {}, &AtomicBool::new(false)).unwrap();
        back.commit(&root).unwrap();
        assert!(
            root.join("steamapps/common/Game Dir/bin/game.exe")
                .is_file()
        );

        // Uninstalled: the prefix (saves) stays.
        let (_, parts, _) = uninstall_plan(&root, 70).unwrap();
        assert_eq!(parts.len(), 1);
        uninstall(&root, 70).unwrap();
        assert!(!root.join("steamapps/common/Game Dir").exists());
        assert!(!root.join("steamapps/appmanifest_70.acf").exists());
        assert!(
            root.join("steamapps/compatdata/70/pfx/drive_c/save.dat")
                .is_file()
        );

        // Installed again into the other library: a manifest Steam downloads.
        install(&root, &Place::Lib(other.clone()), 70, "Some: Game").unwrap();
        let kv = vdf::parse_text(
            &std::fs::read_to_string(other.join("steamapps/appmanifest_70.acf")).unwrap(),
        );
        assert_eq!(
            kv.path(&["AppState", "StateFlags"]).and_then(Kv::int),
            Some(1026)
        );
        assert_eq!(
            kv.path(&["AppState", "installdir"]).and_then(Kv::str),
            Some("Some Game")
        );
        assert!(
            install(&root, &Place::Lib(other.clone()), 70, "x").is_err(),
            "not twice"
        );
        let _ = std::fs::remove_dir_all(t);
    }

    #[test]
    fn cancelled_copy_leaves_nothing() {
        let (t, root, other) = fake();
        add_library(&root, &other).unwrap();
        let m = plan_move(&root, 70, &other).unwrap();
        let m = Move {
            same_drive: false,
            ..m
        };
        let stop = AtomicBool::new(true);
        assert_eq!(m.copy(&mut |_| {}, &stop), Err(CANCELLED.to_string()));
        assert!(!other.join("steamapps/common/Game Dir").exists());
        assert!(root.join("steamapps/appmanifest_70.acf").is_file());
        let _ = std::fs::remove_dir_all(t);
    }

    #[test]
    fn drives_without_a_library() {
        let mounts = "/dev/a / btrfs rw 0 0\n/dev/b /boot vfat rw 0 0\nproc /proc proc rw 0 0\n";
        // The system drive already holds the library here.
        let home = dirs::home_dir().unwrap();
        let libs = [Library {
            path: home.clone(),
            free: 0,
            total: 0,
            games: 0,
        }];
        assert!(drives_in(mounts, &libs).is_empty());
        let d = drives_in(mounts, &[]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].folder, home.join("SteamLibrary"));
    }
}
