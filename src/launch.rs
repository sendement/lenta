//! How games start: the launch options (lenta's, kept in step with Steam's
//! and Lutris's) and what modules add (environment, wrappers such as
//! MangoHud).
//!
//! Both sides may change the options. Whoever changed them last wins: a
//! change made in Steam or Lutris is taken in; one made here is written
//! there. Lutris reads its game file at every start, so it's written at
//! once. Steam keeps launch options in memory while it runs and writes its
//! own on exit, so a change made here is written while it's closed; until
//! then the game's start goes through a small script,
//! `~/.local/share/lenta/bin/lenta-run`, put at the front of the game's
//! Steam launch options once. It reads the game's file (rewritten here at
//! every change) and starts the game with lenta's options when Steam's are
//! still what lenta last saw, or with Steam's when they changed there
//! since. Modules' parts go through the same script.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::vdf::{self, Kv};

/// One module's part in a game's start.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Part {
    pub env: BTreeMap<String, String>,
    pub wrap: Vec<String>,
}

/// One game's start as lenta keeps it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Start {
    /// Module id → its part.
    pub parts: BTreeMap<String, Part>,
    /// The launch options as lenta has them (None: not looked at yet).
    pub line: Option<String>,
    /// The launcher's options when last looked at (or written).
    pub seen: String,
    /// lenta's line isn't in the launcher yet.
    pub dirty: bool,
    /// When lenta's line was changed (seconds since 1970).
    pub changed: u64,
    /// The last look took the launcher's change in.
    pub taken: bool,
}

fn base() -> PathBuf {
    ::dirs::data_dir()
        .unwrap_or_else(|| "/tmp".into())
        .join("lenta")
}

fn dry() -> bool {
    std::env::var_os("LENTA_DRY_RUN").is_some()
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn mtime(p: &Path) -> u64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

pub fn script() -> PathBuf {
    base().join("bin/lenta-run")
}

const SCRIPT: &str = r#"#!/bin/bash
# lenta: starts a game the way lenta has it. Steam's launch options call
#   lenta-run <game> <before> --lenta-cmd %command% --lenta-end <after>
# The game's file (written by lenta) may hold lenta's own options, still
# to reach Steam, and what lenta's modules add (environment, wrappers).
# Without the file the game starts with Steam's options, as if this
# weren't here.
id=$1
shift
pre=() cmd=() suf=() part=0
for a in "$@"; do
    case "$part:$a" in
        0:--lenta-cmd) part=1; continue ;;
        1:--lenta-end) part=2; continue ;;
    esac
    case $part in
        0) pre+=("$a") ;;
        1) cmd+=("$a") ;;
        *) suf+=("$a") ;;
    esac
done
# No marks: everything is the game's command.
if [ "$part" = 0 ]; then cmd=("${pre[@]}"); pre=(); fi
LENTA_WRAP= LENTA_LINE= LENTA_SEEN= LENTA_DIRTY=
f="${XDG_DATA_HOME:-$HOME/.local/share}/lenta/launch/$id.sh"
[ -r "$f" ] && . "$f"
steam="${pre[*]}|${suf[*]}"
if [ "$LENTA_DIRTY" = 1 ] && [ "$steam" = "$LENTA_SEEN" ]; then
    # Changed in lenta, not yet in Steam: lenta's options.
    game='$LENTA_WRAP "${cmd[@]}"'
    case "$LENTA_LINE" in
        *%command%*) line=${LENTA_LINE/\%command\%/$game} ;;
        *) line="$game $LENTA_LINE" ;;
    esac
    eval "exec env $line"
fi
# Steam's options (changed there, or the same as lenta's).
exec env "${pre[@]}" $LENTA_WRAP "${cmd[@]}" "${suf[@]}"
"#;

/// The script in place (a dry run writes nothing).
pub fn ensure_script() -> Result<PathBuf, String> {
    let p = script();
    if dry() {
        return Ok(p);
    }
    if std::fs::read_to_string(&p).ok().as_deref() != Some(SCRIPT) {
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, SCRIPT).map_err(|e| e.to_string())?;
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    Ok(p)
}

