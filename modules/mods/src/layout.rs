//! Where a game takes its mods, and where an archive's files go in it.
//!
//! Unreal games: `.pak` (with `.ucas`/`.utoc`) in `Content/Paks/~mods`
//! (`LogicMods` for Blueprint mods), UE4SS's Lua and C++ mods in
//! `Binaries/Win64/ue4ss/Mods/<name>` (or `Win64/Mods` with the older
//! layout), UE4SS itself beside the program. Unity: BepInEx at the top,
//! its plugins in `BepInEx/plugins`. An archive laid out like the game
//! (`<Project>/…`, `Content/…`, `Binaries/…`) goes where it says.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// The game's folder (what lenta calls its install folder).
    pub root: PathBuf,
    /// An Unreal game's project folder (the one with `Content/Paks`).
    pub project: Option<PathBuf>,
    pub win64: Option<PathBuf>,
    /// Where UE4SS takes mods (UE4SS installed).
    pub ue4ss_mods: Option<PathBuf>,
    /// BepInEx installed (a Unity game).
    pub bepinex: bool,
    pub unity: bool,
}

fn ci_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl Layout {
    pub fn detect(root: &Path) -> Layout {
        let mut l = Layout {
            root: root.to_path_buf(),
            ..Default::default()
        };
        let mut cands = vec![root.to_path_buf()];
        if let Ok(rd) = std::fs::read_dir(root) {
            let mut v: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            v.sort();
            cands.extend(v.into_iter().filter(|p| {
                !p.file_name()
                    .is_some_and(|n| ci_eq(&n.to_string_lossy(), "Engine"))
            }));
        }
        l.project = cands.into_iter().find(|p| p.join("Content/Paks").is_dir());
        if let Some(p) = &l.project {
            let w = p.join("Binaries/Win64");
            if w.is_dir() {
                if w.join("ue4ss/Mods").is_dir() {
                    l.ue4ss_mods = Some(w.join("ue4ss/Mods"));
                } else if w.join("Mods").is_dir() && w.join("UE4SS.dll").is_file() {
                    l.ue4ss_mods = Some(w.join("Mods"));
                }
                l.win64 = Some(w);
            }
        }
        l.bepinex = root.join("BepInEx").is_dir();
        l.unity = std::fs::read_dir(root)
            .map(|rd| {
                rd.flatten().any(|e| {
                    e.file_name().to_string_lossy().ends_with("_Data") && e.path().is_dir()
                })
            })
            .unwrap_or(false)
            || root.join("UnityPlayer.dll").is_file();
        l
    }

    pub fn project_name(&self) -> Option<String> {
        self.project
            .as_ref()
            .filter(|p| *p != &self.root)
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
    }

    /// Relative to the root.
    pub fn rel(&self, p: &Path) -> PathBuf {
        p.strip_prefix(&self.root).unwrap_or(p).to_path_buf()
    }

    pub fn paks(&self) -> Option<PathBuf> {
        self.project.as_ref().map(|p| p.join("Content/Paks"))
    }

    /// Where UE4SS's mods go, installed or not yet (the newer layout).
    pub fn ue4ss_mods_or_new(&self) -> Option<PathBuf> {
        self.ue4ss_mods
            .clone()
            .or_else(|| self.win64.as_ref().map(|w| w.join("ue4ss/Mods")))
    }

    pub fn kind_name(&self) -> &'static str {
        if self.project.is_some() {
            "Unreal"
        } else if self.unity {
            "Unity"
        } else {
            ""
        }
    }
}

/// Every file under a folder, relative to it.
pub fn files(dir: &Path) -> Vec<PathBuf> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else if let Ok(r) = p.strip_prefix(base) {
                out.push(r.to_path_buf());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn ext(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn is_pak(p: &Path) -> bool {
    matches!(ext(p).as_str(), "pak" | "ucas" | "utoc" | "sig")
}

/// Files that come with a mod but aren't it (read-me, pictures).
fn is_extra(p: &Path) -> bool {
    let n = p
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        ext(p).as_str(),
        "md" | "url" | "jpg" | "jpeg" | "png" | "webp" | "gif" | "pdf"
    ) || n.starts_with("readme")
        || n.starts_with("read_me")
        || n == "changelog.txt"
        || n == "license"
        || n == "license.txt"
}

/// One way to put an archive in: (where from in the unpacked folder,
/// where to relative to the game's root).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub kind: String,
    pub moves: Vec<(PathBuf, PathBuf)>,
    /// Made by the manager (UE4SS mods: `enabled.txt`).
    pub made: Vec<PathBuf>,
    /// Files left out (read-me's, things it couldn't place).
    pub skipped: Vec<PathBuf>,
}

/// An archive's choices: several plans (variants in their own folders)
/// or one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choices {
    pub options: Vec<(String, Plan)>,
}

