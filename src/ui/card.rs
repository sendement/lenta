//! A card drawn as a mesh: a rounded rectangle turned in 3D (around the
//! upright axis on a horizontal ribbon, the level one on a vertical one) and
//! seen in perspective, with its glow, shadow and mirror image.

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Painter, Pos2, Rect, TextureId, Vec2, pos2, vec2};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Turns left/right (horizontal ribbon).
    Upright,
    /// Tips forward/back (vertical ribbon).
    Level,
}

#[derive(Clone, Copy)]
pub struct Pose {
    pub center: Pos2,
    /// Size on screen before turning.
    pub size: Vec2,
    /// Radians.
    pub angle: f32,
    pub axis: Axis,
    pub radius: f32,
}

impl Pose {
    pub fn flat(rect: Rect, radius: f32) -> Pose {
        Pose {
            center: rect.center(),
            size: rect.size(),
            angle: 0.0,
            axis: Axis::Upright,
            radius,
        }
    }

    fn focal(&self) -> f32 {
        self.size.x.max(self.size.y) * 2.6
    }

    /// A point of the card (from its middle) on screen.
    pub fn project(&self, p: Vec2) -> Pos2 {
        let (s, c) = self.angle.sin_cos();
        let f = self.focal();
        let (x, y, z) = match self.axis {
            Axis::Upright => (p.x * c, p.y, p.x * s),
            Axis::Level => (p.x, p.y * c, p.y * s),
        };
        let k = f / (f + z).max(f * 0.2);
        self.center + vec2(x * k, y * k)
    }

    /// The screen rectangle around the turned card.
    pub fn bounds(&self) -> Rect {
        let h = self.size / 2.0;
        let pts = [
            vec2(-h.x, -h.y),
            vec2(h.x, -h.y),
            vec2(h.x, h.y),
            vec2(-h.x, h.y),
        ]
        .map(|p| self.project(p));
        Rect::from_points(&pts)
    }

    /// The outline (from the middle), clockwise, with outward normals;
    /// straight sides cut into pieces so the perspective stays smooth.
    pub fn outline(&self) -> Vec<(Vec2, Vec2)> {
        let h = self.size / 2.0;
        let r = self.radius.clamp(0.0, h.x.min(h.y));
        let corners = [
            (vec2(-h.x + r, -h.y + r), std::f32::consts::PI),
            (vec2(h.x - r, -h.y + r), 1.5 * std::f32::consts::PI),
            (vec2(h.x - r, h.y - r), 0.0),
            (vec2(-h.x + r, h.y - r), 0.5 * std::f32::consts::PI),
        ];
        const ARC: usize = 8;
        const SIDE: usize = 6;
        let mut out = Vec::with_capacity(4 * (ARC + SIDE));
        for i in 0..4 {
            let (c, a0) = corners[i];
            for k in 0..=ARC {
                let a = a0 + 0.5 * std::f32::consts::PI * k as f32 / ARC as f32;
                let n = vec2(a.cos(), a.sin());
                out.push((c + n * r, n));
            }
            // The straight side to the next corner.
            let (next, b0) = corners[(i + 1) % 4];
            let from = c + vec2(
                (a0 + 0.5 * std::f32::consts::PI).cos(),
                (a0 + 0.5 * std::f32::consts::PI).sin(),
            ) * r;
            let to = next + vec2(b0.cos(), b0.sin()) * r;
            let n = vec2(
                (a0 + 0.5 * std::f32::consts::PI).cos(),
                (a0 + 0.5 * std::f32::consts::PI).sin(),
            );
            for k in 1..SIDE {
                let t = k as f32 / SIDE as f32;
                out.push((from + (to - from) * t, n));
            }
        }
        out
    }
}

/// What fills the card.
pub enum Fill {
    /// A texture and the part of it to show.
    Image(TextureId, Rect),
    /// No art: a gradient.
    Gradient(Color32, Color32),
}

/// The part of a `tex`-sized picture that covers a card of `card` size.
pub fn cover_uv(tex: Vec2, card: Vec2) -> Rect {
    let (ta, ca) = (tex.x / tex.y.max(1.0), card.x / card.y.max(1.0));
    if ta > ca {
        let w = ca / ta;
        Rect::from_min_size(pos2((1.0 - w) / 2.0, 0.0), vec2(w, 1.0))
    } else {
        let h = ta / ca;
        // Keep the top (titles and faces sit there more often).
        Rect::from_min_size(pos2(0.0, (1.0 - h) * 0.3), vec2(1.0, h))
    }
}

const RINGS: [f32; 4] = [0.3, 0.6, 0.85, 1.0];

