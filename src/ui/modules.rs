//! Modules in the interface: the Modules section (on/off, where each is
//! shown, opening those kept out of the side panel), a module's own
//! section, its window, and its tab on a game's page. The pages are the
//! module's: the launcher draws what it's told (see src/modules.rs).

use std::time::{Duration, Instant};

use egui::{
    Align2, Color32, FontId, Id, LayerId, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};
use serde_json::{Value, json};

use super::widgets::{self, Btn, Icon};
use crate::app::{App, Page, Tab};
use crate::config::ModCfg;
use crate::game::{Art, Game};
use crate::i18n::tr;
use crate::images::Kind as ImgKind;
use crate::input::Action;
use crate::modules::{self, Kind, ModView, Module, num_value, slide};

/// Which of the shown module pages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Which {
    /// A module's own section.
    Page,
    /// Its window, opened from Modules.
    Over,
    /// Its tab on a game's page.
    Game,
}

pub struct Mods {
    pub list: Vec<Module>,
    /// Where they're looked for.
    pub dirs: Vec<(std::path::PathBuf, bool)>,
    scanned: Instant,
    pub page: Option<ModView>,
    pub over: Option<ModView>,
    pub game: Option<ModView>,
    /// The Modules section's choice and scroll.
    pub sel: usize,
    pub scroll: f32,
    /// The module shown in a game's gathered "Modules" tab (its id).
    pub pick: Option<String>,
}

impl Mods {
    pub fn load() -> Mods {
        modules::sync_bundled();
        Mods::load_from(modules::dirs())
    }

    pub fn load_from(dirs: Vec<(std::path::PathBuf, bool)>) -> Mods {
        Mods {
            list: modules::scan(&dirs),
            dirs,
            scanned: Instant::now(),
            page: None,
            over: None,
            game: None,
            sel: 0,
            scroll: 0.0,
            pick: None,
        }
    }

    /// How deep in pages the views are (the sound watcher's).
    pub fn depth(&self) -> usize {
        [&self.page, &self.over, &self.game]
            .iter()
            .map(|v| v.as_ref().map_or(0, |v| v.stack.len()))
            .sum()
    }
}

impl ModView {
    /// Nothing to choose above the choice.
    pub fn at_top(&self) -> bool {
        match &self.page {
            Some(p) => !p.items[..self.sel.min(p.items.len())]
                .iter()
                .any(|i| i.selectable()),
            None => true,
        }
    }
}

/// What a click or a button does to an item.
#[derive(Clone, Copy, Debug)]
enum Act {
    Step(i32),
    /// The slider's knob put here (0…1).
    At(f64),
    Press,
}

/// What comes of it.
enum Out {
    Set(String, Value),
    Press(String),
    Ask(String, String),
    Open(String),
    Type(String, String, String, bool),
    File(String, String, Vec<String>, String),
}

/// One of the launcher's icons, by the name a module gives.
pub fn icon_named(name: &str) -> Option<Icon> {
    Some(match name {
        "home" => Icon::Home,
        "library" => Icon::Library,
        "system" => Icon::System,
        "settings" => Icon::Settings,
        "about" | "info" => Icon::About,
        "proton" => Icon::Proton,
        "sound" => Icon::Sound,
        "play" => Icon::Play,
        "clock" => Icon::Clock,
        "trophy" => Icon::Trophy,
        "wifi" => Icon::Wifi,
        "bluetooth" => Icon::Bluetooth,
        "module" => Icon::Module,
        "sliders" => Icon::Sliders,
        _ => return None,
    })
}

fn source_name(g: &Game) -> String {
    format!("{:?}", g.source).to_lowercase()
}

impl App {
    pub fn mod_cfg(&self, m: &Module) -> ModCfg {
        self.cfg.modules.get(&m.id).cloned().unwrap_or_default()
    }

    /// Shown as a section in the side panel.
    pub fn mod_section(&self, m: &Module) -> bool {
        let c = self.mod_cfg(m);
        c.on && m.pages && c.section.unwrap_or(m.wants_section)
    }

    /// The modules' tabs on this game's page.
    pub fn mod_tabs(&self, g: &Game) -> Vec<Tab> {
        let src = source_name(g);
        self.mods
            .list
            .iter()
            .enumerate()
            .filter(|(_, m)| self.mod_cfg(m).on && m.game_tab(&src, g.installed).is_some())
            .map(|(i, _)| Tab::Module(i as u16))
            .collect()
    }

    /// Module tabs gathered into one ("Modules") on a game's page: the
    /// modules, when there are two or more and that's set.
    pub fn mod_group(&self, g: &Game) -> Option<Vec<u16>> {
        let v: Vec<u16> = self
            .mod_tabs(g)
            .into_iter()
            .filter_map(|t| match t {
                Tab::Module(i) => Some(i),
                _ => None,
            })
            .collect();
        (self.cfg.group_mod_tabs && v.len() >= 2).then_some(v)
    }

    /// The one shown in the gathered tab: the one chosen last, else the first.
    pub fn mod_group_pick(&self, group: &[u16]) -> u16 {
        self.mods
            .pick
            .as_ref()
            .and_then(|id| {
                group
                    .iter()
                    .copied()
                    .find(|i| self.mods.list.get(*i as usize).is_some_and(|m| &m.id == id))
            })
            .unwrap_or(group[0])
    }

    /// Show another module in the gathered tab.
    pub fn mod_group_show(&mut self, i: u16) {
        self.mods.pick = self.mods.list.get(i as usize).map(|m| m.id.clone());
        if let Some(v) = self.game.as_mut() {
            v.tab = Tab::Module(i);
            v.scroll = 0.0;
        }
        self.sfx.play(crate::sfx::Sound::Navigate);
    }

    /// The row of the gathered modules over the tab; the one clicked.
    pub fn draw_mod_chips(
        &mut self,
        ui: &mut Ui,
        at: Rect,
        group: &[u16],
        cur: u16,
    ) -> Option<u16> {
        let p = ui.painter_at(at.expand2(vec2(0.0, 4.0)));
        let (c1, _) = self.accent;
        let mut x = at.left();
        let mut clicked = None;
        for (k, i) in group.iter().enumerate() {
            let g = p.layout_no_wrap(
                self.mod_tab_label(*i),
                FontId::proportional(16.0),
                Color32::WHITE,
            );
            let r = Rect::from_min_size(pos2(x, at.top()), vec2(g.size().x + 28.0, 34.0));
            let resp = ui.interact(r, ui.id().with(("mod_chip", k)), Sense::click());
            let on = *i == cur;
            p.rect_filled(
                r,
                17.0,
                if on {
                    c1.gamma_multiply(0.75)
                } else if resp.hovered() {
                    Color32::from_white_alpha(28)
                } else {
                    Color32::from_white_alpha(12)
                },
            );
            p.galley(r.center() - g.size() / 2.0, g, Color32::WHITE);
            if resp.clicked() {
                clicked = Some(*i);
            }
            x = r.right() + 10.0;
        }
        clicked
    }

