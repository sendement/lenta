//! Game art as textures: decoded (and fetched, for addresses) on worker
//! threads, the most recently asked for first, each with its dominant
//! colours for the accent. Unused ones are dropped after a while.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use egui::{Color32, ColorImage, TextureHandle, TextureOptions, Vec2};

use crate::game::Art;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Longest side at most this many pixels.
    Fit(u32),
    /// Card art: longest side this many pixels, scaled up too (up to
    /// twice, with a bicubic filter: smoother than the GPU stretching a
    /// small picture).
    Card(u32),
    /// Small and blurred, for backgrounds.
    Blur,
}

type Key = (Art, Kind);

pub struct Tex {
    pub handle: TextureHandle,
    pub size: Vec2,
    pub color: Color32,
    pub color2: Color32,
}

enum Slot {
    Loading,
    Ready(Tex, Instant),
    Failed,
}

struct Decoded {
    key: Key,
    img: Option<(ColorImage, Color32, Color32)>,
}

#[derive(Default)]
struct Queue {
    jobs: Vec<Key>,
}

pub struct Images {
    ctx: egui::Context,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    rx: Receiver<Decoded>,
    map: HashMap<Key, Slot>,
    last_sweep: Instant,
}

const KEEP: Duration = Duration::from_secs(90);

impl Images {
    pub fn new(ctx: egui::Context) -> Self {
        let queue: Arc<(Mutex<Queue>, Condvar)> = Arc::default();
        let (tx, rx) = crossbeam_channel::unbounded();
        for i in 0..4 {
            let q = queue.clone();
            let tx: Sender<Decoded> = tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("lenta-img{i}"))
                .spawn(move || {
                    loop {
                        let key = {
                            let (m, cv) = &*q;
                            let mut g = m.lock().unwrap();
                            loop {
                                if let Some(k) = g.jobs.pop() {
                                    break k;
                                }
                                g = cv.wait(g).unwrap();
                            }
                        };
                        let img = load(&key);
                        let _ = tx.send(Decoded { key, img });
                        ctx.request_repaint();
                    }
                })
                .expect("spawn image thread");
        }
        Images {
            ctx,
            queue,
            rx,
            map: HashMap::new(),
            last_sweep: Instant::now(),
        }
    }

    /// Upload what the workers finished; drop what wasn't drawn lately.
    pub fn pump(&mut self) {
        let mut n = 0;
        while let Ok(d) = self.rx.try_recv() {
            let slot = match d.img {
                Some((img, color, color2)) => {
                    let size = Vec2::new(img.size[0] as f32, img.size[1] as f32);
                    let opts = TextureOptions {
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                        ..TextureOptions::LINEAR
                    };
                    let handle = self.ctx.load_texture(format!("{:?}", d.key), img, opts);
                    Slot::Ready(
                        Tex {
                            handle,
                            size,
                            color,
                            color2,
                        },
                        Instant::now(),
                    )
                }
                None => Slot::Failed,
            };
            self.map.insert(d.key, slot);
            n += 1;
            // Spread big uploads over frames.
            if n >= 6 {
                self.ctx.request_repaint();
                break;
            }
        }
        if self.last_sweep.elapsed() > Duration::from_secs(10) {
            self.last_sweep = Instant::now();
            self.map
                .retain(|_, s| !matches!(s, Slot::Ready(_, t) if t.elapsed() > KEEP));
        }
    }

    /// The texture, if ready; asks for it otherwise.
    pub fn get(&mut self, art: &Art, kind: Kind) -> Option<&Tex> {
        let key = (art.clone(), kind);
        match self.map.get_mut(&key) {
            Some(Slot::Ready(_, t)) => *t = Instant::now(),
            Some(_) => {}
            None => {
                self.map.insert(key.clone(), Slot::Loading);
                let (m, cv) = &*self.queue;
                m.lock().unwrap().jobs.push(key.clone());
                cv.notify_one();
            }
        }
        match self.map.get(&key) {
            Some(Slot::Ready(t, _)) => Some(t),
            _ => None,
        }
    }

    /// It can't be had (missing, broken).
    pub fn failed(&self, art: &Art, kind: Kind) -> bool {
        matches!(self.map.get(&(art.clone(), kind)), Some(Slot::Failed))
    }

    /// The first of these that loads: skips the failed ones, waits on one
    /// still loading.
    pub fn first(&mut self, arts: &[Option<&Art>], kind: Kind) -> Option<&Tex> {
        let mut pick = None;
        for a in arts.iter().flatten() {
            if self.failed(a, kind) {
                continue;
            }
            pick = Some(*a);
            break;
        }
        let a = pick?.clone();
        self.get(&a, kind)
    }
}

