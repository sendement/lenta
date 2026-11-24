//! Two windows over everything, made for the pad: a list to choose from
//! (a prefix, a Proton, what to do), and a file browser in the manner of
//! Polka's — places on top (home, Downloads, Games, drives), folders, and
//! only the files asked for unless all are shown. The system's own file
//! window (through the portal) can stand in for the browser (Settings).

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use egui::{
    Align2, Color32, FontId, Id, LayerId, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::app::App;
use crate::i18n::tr;
use crate::input::Action;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub title: String,
    pub sub: String,
    pub on: bool,
}

impl Row {
    pub fn new(title: impl Into<String>, sub: impl Into<String>) -> Row {
        Row {
            title: title.into(),
            sub: sub.into(),
            on: true,
        }
    }
}

type Act = Box<dyn FnOnce(&mut App, usize)>;
type Then = Box<dyn FnOnce(&mut App)>;
type Picked = Box<dyn FnOnce(&mut App, PathBuf)>;

pub struct Choose {
    pub title: String,
    pub note: String,
    pub rows: Vec<Row>,
    pub sel: usize,
    pub scroll: f32,
    act: Option<Act>,
    back: Option<Then>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Place,
    Up,
    Dir,
    File,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub sub: String,
    pub path: PathBuf,
    pub kind: Kind,
}

pub struct Files {
    pub title: String,
    /// None: the places.
    pub dir: Option<PathBuf>,
    pub rows: Vec<Entry>,
    pub sel: usize,
    pub scroll: f32,
    /// Every file, not only the kinds asked for.
    pub all: bool,
    pub exts: Vec<String>,
    /// Where it was in each folder left, to come back to it.
    pub came_from: Vec<(PathBuf, usize)>,
    done: Option<Picked>,
}

/// The system's file window, running.
pub struct Portal {
    pub rx: Receiver<Option<PathBuf>>,
    done: Option<Picked>,
}

#[derive(Default)]
pub struct Modals {
    pub choose: Option<Choose>,
    pub files: Option<Files>,
    pub portal: Option<Portal>,
}

const ROW_H: f32 = 62.0;

fn human(b: u64) -> String {
    crate::maint::human(b)
}

/// Home, Downloads, Games folders and mounted drives.
pub fn places() -> Vec<Entry> {
    let mut out = Vec::new();
    let home = dirs::home_dir().unwrap_or_default();
    let mut add = |name: String, path: PathBuf| {
        if path.is_dir() && !out.iter().any(|e: &Entry| e.path == path) {
            out.push(Entry {
                sub: path.display().to_string(),
                name,
                path,
                kind: Kind::Place,
            });
        }
    };
    add(tr("fp_home").to_string(), home.clone());
    if let Some(d) = dirs::download_dir() {
        add(tr("fp_downloads").to_string(), d);
    }
    add("Games".into(), home.join("Games"));
    if let Some(d) = dirs::desktop_dir() {
        add(tr("fp_desktop").to_string(), d);
    }
    let user = std::env::var("USER").unwrap_or_default();
    for base in [format!("/run/media/{user}"), "/media".into(), "/mnt".into()] {
        if let Ok(rd) = std::fs::read_dir(&base) {
            let mut v: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            v.sort();
            for p in v {
                let n = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                add(n, p);
            }
        }
    }
    add(tr("fp_root").to_string(), PathBuf::from("/"));
    out
}

/// A folder's folders, then the files of the kinds asked for (or all);
/// hidden ones left out.
pub fn list(dir: &Path, exts: &[String], all: bool) -> Vec<Entry> {
    let mut dirs_v = Vec::new();
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                dirs_v.push(Entry {
                    name,
                    sub: String::new(),
                    path: p,
                    kind: Kind::Dir,
                });
            } else {
                let low = name.to_lowercase();
                if all || exts.iter().any(|x| low.ends_with(x.as_str())) {
                    let size = e.metadata().map_or(0, |m| m.len());
                    files.push(Entry {
                        name,
                        sub: human(size),
                        path: p,
                        kind: Kind::File,
                    });
                }
            }
        }
    }
    let key = |e: &Entry| e.name.to_lowercase();
    dirs_v.sort_by_key(key);
    files.sort_by_key(key);
    let mut out = vec![Entry {
        name: "..".into(),
        sub: dir
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| tr("fp_places").to_string()),
        path: dir.parent().map(Path::to_path_buf).unwrap_or_default(),
        kind: Kind::Up,
    }];
    out.extend(dirs_v);
    out.extend(files);
    out
}

