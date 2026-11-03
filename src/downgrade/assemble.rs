//! Build one depot version in a staging folder from as little network as
//! possible. For every file of the target manifest, in order:
//!
//! 1. unchanged: the installed file already is the target (same SHA-1 in
//!    both manifests and on disk): nothing is written, apply leaves it;
//! 2. resumed: an interrupted run already wrote this chunk here;
//! 3. reused: the chunk is somewhere in the installed build (any file, any
//!    offset) and is copied locally;
//! 4. downloaded.
//!
//! Every chunk is checked against its SHA-1 before it is written. The
//! folder layout (.sd_files.json, .complete) is steam-downgrader's.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::crypto::sha1;
use super::manifest::{Chunk, FLAG_EXECUTABLE, File};

pub const FILES_LIST: &str = ".sd_files.json";
pub const COMPLETE: &str = ".complete";

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    /// Bytes of all target files.
    pub total: u64,
    pub unchanged: u64,
    pub resumed: u64,
    pub reused: u64,
    pub downloaded: u64,
    pub to_download: u64,
}

impl Stats {
    pub fn done(&self) -> u64 {
        self.unchanged + self.resumed + self.reused + self.downloaded
    }
}

pub struct Assembler {
    pub files: Vec<File>,
    out: PathBuf,
    base_dir: Option<PathBuf>,
    base_by_name: HashMap<String, File>,
    /// chunk sha → (file, offset, size) in the installed build.
    base_chunks: HashMap<[u8; 20], (String, u64, u32)>,
    pub stats: Stats,
    pub unchanged: HashSet<String>,
}

fn read_at(path: &Path, offset: u64, size: u32) -> Option<Vec<u8>> {
    let f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; size as usize];
    f.read_exact_at(&mut buf, offset).ok()?;
    Some(buf)
}

impl Assembler {
    pub fn new(
        files: Vec<File>,
        out: &Path,
        base: Vec<File>,
        base_dir: Option<&Path>,
    ) -> Assembler {
        let total = files.iter().filter(|f| !f.is_dir()).map(|f| f.size).sum();
        let base_by_name: HashMap<String, File> = base
            .into_iter()
            .filter(|f| !f.is_dir())
            .map(|f| (f.name.to_lowercase(), f))
            .collect();
        let mut base_chunks = HashMap::new();
        for f in base_by_name.values() {
            for c in &f.chunks {
                base_chunks
                    .entry(c.sha)
                    .or_insert_with(|| (f.name.clone(), c.offset, c.size));
            }
        }
        Assembler {
            files,
            out: out.to_path_buf(),
            base_dir: base_dir.map(Path::to_path_buf),
            base_by_name,
            base_chunks,
            stats: Stats {
                total,
                ..Default::default()
            },
            unchanged: HashSet::new(),
        }
    }

