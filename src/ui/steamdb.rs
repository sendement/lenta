//! Versions that never were on this machine, from SteamDB: the user opens
//! the depot's manifests page (or the game's patch notes) in the browser,
//! copies the table and pastes it here (Ctrl+V, or the Paste button that
//! reads the clipboard). Saved to steam-downgrader's state, so the
//! versions stay in the list (and in steam-downgrader too).

use crossbeam_channel::Receiver;
use egui::{
    Align2, Color32, FontId, Id, LayerId, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::app::App;
use crate::downgrade::state::State;
use crate::downgrade::steamdb::{self, Build, Entry};
use crate::i18n::tr;
use crate::input::Action;

/// The rows of the window, top to bottom.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Btn {
    Depot,
    OpenManifests,
    OpenNotes,
    Paste,
    Import,
}

const BTNS: [Btn; 5] = [
    Btn::Depot,
    Btn::OpenManifests,
    Btn::OpenNotes,
    Btn::Paste,
    Btn::Import,
];

pub struct Import {
    pub app: String,
    /// The game's depots, biggest (the game itself) first: id, size.
    pub depots: Vec<(String, u64)>,
    pub depot: usize,
    pub text: String,
    pub entries: Vec<Entry>,
    pub builds: Vec<Build>,
    /// How many of them are new.
    pub fresh: usize,
    pub sel: usize,
    /// The clipboard being read.
    pub reading: Option<Receiver<Option<String>>>,
    /// After an import: how many were new.
    pub done: Option<usize>,
}

impl Import {
    fn depot_id(&self) -> &str {
        self.depots.get(self.depot).map_or("", |d| d.0.as_str())
    }

    /// Read the pasted text again (the depot it's for may have changed).
    fn reparse(&mut self) {
        let st = State::load();
        let depot = self.depot_id().to_string();
        self.entries = steamdb::parse(&self.text, &depot);
        self.builds = steamdb::parse_builds(&self.text);
        let known_builds: Vec<i64> = st.builds(&self.app).iter().map(|b| b.0).collect();
        self.fresh = self
            .entries
            .iter()
            .filter(|e| self.depots.iter().any(|d| d.0 == e.depot))
            .filter(|e| !st.steamdb(&e.depot).iter().any(|(g, _)| *g == e.manifest))
            .count()
            + self
                .builds
                .iter()
                .filter(|b| !known_builds.contains(&b.buildid))
                .count();
    }

    fn can_import(&self) -> bool {
        !self.builds.is_empty()
            || self
                .entries
                .iter()
                .any(|e| self.depots.iter().any(|d| d.0 == e.depot))
    }
}

