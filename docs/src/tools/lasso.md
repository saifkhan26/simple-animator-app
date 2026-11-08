# `src/tools/lasso.rs` — turning a lasso outline into a coverage mask

> **Source:** [src/tools/lasso.rs](../../../src/tools/lasso.rs) · **Module:** `crate::tools::lasso` · **Size:** 372 lines · Line links as of commit `67a7cb9`

## What this file is for

Turns a closed lasso outline (a polygon of points) into a **coverage mask**: a bounding box plus one byte
per pixel saying how much of that pixel is inside (0 = outside, 255 = fully inside, in between on the
anti-aliased edge). This is the shape of a selection or a lasso fill.

It fills with the **nonzero winding** rule, so a loop that crosses itself stays solid instead of punching an
even-odd hole where it overlaps. Rendering is a scanline rasteriser with 4 sample rows per pixel and
fractional horizontal coverage, which anti-aliases the edges.

```text
polygon (points)            coverage mask (bytes)
   *-------*                 0   0   0   0   0
  /         \                0  128 255 255 128   ← partial at edges
 *           *      ──►      0  255 255 255 255
  \_________/                0  64  255 255 64
```

**Nonzero vs even-odd, in JS terms.**
```js
// winding number for point P: +1 for each edge crossing upward, −1 downward (on a ray to the right)
const inside = winding !== 0;          // nonzero (used here): a self-overlapping loop stays filled
const insideEvenOdd = crossings % 2 === 1; // even-odd: overlaps become holes
```

## How it connects

- **Used by:** [tools/selection.rs](selection.md) (selection masks), [app/select.rs](../app/select.md),
  [app/lasso_fill.rs](../app/lasso_fill.md), [app/liquify.rs](../app/liquify.md) (`Mask` for the edge).
- **Uses:** [doc/canvas.rs](../doc/canvas.md).

## Rust concepts in this file

