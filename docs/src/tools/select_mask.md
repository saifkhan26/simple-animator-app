# `src/tools/select_mask.rs` — the selection mask in document space

> **Source:** [src/tools/select_mask.rs](../../../src/tools/select_mask.rs) · **Module:** `crate::tools::select_mask` · **Size:** 1698 lines · Line links as of commit `1fa5845`

## What this file is for

`SelectionMask` is **the selection itself**: per-pixel coverage (0 = not selected, 255 = fully selected, in
between = soft/feathered) in **document space**. It belongs to the canvas, not to a drawing, so it survives
changing frame or layer; each layer sees it through its own transform (`to_cell`). Every paint edit is clipped
to it.

Storage: a bounding box with one byte per pixel, plus an `outside` value (0 or 255) for every pixel outside the
box. So "Select All" is free (an empty box with `outside = 255`), and an inverted selection is cheap.

This file also has the selection operations: combining a new shape with the old one (**Replace / Add / Subtract /
Intersect**), rectangle and ellipse paths, invert, grow, shrink, feather, the marching-ants outline, and a
run-length packed form for the undo history.

```text
combine, per pixel (a = old, b = new, 0–255):
  Add       = max(a, b)          Subtract  = a × (255 − b) / 255
  Intersect = a × b / 255        Replace   = b
```

## How it connects

- **Used by:** [app/select.rs](../app/select.md) (`AppState::sel_mask`), [app.rs](../app.md),
  [undo.rs](../undo.md) (`PackedMask` in `Command::Selection`), [ui/shell.rs](../ui/shell.md) (outline drawing),
  [tools/selection.rs](selection.md).
- **Uses:** [tools/lasso.rs](lasso.md) (`coverage_rect`, `Mask`), [doc/transform.rs](../doc/transform.md),
  [tools/scan.rs](scan.md).

## Rust concepts in this file

**Consuming methods returning `Option<Self>`.** `normalize(self) -> Option<Self>` takes the mask by value and
returns a trimmed one, or `None` if nothing is selected — the type says "a selection may vanish".

**Algorithms you may know from graphics:** marching squares (outline), Ramer–Douglas–Peucker (simplifying the
outline), exact Euclidean distance transform (grow/shrink), repeated box blur ≈ gaussian (feather).

## Types

