# `src/tools/krita/qt.rs` — Qt's painting maths, reproduced

> **Source:** [src/tools/krita/qt.rs](../../../../src/tools/krita/qt.rs) · **Module:** `crate::tools::krita::qt` · **Size:** 1725 lines · Line links as of commit `36405ad`

## What this file is for

Krita doesn't rasterise bitmap brush tips itself — it hands them to **Qt** (the C++ UI/graphics library Krita
is built on): `QImage::scaled` builds the tip's size levels and scales the paper texture, and `QPainter`
draws a rotated tip. To paint a Krita preset **bit-for-bit like Krita**, this file reproduces those exact Qt
5.15 routines — including their rounding, fixed-point arithmetic and SSE-specific quirks.

Why exactness matters: the golden tests compare image hashes; a pencil stroke is thousands of overlapping dabs,
so a 1-in-256 rounding difference per dab becomes a visible difference. Examples of what must match:

- **`qRound` rounds halves up**, not away from zero: `qRound(-2.5) = -2`, while Rust's `(-2.5f64).round() = -3`.
- **Fixed-point 16.16 and 26.6 numbers.** Qt's rasteriser stores x as an integer scaled by 65536 (16.16) or
  points as integers scaled by 64 (26.6). E.g. 3.25 in 26.6 is `3.25 × 64 = 208`.
- **`_mm_rcp_ss`**: an approximate reciprocal (`1/x`) instruction whose result depends on the CPU vendor; Krita
  divides through it, so this port does too (and the `exact-rcp` feature replaces it for portable tests).
- **Premultiplied 16-bit intermediates** (`Rgba64`) and `div_257` (exact `x/257` for 16-bit → 8-bit).

## How it connects

- **Used by:** [tools/krita/tip/mod.rs](tip/mod.md) and [tip/gbr.rs](tip/gbr.md) (scaling and drawing tips),
  [tools/krita/texture.rs](texture.md) (scaling textures), [tools/golden_tests.rs](../golden_tests.md)
  (`rcp_signature`).
- **Uses:** nothing outside this folder.

## Rust concepts in this file

**Packed pixels in a `u32`.** `0xAARRGGBB` per pixel; `q_red(p) = (p >> 16) & 0xff` etc. — the same bit tricks as
in JS, on unsigned 32-bit integers.

