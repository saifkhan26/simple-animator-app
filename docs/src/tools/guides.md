# `src/tools/guides.rs` — snapping strokes to guides

> **Source:** [src/tools/guides.rs](../../../src/tools/guides.rs) · **Module:** `crate::tools::guides` · **Size:** 1358 lines · Line links as of commit `84b26e8`

## What this file is for

When **Snap strokes** is on, a pen stroke is held to a guide: a perspective grid's lines, a ruler, a
vanishing point, an ellipse. This file is the part that decides *which* curve a stroke keeps to and
moves each stroke point onto it. The app side (when to arm it, the snap strength, freehand mode) is
`AppState::snap_begin` / `snap_doc` in [app.rs](../app.md).

Every **visible guide whose magnet is on** offers curves — `Candidate`s — of two sorts:

- **Through the start.** A line through the point where the stroke went down: toward a vanishing
  point, or along a direction (a flat grid's rows, Parallel lines). An Ellipse with *Rings* also
  offers the ring through the start. These are *picked* once the pointer has moved far enough to
  show where it is heading (`pick`): the one whose direction there best matches the motion.
- **Fixed.** A curve that sits on the canvas whatever the stroke does: a Ruler's segment (or whole
  line, when Infinite), an Ellipse's ring. A press within a few screen pixels of one takes it at
  once (`nearest_fixed`), so "press on the ruler, draw along it" works from the first point.

Either way the pick is held for the rest of the stroke — re-picking per point would let a wobbly
hand flip between curves. Each curve is a `SnapCurve` that can `project` a point onto itself.

The file also holds the geometry of the non-plane guides: where a vanishing-point guide's helper
lines meet (`point_vp`), the ring of an ellipse guide (`ellipse`), and how their handles are grabbed
(`grab_at`) and dragged (`dragged`). The planes' own geometry is in
[perspective.rs](perspective.md).

### Ellipses in perspective

An ellipse guide is stored like a grid: four corners, which define a **homography** (see
[perspective.rs](perspective.md)) from the unit square. The guide is the image of the circle
inscribed in that square — centre `(0.5, 0.5)`, radius `0.5`. A rectangle gives a plain ellipse; a
floor-like quad gives a circle seen in perspective, touching each edge at its middle (in depth).
Projecting a point onto it is done in the square: undo the map (`unmap`), push the point straight
out from the centre to the ring, map it back.

### Curved guides: pans and the fisheye

The four curved kinds — after Toon Boom Harmony's 3-point horizontal and vertical pan, 4-point
continuous and 5-point fisheye guides — keep the same four corners, as a level box round a **centre
point**. `Frame::of` reads the box back: the centre `o`, unit `u` along the horizon, unit `n` square
to it, half-width `w` and half-height `h`. The **side points** sit where the box's edges cross the
horizon (and, for the vertical pan and the fisheye, the line through the centre square to it).

- **Pans** bend their lines into `SnapCurve::Wave`s. In the pan's own axis (`pan_axis`: along `u`
  for a horizontal or 360° pan, along `n` for a vertical one) a curve is the graph
  `y = amp · wave(x / a − shift)`. `wave` is an `arch` over the span between two side points, the
  next span's arch hanging below, so it runs on for ever with a cycle of four spans — which is what
  lets a 360° pan loop. With `shift` 0 the curves run through the side points (the lines across the
  view); with `shift` 1 the same wave runs through the centre point (the lines going away). A
  third family stays straight (`PanAxis::straight`). A camera turning on the spot sees a level line
  as a cosine on a cylindrical panorama; the `arch` at bend 0, a parabola, is close to it, and
  `PerspectiveGrid::bend` (the diamond handle) makes the arches fuller or flatter. Projecting a point
  finds the true nearest point of the graph: it can only lie as far along the axis as the curve is
  from the point straight across, so that stretch is sampled, then narrowed by golden-section search.
- **The fisheye** bends its lines into `SnapCurve::Arc`s, the classic five-point construction:
  circles through two opposite side points, in the box scaled to a square, so the widest one — apex
  1 — is the 180° rim through all four. A point projects straight out from its circle's middle in
  that scaled frame, as an ellipse ring does in its square. Depth lines are plain `Line`s to the
  centre.

A press on a curved guide (`grab_at`) takes the centre point to move it, a side point
(`side_points`) to stretch it — its partner mirrors it, and Shift scales the whole guide — a pan's
shape handle (`bend_handle`, set by `bend_at`), or else the horizon to turn it.

## Types

