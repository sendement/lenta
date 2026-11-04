//! A question before something that can't be taken back: a title, what
//! will happen, Yes (A) and Cancel (B).

use egui::{Align2, Color32, FontId, Id, LayerId, Order, Rect, Sense, Ui, pos2, vec2};

use crate::app::App;
use crate::i18n::tr;
use crate::input::Action;

pub struct Confirm {
    pub title: String,
    pub lines: Vec<String>,
    pub yes: String,
    /// Runs on Yes.
    pub action: Box<dyn FnOnce(&mut App)>,
}

impl App {
    pub fn ask(
        &mut self,
        title: String,
        lines: Vec<String>,
        yes: &str,
        action: impl FnOnce(&mut App) + 'static,
    ) {
        self.confirm = Some(Confirm {
            title,
            lines,
            yes: yes.to_string(),
            action: Box::new(action),
        });
    }

    pub fn confirm_input(&mut self, a: Action) {
        match a {
            Action::Accept => {
                if let Some(c) = self.confirm.take() {
                    (c.action)(self);
                }
            }
            Action::Back => self.confirm = None,
            _ => {}
        }
    }

    pub fn draw_confirm(&mut self, ui: &mut Ui, area: Rect) {
        let Some(c) = &self.confirm else { return };
        let layer = LayerId::new(Order::Tooltip, Id::new("confirm"));
        let p = ui.ctx().layer_painter(layer);
        p.rect_filled(area, 0.0, Color32::from_black_alpha(195));
        let (c1, c2) = self.accent;
        let w = 680.0;
        let title = p.layout(
            c.title.clone(),
            FontId::proportional(24.0),
            Color32::WHITE,
            w - 64.0,
        );
        let lines: Vec<_> = c
            .lines
            .iter()
            .map(|l| {
                p.layout(
                    format!("•  {l}"),
                    FontId::proportional(16.0),
                    Color32::from_gray(205),
                    w - 64.0,
                )
            })
            .collect();
        let body_h: f32 = lines.iter().map(|g| g.size().y + 8.0).sum();
        let card = Rect::from_center_size(
            area.center(),
            vec2(w, 28.0 + title.size().y + 16.0 + body_h + 96.0),
        );
        p.rect_filled(card, 20.0, Color32::from_rgb(24, 27, 36));
        let mut y = card.top() + 28.0;
        let th = title.size().y;
        p.galley(pos2(card.left() + 32.0, y), title, Color32::WHITE);
        y += th + 16.0;
        for g in lines {
            let h = g.size().y;
            p.galley(pos2(card.left() + 32.0, y), g, Color32::WHITE);
            y += h + 8.0;
        }
        let bw = 190.0;
        let yes = Rect::from_min_size(
            pos2(card.right() - 32.0 - bw, card.bottom() - 70.0),
            vec2(bw, 46.0),
        );
        let no = yes.translate(vec2(-bw - 14.0, 0.0));
        crate::ui::widgets::gradient_pill(&p, yes, 14.0, c1, c2);
        p.rect_filled(no, 14.0, Color32::from_white_alpha(22));
        p.text(
            yes.center(),
            Align2::CENTER_CENTER,
            &c.yes,
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
        let _veil = top.interact(area, Id::new("confirm_veil"), Sense::click());
        if top
            .interact(yes, Id::new("confirm_yes"), Sense::click())
            .clicked()
        {
            self.confirm_input(Action::Accept);
        } else if top
            .interact(no, Id::new("confirm_no"), Sense::click())
            .clicked()
        {
            self.confirm_input(Action::Back);
        }
    }
}