**Interior mutability with `Cell`.** `QTransform` caches its type classification in `Cell<Tx>` so `ty(&self)` can
update the cache through a shared reference, exactly as Qt's `mutable` members do. More:
[Smart pointers](../../../rust-primer.md#smart-pointers)

**Enums with explicit discriminants.** `enum Tx { None = 0, Translate = 1, Scale = 2, Rotate = 4, Shear = 8 }` —
compared numerically, as Qt does.

**CPU intrinsics.** `rcp` uses `std::arch::x86_64::_mm_rcp_ss` inside `unsafe` (an SSE instruction), with a
fallback on other CPUs. More: [Unsafe and FFI](../../../rust-primer.md#unsafe-and-ffi)

## Types

### `struct Image32`
[src/tools/krita/qt.rs:23](../../../../src/tools/krita/qt.rs#L23)

A `QImage` in one of the 32-bit formats: one `0xAARRGGBB` word per pixel, rows packed. Whether the colour is premultiplied is the caller's business, exactly as with `Format_ARGB32` versus `Format_ARGB32_Premultiplied`.

| Field | Type | Meaning |
|---|---|---|
| `w` | `usize` | Width. |
| `h` | `usize` | Height. |
| `px` | `Vec<u32>` | Pixels, one `0xAARRGGBB` word each, rows packed. |

### `enum Tx`
[src/tools/krita/qt.rs:168](../../../../src/tools/krita/qt.rs#L168)

`QTransform::TransformationType`. The discriminants are Qt's: the "dirty" bookkeeping compares them numerically.

| Variant | Meaning |
|---|---|
| `None = 0` | The identity. |
| `Translate = 1` | Translation only. |
| `Scale = 2` | Scale (and translation). |
| `Rotate = 4` | Rotation (and scale/translation). |
| `Shear = 8` | A general affine transform. |

### `struct QTransform`
[src/tools/krita/qt.rs:183](../../../../src/tools/krita/qt.rs#L183)

An affine `QTransform`, including its lazily classified type.

The classification is part of the arithmetic, not an optimisation to be skipped: `scale()` on a matrix Qt has already decided is the identity *overwrites* the diagonal instead of multiplying into it, and Krita's pyramid leans on that when it nudges a pure translation off being one.

| Field | Type | Meaning |
|---|---|---|
| `m11` | `f64` | Matrix element row 1, column 1. |
| `m12` | `f64` | Row 1, column 2. |
| `m21` | `f64` | Row 2, column 1. |
| `m22` | `f64` | Row 2, column 2. |
| `dx` | `f64` | Translation x. |
| `dy` | `f64` | Translation y. |
| `ty` | `Cell<Tx>` | Cached transformation type. |
| `dirty` | `Cell<Tx>` | How far the cached type may be out of date. |

### `struct RectF`
[src/tools/krita/qt.rs:594](../../../../src/tools/krita/qt.rs#L594)

`QRectF`: origin and size.

| Field | Type | Meaning |
|---|---|---|
| `x` | `f64` | Left. |
| `y` | `f64` | Top. |
| `w` | `f64` | Width. |
| `h` | `f64` | Height. |

### `struct Rect`
[src/tools/krita/qt.rs:622](../../../../src/tools/krita/qt.rs#L622)

`QRect`, as origin and size (Qt's inclusive `right()` is `x + w - 1`).

| Field | Type | Meaning |
|---|---|---|
| `x` | `i32` | Left. |
| `y` | `i32` | Top. |
| `w` | `i32` | Width. |
| `h` | `i32` | Height. |

### `type Rgba64`
[src/tools/krita/qt.rs:893](../../../../src/tools/krita/qt.rs#L893)

```rust
type Rgba64 = [u16; 4];
```

A premultiplied 16-bit-per-channel pixel (Qt's `QRgba64`), as four `u16`.

### `struct TextureMatrix`
[src/tools/krita/qt.rs:1010](../../../../src/tools/krita/qt.rs#L1010)

The inverse matrix and its 16.16 set-up, from `QSpanData::setupMatrix`.

| Field | Type | Meaning |
|---|---|---|
| `m11` | `f64` | Inverse matrix row 1, column 1. |
| `m12` | `f64` | Row 1, column 2. |
| `m21` | `f64` | Row 2, column 1. |
| `m22` | `f64` | Row 2, column 2. |
| `dx` | `f64` | Inverse translation x. |
| `dy` | `f64` | Inverse translation y. |

### `struct ScanLine`
[src/tools/krita/qt.rs:1381](../../../../src/tools/krita/qt.rs#L1381)

A `QScanConverter::Line`: an edge, with x in 16.16 fixed point.

| Field | Type | Meaning |
|---|---|---|
| `x` | `i32` | Current x, 16.16 fixed point. |
| `delta` | `i32` | x step per row, 16.16. |
| `top` | `i32` | First row. |
| `bottom` | `i32` | Last row. |
| `winding` | `i32` | +1 or −1 by edge direction. |

### `struct ScanConverter`
[src/tools/krita/qt.rs:1395](../../../../src/tools/krita/qt.rs#L1395)

`QScanConverter` with no legacy rounding.

| Field | Type | Meaning |
|---|---|---|
| `top` | `i32` | Clip top row. |
| `bottom` | `i32` | Clip bottom row. |
| `left_fp` | `i32` | Clip left, fixed point. |
| `right_fp` | `i32` | Clip right, fixed point. |
| `lines` | `Vec<ScanLine>` | The edges collected so far. |

## `impl Image32`

### `Image32::new`
[src/tools/krita/qt.rs:30](../../../../src/tools/krita/qt.rs#L30)

```rust
pub fn new(w: usize, h: usize) -> Self
```

A transparent `w`×`h` image.

### `Image32::copy`
[src/tools/krita/qt.rs:40](../../../../src/tools/krita/qt.rs#L40)

```rust
pub fn copy(&self, x: i32, y: i32, w: usize, h: usize) -> Self
```

`QImage::copy(x, y, w, h)`: the part outside the source comes back zero, which for `Format_ARGB32` is transparent.

## Free functions

### `q_red`
[src/tools/krita/qt.rs:60](../../../../src/tools/krita/qt.rs#L60)

```rust
pub fn q_red(p: u32) -> u32
```

The red byte of a packed pixel (`qRed`).

### `q_green`
[src/tools/krita/qt.rs:64](../../../../src/tools/krita/qt.rs#L64)

```rust
pub fn q_green(p: u32) -> u32
```

The green byte (`qGreen`).

### `q_blue`
[src/tools/krita/qt.rs:68](../../../../src/tools/krita/qt.rs#L68)

```rust
pub fn q_blue(p: u32) -> u32
```

The blue byte (`qBlue`).

### `q_alpha`
[src/tools/krita/qt.rs:72](../../../../src/tools/krita/qt.rs#L72)

```rust
pub fn q_alpha(p: u32) -> u32
```

The alpha byte (`qAlpha`).

### `q_rgba`
[src/tools/krita/qt.rs:76](../../../../src/tools/krita/qt.rs#L76)

```rust
pub fn q_rgba(r: u32, g: u32, b: u32, a: u32) -> u32
```

Packs four bytes into `0xAARRGGBB` (`qRgba`).

### `qround`
[src/tools/krita/qt.rs:82](../../../../src/tools/krita/qt.rs#L82)

```rust
pub fn qround(d: f64) -> i32
```

`qRound(double)`, which rounds halves up rather than away from zero.

### `fuzzy_is_null`
[src/tools/krita/qt.rs:91](../../../../src/tools/krita/qt.rs#L91)

```rust
pub fn fuzzy_is_null(d: f64) -> bool
```

Qt's `qFuzzyIsNull`: |d| ≤ 1e-12.

### `fuzzy_compare`
[src/tools/krita/qt.rs:96](../../../../src/tools/krita/qt.rs#L96)

```rust
pub fn fuzzy_compare(p1: f64, p2: f64) -> bool
```

Qt's `qFuzzyCompare`: equal within a relative 1e-12.

### `rcp`
[src/tools/krita/qt.rs:109](../../../../src/tools/krita/qt.rs#L109)

```rust
pub fn rcp(x: f32) -> f32
```

Approximate reciprocal, `_mm_rcp_ss`. Krita and Qt both divide through this instruction in their pixel loops; its result is the CPU's own table lookup, so the only way to agree with them bit for bit is to ask the same instruction.

The `exact-rcp` feature swaps it for a true division, which no Krita matches but every CPU agrees on: the golden tests use that build for hashes that do not depend on the machine.

### `rcp_signature`
[src/tools/krita/qt.rs:127](../../../../src/tools/krita/qt.rs#L127)

```rust
pub fn rcp_signature() -> u64
```

A fingerprint of this CPU's `rcp` table: two machines with the same signature divide identically, so they paint Krita strokes identically.

### `reciprocal_mul`
[src/tools/krita/qt.rs:144](../../../../src/tools/krita/qt.rs#L144)

```rust
fn reciprocal_mul(a: f32, mul: f32) -> f32
```

`reciprocal_mul_ss` / `reciprocal_mul_ps`: the approximate reciprocal sharpened by one Newton-Raphson step, then scaled.

### `cvt_round`
[src/tools/krita/qt.rs:152](../../../../src/tools/krita/qt.rs#L152)

```rust
fn cvt_round(x: f32) -> i32
```

`_mm_cvtps_epi32` under the default rounding mode: nearest, ties to even.

## `impl Default for QTransform`

### `QTransform::default`
[src/tools/krita/qt.rs:195](../../../../src/tools/krita/qt.rs#L195)

```rust
fn default() -> Self
```

The identity transform.

## `impl QTransform`

### `QTransform::identity`
[src/tools/krita/qt.rs:201](../../../../src/tools/krita/qt.rs#L201)

```rust
pub fn identity() -> Self
```

The identity transform.

### `QTransform::from_translate`
[src/tools/krita/qt.rs:214](../../../../src/tools/krita/qt.rs#L214)

```rust
pub fn from_translate(dx: f64, dy: f64) -> Self
```

A translation by `(dx, dy)`.

### `QTransform::from_scale`
[src/tools/krita/qt.rs:226](../../../../src/tools/krita/qt.rs#L226)

```rust
pub fn from_scale(sx: f64, sy: f64) -> Self
```

A scale by `(sx, sy)`.

### `QTransform::ty`
[src/tools/krita/qt.rs:235](../../../../src/tools/krita/qt.rs#L235)

```rust
pub fn ty(&self) -> Tx
```

`QTransform::type()`, caching its answer the way Qt does.

### `QTransform::inline_type`
[src/tools/krita/qt.rs:272](../../../../src/tools/krita/qt.rs#L272)

```rust
fn inline_type(&self) -> Tx
```

The type implied by the matrix entries, computed directly.

### `QTransform::mark_dirty`
[src/tools/krita/qt.rs:280](../../../../src/tools/krita/qt.rs#L280)

```rust
fn mark_dirty(&self, at_least: Tx)
```

Records that the cached type must be at least `at_least` after an edit.

### `QTransform::is_identity`
[src/tools/krita/qt.rs:286](../../../../src/tools/krita/qt.rs#L286)

```rust
pub fn is_identity(&self) -> bool
```

Whether the transform is the identity.

### `QTransform::translate`
[src/tools/krita/qt.rs:290](../../../../src/tools/krita/qt.rs#L290)

```rust
pub fn translate(&mut self, dx: f64, dy: f64) -> &mut Self
```

Qt's `translate`: appends a translation, in place; returns `&mut Self` for chaining.

### `QTransform::scale`
[src/tools/krita/qt.rs:316](../../../../src/tools/krita/qt.rs#L316)

```rust
pub fn scale(&mut self, sx: f64, sy: f64) -> &mut Self
```

Qt's `scale`: appends a scale, in place (with Qt's type-dependent arithmetic).

### `QTransform::rotate_radians`
[src/tools/krita/qt.rs:340](../../../../src/tools/krita/qt.rs#L340)

```rust
pub fn rotate_radians(&mut self, a: f64) -> &mut Self
```

Qt's `rotateRadians`: appends a rotation, using Qt's exact special cases for multiples of 90°.

### `QTransform::mul`
[src/tools/krita/qt.rs:376](../../../../src/tools/krita/qt.rs#L376)

```rust
pub fn mul(&self, m: &QTransform) -> QTransform
```

`self * m`: apply `self`, then `m`.

### `QTransform::mul_assign`
[src/tools/krita/qt.rs:415](../../../../src/tools/krita/qt.rs#L415)

```rust
pub fn mul_assign(&mut self, o: &QTransform)
```

`operator*=`. Differs from `mul` only in how it treats an identity left-hand side, which it replaces wholesale.

### `QTransform::inverted`
[src/tools/krita/qt.rs:461](../../../../src/tools/krita/qt.rs#L461)

```rust
pub fn inverted(&self) -> QTransform
```

The inverse transform (Qt's `inverted`), with Qt's per-type shortcuts.

### `QTransform::map_with`
[src/tools/krita/qt.rs:497](../../../../src/tools/krita/qt.rs#L497)

```rust
fn map_with(&self, t: Tx, x: f64, y: f64) -> (f64, f64)
```

Maps a point using the arithmetic for transform type `t`.

### `QTransform::map`
[src/tools/krita/qt.rs:509](../../../../src/tools/krita/qt.rs#L509)

```rust
pub fn map(&self, x: f64, y: f64) -> (f64, f64)
```

Maps a point (`QTransform::map`).

### `QTransform::map_rect`
[src/tools/krita/qt.rs:515](../../../../src/tools/krita/qt.rs#L515)

```rust
pub fn map_rect(&self, r: RectF) -> RectF
```

`QTransform::mapRect(const QRectF &)`.

### `QTransform::map_rect_int`
[src/tools/krita/qt.rs:565](../../../../src/tools/krita/qt.rs#L565)

```rust
pub fn map_rect_int(&self, r: Rect) -> Rect
```

`QTransform::mapRect(const QRect &)`, for the scale-only case it is used with.

## `impl RectF`

### `RectF::is_valid`
[src/tools/krita/qt.rs:602](../../../../src/tools/krita/qt.rs#L602)

```rust
pub fn is_valid(self) -> bool
```

Whether width and height are positive.

### `RectF::to_aligned_rect`
[src/tools/krita/qt.rs:606](../../../../src/tools/krita/qt.rs#L606)

```rust
pub fn to_aligned_rect(self) -> Rect
```

Qt's `toAlignedRect`: the smallest integer rectangle containing it.

## Free functions

### `calc_points`
[src/tools/krita/qt.rs:633](../../../../src/tools/krita/qt.rs#L633)

```rust
fn calc_points(s: i32, d: i32) -> Vec<i32>
```

Qt's `qimageCalcYPoints`/`XPoints`: source sample positions for each destination row/column when scaling.

### `calc_a_points`
[src/tools/krita/qt.rs:649](../../../../src/tools/krita/qt.rs#L649)

```rust
fn calc_a_points(s: i32, d: i32, up: bool) -> Vec<i32>
```

Qt's `qimageCalcApoints`: the fixed-point weights for each destination row/column (up- or down-scaling).

### `channels`
[src/tools/krita/qt.rs:677](../../../../src/tools/krita/qt.rs#L677)

```rust
fn channels(p: u32) -> [u32; 4]
```

Splits a packed pixel into four channel values.

### `scale_helper`
[src/tools/krita/qt.rs:684](../../../../src/tools/krita/qt.rs#L684)

```rust
fn scale_helper(src: &[u32], mut pix: usize, xyap: i32, cxy: i32, step: usize) -> [u32; 4]
```

`qt_qimageScaleAARGBA_helper`, in the unsigned 32-bit lanes the SSE4.1 kernel accumulates in.

### `interpolate_pixel_256`
[src/tools/krita/qt.rs:705](../../../../src/tools/krita/qt.rs#L705)

```rust
fn interpolate_pixel_256(x: u32, a: u32, y: u32, b: u32) -> u32
```

`INTERPOLATE_PIXEL_256`.

### `interpolate_4_pixels`
[src/tools/krita/qt.rs:717](../../../../src/tools/krita/qt.rs#L717)

```rust
fn interpolate_4_pixels(tl: u32, tr: u32, bl: u32, br: u32, distx: u32, disty: u32) -> u32
```

`interpolate_4_pixels_sse2`: vertical first, then horizontal.

### `smooth_scale`
[src/tools/krita/qt.rs:734](../../../../src/tools/krita/qt.rs#L734)

```rust
pub fn smooth_scale(src: &Image32, dw: usize, dh: usize, alpha: bool) -> Image32
```

`qSmoothScaleImage` for a 32-bit source. `alpha` picks the ARGB kernels (`hasAlphaChannel()`); without it the result is forced opaque, as the RGB32 kernels do.

### `scaled_smooth`
[src/tools/krita/qt.rs:851](../../../../src/tools/krita/qt.rs#L851)

```rust
pub fn scaled_smooth(src: &Image32, w: usize, h: usize, alpha: bool) -> Image32
```

`QImage::scaled(size, aspect, Qt::SmoothTransformation)` for a 32-bit image, down to the size it actually produces. `premultiplied` says whether `src` is already `ARGB32_Premultiplied` (or opaque `RGB32`); `Format_ARGB32` sources are premultiplied first, as `smoothScaled` does.

### `premultiply_argb32`
[src/tools/krita/qt.rs:874](../../../../src/tools/krita/qt.rs#L874)

```rust
pub fn premultiply_argb32(p: u32) -> u32
```

`qPremultiply`.

### `argb32_to_rgba64_pm`
[src/tools/krita/qt.rs:896](../../../../src/tools/krita/qt.rs#L896)

```rust
fn argb32_to_rgba64_pm(p: u32) -> Rgba64
```

`QRgba64::fromArgb32(p).premultiplied()`.

### `mulhi`
[src/tools/krita/qt.rs:913](../../../../src/tools/krita/qt.rs#L913)

```rust
fn mulhi(a: u16, b: u16) -> u16
```

High 16 bits of a 16×16-bit product (SSE `mulhi`).

### `interpolate_4_pixels_rgb64`
[src/tools/krita/qt.rs:919](../../../../src/tools/krita/qt.rs#L919)

```rust
fn interpolate_4_pixels_rgb64(t: [Rgba64; 2], b: [Rgba64; 2], distx: u32, disty: u32) -> Rgba64
```

`interpolate_4_pixels_rgb64`, SSE2 flavour.

### `div_257`
[src/tools/krita/qt.rs:943](../../../../src/tools/krita/qt.rs#L943)

```rust
fn div_257(x: u32) -> u32
```

Exact `x / 257` the way Qt computes it (16-bit to 8-bit channel conversion).

### `store_argb32_from_rgba64_pm`
[src/tools/krita/qt.rs:952](../../../../src/tools/krita/qt.rs#L952)

```rust
fn store_argb32_from_rgba64_pm(dst: &mut [u32], src: &[Rgba64])
```

`convertARGBFromRGBA64PM_sse4<false>`: a span of premultiplied 16-bit pixels back to `Format_ARGB32`. It works four pixels at a time, and how it un-premultiplies depends on the whole block, so a pixel's result depends on where in its span it falls.

### `rgba64_to_rgb32_sse4`
[src/tools/krita/qt.rs:993](../../../../src/tools/krita/qt.rs#L993)

```rust
fn rgba64_to_rgb32_sse4(p: Rgba64) -> u32
```

`qConvertRgba64ToRgb32_sse4`.

## `impl TextureMatrix`

### `TextureMatrix::new`
[src/tools/krita/qt.rs:1020](../../../../src/tools/krita/qt.rs#L1020)

```rust
fn new(matrix: &QTransform) -> Self
```

Sets up the inverse matrix from a transform, as `QSpanData::setupMatrix` does.

## Free functions

### `pixel_bounds`
[src/tools/krita/qt.rs:1038](../../../../src/tools/krita/qt.rs#L1038)

```rust
fn pixel_bounds(v: i32, l2: i32) -> (i32, i32)
```

`fetchTransformedBilinear_pixelBounds<BlendTransformedBilinear>`.

### `blend_span`
[src/tools/krita/qt.rs:1050](../../../../src/tools/krita/qt.rs#L1050)

```rust
fn blend_span(dst: &mut Image32, src: &Image32, tm: &TextureMatrix, x: i32, y: i32, len: usize)
```

One span through `blend_src_generic_rgb64`: fetch (bilinear, 16-bit), SourceOver onto the zero-filled target, store back to ARGB32.

### `q26dot6_compare`
[src/tools/krita/qt.rs:1082](../../../../src/tools/krita/qt.rs#L1082)

```rust
fn q26dot6_compare(p1: f64, p2: f64) -> bool
```

Compares two values at 26.6 fixed-point precision (1/64 pixel).

### `snap_26dot6`
[src/tools/krita/qt.rs:1087](../../../../src/tools/krita/qt.rs#L1087)

```rust
fn snap_26dot6(p: (f64, f64)) -> (f64, f64)
```

Rounds a point to 26.6 fixed point.

### `safe_divide`
[src/tools/krita/qt.rs:1095](../../../../src/tools/krita/qt.rs#L1095)

```rust
fn safe_divide(x: f64, y: f64) -> f64
```

Qt's guarded division used by the rasteriser (avoiding overflow/division by zero).

### `safe_float_to_q16`
[src/tools/krita/qt.rs:1108](../../../../src/tools/krita/qt.rs#L1108)

```rust
fn safe_float_to_q16(x: f64) -> i32
```

Converts a float to 16.16 fixed point, clamped to the representable range.

### `rasterize_line`
[src/tools/krita/qt.rs:1122](../../../../src/tools/krita/qt.rs#L1122)

```rust
fn rasterize_line(a: (f64, f64), b: (f64, f64), mut width: f64, dw: i32, dh: i32) -> Vec<(i32, i32, i32)>
```

Qt's `QRasterizer::rasterizeLine` (aliased, no caps): the pixel spans a rotated image rectangle covers when drawn
as a thick line — how `QPainter` decides which pixels a rotated tip touches.

### `no_shear`
[src/tools/krita/qt.rs:1303](../../../../src/tools/krita/qt.rs#L1303)

```rust
fn no_shear(m: &QTransform) -> bool
```

`qt_scaleForTransform`'s answer: is the transform free of shear?

### `draw_image`
[src/tools/krita/qt.rs:1329](../../../../src/tools/krita/qt.rs#L1329)

```rust
pub fn draw_image(dst: &mut Image32, src: &Image32, matrix: &QTransform)
```

Qt's `QPainter::drawImage` with a transform and smooth pixmap filtering, onto a transparent target — the path
Krita's tip pyramid takes: picks the pixel spans with `rasterize_line`/`fill_transformed_rect` and blends each span
with `blend_span`.

### `q16_mul`
[src/tools/krita/qt.rs:1390](../../../../src/tools/krita/qt.rs#L1390)

```rust
fn q16_mul(x: i32, y: i32) -> i32
```

Multiplies two 16.16 fixed-point numbers.

## `impl ScanConverter`

### `ScanConverter::clip`
[src/tools/krita/qt.rs:1405](../../../../src/tools/krita/qt.rs#L1405)

```rust
fn clip(&mut self, x_fp: &mut i32, i_top: &mut i32, i_bottom: &mut i32, slope_fp: i32, edge_fp: i32, winding: i32) -> bool
```

`QScanConverter::clip`.

### `ScanConverter::merge_line`
[src/tools/krita/qt.rs:1453](../../../../src/tools/krita/qt.rs#L1453)

```rust
fn merge_line(&mut self, mut a: (i32, i32), mut b: (i32, i32))
```

`QScanConverter::mergeLine`, points in 26.6.

### `ScanConverter::end`
[src/tools/krita/qt.rs:1485](../../../../src/tools/krita/qt.rs#L1485)

```rust
fn end(mut self) -> Vec<(i32, u32, i32)>
```

`qScanConvert` with the odd-even rule (a path's default): the spans as `(x, len, y)`.

## Free functions

### `fill_transformed_rect`
[src/tools/krita/qt.rs:1545](../../../../src/tools/krita/qt.rs#L1545)

```rust
fn fill_transformed_rect(r: RectF, matrix: &QTransform, dw: i32, dh: i32) -> Vec<(i32, u32, i32)>
```

`fillPath(rect)` through `matrix`, aliased: the outline mapper maps the rect's corners and rounds them to 26.6, and the scan converter fills the pixel centres inside, clipped to the `dw` × `dh` device.

## Tests

### `mod shear_tests`

Run with `cargo test tools::krita::qt::shear_tests` · 2 tests

| Test | What it checks |
|---|---|
| [`the_path_filler_covers_pixel_centres`](../../../../src/tools/krita/qt.rs#L1583) | Filling an unsheared square through the path filler covers exactly the pixel centres inside it, like any aliased fill. |
| [`a_sheared_rect_fills_a_parallelogram`](../../../../src/tools/krita/qt.rs#L1594) | A sheared rect fills a parallelogram, clipped to the device. |

### `mod tests`

Run with `cargo test tools::krita::qt::tests` · 8 tests

| Test | What it checks |
|---|---|
| [`qround_rounds_halves_up`](../../../../src/tools/krita/qt.rs#L1615) | Qround rounds halves up. |
| [`transform_type_is_classified_lazily_and_fuzzily`](../../../../src/tools/krita/qt.rs#L1624) | Transform type is classified lazily and fuzzily. |
| [`inverse_round_trips`](../../../../src/tools/krita/qt.rs#L1633) | Inverse round trips. |
| [`smooth_scale_preserves_a_flat_image`](../../../../src/tools/krita/qt.rs#L1651) | Scaling a flat colour must give the same flat colour back, whichever of the four kernels runs. |
| [`smooth_downscale_averages`](../../../../src/tools/krita/qt.rs#L1668) | Smooth downscale averages. |
| [`unpremultiply_block_matches_exact_division_closely`](../../../../src/tools/krita/qt.rs#L1684) | Unpremultiply block matches exact division closely. |
| [`draw_image_scales_onto_the_target`](../../../../src/tools/krita/qt.rs#L1703) | An axis-aligned 2x upscale through the transformed path lands every source pixel where it should, and fills the whole target. |
| [`rotated_spans_stay_inside_the_target`](../../../../src/tools/krita/qt.rs#L1717) | A rotated image covers a diamond of spans: none may fall outside the target, and the middle row must be the widest. |