### `enum SnapCurve`
[src/tools/guides.rs:37](../../../src/tools/guides.rs#L37)

A curve a stroke can be held to, in document space.

| Variant | Meaning |
|---|---|
| `Line { origin: P, dir: P, vp: Option<P> }` | The infinite line through `origin` along unit `dir`. `vp` is the vanishing point it runs to, if finite — past it the line means nothing, so its guide stops there. |
| `Segment { a: P, b: P }` | The segment from `a` to `b`: points past an end stay on that end. |
| `Conic { h: Homography, r: f32 }` | The image under `h` of the circle of radius `r` about the middle of the unit square: a circle seen in perspective, or — when `h` is affine — a plain ellipse. |
| `Wave { o: P, u: P, n: P, a: f32, shift: f32, amp: f32, bend: f32, span: f32 }` | A pan's curve, in the frame at `o` along unit `u` and unit `n` square to it: the graph `y = amp · wave(x / a − shift)`, where the wave is an arch of height 1 over every span 2 wide between its zeros at the odd numbers, above and below by turns, shaped by `bend` (see `arch`). So its points are `a` apart, at `x = (shift ± 1) · a`, `(shift ± 3) · a`… It is drawn `span` points' spacing either side of `o`. |
| `Arc { o: P, u: P, n: P, a: f32, b: f32, apex: f32 }` | A fisheye's curve: in the frame at `o` along `u` and `n`, scaled by `a` along `u` and `b` along `n`, the circle through the side points (±1, 0) whose middle point — on the axis through `o` along `n` — is `apex`. An ellipse arc, square to the box. |

### `const BEND`
[src/tools/guides.rs:63](../../../src/tools/guides.rs#L63)

```rust
pub const BEND: std::ops::RangeInclusive<f32> = -0.75..=1.0;
```

Flattest and fullest a pan's arches may be — see `PerspectiveGrid::bend`.

### `struct Frame`
[src/tools/guides.rs:98](../../../src/tools/guides.rs#L98)

A curved guide's own frame, read off its box: the centre point `o`, unit `u` along its horizon and unit `n` square to it toward the box's bottom edge, and how far the box reaches along each — `w` to its sides, `h` to its top and bottom.

| Field | Type | Meaning |
|---|---|---|
| `o` | `P` | The centre point, in document space. |
| `u` | `P` | Unit direction along the horizon, left to right across the box. |
| `n` | `P` | Unit direction square to the horizon, toward the box's bottom edge. |
| `w` | `f32` | Half the box's width: how far its side edges are along `u`. |
| `h` | `f32` | Half the box's height: how far its top and bottom are along `n`. |

### `const CONIC_SAMPLES`
[src/tools/guides.rs:179](../../../src/tools/guides.rs#L179)

```rust
pub const CONIC_SAMPLES: usize = 96;
```

Points round a conic, for drawing it and for projecting onto it where the map can't be undone.

### `const CENTRE`
[src/tools/guides.rs:181](../../../src/tools/guides.rs#L181)

```rust
const CENTRE: P = [0.5, 0.5];
```

The middle of the unit square: the centre of every ring an ellipse guide draws, in the square's own
`(u, v)` coordinates.

### `struct Candidate`
[src/tools/guides.rs:395](../../../src/tools/guides.rs#L395)

One way a stroke could go, and the guide that offers it.

| Field | Type | Meaning |
|---|---|---|
| `grid` | `usize` | Index of the guide in the file's list. |
| `curve` | `SnapCurve` | The curve itself, in document space. |
| `fixed` | `bool` | Sits on the canvas whatever the stroke does (a ruler, an ellipse), rather than running through the stroke's start. |

### `struct PanAxis`
[src/tools/guides.rs:470](../../../src/tools/guides.rs#L470)

Which way a pan bends, in its box's `Frame`: its curves are graphs across `u`, out along `n`, with side points `a` apart along `u`; drawn `span` side points' spacing either side; its third family straight along `straight`. Its box reaches `reach` out along `n` either side: the outermost arch drawn, which carries the curve-shape handle on its `side` (−1 or 1 along `n`).

| Field | Type | Meaning |
|---|---|---|
| `u` | `P` | Along the line through the side points. |
| `n` | `P` | Across it: the way the curves bow out. |
| `a` | `f32` | From the centre point to the nearest side point. |
| `span` | `f32` | How many `a`s either side of the centre the curves are drawn: 1 for a three-point pan, unbounded for a 360° one. |
| `straight` | `P` | The direction of the family that stays straight. |
| `reach` | `f32` | How far the box reaches along `n`: the outermost arch drawn firm. |
| `side` | `f32` | Which side of the axis the shape handle sits, −1 or 1 along `n`. |

## Free functions

### `arch`
[src/tools/guides.rs:71](../../../src/tools/guides.rs#L71)

```rust
pub fn arch(bend: f32, t: f32) -> f32
```

A pan's arch at `t` in −1..=1, 1 high at 0 and 0 at the ends: `1 − (1 − c)t² − ct⁴` for `bend` `c`. At 0 a parabola; toward 1 the shoulders fill out and it meets its ends steeply; below 0 it flattens and meets them at a shallow angle. Whatever the bend it falls all the way, and its arches cross at their points at an angle that grows with their height — as lines running to a vanishing point do.

### `arch_slope`
[src/tools/guides.rs:78](../../../src/tools/guides.rs#L78)

```rust
fn arch_slope(bend: f32, t: f32) -> f32
```

The slope of `arch` at `t`.

### `wave`
[src/tools/guides.rs:86](../../../src/tools/guides.rs#L86)

```rust
pub fn wave(bend: f32, t: f32) -> (f32, f32)
```

The wave a pan's curves follow, and its slope, at `t`: `arch` over every span from one odd number to the next, upside down every other span, so it runs on for ever, 4 to a cycle.

## `impl Frame`

### `Frame::of`
[src/tools/guides.rs:115](../../../src/tools/guides.rs#L115)

```rust
pub fn of(c: &[P; 4]) -> Option<Self>
```

The frame of a curved guide's corners. `n` is taken from the box, not from `u`, so a mirrored space keeps its handedness. `None` for a box with no width or height.

### `Frame::at`
[src/tools/guides.rs:127](../../../src/tools/guides.rs#L127)

```rust
pub fn at(&self, x: f32, y: f32) -> P
```

The document point `x` along `u` and `y` along `n` from the centre.

### `Frame::corners`
[src/tools/guides.rs:135](../../../src/tools/guides.rs#L135)

```rust
pub fn corners(&self) -> [P; 4]
```

The box this frame reads off, TL, TR, BR, BL.

## Free functions

### `wave_through`
[src/tools/guides.rs:145](../../../src/tools/guides.rs#L145)

```rust
pub fn wave_through(o: P, u: P, n: P, a: f32, shift: f32, bend: f32, span: f32, p: P) -> Option<SnapCurve>
```

The wave of a pan's family through `p`: `None` when `p` sits square over one of its points, where no curve of the family passes but the straight line square to the axis.

### `arc_through`
[src/tools/guides.rs:157](../../../src/tools/guides.rs#L157)

```rust
pub fn arc_through(o: P, u: P, n: P, a: f32, b: f32, p: P) -> SnapCurve
```

A fisheye's arc through `p`, round the side points `o ± a·u`, in a box `b` deep along `n`. On the axis itself, the straight line along it.

### `arc_circle`
[src/tools/guides.rs:172](../../../src/tools/guides.rs#L172)

```rust
fn arc_circle(apex: f32) -> (f32, f32)
```

The middle and radius, in its scaled frame, of the circle an arc is part of.

## `impl SnapCurve`

### `SnapCurve::project`
[src/tools/guides.rs:185](../../../src/tools/guides.rs#L185)

```rust
pub fn project(&self, p: P) -> P
```

The point of the curve a stroke at `p` is held to.

### `SnapCurve::wave_nearest`
[src/tools/guides.rs:228](../../../src/tools/guides.rs#L228)

```rust
fn wave_nearest(&self, p: P) -> (f32, f32)
```

The nearest point of a wave to `p`, as `(x, y)` in its frame. It lies no farther along the axis than the curve lies from `p` straight across, so that stretch is searched — coarsely, then closer in.

### `SnapCurve::tangent_at`
[src/tools/guides.rs:262](../../../src/tools/guides.rs#L262)

```rust
pub fn tangent_at(&self, p: P) -> Option<P>
```

The curve's unit direction at the point nearest `p`.

### `SnapCurve::distance`
[src/tools/guides.rs:295](../../../src/tools/guides.rs#L295)

```rust
pub fn distance(&self, p: P) -> f32
```

How far `p` is from the curve.

### `SnapCurve::through`
[src/tools/guides.rs:304](../../../src/tools/guides.rs#L304)

```rust
pub fn through(self, start: P) -> Self
```

The same curve, through `start` instead, for a line that runs through a stroke's start — what a shape slid mid-drag keeps to. Fixed curves stay where they are.

### `SnapCurve::bend_points`
[src/tools/guides.rs:319](../../../src/tools/guides.rs#L319)

```rust
pub fn bend_points(&self, lo: P, hi: P) -> Vec<Vec<P>>
```

A pan's or a fisheye's curve as polylines in document space, for drawing over the box `lo`–`hi`: a wave where it crosses the box, out to its `span`, broken where it leaves the box far behind; an arc from one side point to the other.

### `SnapCurve::conic_points`
[src/tools/guides.rs:383](../../../src/tools/guides.rs#L383)

```rust
pub fn conic_points(&self) -> Vec<P>
```

A conic as a closed polyline in document space, skipping any part that would fall behind the viewer.

## Free functions

### `candidates`
[src/tools/guides.rs:412](../../../src/tools/guides.rs#L412)

```rust
pub fn candidates(g: &PerspectiveGrid, s: &Space, start: P, vertical: bool, grid: usize) -> Vec<Candidate>
```

Everything guide `g` (at `grid` in the list) offers a stroke that starts at `start`. `vertical` adds a perspective plane's vertical.

- A ruler is fixed: its segment, or with `infinite_ruler` its whole line.
- Parallel lines run through the start along the guide's line.
- A vanishing point's line runs from the start to the point.
- An ellipse is fixed; with `rings`, a stroke starting inside its quad may also go round the ring through its start.

### `pan_axis`
[src/tools/guides.rs:494](../../../src/tools/guides.rs#L494)

```rust
pub fn pan_axis(kind: GridKind, f: &Frame) -> Option<PanAxis>
```

A pan's axis. A horizontal pan's side points are at its box's sides and its verticals stay straight; a vertical pan's are at the top and bottom and its horizontals stay straight. A 360° pan's box is a whole turn, its side points a quarter turn apart, and its curves run on past it. The shape handle sits top right, or on a vertical pan bottom right, as in Harmony.

### `bend_handle`
[src/tools/guides.rs:522](../../../src/tools/guides.rs#L522)

```rust
pub fn bend_handle(kind: GridKind, bend: f32, c: &[P; 4]) -> Option<P>
```

Where a pan's curve-shape handle sits: on its outermost arch, half way from the middle to a side point.

### `bend_at`
[src/tools/guides.rs:531](../../../src/tools/guides.rs#L531)

```rust
pub fn bend_at(kind: GridKind, c: &[P; 4], p: P) -> Option<f32>
```

The bend that puts a pan's shape handle as near `p` as it can go: out toward the box's edge for fuller arches, in for flatter ones.

### `side_points`
[src/tools/guides.rs:543](../../../src/tools/guides.rs#L543)

```rust
pub fn side_points(kind: GridKind, c: &[P; 4]) -> Vec<(P, GridGrab)>
```

A curved guide's side points, each with the grab that stretches it: a pan's two, a 360° pan's four (the inner pair a quarter turn out, the outer the box's ends), a fisheye's four. See `dragged`.

### `curved_candidates`
[src/tools/guides.rs:566](../../../src/tools/guides.rs#L566)

```rust
fn curved_candidates(g: &PerspectiveGrid, c: &[P; 4], start: P, grid: usize) -> Vec<Candidate>
```

A curved guide's families through the start. A pan's: its arch through the side points (the lines across), the same wave a span along through the centre point (the lines going away), and its straight lines. A fisheye's: its arcs through the side points either way, and the line to the centre.

### `plane_candidates`
[src/tools/guides.rs:595](../../../src/tools/guides.rs#L595)

```rust
fn plane_candidates(g: &PerspectiveGrid, s: &Space, start: P, vertical: bool, grid: usize) -> Vec<Candidate>
```

A plane's families through the start, toward their vanishing points or along their directions, and its extra vanishing points.

### `pick`
[src/tools/guides.rs:615](../../../src/tools/guides.rs#L615)

```rust
pub fn pick(cands: &[Candidate], start: P, motion: P) -> Option<Candidate>
```

Of the curves through the start, the one whose direction there best matches `motion`, either way along it. Ties go to the later one, as `super::perspective::pick_dir` has it.

### `nearest_fixed`
[src/tools/guides.rs:626](../../../src/tools/guides.rs#L626)

```rust
pub fn nearest_fixed(cands: &[Candidate], p: P, tol: f32) -> Option<Candidate>
```

The fixed curve nearest `p`, if one is within `tol`.

### `pull`
[src/tools/guides.rs:638](../../../src/tools/guides.rs#L638)

```rust
pub fn pull(raw: P, to: P, strength: f32) -> P
```

`raw` pulled `strength` of the way toward `to`: all the way at 1, not at all at 0.

### `point_vp`
[src/tools/guides.rs:648](../../../src/tools/guides.rs#L648)

```rust
pub fn point_vp(c: &[P; 4]) -> Option<P>
```

Where a vanishing-point guide's helper lines meet: its quad's left edge (corners 0–3) and right edge (1–2). `None` while they run parallel.

### `ellipse`
[src/tools/guides.rs:654](../../../src/tools/guides.rs#L654)

```rust
pub fn ellipse(c: &[P; 4]) -> Option<SnapCurve>
```

The ring an ellipse guide draws and snaps to, from its corners in document space.

### `grab_at`
[src/tools/guides.rs:668](../../../src/tools/guides.rs#L668)

```rust
pub fn grab_at(kind: GridKind, bend: f32, c: &[P; 4], p: P, tol: f32) -> Option<GridGrab>
```

What a press at `p` takes hold of on a guide that isn't a plane, within `tol` document units. Planes go through `perspective::grab_at`.

- A ruler: its ends, else its line to move it.
- A vanishing point: the point, a helper's end, else a helper to move it.
- An ellipse: a corner, inside to move it, just outside a corner to turn it.

### `dragged`
[src/tools/guides.rs:712](../../../src/tools/guides.rs#L712)

```rust
pub fn dragged(kind: GridKind, c0: &[P; 4], grab: GridGrab, start: P, now: P, snap: bool) -> Option<[P; 4]>
```

A non-plane guide's corners after dragging `grab` from `start` to `now`, solved against the corners at the press. `snap` (Shift, once dragging) turns a ruler or a helper line in 15° steps about its other end. `None` for a drag that would leave the guide meaningless — a ruler of no length, helper lines that never meet.

### `ruler_ticks`
[src/tools/guides.rs:784](../../../src/tools/guides.rs#L784)

```rust
pub fn ruler_ticks(divisions: u32, subdivisions: u32, t0: f32, t1: f32, max: usize) -> Vec<(f32, bool)>
```

A ruler's ticks between `t0` and `t1`, along it from one end (0) to the other (1): `(t, major)` for each, the major ones splitting it into `divisions` equal parts and the minor ones each part into `subdivisions`. The ends themselves are left out — the ruler marks those its own way — and an infinite ruler, asking past them, gets the same spacing on and on. At most `max` ticks; none if that would be more.

### `turn_in_steps`
[src/tools/guides.rs:802](../../../src/tools/guides.rs#L802)

```rust
fn turn_in_steps(o: P, p: P) -> P
```

`p` turned about `o` to the nearest 15° step, at the same distance.

### `parallel`
[src/tools/guides.rs:812](../../../src/tools/guides.rs#L812)

```rust
fn parallel(a: P, b: P) -> bool
```

Whether two vectors run the same way or opposite, near enough.

### `angle_of`
[src/tools/guides.rs:817](../../../src/tools/guides.rs#L817)

```rust
fn angle_of(uv: P) -> f32
```

Angle of `uv` about the square's middle.

### `on_circle`
[src/tools/guides.rs:822](../../../src/tools/guides.rs#L822)

```rust
fn on_circle(h: &Homography, r: f32, a: f32) -> Option<P>
```

The point at angle `a` round the circle, mapped onto the canvas.

### `nearest_sample`
[src/tools/guides.rs:827](../../../src/tools/guides.rs#L827)

```rust
fn nearest_sample(h: &Homography, r: f32, p: P) -> Option<P>
```

The fallback for projecting onto a ring when `p` can't be mapped back into the square (it lies past
the plane's horizon): of `CONIC_SAMPLES` points round the ring, the one nearest `p`. `None` if every
sample falls behind the viewer.

## Tests

### `mod tests`

Run with `cargo test tools::guides::tests` · 26 tests

| Test | What it checks |
|---|---|
| [`a_line_projects_square_onto_itself`](../../../src/tools/guides.rs#L854) | A line projects square onto itself. |
| [`a_segment_holds_points_past_its_ends_on_the_ends`](../../../src/tools/guides.rs#L860) | A segment holds points past its ends on the ends. |
| [`a_conic_in_a_square_is_its_inscribed_circle`](../../../src/tools/guides.rs#L868) | A conic in a square is its inscribed circle. |
| [`a_conic_in_perspective_touches_each_edge_at_its_middle_in_depth`](../../../src/tools/guides.rs#L881) | A conic in perspective touches each edge at its middle in depth. |
| [`the_pick_follows_the_motion_and_skips_fixed_curves`](../../../src/tools/guides.rs#L902) | The pick follows the motion and skips fixed curves. |
| [`a_press_takes_the_nearest_fixed_curve_in_reach`](../../../src/tools/guides.rs#L921) | A press takes the nearest fixed curve in reach. |
| [`a_ruler_is_fixed_and_an_infinite_one_runs_on`](../../../src/tools/guides.rs#L937) | A ruler is fixed and an infinite one runs on. |
| [`parallel_lines_and_a_point_run_through_the_start`](../../../src/tools/guides.rs#L951) | Parallel lines and a point run through the start. |
| [`an_ellipse_offers_rings_only_from_inside_it`](../../../src/tools/guides.rs#L965) | An ellipse offers rings only from inside it. |
| [`a_ruler_end_drags_alone_and_shift_turns_it_in_steps`](../../../src/tools/guides.rs#L981) | A ruler end drags alone and shift turns it in steps. |
| [`dragging_a_vanishing_point_swings_its_helpers_round_their_far_ends`](../../../src/tools/guides.rs#L995) | Dragging a vanishing point swings its helpers round their far ends. |
| [`a_pan_wave_crosses_at_its_points_and_runs_on_for_ever`](../../../src/tools/guides.rs#L1014) | A pan wave crosses at its points and runs on for ever. |
| [`a_wave_holds_points_on_its_graph_at_the_nearest_spot`](../../../src/tools/guides.rs#L1053) | A wave holds points on its graph at the nearest spot. |
| [`a_wave_on_the_horizon_is_the_horizon_and_none_stands_over_a_point`](../../../src/tools/guides.rs#L1080) | A wave on the horizon is the horizon and none stands over a point. |
| [`an_arc_in_a_square_box_is_a_circle_through_both_side_points`](../../../src/tools/guides.rs#L1093) | An arc in a square box is a circle through both side points. |
| [`a_fisheye_arc_in_a_long_box_passes_its_points_and_the_widest_is_the_rim`](../../../src/tools/guides.rs#L1119) | A fisheye arc in a long box passes its points and the widest is the rim. |
| [`a_horizontal_pan_bends_its_level_lines_through_the_side_points`](../../../src/tools/guides.rs#L1143) | A horizontal pan bends its level lines through the side points. |
| [`a_vertical_pan_bends_its_verticals_through_points_above_and_below`](../../../src/tools/guides.rs#L1172) | A vertical pan bends its verticals through points above and below. |
| [`a_360_pan_puts_four_points_to_its_box_and_runs_on_past_it`](../../../src/tools/guides.rs#L1183) | A 360 pan puts four points to its box and runs on past it. |
| [`a_fisheye_arcs_both_ways_and_runs_depth_lines_to_its_centre`](../../../src/tools/guides.rs#L1203) | A fisheye arcs both ways and runs depth lines to its centre. |
| [`a_wave_draws_inside_its_span_and_stays_on_the_canvas`](../../../src/tools/guides.rs#L1223) | A wave draws inside its span and stays on the canvas. |
| [`a_curved_guide_moves_by_its_centre_stretches_by_its_points_and_turns_by_its_horizon`](../../../src/tools/guides.rs#L1237) | A curved guide moves by its centre stretches by its points and turns by its horizon. |
| [`each_curved_kind_has_its_own_side_points`](../../../src/tools/guides.rs#L1276) | Each curved kind has its own side points. |
| [`a_pans_shape_handle_sets_its_bend_from_how_far_out_it_is`](../../../src/tools/guides.rs#L1306) | A pans shape handle sets its bend from how far out it is. |
| [`a_ruler_ticks_its_divisions_and_their_parts`](../../../src/tools/guides.rs#L1330) | A ruler ticks its divisions and their parts. |
| [`pull_goes_part_way`](../../../src/tools/guides.rs#L1352) | Pull goes part way. |

**Test helpers** (used only by the tests above):

- [`near`](../../../src/tools/guides.rs#L841) — Near.
- [`line`](../../../src/tools/guides.rs#L845) — Line.
- [`fresh_doc`](../../../src/tools/guides.rs#L932) — Fresh doc.
- [`tilted_wave`](../../../src/tools/guides.rs#L1046) — A pan's arch family on a frame turned 30°, `a` = 100, through `p`.
- [`doc_frame`](../../../src/tools/guides.rs#L1138) — Doc frame.
