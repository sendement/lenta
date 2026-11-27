//! A game's launch options, kept in step with its launcher's (Steam,
//! Lutris): the Launch tab on its page, a look before every start, and the
//! writes (see src/launch.rs for who wins and how Steam is reached).

use egui::{Align2, Color32, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::app::App;
use crate::game::Game;
use crate::i18n::tr;
use crate::input::Action;
use crate::launch::{self, SteamWrite};
use crate::sources::steamplay;
use crate::ui::game_page::INSIDE;
use crate::ui::widgets::Btn;

/// The game's launcher, as far as its options go.
enum Where {
    Steam(u32),
    Lutris(i64),
}

fn place(g: &Game) -> Option<Where> {
    if let Some(id) = g.steam_appid
        && g.key.starts_with("steam:")
    {
        return Some(Where::Steam(id));
    }
    g.key
        .strip_prefix("lutris:")
        .and_then(|i| i.parse().ok())
        .map(Where::Lutris)
}

/// The rows that can be chosen in the tab.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Line,
    Revert,
    Restart,
}

impl App {
    pub fn has_launch_tab(&self, g: &Game) -> bool {
        g.installed && place(g).is_some()
    }

    /// Look at the launcher's options (it may have changed them).
    pub fn start_sync(&mut self, key: &str) {
        let Some(g) = self.game_by_key(key) else {
            return;
        };
        let theirs = match place(g) {
            Some(Where::Steam(id)) => launch::steam_options(id)
                .map(|(o, t)| (launch::without_hook(&o, &launch::token(key)), t)),
            Some(Where::Lutris(id)) => launch::lutris_options(id),
            None => None,
        };
        if let Some((o, t)) = theirs {
            self.launch.sync(key, &o, t);
            let _ = self.launch.save(key);
        }
    }

    /// Bring the launcher in step: lenta's options if changed here, the
    /// script at the front of Steam's while anything needs it.
    pub fn start_push(&mut self, key: &str) {
        let Some(g) = self.game_by_key(key).cloned() else {
            return;
        };
        let s = self.launch.get(key).cloned().unwrap_or_default();
        let line = s.line.clone().unwrap_or_default();
        match place(&g) {
            Some(Where::Lutris(id)) if s.dirty => match launch::write_lutris(id, &line) {
                Ok(()) => self.launch.written(key, &line),
                Err(e) => self.toast(e),
            },
            Some(Where::Steam(appid)) => {
                let hooked = launch::hooked(appid, key);
                self.steam.hooks.retain(|h| h.appid != appid);
                if !steamplay::steam_running() {
                    // Closed: written now, the script only for modules.
                    let hook = self.launch.wants(key);
                    if s.dirty || hook != hooked {
                        let w = SteamWrite {
                            appid,
                            key: key.to_string(),
                            line: s.dirty.then(|| line.clone()),
                            hook,
                        };
                        match launch::write_steam(std::slice::from_ref(&w)) {
                            Ok(()) => self.steam_written(&[w]),
                            Err(e) => self.toast(e),
                        }
                    }
                } else if self.launch.wants_hook(key) && !hooked {
                    // Running and not reached yet: once Steam closes.
                    self.steam.hooks.push(SteamWrite {
                        appid,
                        key: key.to_string(),
                        line: s.dirty.then(|| line.clone()),
                        hook: self.launch.wants(key),
                    });
                    self.toast(tr("launch_pending").to_string());
                } else if !self.launch.wants_hook(key) && hooked {
                    self.steam.hooks.push(SteamWrite {
                        appid,
                        key: key.to_string(),
                        line: None,
                        hook: false,
                    });
                }
            }
            _ => {}
        }
        if let Err(e) = self.launch.save(key) {
            self.toast(e);
        }
    }

    /// These reached Steam's file.
    pub fn steam_written(&mut self, ws: &[SteamWrite]) {
        for w in ws {
            if let Some(l) = &w.line {
                self.launch.written(&w.key, l);
            }
            let _ = self.launch.save(&w.key);
        }
    }

    fn launch_rows(&self, g: &Game) -> Vec<Row> {
        let s = self.launch.get(&g.key);
        let mut v = vec![Row::Line];
        if s.is_some_and(|s| s.dirty) {
            v.push(Row::Revert);
        }
        if g.steam_appid
            .is_some_and(|id| self.steam.hooks.iter().any(|h| h.appid == id))
        {
            v.push(Row::Restart);
        }
        v
    }

