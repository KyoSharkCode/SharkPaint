//! Built-in tip shapes, drawn by code so the application ships no image files.

use std::sync::Arc;

use super::brush::TipImage;

const SIZE: usize = 256;
/// Each pixel averages this many samples per side, for clean edges.
const SAMPLES: usize = 3;

/// Small deterministic generator, so the tips are the same on every run.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// A repeatable value in 0..1 for a point of an integer lattice.
fn lattice(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x85EB_CA6B) ^ (y as u32).wrapping_mul(0xC2B2_AE35) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Smooth noise in 0..1 whose blotches are `1 / frequency` wide.
fn smooth_noise(x: f32, y: f32, frequency: f32, seed: u32) -> f32 {
    let (x, y) = (x * frequency, y * frequency);
    let (x0, y0) = (x.floor(), y.floor());
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (ease(x - x0), ease(y - y0));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let top = lattice(ix, iy, seed) + (lattice(ix + 1, iy, seed) - lattice(ix, iy, seed)) * tx;
    let bottom = lattice(ix, iy + 1, seed) + (lattice(ix + 1, iy + 1, seed) - lattice(ix, iy + 1, seed)) * tx;
    top + (bottom - top) * ty
}

/// Paper-like grain in 0..1: coarse blotches broken up by finer ones.
fn grain(u: f32, v: f32, seed: u32) -> f32 {
    0.6 * smooth_noise(u, v, 9.0, seed) + 0.4 * smooth_noise(u, v, 23.0, seed ^ 0x5bd1_e995)
}

/// 0 below `edge`, 1 above it, with a short ramp of width `soft` in between.
fn step(value: f32, edge: f32, soft: f32) -> f32 {
    ((value - edge) / soft + 0.5).clamp(0.0, 1.0)
}

fn tip(id: &str, name: &str, shade: impl Fn(f32, f32) -> f32) -> Arc<TipImage> {
    let mut alpha = Vec::with_capacity(SIZE * SIZE);
    let scale = 2.0 / (SIZE * SAMPLES) as f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut sum = 0.0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    // Position from -1 to 1 across the tip.
                    let u = ((x * SAMPLES + sx) as f32 + 0.5) * scale - 1.0;
                    let v = ((y * SAMPLES + sy) as f32 + 0.5) * scale - 1.0;
                    sum += shade(u, v).clamp(0.0, 1.0);
                }
            }
            alpha.push((sum / (SAMPLES * SAMPLES) as f32 * 255.0).round() as u8);
        }
    }
    Arc::new(TipImage::new(id, name, SIZE, alpha))
}

/// 1 inside a circle of this radius, 0 outside.
fn inside(d: f32, radius: f32) -> f32 {
    if d <= radius { 1.0 } else { 0.0 }
}

/// The tips every installation has, in menu order.
pub fn builtin_tips() -> Vec<Arc<TipImage>> {
    // Strands of a bristle brush: where each runs, how thick, and how far.
    let mut seed = Noise(0x2F6E_2B15);
    let strands: Vec<(f32, f32, f32, f32)> = (0..15)
        .map(|i| {
            let v = -0.86 + i as f32 * 0.123 + (seed.next() - 0.5) * 0.05;
            let reach = (1.0 - v * v).max(0.0).sqrt();
            (v, 0.018 + seed.next() * 0.03, -reach * (0.7 + 0.3 * seed.next()), reach * (0.7 + 0.3 * seed.next()))
        })
        .collect();
    let mut seed = Noise(0x51ED_270B);
    let drops: Vec<(f32, f32, f32)> = (0..22)
        .map(|_| {
            let (a, r) = (seed.next() * std::f32::consts::TAU, seed.next().sqrt() * 0.8);
            (a.cos() * r, a.sin() * r, 0.03 + seed.next() * 0.12)
        })
        .collect();
    // A five-pointed star with straight edges, as its ten corners.
    let star: Vec<(f32, f32)> = (0..10)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
            let radius = if i % 2 == 0 { 0.94 } else { 0.38 };
            (angle.cos() * radius, angle.sin() * radius)
        })
        .collect();

    vec![
        // Charcoal: a disc eaten away by the grain of the paper.
        tip("carbon", "Carboncillo", |u, v| {
            let d = (u * u + v * v).sqrt();
            let body = step(0.82 - d, 0.0, 0.1);
            body * step(grain(u, v, 11) + 0.25 * (1.0 - d), 0.5, 0.12)
        }),
        // Separate hairs of varying thickness: for hair, grass and dry strokes.
        tip("cerdas", "Cerdas", move |u, v| {
            strands
                .iter()
                .map(|&(sv, half, from, to)| if (v - sv).abs() <= half && u >= from && u <= to { 1.0 } else { 0.0 })
                .fold(0.0, f32::max)
        }),
        // Chalk: a block with a ragged edge and a rough face.
        tip("tiza", "Tiza", |u, v| {
            let ragged = 0.06 * (smooth_noise(u, v, 14.0, 5) - 0.5);
            let body = if u.abs() <= 0.9 + ragged && v.abs() <= 0.58 + ragged { 1.0 } else { 0.0 };
            body * step(grain(u, v, 29), 0.36, 0.1)
        }),
        // Scattered droplets.
        tip("salpicadura", "Salpicadura", move |u, v| {
            drops
                .iter()
                .map(|&(x, y, r)| inside(((u - x).powi(2) + (v - y).powi(2)).sqrt(), r))
                .fold(0.0, f32::max)
        }),
        // A leaf, pointing right, with its midrib left unpainted.
        tip("hoja", "Hoja", |u, v| {
            let width = 0.42 * (1.0 - u * u).max(0.0).sqrt() * (1.0 - 0.35 * u);
            if v.abs() <= width && !(v.abs() < 0.022 && u.abs() < 0.8) { 1.0 } else { 0.0 }
        }),
        // Five-pointed star with straight edges and sharp points.
        tip("estrella", "Estrella", move |u, v| {
            // Even-odd test against the star's outline.
            let mut inside = false;
            for i in 0..star.len() {
                let (a, b) = (star[i], star[(i + 1) % star.len()]);
                if (a.1 > v) != (b.1 > v) && u < a.0 + (v - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                    inside = !inside;
                }
            }
            if inside { 1.0 } else { 0.0 }
        }),
    ]
}
