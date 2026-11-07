//! Signing in to Steam (optional): the client's own QR sign-in through
//! IAuthenticationService over HTTPS, as the Steam app on a phone confirms
//! it. The refresh token (good for about 200 days) is kept in
//! ~/.local/share/lenta/steam.json, readable only by the user; with it the
//! launcher reads the list of owned games.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

const API: &str = "https://api.steampowered.com";
/// Signing in as the Steam client: its token opens a CM session, which
/// downloading old versions needs. Web API access comes from a website
/// session or, failing that, from the CM (see `access_token`).
const PLATFORM_STEAM_CLIENT: &str = "1";

fn data_dir() -> PathBuf {
    std::env::var_os("LENTA_DATA")
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|d| d.join("lenta")))
        .unwrap_or_else(|| "/tmp/lenta-data".into())
}

fn token_path() -> PathBuf {
    data_dir().join("steam.json")
}

fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{k}={}", enc(v)))
        .collect::<Vec<_>>()
        .join("&")
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

/// POST to IAuthenticationService; Steam's verdict comes in X-eresult.
fn call(method: &str, fields: &[(&str, &str)]) -> Result<Value, String> {
    if std::env::var_os("LENTA_OFFLINE").is_some() {
        return Err("offline".into());
    }
    let url = format!("{API}/IAuthenticationService/{method}/v1");
    let resp = crate::net::agent()
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(form(fields));
    let mut resp = resp.map_err(|e| e.to_string())?;
    let eresult: i64 = resp
        .headers()
        .get("x-eresult")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    if eresult != 1 {
        return Err(match eresult {
            9 | 27 => "expired".into(),
            84 => "rate_limit".into(),
            e => format!("EResult {e}"),
        });
    }
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(v.get("response").cloned().unwrap_or(Value::Null))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(h) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A byte from the kernel's random pool.
fn rand_byte() -> u8 {
    use std::io::Read;
    let mut b = [0u8; 1];
    let _ = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    b[0]
}

// ------------------------------------------------------------------ token

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub name: String,
    pub steamid: String,
    refresh_token: String,
}

/// The JWT's payload.
fn claims(token: &str) -> Option<Value> {
    use base64::Engine;
    let part = token.split('.').nth(1)?;
    let b = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(part.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&b).ok()
}

impl Account {
    /// For the CM sign-in (downloads).
    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    /// Signed in as the Steam client: its token also opens a CM session,
    /// which downloading old versions needs.
    pub fn is_client(&self) -> bool {
        claims(&self.refresh_token)
            .and_then(|c| c.get("aud").cloned())
            .and_then(|aud| aud.as_array().cloned())
            .is_some_and(|aud| aud.iter().any(|x| x.as_str() == Some("client")))
    }

    fn from_token(name: String, refresh_token: String) -> Option<Account> {
        let c = claims(&refresh_token)?;
        let steamid = c.get("sub")?.as_str()?.to_string();
        Some(Account {
            name,
            steamid,
            refresh_token,
        })
    }

    /// Still accepted (with a minute to spare)?
    pub fn valid(&self) -> bool {
        let exp = claims(&self.refresh_token)
            .and_then(|c| c.get("exp")?.as_u64())
            .unwrap_or(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        exp == 0 || exp > now + 60
    }

    pub fn load() -> Option<Account> {
        let v: Value = serde_json::from_slice(&std::fs::read(token_path()).ok()?).ok()?;
        let a = Account::from_token(
            v.get("account_name")?.as_str()?.to_string(),
            v.get("refresh_token")?.as_str()?.to_string(),
        )?;
        a.valid().then_some(a)
    }

    fn save(&self) -> Result<(), String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let p = token_path();
        std::fs::create_dir_all(p.parent().ok_or("no data dir")?).map_err(|e| e.to_string())?;
        let tmp = p.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        let body = serde_json::json!({
            "account_name": self.name,
            "refresh_token": self.refresh_token,
        });
        f.write_all(body.to_string().as_bytes())
            .map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
    }

