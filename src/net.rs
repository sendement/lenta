//! Downloads, each kept in ~/.cache/lenta so it's fetched once.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent(concat!("lenta/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    })
}

pub fn get(url: &str) -> Result<Vec<u8>, String> {
    if std::env::var_os("LENTA_OFFLINE").is_some() {
        return Err("offline".into());
    }
    let mut resp = agent().get(url).call().map_err(|e| e.to_string())?;
    resp.body_mut()
        .with_config()
        .limit(48 << 20)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("LENTA_CACHE")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|c| c.join("lenta")))
        .unwrap_or_else(|| "/tmp/lenta".into())
}

/// FNV-1a: a stable name for a cached address.
pub fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

const MISS_AGAIN: Duration = Duration::from_secs(7 * 86400);

/// The address's file in the cache, downloading it the first time. A
/// missing file (404) is remembered for a week.
pub fn cached(url: &str, sub: &str) -> Result<PathBuf, String> {
    let dir = cache_dir().join(sub);
    let ext = url
        .rsplit('/')
        .next()
        .and_then(|n| n.split('?').next())
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, x)| x)
        .filter(|x| x.len() <= 4)
        .unwrap_or("bin");
    let file = dir.join(format!("{:016x}.{ext}", hash(url)));
    if file.is_file() {
        return Ok(file);
    }
    let miss = file.with_extension("missing");
    if let Ok(m) = std::fs::metadata(&miss)
        && m.modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < MISS_AGAIN)
    {
        return Err("missing".into());
    }
    let _ = std::fs::create_dir_all(&dir);
    match get(url) {
        Ok(bytes) => {
            let tmp = file.with_extension("part");
            std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &file).map_err(|e| e.to_string())?;
            Ok(file)
        }
        Err(e) => {
            if e.contains("404") || e.contains("403") {
                let _ = std::fs::write(&miss, b"");
            }
            Err(e)
        }
    }
}