impl App {
    /// A list to choose from; `act` gets the row chosen (an enabled one),
    /// B closes it (or goes back, with `choose_back`).
    pub fn choose(
        &mut self,
        title: impl Into<String>,
        note: impl Into<String>,
        rows: Vec<Row>,
        act: impl FnOnce(&mut App, usize) + 'static,
    ) {
        let sel = rows.iter().position(|r| r.on).unwrap_or(0);
        self.modals.choose = Some(Choose {
            title: title.into(),
            note: note.into(),
            rows,
            sel,
            scroll: 0.0,
            act: Some(Box::new(act)),
            back: None,
        });
    }

    /// What B does in the list open now (instead of only closing it).
    pub fn choose_back(&mut self, back: impl FnOnce(&mut App) + 'static) {
        if let Some(c) = &mut self.modals.choose {
            c.back = Some(Box::new(back));
        }
    }

    pub fn choose_input(&mut self, a: Action) {
        let Some(c) = &mut self.modals.choose else {
            return;
        };
        let n = c.rows.len();
        match a {
            Action::Up => c.sel = c.sel.saturating_sub(1),
            Action::Down => c.sel = (c.sel + 1).min(n.saturating_sub(1)),
            Action::Home => c.sel = 0,
            Action::End => c.sel = n.saturating_sub(1),
            Action::Accept => self.choose_pick(None),
            Action::Back => {
                if let Some(mut c) = self.modals.choose.take()
                    && let Some(b) = c.back.take()
                {
                    b(self);
                }
            }
            _ => {}
        }
    }

    fn choose_pick(&mut self, at: Option<usize>) {
        let Some(c) = &mut self.modals.choose else {
            return;
        };
        let i = at.unwrap_or(c.sel);
        if !c.rows.get(i).is_some_and(|r| r.on) {
            return;
        }
        let act = c.act.take();
        self.modals.choose = None;
        if let Some(f) = act {
            f(self, i);
        }
    }

    // ------------------------------------------------------------ files