fn load(key: &Key) -> Option<(ColorImage, Color32, Color32)> {
    let path = match &key.0 {
        Art::File(p) => p.clone(),
        Art::Url(u) => crate::net::cached(u, "art").ok()?,
    };
    let bytes = std::fs::read(&path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let img = match key.1 {
        Kind::Fit(max) => {
            if img.width().max(img.height()) > max {
                img.resize(max, max, image::imageops::FilterType::Triangle)
            } else {
                img
            }
        }
        Kind::Card(px) => {
            let long = img.width().max(img.height()).max(1);
            let target = px.min(long * 2);
            if target > long {
                img.resize(target, target, image::imageops::FilterType::CatmullRom)
            } else if target < long {
                img.resize(target, target, image::imageops::FilterType::Triangle)
            } else {
                img
            }
        }
        Kind::Blur => {
            let small = img
                .resize(160, 160, image::imageops::FilterType::Triangle)
                .to_rgba8();
            image::DynamicImage::ImageRgba8(image::imageops::blur(&small, 4.0))
        }
    };
    let rgba = img.to_rgba8();
    let (c1, c2) = colors(&rgba);
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some((
        ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()),
        c1,
        c2,
    ))
}

pub fn hsv(c: Color32) -> (f32, f32, f32) {
    let (r, g, b) = (
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

pub fn from_hsv(h: f32, s: f32, v: f32) -> Color32 {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let f = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(r), f(g), f(b))
}

/// The picture's two most telling hues, lifted to read as accents.
fn colors(img: &image::RgbaImage) -> (Color32, Color32) {
    const BINS: usize = 18;
    let mut w = [0f32; BINS];
    let mut sums = [(0f32, 0f32, 0f32); BINS];
    let step = ((img.width() * img.height()) as usize / 4096).max(1);
    for (i, p) in img.pixels().enumerate() {
        if i % step != 0 || p[3] < 128 {
            continue;
        }
        let c = Color32::from_rgb(p[0], p[1], p[2]);
        let (h, s, v) = hsv(c);
        let weight = s * s * v + 0.002;
        let b = ((h * BINS as f32) as usize).min(BINS - 1);
        w[b] += weight;
        sums[b].0 += h.cos_turn() * weight;
        sums[b].1 += h.sin_turn() * weight;
        sums[b].2 += s * weight;
    }
    let mut order: Vec<usize> = (0..BINS).collect();
    order.sort_by(|a, b| w[*b].total_cmp(&w[*a]));
    let color_of = |b: usize| {
        if w[b] <= 0.0 {
            return Color32::from_rgb(0x4c, 0x9a, 0xff);
        }
        let h = sums[b].1.atan2(sums[b].0) / std::f32::consts::TAU;
        let s = (sums[b].2 / w[b]).clamp(0.45, 0.85);
        from_hsv(h, s, 0.95)
    };
    let first = order[0];
    // The second: a hue far enough from the first, else a shifted first.
    let second = order[1..].iter().copied().find(|&b| {
        let d = (b as i32 - first as i32).rem_euclid(BINS as i32);
        d.min(BINS as i32 - d) >= 3 && w[b] > w[first] * 0.08
    });
    let c1 = color_of(first);
    let c2 = match second {
        Some(b) => color_of(b),
        None => {
            let (h, s, v) = hsv(c1);
            from_hsv(h + 0.12, s, v)
        }
    };
    (c1, c2)
}

trait Turn {
    fn cos_turn(self) -> f32;
    fn sin_turn(self) -> f32;
}

impl Turn for f32 {
    fn cos_turn(self) -> f32 {
        (self * std::f32::consts::TAU).cos()
    }
    fn sin_turn(self) -> f32 {
        (self * std::f32::consts::TAU).sin()
    }
}
