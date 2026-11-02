//! Every game in a grid, filtered by source.

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use super::card::{self, Pose};
use super::grid::{self, Layout};
use super::ribbon;
use super::widgets;
use crate::app::App;
use crate::config::Shape;
use crate::game::{Game, Source};
use crate::i18n::tr;
use crate::input::Action;

/// What the library shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibFilter {
    All,
    Source(Source),
    /// Owned on Steam, not installed (after signing in).
    NotInstalled,
    /// One of Steam's collections (index into `steam.collections`).
    Collection(usize),
}

impl App {
    /// Installed games (the hidden ones aside) and, if wanted, the
    /// account's others.
    fn lib_main(&self) -> impl Iterator<Item = &Game> {
        let more = self
            .steam
            .more
            .iter()
            .filter(|g| !g.installed && self.cfg.show_uninstalled);
        self.games.iter().chain(more)
    }

    /// The filters with something in them: All, the sources, Not
    /// installed, then the collections.
    pub(crate) fn lib_filters(&self) -> Vec<LibFilter> {
        let mut v = vec![LibFilter::All];
        v.extend(
            Source::ALL
                .into_iter()
                .filter(|s| self.lib_main().any(|g| g.source == *s))
                .map(LibFilter::Source),
        );
        if self.cfg.show_uninstalled && self.steam.more.iter().any(|g| !g.installed) {
            v.push(LibFilter::NotInstalled);
        }
        for i in 0..self.steam.collections.len() {
            if self.collection_games(i).next().is_some() {
                v.push(LibFilter::Collection(i));
            }
        }
        v
    }

    fn collection_games(&self, i: usize) -> impl Iterator<Item = &Game> {
        let ids: std::collections::HashSet<u32> = self
            .steam
            .collections
            .get(i)
            .map(|c| c.apps.iter().copied().collect())
            .unwrap_or_default();
        self.games
            .iter()
            .chain(&self.steam.more)
            .filter(move |g| g.steam_appid.is_some_and(|id| ids.contains(&id)))
            .filter(|g| g.installed || self.cfg.show_uninstalled)
    }

    pub(crate) fn lib_filter(&self) -> LibFilter {
        self.lib_filters()
            .get(self.lib.filter)
            .copied()
            .unwrap_or(LibFilter::All)
    }

    fn lib_games(&self) -> Vec<&Game> {
        let mut v: Vec<&Game> = match self.lib_filter() {
            LibFilter::All => self.lib_main().collect(),
            LibFilter::Source(s) => self.lib_main().filter(|g| g.source == s).collect(),
            LibFilter::NotInstalled => self.steam.more.iter().filter(|g| !g.installed).collect(),
            LibFilter::Collection(i) => self.collection_games(i).collect(),
        };
        // Installed first, then by name.
        v.sort_by_key(|g| (!g.installed, g.title.to_lowercase()));
        v
    }

    fn filter_label(&self, f: LibFilter) -> String {
        let (name, n) = match f {
            LibFilter::All => (tr("all").to_string(), self.lib_main().count()),
            LibFilter::Source(s) => (
                s.name().to_string(),
                self.lib_main().filter(|g| g.source == s).count(),
            ),
            LibFilter::NotInstalled => (
                tr("not_installed").to_string(),
                self.steam.more.iter().filter(|g| !g.installed).count(),
            ),
            LibFilter::Collection(i) => (
                self.steam.collections[i].name.clone(),
                self.collection_games(i).count(),
            ),
        };
        format!("{name} · {n}")
    }

    pub fn library_input(&mut self, a: Action) -> bool {
        let nf = self.lib_filters().len();
        match a {
            Action::LeftTrigger | Action::RightTrigger => {
                let d = if a == Action::RightTrigger { 1 } else { nf - 1 };
                self.lib.filter = (self.lib.filter + d) % nf;
                self.lib.sel = 0;
                self.lib.scroll = 0.0;
                return false;
            }
            Action::Accept | Action::Select => {
                if let Some(g) = self.lib_games().get(self.lib.sel) {
                    let k = g.key.clone();
                    if a == Action::Accept {
                        self.open_game(k);
                    } else {
                        self.play(&k);
                    }
                }
                return false;
            }
            _ => {}
        }
        let n = self.lib_games().len();
        grid::nav(&mut self.lib, n, a)
    }

