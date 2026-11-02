//! A scrolling grid of tiles driven by the pad, shared by the library and
//! the system programs; and the filter chips above it.

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use crate::app::Grid;
use crate::input::Action;

pub struct Layout {
    pub cols: usize,
    pub cell: egui::Vec2,
    pub gap: f32,
    pub origin: egui::Pos2,
}

impl Layout {
    pub fn new(area: Rect, cell: egui::Vec2, gap: f32) -> Layout {
        let cols = (((area.width() + gap) / (cell.x + gap)).floor() as usize).max(1);
        // Centre the columns.
        let used = cols as f32 * cell.x + (cols - 1) as f32 * gap;
        Layout {
            cols,
            cell,
            gap,
            origin: pos2(area.left() + (area.width() - used) / 2.0, area.top()),
        }
    }

    pub fn rect(&self, i: usize, scroll: f32) -> Rect {
        let (r, c) = (i / self.cols, i % self.cols);
        Rect::from_min_size(
            self.origin
                + vec2(
                    c as f32 * (self.cell.x + self.gap),
                    r as f32 * (self.cell.y + self.gap) - scroll,
                ),
            self.cell,
        )
    }

    pub fn content_height(&self, n: usize) -> f32 {
        let rows = n.div_ceil(self.cols);
        rows as f32 * (self.cell.y + self.gap)
    }
}

/// Move the selection; true on Back.
pub fn nav(g: &mut Grid, n: usize, a: Action) -> bool {
    if n == 0 {
        return a == Action::Back;
    }
    let cols = g.cols.max(1);
    match a {
        Action::Left => {
            if !g.sel.is_multiple_of(cols) {
                g.sel -= 1;
            }
        }
        Action::Right => {
            if g.sel + 1 < n && !(g.sel + 1).is_multiple_of(cols) {
                g.sel += 1;
            }
        }
        Action::Up => {
            if g.sel >= cols {
                g.sel -= cols;
            }
        }
        Action::Down => {
            if g.sel + cols < n {
                g.sel += cols;
            } else if g.sel / cols < (n - 1) / cols {
                // The last row is short: its last tile.
                g.sel = n - 1;
            }
        }
        Action::Home => g.sel = 0,
        Action::End => g.sel = n - 1,
        Action::Back => return true,
        _ => return false,
    }
    g.follow = true;
    false
}

/// Scroll so the selected tile is in view (eased); the mouse wheel too.
pub fn scroll(ui: &Ui, g: &mut Grid, lay: &Layout, view: Rect, n: usize, ease: f32) {
    let max = (lay.content_height(n) - view.height() + lay.gap).max(0.0);
    if ui.rect_contains_pointer(view) {
        let d = ui.input(|i| i.smooth_scroll_delta.y);
        if d != 0.0 {
            g.scroll = (g.scroll - d).clamp(0.0, max);
            g.follow = false;
        }
    }
    if g.follow {
        let r = lay.rect(g.sel, 0.0);
        let top = r.top() - lay.origin.y - 24.0;
        let bottom = r.bottom() - lay.origin.y + 24.0 - view.height();
        let want = g.scroll.min(top).max(bottom).clamp(0.0, max);
        g.scroll += (want - g.scroll) * ease;
        if (want - g.scroll).abs() < 0.5 {
            g.scroll = want;
            g.follow = false;
        }
        ui.ctx().request_repaint();
    }
    g.scroll = g.scroll.clamp(0.0, max);
}

/// Filter chips in a row; returns the one clicked.
pub fn chips(
    ui: &mut Ui,
    at: egui::Pos2,
    labels: &[String],
    current: usize,
    accent: Color32,
) -> Option<usize> {
    let p = ui.painter().clone();
    let mut x = at.x;
    let mut clicked = None;
    for (i, l) in labels.iter().enumerate() {
        let font = FontId::proportional(16.0);
        let galley = p.layout_no_wrap(l.clone(), font.clone(), Color32::WHITE);
        let r = egui::Rect::from_min_size(pos2(x, at.y), galley.size() + vec2(28.0, 14.0));
        let resp = ui.interact(r, ui.id().with(("chip", i, l)), Sense::click());
        let on = i == current;
        let fill = if on {
            accent.gamma_multiply(0.85)
        } else if resp.hovered() {
            Color32::from_white_alpha(30)
        } else {
            Color32::from_white_alpha(16)
        };
        p.rect_filled(r, r.height() / 2.0, fill);
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            l,
            font,
            if on {
                Color32::WHITE
            } else {
                Color32::from_gray(200)
            },
        );
        if resp.clicked() {
            clicked = Some(i);
        }
        x = r.right() + 10.0;
    }
    clicked
}
