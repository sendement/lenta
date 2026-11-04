//! Looking after a game's Proton side: its shader caches (ported from
//! steam-downgrader: only entries known by name, never saves or configs),
//! and resetting its prefix, which goes to the trash rather than away
//! (some games keep their saves in it).

use std::path::{Path, PathBuf};

use crate::game::Game;

pub struct Item {
    pub path: PathBuf,
    pub size: u64,
}

/// steamapps/shadercache/<appid>/… (the video transcodes there are not ours).
const STEAM_CACHES: [&str; 7] = [
    "fozpipelines",
    "DXVK_state_cache",
    "nvidiav",
    "mesa_shader_cache",
    "radv",
    "vkd3d",
    "gl_cache",
];
/// Driver and D3D caches in the prefix, under AppData/Local.
const PREFIX_CACHES: [&str; 6] = [
    "NVIDIA/DXCache",
    "NVIDIA/GLCache",
    "AMD/DxCache",
    "AMD/DxcCache",
    "AMD/VkCache",
    "D3DSCache",
];
/// What DXVK and vkd3d-proton leave next to the game's exe.
const INSTALL_SUFFIXES: [&str; 3] = [
    ".dxvk-cache",
    "vkd3d-proton.cache",
    "vkd3d-proton.cache.write",
];

pub fn size(p: &Path) -> u64 {
    let Ok(m) = p.symlink_metadata() else {
        return 0;
    };
    if !m.is_dir() {
        return m.len();
    }
    std::fs::read_dir(p)
        .map(|rd| rd.flatten().map(|e| size(&e.path())).sum())
        .unwrap_or(0)
}

fn steam_root() -> Option<PathBuf> {
    crate::sources::steam::roots().into_iter().next()
}

/// The library a Steam game is installed in (its prefix sits there).
fn library_of(g: &Game) -> Option<PathBuf> {
    // install_dir = <lib>/steamapps/common/<dir>
    g.install_dir
        .as_ref()
        .and_then(|d| d.parent()?.parent()?.parent().map(Path::to_path_buf))
}

/// The game's Wine prefix: Proton's compatdata for Steam games and
/// shortcuts, the launcher's own for the rest.
pub fn prefix(g: &Game) -> Option<PathBuf> {
    if let Some(id) = g.compat_id {
        let mut libs: Vec<PathBuf> = library_of(g).into_iter().collect();
        libs.extend(steam_root());
        return libs
            .into_iter()
            .map(|l| l.join(format!("steamapps/compatdata/{id}")))
            .find(|p| p.is_dir());
    }
    g.prefix.clone().filter(|p| p.is_dir())
}

/// Every shader cache of the game found, with sizes.
pub fn shader_caches(g: &Game) -> Vec<Item> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<Item>, p: PathBuf| {
        let s = size(&p);
        out.push(Item { path: p, size: s });
    };
    // Next to the exe (not files the game itself ships: those are in its
    // depot; the downgrader checks manifests, here only clear leftovers).
    if let Some(dir) = &g.install_dir {
        let mut stack = vec![(dir.clone(), 0)];
        while let Some((d, depth)) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if depth < 4 {
                        stack.push((p, depth + 1));
                    }
                } else if INSTALL_SUFFIXES
                    .iter()
                    .any(|s| e.file_name().to_string_lossy().ends_with(s))
                {
                    push(&mut out, p);
                }
            }
        }
    }
    if let Some(id) = g.compat_id {
        let mut libs: Vec<PathBuf> = library_of(g).into_iter().collect();
        libs.extend(steam_root());
        libs.dedup();
        for lib in libs {
            let sc = lib.join(format!("steamapps/shadercache/{id}"));
            if let Ok(rd) = std::fs::read_dir(&sc) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if STEAM_CACHES.iter().any(|c| n.starts_with(c)) {
                        push(&mut out, e.path());
                    }
                }
            }
        }
    }
    if let Some(pfx) = prefix(g) {
        let pfx = if pfx.join("pfx").is_dir() {
            pfx.join("pfx")
        } else {
            pfx
        };
        let Ok(users) = std::fs::read_dir(pfx.join("drive_c/users")) else {
            return out;
        };
        for u in users.flatten() {
            for rel in PREFIX_CACHES {
                let p = u.path().join("AppData/Local").join(rel);
                if p.exists() {
                    push(&mut out, p);
                }
            }
        }
    }
    out
}

