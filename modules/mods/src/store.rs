//! What's installed for each game, and putting it into the game's folder.
//!
//! A mod's files are kept in the module's folder (`games/<game>/store/<id>`,
//! laid out as they go into the game) and put into the game as hard links
//! (copies across drives). The game's own files a mod replaces are kept in
//! `backup/` and put back when no enabled mod covers them any more, so
//! turning everything off leaves the game as it was.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Nexus {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rec {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub enabled: bool,
    /// Relative to the game's root.
    pub files: Vec<String>,
    pub archive: String,
    pub nexus: Option<Nexus>,
    /// A Nexus mod id from the archive's name (no domain known).
    pub nexus_id: Option<u64>,
    pub installed: u64,
    /// The newest version Nexus knows, when it's newer.
    pub update: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Game {
    pub key: String,
    pub root: PathBuf,
    pub mods: Vec<Rec>,
    /// The Nexus Mods game ("echoesofaincrad"), once known.
    pub domain: String,
    /// Folders the mods made in the game (taken away when empty again;
    /// the game's own are left).
    pub made_dirs: Vec<String>,
}

/// Lenta's dry run: the game's files left alone. (Tests work in their own
/// folders, so not for them.)
#[cfg(not(test))]
pub fn dry() -> bool {
    std::env::var_os("LENTA_DRY_RUN").is_some()
}

#[cfg(test)]
pub fn dry() -> bool {
    false
}

#[cfg(test)]
pub fn data() -> PathBuf {
    std::env::temp_dir().join(format!("lenta-mods-data-{}", std::process::id()))
}

#[cfg(not(test))]
pub fn data() -> PathBuf {
    std::env::var_os("LENTA_MODULE_DATA")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("lenta/module-data/mods")))
        .unwrap_or_else(|| "/tmp/lenta-mods".into())
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A key as a folder name.
pub fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn game_dir(key: &str) -> PathBuf {
    data().join("games").join(slug(key))
}

