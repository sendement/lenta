//! The home screen: the ribbon, the selected game's art behind it and its
//! details beside it. Also the shared background, clock and toast.

use egui::epaint::{Mesh, Vertex};
use egui::{Align2, Color32, FontId, Pos2, Rect, Ui, pos2, vec2};

use super::card;
use super::ribbon::{self, Look};
use super::widgets::{self, Icon};
use crate::app::App;
use crate::config::Orientation;
use crate::game::{Art, Game};
use crate::i18n::{self, tr};
use crate::images::Kind;
use crate::input::Action;
use crate::store::State;

/// A soft round glow: `color` in the middle, nothing at `radius`.
pub fn blob(p: &egui::Painter, c: Pos2, radius: f32, color: Color32) {
    let mut m = Mesh::default();
    m.colored_vertex(c, color);
    let n = 48;
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        m.vertices.push(Vertex {
            pos: c + vec2(a.cos(), a.sin()) * radius,
            uv: egui::epaint::WHITE_UV,
            color: Color32::TRANSPARENT,
        });
    }
    for i in 0..n {
        m.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(m);
}

/// Draw `tex` covering `rect`, tinted.
pub fn cover(p: &egui::Painter, rect: Rect, tex: &crate::images::Tex, tint: Color32) {
    let uv = card::cover_uv(tex.size, rect.size());
    p.image(tex.handle.id(), rect, uv, tint);
}

/// Local time as "HH:MM".
pub fn clock() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    }
}

impl App {
    /// The art behind the page: the selected game's (home, game page),
    /// fading into the next; accent glows over a dark base.
    pub fn draw_background(&mut self, ui: &mut Ui, full: Rect) {
        let p = ui.painter_at(full);
        // A launcher's tab in the library tints the page in its colours.
        let source = (self.page == crate::app::Page::Library && self.game.is_none())
            .then(|| self.lib_source())
            .flatten();
        let target = match source {
            Some(s) => {
                let (a, b) = widgets::source_colors(s);
                (a, b, 1.0)
            }
            None => (self.accent.0, self.accent.1, 0.0),
        };
        let k = self.ease(6.0);
        self.tone = (
            card::lerp(self.tone.0, target.0, k),
            card::lerp(self.tone.1, target.1, k),
            self.tone.2 + (target.2 - self.tone.2) * k,
        );
        if (self.tone.0, self.tone.1) != (target.0, target.1)
            || (self.tone.2 - target.2).abs() > 0.002
        {
            ui.ctx().request_repaint();
        }
        let (c1, c2, themed) = self.tone;
        // Deep in the launcher's hue, not just black.
        let base = card::lerp(
            Color32::from_rgb(10, 11, 16),
            card::lerp(c2, Color32::BLACK, 0.88),
            themed,
        );
        p.rect_filled(full, 0.0, base);
        blob(
            &p,
            full.left_bottom() + vec2(full.width() * 0.15, 0.0),
            full.width() * (0.6 + 0.15 * themed),
            c1.gamma_multiply(0.22 + 0.12 * themed),
        );
        blob(
            &p,
            full.right_top() + vec2(-full.width() * 0.1, full.height() * 0.1),
            full.width() * (0.5 + 0.1 * themed),
            c2.gamma_multiply(0.16 + 0.12 * themed),
        );

        let show = self.cfg.bg_hero && (self.game.is_some() || self.page == crate::app::Page::Home);
        let target = if show { self.background_art() } else { None };
        if let Some(t) = &target
            && self.bgs.last().map(|(k, _)| k) != Some(t)
        {
            self.bgs.retain(|(k, _)| k != t);
            self.bgs.push((t.clone(), 0.0));
        }
        let k = self.ease(5.0);
        let n = self.bgs.len();
        let dim = Color32::from_gray(((1.0 - self.cfg.bg_dim) * 255.0) as u8);
        for i in 0..n {
            let (key, mut a) = self.bgs[i].clone();
            let ready = self.images.get(&key.0, key.1).is_some();
            if target.is_none() {
                a -= a * k;
            } else if i + 1 == n && ready {
                // The newest fades in over the older ones.
                a += (1.0 - a) * k;
            }
            self.bgs[i].1 = a;
            if let Some(t) = self.images.get(&key.0, key.1) {
                cover(&p, full, t, dim.gamma_multiply(a));
            }
        }
        // Once the newest covers them, the older ones go.
        if self.bgs.last().is_some_and(|(_, a)| *a > 0.995) {
            let last = self.bgs.pop();
            self.bgs.clear();
            self.bgs.extend(last);
        }
        if target.is_none() {
            self.bgs.retain(|(_, a)| *a > 0.01);
        }
        // Shade for the text: from the bottom and the left.
        widgets::vgradient(
            &p,
            Rect::from_min_max(
                pos2(full.left(), full.center().y - full.height() * 0.1),
                full.max,
            ),
            Color32::TRANSPARENT,
            Color32::from_black_alpha(220),
        );
        widgets::gradient_rect(
            &p,
            Rect::from_min_max(
                full.min,
                pos2(full.left() + full.width() * 0.45, full.bottom()),
            ),
            Color32::from_black_alpha(140),
            Color32::TRANSPARENT,
        );
        if self.cfg.bg_tint {
            blob(
                &p,
                full.left_bottom(),
                full.width() * 0.55,
                c1.gamma_multiply(0.18),
            );
            blob(
                &p,
                full.right_bottom(),
                full.width() * 0.35,
                c2.gamma_multiply(0.12),
            );
        }
    }

