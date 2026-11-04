//! Getting the SteamGridDB key from Settings: one button. Signed in through
//! Steam on a thread; when there's no SteamGridDB account yet, a question
//! with a name to pick (← →) before one is made.

use std::time::Duration;

use crossbeam_channel::Receiver;
use egui::{Align2, Color32, FontId, Id, LayerId, Order, Rect, Sense, Ui, pos2, vec2};

use crate::app::App;
use crate::i18n::tr;
use crate::input::Action;
use crate::sgdb::{self, Session, Who};

pub enum Msg {
    Done,
    /// No account: names to offer.
    NoAccount(Vec<String>),
    Failed(String),
}

#[derive(Default)]
pub struct SgdbUi {
    pub job: Option<Receiver<Msg>>,
    /// The question: names and the one picked.
    pub ask: Option<(Vec<String>, usize)>,
    pub has_key: Option<bool>,
}

impl SgdbUi {
    pub fn has_key(&mut self) -> bool {
        *self.has_key.get_or_insert_with(|| sgdb::key().is_some())
    }
}

impl App {
    /// Sign in through Steam; `name`: create the account with it first.
    pub fn sgdb_connect(&mut self, name: Option<String>) {
        let Some(acc) = self.steam.account.clone() else {
            self.toast(tr("sgdb_need_steam"));
            self.start_login();
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let run = || -> Result<Msg, String> {
                let (mut s, who): (Session, Who) = Session::sign_in(&acc)?;
                if who == Who::NoAccount {
                    let Some(name) = name else {
                        let steam_name =
                            sgdb::persona_name(&acc.steamid).unwrap_or(acc.name.clone());
                        let mut names = sgdb::name_choices(&steam_name);
                        // Only the free ones.
                        names.retain(|n| s.check_name(n).is_ok());
                        return Ok(Msg::NoAccount(names));
                    };
                    s.check_name(&name)
                        .map_err(|e| format!("{}: {e}", tr("sgdb_name_taken")))?;
                    s.register(&name)?;
                }
                let key = s.api_key()?;
                if !sgdb::check_key(&key)? {
                    return Err(tr("sgdb_key_refused").into());
                }
                sgdb::save_key(&key)?;
                Ok(Msg::Done)
            };
            let _ = tx.send(run().unwrap_or_else(Msg::Failed));
            ctx.request_repaint();
        });
        self.sgdb.job = Some(rx);
    }

    pub fn sgdb_disconnect(&mut self) {
        sgdb::forget_key();
        self.sgdb.has_key = Some(false);
        self.toast(tr("sgdb_off"));
    }

    pub(crate) fn pump_sgdb(&mut self) {
        self.sgdb.has_key();
        let Some(m) = self.sgdb.job.as_ref().and_then(|r| r.try_recv().ok()) else {
            return;
        };
        self.sgdb.job = None;
        match m {
            Msg::Done => {
                self.sgdb.has_key = Some(true);
                self.toast(tr("sgdb_on"));
            }
            Msg::NoAccount(names) if names.is_empty() => self.toast(tr("sgdb_name_taken")),
            Msg::NoAccount(names) => self.sgdb.ask = Some((names, 0)),
            Msg::Failed(e) => self.toast(format!("SteamGridDB: {e}")),
        }
    }

    /// Keys while the question is up.
    pub fn sgdb_input(&mut self, a: Action) {
        let Some((names, i)) = self.sgdb.ask.as_mut() else {
            return;
        };
        match a {
            Action::Left => *i = (*i + names.len() - 1) % names.len(),
            Action::Right => *i = (*i + 1) % names.len(),
            Action::Accept => {
                let name = names[*i].clone();
                self.sgdb.ask = None;
                self.sgdb_connect(Some(name));
            }
            Action::Back => self.sgdb.ask = None,
            _ => {}
        }
    }

    pub fn draw_sgdb_ask(&mut self, ui: &mut Ui, area: Rect) {
        let Some((names, sel)) = self.sgdb.ask.clone() else {
            return;
        };
        let layer = LayerId::new(Order::Foreground, Id::new("sgdb_ask"));
        let p = ui.ctx().layer_painter(layer);
        p.rect_filled(area, 0.0, Color32::from_black_alpha(200));
        let (c1, c2) = self.accent;
        let card = Rect::from_center_size(area.center(), vec2(640.0, 300.0));
        p.rect_filled(card, 20.0, Color32::from_rgb(24, 27, 36));
        p.text(
            card.min + vec2(32.0, 28.0),
            Align2::LEFT_TOP,
            tr("sgdb_q_title"),
            FontId::proportional(24.0),
            Color32::WHITE,
        );
        let note = p.layout(
            tr("sgdb_q_note").to_string(),
            FontId::proportional(15.0),
            Color32::from_gray(190),
            card.width() - 64.0,
        );
        p.galley(card.min + vec2(32.0, 70.0), note, Color32::WHITE);
        // The name, picked with ← →.
        let nr =
            Rect::from_center_size(pos2(card.center().x, card.top() + 165.0), vec2(360.0, 48.0));
        p.rect_filled(nr, 14.0, Color32::from_white_alpha(18));
        p.text(
            nr.center(),
            Align2::CENTER_CENTER,
            &names[sel],
            FontId::proportional(20.0),
            Color32::WHITE,
        );
        let arrows = Color32::from_gray(if names.len() > 1 { 220 } else { 80 });
        p.text(
            nr.left_center() + vec2(18.0, 0.0),
            Align2::CENTER_CENTER,
            "‹",
            FontId::proportional(28.0),
            arrows,
        );
        p.text(
            nr.right_center() - vec2(18.0, 0.0),
            Align2::CENTER_CENTER,
            "›",
            FontId::proportional(28.0),
            arrows,
        );
        let yes = Rect::from_min_size(
            pos2(card.right() - 32.0 - 190.0, card.bottom() - 70.0),
            vec2(190.0, 46.0),
        );
        let no = yes.translate(vec2(-204.0, 0.0));
        crate::ui::widgets::gradient_pill(&p, yes, 14.0, c1, c2);
        p.rect_filled(no, 14.0, Color32::from_white_alpha(22));
        p.text(
            yes.center(),
            Align2::CENTER_CENTER,
            tr("sgdb_create"),
            FontId::proportional(18.0),
            Color32::WHITE,
        );
        p.text(
            no.center(),
            Align2::CENTER_CENTER,
            tr("cancel"),
            FontId::proportional(18.0),
            Color32::from_gray(220),
        );
        let top = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(area));
        let _veil = top.interact(area, Id::new("sgdb_veil"), Sense::click());
        if top
            .interact(yes, Id::new("sgdb_yes"), Sense::click())
            .clicked()
        {
            self.sgdb_input(Action::Accept);
        } else if top
            .interact(no, Id::new("sgdb_no"), Sense::click())
            .clicked()
        {
            self.sgdb_input(Action::Back);
        } else if top
            .interact(
                nr.with_max_x(nr.center().x),
                Id::new("sgdb_l"),
                Sense::click(),
            )
            .clicked()
        {
            self.sgdb_input(Action::Left);
        } else if top
            .interact(
                nr.with_min_x(nr.center().x),
                Id::new("sgdb_r"),
                Sense::click(),
            )
            .clicked()
        {
            self.sgdb_input(Action::Right);
        }
        if self.sgdb.job.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }
}
