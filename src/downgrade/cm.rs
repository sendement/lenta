//! A Steam CM connection over WebSocket (TLS; no channel encryption of its
//! own), signed in with the account's refresh token like the Steam client.
//! Only what the downloader needs: depot keys, manifest request codes, CDN
//! auth tokens, and web access tokens. Requests are answered in order on
//! one socket; whatever else Steam sends meanwhile is skipped.

use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use serde_json::Value;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use super::pb::{self, Writer};

const PROTO: u32 = 0x8000_0000;
const MULTI: u32 = 1;
const SERVICE_RESPONSE: u32 = 147;
const SERVICE_CALL: u32 = 151;
const HEARTBEAT: u32 = 703;
const LOGON_RESPONSE: u32 = 751;
const LOGGED_OFF: u32 = 757;
const DEPOT_KEY: u32 = 5438;
const DEPOT_KEY_RESPONSE: u32 = 5439;
const LOGON: u32 = 5514;

const PROTOCOL_VERSION: u64 = 65580;
/// SteamID of an individual (instance 1) / anonymous user, account 0.
const INDIVIDUAL: u64 = 0x0110_0001_0000_0000;
const ANON: u64 = 0x01A0_0000_0000_0000;

/// EResults that mean the stored sign-in is no good any more.
pub const AUTH_ERESULTS: [i64; 9] = [5, 15, 26, 27, 63, 65, 85, 88, 96];

#[derive(Debug)]
pub enum CmError {
    /// The sign-in was refused (EResult): sign in again.
    Auth(i64),
    Other(String),
}

impl std::fmt::Display for CmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CmError::Auth(e) => write!(f, "Steam refused the sign-in (EResult {e})"),
            CmError::Other(s) => f.write_str(s),
        }
    }
}

impl From<String> for CmError {
    fn from(s: String) -> Self {
        CmError::Other(s)
    }
}

/// One message: id, header fields, body.
struct Packet {
    emsg: u32,
    header: Vec<u8>,
    body: Vec<u8>,
}

impl Packet {
    fn jobid_target(&self) -> u64 {
        pb::get(&self.header, 11).map_or(u64::MAX, |v| v.int())
    }
    fn eresult(&self) -> i64 {
        pb::get(&self.header, 13).map_or(1, |v| v.int() as i32 as i64)
    }
}

pub struct Cm {
    ws: WebSocket<MaybeTlsStream<TcpStream>>,
    steamid: u64,
    session: i32,
    next_job: u64,
    heartbeat: Duration,
    last_sent: Instant,
    pub cell_id: u32,
}

/// CM servers that speak WebSocket, best first.
fn servers() -> Result<Vec<String>, String> {
    let b = crate::net::get(
        "https://api.steampowered.com/ISteamDirectory/GetCMListForConnect/v1/?cellid=0&cmtype=websockets",
    )?;
    let v: Value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
    let list: Vec<String> = v
        .pointer("/response/serverlist")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|s| s.get("type").and_then(Value::as_str) == Some("websockets"))
        .filter_map(|s| {
            s.get("endpoint")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    if list.is_empty() {
        return Err("no CM servers".into());
    }
    Ok(list)
}

fn connect_one(endpoint: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    use std::net::ToSocketAddrs;
    let addr = endpoint
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("no address")?;
    let tcp =
        TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(Duration::from_secs(30))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(30))).ok();
    let url = format!("wss://{endpoint}/cmsocket/");
    let (ws, _) = tungstenite::client_tls(url, tcp).map_err(|e| e.to_string())?;
    Ok(ws)
}

impl Cm {
    fn connect() -> Result<Cm, String> {
        if std::env::var_os("LENTA_OFFLINE").is_some() {
            return Err("offline".into());
        }
        let mut last = String::from("no CM servers");
        for ep in servers()?.iter().take(5) {
            match connect_one(ep) {
                Ok(ws) => {
                    return Ok(Cm {
                        ws,
                        steamid: 0,
                        session: 0,
                        next_job: 1,
                        heartbeat: Duration::from_secs(9),
                        last_sent: Instant::now(),
                        cell_id: 0,
                    });
                }
                Err(e) => last = format!("{ep}: {e}"),
            }
        }
        Err(last)
    }

