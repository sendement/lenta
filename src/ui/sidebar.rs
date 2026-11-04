//! The side panel: sections, folded to icons until it's in use.

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use super::widgets::{self, Icon};
use crate::app::{App, Page};
use crate::i18n::tr;

pub const COLLAPSED: f32 = 76.0;
pub const EXPANDED: f32 = 250.0;

fn icon_of(p: Page) -> Icon {
    match p {
        Page::Home => Icon::Home,
        Page::Library => Icon::Library,
        Page::System => Icon::System,
        Page::Proton => Icon::Proton,
        Page::Sounds => Icon::Sound,
        Page::Module(_) | Page::Modules => Icon::Module,
        Page::Settings => Icon::Settings,
        Page::About => Icon::About,
    }
}

impl App {
    /// `area` runs to the window's bottom; the last `bottom` points of it
    /// are the hints bar, which the panel only darkens.
    pub fn draw_sidebar(&mut self, ui: &mut Ui, area: Rect, bottom: f32) {
        let id = ui.id().with("sidebar");
        let hover_rect = Rect::from_min_size(
            area.min,
            vec2(
                if self.side_open_last(ui) {
                    EXPANDED
                } else {
                    COLLAPSED
                },
                area.height(),
            ),
        );
        let hovered = ui.rect_contains_pointer(hover_rect) && ui.input(|i| i.pointer.has_pointer());
        let open = self.cfg.sidebar_pinned || self.side_focus || hovered;
        let t = ui.ctx().animate_bool_with_time(id, open, 0.18);
        ui.data_mut(|d| d.insert_temp(id.with("open"), open));
        let w = COLLAPSED + (EXPANDED - COLLAPSED) * t;
        let rect = Rect::from_min_size(area.min, vec2(w, area.height()));
        let p = ui.painter_at(rect.expand2(vec2(40.0, 0.0)));
        // Panel: a dark veil, more solid as it opens.
        let a = (110.0 + 138.0 * t) as u8;
        p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(8, 9, 14, a));
        widgets::gradient_rect(
            &p,
            Rect::from_min_size(rect.right_top(), vec2(28.0, rect.height())),
            Color32::from_black_alpha((a as f32 * 0.5) as u8),
            Color32::TRANSPARENT,
        );
        let (c1, c2) = self.accent;
        // The mark at the top.
        let mark = Rect::from_center_size(
            pos2(rect.left() + COLLAPSED / 2.0, rect.top() + 40.0),
            vec2(28.0, 28.0),
        );
        widgets::gradient_rect(&p, Rect::from_min_size(mark.min, vec2(28.0, 6.0)), c1, c2);
        widgets::gradient_rect(
            &p,
            Rect::from_min_size(mark.min + vec2(0.0, 11.0), vec2(20.0, 6.0)),
            c1,
            c2,
        );
        widgets::gradient_rect(
            &p,
            Rect::from_min_size(mark.min + vec2(0.0, 22.0), vec2(12.0, 6.0)),
            c1,
            c2,
        );
        if t > 0.3 {
            let alpha = ((t - 0.3) / 0.7).clamp(0.0, 1.0);
            p.text(
                pos2(mark.right() + 22.0, mark.center().y),
                Align2::LEFT_CENTER,
                "lenta",
                FontId::proportional(24.0),
                Color32::WHITE.gamma_multiply(alpha),
            );
        }
        let row_h = 58.0;
        let top = rect.top() + 100.0;
        let pages = self.pages();
        for (i, page) in pages.iter().enumerate() {
            // About sits at the bottom.
            let y = if *page == Page::About {
                rect.bottom() - bottom - row_h - 16.0
            } else {
                top + i as f32 * (row_h + 6.0)
            };
            let r = Rect::from_min_size(pos2(rect.left() + 10.0, y), vec2(w - 20.0, row_h));
            let resp = ui.interact(r, id.with(i), Sense::click());
            let current = self.page == *page;
            let focused = self.side_focus && self.side_sel == i;
            if current {
                p.rect_filled(
                    r,
                    14.0,
                    Color32::from_white_alpha(if self.side_focus { 18 } else { 26 }),
                );
            }
            if focused {
                p.rect_stroke(
                    r,
                    14.0,
                    egui::Stroke::new(2.0, c1),
                    egui::StrokeKind::Inside,
                );
            } else if resp.hovered() {
                p.rect_filled(r, 14.0, Color32::from_white_alpha(12));
            }
            let color = if current || focused {
                Color32::WHITE
            } else {
                Color32::from_gray(170)
            };
            let ic = pos2(rect.left() + COLLAPSED / 2.0, r.center().y);
            match page {
                Page::Module(m) => self.mod_icon(&p, ic, 26.0, *m as usize, color),
                _ => widgets::icon(&p, ic, 26.0, icon_of(*page), color),
            }
            if t > 0.3 {
                let alpha = ((t - 0.3) / 0.7).clamp(0.0, 1.0);
                let name = match page {
                    Page::Module(m) => self
                        .mods
                        .list
                        .get(*m as usize)
                        .map(|m| m.name.clone())
                        .unwrap_or_default(),
                    _ => tr(page.key()).to_string(),
                };
                p.text(
                    pos2(rect.left() + COLLAPSED - 2.0, r.center().y),
                    Align2::LEFT_CENTER,
                    name,
                    FontId::proportional(19.0),
                    color.gamma_multiply(alpha),
                );
            }
            if resp.clicked() {
                self.go(*page);
                self.side_focus = false;
            }
        }
    }

    fn side_open_last(&self, ui: &Ui) -> bool {
        ui.data(|d| d.get_temp::<bool>(ui.id().with("sidebar").with("open")))
            .unwrap_or(false)
    }
}
