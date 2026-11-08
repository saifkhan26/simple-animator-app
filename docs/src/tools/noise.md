# `src/tools/noise.rs` — midtone noise for app brushes

> **Source:** [src/tools/noise.rs](../../../src/tools/noise.rs) · **Module:** `crate::tools::noise` · **Size:** 395 lines · Line links as of commit `67a7cb9`

## What this file is for

**Midtone noise** breaks a stroke's soft, half-transparent falloff into specks, while leaving its solid core
solid and the bare paper bare — an airbrush or soft pencil with "tooth". It differs from paper grain
([paper.rs](paper.md)), which multiplies the whole stroke and thins its core too: here the noise averages
out to nothing and is strongest where coverage is around 50%.

```text
coverage c:   0 ──── 0.25 ──── 0.5 ──── 0.75 ──── 1
noise effect: none   some      most     some      none      (envelope ∝ min(c, 1 − c))
```

The noise comes from a periodic field — either fine per-pixel randomness, or Krita's paper scan (the one
Pencil-5 Tilted uses) — **equalised** so a fraction `t` of its values lies below `t`. With *stipple* on,
each pixel is either fully inked or bare, the density following coverage — pure dots.

## How it connects

- **Used by:** [tools/ribbon.rs](ribbon.md) (the stroke compositor shapes coverage through `Noise`),
  [tools/mod.rs](mod.md) (`BrushSettings::noise*`), [ui/shell.rs](../ui/shell.md) (Brush noise controls).
- **Uses:** [tools/krita/texture.rs](krita/texture.md) (`TextureMask`, the paper scan),
  [tools/krita/mod.rs](krita/mod.md) (Pencil-5 preset).

## Rust concepts in this file

**Lifetimes on iterator-like structs.** `NoiseRow<'a>` borrows the `Noise` it walks (`noise: &'a Noise`), so
it can't outlive it. More: [Lifetimes](../../rust-primer.md#lifetimes)

**`Arc<Vec<f32>>` shared fields.** Baked fields are cached and shared between brushes without copying.

## Types