/// The folder past wrappers: one folder alone at the top, that isn't
/// itself something (a UE4SS mod, Content, the project).
fn unwrap(base: &Path, l: &Layout) -> PathBuf {
    let mut d = base.to_path_buf();
    loop {
        let entries: Vec<PathBuf> = std::fs::read_dir(&d)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        let dirs: Vec<&PathBuf> = entries.iter().filter(|p| p.is_dir()).collect();
        let files_here = entries
            .iter()
            .filter(|p| p.is_file() && !is_extra(p))
            .count();
        if dirs.len() != 1 || files_here > 0 {
            return d;
        }
        let only = dirs[0];
        let n = only
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if is_meaningful(only, &n, l) {
            return d;
        }
        d = only.clone();
    }
}

fn ue4ss_mod_dir(p: &Path) -> bool {
    p.join("Scripts/main.lua").is_file()
        || p.join("scripts/main.lua").is_file()
        || p.join("dlls/main.dll").is_file()
        || p.join("enabled.txt").is_file()
            && (p.join("Scripts").is_dir() || p.join("dlls").is_dir())
}

fn is_meaningful(p: &Path, name: &str, l: &Layout) -> bool {
    ue4ss_mod_dir(p)
        || [
            "Content",
            "Binaries",
            "BepInEx",
            "Engine",
            "ue4ss",
            "Mods",
            "LogicMods",
            "~mods",
            "Paks",
        ]
        .iter()
        .any(|m| ci_eq(m, name))
        || l.project_name().is_some_and(|pn| ci_eq(&pn, name))
}

/// How an archive unpacked in `dir` goes into the game.
pub fn plan(dir: &Path, l: &Layout, fallback_name: &str) -> Result<Choices, String> {
    let top = unwrap(dir, l);
    // Variants: several folders at the top, each a mod of its own, and
    // nothing loose beside them.
    let entries: Vec<PathBuf> = std::fs::read_dir(&top)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    let subdirs: Vec<PathBuf> = entries.iter().filter(|p| p.is_dir()).cloned().collect();
    let loose = entries
        .iter()
        .filter(|p| p.is_file() && !is_extra(p))
        .count();
    let variants = subdirs.len() >= 2
        && loose == 0
        && subdirs.iter().all(|d| {
            let n = d
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            !is_meaningful(d, &n, l) && files(d).iter().any(|f| is_pak(f))
        });
    if variants {
        let mut options = Vec::new();
        for d in subdirs {
            let n = d
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            options.push((n.clone(), plan_one(&d, l, &n)?));
        }
        options.sort_by(|a, b| a.0.cmp(&b.0));
        return Ok(Choices { options });
    }
    Ok(Choices {
        options: vec![(String::new(), plan_one(&top, l, fallback_name)?)],
    })
}