    pub fn forget() {
        let _ = std::fs::remove_file(token_path());
        let _ = std::fs::remove_file(owned_cache());
    }

    /// A day-long token for web API calls, the way the Steam website
    /// gets one: the refresh token is turned into a store session, whose
    /// steamLoginSecure cookie holds "steamid||access token".
    /// (IAuthenticationService/GenerateAccessTokenForApp answers
    /// AccessDenied to these tokens.)
    fn access_token(&self) -> Result<String, String> {
        match self.web_session_token() {
            Ok(t) => Ok(t),
            // A client token the website won't take: ask the CM, as the
            // Steam client does.
            Err(e) if self.is_client() => {
                let mut cm = crate::downgrade::cm::Cm::logon(&self.name, &self.refresh_token)
                    .map_err(|c| format!("{e}; CM: {c}"))?;
                let t = cm.access_token(&self.refresh_token);
                cm.close();
                t.map_err(|c| format!("{e}; CM: {c}"))
            }
            Err(e) => Err(e),
        }
    }

    fn web_session_token(&self) -> Result<String, String> {
        for (name, value) in self.web_cookies("store.steampowered.com")? {
            if name == "steamLoginSecure"
                && let Some((_, token)) = percent_decode(&value).split_once("||")
            {
                return Ok(token.to_string());
            }
        }
        Err("settoken: no session cookie".into())
    }

    /// Sign in to one of Steam's sites (`host`: "steamcommunity.com",
    /// "store.steampowered.com"…) as the website does: the refresh token
    /// becomes a session there. Returns the cookies the site set.
    pub fn web_cookies(&self, host: &str) -> Result<Vec<(String, String)>, String> {
        if std::env::var_os("LENTA_OFFLINE").is_some() {
            return Err("offline".into());
        }
        let session: String = (0..12).map(|_| format!("{:02x}", rand_byte())).collect();
        let post = |url: &str, fields: &[(&str, &str)]| {
            crate::net::agent()
                .post(url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Origin", "https://steamcommunity.com")
                .header("Referer", "https://steamcommunity.com/")
                .send(form(fields))
                .map_err(|e| e.to_string())
        };
        let mut r = post(
            "https://login.steampowered.com/jwt/finalizelogin",
            &[
                ("nonce", &self.refresh_token),
                ("sessionid", &session),
                ("redir", "https://steamcommunity.com/login/home/?goto="),
            ],
        )?;
        let v: Value =
            serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if let Some(e) = v.get("error") {
            return Err(format!("finalizelogin: {e}"));
        }
        let steamid = v
            .get("steamID")
            .and_then(Value::as_str)
            .ok_or("finalizelogin: no steamID")?;
        let transfer = v
            .get("transfer_info")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|t| {
                t.get("url")
                    .and_then(Value::as_str)
                    .is_some_and(|u| u.contains(host))
            })
            .ok_or_else(|| format!("finalizelogin: no session for {host}"))?;
        let url = transfer
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = transfer.get("params").ok_or("finalizelogin: no params")?;
        let s = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or_default();
        let resp = post(
            url,
            &[
                ("nonce", s("nonce")),
                ("auth", s("auth")),
                ("steamID", steamid),
            ],
        )?;
        Ok(resp
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|c| {
                let kv = c.to_str().ok()?.split(';').next()?;
                let (n, v) = kv.split_once('=')?;
                Some((n.trim().to_string(), v.trim().to_string()))
            })
            .collect())
    }
}

// ---------------------------------------------------------------- QR login

/// One QR sign-in: show `url` as a QR code, then poll until the phone
/// confirms it.
pub struct QrLogin {
    pub url: String,
    client_id: String,
    request_id: String,
    pub interval: Duration,
}

