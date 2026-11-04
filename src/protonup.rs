//! Custom Proton builds (GE-Proton, Proton-CachyOS) from their GitHub
//! releases into Steam's compatibilitytools.d, like ProtonUp-Qt: listed,
//! downloaded with progress, checked against the release's SHA-512,
//! unpacked; and removed. The GitHub API allows 60 calls an hour without a
//! token, so the lists are kept an hour and the releases' Atom feed stands
//! in when the API refuses.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flavour {
    Ge,
    CachyOs,
}

impl Flavour {
    pub const ALL: [Flavour; 2] = [Flavour::Ge, Flavour::CachyOs];

    pub fn name(self) -> &'static str {
        match self {
            Flavour::Ge => "GE-Proton",
            Flavour::CachyOs => "Proton-CachyOS",
        }
    }

    fn repo(self) -> &'static str {
        match self {
            Flavour::Ge => "GloriousEggroll/proton-ge-custom",
            Flavour::CachyOs => "CachyOS/proton-cachyos",
        }
    }

    /// The x86-64 archive of a release: the API's asset name, else a guess
    /// from the tag (the Atom feed has no asset list).
    fn guess(self, tag: &str) -> Vec<String> {
        match self {
            Flavour::Ge => vec![format!("{tag}-x86_64.tar.gz"), format!("{tag}.tar.gz")],
            Flavour::CachyOs => vec![format!("proton-{tag}-x86_64.tar.xz")],
        }
    }

    fn wants(self, asset: &str) -> bool {
        match self {
            Flavour::Ge => {
                (asset.ends_with(".tar.gz") || asset.ends_with(".tar.xz"))
                    && !asset.contains("aarch64")
            }
            // The plain x86-64 build (v3 needs a newer CPU, arm64 isn't ours).
            Flavour::CachyOs => asset.ends_with("-x86_64.tar.xz"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub flavour: Flavour,
    pub tag: String,
    /// Unix time.
    pub published: i64,
    /// Archive URL candidates (one when the API answered).
    pub urls: Vec<String>,
    pub size: u64,
}

fn cache_file(f: Flavour) -> PathBuf {
    crate::net::cache_dir()
        .join("protonup")
        .join(format!("{f:?}.json"))
}

/// The newest releases of a flavour (from the cache if under an hour old).
pub fn releases(f: Flavour) -> Result<Vec<Release>, String> {
    let file = cache_file(f);
    let fresh = std::fs::metadata(&file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|a| a < Duration::from_secs(3600));
    if fresh && let Some(r) = std::fs::read(&file).ok().and_then(|b| parse_api(f, &b)) {
        return Ok(r);
    }
    let api = format!(
        "https://api.github.com/repos/{}/releases?per_page=8",
        f.repo()
    );
    if let Ok(b) = crate::net::get(&api)
        && let Some(r) = parse_api(f, &b)
    {
        let _ = std::fs::create_dir_all(file.parent().unwrap_or(&file));
        let _ = std::fs::write(&file, &b);
        return Ok(r);
    }
    // Rate-limited (or offline): the feed, else what was kept.
    if let Ok(b) = crate::net::get(&format!("https://github.com/{}/releases.atom", f.repo()))
        && let Some(r) = parse_atom(f, &String::from_utf8_lossy(&b))
    {
        return Ok(r);
    }
    std::fs::read(&file)
        .ok()
        .and_then(|b| parse_api(f, &b))
        .ok_or_else(|| format!("{}: no release list", f.name()))
}

fn parse_api(f: Flavour, b: &[u8]) -> Option<Vec<Release>> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let list = v.as_array()?;
    Some(
        list.iter()
            .filter(|r| r.get("prerelease").and_then(Value::as_bool) != Some(true))
            .filter_map(|r| {
                let tag = r.get("tag_name")?.as_str()?.to_string();
                let asset = r.get("assets")?.as_array()?.iter().find(|a| {
                    a.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|n| f.wants(n))
                })?;
                Some(Release {
                    flavour: f,
                    published: r
                        .get("published_at")
                        .and_then(Value::as_str)
                        .and_then(crate::game::parse_iso)
                        .unwrap_or(0) as i64,
                    urls: vec![asset.get("browser_download_url")?.as_str()?.to_string()],
                    size: asset.get("size").and_then(Value::as_u64).unwrap_or(0),
                    tag,
                })
            })
            .collect(),
    )
}

