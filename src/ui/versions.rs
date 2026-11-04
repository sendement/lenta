//! The Versions tab: a Steam game's known builds. A version is ticked,
//! "Install" asks first (what will happen: download, Steam closed, the lock),
//! then one job downloads what's missing and installs it. The lock can be
//! lifted from the same list.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Receiver;
use egui::{
    Align2, Color32, FontId, Id, LayerId, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::app::App;
use crate::downgrade::flow::{self, Loaded, NEED_LOGIN, Progress, changes};
use crate::game::Game;
use crate::i18n::tr;
use crate::sources::steamplay::steam_running;

/// A download and install (or an unlock) on its thread.
pub struct Op {
    pub app: String,
    pub what: &'static str,
    pub progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
    pub done: Receiver<Result<String, String>>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ask {
    Install(usize),
    Unlock { validate: bool },
    Cancel,
}

#[derive(Default)]
pub struct Versions {
    pub app: String,
    pub loading: Option<Receiver<Option<Loaded>>>,
    pub data: Option<Loaded>,
    pub op: Option<Op>,
    /// The ticked version.
    pub ticked: Option<usize>,
    /// The question on screen.
    pub ask: Option<Ask>,
}

#[derive(Clone, Copy, PartialEq)]
enum Row {
    /// "Install the ticked version" (or the running job, to cancel).
    Action,
    Version(usize),
    Unlock,
    UnlockValidate,
}

impl Versions {
    fn rows(&self) -> Vec<Row> {
        let Some(d) = &self.data else {
            return Vec::new();
        };
        let mut v = vec![Row::Action];
        v.extend((0..d.versions.len()).map(Row::Version));
        if d.lock.is_some() {
            v.push(Row::Unlock);
            v.push(Row::UnlockValidate);
        }
        v
    }

    pub fn row_count(&self) -> usize {
        self.rows().len()
    }

    /// Where the list starts: the installed version.
    pub fn start_row(&self) -> usize {
        let Some(d) = &self.data else { return 0 };
        d.versions
            .iter()
            .position(|v| changes(v, &d.actual).is_empty())
            .map_or(0, |i| i + 1)
    }
}

fn human(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.1} {}", b / 1e9, tr("gb"))
    } else {
        format!("{:.0} {}", b / 1e6, tr("mb"))
    }
}

impl App {
    /// Steam's own games (not shortcuts), installed, can be rolled back.
    pub fn can_downgrade(&self, g: &Game) -> bool {
        self.cfg.steam && g.installed && g.key.starts_with("steam:") && g.steam_appid.is_some()
    }

