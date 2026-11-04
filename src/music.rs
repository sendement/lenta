//! A game's theme music while its page is open (an idea from
//! NonSteamLaunchers): found on YouTube through an Invidious instance
//! ("<game> Game Theme Music"), its AAC audio fetched through the instance
//! and kept in ~/.cache/lenta/music, then played on a loop, fading in and
//! out. Its own thread; the UI only says which game (or none) is wanted.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use serde_json::Value;

/// The instance NonSteamLaunchers uses; `LENTA_INVIDIOUS` picks another.
fn instance() -> String {
    std::env::var("LENTA_INVIDIOUS").unwrap_or_else(|_| "https://yt.chocolatemoo53.com".into())
}

enum Cmd {
    /// (game key, search title), and seconds to fade in; None: silence.
    Want(Option<(String, String)>, f32),
    Volume(f32),
    /// Another of the songs found for the game playing.
    Next,
}

/// What the page shows: the game and the song's title ("" while looking).
pub type Now = Arc<Mutex<Option<(String, String)>>>;

pub struct Music {
    tx: Sender<Cmd>,
    wanted: Option<String>,
    volume: f32,
    pub now: Now,
}

fn dir() -> PathBuf {
    crate::net::cache_dir().join("music")
}

/// Titles that aren't the music itself.
fn unwanted(t: &str) -> bool {
    let t = t.to_lowercase();
    [
        "how to play",
        "tutorial",
        "guitar lesson",
        "piano lesson",
        "cover",
        "remix",
        "reaction",
        "review",
        "walkthrough",
        "gameplay",
    ]
    .iter()
    .any(|w| t.contains(w))
}

/// The tracks found for a game and the one picked: remembered, else
/// searched (the search once; "another track" moves the pick along).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct Found {
    /// (video id, title)
    videos: Vec<(String, String)>,
    pick: usize,
}

fn memo(key: &str) -> PathBuf {
    dir().join(format!("{:016x}.json", crate::net::hash(key)))
}

fn found(key: &str, title: &str) -> Found {
    if let Some(f) = std::fs::read(memo(key))
        .ok()
        .and_then(|b| serde_json::from_slice::<Found>(&b).ok())
    {
        return f;
    }
    let f = Found {
        videos: search(title),
        pick: 0,
    };
    save(key, &f);
    f
}

fn save(key: &str, f: &Found) {
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(memo(key), serde_json::to_string(f).unwrap_or_default());
}

/// Songs for a game, best first.
fn search(title: &str) -> Vec<(String, String)> {
    let q: String = format!("{title} Game Theme Music")
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let Ok(b) = crate::net::get(&format!("{}/api/v1/search?q={q}&type=video", instance())) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_slice::<Value>(&b) else {
        return Vec::new();
    };
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let len = r.get("lengthSeconds").and_then(Value::as_u64).unwrap_or(0);
            let t = r.get("title").and_then(Value::as_str).unwrap_or_default();
            let a = r.get("author").and_then(Value::as_str).unwrap_or_default();
            // A song, not a stream or a ten-hour loop.
            ((30..=600).contains(&len) && !unwanted(&format!("{t} {a}")))
                .then(|| Some((r.get("videoId")?.as_str()?.to_string(), t.to_string())))
                .flatten()
        })
        .take(12)
        .collect()
}

/// The video to play for a game now: (id, title).
fn find(key: &str, title: &str) -> Option<(String, String)> {
    let f = found(key, title);
    f.videos.get(f.pick % f.videos.len().max(1)).cloned()
}

/// The video's AAC track on disk, fetched through the instance once.
fn audio(video: &str) -> Option<PathBuf> {
    let file = dir().join(format!("{video}.m4a"));
    if file.is_file() {
        return Some(file);
    }
    let b = crate::net::get(&format!("{}/api/v1/videos/{video}?local=true", instance())).ok()?;
    let v: Value = serde_json::from_slice(&b).ok()?;
    let url = v
        .get("adaptiveFormats")?
        .as_array()?
        .iter()
        .find(|f| {
            f.get("itag")
                .map(|i| i.to_string().trim_matches('"') == "140")
                .unwrap_or(false)
        })?
        .get("url")?
        .as_str()?
        .to_string();
    // A song is a few MB; give it time.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(180)))
        .build()
        .into();
    let mut resp = agent.get(&url).call().ok()?;
    let data = resp
        .body_mut()
        .with_config()
        .limit(64 << 20)
        .read_to_vec()
        .ok()?;
    let tmp = file.with_extension("part");
    std::fs::write(&tmp, &data).ok()?;
    std::fs::rename(&tmp, &file).ok()?;
    Some(file)
}

