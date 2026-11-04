//! The Proton page: the compatibility tools there are, the newest custom
//! builds to install (GE-Proton, Proton-CachyOS), Valve's own ones (Steam
//! installs and removes those), and removing what isn't wanted.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Receiver;
use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::app::App;
use crate::i18n::tr;
use crate::input::Action;
use crate::protonup::{self, Flavour, Progress, Release};
use crate::sources::steamplay::{self, Tool};

pub struct Job {
    pub tag: String,
    pub progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
    pub done: Receiver<Result<String, String>>,
}

/// A flavour's release list as fetched.
pub type Listed = (Flavour, Result<Vec<Release>, String>);

#[derive(Default)]
pub struct ProtonPage {
    pub lists: HashMap<Flavour, Result<Vec<Release>, String>>,
    pub loading: Option<Receiver<Listed>>,
    pub sel: usize,
    pub scroll: f32,
    /// Moved by keys: bring the choice into view once.
    pub follow: bool,
    pub job: Option<Job>,
}

#[derive(Clone, PartialEq)]
enum Row {
    Head(String),
    Tool(Tool),
    Release(Release),
    Official(Tool),
}

impl Row {
    fn selectable(&self) -> bool {
        !matches!(self, Row::Head(_))
    }
}

/// How many releases of each kind to offer.
const SHOWN: usize = 6;

fn installed_release(tools: &[Tool], r: &Release) -> bool {
    tools.iter().any(|t| {
        !t.official
            && (t.name.contains(&r.tag)
                || t.dir
                    .as_ref()
                    .and_then(|d| d.file_name())
                    .is_some_and(|n| n.to_string_lossy().contains(&r.tag)))
    })
}

impl App {
    fn proton_rows(&self) -> Vec<Row> {
        let tools = &self.steam.tools;
        let mut rows = vec![Row::Head(tr("pu_installed").into())];
        rows.extend(tools.iter().filter(|t| t.installed).cloned().map(Row::Tool));
        for f in Flavour::ALL {
            rows.push(Row::Head(f.name().into()));
            if let Some(Ok(list)) = self.proton.lists.get(&f) {
                rows.extend(list.iter().take(SHOWN).cloned().map(Row::Release));
            }
        }
        let missing: Vec<Row> = tools
            .iter()
            .filter(|t| t.official && !t.installed && t.appid.is_some())
            .cloned()
            .map(Row::Official)
            .collect();
        if !missing.is_empty() {
            rows.push(Row::Head(tr("pu_official").into()));
            rows.extend(missing);
        }
        rows
    }