**`?` on `Option` inside a function returning `Option`.** `let (x, y, w, h, cov) = coverage_rect(...)?;` —
returns `None` early if the polygon is degenerate. More: [Option and Result](../../rust-primer.md#option-and-result)

## Types

### `const SUB`
[src/tools/lasso.rs:22](../../../src/tools/lasso.rs#L22)

```rust
const SUB: i32 = 4;
```

Sample rows per pixel row (4): enough to hide stair-steps on nearly horizontal edges.

### `struct Mask`
[src/tools/lasso.rs:30](../../../src/tools/lasso.rs#L30)

Per-pixel coverage of a closed polygon, as a bounding box plus one byte per pixel inside it (0 = outside, 255 = fully inside).

This is the shape of a selection: `erase` weights alpha by it, a lift copies pixels through it, and a stamp composites with it.

| Field | Type | Meaning |
|---|---|---|
| `x` | `u32` | Left of the mask's box, in cell pixels. |
| `y` | `u32` | Top of the box. |
| `w` | `u32` | Box width. |
| `h` | `u32` | Box height. |
| `cov` | `Vec<u8>` | Row-major, `w * h` bytes. |

## `impl Mask`

### `Mask::at`
[src/tools/lasso.rs:41](../../../src/tools/lasso.rs#L41)

```rust
pub fn at(&self, x: u32, y: u32) -> u8
```

Coverage at a cell pixel, 0 outside the box.

### `Mask::intersect`
[src/tools/lasso.rs:50](../../../src/tools/lasso.rs#L50)

```rust
pub fn intersect(&self, other: &Mask) -> Option<Mask>
```

The overlap of two masks, each pixel the product of the two coverages (a lasso held inside a selection);
`None` if they don't meet.

### `Mask::contains`
[src/tools/lasso.rs:70](../../../src/tools/lasso.rs#L70)

```rust
pub fn contains(&self, x: i32, y: i32) -> bool
```

Whether the mask covers pixel `(x, y)` at all — the "did you press inside the selection" test.

## Free functions

### `coverage`
[src/tools/lasso.rs:85](../../../src/tools/lasso.rs#L85)

```rust
pub fn coverage(pts: &[(f32, f32)], cw: u32, ch: u32) -> Option<Mask>
```

Rasterises a closed polygon in a cell's pixel space into a `Mask`, clipped to the cell; `None` for fewer than 3
points or a path entirely off the cell.

### `coverage_rect`
[src/tools/lasso.rs:100](../../../src/tools/lasso.rs#L100)

```rust
pub fn coverage_rect(
    pts: &[(f32, f32)],
    clip: (i32, i32, i32, i32),
) -> Option<(i32, i32, u32, u32, Vec<u8>)>
```

The rasteriser behind `coverage`, clipped to any integer rectangle (which may start below zero — selections
live in document space, which has no cell edge). For each of `SUB` sample rows per pixel row it finds where
each polygon edge crosses, tracks the winding direction, and adds the inside spans (`add_span`) with
weight `1/SUB`. Returns the covered box and its bytes.

### `erase_masked`
[src/tools/lasso.rs:185](../../../src/tools/lasso.rs#L185)

```rust
pub fn erase_masked(canvas: &mut Canvas, mask: &Mask) -> bool
```

Erases through a mask: each pixel's alpha is scaled by `1 − coverage` (RGB kept), and fully erased pixels are
zeroed entirely. Returns whether anything changed.

### `add_span`
[src/tools/lasso.rs:230](../../../src/tools/lasso.rs#L230)

```rust
fn add_span(cov: &mut [f32], x0: i32, xa: f32, xb: f32, weight: f32)
```

Adds coverage for the horizontal span `[xa, xb)` into one accumulation row: full weight for pixels wholly
inside, the fractional overlap for the two end pixels — that fraction is what anti-aliases the edge.

## Tests

### `mod tests`

Run with `cargo test tools::lasso::tests` · 9 tests

| Test | What it checks |
|---|---|
| [`erases_inside_and_spares_outside`](../../../src/tools/lasso.rs#L279) | Erases inside and spares outside. |
| [`erase_zeroes_rgb_so_no_stale_colour_remains`](../../../src/tools/lasso.rs#L296) | Erase zeroes rgb so no stale colour remains. |
| [`partial_edge_coverage_is_antialiased`](../../../src/tools/lasso.rs#L304) | Partial edge coverage is antialiased. |
| [`overlapping_loop_stays_solid`](../../../src/tools/lasso.rs#L314) | Overlapping loop stays solid. |
| [`degenerate_paths_are_a_noop`](../../../src/tools/lasso.rs#L327) | Degenerate paths are a noop. |
| [`offscreen_path_is_a_noop`](../../../src/tools/lasso.rs#L334) | Offscreen path is a noop. |
| [`path_clipped_at_the_canvas_edge_does_not_wrap`](../../../src/tools/lasso.rs#L342) | Path clipped at the canvas edge does not wrap. |
| [`coverage_bounds_the_path_and_anti_aliases_its_edge`](../../../src/tools/lasso.rs#L351) | Coverage bounds the path and anti aliases its edge. |
| [`coverage_rejects_degenerate_and_offscreen_paths`](../../../src/tools/lasso.rs#L367) | Coverage rejects degenerate and offscreen paths. |

**Test helpers** (used only by the tests above):

- [`erase`](../../../src/tools/lasso.rs#L252) — The pre-selection entry point: rasterise a path and erase through it. The tool now splits these two steps so the mask can outlive the erase, but the composed behaviour is still what these tests pin down.
- [`solid`](../../../src/tools/lasso.rs#L260) — 16x16 fully opaque white.
- [`alpha`](../../../src/tools/lasso.rs#L269) — Alpha.
- [`rect`](../../../src/tools/lasso.rs#L274) — Axis-aligned rectangle path, corners on pixel boundaries.
