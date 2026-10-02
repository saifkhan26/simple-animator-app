//! A predefined (bitmap) brush tip: `KisPngBrush` loading, the
//! `KisQImagePyramid` of pre-scaled copies, and `KisBrush`'s dab geometry
//! and mask generation (libs/brush/kis_brush.cpp, kis_png_brush.cpp,
//! kis_qimage_pyramid.cpp).
//!
//! A dab is made by picking the pyramid level just above the wanted size and
//! having Qt draw it with the dab's scale, rotation and sub-pixel offset;
//! the mask is then `255 - gray`, times the drawn alpha.

use anyhow::{bail, Context, Result};

use super::composite::mul;
use super::qt::{
    draw_image, q_alpha, q_blue, qround, scaled_smooth, Image32, QTransform, RectF, Tx,
};
use super::sensors::normalize_angle;

const MIPMAP_SIZE_THRESHOLD: i32 = 512;
const MAX_MIPMAP_SCALE: f64 = 8.0;
/// `QPAINTER_WORKAROUND_BORDER`: every level carries a one-pixel transparent
/// frame so Qt's bilinear sampling fades out at the tip's edge instead of
/// smearing the edge pixels outwards.
const BORDER: i32 = 1;

/// `KisDabShape`: the size multiplier, the height-to-width ratio and the
/// rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DabShape {
    pub scale: f64,
    pub ratio: f64,
    pub rotation: f64,
}

impl DabShape {
    fn scale_x(&self) -> f64 {
        self.scale
    }
    fn scale_y(&self) -> f64 {
        self.scale * self.ratio
    }
}

struct Level {
    /// The level, `Format_ARGB32`, with its border.
    image: Image32,
    /// Its size without the border.
    size: (i32, i32),
}

/// `KisQImagePyramid`.
struct Pyramid {
    original: (i32, i32),
    base_scale: f64,
    levels: Vec<Level>,
}

/// `QSize * qreal`.
fn size_times(s: (i32, i32), k: f64) -> (i32, i32) {
    (qround(s.0 as f64 * k), qround(s.1 as f64 * k))
}

impl Pyramid {
    /// `base` is the tip as Krita holds it: a `Format_Grayscale8` image,
    /// given here already expanded to opaque `RGB32` gray.
    fn new(base: &Image32) -> Self {
        let original = (base.w as i32, base.h as i32);
        let mut levels = Vec::new();
        let mut base_scale = 1.0;
        let mut have_base_scale = false;

        let mut scale = MAX_MIPMAP_SCALE;
        while scale > 1.0 {
            let s = size_times(original, scale);
            if s.0 <= MIPMAP_SIZE_THRESHOLD || s.1 <= MIPMAP_SIZE_THRESHOLD {
                if !have_base_scale {
                    base_scale = scale;
                    have_base_scale = true;
                }
                levels.push(Self::level(&scaled_smooth(base, s.0 as usize, s.1 as usize, false)));
            }
            scale *= 0.5;
        }
        if !have_base_scale {
            base_scale = 1.0;
        }
        levels.push(Self::level(base));

        let mut scale = 0.5;
        loop {
            let s = size_times(original, scale);
            if s.0 == 0 || s.1 == 0 {
                break;
            }
            levels.push(Self::level(&scaled_smooth(base, s.0 as usize, s.1 as usize, false)));
            scale *= 0.5;
        }
        Self {
            original,
            base_scale,
            levels,
        }
    }

    /// `appendPyramidLevel`.
    fn level(image: &Image32) -> Level {
        // Already opaque ARGB32, so convertToFormat(ARGB32) is a no-op.
        let bordered = image.copy(
            -BORDER,
            -BORDER,
            image.w + 2 * BORDER as usize,
            image.h + 2 * BORDER as usize,
        );
        Level {
            image: bordered,
            size: (image.w as i32, image.h as i32),
        }
    }

    /// `findNearestLevel`.
    fn nearest_level(&self, scale: f64) -> (usize, f64) {
        const EPS: f64 = 1e-6;
        let mut level_scale = self.base_scale;
        let mut level = 0;
        let last = self.levels.len() - 1;
        while (0.5 * level_scale > scale || (0.5 * level_scale - scale).abs() < EPS) && level < last {
            level_scale *= 0.5;
            level += 1;
        }
        (level, level_scale)
    }