    fn background_art(&mut self) -> Option<(Art, Kind)> {
        let g = match &self.game {
            Some(v) => self.game_by_key(&v.key)?.clone(),
            None => self.selected_game()?.clone(),
        };
        // The hero picture sharp; anything smaller blurred.
        if let Some(h) = g
            .art
            .hero
            .as_ref()
            .filter(|h| !self.images.failed(h, Kind::Fit(1920)))
        {
            return Some((h.clone(), Kind::Fit(1920)));
        }
        let a = [g.art.landscape.as_ref(), g.art.portrait.as_ref()]
            .into_iter()
            .flatten()
            .find(|a| !self.images.failed(a, Kind::Blur))?;
        Some((a.clone(), Kind::Blur))
    }

    pub fn home_input(&mut self, a: Action) -> bool {
        let n = self.games.len();
        if n == 0 {
            return a == Action::Back;
        }
        let horizontal = self.cfg.orientation == Orientation::Horizontal;
        let dir: i32 = if self.cfg.reverse { -1 } else { 1 };
        let step = match (a, horizontal) {
            (Action::Right, true) => dir,
            (Action::Left, true) => -dir,
            (Action::Down, false) => dir,
            (Action::Up, false) => -dir,
            (Action::LeftTrigger, _) => -5,
            (Action::RightTrigger, _) => 5,
            _ => 0,
        };
        if step != 0 {
            let to = self.home.sel as i32 + step;
            if to < 0 || to >= n as i32 {
                return false;
            }
            self.home.sel = to as usize;
            return false;
        }
        match a {
            Action::Back => return true,
            Action::Home => self.home.sel = 0,
            Action::End => self.home.sel = n - 1,
            Action::Actions => {
                let k = self.games[self.home.sel].key.clone();
                self.open_menu(crate::ui::menu::MenuFor::Game(k), None);
            }
            Action::Accept => {
                let k = self.games[self.home.sel].key.clone();
                self.open_game(k);
            }
            Action::Select => {
                let k = self.games[self.home.sel].key.clone();
                self.play(&k);
            }
            _ => {}
        }
        false
    }

