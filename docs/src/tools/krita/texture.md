# `src/tools/krita/texture.rs` — Krita brush textures (the Pattern option)

> **Source:** [src/tools/krita/texture.rs](../../../../src/tools/krita/texture.rs) · **Module:** `crate::tools::krita::texture` · **Size:** 206 lines · Line links as of commit `67a7cb9`

## What this file is for

A Krita brush can have a **texture** (the "Pattern" option): a tiling paper or canvas image that shapes every
dab, so strokes show paper tooth. Krita builds an 8-bit **mask** from the pattern image once
(`KisTextureMaskInfo` — scale, brightness, contrast, neutral point, invert, cutoffs), anchored to the canvas
so it doesn't move with the brush, and multiplies (or subtracts) each dab's alpha by the texel under it,
scaled by the option's strength.

**Example.** Dab pixel alpha 200, texel 128 (mid-grey), strength 1.0 in multiply mode → `200 × 128 / 255 ≈ 100`.

## How it connects

- **Used by:** [tools/krita/mod.rs](mod.md) (`PixelBrush::texture`, applied in `KritaStroke::dab`),
  [tools/noise.rs](../noise.md) (reuses Pencil-5's paper scan), [tools/fill.rs](../fill.md) (fill grain).
- **Uses:** [tools/krita/qt.rs](qt.md) (`Image32`, Qt scaling).

## Rust concepts in this file

**`rem_euclid` for tiling.** `x.rem_euclid(w)` wraps a coordinate into `0..w` even for negatives — a true modulo,
which JS's `%` isn't.

## Types

### `struct TextureSettings`
[src/tools/krita/texture.rs:14](../../../../src/tools/krita/texture.rs#L14)

The Pattern option's settings that shape the mask.

| Field | Type | Meaning |
|---|---|---|
| `scale` | `f64` | Pattern scale. |
| `brightness` | `f64` | Brightness adjustment. |
| `contrast` | `f64` | Contrast adjustment. |
| `neutral_point` | `f64` | The grey level treated as "no effect". |
| `invert` | `bool` | Invert the pattern. |
| `cutoff_left` | `i32` | Lower cutoff (values below become 0). |
| `cutoff_right` | `i32` | Upper cutoff (values above become 255). |
| `cutoff_policy` | `i32` | How cutoffs apply (Krita's policy number). |
| `offset_x` | `i32` | Pattern offset x. |
| `offset_y` | `i32` | Pattern offset y. |

### `struct TextureMask`
[src/tools/krita/texture.rs:29](../../../../src/tools/krita/texture.rs#L29)

The baked mask, one byte per texel.

| Field | Type | Meaning |
|---|---|---|
| `w` | `usize` | Mask width (one pattern tile). |
| `h` | `usize` | Mask height. |
| `data` | `Vec<u8>` | One byte per texel. |
| `subtract` | `bool` | Texturing mode 1: the texture is taken away from the dab rather than multiplied into it. |
| `offset_x` | `i32` | Where the tiling starts on the canvas (x). |
| `offset_y` | `i32` | Where it starts (y). |

## `impl TextureMask`

### `TextureMask::build`
[src/tools/krita/texture.rs:46](../../../../src/tools/krita/texture.rs#L46)

```rust
pub fn build(pattern: &Image32, s: &TextureSettings) -> Self
```

`KisTextureMaskInfo::recalculateMask`, for an opaque pattern (the alpha8 mask; patterns with alpha only matter to the lightness and gradient modes).

`pattern` is the pattern image as `Format_ARGB32`.

### `TextureMask::at`
[src/tools/krita/texture.rs:111](../../../../src/tools/krita/texture.rs#L111)

```rust
pub(crate) fn at(&self, x: i32, y: i32) -> u8
```

The texel under canvas pixel `(x, y)`: `KisFillPainter::fillRect`'s pattern tiling, which wraps with a true modulo either side of zero.

### `TextureMask::apply`
[src/tools/krita/texture.rs:121](../../../../src/tools/krita/texture.rs#L121)

```rust
pub fn apply(&self, alpha: &mut [u8], w: usize, h: usize, top_left: (i32, i32), strength: f64)
```

`KisTextureOption::apply`: in multiply mode every dab pixel's alpha times the texel under it times the strength; in subtract mode (`KisMaskingBrushCompositeOp` SUBTRACT with strength) the texel and the strength's inverse taken off the alpha.

## Tests

### `mod tests`

Run with `cargo test tools::krita::texture::tests` · 4 tests

| Test | What it checks |
|---|---|
| [`mask_is_the_patterns_gray`](../../../../src/tools/krita/texture.rs#L160) | Mask is the patterns gray. |
| [`scaling_sizes_the_mask_like_krita`](../../../../src/tools/krita/texture.rs#L173) | Scaling sizes the mask like krita. |
| [`texture_is_anchored_to_the_canvas_either_side_of_zero`](../../../../src/tools/krita/texture.rs#L185) | Texture is anchored to the canvas either side of zero. |
| [`apply_multiplies_by_texel_and_strength`](../../../../src/tools/krita/texture.rs#L195) | Apply multiplies by texel and strength. |

**Test helpers** (used only by the tests above):

- [`settings`](../../../../src/tools/krita/texture.rs#L144) — Settings.
