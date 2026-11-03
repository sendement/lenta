//! Steam's content servers: manifests and chunks over HTTPS, the next
//! server tried when one fails; chunks decrypted with the depot key and
//! checked against their SHA-1.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;

use super::cm::Cm;
use super::crypto;
use super::manifest::Manifest;

#[derive(Clone, Debug)]
pub struct Server {
    pub host: String,
    pub https: bool,
}

pub struct Cdn {
    pub servers: Vec<Server>,
    /// Hosts' auth tokens ("" = none needed), asked of the CM once each.
    tokens: Mutex<HashMap<String, String>>,
    next: Mutex<usize>,
}

impl Cdn {
    pub fn new(cell: u32) -> Result<Cdn, String> {
        let b = crate::net::get(&format!(
            "https://api.steampowered.com/IContentServerDirectoryService/GetServersForSteamPipe/v1/?cell_id={cell}&max_servers=20"
        ))?;
        let v: Value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
        let servers: Vec<Server> = v
            .pointer("/response/servers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            // OpenCache servers sit in ISP networks and often refuse.
            .filter(|s| s.get("type").and_then(Value::as_str) != Some("OpenCache"))
            .filter_map(|s| {
                Some(Server {
                    host: s.get("host")?.as_str()?.to_string(),
                    https: s.get("https_support").and_then(Value::as_str) != Some("unavailable"),
                })
            })
            .collect();
        if servers.is_empty() {
            return Err("no content servers".into());
        }
        Ok(Cdn {
            servers,
            tokens: Mutex::new(HashMap::new()),
            next: Mutex::new(0),
        })
    }

    fn token(&self, cm: Option<&Mutex<Cm>>, app: u32, depot: u32, host: &str) -> String {
        let key = format!("{depot}@{host}");
        if let Some(t) = self.tokens.lock().ok().and_then(|t| t.get(&key).cloned()) {
            return t;
        }
        let t = cm
            .and_then(|cm| cm.lock().ok()?.cdn_token(app, depot, host).ok())
            .unwrap_or_default();
        if let Ok(mut m) = self.tokens.lock() {
            m.insert(key, t.clone());
        }
        t
    }

    /// GET `path` from a content server, moving on to the next on errors.
    pub fn get(
        &self,
        cm: Option<&Mutex<Cm>>,
        app: u32,
        depot: u32,
        path: &str,
    ) -> Result<Vec<u8>, String> {
        let n = self.servers.len();
        let start = self.next.lock().map(|g| *g).unwrap_or(0);
        let mut last = String::new();
        for i in 0..n.min(6) {
            let s = &self.servers[(start + i) % n];
            let token = self.token(cm, app, depot, &s.host);
            let url = format!(
                "{}://{}/{path}{token}",
                if s.https { "https" } else { "http" },
                s.host
            );
            match crate::net::get(&url) {
                Ok(b) => {
                    if i > 0
                        && let Ok(mut g) = self.next.lock()
                    {
                        *g = (start + i) % n;
                    }
                    return Ok(b);
                }
                Err(e) => {
                    // Access refused is the same everywhere.
                    if e.contains("401") || e.contains("403") {
                        return Err(format!("{path}: {e}"));
                    }
                    last = format!("{}: {e}", s.host);
                }
            }
        }
        Err(last)
    }

    /// A manifest with its file names readable.
    pub fn manifest(
        &self,
        cm: &Mutex<Cm>,
        app: u32,
        depot: u32,
        gid: u64,
        key: &[u8],
        branch: &str,
    ) -> Result<Manifest, String> {
        let code = cm
            .lock()
            .map_err(|_| "CM busy")?
            .manifest_code(app, depot, gid, branch)?;
        let path = if code != 0 {
            format!("depot/{depot}/manifest/{gid}/5/{code}")
        } else {
            format!("depot/{depot}/manifest/{gid}/5")
        };
        let mut m = Manifest::from_cdn(&self.get(Some(cm), app, depot, &path)?)?;
        m.decrypt_names(key)?;
        Ok(m)
    }

    /// One chunk's plain bytes, checked.
    pub fn chunk(
        &self,
        cm: Option<&Mutex<Cm>>,
        app: u32,
        depot: u32,
        sha: &[u8; 20],
        key: &[u8],
    ) -> Result<Vec<u8>, String> {
        let hex: String = sha.iter().map(|b| format!("{b:02x}")).collect();
        let raw = self.get(cm, app, depot, &format!("depot/{depot}/chunk/{hex}"))?;
        let data = crypto::decode_chunk(&raw, key)?;
        if crypto::sha1(&data) != *sha {
            return Err(format!("chunk {hex}: checksum"));
        }
        Ok(data)
    }
}
