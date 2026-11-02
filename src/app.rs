//! The launcher: state, input routing and the frame.

use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use egui::{Color32, Rect, Ui, vec2};

use crate::config::{AccentMode, Config};
use crate::game::{Art, Game};
use crate::i18n::tr;
use crate::images::{Images, Kind};
use crate::input::{Action, Input, PadKind};
use crate::sources::{self, desktop};
use crate::store::Store;
use crate::ui::widgets::{Btn, Glyphs};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Home,
    Library,
    System,
    Settings,
    About,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Home,
        Page::Library,
        Page::System,
        Page::Settings,
        Page::About,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Page::Home => "home",
            Page::Library => "library",
            Page::System => "system",
            Page::Settings => "settings",
            Page::About => "about",
        }
    }
}

#[derive(Default)]
pub struct Ribbon {
    pub sel: usize,
    /// Animated position (index in the middle).
    pub pos: f32,
    pub wheel: f32,
}

#[derive(Default)]
pub struct Grid {
    pub sel: usize,
    pub scroll: f32,
    /// Index into the filter list (0 = all).
    pub filter: usize,
    pub cols: usize,
    /// Keep the selection in view on the next frame.
    pub follow: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Overview,
    Achievements,
    Screenshots,
}

pub struct GameView {
    pub key: String,
    /// 0 = Play, then the tabs.
    pub focus: usize,
    pub tab: Tab,
    pub scroll: f32,
    /// The scroll as drawn (eases towards `scroll`).
    pub shown: f32,
    pub max_scroll: f32,
    /// A screenshot shown big.
    pub shot: Option<usize>,
    pub shot_sel: usize,
    pub opened: Instant,
}

#[derive(Default)]
pub struct SettingsState {
    pub sel: usize,
    /// Editing a colour: which component (hue, saturation, brightness).
    pub color_edit: Option<usize>,
    /// The colour being edited as hue, saturation, brightness.
    pub hsv: [f32; 3],
    pub scroll: f32,
    pub follow: bool,
}

struct Scan {
    games: Vec<Game>,
    apps: Vec<desktop::App>,
}

pub struct App {
    pub ctx: egui::Context,
    pub cfg: Config,
    saved: Config,
    changed_at: Option<Instant>,
    pub games: Vec<Game>,
    pub apps: Vec<desktop::App>,
    pub scanning: bool,
    scan_tx: Sender<Scan>,
    scan_rx: Receiver<Scan>,
    last_scan: Instant,
    pub images: Images,
    pub store: Store,
    pad_rx: Receiver<Input>,
    /// Hints follow the device used last: the pad's own glyphs, or keys.
    pub glyphs: Glyphs,
    /// The kind of the pad in control, kept while the keyboard is in use.
    pad_kind: PadKind,
    last_pad: Option<Instant>,
    pub page: Page,
    pub side_focus: bool,
    pub side_sel: usize,
    pub home: Ribbon,
    pub lib: Grid,
    pub sys: Grid,
    pub game: Option<GameView>,
    pub set: SettingsState,
    pub toast: Option<(String, Instant)>,
    /// The accent as drawn (eases towards the target).
    pub accent: (Color32, Color32),
    /// Backgrounds cross-fading: (art, opacity).
    pub bgs: Vec<((Art, Kind), f32)>,
    last_frame: Option<Instant>,
    pub dt: f32,
    pub time: f64,
    was_focused: bool,
}

impl App {
    pub fn new(ctx: egui::Context) -> Self {
        let cfg = Config::load();
        crate::i18n::set(cfg.lang);
        let (pad_tx, pad_rx) = crossbeam_channel::unbounded();
        let c2 = ctx.clone();
        crate::input::spawn_gamepad(pad_tx, move || c2.request_repaint());
        let (scan_tx, scan_rx) = crossbeam_channel::unbounded();
        let mut app = App {
            images: Images::new(ctx.clone()),
            store: Store::new(ctx.clone()),
            ctx,
            saved: cfg.clone(),
            cfg,
            changed_at: None,
            games: Vec::new(),
            apps: Vec::new(),
            scanning: false,
            scan_tx,
            scan_rx,
            last_scan: Instant::now(),
            pad_rx,
            glyphs: Glyphs::Keys,
            pad_kind: PadKind::Xbox,
            last_pad: None,
            page: Page::Home,
            side_focus: false,
            side_sel: 0,
            home: Ribbon::default(),
            lib: Grid::default(),
            sys: Grid::default(),
            game: None,
            set: SettingsState::default(),
            toast: None,
            accent: (
                Color32::from_rgb(0x4c, 0x9a, 0xff),
                Color32::from_rgb(0xc0, 0x4c, 0xff),
            ),
            bgs: Vec::new(),
            last_frame: None,
            dt: 0.016,
            time: 0.0,
            was_focused: true,
        };
        app.rescan();
        app
    }

