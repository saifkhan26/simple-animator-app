# `src/tools/krita/sensors.rs` — sensors: turning pen input into dab size, rotation, opacity

> **Source:** [src/tools/krita/sensors.rs](../../../../src/tools/krita/sensors.rs) · **Module:** `crate::tools::krita::sensors` · **Size:** 560 lines · Line links as of commit `1fa5845`

## What this file is for

In Krita, each brush **option** (Size, Opacity, Flow, Rotation, Scatter, texture Strength…) can be driven by
one or more **sensors**: pressure, tilt, tilt direction, speed, drawing direction, barrel rotation, airbrush
wheel, random ("fuzzy"), or fade (how far into the stroke). Each sensor reads the pen state (`PaintInfo`),
maps it through its **curve** ([curve.rs](curve.md)), and the option combines its sensors (multiply, add,
max, min, difference) and its strength slider into one value for the dab.

**Worked example — Size driven by pressure.** Size option strength 1.0, one Pressure sensor whose curve is
`0,0; 0.5,0.2; 1,1`, brush size 40 px:

```text
pen pressure 0.5
  → Sensor::parameter: curve transfer table at 0.5 → 0.2
  → CurveOption::size_like: scaling = 0.2 (one sensor, multiply mode), × strength 1.0
  → dab diameter = 40 × 0.2 = 8 px
pen pressure 1.0 → curve → 1.0 → 40 px
```

Some sensors are **additive** (tilt direction, rotation, fuzzy): they add an angle rather than scale a value —
`rotation_like` handles those, in half-turns wrapped to −1…1. Tilt direction is measured *on screen*, so it is
corrected for the view's rotation and mirroring.

## How it connects

- **Used by:** [tools/krita/mod.rs](mod.md) (`KritaStroke::dab_at` builds a `PaintInfo` per dab and asks each
  option), [tools/krita/kpp.rs](kpp.md) (reading options into `CurveOption`).
- **Uses:** [tools/krita/curve.rs](curve.md), [tools/krita/random.rs](random.md).

## Rust concepts in this file

**Enum variants with data as sensor configs.** `SensorKind::Fade { length, periodic }` and
`DrawingAngle { offset }` carry their settings; `value(self, info)` matches on the variant.

**`Option<Vec<f64>>` as "skip the table".** An identity curve stores `transfer: None` and reports the raw value,
exactly like Krita.

## Types