/// Delete them; returns the bytes freed.
pub fn clear(items: &[Item]) -> u64 {
    let mut freed = 0;
    for it in items {
        let ok = if it.path.is_dir() && !it.path.is_symlink() {
            std::fs::remove_dir_all(&it.path).is_ok()
        } else {
            std::fs::remove_file(&it.path).is_ok()
        };
        if ok {
            freed += it.size;
        }
    }
    freed
}

// ------------------------------------------------------------------ trash

fn dev(p: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| m.dev())
}

/// The trash for a path, as the freedesktop spec says: the home one on the
/// same disk, else `<mount point>/.Trash-<uid>`.
fn trash_for(p: &Path) -> Option<PathBuf> {
    let home_trash = dirs::data_dir()?.join("Trash");
    let d = dev(p)?;
    if dev(&dirs::home_dir()?) == Some(d) {
        return Some(home_trash);
    }
    let mut top = p.to_path_buf();
    while let Some(parent) = top.parent() {
        if dev(parent) != Some(d) {
            break;
        }
        top = parent.to_path_buf();
    }
    let uid = unsafe { libc::getuid() };
    Some(top.join(format!(".Trash-{uid}")))
}

fn now_iso() -> String {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Move to the trash (restorable from the file manager).
pub fn trash(p: &Path) -> Result<PathBuf, String> {
    let p = p.canonicalize().map_err(|e| e.to_string())?;
    let trash = trash_for(&p).ok_or("no trash")?;
    let files = trash.join("files");
    let info = trash.join("info");
    std::fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&info).map_err(|e| e.to_string())?;
    let base = p
        .file_name()
        .ok_or("bad path")?
        .to_string_lossy()
        .into_owned();
    let mut name = base.clone();
    let mut n = 1;
    while files.join(&name).exists() || info.join(format!("{name}.trashinfo")).exists() {
        n += 1;
        name = format!("{base}.{n}");
    }
    let enc: String = p
        .to_string_lossy()
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    std::fs::write(
        info.join(format!("{name}.trashinfo")),
        format!("[Trash Info]\nPath={enc}\nDeletionDate={}\n", now_iso()),
    )
    .map_err(|e| e.to_string())?;
    let to = files.join(&name);
    if let Err(e) = std::fs::rename(&p, &to) {
        let _ = std::fs::remove_file(info.join(format!("{name}.trashinfo")));
        return Err(e.to_string());
    }
    Ok(to)
}

pub fn human(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.1} {}", b / 1e9, crate::i18n::tr("gb"))
    } else if b >= 1e6 {
        format!("{:.0} {}", b / 1e6, crate::i18n::tr("mb"))
    } else {
        format!("{:.0} {}", b / 1e3, crate::i18n::tr("kb"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder goes to the home trash with its .trashinfo, then back out.
    #[test]
    fn trash_and_back() {
        let tmp = dirs::home_dir()
            .unwrap()
            .join(format!(".cache/lenta-trash-test-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("pfx/drive_c")).unwrap();
        std::fs::write(tmp.join("pfx/drive_c/save.dat"), b"keep me").unwrap();
        let to = trash(&tmp.join("pfx")).unwrap();
        assert!(!tmp.join("pfx").exists());
        assert_eq!(
            std::fs::read(to.join("drive_c/save.dat")).unwrap(),
            b"keep me"
        );
        let name = to.file_name().unwrap().to_string_lossy().into_owned();
        let info = to
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("info")
            .join(format!("{name}.trashinfo"));
        let text = std::fs::read_to_string(&info).unwrap();
        assert!(text.contains("Path=") && text.contains("DeletionDate="));
        // Clean up the test's own entry.
        std::fs::remove_dir_all(&to).unwrap();
        std::fs::remove_file(&info).unwrap();
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