impl Music {
    pub fn new(ctx: egui::Context) -> Music {
        let (tx, rx) = crossbeam_channel::unbounded();
        let now: Now = Arc::default();
        let n = now.clone();
        std::thread::Builder::new()
            .name("lenta-music".into())
            .spawn(move || player(rx, n, ctx))
            .expect("music thread");
        Music {
            tx,
            wanted: None,
            volume: -1.0,
            now,
        }
    }

    /// Play another of the songs found for this game (and keep it).
    pub fn next(&self) {
        let _ = self.tx.send(Cmd::Next);
    }

    /// The game whose theme should play now (None: silence), rising to
    /// full volume over `fade_in` seconds. The same game again changes
    /// nothing (from the ribbon to its page the song goes on).
    pub fn want(&mut self, game: Option<(&str, &str)>, fade_in: f32) {
        let key = game.map(|g| g.0.to_string());
        if key != self.wanted {
            self.wanted = key;
            let _ = self.tx.send(Cmd::Want(
                game.map(|(k, t)| (k.to_string(), t.to_string())),
                fade_in,
            ));
        }
    }

    pub fn volume(&mut self, v: f32) {
        if (v - self.volume).abs() > 0.001 {
            self.volume = v;
            let _ = self.tx.send(Cmd::Volume(v));
        }
    }
}

