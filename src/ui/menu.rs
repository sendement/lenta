//! The context menu: on a game (Y, or the right mouse button) — open,
//! play, and which of the launcher's own collections it's in; on one of
//! those collections — open, change, delete.

use egui::{
    Align2, Color32, FontId, Id, LayerId, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2,
    vec2,
};

use crate::app::App;
use crate::i18n::tr;
use crate::input::Action;

#[derive(Clone, Debug, PartialEq)]
pub enum MenuFor {
    Game(String),
    Coll(u64),
    /// Power off, restart, log out.
    Power,
}

pub struct Menu {
    pub what: MenuFor,
    pub sel: usize,
    /// Where the mouse opened it (the pad's opens in the middle).
    pub at: Option<Pos2>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Open,
    Play,
    /// In this collection or not (a tick).
    Toggle(u64),
    NewColl,
    Edit(u64),
    Delete(u64),
    Power(crate::power::Power),
    Move,
    Uninstall,
    CancelJob,
    /// An own program's settings.
    ProgSettings,
    /// Libraries in its prefix (winetricks, protontricks).
    Tricks,
    /// DLL mods beside it.
    DllMods,
    /// Give GSE the achievements list it lacks.
    GseAch,
}

impl App {
    pub fn open_menu(&mut self, what: MenuFor, at: Option<Pos2>) {
        self.menu = Some(Menu { what, sel: 0, at });
    }

    fn menu_items(&self) -> Vec<(Item, String, Option<bool>)> {
        let Some(m) = &self.menu else {
            return Vec::new();
        };
        match &m.what {
            MenuFor::Game(key) => {
                let Some(g) = self.game_by_key(key) else {
                    return Vec::new();
                };
                let play = g.play_word();
                let mut v = vec![
                    (Item::Open, tr("open").to_string(), None),
                    (Item::Play, play.to_string(), None),
                ];
                if self
                    .manage
                    .job
                    .as_ref()
                    .is_some_and(|j| j.key.as_deref() == Some(key.as_str()))
                {
                    v.push((Item::CancelJob, tr("mg_cancel").to_string(), None));
                } else {
                    if self.can_move(key) {
                        v.push((Item::Move, tr("mg_move_dots").to_string(), None));
                    }
                    if g.source == crate::game::Source::Own {
                        v.push((Item::ProgSettings, tr("pg_settings").to_string(), None));
                    }
                    if self.can_tricks(key) {
                        v.push((Item::Tricks, tr("pg_tricks").to_string(), None));
                    }
                    if self.can_dll_mods(key) {
                        v.push((Item::DllMods, tr("dm_menu").to_string(), None));
                    }
                    if self.can_gse_ach(key) {
                        v.push((Item::GseAch, tr("gse_menu").to_string(), None));
                    }
                    if self.can_uninstall(key) {
                        v.push((Item::Uninstall, tr("mg_uninstall_dots").to_string(), None));
                    }
                }
                let facts = self.facts(g);
                for c in &self.colls.list {
                    v.push((Item::Toggle(c.id), c.name.clone(), Some(c.has(g, &facts))));
                }
                v.push((Item::NewColl, tr("coll_new_dots").to_string(), None));
                v
            }
            MenuFor::Coll(id) => vec![
                (Item::Open, tr("open").to_string(), None),
                (Item::Edit(*id), tr("coll_edit").to_string(), None),
                (Item::Delete(*id), tr("coll_delete").to_string(), None),
            ],
            MenuFor::Power => crate::power::Power::ALL
                .iter()
                .map(|p| (Item::Power(*p), tr(p.key()).to_string(), None))
                .collect(),
        }
    }