    fn proton_load(&mut self) {
        if self.proton.loading.is_some() || !self.proton.lists.is_empty() {
            return;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        let ctx = self.ctx.clone();
        for f in Flavour::ALL {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send((f, protonup::releases(f)));
                ctx.request_repaint();
            });
        }
        self.proton.loading = Some(rx);
    }

    fn proton_pump(&mut self) {
        if let Some(rx) = &self.proton.loading {
            while let Ok((f, r)) = rx.try_recv() {
                self.proton.lists.insert(f, r);
            }
            if self.proton.lists.len() == Flavour::ALL.len() {
                self.proton.loading = None;
            }
        }
        let done = self
            .proton
            .job
            .as_ref()
            .and_then(|j| j.done.try_recv().ok());
        if let Some(r) = done {
            self.proton.job = None;
            match r {
                Ok(m) => self.toast(m),
                Err(e) => self.toast(format!("{}: {e}", tr("sd_failed"))),
            }
            self.refresh_tools();
        }
    }

    /// Read the tools again (after an install or removal).
    pub fn refresh_tools(&mut self) {
        self.steam.tools = steamplay::tools();
    }

    fn proton_install(&mut self, r: Release) {
        if self.proton.job.is_some() {
            self.toast(tr("sd_busy"));
            return;
        }
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (p, c, ctx) = (progress.clone(), cancel.clone(), self.ctx.clone());
        let tag = r.tag.clone();
        std::thread::spawn(move || {
            let res = protonup::install(&r, &c, &|pr| {
                if let Ok(mut g) = p.lock() {
                    *g = pr;
                }
                ctx.request_repaint_after(Duration::from_millis(200));
            })
            .map(|dir| {
                format!(
                    "{} {} — {}",
                    tr("pu_done"),
                    dir.file_name().unwrap_or_default().to_string_lossy(),
                    tr("pu_restart_note")
                )
            });
            let _ = tx.send(res);
            ctx.request_repaint();
        });
        self.proton.job = Some(Job {
            tag,
            progress,
            cancel,
            done: rx,
        });
    }

    fn proton_act(&mut self, row: &Row) {
        match row {
            Row::Head(_) => {}
            Row::Tool(t) if t.official => {
                let Some(id) = t.appid else { return };
                self.ask(
                    format!("{} {}?", tr("pu_remove"), t.display),
                    vec![tr("pu_remove_steam").into()],
                    tr("pu_remove_short"),
                    move |app| {
                        let _ =
                            crate::game::run_first(&crate::sources::steam::steam_run(&[&format!(
                                "steam://uninstall/{id}"
                            )]));
                        app.toast(tr("pu_steam_asked"));
                    },
                );
            }
            Row::Tool(t) if t.removable => {
                let Some(dir) = t.dir.clone() else { return };
                let users = self
                    .steam
                    .mapping
                    .values()
                    .filter(|n| **n == t.name)
                    .count();
                let mut lines = vec![dir.display().to_string()];
                if users > 0 {
                    lines.push(format!(
                        "{} {users} — {}",
                        tr("pu_used_by"),
                        tr("pu_used_note")
                    ));
                }
                self.ask(
                    format!("{} {}?", tr("pu_remove"), t.display),
                    lines,
                    tr("pu_remove_short"),
                    move |app| match protonup::remove(&dir) {
                        Ok(()) => {
                            app.toast(tr("pu_removed"));
                            app.refresh_tools();
                        }
                        Err(e) => app.toast(format!("{}: {e}", tr("sd_failed"))),
                    },
                );
            }
            Row::Tool(_) => self.toast(tr("pu_system")),
            Row::Release(r) => {
                if let Some(j) = &self.proton.job
                    && j.tag == r.tag
                {
                    let cancel = j.cancel.clone();
                    self.ask(
                        format!("{} {}?", tr("pu_cancel"), r.tag),
                        vec![],
                        tr("pu_cancel"),
                        move |_| cancel.store(true, Ordering::Relaxed),
                    );
                } else if installed_release(&self.steam.tools, r) {
                    self.toast(tr("pu_have"));
                } else {
                    self.proton_install(r.clone());
                }
            }
            Row::Official(t) => {
                if let Some(id) = t.appid {
                    let _ = crate::game::run_first(&crate::sources::steam::steam_run(&[&format!(
                        "steam://install/{id}"
                    )]));
                    self.toast(tr("pu_steam_asked"));
                }
            }
        }
    }

    /// Keys on the page; true on Back.
    pub fn proton_input(&mut self, a: Action) -> bool {
        let rows = self.proton_rows();
        let picks: Vec<usize> = (0..rows.len()).filter(|i| rows[*i].selectable()).collect();
        let at = picks
            .iter()
            .position(|i| *i == self.proton.sel)
            .unwrap_or(0);
        self.proton.follow = true;
        match a {
            Action::Up => self.proton.sel = picks[at.saturating_sub(1)],
            Action::Down => self.proton.sel = picks[(at + 1).min(picks.len().saturating_sub(1))],
            Action::Accept => {
                if let Some(r) = rows.get(self.proton.sel).cloned() {
                    self.proton_act(&r);
                }
            }
            Action::Back => return true,
            _ => {}
        }
        false
    }

    pub fn draw_proton(&mut self, ui: &mut Ui, rect: Rect) {
        self.proton_load();
        self.proton_pump();
        let rect = rect.shrink2(vec2(36.0, 0.0));
        let p = ui.painter_at(rect.expand2(vec2(8.0, 0.0)));
        let (c1, c2) = self.accent;
        p.text(
            pos2(rect.left(), rect.top() + 34.0),
            Align2::LEFT_CENTER,
            tr("proton"),
            FontId::proportional(32.0),
            Color32::WHITE,
        );
        let rows = self.proton_rows();
        if !rows.get(self.proton.sel).is_some_and(Row::selectable) {
            self.proton.sel = rows.iter().position(Row::selectable).unwrap_or(0);
        }
        let view = Rect::from_min_max(pos2(rect.left(), rect.top() + 76.0), rect.max);
        let w = view.width().min(1000.0);
        // Where each row sits.
        let mut tops = Vec::with_capacity(rows.len());
        let mut y = 0.0;
        for r in &rows {
            tops.push(y);
            y += if matches!(r, Row::Head(_)) {
                44.0
            } else {
                62.0
            };
        }
        let total = y;
        // Keep the chosen row in view (after a key, not against the wheel).
        let sel_top = tops.get(self.proton.sel).copied().unwrap_or(0.0);
        if !std::mem::take(&mut self.proton.follow) {
        } else if sel_top < self.proton.scroll + 40.0 {
            self.proton.scroll = (sel_top - 60.0).max(0.0);
        } else if sel_top + 62.0 > self.proton.scroll + view.height() {
            self.proton.scroll = sel_top + 62.0 - view.height();
        }
        if ui.rect_contains_pointer(view) {
            let d = ui.input(|i| i.smooth_scroll_delta.y);
            self.proton.scroll =
                (self.proton.scroll - d).clamp(0.0, (total - view.height()).max(0.0));
        }
        let scroll = ui.ctx().animate_value_with_time(
            ui.id().with("proton_scroll"),
            self.proton.scroll,
            0.15,
        );
        let clip = ui.painter_at(view.expand2(vec2(8.0, 0.0)));
        let job = self.proton.job.as_ref().map(|j| {
            (
                j.tag.clone(),
                j.progress.lock().map(|g| *g).unwrap_or_default(),
            )
        });
        if job.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        let mut clicked = None;
        for (i, row) in rows.iter().enumerate() {
            let top = view.top() + tops[i] - scroll;
            if let Row::Head(h) = row {
                clip.text(
                    pos2(view.left(), top + 22.0),
                    Align2::LEFT_CENTER,
                    h,
                    FontId::proportional(15.0),
                    c1,
                );
                if *h == Flavour::Ge.name()
                    && self
                        .proton
                        .lists
                        .get(&Flavour::Ge)
                        .is_some_and(|r| r.is_err())
                {
                    clip.text(
                        pos2(view.left() + 140.0, top + 22.0),
                        Align2::LEFT_CENTER,
                        tr("pu_no_list"),
                        FontId::proportional(13.0),
                        Color32::from_gray(150),
                    );
                }
                continue;
            }
            let r = Rect::from_min_size(pos2(view.left(), top), vec2(w, 54.0));
            if r.bottom() < view.top() || r.top() > view.bottom() {
                continue;
            }
            let resp = ui.interact(
                r.intersect(view),
                ui.id().with(("proton", i)),
                Sense::click(),
            );
            let focused = i == self.proton.sel && !self.side_focus;
            clip.rect_filled(
                r,
                12.0,
                if resp.hovered() {
                    Color32::from_white_alpha(20)
                } else {
                    Color32::from_black_alpha(90)
                },
            );
            if focused {
                clip.rect_stroke(r, 12.0, Stroke::new(2.5, c1), StrokeKind::Inside);
            }
            let (title, sub, state): (String, String, String) = match row {
                Row::Tool(t) => {
                    let users = self
                        .steam
                        .mapping
                        .values()
                        .filter(|n| **n == t.name)
                        .count();
                    let kind = if t.official {
                        tr("pu_kind_steam")
                    } else if t.removable {
                        tr("pu_kind_own")
                    } else {
                        tr("pu_kind_system")
                    };
                    let used = if users > 0 {
                        format!("  ·  {} {users}", tr("pu_used_by"))
                    } else {
                        String::new()
                    };
                    (
                        t.display.clone(),
                        format!("{kind}{used}"),
                        if t.official || t.removable {
                            tr("pu_remove_short").into()
                        } else {
                            String::new()
                        },
                    )
                }
                Row::Release(rel) => {
                    let have = installed_release(&self.steam.tools, rel);
                    let size = if rel.size > 0 {
                        format!("  ·  {}", crate::maint::human(rel.size))
                    } else {
                        String::new()
                    };
                    let newest = self
                        .proton
                        .lists
                        .get(&rel.flavour)
                        .and_then(|l| l.as_ref().ok())
                        .and_then(|l| l.first())
                        .is_some_and(|f| f.tag == rel.tag);
                    let state = match &job {
                        Some((tag, pr)) if *tag == rel.tag => {
                            if pr.unpacking {
                                tr("pu_unpacking").to_string()
                            } else if pr.total > 0 {
                                format!("{:.0}%", pr.done as f32 * 100.0 / pr.total as f32)
                            } else {
                                crate::maint::human(pr.done)
                            }
                        }
                        _ if have => tr("pu_installed_one").into(),
                        _ => tr("pu_install").into(),
                    };
                    (
                        format!(
                            "{}{}",
                            rel.tag,
                            if newest && !have {
                                format!("  ·  {}", tr("pu_new"))
                            } else {
                                String::new()
                            }
                        ),
                        match &job {
                            // While it downloads: how far along.
                            Some((tag, pr)) if *tag == rel.tag && !pr.unpacking => format!(
                                "{} {} {} {}",
                                tr("pu_got"),
                                crate::maint::human(pr.done),
                                tr("pu_of"),
                                crate::maint::human(pr.total.max(rel.size))
                            ),
                            _ => {
                                format!("{}{size}", crate::game::date(rel.published.max(0) as u64))
                            }
                        },
                        state,
                    )
                }
                Row::Official(t) => (
                    t.display.clone(),
                    tr("pu_kind_steam").into(),
                    tr("pu_install_steam").into(),
                ),
                Row::Head(_) => unreachable!(),
            };
            // A downloading release's bar, on the right beside its percent
            // (clear of the text).
            if let (Row::Release(rel), Some((tag, pr))) = (row, &job)
                && *tag == rel.tag
                && pr.total > 0
            {
                let bar = Rect::from_min_max(
                    pos2(r.right() - 300.0, r.center().y - 3.0),
                    pos2(r.right() - 90.0, r.center().y + 3.0),
                );
                crate::ui::widgets::bar(&clip, bar, pr.done as f32 / pr.total as f32, c1, c2);
            }
            clip.text(
                r.left_top() + vec2(18.0, 8.0),
                Align2::LEFT_TOP,
                title,
                FontId::proportional(17.0),
                Color32::WHITE,
            );
            clip.text(
                r.left_top() + vec2(18.0, 31.0),
                Align2::LEFT_TOP,
                sub,
                FontId::proportional(13.0),
                Color32::from_gray(150),
            );
            clip.text(
                r.right_center() - vec2(18.0, 0.0),
                Align2::RIGHT_CENTER,
                state,
                FontId::proportional(15.0),
                Color32::from_gray(205),
            );
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            self.side_focus = false;
            self.proton.sel = i;
            self.proton_act(&rows[i]);
        }
    }
}
