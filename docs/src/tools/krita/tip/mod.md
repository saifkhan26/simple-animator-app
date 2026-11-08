# `src/tools/krita/tip/mod.rs` — bitmap brush tips

> **Source:** [src/tools/krita/tip/mod.rs](../../../../../src/tools/krita/tip/mod.rs) · **Module:** `crate::tools::krita::tip` · **Size:** 483 lines · Line links as of commit `36405ad`

## What this file is for

A **brush tip** is the small image stamped at every dab (a "stamp mask"). This file handles **bitmap tips**
(`KisPngBrush`): a grey image where dark means paint. To draw a tip at any size and angle quickly and exactly as
Krita does, it keeps a **pyramid** (`KisQImagePyramid`) of pre-scaled copies — like mipmaps in a game engine —
and draws the nearest level through Qt's transform (`qt::draw_image`) for each dab's scale, ratio, rotation and
sub-pixel offset. Every level carries a 1-pixel transparent border so bilinear sampling fades out at the edges.

```text
tip image 64×64 → pyramid: 128, 64, 32, 16, 8 …   dab needs 20 px → use the 32 level, scaled 0.625
```

The other tip kinds live in [auto.rs](auto.md) (generated round/square tips) and [gbr.rs](gbr.md) (GIMP
`.gbr`/`.gih` files).

## How it connects

- **Used by:** [tools/krita/mod.rs](../mod.md) (`Tip::Png`, mask generation per dab), [gbr.rs](gbr.md) (cells become
  `PngTip`s).
- **Uses:** [tools/krita/qt.rs](../qt.md) (`Image32`, `QTransform`, `scaled_smooth`, `draw_image`).

## Rust concepts in this file

**Tuples for sizes and boxes.** `(i32, i32)` sizes and `(i32, i32, i32, i32)` boxes keep the Qt-like code compact.

**Unused parameters.** `_base_scale` (leading underscore) tells the compiler "intentionally unused" — kept so the
signature mirrors Krita's.

## Types