### `const THRESHOLD`
[src/tools/select_mask.rs:22](../../../src/tools/select_mask.rs#L22)

```rust
pub const THRESHOLD: u8 = 128;
```

Coverage at or above this counts as "inside" for the outline and for grow.

### `enum SelOp`
[src/tools/select_mask.rs:26](../../../src/tools/select_mask.rs#L26)

How a newly drawn shape combines with the selection already there.

| Variant | Meaning |
|---|---|
| `Replace` | The new shape becomes the selection. |
| `Add` | The new shape is added to the selection. |
| `Subtract` | The new shape is removed from it. |
| `Intersect` | Only where old and new overlap stays selected. |

### `enum SelShape`
[src/tools/select_mask.rs:69](../../../src/tools/select_mask.rs#L69)

What a selection drag draws.

| Variant | Meaning |
|---|---|
| `Freehand` | The lasso: dragged freehand, or tapped out as a polygon. Prefs saved when the polygon was a shape of its own read as this. |
| `Rect` | A rectangle dragged out (Shift for square). |
| `Ellipse` | An ellipse dragged out (Shift for circle). |
| `Wand` | Magic select: a click picks the pixels joined to the one under it. Not a drag shape — see `crate::tools::wand`. |

### `struct SelectionMask`
[src/tools/select_mask.rs:98](../../../src/tools/select_mask.rs#L98)

Selection coverage in document space. See the module doc.

| Field | Type | Meaning |
|---|---|---|
| `x` | `i32` | Left of the box, in document pixels. |
| `y` | `i32` | Top of the box. |
| `w` | `u32` | Box width. |
| `h` | `u32` | Box height. |
| `cov` | `Vec<u8>` | Row-major, `w * h` bytes. |
| `outside` | `u8` | Coverage of every pixel outside the box: 0 or 255. |

### `struct PackedMask`
[src/tools/select_mask.rs:113](../../../src/tools/select_mask.rs#L113)

A selection run-length encoded for the undo history: `(value, count)` pairs.

| Field | Type | Meaning |
|---|---|---|
| `x` | `i32` | Box left. |
| `y` | `i32` | Box top. |
| `w` | `u32` | Box width. |
| `h` | `u32` | Box height. |
| `outside` | `u8` | Coverage outside the box. |
| `runs` | `Vec<(u8, u32)>` | Runs of equal coverage bytes, as `(value, length)`. |

### `const EDT_FAR`
[src/tools/select_mask.rs:837](../../../src/tools/select_mask.rs#L837)

```rust
const EDT_FAR: f64 = 1e12;
```

"Infinitely far" for the distance transform, kept finite so the maths stays exact.

## `impl SelOp`

### `SelOp::label`
[src/tools/select_mask.rs:36](../../../src/tools/select_mask.rs#L36)

```rust
pub fn label(self) -> &'static str
```

The mode's display name.

### `SelOp::apply_row`
[src/tools/select_mask.rs:46](../../../src/tools/select_mask.rs#L46)

```rust
fn apply_row(self, a: &[u8], b: &[u8], out: &mut [u8])
```

`apply` along a row of bytes.

### `SelOp::apply`
[src/tools/select_mask.rs:57](../../../src/tools/select_mask.rs#L57)

```rust
fn apply(self, a: u8, b: u8) -> u8
```

Combines existing coverage `a` with new coverage `b` (see the formulas above).

## `impl SelShape`

### `SelShape::label`
[src/tools/select_mask.rs:83](../../../src/tools/select_mask.rs#L83)

```rust
pub fn label(self) -> &'static str
```

The shape's display name.

## `impl PackedMask`

### `PackedMask::unpack`
[src/tools/select_mask.rs:123](../../../src/tools/select_mask.rs#L123)

```rust
pub fn unpack(&self) -> SelectionMask
```

Expands the runs back into a full `SelectionMask`.

## Free functions

### `integer_offset`
[src/tools/select_mask.rs:144](../../../src/tools/select_mask.rs#L144)

```rust
pub fn integer_offset(xf: &Transform, cw: u32, ch: u32, pw: f32, ph: f32) -> Option<(i32, i32)>
```

If a layer transform is a plain whole-pixel move, the integer offset from cell to document coordinates — then
moving a mask between the two is a crop, not a resample. `None` otherwise.

### `rect_path`
[src/tools/select_mask.rs:158](../../../src/tools/select_mask.rs#L158)

```rust
pub fn rect_path(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)>
```

The four corners of a rectangle selection, snapped to whole pixels so it has hard edges.

### `ellipse_path`
[src/tools/select_mask.rs:167](../../../src/tools/select_mask.rs#L167)

```rust
pub fn ellipse_path(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)>
```

The ellipse inside box `a`–`b` as a polygon fine enough (sagitta ≤ 0.2 px) to look smooth.

## `impl SelectionMask`

### `SelectionMask::all`
[src/tools/select_mask.rs:190](../../../src/tools/select_mask.rs#L190)

```rust
pub fn all() -> Self
```

Everything selected (empty box, `outside = 255`).

### `SelectionMask::is_all`
[src/tools/select_mask.rs:202](../../../src/tools/select_mask.rs#L202)

```rust
pub fn is_all(&self) -> bool
```

Whether this is a normalised Select All.

### `SelectionMask::at`
[src/tools/select_mask.rs:207](../../../src/tools/select_mask.rs#L207)

```rust
pub fn at(&self, x: i32, y: i32) -> u8
```

Coverage at document pixel `(x, y)` (the `outside` value outside the box).

### `SelectionMask::row_into`
[src/tools/select_mask.rs:220](../../../src/tools/select_mask.rs#L220)

```rust
pub fn row_into(&self, x: i32, y: i32, out: &mut [u8])
```

A row of coverage into `out`, a slice at a time (faster than `at` per pixel).

### `SelectionMask::bounds`
[src/tools/select_mask.rs:236](../../../src/tools/select_mask.rs#L236)

```rust
pub fn bounds(&self) -> Option<(i32, i32, i32, i32)>
```

The box as `(x0, y0, x1, y1)`, or `None` if empty.

### `SelectionMask::normalize`
[src/tools/select_mask.rs:249](../../../src/tools/select_mask.rs#L249)

```rust
pub fn normalize(self) -> Option<Self>
```

Trims the box to the pixels that differ from `outside`; `None` when nothing is selected.

### `SelectionMask::inverted`
[src/tools/select_mask.rs:290](../../../src/tools/select_mask.rs#L290)

```rust
pub fn inverted(&self) -> Option<Self>
```

The opposite selection (255 − coverage, outside flipped); `None` if that selects nothing.

### `SelectionMask::translated`
[src/tools/select_mask.rs:300](../../../src/tools/select_mask.rs#L300)

```rust
pub fn translated(&self, dx: i32, dy: i32) -> Self
```

The mask moved by whole pixels.

### `SelectionMask::combine`
[src/tools/select_mask.rs:310](../../../src/tools/select_mask.rs#L310)

```rust
pub fn combine(a: Option<&Self>, b: &Self, op: SelOp) -> Option<Self>
```

`a` combined with `b` under `op` over the union of their boxes (`a` is `None` when nothing was selected).

### `SelectionMask::from_path`
[src/tools/select_mask.rs:352](../../../src/tools/select_mask.rs#L352)

```rust
pub fn from_path(pts: &[(f32, f32)], clip: (i32, i32, i32, i32)) -> Option<Self>
```

Rasterises a closed document-space path into a mask, clipped to `clip` (so a lasso drawn zoomed far out doesn't
allocate a gigantic buffer).

### `SelectionMask::sample`
[src/tools/select_mask.rs:367](../../../src/tools/select_mask.rs#L367)

```rust
fn sample(&self, x: f32, y: f32) -> f32
```

Bilinear coverage at a fractional document point.

### `SelectionMask::to_cell`
[src/tools/select_mask.rs:389](../../../src/tools/select_mask.rs#L389)

```rust
pub fn to_cell(&self, xf: &Transform, cw: u32, ch: u32, pw: f32, ph: f32) -> Mask
```

The mask as seen by a drawing placed by `xf`: a byte-exact crop for whole-pixel moves, otherwise each cell pixel
is mapped into document space and sampled. This is the clip a stroke on that drawing is held to.

### `SelectionMask::from_cell`
[src/tools/select_mask.rs:497](../../../src/tools/select_mask.rs#L497)

```rust
pub fn from_cell(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    cov: &[u8],
    xf: &Transform,
    cw: u32,
    ch: u32,
    pw: f32,
    ph: f32,
) -> Option<Self>
```

The reverse: a cell-space coverage box mapped back into document space (how a scaled/rotated float hands its
shape back to the selection when it lands).

### `SelectionMask::pack`
[src/tools/select_mask.rs:574](../../../src/tools/select_mask.rs#L574)

```rust
pub fn pack(&self) -> PackedMask
```

Run-length encodes the mask for the undo history.

### `SelectionMask::contours`
[src/tools/select_mask.rs:596](../../../src/tools/select_mask.rs#L596)

```rust
pub fn contours(&self) -> Vec<Vec<(f32, f32)>>
```

The marching-ants outline: closed loops along the 50% coverage level (one per boundary, so a hole gives two
loops), by marching squares with interpolated crossings, then simplified.

### `SelectionMask::grow`
[src/tools/select_mask.rs:737](../../../src/tools/select_mask.rs#L737)

```rust
pub fn grow(&self, n: u32) -> Option<Self>
```

Grows by `n` pixels with rounded corners, using an exact distance transform (cost independent of `n`).

### `SelectionMask::shrink`
[src/tools/select_mask.rs:776](../../../src/tools/select_mask.rs#L776)

```rust
pub fn shrink(&self, n: u32) -> Option<Self>
```

Shrinks by `n` pixels (grows the unselected part); `None` once nothing is left.

### `SelectionMask::feather`
[src/tools/select_mask.rs:789](../../../src/tools/select_mask.rs#L789)

```rust
pub fn feather(&self, r: u32) -> Option<Self>
```

Softens the edge over about `r` pixels with three box blurs (approximating a gaussian).

## Free functions

### `edt_2d`
[src/tools/select_mask.rs:843](../../../src/tools/select_mask.rs#L843)

```rust
fn edt_2d(f: &mut [f64], w: usize, h: usize)
```

In-place 2-D squared Euclidean distance transform (Felzenszwalb & Huttenlocher): columns then rows with
`edt_1d`.

### `edt_1d`
[src/tools/select_mask.rs:865](../../../src/tools/select_mask.rs#L865)

```rust
fn edt_1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64])
```

One line of the distance transform: the lower envelope of parabolas `(q − p)² + f[p]`.

### `box_blur`
[src/tools/select_mask.rs:904](../../../src/tools/select_mask.rs#L904)

```rust
fn box_blur(data: &mut [f32], r: usize, tmp: &mut Vec<f32>)
```

One box-blur pass of radius `r` over a line, clamping at the ends.

### `simplify_loop`
[src/tools/select_mask.rs:924](../../../src/tools/select_mask.rs#L924)

```rust
fn simplify_loop(pts: &[(f32, f32)], eps: f32) -> Vec<(f32, f32)>
```

Simplifies a closed outline loop with Ramer–Douglas–Peucker (split at the first point and the farthest one).

### `rdp`
[src/tools/select_mask.rs:954](../../../src/tools/select_mask.rs#L954)

```rust
fn rdp(pts: &[(f32, f32)], eps: f32) -> Vec<(f32, f32)>
```

Ramer–Douglas–Peucker on an open polyline: keep the endpoints, find the point farthest from the line between
them; if it's farther than `eps`, keep it and recurse on both halves, else drop everything between.

## Tests

### `mod tests`

Run with `cargo test tools::select_mask::tests` · 20 tests

| Test | What it checks |
|---|---|
| [`the_row_wise_mask_methods_match_the_per_pixel_ones`](../../../src/tools/select_mask.rs#L1392) | Every faster mask method against the per-pixel original, over random masks: the same bytes, loops and runs. |
| [`a_row_reads_what_each_pixel_reads`](../../../src/tools/select_mask.rs#L1422) | A row reads what each pixel reads. |
| [`a_path_can_sit_left_of_and_above_the_origin`](../../../src/tools/select_mask.rs#L1453) | A path can sit left of and above the origin. |
| [`rectangles_have_hard_edges`](../../../src/tools/select_mask.rs#L1462) | Rectangles have hard edges. |
| [`combine_follows_the_truth_table`](../../../src/tools/select_mask.rs#L1468) | Combine follows the truth table. |
| [`disjoint_intersect_is_nothing_and_add_to_nothing_is_the_shape`](../../../src/tools/select_mask.rs#L1485) | Disjoint intersect is nothing and add to nothing is the shape. |
| [`invert_twice_is_the_original_and_all_minus_a_rect_is_a_hole`](../../../src/tools/select_mask.rs#L1494) | Invert twice is the original and all minus a rect is a hole. |
| [`normalize_trims_the_box`](../../../src/tools/select_mask.rs#L1508) | Normalize trims the box. |
| [`pack_round_trips`](../../../src/tools/select_mask.rs#L1537) | Pack round trips. |
| [`identity_to_cell_is_a_byte_exact_crop`](../../../src/tools/select_mask.rs#L1549) | Identity to cell is a byte exact crop. |
| [`an_evenly_larger_cell_is_an_exact_translation`](../../../src/tools/select_mask.rs#L1560) | An evenly larger cell is an exact translation. |
| [`a_rotated_layer_samples_the_selection_where_it_lands`](../../../src/tools/select_mask.rs#L1572) | A rotated layer samples the selection where it lands. |
| [`select_all_covers_every_cell_pixel`](../../../src/tools/select_mask.rs#L1593) | Select all covers every cell pixel. |
| [`to_cell_then_from_cell_comes_back`](../../../src/tools/select_mask.rs#L1606) | To cell then from cell comes back. |
| [`a_rect_outlines_as_one_loop_on_its_edges`](../../../src/tools/select_mask.rs#L1626) | A rect outlines as one loop on its edges. |
| [`a_donut_outlines_as_two_loops_and_all_as_none`](../../../src/tools/select_mask.rs#L1639) | A donut outlines as two loops and all as none. |
| [`an_ellipse_path_stays_on_the_ellipse`](../../../src/tools/select_mask.rs#L1650) | An ellipse path stays on the ellipse. |
| [`grow_adds_n_on_each_side_and_rounds_the_corners`](../../../src/tools/select_mask.rs#L1661) | Grow adds n on each side and rounds the corners. |
| [`shrink_undoes_grow_on_a_convex_shape_and_too_far_is_nothing`](../../../src/tools/select_mask.rs#L1675) | Shrink undoes grow on a convex shape and too far is nothing. |
| [`feather_softens_the_edge_and_keeps_the_mass`](../../../src/tools/select_mask.rs#L1687) | Feather softens the edge and keeps the mass. |

**Test helpers** (used only by the tests above):

- [`old_normalize`](../../../src/tools/select_mask.rs#L1000) — Old normalize.
- [`old_combine`](../../../src/tools/select_mask.rs#L1039) — Old combine.
- [`old_to_cell`](../../../src/tools/select_mask.rs#L1076) — Old to cell.
- [`old_pack`](../../../src/tools/select_mask.rs#L1182) — Old pack.
- [`old_contours`](../../../src/tools/select_mask.rs#L1200) — Old contours.
- [`old_unpack`](../../../src/tools/select_mask.rs#L1327) — Old unpack.
- [`next`](../../../src/tools/select_mask.rs#L1347) — Next.
- [`below`](../../../src/tools/select_mask.rs#L1354) — Below.
- [`random_mask`](../../../src/tools/select_mask.rs#L1362) — A random mask: boxes of full, empty and in-between coverage, some with feathered or speckled edges, anywhere around the origin, with either `outside`.
- [`rect`](../../../src/tools/select_mask.rs#L1436) — Rect.
- [`count`](../../../src/tools/select_mask.rs#L1440) — Count.

**Test-only types and constants:** [`Rng`](../../../src/tools/select_mask.rs#L1344), [`HUGE`](../../../src/tools/select_mask.rs#L1434)