    /// Pick a file (one of `exts`, ".exe"…), in the browser or the
    /// system's window as set; starts in `start` (or the places).
    pub fn pick_file(
        &mut self,
        title: impl Into<String>,
        exts: &[&str],
        start: Option<PathBuf>,
        done: impl FnOnce(&mut App, PathBuf) + 'static,
    ) {
        let title = title.into();
        let exts: Vec<String> = exts.iter().map(|e| e.to_lowercase()).collect();
        if self.cfg.file_portal && crate::sandbox::has_program("zenity") {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let filter = exts
                .iter()
                .map(|e| format!("*{e} *{}", e.to_uppercase()))
                .collect::<Vec<_>>()
                .join(" ");
            let start = start.clone();
            let ctx = self.ctx.clone();
            let t = title.clone();
            std::thread::spawn(move || {
                let mut c = std::process::Command::new("zenity");
                c.arg("--file-selection")
                    .arg(format!("--title={t}"))
                    .arg(format!("--file-filter={t} | {filter}"))
                    .arg("--file-filter=* | *")
                    .env("GTK_USE_PORTAL", "1");
                if let Some(s) = &start {
                    c.arg(format!("--filename={}/", s.display()));
                }
                let out = crate::sandbox::host(c).output().ok();
                let p = out
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from);
                let _ = tx.send(p);
                ctx.request_repaint();
            });
            self.modals.portal = Some(Portal {
                rx,
                done: Some(Box::new(done)),
            });
            return;
        }
        let mut f = Files {
            title,
            dir: None,
            rows: places(),
            sel: 0,
            scroll: 0.0,
            all: false,
            exts,
            came_from: Vec::new(),
            done: Some(Box::new(done)),
        };
        if let Some(s) = start.filter(|s| s.is_dir()) {
            f.rows = list(&s, &f.exts, false);
            f.dir = Some(s);
            f.sel = 0;
        }
        self.modals.files = Some(f);
    }

    /// The system's window answered.
    pub fn pump_portal(&mut self) {
        let Some(p) = &mut self.modals.portal else {
            return;
        };
        let Ok(r) = p.rx.try_recv() else { return };
        let done = p.done.take();
        self.modals.portal = None;
        if let (Some(path), Some(f)) = (r, done) {
            f(self, path);
        }
    }

    fn files_open(&mut self, dir: Option<PathBuf>, back: bool) {
        let Some(f) = &mut self.modals.files else {
            return;
        };
        if !back && let Some(d) = &f.dir {
            f.came_from.push((d.clone(), f.sel));
        }
        let old = f.dir.clone();
        f.rows = match &dir {
            Some(d) => list(d, &f.exts, f.all),
            None => places(),
        };
        f.dir = dir;
        f.scroll = 0.0;
        f.sel = 0;
        if back {
            // On the folder just left.
            if let Some(i) = old.and_then(|o| f.rows.iter().position(|e| e.path == o)) {
                f.sel = i;
            }
        } else if f.rows.len() > 1 && f.dir.is_some() {
            f.sel = 1;
        }
    }

    fn files_up(&mut self) {
        let Some(f) = &self.modals.files else { return };
        match &f.dir {
            None => self.modals.files = None,
            Some(d) => {
                let at_place = places().iter().any(|p| &p.path == d);
                let up = if at_place {
                    None
                } else {
                    d.parent().map(Path::to_path_buf)
                };
                self.files_open(up, true);
            }
        }
    }

    fn files_accept(&mut self, at: Option<usize>) {
        let Some(f) = &mut self.modals.files else {
            return;
        };
        let i = at.unwrap_or(f.sel);
        f.sel = i;
        let Some(e) = f.rows.get(i).cloned() else {
            return;
        };
        match e.kind {
            Kind::Up => self.files_up(),
            Kind::Place | Kind::Dir => self.files_open(Some(e.path), false),
            Kind::File => {
                let done = f.done.take();
                self.modals.files = None;
                if let Some(d) = done {
                    d(self, e.path);
                }
            }
        }
    }

    pub fn files_input(&mut self, a: Action) {
        let Some(f) = &mut self.modals.files else {
            return;
        };
        let n = f.rows.len();
        match a {
            Action::Up => f.sel = f.sel.saturating_sub(1),
            Action::Down => f.sel = (f.sel + 1).min(n.saturating_sub(1)),
            Action::Left | Action::PrevTab => f.sel = f.sel.saturating_sub(8),
            Action::Right | Action::NextTab => f.sel = (f.sel + 8).min(n.saturating_sub(1)),
            Action::Home => f.sel = 0,
            Action::End => f.sel = n.saturating_sub(1),
            Action::Accept => self.files_accept(None),
            Action::Back => self.files_up(),
            Action::Actions => {
                f.all = !f.all;
                let d = f.dir.clone();
                let sel = f.sel;
                if d.is_some() {
                    self.files_open(d, true);
                    if let Some(f) = &mut self.modals.files {
                        f.sel = sel.min(f.rows.len().saturating_sub(1));
                    }
                }
            }
            Action::Menu => self.modals.files = None,
            _ => {}
        }
    }

    // ------------------------------------------------------------ drawing

    /// A card with a title, a note and rows (title, under it a smaller
    /// line); the row chosen framed, kept in view. The row clicked.
    #[allow(clippy::too_many_arguments)]
    fn draw_rows(
        &self,
        ui: &mut Ui,
        area: Rect,
        id: &str,
        title: &str,
        note: &str,
        rows: &[(String, String, bool, &'static str)],
        sel: usize,
        scroll: &mut f32,
    ) -> (Option<usize>, bool) {
        let layer = LayerId::new(Order::Foreground, Id::new(id));
        let p = ui.ctx().layer_painter(layer);
        p.rect_filled(area, 0.0, Color32::from_black_alpha(195));
        let (c1, _) = self.accent;
        let w = 820.0f32.min(area.width() - 32.0);
        let head = 26.0 + 40.0 + if note.is_empty() { 0.0 } else { 30.0 };
        let h = (head + rows.len() as f32 * ROW_H + 18.0).min(area.height() - 40.0);
        let card = Rect::from_center_size(area.center(), vec2(w, h));
        p.rect_filled(card, 20.0, Color32::from_rgb(24, 27, 36));
        let left = card.left() + 28.0;
        let mut y = card.top() + 26.0;
        let tclip = p.with_clip_rect(Rect::from_min_max(
            pos2(left, y),
            pos2(card.right() - 20.0, y + 36.0),
        ));
        tclip.text(
            pos2(left, y),
            Align2::LEFT_TOP,
            title,
            FontId::proportional(23.0),
            Color32::WHITE,
        );
        y += 40.0;
        if !note.is_empty() {
            let nclip = p.with_clip_rect(Rect::from_min_max(
                pos2(left, y),
                pos2(card.right() - 20.0, y + 26.0),
            ));
            nclip.text(
                pos2(left, y),
                Align2::LEFT_TOP,
                note,
                FontId::proportional(14.0),
                Color32::from_gray(170),
            );
            y += 30.0;
        }
        let view = Rect::from_min_max(
            pos2(left, y),
            pos2(card.right() - 28.0, card.bottom() - 12.0),
        );
        // Keep the chosen row in view.
        let sel_top = sel as f32 * ROW_H;
        if sel_top < *scroll {
            *scroll = sel_top;
        }
        if sel_top + ROW_H > *scroll + view.height() {
            *scroll = sel_top + ROW_H - view.height();
        }
        let top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(area));
        let veil = top.interact(area, Id::new((id, "veil")), Sense::click());
        let wheel = if top.rect_contains_pointer(card) {
            top.input(|i| i.smooth_scroll_delta.y)
        } else {
            0.0
        };
        if wheel != 0.0 {
            let max = (rows.len() as f32 * ROW_H - view.height()).max(0.0);
            *scroll = (*scroll - wheel).clamp(0.0, max);
        }
        let clip = p.with_clip_rect(view);
        let mut clicked = None;
        for (i, (t, s, on, icon)) in rows.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(view.left(), view.top() + i as f32 * ROW_H - *scroll),
                vec2(view.width(), ROW_H - 8.0),
            );
            if r.bottom() < view.top() || r.top() > view.bottom() {
                continue;
            }
            let resp = top.interact(r.intersect(view), Id::new((id, i)), Sense::click());
            clip.rect_filled(
                r,
                12.0,
                Color32::from_white_alpha(if resp.hovered() { 24 } else { 12 }),
            );
            if sel == i {
                clip.rect_stroke(r, 12.0, Stroke::new(2.5, c1), StrokeKind::Inside);
            }
            let col = if *on {
                Color32::WHITE
            } else {
                Color32::from_gray(120)
            };
            let tx = if icon.is_empty() { 16.0 } else { 50.0 };
            if !icon.is_empty() {
                clip.text(
                    r.left_center() + vec2(16.0, 0.0),
                    Align2::LEFT_CENTER,
                    *icon,
                    FontId::proportional(20.0),
                    Color32::from_gray(if *on { 210 } else { 110 }),
                );
            }
            let rc = clip.with_clip_rect(r.shrink(4.0).intersect(view));
            let one_line = s.is_empty();
            rc.text(
                r.left_top() + vec2(tx, if one_line { 17.0 } else { 8.0 }),
                Align2::LEFT_TOP,
                t,
                FontId::proportional(17.0),
                col,
            );
            if !one_line {
                rc.text(
                    r.left_top() + vec2(tx, 32.0),
                    Align2::LEFT_TOP,
                    s,
                    FontId::proportional(13.0),
                    Color32::from_gray(if *on { 160 } else { 110 }),
                );
            }
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        (clicked, clicked.is_none() && veil.clicked())
    }

    pub fn draw_choose(&mut self, ui: &mut Ui, area: Rect) {
        let Some(c) = &self.modals.choose else { return };
        let rows: Vec<(String, String, bool, &'static str)> = c
            .rows
            .iter()
            .map(|r| (r.title.clone(), r.sub.clone(), r.on, ""))
            .collect();
        let (title, note, sel) = (c.title.clone(), c.note.clone(), c.sel);
        let mut scroll = c.scroll;
        let (clicked, away) =
            self.draw_rows(ui, area, "choose", &title, &note, &rows, sel, &mut scroll);
        if let Some(c) = &mut self.modals.choose {
            c.scroll = scroll;
        }
        if let Some(i) = clicked {
            if let Some(c) = &mut self.modals.choose {
                c.sel = i;
            }
            self.choose_pick(Some(i));
        } else if away {
            self.choose_input(Action::Back);
        }
    }

    pub fn draw_files(&mut self, ui: &mut Ui, area: Rect) {
        let Some(f) = &self.modals.files else { return };
        let rows: Vec<(String, String, bool, &'static str)> = f
            .rows
            .iter()
            .map(|e| {
                let icon = match e.kind {
                    Kind::Place => "◆",
                    Kind::Up => "↰",
                    Kind::Dir => "▸",
                    Kind::File => "•",
                };
                (e.name.clone(), e.sub.clone(), true, icon)
            })
            .collect();
        let note = match &f.dir {
            Some(d) => format!(
                "{}{}",
                d.display(),
                if f.all {
                    format!("   ·   {}", tr("fp_all_shown"))
                } else {
                    String::new()
                }
            ),
            None => tr("fp_places").to_string(),
        };
        let (title, sel) = (f.title.clone(), f.sel);
        let mut scroll = f.scroll;
        let (clicked, away) =
            self.draw_rows(ui, area, "files", &title, &note, &rows, sel, &mut scroll);
        if let Some(f) = &mut self.modals.files {
            f.scroll = scroll;
        }
        if let Some(i) = clicked {
            self.files_accept(Some(i));
        } else if away {
            self.modals.files = None;
        }
    }
}
