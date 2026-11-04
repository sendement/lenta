//! Making or changing one of the launcher's own collections: its name,
//! plain or dynamic, and for a dynamic one the rule its games fit.

use egui::{
    Align2, Color32, FontId, Id, LayerId, Order, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::app::App;
use crate::colls::{Coll, Played, Rule, Time, Tri};
use crate::game::Source;
use crate::i18n::tr;
use crate::input::Action;

pub struct CollEdit {
    /// The one changed; None for a new one.
    pub id: Option<u64>,
    pub name: String,
    pub dynamic: bool,
    pub rule: Rule,
    pub sel: usize,
    /// The button chosen on the last row.
    pub btn: usize,
    /// A game to put in the new one.
    pub with: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Name,
    Kind,
    Source,
    Installed,
    Favorite,
    Third,
    Played,
    Time,
    Words,
    Buttons,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Button {
    Save,
    Cancel,
    Delete,
}

impl CollEdit {
    fn rows(&self) -> Vec<Row> {
        let mut v = vec![Row::Name, Row::Kind];
        if self.dynamic {
            v.extend([
                Row::Source,
                Row::Installed,
                Row::Favorite,
                Row::Third,
                Row::Played,
                Row::Time,
                Row::Words,
            ]);
        }
        v.push(Row::Buttons);
        v
    }

    fn buttons(&self) -> Vec<Button> {
        let mut v = vec![Button::Save, Button::Cancel];
        if self.id.is_some() {
            v.push(Button::Delete);
        }
        v
    }

    /// Step a choice row by `d` (±1).
    fn step(&mut self, row: Row, d: i32) {
        fn cycle<T: Copy + PartialEq>(all: &[T], cur: T, d: i32) -> T {
            let i = all.iter().position(|x| *x == cur).unwrap_or(0) as i32;
            all[(i + d).rem_euclid(all.len() as i32) as usize]
        }
        let r = &mut self.rule;
        match row {
            Row::Kind => self.dynamic = !self.dynamic,
            Row::Source => {
                let all: Vec<Option<Source>> = std::iter::once(None)
                    .chain(Source::ALL.into_iter().map(Some))
                    .collect();
                r.source = cycle(&all, r.source, d);
            }
            Row::Installed => r.installed = cycle(&Tri::ALL, r.installed, d),
            Row::Favorite => r.favorite = cycle(&Tri::ALL, r.favorite, d),
            Row::Third => r.third_party = cycle(&Tri::ALL, r.third_party, d),
            Row::Played => r.played = cycle(&Played::ALL, r.played, d),
            Row::Time => r.time = cycle(&Time::ALL, r.time, d),
            Row::Buttons => {
                let n = self.buttons().len() as i32;
                self.btn = (self.btn as i32 + d).clamp(0, n - 1) as usize;
            }
            Row::Name | Row::Words => {}
        }
    }

    fn label(row: Row) -> &'static str {
        match row {
            Row::Name => tr("coll_name"),
            Row::Kind => tr("coll_kind"),
            Row::Source => tr("coll_r_source"),
            Row::Installed => tr("coll_r_installed"),
            Row::Favorite => tr("coll_r_favorite"),
            Row::Third => tr("coll_r_third"),
            Row::Played => tr("coll_r_played"),
            Row::Time => tr("coll_r_time"),
            Row::Words => tr("coll_r_words"),
            Row::Buttons => "",
        }
    }

    fn value(&self, row: Row) -> String {
        let tri = |t: Tri| match t {
            Tri::Any => tr("coll_any"),
            Tri::Yes => tr("yes"),
            Tri::No => tr("no"),
        };
        let r = &self.rule;
        match row {
            Row::Name => self.name.clone(),
            Row::Kind => if self.dynamic {
                tr("coll_dynamic")
            } else {
                tr("coll_plain")
            }
            .into(),
            Row::Source => r
                .source
                .map_or(tr("coll_any_launcher"), |s| s.name())
                .into(),
            Row::Installed => tri(r.installed).into(),
            Row::Favorite => tri(r.favorite).into(),
            Row::Third => tri(r.third_party).into(),
            Row::Played => match r.played {
                Played::Any => tr("coll_any"),
                Played::Week => tr("coll_p_week"),
                Played::Month => tr("coll_p_month"),
                Played::Year => tr("coll_p_year"),
                Played::Long => tr("coll_p_long"),
                Played::Never => tr("coll_p_never"),
            }
            .into(),
            Row::Time => match r.time {
                Time::Any => tr("coll_any"),
                Time::Little => tr("coll_t_little"),
                Time::Ten => tr("coll_t_ten"),
                Time::Fifty => tr("coll_t_fifty"),
            }
            .into(),
            Row::Words => self.rule.words.clone(),
            Row::Buttons => String::new(),
        }
    }
}

impl App {
    /// The editor: for `id`, or a new one (with a game to put in).
    pub fn edit_coll(&mut self, id: Option<u64>, with: Option<String>) {
        let c = id.and_then(|i| self.colls.get(i)).cloned();
        self.cedit = Some(CollEdit {
            id: c.as_ref().map(|c| c.id),
            name: c.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
            dynamic: c.as_ref().is_some_and(|c| c.dynamic()),
            rule: c.and_then(|c| c.rule).unwrap_or_default(),
            sel: 0,
            btn: 0,
            with,
        });
    }

    fn cedit_text(&mut self, row: Row) {
        let Some(e) = &self.cedit else { return };
        let (title, text) = match row {
            Row::Name => (tr("coll_name"), e.name.clone()),
            _ => (tr("coll_r_words"), e.rule.words.clone()),
        };
        self.type_text(title.into(), text, move |app, s| {
            if let Some(e) = &mut app.cedit {
                match row {
                    Row::Name => e.name = s,
                    _ => e.rule.words = s,
                }
            }
        });
    }

    fn cedit_press(&mut self, b: Button) {
        let Some(e) = &self.cedit else { return };
        match b {
            Button::Cancel => self.cedit = None,
            Button::Delete => {
                if let Some(id) = e.id {
                    self.ask_delete_coll(id);
                }
            }
            Button::Save => {
                let Some(e) = self.cedit.take() else { return };
                let name = if e.name.trim().is_empty() {
                    format!("{} {}", tr("coll_default_name"), self.colls.list.len() + 1)
                } else {
                    e.name.trim().to_string()
                };
                let rule = e.dynamic.then_some(e.rule);
                let id = match e.id.and_then(|id| self.colls.get_mut(id)) {
                    Some(c) => {
                        c.name = name;
                        c.rule = rule;
                        c.id
                    }
                    None => self.colls.create(name, rule),
                };
                if let Some(k) = e.with
                    && let Some(c) = self.colls.get_mut(id)
                {
                    c.add(&k);
                }
                self.save_colls();
            }
        }
    }

    pub fn cedit_input(&mut self, a: Action) {
        let Some(e) = &mut self.cedit else { return };
        let rows = e.rows();
        e.sel = e.sel.min(rows.len() - 1);
        let row = rows[e.sel];
        match a {
            Action::Up => e.sel = e.sel.saturating_sub(1),
            Action::Down => e.sel = (e.sel + 1).min(rows.len() - 1),
            Action::Left => e.step(row, -1),
            Action::Right => e.step(row, 1),
            Action::Accept => match row {
                Row::Name | Row::Words => self.cedit_text(row),
                Row::Buttons => {
                    let b = e.buttons()[e.btn.min(e.buttons().len() - 1)];
                    self.cedit_press(b);
                }
                _ => e.step(row, 1),
            },
            Action::Menu => self.cedit_press(Button::Save),
            Action::Back => self.cedit = None,
            _ => {}
        }
    }

    pub fn draw_cedit(&mut self, ui: &mut Ui, area: Rect) {
        let Some(e) = &self.cedit else { return };
        let layer = LayerId::new(Order::Foreground, Id::new("cedit"));
        let p = ui.ctx().layer_painter(layer);
        p.rect_filled(area, 0.0, Color32::from_black_alpha(195));
        let (c1, c2) = self.accent;
        let rows = e.rows();
        let row_h = 50.0;
        let w = 720.0;
        let h = 28.0 + 34.0 + 30.0 + (rows.len() - 1) as f32 * row_h + 24.0 + 46.0 + 30.0;
        let card = Rect::from_center_size(area.center(), vec2(w, h.min(area.height() - 20.0)));
        p.rect_filled(card, 22.0, Color32::from_rgb(24, 27, 36));
        let title = if e.id.is_some() {
            tr("coll_edit_title")
        } else {
            tr("coll_new_title")
        };
        p.text(
            card.min + vec2(32.0, 28.0),
            Align2::LEFT_TOP,
            title,
            FontId::proportional(24.0),
            Color32::WHITE,
        );
        // How many games it has as it stands.
        let preview = Coll {
            id: e.id.unwrap_or(0),
            name: String::new(),
            games: e
                .id
                .and_then(|i| self.colls.get(i))
                .map(|c| c.games.clone())
                .unwrap_or_default()
                .into_iter()
                .chain(e.with.clone())
                .collect(),
            rule: e.dynamic.then(|| e.rule.clone()),
            hide: e
                .id
                .and_then(|i| self.colls.get(i))
                .map(|c| c.hide.clone())
                .unwrap_or_default(),
        };
        let n = self.coll_games(&preview).len();
        p.text(
            pos2(card.right() - 32.0, card.top() + 34.0),
            Align2::RIGHT_TOP,
            format!("{}: {n}", tr("coll_games_n")),
            FontId::proportional(16.0),
            Color32::from_gray(170),
        );
        let top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(area));
        let _veil = top.interact(area, Id::new("cedit_veil"), Sense::click());
        let mut y = card.top() + 28.0 + 34.0 + 22.0;
        let sel = e.sel.min(rows.len() - 1);
        let mut click: Option<(usize, i32)> = None;
        let mut pressed = None;
        for (i, row) in rows.iter().enumerate() {
            if *row == Row::Buttons {
                y += 16.0;
                let bw = 180.0;
                let mut x = card.right() - 32.0;
                for (bi, b) in e.buttons().iter().enumerate() {
                    let r = Rect::from_min_size(pos2(x - bw, y), vec2(bw, 46.0));
                    x -= bw + 14.0;
                    let resp = top.interact(r, Id::new(("cedit_btn", bi)), Sense::click());
                    let (label, fill) = match b {
                        Button::Save => (tr("coll_save"), None),
                        Button::Cancel => (tr("cancel"), Some(Color32::from_white_alpha(22))),
                        Button::Delete => (
                            tr("coll_delete_short"),
                            Some(Color32::from_rgba_unmultiplied(220, 70, 70, 60)),
                        ),
                    };
                    match fill {
                        None => super::widgets::gradient_pill(&p, r, 14.0, c1, c2),
                        Some(f) => {
                            p.rect_filled(r, 14.0, f);
                        }
                    }
                    if sel == i && e.btn == bi {
                        p.rect_stroke(
                            r,
                            14.0,
                            Stroke::new(2.5, Color32::WHITE),
                            StrokeKind::Outside,
                        );
                    }
                    p.text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(18.0),
                        Color32::WHITE,
                    );
                    if resp.clicked() {
                        pressed = Some((i, bi, *b));
                    }
                }
                continue;
            }
            let r = Rect::from_min_size(pos2(card.left() + 20.0, y), vec2(w - 40.0, row_h - 6.0));
            y += row_h;
            let resp = top.interact(r, Id::new(("cedit_row", i)), Sense::click());
            let on = i == sel;
            if on || resp.hovered() {
                p.rect_filled(r, 12.0, Color32::from_white_alpha(if on { 26 } else { 14 }));
            }
            if on {
                p.rect_stroke(r, 12.0, Stroke::new(2.0, c1), StrokeKind::Inside);
            }
            p.text(
                r.left_center() + vec2(16.0, 0.0),
                Align2::LEFT_CENTER,
                CollEdit::label(*row),
                FontId::proportional(17.0),
                Color32::from_gray(210),
            );
            let val = e.value(*row);
            let vr = Rect::from_min_max(
                pos2(r.left() + 300.0, r.top() + 5.0),
                r.right_bottom() - vec2(10.0, 5.0),
            );
            match row {
                Row::Name | Row::Words => {
                    p.rect_filled(vr, 10.0, Color32::from_black_alpha(80));
                    let empty = val.is_empty();
                    let shown = if empty {
                        if *row == Row::Name {
                            tr("coll_name_hint")
                        } else {
                            tr("coll_words_hint")
                        }
                        .to_string()
                    } else {
                        val
                    };
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        shown,
                        FontId::proportional(17.0),
                        Color32::from_gray(if empty { 120 } else { 255 }),
                    );
                    job.wrap.max_width = vr.width() - 28.0;
                    job.wrap.max_rows = 1;
                    let g = p.fonts_mut(|f| f.layout_job(job));
                    p.galley(
                        pos2(vr.left() + 14.0, vr.center().y - g.size().y / 2.0),
                        g,
                        Color32::WHITE,
                    );
                    if resp.clicked() {
                        click = Some((i, 0));
                    }
                }
                _ => {
                    p.text(
                        vr.center(),
                        Align2::CENTER_CENTER,
                        val,
                        FontId::proportional(17.0),
                        Color32::WHITE,
                    );
                    let la = Rect::from_center_size(
                        vr.left_center() + vec2(18.0, 0.0),
                        vec2(36.0, 36.0),
                    );
                    let ra = Rect::from_center_size(
                        vr.right_center() - vec2(18.0, 0.0),
                        vec2(36.0, 36.0),
                    );
                    for (a, s, d) in [(la, "‹", -1), (ra, "›", 1)] {
                        p.text(
                            a.center(),
                            Align2::CENTER_CENTER,
                            s,
                            FontId::proportional(26.0),
                            Color32::from_gray(200),
                        );
                        if top
                            .interact(a, Id::new(("cedit_arrow", i, d)), Sense::click())
                            .clicked()
                        {
                            click = Some((i, d));
                        }
                    }
                    if click.is_none() && resp.clicked() {
                        click = Some((i, 1));
                    }
                }
            }
        }
        if let Some((i, bi, b)) = pressed {
            if let Some(e) = &mut self.cedit {
                e.sel = i;
                e.btn = bi;
            }
            self.cedit_press(b);
        } else if let Some((i, d)) = click {
            let row = rows[i];
            if let Some(e) = &mut self.cedit {
                e.sel = i;
                if d != 0 {
                    e.step(row, d);
                }
            }
            if d == 0 {
                self.cedit_text(row);
            }
        }
    }
}
