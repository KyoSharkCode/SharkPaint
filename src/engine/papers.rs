//! Built-in paper textures: the grain a brush can reveal, drawn by code.
//!
//! Each texture tiles seamlessly and is a height map: high points catch paint
//! first, hollows only under firm pressure.

use std::sync::Arc;

use super::brush::PaperTexture;

const SIZE: usize = 256;

/// A repeatable value in 0..1 for a point of a lattice that wraps every
/// `period` cells, so noise built on it tiles.
fn lattice(x: i32, y: i32, period: i32, seed: u32) -> f32 {
    let (x, y) = (x.rem_euclid(period) as u32, y.rem_euclid(period) as u32);
    let mut h = x.wrapping_mul(0x85EB_CA6B) ^ y.wrapping_mul(0xC2B2_AE35) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Smooth tiling noise in 0..1 with `cells` blotches across the texture.
fn noise(u: f32, v: f32, cells: i32, seed: u32) -> f32 {
    let (x, y) = (u * cells as f32, v * cells as f32);
    let (x0, y0) = (x.floor(), y.floor());
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (ease(x - x0), ease(y - y0));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let at = |dx: i32, dy: i32| lattice(ix + dx, iy + dy, cells, seed);
    let top = at(0, 0) + (at(1, 0) - at(0, 0)) * tx;
    let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * tx;
    top + (bottom - top) * ty
}

fn paper(id: &str, name: &str, height: impl Fn(f32, f32) -> f32) -> Arc<PaperTexture> {
    let mut values: Vec<f32> = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            values.push(height((x as f32 + 0.5) / SIZE as f32, (y as f32 + 0.5) / SIZE as f32));
        }
    }
    // Stretch to the full range so every texture responds to pressure alike.
    let (low, high) = values.iter().fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
    let data = values.iter().map(|v| ((v - low) / (high - low).max(1e-6) * 255.0).round() as u8).collect();
    Arc::new(PaperTexture { id: id.to_owned(), name: name.to_owned(), size: SIZE, data })
}

/// The papers every installation has, in menu order.
pub fn builtin_papers() -> Vec<Arc<PaperTexture>> {
    vec![
        // Smooth drawing paper: a fine, even tooth.
        paper("fino", "Papel fino", |u, v| 0.6 * noise(u, v, 64, 3) + 0.4 * noise(u, v, 128, 17)),
        // Rough sketching paper: larger bumps with fine tooth on top.
        paper("rugoso", "Papel rugoso", |u, v| {
            0.5 * noise(u, v, 20, 41) + 0.3 * noise(u, v, 48, 5) + 0.2 * noise(u, v, 110, 23)
        }),
        // Cold-pressed watercolour paper: soft, rounded hollows.
        paper("acuarela", "Papel de acuarela", |u, v| {
            let broad = noise(u, v, 26, 77);
            0.65 * (broad * broad * (3.0 - 2.0 * broad)) + 0.35 * noise(u, v, 70, 9)
        }),
        // Canvas: crossing threads, with the gaps between them as hollows.
        paper("lienzo", "Lienzo", |u, v| {
            let threads = 44.0 * std::f32::consts::PI;
            let (across, along) = ((u * threads).sin().abs(), (v * threads).sin().abs());
            0.8 * across.max(along) + 0.2 * noise(u, v, 96, 61)
        }),
    ]
}
