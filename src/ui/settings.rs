//! Settings: a list changed with ← →, and a live preview of the ribbon
//! beside it.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::ribbon::{self, Look};
use super::widgets;
use crate::app::App;
use crate::config::{AccentMode, Anchor, Config, Orientation, Shape, Sort, TiltMode};
use crate::game::Game;
use crate::i18n::{Lang, tr};
use crate::images::{from_hsv, hsv};
use crate::input::Action;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum S {
    Lang,
    Fullscreen,
    Pinned,
    Speed,
    Orientation,
    Flow,
    Anchor,
    Sort,
    Shape,
    Size,
    Spacing,
    SelScale,
    Dim,
    Tilt,
    TiltMode,
    Rounding,
    Reflection,
    Titles,
    AccentMode,
    Color1,
    Color2,
    Glow,
    BgHero,
    BgDim,
    BgTint,
    Steam,
    Heroic,
    Lutris,
    Hydra,
    Own,
    Prefixes,
    FilePortal,
    Rescan,
    Account,
    Uninstalled,
    SteamLibs,
    Sgdb,
    ThemeMusic,
    ThemeMusicRibbon,
    MusicVolume,
    UiSounds,
    SoundPack,
    SoundVolume,
    SoundStore,
}

enum Row {
    Head(&'static str),
    Item(S),
}

const ROWS: &[Row] = &[
    Row::Head("s_ribbon"),
    Row::Item(S::Orientation),
    Row::Item(S::Flow),
    Row::Item(S::Anchor),
    Row::Item(S::Sort),
    Row::Head("s_cards"),
    Row::Item(S::Shape),
    Row::Item(S::Size),
    Row::Item(S::Spacing),
    Row::Item(S::SelScale),
    Row::Item(S::Dim),
    Row::Item(S::Tilt),
    Row::Item(S::TiltMode),
    Row::Item(S::Rounding),
    Row::Item(S::Reflection),
    Row::Item(S::Titles),
    Row::Head("s_accent"),
    Row::Item(S::AccentMode),
    Row::Item(S::Color1),
    Row::Item(S::Color2),
    Row::Item(S::Glow),
    Row::Head("s_back"),
    Row::Item(S::BgHero),
    Row::Item(S::BgDim),
    Row::Item(S::BgTint),
    Row::Head("s_general"),
    Row::Item(S::Lang),
    Row::Item(S::Fullscreen),
    Row::Item(S::Pinned),
    Row::Item(S::Speed),
    Row::Item(S::ThemeMusic),
    Row::Item(S::ThemeMusicRibbon),
    Row::Item(S::MusicVolume),
    Row::Head("s_sounds"),
    Row::Item(S::UiSounds),
    Row::Item(S::SoundPack),
    Row::Item(S::SoundVolume),
    Row::Item(S::SoundStore),
    Row::Head("s_sources"),
    Row::Item(S::Steam),
    Row::Item(S::Heroic),
    Row::Item(S::Lutris),
    Row::Item(S::Hydra),
    Row::Item(S::Own),
    Row::Item(S::Prefixes),
    Row::Item(S::FilePortal),
    Row::Item(S::Rescan),
    Row::Head("s_account"),
    Row::Item(S::Account),
    Row::Item(S::Uninstalled),
    Row::Item(S::SteamLibs),
    Row::Head("s_sgdb"),
    Row::Item(S::Sgdb),
];

fn items() -> Vec<S> {
    ROWS.iter()
        .filter_map(|r| if let Row::Item(s) = r { Some(*s) } else { None })
        .collect()
}

enum Kind {
    Slider { min: f32, max: f32, step: f32 },
    Choice,
    Toggle,
    Color,
    Button,
}

fn kind(s: S) -> Kind {
    let sl = |min, max, step| Kind::Slider { min, max, step };
    match s {
        S::Size => sl(160.0, 460.0, 10.0),
        S::Spacing => sl(0.0, 80.0, 2.0),
        S::SelScale => sl(1.0, 1.8, 0.05),
        S::Dim => sl(0.0, 0.85, 0.05),
        S::Tilt => sl(0.0, 60.0, 2.0),
        S::Rounding => sl(0.0, 40.0, 1.0),
        S::Glow => sl(0.0, 2.0, 0.1),
        S::BgDim => sl(0.0, 0.9, 0.05),
        S::Speed => sl(0.3, 2.5, 0.1),
        S::MusicVolume | S::SoundVolume => sl(0.05, 1.0, 0.05),
        S::Fullscreen
        | S::Pinned
        | S::Reflection
        | S::Titles
        | S::BgHero
        | S::BgTint
        | S::Steam
        | S::Heroic
        | S::Lutris
        | S::Hydra
        | S::Own
        | S::FilePortal
        | S::Uninstalled
        | S::ThemeMusic
        | S::ThemeMusicRibbon
        | S::UiSounds => Kind::Toggle,
        S::Color1 | S::Color2 => Kind::Color,
        S::Rescan | S::Account | S::Sgdb | S::SoundStore | S::SteamLibs | S::Prefixes => {
            Kind::Button
        }
        _ => Kind::Choice,
    }
}

fn label(s: S) -> &'static str {
    tr(match s {
        S::Lang => "lang",
        S::Fullscreen => "fullscreen",
        S::Pinned => "sidebar_pinned",
        S::Speed => "speed",
        S::Orientation => "orientation",
        S::Flow => "flow",
        S::Anchor => "anchor",
        S::Sort => "sort",
        S::Shape => "shape",
        S::Size => "card_size",
        S::Spacing => "spacing",
        S::SelScale => "sel_scale",
        S::Dim => "dim",
        S::Tilt => "tilt",
        S::TiltMode => "tilt_mode",
        S::Rounding => "rounding",
        S::Reflection => "reflection",
        S::Titles => "titles",
        S::AccentMode => "accent_mode",
        S::Color1 => "color1",
        S::Color2 => "color2",
        S::Glow => "glow",
        S::BgHero => "bg_hero",
        S::BgDim => "bg_dim",
        S::BgTint => "bg_tint",
        S::Steam => "src_steam",
        S::Heroic => "src_heroic",
        S::Lutris => "src_lutris",
        S::Hydra => "src_hydra",
        S::Own => "src_own_long",
        S::Prefixes => "pg_prefixes",
        S::FilePortal => "fp_portal",
        S::Rescan => "rescan",
        S::Account => "account",
        S::Uninstalled => "show_uninstalled",
        S::SteamLibs => "mg_libs",
        S::Sgdb => "sgdb_key",
        S::ThemeMusic => "theme_music",
        S::ThemeMusicRibbon => "theme_music_ribbon",
        S::UiSounds => "ui_sounds",
        S::SoundPack => "sound_pack",
        S::SoundVolume => "sound_volume",
        S::SoundStore => "sound_store",
        S::MusicVolume => "music_volume",
    })
}

