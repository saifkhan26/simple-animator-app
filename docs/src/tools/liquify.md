# `src/tools/liquify.rs` — Liquify: warp field and resampling

> **Source:** [src/tools/liquify.rs](../../../src/tools/liquify.rs) · **Module:** `crate::tools::liquify` · **Size:** 760 lines · Line links as of commit `1fa5845`

## What this file is for

The maths behind the **Liquify** tool (Photoshop's "Forward Warp"). Pixels are never pushed directly.
Instead the brush edits a **displacement field** `D`: a grid (every `STEP` pixels) where each point says
*where in the original drawing to read from*. The drawing on screen is then **resampled**: pixel `p` shows
the original at `p + D(p)`, interpolated bilinearly.

```text
original            field D (arrows: where to read)        result
  ┃                   → → ·                                   ╲
  ┃          +        → → ·                       =            ┃   (line pushed right near the dab)
  ┃                   · · ·                                    ┃
```

**Example.** A push dab moving the pen right by 10 px at full strength sets `D ≈ (−10, 0)` at the brush
centre: the pixel at x = 50 now reads the original at x = 40, so content appears shifted right by 10. Toward
the brush rim the falloff fades `D` smoothly to 0.

Pushes are *composed* with the existing warp (not simply added), so repeated dabs follow the already-moved
image. Smooth dabs ease each grid point toward the average of its neighbours. Selections limit the warp
through per-point edge weights that fade over a falloff width, so the warp never tears at the selection edge.
Resampling is always from the **original** drawing, so going back and forth never blurs it.

## How it connects

- **Used by:** [app/liquify.rs](../app/liquify.md) (the session on `AppState`), [ui/shell.rs](../ui/shell.md)
  (mesh display reads the field).
- **Uses:** [tools/lasso.rs](lasso.md) (`Mask`), [doc/canvas.rs](../doc/canvas.md) (`DirtyRect`).

## Rust concepts in this file

**Parallel rows with scoped threads.** `each_row` splits a big window's rows across CPU cores
(`std::thread::scope`), requiring the per-row closure to be `Sync` (safe to share between threads). A
1000-pixel brush's dab touches about a quarter of a million points. More:
[Threads and channels](../../rust-primer.md#threads-and-channels)

**Generic function with a closure parameter and `where` bound.**
```rust
pub fn resample<F>(orig: &[u8], …, src: &F) where F: Fn(f32, f32) -> Option<(f32, f32)> + Sync
```
`src` says, for each pixel, where to read in the original (or `None` = unchanged). More:
[Generics](../../rust-primer.md#generics)

## Types

### `const STEP`
[src/tools/liquify.rs:19](../../../src/tools/liquify.rs#L19)

```rust
pub const STEP: u32 = 4;
```

Grid spacing of the warp field, in pixels.

### `const MIN_RADIUS`
[src/tools/liquify.rs:21](../../../src/tools/liquify.rs#L21)

```rust
pub const MIN_RADIUS: f32 = 8.0;
```

The smallest warp brush: two grid steps either side of its centre.

### `enum LiquifyMode`
[src/tools/liquify.rs:25](../../../src/tools/liquify.rs#L25)

What a liquify stroke does to the field.

| Variant | Meaning |
|---|---|
| `Push` | Forward warp: drag the pixels along with the pen. |
| `Smooth` | Ease kinks out of the warp so it flows evenly. |

### `struct LiquifyField`
[src/tools/liquify.rs:36](../../../src/tools/liquify.rs#L36)

Where each point of a drawing reads from in the original: pixel `p` shows the original at `p + D(p)`.

| Field | Type | Meaning |
|---|---|---|
| `ox` | `i32` | Cell pixel of grid point (0, 0). |
| `oy` | `i32` | Cell pixel row of grid point (0, 0). |
| `nw` | `usize` | Grid points across and down. |
| `nh` | `usize` | Grid points down (see `nw`). |
| `d` | `Vec<[f32; 2]>` | The displacement `[dx, dy]` at each grid point, row by row. |

## `impl LiquifyField`

### `LiquifyField::new`
[src/tools/liquify.rs:48](../../../src/tools/liquify.rs#L48)

```rust
pub fn new(r: DirtyRect) -> Self
```

A zero (no-warp) field covering the cell pixels of `r`.

### `LiquifyField::point`
[src/tools/liquify.rs:61](../../../src/tools/liquify.rs#L61)

```rust
pub fn point(&self, i: usize, j: usize) -> (f32, f32)
```

The cell position of grid point `(i, j)`.

### `LiquifyField::at`
[src/tools/liquify.rs:70](../../../src/tools/liquify.rs#L70)

```rust
pub fn at(&self, x: f32, y: f32) -> [f32; 2]
```

The displacement at any cell point, interpolated between grid points; zero off the grid and exactly zero where
the brush never reached.

### `LiquifyField::window`
[src/tools/liquify.rs:93](../../../src/tools/liquify.rs#L93)

```rust
pub fn window(&self, cx: f32, cy: f32, reach: f32) -> Option<(usize, usize, usize, usize)>
```

The grid index ranges within `reach` pixels of `(cx, cy)` — everything a dab there can write.

### `LiquifyField::snapshot`
[src/tools/liquify.rs:105](../../../src/tools/liquify.rs#L105)

```rust
fn snapshot(&self, (i0, i1, j0, j1): (usize, usize, usize, usize), grow: usize) -> LiquifyField
```

A copy of part of the field (grown by `grow` points), so a dab can read old values while writing new ones.

### `LiquifyField::pixels_of`
[src/tools/liquify.rs:125](../../../src/tools/liquify.rs#L125)

```rust
fn pixels_of(&self, (i0, i1, j0, j1): (usize, usize, usize, usize)) -> DirtyRect
```

The cell pixels a change to those grid points can affect (one grid step beyond them), for redraw.

### `LiquifyField::push`
[src/tools/liquify.rs:147](../../../src/tools/liquify.rs#L147)

```rust
pub fn push(&mut self, (cx, cy): (f32, f32), v: (f32, f32), radius: f32, strength: f32, edge: &[f32]) -> Option<DirtyRect>
```

One push dab: points under the brush move by `v` (full at the centre, fading to 0 at the rim), scaled by
`strength` and each point's edge weight, composed with the existing warp. Returns the affected pixels.

### `LiquifyField::each_row`
[src/tools/liquify.rs:173](../../../src/tools/liquify.rs#L173)

```rust
fn each_row(&mut self, (i0, i1, j0, j1): (usize, usize, usize, usize), f: impl Fn(usize, &mut [[f32; 2]]) + Sync)
```

Runs `f` on every grid row of a window, on several threads when the window is big enough to be worth it.

### `LiquifyField::smooth`
[src/tools/liquify.rs:204](../../../src/tools/liquify.rs#L204)

```rust
pub fn smooth(&mut self, (cx, cy): (f32, f32), radius: f32, strength: f32, edge: &[f32]) -> Option<DirtyRect>
```

One smoothing dab: each point under the brush eases toward the average of its four neighbours.

## Free functions

### `falloff`
[src/tools/liquify.rs:235](../../../src/tools/liquify.rs#L235)

```rust
fn falloff(dx: f32, dy: f32, radius: f32) -> f32
```

The brush weight at an offset from its centre: 1 at the centre, smoothly to 0 at the rim (no kink in slope).

### `edge_weights`
[src/tools/liquify.rs:247](../../../src/tools/liquify.rs#L247)

```rust
pub fn edge_weights(field: &LiquifyField, clip: Option<&Mask>, width: f32) -> Vec<f32>
```

How far each grid point may move: 1 with no selection; inside one, the selection coverage eased to 0 over
`width` pixels in from its edge.

### `resample`
[src/tools/liquify.rs:329](../../../src/tools/liquify.rs#L329)

```rust
pub fn resample<F>(
    orig: &[u8],
    cw: u32,
    ch: u32,
    rect: DirtyRect,
    out: &mut [u8],
    clip: Option<&Mask>,
    alpha_lock: bool,
    src: &F,
) where
    F: Fn(f32, f32) -> Option<(f32, f32)> + Sync,
```

Writes `rect` of the output from the original through `src` (where each pixel centre reads from), sampling
bilinearly on premultiplied colour so soft edges don't darken, respecting the selection and alpha lock.

### `resample_field`
[src/tools/liquify.rs:373](../../../src/tools/liquify.rs#L373)

```rust
pub fn resample_field(
    orig: &[u8],
    cw: u32,
    ch: u32,
    rect: DirtyRect,
    out: &mut [u8],
    clip: Option<&Mask>,
    alpha_lock: bool,
    field: &LiquifyField,
    dabs: Option<(&[(f32, f32)], f32)>,
)
```

`resample` specialised to a `LiquifyField`, much faster: each row interpolates the grid down to that row once,
then each pixel blends just two values; with `dabs`, only the area those dabs reached is redone.

### `put`
[src/tools/liquify.rs:455](../../../src/tools/liquify.rs#L455)

```rust
fn put(orig: &[u8], cw: u32, ch: u32, row: &mut [u8], x: u32, y: u32, clip: Option<&Mask>, alpha_lock: bool, to: Option<(f32, f32)>)
```

Writes one output pixel: the original where nothing moved it, otherwise the original sampled at `to`, limited
by selection coverage and alpha lock.

### `sample`
[src/tools/liquify.rs:487](../../../src/tools/liquify.rs#L487)

```rust
fn sample(px: &[u8], w: u32, h: u32, x: f32, y: f32) -> [u8; 4]
```

Bilinear RGBA at `(x, y)` — premultiplied for blending, straightened after; clear off the cell. Shortcuts the
common cases (all four pixels clear, or all identical) without arithmetic.

## Tests

### `mod tests`

Run with `cargo test tools::liquify::tests` · 10 tests

| Test | What it checks |
|---|---|
| [`a_zero_field_is_identity`](../../../src/tools/liquify.rs#L594) | A zero field is identity. |
| [`a_push_moves_a_dot_along_the_push`](../../../src/tools/liquify.rs#L601) | A push moves a dot along the push. |
| [`two_pushes_compose_like_one_twice_as_far`](../../../src/tools/liquify.rs#L612) | Two pushes of 3 go as far as one of 6 — the warp composes. |
| [`pixels_outside_the_selection_are_untouched`](../../../src/tools/liquify.rs#L626) | Pixels outside the selection are untouched. |
| [`the_edge_falloff_is_continuous`](../../../src/tools/liquify.rs#L652) | The weight eases in from the edge: no jump between neighbours that a push could tear along. |
| [`smooth_reduces_field_variance`](../../../src/tools/liquify.rs#L675) | Smooth reduces field variance. |
| [`pressure_scales_strength`](../../../src/tools/liquify.rs#L691) | Pressure scales strength. |
| [`alpha_lock_keeps_alpha`](../../../src/tools/liquify.rs#L704) | Alpha lock keeps alpha. |
| [`back_and_forth_never_compounds_blur`](../../../src/tools/liquify.rs#L719) | Back and forth ten times reads the original once each time, so it is no softer than going once. |
| [`perf_400px_dab_on_4k`](../../../src/tools/liquify.rs#L738) | Timings for one 400 px dab on a 4K drawing: the field update and the resample it causes. `cargo test --release perf_400px -- --ignored --nocapture`. |

**Test helpers** (used only by the tests above):

- [`whole`](../../../src/tools/liquify.rs#L548) — Whole.
- [`dot`](../../../src/tools/liquify.rs#L558) — A clear `W`×`H` cell with an opaque 4×4 dot at `(x, y)`.
- [`through`](../../../src/tools/liquify.rs#L569) — Through.
- [`warp`](../../../src/tools/liquify.rs#L576) — Warp.
- [`centre_x`](../../../src/tools/liquify.rs#L583) — Centroid of alpha, x.

**Test-only types and constants:** [`W`](../../../src/tools/liquify.rs#L545), [`H`](../../../src/tools/liquify.rs#L546)
