//! Small drawn things: button glyphs for the hints, the side panel's icons,
//! bars and chips.

use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2, vec2,
};

use crate::game::Source;
use crate::input::PadKind;

/// Each launcher's own colours, for its tab and badges: Steam's blue,
/// Heroic's violet, Lutris's orange.
pub fn source_colors(s: Source) -> (Color32, Color32) {
    match s {
        Source::Steam => (
            Color32::from_rgb(0x1a, 0x9f, 0xff),
            Color32::from_rgb(0x1a, 0x44, 0xc2),
        ),
        Source::Heroic => (
            Color32::from_rgb(0xb0, 0x5c, 0xff),
            Color32::from_rgb(0x4f, 0x3b, 0xd6),
        ),
        Source::Lutris => (
            Color32::from_rgb(0xff, 0xa2, 0x1c),
            Color32::from_rgb(0xe8, 0x50, 0x1e),
        ),
    }
}

/// A rounded rectangle filled left to right from `a` to `b`, its edge
/// softened over a pixel.
pub fn gradient_pill(p: &Painter, r: Rect, radius: f32, a: Color32, b: Color32) {
    use std::f32::consts::FRAC_PI_2;
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let corners = [
        (pos2(r.right() - rad, r.bottom() - rad), 0.0),
        (pos2(r.left() + rad, r.bottom() - rad), FRAC_PI_2),
        (pos2(r.left() + rad, r.top() + rad), 2.0 * FRAC_PI_2),
        (pos2(r.right() - rad, r.top() + rad), 3.0 * FRAC_PI_2),
    ];
    let mut edge: Vec<(Pos2, Vec2)> = Vec::new();
    for (c, a0) in corners {
        for k in 0..=8 {
            let ang = a0 + FRAC_PI_2 * k as f32 / 8.0;
            let n = vec2(ang.cos(), ang.sin());
            edge.push((c + n * rad, n));
        }
    }
    let color = |x: f32| {
        let t = ((x - r.left()) / r.width().max(1.0)).clamp(0.0, 1.0);
        crate::ui::card::lerp(a, b, t)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.center(), color(r.center().x));
    for (q, _) in &edge {
        mesh.colored_vertex(*q, color(q.x));
    }
    for (q, n) in &edge {
        mesh.colored_vertex(*q + *n * 1.0, Color32::TRANSPARENT);
    }
    let n = edge.len() as u32;
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.add_triangle(0, 1 + i, 1 + j);
        mesh.add_triangle(1 + i, 1 + n + i, 1 + n + j);
        mesh.add_triangle(1 + i, 1 + n + j, 1 + j);
    }
    p.add(mesh);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Btn {
    A,
    B,
    X,
    Y,
    LB,
    LT,
    View,
    Menu,
    Dpad,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyphs {
    Pad(PadKind),
    Keys,
}

fn key_name(b: Btn) -> &'static str {
    match b {
        Btn::A => "Enter",
        Btn::B => "Esc",
        Btn::X => "Space",
        Btn::Y => "F",
        Btn::LB => "Q / E",
        Btn::LT => "PgUp / PgDn",
        Btn::View => "Tab",
        Btn::Menu => "F2",
        Btn::Dpad => "↑↓←→",
    }
}