/// The audio thread: fades between what's playing and what's wanted.
fn player(rx: Receiver<Cmd>, now: Now, ctx: egui::Context) {
    let (found_tx, found_rx) =
        crossbeam_channel::unbounded::<(String, Option<(PathBuf, String)>)>();
    let set_now = |v: Option<(String, String)>| {
        if let Ok(mut n) = now.lock() {
            *n = v;
        }
        ctx.request_repaint();
    };
    let fetch = |key: String, title: String| {
        let tx = found_tx.clone();
        std::thread::spawn(move || {
            let got = find(&key, &title).and_then(|(v, t)| Some((audio(&v)?, t)));
            let _ = tx.send((key, got));
        });
    };
    let mut want_title = String::new();
    let mut fade_in = 1.5f32;
    let mut sink: Option<rodio::MixerDeviceSink> = None;
    // (game, player, level 0…1)
    let mut playing: Option<(String, rodio::Player, f32)> = None;
    let mut fading_out: Vec<(rodio::Player, f32)> = Vec::new();
    let mut wanted: Option<String> = None;
    let mut volume = 0.35f32;
    let mut last = Instant::now();
    loop {
        let busy = playing.as_ref().is_some_and(|p| p.2 < 1.0) || !fading_out.is_empty();
        let wait = if busy {
            Duration::from_millis(30)
        } else {
            Duration::from_millis(500)
        };
        match rx.recv_timeout(wait) {
            Ok(Cmd::Want(w, fade)) => {
                wanted = w.as_ref().map(|(k, _)| k.clone());
                fade_in = fade.max(0.1);
                if playing.as_ref().map(|p| &p.0) != wanted.as_ref() {
                    if let Some((_, p, lvl)) = playing.take() {
                        fading_out.push((p, lvl));
                    }
                    match w {
                        Some((key, title)) => {
                            set_now(Some((key.clone(), String::new())));
                            want_title = title.clone();
                            fetch(key, title);
                        }
                        None => set_now(None),
                    }
                }
            }
            Ok(Cmd::Next) => {
                if let Some(key) = wanted.clone() {
                    let mut f = found(&key, &want_title);
                    if f.videos.len() > 1 {
                        f.pick = (f.pick + 1) % f.videos.len();
                        save(&key, &f);
                        if let Some((_, p, lvl)) = playing.take() {
                            fading_out.push((p, lvl));
                        }
                        set_now(Some((key.clone(), String::new())));
                        fetch(key, want_title.clone());
                    }
                }
            }
            Ok(Cmd::Volume(v)) => volume = v.clamp(0.0, 1.0),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
            Err(_) => {}
        }
        // A track found: start it if that game is still wanted.
        while let Ok((key, got)) = found_rx.try_recv() {
            if wanted.as_deref() != Some(key.as_str()) {
                continue;
            }
            let Some((path, song)) = got else {
                set_now(None);
                continue;
            };
            if playing.is_some() {
                continue;
            }
            if sink.is_none() {
                sink = rodio::DeviceSinkBuilder::open_default_sink()
                    .map(|mut s| {
                        s.log_on_drop(false);
                        s
                    })
                    .ok();
            }
            let Some(s) = &sink else { continue };
            let Ok(file) = std::fs::File::open(&path) else {
                continue;
            };
            let Ok(src) = rodio::Decoder::new_looped(std::io::BufReader::new(file)) else {
                // Not playable: forget it so another is searched next time.
                let _ = std::fs::remove_file(&path);
                continue;
            };
            let p = rodio::Player::connect_new(s.mixer());
            p.set_volume(0.0);
            p.append(src);
            set_now(Some((key.clone(), song)));
            playing = Some((key, p, 0.0));
        }
        // Fades: in over ~1.5 s, out over ~0.8 s.
        let dt = last.elapsed().as_secs_f32();
        last = Instant::now();
        if let Some((_, p, lvl)) = &mut playing {
            *lvl = (*lvl + dt / fade_in).min(1.0);
            p.set_volume(volume * *lvl * *lvl);
        }
        fading_out.retain_mut(|(p, lvl)| {
            *lvl -= dt / 0.8;
            if *lvl <= 0.0 {
                p.stop();
                return false;
            }
            p.set_volume(volume * *lvl * *lvl);
            true
        });
        // Nothing to play for a while: let the sound device go.
        if playing.is_none() && fading_out.is_empty() && wanted.is_none() {
            sink = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Search, fetch and decode a theme (talks to the instance; silent):
    /// `cargo test theme_live -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn theme_live() {
        use rodio::Source;
        unsafe { std::env::set_var("LENTA_CACHE", std::env::temp_dir().join("lenta-music-test")) };
        let (video, _) = find("steam:4358690", "Graveyard Keeper 2").expect("a video");
        let path = audio(&video).expect("audio");
        let size = std::fs::metadata(&path).unwrap().len();
        let dec = rodio::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()))
            .expect("decodes");
        let (rate, ch) = (dec.sample_rate(), dec.channels());
        let samples = dec.take(u32::from(rate) as usize * 5).count();
        eprintln!("{video}: {size} bytes, {rate} Hz × {ch}, decoded {samples} samples");
        assert!(samples > 0);
        // "Another track": the next one found, kept for the game.
        let key = "steam:4358690";
        let mut f = found(key, "Graveyard Keeper 2");
        eprintln!("{} songs found", f.videos.len());
        assert!(f.videos.len() > 1);
        f.pick += 1;
        save(key, &f);
        let (next, title) = find(key, "Graveyard Keeper 2").unwrap();
        assert_ne!(next, video);
        let path = audio(&next).expect("next audio");
        let dec = rodio::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()))
            .unwrap();
        assert!(dec.take(44100).count() > 0);
        eprintln!("next: {title}");
    }

    /// The sound device opens and the player runs the track (at volume 0):
    /// `cargo test device_silent -- --ignored`.
    #[test]
    #[ignore]
    fn device_silent() {
        let dir = std::env::temp_dir().join("lenta-music-test/music");
        let file = std::fs::read_dir(&dir)
            .expect("run theme_live first")
            .flatten()
            .find(|e| e.path().extension().is_some_and(|x| x == "m4a"))
            .expect("a track")
            .path();
        let mut sink = rodio::DeviceSinkBuilder::open_default_sink().expect("sound device");
        sink.log_on_drop(false);
        let p = rodio::Player::connect_new(sink.mixer());
        p.set_volume(0.0);
        p.append(
            rodio::Decoder::new_looped(std::io::BufReader::new(std::fs::File::open(file).unwrap()))
                .unwrap(),
        );
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!p.empty());
        assert!(
            p.get_pos() > Duration::from_millis(500),
            "position {:?}",
            p.get_pos()
        );
        p.stop();
    }
}
