# `src/tools/guides.rs` — snapping strokes to guides

> **Source:** [src/tools/guides.rs](../../../src/tools/guides.rs) · **Module:** `crate::tools::guides` · **Size:** 550 lines · Line links as of commit `bdc0f67`

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

## Types

### `enum SnapCurve`
[src/tools/guides.rs:24](../../../src/tools/guides.rs#L24)

A curve a stroke can be held to, in document space.

| Variant | Meaning |
|---|---|
| `Line { origin: P, dir: P, vp: Option<P> }` | The infinite line through `origin` along unit `dir`. `vp` is the vanishing point it runs to, if finite — past it the line means nothing, so its guide stops there. |
| `Segment { a: P, b: P }` | The segment from `a` to `b`: points past an end stay on that end. |
| `Conic { h: Homography, r: f32 }` | The image under `h` of the circle of radius `r` about the middle of the unit square: a circle seen in perspective, or — when `h` is affine — a plain ellipse. |

### `const CONIC_SAMPLES`
[src/tools/guides.rs:39](../../../src/tools/guides.rs#L39)

```rust
pub const CONIC_SAMPLES: usize = 96;
```

Points round a conic, for drawing it and for projecting onto it where the map can't be undone.

### `const CENTRE`
[src/tools/guides.rs:41](../../../src/tools/guides.rs#L41)

```rust
const CENTRE: P = [0.5, 0.5];
```

The middle of the unit square: the centre of every ring an ellipse guide draws, in the square's own
`(u, v)` coordinates.

### `struct Candidate`
[src/tools/guides.rs:119](../../../src/tools/guides.rs#L119)

One way a stroke could go, and the guide that offers it.

| Field | Type | Meaning |
|---|---|---|
| `grid` | `usize` | Index of the guide in the file's list. |
| `curve` | `SnapCurve` | The curve itself, in document space. |
| `fixed` | `bool` | Sits on the canvas whatever the stroke does (a ruler, an ellipse), rather than running through the stroke's start. |

## `impl SnapCurve`

### `SnapCurve::project`
[src/tools/guides.rs:45](../../../src/tools/guides.rs#L45)

```rust
pub fn project(&self, p: P) -> P
```

The point of the curve a stroke at `p` is held to.

### `SnapCurve::tangent_at`
[src/tools/guides.rs:70](../../../src/tools/guides.rs#L70)

```rust
pub fn tangent_at(&self, p: P) -> Option<P>
```

The curve's unit direction at the point nearest `p`.

### `SnapCurve::distance`
[src/tools/guides.rs:90](../../../src/tools/guides.rs#L90)

```rust
pub fn distance(&self, p: P) -> f32
```

How far `p` is from the curve.

### `SnapCurve::through`
[src/tools/guides.rs:98](../../../src/tools/guides.rs#L98)

```rust
pub fn through(self, start: P) -> Self
```

The same curve, through `start` instead, for a line that runs through a stroke's start — what a shape slid mid-drag keeps to. Fixed curves stay where they are.

### `SnapCurve::conic_points`
[src/tools/guides.rs:107](../../../src/tools/guides.rs#L107)

```rust
pub fn conic_points(&self) -> Vec<P>
```

A conic as a closed polyline in document space, skipping any part that would fall behind the viewer.

## Free functions

### `candidates`
[src/tools/guides.rs:136](../../../src/tools/guides.rs#L136)

```rust
pub fn candidates(g: &PerspectiveGrid, s: &Space, start: P, vertical: bool, grid: usize) -> Vec<Candidate>
```

Everything guide `g` (at `grid` in the list) offers a stroke that starts at `start`. `vertical` adds a perspective plane's vertical.

- A ruler is fixed: its segment, or with `infinite_ruler` its whole line.
- Parallel lines run through the start along the guide's line.
- A vanishing point's line runs from the start to the point.
- An ellipse is fixed; with `rings`, a stroke starting inside its quad may also go round the ring through its start.

### `plane_candidates`
[src/tools/guides.rs:186](../../../src/tools/guides.rs#L186)

```rust
fn plane_candidates(g: &PerspectiveGrid, s: &Space, start: P, vertical: bool, grid: usize) -> Vec<Candidate>
```

A plane's families through the start, toward their vanishing points or along their directions, and its extra vanishing points.

### `pick`
[src/tools/guides.rs:206](../../../src/tools/guides.rs#L206)

```rust
pub fn pick(cands: &[Candidate], start: P, motion: P) -> Option<Candidate>
```

Of the curves through the start, the one whose direction there best matches `motion`, either way along it. Ties go to the later one, as `super::perspective::pick_dir` has it.

### `nearest_fixed`
[src/tools/guides.rs:217](../../../src/tools/guides.rs#L217)

```rust
pub fn nearest_fixed(cands: &[Candidate], p: P, tol: f32) -> Option<Candidate>
```

The fixed curve nearest `p`, if one is within `tol`.

### `pull`
[src/tools/guides.rs:229](../../../src/tools/guides.rs#L229)

```rust
pub fn pull(raw: P, to: P, strength: f32) -> P
```

`raw` pulled `strength` of the way toward `to`: all the way at 1, not at all at 0.

### `point_vp`
[src/tools/guides.rs:239](../../../src/tools/guides.rs#L239)

```rust
pub fn point_vp(c: &[P; 4]) -> Option<P>
```

Where a vanishing-point guide's helper lines meet: its quad's left edge (corners 0–3) and right edge (1–2). `None` while they run parallel.

### `ellipse`
[src/tools/guides.rs:245](../../../src/tools/guides.rs#L245)

```rust
pub fn ellipse(c: &[P; 4]) -> Option<SnapCurve>
```

The ring an ellipse guide draws and snaps to, from its corners in document space.

### `grab_at`
[src/tools/guides.rs:256](../../../src/tools/guides.rs#L256)

```rust
pub fn grab_at(kind: GridKind, c: &[P; 4], p: P, tol: f32) -> Option<GridGrab>
```

What a press at `p` takes hold of on a guide that isn't a plane, within `tol` document units. Planes go through `perspective::grab_at`.

- A ruler: its ends, else its line to move it.
- A vanishing point: the point, a helper's end, else a helper to move it.
- An ellipse: a corner, inside to move it, just outside a corner to turn it.

### `dragged`
[src/tools/guides.rs:284](../../../src/tools/guides.rs#L284)

```rust
pub fn dragged(kind: GridKind, c0: &[P; 4], grab: GridGrab, start: P, now: P, snap: bool) -> Option<[P; 4]>
```

A non-plane guide's corners after dragging `grab` from `start` to `now`, solved against the corners at the press. `snap` (Shift, once dragging) turns a ruler or a helper line in 15° steps about its other end. `None` for a drag that would leave the guide meaningless — a ruler of no length, helper lines that never meet.

### `turn_in_steps`
[src/tools/guides.rs:332](../../../src/tools/guides.rs#L332)

```rust
fn turn_in_steps(o: P, p: P) -> P
```

`p` turned about `o` to the nearest 15° step, at the same distance.

### `parallel`
[src/tools/guides.rs:342](../../../src/tools/guides.rs#L342)

```rust
fn parallel(a: P, b: P) -> bool
```

Whether two vectors run the same way or opposite, near enough.

### `angle_of`
[src/tools/guides.rs:347](../../../src/tools/guides.rs#L347)

```rust
fn angle_of(uv: P) -> f32
```

Angle of `uv` about the square's middle.

### `on_circle`
[src/tools/guides.rs:352](../../../src/tools/guides.rs#L352)

```rust
fn on_circle(h: &Homography, r: f32, a: f32) -> Option<P>
```

The point at angle `a` round the circle, mapped onto the canvas.

### `nearest_sample`
[src/tools/guides.rs:357](../../../src/tools/guides.rs#L357)

```rust
fn nearest_sample(h: &Homography, r: f32, p: P) -> Option<P>
```

The fallback for projecting onto a ring when `p` can't be mapped back into the square (it lies past
the plane's horizon): of `CONIC_SAMPLES` points round the ring, the one nearest `p`. `None` if every
sample falls behind the viewer.

## Tests

### `mod tests`

Run with `cargo test tools::guides::tests` · 12 tests

| Test | What it checks |
|---|---|
| [`a_line_projects_square_onto_itself`](../../../src/tools/guides.rs#L384) | A line projects square onto itself. |
| [`a_segment_holds_points_past_its_ends_on_the_ends`](../../../src/tools/guides.rs#L390) | A segment holds points past its ends on the ends. |
| [`a_conic_in_a_square_is_its_inscribed_circle`](../../../src/tools/guides.rs#L398) | A conic in a square is its inscribed circle. |
| [`a_conic_in_perspective_touches_each_edge_at_its_middle_in_depth`](../../../src/tools/guides.rs#L411) | A conic in perspective touches each edge at its middle in depth. |
| [`the_pick_follows_the_motion_and_skips_fixed_curves`](../../../src/tools/guides.rs#L432) | The pick follows the motion and skips fixed curves. |
| [`a_press_takes_the_nearest_fixed_curve_in_reach`](../../../src/tools/guides.rs#L451) | A press takes the nearest fixed curve in reach. |
| [`a_ruler_is_fixed_and_an_infinite_one_runs_on`](../../../src/tools/guides.rs#L467) | A ruler is fixed and an infinite one runs on. |
| [`parallel_lines_and_a_point_run_through_the_start`](../../../src/tools/guides.rs#L481) | Parallel lines and a point run through the start. |
| [`an_ellipse_offers_rings_only_from_inside_it`](../../../src/tools/guides.rs#L495) | An ellipse offers rings only from inside it. |
| [`a_ruler_end_drags_alone_and_shift_turns_it_in_steps`](../../../src/tools/guides.rs#L511) | A ruler end drags alone and shift turns it in steps. |
| [`dragging_a_vanishing_point_swings_its_helpers_round_their_far_ends`](../../../src/tools/guides.rs#L525) | Dragging a vanishing point swings its helpers round their far ends. |
| [`pull_goes_part_way`](../../../src/tools/guides.rs#L544) | Pull goes part way. |

**Test helpers** (used only by the tests above):

- [`near`](../../../src/tools/guides.rs#L371) — Near.
- [`line`](../../../src/tools/guides.rs#L375) — Line.
- [`fresh_doc`](../../../src/tools/guides.rs#L462) — Fresh doc.