    pub fn rescan(&mut self) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.last_scan = Instant::now();
        let cfg = self.cfg.clone();
        let tx = self.scan_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let apps = std::thread::spawn(desktop::apps);
            let games = sources::scan(&cfg);
            let _ = tx.send(Scan {
                games,
                apps: apps.join().unwrap_or_default(),
            });
            ctx.request_repaint();
        });
    }

    fn take_scan(&mut self) {
        while let Ok(s) = self.scan_rx.try_recv() {
            self.scanning = false;
            let keep = self.games.get(self.home.sel).map(|g| g.key.clone());
            self.games = s.games;
            sources::sort(&mut self.games, self.cfg.sort);
            self.apps = s.apps;
            if let Some(k) = keep
                && let Some(i) = self.games.iter().position(|g| g.key == k)
            {
                self.home.sel = i;
                self.home.pos = i as f32;
            }
            self.home.sel = self.home.sel.min(self.games.len().saturating_sub(1));
            self.store.forget_achievements();
        }
    }

    pub fn resort(&mut self) {
        let keep = self.games.get(self.home.sel).map(|g| g.key.clone());
        sources::sort(&mut self.games, self.cfg.sort);
        if let Some(i) = keep.and_then(|k| self.games.iter().position(|g| g.key == k)) {
            self.home.sel = i;
            self.home.pos = i as f32;
        }
    }

    pub fn selected_game(&self) -> Option<&Game> {
        self.games.get(self.home.sel)
    }

    pub fn game_by_key(&self, key: &str) -> Option<&Game> {
        self.games.iter().find(|g| g.key == key)
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }

    pub fn play(&mut self, key: &str) {
        let Some(g) = self.game_by_key(key) else {
            return;
        };
        let title = g.title.clone();
        match g.play() {
            Ok(()) => self.toast(format!("{} {title}…", tr("launching"))),
            Err(e) => self.toast(format!("{} {title}: {e}", tr("launch_failed"))),
        }
    }

    pub fn open_game(&mut self, key: String) {
        self.game = Some(GameView {
            key,
            focus: 0,
            tab: Tab::Overview,
            scroll: 0.0,
            shown: 0.0,
            max_scroll: 0.0,
            shot: None,
            shot_sel: 0,
            opened: Instant::now(),
        });
    }

    pub fn go(&mut self, page: Page) {
        self.page = page;
        self.side_sel = Page::ALL.iter().position(|p| *p == page).unwrap_or(0);
    }

    /// The accent wanted now: set colours, or the selected card's.
    fn target_accent(&mut self) -> (Color32, Color32) {
        let own = (
            Color32::from_rgb(self.cfg.color1[0], self.cfg.color1[1], self.cfg.color1[2]),
            Color32::from_rgb(self.cfg.color2[0], self.cfg.color2[1], self.cfg.color2[2]),
        );
        let from_card = |app: &mut App| -> Option<(Color32, Color32)> {
            let key = match &app.game {
                Some(v) => v.key.clone(),
                None => app.selected_game()?.key.clone(),
            };
            let g = app.games.iter().find(|g| g.key == key)?.clone();
            let arts = crate::ui::ribbon::arts(&g, app.cfg.shape);
            app.images
                .first(&arts, Kind::Fit(crate::ui::ribbon::CARD_PX))
                .map(|t| (t.color, t.color2))
                .or_else(|| {
                    Some(crate::ui::ribbon::own_colors(&g.title)).map(|(a, b)| (lift(a), lift(b)))
                })
        };
        match self.cfg.accent_mode {
            AccentMode::Custom => (own.0, own.0),
            AccentMode::Gradient => own,
            AccentMode::Card => from_card(self)
                .map(|c| (c.0, c.0))
                .unwrap_or((own.0, own.0)),
            AccentMode::CardGradient => from_card(self).unwrap_or(own),
        }
    }

    fn handle_keys(&mut self, ui: &Ui) -> Vec<Action> {
        let mut out = Vec::new();
        ui.input(|i| {
            for e in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                {
                    use egui::Key as K;
                    let a = match key {
                        K::ArrowUp => Action::Up,
                        K::ArrowDown => Action::Down,
                        K::ArrowLeft => Action::Left,
                        K::ArrowRight => Action::Right,
                        K::Enter => Action::Accept,
                        K::Escape | K::Backspace => Action::Back,
                        K::Space => Action::Select,
                        K::F => Action::Actions,
                        K::Tab => Action::View,
                        K::F2 => Action::Menu,
                        K::Q => Action::PrevTab,
                        K::E => Action::NextTab,
                        K::PageUp => Action::LeftTrigger,
                        K::PageDown => Action::RightTrigger,
                        K::Home => Action::Home,
                        K::End => Action::End,
                        K::F11 => {
                            self.cfg.fullscreen = !self.cfg.fullscreen;
                            continue;
                        }
                        _ => continue,
                    };
                    if modifiers.alt || modifiers.ctrl || modifiers.command {
                        continue;
                    }
                    out.push(a);
                }
            }
        });
        out
    }

    pub(crate) fn handle(&mut self, a: Action) {
        if self.game.is_some() {
            self.game_input(a);
            return;
        }
        match a {
            Action::PrevTab | Action::NextTab => {
                let i = Page::ALL.iter().position(|p| *p == self.page).unwrap_or(0) as i32;
                let n = Page::ALL.len() as i32;
                let j = (i + if a == Action::NextTab { 1 } else { -1 }).rem_euclid(n);
                self.go(Page::ALL[j as usize]);
                self.side_focus = false;
                self.set.color_edit = None;
                return;
            }
            Action::View => {
                self.side_focus = !self.side_focus;
                self.side_sel = Page::ALL.iter().position(|p| *p == self.page).unwrap_or(0);
                return;
            }
            Action::Menu => {
                self.go(Page::Settings);
                self.side_focus = false;
                return;
            }
            _ => {}
        }
        if self.side_focus {
            match a {
                Action::Up => self.side_sel = self.side_sel.saturating_sub(1),
                Action::Down => self.side_sel = (self.side_sel + 1).min(Page::ALL.len() - 1),
                Action::Accept => {
                    self.go(Page::ALL[self.side_sel]);
                    self.side_focus = false;
                }
                Action::Back => self.side_focus = false,
                _ => {}
            }
            // Moving in the panel shows the page at once.
            if matches!(a, Action::Up | Action::Down) {
                self.page = Page::ALL[self.side_sel];
            }
            return;
        }
        // The side panel opens only with View (Tab); Back on a page goes
        // home.
        let back = match self.page {
            Page::Home => self.home_input(a),
            Page::Library => self.library_input(a),
            Page::System => self.system_input(a),
            Page::Settings => self.settings_input(a),
            Page::About => a == Action::Back,
        };
        if back && self.page != Page::Home {
            self.go(Page::Home);
        }
    }

    pub fn hint_items(&self) -> Vec<(Btn, &'static str)> {
        if let Some(v) = &self.game {
            if v.shot.is_some() {
                return vec![(Btn::Dpad, tr("scroll")), (Btn::B, tr("back"))];
            }
            let mut v = vec![(Btn::A, if v.focus == 0 { tr("play") } else { tr("open") })];
            if self.game.as_ref().is_some_and(|g| g.focus != 0) {
                v.push((Btn::X, tr("play")));
            }
            v.extend([
                (Btn::Dpad, tr("scroll")),
                (Btn::LT, tr("tab")),
                (Btn::B, tr("back")),
            ]);
            return v;
        }
        if self.side_focus {
            return vec![(Btn::A, tr("open")), (Btn::B, tr("back"))];
        }
        let mut v = match self.page {
            Page::Home => vec![(Btn::A, tr("open")), (Btn::X, tr("play"))],
            Page::Library => vec![
                (Btn::A, tr("open")),
                (Btn::X, tr("play")),
                (Btn::LT, tr("filter")),
            ],
            Page::System => vec![(Btn::A, tr("launch")), (Btn::LT, tr("filter"))],
            Page::Settings => vec![(Btn::Dpad, tr("change")), (Btn::B, tr("back"))],
            Page::About => vec![],
        };
        v.push((Btn::LB, tr("pages")));
        v.push((Btn::View, tr("menu")));
        v
    }

    fn frame_pacing(&mut self) {
        // At most ~120 frames a second (no vsync: see main.rs).
        const FRAME: Duration = Duration::from_micros(8_333);
        if let Some(t) = self.last_frame {
            if let Some(rest) = FRAME.checked_sub(t.elapsed()) {
                std::thread::sleep(rest);
            }
            self.dt = t.elapsed().as_secs_f32().min(0.1);
        }
        self.last_frame = Some(Instant::now());
        self.time += self.dt as f64;
    }

    fn save_later(&mut self) {
        if self.cfg != self.saved {
            if self.cfg.lang != self.saved.lang {
                crate::i18n::set(self.cfg.lang);
                self.store.clear();
            }
            if self.cfg.fullscreen != self.saved.fullscreen {
                self.ctx
                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.cfg.fullscreen));
            }
            if self.cfg.sort != self.saved.sort {
                self.resort();
            }
            let sources_changed = (self.cfg.steam, self.cfg.heroic, self.cfg.lutris)
                != (self.saved.steam, self.saved.heroic, self.saved.lutris);
            self.saved = self.cfg.clone();
            self.changed_at = Some(Instant::now());
            if sources_changed {
                self.rescan();
            }
        }
        if self
            .changed_at
            .is_some_and(|t| t.elapsed() > Duration::from_millis(600))
        {
            self.changed_at = None;
            self.cfg.save();
        }
    }

    /// Something still moves: keep drawing.
    fn animating(&self) -> bool {
        (self.home.pos - self.home.sel as f32).abs() > 0.001
            || self.bgs.iter().any(|(_, a)| *a < 0.999)
            || self.bgs.len() > 1
            || self.toast.is_some()
            || self.scanning
    }

    pub fn ease(&self, rate: f32) -> f32 {
        1.0 - (-self.dt * rate * self.cfg.speed.max(0.1)).exp()
    }
}

