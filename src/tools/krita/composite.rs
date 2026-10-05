//! Krita's 8-bit pixel arithmetic: the integer helpers from KoIntegerMaths.h
//! and KoColorSpaceMaths.h, the two composite ops a build-up dab is painted
//! with, and the alpha-darken op a wash dab builds its stroke buffer with.
//!
//! "Normal" on an 8-bit RGBA layer is `KoOptimizedCompositeOpOver32`, which
//! works in float and divides by way of the CPU's approximate reciprocal.
//! This is its per-pixel (scalar) form. Krita runs a vectorised form on
//! aligned runs of 8 pixels, and there the fully-transparent and
//! fully-opaque shortcuts are taken per block rather than per pixel, so a
//! pixel can come out one level apart depending on its neighbours and on
//! where Krita's tile memory happens to be aligned. That is the one place
//! this port cannot follow Krita byte for byte.

use super::qt::rcp;

/// `UINT8_MULT`: `a * b / 255`, rounded.
#[inline]
pub fn mul(a: u8, b: u8) -> u8 {
    let c = a as u32 * b as u32 + 0x80;
    (((c >> 8) + c) >> 8) as u8
}

/// `UINT8_MULT3`: `a * b * c / 255²`, rounded.
#[inline]
pub fn mul3(a: u8, b: u8, c: u8) -> u8 {
    let t = a as u32 * b as u32 * c as u32 + 0x7F5B;
    (((t >> 7) + t) >> 16) as u8
}

/// `KoColorSpaceMaths<double, quint8>::scaleToA`.
#[inline]
pub fn scale_to_u8(v: f64) -> u8 {
    let x = (v * 255.0).clamp(0.0, 255.0);
    (x + 0.5) as u8
}

/// `KoColorSpaceMaths<float, quint8>::scaleToA`.
#[inline]
pub fn scale_to_u8_f32(v: f32) -> u8 {
    let x = (v * 255.0f32).clamp(0.0, 255.0);
    (x + 0.5f32) as u8
}

const U8_REC: f32 = 1.0 / 255.0;

/// `round_float_to_u8`: SSE's round-half-to-even, keeping the low byte.
#[inline]
fn round_u8(x: f32) -> u8 {
    x.round_ties_even() as i32 as u8
}

#[inline]
fn lerp_u8(a: u8, b: u8, alpha: f32) -> u8 {
    round_u8((b as i32 - a as i32) as f32 * alpha + a as f32)
}

/// `OverCompositor32::compositeOnePixelScalar`, all channels, alpha
/// unlocked unless `alpha_locked`. `dst` is straight RGBA; `src_alpha` is
/// the dab's own alpha; `mask` is the selection at this pixel.
#[inline]
pub fn over(dst: &mut [u8], color: [u8; 3], src_alpha: u8, opacity: f32, mask: Option<u8>, alpha_locked: bool) {
    let mut sa = src_alpha as f32 * opacity;
    if let Some(m) = mask {
        sa *= m as f32 * U8_REC;
    }
    if sa == 0.0 {
        return;
    }
    let mut da = dst[3] as f32;
    let blend;
    if alpha_locked || da == 255.0 {
        blend = sa * U8_REC;
    } else if da == 0.0 {
        da = sa;
        blend = 1.0;
    } else {
        da += (255.0 - da) * sa * U8_REC;
        blend = rcp(da) * sa;
    }
    if blend == 1.0 {
        dst[..3].copy_from_slice(&color);
    } else if blend != 0.0 {
        for k in 0..3 {
            dst[k] = lerp_u8(dst[k], color[k], blend);
        }
    }
    if !alpha_locked {
        dst[3] = round_u8(da);
    }
}

