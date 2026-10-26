//! Midtone noise: a stroke's soft falloff broken into specks, while its
//! solid core and the bare paper round it stay clean — an airbrush with
//! tooth.
//!
//! Unlike the paper grain, which multiplies the whole stroke and so thins
//! its core too, the noise here averages out to nothing and is strongest
//! where the stroke is half there. The gradient keeps its length; only its
//! texture changes.
//!
//! The field is anchored to the canvas, as the paper grain is: a second pass
//! over a spot darkens the same specks, and a pixel's ink stays a pure
//! function of its position and coverage, which incremental compositing
//! relies on.
//!
//! Also home to the baked periodic tables both this and the fills' paper
//! grain are read from: one period of a field at the size asked for, walked
//! a pixel at a time along a row with no division per pixel.

use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex, OnceLock};

use crate::tools::krita::texture::TextureMask;
use crate::tools::krita::KritaPreset;
use crate::tools::{BrushMode, BrushSettings};

/// The noise sizes the UI offers.
pub const NOISE_SIZES: RangeInclusive<f32> = 1.0..=4.0;

/// Where a brush's noise comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NoiseSource {
    /// Random per pixel at size 1, softly clumped above it.
    #[default]
    Fine,
    /// Krita's paper scan, the one Pencil-5 Tilted draws on: the specks
    /// follow the paper's tooth. Size 1 is Pencil-5's own scale.
    Paper,
}

/// A brush's noise, baked and ready for its stroke.
#[derive(Clone)]
pub struct Noise {
    amount: f32,
    stipple: bool,
    field: Periodic,
}

impl Noise {
    /// The noise `brush` paints with, or `None` when it has none. Krita
    /// brushes never do: they paint dab by dab straight onto the layer,
    /// past where it applies.
    pub fn for_brush(brush: &BrushSettings) -> Option<Self> {
        if brush.noise <= 0.0 || brush.mode == BrushMode::Krita {
            return None;
        }
        let (lo, hi) = (*NOISE_SIZES.start(), *NOISE_SIZES.end());
        Some(Self {
            amount: brush.noise.min(1.0),
            stipple: brush.noise_stipple,
            field: field(brush.noise_source, brush.noise_size.clamp(lo, hi)),
        })
    }

    /// The noise along row `y`, from column `x` on.
    pub fn row(&self, x: u32, y: u32) -> NoiseRow<'_> {
        NoiseRow {
            noise: self,
            walk: self.field.row(x as i32, y as i32, (0, 0)),
        }
    }

    /// Coverage `c` with noise `n` (both 0..=1) in it. 0 and 1 come back as
    /// they are: bare paper stays bare, a solid core stays solid.
    #[inline]
    pub fn shape(&self, c: f32, n: f32) -> f32 {
        if self.stipple {
            // Fully inked or bare: over an even spread of noise, the share
            // of pixels inked is the coverage.
            let dot = if c > n { 1.0 } else { 0.0 };
            c + self.amount * (dot - c)
        } else {
            // Zero-mean, weighted to the midtones, and never past bare or
            // solid: nothing clips, so the stroke keeps its density.
            (c + self.amount * (2.0 * n - 1.0) * c.min(1.0 - c)).clamp(0.0, 1.0)
        }
    }
}

/// A walk along one row of a [`Noise`], a pixel a step.
pub struct NoiseRow<'a> {
    noise: &'a Noise,
    walk: PeriodicRow<'a>,
}

impl NoiseRow<'_> {
    /// Step over a pixel that takes no ink.
    #[inline]
    pub fn skip(&mut self) {
        self.walk.next();
    }

    /// The ink for this pixel's coverage `c`, then on to the next.
    #[inline]
    pub fn shape(&mut self, c: f32) -> f32 {
        let n = self.walk.next();
        self.noise.shape(c, n)
    }
}

/// One period of a field, row-major, tiling the canvas.
#[derive(Clone)]
pub(crate) struct Periodic {
    pub period: (usize, usize),
    pub values: Arc<Vec<f32>>,
}

