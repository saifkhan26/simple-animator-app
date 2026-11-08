# `src/tools/krita/tip/auto.rs` — generated ("auto") brush tips

> **Source:** [src/tools/krita/tip/auto.rs](../../../../../src/tools/krita/tip/auto.rs) · **Module:** `crate::tools::krita::tip::auto` · **Size:** 1007 lines · Line links as of commit `1fa5845`

## What this file is for

Krita's **auto brush** tip isn't an image: it is *generated* by a formula for every dab — a circle or rectangle
with a chosen edge falloff (**default**: hard core + linear fade; **gauss**: a soft gaussian; **soft**: a falloff
curve the user draws), plus ratio, horizontal/vertical fade, spikes (star shapes), angle, randomness (holes)
and density. This file ports `KisAutoBrush` and its mask generators so generated tips match Krita.

For each mask pixel it computes the distance from the dab centre in the tip's rotated, scaled frame, then the
profile's value (0 = solid … 255 = empty, as Krita stores it), with an anti-aliasing fade at the rim
(`Fade1D`). Like Krita, there's a scalar path and a vectorised path processing 8 floats at once
(`Generator::row`), each reproduced because they round differently.

```text
default profile, hfade 0.5        gauss profile
  ████████▓▒░                       ▓▓▓▒▒▒░░░
  distance from centre →            smooth bell curve
```

## How it connects