fn plan_one(top: &Path, l: &Layout, name: &str) -> Result<Plan, String> {
    let all = files(top);
    let mut p = Plan::default();
    let mut kinds: Vec<&str> = Vec::new();
    let mut taken = vec![false; all.len()];
    let pname = l.project_name();

    // 1. Laid out like the game: <Project>/…, Content/…, Binaries/…,
    //    BepInEx/… — from that folder on.
    for (i, f) in all.iter().enumerate() {
        let comps: Vec<String> = f
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        let at = comps.iter().position(|c| {
            pname.as_ref().is_some_and(|pn| ci_eq(pn, c))
                || ci_eq(c, "Content")
                || ci_eq(c, "Binaries")
                || ci_eq(c, "BepInEx")
        });
        let Some(at) = at.filter(|a| *a <= 2) else {
            continue;
        };
        let head = &comps[at];
        let rest: PathBuf = comps[at..].iter().collect();
        let dest = if ci_eq(head, "BepInEx") || pname.as_ref().is_some_and(|pn| ci_eq(pn, head)) {
            rest
        } else if let Some(pr) = &l.project {
            l.rel(pr).join(rest)
        } else {
            rest
        };
        p.moves.push((top.join(f), dest));
        taken[i] = true;
        if !kinds.contains(&"files") {
            kinds.push("files");
        }
    }

    // 2. UE4SS itself: dwmapi.dll with UE4SS beside it.
    if let Some(w) = &l.win64
        && let Some(dw) = all
            .iter()
            .enumerate()
            .find(|(i, f)| {
                !taken[*i]
                    && f.file_name()
                        .is_some_and(|n| n.eq_ignore_ascii_case("dwmapi.dll"))
            })
            .map(|(_, f)| f.clone())
        && all.iter().any(|f| {
            f.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("UE4SS.dll"))
        })
    {
        let base = dw.parent().map(Path::to_path_buf).unwrap_or_default();
        for (i, f) in all.iter().enumerate() {
            if !taken[i]
                && let Ok(r) = f.strip_prefix(&base)
            {
                p.moves.push((top.join(f), l.rel(w).join(r)));
                taken[i] = true;
            }
        }
        kinds.push("UE4SS");
    }

    // 3. UE4SS mods: folders with Scripts/main.lua or dlls/main.dll.
    if let Some(mods) = l.ue4ss_mods_or_new() {
        let mut mod_dirs: Vec<PathBuf> = Vec::new();
        for f in &all {
            let mut d = top.join(f);
            while let Some(parent) = d.parent().map(Path::to_path_buf) {
                if !parent.starts_with(top) {
                    break;
                }
                if ue4ss_mod_dir(&parent) && !mod_dirs.contains(&parent) {
                    mod_dirs.push(parent.clone());
                    break;
                }
                if parent == top {
                    break;
                }
                d = parent;
            }
        }
        for md in &mod_dirs {
            let mname = if md == top {
                name.to_string()
            } else {
                md.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            };
            let dest_dir = l.rel(&mods).join(&mname);
            for (i, f) in all.iter().enumerate() {
                if !taken[i]
                    && let Ok(r) = top.join(f).strip_prefix(md)
                {
                    p.moves.push((top.join(f), dest_dir.join(r)));
                    taken[i] = true;
                }
            }
            if !md.join("enabled.txt").is_file() {
                p.made.push(dest_dir.join("enabled.txt"));
            }
        }
        if !mod_dirs.is_empty() {
            kinds.push("UE4SS mod");
        }
    }

    // 4. Paks: into ~mods (LogicMods for Blueprint mods).
    if let Some(paks) = l.paks() {
        let mut any = false;
        for (i, f) in all.iter().enumerate() {
            if taken[i] || !is_pak(f) {
                continue;
            }
            let logic = f
                .components()
                .any(|c| ci_eq(&c.as_os_str().to_string_lossy(), "LogicMods"));
            let sub = if logic { "LogicMods" } else { "~mods" };
            let fname = f.file_name().unwrap_or_default();
            p.moves
                .push((top.join(f), l.rel(&paks).join(sub).join(fname)));
            taken[i] = true;
            any = true;
        }
        if any {
            kinds.push("pak");
        }
    }

    // 5. Unity: plugin DLLs into BepInEx/plugins/<name>.
    if l.bepinex {
        let mut any = false;
        for (i, f) in all.iter().enumerate() {
            if !taken[i] && ext(f) == "dll" {
                let fname = f.file_name().unwrap_or_default();
                p.moves.push((
                    top.join(f),
                    PathBuf::from("BepInEx/plugins").join(name).join(fname),
                ));
                taken[i] = true;
                any = true;
            }
        }
        if any {
            kinds.push("BepInEx plugin");
        }
    }

    for (i, f) in all.iter().enumerate() {
        if !taken[i] {
            p.skipped.push(f.clone());
        }
    }
    if p.moves.is_empty() {
        return Err(format!(
            "unplaced: {}",
            all.iter()
                .take(6)
                .map(|f| f.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    p.kind = kinds.join(" + ");
    Ok(p)
}

/// A mod's name and version from its archive's name, as Nexus Mods
/// names downloads: "Name-123-1-2-1700000000.zip", or the newer
/// "Name 123 1.2 2026-07-20T00-27Z hash.zip". (name, version, mod id)
pub fn parse_name(file: &str) -> (String, String, Option<u64>) {
    let stem = file
        .trim_end_matches(".zip")
        .trim_end_matches(".7z")
        .trim_end_matches(".rar")
        .trim_end_matches(".tar.gz")
        .to_string();
    // Newer: words, then a number (mod id), a version, a date.
    let words: Vec<&str> = stem.split(' ').collect();
    if let Some(di) = words
        .iter()
        .position(|w| w.len() >= 16 && w.as_bytes().get(4) == Some(&b'-') && w.contains('T'))
        && di >= 3
        && let Ok(id) = words[di - 2].parse::<u64>()
    {
        let name = words[..di - 2].join(" ");
        return (name, words[di - 1].to_string(), Some(id));
    }
    // Older: Name-ID-v-e-r-timestamp.
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 3
        && parts
            .last()
            .is_some_and(|t| t.len() >= 9 && t.chars().all(|c| c.is_ascii_digit()))
        && let Some(ii) = parts
            .iter()
            .position(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
        && ii >= 1
    {
        let name = parts[..ii].join("-");
        let ver = parts[ii + 1..parts.len() - 1].join(".");
        return (name, ver, parts[ii].parse().ok());
    }
    (stem, String::new(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(n: &str) -> PathBuf {
        let t = std::env::temp_dir().join(format!("lenta-mods-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        t
    }

    fn touch(p: PathBuf) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    fn game() -> (PathBuf, Layout) {
        let g = tmp("game");
        touch(g.join("Fox/Content/Paks/pakchunk0.pak"));
        touch(g.join("Fox/Binaries/Win64/Fox-Win64-Shipping.exe"));
        std::fs::create_dir_all(g.join("Fox/Binaries/Win64/ue4ss/Mods")).unwrap();
        touch(g.join("Fox.exe"));
        let l = Layout::detect(&g);
        (g, l)
    }

    #[test]
    fn detected() {
        let (g, l) = game();
        assert_eq!(l.project, Some(g.join("Fox")));
        assert_eq!(l.ue4ss_mods, Some(g.join("Fox/Binaries/Win64/ue4ss/Mods")));
        assert_eq!(l.kind_name(), "Unreal");
    }

    #[test]
    fn archives_placed() {
        let (_g, l) = game();
        // A pak mod wrapped in a folder, with a read-me.
        let a = tmp("pak");
        touch(a.join("HudScaling/HudScaling_P.pak"));
        touch(a.join("HudScaling/HudScaling_P.utoc"));
        touch(a.join("HudScaling/README.md"));
        let c = plan(&a, &l, "Hud").unwrap();
        assert_eq!(c.options.len(), 1);
        let p = &c.options[0].1;
        assert_eq!(p.kind, "pak");
        assert!(
            p.moves
                .iter()
                .any(|(_, d)| d == Path::new("Fox/Content/Paks/~mods/HudScaling_P.pak"))
        );
        assert_eq!(p.skipped, [PathBuf::from("README.md")]);
        // A UE4SS mod: its folder kept, enabled.txt made.
        let b = tmp("ue");
        touch(b.join("ModMenu/Scripts/main.lua"));
        let p = &plan(&b, &l, "x").unwrap().options[0].1;
        assert_eq!(p.kind, "UE4SS mod");
        assert_eq!(
            p.moves[0].1,
            Path::new("Fox/Binaries/Win64/ue4ss/Mods/ModMenu/Scripts/main.lua")
        );
        assert_eq!(
            p.made,
            [PathBuf::from(
                "Fox/Binaries/Win64/ue4ss/Mods/ModMenu/enabled.txt"
            )]
        );
        // Scripts right at the top: named after the archive.
        let c2 = tmp("ue2");
        touch(c2.join("Scripts/main.lua"));
        touch(c2.join("enabled.txt"));
        let p = &plan(&c2, &l, "Auto Loot").unwrap().options[0].1;
        assert!(
            p.moves
                .iter()
                .any(|(_, d)| d.starts_with("Fox/Binaries/Win64/ue4ss/Mods/Auto Loot"))
        );
        assert!(p.made.is_empty());
        // Variants: a folder each.
        let v = tmp("var");
        touch(v.join("Half Price/a_P.pak"));
        touch(v.join("Material Only/b_P.pak"));
        let c = plan(&v, &l, "x").unwrap();
        assert_eq!(
            c.options.iter().map(|o| o.0.as_str()).collect::<Vec<_>>(),
            ["Half Price", "Material Only"]
        );
        // Laid out like the game.
        let r = tmp("root");
        touch(r.join("Fox/Content/Paks/~mods/z_P.pak"));
        let p = &plan(&r, &l, "x").unwrap().options[0].1;
        assert_eq!(p.moves[0].1, Path::new("Fox/Content/Paks/~mods/z_P.pak"));
        // UE4SS itself.
        let u = tmp("ue4ss");
        touch(u.join("dwmapi.dll"));
        touch(u.join("ue4ss/UE4SS.dll"));
        touch(u.join("ue4ss/Mods/Keybinds/Scripts/main.lua"));
        let p = &plan(&u, &l, "x").unwrap().options[0].1;
        assert!(p.kind.starts_with("UE4SS"));
        assert!(
            p.moves
                .iter()
                .any(|(_, d)| d == Path::new("Fox/Binaries/Win64/dwmapi.dll"))
        );
        assert!(
            p.moves
                .iter()
                .any(|(_, d)| d == Path::new("Fox/Binaries/Win64/ue4ss/UE4SS.dll"))
        );
    }

    #[test]
    fn names() {
        assert_eq!(
            parse_name("AlwaysFullHealth 58 2 2026-07-20T00-27Z mh9STl6uh.zip"),
            ("AlwaysFullHealth".into(), "2".into(), Some(58))
        );
        assert_eq!(
            parse_name("Better Lantern 198 1.0 2026-09-24T19-25Z 5Xf2bPYKM.zip"),
            ("Better Lantern".into(), "1.0".into(), Some(198))
        );
        assert_eq!(
            parse_name("carefree-mode-14-1-0-1759448262.zip"),
            ("carefree-mode".into(), "1.0".into(), Some(14))
        );
        assert_eq!(
            parse_name("whatever.7z"),
            ("whatever".into(), String::new(), None)
        );
    }
}