    /// Sign in with a refresh token from the client's own sign-in.
    pub fn logon(account: &str, refresh_token: &str) -> Result<Cm, CmError> {
        let mut cm = Cm::connect()?;
        let mut w = Writer::default();
        w.uint(1, PROTOCOL_VERSION)
            .uint(7, 16) // Windows 10, as Steam clients on Linux report too
            .str(6, "english")
            .bool(8, true)
            .uint(33, 2)
            .bool(102, true)
            .str(50, account)
            .str(108, refresh_token);
        let ip = Writer::default()
            .fixed32(1, 0x7f00_0001 ^ 0xBAAD_F00D)
            .done();
        w.bytes(11, &ip);
        cm.steamid = INDIVIDUAL;
        cm.finish_logon(w.done())?;
        Ok(cm)
    }

    /// Signed in as nobody: public depots only (tests).
    pub fn anonymous() -> Result<Cm, CmError> {
        let mut cm = Cm::connect()?;
        cm.steamid = ANON;
        let body = Writer::default()
            .uint(1, PROTOCOL_VERSION)
            .uint(5, 1_561_159_470)
            .done();
        cm.finish_logon(body)?;
        Ok(cm)
    }

    fn finish_logon(&mut self, body: Vec<u8>) -> Result<(), CmError> {
        self.send(LOGON, Writer::default(), body)?;
        let p = self.wait(|p| p.emsg == LOGON_RESPONSE)?;
        let eresult = pb::get(&p.body, 1).map_or(2, |v| v.int() as i32 as i64);
        if eresult != 1 {
            return Err(if AUTH_ERESULTS.contains(&eresult) {
                CmError::Auth(eresult)
            } else {
                CmError::Other(format!("logon: EResult {eresult}"))
            });
        }
        self.steamid = pb::get(&p.header, 1).map_or(self.steamid, |v| v.int());
        self.session = pb::get(&p.header, 2).map_or(0, |v| v.int() as i32);
        let hb = pb::get(&p.body, 3).map_or(9, |v| v.int()).clamp(5, 60);
        self.heartbeat = Duration::from_secs(hb);
        self.cell_id = pb::get(&p.body, 7).map_or(0, |v| v.int() as u32);
        Ok(())
    }

    fn send(&mut self, emsg: u32, mut header: Writer, body: Vec<u8>) -> Result<(), String> {
        if self.steamid != 0 {
            header.fixed64(1, self.steamid);
        }
        if self.session != 0 {
            header.int(2, self.session as i64);
        }
        let h = header.done();
        let mut msg = Vec::with_capacity(8 + h.len() + body.len());
        msg.extend_from_slice(&(emsg | PROTO).to_le_bytes());
        msg.extend_from_slice(&(h.len() as u32).to_le_bytes());
        msg.extend_from_slice(&h);
        msg.extend_from_slice(&body);
        self.ws
            .send(Message::Binary(msg.into()))
            .map_err(|e| e.to_string())?;
        self.last_sent = Instant::now();
        Ok(())
    }

    /// Keep the session: a heartbeat when one is due.
    fn beat(&mut self) -> Result<(), String> {
        if self.last_sent.elapsed() >= self.heartbeat {
            self.send(HEARTBEAT, Writer::default(), Vec::new())?;
        }
        Ok(())
    }

    fn parse(data: &[u8], out: &mut Vec<Packet>) -> Result<(), String> {
        if data.len() < 8 {
            return Ok(());
        }
        let raw = u32::from_le_bytes(data[0..4].try_into().unwrap());
        let emsg = raw & !PROTO;
        if raw & PROTO == 0 {
            // Old-style (non-protobuf) messages: nothing we need.
            return Ok(());
        }
        let hl = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        let header = data.get(8..8 + hl).ok_or("short header")?.to_vec();
        let body = data[8 + hl..].to_vec();
        if emsg == MULTI {
            let size = pb::get(&body, 1).map_or(0, |v| v.int());
            let inner = pb::get(&body, 2)
                .map(|v| v.bytes().to_vec())
                .unwrap_or_default();
            let inner = if size > 0 {
                let mut out = Vec::with_capacity(size as usize);
                flate2::read::GzDecoder::new(&inner[..])
                    .read_to_end(&mut out)
                    .map_err(|e| e.to_string())?;
                out
            } else {
                inner
            };
            let mut at = 0;
            while at + 4 <= inner.len() {
                let n = u32::from_le_bytes(inner[at..at + 4].try_into().unwrap()) as usize;
                at += 4;
                let Some(m) = inner.get(at..at + n) else {
                    break;
                };
                Cm::parse(m, out)?;
                at += n;
            }
            return Ok(());
        }
        out.push(Packet { emsg, header, body });
        Ok(())
    }

