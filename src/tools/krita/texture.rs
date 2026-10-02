//! Krita's brush texture ("Pattern" option): `KisTextureMaskInfo` builds an
//! 8-bit mask from the pattern image once, and `KisTextureOption::apply`
//! multiplies each dab's alpha by it.
//!
//! The mask is anchored to the canvas, not to the stroke: a dab samples the
//! texel under each of its pixels, so overlapping dabs agree and paper tooth
//! stays put as the pencil moves over it.

use super::composite::{mul3, scale_to_u8};
use super::qt::{q_alpha, q_blue, q_green, q_red, scaled_smooth, fuzzy_compare, Image32, QTransform, Rect};

/// The Pattern option's settings that shape the mask.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureSettings {
    pub scale: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub neutral_point: f64,
    pub invert: bool,
    pub cutoff_left: i32,
    pub cutoff_right: i32,
    pub cutoff_policy: i32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// The baked mask, one byte per texel.
#[derive(Clone, Debug)]
pub struct TextureMask {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
    offset_x: i32,
    offset_y: i32,
}

impl TextureMask {
    /// `KisTextureMaskInfo::recalculateMask`, for an opaque pattern (the
    /// alpha8 mask; patterns with alpha only matter to the lightness and
    /// gradient modes).
    ///
    /// `pattern` is the pattern image as `Format_ARGB32`.
    pub fn build(pattern: &Image32, s: &TextureSettings) -> Self {
        let mut mask = pattern.clone();
        let scale = s.scale;
        // A scaled mask comes back premultiplied; Krita reads its words as
        // they are, which for an opaque pattern changes nothing.
        if !fuzzy_compare(scale, 0.0) && !fuzzy_compare(scale, 1.0) {
            let mut tf = QTransform::identity();
            tf.scale(scale, scale);
            let rc = tf.map_rect_int(Rect {
                x: 0,
                y: 0,
                w: mask.w as i32,
                h: mask.h as i32,
            });
            // ensureRectNotSmaller(…, QSize(2, 2)).
            let (tw, th) = (rc.w.max(2) as i64, rc.h.max(2) as i64);
            // QSize::scale(…, Qt::KeepAspectRatio).
            let (w, h) = (mask.w as i64, mask.h as i64);
            let rw = th * w / h;
            let (nw, nh) = if rw <= tw { (rw, th) } else { (tw, tw * h / w) };
            mask = scaled_smooth(&mask, nw.max(1) as usize, nh.max(1) as usize, true);
        }

        let mut data = Vec::with_capacity(mask.w * mask.h);
        for &p in &mask.px {
            let (r, g, b) = (q_red(p) as i32, q_green(p) as i32, q_blue(p) as i32);
            let alpha: f32 = (q_alpha(p) as f64 / 255.0) as f32;
            let gray = (r * 11 + g * 16 + b * 5) / 32;
            let mut v: f32 = ((gray as f64 / 255.0) * alpha as f64 + (1.0f32 - alpha) as f64) as f32;
            v = (v as f64 - s.brightness) as f32;
            v = (((v as f64 - 0.5) * s.contrast) + 0.5) as f32;
            v = v.clamp(0.0, 1.0);
            if s.invert {
                v = 1.0 - v;
            }
            let v = v.clamp(0.0, 1.0);
            let np = s.neutral_point;
            let mut adjusted: f32 = if np == 1.0 || (np != 0.0 && v as f64 <= np) {
                (v as f64 / (2.0 * np)) as f32
            } else {
                (0.5 + (v as f64 - np) / (2.0 - 2.0 * np)) as f32
            };
            let lo = s.cutoff_left as f64 / 255.0;
            let hi = s.cutoff_right as f64 / 255.0;
            let outside = (adjusted as f64) < lo || (adjusted as f64) > hi;
            if s.cutoff_policy == 1 && outside {
                adjusted = 0.0;
            } else if s.cutoff_policy == 2 && outside {
                adjusted = 1.0;
            }
            data.push(scale_to_u8(adjusted as f64));
        }
        Self {
            w: mask.w,
            h: mask.h,
            data,
            offset_x: s.offset_x,
            offset_y: s.offset_y,
        }
    }

    /// The texel under canvas pixel `(x, y)`: `KisFillPainter::fillRect`'s
    /// pattern tiling, which wraps with a true modulo either side of zero.
    #[inline]
    fn at(&self, x: i32, y: i32) -> u8 {
        let tx = (x - self.offset_x).rem_euclid(self.w as i32) as usize;
        let ty = (y - self.offset_y).rem_euclid(self.h as i32) as usize;
        self.data[ty * self.w + tx]
    }

    /// `KisTextureOption::apply` in multiply mode: every dab pixel's alpha
    /// times the texel under it times the strength.
    pub fn apply(&self, alpha: &mut [u8], w: usize, h: usize, top_left: (i32, i32), strength: f64) {
        let strength = scale_to_u8(strength);
        // Krita starts the pattern patch at `offset % width - offsetX` and
        // tiles from there, which lands on the same texel as wrapping the
        // canvas position itself.
        for j in 0..h {
            for i in 0..w {
                let t = self.at(top_left.0 + i as i32, top_left.1 + j as i32);
                let a = &mut alpha[j * w + i];
                *a = mul3(t, *a, strength);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(scale: f64) -> TextureSettings {
        TextureSettings {
            scale,
            brightness: 0.0,
            contrast: 1.0,
            neutral_point: 0.5,
            invert: false,
            cutoff_left: 0,
            cutoff_right: 255,
            cutoff_policy: 0,
            offset_x: 0,
            offset_y: 0,
        }
    }

    #[test]
    fn mask_is_the_patterns_gray() {
        let mut p = Image32::new(4, 4);
        for (i, px) in p.px.iter_mut().enumerate() {
            let g = (i * 16) as u32;
            *px = 0xff00_0000 | (g << 16) | (g << 8) | g;
        }
        let m = TextureMask::build(&p, &settings(1.0));
        for (i, &v) in m.data.iter().enumerate() {
            assert_eq!(v as usize, i * 16);
        }
    }

    #[test]
    fn scaling_sizes_the_mask_like_krita() {
        let p = Image32 {
            w: 512,
            h: 512,
            px: vec![0xff80_8080; 512 * 512],
        };
        let m = TextureMask::build(&p, &settings(0.6));
        assert_eq!((m.w, m.h), (307, 307));
        assert!(m.data.iter().all(|&v| v == 128));
    }

    #[test]
    fn texture_is_anchored_to_the_canvas_either_side_of_zero() {
        let mut p = Image32::new(3, 1);
        p.px = vec![0xff00_0000, 0xff80_8080, 0xffff_ffff];
        let m = TextureMask::build(&p, &settings(1.0));
        assert_eq!(m.at(-1, 0), m.at(2, 0));
        assert_eq!(m.at(-3, 0), m.at(0, 0));
        assert_eq!(m.at(4, 7), m.at(1, 0));
    }

    #[test]
    fn apply_multiplies_by_texel_and_strength() {
        let mut p = Image32::new(1, 1);
        p.px = vec![0xffff_ffff];
        let m = TextureMask::build(&p, &settings(1.0));
        let mut a = [200u8, 100];
        m.apply(&mut a, 2, 1, (5, 5), 1.0);
        assert_eq!(a, [200, 100]);
        m.apply(&mut a, 2, 1, (5, 5), 0.0);
        assert_eq!(a, [0, 0]);
    }
}