    fn installed_matches(&mut self, f: &File, progress: &mut dyn FnMut(&Stats)) -> bool {
        use sha1::{Digest, Sha1};
        use std::io::Read;
        let Some(base_dir) = &self.base_dir else {
            return false;
        };
        let Some(b) = self.base_by_name.get(&f.name.to_lowercase()) else {
            return false;
        };
        if b.sha != f.sha || b.size != f.size {
            return false;
        }
        let p = base_dir.join(&f.name);
        if std::fs::metadata(&p).map(|m| m.len()).ok() != Some(f.size) {
            return false;
        }
        let Ok(mut fh) = std::fs::File::open(&p) else {
            return false;
        };
        let mut h = Sha1::new();
        let mut buf = vec![0u8; 8 << 20];
        let before = self.stats.unchanged;
        loop {
            match fh.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    h.update(&buf[..n]);
                    self.stats.unchanged += n as u64;
                    progress(&self.stats);
                }
                Err(_) => {
                    self.stats.unchanged = before;
                    return false;
                }
            }
        }
        if h.finalize().as_slice() != f.sha.as_slice() {
            self.stats.unchanged = before;
            return false;
        }
        true
    }

    /// Everything that needs no network; returns the chunks to fetch.
    pub fn plan(
        &mut self,
        progress: &mut dyn FnMut(&Stats),
    ) -> Result<Vec<(usize, Chunk)>, String> {
        std::fs::create_dir_all(&self.out).map_err(|e| e.to_string())?;
        let mut download = Vec::new();
        for i in 0..self.files.len() {
            let f = self.files[i].clone();
            if f.is_dir() || !f.link.is_empty() {
                continue;
            }
            if self.installed_matches(&f, progress) {
                self.unchanged.insert(f.name.clone());
                continue;
            }
            let out = self.out.join(&f.name);
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let existed = std::fs::metadata(&out).is_ok_and(|m| m.len() == f.size);
            let fh = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&out)
                .map_err(|e| format!("{}: {e}", out.display()))?;
            if !existed {
                fh.set_len(f.size).map_err(|e| e.to_string())?;
            }
            for c in &f.chunks {
                if existed {
                    let mut buf = vec![0u8; c.size as usize];
                    if fh.read_exact_at(&mut buf, c.offset).is_ok() && sha1(&buf) == c.sha {
                        self.stats.resumed += c.size as u64;
                        continue;
                    }
                }
                if let (Some((src, off, size)), Some(base_dir)) =
                    (self.base_chunks.get(&c.sha), &self.base_dir)
                    && let Some(data) = read_at(&base_dir.join(src), *off, *size)
                    && sha1(&data) == c.sha
                {
                    fh.write_all_at(&data, c.offset)
                        .map_err(|e| e.to_string())?;
                    self.stats.reused += c.size as u64;
                    progress(&self.stats);
                    continue;
                }
                download.push((i, c.clone()));
                self.stats.to_download += c.size as u64;
            }
            progress(&self.stats);
        }
        Ok(download)
    }

    /// Write a fetched chunk (checked).
    pub fn store(out: &Path, f: &File, c: &Chunk, data: &[u8]) -> Result<(), String> {
        if sha1(data) != c.sha {
            return Err(format!("{}: damaged chunk", f.name));
        }
        let fh = OpenOptions::new()
            .write(true)
            .open(out.join(&f.name))
            .map_err(|e| e.to_string())?;
        fh.write_all_at(data, c.offset).map_err(|e| e.to_string())
    }

    pub fn out(&self) -> &Path {
        &self.out
    }

    /// Directories, links, modes; then the files list and the done mark.
    pub fn finish(&self, manifest: &str, base: &str) -> Result<(), String> {
        let mut entries = Vec::new();
        for f in &self.files {
            let p = self.out.join(&f.name);
            if f.is_dir() {
                std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
                continue;
            }
            if !f.link.is_empty() {
                if let Some(d) = p.parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                if p.symlink_metadata().is_err() {
                    std::os::unix::fs::symlink(&f.link, &p).map_err(|e| e.to_string())?;
                }
                continue;
            }
            let unchanged = self.unchanged.contains(&f.name);
            if !unchanged {
                let len = std::fs::metadata(&p).map_err(|e| e.to_string())?.len();
                if len != f.size {
                    return Err(format!("{}: size {len} instead of {}", f.name, f.size));
                }
                if f.flags & FLAG_EXECUTABLE != 0 {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
                }
            }
            entries.push(json!([f.name, f.size, unchanged]));
        }
        let list = json!({"manifest": manifest, "base_manifest": base, "files": entries});
        std::fs::write(self.out.join(FILES_LIST), list.to_string()).map_err(|e| e.to_string())?;
        std::fs::write(self.out.join(COMPLETE), b"").map_err(|e| e.to_string())
    }
}

#[allow(clippy::type_complexity)]
/// The files list of a finished staging folder: (manifest, base, [(name,
/// size, unchanged)]).
pub fn read_files_list(dir: &Path) -> Option<(String, String, Vec<(String, u64, bool)>)> {
    let v: Value = serde_json::from_slice(&std::fs::read(dir.join(FILES_LIST)).ok()?).ok()?;
    let files = v
        .get("files")?
        .as_array()?
        .iter()
        .filter_map(|e| {
            Some((
                e.get(0)?.as_str()?.to_string(),
                e.get(1)?.as_u64()?,
                e.get(2)?.as_bool()?,
            ))
        })
        .collect();
    Some((
        v.get("manifest")?.as_str()?.to_string(),
        v.get("base_manifest")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        files,
    ))
}
