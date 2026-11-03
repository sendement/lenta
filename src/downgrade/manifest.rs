//! Depot manifests: Steam's depotcache/<depot>_<gid>.manifest files and the
//! same thing zipped from the CDN. A run of `[magic u32][length u32][body]`
//! sections: the payload (files and their chunks), the metadata (depot,
//! gid, when the build was made, whether names are encrypted).

use std::io::Read;
use std::path::Path;

use super::crypto;
use super::pb;

const PAYLOAD: u32 = 0x71F6_17D0;
const METADATA: u32 = 0x1F48_12BE;
const END: u32 = 0x32C4_15AB;

pub const FLAG_EXECUTABLE: u32 = 32;
pub const FLAG_DIRECTORY: u32 = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// SHA-1 of the plain data; also its name on the CDN.
    pub sha: [u8; 20],
    pub offset: u64,
    pub size: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Relative path with forward slashes.
    pub name: String,
    pub size: u64,
    pub flags: u32,
    pub sha: Vec<u8>,
    /// Sorted by offset.
    pub chunks: Vec<Chunk>,
    /// A symlink's target.
    pub link: String,
}

impl File {
    pub fn is_dir(&self) -> bool {
        self.flags & FLAG_DIRECTORY != 0
    }
}

#[derive(Clone, Debug, Default)]
pub struct Manifest {
    pub depot: u32,
    pub gid: u64,
    /// When the build was made (unix time).
    pub created: u64,
    pub encrypted: bool,
    pub size: u64,
    pub files: Vec<File>,
}

fn sections(data: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 4 <= data.len() {
        let magic = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        at += 4;
        if magic == END || at + 4 > data.len() {
            break;
        }
        let len = u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        let Some(body) = data.get(at..at + len) else {
            break;
        };
        out.push((magic, body));
        at += len;
    }
    out
}

/// Parse the sections; files only when `files` (dates need just the header).
pub fn parse(data: &[u8], files: bool) -> Result<Manifest, String> {
    let secs = sections(data);
    let meta = secs
        .iter()
        .find(|(m, _)| *m == METADATA)
        .map(|(_, b)| *b)
        .ok_or("manifest: no metadata")?;
    let mut m = Manifest::default();
    for (f, v) in pb::fields(meta) {
        match f {
            1 => m.depot = v.int() as u32,
            2 => m.gid = v.int(),
            3 => m.created = v.int(),
            4 => m.encrypted = v.int() != 0,
            5 => m.size = v.int(),
            _ => {}
        }
    }
    if !files {
        return Ok(m);
    }
    let Some((_, payload)) = secs.iter().find(|(m, _)| *m == PAYLOAD) else {
        return Ok(m);
    };
    for (f, mapping) in pb::fields(payload) {
        if f != 1 {
            continue;
        }
        let mut file = File {
            name: String::new(),
            size: 0,
            flags: 0,
            sha: Vec::new(),
            chunks: Vec::new(),
            link: String::new(),
        };
        let mut raw_name: &[u8] = &[];
        let mut raw_link: &[u8] = &[];
        for (f2, v) in pb::fields(mapping.bytes()) {
            match f2 {
                1 => raw_name = v.bytes(),
                2 => file.size = v.int(),
                3 => file.flags = v.int() as u32,
                5 => file.sha = v.bytes().to_vec(),
                6 => {
                    let mut c = Chunk {
                        sha: [0; 20],
                        offset: 0,
                        size: 0,
                    };
                    for (f3, v3) in pb::fields(v.bytes()) {
                        match f3 {
                            1 => {
                                let b = v3.bytes();
                                if b.len() == 20 {
                                    c.sha.copy_from_slice(b);
                                }
                            }
                            3 => c.offset = v3.int(),
                            4 => c.size = v3.int() as u32,
                            _ => {}
                        }
                    }
                    file.chunks.push(c);
                }
                7 => raw_link = v.bytes(),
                _ => {}
            }
        }
        // Encrypted names stay base64 text until `decrypt_names`.
        file.name = clean(raw_name);
        file.link = clean(raw_link);
        file.chunks.sort_by_key(|c| c.offset);
        m.files.push(file);
    }
    Ok(m)
}

