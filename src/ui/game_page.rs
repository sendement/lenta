//! A game's own page: its art, Play, and tabs — the store description and
//! details, achievements with when each was earned, screenshots.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::card::{self, Pose};
use super::widgets::{self, Icon};
use crate::app::{App, Tab};
use crate::game::{Game, date};
use crate::i18n::{self, tr};
use crate::images::Kind;
use crate::input::Action;
use crate::sources::steam::Achievement;
use crate::store::State;

const TABS: [Tab; 3] = [Tab::Overview, Tab::Achievements, Tab::Screenshots];
const SHOT_COLS: usize = 3;

impl App {
    fn page_game(&self) -> Option<Game> {
        self.game
            .as_ref()
            .and_then(|v| self.game_by_key(&v.key))
            .cloned()
    }

    fn shot_count(&mut self) -> usize {
        let Some(id) = self.page_game().and_then(|g| g.steam_appid) else {
            return 0;
        };
        match self.store.details(id) {
            State::Ready(d) => d.shots.len(),
            _ => 0,
        }
    }

    pub fn game_input(&mut self, a: Action) {
        let shots = self.shot_count();
        let Some(v) = self.game.as_mut() else { return };
        // A screenshot shown big.
        if let Some(i) = v.shot {
            match a {
                Action::Left | Action::LeftTrigger if i > 0 => v.shot = Some(i - 1),
                Action::Right | Action::RightTrigger if i + 1 < shots => v.shot = Some(i + 1),
                Action::Back | Action::Accept => {
                    v.shot_sel = i;
                    v.shot = None;
                }
                _ => {}
            }
            return;
        }
        let in_shots = v.focus == 4;
        match a {
            Action::Back => {
                if in_shots {
                    v.focus = 3;
                } else {
                    self.game = None;
                }
            }
            Action::Select => {
                let k = v.key.clone();
                self.play(&k);
            }
            Action::LeftTrigger | Action::RightTrigger => {
                let i = TABS.iter().position(|t| *t == v.tab).unwrap_or(0);
                let j = if a == Action::RightTrigger {
                    (i + 1) % 3
                } else {
                    (i + 2) % 3
                };
                v.tab = TABS[j];
                v.scroll = 0.0;
                if v.focus >= 1 {
                    v.focus = j + 1;
                }
            }
            Action::Left if in_shots => v.shot_sel = v.shot_sel.saturating_sub(1),
            Action::Right if in_shots => v.shot_sel = (v.shot_sel + 1).min(shots.saturating_sub(1)),
            Action::Up if in_shots => {
                if v.shot_sel < SHOT_COLS {
                    v.focus = 3;
                } else {
                    v.shot_sel -= SHOT_COLS;
                }
            }
            Action::Down if in_shots => {
                if v.shot_sel + SHOT_COLS < shots {
                    v.shot_sel += SHOT_COLS;
                }
            }
            Action::Accept if in_shots => v.shot = Some(v.shot_sel),
            Action::Left => v.focus = v.focus.saturating_sub(1),
            Action::Right => v.focus = (v.focus + 1).min(3),
            Action::Up => v.scroll = (v.scroll - 160.0).max(0.0),
            Action::Down => {
                if v.tab == Tab::Screenshots && shots > 0 {
                    v.focus = 4;
                } else {
                    v.scroll = (v.scroll + 160.0).min(v.max_scroll);
                }
            }
            Action::Accept => {
                if v.focus == 0 {
                    let k = v.key.clone();
                    self.play(&k);
                } else if v.tab == Tab::Screenshots && shots > 0 {
                    v.focus = 4;
                }
            }
            _ => {}
        }
        if let Some(v) = self.game.as_mut()
            && (1..=3).contains(&v.focus)
            && v.tab != TABS[v.focus - 1]
        {
            v.tab = TABS[v.focus - 1];
            v.scroll = 0.0;
        }
    }

