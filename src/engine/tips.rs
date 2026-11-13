//! Built-in tip shapes, drawn by code so the application ships no image files.

use std::sync::Arc;

use super::brush::TipImage;

const SIZE: usize = 128;

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

fn tip(id: &str, name: &str, shade: impl Fn(f32, f32, &mut Noise) -> f32) -> Arc<TipImage> {
    let mut noise = Noise(0x9E37_79B9 ^ id.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)));
    let mut alpha = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Position from -1 to 1 across the tip.
            let (u, v) = ((x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0, (y as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0);
            alpha.push((shade(u, v, &mut noise).clamp(0.0, 1.0) * 255.0) as u8);
        }
    }
    Arc::new(TipImage { id: id.to_owned(), name: name.to_owned(), size: SIZE, alpha })
}

/// Fades from 1 inside `radius` to 0 at `radius + edge`.
fn disc(d: f32, radius: f32, edge: f32) -> f32 {
    ((radius + edge - d) / edge).clamp(0.0, 1.0)
}

/// The tips every installation has, in menu order.
pub fn builtin_tips() -> Vec<Arc<TipImage>> {
    vec![
        // Grainy disc: charcoal and pastel.
        tip("carbon", "Carboncillo", |u, v, noise| {
            let d = (u * u + v * v).sqrt();
            let grain = if noise.next() < 0.55 { 1.0 } else { 0.15 };
            disc(d, 0.75, 0.25) * grain
        }),
        // Parallel streaks: a dry bristle brush.
        tip("cerdas", "Cerdas", |u, v, noise| {
            let d = (u * u + v * v).sqrt();
            let streak = ((v * 21.0).sin() * 0.5 + 0.5).powf(1.6);
            let gap = if noise.next() < 0.12 { 0.0 } else { 1.0 };
            disc(d, 0.8, 0.18) * streak * gap
        }),
        // Rough block: chalk and crayon.
        tip("tiza", "Tiza", |u, v, noise| {
            let edge = (1.0 - u.abs().max(v.abs() * 1.6)).clamp(0.0, 1.0);
            let grain = 0.35 + 0.65 * noise.next();
            (edge * 6.0).min(1.0) * grain
        }),
        // Scattered droplets.
        tip("salpicadura", "Salpicadura", {
            let mut seed = Noise(0x51ED_270B);
            let drops: Vec<(f32, f32, f32)> = (0..22)
                .map(|_| {
                    let (a, r) = (seed.next() * std::f32::consts::TAU, seed.next().sqrt() * 0.8);
                    (a.cos() * r, a.sin() * r, 0.03 + seed.next() * 0.12)
                })
                .collect();
            move |u, v, _| {
                drops
                    .iter()
                    .map(|&(x, y, r)| disc(((u - x).powi(2) + (v - y).powi(2)).sqrt(), r, 0.03))
                    .fold(0.0, f32::max)
            }
        }),
        // A leaf, pointing right.
        tip("hoja", "Hoja", |u, v, _| {
            let width = 0.42 * (1.0 - u * u).max(0.0).sqrt() * (1.0 - 0.35 * u);
            let body = ((width - v.abs()) / 0.04).clamp(0.0, 1.0);
            let vein = if v.abs() < 0.025 && u.abs() < 0.85 { 0.45 } else { 1.0 };
            body * vein
        }),
        // Five-pointed star.
        tip("estrella", "Estrella", |u, v, _| {
            let (angle, d) = (v.atan2(u) + std::f32::consts::FRAC_PI_2, (u * u + v * v).sqrt());
            let fifth = std::f32::consts::TAU / 5.0;
            let within = (angle.rem_euclid(fifth) - fifth / 2.0).abs() / (fifth / 2.0);
            // Radius of the outline between a point (0.9) and a notch (0.38).
            let radius = 0.38 + (0.9 - 0.38) * within.powf(1.4);
            disc(d, radius - 0.04, 0.05)
        }),
    ]
}
