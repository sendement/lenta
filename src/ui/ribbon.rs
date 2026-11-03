//! The ribbon of game cards on the home screen.

use egui::{Color32, FontId, Rect, Sense, Ui, Vec2, pos2, vec2};

use super::card::{self, Axis, Fill, Pose};
use crate::config::{Anchor, Config, Orientation, Shape, TiltMode};
use crate::game::{Art, Game};
use crate::images::{Images, Kind};

pub fn base_size(cfg: &Config) -> Vec2 {
    let s = cfg.card_size;
    match cfg.shape {
        Shape::Portrait => vec2(s * 2.0 / 3.0, s),
        Shape::Landscape => {
            let w = s * 1.3;
            vec2(w, w * 215.0 / 460.0)
        }
    }
}

pub fn arts(g: &Game, shape: Shape) -> [Option<&Art>; 4] {
    let a = &g.art;
    match shape {
        Shape::Portrait => [
            a.portrait.as_ref(),
            a.portrait_alt.as_ref(),
            a.landscape.as_ref(),
            a.landscape_alt.as_ref(),
        ],
        Shape::Landscape => [
            a.landscape.as_ref(),
            a.landscape_alt.as_ref(),
            a.hero.as_ref(),
            a.portrait.as_ref(),
        ],
    }
}

/// Pixels to decode card art at: set each frame from the biggest card's
/// size on screen.
static CARD_PX: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(768);

/// How card art is decoded now.
pub fn card_kind() -> Kind {
    Kind::Card(CARD_PX.load(std::sync::atomic::Ordering::Relaxed))
}

/// The selected card's longer side in real pixels, in steps (so moving a
/// slider doesn't decode everything again on every notch).
pub fn size_cards(cfg: &Config, pixels_per_point: f32) {
    let s = base_size(cfg);
    let px = s.x.max(s.y) * cfg.selected_scale.max(1.0) * pixels_per_point;
    let step = (px / 256.0).ceil().clamp(2.0, 8.0) as u32 * 256;
    CARD_PX.store(step, std::sync::atomic::Ordering::Relaxed);
}

/// A colour pair of its own for a game without art.
pub fn own_colors(title: &str) -> (Color32, Color32) {
    let h = (crate::net::hash(title) % 360) as f32 / 360.0;
    (
        crate::images::from_hsv(h, 0.55, 0.45),
        crate::images::from_hsv(h + 0.1, 0.65, 0.25),
    )
}

/// A card's face: its art (covering the card), or a gradient with the
/// title written on it.
pub fn fill_for(images: &mut Images, g: &Game, shape: Shape, size: Vec2) -> (Fill, bool) {
    match images.first(&arts(g, shape), card_kind()) {
        Some(t) => (
            Fill::Image(t.handle.id(), card::cover_uv(t.size, size)),
            true,
        ),
        None => {
            let (a, b) = own_colors(&g.title);
            (Fill::Gradient(a, b), false)
        }
    }
}

/// A card without art: the game's icon (if it has one) and its title.
pub fn title_on(painter: &egui::Painter, images: &mut Images, g: &Game, rect: Rect, alpha: f32) {
    let font = FontId::proportional((rect.height() * 0.09).clamp(12.0, 26.0));
    let galley = painter.layout(
        g.title.clone(),
        font,
        Color32::WHITE.gamma_multiply(alpha),
        rect.width() * 0.84,
    );
    let icon = g
        .art
        .icon
        .as_ref()
        .and_then(|i| images.get(i, Kind::Fit(256)));
    let side = (rect.width() * 0.32).min(96.0);
    let gap = if icon.is_some() { side + 14.0 } else { 0.0 };
    let top = rect.center().y - (galley.size().y + gap) / 2.0;
    if let Some(t) = icon {
        let r = Rect::from_center_size(
            egui::pos2(rect.center().x, top + side / 2.0),
            Vec2::splat(side),
        );
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(t.handle.id(), r, uv, Color32::WHITE.gamma_multiply(alpha));
    }
    let at = egui::pos2(rect.center().x - galley.size().x / 2.0, top + gap);
    painter.galley(at, galley, Color32::WHITE);
}

#[derive(Default)]
pub struct Out {
    /// A card was clicked: its index.
    pub clicked: Option<usize>,
    /// Wheel movement over the ribbon (+ towards the next card).
    pub scroll: f32,
    /// Dragged this frame by this many cards (+ towards the next).
    pub drag: f32,
    pub dragging: bool,
    /// The drag ended this frame.
    pub released: bool,
    /// Where the selected card is (for the info beside it).
    pub selected: Option<Rect>,
}

pub struct Look {
    pub accent: (Color32, Color32),
    /// 0…1, for the glow's breathing.
    pub pulse: f32,
}