pub enum Poll {
    Waiting,
    /// The code was scanned; waiting for the tap on the phone.
    Scanned,
    /// Steam turned the code over: show the new one.
    NewCode(String),
    Done(Account),
}

impl QrLogin {
    pub fn start() -> Result<QrLogin, String> {
        let name = format!(
            "Lenta ({})",
            std::fs::read_to_string("/etc/hostname")
                .or_else(|_| std::fs::read_to_string("/proc/sys/kernel/hostname"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "Linux".into())
        );
        let r = call(
            "BeginAuthSessionViaQR",
            &[
                ("device_friendly_name", &name),
                ("platform_type", PLATFORM_STEAM_CLIENT),
            ],
        )?;
        let s = |k: &str| {
            r.get(k)
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    v => v.to_string(),
                })
                .ok_or_else(|| format!("no {k}"))
        };
        Ok(QrLogin {
            url: s("challenge_url")?,
            client_id: s("client_id")?,
            request_id: s("request_id")?,
            interval: Duration::from_secs_f64(
                r.get("interval").and_then(Value::as_f64).unwrap_or(5.0),
            ),
        })
    }

    pub fn poll(&mut self) -> Result<Poll, String> {
        let r = call(
            "PollAuthSessionStatus",
            &[
                ("client_id", &self.client_id),
                ("request_id", &self.request_id),
            ],
        )?;
        if let Some(id) = r.get("new_client_id") {
            self.client_id = match id {
                Value::String(s) => s.clone(),
                v => v.to_string(),
            };
        }
        if let Some(t) = r.get("refresh_token").and_then(Value::as_str) {
            let name = r
                .get("account_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let a = Account::from_token(name, t.to_string()).ok_or("bad token")?;
            a.save()?;
            return Ok(Poll::Done(a));
        }
        if let Some(u) = r.get("new_challenge_url").and_then(Value::as_str) {
            self.url = u.to_string();
            return Ok(Poll::NewCode(u.to_string()));
        }
        if r.get("had_remote_interaction").and_then(Value::as_bool) == Some(true) {
            return Ok(Poll::Scanned);
        }
        Ok(Poll::Waiting)
    }
}

// ------------------------------------------------------------- owned games

#[derive(Clone, Debug)]
pub struct Owned {
    pub appid: u32,
    pub name: String,
    pub playtime_min: u64,
    pub last_played: u64,
    /// Lent by someone in the user's Steam family.
    pub family: bool,
}

fn owned_cache() -> PathBuf {
    crate::net::cache_dir().join("owned.json")
}

/// Every game the account owns (with play time across all devices). Asked
/// at most every hour; the last answer stands in when offline.
pub fn owned(a: &Account) -> Result<Vec<Owned>, String> {
    let file = owned_cache();
    let fresh = std::fs::metadata(&file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(3600));
    let mut error = String::new();
    if !fresh {
        match fetch_owned(a) {
            Ok(b) => {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&file, &b);
            }
            Err(e) => {
                eprintln!("lenta: owned games: {e}");
                error = e;
            }
        }
    }
    // The last list stands in when Steam can't be asked.
    std::fs::read(&file)
        .ok()
        .and_then(|b| parse_owned(&b))
        .ok_or(error)
}

/// Ask Steam for the list; says which step failed.
pub fn fetch_owned(a: &Account) -> Result<Vec<u8>, String> {
    let token = a.access_token().map_err(|e| format!("access token: {e}"))?;
    let url = format!(
        "{API}/IPlayerService/GetOwnedGames/v1/?access_token={}&steamid={}&include_appinfo=true&include_played_free_games=true&language={}",
        enc(&token),
        a.steamid,
        crate::i18n::steam_lang()
    );
    let b = crate::net::get(&url).map_err(|e| format!("GetOwnedGames: {e}"))?;
    let mut v: Value = serde_json::from_slice(&b).map_err(|e| format!("GetOwnedGames: {e}"))?;
    if parse_owned(&b).is_none() {
        return Err(format!(
            "GetOwnedGames: unexpected answer: {}",
            String::from_utf8_lossy(&b[..b.len().min(200)])
        ));
    }
    // The Steam family's shared library, kept beside it (best effort).
    if let Some(apps) = family_apps(a, &token) {
        v["family"] = apps;
    }
    Ok(v.to_string().into_bytes())
}