### `const NOISE_SIZES`
[src/tools/noise.rs:27](../../../src/tools/noise.rs#L27)

```rust
pub const NOISE_SIZES: RangeInclusive<f32> = 1.0..=4.0;
```

The noise sizes offered in the UI.

### `enum NoiseSource`
[src/tools/noise.rs:31](../../../src/tools/noise.rs#L31)

Where a brush's noise comes from.

| Variant | Meaning |
|---|---|
| `Fine` | Random per pixel at size 1, softly clumped above it. |
| `Paper` | Krita's paper scan, the one Pencil-5 Tilted draws on: the specks follow the paper's tooth. Size 1 is Pencil-5's own scale. |

### `struct Noise`
[src/tools/noise.rs:42](../../../src/tools/noise.rs#L42)

A brush's noise, baked and ready for its stroke.

| Field | Type | Meaning |
|---|---|---|
| `amount` | `f32` | Strength 0–1. |
| `stipple` | `bool` | Pure dots: each pixel fully inked or bare. |
| `field` | `Periodic` | The equalised noise field at the chosen size. |

### `struct NoiseRow`
[src/tools/noise.rs:90](../../../src/tools/noise.rs#L90)

A walk along one row of a `Noise`, a pixel a step.

| Field | Type | Meaning |
|---|---|---|
| `noise` | `&'a Noise` | The noise being walked. |
| `walk` | `PeriodicRow<'a>` | The position along the field's row. |

### `struct Periodic`
[src/tools/noise.rs:112](../../../src/tools/noise.rs#L112)

One period of a field, row-major, tiling the canvas.

| Field | Type | Meaning |
|---|---|---|
| `period` | `(usize, usize)` | Width and height of one tile of the field. |
| `values` | `Arc<Vec<f32>>` | The tile's values, row by row (shared). |

### `struct PeriodicRow`
[src/tools/noise.rs:167](../../../src/tools/noise.rs#L167)

A walk along one row of a `Periodic`, a pixel a step, wrapping at the period: no division per pixel.

| Field | Type | Meaning |
|---|---|---|
| `row` | `&'a [f32]` | The current row of the field. |
| `i` | `usize` | Current index within it (wraps at the period). |

## `impl Noise`

### `Noise::for_brush`
[src/tools/noise.rs:52](../../../src/tools/noise.rs#L52)

```rust
pub fn for_brush(brush: &BrushSettings) -> Option<Self>
```

The baked noise for a brush, or `None` if the brush has none (Krita brushes never do).

### `Noise::row`
[src/tools/noise.rs:65](../../../src/tools/noise.rs#L65)

```rust
pub fn row(&self, x: u32, y: u32) -> NoiseRow<'_>
```

A walker along row `y` starting at column `x` — the rasteriser steps it one pixel at a time.

### `Noise::shape`
[src/tools/noise.rs:75](../../../src/tools/noise.rs#L75)

```rust
pub fn shape(&self, c: f32, n: f32) -> f32
```

Coverage `c` with noise `n` mixed in: 0 and 1 unchanged; in between pushed up or down by `amount × envelope`
(or, with stipple, inked if `n < c`).

## `impl NoiseRow<'_>`

### `NoiseRow::skip`
[src/tools/noise.rs:98](../../../src/tools/noise.rs#L98)

```rust
pub fn skip(&mut self)
```

Steps over a pixel that takes no ink.

### `NoiseRow::shape`
[src/tools/noise.rs:104](../../../src/tools/noise.rs#L104)

```rust
pub fn shape(&mut self, c: f32) -> f32
```

Applies the noise to this pixel's coverage, then moves to the next pixel.

## `impl Periodic`

### `Periodic::row`
[src/tools/noise.rs:120](../../../src/tools/noise.rs#L120)

```rust
pub fn row(&self, x: i32, y: i32, offset: (i32, i32)) -> PeriodicRow<'_>
```

A walker over row `y` from column `x`, with the field's origin at `offset`.

### `Periodic::at`
[src/tools/noise.rs:130](../../../src/tools/noise.rs#L130)

```rust
fn at(&self, x: i32, y: i32) -> f32
```

The field value at `(x, y)`, wrapping both coordinates into the period.

### `Periodic::scaled`
[src/tools/noise.rs:139](../../../src/tools/noise.rs#L139)

```rust
pub fn scaled(&self, size: f32) -> Self
```

The field resized by `size` (bilinear), over a whole number of pixels close to the exact period so it still
tiles seamlessly.

## `impl PeriodicRow<'_>`

### `PeriodicRow::next`
[src/tools/noise.rs:174](../../../src/tools/noise.rs#L174)

```rust
pub fn next(&mut self) -> f32
```

The next value along the row, wrapping at the period without a division per pixel.

## Free functions

### `paper_scan`
[src/tools/noise.rs:186](../../../src/tools/noise.rs#L186)

```rust
pub(crate) fn paper_scan() -> &'static TextureMask
```

Krita's `04_paper-C-grain.png` texture at Pencil-5 Tilted's scale, taken from the preset the app carries.

### `field`
[src/tools/noise.rs:200](../../../src/tools/noise.rs#L200)

```rust
fn field(source: NoiseSource, size: f32) -> Periodic
```

The equalised field for a source and size, from a small cache of recently used ones (so the pen and eraser
at different sizes don't rebuild by turns).

### `fine`
[src/tools/noise.rs:230](../../../src/tools/noise.rs#L230)

```rust
fn fine() -> &'static Periodic
```

Independent random values, one per pixel, over a 256-pixel tile.

### `paper`
[src/tools/noise.rs:252](../../../src/tools/noise.rs#L252)

```rust
fn paper() -> &'static Periodic
```

The paper scan as a field, equalised over 0–1.

### `equalize`
[src/tools/noise.rs:271](../../../src/tools/noise.rs#L271)

```rust
fn equalize(values: &mut [f32])
```

Spreads values evenly over 0–1 keeping their order (sort, then assign each its rank / count), so a fraction
`t` of them falls below `t`. Then stipple density follows coverage exactly and midtone noise averages to
zero, whatever field it came from.

## Tests

### `mod tests`

Run with `cargo test tools::noise::tests` · 6 tests

| Test | What it checks |
|---|---|
| [`bare_paper_and_a_solid_core_stay_clean`](../../../src/tools/noise.rs#L306) | Bare paper and a solid core stay clean. |
| [`noise_keeps_the_strokes_density`](../../../src/tools/noise.rs#L320) | Over the noise's even spread, a stroke keeps its density: only the texture of its soft part changes, never how dark it is overall. |
| [`noise_is_strongest_in_the_midtones`](../../../src/tools/noise.rs#L332) | Noise is strongest in the midtones. |
| [`every_field_is_spread_evenly_and_tiles`](../../../src/tools/noise.rs#L344) | Every field is spread evenly and tiles. |
| [`a_bigger_size_clumps_the_specks`](../../../src/tools/noise.rs#L370) | A bigger size clumps the specks. |
| [`brushes_without_noise_and_krita_brushes_get_none`](../../../src/tools/noise.rs#L381) | Brushes without noise and krita brushes get none. |

**Test helpers** (used only by the tests above):

- [`noise`](../../../src/tools/noise.rs#L297) — Noise.
