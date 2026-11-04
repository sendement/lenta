//! The Sounds page: sound packs as cards. Two tabs (LT/RT): the packs
//! installed (Steam's own set first; A uses one, X removes it) and the
//! DeckThemes store of Decky AudioLoader packs, most downloaded first (A
//! installs and uses one).

use std::time::Duration;

use crossbeam_channel::Receiver;
use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use super::card::{self, Fill, Pose};
use super::grid::{self, Layout};
use crate::app::{App, Grid};
use crate::game::Art;
use crate::i18n::tr;
use crate::images::Kind;
use crate::input::Action;
use crate::sfx::{self, Listed, Pack};

#[derive(Default)]
pub struct SoundsPage {
    /// 0: installed, 1: store.
    pub tab: usize,
    pub grid: Grid,
    pub installed: Vec<Pack>,
    pub listed: Option<Result<Vec<Listed>, String>>,
    pub loading: Option<Receiver<Result<Vec<Listed>, String>>>,
    /// Installing a pack: its store id, then the outcome (its name).
    pub job: Option<(String, Receiver<Result<String, String>>)>,
    pub loaded: bool,
}

#[derive(Clone)]
enum Card {
    /// Steam's own sounds.
    Steam,
    Pack(Pack),
    Store(Listed),
}

impl App {
    fn sounds_load(&mut self) {
        if self.sounds.loaded {
            return;
        }
        self.sounds.loaded = true;
        self.sounds.installed = sfx::packs();
        // Last time's list at once; the fresh one replaces it when it comes.
        self.sounds.listed = sfx::store_saved().map(Ok);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(sfx::store_all());
            ctx.request_repaint();
        });
        self.sounds.loading = Some(rx);
    }

    fn sound_cards(&self) -> Vec<Card> {
        let s = &self.sounds;
        if s.tab == 0 {
            let mut v = vec![Card::Steam];
            v.extend(s.installed.iter().cloned().map(Card::Pack));
            v
        } else {
            match &s.listed {
                Some(Ok(l)) => l.iter().cloned().map(Card::Store).collect(),
                _ => Vec::new(),
            }
        }
    }

    /// A store entry for an installed pack (its picture).
    fn listed_for(&self, p: &Pack) -> Option<&Listed> {
        let list = self.sounds.listed.as_ref()?.as_ref().ok()?;
        list.iter()
            .find(|l| p.id.as_deref() == Some(l.id.as_str()) || l.name == p.name)
    }

    fn is_installed(&self, l: &Listed) -> Option<&Pack> {
        self.sounds
            .installed
            .iter()
            .find(|p| p.id.as_deref() == Some(l.id.as_str()) || p.name == l.name)
    }

    fn use_pack(&mut self, name: &str) {
        self.cfg.sound_pack = name.to_string();
        self.sfx.set(
            self.cfg.ui_sounds,
            &self.cfg.sound_pack,
            self.cfg.sound_volume,
        );
        // Hear it at once.
        self.sfx.play(crate::sfx::Sound::Activate);
    }

    fn sound_act(&mut self, c: Card, remove: bool) {
        match (c, remove) {
            (Card::Steam, false) => self.use_pack(""),
            (Card::Pack(p), false) => self.use_pack(&p.name),
            (Card::Pack(p), true) => {
                if !p.dir.starts_with(sfx::packs_dir()) {
                    self.toast(tr("sounds_decky"));
                    return;
                }
                self.ask(
                    format!("{} {}?", tr("pu_remove"), p.name),
                    vec![p.dir.display().to_string()],
                    tr("pu_remove_short"),
                    move |app| match sfx::remove(&p) {
                        Ok(()) => {
                            if app.cfg.sound_pack == p.name {
                                app.use_pack("");
                            }
                            app.sounds.installed = sfx::packs();
                            app.toast(tr("pu_removed"));
                        }
                        Err(e) => app.toast(format!("{}: {e}", tr("sd_failed"))),
                    },
                );
            }
            (Card::Store(l), false) => {
                if let Some(p) = self.is_installed(&l).cloned() {
                    self.use_pack(&p.name);
                    return;
                }
                if self.sounds.job.is_some() {
                    self.toast(tr("sd_busy"));
                    return;
                }
                let (tx, rx) = crossbeam_channel::bounded(1);
                let ctx = self.ctx.clone();
                let item = l.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(sfx::install(&item));
                    ctx.request_repaint();
                });
                self.sounds.job = Some((l.id, rx));
            }
            _ => {}
        }
    }

    /// Keys on the page; true on Back.
    pub fn sounds_input(&mut self, a: Action) -> bool {
        match a {
            Action::LeftTrigger | Action::RightTrigger => {
                self.sounds.tab = 1 - self.sounds.tab;
                self.sounds.grid = Grid::default();
                return false;
            }
            Action::Accept | Action::Select => {
                if let Some(c) = self.sound_cards().get(self.sounds.grid.sel).cloned() {
                    self.sound_act(c, a == Action::Select);
                }
                return false;
            }
            _ => {}
        }
        let n = self.sound_cards().len();
        grid::nav(&mut self.sounds.grid, n, a)
    }

    fn pump_sounds(&mut self) {
        if let Some(rx) = &self.sounds.loading
            && let Ok(r) = rx.try_recv()
        {
            self.sounds.loading = None;
            match r {
                Ok(l) => {
                    sfx::store_keep(&l);
                    if self.sounds.listed != Some(Ok(l.clone())) {
                        self.sounds.listed = Some(Ok(l));
                    }
                }
                // Offline: keep last time's list if there is one.
                Err(e) => {
                    if !matches!(self.sounds.listed, Some(Ok(_))) {
                        self.sounds.listed = Some(Err(e));
                    }
                }
            }
        }
        let done = self
            .sounds
            .job
            .as_ref()
            .and_then(|(_, rx)| rx.try_recv().ok());
        if let Some(r) = done {
            self.sounds.job = None;
            self.sounds.installed = sfx::packs();
            match r {
                Ok(name) => {
                    self.use_pack(&name);
                    self.toast(format!("{}: {name}", tr("sounds_now")));
                }
                Err(e) => self.toast(format!("{}: {e}", tr("sd_failed"))),
            }
        }
    }

    pub fn draw_sounds(&mut self, ui: &mut Ui, rect: Rect) {
        self.sounds_load();
        self.pump_sounds();
        if self.sounds.loading.is_some() || self.sounds.job.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        let rect = rect.shrink2(vec2(36.0, 0.0));
        let p = ui.painter_at(rect);
        let (c1, c2) = self.accent;
        p.text(
            pos2(rect.left(), rect.top() + 34.0),
            Align2::LEFT_CENTER,
            tr("sound_store"),
            FontId::proportional(32.0),
            Color32::WHITE,
        );
        // The two tabs.
        let store_label = match (&self.sounds.listed, self.sounds.loading.is_some()) {
            (Some(Ok(l)), _) => format!("{} · {}", tr("sounds_store_tab"), l.len()),
            (_, true) => format!("{} · {}", tr("sounds_store_tab"), tr("loading")),
            _ => tr("sounds_store_tab").to_string(),
        };
        let labels = vec![
            format!(
                "{} · {}",
                tr("sounds_installed"),
                self.sounds.installed.len() + 1
            ),
            store_label,
        ];
        let accent = self.accent;
        if let Some(i) = grid::chips(
            ui,
            pos2(rect.left(), rect.top() + 66.0),
            rect.right(),
            &labels,
            self.sounds.tab,
            &|_| accent,
        ) {
            self.sounds.tab = i;
            self.sounds.grid = Grid::default();
        }
        if self.sounds.tab == 1
            && let Some(Err(e)) = &self.sounds.listed
        {
            p.text(
                pos2(rect.left(), rect.top() + 140.0),
                Align2::LEFT_TOP,
                e,
                FontId::proportional(16.0),
                Color32::from_gray(170),
            );
        }
        let cards = self.sound_cards();
        let view = Rect::from_min_max(
            pos2(rect.left() - 30.0, rect.top() + 116.0),
            pos2(rect.right() + 30.0, rect.bottom()),
        );
        let cell = vec2(300.0, 228.0);
        let lay = Layout::new(
            view.shrink2(vec2(30.0, 0.0)).translate(vec2(0.0, 16.0)),
            cell,
            22.0,
        );
        self.sounds.grid.cols = lay.cols;
        self.sounds.grid.sel = self.sounds.grid.sel.min(cards.len().saturating_sub(1));
        let ease = self.ease(12.0);
        grid::scroll(ui, &mut self.sounds.grid, &lay, view, cards.len(), ease);
        let pv = ui.painter_at(view);
        let current = self.cfg.sound_pack.clone();
        let job = self.sounds.job.as_ref().map(|j| j.0.clone());
        let mut clicked = None;
        for (i, c) in cards.iter().enumerate() {
            let r = lay.rect(i, self.sounds.grid.scroll);
            if r.bottom() < view.top() || r.top() > view.bottom() {
                continue;
            }
            let resp = ui.interact(
                r.intersect(view),
                ui.id().with(("sound", i)),
                Sense::click(),
            );
            let selected = i == self.sounds.grid.sel && !self.side_focus;
            // The picture: the store's, else a gradient with the name.
            let pic = Rect::from_min_size(r.min, vec2(r.width(), 168.0));
            let (name, sub, image, in_use, state) = match c {
                Card::Steam => (
                    tr("sounds_steam").to_string(),
                    tr("sounds_steam_note").to_string(),
                    None,
                    current.is_empty(),
                    String::new(),
                ),
                Card::Pack(pk) => (
                    pk.name.clone(),
                    if pk.author.is_empty() {
                        String::new()
                    } else {
                        format!("{} {}", tr("sounds_by"), pk.author)
                    },
                    self.listed_for(pk).and_then(|l| l.image.clone()),
                    current == pk.name,
                    String::new(),
                ),
                Card::Store(l) => {
                    let have = self.is_installed(l).map(|p| p.name.clone());
                    (
                        l.name.clone(),
                        format!(
                            "{} {}  ·  {} {}",
                            tr("sounds_by"),
                            l.author,
                            l.downloads,
                            tr("sounds_downloads")
                        ),
                        l.image.clone(),
                        have.as_ref().is_some_and(|n| *n == current),
                        if job.as_deref() == Some(l.id.as_str()) {
                            tr("sounds_getting").to_string()
                        } else if have.is_some() {
                            tr("sounds_installed_one").to_string()
                        } else {
                            String::new()
                        },
                    )
                }
            };
            let grow = if selected { 1.04 } else { 1.0 };
            let pose = Pose::flat(
                Rect::from_center_size(pic.center(), pic.size() * grow),
                14.0,
            );
            card::shadow(&pv, &pose, 1.0);
            if selected {
                card::glow(&pv, &pose, c1, c2, self.cfg.glow * 0.7, 1.0);
            }
            let tex = image.and_then(|u| self.images.get(&Art::Url(u), Kind::Fit(512)));
            match tex {
                Some(t) => card::draw(
                    &pv,
                    &pose,
                    &Fill::Image(t.handle.id(), card::cover_uv(t.size, pose.size)),
                    Color32::WHITE,
                    None,
                ),
                None => {
                    let (a, b) = if matches!(c, Card::Steam) {
                        crate::ui::widgets::source_colors(crate::game::Source::Steam)
                    } else {
                        super::ribbon::own_colors(&name)
                    };
                    card::draw(&pv, &pose, &Fill::Gradient(a, b), Color32::WHITE, None);
                    crate::ui::widgets::icon(
                        &pv,
                        pose.center + vec2(0.0, -14.0),
                        44.0,
                        crate::ui::widgets::Icon::Sound,
                        Color32::from_white_alpha(220),
                    );
                    pv.text(
                        pose.center + vec2(0.0, 34.0),
                        Align2::CENTER_CENTER,
                        &name,
                        FontId::proportional(17.0),
                        Color32::WHITE,
                    );
                }
            }
            // In use / installed / installing, in the corner.
            let tag = if in_use {
                Some((tr("sounds_in_use").to_string(), true))
            } else if !state.is_empty() {
                Some((state, false))
            } else {
                None
            };
            if let Some((t, on)) = tag {
                let g = pv.layout_no_wrap(
                    t,
                    FontId::proportional(13.0),
                    if on {
                        Color32::WHITE
                    } else {
                        Color32::from_rgb(20, 22, 30)
                    },
                );
                let tr_ =
                    Rect::from_min_size(pic.min + vec2(10.0, 10.0), g.size() + vec2(16.0, 8.0));
                if on {
                    crate::ui::widgets::gradient_pill(&pv, tr_, tr_.height() / 2.0, c1, c2);
                } else {
                    // Light, outlined: readable on dark pictures too.
                    pv.rect_filled(
                        tr_,
                        tr_.height() / 2.0,
                        Color32::from_rgba_unmultiplied(235, 238, 245, 230),
                    );
                    pv.rect_stroke(
                        tr_,
                        tr_.height() / 2.0,
                        egui::Stroke::new(1.0, Color32::from_black_alpha(120)),
                        egui::StrokeKind::Outside,
                    );
                }
                pv.galley(tr_.min + vec2(8.0, 4.0), g, Color32::PLACEHOLDER);
            }
            let mut job_n = egui::text::LayoutJob::simple_singleline(
                name.clone(),
                FontId::proportional(16.0),
                Color32::from_gray(if selected { 255 } else { 220 }),
            );
            job_n.wrap.max_width = r.width();
            job_n.wrap.max_rows = 1;
            let ng = pv.fonts_mut(|f| f.layout_job(job_n));
            pv.galley(
                pos2(r.left() + 2.0, pic.bottom() + 12.0),
                ng,
                Color32::WHITE,
            );
            let mut job_s = egui::text::LayoutJob::simple_singleline(
                sub,
                FontId::proportional(13.0),
                Color32::from_gray(150),
            );
            job_s.wrap.max_width = r.width();
            job_s.wrap.max_rows = 1;
            let sg = pv.fonts_mut(|f| f.layout_job(job_s));
            pv.galley(
                pos2(r.left() + 2.0, pic.bottom() + 36.0),
                sg,
                Color32::WHITE,
            );
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.side_focus = false;
            if self.sounds.grid.sel == i {
                self.sound_act(cards[i].clone(), false);
            } else {
                self.sounds.grid.sel = i;
            }
        }
    }
}