/// The game's name in file names and in the launch options.
pub fn file_id(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// The script with the game's name: the front of Steam's launch options.
pub fn token(key: &str) -> String {
    format!("{} {}", script().display(), file_id(key))
}

pub fn quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_./-=+,:%@".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Words as a shell splits them: quotes and backslashes taken off.
pub fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\'' => {
                has = true;
                for c in it.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '"' => {
                has = true;
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' if matches!(it.peek(), Some('"' | '\\' | '$' | '`')) => {
                            cur.push(it.next().unwrap())
                        }
                        c => cur.push(c),
                    }
                }
            }
            '\\' => {
                has = true;
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                has = true;
                cur.push(c);
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// The options split around the game: (before, after), as words.
fn sides(line: &str) -> (Vec<String>, Vec<String>) {
    let w = words(line);
    match w.iter().position(|x| x == "%command%") {
        Some(i) => (w[..i].to_vec(), w[i + 1..].to_vec()),
        None => (Vec::new(), w),
    }
}

/// The game's file for the script.
pub fn script_file(s: &Start) -> String {
    let mut out = String::from("# Written by lenta.\n");
    let mut wrap = Vec::new();
    for p in s.parts.values() {
        for (k, v) in &p.env {
            if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !k.is_empty() {
                out += &format!("export {k}={}\n", quote(v));
            }
        }
        // Plain words only: the script splits them on spaces.
        wrap.extend(
            p.wrap
                .iter()
                .filter(|w| {
                    !w.is_empty()
                        && w.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_./-=+,:".contains(c))
                })
                .cloned(),
        );
    }
    out += &format!("LENTA_WRAP=\"{}\"\n", wrap.join(" "));
    let (pre, suf) = sides(&s.seen);
    out += &format!(
        "LENTA_SEEN={}\n",
        quote(&format!("{}|{}", pre.join(" "), suf.join(" ")))
    );
    if s.dirty
        && let Some(l) = &s.line
    {
        out += &format!("LENTA_LINE={}\nLENTA_DIRTY=1\n", quote(l));
    }
    out
}

/// Steam's launch options with the script at the front.
pub fn with_hook(opts: &str, token: &str) -> String {
    let opts = without_hook(opts, token);
    let (pre, suf) = match opts.find("%command%") {
        Some(i) => (opts[..i].trim(), opts[i + "%command%".len()..].trim()),
        None => ("", opts.trim()),
    };
    let mut out = token.to_string();
    for p in [pre, "--lenta-cmd %command% --lenta-end", suf] {
        if !p.is_empty() {
            out.push(' ');
            out.push_str(p);
        }
    }
    out
}

/// …and the user's options again, without it.
pub fn without_hook(opts: &str, token: &str) -> String {
    let Some(rest) = opts.trim().strip_prefix(token) else {
        return opts.trim().to_string();
    };
    let out = rest
        .replace(" --lenta-cmd %command% --lenta-end", " %command%")
        .replace("--lenta-cmd %command% --lenta-end", "%command%")
        .replace("--lenta-cmd ", "")
        .replace(" --lenta-end", "");
    let out = out.trim();
    // Only "%command%" left: as if there were none.
    if out == "%command%" {
        String::new()
    } else {
        out.to_string()
    }
}

pub struct Launch {
    games: BTreeMap<String, Start>,
}

impl Launch {
    pub fn load() -> Launch {
        let games = if dry() {
            BTreeMap::new()
        } else {
            std::fs::read(base().join("launch.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        };
        Launch { games }
    }

    pub fn get(&self, key: &str) -> Option<&Start> {
        self.games.get(key)
    }

    /// Does anything want this game's start changed by modules?
    pub fn wants(&self, key: &str) -> bool {
        self.games.get(key).is_some_and(|s| !s.parts.is_empty())
    }

    /// Does a Steam game need the script at the front of its options?
    pub fn wants_hook(&self, key: &str) -> bool {
        self.games
            .get(key)
            .is_some_and(|s| !s.parts.is_empty() || s.dirty)
    }

    /// A look at the launcher's options (changed there at `when`).
    pub fn sync(&mut self, key: &str, theirs: &str, when: u64) {
        let s = self.games.entry(key.to_string()).or_default();
        let theirs = theirs.trim().to_string();
        if s.line.is_none() {
            s.line = Some(theirs.clone());
            s.seen = theirs;
            return;
        }
        if theirs == s.seen {
            return;
        }
        // Changed there since the last look.
        if s.dirty && s.changed > when {
            // Ours is newer still: it stays, to be written over theirs.
            s.seen = theirs;
            return;
        }
        s.line = Some(theirs.clone());
        s.seen = theirs;
        s.dirty = false;
        s.taken = true;
    }

    /// Changed here.
    pub fn edit(&mut self, key: &str, line: &str) {
        let s = self.games.entry(key.to_string()).or_default();
        let line = line.trim().to_string();
        s.dirty = line != s.seen;
        s.line = Some(line);
        s.changed = now();
        s.taken = false;
    }

    /// lenta's line is in the launcher now.
    pub fn written(&mut self, key: &str, line: &str) {
        if let Some(s) = self.games.get_mut(key) {
            s.seen = line.trim().to_string();
            if s.line.as_deref().map(str::trim) == Some(s.seen.as_str()) {
                s.dirty = false;
            }
        }
    }

    /// A module's part (None: none any more).
    pub fn set_part(&mut self, key: &str, module: &str, part: Option<Part>) {
        let s = self.games.entry(key.to_string()).or_default();
        match part {
            Some(p) => {
                s.parts.insert(module.to_string(), p);
            }
            None => {
                s.parts.remove(module);
            }
        }
    }

    /// Keep it on disk, with the game's file for the script.
    pub fn save(&self, key: &str) -> Result<(), String> {
        if dry() {
            return Ok(());
        }
        ensure_script()?;
        let dir = base().join("launch");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let f = dir.join(format!("{}.sh", file_id(key)));
        match self.games.get(key) {
            Some(s) if !s.parts.is_empty() || s.dirty => {
                std::fs::write(&f, script_file(s)).map_err(|e| e.to_string())?
            }
            _ => {
                let _ = std::fs::remove_file(&f);
            }
        }
        let body = serde_json::to_vec_pretty(&self.games).map_err(|e| e.to_string())?;
        let tmp = base().join("launch.json.tmp");
        std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, base().join("launch.json")).map_err(|e| e.to_string())
    }
}

// ------------------------------------------------------------------ Steam

const APPS: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

fn localconfigs() -> Vec<PathBuf> {
    let Some(root) = crate::sources::steamplay::root() else {
        return Vec::new();
    };
    crate::sources::steam::users(&root)
        .into_iter()
        .map(|(_, d)| d.join("config/localconfig.vdf"))
        .filter(|p| p.is_file())
        .collect()
}

/// A game's launch options as in Steam's file (the first user that has
/// them), and when that file was written.
pub fn steam_options(appid: u32) -> Option<(String, u64)> {
    for path in localconfigs() {
        let Ok(t) = std::fs::read_to_string(&path) else {
            continue;
        };
        let kv = vdf::parse_text(&t);
        if let Some(a) = kv.path(&APPS).and_then(|a| a.get(&appid.to_string())) {
            let o = a.get_str("LaunchOptions").unwrap_or_default().to_string();
            return Some((o, mtime(&path)));
        }
    }
    None
}

/// Is the script at the front of this game's Steam options?
pub fn hooked(appid: u32, key: &str) -> bool {
    steam_options(appid).is_some_and(|(o, _)| o.trim().starts_with(&token(key)))
}

/// A write to Steam's launch options: the user's line (None: as it is)
/// and whether the script goes at the front.
#[derive(Clone, Debug)]
pub struct SteamWrite {
    pub appid: u32,
    pub key: String,
    pub line: Option<String>,
    pub hook: bool,
}

/// Write these games' launch options for every Steam user here. Steam must
/// be closed (it writes the file on exit); a copy of the old file is kept
/// beside it.
pub fn write_steam(changes: &[SteamWrite]) -> Result<(), String> {
    if dry() || changes.is_empty() {
        return Ok(());
    }
    ensure_script()?;
    if crate::sources::steamplay::steam_running() {
        return Err("Steam is running".into());
    }
    for path in localconfigs() {
        write_file(&path, changes)?;
    }
    Ok(())
}

fn write_file(path: &Path, changes: &[SteamWrite]) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut kv = vdf::parse_text(&text);
    let apps = kv
        .path_mut(&APPS)
        .ok_or("localconfig.vdf: unexpected layout")?;
    let mut changed = false;
    for w in changes {
        let tok = token(&w.key);
        let app = apps
            .map_mut(&w.appid.to_string())
            .ok_or("localconfig.vdf: unexpected layout")?;
        let old = app.get_str("LaunchOptions").unwrap_or_default().to_string();
        let line = w.line.clone().unwrap_or_else(|| without_hook(&old, &tok));
        let new = if w.hook { with_hook(&line, &tok) } else { line };
        if new != old {
            app.set("LaunchOptions", Kv::Str(new));
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    let out = kv.to_text();
    if vdf::parse_text(&out) != kv {
        return Err("localconfig.vdf: could not be written safely".into());
    }
    let _ = std::fs::copy(path, path.with_extension("vdf.lenta-backup"));
    let tmp = path.with_extension("vdf.lenta-tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ------------------------------------------------------------------ Lutris

/// A Lutris game's options as one line: its environment, its prefix
/// command, the game, its arguments.
pub fn lutris_line(yml: &serde_yaml::Value) -> String {
    let mut w: Vec<String> = Vec::new();
    if let Some(env) = yml
        .get("system")
        .and_then(|s| s.get("env"))
        .and_then(|e| e.as_mapping())
    {
        for (k, v) in env {
            let (Some(k), v) = (k.as_str(), yaml_str(v)) else {
                continue;
            };
            w.push(format!("{k}={}", quote(&v)));
        }
    }
    if let Some(p) = yml
        .get("system")
        .and_then(|s| s.get("prefix_command"))
        .map(yaml_str)
        .filter(|p| !p.trim().is_empty())
    {
        w.push(p.trim().to_string());
    }
    let args = yml
        .get("game")
        .and_then(|g| g.get("args"))
        .map(yaml_str)
        .unwrap_or_default();
    if w.is_empty() && args.trim().is_empty() {
        return String::new();
    }
    w.push("%command%".into());
    if !args.trim().is_empty() {
        w.push(args.trim().to_string());
    }
    w.join(" ")
}

fn yaml_str(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// The line put back into a Lutris game's config.
pub fn lutris_apply(yml: &mut serde_yaml::Value, line: &str) {
    use serde_yaml::{Mapping, Value};
    let w = words(line);
    let at = w.iter().position(|x| x == "%command%");
    let (before, after) = match at {
        Some(i) => (&w[..i], &w[i + 1..]),
        None => (&w[..0], &w[..]),
    };
    let mut env = Mapping::new();
    let mut i = 0;
    while i < before.len()
        && let Some((k, v)) = before[i].split_once('=')
        && !k.is_empty()
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        env.insert(Value::String(k.into()), Value::String(v.into()));
        i += 1;
    }
    let prefix: Vec<String> = before[i..].iter().map(|x| quote(x)).collect();
    let args: Vec<String> = after.iter().map(|x| quote(x)).collect();
    if !yml.is_mapping() {
        *yml = Value::Mapping(Mapping::new());
    }
    let root = yml.as_mapping_mut().unwrap();
    let sys = root
        .entry(Value::String("system".into()))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    if let Some(sys) = sys.as_mapping_mut() {
        sys.insert(Value::String("env".into()), Value::Mapping(env));
        if prefix.is_empty() {
            sys.remove("prefix_command");
        } else {
            sys.insert(
                Value::String("prefix_command".into()),
                Value::String(prefix.join(" ")),
            );
        }
    }
    let game = root
        .entry(Value::String("game".into()))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    if let Some(game) = game.as_mapping_mut() {
        if args.is_empty() {
            game.remove("args");
        } else {
            game.insert(Value::String("args".into()), Value::String(args.join(" ")));
        }
    }
}

/// A Lutris game's options and when its file was written.
pub fn lutris_options(id: i64) -> Option<(String, u64)> {
    let path = crate::sources::lutris::config_file(id)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let yml: serde_yaml::Value = serde_yaml::from_str(&text).ok()?;
    Some((lutris_line(&yml), mtime(&path)))
}

/// Write a Lutris game's options (it reads them at every start).
pub fn write_lutris(id: i64, line: &str) -> Result<(), String> {
    if dry() {
        return Ok(());
    }
    let path = crate::sources::lutris::config_file(id).ok_or("Lutris game file not found")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut yml: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| e.to_string())?;
    lutris_apply(&mut yml, line);
    let out = serde_yaml::to_string(&yml).map_err(|e| e.to_string())?;
    let _ = std::fs::copy(&path, path.with_extension("yml.lenta-backup"));
    let tmp = path.with_extension("yml.lenta-tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_at_the_front_and_out_clean() {
        let t = "/h/lenta-run steam_1";
        for (before, with) in [
            ("", "/h/lenta-run steam_1 --lenta-cmd %command% --lenta-end"),
            (
                "%command% -dx11",
                "/h/lenta-run steam_1 --lenta-cmd %command% --lenta-end -dx11",
            ),
            (
                "PROTON_LOG=1 gamemoderun %command% -novid",
                "/h/lenta-run steam_1 PROTON_LOG=1 gamemoderun --lenta-cmd %command% --lenta-end -novid",
            ),
            (
                "-novid",
                "/h/lenta-run steam_1 --lenta-cmd %command% --lenta-end -novid",
            ),
        ] {
            let w = with_hook(before, t);
            assert_eq!(w, with, "{before:?}");
            assert_eq!(with_hook(&w, t), w, "once is enough");
            let back = without_hook(&w, t);
            let want = if before == "-novid" {
                "%command% -novid"
            } else {
                before
            };
            assert_eq!(back, want, "{before:?}");
        }
        // Not ours: left alone.
        assert_eq!(
            without_hook(" mako-run %command% ", t),
            "mako-run %command%"
        );
    }

    #[test]
    fn shell_words() {
        assert_eq!(
            words(r#"A='b c' d"e f" g\ h %command% -x"#),
            vec!["A=b c", "de f", "g h", "%command%", "-x"]
        );
        assert_eq!(words("  "), Vec::<String>::new());
        assert_eq!(words("''"), vec![""]);
    }

    #[test]
    fn whoever_changed_last_wins() {
        let mut l = Launch {
            games: BTreeMap::new(),
        };
        // First look: theirs.
        l.sync("g", "A=1 %command%", 100);
        assert_eq!(l.get("g").unwrap().line.as_deref(), Some("A=1 %command%"));
        // Changed here: to be written.
        l.edit("g", "A=2 %command%");
        assert!(l.get("g").unwrap().dirty);
        // They changed it, earlier than we did: ours stays.
        l.sync("g", "A=3 %command%", 0);
        let s = l.get("g").unwrap();
        assert!(s.dirty && s.line.as_deref() == Some("A=2 %command%"));
        assert_eq!(s.seen, "A=3 %command%");
        // Written: in step.
        l.written("g", "A=2 %command%");
        assert!(!l.get("g").unwrap().dirty);
        // They changed it later: taken in.
        l.sync("g", "A=4 %command%", now() + 10);
        let s = l.get("g").unwrap();
        assert!(!s.dirty && s.taken && s.line.as_deref() == Some("A=4 %command%"));
        // Edited back to what they have: nothing to write.
        l.edit("g", "A=4 %command%");
        assert!(!l.get("g").unwrap().dirty);
    }

    #[test]
    fn lutris_round_trip() {
        let mut y: serde_yaml::Value = serde_yaml::from_str(
            "game:\n  exe: /g/x.exe\n  prefix: /g/pfx\nsystem:\n  env: {}\nwine:\n  version: GE\n",
        )
        .unwrap();
        assert_eq!(lutris_line(&y), "");
        let line = "DXVK_HUD=fps 'MY VAR'=x gamemoderun %command% -windowed 'a b'";
        lutris_apply(&mut y, line);
        let back = lutris_line(&y);
        // Not a variable name: stays a word before the game.
        assert_eq!(
            back,
            "DXVK_HUD=fps 'MY VAR=x' gamemoderun %command% -windowed 'a b'"
        );
        assert_eq!(y["game"]["exe"].as_str(), Some("/g/x.exe"));
        assert_eq!(y["wine"]["version"].as_str(), Some("GE"));
        lutris_apply(&mut y, "");
        assert_eq!(lutris_line(&y), "");
        assert!(y["game"].get("args").is_none());
    }

    /// The real script: lenta's line while Steam's is what lenta saw;
    /// Steam's when it changed there; module parts either way.
    #[test]
    fn script_picks_the_newer_options() {
        let dir = std::env::temp_dir().join(format!("lenta-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lenta/launch")).unwrap();
        let run = dir.join("lenta-run");
        std::fs::write(&run, SCRIPT).unwrap();
        let mut s = Start {
            seen: "A=steam %command%".into(),
            line: Some("A='from lenta' %command%".into()),
            dirty: true,
            ..Default::default()
        };
        let mut env = BTreeMap::new();
        env.insert("B".into(), "module".into());
        s.parts.insert("m".into(), Part { env, wrap: vec![] });
        std::fs::write(dir.join("lenta/launch/g_1.sh"), script_file(&s)).unwrap();
        let start = |pre: &[&str]| {
            let out = std::process::Command::new("bash")
                .arg(&run)
                .arg("g_1")
                .args(pre)
                .args(["--lenta-cmd", "sh", "-c", "echo \"$A/$B\"", "--lenta-end"])
                .env("XDG_DATA_HOME", &dir)
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        // Steam still has what lenta saw: lenta's line.
        assert_eq!(start(&["A=steam"]), "from lenta/module");
        // Changed in Steam since: Steam's.
        assert_eq!(start(&["A=changed"]), "changed/module");
        // No file: as Steam has it.
        let out = std::process::Command::new("bash")
            .arg(&run)
            .args([
                "nope",
                "A=x",
                "--lenta-cmd",
                "sh",
                "-c",
                "echo $A",
                "--lenta-end",
            ])
            .env("XDG_DATA_HOME", &dir)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "x");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// On a copy of a real localconfig.vdf (when there is one): on, then
    /// off, and every game's options are as they were.
    #[test]
    fn localconfig_round_trip() {
        let Some(src) = localconfigs().into_iter().next() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("lenta-lc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("localconfig.vdf");
        std::fs::copy(&src, &f).unwrap();
        let kv = vdf::parse_text(&std::fs::read_to_string(&f).unwrap());
        let ids: Vec<u32> = kv
            .path(&APPS)
            .map(|a| {
                a.entries()
                    .iter()
                    .filter_map(|(k, _)| k.parse().ok())
                    .take(40)
                    .collect()
            })
            .unwrap_or_default();
        assert!(!ids.is_empty(), "no apps in the copy");
        let w = |hook| -> Vec<SteamWrite> {
            ids.iter()
                .map(|i| SteamWrite {
                    appid: *i,
                    key: format!("steam:{i}"),
                    line: None,
                    hook,
                })
                .collect()
        };
        write_file(&f, &w(true)).unwrap();
        let mid = vdf::parse_text(&std::fs::read_to_string(&f).unwrap());
        let get = |kv: &Kv, i: &u32| {
            kv.path(&APPS)
                .and_then(|a| a.get(&i.to_string()))
                .and_then(|a| a.get_str("LaunchOptions"))
                .unwrap_or_default()
                .to_string()
        };
        for i in &ids {
            assert!(get(&mid, i).starts_with(&token(&format!("steam:{i}"))));
        }
        write_file(&f, &w(false)).unwrap();
        let after = vdf::parse_text(&std::fs::read_to_string(&f).unwrap());
        for i in &ids {
            assert_eq!(get(&after, i), get(&kv, i).trim(), "app {i}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
