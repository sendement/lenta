//! The launcher: state, input routing and the frame.

use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use egui::{Color32, Rect, Ui, vec2};

use crate::config::{AccentMode, Config};
use crate::game::{Art, Game, Source};
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
    Proton,
    Sounds,
    /// A module's own section (its place in the modules' list).
    Module(u16),
    /// Modules: on/off, how each is shown; opens those kept off the panel.
    Modules,
    Settings,
    About,
}

impl Page {
    pub fn key(self) -> &'static str {
        match self {
            Page::Home => "home",
            Page::Library => "library",
            Page::System => "system",
            Page::Proton => "proton",
            Page::Sounds => "sound_store",
            Page::Module(_) => "modules",
            Page::Modules => "modules",
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
    /// Held and dragged with the mouse: the ribbon follows the pointer.
    pub dragging: bool,
    /// Cards a second while dragged, for the flick on release.
    pub vel: f32,
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
    Compat,
    Versions,
    Art,
    /// The launch options, in step with the launcher's.
    Launch,
    /// A module's tab (its place in the modules' list).
    Module(u16),
}

impl Tab {
    /// A number for each (the sound watcher compares them).
    pub fn no(self) -> usize {
        match self {
            Tab::Overview => 0,
            Tab::Achievements => 1,
            Tab::Screenshots => 2,
            Tab::Compat => 3,
            Tab::Versions => 4,
            Tab::Art => 5,
            Tab::Launch => 6,
            Tab::Module(i) => 7 + i as usize,
        }
    }
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
    /// The row in the compatibility tab (and the other lists).
    pub tool_sel: usize,
    /// Hidden achievements shown on request (by id).
    pub revealed: std::collections::HashSet<String>,
    /// A was pressed on the chosen achievement: show/hide it.
    pub reveal: bool,
    /// The choice moved by keys: scroll it into view once (the wheel is
    /// left alone otherwise).
    pub follow: bool,
    /// The header folded away by scrolling down (kept until scrolled up
    /// at the top: the page gets taller as it folds, so it can't follow
    /// the scroll itself).
    pub folded: bool,
    /// The tab's content is taller than the page with the header open.
    pub overflow: bool,
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

/// Where the choice is, for the sound watcher: whether a window is up,
/// a game page open, the side panel, the page and the game's tab; then
/// every selection.
#[derive(PartialEq, Clone, Default)]
struct NavMark {
    modal: bool,
    game: bool,
    side: bool,
    page: usize,
    tab: usize,
    sel: Vec<i64>,
}

struct Scan {
    games: Vec<Game>,
    apps: Vec<desktop::App>,
    steam: crate::ui::steam::SteamData,
    shelves: sources::Shelves,
    /// The final word (after the network pass).
    last: bool,
}

pub struct App {
    pub ctx: egui::Context,
    pub cfg: Config,
    saved: Config,
    changed_at: Option<Instant>,
    /// Installed games, for the ribbon (the hidden collection left out).
    pub games: Vec<Game>,
    /// Launchers' categories and soundtracks, for the library.
    pub shelves: sources::Shelves,
    /// Where the library is: its side tab, focus, an open collection.
    pub libv: crate::ui::library::LibView,
    /// Steam's collections, tools and the account's other games.
    pub steam: crate::ui::steam::SteamData,
    /// Signing in to Steam, while its window is up.
    pub login: Option<crate::ui::steam::Login>,
    /// Getting SteamGridDB's key.
    pub sgdb: crate::ui::sgdb::SgdbUi,
    /// The theme music player.
    pub music: crate::music::Music,
    /// Interface sounds.
    pub sfx: crate::sfx::Sfx,
    /// The sound packs page.
    pub sounds: crate::ui::sounds::SoundsPage,
    /// External modules and their pages as shown.
    pub mods: crate::ui::modules::Mods,
    /// Games' launch options and what modules add to their starts.
    pub launch: crate::launch::Launch,
    /// The game whose options were last looked at (the Launch tab).
    pub start_seen: Option<String>,
    /// Games running and what Steam does with them (a thread looks).
    pub running: crate::running::Shared,
    /// The games that thread looks at.
    watch: std::sync::Arc<std::sync::Mutex<Vec<crate::running::Watch>>>,
    /// Games whose emulator keeps achievements, watched while they run.
    ach_watch: std::sync::Arc<std::sync::Mutex<Vec<crate::achwatch::Item>>>,
    /// Games that earned an achievement since last drawn.
    pub ach_earned: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// Running when last drawn (one ending brings lenta back).
    pub was_running: std::collections::HashSet<String>,
    /// A sideways touchpad swipe: how far it went, and whether it has
    /// already turned the tab (one tab per swipe).
    swipe: (f32, bool, f64),
    /// Play pressed: the game and when (until it shows up running).
    pub launching: Option<(String, Instant)>,
    /// The Proton page.
    pub proton: crate::ui::proton::ProtonPage,
    /// A question before an action that can't be undone.
    pub confirm: Option<crate::ui::confirm::Confirm>,
    /// Installing, moving, uninstalling: the place picker and the job.
    pub manage: crate::ui::manage::Manage,
    pub modals: crate::ui::choose::Modals,
    /// The launcher's own collections.
    pub colls: crate::colls::Colls,
    /// Typing a text (the on-screen keyboard).
    pub osk: Option<crate::ui::osk::Osk>,
    /// A context menu.
    pub menu: Option<crate::ui::menu::Menu>,
    /// Making or changing an own collection.
    pub cedit: Option<crate::ui::colledit::CollEdit>,
    /// The card chosen on the ribbon and since when (the theme waits).
    home_pick: (Option<String>, Instant),
    /// The Look tab's SteamGridDB pictures.
    pub art: crate::ui::artpick::ArtPick,
    /// The Versions tab: builds, a download, a rollback.
    pub versions: crate::ui::versions::Versions,
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
    /// The background's glow colours: the accent, or a launcher's own on
    /// its library tab; and how far they are the launcher's (0…1).
    pub tone: (Color32, Color32, f32),
    /// Backgrounds cross-fading: (art, opacity).
    pub bgs: Vec<((Art, Kind), f32)>,
    last_frame: Option<Instant>,
    pub dt: f32,
    pub time: f64,
    was_focused: bool,
    /// The choice as the last frame left it.
    nav: NavMark,
    /// The last input of any kind (the watcher stays quiet otherwise: a
    /// rescan moving the choice makes no sound).
    last_input: Instant,
}

impl App {
    pub fn new(ctx: egui::Context) -> Self {
        let cfg = Config::load();
        crate::i18n::set(cfg.lang);
        crate::keyboard::set_enabled(&crate::keyboard::default_codes(if crate::i18n::ru() {
            "ru"
        } else {
            "en"
        }));
        if crate::i18n::ru() {
            crate::keyboard::set_last_layout(1);
        }
        let (pad_tx, pad_rx) = crossbeam_channel::unbounded();
        let c2 = ctx.clone();
        crate::input::spawn_gamepad(pad_tx, move || c2.request_repaint());
        let (scan_tx, scan_rx) = crossbeam_channel::unbounded();
        let watch: std::sync::Arc<std::sync::Mutex<Vec<crate::running::Watch>>> =
            Default::default();
        let rctx = ctx.clone();
        let running_shared = crate::running::spawn(watch.clone(), move || rctx.request_repaint());
        let ach_watch: std::sync::Arc<std::sync::Mutex<Vec<crate::achwatch::Item>>> =
            Default::default();
        let ach_earned: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>> =
            Default::default();
        {
            let (earned, actx) = (ach_earned.clone(), ctx.clone());
            crate::achwatch::spawn(ach_watch.clone(), running_shared.clone(), move |k| {
                if let Ok(mut e) = earned.lock() {
                    e.insert(k);
                }
                actx.request_repaint();
            });
        }
        let music_ctx = ctx.clone();
        let mut app = App {
            ach_watch,
            ach_earned,
            images: Images::new(ctx.clone()),
            store: Store::new(ctx.clone()),
            ctx,
            saved: cfg.clone(),
            cfg,
            changed_at: None,
            games: Vec::new(),
            steam: Default::default(),
            shelves: Default::default(),
            libv: Default::default(),
            login: None,
            versions: Default::default(),
            sgdb: Default::default(),
            art: Default::default(),
            music: crate::music::Music::new(music_ctx),
            home_pick: (None, Instant::now()),
            confirm: None,
            manage: Default::default(),
            modals: Default::default(),
            colls: crate::colls::Colls::load(),
            osk: None,
            menu: None,
            cedit: None,
            proton: Default::default(),
            sfx: crate::sfx::Sfx::new(),
            sounds: Default::default(),
            mods: crate::ui::modules::Mods::load(),
            launch: crate::launch::Launch::load(),
            start_seen: None,
            running: running_shared,
            watch,
            was_running: Default::default(),
            swipe: (0.0, false, 0.0),
            launching: None,
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
            tone: (
                Color32::from_rgb(0x4c, 0x9a, 0xff),
                Color32::from_rgb(0xc0, 0x4c, 0xff),
                0.0,
            ),
            bgs: Vec::new(),
            last_frame: None,
            dt: 0.016,
            time: 0.0,
            was_focused: true,
            nav: NavMark::default(),
            last_input: Instant::now(),
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
            let mut games = sources::scan(&cfg);
            let apps = apps.join().unwrap_or_default();
            let mut steam = crate::ui::steam::SteamData::local(&cfg);
            // Like steam-downgrader at start: remember installs seen (they
            // stay rollback targets) and keep locks ahead of new builds.
            if cfg.steam
                && std::env::var_os("LENTA_DRY_RUN").is_none()
                && let Some(root) = sources::steam::roots().into_iter().next()
            {
                crate::downgrade::ops::snapshot_installed(&root);
                for (name, old, new) in crate::downgrade::ops::relock(&root) {
                    eprintln!("lenta: {name}: locked build follows Steam {old} → {new}");
                }
            }
            // What's on disk shows at once; store matches, the account's
            // other games and art follow.
            let shelves = sources::shelves(&cfg);
            let _ = tx.send(Scan {
                games: games.clone(),
                apps: apps.clone(),
                steam: steam.clone(),
                shelves: shelves.clone(),
                last: false,
            });
            ctx.request_repaint();
            sources::web::enrich(&mut games);
            steam.online(&cfg, &mut games);
            let _ = tx.send(Scan {
                games,
                apps,
                steam,
                shelves,
                last: true,
            });
            ctx.request_repaint();
        });
    }