    /// `createImage`: the tip drawn at `shape`, `Format_ARGB32`.
    fn create_image(&self, shape: DabShape, sub_x: f64, sub_y: f64) -> Image32 {
        let (level, base_scale) = self.nearest_level(shape.scale);
        let src = &self.levels[level];
        let (mut transform, (w, h)) =
            calculate_params(shape, sub_x, sub_y, self.original, base_scale, src.size);

        if transform.is_identity() {
            return src.image.copy(
                BORDER,
                BORDER,
                src.image.w - 2 * BORDER as usize,
                src.image.h - 2 * BORDER as usize,
            );
        }
        let mut dst = Image32::new(w as usize, h as usize);

        // Qt samples a pure translation nearest-neighbour whatever the
        // render hints say, so Krita scales it by a hair to make it take the
        // bilinear path.
        while transform.ty() == Tx::Translate {
            let s = transform.m11;
            let fake = s - 10.0 * f64::EPSILON;
            transform.mul_assign(&QTransform::from_scale(fake, fake));
        }
        let painter = QTransform::from_translate(-(BORDER as f64), -(BORDER as f64)).mul(&transform);
        draw_image(&mut dst, &src.image, &painter);
        dst
    }
}

/// `baseBrushTransform`.
fn base_brush_transform(shape: DabShape, sub_x: f64, sub_y: f64, bounds: RectF) -> QTransform {
    let mut t = QTransform::identity();
    t.scale(shape.scale_x(), shape.scale_y());
    // qFuzzyCompare against zero only ever holds for zero itself.
    if shape.rotation != 0.0 && !shape.rotation.is_nan() {
        let mut r = QTransform::identity();
        r.rotate_radians(shape.rotation);
        t = t.mul(&r);
        let rotated = t.map_rect(bounds);
        t = t.mul(&QTransform::from_translate(-rotated.x, -rotated.y));
    }
    t.mul(&QTransform::from_translate(sub_x, sub_y))
}

/// `roundRect` in kis_qimage_pyramid.cpp: `toAlignedRect`, but forgiving of
/// float error at the origin and in the size.
fn round_rect(rc: RectF) -> (i32, i32, i32, i32) {
    let mut r = rc;
    if r.x < 0.000001 {
        // QRectF::setLeft keeps the right edge.
        let diff = 0.0 - r.x;
        r.x += diff;
        r.w -= diff;
    }
    if r.y < 0.000001 {
        let diff = 0.0 - r.y;
        r.y += diff;
        r.h -= diff;
    }
    let wr = qround(r.w) as f64;
    let hr = qround(r.h) as f64;
    if (r.w - wr).abs() < 0.000001 {
        r.w = wr;
    }
    if (r.h - hr).abs() < 0.000001 {
        r.h = hr;
    }
    let a = r.to_aligned_rect();
    (a.x, a.y, a.w, a.h)
}

/// `KisQImagePyramid::calculateParams`: the transform that draws a level,
/// and the size of the image it lands in.
fn calculate_params(
    shape: DabShape,
    sub_x: f64,
    sub_y: f64,
    original: (i32, i32),
    _base_scale: f64,
    base_size: (i32, i32),
) -> (QTransform, (i32, i32)) {
    let original_bounds = RectF {
        x: 0.0,
        y: 0.0,
        w: original.0 as f64,
        h: original.1 as f64,
    };
    let original_transform = base_brush_transform(shape, sub_x, sub_y, original_bounds);

    let real_base_scale_x = base_size.0 as f64 / original.0 as f64;
    let real_base_scale_y = base_size.1 as f64 / original.1 as f64;
    let scale_x = shape.scale_x() / real_base_scale_x;
    let scale_y = shape.scale_y() / real_base_scale_y;
    let level_shape = DabShape {
        scale: scale_x,
        ratio: scale_y / scale_x,
        rotation: shape.rotation,
    };
    let base_bounds = RectF {
        x: 0.0,
        y: 0.0,
        w: base_size.0 as f64,
        h: base_size.1 as f64,
    };
    let transform = base_brush_transform(level_shape, sub_x, sub_y, base_bounds);

    let mapped = original_transform.map_rect(original_bounds);
    let (mut w, mut h) = (1, 1);
    if mapped.is_valid() {
        let (x, y, rw, rh) = round_rect(mapped);
        w = (x + rw).max(1);
        h = (y + rh).max(1);
    }
    (transform, (w, h))
}