/// The clipboard's text, from the host's own tools (Wayland, then X).
fn read_clipboard() -> Option<String> {
    let tries: [&[&str]; 3] = [
        &["wl-paste", "--no-newline", "--type", "text"],
        &["xclip", "-selection", "clipboard", "-o"],
        &["xsel", "--clipboard", "--output"],
    ];
    for t in tries {
        if !crate::sandbox::has_program(t[0]) {
            continue;
        }
        let mut c = std::process::Command::new(t[0]);
        c.args(&t[1..]);
        if let Ok(out) = crate::sandbox::host(c)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            && out.status.success()
        {
            let s = String::from_utf8_lossy(&out.stdout).into_owned();
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }
    None
}

impl App {
    pub fn open_steamdb_import(&mut self) {
        let Some(d) = &self.versions.data else { return };
        let mut depots: Vec<(String, u64)> = d
            .game
            .depots
            .iter()
            .map(|(id, x)| (id.clone(), x.size))
            .collect();
        depots.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        self.versions.import = Some(Import {
            app: d.game.app.clone(),
            depots,
            depot: 0,
            text: String::new(),
            entries: Vec::new(),
            builds: Vec::new(),
            fresh: 0,
            sel: 0,
            reading: None,
            done: None,
        });
    }

    fn import_paste(&mut self, text: String) {
        if let Some(im) = &mut self.versions.import {
            im.text = text;
            im.done = None;
            im.reparse();
            // Something found: straight to Import.
            if im.can_import() {
                im.sel = BTNS.iter().position(|b| *b == Btn::Import).unwrap_or(0);
            } else {
                self.toast(tr("sdb_nothing"));
            }
        }
    }

    fn import_now(&mut self) {
        let Some(im) = &mut self.versions.import else {
            return;
        };
        if !im.can_import() {
            return;
        }
        let mut st = State::load();
        let mut n = 0;
        for e in &im.entries {
            if im.depots.iter().any(|d| d.0 == e.depot) {
                n += st.add_steamdb(&e.depot, &e.manifest, e.time) as usize;
            }
        }
        for b in &im.builds {
            n += st.add_build(&im.app, b.buildid, b.time, &b.title) as usize;
        }
        st.mark_steamdb_prompted(&im.app);
        // Tests leave the user's state alone.
        let saved = if std::env::var_os("LENTA_DRY_RUN").is_some() {
            Ok(())
        } else {
            st.save()
        };
        im.text.clear();
        im.entries.clear();
        im.builds.clear();
        im.fresh = 0;
        im.done = Some(n);
        im.sel = BTNS.iter().position(|b| *b == Btn::Paste).unwrap_or(0);
        let app = im.app.clone();
        match saved {
            Ok(()) => {
                self.toast(format!("{}: {n}", tr("sdb_imported")));
                self.load_versions(&app);
            }
            Err(e) => self.toast(format!("{}: {e}", tr("sd_failed"))),
        }
    }

    fn import_press(&mut self, b: Btn) {
        let Some(im) = &mut self.versions.import else {
            return;
        };
        match b {
            Btn::Depot => {
                if !im.depots.is_empty() {
                    im.depot = (im.depot + 1) % im.depots.len();
                    im.reparse();
                }
            }
            Btn::OpenManifests => {
                let url = steamdb::manifests_url(im.depot_id());
                let _ = crate::game::run_first(&[vec!["xdg-open".into(), url]]);
            }
            Btn::OpenNotes => {
                let url = steamdb::patchnotes_url(&im.app);
                let _ = crate::game::run_first(&[vec!["xdg-open".into(), url]]);
            }
            Btn::Paste => {
                if im.reading.is_none() {
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    let ctx = self.ctx.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(read_clipboard());
                        ctx.request_repaint();
                    });
                    im.reading = Some(rx);
                }
            }
            Btn::Import => self.import_now(),
        }
    }

    /// The pad and keys while the window is up. True: it took the press.
    pub fn import_input(&mut self, a: Action) -> bool {
        let Some(im) = &mut self.versions.import else {
            return false;
        };
        match a {
            Action::Up => im.sel = im.sel.saturating_sub(1),
            Action::Down => im.sel = (im.sel + 1).min(BTNS.len() - 1),
            Action::Left | Action::Right if BTNS[im.sel] == Btn::Depot && !im.depots.is_empty() => {
                let n = im.depots.len();
                im.depot = if a == Action::Right {
                    (im.depot + 1) % n
                } else {
                    (im.depot + n - 1) % n
                };
                im.reparse();
            }
            Action::Accept => {
                let b = BTNS[im.sel];
                self.import_press(b);
            }
            Action::Select => self.import_press(Btn::Paste),
            Action::Back => self.versions.import = None,
            _ => {}
        }
        true
    }

    pub fn draw_steamdb_import(&mut self, ui: &mut Ui, area: Rect) {
        // A paste (Ctrl+V) or the clipboard read.
        let pasted = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Paste(t) => Some(t.clone()),
                _ => None,
            })
        });
        let read = self.versions.import.as_mut().and_then(|im| {
            im.reading
                .as_ref()
                .and_then(|r| r.try_recv().ok())
                .inspect(|_| im.reading = None)
        });
        match (pasted, read) {
            (Some(t), _) => self.import_paste(t),
            (None, Some(Some(t))) => self.import_paste(t),
            (None, Some(None)) => self.toast(tr("sdb_clipboard_empty")),
            _ => {}
        }
        let Some(im) = &self.versions.import else {
            return;
        };
        let layer = LayerId::new(Order::Foreground, Id::new("steamdb_import"));
        let p = ui.ctx().layer_painter(layer);
        p.rect_filled(area, 0.0, Color32::from_black_alpha(200));
        let (c1, c2) = self.accent;
        let w = 820.0f32.min(area.width() - 32.0);
        let card =
            Rect::from_center_size(area.center(), vec2(w, (area.height() - 60.0).min(760.0)));
        p.rect_filled(card, 20.0, Color32::from_rgb(24, 27, 36));
        let left = card.left() + 32.0;
        let inner = w - 64.0;
        let mut y = card.top() + 26.0;
        p.text(
            pos2(left, y),
            Align2::LEFT_TOP,
            tr("sdb_title"),
            FontId::proportional(24.0),
            Color32::WHITE,
        );
        y += 40.0;
        let intro = p.layout(
            tr("sdb_intro").to_string(),
            FontId::proportional(15.0),
            Color32::from_gray(190),
            inner,
        );
        let ih = intro.size().y;
        p.galley(pos2(left, y), intro, Color32::WHITE);
        y += ih + 18.0;

        let top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(area));
        let _veil = top.interact(area, Id::new("sdb_veil"), Sense::click());
        let mut pressed = None;
        let row_h = 44.0;
        for (i, b) in BTNS.iter().enumerate() {
            let r = Rect::from_min_size(pos2(left, y), vec2(inner, row_h - 6.0));
            y += row_h;
            let resp = top.interact(r, Id::new(("sdb_btn", i)), Sense::click());
            let focused = im.sel == i;
            let (label, on) = match b {
                Btn::Depot => {
                    let (id, size) = im.depots.get(im.depot).cloned().unwrap_or_default();
                    let main = if im.depot == 0 {
                        format!(" · {}", tr("sdb_main_depot"))
                    } else {
                        String::new()
                    };
                    (
                        format!(
                            "‹  {} {id}  ·  {:.1} {}{main}  ›",
                            tr("sd_depot"),
                            size as f64 / 1e9,
                            tr("gb")
                        ),
                        true,
                    )
                }
                Btn::OpenManifests => (tr("sdb_open_manifests").to_string(), true),
                Btn::OpenNotes => (tr("sdb_open_notes").to_string(), true),
                Btn::Paste => (
                    if im.reading.is_some() {
                        tr("loading").to_string()
                    } else {
                        tr("sdb_paste").to_string()
                    },
                    true,
                ),
                Btn::Import => (
                    if im.can_import() {
                        format!("{} ({} {})", tr("sdb_import"), im.fresh, tr("sdb_new"))
                    } else {
                        tr("sdb_import").to_string()
                    },
                    im.can_import(),
                ),
            };
            if *b == Btn::Import && on {
                crate::ui::widgets::gradient_pill(&p, r, 12.0, c1, c2);
            } else {
                p.rect_filled(
                    r,
                    12.0,
                    Color32::from_white_alpha(if resp.hovered() { 26 } else { 14 }),
                );
            }
            if focused {
                p.rect_stroke(
                    r,
                    12.0,
                    Stroke::new(
                        2.5,
                        if *b == Btn::Import && on {
                            Color32::WHITE
                        } else {
                            c1
                        },
                    ),
                    StrokeKind::Inside,
                );
            }
            p.text(
                r.left_center() + vec2(16.0, 0.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(17.0),
                if on {
                    Color32::WHITE
                } else {
                    Color32::from_gray(130)
                },
            );
            if resp.clicked() {
                pressed = Some(i);
            }
        }
        y += 8.0;
        // What the paste holds.
        let st = State::load();
        let small = FontId::proportional(14.0);
        let mut lines: Vec<(String, String, String)> = Vec::new();
        for e in &im.entries {
            let status = if !im.depots.iter().any(|d| d.0 == e.depot) {
                tr("sdb_not_installed")
            } else if st.steamdb(&e.depot).iter().any(|(g, _)| *g == e.manifest) {
                tr("sdb_known")
            } else {
                tr("sdb_fresh")
            };
            let when = if e.time > 0 {
                crate::game::date_time(e.time as u64)
            } else {
                tr("sdb_no_date").to_string()
            };
            lines.push((
                format!("{} {}  ·  {}", tr("sd_depot"), e.depot, e.manifest),
                when,
                status.to_string(),
            ));
        }
        let known: Vec<i64> = st.builds(&im.app).iter().map(|b| b.0).collect();
        for b in &im.builds {
            let status = if known.contains(&b.buildid) {
                tr("sdb_known")
            } else {
                tr("sdb_fresh")
            };
            let when = if b.time > 0 {
                crate::game::date_time(b.time as u64)
            } else {
                tr("sdb_no_date").to_string()
            };
            let title = if b.title.is_empty() {
                String::new()
            } else {
                format!("  ·  {}", b.title)
            };
            lines.push((
                format!("{} {}{title}", tr("ver_build"), b.buildid),
                when,
                status.to_string(),
            ));
        }
        if let Some(n) = im.done {
            p.text(
                pos2(left, y),
                Align2::LEFT_TOP,
                format!("{}: {n}", tr("sdb_imported")),
                small.clone(),
                Color32::from_gray(210),
            );
        } else if lines.is_empty() {
            p.text(
                pos2(left, y),
                Align2::LEFT_TOP,
                tr("sdb_waiting"),
                small.clone(),
                Color32::from_gray(150),
            );
        } else {
            let room = ((card.bottom() - 24.0 - y) / 24.0).floor().max(1.0) as usize;
            let shown = if lines.len() > room {
                room - 1
            } else {
                lines.len()
            };
            for (a, b, c) in lines.iter().take(shown) {
                let row = Rect::from_min_size(pos2(left, y), vec2(inner, 22.0));
                let pa = ui
                    .ctx()
                    .layer_painter(layer)
                    .with_clip_rect(Rect::from_min_size(row.min, vec2(inner - 260.0, 22.0)));
                pa.text(
                    row.left_center(),
                    Align2::LEFT_CENTER,
                    a,
                    small.clone(),
                    Color32::WHITE,
                );
                p.text(
                    row.right_center() - vec2(110.0, 0.0),
                    Align2::RIGHT_CENTER,
                    b,
                    small.clone(),
                    Color32::from_gray(160),
                );
                p.text(
                    row.right_center(),
                    Align2::RIGHT_CENTER,
                    c,
                    small.clone(),
                    if c == tr("sdb_fresh") {
                        c1
                    } else {
                        Color32::from_gray(160)
                    },
                );
                y += 24.0;
            }
            if shown < lines.len() {
                p.text(
                    pos2(left, y),
                    Align2::LEFT_TOP,
                    format!("{} {}", tr("sdb_more"), lines.len() - shown),
                    small,
                    Color32::from_gray(150),
                );
            }
        }
        if let Some(i) = pressed {
            if let Some(im) = &mut self.versions.import {
                im.sel = i;
            }
            self.import_press(BTNS[i]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_import_button() {
        let mut im = Import {
            app: "1".into(),
            depots: vec![("11".into(), 5), ("12".into(), 1)],
            depot: 0,
            text: "1 March 2019 – 10:00:00 UTC\t9876543210987654321\ndownload_depot 1 99 1111111111111111111\n".into(),
            entries: Vec::new(),
            builds: Vec::new(),
            fresh: 0,
            sel: 0,
            reading: None,
            done: None,
        };
        im.reparse();
        assert_eq!(im.entries.len(), 2);
        assert_eq!(im.entries[0].depot, "11");
        // Depot 99 isn't the game's: only one can go in.
        assert!(im.can_import());
        im.depot = 1;
        im.reparse();
        assert_eq!(im.entries[0].depot, "12");
    }
}
