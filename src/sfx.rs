//! Interface sounds: Steam's own (steamui/sounds) or a Decky AudioLoader
//! pack. A pack is a folder of Steam's sound names (deck_ui_navigation.wav…)
//! with a pack.json: `ignore` silences sounds, `mappings` gives a sound
//! several files to pick from; anything else missing is Steam's. Sounds are
//! decoded once and played from memory on their own thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crossbeam_channel::Sender;
use serde_json::Value;

/// What happened, as Steam names its sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Navigate,
    Activate,
    IntoDetail,
    OutOfDetail,
    Launch,
    Tab,
    MenuIn,
    MenuOut,
    ToggleOn,
    ToggleOff,
    SliderUp,
    SliderDown,
    ShowModal,
    HideModal,
}

impl Sound {
    pub fn file(self) -> &'static str {
        match self {
            Sound::Navigate => "deck_ui_navigation.wav",
            Sound::Activate => "deck_ui_default_activation.wav",
            Sound::IntoDetail => "deck_ui_into_game_detail.wav",
            Sound::OutOfDetail => "deck_ui_out_of_game_detail.wav",
            Sound::Launch => "deck_ui_launch_game.wav",
            Sound::Tab => "deck_ui_tab_transition_01.wav",
            Sound::MenuIn => "deck_ui_side_menu_fly_in.wav",
            Sound::MenuOut => "deck_ui_side_menu_fly_out.wav",
            Sound::ToggleOn => "deck_ui_switch_toggle_on.wav",
            Sound::ToggleOff => "deck_ui_switch_toggle_off.wav",
            Sound::SliderUp => "deck_ui_slider_up.wav",
            Sound::SliderDown => "deck_ui_slider_down.wav",
            Sound::ShowModal => "deck_ui_show_modal.wav",
            Sound::HideModal => "deck_ui_hide_modal.wav",
        }
    }
}

/// An installed pack.
#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    pub name: String,
    pub author: String,
    pub dir: PathBuf,
    /// The store's id, when it came from there.
    pub id: Option<String>,
    ignore: Vec<String>,
    mappings: HashMap<String, Vec<String>>,
}

/// Where our packs go.
pub fn packs_dir() -> PathBuf {
    dirs::data_dir()
        .map(|d| d.join("lenta/sounds"))
        .unwrap_or_else(|| "/tmp/lenta-sounds".into())
}

/// Steam's own sounds.
fn steam_sounds() -> Option<PathBuf> {
    crate::sources::steam::roots()
        .into_iter()
        .map(|r| r.join("steamui/sounds"))
        .find(|p| p.is_dir())
}

fn read_pack(dir: &Path) -> Option<Pack> {
    let v: Value = serde_json::from_slice(&std::fs::read(dir.join("pack.json")).ok()?).ok()?;
    // Music packs (menu music) aren't interface sounds.
    if v.get("music").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let strs = |a: Option<&Value>| -> Vec<String> {
        a.and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(Pack {
        name: v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        author: v
            .get("author")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        id: v.get("id").and_then(Value::as_str).map(str::to_string),
        dir: dir.to_path_buf(),
        ignore: strs(v.get("ignore")),
        mappings: v
            .get("mappings")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), strs(Some(v)))).collect())
            .unwrap_or_default(),
    })
}

/// Installed packs: ours, and Decky AudioLoader's own on this machine.
pub fn packs() -> Vec<Pack> {
    let mut dirs = vec![packs_dir()];
    if let Some(h) = dirs::home_dir() {
        dirs.push(h.join("homebrew/sounds"));
    }
    let mut out: Vec<Pack> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            if let Some(p) = read_pack(&e.path())
                && !out.iter().any(|o| o.name == p.name)
            {
                out.push(p);
            }
        }
    }
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// The file a sound plays from (None: silent).
fn resolve(pack: Option<&Pack>, sound: Sound, pick: usize) -> Option<PathBuf> {
    let name = sound.file();
    if let Some(p) = pack {
        if let Some(m) = p.mappings.get(name).filter(|m| !m.is_empty()) {
            return Some(p.dir.join(&m[pick % m.len()]));
        }
        let own = p.dir.join(name);
        if own.is_file() {
            return Some(own);
        }
        if p.ignore.iter().any(|i| i == name) {
            return None;
        }
    }
    steam_sounds().map(|d| d.join(name)).filter(|f| f.is_file())
}

enum Cmd {
    Play(Sound),
    Pack(Option<String>),
    Volume(f32),
}