    /// Read until a message matches; others are dropped.
    fn wait(&mut self, want: impl Fn(&Packet) -> bool) -> Result<Packet, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Instant::now() > deadline {
                return Err("Steam did not answer".into());
            }
            self.beat()?;
            let msg = match self.ws.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(e) => return Err(format!("connection: {e}")),
            };
            let Message::Binary(data) = msg else { continue };
            let mut got = Vec::new();
            Cm::parse(&data, &mut got)?;
            for p in got {
                if p.emsg == LOGGED_OFF {
                    let e = pb::get(&p.body, 1).map_or(0, |v| v.int());
                    return Err(format!("logged off by Steam (EResult {e})"));
                }
                if want(&p) {
                    return Ok(p);
                }
            }
        }
    }

    fn job(&mut self) -> u64 {
        self.next_job += 1;
        self.next_job
    }

    /// A unified service call ("Interface.Method#1").
    pub fn call(&mut self, method: &str, body: Vec<u8>) -> Result<Vec<u8>, String> {
        let job = self.job();
        let mut h = Writer::default();
        h.fixed64(10, job).str(12, method);
        self.send(SERVICE_CALL, h, body)?;
        let p = self.wait(|p| p.emsg == SERVICE_RESPONSE && p.jobid_target() == job)?;
        match p.eresult() {
            1 => Ok(p.body),
            e => Err(format!("{method}: EResult {e}")),
        }
    }

    pub fn depot_key(&mut self, app: u32, depot: u32) -> Result<Vec<u8>, String> {
        let job = self.job();
        let mut h = Writer::default();
        h.fixed64(10, job);
        let body = Writer::default()
            .uint(1, depot as u64)
            .uint(2, app as u64)
            .done();
        self.send(DEPOT_KEY, h, body)?;
        let p = self.wait(|p| p.emsg == DEPOT_KEY_RESPONSE && p.jobid_target() == job)?;
        let e = pb::get(&p.body, 1).map_or(2, |v| v.int() as i32 as i64);
        if e != 1 {
            return Err(format!("no access to depot {depot} (EResult {e})"));
        }
        pb::get(&p.body, 3)
            .map(|v| v.bytes().to_vec())
            .filter(|k| k.len() == 32)
            .ok_or_else(|| "depot key missing".into())
    }

    pub fn manifest_code(
        &mut self,
        app: u32,
        depot: u32,
        gid: u64,
        branch: &str,
    ) -> Result<u64, String> {
        let mut w = Writer::default();
        w.uint(1, app as u64).uint(2, depot as u64).uint(3, gid);
        if !branch.is_empty() && branch != "public" {
            w.str(4, branch);
        }
        let r = self.call("ContentServerDirectory.GetManifestRequestCode#1", w.done())?;
        Ok(pb::get(&r, 1).map_or(0, |v| v.int()))
    }

    /// "" when the host needs no token.
    pub fn cdn_token(&mut self, app: u32, depot: u32, host: &str) -> Result<String, String> {
        let body = Writer::default()
            .uint(1, depot as u64)
            .str(2, host)
            .uint(3, app as u64)
            .done();
        match self.call("ContentServerDirectory.GetCDNAuthToken#1", body) {
            Ok(r) => Ok(pb::get(&r, 1).map(|v| v.str()).unwrap_or_default()),
            // EResult Fail: this host takes no token.
            Err(e) if e.ends_with("EResult 2") => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    /// A web API access token, as the client gets one.
    pub fn access_token(&mut self, refresh_token: &str) -> Result<String, String> {
        let body = Writer::default()
            .str(1, refresh_token)
            .fixed64(2, self.steamid)
            .uint(3, 0)
            .done();
        let r = self.call("Authentication.GenerateAccessTokenForApp#1", body)?;
        pb::get(&r, 1)
            .map(|v| v.str())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| "no access token".into())
    }

    pub fn close(mut self) {
        let _ = self.ws.close(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Talks to Steam: `cargo test cm_anonymous -- --ignored`.
    #[test]
    #[ignore]
    fn cm_anonymous() {
        let mut cm = Cm::anonymous().expect("anonymous logon");
        // Steamworks Common Redistributables' public depot.
        let key = cm.depot_key(228980, 228990).expect("depot key");
        assert_eq!(key.len(), 32);
        cm.close();
    }
}