/// Draw the ribbon in `rect`; `pos` is the (animated) index in the middle.
pub fn draw(
    ui: &mut Ui,
    rect: Rect,
    cfg: &Config,
    games: &[&Game],
    sel: usize,
    pos: f32,
    images: &mut Images,
    look: &Look,
) -> Out {
    let mut out = Out::default();
    if games.is_empty() {
        return out;
    }
    let resp = ui.interact(rect, ui.id().with("ribbon"), Sense::click_and_drag());
    let horizontal = cfg.orientation == Orientation::Horizontal;
    let dir = if cfg.reverse { -1.0 } else { 1.0 };
    let mut size = base_size(cfg);
    // Shrink to fit the band across.
    let cross_room = if horizontal {
        rect.height()
    } else {
        rect.width()
    };
    let cross = if horizontal { size.y } else { size.x };
    let fit = (cross_room
        * if cfg.reflection && horizontal {
            0.58
        } else {
            0.78
        }
        / (cross * cfg.selected_scale))
        .min(1.0);
    size *= fit;
    let along = if horizontal { size.x } else { size.y };
    let tilt = cfg.tilt.to_radians();
    let tight = 1.0 - 0.45 * tilt.sin();
    let sel_scale = cfg.selected_scale;
    let gap = cfg.spacing * fit;
    let first_step = along * sel_scale / 2.0 + gap + along * tight / 2.0;
    let step = along * tight + gap;
    // Held and moved: the cards follow the pointer, a card per step.
    if resp.dragged() {
        let d = resp.drag_delta();
        let along_d = if horizontal { d.x } else { d.y };
        out.drag = -along_d * dir / step.max(1.0);
        out.dragging = true;
    }
    out.released = resp.drag_stopped();
    let offset = |d: f32| -> f32 {
        let a = d.abs();
        let s = if a <= 1.0 {
            a * first_step
        } else {
            first_step + (a - 1.0) * step
        };
        s * d.signum()
    };
    // The selected card's spot.
    let len = if horizontal {
        rect.width()
    } else {
        rect.height()
    };
    let anchor_along = match cfg.anchor {
        Anchor::Center => len / 2.0,
        Anchor::Start => len * 0.06 + along * sel_scale / 2.0,
    };
    let anchor_along = if dir < 0.0 {
        len - anchor_along
    } else {
        anchor_along
    };
    let cross_at = if horizontal {
        rect.top() + rect.height() * if cfg.reflection { 0.42 } else { 0.5 }
    } else {
        rect.center().x
    };
    let origin = if horizontal {
        pos2(rect.left() + anchor_along, cross_at)
    } else {
        pos2(cross_at, rect.top() + anchor_along)
    };
    let axis_v = if horizontal {
        vec2(dir, 0.0)
    } else {
        vec2(0.0, dir)
    };
    let reach = (len / step.max(1.0)).ceil() as i32 + 2;
    let lo = (pos.floor() as i32 - reach).max(0);
    let hi = (pos.ceil() as i32 + reach).min(games.len() as i32 - 1);
    let painter = ui.painter_at(rect);
    let mut order: Vec<(usize, f32)> = (lo..=hi).map(|i| (i as usize, i as f32 - pos)).collect();
    order.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
    let mut hits: Vec<(usize, Rect)> = Vec::new();
    for (i, d) in order {
        let g = games[i];
        let a = d.abs().min(1.0);
        let scale = 1.0 + (sel_scale - 1.0) * (1.0 - d.abs()).max(0.0);
        let angle = match cfg.tilt_mode {
            TiltMode::Flow => -tilt * d.clamp(-1.0, 1.0) * dir,
            TiltMode::Fan => tilt * a,
        };
        let center = origin + axis_v * offset(d);
        let pose = Pose {
            center,
            size: size * scale,
            angle: if horizontal { angle } else { -angle },
            axis: if horizontal {
                Axis::Upright
            } else {
                Axis::Level
            },
            radius: cfg.rounding * fit * scale,
        };
        // Cards before the first spot fade out.
        let mut alpha = 1.0;
        if cfg.anchor == Anchor::Start && d < 0.0 {
            alpha = (1.0 + d * 1.4).clamp(0.0, 1.0);
        }
        if alpha <= 0.01 {
            continue;
        }
        let light = 1.0 - cfg.dim * a;
        let tint = Color32::from_gray((light * 255.0) as u8).gamma_multiply(alpha);
        let (fill, has_art) = fill_for(images, g, cfg.shape, pose.size);
        if cfg.reflection && horizontal {
            card::draw(&painter, &pose, &fill, tint, Some(pose.size.y * 0.04));
        }
        card::shadow(&painter, &pose, alpha);
        if d.abs() < 0.5 {
            card::glow(
                &painter,
                &pose,
                look.accent.0,
                look.accent.1,
                cfg.glow,
                (0.75 + 0.25 * look.pulse) * (1.0 - d.abs() * 2.0),
            );
        }
        card::draw(&painter, &pose, &fill, tint, None);
        let bounds = pose.bounds();
        if !has_art {
            title_on(&painter, images, g, bounds, alpha * light);
        }
        if cfg.titles {
            let selected = i == sel;
            let font = FontId::proportional(if selected { 17.0 } else { 13.0 });
            let color = Color32::from_gray(if selected { 240 } else { 190 })
                .gamma_multiply(alpha * (1.0 - 0.5 * a));
            let width = if horizontal {
                bounds.width()
            } else {
                rect.right() - bounds.right() - 16.0
            };
            let galley = painter.layout(g.title.clone(), font, color, width.max(40.0));
            let (at, text) = if horizontal {
                (
                    pos2(
                        bounds.center().x - galley.size().x / 2.0,
                        bounds.bottom() + 8.0,
                    ),
                    galley,
                )
            } else {
                (
                    pos2(
                        bounds.right() + 16.0,
                        bounds.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                )
            };
            if !(horizontal && cfg.reflection) || selected {
                painter.galley(at, text, color);
            }
        }
        if i == sel {
            out.selected = Some(bounds);
        }
        hits.push((i, bounds));
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        // The topmost card under the pointer (last drawn).
        out.clicked = hits
            .iter()
            .rev()
            .find(|(_, r)| r.contains(p))
            .map(|(i, _)| *i);
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        let s = if scroll.y.abs() > scroll.x.abs() {
            -scroll.y
        } else {
            -scroll.x
        };
        out.scroll = s;
    }
    out
}