- **Used by:** [tools/krita/mod.rs](../mod.md) (`Tip::Auto`, `KritaStroke::auto_tip`).
- **Uses:** [tools/krita/tip/simd_math.rs](simd_math.md) (Krita's exact `exp`, `erf`, `atan2`),
  [tools/krita/curve.rs](../curve.md) (soft profile), [tools/krita/random.rs](../random.md) (randomness/density),
  [tools/krita/tip/mod.rs](mod.md) (`DabShape`), [tools/krita/qt.rs](../qt.md) (`RectF`).

## Rust concepts in this file

**`RefCell` for a cache inside an immutable object.** `curve: RefCell<CurveState>` lets `curve_for_softness(&self)`
rebuild its cached table, as Krita's mutable members do. More: [Smart pointers](../../../../rust-primer.md#smart-pointers)

**Enum variants holding per-dab constants.** `enum Scaled { Circle {..}, Rect {..}, Gauss {..}, Curve {..} }` — each
generator type's precomputed values, matched in `value_at`.

## Types

### `const OVERSAMPLING`
[src/tools/krita/tip/auto.rs:34](../../../../../src/tools/krita/tip/auto.rs#L34)

```rust
const OVERSAMPLING: f64 = 4.0;
```

`OVERSAMPLING` in kis_base_mask_generator.h: the soft tip's curve is tabulated at four entries per pixel of its diameter.

### `const LANES`
[src/tools/krita/tip/auto.rs:36](../../../../../src/tools/krita/tip/auto.rs#L36)

```rust
const LANES: usize = 8;
```

Floats per vector batch (8, as Krita's AVX2 path).

### `enum Shape`
[src/tools/krita/tip/auto.rs:40](../../../../../src/tools/krita/tip/auto.rs#L40)

`KisMaskGenerator::Type`.

| Variant | Meaning |
|---|---|
| `Circle` | A round (or elliptical) tip. |
| `Rect` | A rectangular tip. |

### `enum Profile`
[src/tools/krita/tip/auto.rs:47](../../../../../src/tools/krita/tip/auto.rs#L47)

The generator's id: how the edge falls off.

| Variant | Meaning |
|---|---|
| `Default` | `default`: a hard core and a linear fade. |
| `Gauss` | `gauss`. |
| `Soft(CubicCurve)` | `soft`: a falloff curve the user draws. |

### `struct AutoDef`
[src/tools/krita/tip/auto.rs:58](../../../../../src/tools/krita/tip/auto.rs#L58)

An auto tip as the preset's `brush_definition` describes it.

| Field | Type | Meaning |
|---|---|---|
| `shape` | `Shape` | Circle or rectangle. |
| `profile` | `Profile` | Edge falloff type. |
| `diameter` | `f64` | The preset's own size, pixels. |
| `ratio` | `f64` | Height-to-width ratio. |
| `hfade` | `f64` | Horizontal fade (softness of the edge). |
| `vfade` | `f64` | Vertical fade. |
| `spikes` | `i32` | Number of spikes (2 = plain shape). |
| `antialias` | `bool` | Anti-alias the edge. |
| `angle` | `f64` | `KisBrush::angle`, radians. |
| `randomness` | `f64` | Random holes in the mask. |
| `density` | `f64` | Fraction of pixels painted. |

### `struct Fade1D`
[src/tools/krita/tip/auto.rs:139](../../../../../src/tools/krita/tip/auto.rs#L139)

`KisAntialiasingFadeMaker1D`, set up for one dab.

| Field | Type | Meaning |
|---|---|---|
| `radius` | `f64` | Radius the fade applies at. |
| `start` | `f64` | Where the fade starts. |
| `start_value` | `u8` | Mask value at the fade start. |
| `coeff` | `f64` | Fade slope. |
| `enabled` | `bool` | Whether anti-aliasing is on. |

### `struct CurveState`
[src/tools/krita/tip/auto.rs:226](../../../../../src/tools/krita/tip/auto.rs#L226)

The soft tip's tabulated curve. Krita rebuilds it whenever the dab's softness is not 1, and keeps whatever it last built otherwise; a stroke starts with a fresh copy of the generator, which always rebuilds once.

| Field | Type | Meaning |
|---|---|---|
| `data` | `Vec<f64>` | The tabulated curve. |
| `dirty` | `bool` | Whether it must be rebuilt. |

### `struct Generator`
[src/tools/krita/tip/auto.rs:233](../../../../../src/tools/krita/tip/auto.rs#L233)

One stroke's mask generator at the stroke's brush size, with precomputed constants.

| Field | Type | Meaning |
|---|---|---|
| `def` | `AutoDef` | The tip definition. |
| `diameter` | `f64` | Diameter at this stroke's size. |
| `cs` | `f64` | Cosine of the spike angle. |
| `ss` | `f64` | Sine of the spike angle. |
| `cached_spikes_angle` | `f64` | Angle per spike. |
| `empty` | `bool` | The tip paints nothing (zero size). |
| `gauss_fade` | `f64` | Gaussian profile fade parameter. |
| `gauss_center` | `f64` | Gaussian centre offset. |
| `gauss_alphafactor` | `f64` | Gaussian normalisation. |
| `curve_resolution` | `i32` | Soft-curve table resolution. |
| `curve_points` | `Vec<(f64, f64)>` | The soft profile's curve points. |
| `curve` | `RefCell<CurveState>` | Cached soft-curve table. |

### `enum Scaled`
[src/tools/krita/tip/auto.rs:253](../../../../../src/tools/krita/tip/auto.rs#L253)

The per-dab constants a generator's `setScale` / `setSoftness` leave behind.

| Variant | Meaning |
|---|---|
| `Circle { xcoef: f64, ycoef: f64, tfx: f64, tfy: f64 }` | Constants for a circle with the default profile. |
| `Rect { xcoeff: f64, ycoeff: f64, tfx: f64, tfy: f64 }` | Constants for a rectangle. |
| `Gauss { ycoef: f64, distfactor: f64, fade: Fade1D }` | Constants for the gaussian profile (with its edge fade). |
| `Curve { xcoef: f64, ycoef: f64, data: Vec<f64>, fade: Fade1D }` | Constants and the curve table for the soft profile. |

### `struct AutoTip`
[src/tools/krita/tip/auto.rs:613](../../../../../src/tools/krita/tip/auto.rs#L613)

One stroke's auto tip: the generator at the stroke's size, the brush's pixel size after `KisAutoBrush`'s two-stage set-up, and the random source its mask applicator draws density and randomness from.

| Field | Type | Meaning |
|---|---|---|
| `gen` | `Generator` | The generator. |
| `width` | `i32` | Tip pixel width. |
| `height` | `i32` | Tip pixel height. |
| `angle` | `f64` | Tip angle. |
| `random` | `RefCell<RandomSource>` | Random source for randomness/density holes. |

## Free functions

### `to_double`
[src/tools/krita/tip/auto.rs:76](../../../../../src/tools/krita/tip/auto.rs#L76)

```rust
fn to_double(s: &str) -> f64
```

`KisDomUtils::toDouble`: C locale, then a German one for presets written with a decimal comma, else zero.

### `to_int`
[src/tools/krita/tip/auto.rs:84](../../../../../src/tools/krita/tip/auto.rs#L84)

```rust
fn to_int(s: &str) -> i32
```

`QString::toInt`: anything that is not an integer is 0.

## `impl AutoDef`

### `AutoDef::parse`
[src/tools/krita/tip/auto.rs:90](../../../../../src/tools/krita/tip/auto.rs#L90)

```rust
pub fn parse(brush: roxmltree::Node) -> Result<Self>
```

`KisAutoBrushFactory::createBrushModel`, from the `<Brush>` element.

## `impl Fade1D`

### `Fade1D::radius`
[src/tools/krita/tip/auto.rs:149](../../../../../src/tools/krita/tip/auto.rs#L149)

```rust
fn radius(radius: f64, enabled: bool, value: impl Fn(f64) -> u8) -> Self
```

`setRadius`.

### `Fade1D::square_norm`
[src/tools/krita/tip/auto.rs:162](../../../../../src/tools/krita/tip/auto.rs#L162)

```rust
fn square_norm(xcoeff: f64, ycoeff: f64, enabled: bool, value: impl Fn(f64) -> u8) -> Self
```

`setSquareNormCoeffs`.

### `Fade1D::need_fade`
[src/tools/krita/tip/auto.rs:179](../../../../../src/tools/krita/tip/auto.rs#L179)

```rust
fn need_fade(&self, dist: f64) -> Option<u8>
```

`needFade`, scalar.

### `Fade1D::need_fade_lane`
[src/tools/krita/tip/auto.rs:194](../../../../../src/tools/krita/tip/auto.rs#L194)

```rust
fn need_fade_lane(&self, dist: &mut f32) -> bool
```

`needFade`, one vector lane: rewrites `dist` and says whether the lane is excluded from the generator's own formula.

## Free functions

### `sat_u8`
[src/tools/krita/tip/auto.rs:212](../../../../../src/tools/krita/tip/auto.rs#L212)

```rust
fn sat_u8(v: f64) -> u8
```

A C++ conversion of a non-negative double to `quint8`: truncation.

## `impl Generator`

### `Generator::new`
[src/tools/krita/tip/auto.rs:261](../../../../../src/tools/krita/tip/auto.rs#L261)

```rust
fn new(def: &AutoDef, diameter: f64) -> Self
```

Sets up the generator for `diameter`: spike constants, gaussian parameters, soft-curve resolution.

### `Generator::width`
[src/tools/krita/tip/auto.rs:300](../../../../../src/tools/krita/tip/auto.rs#L300)

```rust
fn width(&self) -> f64
```

`KisMaskGenerator::width` / `height`.

### `Generator::height`
[src/tools/krita/tip/auto.rs:304](../../../../../src/tools/krita/tip/auto.rs#L304)

```rust
fn height(&self) -> f64
```

The generator's height (`width × ratio`).

### `Generator::curve_for_softness`
[src/tools/krita/tip/auto.rs:313](../../../../../src/tools/krita/tip/auto.rs#L313)

```rust
fn curve_for_softness(&self, softness: f64) -> Vec<f64>
```

`KisCurveCircleMaskGenerator::setSoftness` with its caching.

### `Generator::scaled`
[src/tools/krita/tip/auto.rs:336](../../../../../src/tools/krita/tip/auto.rs#L336)

```rust
fn scaled(&self, scale_x: f64, scale_y: f64, softness: f64) -> Scaled
```

`setSoftness` then `setScale`, as `generateMaskAndApplyMaskOrCreateDab` calls them.

### `Generator::gauss_value`
[src/tools/krita/tip/auto.rs:381](../../../../../src/tools/krita/tip/auto.rs#L381)

```rust
fn gauss_value(&self, dist: f64, distfactor: f64) -> u8
```

`KisGaussCircleMaskGenerator::Private::value`.

### `Generator::fix_rotation`
[src/tools/krita/tip/auto.rs:389](../../../../../src/tools/krita/tip/auto.rs#L389)

```rust
fn fix_rotation(&self, xr: &mut f64, yr: &mut f64)
```

`KisMaskGenerator::fixRotation`: fold a spiky tip's angle back into its first spike.

### `Generator::value_at`
[src/tools/krita/tip/auto.rs:402](../../../../../src/tools/krita/tip/auto.rs#L402)

```rust
fn value_at(&self, s: &Scaled, x: f64, y: f64) -> u8
```

`valueAt`: 0 inside, 255 outside.

### `Generator::row`
[src/tools/krita/tip/auto.rs:478](../../../../../src/tools/krita/tip/auto.rs#L478)

```rust
fn row(&self, s: &Scaled, buf: &mut [f32], y: f32, cosa: f32, sina: f32, cx: f32, cy: f32)
```

`FastRowProcessor::process`: one row of the vector path's buffer, 1 where the dab is empty, 0 where it is solid.

## Free functions

### `sq`
[src/tools/krita/tip/auto.rs:587](../../../../../src/tools/krita/tip/auto.rs#L587)

```rust
fn sq(v: f32) -> f32
```

Square of a float.

### `qround`
[src/tools/krita/tip/auto.rs:592](../../../../../src/tools/krita/tip/auto.rs#L592)

```rust
fn qround(v: f64) -> i32
```

`qRound` for non-negative values.

### `curve_value`
[src/tools/krita/tip/auto.rs:601](../../../../../src/tools/krita/tip/auto.rs#L601)

```rust
fn curve_value(dist: f64, resolution: f64, data: &[f64]) -> u8
```

`KisCurveCircleMaskGenerator::Private::value`.

## `impl AutoTip`

### `AutoTip::new`
[src/tools/krita/tip/auto.rs:624](../../../../../src/tools/krita/tip/auto.rs#L624)

```rust
pub fn new(def: &AutoDef, diameter: f64, random: RandomSource) -> Self
```

A tip `diameter` pixels across: Krita's size slider rewrites the generator's diameter rather than scaling the brush.

### `AutoTip::seed`
[src/tools/krita/tip/auto.rs:652](../../../../../src/tools/krita/tip/auto.rs#L652)

```rust
pub fn seed(&mut self, seed: u32)
```

Fix the mask applicator's random source.

### `AutoTip::size`
[src/tools/krita/tip/auto.rs:657](../../../../../src/tools/krita/tip/auto.rs#L657)

```rust
pub fn size(&self) -> (i32, i32)
```

`KisBrush::width` / `height`.

### `AutoTip::lie`
[src/tools/krita/tip/auto.rs:662](../../../../../src/tools/krita/tip/auto.rs#L662)

```rust
fn lie(&self, shape: DabShape) -> DabShape
```

`lieAboutDabShape`: a spiky tip is measured as if it were round.

### `AutoTip::normalized`
[src/tools/krita/tip/auto.rs:670](../../../../../src/tools/krita/tip/auto.rs#L670)

```rust
fn normalized(&self, shape: DabShape) -> DabShape
```

A dab shape adjusted for the tip's own ratio and scale.

### `AutoTip::rect`
[src/tools/krita/tip/auto.rs:679](../../../../../src/tools/krita/tip/auto.rs#L679)

```rust
fn rect(&self) -> RectF
```

The tip's rectangle.

### `AutoTip::characteristic_size`
[src/tools/krita/tip/auto.rs:689](../../../../../src/tools/krita/tip/auto.rs#L689)

```rust
pub fn characteristic_size(&self, shape: DabShape) -> (f64, f64)
```

`KisAutoBrush::characteristicSize`.

### `AutoTip::hot_spot`
[src/tools/krita/tip/auto.rs:696](../../../../../src/tools/krita/tip/auto.rs#L696)

```rust
pub fn hot_spot(&self, shape: DabShape) -> (f64, f64)
```

`KisBrush::hotSpot`.

### `AutoTip::mask_size`
[src/tools/krita/tip/auto.rs:702](../../../../../src/tools/krita/tip/auto.rs#L702)

```rust
pub fn mask_size(&self, shape: DabShape, sub_x: f64, sub_y: f64) -> (i32, i32)
```

`KisAutoBrush::maskWidth` / `maskHeight`.

### `AutoTip::supports_caching`
[src/tools/krita/tip/auto.rs:709](../../../../../src/tools/krita/tip/auto.rs#L709)

```rust
pub fn supports_caching(&self) -> bool
```

`KisAutoBrush::supportsCaching`: a mask with random holes in it can never be reused.

### `AutoTip::mask`
[src/tools/krita/tip/auto.rs:718](../../../../../src/tools/krita/tip/auto.rs#L718)

```rust
pub fn mask(&self, shape: DabShape, sub_x: f64, sub_y: f64, softness: f64) -> (Vec<u8>, usize, usize)
```

Generates one dab's mask: picks the scalar or vector path as Krita would, evaluates every pixel (rotation,
scale, sub-pixel offset, profile, fade), applies randomness and density from the random source, and returns the
alpha bytes with their size.

### `AutoTip::outline`
[src/tools/krita/tip/auto.rs:807](../../../../../src/tools/krita/tip/auto.rs#L807)

```rust
pub fn outline(&self, shape: DabShape) -> Vec<(f64, f64)>
```

`KisAutoBrush::outline` as the cursor shows it: the tip's ellipse or box, sized and turned like the dab, round the dab's centre.

## Tests

### `mod tests`

Run with `cargo test tools::krita::tip::auto::tests` · 8 tests

| Test | What it checks |
|---|---|
| [`gauss_constants_follow_krita`](../../../../../src/tools/krita/tip/auto.rs#L853) | The gaussian's constants, worked from KisGaussCircleMaskGenerator: with full fades `fade` would be 0, which Krita nudges to 1e-6. |
| [`a_rect_with_ratio_measures_like_krita`](../../../../../src/tools/krita/tip/auto.rs#L864) | A rect with ratio measures like krita. |
| [`a_hard_round_dab_is_a_disc`](../../../../../src/tools/krita/tip/auto.rs#L877) | A hard round tip (Krita's fade 1; fade 0 is the softest) is solid at its centre, empty past its rim, and symmetric. |
| [`the_scalar_and_vector_paths_agree_within_two_levels`](../../../../../src/tools/krita/tip/auto.rs#L898) | Both paths compute the same tip; the vector one in single precision with its own erf and rounding. Krita's own test allows them 2 levels apart (kis_mask_similarity_test.cpp). |
| [`density_punches_holes_and_randomness_dims`](../../../../../src/tools/krita/tip/auto.rs#L931) | Density punches holes and randomness dims. |
| [`a_spiky_tip_is_drawn_with_spikes`](../../../../../src/tools/krita/tip/auto.rs#L948) | Ink-1 Precision's nine spikes: measured round, drawn through the scalar path with the angle folded into the first spike. |
| [`dab_sizes_match_krita_s_test`](../../../../../src/tools/krita/tip/auto.rs#L972) | Krita's own `KisAutoBrushTest::testDabSize`, except its two exact ±90° cases. There Krita's test expects 6×10 and 6×11 (a "ceil rule"); Qt 5.15's `rotateRadians` and `mapRect` and Krita 5.2's `roundRect`, worked through by hand, give 5×10 for both, and so does this port. Unresolved; it only shows on an exact quarter turn (a view turned 90° with rotation off, say), and only in the mask's size. |
| [`a_small_circle_is_krita_s_reference_dab`](../../../../../src/tools/krita/tip/auto.rs#L995) | Krita's `testMaskGeneration`: a 10 px hard circle must come out identical to `result_autobrush_3.png` from Krita's test data. |

**Test helpers** (used only by the tests above):

- [`def`](../../../../../src/tools/krita/tip/auto.rs#L835) — Def.
- [`unit`](../../../../../src/tools/krita/tip/auto.rs#L840) — Unit.

**Test-only types and constants:** [`ERASER_SOFT`](../../../../../src/tools/krita/tip/auto.rs#L848)
