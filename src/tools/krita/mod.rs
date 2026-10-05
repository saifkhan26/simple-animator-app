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
pub mod defaults;
pub mod distance;
pub mod kpp;
pub mod options;
pub mod qt;
pub mod random;
pub mod sensors;
pub mod spacing;
pub mod texture;
pub mod tip;

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, bail, Context, Result};

use crate::doc::canvas::{Canvas, DirtyRect};
use crate::input::pointer::PointerSample;
use crate::tools::lasso::Mask;
use crate::tools::ribbon::StrokeWorkspace;

use distance::DabHistory;
use kpp::Preset;
use random::RandomSource;
use qt::{qround, Image32};
use sensors::{CurveOption, PaintInfo};
use spacing::{effective_spacing, Distance, Spacing};
use texture::{TextureMask, TextureSettings};
use tip::auto::{AutoDef, AutoTip};
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
    pub fn brush(self) -> Arc<PixelBrush> {
        static PENCIL5: OnceLock<Arc<PixelBrush>> = OnceLock::new();
        match self {
            KritaPreset::Pencil5Tilted => PENCIL5
                .get_or_init(|| {
                    Arc::new(
                        PixelBrush::load(PENCIL5_KPP, &[("gradient.png", GRADIENT_PNG)])
                            .expect("embedded Pencil-5 Tilted preset"),
                    )
                })
                .clone(),
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
    softness: f64,
    ratio: f64,
}

const PRECISION_LEVELS: [Precision; 5] = [
    Precision {
        angle: std::f64::consts::PI / 180.0,
        size_frac: 0.05,
        sub_pixel: 1.0,
        softness: 0.01,
        ratio: 0.05,
    },
    Precision {
        angle: std::f64::consts::PI / 180.0,
        size_frac: 0.01,
        sub_pixel: 1.0,
        softness: 0.01,
        ratio: 0.01,
    },
    Precision {
        angle: std::f64::consts::PI / 180.0,
        size_frac: 0.0,
        sub_pixel: 1.0,
        softness: 0.01,
        ratio: 1e-6,
    },
    Precision {
        angle: std::f64::consts::PI / 180.0,
        size_frac: 0.0,
        sub_pixel: 0.5,
        softness: 0.01,
        ratio: 1e-6,
    },
    Precision {
        angle: 1e-6,
        size_frac: 0.0,
        sub_pixel: 1e-6,
        softness: 1e-6,
        ratio: 1e-6,
    },
];

/// `KisPrecisionOption`: the preset's precision level, or with auto
/// precision one chosen per dab from its size.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PrecisionOption {
    /// 1..=5.
    level: i32,
    auto: bool,
    /// Scatter, rotation or a texture is on: dab positions are imprecise
    /// anyway, so auto precision may reuse masks more freely.
    imprecise: bool,
}

impl PrecisionOption {
    /// `effectivePrecisionLevel`, as the index `SavedDabParameters::compare`
    /// takes. `dab_size` is the smaller side of the dab's mask.
    fn for_dab(&self, dab_size: i32) -> &'static Precision {
        let level = if !self.auto {
            self.level
        } else if (dab_size as f64) < 30.0 || !self.imprecise {
            5
        } else {
            3
        };
        &PRECISION_LEVELS[(level.clamp(1, 5) - 1) as usize]
    }
}

/// A preset's brush tip.
pub enum Tip {
    /// A bitmap, scaled to size.
    Png(PngTip),
    /// Generated: the size slider resizes the generator itself, so each
    /// stroke builds its own [`AutoTip`] from this.
    Auto(AutoDef),
}

/// A pixel-brush preset, loaded and checked.
pub struct PixelBrush {
    pub tip: Tip,
    /// The preset's own `KisBrush::scale`.
    pub tip_scale: f64,
    pub size: CurveOption,
    pub rotation: CurveOption,
    pub opacity: CurveOption,
    pub flow: CurveOption,
    pub texture_strength: CurveOption,
    pub texture: Option<TextureMask>,
    spacing: f64,
    auto_spacing: Option<f64>,
    isotropic_spacing: bool,
    precision: PrecisionOption,
    /// Wash mode: dabs build up in a stroke buffer with alpha darken, and
    /// the buffer is merged onto the layer. Otherwise build-up: each dab
    /// goes straight onto the layer.
    pub wash: bool,
    /// The preset erases (its blend is `erase`, or it was saved in eraser
    /// mode), whatever tool paints with it.
    pub erase: bool,
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
];
// Not here, though presets set them: `Texture/Pattern/isRandomOffsetX/Y`.
// Krita 5.2 reads and writes them (KisTextureOptionData) but nothing paints
// with them, so they change nothing.

/// Where a preset's tips come from when it does not carry them itself:
/// the folder or bundle it was found in, or files embedded in the app.
pub trait Resources {
    /// A brush tip, by the file name the preset gives.
    fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>>;
}

