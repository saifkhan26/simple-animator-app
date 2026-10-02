//! Krita's 8-bit pixel arithmetic: the integer helpers from KoIntegerMaths.h
//! and KoColorSpaceMaths.h, and the two composite ops a build-up dab is
//! painted with.
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

#[cfg(test)]
mod tests {
    use super::*;

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
