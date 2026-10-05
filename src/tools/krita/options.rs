//! The curve options a pixel brush reads, as `KisStandardOptionData.h`,
//! `KisSizeOptionData.cpp` and `KisScatterOptionData.cpp` declare them:
//! each option's id, whether it can be switched off, and the range of its
//! strength slider. A preset that leaves the strength out gets the top of
//! the range, so the range matters as much as the curves.

/// One curve option's declaration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OptionSpec {
    /// The key prefix in the preset: `<id>Sensor`, `<id>Value`, …
    pub id: &'static str,
    /// Checkable options are off unless `Pressure<id>` says otherwise;
    /// the others always apply.
    pub checkable: bool,
    pub min: f64,
    pub max: f64,
}

const fn spec(id: &'static str, checkable: bool, min: f64, max: f64) -> OptionSpec {
    OptionSpec { id, checkable, min, max }
}

pub const SIZE: OptionSpec = spec("Size", true, 0.0, 1.0);
pub const RATIO: OptionSpec = spec("Ratio", true, 0.0, 1.0);
pub const ROTATION: OptionSpec = spec("Rotation", true, 0.0, 1.0);
pub const OPACITY: OptionSpec = spec("Opacity", false, 0.0, 1.0);
pub const FLOW: OptionSpec = spec("Flow", false, 0.0, 1.0);
pub const SOFTNESS: OptionSpec = spec("Softness", true, 0.1, 1.0);
/// Scatter's strength is a multiple of the dab's diameter, up to five.
pub const SCATTER: OptionSpec = spec("Scatter", true, 0.0, 5.0);
pub const TEXTURE_STRENGTH: OptionSpec = spec("Texture/Strength/", true, 0.0, 1.0);