    fn launch_act(&mut self, g: &Game, r: Row) {
        let key = g.key.clone();
        match r {
            Row::Line => {
                let now = self
                    .launch
                    .get(&key)
                    .and_then(|s| s.line.clone())
                    .unwrap_or_default();
                self.type_text(tr("launch_opts").to_string(), now, move |app, text| {
                    app.launch.edit(&key, &text);
                    app.start_push(&key);
                });
            }
            Row::Revert => {
                let seen = self
                    .launch
                    .get(&key)
                    .map(|s| s.seen.clone())
                    .unwrap_or_default();
                self.launch.edit(&key, &seen);
                self.start_push(&key);
            }
            Row::Restart => self.restart_steam(),
        }
    }

    /// The pad inside the tab; true: back to the tabs.
    pub fn launch_input(&mut self, g: &Game, a: Action) -> bool {
        let rows = self.launch_rows(g);
        let Some(v) = self.game.as_mut() else {
            return true;
        };
        let sel = v.tool_sel.min(rows.len() - 1);
        match a {
            Action::Up if sel == 0 => return true,
            Action::Up => v.tool_sel = sel - 1,
            Action::Down => v.tool_sel = (sel + 1).min(rows.len() - 1),
            Action::Back => return true,
            Action::Accept => self.launch_act(g, rows[sel]),
            _ => {}
        }
        false
    }

    pub fn launch_hints(&self) -> Vec<(Btn, &'static str)> {
        vec![(Btn::A, tr("choose")), (Btn::B, tr("back"))]
    }

    /// The Launch tab; its content's height.
    pub fn draw_launch(&mut self, ui: &mut Ui, body: Rect, g: &Game, scroll: f32) -> f32 {
        if self.start_seen.as_deref() != Some(g.key.as_str()) {
            self.start_seen = Some(g.key.clone());
            self.start_sync(&g.key);
        }
        let (c1, _) = self.accent;
        let p = ui.painter_at(body.expand2(vec2(8.0, 0.0)));
        let s = self.launch.get(&g.key).cloned().unwrap_or_default();
        let steam = matches!(place(g), Some(Where::Steam(_)));
        let who = if steam { "Steam" } else { "Lutris" };
        let rows = self.launch_rows(g);
        let (focus, sel) = self
            .game
            .as_ref()
            .map(|v| (v.focus == INSIDE, v.tool_sel.min(rows.len() - 1)))
            .unwrap_or((false, 0));
        let w = body.width().min(1000.0);
        let mut y = body.top() - scroll;
        let mut act = None;
        for (i, r) in rows.iter().enumerate() {
            let row = Rect::from_min_size(pos2(body.left(), y), vec2(w, 60.0));
            y += 66.0;
            let resp = ui.interact(
                row.intersect(body),
                Id::new(("launch_row", i)),
                Sense::click(),
            );
            let on = focus && i == sel;
            if on {
                p.rect_filled(row, 12.0, Color32::from_white_alpha(22));
            } else if resp.hovered() {
                p.rect_filled(row, 12.0, Color32::from_white_alpha(10));
            }
            let tc = Color32::from_gray(if on { 255 } else { 210 });
            match r {
                Row::Line => {
                    p.text(
                        pos2(row.left() + 20.0, row.center().y),
                        Align2::LEFT_CENTER,
                        tr("launch_opts"),
                        FontId::proportional(18.0),
                        tc,
                    );
                    let f = Rect::from_min_max(
                        pos2(row.left() + w * 0.32, row.top() + 10.0),
                        pos2(row.right() - 16.0, row.bottom() - 10.0),
                    );
                    p.rect_filled(f, 10.0, Color32::from_black_alpha(90));
                    p.rect_stroke(
                        f,
                        10.0,
                        Stroke::new(1.0, Color32::from_white_alpha(if on { 120 } else { 40 })),
                        StrokeKind::Inside,
                    );
                    let line = s.line.clone().unwrap_or_default();
                    let (text, color) = if line.is_empty() {
                        (tr("launch_none").to_string(), Color32::from_gray(120))
                    } else {
                        (line, Color32::WHITE)
                    };
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        text,
                        FontId::monospace(15.0),
                        color,
                    );
                    job.wrap.max_width = f.width() - 24.0;
                    job.wrap.max_rows = 1;
                    let gal = p.fonts_mut(|fo| fo.layout_job(job));
                    p.galley(
                        pos2(f.left() + 12.0, f.center().y - gal.size().y / 2.0),
                        gal,
                        Color32::WHITE,
                    );
                }
                Row::Revert | Row::Restart => {
                    let (label, button) = if *r == Row::Revert {
                        (
                            format!("{} {who}", tr("launch_revert")),
                            tr("launch_revert_btn"),
                        )
                    } else {
                        (tr("restart_steam").to_string(), tr("launch_restart_btn"))
                    };
                    p.text(
                        pos2(row.left() + 20.0, row.center().y),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(18.0),
                        tc,
                    );
                    let gal = p.layout_no_wrap(
                        button.to_string(),
                        FontId::proportional(16.0),
                        Color32::WHITE,
                    );
                    let bw = gal.size().x + 40.0;
                    let b = Rect::from_min_max(
                        pos2(row.right() - 16.0 - bw, row.top() + 14.0),
                        pos2(row.right() - 16.0, row.bottom() - 14.0),
                    );
                    p.rect_filled(
                        b,
                        b.height() / 2.0,
                        if on {
                            c1.gamma_multiply(0.8)
                        } else {
                            Color32::from_white_alpha(25)
                        },
                    );
                    p.galley(b.center() - gal.size() / 2.0, gal, Color32::WHITE);
                }
            }
            if resp.clicked() {
                if let Some(v) = self.game.as_mut() {
                    v.focus = INSIDE;
                    v.tool_sel = i;
                }
                act = Some(*r);
            }
        }
        // Where it stands.
        let pending = g
            .steam_appid
            .is_some_and(|id| self.steam.hooks.iter().any(|h| h.appid == id));
        let state = if s.dirty && pending {
            tr("launch_wait_restart").to_string()
        } else if s.dirty {
            format!("{} {who}.", tr("launch_dirty"))
        } else if s.taken {
            format!("{} {who}.", tr("launch_taken"))
        } else {
            format!("{} {who}.", tr("launch_same"))
        };
        let mut lines = vec![(state, Color32::from_gray(190))];
        if steam && s.dirty && !pending {
            lines.push((tr("launch_steam_note").to_string(), Color32::from_gray(140)));
        }
        if !s.parts.is_empty() {
            lines.push((tr("launch_modules").to_string(), Color32::from_gray(190)));
            for (m, part) in &s.parts {
                let name = self
                    .mods
                    .list
                    .iter()
                    .find(|x| &x.id == m)
                    .map(|x| x.name.clone())
                    .unwrap_or_else(|| m.clone());
                let mut what: Vec<String> = part.wrap.clone();
                what.extend(part.env.keys().cloned());
                lines.push((
                    format!("{name}: {}", what.join(", ")),
                    Color32::from_gray(150),
                ));
            }
        }
        y += 8.0;
        for (t, c) in lines {
            let gal = p.layout(t, FontId::proportional(16.0), c, w - 40.0);
            let h = gal.size().y;
            p.galley(pos2(body.left() + 20.0, y), gal, Color32::WHITE);
            y += h + 10.0;
        }
        if let Some(r) = act {
            self.launch_act(g, r);
        }
        y + scroll - body.top()
    }
}