    pub fn draw_game_page(&mut self, ui: &mut Ui, rect: Rect) {
        let Some(g) = self.page_game() else {
            self.game = None;
            return;
        };
        let (c1, c2) = self.accent;
        let p = ui.painter_at(rect);
        let pad = 56.0;
        let age = self
            .game
            .as_ref()
            .map(|v| v.opened.elapsed().as_secs_f32())
            .unwrap_or(1.0);
        let rise = (1.0 - (age / 0.35).min(1.0)).powi(2) * 40.0;
        if age < 0.4 {
            ui.ctx().request_repaint();
        }
        let head_bottom = rect.top() + rect.height() * 0.40 + rise;
        // Cover on the left of the header.
        let mut x = rect.left() + pad;
        let cover_h = (rect.height() * 0.30).min(300.0);
        let cover_r = Rect::from_min_size(
            pos2(x, head_bottom - cover_h),
            vec2(cover_h * 2.0 / 3.0, cover_h),
        );
        let pose = Pose::flat(cover_r, self.cfg.rounding * 0.8);
        let (fill, has_art) = super::ribbon::fill_for(
            &mut self.images,
            &g,
            crate::config::Shape::Portrait,
            pose.size,
        );
        card::shadow(&p, &pose, 1.0);
        card::draw(&p, &pose, &fill, Color32::WHITE, None);
        if !has_art {
            super::ribbon::title_on(&p, &mut self.images, &g, cover_r, 1.0);
        }
        x = cover_r.right() + 32.0;
        // Logo or title, then the facts.
        let logo = g
            .art
            .logo
            .as_ref()
            .and_then(|l| self.images.get(l, Kind::Fit(800)));
        let mut y = cover_r.top();
        if let Some(t) = logo {
            let s = (110.0 / t.size.y).min(520.0 / t.size.x);
            let r = Rect::from_min_size(pos2(x, y), t.size * s);
            p.image(
                t.handle.id(),
                r,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            y = r.bottom() + 14.0;
        } else {
            let galley = p.layout(
                g.title.clone(),
                FontId::proportional(42.0),
                Color32::WHITE,
                rect.right() - x - pad,
            );
            let h = galley.size().y;
            p.galley(pos2(x, y), galley, Color32::WHITE);
            y += h + 12.0;
        }
        let meta = FontId::proportional(18.0);
        let mut mx = x;
        let src = match &g.sub {
            Some(s) => format!("{} · {s}", g.source.name()),
            None => g.source.name().to_string(),
        };
        let chip = widgets::chip(
            ui,
            &p,
            pos2(mx, y),
            &src,
            card::lerp(c1, Color32::BLACK, 0.45),
            15.0,
        );
        mx = chip.right() + 18.0;
        let cy = chip.center().y;
        let txt = Color32::from_gray(225);
        widgets::icon(&p, pos2(mx + 9.0, cy), 18.0, Icon::Clock, txt);
        let r = p.text(
            pos2(mx + 24.0, cy),
            Align2::LEFT_CENTER,
            g.playtime_min
                .map(i18n::duration)
                .unwrap_or_else(|| "—".into()),
            meta.clone(),
            txt,
        );
        mx = r.right() + 22.0;
        let last = g
            .last_played
            .map(date)
            .unwrap_or_else(|| tr("never").to_string());
        p.text(
            pos2(mx, cy),
            Align2::LEFT_CENTER,
            format!("{}: {last}", tr("last_played")),
            meta.clone(),
            Color32::from_gray(190),
        );

        // Buttons: Play and the tabs.
        let ach = g.steam_appid.map(|id| match self.store.achievements(id) {
            State::Ready(l) => Some((l.iter().filter(|a| a.unlocked.is_some()).count(), l.len())),
            _ => None,
        });
        let row_y = head_bottom + 28.0;
        let focus = self.game.as_ref().map(|v| v.focus).unwrap_or(0);
        let tab = self.game.as_ref().map(|v| v.tab).unwrap_or(Tab::Overview);
        let play_r = Rect::from_min_size(pos2(rect.left() + pad, row_y), vec2(210.0, 58.0));
        let play_resp = ui.interact(play_r, ui.id().with("play"), Sense::click());
        card::draw(
            &p,
            &Pose::flat(play_r, 14.0),
            &card::Fill::Gradient(c1, c2),
            Color32::WHITE,
            None,
        );
        p.rect_stroke(
            play_r,
            14.0,
            Stroke::new(if focus == 0 { 3.0 } else { 0.0 }, Color32::WHITE),
            StrokeKind::Inside,
        );
        widgets::icon(
            &p,
            pos2(play_r.left() + 40.0, play_r.center().y),
            24.0,
            Icon::Play,
            Color32::WHITE,
        );
        p.text(
            pos2(play_r.left() + 64.0, play_r.center().y),
            Align2::LEFT_CENTER,
            tr("play"),
            FontId::proportional(24.0),
            Color32::WHITE,
        );
        if play_resp.clicked() {
            let k = g.key.clone();
            self.play(&k);
        }
        let mut tx = play_r.right() + 28.0;
        for (i, t) in TABS.iter().enumerate() {
            let label = match t {
                Tab::Overview => tr("overview").to_string(),
                Tab::Achievements => match ach.flatten() {
                    Some((a, b)) => format!("{} {a}/{b}", tr("achievements")),
                    None => tr("achievements").to_string(),
                },
                Tab::Screenshots => tr("screenshots").to_string(),
            };
            let galley = p.layout_no_wrap(label, FontId::proportional(19.0), Color32::WHITE);
            let r = Rect::from_min_size(pos2(tx, row_y + 6.0), galley.size() + vec2(36.0, 22.0));
            let resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click());
            let on = tab == *t;
            p.rect_filled(
                r,
                r.height() / 2.0,
                if on {
                    Color32::from_white_alpha(40)
                } else if resp.hovered() {
                    Color32::from_white_alpha(22)
                } else {
                    Color32::from_white_alpha(10)
                },
            );
            if focus == i + 1 {
                p.rect_stroke(
                    r,
                    r.height() / 2.0,
                    Stroke::new(2.5, c1),
                    StrokeKind::Inside,
                );
            }
            p.galley(
                r.min + vec2(18.0, 11.0),
                galley,
                Color32::from_gray(if on { 255 } else { 200 }),
            );
            if resp.clicked()
                && let Some(v) = self.game.as_mut()
            {
                v.tab = *t;
                v.focus = i + 1;
                v.scroll = 0.0;
            }
            tx = r.right() + 12.0;
        }

        // The tab's content, scrolled.
        let body = Rect::from_min_max(
            pos2(rect.left() + pad, row_y + 84.0),
            pos2(rect.right() - pad, rect.bottom()),
        );
        if ui.rect_contains_pointer(body) {
            let d = ui.input(|i| i.smooth_scroll_delta.y);
            if let Some(v) = self.game.as_mut() {
                v.scroll = (v.scroll - d).clamp(0.0, v.max_scroll);
            }
        }
        let scroll = self.game.as_ref().map(|v| v.scroll).unwrap_or(0.0);
        let shown = self.game.as_ref().map(|v| v.shown).unwrap_or(0.0);
        let k = self.ease(14.0);
        let eased = shown + (scroll - shown) * k;
        if let Some(v) = self.game.as_mut() {
            v.shown = eased;
        }
        if (scroll - eased).abs() > 0.5 {
            ui.ctx().request_repaint();
        }
        let content_h = match tab {
            Tab::Overview => self.draw_overview(ui, body, &g, eased),
            Tab::Achievements => self.draw_achievements(ui, body, &g, eased),
            Tab::Screenshots => self.draw_shots(ui, body, &g, eased),
        };
        if let Some(v) = self.game.as_mut() {
            v.max_scroll = (content_h - body.height() + 24.0).max(0.0);
            v.scroll = v.scroll.min(v.max_scroll);
        }
        self.draw_big_shot(ui, rect.expand2(vec2(0.0, 60.0)), &g);
    }