/// Names end in NUL padding and use backslashes.
fn clean(b: &[u8]) -> String {
    String::from_utf8_lossy(b)
        .trim_end_matches('\0')
        .replace('\\', "/")
}

impl Manifest {
    pub fn read(path: &Path, files: bool) -> Result<Manifest, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        parse(&data, files)
    }

    /// The CDN's answer: a zip around the same sections.
    pub fn from_cdn(zipped: &[u8]) -> Result<Manifest, String> {
        let mut z =
            zip::ZipArchive::new(std::io::Cursor::new(zipped)).map_err(|e| e.to_string())?;
        let mut f = z.by_index(0).map_err(|e| e.to_string())?;
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        parse(&data, true)
    }

    /// Names are base64 of AES with the depot key when `encrypted`.
    pub fn decrypt_names(&mut self, key: &[u8]) -> Result<(), String> {
        use base64::Engine;
        if !self.encrypted {
            return Ok(());
        }
        let dec = |s: &str| -> Result<String, String> {
            if s.is_empty() {
                return Ok(String::new());
            }
            // MIME-style: line breaks inside.
            let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            let raw = base64::engine::general_purpose::STANDARD
                .decode(compact)
                .map_err(|e| e.to_string())?;
            Ok(clean(&crypto::decrypt(&raw, key)?))
        };
        for f in &mut self.files {
            f.name = dec(&f.name)?;
            f.link = dec(&f.link)?;
        }
        self.encrypted = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every manifest in the real depotcache parses, dates and all
    /// (read only).
    #[test]
    fn real_depotcache() {
        for root in crate::sources::steam::roots() {
            let Ok(rd) = std::fs::read_dir(root.join("depotcache")) else {
                continue;
            };
            let mut n = 0;
            for e in rd.flatten().take(40) {
                let p = e.path();
                if p.extension().is_none_or(|x| x != "manifest") {
                    continue;
                }
                let m = Manifest::read(&p, true).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
                assert_eq!(stem, format!("{}_{}", m.depot, m.gid), "{}", p.display());
                assert!(m.created > 1_000_000_000);
                if !m.encrypted {
                    for f in m.files.iter().filter(|f| !f.is_dir()) {
                        let sum: u64 = f.chunks.iter().map(|c| c.size as u64).sum();
                        assert_eq!(sum, f.size, "{}: {}", p.display(), f.name);
                    }
                }
                n += 1;
            }
            eprintln!("{n} manifests read");
        }
    }
}

#[cfg(test)]
mod install_check {
    /// Every file of an install hashes to its manifest:
    /// `LENTA_CHECK=<manifest file>:<install dir> cargo test install_matches -- --ignored`.
    #[test]
    #[ignore]
    fn install_matches_manifest() {
        let spec = std::env::var("LENTA_CHECK").expect("LENTA_CHECK");
        let (m, dir) = spec.split_once(':').unwrap();
        let m = super::Manifest::read(std::path::Path::new(m), true).unwrap();
        let dir = std::path::Path::new(dir);
        let mut bad = Vec::new();
        let mut n = 0;
        for f in m.files.iter().filter(|f| !f.is_dir() && f.link.is_empty()) {
            n += 1;
            match std::fs::read(dir.join(&f.name)) {
                Ok(b) if super::super::crypto::sha1(&b).as_slice() == f.sha.as_slice() => {}
                _ => bad.push(f.name.clone()),
            }
        }
        eprintln!("{n} files, {} differ", bad.len());
        assert!(bad.is_empty(), "{:?}", &bad[..bad.len().min(5)]);
    }
}