// ------------------------------------------------------------------ running

impl App {
    pub fn is_running(&self, key: &str) -> bool {
        self.running.lock().is_ok_and(|s| s.running.contains(key))
    }

    pub fn work_of(&self, key: &str) -> Option<crate::running::Work> {
        self.running
            .lock()
            .ok()
            .and_then(|s| s.work.get(key).copied())
    }

    /// Play pressed, not running yet (for a minute and a half at most).
    pub fn is_starting(&self, key: &str) -> bool {
        self.launching
            .as_ref()
            .is_some_and(|(k, t)| k == key && t.elapsed() < std::time::Duration::from_secs(90))
            && !self.is_running(key)
    }

    pub fn ask_stop(&mut self, key: &str) {
        let Some(g) = self.game_by_key(key) else {
            return;
        };
        let w = crate::running::Watch {
            key: g.key.clone(),
            appid: g.steam_appid,
            dir: g.install_dir.clone(),
        };
        let title = g.title.clone();
        self.ask(
            tr("stop_ask").to_string(),
            vec![title.clone(), tr("stop_lines").to_string()],
            tr("stop_game"),
            move |app| {
                crate::running::stop(&w);
                app.toast(format!("{}: {title}", tr("stopped")));
            },
        );
    }

    /// Every frame: a game that ended brings lenta back to the front (and
    /// to full screen, which Steam's windows may have taken).
    pub fn running_tick(&mut self) {
        let earned: Vec<String> = self
            .ach_earned
            .lock()
            .map(|mut e| e.drain().collect())
            .unwrap_or_default();
        for k in earned {
            self.store.forget_achievements_of(&k);
        }
        let now: std::collections::HashSet<String> = self
            .running
            .lock()
            .map(|s| s.running.clone())
            .unwrap_or_default();
        if let Some((k, t)) = &self.launching
            && (now.contains(k) || t.elapsed() > std::time::Duration::from_secs(90))
        {
            self.launching = None;
        }
        let ended = self.was_running.iter().any(|k| !now.contains(k));
        if ended && now.is_empty() {
            if self.cfg.fullscreen {
                self.ctx
                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.was_running = now;
        if self.launching.is_some() {
            self.ctx
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}