impl Game {
    pub fn load(key: &str, root: &Path) -> Game {
        let mut g: Game = std::fs::read_to_string(game_dir(key).join("mods.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        g.key = key.to_string();
        g.root = root.to_path_buf();
        g
    }

    pub fn save(&self) -> Result<(), String> {
        let d = game_dir(&self.key);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        let tmp = d.join("mods.json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_string_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, d.join("mods.json")).map_err(|e| e.to_string())
    }

    pub fn store(&self, id: &str) -> PathBuf {
        game_dir(&self.key).join("store").join(id)
    }

    fn backup(&self) -> PathBuf {
        game_dir(&self.key).join("backup")
    }

    pub fn get(&self, id: &str) -> Option<&Rec> {
        self.mods.iter().find(|m| m.id == id)
    }

    pub fn new_id(&self, name: &str) -> String {
        let base: String = slug(name).chars().take(40).collect();
        let base = if base.is_empty() { "mod".into() } else { base };
        let mut id = base.clone();
        let mut n = 2;
        while self.mods.iter().any(|m| m.id == id) || self.store(&id).exists() {
            id = format!("{base}-{n}");
            n += 1;
        }
        id
    }

    /// Files more than one enabled mod puts in: file → mods (in order).
    pub fn conflicts(&self) -> BTreeMap<String, Vec<String>> {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for r in self.mods.iter().filter(|r| r.enabled) {
            for f in &r.files {
                m.entry(f.clone()).or_default().push(r.id.clone());
            }
        }
        m.retain(|_, v| v.len() > 1);
        m
    }

    /// Who puts this file in now: the last enabled mod with it.
    fn owner(&self, rel: &str) -> Option<&Rec> {
        self.mods
            .iter()
            .rev()
            .find(|r| r.enabled && r.files.iter().any(|f| f == rel))
    }

    /// Put the file the way it should be: the owning mod's, else the
    /// game's own back (or nothing).
    fn settle(&mut self, rel: &str) -> Result<(), String> {
        let dst = self.root.join(rel);
        let ours = |g: &Game, p: &Path| -> bool {
            g.mods
                .iter()
                .any(|r| same_file(&g.store(&r.id).join(rel), p))
        };
        match self.owner(rel).map(|r| r.id.clone()) {
            Some(owner) => {
                let src = self.store(&owner).join(rel);
                if same_file(&src, &dst) {
                    return Ok(());
                }
                if dst.exists() && !ours(self, &dst) {
                    // The game's own: kept aside.
                    let b = self.backup().join(rel);
                    if !b.exists() {
                        mkparent(&b)?;
                        std::fs::rename(&dst, &b)
                            .or_else(|_| std::fs::copy(&dst, &b).map(|_| ()))
                            .map_err(|e| format!("{}: {e}", dst.display()))?;
                    }
                }
                if dst.exists() || dst.is_symlink() {
                    std::fs::remove_file(&dst).map_err(|e| format!("{}: {e}", dst.display()))?;
                }
                // Note the folders that aren't there yet.
                let mut d = dst.parent().map(Path::to_path_buf);
                while let Some(p) = d {
                    if p.exists() || !p.starts_with(&self.root) || p == self.root {
                        break;
                    }
                    let r = p
                        .strip_prefix(&self.root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .to_string();
                    if !self.made_dirs.contains(&r) {
                        self.made_dirs.push(r);
                    }
                    d = p.parent().map(Path::to_path_buf);
                }
                mkparent(&dst)?;
                std::fs::hard_link(&src, &dst)
                    .or_else(|_| std::fs::copy(&src, &dst).map(|_| ()))
                    .map_err(|e| format!("{}: {e}", dst.display()))
            }
            None => {
                if dst.exists() && ours(self, &dst) {
                    std::fs::remove_file(&dst).map_err(|e| format!("{}: {e}", dst.display()))?;
                }
                let b = self.backup().join(rel);
                if b.exists() {
                    mkparent(&dst)?;
                    std::fs::rename(&b, &dst)
                        .or_else(|_| std::fs::copy(&b, &dst).map(|_| ()))
                        .map_err(|e| format!("{}: {e}", dst.display()))?;
                    let _ = std::fs::remove_file(&b);
                }
                // Folders the mods made, left empty.
                let mut d = dst.parent().map(Path::to_path_buf);
                while let Some(p) = d {
                    let r = p
                        .strip_prefix(&self.root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .to_string();
                    if p == self.root
                        || !p.starts_with(&self.root)
                        || !self.made_dirs.contains(&r)
                        || std::fs::remove_dir(&p).is_err()
                    {
                        break;
                    }
                    self.made_dirs.retain(|x| *x != r);
                    d = p.parent().map(Path::to_path_buf);
                }
                Ok(())
            }
        }
    }

    /// Turn a mod on or off in the game.
    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<(), String> {
        let files = self.get(id).ok_or("no such mod")?.files.clone();
        if let Some(r) = self.mods.iter_mut().find(|m| m.id == id) {
            r.enabled = on;
        }
        if !dry() {
            for f in &files {
                self.settle(f)?;
            }
        }
        self.save()
    }

    /// Take it away: off, then its files gone.
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        self.set_enabled(id, false)?;
        self.mods.retain(|m| m.id != id);
        if !dry() {
            let _ = std::fs::remove_dir_all(self.store(id));
        }
        self.save()
    }

    /// Take in a mod whose files are already in its store folder.
    pub fn add(&mut self, rec: Rec) -> Result<(), String> {
        let id = rec.id.clone();
        let on = rec.enabled;
        self.mods.push(Rec {
            enabled: false,
            ..rec
        });
        self.set_enabled(&id, on)
    }
}

fn mkparent(p: &Path) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    Ok(())
}

/// The same file (a hard link of it), by device and inode.
pub fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

// ------------------------------------------------------------ found mods

/// UE4SS's own mods and helpers: not shown as mods.
const UE4SS_OWN: [&str; 12] = [
    "shared",
    "BPModLoaderMod",
    "BPML_GenericFunctions",
    "Keybinds",
    "ConsoleCommandsMod",
    "ConsoleEnablerMod",
    "LineTraceMod",
    "SplitScreenMod",
    "CheatManagerEnablerMod",
    "ActorDumperMod",
    "jsbLuaProfilerMod",
    "CustomGameConfigs",
];

/// A mod in the game not put there by the manager.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// "ue:<name>" or "pak:<stem>".
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub enabled: bool,
}

fn disabled_dir(key: &str) -> PathBuf {
    game_dir(key).join("disabled")
}

/// A pak's set: "Mod_P" for Mod_P.pak/.ucas/.utoc.
fn pak_stem(p: &Path) -> Option<String> {
    let e = p.extension()?.to_string_lossy().to_lowercase();
    matches!(e.as_str(), "pak" | "ucas" | "utoc" | "sig")
        .then(|| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .flatten()
}

impl Game {
    fn managed(&self, rel: &Path) -> bool {
        let r = rel.to_string_lossy();
        self.mods.iter().any(|m| {
            m.files
                .iter()
                .any(|f| *f == r || f.starts_with(&format!("{r}/")))
        })
    }

    /// Mods in the game the manager didn't put there: UE4SS mods (on with
    /// an enabled.txt) and pak sets (off: kept aside in the module's
    /// folder).
    pub fn found(&self, l: &crate::layout::Layout) -> Vec<Found> {
        let mut out = Vec::new();
        if let Some(m) = &l.ue4ss_mods
            && let Ok(rd) = std::fs::read_dir(m)
        {
            let mut v: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            v.sort();
            for d in v {
                let name = d
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if UE4SS_OWN.iter().any(|o| o.eq_ignore_ascii_case(&name))
                    || self.managed(&l.rel(&d))
                {
                    continue;
                }
                out.push(Found {
                    id: format!("ue:{name}"),
                    enabled: d.join("enabled.txt").is_file(),
                    name,
                    kind: "UE4SS mod",
                });
            }
        }
        if let Some(paks) = l.paks() {
            for (sub, on) in [("~mods", true), ("LogicMods", true)] {
                let mut stems: Vec<String> = Vec::new();
                for base in [
                    paks.join(sub),
                    disabled_dir(&self.key).join(l.rel(&paks)).join(sub),
                ] {
                    let Ok(rd) = std::fs::read_dir(&base) else {
                        continue;
                    };
                    for e in rd.flatten() {
                        let p = e.path();
                        if p.is_file()
                            && !self.managed(&l.rel(&p))
                            && let Some(s) = pak_stem(&p)
                            && !stems.contains(&s)
                        {
                            stems.push(s);
                        }
                    }
                }
                stems.sort();
                for s in stems {
                    let here = std::fs::read_dir(paks.join(sub))
                        .map(|rd| {
                            rd.flatten()
                                .any(|e| pak_stem(&e.path()).as_deref() == Some(&s))
                        })
                        .unwrap_or(false);
                    out.push(Found {
                        id: format!("pak:{sub}/{s}"),
                        name: s,
                        kind: if sub == "LogicMods" {
                            "LogicMods pak"
                        } else {
                            "pak"
                        },
                        enabled: here && on,
                    });
                }
            }
        }
        out
    }

    /// Turn one of those on or off.
    pub fn set_found(&self, l: &crate::layout::Layout, id: &str, on: bool) -> Result<(), String> {
        if dry() {
            return Ok(());
        }
        if let Some(name) = id.strip_prefix("ue:") {
            let d = l.ue4ss_mods.as_ref().ok_or("no UE4SS")?.join(name);
            let f = d.join("enabled.txt");
            return if on {
                std::fs::write(&f, b"").map_err(|e| e.to_string())
            } else if f.exists() {
                std::fs::remove_file(&f).map_err(|e| e.to_string())
            } else {
                Ok(())
            };
        }
        if let Some(rest) = id.strip_prefix("pak:") {
            let (sub, stem) = rest.split_once('/').ok_or("bad id")?;
            let paks = l.paks().ok_or("no paks")?;
            let live = paks.join(sub);
            let aside = disabled_dir(&self.key).join(l.rel(&paks)).join(sub);
            let (from, to) = if on { (&aside, &live) } else { (&live, &aside) };
            std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
            for e in std::fs::read_dir(from)
                .map_err(|e| e.to_string())?
                .flatten()
            {
                let p = e.path();
                if pak_stem(&p).as_deref() == Some(stem) {
                    let dst = to.join(e.file_name());
                    std::fs::rename(&p, &dst)
                        .or_else(|_| std::fs::copy(&p, &dst).and_then(|_| std::fs::remove_file(&p)))
                        .map_err(|e| e.to_string())?;
                }
            }
            return Ok(());
        }
        Err("bad id".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;

    fn tmp(n: &str) -> PathBuf {
        let t = std::env::temp_dir().join(format!("lenta-mstore-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(&t).unwrap();
        t
    }

    /// No process env in tests: the data folder is set per game here.
    fn game(name: &str) -> (Game, PathBuf) {
        let root = tmp(name);
        std::fs::create_dir_all(root.join("Fox/Content/Paks/~mods")).unwrap();
        std::fs::write(root.join("Fox/Content/Paks/base.pak"), b"orig").unwrap();
        let g = Game {
            key: format!("test:{name}-{}", std::process::id()),
            root: root.clone(),
            ..Default::default()
        };
        (g, root)
    }

    fn put(g: &Game, id: &str, rel: &str, body: &[u8]) {
        let p = g.store(id).join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn on_off_conflicts_and_back() {
        let (mut g, root) = game("deploy");
        put(&g, "a", "Fox/Content/Paks/~mods/a_P.pak", b"A");
        put(&g, "a", "Fox/Content/Paks/base.pak", b"A-base");
        put(&g, "b", "Fox/Content/Paks/base.pak", b"B-base");
        let rec = |id: &str, files: &[&str]| Rec {
            id: id.into(),
            name: id.into(),
            enabled: true,
            files: files.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        g.add(rec(
            "a",
            &[
                "Fox/Content/Paks/~mods/a_P.pak",
                "Fox/Content/Paks/base.pak",
            ],
        ))
        .unwrap();
        let base = root.join("Fox/Content/Paks/base.pak");
        assert_eq!(std::fs::read(&base).unwrap(), b"A-base");
        assert!(
            same_file(&g.store("a").join("Fox/Content/Paks/base.pak"), &base),
            "a link"
        );
        g.add(rec("b", &["Fox/Content/Paks/base.pak"])).unwrap();
        assert_eq!(std::fs::read(&base).unwrap(), b"B-base", "the later wins");
        assert_eq!(g.conflicts().len(), 1);
        g.set_enabled("b", false).unwrap();
        assert_eq!(std::fs::read(&base).unwrap(), b"A-base");
        g.remove("a").unwrap();
        assert_eq!(
            std::fs::read(&base).unwrap(),
            b"orig",
            "the game's own back"
        );
        assert!(!root.join("Fox/Content/Paks/~mods/a_P.pak").exists());
        assert!(
            root.join("Fox/Content/Paks/~mods").is_dir(),
            "the game's folder stays"
        );
        // A folder a mod made goes with it.
        put(&g, "c", "Fox/Content/Paks/LogicMods/c_P.pak", b"C");
        g.add(rec("c", &["Fox/Content/Paks/LogicMods/c_P.pak"]))
            .unwrap();
        assert!(root.join("Fox/Content/Paks/LogicMods/c_P.pak").exists());
        g.remove("c").unwrap();
        assert!(!root.join("Fox/Content/Paks/LogicMods").exists());
        g.remove("b").unwrap();
        let _ = std::fs::remove_dir_all(game_dir(&g.key));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn found_toggled() {
        let (g, root) = game("found");
        let mods = root.join("Fox/Binaries/Win64/ue4ss/Mods");
        std::fs::create_dir_all(mods.join("ModMenu/Scripts")).unwrap();
        std::fs::write(mods.join("ModMenu/enabled.txt"), b"").unwrap();
        std::fs::create_dir_all(mods.join("Keybinds/Scripts")).unwrap();
        std::fs::write(root.join("Fox/Content/Paks/~mods/Hud_P.pak"), b"x").unwrap();
        std::fs::write(root.join("Fox/Content/Paks/~mods/Hud_P.utoc"), b"x").unwrap();
        let l = Layout::detect(&root);
        let f = g.found(&l);
        assert_eq!(
            f.iter()
                .map(|x| (x.id.as_str(), x.enabled))
                .collect::<Vec<_>>(),
            [("ue:ModMenu", true), ("pak:~mods/Hud_P", true)]
        );
        g.set_found(&l, "ue:ModMenu", false).unwrap();
        g.set_found(&l, "pak:~mods/Hud_P", false).unwrap();
        assert!(!mods.join("ModMenu/enabled.txt").exists());
        assert!(!root.join("Fox/Content/Paks/~mods/Hud_P.pak").exists());
        let f = g.found(&l);
        assert!(f.iter().all(|x| !x.enabled), "{f:?}");
        g.set_found(&l, "pak:~mods/Hud_P", true).unwrap();
        assert!(root.join("Fox/Content/Paks/~mods/Hud_P.utoc").exists());
        let _ = std::fs::remove_dir_all(game_dir(&g.key));
        let _ = std::fs::remove_dir_all(root);
    }
}