fn parse_atom(f: Flavour, xml: &str) -> Option<Vec<Release>> {
    let mut out = Vec::new();
    for entry in xml.split("<entry>").skip(1) {
        let link = entry.split("href=\"").nth(1)?.split('"').next()?;
        let tag = link.rsplit('/').next()?.to_string();
        let updated = entry
            .split("<updated>")
            .nth(1)
            .and_then(|s| s.split('<').next())
            .and_then(crate::game::parse_iso)
            .unwrap_or(0) as i64;
        let urls = f
            .guess(&tag)
            .into_iter()
            .map(|a| {
                format!(
                    "https://github.com/{}/releases/download/{tag}/{a}",
                    f.repo()
                )
            })
            .collect();
        out.push(Release {
            flavour: f,
            tag,
            published: updated,
            urls,
            size: 0,
        });
    }
    (!out.is_empty()).then_some(out)
}

// ----------------------------------------------------------------- install

/// Where custom tools go: the Steam install's compatibilitytools.d.
pub fn tools_dir() -> Option<PathBuf> {
    crate::sources::steam::roots()
        .into_iter()
        .next()
        .map(|r| r.join("compatibilitytools.d"))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    /// Unpacking (after the download).
    pub unpacking: bool,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        // Only the connection and the server's answer are timed: the body
        // is half a gigabyte (a stuck download is cancelled by hand).
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent("lenta")
        .build()
        .into()
}

/// Download, check, unpack. Returns the folder made.
pub fn install(
    r: &Release,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<PathBuf, String> {
    let dest = tools_dir().ok_or("Steam not found")?;
    install_into(
        r,
        &dest,
        &crate::net::cache_dir().join("protonup"),
        cancel,
        progress,
    )
}

/// The same into `dest`, downloading through `tmp_dir`.
fn install_into(
    r: &Release,
    dest: &Path,
    tmp_dir: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(tmp_dir).map_err(|e| e.to_string())?;
    let agent = agent();
    // The first URL that answers.
    let (url, mut resp) = r
        .urls
        .iter()
        .find_map(|u| {
            let resp = agent.get(u).call().ok()?;
            Some((u.clone(), resp))
        })
        .ok_or("the archive could not be fetched")?;
    let name = url.rsplit('/').next().unwrap_or("proton.tar").to_string();
    let archive = tmp_dir.join(&name);
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(r.size);
    let result = (|| {
        let mut body = resp.body_mut().with_config().limit(4 << 30).reader();
        let mut out = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
        let mut hasher = <sha2::Sha512 as sha2::Digest>::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut done = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(crate::i18n::tr("sd_cancelled").to_string());
            }
            let n = body.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            sha2::Digest::update(&mut hasher, &buf[..n]);
            out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            progress(Progress {
                done,
                total,
                unpacking: false,
            });
        }
        let sum: String = sha2::Digest::finalize(hasher)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        // The release's checksum file sits next to the archive.
        let sum_url = format!(
            "{}.sha512sum",
            url.trim_end_matches(".tar.gz").trim_end_matches(".tar.xz")
        );
        if let Ok(b) = crate::net::get(&sum_url) {
            let want = String::from_utf8_lossy(&b);
            let want = want
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_lowercase();
            if !want.is_empty() && want != sum {
                return Err("checksum mismatch: the download is damaged".into());
            }
        }
        progress(Progress {
            done,
            total,
            unpacking: true,
        });
        // Unpack beside, then move in: a half-unpacked tool never shows up.
        let stage = dest.join(format!(".lenta-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&stage);
        std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
        let st = std::process::Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&stage)
            .status()
            .map_err(|e| format!("tar: {e}"))?;
        if !st.success() {
            let _ = std::fs::remove_dir_all(&stage);
            return Err("the archive could not be unpacked".into());
        }
        let top = std::fs::read_dir(&stage)
            .map_err(|e| e.to_string())?
            .flatten()
            .find(|e| e.path().is_dir())
            .ok_or("the archive holds no folder")?
            .path();
        let target = dest.join(top.file_name().ok_or("bad folder")?);
        if target.exists() {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(format!("{} is already installed", target.display()));
        }
        std::fs::rename(&top, &target).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&stage);
        Ok(target)
    })();
    let _ = std::fs::remove_file(&archive);
    result
}

/// Remove a custom tool (only ever a folder inside compatibilitytools.d).
pub fn remove(dir: &Path) -> Result<(), String> {
    remove_in(&tools_dir().ok_or("Steam not found")?, dir)
}

