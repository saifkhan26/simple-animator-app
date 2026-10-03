//! Krita's pixel brush (the `paintbrush` paintop), ported closely enough to
//! paint a Krita preset the way Krita 5.2 paints it.
//!
//! Only what the presets we ship use is here, and a preset that asks for
//! anything more is refused with the setting's name rather than painted
//! approximately. Today that is one preset, `c) Pencil-5 Tilted`, embedded
//! in the binary together with its tip; its paper texture travels inside
//! the preset itself.
//!
//! A dab goes through the same steps as in `KisBrushOp::paintAt` and the
//! dab executor behind it:
//!
//! 1. sensors → size, rotation, opacity, texture strength (`sensors`);
//! 2. the dab's rectangle and sub-pixel offset (`KisDabCacheBase`), and
//!    whether the last dab's mask can be reused (Krita's precision option);
//! 3. the mask: the tip drawn by Qt at that shape and offset (`tip`, `qt`);
//! 4. the texture multiplied in, anchored to the canvas (`texture`);
//! 5. the dab painted straight onto the layer in build-up mode, with 8-bit
//!    arithmetic (`composite`);
//! 6. the spacing to the next dab, from the dab's size (`spacing`).
//!
//! What cannot match Krita exactly is upstream of all this: where the pen
//! samples land. This app maps tablet packets itself, in single precision,
//! so the same physical stroke arrives here a hair away from where Krita
//! would put it.

pub mod composite;
pub mod curve;
pub mod kpp;
pub mod qt;
pub mod sensors;
pub mod spacing;
pub mod texture;
pub mod tip;

use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, bail, Context, Result};

use crate::doc::canvas::{Canvas, DirtyRect};
use crate::input::pointer::PointerSample;
use crate::tools::lasso::Mask;

use kpp::Preset;
use qt::{qround, Image32};
use sensors::{CurveOption, PaintInfo};
use spacing::{effective_spacing, Distance, Spacing};
use texture::{TextureMask, TextureSettings};
use tip::{DabShape, PngTip};

pub(crate) const PENCIL5_KPP: &[u8] = include_bytes!("assets/Pencil-5_Tilted.kpp");
pub(crate) const GRADIENT_PNG: &[u8] = include_bytes!("assets/gradient.png");

/// The Krita presets this app carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KritaPreset {
    #[default]
    Pencil5Tilted,
}

impl KritaPreset {
    /// The loaded preset. Built once; the embedded files are checked by the
    /// tests, so a failure here is a build that should never have shipped.
    pub fn brush(self) -> &'static PixelBrush {
        static PENCIL5: OnceLock<PixelBrush> = OnceLock::new();
        match self {
            KritaPreset::Pencil5Tilted => PENCIL5.get_or_init(|| {
                PixelBrush::load(PENCIL5_KPP, &[("gradient.png", GRADIENT_PNG)])
                    .expect("embedded Pencil-5 Tilted preset")
            }),
        }
    }
}

/// `precisionLevels` in kis_dab_cache_base.cpp: how far a dab may drift
/// from the last one generated before its mask is made afresh.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Precision {
    angle: f64,
    size_frac: f64,
    sub_pixel: f64,
    ratio: f64,
}

const PRECISION_LEVELS: [Precision; 5] = [
    Precision {
        angle: std::f64::consts::PI / 180.0,
        size_frac: 0.05,
        sub_pixel: 1.0,
        ratio: 0.05,
    },
    Precision {
        angle: std::f64::consts::PI / 180.0,
        size_frac: 0.01,
        sub_pixel: 1.0,
        ratio: 0.01,
    },
    Precision {
        angle: std::f64::consts::PI / 180.0,
        size_frac: 0.0,
        sub_pixel: 1.0,
        ratio: 1e-6,
    },
    Precision {
        angle: std::f64::consts::PI / 180.0,
        size_frac: 0.0,
        sub_pixel: 0.5,
        ratio: 1e-6,
    },
    Precision {
        angle: 1e-6,
        size_frac: 0.0,
        sub_pixel: 1e-6,
        ratio: 1e-6,
    },
];

