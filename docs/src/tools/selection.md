# `src/tools/selection.rs` — floating selected pixels and the transform box

> **Source:** [src/tools/selection.rs](../../../src/tools/selection.rs) · **Module:** `crate::tools::selection` · **Size:** 1563 lines · Line links as of commit `67a7cb9`

## What this file is for

When you drag inside a selection, the selected pixels **lift** off the drawing and **float**: you can move,
scale and rotate them with a transform box, and they **land** (are stamped back) when you're done. This file
is that floating half (`Selection`), plus the transform-box geometry (`Pose`, `Grab`).

How a float works:
1. **Lift**: the pixels under the selection mask are copied out (each pixel's alpha multiplied by the mask's
   coverage), and the source is erased through the *same* mask — so lifted + remaining still add up to the
   original drawing. Erasing is deferred until the first move, so selecting and deselecting changes nothing.
2. **Pose**: the float's position is a `Pose` — scale and rotate about the box centre, then move. While the pose
   is a whole-pixel move it's lossless (a plain byte copy); once scaled/rotated it's resampled.
3. **Land**: `stamp` composites the pixels back under the pose.

A **selection mask** is a per-pixel "how selected" map, 0–255, like an alpha channel:

```text
selection mask (coverage)      drawing alpha        lifted alpha = alpha × coverage / 255
  0   0   0   0                 255 255 255 255       0    0    0    0
  0 255 255 128                 255 255 255 255       0  255  255  128
  0 255 255 128                 255 255   0 255       0  255    0  128
```

## How it connects

- **Used by:** [app/select.rs](../app/select.md) (everything about floating pixels), [ui/shell.rs](../ui/shell.md)
  (drawing the float and its handles), [app.rs](../app.md).
- **Uses:** [tools/lasso.rs](lasso.md) (`Mask`), [tools/select_mask.rs](select_mask.md) (outline tracing),
  [doc/transform.rs](../doc/transform.md), [doc/orient.rs](../doc/orient.md), [doc/canvas.rs](../doc/canvas.md).

## Rust concepts in this file