impl Periodic {
    /// The values along row `y`, from column `x` on, with the field's origin
    /// at `offset`.
    pub fn row(&self, x: i32, y: i32, offset: (i32, i32)) -> PeriodicRow<'_> {
        let (pw, ph) = self.period;
        let ty = (y - offset.1).rem_euclid(ph as i32) as usize;
        PeriodicRow {
            row: &self.values[ty * pw..(ty + 1) * pw],
            i: (x - offset.0).rem_euclid(pw as i32) as usize,
        }
    }

    #[inline]
    fn at(&self, x: i32, y: i32) -> f32 {
        let (pw, ph) = self.period;
        let (x, y) = (x.rem_euclid(pw as i32) as usize, y.rem_euclid(ph as i32) as usize);
        self.values[y * pw + x]
    }

    /// The field at `size` times its own scale, bilinear, over a whole
    /// number of pixels as near the true period as rounding allows, so it
    /// still tiles. Texel centres sit on pixel centres.
    pub fn scaled(&self, size: f32) -> Self {
        let (w, h) = self.period;
        let size = size.max(0.01);
        let pw = ((w as f32 * size).round() as usize).max(1);
        let ph = ((h as f32 * size).round() as usize).max(1);
        let (sx, sy) = (w as f32 / pw as f32, h as f32 / ph as f32);
        let mut values = Vec::with_capacity(pw * ph);
        for y in 0..ph {
            let v = (y as f32 + 0.5) * sy - 0.5;
            let (v0, fy) = (v.floor(), v - v.floor());
            for x in 0..pw {
                let u = (x as f32 + 0.5) * sx - 0.5;
                let (u0, fx) = (u.floor(), u - u.floor());
                let at = |dx: i32, dy: i32| self.at(u0 as i32 + dx, v0 as i32 + dy);
                let top = at(0, 0) + (at(1, 0) - at(0, 0)) * fx;
                let bot = at(0, 1) + (at(1, 1) - at(0, 1)) * fx;
                values.push(top + (bot - top) * fy);
            }
        }
        Self {
            period: (pw, ph),
            values: Arc::new(values),
        }
    }
}

/// A walk along one row of a [`Periodic`], a pixel a step, wrapping at the
/// period: no division per pixel.
pub(crate) struct PeriodicRow<'a> {
    row: &'a [f32],
    i: usize,
}

impl PeriodicRow<'_> {
    #[inline]
    pub fn next(&mut self) -> f32 {
        let v = self.row[self.i];
        self.i += 1;
        if self.i == self.row.len() {
            self.i = 0;
        }
        v
    }
}

/// Krita's `04_paper-C-grain.png` at Pencil-5 Tilted's scale, taken from the
/// preset the app already carries.
pub(crate) fn paper_scan() -> &'static TextureMask {
    static PAPER: OnceLock<TextureMask> = OnceLock::new();
    PAPER.get_or_init(|| {
        KritaPreset::Pencil5Tilted
            .brush()
            .texture
            .clone()
            .expect("Pencil-5 Tilted carries its paper")
    })
}

/// `source` at `size`, spread evenly over 0..1. The last few asked for are
/// kept, so the pen and the eraser at different settings don't bake by
/// turns.
fn field(source: NoiseSource, size: f32) -> Periodic {
    type Baked = ((NoiseSource, u32), Periodic);
    static KEPT: Mutex<Vec<Baked>> = Mutex::new(Vec::new());
    let key = (source, size.to_bits());
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = kept.iter().position(|(k, _)| *k == key) {
        let hit = kept.remove(i);
        let field = hit.1.clone();
        kept.push(hit);
        return field;
    }
    let base = match source {
        NoiseSource::Fine => fine(),
        NoiseSource::Paper => paper(),
    };
    let field = if (size - 1.0).abs() < 1e-3 {
        base.clone()
    } else {
        let mut f = base.scaled(size);
        equalize(Arc::get_mut(&mut f.values).expect("just made"));
        f
    };
    if kept.len() == 4 {
        kept.remove(0);
    }
    kept.push((key, field.clone()));
    field
}

