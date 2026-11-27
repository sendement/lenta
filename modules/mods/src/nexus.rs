//! Nexus Mods: the account's API key (from its account page), facts
//! about mods and their files, and downloads. Without Premium a file is
//! downloaded only from the site's "Mod Manager Download" button: it opens
//! an `nxm://` link, which comes here (`lenta-mods nxm <link>`) with the
//! one-time key that lets the API give the file's address.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::data;

const API: &str = "https://api.nexusmods.com/v1";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub apikey: String,
    pub user: String,
    pub premium: bool,
}

impl Config {
    pub fn load() -> Config {
        std::fs::read_to_string(data().join("nexus.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(data()).map_err(|e| e.to_string())?;
        let p = data().join("nexus.json");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&p)
            .map_err(|e| e.to_string())?;
        f.write_all(
            serde_json::to_string_pretty(self)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
        .map_err(|e| e.to_string())
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into()
}

/// A JSON answer of the API.
pub fn get(key: &str, path: &str) -> Result<Value, String> {
    if key.is_empty() {
        return Err("no API key".into());
    }
    let mut r = agent()
        .get(&format!("{API}{path}"))
        .header("apikey", key)
        .header("Accept", "application/json")
        .header("Application-Name", "Lenta")
        .header("Application-Version", env!("CARGO_PKG_VERSION"))
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401) => "the API key isn't accepted".to_string(),
            ureq::Error::StatusCode(403) => "Nexus refused (Premium only?)".to_string(),
            ureq::Error::StatusCode(429) => "too many requests to Nexus: later".to_string(),
            e => e.to_string(),
        })?;
    let text = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// Who the key belongs to: (name, premium).
pub fn validate(key: &str) -> Result<(String, bool), String> {
    let v = get(key, "/users/validate.json")?;
    Ok((
        v.get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        v.get("is_premium")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ))
}

/// A mod's name and version.
pub fn mod_info(key: &str, domain: &str, id: u64) -> Result<(String, String), String> {
    let v = get(key, &format!("/games/{domain}/mods/{id}.json"))?;
    Ok((
        v.get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        v.get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    ))
}

/// The newest main file's version (for updates).
pub fn latest(key: &str, domain: &str, id: u64) -> Result<Option<String>, String> {
    let v = get(key, &format!("/games/{domain}/mods/{id}/files.json"))?;
    let files = v
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(files
        .iter()
        .filter(|f| f.get("category_name").and_then(Value::as_str) == Some("MAIN"))
        .max_by_key(|f| {
            f.get("uploaded_timestamp")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        })
        .and_then(|f| f.get("version").and_then(Value::as_str))
        .map(str::to_string))
}

/// The Nexus game for a title ("echoesofaincrad"), from its games list.
pub fn find_game(key: &str, title: &str) -> Result<Option<(String, String)>, String> {
    let cache = data().join("games.json");
    let list: Value = match std::fs::read_to_string(&cache)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(v) => v,
        None => {
            let v = get(key, "/games.json")?;
            let _ = std::fs::write(&cache, v.to_string());
            v
        }
    };
    let norm = |s: &str| -> String {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect()
    };
    let want = norm(title);
    Ok(list.as_array().and_then(|a| {
        a.iter().find_map(|g| {
            let n = g.get("name")?.as_str()?;
            (norm(n) == want).then(|| {
                (
                    g.get("domain_name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    n.to_string(),
                )
            })
        })
    }))
}

/// An `nxm://` link's parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub key: String,
    pub expires: String,
}

pub fn parse_link(s: &str) -> Option<Link> {
    let rest = s.trim().strip_prefix("nxm://")?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 5 || parts[1] != "mods" || parts[3] != "files" {
        return None;
    }
    let q = |k: &str| {
        query
            .split('&')
            .find_map(|kv| kv.strip_prefix(&format!("{k}=")))
            .unwrap_or_default()
            .to_string()
    };
    Some(Link {
        domain: parts[0].to_lowercase(),
        mod_id: parts[2].parse().ok()?,
        file_id: parts[4].parse().ok()?,
        key: q("key"),
        expires: q("expires"),
    })
}