**Enum variants with a payload.** `Grab::Scale(u8)` carries which of the eight handles was grabbed:
`match grab { Grab::Scale(id) => …, Grab::Move => …, Grab::Rotate => … }`. More:
[Enums and match](../../rust-primer.md#enums-and-match)

**Pure "solve from the start pose" functions.** `Pose::dragged(&self, …) -> Pose` never mutates; callers keep the
pose from the press and ask for the new pose each frame, so the result depends only on where the pointer is now.

**Premultiplied sampling.** `sample_premul` interpolates in premultiplied space so transparent pixels' zero RGB
doesn't darken soft edges (see [glossary](../../glossary.md)).

## Types

### `const HANDLE_PX`
[src/tools/selection.rs:29](../../../src/tools/selection.rs#L29)

```rust
pub const HANDLE_PX: f32 = 5.0;
```

Half-size of a transform handle, in *screen* pixels. Shared with the overlay so the square that gets drawn is the square that can be grabbed.

### `const MIN_SCALE`
[src/tools/selection.rs:34](../../../src/tools/selection.rs#L34)

```rust
pub const MIN_SCALE: f32 = 0.02;
```

Scale limits for a handle drag. Zero has no inverse and would leave a box with no grabbable handles; the ceiling keeps a runaway drag from asking for a gigapixel resample.

### `const MAX_SCALE`
[src/tools/selection.rs:35](../../../src/tools/selection.rs#L35)

```rust
pub const MAX_SCALE: f32 = 64.0;
```

The largest scale a handle drag allows (see `MIN_SCALE`).

### `enum Grab`
[src/tools/selection.rs:39](../../../src/tools/selection.rs#L39)

Which part of the transform box a press landed on.

| Variant | Meaning |
|---|---|
| `Move` | Inside the covered pixels — the drag moves the selection. |
| `Scale(u8)` | One of the eight box handles. 0..=3 are the corners TL, TR, BR, BL; 4..=7 are the edge midpoints T, R, B, L. |
| `Rotate` | The ring just outside a corner. |

### `struct Pose`
[src/tools/selection.rs:100](../../../src/tools/selection.rs#L100)

Where the lifted buffer sits relative to where it was cut from: scale and rotate about the buffer's centre, then displace.

Kept separate from `Selection` so a drag can be solved against the pose recorded when the handle was grabbed, without cloning the pixels.

| Field | Type | Meaning |
|---|---|---|
| `offset` | `(f32, f32)` | Displacement from where the pixels were lifted, in cell pixels. Fractional only once a scale or rotation has anchored the box somewhere other than its centre — a plain drag or an arrow nudge still moves by whole pixels. |
| `scale` | `(f32, f32)` | Per-axis scale about the box centre. `(1.0, 1.0)` is native size. |
| `rot` | `f32` | Rotation about the box centre, in radians. |

### `struct DragMods`
[src/tools/selection.rs:266](../../../src/tools/selection.rs#L266)

What the modifiers held during a transform-box drag ask for.

| Field | Type | Meaning |
|---|---|---|
| `proportional` | `bool` | Shift: any handle keeps the box's proportions as they were when it was grabbed, and a rotation snaps to 15°. |
| `centred` | `bool` | Alt: scale about the box's centre rather than the opposite handle, so both sides move. |

### `struct Placement`
[src/tools/selection.rs:280](../../../src/tools/selection.rs#L280)

Where a cell sat in the document when a float was cut from it: the layer transform plus the cell and canvas sizes that `crate::doc::transform::Transform::cell_to_doc` needs. Landing the float maps its shape back into the document selection through this.

| Field | Type | Meaning |
|---|---|---|
| `xf` | `Transform` | The layer transform when the float was cut. |
| `cw` | `u32` | Cell width. |
| `ch` | `u32` | Cell height. |
| `pw` | `f32` | Document width. |
| `ph` | `f32` | Document height. |

### `struct Selection`
[src/tools/selection.rs:301](../../../src/tools/selection.rs#L301)

Floating pixels lifted from one drawing, with their pose.

| Field | Type | Meaning |
|---|---|---|
| `cell` | `CellId` | Cell the pixels were lifted from. A float never outlives its cell: changing frame or layer lands it first. |
| `mask` | `Mask` | The coverage mask the pixels were cut through. |
| `pixels` | `Vec<u8>` | RGBA of the masked region, `mask.w * mask.h * 4`, straight alpha with the mask coverage already folded in.  Written once at the lift and never touched again — see the module doc. |
| `pose` | `Pose` | Where the floating pixels currently sit relative to the lift. |
| `outline` | `Vec<Vec<(f32, f32)>>` | The mask's outline as closed loops in cell space, as lifted. Run each point through `Selection::path_point` to draw it under the current pose. |
| `lifted` | `bool` | Whether the source pixels have been erased yet. Deferred until the first move so that selecting and then deselecting changes nothing. |
| `placement` | `Placement` | The cell's place in the document at the lift. |
| `orient` | `Orient` | Every mirror and quarter turn done to it since the lift, so the same can be done to the other drawings an "All frames" landing cuts. |

## `impl Grab`

### `Grab::handle_pos`
[src/tools/selection.rs:51](../../../src/tools/selection.rs#L51)

```rust
pub fn handle_pos(id: u8, w: f32, h: f32) -> (f32, f32)
```

Where handle `id` (0–3 corners, 4–7 edge midpoints) sits in the lifted buffer's pixel space.

### `Grab::handle_positions`
[src/tools/selection.rs:66](../../../src/tools/selection.rs#L66)

```rust
pub fn handle_positions(w: f32, h: f32) -> [(f32, f32); 8]
```

All eight handle positions, in id order, for drawing them.

### `Grab::anchor`
[src/tools/selection.rs:76](../../../src/tools/selection.rs#L76)

```rust
pub fn anchor(self, w: f32, h: f32) -> Option<((f32, f32), (bool, bool))>
```

For a scale handle, the point that must stay fixed while it's dragged (the opposite handle) and which axes the
handle drives; `None` for move/rotate.

## `impl Default for Pose`

### `Pose::default`
[src/tools/selection.rs:113](../../../src/tools/selection.rs#L113)

```rust
fn default() -> Self
```

The identity pose: no offset, scale 1, no rotation.

## `impl Pose`

### `Pose::is_pixel_aligned`
[src/tools/selection.rs:125](../../../src/tools/selection.rs#L125)

```rust
pub fn is_pixel_aligned(&self) -> bool
```

True while the pose is an exact whole-pixel move (the lossless case).

### `Pose::buf_to_cell`
[src/tools/selection.rs:136](../../../src/tools/selection.rs#L136)

```rust
pub fn buf_to_cell(&self, m: &Mask, u: f32, v: f32) -> (f32, f32)
```

Maps a point in the lifted buffer to cell space: scale and rotate about the box centre, then add the offset.

### `Pose::cell_to_buf`
[src/tools/selection.rs:148](../../../src/tools/selection.rs#L148)

```rust
pub fn cell_to_buf(&self, m: &Mask, x: f32, y: f32) -> (f32, f32)
```

The exact inverse of `buf_to_cell`.

### `Pose::corners`
[src/tools/selection.rs:161](../../../src/tools/selection.rs#L161)

```rust
pub fn corners(&self, m: &Mask) -> [(f32, f32); 4]
```

The posed box's four corners in cell space.

### `Pose::dragged`
[src/tools/selection.rs:181](../../../src/tools/selection.rs#L181)

```rust
pub fn dragged(
    &self,
    m: &Mask,
    grab: Grab,
    start: (f32, f32),
    now: (f32, f32),
    mods: DragMods,
) -> Pose
```

The pose after dragging `grab` from `start` to `now` (solved from `self`, the pose at the press): a move offsets
(in whole pixels, Shift locking to an axis); a scale handle scales about the opposite handle (or the centre with
Alt; proportional with Shift); a rotate turns about the centre (15° steps with Shift).

## `impl Default for Placement`

### `Placement::default`
[src/tools/selection.rs:289](../../../src/tools/selection.rs#L289)

```rust
fn default() -> Self
```

A default placement (identity transform, zero sizes).

## Free functions

### `outline_of`
[src/tools/selection.rs:329](../../../src/tools/selection.rs#L329)

```rust
fn outline_of(mask: &Mask) -> Vec<Vec<(f32, f32)>>
```

The mask's outline loops in cell space, traced with the selection mask's contour tracer (for marching ants).

## `impl Selection`

### `Selection::new`
[src/tools/selection.rs:344](../../../src/tools/selection.rs#L344)

```rust
pub fn new(cell: CellId, canvas: &Canvas, mask: Mask, placement: Placement) -> Self
```

Copies the masked pixels out of `canvas` (alpha × coverage). The source is left intact until `lift_source`.

### `Selection::from_clip`
[src/tools/selection.rs:384](../../../src/tools/selection.rs#L384)

```rust
pub fn from_clip(cell: CellId, mask: Mask, pixels: Vec<u8>, placement: Placement) -> Self
```

A float made from clipboard pixels: already lifted (there's no source to erase).

### `Selection::lift_source`
[src/tools/selection.rs:398](../../../src/tools/selection.rs#L398)

```rust
pub fn lift_source(&mut self, canvas: &mut Canvas)
```

Erases the source region through the mask, once, the first time the float moves.

### `Selection::orient`
[src/tools/selection.rs:419](../../../src/tools/selection.rs#L419)

```rust
pub fn orient(&mut self, o: Orient)
```

Mirrors or quarter-turns the floating pixels in place about the box centre, moving pixels rather than resampling
(lossless), together with the mask and outline.

### `Selection::origin`
[src/tools/selection.rs:452](../../../src/tools/selection.rs#L452)

```rust
pub fn origin(&self) -> (i32, i32)
```

The float's top-left in cell space, for the pixel-aligned path.

### `Selection::buf_to_cell`
[src/tools/selection.rs:459](../../../src/tools/selection.rs#L459)

```rust
pub fn buf_to_cell(&self, u: f32, v: f32) -> (f32, f32)
```

`Pose::buf_to_cell` with this float's mask.

### `Selection::cell_to_buf`
[src/tools/selection.rs:463](../../../src/tools/selection.rs#L463)

```rust
pub fn cell_to_buf(&self, x: f32, y: f32) -> (f32, f32)
```

`Pose::cell_to_buf` with this float's mask.

### `Selection::corners`
[src/tools/selection.rs:468](../../../src/tools/selection.rs#L468)

```rust
pub fn corners(&self) -> [(f32, f32); 4]
```

The posed box's corners.

### `Selection::path_point`
[src/tools/selection.rs:474](../../../src/tools/selection.rs#L474)

```rust
pub fn path_point(&self, x: f32, y: f32) -> (f32, f32)
```

A point of the lifted outline mapped through the current pose (so marching ants follow a scaled/rotated box).

### `Selection::hit`
[src/tools/selection.rs:482](../../../src/tools/selection.rs#L482)

```rust
pub fn hit(&self, x: f32, y: f32) -> bool
```

Whether a cell point lands on covered pixels of the posed float — decides "move the float" vs "start a new lasso".

### `Selection::grab_at`
[src/tools/selection.rs:494](../../../src/tools/selection.rs#L494)

```rust
pub fn grab_at(&self, x: f32, y: f32, tol: f32) -> Option<Grab>
```

Which part of the transform box a cell point hits (see the free `grab_at`).

### `Selection::posed_cov`
[src/tools/selection.rs:501](../../../src/tools/selection.rs#L501)

```rust
pub fn posed_cov(&self) -> (i32, i32, u32, u32, Vec<u8>)
```

The selection coverage where the float now sits — what the document selection becomes on landing. Exact when
pixel-aligned, resampled otherwise.

### `Selection::stamp`
[src/tools/selection.rs:549](../../../src/tools/selection.rs#L549)

```rust
pub fn stamp(&self, canvas: &mut Canvas)
```

Composites the floating pixels into `canvas` under the current pose (straight-alpha source-over).

### `Selection::stamp_aligned`
[src/tools/selection.rs:558](../../../src/tools/selection.rs#L558)

```rust
fn stamp_aligned(&self, canvas: &mut Canvas)
```

The lossless stamp: a whole-pixel blit of exactly what was lifted.

### `Selection::stamp_transformed`
[src/tools/selection.rs:599](../../../src/tools/selection.rs#L599)

```rust
fn stamp_transformed(&self, canvas: &mut Canvas)
```

The resampled stamp: walks the destination bounds and inverse-maps each pixel into the lifted buffer.

### `Selection::bake`
[src/tools/selection.rs:620](../../../src/tools/selection.rs#L620)

```rust
pub fn bake(&self) -> Option<(Mask, Vec<u8>)>
```

Resamples the current pose into a new mask and pixel buffer, as if stamped and lifted again (used before copying a
transformed float). `None` when already pixel-aligned (keep the pristine buffer).

### `Selection::dest_bounds`
[src/tools/selection.rs:661](../../../src/tools/selection.rs#L661)

```rust
fn dest_bounds(&self) -> (i32, i32, i32, i32)
```

Integer bounds of the posed box in cell space.

### `Selection::supersample`
[src/tools/selection.rs:681](../../../src/tools/selection.rs#L681)

```rust
fn supersample(&self) -> i32
```

How many samples per axis per destination pixel: more when shrinking, so fine line art doesn't sparkle.

### `Selection::resample`
[src/tools/selection.rs:699](../../../src/tools/selection.rs#L699)

```rust
fn resample(&self, px: i32, py: i32, n: i32) -> Option<([f32; 3], f32)>
```

Resolves one destination pixel by inverse-mapping `n × n` sample points and averaging; `None` where nothing lands.

### `Selection::sample_premul`
[src/tools/selection.rs:733](../../../src/tools/selection.rs#L733)

```rust
fn sample_premul(&self, x: f32, y: f32) -> [f32; 4]
```

One bilinear sample of the lifted buffer in premultiplied space (0–1 RGBA).

## Free functions

### `grab_at`
[src/tools/selection.rs:776](../../../src/tools/selection.rs#L776)

```rust
pub fn grab_at(mask: &Mask, pose: &Pose, x: f32, y: f32, tol: f32) -> Option<Grab>
```

Which part of a transform box a point hits: a handle (within `tol`), the rotate ring just outside a corner, or
inside the covered pixels (move). A free function so it works on the document selection before anything is
lifted.

### `blend_over`
[src/tools/selection.rs:837](../../../src/tools/selection.rs#L837)

```rust
fn blend_over(canvas: &mut Canvas, px: i32, py: i32, src: [f32; 3], sa: f32)
```

Straight-alpha source-over of one sample onto a canvas pixel.

### `clip_rect`
[src/tools/selection.rs:855](../../../src/tools/selection.rs#L855)

```rust
fn clip_rect(b: (i32, i32, i32, i32), cw: i32, ch: i32) -> Option<(i32, i32, i32, i32)>
```

Clips integer bounds to the canvas, or `None` if nothing is left.

## Tests

### `mod tests`

Run with `cargo test tools::selection::tests` · 33 tests

| Test | What it checks |
|---|---|
| [`a_flipped_float_mirrors_pixels_and_mask_in_place`](../../../src/tools/selection.rs#L911) | A flipped float mirrors pixels and mask in place. |
| [`quarter_turns_return_exactly`](../../../src/tools/selection.rs#L938) | Quarter turns return exactly. |
| [`a_flip_of_a_rotated_float_mirrors_on_screen`](../../../src/tools/selection.rs#L960) | A flip of a scaled, rotated float is the float as it was on screen, mirrored about its centre: the pixel at the old top-left corner is now where that corner's mirror image is. |
| [`a_quarter_turn_swaps_a_non_uniform_scale`](../../../src/tools/selection.rs#L979) | A quarter turn swaps a non uniform scale. |
| [`a_turn_near_the_cell_edge_keeps_its_place`](../../../src/tools/selection.rs#L993) | Turned against the top edge, the box can't move its origin up past zero; the pose carries the rest, still whole pixels, and turning back lands it exactly where it was. |
| [`lift_then_stamp_in_place_restores_the_drawing`](../../../src/tools/selection.rs#L1007) | Lift then stamp in place restores the drawing. |
| [`moving_puts_the_pixels_at_the_offset`](../../../src/tools/selection.rs#L1027) | Moving puts the pixels at the offset. |
| [`pixels_moved_off_canvas_are_dropped_not_wrapped`](../../../src/tools/selection.rs#L1043) | Pixels moved off canvas are dropped not wrapped. |
| [`hit_testing_follows_the_offset`](../../../src/tools/selection.rs#L1055) | Hit testing follows the offset. |
| [`a_whole_pixel_move_is_still_pixel_aligned`](../../../src/tools/selection.rs#L1068) | A whole-pixel move must keep taking the exact blit, or the README's "moves never resample" promise quietly stops being true. |
| [`buf_and_cell_mappings_round_trip`](../../../src/tools/selection.rs#L1087) | `cell_to_buf` is a hand-written inverse of `buf_to_cell`, so the two can silently disagree — exactly the failure the `Xform` round-trip test in the shell exists to catch. Check a pose using every term at once. |
| [`a_quarter_turn_puts_the_corners_where_they_belong`](../../../src/tools/selection.rs#L1113) | A quarter turn about the box centre must land the corners on each other's places, not somewhere near them. |
| [`rotating_a_selection_stamps_it_turned`](../../../src/tools/selection.rs#L1137) | Rotating a tall selection by a quarter turn should make it wide, and the pixels should actually be there. |
| [`scaling_up_covers_twice_the_span`](../../../src/tools/selection.rs#L1161) | Doubling the scale about the centre must widen the covered span to match. |
| [`scaling_does_not_darken_the_edge`](../../../src/tools/selection.rs#L1185) | Straight-alpha bilinear would pull the zeroed RGB of the transparent surround into the edge and mud the colour toward black. Premultiplied sampling must not. |
| [`adjusting_the_pose_does_not_accumulate_blur`](../../../src/tools/selection.rs#L1221) | The pose is metadata over a pristine buffer, so scaling out and back again must not soften anything: the second pose is resolved from the original pixels, not from the result of the first. |
| [`a_corner_drag_pins_the_opposite_corner`](../../../src/tools/selection.rs#L1246) | Scaling from a corner handle must pin the opposite corner, or the drawing slides out from under the pointer. |
| [`shift_on_a_corner_projects_onto_the_diagonal`](../../../src/tools/selection.rs#L1305) | Shift on a corner keeps one ratio, at the point on the box's diagonal nearest the pointer: pulled to (40, 20) from (10, 10), that is 3×. |
| [`shift_on_a_corner_keeps_a_stretched_aspect`](../../../src/tools/selection.rs#L1316) | Shift on a box already stretched 2:1 keeps it 2:1, not square. |
| [`shift_on_an_edge_keeps_the_aspect`](../../../src/tools/selection.rs#L1331) | Shift on an edge handle scales both axes, the far one about the box's centre line, and keeps the aspect the box had. |
| [`alt_scales_about_the_centre`](../../../src/tools/selection.rs#L1352) | Alt pins the centre instead of the opposite handle, so the box grows on both sides: twice the pointer's travel in all. |
| [`shift_alt_keeps_aspect_about_the_centre`](../../../src/tools/selection.rs#L1365) | Shift alt keeps aspect about the centre. |
| [`proportional_clamps_keep_the_aspect`](../../../src/tools/selection.rs#L1383) | At the scale limits the factor stops as one, so the aspect holds. |
| [`an_edge_drag_scales_one_axis`](../../../src/tools/selection.rs#L1400) | An edge handle drives one axis and leaves the other alone. |
| [`dragging_past_the_anchor_clamps_rather_than_flips`](../../../src/tools/selection.rs#L1416) | Dragging a handle past its anchor clamps instead of mirroring the art. |
| [`a_rotate_drag_keeps_the_centre_put`](../../../src/tools/selection.rs#L1431) | Rotation turns about the centre and leaves it where it was. |
| [`shift_snaps_rotation_to_fifteen_degrees`](../../../src/tools/selection.rs#L1454) | Shift snaps a rotation to 15°, for squaring a drawing back up. |
| [`grab_picks_handles_then_the_interior`](../../../src/tools/selection.rs#L1471) | The handles must be grabbable, and the interior must still read as a move so a plain drag is unchanged. |
| [`hit_testing_follows_a_rotation`](../../../src/tools/selection.rs#L1489) | Hit testing goes through the pose, so a rotated box is grabbable where it looks rather than where it was lifted. |
| [`bake_resamples_only_when_the_pose_needs_it`](../../../src/tools/selection.rs#L1503) | The clipboard bakes a transformed selection so it pastes as it looked, and leaves a pixel-aligned one alone so a plain copy stays lossless. |
| [`posed_coverage_is_the_mask_moved_when_aligned`](../../../src/tools/selection.rs#L1534) | Landing a plain move must hand back exactly the lifted coverage, moved. |
| [`posed_coverage_follows_a_quarter_turn`](../../../src/tools/selection.rs#L1546) | A quarter turn swaps the posed coverage's width and height. |
| [`a_new_float_outlines_its_mask`](../../../src/tools/selection.rs#L1557) | A new float outlines its mask. |

**Test helpers** (used only by the tests above):

- [`filled`](../../../src/tools/selection.rs#L872) — Filled.
- [`square`](../../../src/tools/selection.rs#L880) — Square.
- [`sel_on`](../../../src/tools/selection.rs#L884) — Sel on.
- [`rgba`](../../../src/tools/selection.rs#L889) — Rgba.
- [`ramp`](../../../src/tools/selection.rs#L895) — A canvas with every pixel its own colour, so any misplaced byte shows.
- [`lifted`](../../../src/tools/selection.rs#L904) — Lifted, so it can be turned.
- [`box10`](../../../src/tools/selection.rs#L1288) — Box10.
- [`near`](../../../src/tools/selection.rs#L1298) — Near.

**Test-only types and constants:** [`PLAIN`](../../../src/tools/selection.rs#L1271), [`SHIFT`](../../../src/tools/selection.rs#L1275), [`ALT`](../../../src/tools/selection.rs#L1279), [`SHIFT_ALT`](../../../src/tools/selection.rs#L1283)
