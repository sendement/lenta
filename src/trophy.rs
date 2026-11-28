//! A game's grade by how many of its achievements are earned: bronze,
//! silver, gold medals on the way and a platinum cup for all of them.
//! Shown on the cards and the Achievements tab; each game's tally is
//! kept on disk so cards have it before their page is opened.

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Color32, Painter, Pos2, Shape, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::sources::steam::Achievement;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
    Platinum,
}

impl Tier {
    /// The share of achievements each grade starts at.
    pub fn from(share: f32) -> Option<Tier> {
        if share >= 1.0 {
            Some(Tier::Platinum)
        } else if share >= 0.75 {
            Some(Tier::Gold)
        } else if share >= 0.5 {
            Some(Tier::Silver)
        } else if share >= 0.25 {
            Some(Tier::Bronze)
        } else {
            None
        }
    }

    pub fn next(self) -> Option<(Tier, f32)> {
        match self {
            Tier::Bronze => Some((Tier::Silver, 0.5)),
            Tier::Silver => Some((Tier::Gold, 0.75)),
            Tier::Gold => Some((Tier::Platinum, 1.0)),
            Tier::Platinum => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tier::Bronze => "tr_bronze",
            Tier::Silver => "tr_silver",
            Tier::Gold => "tr_gold",
            Tier::Platinum => "tr_platinum",
        }
    }

    /// (light, dark) of the metal.
    pub fn colors(self) -> (Color32, Color32) {
        match self {
            Tier::Bronze => (
                Color32::from_rgb(230, 160, 105),
                Color32::from_rgb(140, 78, 40),
            ),
            Tier::Silver => (
                Color32::from_rgb(235, 240, 245),
                Color32::from_rgb(128, 138, 150),
            ),
            Tier::Gold => (
                Color32::from_rgb(255, 222, 110),
                Color32::from_rgb(178, 120, 20),
            ),
            Tier::Platinum => (
                Color32::from_rgb(225, 240, 255),
                Color32::from_rgb(110, 140, 185),
            ),
        }
    }
}

/// How far a game is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tally {
    pub got: u32,
    pub total: u32,
}

impl Tally {
    pub fn of(list: &[Achievement]) -> Tally {
        Tally {
            got: list.iter().filter(|a| a.unlocked.is_some()).count() as u32,
            total: list.len() as u32,
        }
    }

    pub fn share(self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.got as f32 / self.total as f32
        }
    }

    pub fn tier(self) -> Option<Tier> {
        if self.total == 0 {
            None
        } else {
            Tier::from(self.share())
        }
    }

    /// The next grade and how many more achievements it takes.
    pub fn to_next(self) -> Option<(Tier, u32)> {
        if self.total == 0 {
            return None;
        }
        let (t, at) = match self.tier() {
            None => (Tier::Bronze, 0.25),
            Some(t) => t.next()?,
        };
        let need = (at * self.total as f32).ceil() as u32;
        Some((t, need.saturating_sub(self.got).max(1)))
    }
}

fn file() -> PathBuf {
    crate::net::cache_dir().join("trophies.json")
}

pub fn load() -> HashMap<String, Tally> {
    std::fs::read(file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save(all: &HashMap<String, Tally>) {
    let f = file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(s) = serde_json::to_vec(all) {
        let tmp = f.with_extension("json.tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(tmp, f);
        }
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A disc lit from the top left: light to dark.
fn shaded_disc(p: &Painter, c: Pos2, r: f32, light: Color32, dark: Color32) {
    let mut mesh = egui::Mesh::default();
    let n = 40;
    mesh.colored_vertex(c + vec2(-r * 0.25, -r * 0.3), light);
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let d = vec2(a.cos(), a.sin());
        // Lit side towards the top left.
        let t = ((d.x + d.y) * 0.5 + 0.5).clamp(0.0, 1.0);
        mesh.colored_vertex(c + d * r, mix(light, dark, 0.35 + 0.65 * t));
    }
    for i in 0..n {
        mesh.add_triangle(0, i + 1, i + 2);
    }
    p.add(Shape::mesh(mesh));
}

/// A medal: ribbon, rim, face and a star.
pub fn medal(p: &Painter, c: Pos2, size: f32, tier: Tier, alpha: f32) {
    let (light, dark) = tier.colors();
    let (light, dark) = (light.gamma_multiply(alpha), dark.gamma_multiply(alpha));
    let r = size * 0.36;
    let mc = c + vec2(0.0, size * 0.12);
    // Ribbon: two tails up from the disc.
    let rib_a = Color32::from_rgb(70, 110, 200).gamma_multiply(alpha);
    let rib_b = Color32::from_rgb(200, 60, 70).gamma_multiply(alpha);
    let top = c.y - size * 0.5;
    let w = size * 0.16;
    p.add(Shape::convex_polygon(
        vec![
            pos2(c.x - size * 0.3, top),
            pos2(c.x - size * 0.3 + w, top),
            pos2(c.x + w * 0.4, mc.y - r * 0.6),
            pos2(c.x - w * 0.6, mc.y - r * 0.4),
        ],
        rib_a,
        Stroke::NONE,
    ));
    p.add(Shape::convex_polygon(
        vec![
            pos2(c.x + size * 0.3 - w, top),
            pos2(c.x + size * 0.3, top),
            pos2(c.x + w * 0.6, mc.y - r * 0.4),
            pos2(c.x - w * 0.4, mc.y - r * 0.6),
        ],
        rib_b,
        Stroke::NONE,
    ));
    p.circle_filled(
        mc + vec2(0.0, r * 0.06),
        r * 1.04,
        Color32::from_black_alpha((90.0 * alpha) as u8),
    );
    shaded_disc(p, mc, r, dark, light);
    shaded_disc(p, mc, r * 0.78, light, dark);
    star(p, mc, r * 0.5, dark.gamma_multiply(0.9));
}

fn star(p: &Painter, c: Pos2, r: f32, color: Color32) {
    let pts: Vec<Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let rr = if i % 2 == 0 { r } else { r * 0.45 };
            c + vec2(a.cos(), a.sin()) * rr
        })
        .collect();
    // Not convex: a fan from the centre.
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, color);
    for q in &pts {
        mesh.colored_vertex(*q, color);
    }
    for i in 0..10u32 {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % 10);
    }
    p.add(Shape::mesh(mesh));
}

