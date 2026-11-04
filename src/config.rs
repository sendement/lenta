//! Settings, kept in ~/.config/lenta/config.toml.

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shape {
    Portrait,
    Landscape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Anchor {
    Center,
    Start,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TiltMode {
    /// Side cards turn towards the selected one.
    Flow,
    /// Every card but the selected turns the same way.
    Fan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccentMode {
    Custom,
    Gradient,
    Card,
    CardGradient,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    Recent,
    Name,
    Playtime,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub lang: Lang,
    pub fullscreen: bool,
    pub sidebar_pinned: bool,
    pub orientation: Orientation,
    /// Right to left / bottom to top.
    pub reverse: bool,
    pub anchor: Anchor,
    pub sort: Sort,
    pub shape: Shape,
    /// Card height (portrait) or width (landscape), in points.
    pub card_size: f32,
    pub spacing: f32,
    pub selected_scale: f32,
    /// 0 — off, up to 0.85.
    pub dim: f32,
    /// Degrees.
    pub tilt: f32,
    pub tilt_mode: TiltMode,
    pub rounding: f32,
    pub reflection: bool,
    pub titles: bool,
    pub accent_mode: AccentMode,
    pub color1: [u8; 3],
    pub color2: [u8; 3],
    /// 0 — none.
    pub glow: f32,
    pub bg_hero: bool,
    pub bg_dim: f32,
    pub bg_tint: bool,
    pub speed: f32,
    pub steam: bool,
    pub heroic: bool,
    pub lutris: bool,
    /// With a Steam account: the owned games that aren't installed, in
    /// the library.
    pub show_uninstalled: bool,
    /// Play a game's theme music while its page is open.
    pub theme_music: bool,
    /// Interface sounds, from Steam's set or a pack.
    pub ui_sounds: bool,
    /// The pack's name ("" = Steam's own sounds).
    pub sound_pack: String,
    pub sound_volume: f32,
    /// …and for the card chosen on the ribbon, after a moment.
    pub theme_music_ribbon: bool,
    pub music_volume: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            lang: Lang::Auto,
            fullscreen: false,
            sidebar_pinned: false,
            orientation: Orientation::Horizontal,
            reverse: false,
            anchor: Anchor::Center,
            sort: Sort::Recent,
            shape: Shape::Portrait,
            card_size: 300.0,
            spacing: 28.0,
            selected_scale: 1.25,
            dim: 0.45,
            tilt: 30.0,
            tilt_mode: TiltMode::Flow,
            rounding: 14.0,
            reflection: true,
            titles: true,
            accent_mode: AccentMode::Card,
            color1: [0x4c, 0x9a, 0xff],
            color2: [0xc0, 0x4c, 0xff],
            glow: 1.0,
            bg_hero: true,
            bg_dim: 0.55,
            bg_tint: true,
            speed: 1.0,
            steam: true,
            heroic: true,
            lutris: true,
            show_uninstalled: true,
            theme_music: false,
            ui_sounds: true,
            sound_pack: String::new(),
            sound_volume: 0.5,
            theme_music_ribbon: true,
            music_volume: 0.35,
        }
    }
}

fn path() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("LENTA_CONFIG") {
        return Some(p.into());
    }
    Some(dirs::config_dir()?.join("lenta/config.toml"))
}

impl Config {
    pub fn load() -> Self {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = toml::to_string_pretty(self) {
            let tmp = p.with_extension("toml.tmp");
            if std::fs::write(&tmp, t).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }
}
