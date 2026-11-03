//! A game from any launcher, as the ribbon and the library show it.

use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Source {
    Steam,
    Heroic,
    Lutris,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Steam, Source::Heroic, Source::Lutris];

    pub fn name(self) -> &'static str {
        match self {
            Source::Steam => "Steam",
            Source::Heroic => "Heroic",
            Source::Lutris => "Lutris",
        }
    }
}

/// A picture: a file on disk, or an address to fetch (and keep) first.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Art {
    File(PathBuf),
    Url(String),
}

#[derive(Clone, Debug, Default)]
pub struct Arts {
    /// Tall cover (600×900).
    pub portrait: Option<Art>,
    /// A smaller cover, when the big one has to be fetched (and may fail).
    pub portrait_alt: Option<Art>,
    /// Wide capsule (460×215, or twice that from the store).
    pub landscape: Option<Art>,
    /// The local one, when the wide capsule has to be fetched.
    pub landscape_alt: Option<Art>,
    /// Background behind the ribbon and the game page.
    pub hero: Option<Art>,
    /// Transparent title logo.
    pub logo: Option<Art>,
    pub icon: Option<Art>,
}

#[derive(Clone, Debug)]
pub struct Game {
    /// Unique across sources: "steam:570", "heroic:legendary:Fortnite", "lutris:12".
    pub key: String,
    pub title: String,
    pub source: Source,
    /// Store or runner within the source: "Epic", "GOG", "wine", "Non-Steam"…
    pub sub: Option<String>,
    pub art: Arts,
    pub playtime_min: Option<u64>,
    /// Unix seconds.
    pub last_played: Option<u64>,
    /// Owned on Steam: its stats and art are Steam's own.
    pub steam_appid: Option<u32>,
    /// The same game in Steam's store, for one from elsewhere: from an
    /// emulator's files, or found by title. Gives description, art and
    /// the achievement list.
    pub store_appid: Option<u32>,
    /// A Steam emulator's achievement save for it.
    pub emu: Option<crate::sources::emu::Save>,
    /// Its Wine prefix, if it runs in one.
    pub prefix: Option<PathBuf>,
    /// Owned but not installed (from the Steam account): Play installs it.
    pub installed: bool,
    /// The id Steam's compatibility tool setting is kept under (Steam
    /// games and shortcuts).
    pub compat_id: Option<u32>,
    pub install_dir: Option<PathBuf>,
    /// Commands to try in turn until one starts.
    pub launch: Vec<Vec<String>>,
}

impl Game {
    /// Where its store page lives: its own Steam id, or the match.
    pub fn info_appid(&self) -> Option<u32> {
        self.steam_appid.or(self.store_appid)
    }

    pub fn play(&self) -> Result<(), String> {
        run_first(&self.launch)
    }
}

/// Run the first of these that starts.
pub fn run_first(cmds: &[Vec<String>]) -> Result<(), String> {
    let mut last = "nothing to run".to_string();
    for argv in cmds {
        let Some((prog, args)) = argv.split_first() else {
            continue;
        };
        match spawn(prog, args) {
            Ok(()) => return Ok(()),
            Err(e) => last = format!("{prog}: {e}"),
        }
    }
    Err(last)
}

pub fn spawn(prog: &str, args: &[String]) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    // Tests drive the real interface on real games: nothing may start.
    if std::env::var_os("LENTA_DRY_RUN").is_some() {
        eprintln!("lenta: dry run, not starting {prog} {args:?}");
        return Ok(());
    }
    let mut cmd = Command::new(prog);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    // Reap it so it never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub fn has_program(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// "2024-05-01T12:34:56.000Z" → unix seconds.
pub fn parse_iso(s: &str) -> Option<u64> {
    let d = s.get(0..10)?;
    let mut it = d.split('-').map(|x| x.parse::<i64>());
    let (y, m, day) = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    let (mut hh, mut mm, mut ss) = (0, 0, 0);
    if let Some(t) = s.get(11..19) {
        let mut it = t.split(':').map(|x| x.parse::<i64>().unwrap_or(0));
        hh = it.next().unwrap_or(0);
        mm = it.next().unwrap_or(0);
        ss = it.next().unwrap_or(0);
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let t = days * 86400 + hh * 3600 + mm * 60 + ss;
    (t >= 0).then_some(t as u64)
}

/// Unix seconds → (year, month, day).
pub fn civil(t: u64) -> (i64, u32, u32) {
    let z = (t / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The local calendar date of a unix time, "dd.mm.yyyy".
pub fn date(t: u64) -> String {
    let tt = t as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&tt, &mut tm) }.is_null() {
        let (y, m, d) = civil(t);
        return format!("{d:02}.{m:02}.{y}");
    }
    format!(
        "{:02}.{:02}.{}",
        tm.tm_mday,
        tm.tm_mon + 1,
        tm.tm_year + 1900
    )
}

/// …and the local time of day, "dd.mm.yyyy hh:mm".
pub fn date_time(t: u64) -> String {
    let tt = t as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&tt, &mut tm) }.is_null() {
        return date(t);
    }
    format!("{} {:02}:{:02}", date(t), tm.tm_hour, tm.tm_min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso("1970-01-02T00:00:00.000Z"), Some(86400));
        assert_eq!(parse_iso("2024-05-01T12:34:56Z"), Some(1714566896));
        assert_eq!(civil(1714566896), (2024, 5, 1));
    }
}