/// The card's face. `tint` multiplies it (dimming, fading). `mirror`
/// draws its reflection under it instead (horizontal ribbons).
pub fn draw(painter: &Painter, pose: &Pose, fill: &Fill, tint: Color32, mirror: Option<f32>) {
    let outline = pose.outline();
    let h = pose.size / 2.0;
    let (tex, uv, c1, c2) = match fill {
        Fill::Image(t, uv) => (*t, *uv, Color32::WHITE, Color32::WHITE),
        Fill::Gradient(a, b) => (
            TextureId::default(),
            Rect::from_min_size(egui::epaint::WHITE_UV, Vec2::ZERO),
            *a,
            *b,
        ),
    };
    let mut mesh = Mesh::with_texture(tex);
    let vertex = |p: Vec2| -> Vertex {
        let t = ((p.x / pose.size.x + 0.5) * 0.4 + (p.y / pose.size.y + 0.5) * 0.6).clamp(0.0, 1.0);
        let mut color = mul(lerp(c1, c2, t), tint);
        let mut at = p;
        if let Some(gap) = mirror {
            // Mirrored under the bottom edge, fading out downwards.
            at = vec2(p.x, pose.size.y + gap - p.y);
            let below = (h.y - p.y) / pose.size.y;
            let fade = (1.0 - below / 0.5).clamp(0.0, 1.0);
            color = color.gamma_multiply(0.42 * fade.powf(1.6));
        }
        let u = uv.min.x + (p.x / pose.size.x + 0.5) * uv.width();
        let v = uv.min.y + (p.y / pose.size.y + 0.5) * uv.height();
        Vertex {
            pos: pose.project(at),
            uv: pos2(u, v),
            color,
        }
    };
    mesh.vertices.push(vertex(Vec2::ZERO));
    let n = outline.len() as u32;
    for &t in &RINGS {
        for (p, _) in &outline {
            mesh.vertices.push(vertex(*p * t));
        }
    }
    // A clear ring just outside: the edge fades over a pixel instead of
    // stepping.
    for (p, nrm) in &outline {
        let mut v = vertex(*p + *nrm * FEATHER);
        v.color = Color32::TRANSPARENT;
        mesh.vertices.push(v);
    }
    // Fan around the middle, then strips between rings.
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    for r in 0..RINGS.len() as u32 {
        let a = 1 + r * n;
        let b = a + n;
        for i in 0..n {
            let j = (i + 1) % n;
            mesh.add_triangle(a + i, b + i, b + j);
            mesh.add_triangle(a + i, b + j, a + j);
        }
    }
    painter.add(mesh);
}

/// Rings around the card, each `(distance out, alpha)`, coloured by
/// `color(position on the card 0…1)` and blended between neighbours.
/// `fill` also covers the inside with the first ring's alpha.
fn rings(
    painter: &Painter,
    pose: &Pose,
    rings: &[(f32, f32)],
    shift: Vec2,
    fill: bool,
    color: &dyn Fn(f32) -> Color32,
) {
    let outline = pose.outline();
    let n = outline.len() as u32;
    let mut mesh = Mesh::default();
    for &(out, alpha) in rings {
        for (p, nrm) in &outline {
            let t = ((p.x / pose.size.x + 0.5) * 0.5 + (p.y / pose.size.y + 0.5) * 0.5)
                .clamp(0.0, 1.0);
            mesh.colored_vertex(
                pose.project(*p + *nrm * out) + shift,
                color(t).gamma_multiply(alpha),
            );
        }
    }
    for r in 0..rings.len() as u32 - 1 {
        let (a, b) = (r * n, (r + 1) * n);
        for i in 0..n {
            let j = (i + 1) % n;
            mesh.add_triangle(a + i, b + i, b + j);
            mesh.add_triangle(a + i, b + j, a + j);
        }
    }
    if fill {
        let mid = mesh.vertices.len() as u32;
        mesh.colored_vertex(
            pose.project(Vec2::ZERO) + shift,
            color(0.5).gamma_multiply(rings[0].1),
        );
        for i in 0..n {
            mesh.add_triangle(mid, i, (i + 1) % n);
        }
    }
    painter.add(mesh);
}

/// Width of the soft edge that stands in for anti-aliasing on meshes.
const FEATHER: f32 = 1.0;

pub fn shadow(painter: &Painter, pose: &Pose, strength: f32) {
    let s = pose.size.y.min(pose.size.x) * 0.06;
    // Filled, so that moving it down leaves no light gap under the card.
    rings(
        painter,
        pose,
        &[(-2.0, 0.55 * strength), (s * 2.2, 0.0)],
        vec2(0.0, s * 0.6),
        true,
        &|_| Color32::BLACK,
    );
}

/// The selected card's frame and glow in the accent colours.
pub fn glow(painter: &Painter, pose: &Pose, c1: Color32, c2: Color32, glow: f32, pulse: f32) {
    let color = |t: f32| lerp(c1, c2, t);
    if glow > 0.0 {
        let w = pose.size.y.min(pose.size.x) * 0.16 * glow;
        rings(
            painter,
            pose,
            &[(2.0, 0.55 * pulse), (2.0 + w, 0.0)],
            Vec2::ZERO,
            false,
            &color,
        );
    }
    rings(
        painter,
        pose,
        &[
            (-0.5 - FEATHER, 0.0),
            (-0.5, 1.0),
            (3.0, 1.0),
            (3.0 + FEATHER, 0.0),
        ],
        Vec2::ZERO,
        false,
        &color,
    );
}

pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
        f(a.a(), b.a()),
    )
}

pub fn mul(a: Color32, b: Color32) -> Color32 {
    let f = |x: u8, y: u8| ((x as u16 * y as u16 + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
        f(a.a(), b.a()),
    )
}
