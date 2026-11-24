//! Installing, moving and uninstalling games from the launcher: Steam's
//! libraries (pick one, or a new one on a drive), the job running in the
//! background with its card in the corner, Steam closed only for the
//! moment its files are written and started again in the tray.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Receiver;
use egui::{
    Align2, Color32, FontId, Id, LayerId, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::app::App;
use crate::game::Source;
use crate::i18n::tr;
use crate::input::Action;
use crate::maint::human;
use crate::sources::steamplay::steam_running;
use crate::steamlib::{self, Place, Progress};

/// What the list of places is for.
#[derive(Clone, Debug, PartialEq)]
pub enum For {
    /// Settings → Steam libraries: what there is, and new ones.
    Libraries,
    Install {
        key: String,
        appid: u32,
        name: String,
    },
    Move {
        key: String,
        appid: u32,
        name: String,
        from: PathBuf,
        bytes: u64,
    },
    /// Every game of a library.
    MoveAll { from: PathBuf },
}

#[derive(Clone, Debug)]
pub struct Opt {
    pub place: Place,
    pub title: String,
    pub sub: String,
    pub on: bool,
    /// Games in it (a library).
    pub games: usize,
}

pub struct Pick {
    pub what: For,
    pub sel: usize,
    pub opts: Vec<Opt>,
}

pub struct Job {
    pub key: Option<String>,
    pub label: String,
    pub progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
    pub done: Receiver<Result<String, String>>,
    /// Run when it's done (with how it went).
    pub then: Option<Then>,
}

pub type Then = Box<dyn FnOnce(&mut App, &Result<String, String>)>;

#[derive(Default)]
pub struct Manage {
    pub pick: Option<Pick>,
    pub job: Option<Job>,
}

fn dry() -> bool {
    std::env::var_os("LENTA_DRY_RUN").is_some()
}

use crate::steamlib::steam_closed;

fn gb(b: u64) -> String {
    human(b)
}

impl App {
    fn places(&self, what: &For) -> Vec<Opt> {
        let Some(root) = steamlib::root() else {
            return Vec::new();
        };
        let libs = steamlib::libraries(&root);
        let need = match what {
            For::Move { bytes, .. } => *bytes,
            _ => 0,
        };
        let from = match what {
            For::Move { from, .. } | For::MoveAll { from } => Some(from.clone()),
            _ => None,
        };
        let mut out: Vec<Opt> = libs
            .iter()
            .filter(|l| Some(&l.path) != from.as_ref())
            .map(|l| Opt {
                place: Place::Lib(l.path.clone()),
                title: l.path.display().to_string(),
                sub: format!(
                    "{} {} {} {}  ·  {}: {}{}",
                    tr("mg_free"),
                    gb(l.free),
                    tr("mg_of"),
                    gb(l.total),
                    tr("mg_games"),
                    l.games,
                    steamlib::fs_of(&l.path)
                        .filter(|f| steamlib::plain_fs(f))
                        .map(|f| format!("  ·  {f}: {}", tr("mg_plain_fs")))
                        .unwrap_or_default()
                ),
                on: l.free > need,
                games: l.games,
            })
            .collect();
        for d in steamlib::drives(&libs) {
            out.push(Opt {
                title: format!("{}: {}", tr("mg_new_lib"), d.folder.display()),
                sub: format!(
                    "{} {} {} {}  ·  {}",
                    tr("mg_free"),
                    gb(d.free),
                    tr("mg_of"),
                    gb(d.total),
                    d.fs
                ),
                on: d.free > need,
                place: Place::New(d.folder),
                games: 0,
            });
        }
        out
    }

    fn open_pick(&mut self, what: For) {
        let opts = self.places(&what);
        if opts.is_empty() {
            self.toast(tr("mg_no_places"));
            return;
        }
        let sel = opts.iter().position(|o| o.on).unwrap_or(0);
        self.manage.pick = Some(Pick { what, sel, opts });
    }

    pub(crate) fn busy(&mut self) -> bool {
        if let Some(j) = &self.manage.job {
            let l = j.label.clone();
            self.toast(format!("{}: {l}", tr("mg_busy")));
            return true;
        }
        false
    }

    /// Settings → Steam libraries.
    pub fn open_libraries(&mut self) {
        self.open_pick(For::Libraries);
    }

    /// Play on a Steam game that isn't installed: where to.
    pub fn manage_install(&mut self, key: &str) {
        let Some(g) = self.game_by_key(key) else {
            return;
        };
        let (Some(appid), name) = (g.steam_appid, g.title.clone()) else {
            return;
        };
        if self.busy() {
            return;
        }
        self.open_pick(For::Install {
            key: key.to_string(),
            appid,
            name,
        });
    }

    pub fn can_move(&self, key: &str) -> bool {
        self.game_by_key(key)
            .is_some_and(|g| g.installed && g.key.starts_with("steam:") && g.steam_appid.is_some())
    }

    pub fn can_uninstall(&self, key: &str) -> bool {
        self.game_by_key(key).is_some_and(|g| {
            g.installed
                && match g.source {
                    Source::Steam => g.key.starts_with("steam:") && g.steam_appid.is_some(),
                    Source::Heroic | Source::Lutris | Source::Hydra | Source::Own => true,
                }
        })
    }

    pub fn manage_move(&mut self, key: &str) {
        let Some(g) = self.game_by_key(key) else {
            return;
        };
        let (Some(appid), name) = (g.steam_appid, g.title.clone()) else {
            return;
        };
        if self.busy() {
            return;
        }
        if self.is_running(key) {
            self.toast(tr("mg_close_game"));
            return;
        }
        let Some(root) = steamlib::root() else { return };
        let Some((from, acf)) = steamlib::library_of(&root, appid) else {
            return;
        };
        let bytes = steamlib::size_on_disk(&acf);
        self.open_pick(For::Move {
            key: key.to_string(),
            appid,
            name,
            from,
            bytes,
        });
    }

    pub fn manage_uninstall(&mut self, key: &str) {
        let Some(g) = self.game_by_key(key).cloned() else {
            return;
        };
        if self.busy() {
            return;
        }
        if self.is_running(key) {
            self.toast(tr("mg_close_game"));
            return;
        }
        let title = format!("{} «{}»?", tr("mg_uninstall_q"), g.title);
        match g.source {
            Source::Steam => {
                let Some(appid) = g.steam_appid else { return };
                let Some(root) = steamlib::root() else { return };
                let (_, _, bytes) = match steamlib::uninstall_plan(&root, appid) {
                    Ok(p) => p,
                    Err(e) => {
                        self.toast(format!("{}: {e}", tr("mg_failed")));
                        return;
                    }
                };
                let mut lines = vec![format!("{} {}", tr("mg_frees"), gb(bytes))];
                if let Some(p) = crate::maint::prefix(&g) {
                    lines.push(format!("{}: {}", tr("mg_prefix_kept"), p.display()));
                }
                if steam_running() {
                    lines.push(tr("mg_steam_restart").to_string());
                }
                let label = format!("{} «{}»", tr("mg_uninstalling"), g.title);
                let key = g.key.clone();
                self.ask(title, lines, tr("mg_uninstall"), move |app| {
                    app.start_manage(Some(key), label, move |_, _| {
                        let freed = steam_closed(false, || steamlib::uninstall(&root, appid))?;
                        Ok(format!("{} {}", tr("mg_freed"), gb(freed)))
                    });
                });
            }
            Source::Heroic | Source::Lutris | Source::Hydra | Source::Own => {
                let plan = match crate::uninstall::plan(&g) {
                    Ok(p) => p,
                    Err(e) => {
                        self.toast(format!("{}: {e}", tr("mg_failed")));
                        return;
                    }
                };
                let mut lines = Vec::new();
                let gone: Vec<&std::path::PathBuf> = plan.delete.iter().chain(&plan.also).collect();
                if gone.is_empty() {
                    lines.push(tr("mg_list_only").to_string());
                } else {
                    lines.push(format!("{} {}", tr("mg_frees"), gb(plan.bytes)));
                    for d in gone {
                        lines.push(format!("{}: {}", tr("mg_deleted"), d.display()));
                    }
                }
                for k in &plan.kept {
                    lines.push(format!("{}: {}", tr("mg_kept"), k.display()));
                }
                // The launcher closes for it; Hydra lives in the tray, so it
                // comes back there.
                let up = plan.close.filter(|(_, p)| crate::running::named(p));
                if let Some((name, _)) = up {
                    lines.push(format!("{} {name}.", tr("mg_launcher_close")));
                }
                let label = format!("{} «{}»", tr("mg_uninstalling"), g.title);
                let key = g.key.clone();
                self.ask(title, lines, tr("mg_uninstall"), move |app| {
                    app.start_manage(Some(key), label, move |_, _| {
                        if let Some((name, p)) = up
                            && !crate::running::close_named(p, 15)
                        {
                            return Err(format!("{} {name}", tr("mg_launcher_stuck")));
                        }
                        let freed = crate::uninstall::run(&g, &plan);
                        if let Some(("Hydra", p)) = up {
                            let _ =
                                crate::game::run_first(&[vec![p.to_string(), "--hidden".into()]]);
                        }
                        let freed = freed?;
                        Ok(if plan.delete.is_some() || !plan.also.is_empty() {
                            format!("{} {}", tr("mg_freed"), gb(freed))
                        } else {
                            tr("mg_removed_from_list").to_string()
                        })
                    });
                });
            }
        }
    }

    /// The job on its thread. Nothing is done under the dry run.
    fn start_manage(
        &mut self,
        key: Option<String>,
        label: String,
        job: impl FnOnce(&(dyn Fn(Progress) + Sync), &AtomicBool) -> Result<String, String>
        + Send
        + 'static,
    ) {
        self.start_job(key, label, job, |_, _| {});
    }

    /// A job on its thread, its card in the corner; `then` on this side
    /// when it's done. Nothing is done under the dry run.
    pub fn start_job(
        &mut self,
        key: Option<String>,
        label: String,
        job: impl FnOnce(&(dyn Fn(Progress) + Sync), &AtomicBool) -> Result<String, String>
        + Send
        + 'static,
        then: impl FnOnce(&mut App, &Result<String, String>) + 'static,
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
                ctx.request_repaint_after(Duration::from_millis(250));
            };
            let r = if dry() {
                Ok(tr("mg_dry").to_string())
            } else {
                job(&report, &c)
            };
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.manage.job = Some(Job {
            key,
            label,
            progress,
            cancel,
            done: rx,
            then: Some(Box::new(then)),
        });
    }

    /// Move these games (all from one library) to `to`: copied first with
    /// Steam running, then Steam closed for a moment to switch them over.
    fn start_move(&mut self, key: Option<String>, label: String, apps: Vec<u32>, to: Place) {
        let Some(root) = steamlib::root() else { return };
        self.start_manage(key, label, move |report, cancel| {
            let (n, skipped) =
                steamlib::move_games(&root, &apps, &to, report, cancel).map_err(|e| {
                    match e.as_str() {
                        steamlib::CANCELLED => tr("mg_cancelled").to_string(),
                        steamlib::NO_LINKS => tr("mg_no_links").to_string(),
                        _ => e,
                    }
                })?;
            let mut msg = format!("{}: {n}", tr("mg_moved"));
            if skipped > 0 {
                msg += &format!(" · {}: {skipped}", tr("mg_skipped"));
            }
            Ok(msg)
        });
    }

    /// A on a place.
    fn pick_place(&mut self, i: usize) {
        let Some(pick) = &self.manage.pick else {
            return;
        };
        let Some(opt) = pick.opts.get(i).cloned() else {
            return;
        };
        let what = pick.what.clone();
        if !opt.on {
            self.toast(tr("mg_no_room"));
            return;
        }
        let Some(root) = steamlib::root() else { return };
        let restart = steam_running();
        let path = opt.place.path().display().to_string();
        match what {
            For::Libraries => match &opt.place {
                Place::Lib(p) => {
                    if opt.games == 0 {
                        self.toast(tr("mg_lib_empty"));
                        return;
                    }
                    let from = p.clone();
                    self.open_pick(For::MoveAll { from });
                }
                Place::New(d) => {
                    let d = d.clone();
                    let mut lines = Vec::new();
                    if restart {
                        lines.push(tr("mg_steam_restart").to_string());
                    }
                    self.manage.pick = None;
                    self.ask(
                        format!("{} {path}?", tr("mg_add_lib_q")),
                        lines,
                        tr("mg_create"),
                        move |app| {
                            app.start_manage(None, tr("mg_adding_lib").to_string(), move |_, _| {
                                steam_closed(false, || steamlib::add_library(&root, &d))?;
                                Ok(tr("mg_lib_added").to_string())
                            });
                        },
                    );
                }
            },
            For::Install { key, appid, name } => {
                let mut lines = vec![tr("mg_install_note").to_string()];
                if restart {
                    lines.push(tr("mg_steam_restart").to_string());
                }
                let place = opt.place.clone();
                let label = format!("{} «{name}»", tr("mg_installing"));
                self.manage.pick = None;
                self.ask(
                    format!("{} «{name}» → {path}?", tr("mg_install_q")),
                    lines,
                    tr("install"),
                    move |app| {
                        app.start_manage(Some(key), label, move |_, _| {
                            steam_closed(true, || steamlib::install(&root, &place, appid, &name))?;
                            Ok(tr("mg_install_started").to_string())
                        });
                    },
                );
            }
            For::Move {
                key,
                appid,
                name,
                bytes,
                ..
            } => {
                let mut lines = vec![format!("{}: {}", tr("mg_size"), gb(bytes))];
                lines.push(tr("mg_move_note").to_string());
                if restart {
                    lines.push(tr("mg_steam_restart_end").to_string());
                }
                let place = opt.place.clone();
                let label = format!("{} «{name}»", tr("mg_moving"));
                self.manage.pick = None;
                self.ask(
                    format!("{} «{name}» → {path}?", tr("mg_move_q")),
                    lines,
                    tr("mg_move"),
                    move |app| app.start_move(Some(key), label, vec![appid], place),
                );
            }
            For::MoveAll { from } => {
                let apps: Vec<u32> = self
                    .games
                    .iter()
                    .filter(|g| g.installed && g.key.starts_with("steam:"))
                    .filter_map(|g| g.steam_appid)
                    .filter(|id| steamlib::library_of(&root, *id).is_some_and(|(l, _)| l == from))
                    .collect();
                if apps.is_empty() {
                    self.toast(tr("mg_lib_empty"));
                    return;
                }
                let mut lines = vec![format!("{}: {}", tr("mg_games"), apps.len())];
                lines.push(tr("mg_move_note").to_string());
                if restart {
                    lines.push(tr("mg_steam_restart_end").to_string());
                }
                let place = opt.place.clone();
                let label = tr("mg_moving_lib").to_string();
                self.manage.pick = None;
                self.ask(
                    format!("{} {} → {path}?", tr("mg_move_all_q"), from.display()),
                    lines,
                    tr("mg_move"),
                    move |app| app.start_move(None, label, apps, place),
                );
            }
        }
    }

    pub fn cancel_manage(&mut self) {
        if let Some(j) = &self.manage.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The job's end: say so, look at the games again.
    pub fn pump_manage(&mut self) {
        let Some(r) = self
            .manage
            .job
            .as_ref()
            .and_then(|j| j.done.try_recv().ok())
        else {
            return;
        };
        let (label, then) = self
            .manage
            .job
            .take()
            .map(|j| (j.label, j.then))
            .unwrap_or_default();
        match &r {
            Ok(msg) => self.toast(msg.clone()),
            Err(e) => self.toast(format!("{label}: {e}")),
        }
        if let Some(f) = then {
            f(self, &r);
        }
        self.rescan();
    }

    pub fn manage_input(&mut self, a: Action) {
        let Some(p) = &mut self.manage.pick else {
            return;
        };
        let n = p.opts.len();
        match a {
            Action::Up => p.sel = p.sel.saturating_sub(1),
            Action::Down => p.sel = (p.sel + 1).min(n.saturating_sub(1)),
            Action::Accept => {
                let i = p.sel;
                self.pick_place(i);
            }
            Action::Back => {
                // From a library's "move all" back to the libraries.
                let back = matches!(p.what, For::MoveAll { .. });
                self.manage.pick = None;
                if back {
                    self.open_libraries();
                }
            }
            _ => {}
        }
    }

    pub fn draw_manage_pick(&mut self, ui: &mut Ui, area: Rect) {
        let Some(pick) = &self.manage.pick else {
            return;
        };
        let layer = LayerId::new(Order::Foreground, Id::new("manage_pick"));
        let p = ui.ctx().layer_painter(layer);
        p.rect_filled(area, 0.0, Color32::from_black_alpha(195));
        let (c1, _) = self.accent;
        let title = match &pick.what {
            For::Libraries => tr("mg_libs").to_string(),
            For::Install { name, .. } => format!("{} «{name}»", tr("mg_where_install")),
            For::Move { name, .. } => format!("{} «{name}»", tr("mg_where_move")),
            For::MoveAll { from } => format!("{} {}", tr("mg_where_move_all"), from.display()),
        };
        let note = match &pick.what {
            For::Libraries => tr("mg_libs_note"),
            _ => "",
        };
        let row_h = 66.0;
        let w = 780.0f32.min(area.width() - 32.0);
        let h = (26.0
            + 40.0
            + if note.is_empty() { 0.0 } else { 30.0 }
            + pick.opts.len() as f32 * row_h
            + 18.0)
            .min(area.height() - 40.0);
        let card = Rect::from_center_size(area.center(), vec2(w, h));
        p.rect_filled(card, 20.0, Color32::from_rgb(24, 27, 36));
        let left = card.left() + 28.0;
        let mut y = card.top() + 26.0;
        p.text(
            pos2(left, y),
            Align2::LEFT_TOP,
            title,
            FontId::proportional(23.0),
            Color32::WHITE,
        );
        y += 40.0;
        if !note.is_empty() {
            p.text(
                pos2(left, y),
                Align2::LEFT_TOP,
                note,
                FontId::proportional(14.0),
                Color32::from_gray(170),
            );
            y += 30.0;
        }
        let top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(area));
        let veil = top.interact(area, Id::new("mg_veil"), Sense::click());
        let mut clicked = None;
        for (i, o) in pick.opts.iter().enumerate() {
            let r = Rect::from_min_size(pos2(left, y), vec2(w - 56.0, row_h - 8.0));
            y += row_h;
            if r.bottom() > card.bottom() {
                break;
            }
            let resp = top.interact(r, Id::new(("mg_opt", i)), Sense::click());
            p.rect_filled(
                r,
                12.0,
                Color32::from_white_alpha(if resp.hovered() { 24 } else { 12 }),
            );
            if pick.sel == i {
                p.rect_stroke(r, 12.0, Stroke::new(2.5, c1), StrokeKind::Inside);
            }
            let col = if o.on {
                Color32::WHITE
            } else {
                Color32::from_gray(120)
            };
            let clip = p.with_clip_rect(r.shrink(4.0));
            clip.text(
                r.left_top() + vec2(16.0, 9.0),
                Align2::LEFT_TOP,
                &o.title,
                FontId::proportional(17.0),
                col,
            );
            clip.text(
                r.left_top() + vec2(16.0, 34.0),
                Align2::LEFT_TOP,
                &o.sub,
                FontId::proportional(13.0),
                Color32::from_gray(if o.on { 160 } else { 110 }),
            );
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            if let Some(p) = &mut self.manage.pick {
                p.sel = i;
            }
            self.pick_place(i);
        } else if veil.clicked() {
            self.manage.pick = None;
        }
    }

    /// The running job, in the corner above the hints.
    pub fn draw_manage_job(&mut self, ui: &mut Ui, full: Rect) {
        self.pump_manage();
        let Some(j) = &self.manage.job else { return };
        ui.ctx().request_repaint_after(Duration::from_millis(400));
        let pr = j.progress.lock().map(|g| g.clone()).unwrap_or_default();
        let p = ui
            .ctx()
            .layer_painter(LayerId::new(Order::Foreground, Id::new("manage_job")));
        let r = Rect::from_min_size(full.right_bottom() - vec2(460.0, 150.0), vec2(436.0, 70.0));
        p.rect_filled(r, 14.0, Color32::from_rgba_unmultiplied(20, 22, 30, 235));
        let (c1, c2) = self.accent;
        let clip = p.with_clip_rect(r.shrink(6.0));
        clip.text(
            r.left_top() + vec2(16.0, 12.0),
            Align2::LEFT_TOP,
            &j.label,
            FontId::proportional(15.0),
            Color32::WHITE,
        );
        let mut line = if pr.stage.is_empty() {
            tr("mg_working").to_string()
        } else {
            tr(pr.stage).to_string()
        };
        if pr.total > 0 {
            let f = pr.done as f32 / pr.total as f32;
            line += &format!(" · {:.0}% · {} / {}", f * 100.0, gb(pr.done), gb(pr.total));
            let bar = Rect::from_min_max(
                pos2(r.left() + 16.0, r.bottom() - 12.0),
                pos2(r.right() - 16.0, r.bottom() - 7.0),
            );
            crate::ui::widgets::bar(&p, bar, f.clamp(0.0, 1.0), c1, c2);
        }
        clip.text(
            r.left_top() + vec2(16.0, 34.0),
            Align2::LEFT_TOP,
            line,
            FontId::proportional(13.0),
            Color32::from_gray(180),
        );
    }
}
