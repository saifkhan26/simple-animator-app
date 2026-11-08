# `src/doc/camera.rs` — the camera (what each exported frame shows)

> **Source:** [src/doc/camera.rs](../../../src/doc/camera.rs) · **Module:** `crate::doc::camera` · **Size:** 916 lines · Line links as of commit `67a7cb9`

## What this file is for

Layers can sit anywhere in "document space", and the **camera** decides which rectangle of it ends up
in each exported frame — like a film camera over an animation stand. Its frame is the output size (the
canvas size unless given its own size), and it can **pan** (`tx`, `ty`), **zoom** (`zoom`, 2 = pushed in
so the frame covers half as much) and **roll** (`rot`, radians). Camera keys animate it over time, with
easing, optionally along a smooth curve, and a move can be stretched over several loops of the
drawings.

The file also has the geometry for editing the camera on the canvas: which part of its frame a press
grabbed (`grab_at`), what a drag does (`dragged`), drawing a new frame as a box (`boxed`), and keeping
the frame inside the canvas (`kept_inside`).

```text
document space                         exported frame
┌───────────────────────────────┐
│   layers anywhere…            │      ┌─────────┐
│        ┌ ─ ─ ─ ─ ┐  camera    │ ───► │         │  output_size()
│          frame      (tx,ty,   │      │         │
│        └ ─ ─ ─ ─ ┘  zoom,rot) │      └─────────┘
└───────────────────────────────┘
```

## How it connects

- **Used by:** [doc/project.rs](project.md) (`Project::camera`, `camera_keys`, `camera_opts`,
  `resolve_camera`), [app.rs](../app.md) (camera edit/box modes), [ui/shell.rs](../ui/shell.md) (camera
  guide and panel), [io/composite.rs](../io/composite.md) (`Camera::apply` when rendering export frames),
  [io/frame_order.rs](../io/frame_order.md), [timeline/playback.rs](../timeline/playback.md).
- **Uses:** [doc/transform.rs](transform.md) (`Transform`, `Ease`).

## Rust concepts in this file

**Copy structs passed by value.** `Camera` is small and `Copy`, so methods take `self` (not `&self`) and
return a modified copy: `cam.centred_on(p, canvas)` doesn't change `cam`, it returns a new camera — like
working with immutable values in JS.

