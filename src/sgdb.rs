//! SteamGridDB: art for games (covers, heroes, logos), and getting its API
//! key without typing. The site signs in through Steam (OpenID); with the
//! launcher's Steam sign-in that happens on its own: the Steam community
//! session confirms the OpenID request, SteamGridDB sets its session, and
//! the key is read from the profile (or created). An account is created
//! only when the user agrees, with the name they pick.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use crate::account::Account;

const SITE: &str = "https://www.steamgriddb.com";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

fn key_path() -> PathBuf {
    std::env::var_os("LENTA_DATA")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("lenta")))
        .unwrap_or_else(|| "/tmp/lenta-data".into())
        .join("steamgriddb.json")
}

/// The stored API key.
pub fn key() -> Option<String> {
    let v: Value = serde_json::from_slice(&std::fs::read(key_path()).ok()?).ok()?;
    v.get("api_key")?
        .as_str()
        .map(str::to_string)
        .filter(|k| !k.is_empty())
}

pub fn save_key(key: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let p = key_path();
    std::fs::create_dir_all(p.parent().ok_or("no data dir")?).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    f.write_all(json!({ "api_key": key }).to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub fn forget_key() {
    let _ = std::fs::remove_file(key_path());
}

/// Is this key accepted by the API?
pub fn check_key(key: &str) -> Result<bool, String> {
    let agent = agent();
    let resp = agent
        .get(&format!("{SITE}/api/v2/search/autocomplete/portal"))
        .header("Authorization", &format!("Bearer {key}"))
        .call()
        .map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16() == 200)
}

// ------------------------------------------------------------- sign-in

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(25)))
        .user_agent(UA)
        .build()
        .into()
}

/// A tiny cookie jar: (domain, name, value), sent to hosts ending in the
/// domain.
#[derive(Default)]
struct Jar(Vec<(String, String, String)>);

impl Jar {
    fn set(&mut self, domain: &str, name: &str, value: &str) {
        self.0.retain(|(d, n, _)| !(d == domain && n == name));
        if value != "deleted" && !value.is_empty() {
            self.0.push((domain.into(), name.into(), value.into()));
        }
    }

    fn take(&mut self, host: &str, resp: &ureq::http::Response<ureq::Body>) {
        for c in resp.headers().get_all("set-cookie") {
            let Ok(c) = c.to_str() else { continue };
            let mut parts = c.split(';');
            let Some((n, v)) = parts.next().and_then(|kv| kv.split_once('=')) else {
                continue;
            };
            let domain = parts
                .filter_map(|p| {
                    p.trim()
                        .strip_prefix("domain=")
                        .or_else(|| p.trim().strip_prefix("Domain="))
                })
                .next()
                .map(|d| d.trim_start_matches('.').to_string())
                .unwrap_or_else(|| host.to_string());
            let gone = c.to_ascii_lowercase().contains("max-age=0");
            self.set(&domain, n.trim(), if gone { "" } else { v.trim() });
        }
    }