/// The games the user's Steam family lends them (their own left out).
fn family_apps(a: &Account, token: &str) -> Option<Value> {
    let t = enc(token);
    let url = format!(
        "{API}/IFamilyGroupsService/GetFamilyGroupForUser/v1/?access_token={t}&steamid={}",
        a.steamid
    );
    let v: Value = serde_json::from_slice(&crate::net::get(&url).ok()?).ok()?;
    let gid = v
        .pointer("/response/family_groupid")
        .and_then(|g| match g {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })?;
    let url = format!(
        "{API}/IFamilyGroupsService/GetSharedLibraryApps/v1/?access_token={t}&family_groupid={gid}&include_own=false&include_excluded=false&include_free=false&language={}",
        crate::i18n::steam_lang()
    );
    let v: Value = serde_json::from_slice(&crate::net::get(&url).ok()?).ok()?;
    v.pointer("/response/apps").cloned()
}

fn parse_owned(b: &[u8]) -> Option<Vec<Owned>> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let games = v.pointer("/response/games")?.as_array()?;
    let mut out: Vec<Owned> = games
        .iter()
        .filter_map(|g| {
            Some(Owned {
                appid: g.get("appid")?.as_u64()? as u32,
                name: g.get("name")?.as_str()?.to_string(),
                playtime_min: g
                    .get("playtime_forever")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                last_played: g
                    .get("rtime_last_played")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                family: false,
            })
        })
        .collect();
    let mine: std::collections::HashSet<u32> = out.iter().map(|o| o.appid).collect();
    for a in v
        .get("family")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // Games only (app_type 1), not tools or soundtracks.
        if a.get("app_type")
            .and_then(Value::as_u64)
            .is_some_and(|t| t != 1)
        {
            continue;
        }
        let Some(id) = a.get("appid").and_then(Value::as_u64).map(|i| i as u32) else {
            continue;
        };
        let Some(name) = a.get("name").and_then(Value::as_str) else {
            continue;
        };
        if mine.contains(&id) {
            continue;
        }
        out.push(Owned {
            appid: id,
            name: name.to_string(),
            playtime_min: 0,
            last_played: a.get("rt_last_played").and_then(Value::as_u64).unwrap_or(0),
            family: true,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_claims() {
        // {"sub":"76561190000000000","exp":4102444800}
        let t =
            "eyJhbGciOiJFZERTQSJ9.eyJzdWIiOiI3NjU2MTE5MDAwMDAwMDAwMCIsImV4cCI6NDEwMjQ0NDgwMH0.sig";
        let a = Account::from_token("me".into(), t.into()).unwrap();
        assert_eq!(a.steamid, "76561190000000000");
        assert!(a.valid());
    }

    /// Talks to Steam (starts a sign-in nobody finishes):
    /// `cargo test qr_live -- --ignored`.
    #[test]
    #[ignore]
    fn qr_live() {
        let mut q = QrLogin::start().unwrap();
        assert!(q.url.starts_with("https://s.team/q/"), "{}", q.url);
        assert!(matches!(q.poll().unwrap(), Poll::Waiting));
    }

    #[test]
    fn owned_json() {
        let b = br#"{"response":{"game_count":1,"games":[{"appid":570,"name":"Dota 2","playtime_forever":61,"rtime_last_played":5}]}}"#;
        let o = parse_owned(b).unwrap();
        assert_eq!(o[0].appid, 570);
        assert_eq!(o[0].playtime_min, 61);
    }
}