pub struct Sfx {
    tx: Sender<Cmd>,
    pack: Option<String>,
    volume: f32,
    pub on: bool,
    /// A sound was asked for since `take_played` (the frame's watcher
    /// adds none then).
    played: std::cell::Cell<bool>,
    /// What was asked for (tests: the sounds are off there).
    #[cfg(test)]
    pub heard: std::cell::RefCell<Vec<Sound>>,
}

impl Sfx {
    pub fn new() -> Sfx {
        let (tx, rx) = crossbeam_channel::unbounded::<Cmd>();
        std::thread::Builder::new()
            .name("lenta-sfx".into())
            .spawn(move || {
                let mut sink: Option<rodio::MixerDeviceSink> = None;
                let mut pack: Option<Pack> = None;
                let mut volume = 0.5f32;
                let mut cache: HashMap<PathBuf, rodio::buffer::SamplesBuffer> = HashMap::new();
                let mut n = 0usize;
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::Pack(name) => {
                            pack = name.and_then(|n| packs().into_iter().find(|p| p.name == n));
                            cache.clear();
                        }
                        Cmd::Volume(v) => volume = v,
                        Cmd::Play(s) => {
                            n += 1;
                            let Some(file) = resolve(pack.as_ref(), s, n) else {
                                continue;
                            };
                            if !cache.contains_key(&file) {
                                let Some(b) = decode(&file) else { continue };
                                cache.insert(file.clone(), b);
                            }
                            if sink.is_none() {
                                sink = rodio::DeviceSinkBuilder::open_default_sink()
                                    .map(|mut s| {
                                        s.log_on_drop(false);
                                        s
                                    })
                                    .ok();
                            }
                            if let (Some(sk), Some(b)) = (&sink, cache.get(&file)) {
                                use rodio::Source;
                                sk.mixer().add(b.clone().amplify(volume));
                            }
                        }
                    }
                }
            })
            .expect("sfx thread");
        Sfx {
            tx,
            pack: None,
            volume: -1.0,
            on: false,
            played: Default::default(),
            #[cfg(test)]
            heard: Default::default(),
        }
    }

    /// Was anything played since the last call?
    pub fn take_played(&self) -> bool {
        self.played.replace(false)
    }

    pub fn play(&self, s: Sound) {
        self.played.set(true);
        #[cfg(test)]
        self.heard.borrow_mut().push(s);
        if self.on {
            let _ = self.tx.send(Cmd::Play(s));
        }
    }

    /// Keep the thread in step with the settings.
    pub fn set(&mut self, on: bool, pack: &str, volume: f32) {
        self.on = on && std::env::var_os("LENTA_DRY_RUN").is_none();
        let p = (!pack.is_empty()).then(|| pack.to_string());
        if p != self.pack {
            self.pack = p.clone();
            let _ = self.tx.send(Cmd::Pack(p));
        }
        if (volume - self.volume).abs() > 0.001 {
            self.volume = volume;
            let _ = self.tx.send(Cmd::Volume(volume));
        }
    }
}

fn decode(f: &Path) -> Option<rodio::buffer::SamplesBuffer> {
    use rodio::Source;
    let file = std::fs::File::open(f).ok()?;
    let dec = rodio::Decoder::new(std::io::BufReader::new(file)).ok()?;
    let (ch, rate) = (dec.channels(), dec.sample_rate());
    let samples: Vec<rodio::Sample> = dec.collect();
    Some(rodio::buffer::SamplesBuffer::new(ch, rate, samples))
}

// ------------------------------------------------------------------ store

const STORE: &str = "https://api.deckthemes.com";

/// A pack in the DeckThemes store.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub id: String,
    pub name: String,
    pub author: String,
    pub downloads: u64,
    pub blob: String,
    pub image: Option<String>,
}

/// Every sound pack in the store, most downloaded first (page by page).
pub fn store_all() -> Result<Vec<Listed>, String> {
    let mut all: Vec<Listed> = Vec::new();
    for page in 1..=20 {
        let got = store(page)?;
        let last = got.len() < 50;
        for l in got {
            if !all.iter().any(|a| a.id == l.id) {
                all.push(l);
            }
        }
        if last {
            break;
        }
    }
    Ok(all)
}

