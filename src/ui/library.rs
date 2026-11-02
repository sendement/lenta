//! Every game in a grid, filtered by source.

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use super::card::{self, Pose};
use super::grid::{self, Layout};
use super::ribbon;
use crate::app::App;
use crate::config::Shape;
use crate::game::{Game, Source};
use crate::i18n::tr;
use crate::input::Action;

impl App {
    /// The sources that have games: the filters after "All".
    fn lib_filters(&self) -> Vec<Source> {
        Source::ALL
            .into_iter()
            .filter(|s| self.games.iter().any(|g| g.source == *s))
            .collect()
    }

    fn lib_games(&self) -> Vec<&Game> {
        let f = self.lib_filters();
        let pick = self
            .lib
            .filter
            .checked_sub(1)
            .and_then(|i| f.get(i).copied());
        let mut v: Vec<&Game> = self
            .games
            .iter()
            .filter(|g| pick.is_none_or(|s| g.source == s))
            .collect();
        v.sort_by_key(|g| g.title.to_lowercase());
        v
    }

    pub fn library_input(&mut self, a: Action) -> bool {
        let nf = self.lib_filters().len() + 1;
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
        let mut labels = vec![tr("all").to_string()];
        let filters = self.lib_filters();
        for s in &filters {
            labels.push(format!(
                "{} · {}",
                s.name(),
                self.games.iter().filter(|g| g.source == *s).count()
            ));
        }
        labels[0] = format!("{} · {}", tr("all"), self.games.len());
        if let Some(i) = grid::chips(
            ui,
            pos2(rect.left(), rect.top() + 66.0),
            &labels,
            self.lib.filter,
            self.accent.0,
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
            let tint = if selected || self.cfg.dim == 0.0 {
                Color32::WHITE
            } else {
                Color32::from_gray(((1.0 - self.cfg.dim * 0.5) * 255.0) as u8)
            };
            card::draw(&p, &pose, &fill, tint, None);
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