    pub fn load_versions(&mut self, app: &str) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        let a = app.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(flow::load(&a));
            ctx.request_repaint();
        });
        if self.versions.app != app {
            self.versions.ticked = None;
            self.versions.data = None;
        }
        self.versions.app = app.to_string();
        self.versions.loading = Some(rx);
        self.versions.ask = None;
    }

    pub(crate) fn pump_versions(&mut self) {
        if let Some(rx) = &self.versions.loading
            && let Ok(d) = rx.try_recv()
        {
            self.versions.data = d;
            self.versions.loading = None;
        }
        let done = self
            .versions
            .op
            .as_ref()
            .and_then(|o| o.done.try_recv().ok().map(|r| (o.app.clone(), r)));
        if let Some((app, res)) = done {
            self.versions.op = None;
            match res {
                Ok(msg) => {
                    self.versions.ticked = None;
                    self.toast(msg);
                }
                Err(e) if e == NEED_LOGIN => {
                    self.toast(tr("sd_need_login"));
                    self.start_login();
                }
                Err(e) => self.toast(format!("{}: {e}", tr("sd_failed"))),
            }
            if self.versions.app == app {
                self.load_versions(&app);
            }
            self.rescan();
        }
    }

    fn start_op(
        &mut self,
        what: &'static str,
        job: impl FnOnce(&AtomicBool, &(dyn Fn(Progress) + Sync)) -> Result<String, String>
        + Send
        + 'static,
    ) {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (p, c, ctx) = (progress.clone(), cancel.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let report = |pr: Progress| {
                if let Ok(mut g) = p.lock() {
                    *g = pr;
                }
                ctx.request_repaint_after(Duration::from_millis(200));
            };
            let _ = tx.send(job(&c, &report));
            ctx.request_repaint();
        });
        self.versions.op = Some(Op {
            app: self.versions.app.clone(),
            what,
            progress,
            cancel,
            done: rx,
        });
    }

    /// Download what's missing, then install and lock.
    fn install_version(&mut self, i: usize) {
        let Some(data) = self.versions.data.clone() else {
            return;
        };
        let Some(v) = data.versions.get(i).cloned() else {
            return;
        };
        let target = changes(&v, &data.actual);
        if target.is_empty() {
            return;
        }
        let jobs = data.jobs(&target);
        let account = self.steam.account.clone();
        if !jobs.is_empty() && !account.as_ref().is_some_and(|a| a.is_client()) {
            self.toast(tr("sd_need_login"));
            self.start_login();
            return;
        }
        let app = data.game.app.clone();
        let buildid = (v.exact && v.buildid != 0).then_some(v.buildid);
        self.start_op("sd_installing_version", move |cancel, report| {
            if let Some(acc) = &account
                && !jobs.is_empty()
            {
                flow::download_all(&jobs, acc, cancel, report)?;
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(tr("sd_cancelled").into());
            }
            report(Progress {
                status: "sd_installing",
                ..Default::default()
            });
            flow::install(&app, &target, buildid)
        });
    }

    fn unlock_version(&mut self, validate: bool) {
        let app = self.versions.app.clone();
        self.start_op("sd_unlocking", move |_, report| {
            report(Progress {
                status: "sd_unlocking",
                ..Default::default()
            });
            flow::unlock(&app, validate)
        });
    }

    /// A on a list row: tick a version, press the button, ask to unlock.
    pub fn versions_accept(&mut self, row: usize) {
        let rows = self.versions.rows();
        match rows.get(row).copied() {
            Some(Row::Action) => self.versions_install(),
            Some(Row::Version(i)) => {
                let Some(d) = &self.versions.data else { return };
                // The installed one can't be ticked.
                if changes(&d.versions[i], &d.actual).is_empty() {
                    return;
                }
                self.versions.ticked = if self.versions.ticked == Some(i) {
                    None
                } else {
                    Some(i)
                };
            }
            Some(Row::Unlock) if self.versions.op.is_none() => {
                self.versions.ask = Some(Ask::Unlock { validate: false })
            }
            Some(Row::UnlockValidate) if self.versions.op.is_none() => {
                self.versions.ask = Some(Ask::Unlock { validate: true })
            }
            _ => {}
        }
    }

    /// X, or the button: ask about the ticked version (or cancelling).
    pub fn versions_install(&mut self) {
        if self.versions.op.is_some() {
            self.versions.ask = Some(Ask::Cancel);
        } else if let Some(i) = self.versions.ticked {
            self.versions.ask = Some(Ask::Install(i));
        } else {
            self.toast(tr("sd_tick_first"));
        }
    }

    /// The question's answer: A yes, B no.
    pub fn versions_answer(&mut self, yes: bool) {
        let Some(ask) = self.versions.ask.take() else {
            return;
        };
        if !yes {
            return;
        }
        match ask {
            Ask::Install(i) => self.install_version(i),
            Ask::Unlock { validate } => self.unlock_version(validate),
            Ask::Cancel => {
                if let Some(o) = &self.versions.op {
                    o.cancel.store(true, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn draw_versions(
        &mut self,
        ui: &mut Ui,
        body: Rect,
        g: &Game,
        scroll: f32,
        focus_inside: bool,
        sel: usize,
    ) -> f32 {
        self.pump_versions();
        let app = g.steam_appid.map(|a| a.to_string()).unwrap_or_default();
        if self.versions.app != app
            || (self.versions.data.is_none() && self.versions.loading.is_none())
        {
            self.load_versions(&app);
        }
        if self.versions.op.is_some() || self.versions.loading.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        let p = ui.painter_at(body);
        let (c1, c2) = self.accent;
        let small = FontId::proportional(15.0);
        let Some(data) = self.versions.data.clone() else {
            let t = if self.versions.loading.is_some() {
                tr("loading")
            } else {
                tr("sd_no_data")
            };
            p.text(
                body.left_top(),
                Align2::LEFT_TOP,
                t,
                FontId::proportional(18.0),
                Color32::from_gray(180),
            );
            return 30.0;
        };
        let mut y = body.top() - scroll;
        let installed = format!(
            "{}: {} {}{}",
            tr("sd_installed"),
            tr("ver_build"),
            data.lock
                .as_ref()
                .and_then(|l| l.actual_buildid)
                .unwrap_or(data.game.buildid),
            if data.lock.is_some() {
                format!("  ·  {}", tr("sd_locked"))
            } else {
                String::new()
            }
        );
        p.text(
            pos2(body.left(), y),
            Align2::LEFT_TOP,
            installed,
            FontId::proportional(18.0),
            Color32::WHITE,
        );
        y += 34.0;
        let rows = self.versions.rows();
        let w = body.width().min(980.0);
        let mut clicked = None;
        for (i, row) in rows.iter().enumerate() {
            let h = if *row == Row::Action { 58.0 } else { 64.0 };
            let r = Rect::from_min_size(pos2(body.left(), y), vec2(w, h - 8.0));
            y += h;
            if focus_inside && i == sel && self.game.as_ref().is_some_and(|v| v.follow) {
                let top = r.top() + scroll - body.top();
                if let Some(v) = self.game.as_mut() {
                    if r.bottom() > body.bottom() {
                        v.scroll = (top + h - body.height()).max(0.0);
                    } else if r.top() < body.top() {
                        v.scroll = top.max(0.0);
                    }
                }
            }
            if r.bottom() < body.top() || r.top() > body.bottom() {
                continue;
            }
            let resp = ui.interact(r, ui.id().with(("ver", i)), Sense::click());
            let focused = focus_inside && i == sel;
            match row {
                Row::Action => self.draw_action_row(&p, r, &data, &app, focused),
                Row::Version(vi) => {
                    let v = &data.versions[*vi];
                    let target = changes(v, &data.actual);
                    let current = target.is_empty();
                    let ticked = self.versions.ticked == Some(*vi);
                    p.rect_filled(
                        r,
                        12.0,
                        if current || ticked {
                            Color32::from_white_alpha(30)
                        } else if resp.hovered() {
                            Color32::from_white_alpha(18)
                        } else {
                            Color32::from_black_alpha(90)
                        },
                    );
                    if focused {
                        p.rect_stroke(r, 12.0, Stroke::new(2.5, c1), StrokeKind::Inside);
                    }
                    // The tick box; the installed version gets a dot.
                    let bx =
                        Rect::from_center_size(r.left_center() + vec2(24.0, 0.0), vec2(20.0, 20.0));
                    if current {
                        p.circle_filled(bx.center(), 6.0, c1);
                    } else if ticked {
                        crate::ui::widgets::gradient_pill(&p, bx, 5.0, c1, c2);
                        let st = Stroke::new(2.2, Color32::WHITE);
                        let a = bx.left_center() + vec2(4.5, 0.5);
                        let b = bx.center_bottom() + vec2(-1.5, -5.0);
                        let c = bx.right_top() + vec2(-4.0, 5.0);
                        p.line_segment([a, b], st);
                        p.line_segment([b, c], st);
                    } else {
                        p.rect_stroke(
                            bx,
                            5.0,
                            Stroke::new(1.6, Color32::from_gray(140)),
                            StrokeKind::Inside,
                        );
                    }
                    let mut sub = crate::game::date_time(v.time.max(0) as u64);
                    if !v.game_version.is_empty() {
                        sub += &format!(
                            "  ·  v{}{}",
                            if v.game_version_exact { "" } else { "≈" },
                            v.game_version
                        );
                    }
                    sub += &format!(
                        "  ·  {}{}",
                        v.source.label(),
                        if v.exact { "" } else { tr("sd_rebuilt") }
                    );
                    let state = if current {
                        tr("sd_current")
                    } else if data.ready(&target) {
                        tr("sd_downloaded_state")
                    } else {
                        ""
                    };
                    p.text(
                        r.left_top() + vec2(48.0, 9.0),
                        Align2::LEFT_TOP,
                        &v.title,
                        FontId::proportional(17.0),
                        Color32::WHITE,
                    );
                    p.text(
                        r.left_top() + vec2(48.0, 33.0),
                        Align2::LEFT_TOP,
                        sub,
                        FontId::proportional(13.0),
                        Color32::from_gray(150),
                    );
                    p.text(
                        r.right_center() - vec2(18.0, 0.0),
                        Align2::RIGHT_CENTER,
                        state,
                        small.clone(),
                        Color32::from_gray(205),
                    );
                }
                Row::Unlock | Row::UnlockValidate => {
                    let validate = *row == Row::UnlockValidate;
                    p.rect_filled(
                        r,
                        12.0,
                        if resp.hovered() {
                            Color32::from_white_alpha(18)
                        } else {
                            Color32::from_black_alpha(90)
                        },
                    );
                    if focused {
                        p.rect_stroke(r, 12.0, Stroke::new(2.5, c1), StrokeKind::Inside);
                    }
                    p.text(
                        r.left_top() + vec2(48.0, 9.0),
                        Align2::LEFT_TOP,
                        tr(if validate {
                            "sd_unlock_validate"
                        } else {
                            "sd_unlock"
                        }),
                        FontId::proportional(17.0),
                        Color32::WHITE,
                    );
                    p.text(
                        r.left_top() + vec2(48.0, 33.0),
                        Align2::LEFT_TOP,
                        tr(if validate {
                            "sd_unlock_validate_note"
                        } else {
                            "sd_unlock_note"
                        }),
                        FontId::proportional(13.0),
                        Color32::from_gray(150),
                    );
                }
            }
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked
            && self.versions.ask.is_none()
        {
            if let Some(v) = self.game.as_mut() {
                v.focus = crate::ui::game_page::INSIDE;
                v.tool_sel = i;
            }
            self.versions_accept(i);
        }
        y + scroll - body.top()
    }

    /// The button over the list: install the ticked version, or the job
    /// in progress (pressed: cancel).
    fn draw_action_row(&self, p: &egui::Painter, r: Rect, data: &Loaded, app: &str, focused: bool) {
        let (c1, c2) = self.accent;
        let small = FontId::proportional(15.0);
        if let Some(op) = self.versions.op.as_ref().filter(|o| o.app == app) {
            let pr = op.progress.lock().map(|g| g.clone()).unwrap_or_default();
            let s = pr.stats;
            let frac = if s.total > 0 {
                s.done() as f32 / s.total as f32
            } else {
                0.0
            };
            p.rect_filled(r, 12.0, Color32::from_black_alpha(110));
            if s.total > 0 {
                let bar = Rect::from_min_max(
                    pos2(r.left() + 16.0, r.bottom() - 14.0),
                    pos2(r.right() - 16.0, r.bottom() - 8.0),
                );
                crate::ui::widgets::bar(p, bar, frac, c1, c2);
            }
            let status = if pr.status.is_empty() {
                "dl_signing_in"
            } else {
                pr.status
            };
            let mut line = format!("{} · {}", tr(op.what), tr(status));
            if pr.depots > 1 {
                line += &format!(" · {} {}/{}", tr("sd_depot"), pr.depot + 1, pr.depots);
            }
            if s.total > 0 {
                line += &format!(" · {:.0}%", frac * 100.0);
            }
            if pr.speed > 0 {
                line += &format!(" · {}/{}", human(pr.speed), tr("sec"));
            }
            p.text(
                r.left_top() + vec2(16.0, 10.0),
                Align2::LEFT_TOP,
                line,
                small.clone(),
                Color32::WHITE,
            );
            p.text(
                r.right_top() + vec2(-16.0, 10.0),
                Align2::RIGHT_TOP,
                tr("sd_cancel"),
                small,
                Color32::from_gray(190),
            );
        } else {
            let on = self.versions.ticked.is_some();
            if on {
                crate::ui::widgets::gradient_pill(
                    p,
                    r,
                    12.0,
                    c1.gamma_multiply(0.9),
                    c2.gamma_multiply(0.9),
                );
            } else {
                p.rect_filled(r, 12.0, Color32::from_white_alpha(14));
            }
            let label = match self.versions.ticked.and_then(|t| data.versions.get(t)) {
                Some(v) => format!("{} «{}»", tr("sd_install_ticked"), v.title),
                None => tr("sd_tick_hint").to_string(),
            };
            p.text(
                r.left_center() + vec2(18.0, 0.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(17.0),
                if on {
                    Color32::WHITE
                } else {
                    Color32::from_gray(160)
                },
            );
        }
        if focused {
            p.rect_stroke(
                r,
                12.0,
                Stroke::new(2.5, Color32::WHITE),
                StrokeKind::Inside,
            );
        }
    }

    /// The question over the page, when one is asked.
    pub fn draw_versions_ask(&mut self, ui: &mut Ui, area: Rect) {
        let Some(ask) = self.versions.ask else { return };
        let Some(data) = self.versions.data.clone() else {
            self.versions.ask = None;
            return;
        };
        let p = ui
            .ctx()
            .layer_painter(LayerId::new(Order::Foreground, Id::new("versions_ask")));
        p.rect_filled(area, 0.0, Color32::from_black_alpha(190));
        let (c1, c2) = self.accent;
        let (title, lines, yes): (String, Vec<String>, &str) = match ask {
            Ask::Install(i) => {
                let v = &data.versions[i];
                let target = changes(v, &data.actual);
                let mut lines = Vec::new();
                let jobs = data.jobs(&target);
                if !jobs.is_empty() {
                    lines.push(format!(
                        "{} {} {}",
                        tr("sd_q_download"),
                        jobs.len(),
                        tr("sd_q_depots")
                    ));
                }
                if steam_running() {
                    lines.push(tr("sd_q_steam").to_string());
                }
                lines.push(tr("sd_q_lock").to_string());
                lines.push(tr("sd_q_saves").to_string());
                let ver = if v.game_version.is_empty() {
                    String::new()
                } else {
                    format!(" (v{})", v.game_version)
                };
                (
                    format!("{} «{}»{ver}?", tr("sd_q_install"), v.title),
                    lines,
                    tr("sd_install"),
                )
            }
            Ask::Unlock { validate } => {
                let mut lines = vec![
                    tr(if validate {
                        "sd_unlock_validate_note"
                    } else {
                        "sd_unlock_note"
                    })
                    .to_string(),
                ];
                if steam_running() {
                    lines.push(tr("sd_q_steam").to_string());
                }
                (
                    format!(
                        "{}?",
                        tr(if validate {
                            "sd_unlock_validate"
                        } else {
                            "sd_unlock"
                        })
                    ),
                    lines,
                    tr("sd_unlock_short"),
                )
            }
            Ask::Cancel => (
                format!("{}?", tr("sd_cancel")),
                vec![tr("sd_q_cancel").to_string()],
                tr("sd_cancel"),
            ),
        };
        let card = Rect::from_center_size(
            area.center(),
            vec2(680.0, 150.0 + lines.len() as f32 * 30.0),
        );
        p.rect_filled(card, 20.0, Color32::from_rgb(24, 27, 36));
        let tg = p.layout(
            title,
            FontId::proportional(24.0),
            Color32::WHITE,
            card.width() - 64.0,
        );
        let th = tg.size().y;
        p.galley(card.min + vec2(32.0, 28.0), tg, Color32::WHITE);
        let mut y = card.top() + 28.0 + th + 14.0;
        for l in &lines {
            p.text(
                pos2(card.left() + 32.0, y),
                Align2::LEFT_TOP,
                format!("•  {l}"),
                FontId::proportional(16.0),
                Color32::from_gray(200),
            );
            y += 28.0;
        }
        // Yes (A) on the right, no (B) beside it.
        let bw = 190.0;
        let yes_r = Rect::from_min_size(
            pos2(card.right() - 32.0 - bw, card.bottom() - 70.0),
            vec2(bw, 46.0),
        );
        let no_r = yes_r.translate(vec2(-bw - 14.0, 0.0));
        crate::ui::widgets::gradient_pill(&p, yes_r, 14.0, c1, c2);
        p.rect_filled(no_r, 14.0, Color32::from_white_alpha(22));
        p.text(
            yes_r.center(),
            Align2::CENTER_CENTER,
            yes,
            FontId::proportional(18.0),
            Color32::WHITE,
        );
        p.text(
            no_r.center(),
            Align2::CENTER_CENTER,
            tr("cancel"),
            FontId::proportional(18.0),
            Color32::from_gray(220),
        );
        // The veil takes clicks, the buttons answer.
        let layer = LayerId::new(Order::Foreground, Id::new("versions_ask"));
        let top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(area));
        let _veil = top.interact(area, Id::new("ask_veil"), Sense::click());
        let yes_resp = top.interact(yes_r, Id::new("ask_yes"), Sense::click());
        let no_resp = top.interact(no_r, Id::new("ask_no"), Sense::click());
        if yes_resp.clicked() {
            self.versions_answer(true);
        } else if no_resp.clicked() {
            self.versions_answer(false);
        }
    }
}