impl Resources for [(&str, &[u8])] {
    fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>> {
        self.iter().find(|(n, _)| *n == filename).map(|(_, d)| Cow::Borrowed(*d))
    }
}

impl<const N: usize> Resources for [(&str, &[u8]); N] {
    fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>> {
        self[..].tip(filename)
    }
}

impl PixelBrush {
    /// Load a `.kpp`. A tip the preset embeds (a Krita 5 preset carries
    /// its own) is used first; otherwise `resources` supplies it.
    pub fn load<R: Resources + ?Sized>(kpp: &[u8], resources: &R) -> Result<Self> {
        let p = Preset::read(kpp)?;
        if p.paintop != "paintbrush" {
            bail!("`{}` uses the `{}` engine; only the pixel brush is supported", p.name, p.paintop);
        }
        for key in MUST_BE_OFF {
            if p.bool(key, false) {
                bail!("`{}` turns on `{key}`, which is not supported yet", p.name);
            }
        }
        // Build-up (1) paints dabs straight onto the layer; wash (2) goes
        // through a stroke buffer with alpha-darken. A preset without the
        // key is build-up (`KisBrushBasedPaintOpSettings::paintIncremental`).
        let wash = match p.int("PaintOpAction", 1) {
            1 => false,
            2 => true,
            m => bail!("`{}` has an unknown painting mode {m}", p.name),
        };
        let erase_op = match p.string("CompositeOp").as_deref() {
            None | Some("normal") => false,
            Some("erase") => true,
            Some(op) => bail!("`{}` blends with `{op}`; only normal and erase are supported", p.name),
        };
        let erase = erase_op || p.bool("EraserMode", false);
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
        let (tip, tip_scale, spacing, auto_spacing) = match attr("type") {
            Some("png_brush") => {
                let file = attr("filename").unwrap_or_default();
                let embedded = p
                    .resources
                    .iter()
                    .find(|r| r.kind == "brushes" && r.filename == file)
                    .map(|r| Cow::Borrowed(r.data.as_slice()));
                let png = embedded
                    .or_else(|| resources.tip(file))
                    .ok_or_else(|| anyhow!("`{}` needs the tip `{file}`", p.name))?;
                let mut tip_scale = num("scale", 1.0);
                // KisBrush::fromXMLLoadResult: version-1 definitions stored
                // half the scale.
                if attr("BrushVersion").unwrap_or("1") == "1" {
                    tip_scale *= 2.0;
                }
                let angle = num("angle", 0.0);
                let spacing = num("spacing", 0.1).max(0.02);
                let auto_spacing = (attr("useAutoSpacing") == Some("1")).then(|| num("autoSpacingCoeff", 1.0));
                let tip = PngTip::load(&png, angle).with_context(|| format!("tip `{file}`"))?;
                (Tip::Png(tip), tip_scale, spacing, auto_spacing)
            }
            Some("auto_brush") => {
                let def = AutoDef::parse(b).with_context(|| format!("`{}`", p.name))?;
                // KisAutoBrushFactory: spacing defaults to 1, clamped like
                // any brush's to 0.02.
                let spacing = num("spacing", 1.0).max(0.02);
                let auto = attr("useAutoSpacing").and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0) != 0;
                let auto_spacing = auto.then(|| num("autoSpacingCoeff", 1.0));
                (Tip::Auto(def), 1.0, spacing, auto_spacing)
            }
            other => bail!("`{}` uses a `{}` tip, which is not supported yet", p.name, other.unwrap_or("?")),
        };