    fn draw_overview(&mut self, ui: &mut Ui, body: Rect, g: &Game, scroll: f32) -> f32 {
        let p = ui.painter_at(body);
        let right_w = (body.width() * 0.32).clamp(260.0, 420.0);
        let left_w = body.width() - right_w - 48.0;
        let mut y = body.top() - scroll;
        let mut facts: Vec<(&str, String)> = vec![(
            tr("source"),
            g.sub
                .clone()
                .map(|s| format!("{} · {s}", g.source.name()))
                .unwrap_or_else(|| g.source.name().into()),
        )];
        if let Some(m) = g.playtime_min {
            facts.push((tr("playtime"), i18n::duration(m)));
        }
        match g.steam_appid.map(|id| self.store.details(id)) {
            Some(State::Ready(d)) => {
                let d = d.clone();
                if !d.short.is_empty() {
                    let galley = p.layout(
                        d.short.clone(),
                        FontId::proportional(20.0),
                        Color32::WHITE,
                        left_w,
                    );
                    let h = galley.size().y;
                    p.galley(pos2(body.left(), y), galley, Color32::WHITE);
                    y += h + 20.0;
                }
                let about = if d.about.is_empty() {
                    tr("no_desc").to_string()
                } else {
                    d.about.clone()
                };
                let galley = p.layout(
                    about,
                    FontId::proportional(17.0),
                    Color32::from_gray(205),
                    left_w,
                );
                let h = galley.size().y;
                p.galley(pos2(body.left(), y), galley, Color32::WHITE);
                y += h;
                if !d.developers.is_empty() {
                    facts.push((tr("developer"), d.developers.join(", ")));
                }
                if !d.publishers.is_empty() {
                    facts.push((tr("publisher"), d.publishers.join(", ")));
                }
                if !d.genres.is_empty() {
                    facts.push((tr("genres"), d.genres.join(", ")));
                }
                if let Some(r) = &d.released {
                    facts.push((tr("released"), r.clone()));
                }
                if let Some(m) = d.metacritic {
                    facts.push((tr("metacritic"), m.to_string()));
                }
            }
            Some(State::Loading) => {
                p.text(
                    pos2(body.left(), y),
                    Align2::LEFT_TOP,
                    tr("loading"),
                    FontId::proportional(18.0),
                    Color32::from_gray(180),
                );
                y += 30.0;
            }
            Some(State::Failed) => {
                p.text(
                    pos2(body.left(), y),
                    Align2::LEFT_TOP,
                    tr("offline"),
                    FontId::proportional(18.0),
                    Color32::from_gray(180),
                );
                y += 30.0;
            }
            None => {
                p.text(
                    pos2(body.left(), y),
                    Align2::LEFT_TOP,
                    tr("no_desc"),
                    FontId::proportional(18.0),
                    Color32::from_gray(180),
                );
                y += 30.0;
            }
        }
        if let Some(dir) = &g.install_dir {
            facts.push((tr("folder"), dir.display().to_string()));
        }
        // The facts in a card on the right.
        let fx = body.right() - right_w;
        let mut fy = body.top() - scroll;
        let start = fy;
        let mut rows = Vec::new();
        for (k, v) in &facts {
            let kg = p.layout_no_wrap(
                k.to_string(),
                FontId::proportional(14.0),
                Color32::from_gray(150),
            );
            let vg = p.layout(
                v.clone(),
                FontId::proportional(17.0),
                Color32::WHITE,
                right_w - 40.0,
            );
            let h = kg.size().y + vg.size().y + 18.0;
            rows.push((fy, kg, vg));
            fy += h;
        }
        p.rect_filled(
            Rect::from_min_max(pos2(fx, start), pos2(body.right(), fy + 10.0)),
            16.0,
            Color32::from_black_alpha(110),
        );
        for (ry, kg, vg) in rows {
            let kh = kg.size().y;
            p.galley(pos2(fx + 20.0, ry + 14.0), kg, Color32::WHITE);
            p.galley(pos2(fx + 20.0, ry + 16.0 + kh), vg, Color32::WHITE);
        }
        (y.max(fy + 10.0)) - (body.top() - scroll)
    }

