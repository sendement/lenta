//! About: what this is, what it found, the controls.

use egui::{Align2, Color32, FontId, Rect, Ui, pos2, vec2};

use super::widgets;
use crate::app::App;
use crate::game::Source;
use crate::i18n::tr;

impl App {
    pub fn draw_about(&mut self, ui: &mut Ui, rect: Rect) {
        let rect = rect.shrink2(vec2(56.0, 0.0));
        let p = ui.painter_at(rect);
        let (c1, c2) = self.accent;
        let mut y = rect.top() + 60.0;
        let title = p.text(
            pos2(rect.left(), y),
            Align2::LEFT_TOP,
            "lenta",
            FontId::proportional(64.0),
            Color32::WHITE,
        );
        p.text(
            pos2(title.right() + 18.0, title.bottom() - 12.0),
            Align2::LEFT_BOTTOM,
            concat!("v", env!("CARGO_PKG_VERSION")),
            FontId::proportional(20.0),
            Color32::from_gray(170),
        );
        widgets::gradient_rect(
            &p,
            Rect::from_min_size(pos2(rect.left(), title.bottom() + 10.0), vec2(220.0, 4.0)),
            c1,
            c2,
        );
        y = title.bottom() + 36.0;
        let w = rect.width().min(820.0);
        let g = p.layout(
            tr("about_text").to_string(),
            FontId::proportional(20.0),
            Color32::from_gray(220),
            w,
        );
        let h = g.size().y;
        p.galley(pos2(rect.left(), y), g, Color32::WHITE);
        y += h + 26.0;
        let mut found = format!("{}: ", tr("found"));
        let parts: Vec<String> = Source::ALL
            .iter()
            .map(|s| {
                format!(
                    "{} — {}",
                    s.name(),
                    self.games.iter().filter(|g| g.source == *s).count()
                )
            })
            .chain(std::iter::once(format!(
                "{} — {}",
                tr("system"),
                self.apps.len()
            )))
            .collect();
        found += &parts.join(",  ");
        p.text(
            pos2(rect.left(), y),
            Align2::LEFT_TOP,
            found,
            FontId::proportional(17.0),
            Color32::from_gray(200),
        );
        y += 30.0;
        p.text(
            pos2(rect.left(), y),
            Align2::LEFT_TOP,
            tr("backlog"),
            FontId::proportional(17.0),
            Color32::from_gray(160),
        );
        y += 50.0;
        p.text(
            pos2(rect.left(), y),
            Align2::LEFT_TOP,
            tr("controls"),
            FontId::proportional(22.0),
            c1,
        );
        y += 38.0;
        for k in [
            "ctl_move",
            "ctl_accept",
            "ctl_play",
            "ctl_back",
            "ctl_pages",
            "ctl_trig",
            "ctl_side",
            "ctl_full",
        ] {
            p.text(
                pos2(rect.left(), y),
                Align2::LEFT_TOP,
                tr(k),
                FontId::proportional(17.0),
                Color32::from_gray(210),
            );
            y += 28.0;
        }
        y += 24.0;
        p.text(
            pos2(rect.left(), y),
            Align2::LEFT_TOP,
            format!("{} · © 2026 Sendem", tr("license")),
            FontId::proportional(15.0),
            Color32::from_gray(140),
        );
    }
}