    pub fn draw_library(&mut self, ui: &mut Ui, rect: Rect) {
        let rect = rect.shrink2(vec2(36.0, 0.0));
        let p = ui.painter_at(rect);
        let games: Vec<Game> = self.lib_games().into_iter().cloned().collect();
        p.text(
            pos2(rect.left(), rect.top() + 34.0),
            Align2::LEFT_CENTER,
            tr("library"),
            FontId::proportional(32.0),
            Color32::WHITE,
        );
        let filters = self.lib_filters();
        self.lib.filter = self.lib.filter.min(filters.len().saturating_sub(1));
        let labels: Vec<String> = filters.iter().map(|f| self.filter_label(*f)).collect();
        if let Some(i) = grid::chips(
            ui,
            pos2(rect.left(), rect.top() + 66.0),
            rect.right(),
            &labels,
            self.lib.filter,
            &|i| match filters.get(i) {
                Some(LibFilter::Source(s)) => widgets::source_colors(*s),
                Some(LibFilter::Collection(_)) => widgets::source_colors(Source::Steam),
                _ => self.accent,
            },
        ) {
            self.lib.filter = i;
            self.lib.sel = 0;
            self.lib.scroll = 0.0;
        }
        let view = Rect::from_min_max(
            pos2(rect.left() - 30.0, rect.top() + 116.0),
            pos2(rect.right() + 30.0, rect.bottom()),
        );
        let cell = match self.cfg.shape {
            Shape::Portrait => vec2(160.0, 240.0),
            Shape::Landscape => vec2(260.0, 260.0 * 215.0 / 460.0),
        };
        let title_h = 30.0;
        let lay = Layout::new(
            view.shrink2(vec2(30.0, 0.0)).translate(vec2(0.0, 20.0)),
            cell + vec2(0.0, title_h),
            22.0,
        );
        self.lib.cols = lay.cols;
        self.lib.sel = self.lib.sel.min(games.len().saturating_sub(1));
        let ease = self.ease(12.0);
        grid::scroll(ui, &mut self.lib, &lay, view, games.len(), ease);
        let p = ui.painter_at(view);
        let (c1, c2) = self.accent;
        let pulse = ((self.time * 2.2).sin() * 0.5 + 0.5) as f32;
        for (i, g) in games.iter().enumerate() {
            let cell_r = lay.rect(i, self.lib.scroll);
            if cell_r.bottom() < view.top() || cell_r.top() > view.bottom() {
                continue;
            }
            let r = Rect::from_min_size(cell_r.min, cell);
            let selected = i == self.lib.sel && !self.side_focus;
            let resp = ui.interact(
                r.intersect(view),
                ui.id().with(("lib", &g.key)),
                Sense::click(),
            );
            let grow = if selected {
                1.06
            } else if resp.hovered() {
                1.03
            } else {
                1.0
            };
            let pose = Pose::flat(
                Rect::from_center_size(r.center(), r.size() * grow),
                self.cfg.rounding * 0.7,
            );
            let (fill, has_art) = ribbon::fill_for(&mut self.images, g, self.cfg.shape, pose.size);
            card::shadow(&p, &pose, 1.0);
            if selected {
                card::glow(&p, &pose, c1, c2, self.cfg.glow * 0.7, 0.75 + 0.25 * pulse);
            }
            let mut light = if selected || self.cfg.dim == 0.0 {
                1.0
            } else {
                1.0 - self.cfg.dim * 0.5
            };
            // Not installed: faded, with a mark.
            if !g.installed {
                light *= 0.55;
            }
            let tint = Color32::from_gray((light * 255.0) as u8);
            card::draw(&p, &pose, &fill, tint, None);
            if !g.installed {
                let b = pose.bounds();
                let mark = p.layout_no_wrap(
                    tr("not_installed_one").to_string(),
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
                let r = Rect::from_min_size(
                    pos2(b.left() + 8.0, b.bottom() - 8.0 - mark.size().y - 8.0),
                    mark.size() + vec2(16.0, 8.0),
                );
                p.rect_filled(r, r.height() / 2.0, Color32::from_black_alpha(170));
                p.galley(r.min + vec2(8.0, 4.0), mark, Color32::WHITE);
            }
            if !has_art {
                ribbon::title_on(&p, &mut self.images, g, pose.bounds(), 1.0);
            }
            let galley = p.layout(
                g.title.clone(),
                FontId::proportional(14.0),
                Color32::from_gray(if selected { 255 } else { 190 }),
                cell.x,
            );
            let mut job = egui::text::LayoutJob::simple_singleline(
                g.title.clone(),
                FontId::proportional(14.0),
                Color32::from_gray(if selected { 255 } else { 190 }),
            );
            job.wrap.max_width = cell.x;
            job.wrap.max_rows = 1;
            let one = p.fonts_mut(|f| f.layout_job(job));
            let _ = galley;
            p.galley(
                pos2(r.center().x - one.size().x / 2.0, r.bottom() + 8.0),
                one,
                Color32::WHITE,
            );
            if resp.clicked() {
                self.side_focus = false;
                if self.lib.sel == i {
                    self.open_game(g.key.clone());
                } else {
                    self.lib.sel = i;
                }
            }
        }
    }
}