    fn header(&self, host: &str) -> String {
        self.0
            .iter()
            .filter(|(d, _, _)| host == d || host.ends_with(&format!(".{d}")))
            .map(|(_, n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn get(&self, host: &str, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(d, n, _)| n == name && (host == d || host.ends_with(&format!(".{d}"))))
            .map(|(_, _, v)| v.as_str())
    }
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or_default()
        .to_string()
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A signed-in SteamGridDB session.
pub struct Session {
    agent: ureq::Agent,
    jar: Jar,
}

#[derive(Debug, PartialEq)]
pub enum Who {
    /// Signed in to an account.
    User(String),
    /// Steam knows the person, SteamGridDB doesn't yet.
    NoAccount,
}

impl Session {
    fn get(&mut self, url: &str) -> Result<(u16, String, String), String> {
        let host = host_of(url);
        let mut req = self
            .agent
            .get(url)
            .header("Accept", "application/json, text/html");
        let cookies = self.jar.header(&host);
        if !cookies.is_empty() {
            req = req.header("Cookie", &cookies);
        }
        let mut resp = req.call().map_err(|e| e.to_string())?;
        self.jar.take(&host, &resp);
        let status = resp.status().as_u16();
        let loc = resp
            .headers()
            .get("location")
            .and_then(|l| l.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        Ok((status, loc, body))
    }

    /// POST JSON to SteamGridDB, as its own pages do (with the XSRF echo).
    fn post_json(&mut self, path: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("{SITE}{path}");
        let host = host_of(&url);
        let mut req = self
            .agent
            .post(&url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Origin", SITE)
            .header("Referer", &format!("{SITE}/"));
        let cookies = self.jar.header(&host);
        if !cookies.is_empty() {
            req = req.header("Cookie", &cookies);
        }
        if let Some(x) = self.jar.get(&host, "XSRF-TOKEN") {
            let x = x.to_string();
            req = req.header("X-XSRF-TOKEN", &x);
        }
        let mut resp = req
            .send(body.unwrap_or(json!({})).to_string())
            .map_err(|e| e.to_string())?;
        self.jar.take(&host, &resp);
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        serde_json::from_str(&text).map_err(|_| format!("{path}: unexpected answer"))
    }

    /// Sign in through Steam with the launcher's Steam session.
    pub fn sign_in(account: &Account) -> Result<(Session, Who), String> {
        let mut s = Session {
            agent: agent(),
            jar: Jar::default(),
        };
        for (n, v) in account.web_cookies("steamcommunity.com")? {
            s.jar.set("steamcommunity.com", &n, &v);
        }
        // Steam's OpenID page, as SteamGridDB's "Sign in through Steam".
        let params = [
            (
                "openid.identity",
                "http://specs.openid.net/auth/2.0/identifier_select",
            ),
            (
                "openid.claimed_id",
                "http://specs.openid.net/auth/2.0/identifier_select",
            ),
            ("openid.ns", "http://specs.openid.net/auth/2.0"),
            ("openid.mode", "checkid_setup"),
            ("openid.realm", SITE),
            ("openid.return_to", &format!("{SITE}/login/steam")),
        ];
        let q: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{k}={}", enc(v)))
            .collect();
        let openid = format!("https://steamcommunity.com/openid/login?{}", q.join("&"));
        let (_, _, page) = s.get(&openid)?;
        let form_start = page
            .find("id=\"openidForm\"")
            .ok_or("Steam did not offer the sign-in (session?)")?;
        let form = &page[page[..form_start].rfind("<form").unwrap_or(form_start)..];
        let form = &form[..form.find("</form>").unwrap_or(form.len())];
        let attr = |tag: &str, name: &str| -> Option<String> {
            let i = tag.find(&format!("{name}=\""))? + name.len() + 2;
            Some(unescape(&tag[i..i + tag[i..].find('"')?]))
        };
        let action = attr(&form[..form.find('>').unwrap_or(0)], "action")
            .unwrap_or_else(|| "https://steamcommunity.com/openid/login".into());
        let mut fields: Vec<(String, String)> = Vec::new();
        for part in form.split("<input").skip(1) {
            let tag = &part[..part.find('>').unwrap_or(part.len())];
            if let Some(n) = attr(tag, "name") {
                fields.push((n, attr(tag, "value").unwrap_or_default()));
            }
        }
        // The form is multipart.
        let boundary = format!("----lenta{:016x}", crate::net::hash(&page));
        let mut body = String::new();
        for (k, v) in &fields {
            body += &format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"
            );
        }
        body += &format!("--{boundary}--\r\n");
        let host = host_of(&action);
        let mut resp = s
            .agent
            .post(&action)
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .header("Origin", "https://steamcommunity.com")
            .header("Referer", &openid)
            .header("Cookie", &s.jar.header(&host))
            .send(body)
            .map_err(|e| e.to_string())?;
        s.jar.take(&host, &resp);
        let back = resp
            .headers()
            .get("location")
            .and_then(|l| l.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let _ = resp.body_mut().read_to_string();
        if !back.starts_with(&format!("{SITE}/login/steam")) {
            return Err("Steam did not confirm the sign-in".into());
        }
        let (status, to, _) = s.get(&back)?;
        if status >= 400 {
            return Err(format!("SteamGridDB refused the sign-in ({status})"));
        }
        let _ = to;
        let who = s.whoami()?;
        Ok((s, who))
    }

    pub fn whoami(&mut self) -> Result<Who, String> {
        let (_, _, body) = self.get(&format!("{SITE}/api/public/user"))?;
        let v: Value = serde_json::from_str(&body).map_err(|_| "user: unexpected answer")?;
        if v.get("success").and_then(Value::as_bool) == Some(true) {
            let name = v
                .pointer("/data/user/username")
                .or_else(|| v.pointer("/data/username"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            return Ok(Who::User(name));
        }
        let err = v.get("error").map(|e| e.to_string()).unwrap_or_default();
        if err.contains("no_account") {
            Ok(Who::NoAccount)
        } else {
            Err(format!("SteamGridDB: {err}"))
        }
    }

    /// Is the name free (SteamGridDB's own check)? Returns its error if not.
    pub fn check_name(&mut self, name: &str) -> Result<(), String> {
        let (_, _, body) = self.get(&format!(
            "{SITE}/api/v2/users/checkname?username={}",
            enc(name)
        ))?;
        let v: Value = serde_json::from_str(&body).map_err(|_| "checkname: unexpected answer")?;
        if v.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(v
                .get("data")
                .map(|d| d.to_string())
                .unwrap_or_else(|| "taken".into()))
        }
    }

    /// Create the account (the user agreed, and chose the name).
    pub fn register(&mut self, name: &str) -> Result<(), String> {
        let v = self.post_json("/register", Some(json!({ "username": name })))?;
        if v.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(format!(
                "register: {}",
                v.get("data")
                    .or_else(|| v.get("error"))
                    .map(|d| d.to_string())
                    .unwrap_or_default()
            ))
        }
    }

    /// The account's API key, made if it has none.
    pub fn api_key(&mut self) -> Result<String, String> {
        let (_, _, body) = self.get(&format!("{SITE}/api/public/prefs"))?;
        let v: Value = serde_json::from_str(&body).map_err(|_| "prefs: unexpected answer")?;
        let have = v
            .pointer("/data/user/api_key")
            .or_else(|| v.pointer("/user/api_key"))
            .and_then(Value::as_str)
            .filter(|k| !k.is_empty());
        if let Some(k) = have {
            return Ok(k.to_string());
        }
        let v = self.post_json("/api/v2/users/apikey", None)?;
        v.get("data")
            .and_then(Value::as_str)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .ok_or_else(|| "apikey: no key in the answer".into())
    }
}

/// Names to offer for a new account: the Steam name, then variants.
pub fn name_choices(steam_name: &str) -> Vec<String> {
    let base: String = steam_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(20)
        .collect();
    let base = if base.is_empty() {
        "player".to_string()
    } else {
        base
    };
    vec![
        base.clone(),
        format!("{base}_lenta"),
        format!("{base}{}", crate::net::hash(steam_name) % 1000),
    ]
}

/// The person's Steam display name, from their public profile.
pub fn persona_name(steamid: &str) -> Option<String> {
    let b = crate::net::get(&format!(
        "https://steamcommunity.com/profiles/{steamid}?xml=1"
    ))
    .ok()?;
    let t = String::from_utf8_lossy(&b);
    let i = t.find("<steamID>")? + 9;
    let j = t[i..].find("</steamID>")? + i;
    let v = t[i..j].trim();
    let v = v
        .strip_prefix("<![CDATA[")
        .and_then(|v| v.strip_suffix("]]>"))
        .unwrap_or(v);
    Some(v.to_string()).filter(|v| !v.is_empty())
}

// ------------------------------------------------------------------- art

/// One picture on offer.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub url: String,
    pub thumb: String,
    pub width: u32,
    pub height: u32,
    pub style: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArtKind {
    Portrait,
    Landscape,
    Hero,
    Logo,
}

impl ArtKind {
    pub const ALL: [ArtKind; 4] = [
        ArtKind::Portrait,
        ArtKind::Landscape,
        ArtKind::Hero,
        ArtKind::Logo,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ArtKind::Portrait => "portrait",
            ArtKind::Landscape => "landscape",
            ArtKind::Hero => "hero",
            ArtKind::Logo => "logo",
        }
    }

    fn path(self) -> &'static str {
        match self {
            ArtKind::Portrait => "grids/game/{id}?dimensions=600x900,342x482,660x930",
            ArtKind::Landscape => "grids/game/{id}?dimensions=920x430,460x215",
            ArtKind::Hero => "heroes/game/{id}",
            ArtKind::Logo => "logos/game/{id}",
        }
    }
}

/// A GET to the API with the key, answers kept a week (`sub/name`).
fn api(key: &str, path: &str, cache: &str) -> Option<Value> {
    let file = crate::net::cache_dir().join("sgdb").join(cache);
    let fresh = std::fs::metadata(&file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|a| a < Duration::from_secs(7 * 86400));
    if fresh
        && let Some(v) = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    {
        return Some(v);
    }
    if std::env::var_os("LENTA_OFFLINE").is_some() {
        return std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
    }
    let resp = agent()
        .get(&format!("{SITE}/api/v2/{path}"))
        .header("Authorization", &format!("Bearer {key}"))
        .call();
    let v: Option<Value> = resp.ok().and_then(|mut r| {
        let ok = r.status().as_u16() < 500;
        let t = r.body_mut().read_to_string().ok()?;
        ok.then(|| serde_json::from_str(&t).ok()).flatten()
    });
    match v {
        Some(v) => {
            let _ = std::fs::create_dir_all(file.parent()?);
            let _ = std::fs::write(&file, v.to_string());
            Some(v)
        }
        None => std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok()),
    }
}

/// SteamGridDB's id for a game: by its Steam id, else by an exact title.
pub fn game_id(key: &str, steam: Option<u32>, title: &str) -> Option<u64> {
    if let Some(app) = steam
        && let Some(v) = api(
            key,
            &format!("games/steam/{app}"),
            &format!("steam-{app}.json"),
        )
        && let Some(id) = v.pointer("/data/id").and_then(Value::as_u64)
    {
        return Some(id);
    }
    let want = crate::sources::web::words(title);
    if want.is_empty() {
        return None;
    }
    let term = want.join(" ");
    let v = api(
        key,
        &format!("search/autocomplete/{}", enc(&term)),
        &format!("search-{:016x}.json", crate::net::hash(&term)),
    )?;
    v.get("data")?
        .as_array()?
        .iter()
        .find(|g| {
            g.get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| crate::sources::web::words(n) == want)
        })
        .and_then(|g| g.get("id")?.as_u64())
}

/// The pictures of one kind for a game, best first; none of the NSFW,
/// joke or flashing ones.
pub fn assets(key: &str, id: u64, kind: ArtKind) -> Vec<Asset> {
    let path = kind.path().replace("{id}", &id.to_string());
    let sep = if path.contains('?') { '&' } else { '?' };
    let path = format!("{path}{sep}nsfw=false&humor=false");
    let Some(v) = api(key, &path, &format!("{}-{id}.json", kind.key())) else {
        return Vec::new();
    };
    v.get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|a| {
            !["nsfw", "humor", "epilepsy"]
                .iter()
                .any(|f| a.get(*f).and_then(Value::as_bool) == Some(true))
        })
        .filter_map(|a| {
            Some(Asset {
                url: a.get("url")?.as_str()?.to_string(),
                thumb: a.get("thumb")?.as_str()?.to_string(),
                width: a.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
                height: a.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
                style: a
                    .get("style")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect()
}

// ------------------------------------------------------- the user's picks

fn picks_path() -> PathBuf {
    key_path().with_file_name("art.json")
}

/// Pictures picked per game: game key → kind → url.
pub fn picks() -> std::collections::HashMap<String, std::collections::HashMap<String, String>> {
    std::fs::read(picks_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Pick a picture (None: back to the game's own).
pub fn set_pick(game: &str, kind: ArtKind, url: Option<&str>) -> Result<(), String> {
    let mut all = picks();
    let e = all.entry(game.to_string()).or_default();
    match url {
        Some(u) => {
            e.insert(kind.key().to_string(), u.to_string());
        }
        None => {
            e.remove(kind.key());
        }
    }
    all.retain(|_, m| !m.is_empty());
    let p = picks_path();
    std::fs::create_dir_all(p.parent().ok_or("no data dir")?).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}