fn remove_in(base: &Path, dir: &Path) -> Result<(), String> {
    let base = base.canonicalize().map_err(|e| e.to_string())?;
    let d = dir.canonicalize().map_err(|e| e.to_string())?;
    if d.parent() != Some(base.as_path()) {
        return Err("not one of Steam's custom tools".into());
    }
    std::fs::remove_dir_all(&d).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atom_feed() {
        let xml = r#"<feed><entry><updated>2026-11-03T07:19:38Z</updated><link rel="alternate" type="text/html" href="https://github.com/GloriousEggroll/proton-ge-custom/releases/tag/GE-Proton11-12"/></entry></feed>"#;
        let r = parse_atom(Flavour::Ge, xml).unwrap();
        assert_eq!(r[0].tag, "GE-Proton11-12");
        assert!(r[0].urls[0].ends_with("/download/GE-Proton11-12/GE-Proton11-12-x86_64.tar.gz"));
        assert!(r[0].published > 1_700_000_000);
    }

    /// The whole install on a tiny archive from a local server, into a
    /// fake Steam: download, SHA-512 check, unpack, then remove; and a bad
    /// checksum refused.
    #[test]
    fn install_from_local_server() {
        use std::net::TcpListener;
        let tmp = std::env::temp_dir().join(format!("lenta-pu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let root = tmp.join("Steam");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        // A tool folder, packed.
        let src = tmp.join("src/GE-ProtonTest-1");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("compatibilitytool.vdf"), "\"compatibilitytools\" { \"compat_tools\" { \"GE-ProtonTest-1\" { \"display_name\" \"GE-ProtonTest-1\" } } }").unwrap();
        std::fs::write(src.join("proton"), "#!/bin/sh\n").unwrap();
        let archive = tmp.join("GE-ProtonTest-1.tar.gz");
        assert!(
            std::process::Command::new("tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(tmp.join("src"))
                .arg("GE-ProtonTest-1")
                .status()
                .unwrap()
                .success()
        );
        let data = std::fs::read(&archive).unwrap();
        let sum: String =
            sha2::Digest::finalize(<sha2::Sha512 as sha2::Digest>::new_with_prefix(&data))
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
        let good = format!("{sum}  GE-ProtonTest-1.tar.gz\n");
        let bad = format!("{}  GE-ProtonTest-1.tar.gz\n", "0".repeat(128));
        let serve = |sum_body: String| {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            let data = data.clone();
            std::thread::spawn(move || {
                for s in l.incoming().take(4) {
                    let mut s = s.unwrap();
                    let mut req = [0u8; 2048];
                    let n = std::io::Read::read(&mut s, &mut req).unwrap();
                    let line = String::from_utf8_lossy(&req[..n])
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    let body: Vec<u8> = if line.contains(".sha512sum") {
                        sum_body.clone().into_bytes()
                    } else {
                        data.clone()
                    };
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(head.as_bytes());
                    let _ = s.write_all(&body);
                }
            });
            port
        };
        let release = |port: u16| Release {
            flavour: Flavour::Ge,
            tag: "GE-ProtonTest-1".into(),
            published: 0,
            urls: vec![format!("http://127.0.0.1:{port}/GE-ProtonTest-1.tar.gz")],
            size: data.len() as u64,
        };
        let cancel = AtomicBool::new(false);
        // A wrong checksum: refused, nothing installed.
        let tools = root.join("compatibilitytools.d");
        let cache = tmp.join("cache");
        let err = install_into(&release(serve(bad)), &tools, &cache, &cancel, &|_| {}).unwrap_err();
        assert!(err.contains("checksum"), "{err}");
        assert!(!root.join("compatibilitytools.d/GE-ProtonTest-1").exists());
        // The right one: unpacked into compatibilitytools.d.
        let dir = install_into(&release(serve(good)), &tools, &cache, &cancel, &|_| {}).unwrap();
        assert_eq!(dir, root.join("compatibilitytools.d/GE-ProtonTest-1"));
        assert!(dir.join("compatibilitytool.vdf").is_file());
        remove_in(&tools, &dir).unwrap();
        assert!(!dir.exists());
        // Never outside compatibilitytools.d.
        assert!(remove_in(&tools, &tmp.join("src")).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The download keeps going past a minute (no whole-body timeout):
    /// reads the newest GE archive for 75 s and drops it (nothing kept):
    /// `cargo test long_download -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn long_download() {
        let r = releases(Flavour::Ge).unwrap();
        let mut resp = agent().get(&r[0].urls[0]).call().expect("answer");
        let mut body = resp.body_mut().with_config().limit(4 << 30).reader();
        let mut buf = vec![0u8; 1 << 20];
        let (start, mut got) = (std::time::Instant::now(), 0u64);
        while start.elapsed() < Duration::from_secs(75) {
            let n = body.read(&mut buf).expect("still reading after a minute");
            if n == 0 {
                break;
            }
            got += n as u64;
        }
        eprintln!("{} MB in {:?}", got >> 20, start.elapsed());
        assert!(got > 0);
    }

    /// Lists both flavours (talks to GitHub): `cargo test protonup_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn protonup_live() {
        for f in Flavour::ALL {
            let r = releases(f).unwrap();
            eprintln!(
                "{}: {:?}",
                f.name(),
                r.iter()
                    .take(3)
                    .map(|x| (&x.tag, &x.urls[0]))
                    .collect::<Vec<_>>()
            );
            assert!(!r.is_empty());
        }
    }
}