fn slider_ref(c: &mut Config, s: S) -> Option<&mut f32> {
    Some(match s {
        S::Size => &mut c.card_size,
        S::Spacing => &mut c.spacing,
        S::SelScale => &mut c.selected_scale,
        S::Dim => &mut c.dim,
        S::Tilt => &mut c.tilt,
        S::Rounding => &mut c.rounding,
        S::Glow => &mut c.glow,
        S::BgDim => &mut c.bg_dim,
        S::Speed => &mut c.speed,
        S::MusicVolume => &mut c.music_volume,
        S::SoundVolume => &mut c.sound_volume,
        _ => return None,
    })
}

fn toggle_ref(c: &mut Config, s: S) -> Option<&mut bool> {
    Some(match s {
        S::Fullscreen => &mut c.fullscreen,
        S::Pinned => &mut c.sidebar_pinned,
        S::Reflection => &mut c.reflection,
        S::Titles => &mut c.titles,
        S::BgHero => &mut c.bg_hero,
        S::BgTint => &mut c.bg_tint,
        S::Steam => &mut c.steam,
        S::Heroic => &mut c.heroic,
        S::Lutris => &mut c.lutris,
        S::Hydra => &mut c.hydra,
        S::Own => &mut c.own,
        S::FilePortal => &mut c.file_portal,
        S::Uninstalled => &mut c.show_uninstalled,
        S::ThemeMusic => &mut c.theme_music,
        S::ThemeMusicRibbon => &mut c.theme_music_ribbon,
        S::UiSounds => &mut c.ui_sounds,
        _ => return None,
    })
}

fn color_ref(c: &mut Config, s: S) -> Option<&mut [u8; 3]> {
    match s {
        S::Color1 => Some(&mut c.color1),
        S::Color2 => Some(&mut c.color2),
        _ => None,
    }
}

fn cycle<T: Copy + PartialEq>(all: &[T], cur: T, d: i32) -> T {
    let i = all.iter().position(|x| *x == cur).unwrap_or(0) as i32;
    all[(i + d).rem_euclid(all.len() as i32) as usize]
}