    pub fn mod_tab_label(&self, i: u16) -> String {
        self.mods
            .list
            .get(i as usize)
            .and_then(|m| m.game.as_ref())
            .map(|h| h.label.clone())
            .unwrap_or_default()
    }

    /// What a module is told about a game.
    fn game_json(&self, g: &Game) -> Value {
        json!({
            "key": g.key,
            "title": g.title,
            "source": source_name(g),
            "installed": g.installed,
            "steam_appid": g.steam_appid,
            "store_appid": g.store_appid,
            "install_dir": g.install_dir,
            "prefix": g.prefix,
            "compat_tool": g.compat_id.and_then(|id| self.steam.mapping.get(&id).cloned()),
            "launch": self.launch_state(g),
        })
    }

    /// Where a game's start stands for the modules: "none" (nothing
    /// added), "ready", or "pending" (Steam must be restarted first).
    fn launch_state(&self, g: &Game) -> &'static str {
        if !self.launch.wants(&g.key) {
            return "none";
        }
        match g.steam_appid {
            Some(id) if self.steam.hooks.iter().any(|h| h.appid == id) => "pending",
            Some(id) if !crate::launch::hooked(id, &g.key) => "pending",
            _ => "ready",
        }
    }

    /// A module changed a game's start.
    fn mod_launch(&mut self, module: &str, key: &str, part: Option<crate::launch::Part>) {
        self.launch.set_part(key, module, part);
        self.start_push(key);
        let Some(g) = self.game_by_key(key).cloned() else {
            return;
        };
        // The module sees the new state on its next question.
        let state = self.launch_state(&g);
        for w in [Which::Page, Which::Over, Which::Game] {
            if let Some(v) = self.view_mut(w)
                && v.game_key.as_deref() == Some(key)
                && let Some(gj) = v.game.as_mut()
            {
                gj["launch"] = json!(state);
                // Its page was drawn with the old state: ask again.
                v.fetch();
            }
        }
    }

    /// A module's icon: its picture, one of ours by name, or the modules'.
    pub fn mod_icon(&mut self, p: &egui::Painter, c: Pos2, s: f32, i: usize, color: Color32) {
        let Some(m) = self.mods.list.get(i) else {
            return;
        };
        if let Some(f) = m.icon_file() {
            if let Some(t) = self.images.get(&Art::File(f), ImgKind::Fit(96)) {
                let r = Rect::from_center_size(c, vec2(s, s) * 1.1);
                p.image(
                    t.handle.id(),
                    r,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            return;
        }
        let ic = m
            .icon
            .as_deref()
            .and_then(icon_named)
            .unwrap_or(Icon::Module);
        widgets::icon(p, c, s, ic, color);
    }

    fn view(&self, w: Which) -> Option<&ModView> {
        match w {
            Which::Page => self.mods.page.as_ref(),
            Which::Over => self.mods.over.as_ref(),
            Which::Game => self.mods.game.as_ref(),
        }
    }

    fn view_mut(&mut self, w: Which) -> Option<&mut ModView> {
        match w {
            Which::Page => self.mods.page.as_mut(),
            Which::Over => self.mods.over.as_mut(),
            Which::Game => self.mods.game.as_mut(),
        }
    }

    fn slot(&mut self, w: Which) -> &mut Option<ModView> {
        match w {
            Which::Page => &mut self.mods.page,
            Which::Over => &mut self.mods.over,
            Which::Game => &mut self.mods.game,
        }
    }

    fn module_of(&self, v: &ModView) -> Option<&Module> {
        self.mods.list.iter().find(|m| m.id == v.module)
    }

    /// Every frame: look for modules come and gone, take the answers.
    pub fn mods_tick(&mut self) {
        let every = Duration::from_secs(3);
        if self.mods.scanned.elapsed() > every {
            self.mods.scanned = Instant::now();
            let list = modules::scan(&self.mods.dirs);
            if list != self.mods.list {
                self.mods_replace(list);
            }
        }
        // A tab of a module turned off (or gone): back to the overview.
        let tabs = self
            .game
            .as_ref()
            .and_then(|v| self.games_and_more_key(&v.key))
            .map(|g| self.mod_tabs(&g));
        if let (Some(v), Some(tabs)) = (self.game.as_mut(), tabs)
            && matches!(v.tab, Tab::Module(_))
            && !tabs.contains(&v.tab)
        {
            v.tab = Tab::Overview;
            v.focus = 1;
        }
        let mut busy = false;
        let focused = self.ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let id_at = |i: u16| self.mods.list.get(i as usize).map(|m| m.id.clone());
        let page_id = match self.page {
            Page::Module(i) if self.game.is_none() => id_at(i),
            _ => None,
        };
        let tab_id = match self.game.as_ref().map(|v| v.tab) {
            Some(Tab::Module(i)) => id_at(i),
            _ => None,
        };
        let mut soon: Option<Duration> = None;
        for w in [Which::Page, Which::Over, Which::Game] {
            let Some(mut v) = self.slot(w).take() else {
                continue;
            };
            // Asked again only while it's on screen.
            let shown = focused
                && match w {
                    Which::Page => page_id.as_deref() == Some(v.module.as_str()),
                    Which::Over => true,
                    Which::Game => tab_id.as_deref() == Some(v.module.as_str()),
                };
            let said = match self.module_of(&v).cloned() {
                Some(m) => v.pump(&m, shown),
                None => Default::default(),
            };
            let module = v.module.clone();
            if shown && let Some(d) = v.refresh_in() {
                soon = Some(soon.map_or(d, |s| s.min(d)));
            }
            busy |= v.busy();
            *self.slot(w) = Some(v);
            if let Some(t) = said.toast {
                self.toast(t);
            }
            if let Some((key, part)) = said.launch {
                self.mod_launch(&module, &key, part);
            }
            if said.restart_steam {
                self.restart_steam();
            }
        }
        if busy {
            self.ctx.request_repaint_after(Duration::from_millis(40));
        }
        if let Some(d) = soon {
            self.ctx.request_repaint_after(d + Duration::from_millis(5));
        }
        if matches!(self.page, Page::Modules | Page::Module(_)) {
            self.ctx.request_repaint_after(every);
        }
    }

    /// Look for modules now (tests).
    #[cfg(test)]
    pub fn mods_rescan_now(&mut self) {
        let list = modules::scan(&self.mods.dirs);
        self.mods_replace(list);
    }

    /// A game by key, installed or not.
    fn games_and_more_key(&self, key: &str) -> Option<Game> {
        self.game_by_key(key).cloned()
    }

    /// The list changed: places by number follow their modules.
    fn mods_replace(&mut self, list: Vec<Module>) {
        let id_at = |i: u16| self.mods.list.get(i as usize).map(|m| m.id.clone());
        let place = |id: Option<String>| -> Option<u16> {
            let id = id?;
            list.iter().position(|m| m.id == id).map(|i| i as u16)
        };
        let page = match self.page {
            Page::Module(i) => Some(place(id_at(i))),
            _ => None,
        };
        let tab = match self.game.as_ref().map(|v| v.tab) {
            Some(Tab::Module(i)) => Some(place(id_at(i))),
            _ => None,
        };
        self.mods.list = list;
        for w in [Which::Page, Which::Over, Which::Game] {
            let gone = self
                .view(w)
                .is_some_and(|v| !self.mods.list.iter().any(|m| m.id == v.module));
            if gone {
                *self.slot(w) = None;
            }
        }
        match page {
            Some(Some(i)) => self.go(Page::Module(i)),
            Some(None) => self.go(Page::Home),
            None => {
                let p = self.page;
                self.go(p);
            }
        }
        if let (Some(t), Some(v)) = (tab, self.game.as_mut()) {
            match t {
                Some(i) => v.tab = Tab::Module(i),
                None => {
                    v.tab = Tab::Overview;
                    v.focus = 1;
                }
            }
        }
    }

    /// Open a module from Modules: its section, or its window.
    fn open_module(&mut self, i: usize) {
        let Some(m) = self.mods.list.get(i).cloned() else {
            return;
        };
        if !self.mod_cfg(&m).on {
            self.toast(tr("mod_off").to_string());
            return;
        }
        if !m.pages {
            self.toast(tr("mod_game_only").to_string());
            return;
        }
        if self.mod_section(&m) {
            self.go(Page::Module(i as u16));
        } else {
            self.mods.over = Some(ModView::new(&m.id, None, "main"));
        }
    }

    fn mod_switch(&mut self, i: usize) {
        let Some(id) = self.mods.list.get(i).map(|m| m.id.clone()) else {
            return;
        };
        let c = self.cfg.modules.entry(id).or_default();
        c.on = !c.on;
        let on = c.on;
        self.sfx.play(if on {
            crate::sfx::Sound::ToggleOn
        } else {
            crate::sfx::Sound::ToggleOff
        });
    }

    fn mod_where(&mut self, i: usize) {
        let Some(m) = self.mods.list.get(i).cloned() else {
            return;
        };
        if !m.pages {
            return;
        }
        let now = self.mod_section(&m) || !self.mod_cfg(&m).on && m.wants_section;
        let c = self.cfg.modules.entry(m.id.clone()).or_default();
        // Back to what it asks for: no word of ours kept.
        c.section = if !now == m.wants_section {
            None
        } else {
            Some(!now)
        };
        if *c == ModCfg::default() {
            self.cfg.modules.remove(&m.id);
        }
        self.sfx.play(crate::sfx::Sound::Navigate);
    }

    // ------------------------------------------------------------ Modules

    pub fn modules_input(&mut self, a: Action) -> bool {
        let n = self.mods.list.len();
        match a {
            Action::Up => self.mods.sel = self.mods.sel.saturating_sub(1),
            Action::Down => self.mods.sel = (self.mods.sel + 1).min(n.saturating_sub(1)),
            Action::Accept => self.open_module(self.mods.sel),
            Action::Select => self.mod_switch(self.mods.sel),
            Action::Actions => self.mod_where(self.mods.sel),
            Action::Back => return true,
            _ => {}
        }
        false
    }

    pub fn modules_hints(&self) -> Vec<(Btn, &'static str)> {
        if self.mods.list.is_empty() {
            return vec![(Btn::B, tr("back"))];
        }
        vec![
            (Btn::A, tr("open")),
            (Btn::X, tr("mod_onoff")),
            (Btn::Y, tr("mod_show")),
            (Btn::B, tr("back")),
        ]
    }

    pub fn draw_modules(&mut self, ui: &mut Ui, rect: Rect) {
        let rect = rect.shrink2(vec2(36.0, 0.0));
        let p = ui.painter_at(rect);
        let (c1, c2) = self.accent;
        p.text(
            pos2(rect.left(), rect.top() + 34.0),
            Align2::LEFT_CENTER,
            tr("modules"),
            FontId::proportional(32.0),
            Color32::WHITE,
        );
        let n = self.mods.list.len();
        let user = modules::user_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        if n == 0 {
            let mut y = rect.top() + 100.0;
            for (t, c) in [
                (tr("mod_none").to_string(), 210),
                (tr("mod_where").to_string(), 150),
                (user, 200),
                (tr("mod_or_pkg").to_string(), 150),
            ] {
                p.text(
                    pos2(rect.left(), y),
                    Align2::LEFT_TOP,
                    t,
                    FontId::proportional(if c == 210 { 22.0 } else { 17.0 }),
                    Color32::from_gray(c),
                );
                y += if c == 210 { 44.0 } else { 28.0 };
            }
            return;
        }
        self.mods.sel = self.mods.sel.min(n - 1);
        let view = Rect::from_min_max(
            pos2(rect.left() - 8.0, rect.top() + 76.0),
            pos2(
                rect.left() + (rect.width()).min(1100.0) + 8.0,
                rect.bottom(),
            ),
        );
        let row_h = 104.0;
        let total = n as f32 * row_h + 70.0;
        let max = (total - view.height()).max(0.0);
        if ui.rect_contains_pointer(view) {
            let d = ui.input(|i| i.smooth_scroll_delta.y);
            self.mods.scroll = (self.mods.scroll - d).clamp(0.0, max);
        }
        let sel = self.mods.sel;
        let (top, bottom) = (sel as f32 * row_h, sel as f32 * row_h + row_h);
        let want = self
            .mods
            .scroll
            .min(top - 20.0)
            .max(bottom + 20.0 - view.height())
            .clamp(0.0, max);
        if ui.input(|i| i.pointer.is_still()) || ui.input(|i| i.smooth_scroll_delta.y == 0.0) {
            let k = self.ease(12.0);
            self.mods.scroll += (want - self.mods.scroll) * k;
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(view));
        child.set_clip_rect(view);
        let p = child.painter().clone();
        let mut open = None;
        let mut switch = None;
        let mut place = None;
        for i in 0..n {
            let m = self.mods.list[i].clone();
            let c = self.mod_cfg(&m);
            let r = Rect::from_min_size(
                pos2(
                    view.left() + 8.0,
                    view.top() + i as f32 * row_h - self.mods.scroll,
                ),
                vec2(view.width() - 16.0, row_h - 12.0),
            );
            if r.bottom() < view.top() || r.top() > view.bottom() {
                continue;
            }
            let resp = child.interact(r, Id::new(("mod_row", i)), Sense::click());
            let on_sel = i == sel && !self.side_focus;
            p.rect_filled(
                r,
                18.0,
                Color32::from_white_alpha(if on_sel {
                    26
                } else if resp.hovered() {
                    16
                } else {
                    9
                }),
            );
            if on_sel {
                p.rect_stroke(r, 18.0, Stroke::new(2.0, c1), StrokeKind::Inside);
            }
            let dim = if c.on { 1.0 } else { 0.45 };
            self.mod_icon(
                &p,
                pos2(r.left() + 46.0, r.center().y),
                40.0,
                i,
                Color32::WHITE.gamma_multiply(dim),
            );
            // The controls on the right: where, then on/off.
            let sw = Rect::from_center_size(pos2(r.right() - 48.0, r.center().y), vec2(52.0, 28.0));
            let pill = if m.pages {
                let label = if c.section.unwrap_or(m.wants_section) {
                    tr("mod_section")
                } else {
                    tr("mod_menu_only")
                };
                let g = p.layout_no_wrap(
                    label.to_string(),
                    FontId::proportional(16.0),
                    Color32::WHITE,
                );
                let pr = Rect::from_min_size(
                    pos2(sw.left() - 28.0 - g.size().x - 32.0, r.center().y - 17.0),
                    vec2(g.size().x + 32.0, 34.0),
                );
                let presp = child.interact(pr, Id::new(("mod_where", i)), Sense::click());
                p.rect_filled(
                    pr,
                    17.0,
                    Color32::from_white_alpha(if presp.hovered() { 40 } else { 22 }),
                );
                p.galley(
                    pr.min + vec2(16.0, (34.0 - g.size().y) / 2.0),
                    g,
                    Color32::from_gray((230.0 * dim) as u8),
                );
                if presp.clicked() {
                    place = Some(i);
                }
                Some(pr)
            } else {
                None
            };
            let sresp = child.interact(sw.expand(6.0), Id::new(("mod_on", i)), Sense::click());
            if c.on {
                widgets::gradient_pill(&p, sw, 14.0, c1, c2);
            } else {
                p.rect_filled(sw, 14.0, Color32::from_white_alpha(40));
            }
            p.circle_filled(
                pos2(
                    if c.on {
                        sw.right() - 14.0
                    } else {
                        sw.left() + 14.0
                    },
                    sw.center().y,
                ),
                10.0,
                Color32::WHITE,
            );
            if sresp.clicked() {
                switch = Some(i);
            }
            // Name, what it is, where from.
            let text_right = pill.map_or(sw.left(), |p| p.left()) - 20.0;
            let x = r.left() + 88.0;
            let w = (text_right - x).max(40.0);
            let line = |t: String, size: f32, gray: u8| {
                let mut job = egui::text::LayoutJob::simple_singleline(
                    t,
                    FontId::proportional(size),
                    Color32::from_gray((gray as f32 * dim) as u8),
                );
                job.wrap.max_width = w;
                job.wrap.max_rows = 1;
                p.fonts_mut(|f| f.layout_job(job))
            };
            let mut extra = vec![
                if m.system {
                    tr("mod_system")
                } else {
                    tr("mod_user")
                }
                .to_string(),
            ];
            if !m.version.is_empty() {
                extra.insert(0, format!("v{}", m.version));
            }
            if m.game.is_some() {
                extra.push(tr("mod_tab").to_string());
            }
            let rows = [
                line(m.name.clone(), 20.0, 255),
                line(m.description.clone(), 15.0, 175),
                line(extra.join(" · "), 13.0, 125),
            ];
            let mut y = r.top() + 14.0;
            for (k, g) in rows.into_iter().enumerate() {
                let h = g.size().y;
                p.galley(pos2(x, y), g, Color32::WHITE);
                y += h + if k == 0 { 4.0 } else { 3.0 };
            }
            if resp.clicked() {
                self.mods.sel = i;
                open = Some(i);
            }
        }
        if let Some(i) = switch {
            self.mods.sel = i;
            self.mod_switch(i);
        } else if let Some(i) = place {
            self.mods.sel = i;
            self.mod_where(i);
        } else if let Some(i) = open {
            self.open_module(i);
        }
    }

    // ------------------------------------------------------------ a module's pages

    pub fn draw_mod_section(&mut self, ui: &mut Ui, rect: Rect, i: u16) {
        let Some(m) = self.mods.list.get(i as usize).cloned() else {
            return;
        };
        if self.mods.page.as_ref().is_none_or(|v| v.module != m.id) {
            self.mods.page = Some(ModView::new(&m.id, None, "main"));
        }
        let rect = rect.shrink2(vec2(36.0, 0.0));
        let head = self.draw_view_head(ui, rect, Which::Page, &m.name);
        let body = Rect::from_min_max(
            pos2(rect.left(), rect.top() + head),
            pos2(rect.left() + rect.width().min(1000.0), rect.bottom()),
        );
        let focused = !self.side_focus;
        self.draw_view(ui, body, Which::Page, None, focused);
    }

    /// The title, and the page's own under it with a way back; its height.
    fn draw_view_head(&mut self, ui: &mut Ui, rect: Rect, w: Which, name: &str) -> f32 {
        let p = ui.painter_at(rect);
        let Some(v) = self.view(w) else { return 76.0 };
        // With its own panel the way back is beside it, over the page.
        let deep = v.stack.len() > 1 && v.nav.is_empty();
        let sub = v.page.as_ref().map(|p| p.title.clone()).unwrap_or_default();
        let busy = v.busy() && v.since.elapsed() > Duration::from_millis(300);
        let g = p.layout_no_wrap(name.to_string(), FontId::proportional(32.0), Color32::WHITE);
        let tw = g.size().x;
        p.galley(
            pos2(rect.left(), rect.top() + 34.0 - g.size().y / 2.0),
            g,
            Color32::WHITE,
        );
        if busy {
            // Three dots, one lit after another.
            let t = (self.time * 3.0) as usize % 3;
            for k in 0..3 {
                p.circle_filled(
                    pos2(rect.left() + tw + 22.0 + k as f32 * 14.0, rect.top() + 38.0),
                    4.0,
                    Color32::from_white_alpha(if k == t { 220 } else { 70 }),
                );
            }
        }
        if !deep {
            return 76.0;
        }
        let r = Rect::from_min_size(pos2(rect.left(), rect.top() + 64.0), vec2(500.0, 32.0));
        let resp = ui.interact(r, Id::new(("mod_back", w as u8)), Sense::click());
        p.text(
            pos2(rect.left(), r.center().y),
            Align2::LEFT_CENTER,
            format!("‹ {sub}"),
            FontId::proportional(19.0),
            Color32::from_gray(if resp.hovered() { 255 } else { 200 }),
        );
        if resp.clicked()
            && let Some(v) = self.view_mut(w)
        {
            v.back();
        }
        112.0
    }

    pub fn draw_mod_over(&mut self, ui: &mut Ui, full: Rect) {
        let Some(name) = self
            .mods
            .over
            .as_ref()
            .and_then(|v| self.module_of(v))
            .map(|m| m.name.clone())
        else {
            self.mods.over = None;
            return;
        };
        let layer = LayerId::new(Order::Foreground, Id::new("mod_over"));
        let p = ui.ctx().layer_painter(layer);
        let mut top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(full));
        let veil = top.interact(full, Id::new("mod_veil"), Sense::click());
        p.rect_filled(full, 0.0, Color32::from_black_alpha(150));
        let w = (full.width() - 80.0).min(1000.0);
        let card = Rect::from_center_size(
            full.center() - vec2(0.0, 20.0),
            vec2(w, full.height() - 140.0),
        );
        // Clicks on the card stay on it.
        let _ = top.interact(card, Id::new("mod_card"), Sense::click());
        p.rect_filled(card, 22.0, Color32::from_rgb(20, 22, 31));
        p.rect_stroke(
            card,
            22.0,
            Stroke::new(1.0, Color32::from_white_alpha(24)),
            StrokeKind::Inside,
        );
        let inner = card.shrink2(vec2(30.0, 18.0));
        let head = self.draw_view_head(&mut top, inner, Which::Over, &name);
        let body = Rect::from_min_max(pos2(inner.left(), inner.top() + head), inner.max);
        self.draw_view(&mut top, body, Which::Over, None, true);
        if veil.clicked() {
            self.mods.over = None;
        }
    }

    /// A module's tab on a game's page; the content's height.
    pub fn draw_mod_tab(
        &mut self,
        ui: &mut Ui,
        body: Rect,
        g: &Game,
        i: u16,
        scroll: f32,
        focus: bool,
    ) -> f32 {
        let Some(m) = self.mods.list.get(i as usize).cloned() else {
            return 0.0;
        };
        let fresh = self
            .mods
            .game
            .as_ref()
            .is_none_or(|v| v.module != m.id || v.game_key.as_deref() != Some(&g.key));
        if fresh {
            self.mods.game = Some(ModView::new(
                &m.id,
                Some((g.key.clone(), self.game_json(g))),
                "game",
            ));
        }
        let mut head = 0.0;
        if self.mods.game.as_ref().is_some_and(|v| v.stack.len() > 1) {
            head = self.draw_view_head(
                ui,
                body.translate(vec2(0.0, -scroll - 64.0)),
                Which::Game,
                "",
            ) - 64.0;
        }
        let area = Rect::from_min_max(pos2(body.left(), body.top() + head), body.max);
        let w = area.width().min(1000.0);
        let area = Rect::from_min_size(area.min, vec2(w, area.height()));
        head + self.draw_view(ui, area, Which::Game, Some(scroll), focus)
    }

    /// A module's page: its items, as rows. The scroll is the page's own,
    /// or the game page's (`outer`). The content's height.
    fn draw_view(
        &mut self,
        ui: &mut Ui,
        area: Rect,
        w: Which,
        outer: Option<f32>,
        focused: bool,
    ) -> f32 {
        let Some(mut v) = self.slot(w).take() else {
            return 0.0;
        };
        let (c1, c2) = self.accent;
        // The module's own side panel, left of its page.
        let mut area = area;
        let mut focused = focused;
        if outer.is_none() && !v.nav.is_empty() {
            let nav_w = (area.width() * 0.28).clamp(210.0, 290.0);
            let nav_r = Rect::from_min_size(area.min, vec2(nav_w, area.height()));
            self.draw_nav(ui, nav_r, w, &mut v, focused);
            area = Rect::from_min_max(pos2(nav_r.right() + 28.0, area.top()), area.max);
            focused = focused && !v.side;
            // A page further in: its name and the way back, over it.
            if v.stack.len() > 1 {
                let sub = v.page.as_ref().map(|p| p.title.clone()).unwrap_or_default();
                let r = Rect::from_min_size(area.min, vec2(area.width(), 40.0));
                let resp = ui.interact(r, Id::new(("mod_back_in", w as u8)), Sense::click());
                ui.painter().text(
                    pos2(area.left() + 4.0, r.center().y),
                    Align2::LEFT_CENTER,
                    format!("‹ {sub}"),
                    FontId::proportional(20.0),
                    Color32::from_gray(if resp.hovered() { 255 } else { 215 }),
                );
                if resp.clicked() {
                    v.back();
                }
                area = Rect::from_min_max(pos2(area.left(), area.top() + 48.0), area.max);
            }
        }
        let p = ui.painter_at(area.expand2(vec2(8.0, 0.0)));
        let wrap = area.width() - 40.0;
        let mut y0 = 0.0;
        if let Some(e) = &v.error {
            let g = p.layout(
                e.clone(),
                FontId::proportional(16.0),
                Color32::from_rgb(255, 150, 150),
                wrap,
            );
            let h = g.size().y;
            p.galley(
                pos2(area.left() + 4.0, area.top() + 6.0 - outer.unwrap_or(0.0)),
                g,
                Color32::WHITE,
            );
            y0 = h + 22.0;
        }
        let Some(page) = v.page.clone() else {
            if v.error.is_none() && v.since.elapsed() > Duration::from_millis(250) {
                p.text(
                    pos2(area.left() + 4.0, area.top() + 10.0),
                    Align2::LEFT_TOP,
                    tr("mod_loading"),
                    FontId::proportional(18.0),
                    Color32::from_gray(160),
                );
            }
            *self.slot(w) = Some(v);
            return y0 + 60.0;
        };
        // Where each row is.
        let row_h = 56.0;
        let mut ys = Vec::with_capacity(page.items.len());
        let mut y = y0;
        let mut texts = Vec::new();
        for it in &page.items {
            ys.push(y);
            let h = match it.kind {
                Kind::Header => 46.0,
                Kind::Info(_) => 44.0,
                Kind::Progress(_) => 52.0,
                Kind::Text => {
                    let g = p.layout(
                        it.label.clone(),
                        FontId::proportional(16.0),
                        Color32::from_gray(185),
                        wrap,
                    );
                    let h = g.size().y + 14.0;
                    texts.push(Some(g));
                    y += h;
                    continue;
                }
                _ if !it.hint.is_empty() => row_h + 18.0,
                _ => row_h,
            };
            texts.push(None);
            y += h;
        }
        let total = y + 20.0;
        if page.items.is_empty() {
            p.text(
                pos2(area.left() + 4.0, area.top() + y0 + 10.0),
                Align2::LEFT_TOP,
                tr("mod_empty"),
                FontId::proportional(18.0),
                Color32::from_gray(150),
            );
        }
        let sel = v.sel.min(page.items.len().saturating_sub(1));
        let span = |i: usize| -> (f32, f32) {
            let top = ys.get(i).copied().unwrap_or(0.0);
            let bottom = ys.get(i + 1).copied().unwrap_or(y);
            (top, bottom)
        };
        let scroll = match outer {
            Some(s) => {
                // Keep the choice in view: the game page scrolls.
                if focused && self.game.as_ref().is_some_and(|g| g.follow) {
                    let (t, b) = span(sel);
                    if let Some(gv) = self.game.as_mut() {
                        if area.top() + b - s > area.bottom() {
                            gv.scroll = (b - area.height() + 20.0).max(0.0);
                        } else if t - s < 0.0 {
                            gv.scroll = t.max(0.0);
                        }
                    }
                }
                s
            }
            None => {
                let max = (total - area.height()).max(0.0);
                if ui.rect_contains_pointer(area) {
                    let d = ui.input(|i| i.smooth_scroll_delta.y);
                    if d != 0.0 {
                        v.scroll = (v.scroll - d).clamp(0.0, max);
                        v.follow = false;
                    }
                }
                if v.follow {
                    let (t, b) = span(sel);
                    let want = v
                        .scroll
                        .min(t - 40.0)
                        .max(b + 40.0 - area.height())
                        .clamp(0.0, max);
                    let k = self.ease(12.0);
                    v.scroll += (want - v.scroll) * k;
                    if (want - v.scroll).abs() < 0.5 {
                        v.scroll = want;
                        v.follow = false;
                    }
                }
                v.scroll.min(max)
            }
        };
        let mut act: Option<(usize, Act)> = None;
        let font = FontId::proportional(17.0);
        for (i, it) in page.items.iter().enumerate() {
            let top = area.top() + ys[i] - scroll;
            let (t, b) = span(i);
            let h = b - t;
            if top > area.bottom() || top + h < area.top() {
                continue;
            }
            match &it.kind {
                Kind::Header => {
                    p.text(
                        pos2(area.left() + 4.0, top + h - 12.0),
                        Align2::LEFT_BOTTOM,
                        &it.label,
                        FontId::proportional(15.0),
                        c1,
                    );
                    continue;
                }
                Kind::Progress(f) => {
                    let row = Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), h));
                    p.text(
                        pos2(row.left() + 20.0, row.top() + 6.0),
                        Align2::LEFT_TOP,
                        &it.label,
                        FontId::proportional(16.0),
                        Color32::from_gray(220),
                    );
                    if !it.hint.is_empty() {
                        p.text(
                            pos2(row.right() - 16.0, row.top() + 6.0),
                            Align2::RIGHT_TOP,
                            &it.hint,
                            FontId::proportional(14.0),
                            Color32::from_gray(160),
                        );
                    }
                    let bar = Rect::from_min_max(
                        pos2(row.left() + 20.0, row.bottom() - 16.0),
                        pos2(row.right() - 16.0, row.bottom() - 8.0),
                    );
                    p.rect_filled(bar, 4.0, Color32::from_white_alpha(25));
                    let done = Rect::from_min_size(bar.min, vec2(bar.width() * *f, bar.height()));
                    p.rect_filled(done, 4.0, c1);
                    continue;
                }
                Kind::Info(value) => {
                    let row = Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), h));
                    p.text(
                        pos2(row.left() + 20.0, row.center().y),
                        Align2::LEFT_CENTER,
                        &it.label,
                        FontId::proportional(17.0),
                        Color32::from_gray(160),
                    );
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        value.clone(),
                        FontId::proportional(17.0),
                        Color32::from_gray(235),
                    );
                    job.wrap.max_width = row.width() * 0.62;
                    job.wrap.max_rows = 1;
                    job.halign = egui::Align::RIGHT;
                    let g = p.fonts_mut(|f| f.layout_job(job));
                    p.galley(
                        pos2(row.right() - 16.0, row.center().y - g.size().y / 2.0),
                        g,
                        Color32::WHITE,
                    );
                    // A thin line under it.
                    p.hline(
                        row.left() + 20.0..=row.right() - 16.0,
                        row.bottom(),
                        Stroke::new(1.0, Color32::from_white_alpha(14)),
                    );
                    continue;
                }
                Kind::Text => {
                    if let Some(Some(g)) = texts.get(i).cloned() {
                        p.galley(pos2(area.left() + 4.0, top + 4.0), g, Color32::WHITE);
                    }
                    continue;
                }
                _ => {}
            }
            let row = Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), h - 6.0));
            let resp = ui.interact(
                row.intersect(area),
                Id::new(("mod_item", w as u8, i)),
                Sense::click_and_drag(),
            );
            let on = focused && i == sel;
            if on {
                p.rect_filled(row, 12.0, Color32::from_white_alpha(22));
            } else if resp.hovered() && !it.disabled {
                p.rect_filled(row, 12.0, Color32::from_white_alpha(10));
            }
            let tc = Color32::from_gray(if it.disabled {
                110
            } else if on {
                255
            } else {
                210
            });
            let line_y = if it.hint.is_empty() {
                row.center().y
            } else {
                row.top() + 25.0
            };
            p.text(
                pos2(row.left() + 20.0, line_y),
                Align2::LEFT_CENTER,
                &it.label,
                FontId::proportional(18.0),
                tc,
            );
            if !it.hint.is_empty() {
                p.text(
                    pos2(row.left() + 20.0, line_y + 22.0),
                    Align2::LEFT_CENTER,
                    &it.hint,
                    FontId::proportional(14.0),
                    Color32::from_gray(if it.disabled { 90 } else { 140 }),
                );
            }
            let vr = Rect::from_min_max(
                pos2(row.left() + row.width() * 0.52, line_y - 24.0),
                pos2(row.right() - 16.0, line_y + 24.0),
            );
            match &it.kind {
                Kind::Toggle(b) => {
                    let r = Rect::from_center_size(
                        pos2(vr.right() - 30.0, vr.center().y),
                        vec2(52.0, 28.0),
                    );
                    if *b {
                        widgets::gradient_pill(&p, r, 14.0, c1, c2);
                    } else {
                        p.rect_filled(r, 14.0, Color32::from_white_alpha(40));
                    }
                    let k = if *b {
                        r.right() - 14.0
                    } else {
                        r.left() + 14.0
                    };
                    p.circle_filled(pos2(k, r.center().y), 10.0, Color32::WHITE);
                    p.text(
                        pos2(r.left() - 14.0, r.center().y),
                        Align2::RIGHT_CENTER,
                        if *b { tr("on") } else { tr("off") },
                        font.clone(),
                        tc,
                    );
                }
                Kind::Slider {
                    value,
                    min,
                    max,
                    step,
                    unit,
                } => {
                    let bar = Rect::from_min_max(
                        pos2(vr.left() + 6.0, vr.center().y - 3.0),
                        pos2(vr.right() - 84.0, vr.center().y + 3.0),
                    );
                    let t = ((value - min) / (max - min)) as f32;
                    widgets::bar(&p, bar, t, c1, c2);
                    p.circle_filled(
                        pos2(bar.left() + bar.width() * t, bar.center().y),
                        if on { 9.0 } else { 7.0 },
                        Color32::WHITE,
                    );
                    let text = match num_value(*value, *step) {
                        Value::Number(n) => n.to_string(),
                        other => other.to_string(),
                    };
                    p.text(
                        pos2(vr.right(), vr.center().y),
                        Align2::RIGHT_CENTER,
                        format!("{text}{unit}"),
                        font.clone(),
                        tc,
                    );
                    if (resp.dragged() || resp.clicked())
                        && let Some(pos) = resp.interact_pointer_pos()
                        && pos.x >= bar.left() - 12.0
                        && pos.x <= bar.right() + 12.0
                    {
                        let t = ((pos.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
                        act = Some((i, Act::At(t as f64)));
                    }
                }
                Kind::Choice { value, options } => {
                    let text = options
                        .iter()
                        .find(|o| o.0 == *value)
                        .map(|o| o.1.clone())
                        .unwrap_or_else(|| modules::show(value));
                    p.text(vr.center(), Align2::CENTER_CENTER, text, font.clone(), tc);
                    let a = Color32::from_gray(if on { 230 } else { 110 });
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
                Kind::Button { text, danger } => {
                    let label = if text.is_empty() {
                        tr("mod_press").to_string()
                    } else {
                        text.clone()
                    };
                    let g = p.layout_no_wrap(label, FontId::proportional(16.0), Color32::WHITE);
                    let bw = (g.size().x + 40.0).max(120.0);
                    let r = Rect::from_min_max(
                        pos2(vr.right() - bw, vr.top() + 8.0),
                        pos2(vr.right(), vr.bottom() - 8.0),
                    );
                    let fill = if *danger && on {
                        Color32::from_rgb(190, 60, 70)
                    } else if *danger {
                        Color32::from_rgba_unmultiplied(190, 60, 70, 70)
                    } else if on {
                        c1.gamma_multiply(0.8)
                    } else {
                        Color32::from_white_alpha(25)
                    };
                    p.rect_filled(r, r.height() / 2.0, fill);
                    p.galley(r.center() - g.size() / 2.0, g, Color32::WHITE);
                }
                Kind::Link { value, .. } | Kind::File { value, .. } => {
                    p.text(
                        pos2(vr.right(), vr.center().y),
                        Align2::RIGHT_CENTER,
                        "›",
                        FontId::proportional(26.0),
                        tc,
                    );
                    p.text(
                        pos2(vr.right() - 24.0, vr.center().y),
                        Align2::RIGHT_CENTER,
                        value,
                        font.clone(),
                        Color32::from_gray(150),
                    );
                }
                Kind::Input { value, secret } => {
                    let text = if *secret {
                        "•".repeat(value.chars().count())
                    } else {
                        value.clone()
                    };
                    let r = Rect::from_min_max(
                        pos2(vr.left(), vr.top() + 8.0),
                        pos2(vr.right(), vr.bottom() - 8.0),
                    );
                    p.rect_filled(r, 10.0, Color32::from_black_alpha(90));
                    p.rect_stroke(
                        r,
                        10.0,
                        Stroke::new(1.0, Color32::from_white_alpha(if on { 120 } else { 40 })),
                        StrokeKind::Inside,
                    );
                    let mut job =
                        egui::text::LayoutJob::simple_singleline(text.clone(), font.clone(), tc);
                    job.wrap.max_width = r.width() - 24.0;
                    job.wrap.max_rows = 1;
                    let g = p.fonts_mut(|f| f.layout_job(job));
                    p.galley(
                        pos2(r.left() + 12.0, r.center().y - g.size().y / 2.0),
                        g,
                        Color32::WHITE,
                    );
                }
                Kind::Header | Kind::Text | Kind::Info(_) | Kind::Progress(_) => {}
            }
            if it.disabled {
                continue;
            }
            if resp.clicked() || resp.drag_started() {
                v.sel = i;
            }
            if resp.clicked() && act.is_none() {
                let x = resp.interact_pointer_pos().map(|p| p.x).unwrap_or(0.0);
                act = Some((
                    i,
                    match it.kind {
                        Kind::Choice { .. } if x < vr.left() + vr.width() * 0.25 => Act::Step(-1),
                        Kind::Choice { .. } => Act::Step(1),
                        _ => Act::Press,
                    },
                ));
            }
        }
        *self.slot(w) = Some(v);
        if let Some((i, a)) = act {
            self.view_act(w, i, a);
        }
        total
    }

    /// The module's own panel of sections.
    fn draw_nav(&mut self, ui: &mut Ui, r: Rect, w: Which, v: &mut ModView, focused: bool) {
        let c1 = self.accent.0;
        let p = ui.painter_at(r.expand(4.0));
        let row_h = 54.0;
        let root = v.stack.first().cloned().unwrap_or_default();
        let mut go = None;
        for (i, n) in v.nav.iter().enumerate() {
            let row = Rect::from_min_size(
                pos2(r.left(), r.top() + i as f32 * (row_h + 4.0)),
                vec2(r.width(), row_h),
            );
            if row.top() > r.bottom() {
                break;
            }
            let resp = ui.interact(row, Id::new(("mod_nav", w as u8, i)), Sense::click());
            let current = n.page == root || (i == 0 && !v.nav.iter().any(|n| n.page == root));
            let on = focused && v.side && current;
            if current {
                p.rect_filled(
                    row,
                    14.0,
                    Color32::from_white_alpha(if v.side { 26 } else { 16 }),
                );
            } else if resp.hovered() {
                p.rect_filled(row, 14.0, Color32::from_white_alpha(10));
            }
            if on {
                p.rect_stroke(row, 14.0, Stroke::new(2.0, c1), StrokeKind::Inside);
            }
            let color = Color32::from_gray(if current { 255 } else { 185 });
            let mut x = row.left() + 18.0;
            if let Some(ic) = n.icon.as_deref().and_then(icon_named) {
                widgets::icon(&p, pos2(x + 11.0, row.center().y), 22.0, ic, color);
                x += 38.0;
            }
            let lg = p.layout_no_wrap(n.label.clone(), FontId::proportional(18.0), color);
            let label_w = lg.size().x;
            p.galley(pos2(x, row.center().y - lg.size().y / 2.0), lg, color);
            if !n.value.is_empty() {
                // What's left beside the name, cut short with "…".
                let room = row.right() - 16.0 - (x + label_w + 14.0);
                if room > 24.0 {
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        n.value.clone(),
                        FontId::proportional(14.0),
                        Color32::from_gray(140),
                    );
                    job.wrap.max_width = room;
                    job.wrap.max_rows = 1;
                    let g = p.fonts_mut(|f| f.layout_job(job));
                    p.galley(
                        pos2(
                            row.right() - 16.0 - g.size().x,
                            row.center().y - g.size().y / 2.0,
                        ),
                        g,
                        Color32::WHITE,
                    );
                }
            }
            if resp.clicked() {
                go = Some(n.page.clone());
            }
        }
        if let Some(page) = go {
            v.root(&page);
            v.side = false;
        }
    }

    /// Do it to the item: change the value here at once, tell the module.
    fn view_act(&mut self, w: Which, i: usize, a: Act) {
        use crate::sfx::Sound;
        let Some(v) = self.view_mut(w) else { return };
        let Some(page) = v.page.as_mut() else { return };
        let Some(it) = page.items.get_mut(i) else {
            return;
        };
        if !it.selectable() {
            return;
        }
        let id = it.id.clone();
        let mut sound = None;
        let out = match (&mut it.kind, a) {
            (Kind::Toggle(b), Act::Step(d)) if (d > 0) == *b => None,
            (Kind::Toggle(b), _) => {
                *b = !*b;
                sound = Some(if *b {
                    Sound::ToggleOn
                } else {
                    Sound::ToggleOff
                });
                Some(Out::Set(id, json!(*b)))
            }
            (
                Kind::Slider {
                    value,
                    min,
                    max,
                    step,
                    ..
                },
                Act::Step(_) | Act::At(_),
            ) => {
                let new = match a {
                    Act::Step(d) => slide(*value, *min, *max, *step, d),
                    Act::At(t) => slide(*min + (*max - *min) * t, *min, *max, *step, 0),
                    Act::Press => *value,
                };
                if (new - *value).abs() < f64::EPSILON {
                    None
                } else {
                    sound = Some(if new > *value {
                        Sound::SliderUp
                    } else {
                        Sound::SliderDown
                    });
                    *value = new;
                    Some(Out::Set(id, num_value(new, *step)))
                }
            }
            (Kind::Choice { value, options }, Act::Step(_) | Act::Press) if !options.is_empty() => {
                let d = if let Act::Step(d) = a { d } else { 1 };
                let n = options.len() as i32;
                let at = options.iter().position(|o| o.0 == *value).unwrap_or(0) as i32;
                let next = options[(at + d).rem_euclid(n) as usize].0.clone();
                *value = next.clone();
                sound = Some(Sound::Navigate);
                Some(Out::Set(id, next))
            }
            (Kind::Button { danger: true, .. }, Act::Press) => Some(Out::Ask(id, it.label.clone())),
            (Kind::Button { .. }, Act::Press) => Some(Out::Press(id)),
            (Kind::Link { page, .. }, Act::Press | Act::Step(1)) => Some(Out::Open(page.clone())),
            (Kind::File { exts, start, .. }, Act::Press) => {
                Some(Out::File(id, it.label.clone(), exts.clone(), start.clone()))
            }
            (Kind::Input { value, secret }, Act::Press) => {
                Some(Out::Type(id, it.label.clone(), value.clone(), *secret))
            }
            _ => None,
        };
        if let Some(s) = sound {
            self.sfx.play(s);
        }
        let Some(out) = out else { return };
        match out {
            Out::Set(id, val) => {
                if let Some(v) = self.view_mut(w) {
                    v.set(&id, val);
                }
            }
            Out::Press(id) => {
                if let Some(v) = self.view_mut(w) {
                    v.press(&id);
                }
            }
            Out::Open(page) => {
                if let Some(v) = self.view_mut(w) {
                    v.open(&page);
                }
            }
            Out::Ask(id, label) => {
                self.ask(label, Vec::new(), tr("yes"), move |app| {
                    if let Some(v) = app.view_mut(w) {
                        v.press(&id);
                    }
                });
            }
            Out::File(id, label, exts, start) => {
                let exts: Vec<&str> = exts.iter().map(String::as_str).collect();
                let start = (!start.is_empty())
                    .then(|| std::path::PathBuf::from(&start))
                    .or_else(dirs::download_dir);
                self.pick_file(label, &exts, start, move |app, path| {
                    if let Some(v) = app.view_mut(w) {
                        v.set(&id, json!(path.display().to_string()));
                    }
                });
            }
            Out::Type(id, label, text, secret) => {
                self.type_text(label, text, move |app, s| {
                    let Some(v) = app.view_mut(w) else { return };
                    if let Some(it) = v
                        .page
                        .as_mut()
                        .and_then(|p| p.items.iter_mut().find(|it| it.id == id))
                    {
                        it.kind = Kind::Input {
                            value: s.clone(),
                            secret,
                        };
                    }
                    v.set(&id, json!(s));
                });
                if let Some(o) = self.osk.as_mut() {
                    o.secret = secret;
                }
            }
        }
    }

    /// The pad on a module's page; true: leave it (Back on its first page).
    pub fn modview_input(&mut self, w: Which, a: Action) -> bool {
        let Some(v) = self.view_mut(w) else {
            return true;
        };
        let has_nav = !v.nav.is_empty() && w != Which::Game;
        // The module's own panel: ↑ ↓ through its sections (shown at once),
        // → or A into one, LT/RT from anywhere.
        let at = v
            .nav
            .iter()
            .position(|n| Some(n.page.as_str()) == v.stack.first().map(String::as_str));
        let go = |v: &mut ModView, d: i32| {
            let n = v.nav.len() as i32;
            // A first page not in the panel ("main") is its first section.
            let i = (at.unwrap_or(0) as i32 + d).clamp(0, n - 1) as usize;
            let page = v.nav[i].page.clone();
            v.root(&page);
        };
        if has_nav {
            match a {
                Action::LeftTrigger => {
                    go(v, -1);
                    return false;
                }
                Action::RightTrigger => {
                    go(v, 1);
                    return false;
                }
                _ => {}
            }
        }
        if has_nav && v.side {
            match a {
                Action::Up => go(v, -1),
                Action::Down => go(v, 1),
                Action::Right | Action::Accept => {
                    v.side = false;
                    v.fix_sel();
                }
                Action::Back => return true,
                _ => {}
            }
            return false;
        }
        let sel = v.sel;
        // ← on what has no value to change: to the panel.
        let adjustable = v
            .page
            .as_ref()
            .and_then(|p| p.items.get(sel))
            .is_some_and(|it| {
                it.selectable()
                    && matches!(
                        it.kind,
                        Kind::Toggle(_) | Kind::Slider { .. } | Kind::Choice { .. }
                    )
            });
        match a {
            Action::Up => v.step(false),
            Action::Down => v.step(true),
            Action::Left if has_nav && !adjustable && v.stack.len() == 1 => v.side = true,
            Action::Left => self.view_act(w, sel, Act::Step(-1)),
            Action::Right => self.view_act(w, sel, Act::Step(1)),
            Action::Accept => self.view_act(w, sel, Act::Press),
            Action::Back if !v.back() => {
                if has_nav {
                    v.side = true;
                } else {
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    pub fn modview_hints(&self, w: Which) -> Vec<(Btn, &'static str)> {
        let mut out = Vec::new();
        if let Some(v) = self.view(w)
            && v.side
            && !v.nav.is_empty()
            && w != Which::Game
        {
            return vec![
                (Btn::Dpad, tr("mod_section")),
                (Btn::A, tr("open")),
                (
                    Btn::B,
                    if w == Which::Over {
                        tr("close")
                    } else {
                        tr("back")
                    },
                ),
            ];
        }
        let item = self
            .view(w)
            .and_then(|v| v.page.as_ref().and_then(|p| p.items.get(v.sel)));
        if let Some(it) = item.filter(|it| it.selectable()) {
            match it.kind {
                Kind::Toggle(_) => out.push((Btn::A, tr("mod_toggle"))),
                Kind::Slider { .. } | Kind::Choice { .. } => out.push((Btn::Dpad, tr("change"))),
                Kind::Button { .. } => out.push((Btn::A, tr("mod_press"))),
                Kind::Link { .. } | Kind::File { .. } => out.push((Btn::A, tr("open"))),
                Kind::Input { .. } => out.push((Btn::A, tr("mod_type"))),
                Kind::Header | Kind::Text | Kind::Info(_) | Kind::Progress(_) => {}
            }
        }
        let deep = self.view(w).is_some_and(|v| v.stack.len() > 1);
        out.push((
            Btn::B,
            if w == Which::Over && !deep {
                tr("close")
            } else {
                tr("back")
            },
        ));
        out
    }
}
