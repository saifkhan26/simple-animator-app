# `src/tools/wand.rs` — magic select (the wand)

> **Source:** [src/tools/wand.rs](../../../src/tools/wand.rs) · **Module:** `crate::tools::wand` · **Size:** 773 lines · Line links as of commit `1fa5845`

## What this file is for

The **magic select** (wand) is one of the Lasso tool's shapes: a click selects the connected run of pixels
that look like the one clicked — just like the bucket fill floods, but making a selection instead of
painting. Options: tolerance, gap closing, expand, "any paint" (take every painted pixel touching the click,
whatever its colour), non-contiguous (every matching pixel), and reading all visible layers.

A press can be **dragged to retune** tolerance or gap live. To make that cheap on big canvases, the press reads
the pixels once into a **reach map**: for each pixel, the smallest tolerance that would let it in. After that a
new tolerance is just one compare per pixel plus a flood (`scan::flood`), not a full colour comparison.

```text
pixels' colour distance from the clicked one     reach map      tolerance 20 → open (≤20)
   0   5  12  80                                 0  5 12 80        ✓ ✓ ✓ ✗
   3   9  40  90                         ──►     3  9 40 90  ──►   ✓ ✓ ✗ ✗   then flood from the click
```

## How it connects

- **Used by:** [app/select.rs](../app/select.md) (`WandDrag`, press/drag/release with the wand shape),
  [ui/shell.rs](../ui/shell.md) (wand options).
- **Uses:** [tools/scan.rs](scan.md) (`flood`, `SHUT`/`OPEN`/`TAKEN`), [tools/fill.rs](fill.md) (gap-closing
  distance field `Field`, tolerance ladder, the same paint/bare rules as the bucket).

## Rust concepts in this file

**Caching inside a struct with `Option` fields.** `WandPick` keeps `level: Option<Level>` and
`field: Option<(u8, Field)>` — computed on first use, reused when the next `pick` asks for the same cut, rebuilt
otherwise. A common Rust memoisation pattern.

**Closures as parameters with `impl Fn`.** `cover(bbox, |x, y| …)` builds coverage from any rule; `+ Copy` on a
closure bound lets it be used more than once by value.

## Types