    fn menu_act(&mut self, it: Item) {
        let Some(what) = self.menu.as_ref().map(|m| m.what.clone()) else {
            return;
        };
        match (it, what) {
            (Item::Open, MenuFor::Game(k)) => {
                self.menu = None;
                self.open_game(k);
            }
            (Item::Play, MenuFor::Game(k)) => {
                self.menu = None;
                self.play(&k);
            }
            (Item::Toggle(id), MenuFor::Game(k)) => {
                let Some(g) = self.game_by_key(&k) else {
                    return;
                };
                let facts = self.facts(g);
                let inside = self.colls.get(id).is_some_and(|c| c.has(g, &facts));
                if let Some(c) = self.colls.get_mut(id) {
                    if inside {
                        c.remove(&k);
                    } else {
                        c.add(&k);
                    }
                }
                self.save_colls();
            }
            (Item::NewColl, MenuFor::Game(k)) => {
                self.menu = None;
                self.new_coll_with(k);
            }
            (Item::Open, MenuFor::Coll(id)) => {
                self.menu = None;
                self.open_my(id);
            }
            (Item::Edit(id), _) => {
                self.menu = None;
                self.edit_coll(Some(id), None);
            }
            (Item::Delete(id), _) => {
                self.menu = None;
                self.ask_delete_coll(id);
            }
            (Item::Move, MenuFor::Game(k)) => {
                self.menu = None;
                self.manage_move(&k);
            }
            (Item::GseAch, MenuFor::Game(k)) => {
                self.menu = None;
                self.gse_ach(&k);
            }
            (Item::DllMods, MenuFor::Game(k)) => {
                self.menu = None;
                self.dll_mods(&k);
            }
            (Item::Tricks, MenuFor::Game(k)) => {
                self.menu = None;
                if self.is_running(&k) {
                    self.toast(tr("mg_close_game"));
                } else {
                    self.game_tricks(&k);
                }
            }
            (Item::ProgSettings, MenuFor::Game(k)) => {
                self.menu = None;
                self.program_settings(&k);
            }
            (Item::Uninstall, MenuFor::Game(k)) => {
                self.menu = None;
                self.manage_uninstall(&k);
            }
            (Item::CancelJob, _) => {
                self.menu = None;
                self.cancel_manage();
            }
            (Item::Power(pw), _) => {
                self.menu = None;
                self.ask_power(pw);
            }
            _ => {}
        }
    }

    pub fn menu_input(&mut self, a: Action) {
        let items = self.menu_items();
        let n = items.len();
        let Some(m) = &mut self.menu else { return };
        match a {
            Action::Up => m.sel = m.sel.saturating_sub(1),
            Action::Down => m.sel = (m.sel + 1).min(n.saturating_sub(1)),
            Action::Accept => {
                let it = items.get(m.sel).map(|i| i.0);
                if let Some(it) = it {
                    self.menu_act(it);
                }
            }
            Action::Back | Action::Actions | Action::Menu => self.menu = None,
            _ => {}
        }
    }

