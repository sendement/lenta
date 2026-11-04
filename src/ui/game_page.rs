//! A game's own page: its art, Play, and tabs — the store description and
//! details, achievements with when each was earned, screenshots.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::card::{self, Pose};
use super::widgets::{self, Btn, Icon};
use crate::app::{App, Tab};
use crate::game::{Game, date};
use crate::i18n::{self, tr};
use crate::images::Kind;
use crate::input::Action;
use crate::sources::steam::Achievement;
use crate::store::State;

/// Focus inside the tab's content (the screenshots, the tool list).
pub const INSIDE: usize = 9;
const SHOT_COLS: usize = 3;
/// Clear shader caches, reset the prefix: under the tools.
const MAINT_ROWS: usize = 2;

impl App {
    fn page_game(&self) -> Option<Game> {
        self.game
            .as_ref()
            .and_then(|v| self.game_by_key(&v.key))
            .cloned()
    }

    fn shot_count(&mut self) -> usize {
        let Some(id) = self.page_game().and_then(|g| g.info_appid()) else {
            return 0;
        };
        match self.store.details(id) {
            State::Ready(d) => d.shots.len(),
            _ => 0,
        }
    }

    /// The tabs this game has: compatibility only for Steam's.
    pub fn tabs_of(&self, g: &Game) -> Vec<Tab> {
        let mut v = vec![Tab::Overview, Tab::Achievements, Tab::Screenshots];
        if g.compat_id.is_some() && !self.steam.tools.is_empty() {
            v.push(Tab::Compat);
        }
        if self.can_downgrade(g) {
            v.push(Tab::Versions);
        }
        if self.sgdb.has_key == Some(true) {
            v.push(Tab::Art);
        }
        v.extend(self.mod_tabs(g));
        v
    }

    /// Rows in the compatibility tab: the tools, then "Restart Steam"
    /// while changes wait.
    fn tool_rows(&self) -> usize {
        self.tool_choices().len() + usize::from(!self.steam.pending.is_empty()) + MAINT_ROWS
    }

    /// Ask, then clear the game's shader caches.
    pub fn clear_shaders(&mut self, g: &Game) {
        let items = crate::maint::shader_caches(g);
        let total: u64 = items.iter().map(|i| i.size).sum();
        if items.is_empty() || total == 0 {
            self.toast(tr("maint_no_cache"));
            return;
        }
        let lines = vec![
            format!(
                "{} ({} {})",
                crate::maint::human(total),
                items.len(),
                tr("maint_places")
            ),
            tr("maint_shaders_note").to_string(),
        ];
        self.ask(
            format!("{} — {}?", tr("maint_shaders"), g.title),
            lines,
            tr("maint_clear"),
            move |app| {
                let freed = crate::maint::clear(&items);
                app.toast(format!(
                    "{}: {}",
                    tr("maint_freed"),
                    crate::maint::human(freed)
                ));
            },
        );
    }

    /// Ask, then move the game's prefix to the trash.
    pub fn reset_prefix(&mut self, g: &Game) {
        let Some(pfx) = crate::maint::prefix(g) else {
            self.toast(tr("maint_no_prefix"));
            return;
        };
        let lines = vec![
            format!(
                "{} ({})",
                pfx.display(),
                crate::maint::human(crate::maint::size(&pfx))
            ),
            tr("maint_prefix_trash").to_string(),
            tr("maint_prefix_new").to_string(),
            tr("maint_prefix_saves").to_string(),
        ];
        self.ask(
            format!("{} — {}?", tr("maint_prefix"), g.title),
            lines,
            tr("maint_reset"),
            move |app| match crate::maint::trash(&pfx) {
                Ok(_) => app.toast(tr("maint_prefix_done")),
                Err(e) => app.toast(format!("{}: {e}", tr("sd_failed"))),
            },
        );
    }