/// A bitmap tip, as the preset's `brush_definition` sets it up.
pub struct PngTip {
    pyramid: Pyramid,
    width: i32,
    height: i32,
    /// `KisBrush::angle`, radians.
    pub angle: f64,
    /// The box round every pixel the tip marks at all, in tip pixels:
    /// left, top, right, bottom (exclusive). Krita outlines the tip by where
    /// its mask is non-zero.
    footprint: (i32, i32, i32, i32),
}

impl PngTip {
    /// `KisPngBrush::loadFromDevice`. Only the gray, opaque tips Krita turns
    /// into a mask are accepted; colour and transparent tips are painted as
    /// image stamps or lightness maps, which this engine does not do.
    pub fn load(png: &[u8], angle: f64) -> Result<Self> {
        let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .context("decoding the brush tip")?
            .to_rgba8();
        let (w, h) = img.dimensions();
        let mut gray = Image32::new(w as usize, h as usize);
        let mut footprint = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (i, p) in img.pixels().enumerate() {
            let [r, g, b, a] = p.0;
            if a != 255 || r != g || g != b {
                bail!("only opaque grayscale tips are supported");
            }
            if r < 255 {
                let (x, y) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
                footprint = (
                    footprint.0.min(x),
                    footprint.1.min(y),
                    footprint.2.max(x + 1),
                    footprint.3.max(y + 1),
                );
            }
            // Grayscale8 expanded to RGB32, as smoothScaled and
            // convertToFormat(ARGB32) both see it.
            gray.px[i] = 0xff00_0000 | ((r as u32) * 0x0001_0101);
        }
        Ok(Self {
            pyramid: Pyramid::new(&gray),
            width: w as i32,
            height: h as i32,
            angle,
            footprint: if footprint.0 == i32::MAX {
                (0, 0, w as i32, h as i32)
            } else {
                footprint
            },
        })
    }

    fn original_rect(&self) -> RectF {
        RectF {
            x: 0.0,
            y: 0.0,
            w: self.width as f64,
            h: self.height as f64,
        }
    }

    /// The tip image's size in pixels.
    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// The corners of the tip's footprint as a dab of `shape` lays it down,
    /// relative to the dab's centre, in canvas pixels. Turned the way the
    /// tip is drawn: by minus the dab's rotation plus the brush's angle.
    pub fn outline(&self, brush_scale: f64, shape: DabShape) -> [(f64, f64); 4] {
        let s = shape.scale * brush_scale;
        let theta = -normalize_angle(shape.rotation + self.angle);
        let (sin, cos) = theta.sin_cos();
        let (cx, cy) = (self.width as f64 / 2.0, self.height as f64 / 2.0);
        let (l, t, r, b) = self.footprint;
        [(l, t), (r, t), (r, b), (l, b)].map(|(x, y)| {
            let (x, y) = ((x as f64 - cx) * s, (y as f64 - cy) * s);
            // QTransform::rotateRadians: (x cos - y sin, x sin + y cos).
            (x * cos - y * sin, x * sin + y * cos)
        })
    }

    // `brush_scale` below is `KisBrush::scale`: dab pixels per tip pixel at
    // size 1. Krita's size slider sets it; so does ours.

    /// `KisBrush::characteristicSize`.
    pub fn characteristic_size(&self, brush_scale: f64, shape: DabShape) -> (f64, f64) {
        let normalized = DabShape {
            scale: shape.scale * brush_scale,
            ratio: shape.ratio,
            rotation: normalize_angle(shape.rotation + self.angle),
        };
        let r = self.original_rect();
        let m = base_brush_transform(normalized, 0.0, 0.0, r).map_rect(r);
        (m.w, m.h)
    }

    /// `KisBrush::hotSpot`: the middle of the dab, at least half a pixel in.
    pub fn hot_spot(&self, brush_scale: f64, shape: DabShape) -> (f64, f64) {
        let (w, h) = self.characteristic_size(brush_scale, shape);
        (w.max(1.0) / 2.0, h.max(1.0) / 2.0)
    }