    pub fn draw_menu(&mut self, ui: &mut Ui, area: Rect) {
        let items = self.menu_items();
        let Some(m) = &self.menu else { return };
        if items.is_empty() {
            self.menu = None;
            return;
        }
        let layer = LayerId::new(Order::Foreground, Id::new("ctx_menu"));
        let p = ui.ctx().layer_painter(layer);
        let (c1, _) = self.accent;
        let title = match &m.what {
            MenuFor::Game(k) => self
                .game_by_key(k)
                .map(|g| g.title.clone())
                .unwrap_or_default(),
            MenuFor::Coll(id) => self
                .colls
                .get(*id)
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            MenuFor::Power => tr("power").to_string(),
        };
        let row_h = 44.0;
        let w = 380.0;
        // A line above the collections' ticks.
        let ticks_at = items.iter().position(|i| i.2.is_some());
        let label_h = if ticks_at.is_some() || items.iter().any(|i| i.0 == Item::NewColl) {
            30.0
        } else {
            0.0
        };
        let h = 18.0 + 34.0 + items.len() as f32 * row_h + label_h + 14.0;
        let card = match m.at {
            Some(at) => {
                let r = Rect::from_min_size(at + vec2(4.0, 4.0), vec2(w, h));
                // Kept inside the window.
                r.translate(vec2(
                    (area.right() - 12.0 - r.right()).min(0.0),
                    (area.bottom() - 12.0 - r.bottom()).min(0.0),
                ))
            }
            None => Rect::from_center_size(area.center(), vec2(w, h)),
        };
        let top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(area));
        // A click outside closes it.
        let veil = top.interact(area, Id::new("ctx_veil"), Sense::click());
        p.rect_filled(area, 0.0, Color32::from_black_alpha(90));
        p.rect_filled(card, 18.0, Color32::from_rgb(26, 29, 39));
        p.rect_stroke(
            card,
            18.0,
            Stroke::new(1.0, Color32::from_white_alpha(24)),
            StrokeKind::Inside,
        );
        let mut job = egui::text::LayoutJob::simple_singleline(
            title,
            FontId::proportional(17.0),
            Color32::from_gray(170),
        );
        job.wrap.max_width = w - 40.0;
        job.wrap.max_rows = 1;
        let tg = p.fonts_mut(|f| f.layout_job(job));
        p.galley(card.min + vec2(20.0, 18.0), tg, Color32::WHITE);
        let mut y = card.top() + 18.0 + 34.0;
        let sel = m.sel;
        let mut act = None;
        for (i, (it, label, tick)) in items.iter().enumerate() {
            if Some(i) == ticks_at || (ticks_at.is_none() && *it == Item::NewColl && label_h > 0.0)
            {
                p.text(
                    pos2(card.left() + 20.0, y + 8.0),
                    Align2::LEFT_TOP,
                    tr("my_colls"),
                    FontId::proportional(13.0),
                    Color32::from_gray(130),
                );
                y += label_h;
            }
            let r = Rect::from_min_size(pos2(card.left() + 8.0, y), vec2(w - 16.0, row_h - 4.0));
            y += row_h;
            let resp = top.interact(r, Id::new(("ctx_item", i)), Sense::click());
            let on = i == sel;
            if on || resp.hovered() {
                p.rect_filled(r, 12.0, Color32::from_white_alpha(if on { 30 } else { 16 }));
            }
            if on {
                p.rect_stroke(r, 12.0, Stroke::new(2.0, c1), StrokeKind::Inside);
            }
            let mut x = r.left() + 14.0;
            if let Some(t) = tick {
                let b = Rect::from_center_size(pos2(x + 10.0, r.center().y), vec2(20.0, 20.0));
                if *t {
                    p.rect_filled(b, 6.0, c1);
                    tick_mark(&p, b);
                } else {
                    p.rect_stroke(
                        b,
                        6.0,
                        Stroke::new(1.6, Color32::from_gray(170)),
                        StrokeKind::Inside,
                    );
                }
                x += 32.0;
            }
            let color = if matches!(it, Item::Delete(_)) {
                Color32::from_rgb(255, 140, 140)
            } else {
                Color32::WHITE
            };
            p.text(
                pos2(x, r.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(17.0),
                color,
            );
            if resp.clicked() {
                act = Some((i, *it));
            }
        }
        if let Some((i, it)) = act {
            if let Some(m) = &mut self.menu {
                m.sel = i;
            }
            self.menu_act(it);
        } else if veil.clicked() || veil.secondary_clicked() {
            self.menu = None;
        }
    }
}

/// A tick inside a box.
pub fn tick_mark(p: &egui::Painter, b: Rect) {
    let s = b.width();
    p.line(
        vec![
            b.left_top() + vec2(s * 0.24, s * 0.52),
            b.left_top() + vec2(s * 0.43, s * 0.70),
            b.left_top() + vec2(s * 0.77, s * 0.32),
        ],
        Stroke::new(2.4, Color32::WHITE),
    );
}