    fn draw_achievements(&mut self, ui: &mut Ui, body: Rect, g: &Game, scroll: f32) -> f32 {
        let p = ui.painter_at(body);
        let (c1, c2) = self.accent;
        let list: Vec<Achievement> = match g.steam_appid.map(|id| self.store.achievements(id)) {
            Some(State::Ready(l)) => l.clone(),
            Some(State::Loading) => {
                p.text(
                    body.left_top(),
                    Align2::LEFT_TOP,
                    tr("loading"),
                    FontId::proportional(18.0),
                    Color32::from_gray(180),
                );
                return 30.0;
            }
            _ => {
                p.text(
                    body.left_top(),
                    Align2::LEFT_TOP,
                    tr("no_ach"),
                    FontId::proportional(18.0),
                    Color32::from_gray(180),
                );
                return 30.0;
            }
        };
        let got = list.iter().filter(|a| a.unlocked.is_some()).count();
        let mut y = body.top() - scroll;
        let pct = got as f32 / list.len().max(1) as f32;
        p.text(
            pos2(body.left(), y),
            Align2::LEFT_TOP,
            format!(
                "{}: {got} / {}  ({:.0}%)",
                tr("unlocked"),
                list.len(),
                pct * 100.0
            ),
            FontId::proportional(20.0),
            Color32::WHITE,
        );
        widgets::bar(
            &p,
            Rect::from_min_size(
                pos2(body.left(), y + 34.0),
                vec2(body.width().min(700.0), 8.0),
            ),
            pct,
            c1,
            c2,
        );
        y += 64.0;
        let mut sorted = list;
        sorted.sort_by(|a, b| match (a.unlocked, b.unlocked) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.hidden.cmp(&b.hidden),
        });
        let row_h = 84.0;
        for a in &sorted {
            let r = Rect::from_min_size(pos2(body.left(), y), vec2(body.width(), row_h - 10.0));
            y += row_h;
            if r.bottom() < body.top() || r.top() > body.bottom() {
                continue;
            }
            let got = a.unlocked.is_some();
            p.rect_filled(
                r,
                14.0,
                Color32::from_black_alpha(if got { 120 } else { 80 }),
            );
            let ir = Rect::from_min_size(r.left_top() + vec2(14.0, 9.0), vec2(56.0, 56.0));
            let icon = if got {
                a.icon.as_ref()
            } else {
                a.icon_gray.as_ref().or(a.icon.as_ref())
            };
            match icon.and_then(|i| self.images.get(i, Kind::Fit(128))) {
                Some(t) => {
                    p.image(
                        t.handle.id(),
                        ir,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    p.rect_filled(ir, 8.0, Color32::from_white_alpha(20));
                    widgets::icon(&p, ir.center(), 26.0, Icon::Trophy, Color32::from_gray(150));
                }
            }
            let secret = a.hidden && !got;
            let name = if secret {
                tr("hidden_ach").to_string()
            } else {
                a.name.clone()
            };
            let tx = ir.right() + 16.0;
            let date_w = 160.0;
            let name_g = p.layout_no_wrap(
                name,
                FontId::proportional(18.0),
                Color32::from_gray(if got { 255 } else { 170 }),
            );
            p.galley(pos2(tx, r.top() + 12.0), name_g, Color32::WHITE);
            if !secret && !a.desc.is_empty() {
                let mut job = egui::text::LayoutJob::simple(
                    a.desc.clone(),
                    FontId::proportional(15.0),
                    Color32::from_gray(if got { 200 } else { 140 }),
                    r.right() - tx - date_w,
                );
                job.wrap.max_rows = 2;
                let dg = p.fonts_mut(|f| f.layout_job(job));
                p.galley(pos2(tx, r.top() + 38.0), dg, Color32::WHITE);
            }
            if let Some(t) = a.unlocked.filter(|t| *t > 0) {
                p.text(
                    pos2(r.right() - 18.0, r.center().y),
                    Align2::RIGHT_CENTER,
                    date(t),
                    FontId::proportional(15.0),
                    Color32::from_gray(190),
                );
            }
        }
        y - (body.top() - scroll)
    }