/// A dull colour made bright enough to be an accent.
fn lift(c: Color32) -> Color32 {
    let (h, s, _) = crate::images::hsv(c);
    crate::images::from_hsv(h, s.max(0.5), 0.92)
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.frame_pacing();
        let ctx = ui.ctx().clone();
        self.take_scan();
        self.images.pump();
        self.store.pump();
        // The pad is read globally; never act on it while another window
        // (a game) has focus.
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let inputs: Vec<Input> = self.pad_rx.try_iter().collect();
        for i in inputs {
            match i {
                Input::Pad(k) => {
                    self.pad_kind = k;
                    self.glyphs = Glyphs::Pad(k);
                }
                Input::Press(a) if focused => {
                    self.glyphs = Glyphs::Pad(self.pad_kind);
                    self.last_pad = Some(Instant::now());
                    self.handle(a)
                }
                _ => {}
            }
        }
        let keys = self.handle_keys(ui);
        // Keys right after a pad press may be the pad itself (Steam's
        // desktop layout turns it into a keyboard): hints stay on the pad.
        if !keys.is_empty()
            && self
                .last_pad
                .is_none_or(|t| t.elapsed() > Duration::from_millis(400))
        {
            self.glyphs = Glyphs::Keys;
        }
        for a in keys {
            self.handle(a);
        }
        // Back from a game: play time and achievements may have changed.
        if focused && !self.was_focused && self.last_scan.elapsed() > Duration::from_secs(30) {
            self.rescan();
        }
        self.was_focused = focused;
        if self
            .toast
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(4))
        {
            self.toast = None;
        }

        let target = self.target_accent();
        let k = self.ease(6.0);
        self.accent = (
            crate::ui::card::lerp(self.accent.0, target.0, k),
            crate::ui::card::lerp(self.accent.1, target.1, k),
        );
        if self.accent != target {
            ctx.request_repaint();
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(10, 11, 16)))
            .show(ui, |ui| {
                let full = ui.max_rect();
                self.draw_background(ui, full);
                let hints_h = 52.0;
                if self.game.is_some() {
                    let body = Rect::from_min_max(
                        full.min,
                        egui::pos2(full.right(), full.bottom() - hints_h),
                    );
                    self.draw_game_page(ui, body);
                } else {
                    let side_w = crate::ui::sidebar::COLLAPSED;
                    let left = if self.cfg.sidebar_pinned {
                        crate::ui::sidebar::EXPANDED
                    } else {
                        side_w
                    };
                    let body = Rect::from_min_max(
                        egui::pos2(full.left() + left, full.top()),
                        egui::pos2(full.right(), full.bottom() - hints_h),
                    );
                    match self.page {
                        Page::Home => self.draw_home(ui, body),
                        Page::Library => self.draw_library(ui, body),
                        Page::System => self.draw_system(ui, body),
                        Page::Settings => self.draw_settings(ui, body),
                        Page::About => self.draw_about(ui, body),
                    }
                }
                let hints =
                    Rect::from_min_max(egui::pos2(full.left(), full.bottom() - hints_h), full.max);
                let p = ui.painter();
                crate::ui::widgets::vgradient(
                    p,
                    hints,
                    Color32::from_black_alpha(0),
                    Color32::from_black_alpha(140),
                );
                crate::ui::widgets::hints(
                    p,
                    hints.shrink2(vec2(16.0, 10.0)),
                    &self.hint_items(),
                    self.glyphs,
                );
                self.draw_status(ui, hints);
                if self.game.is_none() {
                    // Over the hints bar too: its veil reaches the bottom.
                    self.draw_sidebar(
                        ui,
                        Rect::from_min_max(
                            full.min,
                            egui::pos2(full.left() + crate::ui::sidebar::EXPANDED, full.bottom()),
                        ),
                        hints_h,
                    );
                }
                self.draw_toast(ui, full);
            });
        self.save_later();
        if self.animating() {
            ctx.request_repaint();
        } else if focused && self.cfg.glow > 0.0 && self.game.is_none() {
            // The selected card's glow breathes; never while a game is up.
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}