/// A pixel-brush preset, loaded and checked.
pub struct PixelBrush {
    pub tip: PngTip,
    /// The preset's own `KisBrush::scale`.
    pub tip_scale: f64,
    pub size: CurveOption,
    pub rotation: CurveOption,
    pub opacity: CurveOption,
    pub texture_strength: CurveOption,
    pub texture: Option<TextureMask>,
    spacing: f64,
    auto_spacing: Option<f64>,
    isotropic_spacing: bool,
    precision: Precision,
}

/// Settings a preset may carry only switched off: each is a Krita feature
/// this port does not paint.
const MUST_BE_OFF: &[&str] = &[
    "PressureRatio",
    "PressureScatter",
    "PressureMirror",
    "PressureSoftness",
    "PressureSharpness",
    "PressureDarken",
    "PressureMix",
    "Pressureh",
    "Pressures",
    "Pressurev",
    "PressureSpacing",
    "PressureRate",
    "PressureLightnessStrength",
    "MaskingBrush/Enabled",
    "PaintOpSettings/isAirbrushing",
    "PaintOpSettings/updateSpacingBetweenDabs",
    "KisPrecisionOption/AutoPrecisionEnabled",
    "EraserMode",
    "Texture/Pattern/isRandomOffsetX",
    "Texture/Pattern/isRandomOffsetY",
    "Texture/Pattern/Invert",
];