    fn draw_shots(&mut self, ui: &mut Ui, body: Rect, g: &Game, scroll: f32) -> f32 {
        let p = ui.painter_at(body);
        let shots = match g.steam_appid.map(|id| self.store.details(id)) {
            Some(State::Ready(d)) => d.shots.clone(),
            Some(State::Loading) => {
                p.text(
                    body.left_top(),
                    Align2::LEFT_TOP,
                    tr("loading"),
                    FontId::proportional(18.0),
                    Color32::from_gray(180),
                );
                return 30.0;
            }
            _ => Vec::new(),
        };
        if shots.is_empty() {
            p.text(
                body.left_top(),
                Align2::LEFT_TOP,
                tr("no_shots"),
                FontId::proportional(18.0),
                Color32::from_gray(180),
            );
            return 30.0;
        }
        let (focus, sel) = self
            .game
            .as_ref()
            .map(|v| (v.focus, v.shot_sel))
            .unwrap_or((0, 0));
        let gap = 18.0;
        let w = (body.width() - gap * (SHOT_COLS - 1) as f32) / SHOT_COLS as f32;
        let h = w * 9.0 / 16.0;
        // Keep the chosen one in view.
        if focus == 4 {
            let row_top = (sel / SHOT_COLS) as f32 * (h + gap);
            if let Some(v) = self.game.as_mut() {
                if row_top < v.scroll {
                    v.scroll = row_top;
                } else if row_top + h > v.scroll + body.height() - 20.0 {
                    v.scroll = row_top + h - body.height() + 20.0;
                }
            }
        }
        let mut open = None;
        for (i, (thumb, _)) in shots.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(
                    body.left() + (i % SHOT_COLS) as f32 * (w + gap),
                    body.top() - scroll + (i / SHOT_COLS) as f32 * (h + gap),
                ),
                vec2(w, h),
            );
            if r.bottom() < body.top() || r.top() > body.bottom() {
                continue;
            }
            let resp = ui.interact(r.intersect(body), ui.id().with(("shot", i)), Sense::click());
            match self.images.get(thumb, Kind::Fit(640)) {
                Some(t) => super::home::cover(&p, r, t, Color32::WHITE),
                None => {
                    p.rect_filled(r, 10.0, Color32::from_white_alpha(14));
                }
            }
            if focus == 4 && sel == i {
                p.rect_stroke(r, 6.0, Stroke::new(3.0, self.accent.0), StrokeKind::Outside);
            }
            if resp.clicked() {
                open = Some(i);
            }
        }
        if let (Some(i), Some(v)) = (open, self.game.as_mut()) {
            v.shot = Some(i);
            v.shot_sel = i;
            v.focus = 4;
        }
        let rows = shots.len().div_ceil(SHOT_COLS);
        rows as f32 * (h + gap)
    }

    fn draw_big_shot(&mut self, ui: &mut Ui, area: Rect, g: &Game) {
        let Some(i) = self.game.as_ref().and_then(|v| v.shot) else {
            return;
        };
        let Some(id) = g.steam_appid else { return };
        let full = match self.store.details(id) {
            State::Ready(d) => d.shots.get(i).map(|s| s.1.clone()),
            _ => None,
        };
        let p = ui.painter_at(area);
        p.rect_filled(area, 0.0, Color32::from_black_alpha(235));
        let resp = ui.interact(area, ui.id().with("bigshot"), Sense::click());
        if let Some(full) = full {
            match self.images.get(&full, Kind::Fit(1920)) {
                Some(t) => {
                    let s = (area.width() * 0.92 / t.size.x).min(area.height() * 0.86 / t.size.y);
                    let r = Rect::from_center_size(area.center(), t.size * s);
                    p.image(
                        t.handle.id(),
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    p.text(
                        area.center(),
                        Align2::CENTER_CENTER,
                        tr("loading"),
                        FontId::proportional(20.0),
                        Color32::from_gray(200),
                    );
                }
            }
        }
        if resp.clicked()
            && let Some(v) = self.game.as_mut()
        {
            v.shot = None;
        }
    }
}
