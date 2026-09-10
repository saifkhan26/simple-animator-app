//! Paper grain.
//!
//! A tileable multi-octave value-noise field, baked once and sampled by
//! canvas position. It replaces a single-pixel hash, which had no spatial
//! frequency content at all and so read as uniform speckle rather than as
//! tooth: real paper has structure at several scales at once, and that is
//! what a pencil catches on.
//!
//! Sampling by *canvas* position rather than by position along the stroke is
//! not cosmetic — it is required for correctness. Overlapping dabs and
//! re-rasterized pixels must see the same value every time, or the coverage
//! buffer's combine rules stop being idempotent.

use std::sync::OnceLock;

/// Tile edge in texels. The field wraps at this period, so every octave's
/// lattice count has to divide it.
const TILE: usize = 256;

/// (lattice cells across the tile, weight). Wide-to-fine, weighted so the
/// finest octave dominates — pencil tooth is mostly high frequency, with
/// just enough coarse structure to keep it from looking like TV static.
const OCTAVES: [(usize, f32); 4] = [(4, 0.12), (16, 0.18), (32, 0.25), (64, 0.45)];

pub struct Paper {
    tile: Vec<f32>,
}

/// The one shared grain field. Building it costs a few hundred microseconds
/// and 256 KB, once per process.
pub fn paper() -> &'static Paper {
    static PAPER: OnceLock<Paper> = OnceLock::new();
    PAPER.get_or_init(Paper::new)
}

impl Paper {
    fn new() -> Self {
        let mut tile = vec![0.0f32; TILE * TILE];
        for (cells, weight) in OCTAVES {
            let step = cells as f32 / TILE as f32;
            for y in 0..TILE {
                for x in 0..TILE {
                    tile[y * TILE + x] +=
                        weight * value_noise(x as f32 * step, y as f32 * step, cells);
                }
            }
        }

        // Normalise to the full 0..1 range so `grain` means the same depth
        // whatever the octave weights happen to sum to.
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for &v in &tile {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        let span = (hi - lo).max(1e-6);
        for v in &mut tile {
            let t = (*v - lo) / span;
            // Push the histogram towards its ends. Summed octaves land in a
            // bell, and a bell reads as a smudge: paper tooth is closer to
            // "peak or valley" than to an even spread of greys, and the peaks
            // are what a pencil actually catches on.
            *v = smoothstep(smoothstep(t));
        }

        Self { tile }
    }

    /// Bilinear sample at texel coordinates, wrapping. Out-of-range and
    /// negative coordinates are fine — the field is periodic.
    #[inline]
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let x0f = u.floor();
        let y0f = v.floor();
        let fx = u - x0f;
        let fy = v - y0f;
        let x0 = wrap(x0f as i64);
        let y0 = wrap(y0f as i64);
        let x1 = if x0 + 1 == TILE { 0 } else { x0 + 1 };
        let y1 = if y0 + 1 == TILE { 0 } else { y0 + 1 };

        let a = self.tile[y0 * TILE + x0];
        let b = self.tile[y0 * TILE + x1];
        let c = self.tile[y1 * TILE + x0];
        let d = self.tile[y1 * TILE + x1];
        let top = a + (b - a) * fx;
        let bot = c + (d - c) * fx;
        top + (bot - top) * fy
    }
}

#[inline]
fn wrap(i: i64) -> usize {
    i.rem_euclid(TILE as i64) as usize
}

/// One octave: lattice values hashed from integer cell coordinates,
/// smoothstep-interpolated. Indices wrap at `cells`, which is what makes the
/// whole tile seamless.
fn value_noise(u: f32, v: f32, cells: usize) -> f32 {
    let x0 = u.floor();
    let y0 = v.floor();
    let fx = smoothstep(u - x0);
    let fy = smoothstep(v - y0);
    let (xi, yi) = (x0 as i64, y0 as i64);
    let m = cells as i64;

    let at = |dx: i64, dy: i64| -> f32 {
        hash01(
            (xi + dx).rem_euclid(m) as u32,
            (yi + dy).rem_euclid(m) as u32,
            cells as u32,
        )
    };
    let top = at(0, 0) + (at(1, 0) - at(0, 0)) * fx;
    let bot = at(0, 1) + (at(1, 1) - at(0, 1)) * fx;
    top + (bot - top) * fy
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Integer-mix hash, 0..=1. `salt` keeps the octaves from sharing a lattice
/// where their cell counts line up.
#[inline]
fn hash01(x: u32, y: u32, salt: u32) -> f32 {
    let mut h = x
        .wrapping_mul(0x9E37_79B9)
        ^ y.wrapping_mul(0x85EB_CA6B)
        ^ salt.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_is_seamless() {
        let p = paper();
        // Sampling either side of the seam must agree, or a stroke crossing a
        // tile boundary would show a hard edge in its grain.
        for i in 0..TILE {
            let f = i as f32 + 0.37;
            assert!(
                (p.sample(f, -0.25) - p.sample(f, TILE as f32 - 0.25)).abs() < 1e-5,
                "horizontal seam visible at x={f}"
            );
            assert!(
                (p.sample(-0.25, f) - p.sample(TILE as f32 - 0.25, f)).abs() < 1e-5,
                "vertical seam visible at y={f}"
            );
        }
    }

    #[test]
    fn sampling_is_deterministic_and_in_range() {
        let p = paper();
        for i in 0..500 {
            let u = i as f32 * 1.7 - 300.0;
            let v = i as f32 * -0.9 + 50.0;
            let a = p.sample(u, v);
            assert_eq!(a, p.sample(u, v), "grain must not move under a re-read");
            assert!((0.0..=1.0).contains(&a), "grain out of range: {a}");
        }
    }

    /// The whole point of going multi-octave: a single-texel hash has no
    /// correlation between neighbours, so it reads as speckle. Real tooth has
    /// neighbouring texels agreeing more often than chance.
    #[test]
    fn the_field_has_structure_at_more_than_one_scale() {
        let p = paper();
        let mut near = 0.0f32;
        let mut far = 0.0f32;
        let n = 4000;
        for i in 0..n {
            let u = (i % 200) as f32 + 0.5;
            let v = (i / 200) as f32 + 0.5;
            let c = p.sample(u, v);
            near += (c - p.sample(u + 1.0, v)).abs();
            far += (c - p.sample(u + 37.0, v)).abs();
        }
        assert!(
            near < far * 0.8,
            "neighbouring texels are as unrelated as distant ones ({}, {}) — \
             this is white noise, not tooth",
            near / n as f32,
            far / n as f32
        );
    }
}