    pub fn draw_home(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter_at(rect);
        if self.games.is_empty() {
            let msg = if self.scanning {
                tr("scanning")
            } else {
                tr("no_games")
            };
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                msg,
                FontId::proportional(24.0),
                Color32::from_gray(200),
            );
            return;
        }
        // While dragged the ribbon sits where the pointer put it.
        if !self.home.dragging {
            let k = self.ease(9.0);
            self.home.pos += (self.home.sel as f32 - self.home.pos) * k;
            if (self.home.pos - self.home.sel as f32).abs() < 0.001 {
                self.home.pos = self.home.sel as f32;
            }
        }
        let horizontal = self.cfg.orientation == Orientation::Horizontal;
        let (band, info) = if horizontal {
            let band = Rect::from_min_max(
                pos2(rect.left(), rect.top() + rect.height() * 0.06),
                pos2(rect.right(), rect.top() + rect.height() * 0.74),
            );
            let info = Rect::from_min_max(
                pos2(rect.left() + 48.0, band.bottom() - rect.height() * 0.02),
                pos2(rect.right() - 48.0, rect.bottom()),
            );
            (band, info)
        } else {
            let band = Rect::from_min_max(
                rect.min,
                pos2(rect.left() + rect.width() * 0.46, rect.bottom()),
            );
            let info = Rect::from_min_max(
                pos2(band.right() + 24.0, rect.top() + rect.height() * 0.25),
                pos2(rect.right() - 48.0, rect.bottom() - 24.0),
            );
            (band, info)
        };
        let pulse = ((self.time * 2.2).sin() * 0.5 + 0.5) as f32;
        let look = Look {
            accent: self.accent,
            pulse,
        };
        let games: Vec<&Game> = self.games.iter().collect();
        let out = ribbon::draw(
            ui,
            band,
            &self.cfg,
            &games,
            self.home.sel,
            self.home.pos,
            &mut self.images,
            &look,
        );
        if let Some(i) = out.clicked {
            if i == self.home.sel {
                let k = self.games[i].key.clone();
                self.open_game(k);
            } else {
                self.home.sel = i;
            }
            self.side_focus = false;
        }
        let last = (self.games.len() - 1) as f32;
        if out.dragging {
            self.home.dragging = true;
            self.side_focus = false;
            self.home.pos = (self.home.pos + out.drag).clamp(-0.4, last + 0.4);
            self.home.sel = self.home.pos.round().clamp(0.0, last) as usize;
            let v = out.drag / self.dt.max(0.001);
            self.home.vel = self.home.vel * 0.7 + v * 0.3;
        }
        if out.released {
            // A flick carries on a little.
            let to = self.home.pos + (self.home.vel * 0.18).clamp(-6.0, 6.0);
            self.home.sel = to.round().clamp(0.0, last) as usize;
            self.home.dragging = false;
            self.home.vel = 0.0;
        }
        self.home.wheel += out.scroll;
        while self.home.wheel.abs() >= 50.0 {
            let s = self.home.wheel.signum();
            self.home.wheel -= s * 50.0;
            let to = (self.home.sel as i32 + s as i32).clamp(0, self.games.len() as i32 - 1);
            self.home.sel = to as usize;
        }
        if out.scroll == 0.0 {
            self.home.wheel *= 0.9;
        }
        self.draw_info(ui, info, horizontal);
    }

    fn draw_info(&mut self, ui: &mut Ui, rect: Rect, horizontal: bool) {
        let Some(g) = self.selected_game().cloned() else {
            return;
        };
        let p = ui.painter_at(rect.expand(8.0));
        let (c1, c2) = self.accent;
        let mut y = rect.top();
        // The logo if there is one, else the title.
        let logo = g
            .art
            .logo
            .as_ref()
            .and_then(|l| self.images.get(l, Kind::Fit(800)));
        let max_h = if horizontal {
            (rect.height() * 0.32).min(90.0)
        } else {
            120.0
        };
        if let Some(t) = logo {
            let s = (max_h / t.size.y).min(rect.width().min(460.0) / t.size.x);
            let r = Rect::from_min_size(pos2(rect.left(), y), t.size * s);
            p.image(
                t.handle.id(),
                r,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            y = r.bottom() + 12.0;
        } else {
            let galley = p.layout(
                g.title.clone(),
                FontId::proportional(if horizontal { 34.0 } else { 40.0 }),
                Color32::WHITE,
                rect.width(),
            );
            let h = galley.size().y;
            p.galley(pos2(rect.left(), y), galley, Color32::WHITE);
            y += h + 10.0;
        }
        // Source, play time, last played, achievements.
        let mut x = rect.left();
        let src = match &g.sub {
            Some(s) => format!("{} · {s}", g.source.name()),
            None => g.source.name().to_string(),
        };
        let chip = widgets::chip(&p, pos2(x, y), &src, widgets::source_colors(g.source), 15.0);
        x = chip.right() + 18.0;
        let line_c = chip.center().y;
        let meta = FontId::proportional(17.0);
        let text_c = Color32::from_gray(225);
        if let Some(m) = g.playtime_min {
            widgets::icon(&p, pos2(x + 9.0, line_c), 18.0, Icon::Clock, text_c);
            let r = p.text(
                pos2(x + 24.0, line_c),
                Align2::LEFT_CENTER,
                i18n::duration(m),
                meta.clone(),
                text_c,
            );
            x = r.right() + 22.0;
        }
        if let Some(t) = g.last_played {
            let r = p.text(
                pos2(x, line_c),
                Align2::LEFT_CENTER,
                format!("{}: {}", tr("last_played"), crate::game::date(t)),
                meta.clone(),
                Color32::from_gray(190),
            );
            x = r.right() + 22.0;
        }
        if let State::Ready(list) = self.store.achievements(&g) {
            let got = list.iter().filter(|a| a.unlocked.is_some()).count();
            let total = list.len();
            widgets::icon(&p, pos2(x + 9.0, line_c), 18.0, Icon::Trophy, text_c);
            let r = p.text(
                pos2(x + 24.0, line_c),
                Align2::LEFT_CENTER,
                format!("{got}/{total}"),
                meta.clone(),
                text_c,
            );
            let bar = Rect::from_min_size(pos2(r.right() + 10.0, line_c - 3.0), vec2(90.0, 6.0));
            widgets::bar(&p, bar, got as f32 / total.max(1) as f32, c1, c2);
        }
        y = chip.bottom() + 14.0;
        // A few lines from the store, once the choice settles.
        if let Some(appid) = g.info_appid() {
            let settled = (self.home.pos - self.home.sel as f32).abs() < 0.05;
            if settled && let State::Ready(d) = self.store.details(appid) {
                let room = rect.bottom() - y;
                if room > 40.0 && !d.short.is_empty() {
                    let galley = p.layout(
                        d.short.clone(),
                        FontId::proportional(16.0),
                        Color32::from_gray(205),
                        rect.width().min(760.0),
                    );
                    // At most what fits.
                    let rows = galley.rows.len();
                    let row_h = galley.size().y / rows.max(1) as f32;
                    let fit = ((room / row_h).floor() as usize).clamp(1, 3);
                    let text = if rows > fit {
                        let mut job = egui::text::LayoutJob::simple(
                            d.short.clone(),
                            FontId::proportional(16.0),
                            Color32::from_gray(205),
                            rect.width().min(760.0),
                        );
                        job.wrap.max_rows = fit;
                        p.fonts_mut(|f| f.layout_job(job))
                    } else {
                        galley
                    };
                    p.galley(pos2(rect.left(), y), text, Color32::WHITE);
                }
            }
        }
    }

    pub fn draw_status(&mut self, ui: &mut Ui, hints: Rect) {
        let p = ui.painter();
        let left = hints.left()
            + if self.game.is_none() {
                crate::ui::sidebar::COLLAPSED + 16.0
            } else {
                24.0
            };
        let mut s = clock();
        if self.scanning {
            s += &format!("   ·   {}", tr("scanning"));
        } else if self.game.is_none() {
            s += &if self.page == crate::app::Page::System {
                format!("   ·   {} {}", self.apps.len(), tr("apps_count"))
            } else {
                format!("   ·   {} {}", self.games.len(), tr("games_count"))
            };
        }
        p.text(
            pos2(left, hints.center().y),
            Align2::LEFT_CENTER,
            s,
            FontId::proportional(17.0),
            Color32::from_gray(200),
        );
    }

    pub fn draw_toast(&mut self, ui: &mut Ui, full: Rect) {
        let Some((text, t)) = &self.toast else { return };
        let age = t.elapsed().as_secs_f32();
        let a = (age / 0.2)
            .min(1.0)
            .min(((4.0 - age) / 0.4).clamp(0.0, 1.0));
        let p = ui.painter();
        let galley = p.layout_no_wrap(
            text.clone(),
            FontId::proportional(19.0),
            Color32::WHITE.gamma_multiply(a),
        );
        let r = Rect::from_center_size(
            pos2(full.center().x, full.top() + 48.0 - (1.0 - a) * 20.0),
            galley.size() + vec2(40.0, 22.0),
        );
        p.rect_filled(
            r,
            r.height() / 2.0,
            Color32::from_rgba_unmultiplied(20, 22, 30, (230.0 * a) as u8),
        );
        widgets::gradient_rect(
            p,
            Rect::from_min_size(
                r.left_bottom() + vec2(r.height() / 2.0, -3.0),
                vec2(r.width() - r.height(), 3.0),
            ),
            self.accent.0.gamma_multiply(a),
            self.accent.1.gamma_multiply(a),
        );
        p.galley(r.min + vec2(20.0, 11.0), galley, Color32::WHITE);
    }
}