### `struct PaintInfo`
[src/tools/krita/sensors.rs:21](../../../../src/tools/krita/sensors.rs#L21)

What a dab knows about the pen and the stroke so far, in Krita's own units: `KisPaintInformation` with its `DirectionHistoryInfo`.

| Field | Type | Meaning |
|---|---|---|
| `pressure` | `f64` | 0..=1, after the global pressure curve. |
| `x_tilt` | `f64` | Degrees, -60..=60 in practice. Qt reports these as whole degrees, truncated, and that is what Krita's sensors see. |
| `y_tilt` | `f64` | Tilt toward screen-down, whole degrees (see `x_tilt`). |
| `rotation` | `f64` | Barrel rotation, degrees. |
| `tangential_pressure` | `f64` | 0..=1. |
| `speed` | `f64` | 0..=1, already normalized by the speed smoother. |
| `drawing_angle` | `f64` | Direction of travel from the last dab, radians. |
| `dab_seq_no` | `i32` | Dabs painted so far in this stroke. |
| `canvas_rotation` | `f64` | The view's rotation, degrees, positive clockwise on screen. |
| `canvas_mirrored_h` | `bool` | The view is mirrored horizontally. |
| `canvas_mirrored_v` | `bool` | The view is mirrored vertically. |
| `hovering` | `bool` | The pen is hovering, not painting: the brush outline asks this way. Fuzzy sensors stay still and fade reads as finished. |
| `random` | `Option<Rc<RefCell<RandomSource>>>` | The stroke's random source. `None` reads like hovering to the sensors that need one. |

### `enum SensorKind`
[src/tools/krita/sensors.rs:95](../../../../src/tools/krita/sensors.rs#L95)

The sensors this port knows, with Krita's IDs as they appear in presets (`KisDynamicSensors.h`, `KisDynamicSensorDrawingAngle`, `…Fuzzy`, `…Fade`).

| Variant | Meaning |
|---|---|
| `Pressure` | `pressure` |
| `TangentialPressure` | `tangentialpressure`: an airbrush wheel. |
| `DrawingAngle { offset: i32 }` | `drawingangle`: the direction of travel, an absolute rotation. `offset` is whole degrees. |
| `XTilt` | `xtilt` / `ytilt`: 1 upright, 0 leaning 60° along that axis. |
| `YTilt` | `ytilt`: like XTilt along the other axis. |
| `TiltDirection` | `ascension`: tilt direction. Additive. |
| `TiltElevation` | `declination`: tilt elevation. |
| `Rotation` | `rotation`: barrel rotation. Additive. |
| `FuzzyPerDab` | `fuzzy`: a fresh random value every dab. Additive. |
| `Speed` | `speed`: how fast the pen moves (smoothed). |
| `Fade { length: i32, periodic: bool }` | `fade`: how far through its first `length` dabs the stroke is. |

### `struct Sensor`
[src/tools/krita/sensors.rs:186](../../../../src/tools/krita/sensors.rs#L186)

One active sensor on an option, with its curve baked to Krita's transfer table. `None` for an identity curve, which Krita skips.

| Field | Type | Meaning |
|---|---|---|
| `kind` | `SensorKind` | Which sensor. |
| `transfer` | `Option<Vec<f64>>` | The curve's transfer table, or `None` for an identity curve. |

### `struct Components`
[src/tools/krita/sensors.rs:225](../../../../src/tools/krita/sensors.rs#L225)

`KisCurveOption::ValueComponents`.

| Field | Type | Meaning |
|---|---|---|
| `constant` | `f64` | The option's constant part. |
| `scaling` | `f64` | The combined scaling sensors' value. |
| `additive` | `f64` | The combined additive sensors' value. |
| `absolute_offset` | `f64` | An absolute rotation offset (drawing angle). |
| `has_scaling` | `bool` | Whether any scaling sensor is active. |
| `has_additive` | `bool` | Whether any additive sensor is active. |
| `has_absolute_offset` | `bool` | Whether an absolute-rotation sensor is active. |
| `min` | `f64` | The option's minimum. |
| `max` | `f64` | The option's maximum. |

### `struct CurveOption`
[src/tools/krita/sensors.rs:240](../../../../src/tools/krita/sensors.rs#L240)

A curve option (Size, Rotation, Opacity, texture Strength …) as the preset configures it.

| Field | Type | Meaning |
|---|---|---|
| `checked` | `bool` | `isChecked`. An unchecked option does not apply at all. |
| `use_curve` | `bool` | Whether sensors apply (otherwise the option is constant). |
| `curve_mode` | `i32` | How several scaling sensors combine: 0 multiply, 1 add, 2 max, 3 min, 4 difference. |
| `strength` | `f64` | The option's strength slider, `<Id>Value`. |
| `min` | `f64` | Lower bound of the option's value. |
| `max` | `f64` | Upper bound. |
| `sensors` | `Vec<Sensor>` | The active sensors. |
| `fan_corners` | `Option<f64>` | With a drawing-angle sensor: paint a fan of dabs, this many degrees apart, round a sharp corner (`KisPaintOpUtils::paintFan`). Only the Rotation option's counts. |

## Free functions

### `tilt_direction`
[src/tools/krita/sensors.rs:52](../../../../src/tools/krita/sensors.rs#L52)

```rust
pub fn tilt_direction(info: &PaintInfo, normalize: bool) -> f64
```

`KisPaintInformation::tiltDirection`: which way the pen leans, as an angle on screen. Normalized, 0..=1 going once round.

### `tilt_elevation`
[src/tools/krita/sensors.rs:64](../../../../src/tools/krita/sensors.rs#L64)

```rust
pub fn tilt_elevation(info: &PaintInfo, max_tilt_x: f64, max_tilt_y: f64, normalize: bool) -> f64
```

`KisPaintInformation::tiltElevation`: how upright the pen stands. Normalized, 1 for a pen perpendicular to the tablet, 0 for one lying at the maximum tilt or flatter.

### `scaling_to_additive`
[src/tools/krita/sensors.rs:82](../../../../src/tools/krita/sensors.rs#L82)

```rust
pub fn scaling_to_additive(x: f64) -> f64
```

Krita's conversion of a 0–1 scaling sensor value to an additive one (`−1…1`).

### `additive_to_scaling`
[src/tools/krita/sensors.rs:87](../../../../src/tools/krita/sensors.rs#L87)

```rust
pub fn additive_to_scaling(x: f64) -> f64
```

The reverse conversion.

## `impl SensorKind`

### `SensorKind::from_id`
[src/tools/krita/sensors.rs:122](../../../../src/tools/krita/sensors.rs#L122)

```rust
pub fn from_id(id: &str) -> Option<Self>
```

The sensors that need nothing beyond their ID.

### `SensorKind::is_additive`
[src/tools/krita/sensors.rs:137](../../../../src/tools/krita/sensors.rs#L137)

```rust
fn is_additive(self) -> bool
```

Whether the sensor adds to the value (an angle) instead of scaling it.

### `SensorKind::is_absolute_rotation`
[src/tools/krita/sensors.rs:141](../../../../src/tools/krita/sensors.rs#L141)

```rust
fn is_absolute_rotation(self) -> bool
```

Whether the sensor gives an absolute direction (drawing angle), which mirroring flips.

### `SensorKind::value`
[src/tools/krita/sensors.rs:145](../../../../src/tools/krita/sensors.rs#L145)

```rust
fn value(self, info: &PaintInfo) -> f64
```

The sensor's raw 0–1 value for this dab: pressure, normalised tilt, tilt direction/elevation, rotation, speed,
a random number (fuzzy), fade progress, or drawing angle.

## `impl Sensor`

### `Sensor::new`
[src/tools/krita/sensors.rs:192](../../../../src/tools/krita/sensors.rs#L192)

```rust
pub fn new(kind: SensorKind, curve: &CubicCurve) -> Self
```

A sensor of `kind` with `curve` baked into a transfer table (unless it's the identity).

### `Sensor::parameter`
[src/tools/krita/sensors.rs:198](../../../../src/tools/krita/sensors.rs#L198)

```rust
pub fn parameter(&self, info: &PaintInfo) -> f64
```

`KisDynamicSensor::parameter`.

## `impl CurveOption`

### `CurveOption::components`
[src/tools/krita/sensors.rs:259](../../../../src/tools/krita/sensors.rs#L259)

```rust
fn components(&self, info: &PaintInfo, use_strength: bool) -> Components
```

Evaluates every active sensor and combines them (`curve_mode` for scaling sensors, summing additive ones),
optionally scaled by the strength — Krita's `computeValueComponents`.

### `CurveOption::size_like`
[src/tools/krita/sensors.rs:309](../../../../src/tools/krita/sensors.rs#L309)

```rust
pub fn size_like(&self, info: &PaintInfo, use_strength: bool) -> f64
```

`computeSizeLikeValue`.

### `CurveOption::rotation_like`
[src/tools/krita/sensors.rs:324](../../../../src/tools/krita/sensors.rs#L324)

```rust
pub fn rotation_like(
    &self,
    info: &PaintInfo,
    base: f64,
    absolute_axes_flipped: bool,
    scaling_coeff: f64,
    disable_scaling: bool,
) -> f64
```

`computeRotationLikeValue`, in half-turns, wrapped to -1..1. `absolute_axes_flipped`: the view is mirrored on exactly one axis, which turns an absolute angle (the drawing direction) the other way.

### `CurveOption::apply`
[src/tools/krita/sensors.rs:355](../../../../src/tools/krita/sensors.rs#L355)

```rust
pub fn apply(&self, info: &PaintInfo) -> f64
```

`KisStandardOption::apply`: the option's value, or 1 when it is off.

## Free functions

### `wrap`
[src/tools/krita/sensors.rs:364](../../../../src/tools/krita/sensors.rs#L364)

```rust
pub fn wrap(value: f64, min: f64, max: f64) -> f64
```

`KisAlgebra2D::wrapValue(value, min, max)`.

### `normalize_angle`
[src/tools/krita/sensors.rs:374](../../../../src/tools/krita/sensors.rs#L374)

```rust
pub fn normalize_angle(a: f64) -> f64
```

`normalizeAngle`: into `[0, 2π)`.

### `rotation`
[src/tools/krita/sensors.rs:387](../../../../src/tools/krita/sensors.rs#L387)

```rust
pub fn rotation(option: &CurveOption, info: &PaintInfo) -> f64
```

`KisRotationOption::apply`: the dab's rotation, radians.

## Tests

### `mod tests`

Run with `cargo test tools::krita::sensors::tests` · 8 tests

| Test | What it checks |
|---|---|
| [`elevation_is_one_upright_and_zero_at_full_tilt`](../../../../src/tools/krita/sensors.rs#L411) | Elevation is one upright and zero at full tilt. |
| [`direction_goes_once_round`](../../../../src/tools/krita/sensors.rs#L420) | Direction goes once round. |
| [`rotation_follows_tilt_direction`](../../../../src/tools/krita/sensors.rs#L443) | The dab turns with the lean: a quarter turn of the pen is a quarter turn of the dab. |
| [`rotation_compensates_canvas_rotation`](../../../../src/tools/krita/sensors.rs#L455) | Rotating the canvas moves the dab's angle by the same amount. The tip is drawn at minus that angle, so on screen the dab keeps its angle to the pen. |
| [`size_like_multiplies_strength_curve_and_clamps`](../../../../src/tools/krita/sensors.rs#L466) | Size like multiplies strength curve and clamps. |
| [`rotation_follows_the_drawing_direction`](../../../../src/tools/krita/sensors.rs#L500) | The drawing angle is an absolute rotation: heading right turns the dab to match, and on a view mirrored along one axis the angle turns the other way. |
| [`fuzzy_draws_from_the_stroke_and_stays_still_while_hovering`](../../../../src/tools/krita/sensors.rs#L522) | Fuzzy draws from the stroke and stays still while hovering. |
| [`fade_runs_over_its_length_then_holds_or_repeats`](../../../../src/tools/krita/sensors.rs#L544) | Fade runs over its length then holds or repeats. |

**Test helpers** (used only by the tests above):

- [`tilt`](../../../../src/tools/krita/sensors.rs#L401) — Tilt.
- [`rotation_option`](../../../../src/tools/krita/sensors.rs#L427) — Rotation option.
- [`option`](../../../../src/tools/krita/sensors.rs#L483) — Option.