### `const PAINT`
[src/tools/wand.rs:23](../../../src/tools/wand.rs#L23)

```rust
const PAINT: u8 = 8;
```

The alpha at or above which a pixel counts as "paint" for any-paint mode (the same line the bucket uses).

### `const MAX_TOLERANCE`
[src/tools/wand.rs:26](../../../src/tools/wand.rs#L26)

```rust
pub const MAX_TOLERANCE: u8 = 128;
```

The highest tolerance (matches the bucket's slider).

### `const TOLERANCE_LADDER`
[src/tools/wand.rs:30](../../../src/tools/wand.rs#L30)

```rust
const TOLERANCE_LADDER: [u8; 23] = [
    0, 1, 2, 3, 4, 6, 8, 10, 12, 14, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128,
];
```

The tolerances a press-drag steps through: every value while small, wider steps as they grow, so the whole
range fits a comfortable pen movement.

### `struct WandOptions`
[src/tools/wand.rs:43](../../../src/tools/wand.rs#L43)

The wand's settings (workspace preferences).

| Field | Type | Meaning |
|---|---|---|
| `tolerance` | `u8` | 0..=`MAX_TOLERANCE` per channel, matched as the bucket matches. |
| `gap` | `u8` | Treat breaks in the lines up to this many pixels wide as shut. 0 = off. |
| `expand` | `u8` | Grow each pick by this many pixels, so a flat colour's selection reaches under its anti-aliased lines. 0 = off. |
| `any_paint` | `bool` | A press on paint takes every painted pixel touching it, whatever its colour. A press on bare canvas picks as it always does. |
| `contiguous` | `bool` | Off: every matching pixel on the drawing, touching or not. |
| `all_visible` | `bool` | Read every visible layer at once instead of the active drawing. |

### `struct Picked`
[src/tools/wand.rs:76](../../../src/tools/wand.rs#L76)

What a pick covers: a box in the pixels' own space, and a byte per pixel in it, 0 or 255.

| Field | Type | Meaning |
|---|---|---|
| `x` | `i32` | Left of the pick's box. |
| `y` | `i32` | Top of the box. |
| `w` | `u32` | Box width. |
| `h` | `u32` | Box height. |
| `cov` | `Vec<u8>` | One byte per pixel in the box: 0 or 255. |

### `struct Level`
[src/tools/wand.rs:88](../../../src/tools/wand.rs#L88)

The pick at one cut: a byte per pixel, `SHUT` or `OPEN`, and once flooded, `TAKEN` for the run joined to the press — a `scan::flood` map — and `EDGE` for the soft edges a press on bare canvas takes with it.

| Field | Type | Meaning |
|---|---|---|
| `cut` | `u8` | The most reach let in. |
| `map` | `Vec<u8>` | Per pixel: `SHUT`/`OPEN`, then `TAKEN` after flooding, and `EDGE` for soft edges taken on bare canvas. |
| `flood` | `Option<Bbox>` | The flood's bounds, once it has run. |
| `edged` | `Option<Bbox>` | On bare canvas, the bounds of the flood and its soft edges, once those are marked. |
| `taken` | `Option<Vec<bool>>` | The flood as `bool`s, for the gap and the expand, once asked for. |

### `const EDGE`
[src/tools/wand.rs:103](../../../src/tools/wand.rs#L103)

```rust
const EDGE: u8 = TAKEN + 1;
```

Map value: the soft edge of a line beside a pick on bare canvas, taken along with it as a bucket fill would.

### `struct WandPick`
[src/tools/wand.rs:111](../../../src/tools/wand.rs#L111)

One press of the wand, holding the reach map so repeated `pick`s (as a drag retunes it) are cheap.

| Field | Type | Meaning |
|---|---|---|
| `w` | `i32` | Pixel width. |
| `h` | `i32` | Pixel height. |
| `seed` | `(i32, i32)` | The pressed pixel. |
| `reach` | `Vec<u8>` | Per pixel: the smallest tolerance that lets it in, `base` aside. 255 is never. On bare canvas that is how covered the pixel is. |
| `bare` | `bool` | Pressed on bare canvas. |
| `base` | `u8` | How covered the pixel pressed on is, on bare canvas: a tolerance lets in that much more. 0 otherwise. |
| `level` | `Option<Level>` | The cached flood at the last cut. |
| `field` | `Option<(u8, Field)>` | Distance to the walls, for the cut it was built at. |

## Free functions

### `step_tolerance`
[src/tools/wand.rs:36](../../../src/tools/wand.rs#L36)

```rust
pub fn step_tolerance(base: u8, steps: i32) -> u8
```

The tolerance `steps` rungs up or down the ladder from `base`.

## `impl Default for WandOptions`

### `WandOptions::default`
[src/tools/wand.rs:61](../../../src/tools/wand.rs#L61)

```rust
fn default() -> Self
```

Defaults: a moderate tolerance, no gap, no expand, contiguous, active drawing only.

## `impl WandPick`

### `WandPick::new`
[src/tools/wand.rs:131](../../../src/tools/wand.rs#L131)

```rust
pub fn new(src: &[u8], size: (u32, u32), seed: (i32, i32), any_paint: bool) -> Option<Self>
```

Builds the press at `seed` on RGBA pixels `src`: computes the reach map (each pixel's colour distance from the
pressed one, or how covered it is when pressing bare canvas, or paint/no-paint in any-paint mode). `None` if
the press is off the pixels.

### `WandPick::pick`
[src/tools/wand.rs:173](../../../src/tools/wand.rs#L173)

```rust
pub fn pick(&mut self, o: &WandOptions) -> Picked
```

What options `o` select: opens every pixel whose reach is within the tolerance, floods from the seed (unless
non-contiguous), closes gaps with the distance field if `gap` is set, takes soft edges on bare canvas, grows by
`expand`, and returns the coverage. Always includes the pressed pixel.

## Free functions

### `bounds`
[src/tools/wand.rs:283](../../../src/tools/wand.rs#L283)

```rust
fn bounds(map: &[u8], w: usize, keep: impl Fn(u8) -> bool + Copy) -> Option<Bbox>
```

The bounding box of the map pixels `keep` accepts.

### `cover_map`
[src/tools/wand.rs:299](../../../src/tools/wand.rs#L299)

```rust
fn cover_map(map: &[u8], w: i32, bbox: Bbox, keep: impl Fn(u8) -> bool) -> Picked
```

A `Picked` over `bbox` with 255 where `keep` accepts the map value.

### `cover_bits`
[src/tools/wand.rs:307](../../../src/tools/wand.rs#L307)

```rust
fn cover_bits(mask: &[bool], w: i32, bbox: Bbox) -> Picked
```

A `Picked` over `bbox` from a `bool` mask.

### `cover`
[src/tools/wand.rs:315](../../../src/tools/wand.rs#L315)

```rust
fn cover(bbox: Bbox, set: impl Fn(i32, i32) -> bool) -> Picked
```

A `Picked` over `bbox` where `set(x, y)` is true.

### `cover_rows`
[src/tools/wand.rs:322](../../../src/tools/wand.rs#L322)

```rust
fn cover_rows(bbox: Bbox, row: impl Fn(i32, i32, i32, &mut Vec<u8>)) -> Picked
```

A `Picked` over `bbox` built row by row by the `row` callback (fast path for word scans).

## Tests

### `mod tests`

Run with `cargo test tools::wand::tests` · 16 tests

| Test | What it checks |
|---|---|
| [`a_click_inside_a_ring_takes_the_inside_only`](../../../src/tools/wand.rs#L384) | A click inside a ring takes the inside only. |
| [`a_click_on_the_line_takes_the_line`](../../../src/tools/wand.rs#L393) | A click on the line takes the line. |
| [`a_click_outside_takes_the_bare_canvas_round_it`](../../../src/tools/wand.rs#L402) | A click outside takes the bare canvas round it. |
| [`tolerance_decides_how_far_a_colour_runs`](../../../src/tools/wand.rs#L411) | Tolerance decides how far a colour runs. |
| [`any_paint_takes_the_whole_blob_whatever_its_colours`](../../../src/tools/wand.rs#L422) | Any paint takes the whole blob whatever its colours. |
| [`contiguous_off_takes_every_match_anywhere`](../../../src/tools/wand.rs#L438) | Contiguous off takes every match anywhere. |
| [`gap_closing_stops_at_a_break_in_the_line`](../../../src/tools/wand.rs#L451) | Gap closing stops at a break in the line. |
| [`expand_grows_the_pick`](../../../src/tools/wand.rs#L465) | Expand grows the pick. |
| [`a_retuned_pick_matches_a_fresh_one`](../../../src/tools/wand.rs#L475) | A retuned pick matches a fresh one. |
| [`a_grainy_line_holds_the_pick`](../../../src/tools/wand.rs#L513) | A grainy line holds the pick. |
| [`a_pick_on_bare_canvas_takes_the_soft_edges_of_its_lines`](../../../src/tools/wand.rs#L533) | A pick on bare canvas takes the soft edges of its lines. |
| [`tolerance_on_bare_canvas_is_how_covered_a_pixel_may_be`](../../../src/tools/wand.rs#L547) | Tolerance on bare canvas is how covered a pixel may be. |
| [`the_fast_pick_matches_the_first_one_exactly`](../../../src/tools/wand.rs#L671) | The fast pick against the first one, over random scenes, presses and settings, a press retuned the way a drag retunes it. |
| [`a_press_off_the_pixels_picks_nothing`](../../../src/tools/wand.rs#L697) | A press off the pixels picks nothing. |
| [`tolerance_steps_climb_a_ladder`](../../../src/tools/wand.rs#L703) | Tolerance steps climb a ladder. |
| [`wand_phases`](../../../src/tools/wand.rs#L716) | Where an 8K pick's time goes, phase by phase. `cargo test --release wand_phases -- --ignored --nocapture` |

**Test helpers** (used only by the tests above):

- [`blank`](../../../src/tools/wand.rs#L349) — Blank.
- [`rect`](../../../src/tools/wand.rs#L353) — Rect.
- [`ring`](../../../src/tools/wand.rs#L363) — A square outline from (5,5) to (20,20), one pixel thick.
- [`pick`](../../../src/tools/wand.rs#L370) — Pick.
- [`count`](../../../src/tools/wand.rs#L374) — Count.
- [`at`](../../../src/tools/wand.rs#L378) — At.
- [`grainy_ring`](../../../src/tools/wand.rs#L492) — A grainy pencil ring of radius 10 round (15, 15): at most about 80% covered, fading over 3 px each side and thinned by a hash of the pixel to as little as a third, so pixels under half covered run right through it.
- [`reference_pick`](../../../src/tools/wand.rs#L563) — The pick as first written: the bucket's own flood and rules, straight off the pixels. What the fast one must match, pixel for pixel.
- [`next`](../../../src/tools/wand.rs#L632) — Next.
- [`below`](../../../src/tools/wand.rs#L639) — Below.
- [`scene`](../../../src/tools/wand.rs#L647) — Random line art: boxes of a few close colours, broken outlines, faint anti-aliased specks and bare canvas, at sizes that are and aren't a multiple of the scan's word.

**Test-only types and constants:** [`W`](../../../src/tools/wand.rs#L344), [`H`](../../../src/tools/wand.rs#L345), [`INK`](../../../src/tools/wand.rs#L346), [`RED`](../../../src/tools/wand.rs#L347), [`Rng`](../../../src/tools/wand.rs#L629)