    fn take_scan(&mut self) {
        while let Ok(s) = self.scan_rx.try_recv() {
            if s.last {
                self.scanning = false;
            }
            let keep = self.games.get(self.home.sel).map(|g| g.key.clone());
            // Games in the hidden collection stay out of the ribbon.
            let hidden = s.steam.hidden();
            let mut s = s;
            sources::web::apply_picks(&mut s.games);
            sources::web::apply_picks(&mut s.steam.more);
            // Launchers' games not installed join Steam's owned ones.
            let (games, absent): (Vec<Game>, Vec<Game>) =
                s.games.into_iter().partition(|g| g.installed);
            let (shown, hidden): (Vec<Game>, Vec<Game>) = games
                .into_iter()
                .partition(|g| g.steam_appid.is_none_or(|id| !hidden.contains(&id)));
            self.games = shown;
            if let Ok(mut w) = self.watch.lock() {
                *w = self
                    .games
                    .iter()
                    .filter(|g| g.installed)
                    .map(|g| crate::running::Watch {
                        key: g.key.clone(),
                        appid: g.steam_appid,
                        dir: g.install_dir.clone(),
                    })
                    .collect();
            }
            if let Ok(mut w) = self.ach_watch.lock() {
                *w = self
                    .games
                    .iter()
                    .filter(|g| g.installed && g.steam_appid.is_none())
                    .filter_map(|g| {
                        let e = g.emu.as_ref();
                        let appid = e.map(|e| e.appid).or(g.store_appid)?;
                        let mut files: Vec<std::path::PathBuf> =
                            e.map(|e| e.file.clone()).into_iter().collect();
                        // An own program's emulator writes its first
                        // save during play.
                        if g.source == Source::Own
                            && let Some(p) = &g.prefix
                        {
                            files.extend(crate::sources::emu::candidates(p, appid));
                        }
                        (!files.is_empty()).then(|| crate::achwatch::Item {
                            key: g.key.clone(),
                            title: g.title.clone(),
                            appid,
                            files,
                        })
                    })
                    .collect();
            }
            sources::sort(&mut self.games, self.cfg.sort);
            // Changes still waiting on Steam outlive the rescan.
            let pending = std::mem::take(&mut self.steam.pending);
            let hooks = std::mem::take(&mut self.steam.hooks);
            let restart = self.steam.restart.take();
            let more = std::mem::take(&mut self.steam.more);
            let first = !s.last;
            self.steam = s.steam;
            for (id, t) in &pending {
                match t {
                    Some(t) => self.steam.mapping.insert(*id, t.clone()),
                    None => self.steam.mapping.remove(id),
                };
            }
            self.steam.pending = pending;
            self.steam.hooks = hooks;
            self.steam.restart = restart;
            // The quick pass has no account games yet: keep the last ones.
            if first && self.steam.more.is_empty() {
                self.steam.more = more
                    .into_iter()
                    .filter(|g| !g.installed && g.source == Source::Steam)
                    .collect();
            }
            self.steam.more.extend(hidden);
            self.steam.more.extend(absent);
            if s.last {
                self.apply_pending_tools(false);
                if let Some(e) = self.steam.error.take() {
                    self.toast(format!("{}: {e}", tr("owned_failed")));
                }
            }
            self.apps = s.apps;
            self.shelves = s.shelves;
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
        self.games
            .iter()
            .chain(&self.steam.more)
            .find(|g| g.key == key)
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }

    pub fn play(&mut self, key: &str) {
        // Running: Play is Close.
        if self.is_running(key) {
            self.ask_stop(key);
            return;
        }
        let Some(g) = self.game_by_key(key) else {
            return;
        };
        // A Steam game not installed: which library it goes in first.
        if !g.installed && g.key.starts_with("steam:") && g.steam_appid.is_some() {
            let k = g.key.clone();
            self.manage_install(&k);
            return;
        }
        // Own: its program gone, its settings to point at another; else
        // run with the umu id of its Steam match (protonfixes), noted as
        // played.
        if g.source == Source::Own {
            let key = g.key.clone();
            if !g.installed {
                self.toast(tr("pg_exe_missing"));
                self.program_settings(&key);
                return;
            }
            let id = crate::progs::gameid(g.store_appid);
            let cmds: Vec<Vec<String>> = g
                .launch
                .iter()
                .map(|c| {
                    c.iter()
                        .map(|a| {
                            if a == "GAMEID=umu-default" {
                                format!("GAMEID={id}")
                            } else {
                                a.clone()
                            }
                        })
                        .collect()
                })
                .collect();
            let title = g.title.clone();
            self.sfx.play(crate::sfx::Sound::Launch);
            match crate::game::run_first(&cmds) {
                Ok(()) => {
                    if let Some(pid) = crate::progs::id_of(&key) {
                        let _ = crate::progs::edit(|s| {
                            if let Some(p) = s.programs.iter_mut().find(|p| p.id == pid) {
                                p.last_played = crate::progs::now();
                            }
                        });
                    }
                    if let Some(g) = self.games.iter_mut().find(|g| g.key == key) {
                        g.last_played = Some(crate::progs::now());
                    }
                    self.launching = Some((key, Instant::now()));
                    self.toast(format!("{} {title}…", tr("launching")));
                }
                Err(e) => self.toast(format!("{} {title}: {e}", tr("launch_failed"))),
            }
            return;
        }
        // Hydra's: nothing downloaded, nothing to do; a download is
        // resumed or installed in Hydra's window (from outside, its
        // torrent service loses track of it).
        if !g.installed && g.source == Source::Hydra {
            match g.dl {
                None => self.toast(tr("dl_get_in_hydra")),
                Some(_) => match crate::game::run_first(&[vec!["hydralauncher".into()]]) {
                    Ok(()) => self.toast(format!("{} Hydra", tr("opening"))),
                    Err(e) => self.toast(format!("{} Hydra: {e}", tr("launch_failed"))),
                },
            }
            return;
        }
        // Another launcher's game: it installs it, in its own window.
        if !g.installed && g.source != Source::Steam {
            let msg = format!("{} {}", tr("opening"), g.source.name());
            match g.play() {
                Ok(()) => self.toast(msg),
                Err(e) => self.toast(format!("{} {}: {e}", tr("launch_failed"), g.title)),
            }
            return;
        }
        let title = g.title.clone();
        self.sfx.play(crate::sfx::Sound::Launch);
        let verb = if g.installed {
            tr("launching")
        } else {
            tr("installing")
        };
        // The options as they should be, before the launcher starts it.
        let key = g.key.clone();
        self.start_sync(&key);
        self.start_push(&key);
        let Some(g) = self.game_by_key(&key) else {
            return;
        };
        match g.play() {
            Ok(()) => {
                self.launching = Some((key.clone(), Instant::now()));
                self.toast(format!("{verb} {title}…"))
            }
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
            tool_sel: 0,
            revealed: Default::default(),
            reveal: false,
            follow: false,
            folded: false,
            overflow: false,
            opened: Instant::now(),
        });
    }

