//! Drawing tools — pencil, ink, eraser, fill.

pub mod dab;
pub mod fill;
pub mod lasso;
pub mod paper;
pub mod ribbon;
pub mod selection;
pub mod shape;
pub mod stroke;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTool {
    Pencil,
    Ink,
    Eraser,
    Fill,
    Shape,
    /// Stabilization tracker — places per-frame tracking points, draws nothing.
    Tracker,
    /// Freehand lasso; everything inside the closed path is erased from the
    /// active layer's cell on pointer-up.
    Lasso,
}

impl ActiveTool {
    pub fn idx(self) -> usize {
        match self {
            ActiveTool::Pencil => 0,
            ActiveTool::Ink => 1,
            ActiveTool::Eraser => 2,
            ActiveTool::Fill => 3,
            ActiveTool::Shape => 4,
            ActiveTool::Tracker => 5,
            ActiveTool::Lasso => 6,
        }
    }
}

/// Outline shape drawn by the Shape tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ShapeKind {
    Line,
    Rect,
    Ellipse,
}

/// Line smoothing, following Krita's `KisSmoothingOptions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Smoothing {
    /// Straight lines between raw samples.
    None,
    /// Krita's "Basic": no positional filtering at all, but consecutive
    /// samples are joined by a cubic Bezier fitted to the local tangents.
    /// This is Krita's own default, and most of what makes its lines clean —
    /// its straight lines come from input fidelity and curve fitting rather
    /// than from averaging the hand's tremor away.
    Basic,
    /// Krita's "Weighted": a gaussian average over the recent samples, keyed
    /// to distance travelled rather than to time (timings are too unstable to
    /// key a filter on), then the same Bezier fit.
    Weighted,
}

/// Smoothing parameters. Defaults are Krita's own, read from
/// `kis_config.cc`: Basic smoothing, a 50-pixel filter width, 0.15 tail
/// aggressiveness, pressure unsmoothed, distances scaled by zoom.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SmoothingOptions {
    pub kind: Smoothing,
    /// Filter width at low speed, in screen pixels. Krita's
    /// `LineSmoothingDistanceMax`.
    pub distance_max: f32,
    /// Filter width at high speed. Krita's `LineSmoothingDistanceMin`.
    /// Equal to `distance_max` by default, which makes speed irrelevant.
    pub distance_min: f32,
    /// How hard the filter resists the position drift caused by pressure
    /// rising at the start of a stroke. Krita's `LineSmoothingTailAggressiveness`.
    pub tail_aggressiveness: f32,
    /// Run pressure through the same filter as position.
    pub smooth_pressure: bool,
    /// Interpret the distances above in *screen* pixels rather than canvas
    /// pixels, so the filter feels identical at every zoom. This is what
    /// keeps a zoomed-out line as clean as a zoomed-in one.
    pub scalable_distance: bool,
}

impl Default for SmoothingOptions {
    fn default() -> Self {
        Self {
            kind: Smoothing::Basic,
            distance_max: 50.0,
            distance_min: 50.0,
            tail_aggressiveness: 0.15,
            smooth_pressure: false,
            scalable_distance: true,
        }
    }
}

/// How a stroke is turned into pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BrushMode {
    /// One variable-radius capsule per spine segment, coverage combined with
    /// `max()`. Uniform density however slowly the stroke is drawn, and no
    /// darkening where it crosses itself — what an ink line wants.
    Ribbon,
    /// Krita's pixel brush: a dab stamped every `spacing` of a diameter, each
    /// one accumulating into the coverage already there. Overlapping dabs
    /// darken, which is what makes graphite read as graphite rather than as
    /// flat fill.
    Dab,
}

/// Response of a brush property to pressure or tilt.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Dyn {
    /// How much of the property the input controls. 0 leaves it constant; 1
    /// lets the input take it all the way down to nothing.
    pub amount: f32,
    /// Curve shape. Below 1 the property rises early, like a blunt nib that
    /// reaches full width the moment it touches the paper. Above 1 it holds
    /// back until you lean on it, which is what gives a pencil its long
    /// taper.
    pub gamma: f32,
}

impl Dyn {
    pub const fn new(amount: f32, gamma: f32) -> Self {
        Self { amount, gamma }
    }