/// The button's glyph, `h` tall, from `left`; returns its width.
pub fn glyph(p: &Painter, left: Pos2, h: f32, b: Btn, g: Glyphs) -> f32 {
    let text = Color32::from_gray(235);
    let dark = Color32::from_rgba_unmultiplied(255, 255, 255, 34);
    let pill = |label: &str, p: &Painter| -> f32 {
        let font = FontId::proportional(h * 0.5);
        let w = p
            .layout_no_wrap(label.to_string(), font.clone(), text)
            .size()
            .x
            + h * 0.6;
        let w = w.max(h);
        let r = Rect::from_min_size(left, vec2(w, h));
        p.rect(
            r,
            h * 0.3,
            dark,
            Stroke::new(1.0, Color32::from_gray(150)),
            StrokeKind::Inside,
        );
        p.text(r.center(), Align2::CENTER_CENTER, label, font, text);
        w
    };
    let face = |p: &Painter, fill: Color32| -> Pos2 {
        let c = left + vec2(h / 2.0, h / 2.0);
        p.circle_filled(c, h / 2.0, fill);
        c
    };
    match g {
        Glyphs::Keys => pill(key_name(b), p),
        Glyphs::Pad(kind) => match b {
            Btn::A | Btn::B | Btn::X | Btn::Y => {
                match kind {
                    PadKind::Xbox => {
                        let (fill, l) = match b {
                            Btn::A => (Color32::from_rgb(0x3d, 0xa0, 0x4a), "A"),
                            Btn::B => (Color32::from_rgb(0xd0, 0x3b, 0x3b), "B"),
                            Btn::X => (Color32::from_rgb(0x2f, 0x6f, 0xd6), "X"),
                            _ => (Color32::from_rgb(0xe0, 0xb0, 0x2a), "Y"),
                        };
                        let c = face(p, fill);
                        p.text(
                            c,
                            Align2::CENTER_CENTER,
                            l,
                            FontId::proportional(h * 0.58),
                            Color32::WHITE,
                        );
                    }
                    PadKind::PlayStation => {
                        let c = face(p, Color32::from_gray(30));
                        let r = h * 0.24;
                        let st = |col: Color32| Stroke::new(h * 0.08, col);
                        match b {
                            Btn::A => {
                                let s = st(Color32::from_rgb(0x8f, 0xa8, 0xff));
                                p.line_segment([c + vec2(-r, -r), c + vec2(r, r)], s);
                                p.line_segment([c + vec2(-r, r), c + vec2(r, -r)], s);
                            }
                            Btn::B => {
                                p.circle_stroke(c, r, st(Color32::from_rgb(0xff, 0x6b, 0x6b)));
                            }
                            Btn::X => {
                                p.rect_stroke(
                                    Rect::from_center_size(c, Vec2::splat(r * 1.8)),
                                    0.0,
                                    st(Color32::from_rgb(0xf0, 0x8c, 0xd0)),
                                    StrokeKind::Middle,
                                );
                            }
                            _ => {
                                let pts = vec![
                                    c + vec2(0.0, -r * 1.05),
                                    c + vec2(r * 1.0, r * 0.75),
                                    c + vec2(-r * 1.0, r * 0.75),
                                ];
                                p.add(Shape::closed_line(
                                    pts,
                                    st(Color32::from_rgb(0x4f, 0xd6, 0xb4)),
                                ));
                            }
                        }
                    }
                }
                h
            }
            Btn::LB | Btn::LT => {
                let l = match (kind, b) {
                    (PadKind::PlayStation, Btn::LB) => "L1 / R1",
                    (PadKind::PlayStation, Btn::LT) => "L2 / R2",
                    (PadKind::PlayStation, _) => "R2",
                    (_, Btn::LB) => "LB / RB",
                    (_, Btn::LT) => "LT / RT",
                    _ => "RT",
                };
                pill(l, p)
            }
            Btn::View | Btn::Menu => {
                let w = h * 1.3;
                let r = Rect::from_min_size(left, vec2(w, h));
                p.rect(
                    r,
                    h * 0.5,
                    dark,
                    Stroke::new(1.0, Color32::from_gray(150)),
                    StrokeKind::Inside,
                );
                let c = r.center();
                let s = Stroke::new(h * 0.07, text);
                if b == Btn::Menu {
                    for k in [-1.0, 0.0, 1.0] {
                        p.line_segment(
                            [
                                c + vec2(-h * 0.22, k * h * 0.15),
                                c + vec2(h * 0.22, k * h * 0.15),
                            ],
                            s,
                        );
                    }
                } else {
                    let a = Rect::from_center_size(
                        c + vec2(-h * 0.06, -h * 0.06),
                        Vec2::splat(h * 0.3),
                    );
                    p.rect_stroke(a, 1.0, s, StrokeKind::Middle);
                    p.rect_stroke(
                        a.translate(vec2(h * 0.13, h * 0.13)),
                        1.0,
                        s,
                        StrokeKind::Middle,
                    );
                }
                w
            }
            Btn::Dpad => {
                let c = left + vec2(h / 2.0, h / 2.0);
                let a = h * 0.17;
                let l = h * 0.48;
                let fill = Color32::from_gray(210);
                p.rect_filled(Rect::from_center_size(c, vec2(2.0 * a, 2.0 * l)), 2.0, fill);
                p.rect_filled(Rect::from_center_size(c, vec2(2.0 * l, 2.0 * a)), 2.0, fill);
                h
            }
        },
    }
}