fn choice_step(c: &mut Config, s: S, d: i32) {
    match s {
        S::Lang => c.lang = cycle(&[Lang::Auto, Lang::Ru, Lang::En], c.lang, d),
        S::Orientation => {
            c.orientation = cycle(
                &[Orientation::Horizontal, Orientation::Vertical],
                c.orientation,
                d,
            )
        }
        S::Flow => c.reverse = !c.reverse,
        S::Anchor => c.anchor = cycle(&[Anchor::Center, Anchor::Start], c.anchor, d),
        S::Sort => c.sort = cycle(&[Sort::Recent, Sort::Name, Sort::Playtime], c.sort, d),
        S::Shape => c.shape = cycle(&[Shape::Portrait, Shape::Landscape], c.shape, d),
        S::TiltMode => c.tilt_mode = cycle(&[TiltMode::Flow, TiltMode::Fan], c.tilt_mode, d),
        S::SoundPack => {
            // Steam's own, then the installed packs.
            let mut names = vec![String::new()];
            names.extend(crate::sfx::packs().into_iter().map(|p| p.name));
            let i = names.iter().position(|n| *n == c.sound_pack).unwrap_or(0) as i32;
            c.sound_pack = names[(i + d).rem_euclid(names.len() as i32) as usize].clone();
        }
        S::AccentMode => {
            c.accent_mode = cycle(
                &[
                    AccentMode::Card,
                    AccentMode::CardGradient,
                    AccentMode::Custom,
                    AccentMode::Gradient,
                ],
                c.accent_mode,
                d,
            )
        }
        _ => {}
    }
}

fn choice_text(c: &Config, s: S) -> &'static str {
    tr(match s {
        S::Lang => match c.lang {
            Lang::Auto => "lang_auto",
            Lang::Ru => return "Русский",
            Lang::En => return "English",
        },
        S::Orientation => match c.orientation {
            Orientation::Horizontal => "horizontal",
            Orientation::Vertical => "vertical",
        },
        S::Flow => match (c.orientation, c.reverse) {
            (Orientation::Horizontal, false) => "ltr",
            (Orientation::Horizontal, true) => "rtl",
            (Orientation::Vertical, false) => "ttb",
            (Orientation::Vertical, true) => "btt",
        },
        S::Anchor => match c.anchor {
            Anchor::Center => "anchor_center",
            Anchor::Start => "anchor_start",
        },
        S::Sort => match c.sort {
            Sort::Recent => "sort_recent",
            Sort::Name => "sort_name",
            Sort::Playtime => "sort_time",
        },
        S::Shape => match c.shape {
            Shape::Portrait => "portrait",
            Shape::Landscape => "landscape",
        },
        S::TiltMode => match c.tilt_mode {
            TiltMode::Flow => "tilt_flow",
            TiltMode::Fan => "tilt_fan",
        },
        S::AccentMode => match c.accent_mode {
            AccentMode::Custom => "acc_custom",
            AccentMode::Gradient => "acc_gradient",
            AccentMode::Card => "acc_card",
            AccentMode::CardGradient => "acc_card_grad",
        },
        _ => return "",
    })
}

fn slider_text(s: S, v: f32) -> String {
    match s {
        S::Size | S::Spacing | S::Rounding => format!("{v:.0}"),
        S::SelScale | S::Speed => format!("×{v:.2}"),
        S::Tilt => format!("{v:.0}°"),
        S::Dim | S::BgDim | S::MusicVolume | S::SoundVolume => {
            if v <= 0.001 {
                tr("off").to_string()
            } else {
                format!("{:.0}%", v * 100.0)
            }
        }
        S::Glow => {
            if v <= 0.001 {
                tr("off").to_string()
            } else {
                format!("{:.0}%", v * 100.0)
            }
        }
        _ => format!("{v}"),
    }
}

/// Accent colours to step through with ← →.
const PRESETS: [[u8; 3]; 12] = [
    [0x4c, 0x9a, 0xff],
    [0x2f, 0xd3, 0xc8],
    [0x3d, 0xd6, 0x6b],
    [0xb6, 0xe3, 0x3a],
    [0xff, 0xd0, 0x3a],
    [0xff, 0x93, 0x2f],
    [0xff, 0x52, 0x52],
    [0xff, 0x4f, 0xa3],
    [0xc0, 0x4c, 0xff],
    [0x7a, 0x5c, 0xff],
    [0xe8, 0xe8, 0xf0],
    [0x66, 0xc0, 0xf4],
];