/// `KoCompositeOpErase<quint8>`: takes alpha away and leaves colour. A pixel
/// left with no alpha is cleared outright, which is this app's convention
/// for empty pixels and invisible either way.
#[inline]
pub fn erase(dst: &mut [u8], src_alpha: u8, opacity: f32, mask: Option<u8>) {
    let opacity = scale_to_u8_f32(opacity);
    let mut sa = src_alpha;
    if let Some(m) = mask {
        sa = if m != 0 { mul(sa, m) } else { 0 };
    }
    let sa = 255 - mul(sa, opacity);
    let a = mul(sa, dst[3]);
    if a == 0 {
        dst.copy_from_slice(&[0, 0, 0, 0]);
    } else {
        dst[3] = a;
    }
}

/// One wash dab's alpha-darken parameters: `ParameterInfo` after
/// `setOpacityAndAverage`, through `KoAlphaDarkenParamsWrapperCreamy` —
/// creamy being Krita's default (`useCreamyAlphaDarken`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Darken {
    pub opacity: f32,
    pub flow: f32,
    pub average_opacity: f32,
}

impl Darken {
    pub fn new(opacity: f32, flow: f32, average_opacity: f32) -> Self {
        // qFuzzyCompare(float, float): a dab whose opacity is the running
        // average uses its own value for both.
        let same = (opacity - average_opacity).abs() * 100_000.0 <= opacity.abs().min(average_opacity.abs());
        Self {
            opacity,
            flow,
            average_opacity: if same { opacity } else { average_opacity },
        }
    }
}

/// `KisPainter::blendAverageOpacity`: the running opacity a wash stroke
/// carries from dab to dab.
pub fn blend_average_opacity(opacity: f64, average: f64) -> f64 {
    const EXPONENT: f32 = 0.1;
    if average < opacity {
        opacity
    } else {
        EXPONENT as f64 * opacity + (1.0 - EXPONENT as f64) * average
    }
}

/// `Arithmetic::lerp` on floats.
#[inline]
fn lerp_f(a: f32, b: f32, t: f32) -> f32 {
    (b - a) * t + a
}

/// `AlphaDarkenCompositor32::compositeOnePixelScalar`, alpha only: the
/// stroke buffer only ever holds the brush colour, so its colour channels
/// never change.
#[inline]
pub fn darken_pixel(dst: u8, src: u8, p: &Darken) -> u8 {
    let opacity = p.opacity;
    let dst_norm: f32 = if dst != 0 { dst as f32 * U8_REC } else { 0.0 };
    let msk = src as f32 * U8_REC;
    let src_norm = msk * opacity;
    let avg = p.average_opacity;
    let full = if avg > opacity {
        if avg > dst_norm {
            lerp_f(src_norm, avg, dst_norm / avg)
        } else {
            dst_norm
        }
    } else if opacity > dst_norm {
        lerp_f(dst_norm, opacity, msk)
    } else {
        dst_norm
    };
    let a = if p.flow == 1.0 {
        full * 255.0
    } else {
        // Creamy: with no flow the pixel keeps its own alpha.
        lerp_f(dst_norm, full, p.flow) * 255.0
    };
    round_u8(a)
}

/// Pixels in a float batch on `fma3<avx2>`.
const LANES: usize = 8;