        let texture = if p.bool("Texture/Pattern/Enabled", false) {
            if p.int("Texture/Pattern/TexturingMode", 0) != 0 {
                bail!("`{}` uses a texturing mode other than multiply, which is not supported yet", p.name);
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
                    invert: p.bool("Texture/Pattern/Invert", false),
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

        let size = p.curve_option(&options::SIZE)?;
        let rotation = p.curve_option(&options::ROTATION)?;
        let opacity = p.curve_option(&options::OPACITY)?;
        let flow = p.curve_option(&options::FLOW)?;
        let texture_strength = p.curve_option(&options::TEXTURE_STRENGTH)?;
        // Read but not painted yet: refuse rather than paint them wrong.
        for (name, o) in [("Size", &size), ("Rotation", &rotation), ("Opacity", &opacity), ("Flow", &flow), ("Texture strength", &texture_strength)] {
            if o.checked && o.sensors.iter().any(|s| s.kind == sensors::SensorKind::Speed) {
                bail!("`{}`: {name} follows the speed sensor, which is not supported yet", p.name);
            }
        }
        if rotation.checked && rotation.fan_corners.is_some() {
            bail!("`{}`: drawing-angle fan corners are not supported yet", p.name);
        }
        let precision = PrecisionOption {
            level: p.int("KisPrecisionOption/precisionLevel", 5).clamp(1, 5),
            auto: p.bool("KisPrecisionOption/AutoPrecisionEnabled", false),
            // KisBrushOp and KisBrushBasedPaintOp's imprecise options.
            imprecise: rotation.checked || texture.is_some(),
        };
        Ok(Self {
            tip,
            tip_scale,
            size,
            rotation,
            opacity,
            flow,
            texture_strength,
            texture,
            spacing,
            auto_spacing,
            isotropic_spacing: p.bool("Spacing/Isotropic", false),
            precision,
            wash,
            erase,
        })
    }

    /// The preset's own brush size, in pixels.
    pub fn diameter(&self) -> f64 {
        match &self.tip {
            Tip::Png(t) => {
                let (w, h) = t.size();
                w.max(h) as f64 * self.tip_scale
            }
            Tip::Auto(d) => d.diameter,
        }
    }

    /// `KisBrush::scale` for a brush `diameter` pixels across. At the
    /// preset's own size this is the preset's scale, not one recomputed from
    /// a rounded diameter, so an untouched brush is the preset. An auto tip
    /// is resized through its generator instead, so its scale stays 1.
    fn brush_scale(&self, diameter: f64) -> f64 {
        match &self.tip {
            Tip::Png(t) => {
                let (w, h) = t.size();
                if (diameter - self.diameter()).abs() < 0.01 {
                    self.tip_scale
                } else {
                    diameter / w.max(h) as f64
                }
            }
            Tip::Auto(_) => 1.0,
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
    ) -> Vec<(f64, f64)> {
        let info = PaintInfo {
            pressure: pressure.clamp(0.0, 1.0) as f64,
            x_tilt: qt_tilt(tilt.0) as f64,
            y_tilt: qt_tilt(tilt.1) as f64,
            canvas_rotation: rotation_deg,
            canvas_mirrored_h: mirrored.0,
            canvas_mirrored_v: mirrored.1,
            hovering: true,
            ..Default::default()
        };
        let shape = DabShape {
            scale: self.size.apply(&info),
            ratio: 1.0,
            rotation: sensors::rotation(&self.rotation, &info),
        };
        match &self.tip {
            Tip::Png(t) => t.outline(self.brush_scale(diameter), shape).to_vec(),
            Tip::Auto(d) => AutoTip::new(d, diameter, RandomSource::new(0)).outline(shape),
        }
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
    /// Wash mode's alpha-darken flow; build-up ignores it.
    flow: f64,
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
    softness: f64,
    ratio: f64,
    /// A pipe tip's cell.
    index: i32,
}

impl DabParams {
    fn matches(&self, last: &DabParams, prec: &Precision) -> bool {
        (self.angle - last.angle).abs() <= prec.angle
            && (self.w - last.w).abs() <= (prec.size_frac * self.w as f64) as i32
            && (self.h - last.h).abs() <= (prec.size_frac * self.h as f64) as i32
            && (self.sub_x - last.sub_x).abs() <= prec.sub_pixel
            && (self.sub_y - last.sub_y).abs() <= prec.sub_pixel
            && (self.softness - last.softness).abs() <= prec.softness
            && (self.ratio - last.ratio).abs() <= prec.ratio
            && self.index == last.index
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
    brush: Arc<PixelBrush>,
    brush_scale: f64,
    color: [u8; 3],
    /// The preset's opacity option with the app's opacity slider as its
    /// strength, which is what Krita's own opacity slider sets.
    opacity: CurveOption,
    erase: bool,
    /// Wash mode's running `averageOpacity`, dab to dab.
    average_opacity: f64,
    canvas_rotation: f64,
    mirrored: (bool, bool),
    distance: Distance,
    /// The dabs painted so far, for the sensors that read the stroke's
    /// history. Set by [`start`](Self::start) at pen-down.
    history: DabHistory,
    random: Rc<RefCell<RandomSource>>,
    /// An auto tip, built at this stroke's size.
    auto_tip: Option<AutoTip>,
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
        brush: Arc<PixelBrush>,
        diameter: f64,
        color: [u8; 4],
        opacity: f64,
        erase: bool,
        rotation_deg: f64,
        mirrored: (bool, bool),
    ) -> Self {
        let brush_scale = brush.brush_scale(diameter);
        let auto_tip = match &brush.tip {
            Tip::Auto(d) => Some(AutoTip::new(d, diameter, RandomSource::for_stroke())),
            Tip::Png(_) => None,
        };
        let mut opacity_option = brush.opacity.clone();
        opacity_option.strength = opacity.clamp(0.0, 1.0);
        let erase = erase || brush.erase;
        Self {
            brush,
            brush_scale,
            color: [color[0], color[1], color[2]],
            opacity: opacity_option,
            erase,
            average_opacity: 0.0,
            canvas_rotation: rotation_deg,
            mirrored,
            // KisSpacingInformation(): zero spacing until the first dab.
            distance: Distance::new(Spacing::isotropic(0.0)),
            history: DabHistory::start((0.0, 0.0), 0.0),
            random: Rc::new(RefCell::new(RandomSource::for_stroke())),
            auto_tip,
            pending: Vec::new(),
            cache: None,
        }
    }

    /// Pen-down at `s`: the history starts there. Krita's starting angle
    /// comes from the hover path before the press, which the app does not
    /// keep; it only shows if the first dab lands exactly on the press.
    pub fn start(&mut self, s: &PointerSample) {
        self.history = DabHistory::start((s.x as f64, s.y as f64), 0.0);
    }

    /// Fix the stroke's random source, so a test sees the same stroke twice.
    #[cfg(test)]
    pub fn seed(&mut self, seed: u32) {
        self.random = Rc::new(RefCell::new(RandomSource::new(seed)));
        if let Some(t) = &mut self.auto_tip {
            t.seed(seed.wrapping_mul(0x9e37_79b9).wrapping_add(1));
        }
    }

    // The tip's geometry, whichever kind it is.

    fn png(&self) -> &PngTip {
        match &self.brush.tip {
            Tip::Png(t) => t,
            Tip::Auto(_) => unreachable!("an auto tip's stroke has its own"),
        }
    }

    fn tip_size(&self) -> (i32, i32) {
        match &self.auto_tip {
            Some(t) => t.size(),
            None => self.png().size(),
        }
    }

    fn characteristic_size(&self, shape: DabShape) -> (f64, f64) {
        match &self.auto_tip {
            Some(t) => t.characteristic_size(shape),
            None => self.png().characteristic_size(self.brush_scale, shape),
        }
    }

    fn hot_spot(&self, shape: DabShape) -> (f64, f64) {
        match &self.auto_tip {
            Some(t) => t.hot_spot(shape),
            None => self.png().hot_spot(self.brush_scale, shape),
        }
    }

    fn mask_size(&self, shape: DabShape, sub_x: f64, sub_y: f64) -> (i32, i32) {
        match &self.auto_tip {
            Some(t) => t.mask_size(shape, sub_x, sub_y),
            None => self.png().mask_size(self.brush_scale, shape, sub_x, sub_y),
        }
    }

    fn tip_mask(&self, shape: DabShape, sub_x: f64, sub_y: f64, softness: f64) -> (Vec<u8>, usize, usize) {
        match &self.auto_tip {
            Some(t) => t.mask(shape, sub_x, sub_y, softness),
            None => self.png().mask(self.brush_scale, shape, sub_x, sub_y),
        }
    }

    fn supports_caching(&self) -> bool {
        self.auto_tip.as_ref().map_or(true, AutoTip::supports_caching)
    }

    /// The preset this stroke paints with.
    pub fn brush(&self) -> &Arc<PixelBrush> {
        &self.brush
    }

    fn info(&self, s: &PointerSample) -> PaintInfo {
        let pos = (s.x as f64, s.y as f64);
        PaintInfo {
            pressure: s.pressure.clamp(0.0, 1.0) as f64,
            x_tilt: s.tilt_x as f64,
            y_tilt: s.tilt_y as f64,
            rotation: s.rotation as f64,
            tangential_pressure: s.tangential as f64,
            speed: s.speed as f64,
            drawing_angle: self.history.drawing_angle(pos),
            dab_seq_no: self.history.dab_seq_no,
            canvas_rotation: self.canvas_rotation,
            canvas_mirrored_h: self.mirrored.0,
            canvas_mirrored_v: self.mirrored.1,
            hovering: false,
            random: Some(self.random.clone()),
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
        self.dab_at(s);
        // KisPaintInformation::paintAt: every dab is registered, even one
        // too small to paint.
        self.history.register_painted_dab((s.x as f64, s.y as f64), s.pressure.clamp(0.0, 1.0) as f64);
    }

    fn dab_at(&mut self, s: &PointerSample) {
        let b = self.brush.clone();
        let info = self.info(s);
        let scale = b.size.apply(&info);
        let (w, h) = self.tip_size();
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
        // KisFlowOpacityOption2::apply: the opacity option's value, with
        // its strength in build-up only; in wash the strength is the merge's
        // opacity instead. Flow is computed either way, but the 8-bit
        // "normal" op build-up paints with never reads it.
        let opacity = if self.opacity.checked {
            self.opacity.size_like(&info, !b.wash)
        } else {
            1.0
        };
        let flow = b.flow.apply(&info);
        let texture_strength = b.texture_strength.apply(&info);
        self.pending.push(DabRequest {
            x: s.x as f64,
            y: s.y as f64,
            shape,
            opacity,
            flow,
            texture_strength,
        });

        // effectiveSpacing: the dab's size unrotated; the rotation turns
        // the spacing ellipse.
        let (cw, ch) = self.characteristic_size(DabShape {
            scale,
            ratio: 1.0,
            rotation: 0.0,
        });
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
    /// Paint every dab decided since the last drain onto `canvas`. A wash
    /// stroke builds them up in `ws`'s buffer and merges the buffer over
    /// `pre`, the layer as it was before the stroke.
    pub fn drain(&mut self, canvas: &mut Canvas, ws: &mut StrokeWorkspace, pre: &[u8]) -> Option<DirtyRect> {
        let mut acc: Option<DirtyRect> = None;
        let pending = std::mem::take(&mut self.pending);
        if !self.brush.wash {
            let clip = ws.clip().cloned();
            for req in pending {
                if let Some(r) = self.render(req, canvas, clip.as_ref(), ws.alpha_lock()) {
                    acc = Some(crate::tools::ribbon::union_rect(acc, r));
                }
            }
            return acc;
        }
        // The app's rule: the eraser does nothing on an alpha-locked layer.
        if self.erase && ws.alpha_lock() {
            return None;
        }
        for req in pending {
            if let Some(r) = self.darken(req, ws) {
                acc = Some(crate::tools::ribbon::union_rect(acc, r));
            }
        }
        acc.map(|r| self.merge(canvas, ws, pre, r))
    }

    /// The dab's alpha, textured, and where it lands: `(alpha, x, y, w, h)`.
    fn dab(&mut self, req: &DabRequest) -> Option<(Vec<u8>, i32, i32, usize, usize)> {
        let b = self.brush.clone();
        // calculateDabRect.
        let (hx, hy) = self.hot_spot(req.shape);
        let (x, sub_x) = split(req.x - hx);
        let (y, sub_y) = split(req.y - hy);
        let (w, h) = self.mask_size(req.shape, sub_x, sub_y);
        let params = DabParams {
            angle: req.shape.rotation,
            w,
            h,
            sub_x,
            sub_y,
            softness: 1.0,
            ratio: req.shape.ratio,
            index: 0,
        };
        let precision = b.precision.for_dab(w.min(h));
        let reuse = self.supports_caching()
            && self
                .cache
                .as_ref()
                .is_some_and(|c| params.matches(&c.params, precision));
        if !reuse {
            let (alpha, mw, mh) = self.tip_mask(req.shape, sub_x, sub_y, 1.0);
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
        Some((alpha, ox, oy, dw, dh))
    }

    /// Wash: alpha-darken one dab into the stroke buffer, in the order
    /// Krita's dab queue hands them over. Returns the buffer rect touched.
    fn darken(&mut self, req: DabRequest, ws: &mut StrokeWorkspace) -> Option<DirtyRect> {
        let (alpha, ox, oy, dw, dh) = self.dab(&req)?;
        // KisDabRenderingQueue::takeReadyDabs.
        self.average_opacity = composite::blend_average_opacity(req.opacity, self.average_opacity);
        let p = composite::Darken::new(req.opacity as f32, req.flow as f32, self.average_opacity as f32);
        let (buf, stride) = ws.wash();
        let (cw, ch) = (stride as i32, (buf.len() / stride.max(1) as usize) as i32);
        let (x0, y0) = (ox.max(0), oy.max(0));
        let (x1, y1) = ((ox + dw as i32).min(cw), (oy + dh as i32).min(ch));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        const TILE: i32 = 64;
        let mut dst = [0u8; TILE as usize];
        for y in y0..y1 {
            let src_row = &alpha[(y - oy) as usize * dw..(y - oy + 1) as usize * dw];
            let row = y as usize * stride as usize;
            // KisPainter::bltFixed composites a dab tile by tile, so each
            // run stops at a 64-px tile edge. Pixels off the canvas join
            // in as empty.
            let mut x = ox;
            while x < ox + dw as i32 {
                let end = ((x.div_euclid(TILE) + 1) * TILE).min(ox + dw as i32);
                let n = (end - x) as usize;
                for (i, d) in dst[..n].iter_mut().enumerate() {
                    let cx = x + i as i32;
                    *d = if (0..cw).contains(&cx) { buf[row + cx as usize] } else { 0 };
                }
                let src = &src_row[(x - ox) as usize..(end - ox) as usize];
                composite::darken_run(&mut dst[..n], src, x, &p);
                for (i, &d) in dst[..n].iter().enumerate() {
                    let cx = x + i as i32;
                    if (0..cw).contains(&cx) {
                        buf[row + cx as usize] = d;
                    }
                }
                x = end;
            }
        }
        let r = DirtyRect {
            min_x: x0 as u32,
            min_y: y0 as u32,
            max_x: x1 as u32,
            max_y: y1 as u32,
        };
        ws.note_wash(r);
        Some(r)
    }

    /// Wash: the stroke buffer merged onto the pre-stroke layer within `r`
    /// (`KisIndirectPaintingSupport::mergeToLayer`): the preset's blend at
    /// the opacity slider's strength, held to the selection and the alpha
    /// lock. A pure function of `pre` and the buffer, so redoing it every
    /// flush is exact.
    fn merge(&self, canvas: &mut Canvas, ws: &mut StrokeWorkspace, pre: &[u8], r: DirtyRect) -> DirtyRect {
        let clip = ws.clip().cloned();
        let alpha_lock = ws.alpha_lock();
        let opacity = self.opacity.strength as f32;
        let (buf, stride) = ws.wash();
        let w = canvas.width as usize;
        let px = canvas.pixels_mut();
        for y in r.min_y..r.max_y {
            for x in r.min_x..r.max_x {
                let a = buf[y as usize * stride as usize + x as usize];
                if a == 0 {
                    continue;
                }
                let idx = (y as usize * w + x as usize) * 4;
                let mut d = [pre[idx], pre[idx + 1], pre[idx + 2], pre[idx + 3]];
                let mask = clip.as_ref().map(|m| m.at(x, y));
                if self.erase {
                    composite::erase(&mut d, a, opacity, mask);
                } else if !(alpha_lock && d[3] == 0) {
                    composite::over(&mut d, self.color, a, opacity, mask, alpha_lock);
                }
                px[idx..idx + 4].copy_from_slice(&d);
            }
        }
        canvas.mark_dirty(r.min_x, r.min_y, r.max_x - r.min_x, r.max_y - r.min_y);
        r
    }

    /// Build-up: one dab straight onto the layer.
    fn render(&mut self, req: DabRequest, canvas: &mut Canvas, clip: Option<&Arc<Mask>>, alpha_lock: bool) -> Option<DirtyRect> {
        let (alpha, ox, oy, dw, dh) = self.dab(&req)?;
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
        let Tip::Png(tip) = &b.tip else { panic!("a bitmap tip") };
        assert!((tip.angle - 3.14159).abs() < 1e-12);
        assert_eq!(b.auto_spacing, Some(0.3));
        assert!(!b.isotropic_spacing);
        assert_eq!(*b.precision.for_dab(10), PRECISION_LEVELS[0]);

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
        let sides = |c: &[(f64, f64)]| {
            let d = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).hypot(p.1 - q.1);
            (d(c[0], c[1]), d(c[1], c[2]))
        };
        let upright = b.outline(40.0, 1.0, (0.0, 0.0), 0.0, (false, false));
        let flat = b.outline(40.0, 1.0, (60.0, 0.0), 0.0, (false, false));
        let (uw, uh) = sides(&upright);
        let (fw, fh) = sides(&flat);
        assert!(uh > 4.0 * uw, "a thin bar: {uw} x {uh}");
        assert!((fh - 40.0).abs() < 1.0, "full size flat: {fh}");
        assert!((fh / uh - 1.0 / 0.246231).abs() < 0.01, "{uh} -> {fh}");
        assert!((fw / fh - uw / uh).abs() < 1e-9);

        // Leaning along x lays the bar along x; along y, along y.
        let long_axis = |c: &[(f64, f64)]| {
            let e = (c[2].0 - c[1].0, c[2].1 - c[1].1);
            e.1.atan2(e.0).rem_euclid(std::f64::consts::PI)
        };
        let ax = long_axis(&flat);
        let ay = long_axis(&b.outline(40.0, 1.0, (0.0, 60.0), 0.0, (false, false)));
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

    /// A Krita 5 preset (`version` 5.0) carries its tip in `<resources>`,
    /// so it loads with nothing else supplied.
    #[test]
    fn a_krita5_preset_brings_its_own_tip() {
        fn b64(data: &[u8]) -> String {
            const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for c in data.chunks(3) {
                let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
                for i in 0..4 {
                    if i <= c.len() {
                        out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                    } else {
                        out.push('=');
                    }
                }
            }
            out
        }
        let xml = kpp::preset_xml(PENCIL5_KPP).unwrap();
        let res = format!(
            r#"<resources><resource type="brushes" md5sum="x" name="gradient" filename="gradient.png"><![CDATA[{}]]></resource></resources></Preset>"#,
            b64(GRADIENT_PNG)
        );
        let xml = xml.replacen("</Preset>", &res, 1);
        let mut kpp = kpp_with(&xml);
        assert!(PixelBrush::load(&kpp, &[]).is_err(), "no version chunk: resources ignored");
        // Splice a `version` text chunk in after the signature.
        let body = b"version\x005.0";
        let mut chunk = (body.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(body);
        chunk.extend_from_slice(&[0; 4]);
        kpp.splice(8..8, chunk);
        let b = PixelBrush::load(&kpp, &[]).expect("loads from its own resources");
        assert!((b.diameter() - 40.0).abs() < 1e-3);
    }

    #[test]
    fn unsupported_settings_are_refused_by_name() {
        let xml = kpp::preset_xml(PENCIL5_KPP).unwrap();
        let tips: &[(&str, &[u8])] = &[("gradient.png", GRADIENT_PNG)];
        assert!(PixelBrush::load(&kpp_with(&xml), tips).is_ok());

        let wash = xml.replace(r#"name="PaintOpAction">1<"#, r#"name="PaintOpAction">2<"#);
        assert!(PixelBrush::load(&kpp_with(&wash), tips).unwrap().wash);

        let multiply = xml.replace(
            r#"name="CompositeOp"><![CDATA[normal]]>"#,
            r#"name="CompositeOp"><![CDATA[multiply]]>"#,
        );
        assert!(multiply != xml, "the preset names its blend");
        let err = PixelBrush::load(&kpp_with(&multiply), tips).err().unwrap();
        assert!(err.to_string().contains("multiply"), "{err}");

        let scatter = xml.replace(r#"name="PressureScatter">false<"#, r#"name="PressureScatter">true<"#);
        let err = PixelBrush::load(&kpp_with(&scatter), tips).err().unwrap();
        assert!(err.to_string().contains("PressureScatter"), "{err}");

        let err = PixelBrush::load(&kpp_with(&xml), &[]).err().unwrap();
        assert!(err.to_string().contains("gradient.png"), "{err}");
    }

    // --- Wash mode, on the shipped presets ---

    use crate::doc::canvas::Canvas;
    use crate::input::pointer::PointerSample;
    use crate::tools::ribbon::StrokeWorkspace;
    use crate::tools::stroke::StrokeBuilder;
    use crate::tools::{ActiveTool, BrushMode, BrushSettings, SmoothingOptions};

    /// Draw `path` with a shipped preset at full pressure, the app's way.
    fn wash_stroke(
        c: &mut Canvas,
        ws: &mut StrokeWorkspace,
        preset: &str,
        tool: ActiveTool,
        opacity: f32,
        path: &[(f32, f32)],
        setup: impl FnOnce(&mut StrokeWorkspace),
    ) {
        let brush = defaults::brush(preset).unwrap();
        let settings = BrushSettings {
            radius: (brush.diameter() / 2.0) as f32,
            opacity,
            color: [200, 30, 40, 255],
            mode: BrushMode::Krita,
            ..BrushSettings::default()
        };
        let pre = c.pixels().into_owned();
        ws.begin(c.width, c.height, &settings);
        setup(ws);
        let mut b = StrokeBuilder::new_with_krita(settings, tool, 1.0, SmoothingOptions::default(), brush);
        b.seed_random(7);
        for &(x, y) in path {
            b.push(PointerSample::at(x, y, 1.0));
            b.flush(c, ws, &pre);
        }
        b.finish(c, ws, &pre);
    }

    fn line(x0: f32, x1: f32, y: f32) -> Vec<(f32, f32)> {
        (0..=60).map(|i| (x0 + (x1 - x0) * i as f32 / 60.0, y)).collect()
    }

    fn alpha_at(c: &Canvas, x: u32, y: u32) -> u8 {
        c.pixels()[((y * c.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn the_wash_presets_are_wash() {
        assert!(defaults::brush("b)_Basic-2_Opacity.kpp").unwrap().wash);
        assert!(!defaults::brush("c)_Pencil-5_Tilted.kpp").unwrap().wash);
        assert!(defaults::brush("a)_Eraser_Circle.kpp").unwrap().erase);
    }

    /// Going back over the same place within one wash stroke adds nothing:
    /// the buffer's alpha darken stops at the dab's opacity.
    #[test]
    fn a_wash_stroke_does_not_darken_where_it_crosses_itself() {
        let mut once = Canvas::new(200, 80);
        let mut ws = StrokeWorkspace::new();
        wash_stroke(&mut once, &mut ws, "b)_Basic-2_Opacity.kpp", ActiveTool::Pencil, 1.0, &line(20.0, 180.0, 40.0), |_| {});
        let mut there_and_back = Canvas::new(200, 80);
        let mut path = line(20.0, 180.0, 40.0);
        path.extend(line(180.0, 20.0, 40.0));
        wash_stroke(&mut there_and_back, &mut ws, "b)_Basic-2_Opacity.kpp", ActiveTool::Pencil, 1.0, &path, |_| {});
        let a = alpha_at(&once, 100, 40);
        let b = alpha_at(&there_and_back, 100, 40);
        assert!(a > 0, "painted");
        assert!((a as i32 - b as i32).abs() <= 1, "{a} vs {b}");
    }

    /// The opacity slider is the merge's opacity: a stroke at half opacity
    /// lands at about half the alpha.
    #[test]
    fn the_opacity_slider_sets_the_merge() {
        let mut ws = StrokeWorkspace::new();
        let mut full = Canvas::new(200, 80);
        wash_stroke(&mut full, &mut ws, "b)_Basic-5_Size_default.kpp", ActiveTool::Pencil, 1.0, &line(20.0, 180.0, 40.0), |_| {});
        let mut half = Canvas::new(200, 80);
        wash_stroke(&mut half, &mut ws, "b)_Basic-5_Size_default.kpp", ActiveTool::Pencil, 0.5, &line(20.0, 180.0, 40.0), |_| {});
        let (f, h) = (alpha_at(&full, 100, 40) as f32, alpha_at(&half, 100, 40) as f32);
        assert!(f > 200.0, "{f}");
        assert!((h - f * 0.5).abs() <= 2.0, "{f} → {h}");
    }

    /// The dabs build up unclipped; the selection holds the merge.
    #[test]
    fn a_wash_stroke_stays_in_the_selection() {
        let mut c = Canvas::new(200, 80);
        let mut ws = StrokeWorkspace::new();
        let clip = Arc::new(Mask {
            x: 0,
            y: 0,
            w: 100,
            h: 80,
            cov: vec![255; 100 * 80],
        });
        wash_stroke(&mut c, &mut ws, "b)_Basic-2_Opacity.kpp", ActiveTool::Pencil, 1.0, &line(20.0, 180.0, 40.0), |ws| {
            ws.set_clip(Some(clip.clone()))
        });
        assert!(alpha_at(&c, 60, 40) > 0);
        assert!((100..200).all(|x| (0..80).all(|y| alpha_at(&c, x, y) == 0)), "painted outside");
    }

    /// Alpha locked, a wash recolours paint and adds none.
    #[test]
    fn a_wash_stroke_honours_the_alpha_lock() {
        let mut c = Canvas::new(200, 80);
        for x in 0..100usize {
            for y in 0..80usize {
                c.pixels_mut()[(y * 200 + x) * 4..(y * 200 + x) * 4 + 4].copy_from_slice(&[0, 0, 255, 120]);
            }
        }
        let mut ws = StrokeWorkspace::new();
        wash_stroke(&mut c, &mut ws, "b)_Basic-2_Opacity.kpp", ActiveTool::Pencil, 1.0, &line(20.0, 180.0, 40.0), |ws| {
            ws.set_alpha_lock(true)
        });
        let px = |x: u32, y: u32| {
            let i = ((y * 200 + x) * 4) as usize;
            [c.pixels()[i], c.pixels()[i + 1], c.pixels()[i + 2], c.pixels()[i + 3]]
        };
        assert_eq!(px(60, 40)[3], 120, "alpha kept");
        assert!(px(60, 40)[0] > 150, "recoloured: {:?}", px(60, 40));
        assert_eq!(px(150, 40)[3], 0, "nothing added");
    }

    /// Krita's erasers carry an erase blend: they erase on any tool, in
    /// wash mode, softly at a soft edge.
    #[test]
    fn the_eraser_presets_erase() {
        for preset in ["a)_Eraser_Circle.kpp", "a)_Eraser_Soft.kpp"] {
            let mut c = Canvas::new(200, 80);
            for p in c.pixels_mut().chunks_mut(4) {
                p.copy_from_slice(&[10, 10, 10, 255]);
            }
            let mut ws = StrokeWorkspace::new();
            wash_stroke(&mut c, &mut ws, preset, ActiveTool::Pencil, 1.0, &line(20.0, 180.0, 40.0), |_| {});
            assert!(alpha_at(&c, 100, 40) < 30, "{preset}: {}", alpha_at(&c, 100, 40));
            assert_eq!(alpha_at(&c, 100, 2), 255, "{preset}: untouched away from the stroke");
        }
    }

    /// Two strokes through one workspace: the second starts with a clear
    /// buffer, so it is the same as on a fresh one.
    #[test]
    fn the_wash_buffer_is_cleared_between_strokes() {
        let mut ws = StrokeWorkspace::new();
        let mut first = Canvas::new(200, 80);
        wash_stroke(&mut first, &mut ws, "b)_Basic-2_Opacity.kpp", ActiveTool::Pencil, 1.0, &line(20.0, 180.0, 40.0), |_| {});
        let mut second = Canvas::new(200, 80);
        wash_stroke(&mut second, &mut ws, "b)_Basic-2_Opacity.kpp", ActiveTool::Pencil, 1.0, &line(20.0, 180.0, 60.0), |_| {});
        let mut fresh = Canvas::new(200, 80);
        wash_stroke(&mut fresh, &mut StrokeWorkspace::new(), "b)_Basic-2_Opacity.kpp", ActiveTool::Pencil, 1.0, &line(20.0, 180.0, 60.0), |_| {});
        assert_eq!(second.pixels(), fresh.pixels());
    }

    /// An auto tip's cursor is its ellipse at the dab's size.
    #[test]
    fn an_auto_tip_outline_is_its_ellipse() {
        let b = defaults::brush("b)_Basic-5_Size_default.kpp").unwrap();
        let pts = b.outline(30.0, 1.0, (0.0, 0.0), 0.0, (false, false));
        assert!(pts.len() > 8, "a round outline");
        for (x, y) in &pts {
            let r = x.hypot(*y);
            assert!((r - 15.0).abs() < 0.6, "radius {r}");
        }
    }
}