/// Independent random values, one per pixel, over a 256-pixel tile.
fn fine() -> &'static Periodic {
    const TILE: usize = 256;
    static FINE: OnceLock<Periodic> = OnceLock::new();
    FINE.get_or_init(|| {
        let values = (0..TILE * TILE)
            .map(|i| {
                // splitmix64 of the index: no visible lattice.
                let mut h = (i as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
                h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                h = (h ^ (h >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
                h ^= h >> 31;
                ((h >> 40) as f32 + 0.5) / (1u64 << 24) as f32
            })
            .collect();
        Periodic {
            period: (TILE, TILE),
            values: Arc::new(values),
        }
    })
}

/// The paper scan, spread evenly over 0..1.
fn paper() -> &'static Periodic {
    static PAPER: OnceLock<Periodic> = OnceLock::new();
    PAPER.get_or_init(|| {
        let p = paper_scan();
        let mut values: Vec<f32> = (0..p.h as i32)
            .flat_map(|y| (0..p.w as i32).map(move |x| p.at(x, y) as f32 / 255.0))
            .collect();
        equalize(&mut values);
        Periodic {
            period: (p.w, p.h),
            values: Arc::new(values),
        }
    })
}

/// Spread `values` (each 0..=1) evenly over 0..1, keeping their order, so
/// a fraction `t` of them falls below `t`: stipple density then follows
/// coverage, and the midtone noise averages out to nothing whatever field
/// it came from.
fn equalize(values: &mut [f32]) {
    const BINS: usize = 4096;
    let bin = |v: f32| (v.clamp(0.0, 1.0) * (BINS - 1) as f32).round() as usize;
    let mut count = vec![0u32; BINS];
    for &v in values.iter() {
        count[bin(v)] += 1;
    }
    let n = values.len() as f32;
    let mut below = 0u32;
    let map: Vec<f32> = count
        .iter()
        .map(|&c| {
            let m = (below as f32 + c as f32 * 0.5) / n;
            below += c;
            m
        })
        .collect();
    for v in values {
        *v = map[bin(*v)];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(amount: f32, stipple: bool) -> Noise {
        Noise {
            amount,
            stipple,
            field: fine().clone(),
        }
    }

    #[test]
    fn bare_paper_and_a_solid_core_stay_clean() {
        for stipple in [false, true] {
            let n = noise(1.0, stipple);
            for i in 0..=20 {
                let v = i as f32 / 20.0;
                assert_eq!(n.shape(0.0, v), 0.0, "bare stays bare (stipple {stipple})");
                assert_eq!(n.shape(1.0, v.min(0.999)), 1.0, "solid stays solid (stipple {stipple})");
            }
        }
    }

    /// Over the noise's even spread, a stroke keeps its density: only the
    /// texture of its soft part changes, never how dark it is overall.
    #[test]
    fn noise_keeps_the_strokes_density() {
        let f = fine();
        for stipple in [false, true] {
            let n = noise(1.0, stipple);
            for c in [0.1f32, 0.3, 0.5, 0.7, 0.9] {
                let mean = f.values.iter().map(|&v| n.shape(c, v)).sum::<f32>() / f.values.len() as f32;
                assert!((mean - c).abs() < 0.02, "coverage {c} came out {mean} (stipple {stipple})");
            }
        }
    }

    #[test]
    fn noise_is_strongest_in_the_midtones() {
        let f = fine();
        let n = noise(0.5, false);
        let spread = |c: f32| {
            let v: Vec<f32> = f.values.iter().map(|&x| n.shape(c, x)).collect();
            let m = v.iter().sum::<f32>() / v.len() as f32;
            (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
        };
        assert!(spread(0.5) > spread(0.1) * 2.0 && spread(0.5) > spread(0.9) * 2.0);
    }

    #[test]
    fn every_field_is_spread_evenly_and_tiles() {
        for (source, size) in [
            (NoiseSource::Fine, 1.0),
            (NoiseSource::Fine, 2.5),
            (NoiseSource::Paper, 1.0),
            (NoiseSource::Paper, 3.0),
        ] {
            let f = field(source, size);
            let (pw, ph) = f.period;
            assert_eq!(f.values.len(), pw * ph);
            let below = |t: f32| f.values.iter().filter(|&&v| v < t).count() as f32 / f.values.len() as f32;
            for t in [0.25, 0.5, 0.75] {
                assert!((below(t) - t).abs() < 0.03, "{source:?} at {size}: {} below {t}", below(t));
            }
            // A row walked past its period comes round to its start.
            let mut row = f.row(0, 3, (0, 0));
            let first: Vec<f32> = (0..pw).map(|_| row.next()).collect();
            let again: Vec<f32> = (0..pw).map(|_| row.next()).collect();
            assert_eq!(first, again);
            // And a walk from anywhere reads what the canvas position says.
            let mut r = f.row(-7, -2, (0, 0));
            assert_eq!(r.next(), f.at(-7, -2));
        }
    }

    #[test]
    fn a_bigger_size_clumps_the_specks() {
        let step = |f: &Periodic| {
            let (pw, _) = f.period;
            let row = &f.values[..pw];
            row.windows(2).map(|p| (p[0] - p[1]).abs()).sum::<f32>() / (pw - 1) as f32
        };
        let (one, three) = (field(NoiseSource::Fine, 1.0), field(NoiseSource::Fine, 3.0));
        assert!(step(&three) < step(&one) * 0.6, "{} vs {}", step(&three), step(&one));
    }

    #[test]
    fn brushes_without_noise_and_krita_brushes_get_none() {
        assert!(Noise::for_brush(&BrushSettings::default()).is_none());
        let krita = BrushSettings {
            noise: 0.5,
            ..BrushSettings::pencil5_krita()
        };
        assert!(Noise::for_brush(&krita).is_none());
        let air = BrushSettings {
            noise: 0.5,
            ..BrushSettings::airbrush()
        };
        assert!(Noise::for_brush(&air).is_some());
    }
}
