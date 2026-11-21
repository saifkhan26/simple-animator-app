# `src/tools/recolor.rs` — swapping one colour for another

> **Source:** [src/tools/recolor.rs](../../../src/tools/recolor.rs) · **Module:** `crate::tools::recolor` · **Size:** 347 lines · Line links as of commit `c09072f`

## What this file is for

The pixel work behind **Replace color**: one colour swapped for another on a single drawing.
[`recolored`](#recolored) takes a canvas and a [`Recolor`](#struct-recolor) (old colour, new colour,
tolerance) and hands back a new canvas, or `None` when nothing would change.

- **Which pixels:** a pixel is the old colour when its red, green and blue are each within the tolerance
  of it. Alpha doesn't count. Pixels are stored unmultiplied, so a line's soft edge and a pencil's grain
  hold the line's own colour and go over whole.
- **What they become:** the new colour plus the pixel's offset from the old one, with its alpha kept,
  so grain and noise inside the tolerance survive.
- **Mixed edges:** where the old colour meets another on the same drawing (a fill against its lines),
  the pixels between are a blend of the two. For a painted pixel within 2 px of the old colour,
  [`mixed`](#mixed) checks whether it lies on the straight line from a nearby colour to the old one.
  If so, it moves toward the new colour by as much of the old as it holds, so no fringe is left.
- **Clip:** an optional [`Mask`](lasso.md) in the canvas's own pixels holds the change to a selection,
  by coverage.

It reads and writes only the pixels the canvas holds ([`Canvas::held`](../doc/canvas.md)), so a packed
drawing costs its drawn area, not the whole canvas, and stays packed. Rows run on several threads
through `par_rows`.

Pure: pixels in, pixels out. Which drawings, and the undo step, are [app/recolor.rs](../app/recolor.md).

## How it connects

- **Used by:** [app/recolor.rs](../app/recolor.md) (`replace_color`).
- **Uses:** [doc/canvas.rs](../doc/canvas.md) (`Canvas::held`, `Canvas::with_held`, `par_rows`),
  [tools/lasso.rs](lasso.md) (`Mask`).

## Types

### `const MAX_TOLERANCE`
[src/tools/recolor.rs:24](../../../src/tools/recolor.rs#L24)

```rust
pub const MAX_TOLERANCE: u8 = 128;
```

Upper bound on the tolerance, as for the bucket and the wand.

### `const REACH`
[src/tools/recolor.rs:28](../../../src/tools/recolor.rs#L28)

```rust
const REACH: usize = 2;
```

How far from the old colour a mixed pixel is looked for, and how far from a mixed pixel the colour it is mixed with.

### `const MIN_MIX`
[src/tools/recolor.rs:32](../../../src/tools/recolor.rs#L32)

```rust
const MIN_MIX: f32 = 0.08;
```

Less of the old colour than this in a pixel is no mix: it is the other colour, give or take its own noise.

### `struct Recolor`
[src/tools/recolor.rs:35](../../../src/tools/recolor.rs#L35)

One colour replace: what to swap for what, and how near counts.

| Field | Type | Meaning |
|---|---|---|
| `from` | `[u8; 3]` | The colour to replace, as RGB. |
| `to` | `[u8; 3]` | The colour to put in its place. |
| `tolerance` | `u8` | 0..=`MAX_TOLERANCE` per channel. |

## `impl Recolor`

### `Recolor::is_old`
[src/tools/recolor.rs:44](../../../src/tools/recolor.rs#L44)

```rust
fn is_old(&self, p: &[u8]) -> bool
```

Whether `p`, one RGBA pixel, is painted in the old colour.

### `Recolor::slack`
[src/tools/recolor.rs:51](../../../src/tools/recolor.rs#L51)

```rust
fn slack(&self) -> f32
```

How far off the line from a colour to the old one a pixel may be and still count as a mix of the two: the old colour's own spread, within reason.

## Free functions

### `recolored`
[src/tools/recolor.rs:58](../../../src/tools/recolor.rs#L58)

```rust
pub fn recolored(canvas: &Canvas, r: &Recolor, clip: Option<&Mask>) -> Option<Canvas>
```

`canvas` with the colour replaced, or `None` when no pixel changed. Held to `clip` — in the canvas's own pixels — by its coverage, when given.

### `near_old`
[src/tools/recolor.rs:128](../../../src/tools/recolor.rs#L128)

```rust
fn near_old(old: &[bool], w: usize, h: usize) -> Vec<bool>
```

Each pixel with one of `old` within `REACH` of it, across and down: the only ones that can be a mix. A run along each row, then a look up and down the column, rather than the whole square at every pixel.

### `mixed`
[src/tools/recolor.rs:163](../../../src/tools/recolor.rs#L163)

```rust
fn mixed(src: &[u8], old: &[bool], w: usize, h: usize, x: usize, y: usize, r: &Recolor) -> f32
```

How much of the old colour the pixel at `(x, y)` holds when it is a mix of the old colour and one beside it; 0 when it isn't one. The colour it is mixed with is taken as the farthest from the old one that it lies on the way from — the core of a line, not another pixel of its soft edge.

## Tests

### `mod tests`

Run with `cargo test tools::recolor::tests` · 9 tests

| Test | What it checks |
|---|---|
| [`the_old_colour_goes_over_whole_keeping_its_alpha`](../../../src/tools/recolor.rs#L216) | The old colour goes over whole keeping its alpha. |
| [`tolerance_takes_near_colours_and_keeps_their_offset`](../../../src/tools/recolor.rs#L227) | Tolerance takes near colours and keeps their offset. |
| [`a_mix_with_another_colour_moves_by_as_much_of_the_old_as_it_holds`](../../../src/tools/recolor.rs#L238) | A mix with another colour moves by as much of the old as it holds. |
| [`a_colour_beside_the_old_one_but_off_the_way_to_it_is_left`](../../../src/tools/recolor.rs#L261) | A colour beside the old one but off the way to it is left. |
| [`a_clip_holds_it_by_coverage`](../../../src/tools/recolor.rs#L270) | A clip holds it by coverage. |
| [`near_marks_what_lies_within_reach_of_the_old_colour`](../../../src/tools/recolor.rs#L280) | Near marks what lies within reach of the old colour. |
| [`nothing_to_change_gives_nothing`](../../../src/tools/recolor.rs#L299) | Nothing to change gives nothing. |
| [`perf_recolor`](../../../src/tools/recolor.rs#L311) | Time on a 1080p drawing, half of it the old colour and its edge against black. `cargo test perf_recolor -- --ignored --nocapture` |
| [`a_packed_drawing_stays_packed_over_the_same_pixels`](../../../src/tools/recolor.rs#L330) | A packed drawing stays packed over the same pixels. |

**Test helpers** (used only by the tests above):

- [`swap`](../../../src/tools/recolor.rs#L202) — Swap.
- [`strip`](../../../src/tools/recolor.rs#L207) — A `w`×1 strip of these pixels.
- [`rgba`](../../../src/tools/recolor.rs#L211) — Rgba.

**Test-only types and constants:** [`RED`](../../../src/tools/recolor.rs#L199), [`BLUE`](../../../src/tools/recolor.rs#L200)