/// One page of 50 sound packs, most downloaded first.
pub fn store(page: u32) -> Result<Vec<Listed>, String> {
    let b = crate::net::get(&format!(
        "{STORE}/themes?page={page}&perPage=50&filters=AUDIO.Audio&order=Most%20Downloaded"
    ))?;
    let v: Value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
    Ok(v.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("target").and_then(Value::as_str) == Some("Audio"))
        .filter_map(|t| {
            Some(Listed {
                id: t.get("id")?.as_str()?.to_string(),
                name: t
                    .get("displayName")
                    .or_else(|| t.get("name"))?
                    .as_str()?
                    .to_string(),
                author: t
                    .get("specifiedAuthor")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                downloads: t
                    .pointer("/download/downloadCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                blob: t.pointer("/download/id")?.as_str()?.to_string(),
                image: t
                    .pointer("/images/0/id")
                    .and_then(Value::as_str)
                    .map(|i| format!("{STORE}/blobs/{i}")),
            })
        })
        .collect())
}

/// Download a pack's zip and unpack it into our packs folder; returns the
/// pack's name.
pub fn install(l: &Listed) -> Result<String, String> {
    install_into(l, &packs_dir()).map(|p| p.name)
}

fn install_into(l: &Listed, dest: &Path) -> Result<Pack, String> {
    let b = crate::net::get(&format!("{STORE}/blobs/{}", l.blob))?;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(b)).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    // Unpack beside, then move in, so a broken zip leaves nothing.
    let stage = dest.join(format!(".lenta-unpack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    z.extract(&stage).map_err(|e| e.to_string())?;
    let top = std::fs::read_dir(&stage)
        .map_err(|e| e.to_string())?
        .flatten()
        .find(|e| e.path().join("pack.json").is_file())
        .map(|e| e.path())
        .ok_or("the zip holds no pack")?;
    let mut pack = read_pack(&top).ok_or("not a sound pack")?;
    let target = dest.join(top.file_name().ok_or("bad pack")?);
    if target.exists() {
        let _ = std::fs::remove_dir_all(&target);
    }
    std::fs::rename(&top, &target).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&stage);
    pack.dir = target;
    Ok(pack)
}

/// Remove one of our packs (not Decky's).
pub fn remove(p: &Pack) -> Result<(), String> {
    let base = packs_dir().canonicalize().map_err(|e| e.to_string())?;
    let d = p.dir.canonicalize().map_err(|e| e.to_string())?;
    if d.parent() != Some(base.as_path()) {
        return Err("not one of the launcher's packs".into());
    }
    std::fs::remove_dir_all(&d).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_rules() {
        let dir = std::env::temp_dir().join(format!("lenta-pack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("deck_ui_navigation.wav"), b"x").unwrap();
        std::fs::write(dir.join("a.wav"), b"x").unwrap();
        std::fs::write(dir.join("b.wav"), b"x").unwrap();
        std::fs::write(
            dir.join("pack.json"),
            r#"{"name":"T","author":"me","music":false,"ignore":["deck_ui_hide_modal.wav"],"mappings":{"deck_ui_default_activation.wav":["a.wav","b.wav"]}}"#,
        )
        .unwrap();
        let p = read_pack(&dir).unwrap();
        assert_eq!(
            resolve(Some(&p), Sound::Navigate, 0),
            Some(dir.join("deck_ui_navigation.wav"))
        );
        assert_eq!(resolve(Some(&p), Sound::HideModal, 0), None);
        assert_eq!(
            resolve(Some(&p), Sound::Activate, 1),
            Some(dir.join("b.wav"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The store lists packs and one installs into a temp HOME-free place
    /// (talks to deckthemes.com): `cargo test sound_store -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn sound_store() {
        let list = store_all().unwrap();
        eprintln!(
            "{} packs, first: {:?}",
            list.len(),
            list.first().map(|l| &l.name)
        );
        assert!(list.len() > 10);
        // The top pack installs (into a temp folder) and all its sounds
        // decode.
        let tmp = std::env::temp_dir().join(format!("lenta-sounds-{}", std::process::id()));
        let p = install_into(&list[0], &tmp).unwrap();
        let mut n = 0;
        for e in std::fs::read_dir(&p.dir).unwrap().flatten() {
            let f = e.path();
            if f.extension()
                .is_some_and(|x| x == "wav" || x == "mp3" || x == "ogg" || x == "flac")
            {
                assert!(decode(&f).is_some(), "{}", f.display());
                n += 1;
            }
        }
        eprintln!("{}: {n} sounds decode", p.name);
        assert!(n > 5);
        let _ = std::fs::remove_dir_all(&tmp);
        // Decoding Steam's own sound works.
        let f = steam_sounds().unwrap().join(Sound::Navigate.file());
        assert!(decode(&f).is_some());
    }
}
