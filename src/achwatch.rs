//! Achievements earned while a game from elsewhere runs: its Steam
//! emulator's save is looked at every two seconds, and each new one is
//! told by a desktop notification (the game is in front, not Lenta).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// A game whose achievements an emulator keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: String,
    pub title: String,
    pub appid: u32,
    /// The save, or where it would show up (the first that's there).
    pub files: Vec<PathBuf>,
}

impl Item {
    fn file(&self) -> Option<&PathBuf> {
        self.files.iter().find(|f| f.is_file())
    }
}

/// What was known of one save.
struct Seen {
    file: Option<PathBuf>,
    stamp: Option<SystemTime>,
    got: HashSet<String>,
}

fn stamp(p: Option<&PathBuf>) -> Option<SystemTime> {
    std::fs::metadata(p?).ok()?.modified().ok()
}

fn earned(p: Option<&PathBuf>) -> HashSet<String> {
    p.map(|p| crate::sources::emu::unlocked(p).into_keys().collect())
        .unwrap_or_default()
}

/// The ids in `now` not in `before` (names compared without case, as
/// emulators write them either way).
pub fn new_ones(before: &HashSet<String>, now: &HashSet<String>) -> Vec<String> {
    let old: HashSet<String> = before.iter().map(|s| s.to_lowercase()).collect();
    let mut v: Vec<String> = now
        .iter()
        .filter(|s| !old.contains(&s.to_lowercase()))
        .cloned()
        .collect();
    v.sort();
    v
}

/// Watch `items` (kept fresh by the caller) while `running` says they run;
/// `earned_in` gets the key of a game that earned something (its page
/// reloads).
pub fn spawn(
    items: Arc<Mutex<Vec<Item>>>,
    running: crate::running::Shared,
    earned_in: impl Fn(String) + Send + 'static,
) {
    let _ = std::thread::Builder::new()
        .name("lenta-achievements".into())
        .spawn(move || {
            let mut seen: HashMap<String, Seen> = HashMap::new();
            loop {
                let list = items.lock().map(|l| l.clone()).unwrap_or_default();
                let on: HashSet<String> = running
                    .lock()
                    .map(|s| s.running.clone())
                    .unwrap_or_default();
                seen.retain(|k, _| list.iter().any(|i| &i.key == k));
                for it in &list {
                    let file = it.file();
                    let st = stamp(file);
                    let fresh_seen = || Seen {
                        file: file.cloned(),
                        stamp: st,
                        got: earned(file),
                    };
                    let s = seen.entry(it.key.clone()).or_insert_with(fresh_seen);
                    // Another save turned up instead (not one appearing
                    // for the first time): start over from it.
                    if s.file.is_some() && s.file.as_ref() != file {
                        *s = fresh_seen();
                        continue;
                    }
                    if s.stamp == st && s.file.as_ref() == file {
                        continue;
                    }
                    s.file = file.cloned();
                    s.stamp = st;
                    let now = earned(file);
                    let fresh = new_ones(&s.got, &now);
                    s.got = now;
                    if fresh.is_empty() {
                        continue;
                    }
                    // Changed while not running (another tool, a copied
                    // save): taken in quietly.
                    if on.contains(&it.key) {
                        tell(it, &fresh);
                    }
                    earned_in(it.key.clone());
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
}

/// A desktop notification per achievement, with its name, description and
/// icon from the store's list when it answers.
fn tell(it: &Item, ids: &[String]) {
    if std::env::var_os("LENTA_DRY_RUN").is_some() {
        return;
    }
    let lang = crate::i18n::steam_lang();
    let list = crate::sources::web::achievement_list(it.appid, lang).unwrap_or_default();
    for id in ids.iter().take(5) {
        let a = list.iter().find(|a| a.id.eq_ignore_ascii_case(id));
        let name = a.map_or(id.clone(), |a| a.name.clone());
        let desc = a.map(|a| a.desc.clone()).unwrap_or_default();
        let icon = a.and_then(|a| a.icon.as_ref()).and_then(icon_file);
        let mut c = std::process::Command::new("notify-send");
        c.args(["-a", "Lenta", "-u", "normal"]);
        match &icon {
            Some(p) => c.arg(format!("--icon={}", p.display())),
            None => c.arg("--icon=io.github.sendement.Lenta"),
        };
        let head = format!("{} · {}", crate::i18n::tr("ach_got"), it.title);
        let body = if desc.is_empty() {
            name
        } else {
            format!("{name}\n{desc}")
        };
        c.arg(head).arg(body);
        let _ = crate::sandbox::host(c)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// The icon as a file the notification daemon can open.
fn icon_file(a: &crate::game::Art) -> Option<PathBuf> {
    match a {
        crate::game::Art::File(p) => Some(p.clone()),
        crate::game::Art::Url(u) => {
            let f = crate::net::cache_dir()
                .join("achievements/icons")
                .join(format!("{:016x}.jpg", crate::net::hash(u)));
            if !f.is_file() {
                let b = crate::net::get(u).ok()?;
                std::fs::create_dir_all(f.parent()?).ok()?;
                std::fs::write(&f, b).ok()?;
            }
            Some(f)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_new_ones() {
        let a: HashSet<String> = ["ACH_A".to_string()].into();
        let b: HashSet<String> = ["ach_a".to_string(), "ACH_B".to_string()].into();
        assert_eq!(new_ones(&a, &b), vec!["ACH_B".to_string()]);
        assert!(new_ones(&b, &a).is_empty());
    }
}

/// The whole way with a real notification on this desktop (run by hand,
/// without LENTA_DRY_RUN): a save shows up while the game "runs".
#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn real_notification() {
        let t = std::env::temp_dir().join(format!("lenta-ach-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        std::fs::create_dir_all(t.join("drive_c/users/steamuser")).unwrap();
        let appid = 4115450;
        let files = crate::sources::emu::candidates(&t, appid);
        let items = Arc::new(Mutex::new(vec![Item {
            key: "own:test".into(),
            title: "Phantom Blade Zero".into(),
            appid,
            files: files.clone(),
        }]));
        let running: crate::running::Shared = Default::default();
        running.lock().unwrap().running.insert("own:test".into());
        let (tx, rx) = std::sync::mpsc::channel();
        spawn(items, running, move |k| {
            let _ = tx.send(k);
        });
        std::thread::sleep(Duration::from_secs(3));
        std::fs::create_dir_all(files[0].parent().unwrap()).unwrap();
        std::fs::write(
            &files[0],
            r#"{"ACH_001":{"earned":true,"earned_time":1764300000}}"#,
        )
        .unwrap();
        let k = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_eq!(k, "own:test");
        let _ = std::fs::remove_dir_all(&t);
    }
}