/// Hints along the bottom: glyph and words, right-aligned in `rect`.
pub fn hints(p: &Painter, rect: Rect, items: &[(Btn, &str)], g: Glyphs) {
    let h = (rect.height() * 0.62).clamp(18.0, 30.0);
    let font = FontId::proportional(h * 0.62);
    let mut x = rect.right() - 12.0;
    for (b, label) in items.iter().rev() {
        let galley = p.layout_no_wrap(label.to_string(), font.clone(), Color32::from_gray(225));
        x -= galley.size().x;
        let y = rect.center().y;
        p.galley(pos2(x, y - galley.size().y / 2.0), galley, Color32::WHITE);
        x -= 8.0;
        // Measure the glyph by drawing it off-screen first.
        let w = glyph_width(p, h, *b, g);
        x -= w;
        glyph(p, pos2(x, y - h / 2.0), h, *b, g);
        x -= 22.0;
    }
}

fn glyph_width(p: &Painter, h: f32, b: Btn, g: Glyphs) -> f32 {
    let label = match g {
        Glyphs::Keys => key_name(b),
        Glyphs::Pad(kind) => match b {
            Btn::A | Btn::B | Btn::X | Btn::Y | Btn::Dpad => return h,
            Btn::View | Btn::Menu => return h * 1.3,
            Btn::LB => {
                if kind == PadKind::PlayStation {
                    "L1 / R1"
                } else {
                    "LB / RB"
                }
            }
            Btn::LT => {
                if kind == PadKind::PlayStation {
                    "L2 / R2"
                } else {
                    "LT / RT"
                }
            }
        },
    };
    (p.layout_no_wrap(
        label.to_string(),
        FontId::proportional(h * 0.5),
        Color32::WHITE,
    )
    .size()
    .x + h * 0.6)
        .max(h)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Home,
    Library,
    System,
    Settings,
    About,
    Proton,
    Sound,
    Play,
    Clock,
    Trophy,
    /// Modules: four blocks, one set apart.
    Module,
    Wifi,
    Bluetooth,
    /// Three sliders: system settings.
    Sliders,
}