    /// `KisBrush::maskWidth` and `maskHeight`.
    pub fn mask_size(&self, brush_scale: f64, shape: DabShape, sub_x: f64, sub_y: f64) -> (i32, i32) {
        let s = DabShape {
            scale: shape.scale * brush_scale,
            ratio: shape.ratio,
            rotation: normalize_angle(shape.rotation + self.angle),
        };
        let (_, size) = calculate_params(s, sub_x, sub_y, (self.width, self.height), 1.0, (self.width, self.height));
        size
    }

    /// `KisBrush::generateMaskAndApplyMaskOrCreateDab` for a plain colour:
    /// the dab's alpha, row-major, with its size. The tip is drawn at the
    /// *negated* angle, while `mask_size` measures it at the positive one —
    /// Krita does both, and the two can differ by a pixel.
    pub fn mask(&self, brush_scale: f64, shape: DabShape, sub_x: f64, sub_y: f64) -> (Vec<u8>, usize, usize) {
        let img = self.pyramid.create_image(
            DabShape {
                scale: shape.scale * brush_scale,
                ratio: shape.ratio,
                rotation: -normalize_angle(shape.rotation + self.angle),
            },
            sub_x,
            sub_y,
        );
        // fillGrayBrushWithColor, SIMD form: it reads the low byte, which
        // for a gray tip is the same as red.
        let alpha = img
            .px
            .iter()
            .map(|&p| mul(255 - q_blue(p) as u8, q_alpha(p) as u8))
            .collect();
        (alpha, img.w, img.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALE: f64 = 40.0 / 220.0;

    // The preset's own angle, which is only nearly π.
    #[allow(clippy::approx_constant)]
    fn tip() -> PngTip {
        PngTip::load(super::super::GRADIENT_PNG, 3.14159).unwrap()
    }

    #[test]
    fn pyramid_levels_match_krita_for_a_220px_tip() {
        let t = tip();
        let sizes: Vec<_> = t.pyramid.levels.iter().map(|l| l.size).collect();
        assert_eq!(
            sizes,
            vec![(440, 440), (220, 220), (110, 110), (55, 55), (28, 28), (14, 14), (7, 7), (3, 3), (2, 2), (1, 1)]
        );
        assert_eq!(t.pyramid.base_scale, 2.0);
        assert_eq!(t.pyramid.nearest_level(0.18), (3, 0.25));
    }

    /// The mask's footprint follows the rotation: the tip is a tall thin
    /// bar, so a quarter turn swaps the mask's width and height.
    #[test]
    fn mask_rotates_with_the_dab() {
        let t = tip();
        let upright = DabShape {
            scale: 1.0,
            ratio: 1.0,
            rotation: 0.0,
        };
        let (a, w, h) = t.mask(SCALE, upright, 0.25, 0.5);
        assert!(w >= 40 && h >= 40, "{w}x{h}");
        let ink = |a: &[u8], w: usize| {
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
            let (mut xx, mut yy) = (0.0, 0.0);
            for (i, &v) in a.iter().enumerate() {
                let (x, y) = ((i % w) as f64, (i / w) as f64);
                let v = v as f64;
                sx += x * v;
                sy += y * v;
                n += v;
                xx += x * x * v;
                yy += y * y * v;
            }
            let (mx, my) = (sx / n, sy / n);
            (xx / n - mx * mx, yy / n - my * my)
        };
        let (vx, vy) = ink(&a, w);
        assert!(vy > 4.0 * vx, "upright bar should be tall: var x {vx}, y {vy}");
        let turned = DabShape {
            rotation: std::f64::consts::FRAC_PI_2,
            ..upright
        };
        let (b, w2, _) = t.mask(SCALE, turned, 0.25, 0.5);
        let (vx, vy) = ink(&b, w2);
        assert!(vx > 4.0 * vy, "turned bar should be wide: var x {vx}, y {vy}");
    }

    #[test]
    fn mask_size_agrees_with_the_drawn_image_to_a_pixel() {
        let t = tip();
        for k in 0..24 {
            let shape = DabShape {
                scale: 0.3 + 0.03 * k as f64,
                ratio: 1.0,
                rotation: k as f64 * 0.27,
            };
            let (w, h) = t.mask_size(SCALE, shape, 0.3, 0.7);
            let (_, iw, ih) = t.mask(SCALE, shape, 0.3, 0.7);
            assert!((w - iw as i32).abs() <= 1 && (h - ih as i32).abs() <= 1, "{w}x{h} vs {iw}x{ih}");
        }
    }
}