/// `AlphaDarkenCompositor32::compositeVector` on one batch of eight,
/// alpha only. The batch's shortcuts (nothing to paint, opaque onto
/// opaque) are taken for the eight together, and its arithmetic is in the
/// 0..255 domain, so it can round differently from the scalar form.
pub fn darken_batch(dst: &mut [u8], src: &[u8], p: &Darken) {
    debug_assert!(dst.len() == LANES && src.len() == LANES);
    let opacity_vec = 255.0f32 * p.opacity;
    let average_vec = (255.0f64 * p.average_opacity as f64) as f32;
    let mut msk = [0f32; LANES];
    let mut sa = [0f32; LANES];
    let mut da = [0f32; LANES];
    for i in 0..LANES {
        msk[i] = src[i] as f32 * U8_REC;
        sa[i] = msk[i] * opacity_vec;
        da[i] = dst[i] as f32;
    }
    if sa.iter().all(|&v| v == 0.0) {
        return;
    }
    let dst_all_zero = da.iter().all(|&v| v == 0.0);
    if !dst_all_zero && sa.iter().all(|&v| v == 255.0) && da.iter().all(|&v| v == 255.0) {
        // memcpy(dst, src): the source pixels as they are.
        dst.copy_from_slice(src);
        return;
    }
    for i in 0..LANES {
        let full = if p.average_opacity > p.opacity {
            if average_vec > da[i] {
                let reverse_blend = da[i] / average_vec;
                (average_vec - sa[i]) * reverse_blend + sa[i]
            } else {
                da[i]
            }
        } else if opacity_vec > da[i] {
            (opacity_vec - da[i]) * msk[i] + da[i]
        } else {
            da[i]
        };
        let a = if p.flow == 1.0 {
            full
        } else {
            (full - da[i]) * p.flow + da[i]
        };
        dst[i] = a.round_ties_even() as i32 as u8;
    }
}

