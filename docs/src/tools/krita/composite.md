# `src/tools/krita/composite.rs` — Krita's 8-bit blending of dabs

> **Source:** [src/tools/krita/composite.rs](../../../../src/tools/krita/composite.rs) · **Module:** `crate::tools::krita::composite` · **Size:** 383 lines · Line links as of commit `67a7cb9`

## What this file is for

How a Krita dab's alpha lands on pixels, using Krita's own 8-bit integer arithmetic (from `KoIntegerMaths.h`
and `KoColorSpaceMaths.h`) so results match bit-for-bit:

- **`over`** — "build-up" mode: each dab is composited source-over directly onto the layer (with selection mask
  and alpha lock).
- **`erase`** — takes alpha away, leaves colour; fully transparent pixels are cleared.
- **alpha-darken** (`darken_pixel`, `darken_batch`, `darken_run`) — "wash" mode: dabs build up in a stroke buffer
  where each pixel's alpha only rises toward the dab's (never past the stroke's opacity), so overlapping dabs
  in one stroke don't darken; the buffer is merged onto the layer afterwards.

**Integer multiply example.** `UINT8_MULT(a, b) = a·b/255` rounded: `mul(200, 128)` = `(200·128 + 128 + …)/255` = 100.

Krita's vectorised (AVX2) path processes 8 pixels at a time and takes shortcuts per batch; `darken_batch`
reproduces that batching because the shortcuts change results slightly.

## How it connects

- **Used by:** [tools/krita/mod.rs](mod.md) (`render`, `darken`, `merge`).

## Rust concepts in this file

**`Option<u8>` for "maybe a mask".** `mask: Option<u8>` is the selection coverage at this pixel, `None` when
nothing is selected — so one function handles both cases. More:
[Option and Result](../../../rust-primer.md#option-and-result)

## Types

### `const U8_REC`
[src/tools/krita/composite.rs:44](../../../../src/tools/krita/composite.rs#L44)

```rust
const U8_REC: f32 = 1.0 / 255.0;
```

`1/255` as a float, for converting to 0–1.

### `struct Darken`
[src/tools/krita/composite.rs:115](../../../../src/tools/krita/composite.rs#L115)

One wash dab's alpha-darken parameters: `ParameterInfo` after `setOpacityAndAverage`, through `KoAlphaDarkenParamsWrapperCreamy` — creamy being Krita's default (`useCreamyAlphaDarken`).

| Field | Type | Meaning |
|---|---|---|
| `opacity` | `f32` | The stroke's opacity. |
| `flow` | `f32` | The dab's flow. |
| `average_opacity` | `f32` | The running average opacity (Krita's "creamy" alpha darken). |

### `const LANES`
[src/tools/krita/composite.rs:182](../../../../src/tools/krita/composite.rs#L182)

```rust
const LANES: usize = 8;
```

Pixels per vector batch (8, as Krita's AVX2 path).

## Free functions

### `mul`
[src/tools/krita/composite.rs:18](../../../../src/tools/krita/composite.rs#L18)

```rust
pub fn mul(a: u8, b: u8) -> u8
```

`UINT8_MULT`: `a * b / 255`, rounded.

### `mul3`
[src/tools/krita/composite.rs:25](../../../../src/tools/krita/composite.rs#L25)

```rust
pub fn mul3(a: u8, b: u8, c: u8) -> u8
```

`UINT8_MULT3`: `a * b * c / 255²`, rounded.

### `scale_to_u8`
[src/tools/krita/composite.rs:32](../../../../src/tools/krita/composite.rs#L32)

```rust
pub fn scale_to_u8(v: f64) -> u8
```

`KoColorSpaceMaths<double, quint8>::scaleToA`.

### `scale_to_u8_f32`
[src/tools/krita/composite.rs:39](../../../../src/tools/krita/composite.rs#L39)

```rust
pub fn scale_to_u8_f32(v: f32) -> u8
```

`KoColorSpaceMaths<float, quint8>::scaleToA`.

### `round_u8`
[src/tools/krita/composite.rs:48](../../../../src/tools/krita/composite.rs#L48)

```rust
fn round_u8(x: f32) -> u8
```

`round_float_to_u8`: SSE's round-half-to-even, keeping the low byte.

### `lerp_u8`
[src/tools/krita/composite.rs:53](../../../../src/tools/krita/composite.rs#L53)

```rust
fn lerp_u8(a: u8, b: u8, alpha: f32) -> u8
```

Linear interpolation between two bytes by `alpha` (0–1), rounded the SSE way.

### `over`
[src/tools/krita/composite.rs:61](../../../../src/tools/krita/composite.rs#L61)

```rust
pub fn over(dst: &mut [u8], color: [u8; 3], src_alpha: u8, opacity: f32, mask: Option<u8>, alpha_locked: bool)
```

`OverCompositor32::compositeOnePixelScalar`, all channels, alpha unlocked unless `alpha_locked`. `dst` is straight RGBA; `src_alpha` is the dab's own alpha; `mask` is the selection at this pixel.

### `erase`
[src/tools/krita/composite.rs:96](../../../../src/tools/krita/composite.rs#L96)

```rust
pub fn erase(dst: &mut [u8], src_alpha: u8, opacity: f32, mask: Option<u8>)
```

`KoCompositeOpErase<quint8>`: takes alpha away and leaves colour. A pixel left with no alpha is cleared outright, which is this app's convention for empty pixels and invisible either way.

## `impl Darken`

### `Darken::new`
[src/tools/krita/composite.rs:122](../../../../src/tools/krita/composite.rs#L122)

```rust
pub fn new(opacity: f32, flow: f32, average_opacity: f32) -> Self
```

Builds the alpha-darken parameters from opacity, flow and running average opacity (Krita's creamy variant).

## Free functions

### `blend_average_opacity`
[src/tools/krita/composite.rs:136](../../../../src/tools/krita/composite.rs#L136)

```rust
pub fn blend_average_opacity(opacity: f64, average: f64) -> f64
```

`KisPainter::blendAverageOpacity`: the running opacity a wash stroke carries from dab to dab.

### `lerp_f`
[src/tools/krita/composite.rs:147](../../../../src/tools/krita/composite.rs#L147)

```rust
fn lerp_f(a: f32, b: f32, t: f32) -> f32
```

`Arithmetic::lerp` on floats.

### `darken_pixel`
[src/tools/krita/composite.rs:155](../../../../src/tools/krita/composite.rs#L155)

```rust
pub fn darken_pixel(dst: u8, src: u8, p: &Darken) -> u8
```

`AlphaDarkenCompositor32::compositeOnePixelScalar`, alpha only: the stroke buffer only ever holds the brush colour, so its colour channels never change.

### `darken_batch`
[src/tools/krita/composite.rs:188](../../../../src/tools/krita/composite.rs#L188)

```rust
pub fn darken_batch(dst: &mut [u8], src: &[u8], p: &Darken)
```

`AlphaDarkenCompositor32::compositeVector` on one batch of eight, alpha only. The batch's shortcuts (nothing to paint, opaque onto opaque) are taken for the eight together, and its arithmetic is in the 0..255 domain, so it can round differently from the scalar form.

### `darken_run`
[src/tools/krita/composite.rs:238](../../../../src/tools/krita/composite.rs#L238)

```rust
pub fn darken_run(dst: &mut [u8], src: &[u8], x0: i32, p: &Darken)
```

`genericComposite32` over one run of a row: a scalar head up to the next 32-byte boundary, whole batches, a scalar tail. Krita's tile rows are taken to start on a 32-byte boundary, so the boundary falls every eight pixels of `x0`, the run's first column; runs never cross a 64-px tile. Unverifiable without Krita's allocator, and the scheduler can split a dab's rect further on a busy canvas, so this is the common case rather than a guarantee.

## Tests

### `mod tests`

Run with `cargo test tools::krita::composite::tests` · 10 tests

| Test | What it checks |
|---|---|
| [`alpha_darken_caps_at_the_dab_opacity`](../../../../src/tools/krita/composite.rs#L269) | Creamy alpha darken never lets a stroke darken itself: going over the same pixel at the same opacity stops at that opacity. |
| [`alpha_darken_follows_the_average_and_the_flow`](../../../../src/tools/krita/composite.rs#L286) | Values worked from compositeOnePixelScalar: opacity 0.6, flow 0.5, a running average of 0.8 above it. |
| [`a_batch_with_nothing_to_paint_leaves_dst_alone`](../../../../src/tools/krita/composite.rs#L298) | A batch with nothing to paint leaves dst alone. |
| [`a_run_splits_into_head_batches_and_tail`](../../../../src/tools/krita/composite.rs#L306) | A run splits into head batches and tail. |
| [`the_running_average_rises_at_once_and_falls_slowly`](../../../../src/tools/krita/composite.rs#L330) | The running average rises at once and falls slowly. |
| [`integer_multiplies_round_like_krita`](../../../../src/tools/krita/composite.rs#L337) | Integer multiplies round like krita. |
| [`over_onto_empty_copies_the_dab`](../../../../src/tools/krita/composite.rs#L347) | Over onto empty copies the dab. |
| [`faint_build_up_plateaus_below_opaque`](../../../../src/tools/krita/composite.rs#L357) | Build-up in 8 bits stalls: a faint dab stops adding anything once the pixel is dark enough that its share rounds away. Krita's light pencil passes plateau short of black for exactly this reason. |
| [`alpha_lock_keeps_alpha_and_recolours`](../../../../src/tools/krita/composite.rs#L367) | Alpha lock keeps alpha and recolours. |
| [`erase_removes_alpha`](../../../../src/tools/krita/composite.rs#L374) | Erase removes alpha. |