    /// Multiplier in `1 - amount ..= 1` for an input in 0..=1.
    #[inline]
    pub fn apply(&self, p: f32) -> f32 {
        let lo = (1.0 - self.amount).clamp(0.0, 1.0);
        lo + (1.0 - lo) * p.clamp(0.0, 1.0).powf(self.gamma.max(0.01))
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BrushSettings {
    /// Brush radius in pixels at pressure = 1.0.
    pub radius: f32,
    /// Whole-stroke opacity, 0..=1, applied once at composite time.
    pub opacity: f32,
    /// Alpha of a single dab at full pressure. Below 1 the stroke builds its
    /// density out of overlapping dabs instead of laying it down in one pass,
    /// which is where a pencil's mottled darkness comes from. Ignored in
    /// `Ribbon` mode, where coverage cannot accumulate.
    pub flow: f32,
    /// Edge hardness, 0..=1: fraction of the radius that is fully solid.
    /// 1.0 = crisp edge (~1px AA rim only), 0.0 = airbrush falloff.
    pub hardness: f32,
    /// Exponent on the edge falloff. 1.0 is a plain smoothstep; higher values
    /// pull the taper inwards so the dab fades from well inside its rim —
    /// the difference between a hard pencil and a soft one.
    pub softness: f32,
    /// Distance between dabs as a fraction of the dab's *diameter*, which is
    /// Krita's convention. `Dab` mode only.
    pub spacing: f32,
    /// Paper-grain strength, 0..=1. Canvas-position noise eats into the
    /// coverage — pencil tooth. 0 = smooth ink.
    pub grain: f32,
    /// Canvas pixels per grain texel. Larger values magnify the tooth.
    pub grain_scale: f32,
    /// Ink color (unmultiplied RGBA).
    pub color: [u8; 4],
    /// Pressure → radius.
    pub size: Dyn,
    /// Pressure → per-dab alpha.
    pub flow_dyn: Dyn,
    /// How far tilting the pen flattens the dab into an ellipse, 0..=1. This
    /// is what turns a pencil onto its side for shading.
    pub tilt_elongation: f32,
    /// How much tilting the pen widens the dab, 0..=1. The side of a lead
    /// covers more paper than its point does.
    pub tilt_size: f32,
    /// Rasterization model. See [`BrushMode`].
    pub mode: BrushMode,
    /// Flood-fill tolerance per channel (0..=255). Only used by Fill tool.
    pub fill_tolerance: u8,
    /// Grow the filled region by this many pixels after the flood, so colour
    /// tucks under the anti-aliased edge of lines living on another layer.
    /// Only used by the Fill tool.
    pub fill_expand: u8,
    /// Outline shape to draw. Only used by the Shape tool.
    pub shape_kind: ShapeKind,
}

impl Default for BrushSettings {
    /// A plain ribbon brush. Every preset below is a delta on this, and
    /// `#[serde(default)]` fills in from it when a preference blob written by
    /// an older build is missing a field.
    fn default() -> Self {
        Self {
            radius: 4.0,
            opacity: 1.0,
            flow: 1.0,
            hardness: 0.9,
            softness: 1.0,
            spacing: 0.1,
            grain: 0.0,
            grain_scale: 1.5,
            color: [20, 20, 20, 255],
            size: Dyn::new(0.7, 1.0),
            flow_dyn: Dyn::new(0.5, 1.0),
            tilt_elongation: 0.0,
            tilt_size: 0.0,
            mode: BrushMode::Ribbon,
            fill_tolerance: 16,
            fill_expand: 0,
            shape_kind: ShapeKind::Line,
        }
    }
}

impl BrushSettings {
    /// Everyday pencil, after Krita's `Pencil-4_Soft`: a soft round tip with
    /// visible tooth, tapering at both ends because size and flow both follow
    /// pressure on a curve that holds back until pressed.
    pub fn default_pencil() -> Self {
        Self {
            radius: 6.0,
            flow: 0.15,
            hardness: 0.3,
            softness: 1.4,
            spacing: 0.08,
            grain: 0.5,
            grain_scale: 1.1,
            size: Dyn::new(0.7, 1.3),
            flow_dyn: Dyn::new(0.7, 1.4),
            tilt_elongation: 0.3,
            tilt_size: 0.25,
            mode: BrushMode::Dab,
            ..Self::default()
        }
    }

    /// After Krita's `Pencil-3_Large_4B`: soft graphite laid down broad and
    /// dark, with the grain doing most of the work.
    pub fn pencil_4b() -> Self {
        Self {
            radius: 14.0,
            flow: 0.16,
            hardness: 0.2,
            softness: 1.1,
            spacing: 0.07,
            grain: 0.62,
            grain_scale: 1.3,
            size: Dyn::new(0.45, 1.0),
            flow_dyn: Dyn::new(0.85, 1.5),
            tilt_elongation: 0.35,
            tilt_size: 0.35,
            mode: BrushMode::Dab,
            ..Self::default()
        }
    }

    /// After Krita's `Pencil-5_Tilted`: the shading pencil. Leaning the pen
    /// flattens the dab into a broad ellipse across the direction of lean, so
    /// the side of the lead covers ground the point never could.
    pub fn pencil_tilted() -> Self {
        Self {
            radius: 18.0,
            flow: 0.12,
            hardness: 0.1,
            softness: 1.5,
            spacing: 0.06,
            grain: 0.55,
            grain_scale: 1.4,
            size: Dyn::new(0.4, 1.0),
            flow_dyn: Dyn::new(0.6, 1.2),
            tilt_elongation: 0.75,
            tilt_size: 0.6,
            mode: BrushMode::Dab,
            ..Self::default()
        }
    }

    /// After Krita's `Ink-3_G-Pen`: opaque, crisp, no tooth, with the sharp
    /// pressure taper a G-pen nib gives. A ribbon rather than dabs — an ink
    /// line must not darken where it crosses itself.
    pub fn default_ink() -> Self {
        Self {
            radius: 6.0,
            hardness: 0.95,
            color: [10, 10, 10, 255],
            size: Dyn::new(0.92, 1.6),
            flow_dyn: Dyn::new(0.15, 1.0),
            mode: BrushMode::Ribbon,
            ..Self::default()
        }
    }

    pub fn default_eraser() -> Self {
        Self {
            radius: 16.0,
            color: [0, 0, 0, 0],
            size: Dyn::new(0.5, 1.0),
            flow_dyn: Dyn::new(0.3, 1.0),
            ..Self::default()
        }
    }

    pub fn default_fill() -> Self {
        Self {
            radius: 1.0,
            hardness: 1.0,
            size: Dyn::new(0.0, 1.0),
            flow_dyn: Dyn::new(0.0, 1.0),
            fill_tolerance: 24,
            // Off by default: on a same-layer fill, growing the region eats
            // into the user's own strokes. It is opt-in for the line-art-on-
            // another-layer workflow, where the grown ring hides under the ink.
            fill_expand: 0,
            ..Self::default()
        }
    }

    /// Crisp outline shapes — full opacity, hard edge, no pressure dynamics.
    pub fn default_shape() -> Self {
        Self {
            radius: 3.0,
            hardness: 1.0,
            size: Dyn::new(0.0, 1.0),
            flow_dyn: Dyn::new(0.0, 1.0),
            ..Self::default()
        }
    }

    /// Presets offered in the brush panel, as (label, tooltip, builder).
    /// Starting points matched to the Krita brushes they are named after, not
    /// bit-exact copies of them.
    pub const PRESETS: &'static [(&'static str, &'static str, fn() -> Self)] = &[
        (
            "Pencil",
            "Soft round pencil with visible tooth. After Krita's Pencil-4_Soft.",
            BrushSettings::default_pencil,
        ),
        (
            "4B",
            "Broad soft graphite, dark and grainy. After Krita's Pencil-3_Large_4B.",
            BrushSettings::pencil_4b,
        ),
        (
            "Tilted",
            "Shading pencil: lean the pen and the dab flattens across the lean. \
             After Krita's Pencil-5_Tilted. Needs a tilt-capable tablet.",
            BrushSettings::pencil_tilted,
        ),
        (
            "G-Pen",
            "Opaque ink with a sharp pressure taper. After Krita's Ink-3_G-Pen.",
            BrushSettings::default_ink,
        ),
    ];
}