**Closures for tiny local helpers.** `let l = |x: f32, y: f32| x + (y - x) * t;` defines a lerp used three
times inside `Camera::lerp`. More: [Closures](../../rust-primer.md#closures)

**Tuples for 2-D points.** `(f32, f32)` is used for points and sizes, unpacked with `let (w, h) = size;`.

## Types

### `struct Camera`
[src/doc/camera.rs:17](../../../src/doc/camera.rs#L17)

The camera's pose: pan offset from the document centre, zoom and roll. `Copy`, so it's passed around by
value.

| Field | Type | Meaning |
|---|---|---|
| `tx` | `f32` | Offset of the frame center from the document center, in document pixels. |
| `ty` | `f32` | Vertical offset of the frame centre from the document centre, in document pixels. |
| `zoom` | `f32` | How far in the camera is pushed. `2.0` = the frame covers half as much document as it does at rest (and the export upscales to compensate). |
| `rot` | `f32` | Camera roll in radians. |

### `struct CameraKey`
[src/doc/camera.rs:46](../../../src/doc/camera.rs#L46)

A camera keyframe. `ease` shapes the segment running from this key to the *next* one; it is ignored on the last key.

| Field | Type | Meaning |
|---|---|---|
| `frame` | `usize` | The frame the key is on. |
| `camera` | `Camera` | The camera pose at that frame. |
| `ease` | `Ease` | How motion eases out of this key toward the next. |

### `enum Fill`
[src/doc/camera.rs:56](../../../src/doc/camera.rs#L56)

What the drawings do while a camera move that spans several passes of them is still going — see `CameraOpts::loops`.

| Variant | Meaning |
|---|---|
| `Loop` | Start again from the first frame each pass: a cycle. |
| `PingPong` | Forward, then back, then forward again. |
| `Hold` | Play once and stay on the last frame. |

### `const Fill::ALL`
[src/doc/camera.rs:67](../../../src/doc/camera.rs#L67)

```rust
pub const ALL: [Fill; 3] = [Fill::Loop, Fill::PingPong, Fill::Hold];
```

All three `Fill` choices, in menu order.

### `struct CameraOpts`
[src/doc/camera.rs:82](../../../src/doc/camera.rs#L82)

Camera settings that last for the session. Not written to `.anim`, so a project always opens with the defaults — and the defaults are the camera as it has always been.

| Field | Type | Meaning |
|---|---|---|
| `size` | `Option<(u32, u32)>` | Size of the camera's frame, which is the size of every exported frame. `None` = the canvas size, the frame a resting camera has always had. Smaller than the canvas, it is a window to pan around it. |
| `keep_inside` | `bool` | Hold the whole frame on the canvas: pushed in and moved over by the least that takes, so no frame — keyed, tweened or dragged — shows past the canvas's edge. See `Camera::kept_inside`. Off here; the app turns it on for each project it starts or opens, unless that project's camera already leaves the canvas (`crate::doc::project::Project::keep_inside_by_default`). |
| `smooth` | `bool` | Curve through the keys instead of turning a corner at each: see `Camera::resolve_at`. Changes nothing until there are three keys. |
| `loops` | `u32` | How many passes of the animation the camera's move lasts. `1` = the two run together, frame for frame. More, and the move keyed along the timeline is stretched over that many passes of it: the drawings come round again (as `fill` has them) under one unbroken move, in playback and in exports. See `crate::io::frame_order::Shot`. |
| `fill` | `Fill` | What the drawings do while a stretched camera move is still going (loop, ping-pong, hold). |

### `enum CamGrab`
[src/doc/camera.rs:123](../../../src/doc/camera.rs#L123)

Which part of the camera's frame a press landed on. The same grabs, by the same rules, as a selection's transform box (`crate::tools::selection::grab_at`) — less its one-axis handles, which a frame of a fixed shape has no use for.

| Variant | Meaning |
|---|---|
| `Move` | Inside the frame: the drag moves the camera. |
| `Zoom` | A corner: dragging it away from the centre widens the frame. |
| `Roll` | Just outside a corner: the drag rolls the camera about its centre. |

### `const Camera::ZOOM_MIN`
[src/doc/camera.rs:134](../../../src/doc/camera.rs#L134)

```rust
pub const ZOOM_MIN: f32 = 0.05;
```

How far the camera pulls out and pushes in.

### `const Camera::ZOOM_MAX`
[src/doc/camera.rs:135](../../../src/doc/camera.rs#L135)

```rust
pub const ZOOM_MAX: f32 = 64.0;
```

The largest zoom allowed (64×); see `ZOOM_MIN`.

## `impl Default for Camera`

### `Camera::default`
[src/doc/camera.rs:29](../../../src/doc/camera.rs#L29)

```rust
fn default() -> Self
```

The resting camera: no offset, zoom 1, no roll — the frame exactly covers the canvas.

## `impl Fill`

### `Fill::label`
[src/doc/camera.rs:69](../../../src/doc/camera.rs#L69)

```rust
pub fn label(self) -> &'static str
```

The menu label for a fill mode.

## `impl Default for CameraOpts`

### `CameraOpts::default`
[src/doc/camera.rs:107](../../../src/doc/camera.rs#L107)

```rust
fn default() -> Self
```

Session defaults: canvas-sized frame, keep-inside off (the app turns it on per project), no smoothing, one
pass, loop fill.

## `impl Camera`

### `Camera::is_identity`
[src/doc/camera.rs:137](../../../src/doc/camera.rs#L137)

```rust
pub fn is_identity(&self) -> bool
```

Whether the camera is at rest (no pan, zoom 1, no roll).

### `Camera::lerp`
[src/doc/camera.rs:144](../../../src/doc/camera.rs#L144)

```rust
pub fn lerp(a: Camera, b: Camera, t: f32) -> Camera
```

Interpolates between two cameras at `t` (0–1). Pan and roll move linearly; zoom moves geometrically
(`za × (zb/za)^t`), so a 1→4 push looks like a constant speed: halfway is ×2, not ×2.5.

### `Camera::resolve`
[src/doc/camera.rs:158](../../../src/doc/camera.rs#L158)

```rust
pub fn resolve(keys: &[CameraKey], static_cam: Camera, frame: usize) -> Camera
```

The camera on `frame`: the static camera when there are no keys; otherwise eased interpolation between
the keys around `frame`, held flat before the first and after the last.

### `Camera::resolve_at`
[src/doc/camera.rs:189](../../../src/doc/camera.rs#L189)

```rust
pub fn resolve_at(keys: &[CameraKey], static_cam: Camera, t: f32, smooth: bool) -> Camera
```

`resolve` at any fractional time `t`. With `smooth` and three or more keys, the frame's centre follows a
smooth curve (cubic Hermite segments) through the keys instead of straight lines with corners.

### `Camera::apply`
[src/doc/camera.rs:240](../../../src/doc/camera.rs#L240)

```rust
pub fn apply(&self, l: &Transform) -> Transform
```

Combines the camera with a layer's transform into one "cell → exported frame" transform. Both are
similarity transforms (move + uniform scale + rotate), and two of those compose into one, so the camera
adds no cost when rendering.

### `Camera::rolled_half`
[src/doc/camera.rs:254](../../../src/doc/camera.rs#L254)

```rust
fn rolled_half(&self, out: (f32, f32)) -> (f32, f32)
```

Half the width/height of the box a rolled frame of size `out` occupies (a rotated rectangle's bounding
box).

### `Camera::fill_zoom`
[src/doc/camera.rs:263](../../../src/doc/camera.rs#L263)

```rust
pub fn fill_zoom(&self, canvas: (f32, f32), out: (f32, f32)) -> f32
```

The smallest zoom at which a frame of size `out` (rolled like this camera) lies completely on the canvas.

### `Camera::fit_zoom`
[src/doc/camera.rs:270](../../../src/doc/camera.rs#L270)

```rust
pub fn fit_zoom(&self, canvas: (f32, f32), out: (f32, f32)) -> f32
```

The largest zoom at which the frame, centred, takes in the whole canvas.

### `Camera::kept_inside`
[src/doc/camera.rs:281](../../../src/doc/camera.rs#L281)

```rust
pub fn kept_inside(self, canvas: (f32, f32), out: (f32, f32)) -> Camera
```

This camera adjusted so its whole frame stays on the canvas: zoomed in until it fits, then moved by the
least amount so nothing hangs off an edge. A camera already inside is returned unchanged.

### `Camera::centre`
[src/doc/camera.rs:295](../../../src/doc/camera.rs#L295)

```rust
fn centre(&self, canvas: (f32, f32)) -> (f32, f32)
```

The frame's centre point in document coordinates.

### `Camera::grab_at`
[src/doc/camera.rs:306](../../../src/doc/camera.rs#L306)

```rust
pub fn grab_at(&self, p: (f32, f32), canvas: (f32, f32), out: (f32, f32), tol: f32) -> Option<CamGrab>
```

What a press at document point `p` grabs: a corner (zoom), the ring just outside a corner (roll), or the
inside (move); `None` if it misses. Tested in the frame's own rotated coordinates, so it's a plain
rectangle check. `tol` keeps handles a constant size on screen.

### `Camera::dragged`
[src/doc/camera.rs:330](../../../src/doc/camera.rs#L330)

```rust
pub fn dragged(self, grab: CamGrab, from: (f32, f32), to: (f32, f32), canvas: (f32, f32)) -> Camera
```

The camera after dragging its frame with `grab` from `from` to `to`: moved by the drag, or zoomed/rolled
about the centre so the grabbed corner follows the cursor.

### `Camera::boxed`
[src/doc/camera.rs:362](../../../src/doc/camera.rs#L362)

```rust
pub fn boxed(self, from: (f32, f32), to: (f32, f32), canvas: (f32, f32), out: (f32, f32)) -> Camera
```

The camera whose frame is the box dragged from `from` to `to` (Draw camera mode), keeping the frame's
aspect ratio and the camera's roll, with the pressed corner staying put; limited by the zoom limits.

### `Camera::centred_on`
[src/doc/camera.rs:383](../../../src/doc/camera.rs#L383)

```rust
pub fn centred_on(self, at: (f32, f32), canvas: (f32, f32)) -> Camera
```

This camera with its frame's middle on document point `at`.

### `Camera::as_frame_transform`
[src/doc/camera.rs:394](../../../src/doc/camera.rs#L394)

```rust
pub fn as_frame_transform(&self) -> Transform
```

The camera frame expressed as a `Transform` placing a frame-sized "cell" in the document — so the editor
can reuse its cell-corner helpers to draw the camera guide.

## Tests

### `mod tests`

Run with `cargo test doc::camera::tests` · 24 tests

| Test | What it checks |
|---|---|
| [`identity_camera_is_a_noop`](../../../src/doc/camera.rs#L415) | The identity camera must leave layer transforms untouched, so existing projects composite through the exact same path as before. |
| [`pan_shifts_layers_opposite`](../../../src/doc/camera.rs#L427) | A pure pan shifts every layer by exactly minus the camera offset. |
| [`apply_agrees_with_frame_transform`](../../../src/doc/camera.rs#L442) | `apply` and `as_frame_transform` must describe the same camera: mapping a frame corner to doc space and back through `apply` returns it. |
| [`a_frame_of_its_own_size_folds_about_its_centre`](../../../src/doc/camera.rs#L470) | A frame of its own size is still placed by its centre: one of its pixels mapped into the document, onto a layer's cell and back through the fold lands where it started, with the frame's size standing where the canvas's would. |
| [`a_camera_inside_the_canvas_is_left_exactly_as_it_was`](../../../src/doc/camera.rs#L503) | A camera already showing nothing but canvas is not touched — to the bit, so turning the hold on changes no frame that didn't need it. |
| [`a_camera_past_the_edge_is_moved_back_onto_the_canvas`](../../../src/doc/camera.rs#L524) | A camera past the edge is moved back onto the canvas. |
| [`a_camera_pulled_out_too_far_is_pushed_in_until_it_fits`](../../../src/doc/camera.rs#L538) | A camera pulled out too far is pushed in until it fits. |
| [`a_rolled_frame_is_held_by_the_box_around_it`](../../../src/doc/camera.rs#L555) | A rolled frame is held by the box around it: on its side, a 100×50 frame is 50 wide and 100 tall. |
| [`fit_takes_in_the_canvas_and_fill_stays_on_it`](../../../src/doc/camera.rs#L571) | Fit takes in the whole canvas; fill shows nothing but canvas. On a frame the canvas's shape they are the same zoom. |
| [`a_press_grabs_the_inside_a_corner_or_the_ring_past_one`](../../../src/doc/camera.rs#L582) | A level 100×50 frame at zoom 1 in the middle of the 400×200 canvas covers x 150..250, y 75..125. |
| [`grabs_follow_a_moved_zoomed_and_rolled_frame`](../../../src/doc/camera.rs#L598) | The handles are found where the frame is drawn, however it is moved, pushed in and rolled. |
| [`dragging_moves_zooms_and_rolls_about_the_centre`](../../../src/doc/camera.rs#L617) | Dragging moves zooms and rolls about the centre. |
| [`a_dragged_corner_follows_the_cursor`](../../../src/doc/camera.rs#L644) | The corner under the cursor stays under it: after a zoom drag the grabbed corner is where the cursor went. |
| [`a_dragged_box_is_the_new_frame`](../../../src/doc/camera.rs#L675) | A box dragged corner to corner is the frame: the pressed corner where the press was, the far one where the drag went, the shape the frame's. Whichever way the drag goes, and whichever side it goes further along, the same box gives the same camera. |
| [`a_rolled_cameras_box_keeps_its_roll`](../../../src/doc/camera.rs#L696) | A rolled camera's box is drawn along its own axes, and keeps the roll. |
| [`a_tiny_box_stops_at_the_zoom_limit`](../../../src/doc/camera.rs#L719) | Past the closest the camera goes, the box is as small as it can be, still from the pressed corner. |
| [`centring_moves_only_the_middle`](../../../src/doc/camera.rs#L728) | Centring moves only the middle. |
| [`resolving_at_a_whole_frame_is_resolve`](../../../src/doc/camera.rs#L757) | The straight path at a time between frames is, on every whole frame, bit for bit what `resolve` has always given there. |
| [`resolving_between_frames_follows_the_segment`](../../../src/doc/camera.rs#L800) | Between frames the straight path is the straight line it is on them. |
| [`a_smooth_path_passes_through_its_keys`](../../../src/doc/camera.rs#L820) | The smooth path still goes exactly through every key. |
| [`a_smooth_path_rounds_the_corner_a_straight_one_turns`](../../../src/doc/camera.rs#L832) | The straight path turns the corner on the middle key in one frame; the smooth one is already turning as it arrives, and carries on the same way as it leaves. |
| [`smoothing_changes_nothing_with_fewer_than_three_keys`](../../../src/doc/camera.rs#L859) | Two keys have nothing to curve round: smooth or not, the same camera, to the bit. Nor does a camera with no keys, or a time outside them. |
| [`resolve_holds_and_eases`](../../../src/doc/camera.rs#L873) | Keys hold flat outside their range and ease within a segment. |
| [`zoom_is_geometric`](../../../src/doc/camera.rs#L904) | Zoom interpolates geometrically: the halfway point of 1 → 4 is 2, not 2.5. |

**Test helpers** (used only by the tests above):

- [`approx`](../../../src/doc/camera.rs#L408) — Approx.
- [`frame_at`](../../../src/doc/camera.rs#L666) — Where a point of the frame, in its own pixels, is in the document.
- [`key`](../../../src/doc/camera.rs#L742) — Key.
- [`corner`](../../../src/doc/camera.rs#L810) — Right, then down: three keys with a square corner on the middle one.

**Test-only types and constants:** [`CANVAS`](../../../src/doc/camera.rs#L497), [`FRAME`](../../../src/doc/camera.rs#L498)