/// The file's address for this link.
pub fn download_url(apikey: &str, l: &Link) -> Result<(String, String), String> {
    let mut path = format!(
        "/games/{}/mods/{}/files/{}/download_link.json",
        l.domain, l.mod_id, l.file_id
    );
    if !l.key.is_empty() {
        path.push_str(&format!("?key={}&expires={}", l.key, l.expires));
    }
    let v = get(apikey, &path)?;
    let url = v
        .as_array()
        .and_then(|a| a.first())
        .and_then(|m| m.get("URI"))
        .and_then(Value::as_str)
        .ok_or("no download address")?
        .to_string();
    let info = get(
        apikey,
        &format!(
            "/games/{}/mods/{}/files/{}.json",
            l.domain, l.mod_id, l.file_id
        ),
    )
    .ok();
    let name = info
        .as_ref()
        .and_then(|i| i.get("file_name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            url.split('?')
                .next()
                .and_then(|u| u.rsplit('/').next())
                .map(|n| n.replace("%20", " "))
        })
        .unwrap_or_else(|| format!("{}-{}.zip", l.mod_id, l.file_id));
    Ok((url, name))
}

/// Fetch a file, telling how far it got.
pub fn fetch(url: &str, to: &Path, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let mut r = ureq::Agent::config_builder()
        .timeout_global(None)
        .build()
        .new_agent()
        .get(url)
        .header("User-Agent", concat!("Lenta/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| e.to_string())?;
    let total = r
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let tmp = to.with_extension("part");
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut body = r.body_mut().as_reader();
    let mut buf = vec![0u8; 1 << 16];
    let mut got = 0u64;
    let mut last = std::time::Instant::now();
    loop {
        let n = body.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        got += n as u64;
        if last.elapsed().as_millis() > 300 {
            progress(got, total);
            last = std::time::Instant::now();
        }
    }
    progress(got, total);
    std::fs::rename(&tmp, to).map_err(|e| e.to_string())
}

pub fn downloads() -> PathBuf {
    data().join("downloads")
}

/// Make links from the site's "Mod Manager Download" come here.
pub fn register_handler() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let apps = dirs::data_dir().ok_or("no data dir")?.join("applications");
    std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=Lenta mods (Nexus links)\nName[ru]=Моды Ленты (ссылки Nexus)\nExec=\"{}\" nxm %u\nNoDisplay=true\nTerminal=false\nMimeType=x-scheme-handler/nxm;\n",
        exe.display()
    );
    std::fs::write(apps.join("lenta-nxm.desktop"), desktop).map_err(|e| e.to_string())?;
    let st = std::process::Command::new("xdg-mime")
        .args(["default", "lenta-nxm.desktop", "x-scheme-handler/nxm"])
        .status()
        .map_err(|e| format!("xdg-mime: {e}"))?;
    let _ = std::process::Command::new("update-desktop-database")
        .arg(&apps)
        .status();
    if st.success() {
        Ok(())
    } else {
        Err("xdg-mime failed".into())
    }
}

/// Who opens nxm links now ("lenta-nxm.desktop" when it's us).
pub fn handler() -> String {
    std::process::Command::new("xdg-mime")
        .args(["query", "default", "x-scheme-handler/nxm"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        let l = parse_link(
            "nxm://EchoesOfAincrad/mods/58/files/321?key=abc&expires=1700000000&user_id=9",
        )
        .unwrap();
        assert_eq!(
            l,
            Link {
                domain: "echoesofaincrad".into(),
                mod_id: 58,
                file_id: 321,
                key: "abc".into(),
                expires: "1700000000".into()
            }
        );
        assert!(parse_link("https://x").is_none());
        assert!(parse_link("nxm://g/collections/x").is_none());
    }
}