### `const MIPMAP_SIZE_THRESHOLD`
[src/tools/krita/tip/mod.rs:22](../../../../../src/tools/krita/tip/mod.rs#L22)

```rust
const MIPMAP_SIZE_THRESHOLD: i32 = 512;
```

Smallest level size the pyramid builds down to.

### `const MAX_MIPMAP_SCALE`
[src/tools/krita/tip/mod.rs:23](../../../../../src/tools/krita/tip/mod.rs#L23)

```rust
const MAX_MIPMAP_SCALE: f64 = 8.0;
```

Largest up-scaled level (above the original).

### `const BORDER`
[src/tools/krita/tip/mod.rs:27](../../../../../src/tools/krita/tip/mod.rs#L27)

```rust
const BORDER: i32 = 1;
```

`QPAINTER_WORKAROUND_BORDER`: every level carries a one-pixel transparent frame so Qt's bilinear sampling fades out at the tip's edge instead of smearing the edge pixels outwards.

### `struct DabShape`
[src/tools/krita/tip/mod.rs:32](../../../../../src/tools/krita/tip/mod.rs#L32)

`KisDabShape`: the size multiplier, the height-to-width ratio and the rotation.

| Field | Type | Meaning |
|---|---|---|
| `scale` | `f64` | Size multiplier. |
| `ratio` | `f64` | Height-to-width ratio. |
| `rotation` | `f64` | Rotation (radians). |

### `struct Level`
[src/tools/krita/tip/mod.rs:47](../../../../../src/tools/krita/tip/mod.rs#L47)

One pre-scaled copy of the tip.

| Field | Type | Meaning |
|---|---|---|
| `image` | `Image32` | The level, `Format_ARGB32`, with its border. |
| `size` | `(i32, i32)` | Its size without the border. |

### `struct Pyramid`
[src/tools/krita/tip/mod.rs:55](../../../../../src/tools/krita/tip/mod.rs#L55)

The pre-scaled copies of a tip (`KisQImagePyramid`).

| Field | Type | Meaning |
|---|---|---|
| `original` | `(i32, i32)` | The tip's original size. |
| `base_scale` | `f64` | Scale of the base level. |
| `levels` | `Vec<Level>` | The levels, largest first. |

### `struct PngTip`
[src/tools/krita/tip/mod.rs:255](../../../../../src/tools/krita/tip/mod.rs#L255)

A bitmap tip, as the preset's `brush_definition` sets it up.

| Field | Type | Meaning |
|---|---|---|
| `pyramid` | `Pyramid` | Pre-scaled copies. |
| `width` | `i32` | Tip width. |
| `height` | `i32` | Tip height. |
| `angle` | `f64` | `KisBrush::angle`, radians. |
| `footprint` | `(i32, i32, i32, i32)` | The box round every pixel the tip marks at all, in tip pixels: left, top, right, bottom (exclusive). Krita outlines the tip by where its mask is non-zero. |

## `impl DabShape`

### `DabShape::scale_x`
[src/tools/krita/tip/mod.rs:39](../../../../../src/tools/krita/tip/mod.rs#L39)

```rust
fn scale_x(&self) -> f64
```

The horizontal scale (`scale`).

### `DabShape::scale_y`
[src/tools/krita/tip/mod.rs:42](../../../../../src/tools/krita/tip/mod.rs#L42)

```rust
fn scale_y(&self) -> f64
```

The vertical scale (`scale × ratio`).

## Free functions

### `size_times`
[src/tools/krita/tip/mod.rs:62](../../../../../src/tools/krita/tip/mod.rs#L62)

```rust
fn size_times(s: (i32, i32), k: f64) -> (i32, i32)
```

`QSize * qreal`.

## `impl Pyramid`

### `Pyramid::new`
[src/tools/krita/tip/mod.rs:69](../../../../../src/tools/krita/tip/mod.rs#L69)

```rust
fn new(base: &Image32) -> Self
```

`base` is the tip as Krita holds it: a `Format_Grayscale8` image, given here already expanded to opaque `RGB32` gray.

### `Pyramid::level`
[src/tools/krita/tip/mod.rs:109](../../../../../src/tools/krita/tip/mod.rs#L109)

```rust
fn level(image: &Image32) -> Level
```

`appendPyramidLevel`.

### `Pyramid::nearest_level`
[src/tools/krita/tip/mod.rs:124](../../../../../src/tools/krita/tip/mod.rs#L124)

```rust
fn nearest_level(&self, scale: f64) -> (usize, f64)
```

`findNearestLevel`.

### `Pyramid::create_image`
[src/tools/krita/tip/mod.rs:137](../../../../../src/tools/krita/tip/mod.rs#L137)

```rust
fn create_image(&self, shape: DabShape, sub_x: f64, sub_y: f64) -> Image32
```

`createImage`: the tip drawn at `shape`, `Format_ARGB32`.

## Free functions

### `base_brush_transform`
[src/tools/krita/tip/mod.rs:168](../../../../../src/tools/krita/tip/mod.rs#L168)

```rust
fn base_brush_transform(shape: DabShape, sub_x: f64, sub_y: f64, bounds: RectF) -> QTransform
```

`baseBrushTransform`.

### `round_rect`
[src/tools/krita/tip/mod.rs:184](../../../../../src/tools/krita/tip/mod.rs#L184)

```rust
fn round_rect(rc: RectF) -> (i32, i32, i32, i32)
```

`roundRect` in kis_qimage_pyramid.cpp: `toAlignedRect`, but forgiving of float error at the origin and in the size.

### `calculate_params`
[src/tools/krita/tip/mod.rs:211](../../../../../src/tools/krita/tip/mod.rs#L211)

```rust
fn calculate_params(
    shape: DabShape,
    sub_x: f64,
    sub_y: f64,
    original: (i32, i32),
    _base_scale: f64,
    base_size: (i32, i32),
) -> (QTransform, (i32, i32))
```

`KisQImagePyramid::calculateParams`: the transform that draws a level, and the size of the image it lands in.

## `impl PngTip`

### `PngTip::load`
[src/tools/krita/tip/mod.rs:271](../../../../../src/tools/krita/tip/mod.rs#L271)

```rust
pub fn load(png: &[u8], angle: f64) -> Result<Self>
```

`KisPngBrush::loadFromDevice`. Only the gray, opaque tips Krita turns into a mask are accepted; colour and transparent tips are painted as image stamps or lightness maps, which this engine does not do.

### `PngTip::from_gray`
[src/tools/krita/tip/mod.rs:289](../../../../../src/tools/krita/tip/mod.rs#L289)

```rust
pub fn from_gray(w: u32, h: u32, values: &[u8], angle: f64) -> Self
```

A tip from gray values, row-major (255 = nothing, as Krita holds a mask tip's image): a PNG's pixels, or a GIMP brush cell's.

### `PngTip::original_rect`
[src/tools/krita/tip/mod.rs:319](../../../../../src/tools/krita/tip/mod.rs#L319)

```rust
fn original_rect(&self) -> RectF
```

The tip's original rectangle (for transforms).

### `PngTip::size`
[src/tools/krita/tip/mod.rs:329](../../../../../src/tools/krita/tip/mod.rs#L329)

```rust
pub fn size(&self) -> (i32, i32)
```

The tip image's size in pixels.

### `PngTip::outline`
[src/tools/krita/tip/mod.rs:336](../../../../../src/tools/krita/tip/mod.rs#L336)

```rust
pub fn outline(&self, brush_scale: f64, shape: DabShape) -> [(f64, f64); 4]
```

The corners of the tip's footprint as a dab of `shape` lays it down, relative to the dab's centre, in canvas pixels. Turned the way the tip is drawn: by minus the dab's rotation plus the brush's angle.

### `PngTip::characteristic_size`
[src/tools/krita/tip/mod.rs:353](../../../../../src/tools/krita/tip/mod.rs#L353)

```rust
pub fn characteristic_size(&self, brush_scale: f64, shape: DabShape) -> (f64, f64)
```

`KisBrush::characteristicSize`.

### `PngTip::hot_spot`
[src/tools/krita/tip/mod.rs:365](../../../../../src/tools/krita/tip/mod.rs#L365)

```rust
pub fn hot_spot(&self, brush_scale: f64, shape: DabShape) -> (f64, f64)
```

`KisBrush::hotSpot`: the middle of the dab, at least half a pixel in.

### `PngTip::mask_size`
[src/tools/krita/tip/mod.rs:371](../../../../../src/tools/krita/tip/mod.rs#L371)

```rust
pub fn mask_size(&self, brush_scale: f64, shape: DabShape, sub_x: f64, sub_y: f64) -> (i32, i32)
```

`KisBrush::maskWidth` and `maskHeight`.

### `PngTip::mask`
[src/tools/krita/tip/mod.rs:385](../../../../../src/tools/krita/tip/mod.rs#L385)

```rust
pub fn mask(&self, brush_scale: f64, shape: DabShape, sub_x: f64, sub_y: f64) -> (Vec<u8>, usize, usize)
```

`KisBrush::generateMaskAndApplyMaskOrCreateDab` for a plain colour: the dab's alpha, row-major, with its size. The tip is drawn at the *negated* angle, while `mask_size` measures it at the positive one — Krita does both, and the two can differ by a pixel.

## Tests

### `mod tests`

Run with `cargo test tools::krita::tip::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`pyramid_levels_match_krita_for_a_220px_tip`](../../../../../src/tools/krita/tip/mod.rs#L419) | Pyramid levels match krita for a 220px tip. |
| [`mask_rotates_with_the_dab`](../../../../../src/tools/krita/tip/mod.rs#L433) | The mask's footprint follows the rotation: the tip is a tall thin bar, so a quarter turn swaps the mask's width and height. |
| [`mask_size_agrees_with_the_drawn_image_to_a_pixel`](../../../../../src/tools/krita/tip/mod.rs#L469) | Mask size agrees with the drawn image to a pixel. |

**Test helpers** (used only by the tests above):

- [`tip`](../../../../../src/tools/krita/tip/mod.rs#L414) — Tip.

**Test-only types and constants:** [`SCALE`](../../../../../src/tools/krita/tip/mod.rs#L410)