impl PixelBrush {
    /// Load a `.kpp`. `resources` supplies the tip files the preset names,
    /// by file name.
    pub fn load(kpp: &[u8], resources: &[(&str, &[u8])]) -> Result<Self> {
        let p = Preset::read(kpp)?;
        if p.paintop != "paintbrush" {
            bail!("`{}` uses the `{}` engine; only the pixel brush is supported", p.name, p.paintop);
        }
        for key in MUST_BE_OFF {
            if p.bool(key, false) {
                bail!("`{}` turns on `{key}`, which is not supported yet", p.name);
            }
        }
        // Build-up (1) paints dabs straight onto the layer; wash (2, the
        // default) goes through a stroke buffer with alpha-darken.
        if p.int("PaintOpAction", 2) != 1 {
            bail!("`{}` paints in wash mode; only build-up is supported yet", p.name);
        }
        if let Some(op) = p.string("CompositeOp") {
            if op != "normal" {
                bail!("`{}` blends with `{op}`; only normal is supported", p.name);
            }
        }
        if let Some(src) = p.string("ColorSource/Type") {
            if src != "plain" {
                bail!("`{}` takes its colour from `{src}`; only plain colour is supported", p.name);
            }
        }

        // brush_definition: <Brush type=… filename=… scale=… angle=… />
        let def = p
            .string("brush_definition")
            .ok_or_else(|| anyhow!("`{}` has no brush tip", p.name))?;
        let doc = roxmltree::Document::parse(def.trim()).context("brush_definition")?;
        let b = doc.root_element();
        let attr = |k: &str| b.attribute(k);
        let num = |k: &str, d: f64| attr(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
        if attr("type") != Some("png_brush") {
            bail!("`{}` uses a `{}` tip; only PNG tips are supported", p.name, attr("type").unwrap_or("?"));
        }
        let file = attr("filename").unwrap_or_default();
        let png = resources
            .iter()
            .find(|(n, _)| *n == file)
            .map(|(_, d)| *d)
            .ok_or_else(|| anyhow!("`{}` needs the tip `{file}`", p.name))?;
        let mut tip_scale = num("scale", 1.0);
        // KisBrush::fromXMLLoadResult: version-1 definitions stored half the
        // scale.
        if attr("BrushVersion").unwrap_or("1") == "1" {
            tip_scale *= 2.0;
        }
        let angle = num("angle", 0.0);
        let spacing = num("spacing", 0.1).max(0.02);
        let auto_spacing = (attr("useAutoSpacing") == Some("1")).then(|| num("autoSpacingCoeff", 1.0));
        let tip = PngTip::load(png, angle).with_context(|| format!("tip `{file}`"))?;

        let texture = if p.bool("Texture/Pattern/Enabled", false) {
            if p.int("Texture/Pattern/TexturingMode", 0) != 0 {
                bail!("`{}` uses a texturing mode other than multiply", p.name);
            }
            let outer = p
                .bytes("Texture/Pattern/Pattern")
                .ok_or_else(|| anyhow!("`{}` does not embed its texture", p.name))?;
            let png = kpp_base64(outer);
            let pattern = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
                .context("decoding the texture")?
                .to_rgba8();
            let (w, h) = pattern.dimensions();
            let img = Image32 {
                w: w as usize,
                h: h as usize,
                px: pattern
                    .pixels()
                    .map(|q| {
                        let [r, g, b, a] = q.0;
                        qt::q_rgba(r as u32, g as u32, b as u32, a as u32)
                    })
                    .collect(),
            };
            if img.px.iter().any(|&q| qt::q_alpha(q) != 255) {
                bail!("`{}` has a texture with transparency, which is not supported yet", p.name);
            }
            Some(TextureMask::build(
                &img,
                &TextureSettings {
                    scale: p.double("Texture/Pattern/Scale", 1.0),
                    brightness: p.double("Texture/Pattern/Brightness", 0.0),
                    contrast: p.double("Texture/Pattern/Contrast", 1.0),
                    neutral_point: p.double("Texture/Pattern/NeutralPoint", 0.5),
                    invert: false,
                    cutoff_left: p.int("Texture/Pattern/CutoffLeft", 0),
                    cutoff_right: p.int("Texture/Pattern/CutoffRight", 255),
                    cutoff_policy: p.int("Texture/Pattern/CutoffPolicy", 0),
                    offset_x: p.int("Texture/Pattern/OffsetX", 0),
                    offset_y: p.int("Texture/Pattern/OffsetY", 0),
                },
            ))
        } else {
            None
        };

        let level = p.int("KisPrecisionOption/precisionLevel", 5).clamp(1, 5);
        Ok(Self {
            tip,
            tip_scale,
            size: p.curve_option("Size", true)?,
            rotation: p.curve_option("Rotation", true)?,
            opacity: p.curve_option("Opacity", false)?,
            texture_strength: p.curve_option("Texture/Strength/", true)?,
            texture,
            spacing,
            auto_spacing,
            isotropic_spacing: p.bool("Spacing/Isotropic", false),
            precision: PRECISION_LEVELS[(level - 1) as usize],
        })
    }

    /// The preset's own brush size, in pixels.
    pub fn diameter(&self) -> f64 {
        let (w, h) = self.tip.size();
        w.max(h) as f64 * self.tip_scale
    }

    /// `KisBrush::scale` for a brush `diameter` pixels across. At the
    /// preset's own size this is the preset's scale, not one recomputed from
    /// a rounded diameter, so an untouched brush is the preset.
    fn brush_scale(&self, diameter: f64) -> f64 {
        let (w, h) = self.tip.size();
        if (diameter - self.diameter()).abs() < 0.01 {
            self.tip_scale
        } else {
            diameter / w.max(h) as f64
        }
    }

    /// The brush outline for the pen as it is now: the tip's footprint,
    /// sized and turned by the same sensors a dab would be, as corners
    /// around the pointer in canvas pixels. Krita's cursor does the same
    /// (`KisCurrentOutlineFetcher`), so leaning the pen turns and grows it.
    pub fn outline(
        &self,
        diameter: f64,
        pressure: f32,
        tilt: (f32, f32),
        rotation_deg: f64,
        mirrored: (bool, bool),
    ) -> [(f64, f64); 4] {
        let info = PaintInfo {
            pressure: pressure.clamp(0.0, 1.0) as f64,
            x_tilt: qt_tilt(tilt.0) as f64,
            y_tilt: qt_tilt(tilt.1) as f64,
            canvas_rotation: rotation_deg,
            canvas_mirrored_h: mirrored.0,
            canvas_mirrored_v: mirrored.1,
        };
        let shape = DabShape {
            scale: self.size.apply(&info),
            ratio: 1.0,
            rotation: sensors::rotation(&self.rotation, &info),
        };
        self.tip.outline(self.brush_scale(diameter), shape)
    }
}

fn kpp_base64(text: &[u8]) -> Vec<u8> {
    // The embedded pattern is base64 text inside the bytearray.
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => continue,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// What the stroke asked for at one dab position, decided when the path
/// walker reaches it and painted when the stroke next flushes.
#[derive(Clone, Copy, Debug)]
struct DabRequest {
    x: f64,
    y: f64,
    shape: DabShape,
    opacity: f64,
    texture_strength: f64,
}

/// `SavedDabParameters`: what decides whether the last mask can be reused.
#[derive(Clone, Copy, Debug, PartialEq)]
struct DabParams {
    angle: f64,
    w: i32,
    h: i32,
    sub_x: f64,
    sub_y: f64,
    ratio: f64,
}

impl DabParams {
    fn matches(&self, last: &DabParams, prec: &Precision) -> bool {
        (self.angle - last.angle).abs() <= prec.angle
            && (self.w - last.w).abs() <= (prec.size_frac * self.w as f64) as i32
            && (self.h - last.h).abs() <= (prec.size_frac * self.h as f64) as i32
            && (self.sub_x - last.sub_x).abs() <= prec.sub_pixel
            && (self.sub_y - last.sub_y).abs() <= prec.sub_pixel
            && (self.ratio - last.ratio).abs() <= prec.ratio
    }
}

/// The last mask generated, before texture: Krita's "original device".
struct CachedDab {
    params: DabParams,
    alpha: Vec<u8>,
    w: usize,
    h: usize,
}

/// One stroke with a Krita brush.
pub struct KritaStroke {
    brush: &'static PixelBrush,
    brush_scale: f64,
    color: [u8; 3],
    /// The preset's opacity option with the app's opacity slider as its
    /// strength, which is what Krita's own opacity slider sets.
    opacity: CurveOption,
    erase: bool,
    canvas_rotation: f64,
    mirrored: (bool, bool),
    distance: Distance,
    pending: Vec<DabRequest>,
    cache: Option<CachedDab>,
}

/// Qt hands tilt to Krita in whole degrees, truncated.
pub fn qt_tilt(t: f32) -> f32 {
    t.trunc()
}

/// `KisPaintOp::splitCoordinate`.
fn split(c: f64) -> (i32, f64) {
    let i = c.floor() as i32;
    (i, c - i as f64)
}

impl KritaStroke {
    /// `diameter` is the brush size in canvas pixels, `rotation_deg` the
    /// view's rotation, positive clockwise on screen.
    pub fn new(
        preset: KritaPreset,
        diameter: f64,
        color: [u8; 4],
        opacity: f64,
        erase: bool,
        rotation_deg: f64,
        mirrored: (bool, bool),
    ) -> Self {
        let brush = preset.brush();
        let brush_scale = brush.brush_scale(diameter);
        let mut opacity_option = brush.opacity.clone();
        opacity_option.strength = opacity.clamp(0.0, 1.0);
        Self {
            brush,
            brush_scale,
            color: [color[0], color[1], color[2]],
            opacity: opacity_option,
            erase,
            canvas_rotation: rotation_deg,
            mirrored,
            // KisSpacingInformation(): zero spacing until the first dab.
            distance: Distance::new(Spacing::isotropic(0.0)),
            pending: Vec::new(),
            cache: None,
        }
    }

    fn info(&self, s: &PointerSample) -> PaintInfo {
        PaintInfo {
            pressure: s.pressure.clamp(0.0, 1.0) as f64,
            x_tilt: s.tilt_x as f64,
            y_tilt: s.tilt_y as f64,
            canvas_rotation: self.canvas_rotation,
            canvas_mirrored_h: self.mirrored.0,
            canvas_mirrored_v: self.mirrored.1,
        }
    }

    /// Where along `start → end` the next dab falls (0..=1), or negative.
    pub fn next_point(&mut self, start: (f32, f32), end: (f32, f32)) -> f32 {
        let t = self
            .distance
            .next_point((start.0 as f64, start.1 as f64), (end.0 as f64, end.1 as f64));
        t as f32
    }

    /// `KisBrushOp::paintAt`: decide the dab at `s` and the spacing to the
    /// next one. The dab is painted by the next [`drain`](Self::drain).
    pub fn paint_at(&mut self, s: &PointerSample) {
        let b = self.brush;
        let info = self.info(s);
        let scale = b.size.apply(&info);
        let (w, h) = b.tip.size();
        // checkSizeTooSmall.
        let s_px = scale * self.brush_scale;
        if s_px * (w as f64) < 0.01 || s_px * (h as f64) < 0.01 {
            self.distance.set_spacing(Spacing::isotropic(0.0));
            return;
        }
        let rotation = sensors::rotation(&b.rotation, &info);
        let shape = DabShape {
            scale,
            ratio: 1.0,
            rotation,
        };
        // KisFlowOpacityOption2::apply in build-up: the opacity option's
        // value, strength included. Flow is computed there too, but the
        // 8-bit "normal" op build-up paints with never reads it.
        let opacity = if self.opacity.checked {
            self.opacity.size_like(&info, true)
        } else {
            1.0
        };
        let texture_strength = b.texture_strength.apply(&info);
        self.pending.push(DabRequest {
            x: s.x as f64,
            y: s.y as f64,
            shape,
            opacity,
            texture_strength,
        });

        // effectiveSpacing: the dab's size unrotated; the rotation turns
        // the spacing ellipse.
        let (cw, ch) = b.tip.characteristic_size(
            self.brush_scale,
            DabShape {
                scale,
                ratio: 1.0,
                rotation: 0.0,
            },
        );
        // `flipped` is the Mirror option's, which this brush has off; the
        // canvas's own mirroring does not enter.
        let spacing = effective_spacing(
            cw,
            ch,
            b.isotropic_spacing,
            rotation,
            false,
            b.spacing,
            b.auto_spacing.is_some(),
            b.auto_spacing.unwrap_or(1.0),
        );
        self.distance.set_spacing(spacing);
    }

    /// Paint every dab decided since the last drain onto `canvas`.
    pub fn drain(&mut self, canvas: &mut Canvas, clip: Option<&Arc<Mask>>, alpha_lock: bool) -> Option<DirtyRect> {
        let mut acc: Option<DirtyRect> = None;
        let pending = std::mem::take(&mut self.pending);
        for req in pending {
            if let Some(r) = self.render(req, canvas, clip, alpha_lock) {
                acc = Some(crate::tools::ribbon::union_rect(acc, r));
            }
        }
        acc
    }

    fn render(&mut self, req: DabRequest, canvas: &mut Canvas, clip: Option<&Arc<Mask>>, alpha_lock: bool) -> Option<DirtyRect> {
        let b = self.brush;
        // calculateDabRect.
        let (hx, hy) = b.tip.hot_spot(self.brush_scale, req.shape);
        let (x, sub_x) = split(req.x - hx);
        let (y, sub_y) = split(req.y - hy);
        let (w, h) = b.tip.mask_size(self.brush_scale, req.shape, sub_x, sub_y);
        let params = DabParams {
            angle: req.shape.rotation,
            w,
            h,
            sub_x,
            sub_y,
            ratio: req.shape.ratio,
        };
        let reuse = self
            .cache
            .as_ref()
            .is_some_and(|c| params.matches(&c.params, &b.precision));
        if !reuse {
            let (alpha, mw, mh) = b.tip.mask(self.brush_scale, req.shape, sub_x, sub_y);
            self.cache = Some(CachedDab {
                params,
                alpha,
                w: mw,
                h: mh,
            });
        }
        let cached = self.cache.as_ref()?;
        let (dw, dh) = (cached.w, cached.h);
        let mut alpha = cached.alpha.clone();
        if let Some(tex) = &b.texture {
            // Textured at the dab's theoretical corner, not where the
            // (possibly reused) mask ends up being centred.
            tex.apply(&mut alpha, dw, dh, (x, y), req.texture_strength);
        }
        // KisDabRenderingJob::dstDabOffset: centre the mask in the rect.
        let ox = x + qround((w - dw as i32) as f64 / 2.0);
        let oy = y + qround((h - dh as i32) as f64 / 2.0);

        let (cw, ch) = (canvas.width as i32, canvas.height as i32);
        let x0 = ox.max(0);
        let y0 = oy.max(0);
        let x1 = (ox + dw as i32).min(cw);
        let y1 = (oy + dh as i32).min(ch);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        if self.erase && alpha_lock {
            return None;
        }
        let opacity = req.opacity as f32;
        for py in y0..y1 {
            for px in x0..x1 {
                let a = alpha[(py - oy) as usize * dw + (px - ox) as usize];
                if a == 0 {
                    continue;
                }
                let mask = clip.map(|m| m.at(px as u32, py as u32));
                let idx = (py as usize * canvas.width as usize + px as usize) * 4;
                let dst = &mut canvas.pixels_mut()[idx..idx + 4];
                if self.erase {
                    composite::erase(dst, a, opacity, mask);
                } else {
                    if alpha_lock && dst[3] == 0 {
                        // Krita recolours even empty pixels here; nothing
                        // shows either way, and ours stay clean.
                        continue;
                    }
                    composite::over(dst, self.color, a, opacity, mask, alpha_lock);
                }
            }
        }
        canvas.mark_dirty(x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32);
        Some(DirtyRect {
            min_x: x0 as u32,
            min_y: y0 as u32,
            max_x: x1 as u32,
            max_y: y1 as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The preset's angle is the literal 3.14159, not π.
    #[allow(clippy::approx_constant)]
    #[test]
    fn embedded_pencil_loads_as_krita_configures_it() {
        let b = KritaPreset::Pencil5Tilted.brush();
        assert!((b.diameter() - 40.0).abs() < 1e-3);
        assert!((b.tip_scale - 0.181818).abs() < 1e-12);
        assert!((b.tip.angle - 3.14159).abs() < 1e-12);
        assert_eq!(b.auto_spacing, Some(0.3));
        assert!(!b.isotropic_spacing);
        assert_eq!(b.precision, PRECISION_LEVELS[0]);

        // Size: tilt elevation through 0,1 → 1,0.246.
        assert!(b.size.checked);
        assert_eq!(b.size.sensors.len(), 1);
        assert_eq!(b.size.sensors[0].kind, sensors::SensorKind::TiltElevation);
        let upright = PaintInfo {
            pressure: 1.0,
            ..Default::default()
        };
        let flat = PaintInfo {
            pressure: 1.0,
            x_tilt: 60.0,
            ..Default::default()
        };
        assert!((b.size.apply(&upright) - 0.246231).abs() < 1e-9);
        assert!((b.size.apply(&flat) - 1.0).abs() < 1e-9);

        // Rotation: tilt direction, linear.
        assert!(b.rotation.checked);
        assert_eq!(b.rotation.sensors[0].kind, sensors::SensorKind::TiltDirection);

        // Opacity: pressure through the preset's curve; always on.
        assert!(b.opacity.checked);
        assert_eq!(b.opacity.sensors[0].kind, sensors::SensorKind::Pressure);
        let light = PaintInfo {
            pressure: 0.144578,
            ..Default::default()
        };
        assert!((b.opacity.size_like(&light, true) - 0.0481932).abs() < 2e-3);

        // Texture strength follows pressure.
        assert!(b.texture_strength.checked);
        let half = PaintInfo {
            pressure: 0.5,
            ..Default::default()
        };
        assert!((b.texture_strength.apply(&half) - 0.5).abs() < 1e-12);

        let tex = b.texture.as_ref().expect("texture");
        assert_eq!((tex.w, tex.h), (307, 307));
    }

    /// The cursor outline is the bar: long and thin, a quarter size with the
    /// pen upright and full size laid flat, and it turns with the lean.
    #[test]
    fn outline_is_the_bar_sized_and_turned_like_a_dab() {
        let b = KritaPreset::Pencil5Tilted.brush();
        let sides = |c: [(f64, f64); 4]| {
            let d = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).hypot(p.1 - q.1);
            (d(c[0], c[1]), d(c[1], c[2]))
        };
        let upright = b.outline(40.0, 1.0, (0.0, 0.0), 0.0, (false, false));
        let flat = b.outline(40.0, 1.0, (60.0, 0.0), 0.0, (false, false));
        let (uw, uh) = sides(upright);
        let (fw, fh) = sides(flat);
        assert!(uh > 4.0 * uw, "a thin bar: {uw} x {uh}");
        assert!((fh - 40.0).abs() < 1.0, "full size flat: {fh}");
        assert!((fh / uh - 1.0 / 0.246231).abs() < 0.01, "{uh} -> {fh}");
        assert!((fw / fh - uw / uh).abs() < 1e-9);

        // Leaning along x lays the bar along x; along y, along y.
        let long_axis = |c: [(f64, f64); 4]| {
            let e = (c[2].0 - c[1].0, c[2].1 - c[1].1);
            e.1.atan2(e.0).rem_euclid(std::f64::consts::PI)
        };
        let ax = long_axis(flat);
        let ay = long_axis(b.outline(40.0, 1.0, (0.0, 60.0), 0.0, (false, false)));
        let d = (ax - ay).abs();
        assert!((d - std::f64::consts::FRAC_PI_2).abs() < 1e-6, "{}", d.to_degrees());
    }

    /// A `.kpp` holding `xml`: just the text chunk, which is all the
    /// reader looks at.
    fn kpp_with(xml: &str) -> Vec<u8> {
        let mut body = b"preset\0".to_vec();
        body.extend_from_slice(xml.as_bytes());
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&(body.len() as u32).to_be_bytes());
        v.extend_from_slice(b"tEXt");
        v.extend_from_slice(&body);
        v.extend_from_slice(&[0; 4]);
        v
    }

    #[test]
    fn unsupported_settings_are_refused_by_name() {
        let xml = kpp::preset_xml(PENCIL5_KPP).unwrap();
        let tips: &[(&str, &[u8])] = &[("gradient.png", GRADIENT_PNG)];
        assert!(PixelBrush::load(&kpp_with(&xml), tips).is_ok());

        let wash = xml.replace(r#"name="PaintOpAction">1<"#, r#"name="PaintOpAction">2<"#);
        let err = PixelBrush::load(&kpp_with(&wash), tips).err().unwrap();
        assert!(err.to_string().contains("wash"), "{err}");

        let scatter = xml.replace(r#"name="PressureScatter">false<"#, r#"name="PressureScatter">true<"#);
        let err = PixelBrush::load(&kpp_with(&scatter), tips).err().unwrap();
        assert!(err.to_string().contains("PressureScatter"), "{err}");

        let err = PixelBrush::load(&kpp_with(&xml), &[]).err().unwrap();
        assert!(err.to_string().contains("gradient.png"), "{err}");
    }
}