/// Does the setting matter with the others as they are?
fn relevant(c: &Config, s: S) -> bool {
    match s {
        S::Color1 => true,
        S::Color2 => matches!(
            c.accent_mode,
            AccentMode::Gradient | AccentMode::CardGradient
        ),
        S::Reflection => c.orientation == Orientation::Horizontal,
        S::TiltMode => c.tilt > 0.0,
        S::BgDim => c.bg_hero,
        S::Account | S::Uninstalled | S::SteamLibs => c.steam,
        S::Prefixes => c.own,
        S::MusicVolume | S::ThemeMusicRibbon => c.theme_music,
        S::SoundPack | S::SoundVolume => c.ui_sounds,
        _ => true,
    }
}

impl App {
    fn adjust(&mut self, s: S, d: i32) {
        use crate::sfx::Sound;
        match kind(s) {
            Kind::Slider { min, max, step } => {
                if let Some(v) = slider_ref(&mut self.cfg, s) {
                    *v = ((*v + step * d as f32) / step).round() * step;
                    *v = v.clamp(min, max);
                }
                self.sfx.play(if d > 0 {
                    Sound::SliderUp
                } else {
                    Sound::SliderDown
                });
            }
            Kind::Choice => {
                choice_step(&mut self.cfg, s, d);
                self.sfx.play(Sound::Navigate);
            }
            Kind::Toggle => {
                if let Some(v) = toggle_ref(&mut self.cfg, s) {
                    *v = !*v;
                    let on = *v;
                    // Turning the sounds on gives no sound yet: set first.
                    if s == S::UiSounds {
                        self.sfx.set(
                            self.cfg.ui_sounds,
                            &self.cfg.sound_pack,
                            self.cfg.sound_volume,
                        );
                    }
                    self.sfx.play(if on {
                        Sound::ToggleOn
                    } else {
                        Sound::ToggleOff
                    });
                }
            }
            Kind::Color => {
                if let Some(e) = self.set.color_edit {
                    let h = &mut self.set.hsv;
                    match e {
                        0 => h[0] = (h[0] + d as f32 / 72.0).rem_euclid(1.0),
                        1 => h[1] = (h[1] + d as f32 * 0.05).clamp(0.0, 1.0),
                        _ => h[2] = (h[2] + d as f32 * 0.05).clamp(0.1, 1.0),
                    }
                    let c = from_hsv(h[0], h[1], h[2]);
                    if let Some(v) = color_ref(&mut self.cfg, s) {
                        *v = [c.r(), c.g(), c.b()];
                    }
                } else if let Some(v) = color_ref(&mut self.cfg, s) {
                    let i = PRESETS
                        .iter()
                        .position(|p| p == v)
                        .map(|i| i as i32)
                        .unwrap_or(-1);
                    *v = PRESETS[(i + d).rem_euclid(PRESETS.len() as i32) as usize];
                }
            }
            Kind::Button => {}
        }
    }

    fn activate(&mut self, s: S) {
        match kind(s) {
            Kind::Toggle | Kind::Choice => self.adjust(s, 1),
            Kind::Color => {
                if self.set.color_edit.is_some() {
                    self.set.color_edit = None;
                } else if let Some(v) = color_ref(&mut self.cfg, s) {
                    let (h, sat, val) = hsv(Color32::from_rgb(v[0], v[1], v[2]));
                    self.set.hsv = [h, sat, val];
                    self.set.color_edit = Some(0);
                }
            }
            Kind::Button => match s {
                S::Rescan => {
                    self.sfx.play(crate::sfx::Sound::Activate);
                    self.rescan()
                }
                S::Account if self.steam.account.is_some() => self.sign_out(),
                S::Account => self.start_login(),
                S::Sgdb if self.sgdb.job.is_some() => {}
                S::Sgdb if self.sgdb.has_key() => self.sgdb_disconnect(),
                S::Sgdb => self.sgdb_connect(None),
                S::SoundStore => self.go(crate::app::Page::Sounds),
                S::SteamLibs => self.open_libraries(),
                S::Prefixes => self.open_prefixes(),
                _ => {}
            },
            Kind::Slider { .. } => {}
        }
    }