    pub fn go(&mut self, page: Page) {
        self.page = page;
        self.side_sel = self.pages().iter().position(|p| *p == page).unwrap_or(0);
    }

    /// The side panel's sections: modules' own ones before Modules.
    pub fn pages(&self) -> Vec<Page> {
        let mut v = vec![
            Page::Home,
            Page::Library,
            Page::System,
            Page::Proton,
            Page::Sounds,
        ];
        for (i, m) in self.mods.list.iter().enumerate() {
            if self.mod_section(m) {
                v.push(Page::Module(i as u16));
            }
        }
        v.extend([Page::Modules, Page::Settings, Page::About]);
        v
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
                .first(&arts, crate::ui::ribbon::card_kind())
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
        // Typing: the keys are text.
        if self.osk.is_some() {
            self.osk_keys(ui);
            return out;
        }
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

    /// A sideways two-finger swipe off the ribbon turns the tab, as LT/RT
    /// do (the ribbon itself scrolls with it). One tab per swipe: the
    /// swipe ends when the touchpad has been still a moment (its inertia
    /// keeps sending after the fingers lift).
    fn swipe_tabs(&mut self, ui: &Ui) -> Option<Action> {
        let (d, now) = ui.input(|i| (i.smooth_scroll_delta, i.time));
        let sideways = d.x.abs() > 0.5 && d.x.abs() > d.y.abs() * 1.5;
        if !sideways {
            if now - self.swipe.2 > 0.25 {
                self.swipe = (0.0, false, self.swipe.2);
            }
            return None;
        }
        self.swipe.2 = now;
        let on_ribbon = self.game.is_none() && self.page == Page::Home;
        if on_ribbon || self.modal_open() || self.swipe.1 {
            return None;
        }
        self.swipe.0 += d.x;
        if self.swipe.0.abs() < 80.0 {
            return None;
        }
        self.swipe.1 = true;
        // Fingers to the left: the next tab (as the ribbon goes on).
        Some(if self.swipe.0 < 0.0 {
            Action::RightTrigger
        } else {
            Action::LeftTrigger
        })
    }

    /// Is a window up over everything (a question, the sign-in…)?
    fn modal_open(&self) -> bool {
        self.login.is_some()
            || self.sgdb.ask.is_some()
            || self.confirm.is_some()
            || self.versions.ask.is_some()
            || self.versions.import.is_some()
            || self.manage.pick.is_some()
            || self.modals.choose.is_some()
            || self.modals.files.is_some()
            || self.osk.is_some()
            || self.menu.is_some()
            || self.cedit.is_some()
            || self.mods.over.is_some()
    }

    fn nav_mark(&self) -> NavMark {
        let g = self.game.as_ref();
        let o = |x: Option<usize>| x.map_or(-1, |v| v as i64);
        NavMark {
            modal: self.modal_open(),
            game: g.is_some(),
            side: self.side_focus,
            page: self
                .pages()
                .iter()
                .position(|p| *p == self.page)
                .unwrap_or(0),
            tab: g.map_or(0, |v| v.tab.no()),
            sel: vec![
                self.side_sel as i64,
                self.home.sel as i64,
                self.lib.sel as i64,
                self.lib.filter as i64,
                self.libv.shelf as i64,
                self.libv.side as i64,
                o(self.libv.open),
                self.libv.my_open.map_or(-1, |v| v as i64),
                self.sys.sel as i64,
                self.sys.filter as i64,
                self.set.sel as i64,
                self.sounds.tab as i64,
                self.sounds.grid.sel as i64,
                self.proton.sel as i64,
                g.map_or(-1, |v| v.focus as i64),
                g.map_or(-1, |v| v.tool_sel as i64),
                g.map_or(-1, |v| v.shot_sel as i64),
                o(g.and_then(|v| v.shot)),
                o(self.menu.as_ref().map(|m| m.sel)),
                o(self.cedit.as_ref().map(|e| e.sel)),
                o(self.cedit.as_ref().map(|e| e.btn)),
                self.mods.sel as i64,
                o(self.mods.page.as_ref().map(|v| v.sel)),
                o(self.mods.over.as_ref().map(|v| v.sel)),
                o(self.mods.game.as_ref().map(|v| v.sel)),
                self.mods.depth() as i64,
            ],
        }
    }

    /// The mouse, the wheel and dragging change things without an action:
    /// a sound for what changed this frame, when nothing played one; and
    /// a click on anything gives the press sound.
    fn sound_watch(&mut self, ctx: &egui::Context) {
        use crate::sfx::Sound;
        let now = self.nav_mark();
        let before = std::mem::replace(&mut self.nav, now.clone());
        if ctx.input(|i| !i.events.is_empty() || i.pointer.any_down()) {
            self.last_input = Instant::now();
        }
        // The first frame, or nothing done lately (a flicked ribbon still
        // gliding counts as lately).
        if self.sfx.take_played()
            || before.sel.is_empty()
            || self.last_input.elapsed() > Duration::from_millis(1500)
        {
            return;
        }
        let clicked = ctx.interaction_snapshot(|s| s.clicked.is_some());
        let sound = if before.modal != now.modal {
            Some(if now.modal {
                Sound::ShowModal
            } else {
                Sound::HideModal
            })
        } else if before.game != now.game {
            Some(if now.game {
                Sound::IntoDetail
            } else {
                Sound::OutOfDetail
            })
        } else if before.side != now.side {
            Some(if now.side {
                Sound::MenuIn
            } else {
                Sound::MenuOut
            })
        } else if before.page != now.page || before.tab != now.tab {
            Some(Sound::Tab)
        } else if clicked {
            Some(Sound::Activate)
        } else if before.sel != now.sel {
            Some(Sound::Navigate)
        } else {
            None
        };
        if let Some(s) = sound {
            self.sfx.play(s);
            self.sfx.take_played();
        }
    }

    /// An action, with the sound for what it did.
    pub(crate) fn handle(&mut self, a: Action) {
        use crate::sfx::Sound;
        let snap = |app: &App| {
            (
                app.game.is_some(),
                app.side_focus,
                app.page,
                app.modal_open(),
                app.game.as_ref().map(|v| v.tab),
            )
        };
        let before = snap(self);
        let pre = self.sfx.take_played();
        self.handle_quiet(a);
        // What the action played itself (a switch, a slider) stands alone.
        let own = self.sfx.take_played();
        let after = snap(self);
        let sound = if before.3 != after.3 {
            Some(if after.3 {
                Sound::ShowModal
            } else {
                Sound::HideModal
            })
        } else if before.0 != after.0 {
            Some(if after.0 {
                Sound::IntoDetail
            } else {
                Sound::OutOfDetail
            })
        } else if before.1 != after.1 {
            Some(if after.1 {
                Sound::MenuIn
            } else {
                Sound::MenuOut
            })
        } else if before.2 != after.2 || before.4 != after.4 {
            Some(Sound::Tab)
        } else {
            match a {
                // Settings' ← → change values: their own sounds.
                Action::Left | Action::Right
                    if self.page == Page::Settings && self.game.is_none() && !self.side_focus =>
                {
                    None
                }
                // …and A there toggles or presses: theirs too.
                Action::Accept
                    if self.page == Page::Settings && self.game.is_none() && !self.side_focus =>
                {
                    None
                }
                Action::Up | Action::Down | Action::Left | Action::Right => Some(Sound::Navigate),
                Action::Accept => Some(Sound::Activate),
                Action::LeftTrigger | Action::RightTrigger => Some(Sound::Navigate),
                _ => None,
            }
        };
        if let Some(s) = sound
            && !own
        {
            self.sfx.play(s);
        }
        if pre || own {
            self.sfx.mark_played();
        }
    }

    fn handle_quiet(&mut self, a: Action) {
        if self.osk.is_some() {
            self.osk_input(a);
            return;
        }
        if self.login.is_some() {
            self.login_input(a);
            return;
        }
        if self.sgdb.ask.is_some() {
            self.sgdb_input(a);
            return;
        }
        if self.confirm.is_some() {
            self.confirm_input(a);
            return;
        }
        if self.cedit.is_some() {
            self.cedit_input(a);
            return;
        }
        if self.manage.pick.is_some() {
            self.manage_input(a);
            return;
        }
        if self.modals.files.is_some() {
            self.files_input(a);
            return;
        }
        if self.modals.choose.is_some() {
            self.choose_input(a);
            return;
        }
        if self.mods.over.is_some() {
            if self.modview_input(crate::ui::modules::Which::Over, a) {
                self.mods.over = None;
            }
            return;
        }
        if self.menu.is_some() {
            self.menu_input(a);
            return;
        }

        if self.game.is_some() {
            self.game_input(a);
            return;
        }
        match a {
            Action::PrevTab | Action::NextTab => {
                let pages = self.pages();
                let i = pages.iter().position(|p| *p == self.page).unwrap_or(0) as i32;
                let n = pages.len() as i32;
                let j = (i + if a == Action::NextTab { 1 } else { -1 }).rem_euclid(n);
                self.go(pages[j as usize]);
                self.side_focus = false;
                self.set.color_edit = None;
                return;
            }
            Action::View => {
                self.side_focus = !self.side_focus;
                self.side_sel = self
                    .pages()
                    .iter()
                    .position(|p| *p == self.page)
                    .unwrap_or(0);
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
            let pages = self.pages();
            // The pages, then Power at the bottom.
            let power = pages.len();
            match a {
                Action::Up => self.side_sel = self.side_sel.saturating_sub(1),
                Action::Down => self.side_sel = (self.side_sel + 1).min(power),
                Action::Accept if self.side_sel == power => {
                    self.side_focus = false;
                    self.open_menu(crate::ui::menu::MenuFor::Power, None);
                }
                Action::Accept => {
                    self.go(pages[self.side_sel.min(power - 1)]);
                    self.side_focus = false;
                }
                Action::Back => self.side_focus = false,
                _ => {}
            }
            // Moving in the panel shows the page at once.
            if matches!(a, Action::Up | Action::Down) && self.side_sel < power {
                self.page = pages[self.side_sel];
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
            Page::Proton => self.proton_input(a),
            Page::Sounds => self.sounds_input(a),
            Page::Module(_) => self.modview_input(crate::ui::modules::Which::Page, a),
            Page::Modules => self.modules_input(a),
        };
        if back && self.page != Page::Home {
            self.go(Page::Home);
        }
    }

    pub fn hint_items(&self) -> Vec<(Btn, &'static str)> {
        if self.osk.is_some() {
            return vec![
                (Btn::A, tr("osk_key")),
                (Btn::X, tr("osk_erase")),
                (Btn::Y, tr("osk_space")),
                (Btn::LT, "Shift"),
                (Btn::View, tr("osk_layout")),
                (Btn::Menu, tr("osk_done")),
                (Btn::B, tr("cancel")),
            ];
        }
        if self.confirm.is_some() {
            return vec![(Btn::A, tr("yes")), (Btn::B, tr("cancel"))];
        }
        if self.manage.pick.is_some() || self.modals.choose.is_some() {
            return vec![(Btn::A, tr("choose")), (Btn::B, tr("back"))];
        }
        if self.modals.files.is_some() {
            return vec![
                (Btn::A, tr("open")),
                (Btn::Y, tr("fp_toggle_all")),
                (Btn::B, tr("fp_up")),
                (Btn::Menu, tr("close")),
            ];
        }
        if self.cedit.is_some() {
            return vec![
                (Btn::Dpad, tr("change")),
                (Btn::A, tr("choose")),
                (Btn::Menu, tr("coll_save")),
                (Btn::B, tr("cancel")),
            ];
        }
        if self.menu.is_some() {
            return vec![(Btn::A, tr("choose")), (Btn::B, tr("close"))];
        }
        if self.mods.over.is_some() {
            return self.modview_hints(crate::ui::modules::Which::Over);
        }
        if self.sgdb.ask.is_some() {
            return vec![
                (Btn::Dpad, tr("sgdb_pick_name")),
                (Btn::A, tr("sgdb_create")),
                (Btn::B, tr("cancel")),
            ];
        }
        if let Some(l) = &self.login {
            let mut v = vec![(Btn::B, tr("back"))];
            if l.failed() {
                v.insert(0, (Btn::A, tr("retry")));
            }
            return v;
        }
        if let Some(v) = &self.game {
            if v.shot.is_some() {
                return vec![(Btn::Dpad, tr("scroll")), (Btn::B, tr("back"))];
            }
            let installed = self.game_by_key(&v.key).is_none_or(|g| g.installed);
            let play = if self.is_running(&v.key) {
                tr("stop_game")
            } else if installed {
                tr("play")
            } else {
                tr("install")
            };
            if self.versions.ask.is_some() {
                return vec![(Btn::A, tr("yes")), (Btn::B, tr("cancel"))];
            }
            if self.versions.import.is_some() {
                return vec![
                    (Btn::A, tr("choose")),
                    (Btn::X, tr("sdb_paste_short")),
                    (Btn::B, tr("close")),
                ];
            }
            if v.focus == crate::ui::game_page::INSIDE && v.tab == Tab::Versions {
                return vec![
                    (Btn::A, tr("sd_tick")),
                    (Btn::X, tr("sd_install")),
                    (Btn::Dpad, tr("scroll")),
                    (Btn::B, tr("back")),
                ];
            }
            if v.focus == crate::ui::game_page::INSIDE && v.tab == Tab::Achievements {
                return vec![
                    (Btn::Dpad, tr("scroll")),
                    (Btn::A, tr("ach_toggle")),
                    (Btn::B, tr("back")),
                ];
            }
            if v.focus == crate::ui::game_page::INSIDE && v.tab == Tab::Art {
                return vec![
                    (Btn::Dpad, tr("art_browse")),
                    (Btn::A, tr("art_take_btn")),
                    (Btn::B, tr("back")),
                ];
            }
            if v.focus == crate::ui::game_page::INSIDE && v.tab == Tab::Launch {
                return self.launch_hints();
            }
            if v.focus == crate::ui::game_page::INSIDE && matches!(v.tab, Tab::Module(_)) {
                let mut h = self.modview_hints(crate::ui::modules::Which::Game);
                if self
                    .page_game()
                    .is_some_and(|g| self.mod_group(&g).is_some())
                {
                    h.push((Btn::LB, tr("mod_group_switch")));
                }
                return h;
            }
            if v.focus == crate::ui::game_page::INSIDE && v.tab == Tab::Compat {
                return vec![
                    (Btn::A, tr("choose")),
                    (Btn::Dpad, tr("scroll")),
                    (Btn::B, tr("back")),
                ];
            }
            let mut v = vec![(Btn::A, if v.focus == 0 { play } else { tr("open") })];
            if self.game.as_ref().is_some_and(|g| g.focus != 0) {
                v.push((Btn::X, play));
            }
            v.extend([
                (Btn::Dpad, tr("scroll")),
                (Btn::LT, tr("tab")),
                (Btn::B, tr("back")),
            ]);
            if self.cfg.theme_music
                && self
                    .music
                    .now
                    .lock()
                    .ok()
                    .and_then(|n| n.clone())
                    .is_some_and(|(_, song)| !song.is_empty())
            {
                v.insert(v.len() - 1, (Btn::Y, tr("music_next")));
            }
            return v;
        }
        if self.side_focus {
            return vec![(Btn::A, tr("open")), (Btn::B, tr("back"))];
        }
        let mut v = match self.page {
            Page::Home => vec![
                (Btn::A, tr("open")),
                (Btn::X, tr("play")),
                (Btn::Y, tr("ctx_menu")),
            ],
            Page::Library if self.libv.side => vec![
                (Btn::Dpad, tr("lib_shelf")),
                (Btn::A, tr("open")),
                (Btn::LT, tr("lib_launcher")),
            ],
            Page::Library => vec![
                (Btn::A, tr("open")),
                (Btn::X, tr("play")),
                (Btn::Y, tr("ctx_menu")),
                (
                    Btn::B,
                    if self.libv.open.is_some() || self.libv.my_open.is_some() {
                        tr("back")
                    } else {
                        tr("lib_shelves")
                    },
                ),
                (Btn::LT, tr("lib_launcher")),
            ],
            Page::System => vec![(Btn::A, tr("launch")), (Btn::LT, tr("filter"))],
            Page::Settings => vec![(Btn::Dpad, tr("change")), (Btn::B, tr("back"))],
            Page::About => vec![],
            Page::Proton => vec![(Btn::A, tr("pu_act")), (Btn::Dpad, tr("scroll"))],
            Page::Module(_) => self.modview_hints(crate::ui::modules::Which::Page),
            Page::Modules => self.modules_hints(),
            Page::Sounds => {
                let mut v = vec![(Btn::A, tr("sounds_use_btn")), (Btn::LT, tr("sounds_tab"))];
                if self.sounds.tab == 0 {
                    v.push((Btn::X, tr("pu_remove_short")));
                }
                v
            }
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
            let sources_changed = (
                self.cfg.steam,
                self.cfg.heroic,
                self.cfg.lutris,
                self.cfg.hydra,
                self.cfg.own,
            ) != (
                self.saved.steam,
                self.saved.heroic,
                self.saved.lutris,
                self.saved.hydra,
                self.saved.own,
            );
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
            || self.steam.restart.is_some()
            || self.versions.op.is_some()
            || self.sgdb.job.is_some()
            || self.proton.job.is_some()
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
        crate::ui::ribbon::size_cards(&self.cfg, ui.ctx().pixels_per_point());
        let ctx = ui.ctx().clone();
        self.take_scan();
        self.pump_restart();
        self.pump_versions();
        self.pump_sgdb();
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
                    self.last_input = Instant::now();
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
        if let Some(a) = self.swipe_tabs(ui) {
            self.last_input = Instant::now();
            self.handle(a);
        }
        // Back from a game: play time and achievements may have changed.
        if focused && !self.was_focused && self.last_scan.elapsed() > Duration::from_secs(30) {
            self.rescan();
        }
        self.was_focused = focused;
        // Theme music while a game's page is open and the launcher is in
        // front (a game started takes the focus: the music fades out).
        // On the ribbon: the chosen card's, once the ribbon has settled on it
        // for a moment, rising slowly.
        let sel_key = self.games.get(self.home.sel).map(|g| g.key.clone());
        if sel_key != self.home_pick.0 {
            self.home_pick = (sel_key, Instant::now());
        }
        let settled = (self.home.pos - self.home.sel as f32).abs() < 0.02 && !self.home.dragging;
        let on = self.cfg.theme_music && focused && std::env::var_os("LENTA_DRY_RUN").is_none();
        let (theme, fade) = if !on {
            (None, 1.0)
        } else if let Some(v) = &self.game {
            (
                self.game_by_key(&v.key)
                    .map(|g| (g.key.clone(), g.title.clone())),
                1.5,
            )
        } else if self.page == Page::Home
            && self.cfg.theme_music_ribbon
            && !self.side_focus
            && settled
            && self.home_pick.1.elapsed() > Duration::from_millis(1200)
        {
            (
                self.selected_game()
                    .map(|g| (g.key.clone(), g.title.clone())),
                4.0,
            )
        } else {
            (None, 1.0)
        };
        self.music
            .want(theme.as_ref().map(|(k, t)| (k.as_str(), t.as_str())), fade);
        // The wait above ends without any input: look again then.
        if on && self.home_pick.1.elapsed() < Duration::from_millis(1300) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.music.volume(self.cfg.music_volume);
        self.sfx.set(
            self.cfg.ui_sounds,
            &self.cfg.sound_pack,
            self.cfg.sound_volume,
        );
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
                        Page::Proton => self.draw_proton(ui, body),
                        Page::Sounds => self.draw_sounds(ui, body),
                        Page::Module(i) => self.draw_mod_section(ui, body, i),
                        Page::Modules => self.draw_modules(ui, body),
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
                if self.login.is_some() {
                    self.draw_login(ui, full);
                }
                if self.sgdb.ask.is_some() {
                    self.draw_sgdb_ask(ui, full);
                }

                if self.mods.over.is_some() {
                    self.draw_mod_over(ui, full);
                }
                if self.menu.is_some() {
                    self.draw_menu(ui, full);
                }
                if self.manage.pick.is_some() {
                    self.draw_manage_pick(ui, full);
                }
                if self.modals.choose.is_some() {
                    self.draw_choose(ui, full);
                }
                if self.modals.files.is_some() {
                    self.draw_files(ui, full);
                }
                self.pump_portal();
                self.draw_manage_job(ui, full);
                if self.cedit.is_some() {
                    self.draw_cedit(ui, full);
                }
                if self.confirm.is_some() {
                    self.draw_confirm(ui, full);
                }
                if self.osk.is_some() {
                    self.draw_osk(ui, full);
                }
                self.draw_toast(ui, full);
            });
        self.mods_tick();
        self.running_tick();
        self.sound_watch(&ctx);
        self.save_later();
        if self.animating() {
            ctx.request_repaint();
        } else if focused && self.cfg.glow > 0.0 && self.game.is_none() {
            // The selected card's glow breathes; never while a game is up.
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}