pub fn icon(p: &Painter, c: Pos2, s: f32, icon: Icon, color: Color32) {
    let st = Stroke::new((s * 0.09).max(1.5), color);
    let h = s / 2.0;
    match icon {
        Icon::Home => {
            let roof = vec![
                c + vec2(-h, -h * 0.05),
                c + vec2(0.0, -h * 0.9),
                c + vec2(h, -h * 0.05),
            ];
            p.add(Shape::line(roof, st));
            let body = vec![
                c + vec2(-h * 0.72, -h * 0.25),
                c + vec2(-h * 0.72, h * 0.85),
                c + vec2(h * 0.72, h * 0.85),
                c + vec2(h * 0.72, -h * 0.25),
            ];
            p.add(Shape::line(body, st));
            p.rect_stroke(
                Rect::from_min_max(c + vec2(-h * 0.2, h * 0.25), c + vec2(h * 0.2, h * 0.85)),
                1.0,
                st,
                StrokeKind::Middle,
            );
        }
        Icon::Library => {
            let w = s * 0.24;
            for k in [-1.0, 0.0, 1.0] {
                let r = Rect::from_center_size(c + vec2(k * w * 1.25, 0.0), vec2(w, s * 0.8));
                p.rect_stroke(r, 2.0, st, StrokeKind::Middle);
            }
        }
        Icon::System => {
            let d = s * 0.32;
            for y in [-1.0, 0.0, 1.0] {
                for x in [-1.0, 0.0, 1.0] {
                    p.circle_filled(c + vec2(x * d, y * d), s * 0.085, color);
                }
            }
        }
        Icon::Settings => {
            let teeth = 8;
            let mut pts = Vec::new();
            for i in 0..teeth * 4 {
                let a = i as f32 / (teeth * 4) as f32 * std::f32::consts::TAU;
                let r = if (i / 2) % 2 == 0 { h * 0.92 } else { h * 0.68 };
                pts.push(c + vec2(a.cos(), a.sin()) * r);
            }
            p.add(Shape::closed_line(pts, st));
            p.circle_stroke(c, h * 0.3, st);
        }
        Icon::About => {
            p.circle_stroke(c, h * 0.9, st);
            p.circle_filled(c + vec2(0.0, -h * 0.4), s * 0.06, color);
            p.line_segment([c + vec2(0.0, -h * 0.1), c + vec2(0.0, h * 0.5)], st);
        }
        Icon::Sliders => {
            for (k, at) in [(-1.0f32, 0.35f32), (0.0, -0.4), (1.0, 0.15)] {
                let y = c.y + k * h * 0.62;
                p.add(Shape::line_segment(
                    [pos2(c.x - h * 0.85, y), pos2(c.x + h * 0.85, y)],
                    st,
                ));
                p.circle_filled(pos2(c.x + at * h, y), st.width * 1.9, st.color);
            }
        }
        Icon::Wifi => {
            // Three arcs over a dot.
            let base = c + vec2(0.0, h * 0.7);
            for r in [h * 0.55, h * 1.0, h * 1.45] {
                let arc: Vec<Pos2> = (0..=10)
                    .map(|k| {
                        let a = -std::f32::consts::FRAC_PI_2 - 0.75 + 1.5 * k as f32 / 10.0;
                        base + vec2(r * a.cos(), r * a.sin())
                    })
                    .collect();
                p.add(Shape::line(arc, st));
            }
            p.circle_filled(base, st.width * 0.9, st.color);
        }
        Icon::Bluetooth => {
            // The rune: a spine and two loops to the right.
            let x = h * 0.42;
            let pts = vec![
                c + vec2(-x, -h * 0.45),
                c + vec2(x, h * 0.42),
                c + vec2(0.0, h * 0.9),
                c + vec2(0.0, -h * 0.9),
                c + vec2(x, -h * 0.42),
                c + vec2(-x, h * 0.45),
            ];
            p.add(Shape::line(pts, st));
        }
        Icon::Module => {
            let b = h * 0.78;
            let g = h * 0.16;
            for (dx, dy, lift) in [
                (-1.0, -1.0, 0.0),
                (-1.0, 1.0, 0.0),
                (1.0, 1.0, 0.0),
                (1.0, -1.0, 1.0),
            ] {
                let o = c + vec2(
                    dx * (b / 2.0 + g / 2.0) + lift * g,
                    dy * (b / 2.0 + g / 2.0) - lift * g,
                );
                p.rect_stroke(
                    Rect::from_center_size(o, vec2(b, b)),
                    b * 0.22,
                    st,
                    egui::StrokeKind::Middle,
                );
            }
        }
        Icon::Sound => {
            // A speaker and two waves.
            let body = vec![
                c + vec2(-h * 0.85, -h * 0.3),
                c + vec2(-h * 0.45, -h * 0.3),
                c + vec2(0.0, -h * 0.75),
                c + vec2(0.0, h * 0.75),
                c + vec2(-h * 0.45, h * 0.3),
                c + vec2(-h * 0.85, h * 0.3),
            ];
            p.add(Shape::closed_line(body, st));
            for r in [h * 0.45, h * 0.85] {
                let arc: Vec<Pos2> = (0..=8)
                    .map(|k| {
                        let a = -0.8 + 1.6 * k as f32 / 8.0;
                        c + vec2(h * 0.1 + r * a.cos(), r * a.sin())
                    })
                    .collect();
                p.add(Shape::line(arc, st));
            }
        }
        Icon::Proton => {
            // A flask.
            let neck = h * 0.32;
            let pts = vec![
                c + vec2(-neck, -h * 0.9),
                c + vec2(-neck, -h * 0.25),
                c + vec2(-h * 0.85, h * 0.75),
                c + vec2(h * 0.85, h * 0.75),
                c + vec2(neck, -h * 0.25),
                c + vec2(neck, -h * 0.9),
            ];
            p.add(Shape::line(pts, st));
            p.line_segment(
                [
                    c + vec2(-neck * 1.4, -h * 0.9),
                    c + vec2(neck * 1.4, -h * 0.9),
                ],
                st,
            );
            p.line_segment(
                [c + vec2(-h * 0.5, h * 0.25), c + vec2(h * 0.5, h * 0.25)],
                st,
            );
        }
        Icon::Play => {
            let pts = vec![
                c + vec2(-h * 0.55, -h * 0.75),
                c + vec2(h * 0.8, 0.0),
                c + vec2(-h * 0.55, h * 0.75),
            ];
            p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Clock => {
            p.circle_stroke(c, h * 0.85, st);
            p.add(Shape::line(
                vec![c + vec2(0.0, -h * 0.5), c, c + vec2(h * 0.38, h * 0.2)],
                st,
            ));
        }
        Icon::Trophy => {
            let cup = vec![
                c + vec2(-h * 0.55, -h * 0.8),
                c + vec2(-h * 0.5, -h * 0.1),
                c + vec2(0.0, h * 0.25),
                c + vec2(h * 0.5, -h * 0.1),
                c + vec2(h * 0.55, -h * 0.8),
            ];
            p.add(Shape::closed_line(cup, st));
            p.line_segment([c + vec2(0.0, h * 0.25), c + vec2(0.0, h * 0.7)], st);
            p.line_segment(
                [c + vec2(-h * 0.4, h * 0.8), c + vec2(h * 0.4, h * 0.8)],
                st,
            );
        }
    }
}

/// A thin progress bar.
pub fn bar(p: &Painter, rect: Rect, frac: f32, c1: Color32, c2: Color32) {
    p.rect_filled(rect, rect.height() / 2.0, Color32::from_white_alpha(30));
    let w = rect.width() * frac.clamp(0.0, 1.0);
    if w > 0.5 {
        gradient_pill(
            p,
            Rect::from_min_size(rect.min, vec2(w, rect.height())),
            rect.height() / 2.0,
            c1,
            c2,
        );
    }
}

/// A rectangle filled left to right from `a` to `b`.
pub fn gradient_rect(p: &Painter, r: Rect, a: Color32, b: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), a);
    mesh.colored_vertex(r.right_top(), b);
    mesh.colored_vertex(r.right_bottom(), b);
    mesh.colored_vertex(r.left_bottom(), a);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
}

/// A rectangle fading from `top` to `bottom`.
pub fn vgradient(p: &Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), top);
    mesh.colored_vertex(r.right_top(), top);
    mesh.colored_vertex(r.right_bottom(), bottom);
    mesh.colored_vertex(r.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
}

/// A rounded label.
/// A label on a pill filled with the gradient `fill`.
pub fn chip(p: &Painter, at: Pos2, text: &str, fill: (Color32, Color32), size: f32) -> Rect {
    let galley = p.layout_no_wrap(text.to_string(), FontId::proportional(size), Color32::WHITE);
    let r = Rect::from_min_size(at, galley.size() + vec2(size * 1.1, size * 0.6));
    gradient_pill(p, r, r.height() / 2.0, fill.0, fill.1);
    p.galley(
        r.min + vec2(size * 0.55, size * 0.3),
        galley,
        Color32::WHITE,
    );
    r
}