    pub fn game_input(&mut self, a: Action) {
        if let Some(v) = self.game.as_mut() {
            v.follow = true;
        }
        // A question about a version is up: A yes, B no.
        if self.versions.ask.is_some() {
            match a {
                Action::Accept => self.versions_answer(true),
                Action::Back => self.versions_answer(false),
                _ => {}
            }
            return;
        }
        let shots = self.shot_count();
        let Some(g) = self.page_game() else { return };
        let tabs = self.tabs_of(&g);
        let rows = self.tool_rows();
        let choices = self.tool_choices();
        // The row of the tool in use, to start the list on.
        let cur_tool = g
            .compat_id
            .map(|id| self.steam.mapping.get(&id).cloned())
            .and_then(|cur| choices.iter().position(|c| c.0 == cur))
            .unwrap_or(0);
        let ver_rows = self.versions.row_count();
        let art_rows = self.art_rows();
        let ach_rows = match self.store.achievements(&g) {
            State::Ready(l) => l.len(),
            _ => 0,
        };
        let ver_start = self.versions.start_row();
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
        let tab_i = tabs.iter().position(|t| *t == v.tab).unwrap_or(0);
        let inside = v.focus == INSIDE;
        // Achievements: chosen one by one; A shows a hidden one.
        if inside && v.tab == Tab::Achievements {
            match a {
                Action::Up if v.tool_sel == 0 => v.focus = tab_i + 1,
                Action::Up => v.tool_sel -= 1,
                Action::Down => v.tool_sel = (v.tool_sel + 1).min(ach_rows.saturating_sub(1)),
                Action::Back => v.focus = tab_i + 1,
                Action::Accept => v.reveal = true,
                _ => {}
            }
            return;
        }
        // The pictures: a row per kind, ← → through it, A takes.
        if inside && v.tab == Tab::Art {
            let row = v.tool_sel.min(art_rows.saturating_sub(1));
            match a {
                Action::Back => v.focus = tab_i + 1,
                Action::Accept => self.art_take(row),
                Action::Up | Action::Down | Action::Left | Action::Right => {
                    // Through the grid; past the top: back to the tabs.
                    match self.art_nav(row, a) {
                        Some(r) => {
                            if let Some(v) = self.game.as_mut() {
                                v.tool_sel = r;
                            }
                        }
                        None => {
                            if let Some(v) = self.game.as_mut() {
                                v.focus = tab_i + 1;
                            }
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        // A module's tab: its own page.
        if inside && let Tab::Module(_) = v.tab {
            let at_top = self.mods.game.as_ref().is_none_or(|m| m.at_top());
            if ((a == Action::Up && at_top)
                || self.modview_input(crate::ui::modules::Which::Game, a))
                && let Some(v) = self.game.as_mut()
            {
                v.focus = tab_i + 1;
            }
            return;
        }
        // The versions list.
        if inside && v.tab == Tab::Versions {
            match a {
                Action::Up if v.tool_sel == 0 => v.focus = tab_i + 1,
                Action::Up => v.tool_sel -= 1,
                Action::Down => v.tool_sel = (v.tool_sel + 1).min(ver_rows.saturating_sub(1)),
                Action::Back => v.focus = tab_i + 1,
                Action::Accept => {
                    let row = v.tool_sel;
                    self.versions_accept(row);
                }
                // X: install the ticked version (asks first).
                Action::Select => self.versions_install(),
                _ => {}
            }
            return;
        }
        // The compatibility list.
        if inside && v.tab == Tab::Compat {
            match a {
                Action::Up if v.tool_sel == 0 => v.focus = tab_i + 1,
                Action::Up => v.tool_sel -= 1,
                Action::Down => v.tool_sel = (v.tool_sel + 1).min(rows.saturating_sub(1)),
                Action::Back => v.focus = tab_i + 1,
                Action::Accept => {
                    let sel = v.tool_sel;
                    let restart_row = usize::from(!self.steam.pending.is_empty());
                    if sel < choices.len() {
                        if let Some(id) = g.compat_id {
                            self.choose_tool(id, choices[sel].0.clone());
                        }
                    } else if sel < choices.len() + restart_row {
                        self.restart_steam();
                    } else if sel == choices.len() + restart_row {
                        self.clear_shaders(&g);
                    } else {
                        self.reset_prefix(&g);
                    }
                }
                _ => {}
            }
            return;
        }
        match a {
            Action::Back => {
                if inside {
                    v.focus = tab_i + 1;
                } else {
                    self.game = None;
                }
            }
            Action::Select => {
                let k = v.key.clone();
                self.play(&k);
            }
            // Y: another theme song.
            Action::Actions if self.cfg.theme_music => self.music.next(),
            Action::LeftTrigger | Action::RightTrigger => {
                let n = tabs.len();
                let j = if a == Action::RightTrigger {
                    (tab_i + 1) % n
                } else {
                    (tab_i + n - 1) % n
                };
                v.tab = tabs[j];
                v.scroll = 0.0;
                if v.focus >= 1 {
                    v.focus = j + 1;
                }
            }
            Action::Left if inside => v.shot_sel = v.shot_sel.saturating_sub(1),
            Action::Right if inside => v.shot_sel = (v.shot_sel + 1).min(shots.saturating_sub(1)),
            Action::Up if inside => {
                if v.shot_sel < SHOT_COLS {
                    v.focus = tab_i + 1;
                } else {
                    v.shot_sel -= SHOT_COLS;
                }
            }
            Action::Down if inside => {
                if v.shot_sel + SHOT_COLS < shots {
                    v.shot_sel += SHOT_COLS;
                }
            }
            Action::Accept if inside => v.shot = Some(v.shot_sel),
            Action::Left => v.focus = v.focus.saturating_sub(1),
            Action::Right => v.focus = (v.focus + 1).min(tabs.len()),
            Action::Up => {
                if v.scroll <= 0.5 {
                    v.folded = false;
                }
                v.scroll = (v.scroll - 160.0).max(0.0);
            }
            Action::Down => match v.tab {
                Tab::Screenshots if shots > 0 => v.focus = INSIDE,
                Tab::Compat => {
                    v.focus = INSIDE;
                    v.tool_sel = cur_tool;
                }
                Tab::Versions if ver_rows > 0 => {
                    v.focus = INSIDE;
                    v.tool_sel = ver_start;
                }
                Tab::Art if art_rows > 0 => {
                    v.focus = INSIDE;
                    v.tool_sel = 0;
                }
                Tab::Achievements if ach_rows > 0 => {
                    v.focus = INSIDE;
                    v.tool_sel = 0;
                }
                Tab::Module(_) => v.focus = INSIDE,
                _ => {
                    v.folded |= v.overflow;
                    v.scroll = (v.scroll + 160.0).min(v.max_scroll);
                }
            },
            Action::Accept => {
                if v.focus == 0 {
                    let k = v.key.clone();
                    self.play(&k);
                } else if v.tab == Tab::Screenshots && shots > 0 {
                    v.focus = INSIDE;
                } else if v.tab == Tab::Compat {
                    v.focus = INSIDE;
                    v.tool_sel = cur_tool;
                } else if v.tab == Tab::Versions && ver_rows > 0 {
                    v.focus = INSIDE;
                    v.tool_sel = ver_start;
                } else if matches!(v.tab, Tab::Module(_)) {
                    v.focus = INSIDE;
                } else if (v.tab == Tab::Achievements && ach_rows > 0)
                    || (v.tab == Tab::Art && art_rows > 0)
                {
                    v.focus = INSIDE;
                    v.tool_sel = 0;
                }
            }
            _ => {}
        }
        if let Some(v) = self.game.as_mut()
            && (1..=tabs.len()).contains(&v.focus)
            && v.tab != tabs[v.focus - 1]
        {
            v.tab = tabs[v.focus - 1];
            v.scroll = 0.0;
            v.folded = false;
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
        // Inside a tab (or scrolled) the header slides away and Play with
        // the tabs rises to the top, giving the content the page.
        let (inside, scrolled) = self
            .game
            .as_ref()
            .map(|v| (v.focus == INSIDE, v.folded))
            .unwrap_or((false, false));
        let collapse = ui.ctx().animate_value_with_time(
            ui.id().with("game_collapse"),
            if inside || scrolled { 1.0 } else { 0.0 },
            0.28,
        );
        let full_head = rect.height() * 0.40 + rise;
        let lift = collapse * (full_head + 28.0 - 16.0);
        let head_bottom = rect.top() + full_head - lift;
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
            &p,
            pos2(mx, y),
            &src,
            widgets::source_colors(g.source),
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
        // The theme playing, and "another track".
        let now = self.music.now.lock().ok().and_then(|n| n.clone());
        if self.cfg.theme_music
            && let Some((key, song)) = now.filter(|(k, _)| *k == g.key)
        {
            let _ = key;
            let ny = chip.bottom() + 22.0;
            let nc = pos2(x + 8.0, ny);
            // A little note.
            p.circle_filled(nc + vec2(-2.0, 5.0), 4.0, Color32::from_gray(210));
            p.line_segment(
                [nc + vec2(1.6, 5.0), nc + vec2(1.6, -7.0)],
                Stroke::new(1.8, Color32::from_gray(210)),
            );
            p.line_segment(
                [nc + vec2(1.6, -7.0), nc + vec2(7.0, -4.0)],
                Stroke::new(1.8, Color32::from_gray(210)),
            );
            let text = if song.is_empty() {
                tr("music_looking").to_string()
            } else {
                song
            };
            let mut job = egui::text::LayoutJob::simple_singleline(
                text,
                FontId::proportional(15.0),
                Color32::from_gray(200),
            );
            job.wrap.max_width = 520.0;
            job.wrap.max_rows = 1;
            let tg = p.fonts_mut(|f| f.layout_job(job));
            let tw = tg.size().x;
            p.galley(pos2(x + 22.0, ny - tg.size().y / 2.0), tg, Color32::WHITE);
            let label = p.layout_no_wrap(
                tr("music_next").to_string(),
                FontId::proportional(14.0),
                Color32::WHITE,
            );
            let br = Rect::from_min_size(
                pos2(x + 22.0 + tw + 14.0, ny - 13.0),
                vec2(label.size().x + 50.0, 26.0),
            );
            let resp = ui.interact(br, ui.id().with("music_next"), Sense::click());
            p.rect_filled(
                br,
                13.0,
                Color32::from_white_alpha(if resp.hovered() { 40 } else { 22 }),
            );
            widgets::glyph(
                &p,
                br.left_top() + vec2(6.0, 4.0),
                18.0,
                Btn::Y,
                self.glyphs,
            );
            p.galley(
                br.left_top() + vec2(30.0, 13.0 - label.size().y / 2.0),
                label,
                Color32::WHITE,
            );
            if resp.clicked() {
                self.music.next();
            }
        }

        // Buttons: Play and the tabs.
        let ach = match self.store.achievements(&g) {
            State::Ready(l) => Some((l.iter().filter(|a| a.unlocked.is_some()).count(), l.len())),
            _ => None,
        };
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
            if g.installed {
                tr("play")
            } else {
                tr("install")
            },
            FontId::proportional(24.0),
            Color32::WHITE,
        );
        if play_resp.clicked() {
            let k = g.key.clone();
            self.play(&k);
        }
        let mut tx = play_r.right() + 28.0;
        for (i, t) in self.tabs_of(&g).iter().enumerate() {
            let label = match t {
                Tab::Compat => tr("compat").to_string(),
                Tab::Versions => tr("versions").to_string(),
                Tab::Art => tr("art_tab").to_string(),
                Tab::Overview => tr("overview").to_string(),
                Tab::Achievements => match ach {
                    Some((a, b)) => format!("{} {a}/{b}", tr("achievements")),
                    None => tr("achievements").to_string(),
                },
                Tab::Screenshots => tr("screenshots").to_string(),
                Tab::Module(i) => self.mod_tab_label(*i),
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
                v.folded = false;
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
                // Down folds the header (when there's more below); up at
                // the very top opens it again.
                if d < 0.0 && v.overflow {
                    v.folded = true;
                } else if d > 0.0 && v.scroll <= 0.5 {
                    v.folded = false;
                }
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
            Tab::Compat => self.draw_compat(ui, body, &g, eased),
            Tab::Versions => {
                let (focus, sel) = self
                    .game
                    .as_ref()
                    .map(|v| (v.focus == INSIDE, v.tool_sel))
                    .unwrap_or((false, 0));
                self.draw_versions(ui, body, &g, eased, focus, sel)
            }
            Tab::Art => {
                let (focus, sel) = self
                    .game
                    .as_ref()
                    .map(|v| (v.focus == INSIDE, v.tool_sel))
                    .unwrap_or((false, 0));
                self.draw_art(ui, body, &g, eased, focus, sel)
            }
            Tab::Module(i) => {
                let focus = self.game.as_ref().is_some_and(|v| v.focus == INSIDE);
                self.draw_mod_tab(ui, body, &g, i, eased, focus)
            }
        };
        if let Some(v) = self.game.as_mut() {
            v.follow = false;
            // Measured against the page with the header folded (what it
            // is while scrolled): the end doesn't move as it folds.
            let full_lift = full_head + 28.0 - 16.0;
            let folded_h = body.height() + (full_lift - lift);
            v.max_scroll = (content_h - folded_h + 24.0).max(0.0);
            // Taller than the page would be with the header open.
            v.overflow = content_h + 24.0 > body.height() - lift;
            v.scroll = v.scroll.min(v.max_scroll);
        }
        self.draw_big_shot(ui, rect.expand2(vec2(0.0, 60.0)), &g);
        if self.versions.ask.is_some() {
            self.draw_versions_ask(ui, rect.expand2(vec2(0.0, 60.0)));
        }
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
        match g.info_appid().map(|id| self.store.details(id)) {
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
        let list: Vec<Achievement> = match Some(self.store.achievements(g)) {
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
            // Not yet: the common ones first, the secret ones last.
            (None, None) => a.hidden.cmp(&b.hidden).then(
                b.percent
                    .unwrap_or(0.0)
                    .total_cmp(&a.percent.unwrap_or(0.0)),
            ),
        });
        let row_h = 84.0;
        let (inside, sel) = self
            .game
            .as_ref()
            .map(|v| {
                (
                    v.focus == INSIDE,
                    v.tool_sel.min(sorted.len().saturating_sub(1)),
                )
            })
            .unwrap_or((false, 0));
        let list_top = y + scroll;
        if inside && let Some(v) = self.game.as_mut() {
            // A on the chosen one: show or hide it.
            if std::mem::take(&mut v.reveal)
                && let Some(a) = sorted.get(sel)
                && !v.revealed.remove(&a.id)
            {
                v.revealed.insert(a.id.clone());
            }
            // Keep it in view (after a key, not against the wheel).
            let top = list_top - body.top() + sel as f32 * row_h;
            if !v.follow {
            } else if top < v.scroll {
                v.scroll = (top - 70.0).max(0.0);
            } else if top + row_h > v.scroll + body.height() {
                v.scroll = top + row_h - body.height();
            }
        }
        let revealed = self
            .game
            .as_ref()
            .map(|v| v.revealed.clone())
            .unwrap_or_default();
        let mut clicked = None;
        for (ai, a) in sorted.iter().enumerate() {
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
            if inside && ai == sel {
                p.rect_stroke(r, 14.0, Stroke::new(2.5, c1), StrokeKind::Inside);
            }
            if ui
                .interact(r.intersect(body), ui.id().with(("ach", ai)), Sense::click())
                .clicked()
            {
                clicked = Some(ai);
            }
            let shown = revealed.contains(&a.id);
            let secret = a.hidden && !got && !shown;
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
            if secret && inside && ai == sel {
                p.text(
                    pos2(tx, r.top() + 40.0),
                    Align2::LEFT_TOP,
                    tr("ach_reveal"),
                    FontId::proportional(14.0),
                    Color32::from_gray(150),
                );
            }
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
            // When it was earned, and how many players have it.
            let when = a.unlocked.filter(|t| *t > 0).map(date);
            let share = a.percent.map(|v| {
                let n = format!("{v:.1}");
                let n = if i18n::ru() { n.replace('.', ",") } else { n };
                format!("{n}% {}", tr("of_players"))
            });
            let lines: Vec<(String, f32, u8)> = [(when, 15.0, 190), (share, 13.0, 140)]
                .into_iter()
                .filter_map(|(t, size, gray)| Some((t?, size, gray)))
                .collect();
            let step = 22.0;
            let mut ly = r.center().y - step * (lines.len() as f32 - 1.0) / 2.0;
            for (text, size, gray) in lines {
                p.text(
                    pos2(r.right() - 18.0, ly),
                    Align2::RIGHT_CENTER,
                    text,
                    FontId::proportional(size),
                    Color32::from_gray(gray),
                );
                ly += step;
            }
        }
        if let Some(ai) = clicked
            && let Some(v) = self.game.as_mut()
        {
            v.focus = INSIDE;
            v.tool_sel = ai;
            v.reveal = true;
        }
        y - (body.top() - scroll)
    }

    fn draw_shots(&mut self, ui: &mut Ui, body: Rect, g: &Game, scroll: f32) -> f32 {
        // Room at the sides for the focus frame.
        let p = ui.painter_at(body.expand2(vec2(8.0, 0.0)));
        let shots = match g.info_appid().map(|id| self.store.details(id)) {
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
        let top = 8.0;
        let w = (body.width() - gap * (SHOT_COLS - 1) as f32) / SHOT_COLS as f32;
        let h = w * 9.0 / 16.0;
        // Keep the chosen one in view.
        if focus == INSIDE && self.game.as_ref().is_some_and(|v| v.follow) {
            let row_top = (sel / SHOT_COLS) as f32 * (h + gap);
            let row_top = row_top.max(0.0);
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
                    body.top() + top - scroll + (i / SHOT_COLS) as f32 * (h + gap),
                ),
                vec2(w, h),
            );
            if r.bottom() < body.top() || r.top() > body.bottom() {
                continue;
            }
            let resp = ui.interact(r.intersect(body), ui.id().with(("shot", i)), Sense::click());
            // Rounded, edges smoothed like the cards.
            match self.images.get(thumb, Kind::Fit(640)) {
                Some(t) => card::draw(
                    &p,
                    &Pose::flat(r, 12.0),
                    &card::Fill::Image(t.handle.id(), card::cover_uv(t.size, r.size())),
                    Color32::WHITE,
                    None,
                ),
                None => {
                    p.rect_filled(r, 12.0, Color32::from_white_alpha(14));
                }
            }
            if focus == INSIDE && sel == i {
                p.rect_stroke(
                    r,
                    12.0,
                    Stroke::new(3.0, self.accent.0),
                    StrokeKind::Outside,
                );
            }
            if resp.clicked() {
                open = Some(i);
            }
        }
        if let (Some(i), Some(v)) = (open, self.game.as_mut()) {
            v.shot = Some(i);
            v.shot_sel = i;
            v.focus = INSIDE;
        }
        let rows = shots.len().div_ceil(SHOT_COLS);
        top + rows as f32 * (h + gap)
    }

    /// The compatibility tools as a list: the one in use marked, the
    /// focused one framed; a note and "Restart Steam" while a change waits.
    fn draw_compat(&mut self, ui: &mut Ui, body: Rect, g: &Game, scroll: f32) -> f32 {
        let Some(id) = g.compat_id else { return 0.0 };
        let p = ui.painter_at(body);
        let (c1, c2) = self.accent;
        let choices = self.tool_choices();
        let cur = self.steam.mapping.get(&id).cloned();
        let pending = !self.steam.pending.is_empty();
        let (focus, sel) = self
            .game
            .as_ref()
            .map(|v| (v.focus, v.tool_sel))
            .unwrap_or((0, 0));
        let mut y = body.top() - scroll;
        let note = p.layout(
            tr("tool_note").to_string(),
            FontId::proportional(15.0),
            Color32::from_gray(150),
            body.width().min(820.0),
        );
        let note_h = note.size().y;
        p.galley(pos2(body.left(), y), note, Color32::WHITE);
        y += note_h + 18.0;
        let row_h = 52.0;
        let w = body.width().min(620.0);
        let mut chosen = None;
        let mut restart = false;
        let mut maint: Option<usize> = None;
        let maint_at = choices.len() + usize::from(pending);
        let n = maint_at + MAINT_ROWS;
        for i in 0..n {
            if i == maint_at {
                // A little gap before the upkeep rows.
                y += 14.0;
            }
            let r = Rect::from_min_size(pos2(body.left(), y), vec2(w, row_h - 8.0));
            y += row_h;
            // Keep the focused row in view.
            if focus == INSIDE && i == sel && self.game.as_ref().is_some_and(|v| v.follow) {
                let top = r.top() + scroll - body.top();
                if let Some(v) = self.game.as_mut() {
                    if r.bottom() > body.bottom() {
                        v.scroll = (top + row_h - body.height()).max(0.0);
                    } else if r.top() < body.top() {
                        v.scroll = top.max(0.0);
                    }
                }
            }
            if r.bottom() < body.top() || r.top() > body.bottom() {
                continue;
            }
            let resp = ui.interact(r, ui.id().with(("tool", i)), Sense::click());
            let is_restart = pending && i == choices.len();
            let is_maint = i >= maint_at;
            let on = !is_restart && !is_maint && choices[i].0 == cur;
            p.rect_filled(
                r,
                12.0,
                if on {
                    Color32::from_white_alpha(34)
                } else if resp.hovered() {
                    Color32::from_white_alpha(20)
                } else {
                    Color32::from_black_alpha(90)
                },
            );
            if focus == INSIDE && i == sel {
                p.rect_stroke(r, 12.0, Stroke::new(2.5, c1), StrokeKind::Inside);
            }
            if is_maint {
                let (label, note) = if i == maint_at {
                    (tr("maint_shaders"), tr("maint_shaders_row"))
                } else {
                    (tr("maint_prefix"), tr("maint_prefix_row"))
                };
                p.text(
                    r.left_center() + vec2(48.0, -9.0),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(17.0),
                    Color32::WHITE,
                );
                p.text(
                    r.left_center() + vec2(48.0, 12.0),
                    Align2::LEFT_CENTER,
                    note,
                    FontId::proportional(13.0),
                    Color32::from_gray(150),
                );
            } else if is_restart {
                widgets::gradient_pill(
                    &p,
                    Rect::from_min_size(r.left_center() + vec2(16.0, -9.0), vec2(18.0, 18.0)),
                    9.0,
                    c1,
                    c2,
                );
                p.text(
                    r.left_center() + vec2(48.0, 0.0),
                    Align2::LEFT_CENTER,
                    if self.steam.restart.is_some() {
                        tr("steam_restarting")
                    } else {
                        tr("restart_steam")
                    },
                    FontId::proportional(17.0),
                    Color32::WHITE,
                );
            } else {
                // A dot for the one in use.
                if on {
                    p.circle_filled(r.left_center() + vec2(25.0, 0.0), 6.0, c1);
                } else {
                    p.circle_stroke(
                        r.left_center() + vec2(25.0, 0.0),
                        6.0,
                        Stroke::new(1.5, Color32::from_gray(120)),
                    );
                }
                p.text(
                    r.left_center() + vec2(48.0, 0.0),
                    Align2::LEFT_CENTER,
                    &choices[i].1,
                    FontId::proportional(17.0),
                    Color32::from_gray(if on { 255 } else { 205 }),
                );
                if on && self.has_pending(id) {
                    p.text(
                        r.right_center() - vec2(16.0, 0.0),
                        Align2::RIGHT_CENTER,
                        tr("tool_waiting"),
                        FontId::proportional(14.0),
                        Color32::from_gray(180),
                    );
                }
            }
            if resp.clicked() {
                if is_maint {
                    maint = Some(i - maint_at);
                } else if is_restart {
                    restart = true;
                } else {
                    chosen = Some(choices[i].0.clone());
                }
                if let Some(v) = self.game.as_mut() {
                    v.focus = INSIDE;
                    v.tool_sel = i;
                }
            }
        }
        if let Some(t) = chosen {
            self.choose_tool(id, t);
        }
        if restart {
            self.restart_steam();
        }
        match maint {
            Some(0) => self.clear_shaders(g),
            Some(_) => self.reset_prefix(g),
            None => {}
        }
        note_h + 18.0 + n as f32 * row_h + 28.0
    }

    fn draw_big_shot(&mut self, ui: &mut Ui, area: Rect, g: &Game) {
        let Some(i) = self.game.as_ref().and_then(|v| v.shot) else {
            return;
        };
        let Some(id) = g.info_appid() else { return };
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