/// The platinum cup: bowl, handles, stem and base.
pub fn cup(p: &Painter, c: Pos2, size: f32, alpha: f32) {
    let (light, dark) = Tier::Platinum.colors();
    let (light, dark) = (light.gamma_multiply(alpha), dark.gamma_multiply(alpha));
    let h = size / 2.0;
    let st = Stroke::new((size * 0.07).max(1.5), dark);
    // Handles.
    for s in [-1.0f32, 1.0] {
        let pts: Vec<Pos2> = (0..=8)
            .map(|i| {
                let a = i as f32 / 8.0 * std::f32::consts::PI;
                c + vec2(
                    s * (h * 0.52 + a.sin() * h * 0.28),
                    -h * 0.62 + (1.0 - a.cos()) * h * 0.26,
                )
            })
            .collect();
        p.add(Shape::line(pts, st));
    }
    // Bowl.
    let mut bowl = vec![c + vec2(-h * 0.6, -h * 0.75), c + vec2(h * 0.6, -h * 0.75)];
    for i in 0..=10 {
        let a = i as f32 / 10.0 * std::f32::consts::PI;
        bowl.push(c + vec2(h * 0.6 * a.cos(), -h * 0.75 + h * 0.75 * a.sin()));
    }
    let mut mesh = egui::Mesh::default();
    let mid = c + vec2(-h * 0.15, -h * 0.4);
    mesh.colored_vertex(mid, light);
    for q in &bowl {
        let t = ((q.x - c.x) / h * 0.5 + 0.5).clamp(0.0, 1.0);
        mesh.colored_vertex(*q, mix(light, dark, 0.2 + 0.8 * t));
    }
    for i in 0..bowl.len() as u32 - 1 {
        mesh.add_triangle(0, 1 + i, 2 + i);
    }
    p.add(Shape::mesh(mesh));
    // Stem and base.
    p.add(Shape::convex_polygon(
        vec![
            c + vec2(-h * 0.1, 0.0),
            c + vec2(h * 0.1, 0.0),
            c + vec2(h * 0.14, h * 0.5),
            c + vec2(-h * 0.14, h * 0.5),
        ],
        mix(light, dark, 0.5),
        Stroke::NONE,
    ));
    let base = egui::Rect::from_min_max(c + vec2(-h * 0.45, h * 0.5), c + vec2(h * 0.45, h * 0.78));
    p.rect_filled(base, h * 0.06, dark);
    p.rect_filled(base.shrink2(vec2(h * 0.08, h * 0.07)), h * 0.04, light);
    star(p, c + vec2(0.0, -h * 0.42), h * 0.2, dark);
}

/// The grade's mark: the cup or a medal.
pub fn mark(p: &Painter, c: Pos2, size: f32, tier: Tier, alpha: f32) {
    match tier {
        Tier::Platinum => cup(p, c, size, alpha),
        t => medal(p, c, size, t, alpha),
    }
}

/// The mark on a card's corner, on a dark round plate.
pub fn badge(p: &Painter, card: egui::Rect, tier: Tier, alpha: f32) {
    let s = (card.width().min(card.height()) * 0.2).clamp(24.0, 46.0);
    let c = pos2(card.right() - s * 0.62, card.top() + s * 0.62);
    p.circle_filled(
        c,
        s * 0.56,
        Color32::from_black_alpha((205.0 * alpha) as u8),
    );
    p.circle_stroke(
        c,
        s * 0.56,
        Stroke::new(1.5, tier.colors().0.gamma_multiply(alpha * 0.7)),
    );
    // The medal's disc hangs below its ribbon: lift it to the middle.
    let lift = if tier == Tier::Platinum {
        0.0
    } else {
        s * 0.06
    };
    mark(p, c - vec2(0.0, lift), s * 0.78, tier, alpha);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_names() {
        use crate::sources::steam::unhex;
        assert_eq!(unhex("e8 a8 98 e6 86 b6".into()), "記憶");
        assert_eq!(unhex("ab cd".into()), "ab cd");
        assert_eq!(unhex("Daredevil".into()), "Daredevil");
    }

    #[test]
    fn grades() {
        let t = |got, total| Tally { got, total }.tier();
        assert_eq!(t(0, 0), None);
        assert_eq!(t(1, 10), None);
        assert_eq!(t(3, 10), Some(Tier::Bronze));
        assert_eq!(t(5, 10), Some(Tier::Silver));
        assert_eq!(t(8, 10), Some(Tier::Gold));
        assert_eq!(t(10, 10), Some(Tier::Platinum));
        assert_eq!(
            Tally { got: 1, total: 10 }.to_next(),
            Some((Tier::Bronze, 2))
        );
        assert_eq!(
            Tally { got: 8, total: 10 }.to_next(),
            Some((Tier::Platinum, 2))
        );
        assert_eq!(Tally { got: 10, total: 10 }.to_next(), None);
    }
}