/// `genericComposite32` over one run of a row: a scalar head up to the
/// next 32-byte boundary, whole batches, a scalar tail. Krita's tile rows
/// are taken to start on a 32-byte boundary, so the boundary falls every
/// eight pixels of `x0`, the run's first column; runs never cross a 64-px
/// tile. Unverifiable without Krita's allocator, and the scheduler can
/// split a dab's rect further on a busy canvas, so this is the common case
/// rather than a guarantee.
pub fn darken_run(dst: &mut [u8], src: &[u8], x0: i32, p: &Darken) {
    let cols = dst.len();
    let align = (x0.rem_euclid(LANES as i32) * 4) as usize;
    let (head, batches) = if align == 0 {
        (0, cols / LANES)
    } else if cols > 2 * LANES {
        let h = (4 * LANES - align) / 4;
        (h, (cols - h) / LANES)
    } else {
        (cols, 0)
    };
    for i in 0..head {
        dst[i] = darken_pixel(dst[i], src[i], p);
    }
    let mut i = head;
    for _ in 0..batches {
        darken_batch(&mut dst[i..i + LANES], &src[i..i + LANES], p);
        i += LANES;
    }
    for j in i..cols {
        dst[j] = darken_pixel(dst[j], src[j], p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creamy alpha darken never lets a stroke darken itself: going over the
    /// same pixel at the same opacity stops at that opacity.
    #[test]
    fn alpha_darken_caps_at_the_dab_opacity() {
        let p = Darken::new(0.5, 1.0, 0.5);
        let mut a = 0u8;
        for _ in 0..20 {
            a = darken_pixel(a, 255, &p);
        }
        assert_eq!(a, 128, "0.5 · 255 rounded");
        let mut b = [0u8; 8];
        for _ in 0..20 {
            darken_batch(&mut b, &[255; 8], &p);
        }
        assert_eq!(b, [128; 8]);
    }

    /// Values worked from compositeOnePixelScalar: opacity 0.6, flow 0.5,
    /// a running average of 0.8 above it.
    #[test]
    fn alpha_darken_follows_the_average_and_the_flow() {
        let p = Darken::new(0.6, 0.5, 0.8);
        // dst 51 (0.2), src 255: srcNorm 0.6; avg 0.8 > 0.2 →
        // lerp(0.6, 0.8, 0.25) = 0.65; flow: lerp(0.2, 0.65, 0.5) = 0.425.
        let want = ((0.8f32 - 0.6) * (51.0 * U8_REC / 0.8) + 0.6 - 51.0 * U8_REC) * 0.5 + 51.0 * U8_REC;
        assert_eq!(darken_pixel(51, 255, &p), (want * 255.0).round_ties_even() as u8);
        assert_eq!(darken_pixel(51, 255, &p), 108);
        // Above the average, the pixel stays.
        assert_eq!(darken_pixel(230, 255, &p), 230);
    }

    #[test]
    fn a_batch_with_nothing_to_paint_leaves_dst_alone() {
        let p = Darken::new(0.7, 1.0, 0.9);
        let mut d = [10u8, 20, 30, 40, 50, 60, 70, 80];
        darken_batch(&mut d, &[0; 8], &p);
        assert_eq!(d, [10, 20, 30, 40, 50, 60, 70, 80]);
    }

    #[test]
    fn a_run_splits_into_head_batches_and_tail() {
        // Starting at x0 = 3 with 30 columns: 5 scalar, 3 batches, 1 tail.
        let p = Darken::new(0.37, 0.8, 0.61);
        let src: Vec<u8> = (0..30).map(|i| (i * 37 % 256) as u8).collect();
        let mut d: Vec<u8> = (0..30).map(|i| (i * 11 % 200) as u8).collect();
        let mut want = d.clone();
        for i in 0..5 {
            want[i] = darken_pixel(want[i], src[i], &p);
        }
        for b in 0..3 {
            let r = 5 + b * 8..13 + b * 8;
            darken_batch(&mut want[r.clone()], &src[r], &p);
        }
        want[29] = darken_pixel(want[29], src[29], &p);
        darken_run(&mut d, &src, 3, &p);
        assert_eq!(d, want);
        // A short misaligned run is all scalar.
        let mut short = vec![40u8; 12];
        darken_run(&mut short, &src[..12], 5, &p);
        let all: Vec<u8> = (0..12).map(|i| darken_pixel(40, src[i], &p)).collect();
        assert_eq!(short, all);
    }

    #[test]
    fn the_running_average_rises_at_once_and_falls_slowly() {
        assert_eq!(blend_average_opacity(0.8, 0.2), 0.8);
        let a = blend_average_opacity(0.2, 0.8);
        assert!((a - (0.1f32 as f64 * 0.2 + (1.0 - 0.1f32 as f64) * 0.8)).abs() < 1e-15);
    }

    #[test]
    fn integer_multiplies_round_like_krita() {
        assert_eq!(mul(255, 255), 255);
        assert_eq!(mul(255, 0), 0);
        assert_eq!(mul(128, 128), 64);
        assert_eq!(mul3(255, 255, 255), 255);
        assert_eq!(mul3(128, 255, 128), 64);
        assert_eq!(scale_to_u8(0.5), 128);
    }

    #[test]
    fn over_onto_empty_copies_the_dab() {
        let mut px = [0u8, 0, 0, 0];
        over(&mut px, [10, 20, 30], 200, 1.0, None, false);
        assert_eq!(px, [10, 20, 30, 200]);
    }

    /// Build-up in 8 bits stalls: a faint dab stops adding anything once
    /// the pixel is dark enough that its share rounds away. Krita's light
    /// pencil passes plateau short of black for exactly this reason.
    #[test]
    fn faint_build_up_plateaus_below_opaque() {
        let mut px = [0u8, 0, 0, 0];
        for _ in 0..2000 {
            over(&mut px, [0, 0, 0], 3, 1.0, None, false);
        }
        assert!(px[3] < 255, "reached {}", px[3]);
        assert!(px[3] > 150, "reached only {}", px[3]);
    }

    #[test]
    fn alpha_lock_keeps_alpha_and_recolours() {
        let mut px = [0u8, 0, 0, 100];
        over(&mut px, [255, 255, 255], 255, 1.0, None, true);
        assert_eq!(px, [255, 255, 255, 100]);
    }

    #[test]
    fn erase_removes_alpha() {
        let mut px = [9u8, 9, 9, 200];
        erase(&mut px, 255, 1.0, None);
        assert_eq!(px, [0, 0, 0, 0]);
        let mut px = [9u8, 9, 9, 200];
        erase(&mut px, 128, 1.0, None);
        assert_eq!(px[3], mul(127, 200));
    }
}