    pub fn settings_input(&mut self, a: Action) -> bool {
        let list = items();
        let s = list[self.set.sel.min(list.len() - 1)];
        if let Some(e) = self.set.color_edit {
            match a {
                Action::Up => self.set.color_edit = Some(e.saturating_sub(1)),
                Action::Down => self.set.color_edit = Some((e + 1).min(2)),
                Action::Left => self.adjust(s, -1),
                Action::Right => self.adjust(s, 1),
                Action::Accept | Action::Back => self.set.color_edit = None,
                _ => {}
            }
            return false;
        }
        match a {
            Action::Up => {
                self.set.sel = self.set.sel.saturating_sub(1);
                self.set.follow = true;
            }
            Action::Down => {
                self.set.sel = (self.set.sel + 1).min(list.len() - 1);
                self.set.follow = true;
            }
            Action::Left => self.adjust(s, -1),
            Action::Right => self.adjust(s, 1),
            Action::Accept => self.activate(s),
            Action::Back => return true,
            _ => {}
        }
        false
    }

    pub fn draw_settings(&mut self, ui: &mut Ui, rect: Rect) {
        let rect = rect.shrink2(vec2(36.0, 0.0));
        let p = ui.painter_at(rect);
        p.text(
            pos2(rect.left(), rect.top() + 34.0),
            Align2::LEFT_CENTER,
            tr("settings"),
            FontId::proportional(32.0),
            Color32::WHITE,
        );
        let list_w = (rect.width() * 0.5).clamp(420.0, 680.0);
        let view = Rect::from_min_max(
            pos2(rect.left(), rect.top() + 76.0),
            pos2(rect.left() + list_w, rect.bottom()),
        );
        let row_h = 54.0;
        let head_h = 50.0;
        let edit_h = 3.0 * 40.0 + 10.0;
        // Where each row is.
        let list = items();
        let sel = self.set.sel.min(list.len() - 1);
        let mut ys = Vec::new();
        let mut y = 0.0;
        let mut sel_span = (0.0, 0.0);
        for r in ROWS {
            match r {
                Row::Head(_) => {
                    ys.push(y);
                    y += head_h;
                }
                Row::Item(s) => {
                    ys.push(y);
                    let mut h = row_h;
                    if list[sel] == *s {
                        if self.set.color_edit.is_some() {
                            h += edit_h;
                        }
                        sel_span = (y, y + h);
                    }
                    y += h;
                }
            }
        }
        let total = y;
        let max = (total - view.height() + 20.0).max(0.0);
        if ui.rect_contains_pointer(view) {
            let d = ui.input(|i| i.smooth_scroll_delta.y);
            if d != 0.0 {
                self.set.scroll = (self.set.scroll - d).clamp(0.0, max);
                self.set.follow = false;
            }
        }
        if self.set.follow || self.set.color_edit.is_some() {
            let want = self
                .set
                .scroll
                .min(sel_span.0 - 60.0)
                .max(sel_span.1 + 60.0 - view.height())
                .clamp(0.0, max);
            let k = self.ease(12.0);
            self.set.scroll += (want - self.set.scroll) * k;
            if (want - self.set.scroll).abs() < 0.5 {
                self.set.scroll = want;
                self.set.follow = false;
            }
        }
        let scroll = self.set.scroll;
        let c1 = self.accent.0;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(view));
        child.set_clip_rect(view);
        let p = child.painter().clone();
        let mut item_i = 0;
        let mut clicked: Option<(usize, i32)> = None;
        let mut drag: Option<(usize, f32)> = None;
        for (ri, r) in ROWS.iter().enumerate() {
            let top = view.top() + ys[ri] - scroll;
            match r {
                Row::Head(k) => {
                    if top > view.bottom() || top + head_h < view.top() {
                        continue;
                    }
                    p.text(
                        pos2(view.left() + 4.0, top + head_h - 12.0),
                        Align2::LEFT_BOTTOM,
                        tr(k),
                        FontId::proportional(15.0),
                        c1,
                    );
                }
                Row::Item(s) => {
                    let i = item_i;
                    item_i += 1;
                    let editing = i == sel && self.set.color_edit.is_some();
                    let h = row_h + if editing { edit_h } else { 0.0 };
                    if top > view.bottom() || top + h < view.top() {
                        continue;
                    }
                    let row = Rect::from_min_size(
                        pos2(view.left(), top),
                        vec2(view.width(), row_h - 6.0),
                    );
                    let resp = child.interact(
                        row.intersect(view),
                        child.id().with(("set", i)),
                        Sense::click_and_drag(),
                    );
                    let focused = i == sel && !self.side_focus;
                    let on = relevant(&self.cfg, *s);
                    if focused {
                        p.rect_filled(
                            Rect::from_min_size(row.min, vec2(row.width(), h - 6.0)),
                            12.0,
                            Color32::from_white_alpha(22),
                        );
                    } else if resp.hovered() {
                        p.rect_filled(row, 12.0, Color32::from_white_alpha(10));
                    }
                    let tc = Color32::from_gray(if !on {
                        120
                    } else if focused {
                        255
                    } else {
                        210
                    });
                    p.text(
                        pos2(row.left() + 20.0, row.center().y),
                        Align2::LEFT_CENTER,
                        label(*s),
                        FontId::proportional(18.0),
                        tc,
                    );
                    let vr = Rect::from_min_max(
                        pos2(row.left() + row.width() * 0.52, row.top()),
                        pos2(row.right() - 16.0, row.bottom()),
                    );
                    self.draw_value(&p, vr, *s, focused, tc);
                    if resp.clicked() || resp.drag_started() {
                        let x = resp.interact_pointer_pos().map(|p| p.x).unwrap_or(0.0);
                        // Left third of the value: back; else forward.
                        let d = if x < vr.left() + vr.width() * 0.25 {
                            -1
                        } else {
                            1
                        };
                        clicked = Some((i, if x >= vr.left() { d } else { 0 }));
                    }
                    if resp.dragged()
                        && let (Kind::Slider { .. }, Some(pos)) =
                            (kind(*s), resp.interact_pointer_pos())
                    {
                        let bar = slider_bar(vr);
                        drag = Some((i, ((pos.x - bar.left()) / bar.width()).clamp(0.0, 1.0)));
                    }
                    if editing {
                        self.draw_color_edit(
                            &child,
                            Rect::from_min_size(
                                pos2(row.left() + 20.0, row.bottom() + 4.0),
                                vec2(row.width() - 36.0, edit_h - 10.0),
                            ),
                            *s,
                        );
                    }
                }
            }
        }
        if let Some((i, d)) = clicked {
            self.side_focus = false;
            if self.set.sel != i {
                self.set.sel = i;
                self.set.color_edit = None;
            }
            let s = list[i];
            match kind(s) {
                Kind::Slider { .. } => {}
                Kind::Color if d == 0 => {}
                Kind::Color => self.activate(s),
                _ if d != 0 => {
                    if matches!(kind(s), Kind::Choice) {
                        self.adjust(s, d)
                    } else {
                        self.activate(s)
                    }
                }
                _ => {}
            }
        }
        if let Some((i, t)) = drag {
            let s = list[i];
            if let Kind::Slider { min, max, step } = kind(s)
                && let Some(v) = slider_ref(&mut self.cfg, s)
            {
                let old = *v;
                *v = ((min + (max - min) * t) / step).round() * step;
                // A tick per step the knob passes, as with the pad.
                if (*v - old).abs() > step * 0.5 {
                    let up = *v > old;
                    self.sfx.play(if up {
                        crate::sfx::Sound::SliderUp
                    } else {
                        crate::sfx::Sound::SliderDown
                    });
                }
            }
        }
        self.draw_preview(
            ui,
            Rect::from_min_max(
                pos2(view.right() + 36.0, rect.top() + 76.0),
                pos2(rect.right(), rect.bottom() - 10.0),
            ),
        );
    }

    fn draw_value(&self, p: &egui::Painter, vr: Rect, s: S, focused: bool, tc: Color32) {
        let (c1, c2) = self.accent;
        let font = FontId::proportional(17.0);
        match kind(s) {
            Kind::Slider { min, max, .. } => {
                let mut c = self.cfg.clone();
                let v = *slider_ref(&mut c, s).unwrap();
                let bar = slider_bar(vr);
                let t = (v - min) / (max - min);
                widgets::bar(p, bar, t, c1, c2);
                p.circle_filled(
                    pos2(bar.left() + bar.width() * t, bar.center().y),
                    if focused { 9.0 } else { 7.0 },
                    Color32::WHITE,
                );
                p.text(
                    pos2(vr.right(), vr.center().y),
                    Align2::RIGHT_CENTER,
                    slider_text(s, v),
                    font,
                    tc,
                );
            }
            Kind::Choice => {
                let text = if s == S::SoundPack {
                    if self.cfg.sound_pack.is_empty() {
                        tr("sounds_steam").to_string()
                    } else {
                        self.cfg.sound_pack.clone()
                    }
                } else {
                    choice_text(&self.cfg, s).to_string()
                };
                p.text(vr.center(), Align2::CENTER_CENTER, text, font.clone(), tc);
                let a = Color32::from_gray(if focused { 230 } else { 110 });
                p.text(
                    pos2(vr.left() + 6.0, vr.center().y),
                    Align2::LEFT_CENTER,
                    "‹",
                    FontId::proportional(24.0),
                    a,
                );
                p.text(
                    pos2(vr.right() - 6.0, vr.center().y),
                    Align2::RIGHT_CENTER,
                    "›",
                    FontId::proportional(24.0),
                    a,
                );
            }
            Kind::Toggle => {
                let mut c = self.cfg.clone();
                let on = *toggle_ref(&mut c, s).unwrap();
                let r = Rect::from_center_size(
                    pos2(vr.right() - 30.0, vr.center().y),
                    vec2(52.0, 28.0),
                );
                if on {
                    super::card::draw(
                        p,
                        &super::card::Pose::flat(r, 14.0),
                        &super::card::Fill::Gradient(c1, c2),
                        Color32::WHITE,
                        None,
                    );
                } else {
                    p.rect_filled(r, 14.0, Color32::from_white_alpha(40));
                }
                let k = if on {
                    r.right() - 14.0
                } else {
                    r.left() + 14.0
                };
                p.circle_filled(pos2(k, r.center().y), 10.0, Color32::WHITE);
                p.text(
                    pos2(r.left() - 14.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    if on { tr("on") } else { tr("off") },
                    font,
                    tc,
                );
            }
            Kind::Color => {
                let mut c = self.cfg.clone();
                let v = *color_ref(&mut c, s).unwrap();
                let col = Color32::from_rgb(v[0], v[1], v[2]);
                let sw = Rect::from_center_size(
                    pos2(vr.right() - 30.0, vr.center().y),
                    vec2(52.0, 28.0),
                );
                p.rect_filled(sw, 8.0, col);
                p.rect_stroke(
                    sw,
                    8.0,
                    Stroke::new(1.5, Color32::from_white_alpha(120)),
                    StrokeKind::Inside,
                );
                p.text(
                    pos2(sw.left() - 14.0, sw.center().y),
                    Align2::RIGHT_CENTER,
                    format!("#{:02X}{:02X}{:02X}", v[0], v[1], v[2]),
                    FontId::monospace(16.0),
                    tc,
                );
                // The presets, small.
                let mut x = vr.left() + 4.0;
                for pr in PRESETS.iter().take(6) {
                    let r = Rect::from_center_size(pos2(x + 7.0, vr.center().y), vec2(14.0, 14.0));
                    p.circle_filled(r.center(), 6.0, Color32::from_rgb(pr[0], pr[1], pr[2]));
                    if *pr == v {
                        p.circle_stroke(r.center(), 8.5, Stroke::new(1.5, Color32::WHITE));
                    }
                    x += 18.0;
                }
            }
            Kind::Button => {
                let text = match s {
                    S::Account => match &self.steam.account {
                        Some(_) => tr("sign_out").to_string(),
                        None => tr("sign_in").to_string(),
                    },
                    S::Sgdb if self.sgdb.job.is_some() => "…".to_string(),
                    S::Sgdb if self.sgdb.has_key == Some(true) => tr("sgdb_disconnect").to_string(),
                    S::Sgdb => tr("sgdb_connect").to_string(),
                    S::SoundStore | S::SteamLibs | S::Prefixes => tr("open").to_string(),
                    _ if self.scanning => "…".to_string(),
                    _ => "↻".to_string(),
                };
                if s == S::Sgdb && self.sgdb.has_key == Some(true) {
                    p.text(
                        pos2(vr.right() - 156.0, vr.center().y),
                        Align2::RIGHT_CENTER,
                        tr("sgdb_connected"),
                        font.clone(),
                        tc,
                    );
                }
                // Who is signed in, beside the button.
                if s == S::Account
                    && let Some(a) = &self.steam.account
                {
                    p.text(
                        pos2(vr.right() - 156.0, vr.center().y),
                        Align2::RIGHT_CENTER,
                        &a.name,
                        font.clone(),
                        tc,
                    );
                }
                let r = Rect::from_min_max(
                    pos2(vr.right() - 140.0, vr.top() + 8.0),
                    pos2(vr.right(), vr.bottom() - 8.0),
                );
                p.rect_filled(
                    r,
                    r.height() / 2.0,
                    if focused {
                        c1.gamma_multiply(0.8)
                    } else {
                        Color32::from_white_alpha(25)
                    },
                );
                p.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(if matches!(s, S::Account | S::Sgdb) {
                        16.0
                    } else {
                        20.0
                    }),
                    Color32::WHITE,
                );
            }
        }
    }

    fn draw_color_edit(&mut self, ui: &Ui, area: Rect, s: S) {
        let p = ui.painter();
        let e = self.set.color_edit.unwrap_or(0);
        let h = self.set.hsv;
        let names = [tr("hue"), tr("sat"), tr("val")];
        for k in 0..3 {
            let r = Rect::from_min_size(
                pos2(area.left(), area.top() + k as f32 * 40.0),
                vec2(area.width(), 34.0),
            );
            p.text(
                pos2(r.left() + 8.0, r.center().y),
                Align2::LEFT_CENTER,
                names[k],
                FontId::proportional(15.0),
                Color32::from_gray(if e == k { 255 } else { 170 }),
            );
            let bar = Rect::from_min_max(
                pos2(r.left() + 150.0, r.center().y - 7.0),
                pos2(r.right() - 10.0, r.center().y + 7.0),
            );
            // The bar shows what the value does.
            let steps = 24;
            for i in 0..steps {
                let t0 = i as f32 / steps as f32;
                let t1 = (i + 1) as f32 / steps as f32;
                let col = |t: f32| match k {
                    0 => from_hsv(t, 0.85, 0.95),
                    1 => from_hsv(h[0], t, h[2]),
                    _ => from_hsv(h[0], h[1], t),
                };
                let seg = Rect::from_min_max(
                    pos2(bar.left() + bar.width() * t0, bar.top()),
                    pos2(bar.left() + bar.width() * t1, bar.bottom()),
                );
                widgets::gradient_rect(p, seg, col(t0), col(t1));
            }
            let x = bar.left() + bar.width() * h[k];
            p.circle_filled(
                pos2(x, bar.center().y),
                if e == k { 10.0 } else { 7.0 },
                Color32::WHITE,
            );
            p.circle_stroke(
                pos2(x, bar.center().y),
                if e == k { 10.0 } else { 7.0 },
                Stroke::new(1.5, Color32::BLACK),
            );
            let resp = ui.interact(
                bar.expand(8.0),
                ui.id().with(("hsv", k)),
                Sense::click_and_drag(),
            );
            if let Some(pos) = resp
                .interact_pointer_pos()
                .filter(|_| resp.dragged() || resp.clicked())
            {
                self.set.color_edit = Some(k);
                self.set.hsv[k] =
                    ((pos.x - bar.left()) / bar.width()).clamp(if k == 2 { 0.1 } else { 0.0 }, 1.0);
                let c = from_hsv(self.set.hsv[0], self.set.hsv[1], self.set.hsv[2]);
                if let Some(v) = color_ref(&mut self.cfg, s) {
                    *v = [c.r(), c.g(), c.b()];
                }
            }
        }
    }

    /// The ribbon as it will look, small.
    fn draw_preview(&mut self, ui: &mut Ui, area: Rect) {
        if area.width() < 200.0 {
            return;
        }
        let p = ui.painter_at(area);
        p.rect_filled(area, 22.0, Color32::from_black_alpha(120));
        p.rect_stroke(
            area,
            22.0,
            Stroke::new(1.0, Color32::from_white_alpha(30)),
            StrokeKind::Inside,
        );
        let mut cfg = self.cfg.clone();
        let k = (area.height() / 900.0).clamp(0.35, 0.8);
        cfg.card_size *= k;
        cfg.spacing *= k;
        cfg.rounding *= k;
        let games: Vec<&Game> = self.games.iter().take(40).collect();
        if games.is_empty() {
            return;
        }
        let sel = self.home.sel.min(games.len() - 1);
        let pulse = ((self.time * 2.2).sin() * 0.5 + 0.5) as f32;
        let band = area.shrink(16.0);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(band));
        child.set_clip_rect(band);
        ribbon::draw(
            &mut child,
            band,
            &cfg,
            &games,
            sel,
            sel as f32,
            &mut self.images,
            &Look {
                accent: self.accent,
                pulse,
            },
        );
    }
}

fn slider_bar(vr: Rect) -> Rect {
    Rect::from_min_max(
        pos2(vr.left() + 6.0, vr.center().y - 3.0),
        pos2(vr.right() - 74.0, vr.center().y + 3.0),
    )
}
