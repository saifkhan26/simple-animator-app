# `src/tools/perspective.rs` — perspective grids: geometry, walls, snapping

> **Source:** [src/tools/perspective.rs](../../../src/tools/perspective.rs) · **Module:** `crate::tools::perspective` · **Size:** 3317 lines · Line links as of commit `42a83e1`

## What this file is for

**Perspective grids** are drawing guides that live in the viewport, not on a layer (they never appear in the
artwork or exports). Each grid is a plane seen in perspective — a floor, a wall — defined by **four free
corners** (TL, TR, BR, BL). From those corners the code derives everything an artist needs: the grid lines,
the **vanishing points** where the rows and columns converge, the **horizon** (eye level), extra vanishing
points, walls standing on the floor's edges, a ceiling, fans of rays covering the canvas — and **snapping**,
which holds a pen stroke to the direction of whichever vanishing point it is heading for.

Grids can also be flat (square) or isometric, can follow the document, the camera or a layer, and can be
keyframed over time.

### The key maths: a homography

Four corners define a **projective map** (homography) `H` from the unit square to the quad:
`(u, v) → document point`. It is a 3×3 matrix applied to `(u, v, 1)` followed by a divide:

```text
[x']   [m0 m1 m2] [u]
[y'] = [m3 m4 m5] [v]        point = (x'/w', y'/w')
[w']   [m6 m7 m8] [1]
```

Grid line `v = 0.25` is the image of a horizontal line in the square. Because of the divide by `w'`, parallel
lines in the square can meet in the picture: that meeting point is a **vanishing point**. Points with `w' ≤ 0`
lie behind the viewer (beyond the horizon) — `map_front` and `unmap` refuse them.

**Example (one-point perspective).** Corners TL (300, 200), TR (500, 200), BR (700, 400), BL (100, 400): the
left and right edges (the columns) extended meet at (400, 100) — the vanishing point; the horizon is the
horizontal line y = 100 through it. The rows stay horizontal (parallel to the horizon), so their vanishing
point is "at infinity" — `Vp::Dir` instead of `Vp::Point`.

```text
              ● VP (400,100)       ← horizon y = 100
            ╱   ╲
     TL ●──╱─────╲──● TR
       ╱  ╱       ╲  ╲
 BL ●─────────────────● BR
```

## How it connects

- **Used by:** [app/grids.rs](../app/grids.md) (editing grids on `AppState`, keys, per-file storage),
  [app.rs](../app.md) (stroke snapping `snap_lock`), [tools/shape.rs](shape.md) (shapes on a grid plane),
  [ui/shell.rs](../ui/shell.md) (drawing grids, perspective options panel).
- **Uses:** [doc/transform.rs](../doc/transform.md) (`Transform`, `Ease`), [doc/camera.rs](../doc/camera.md).

## Rust concepts in this file

**`f64` for geometry precision.** The homography uses `f64` (JS `number` precision); points are `f32` tuples.
More: [Numbers and casts](../../rust-primer.md#numbers-and-casts)

**Enums for "finite or at infinity".** `enum Vp { Point(P), Dir(P) }` makes the two cases explicit; every use
must handle both — no special `Infinity` values. More: [Enums and match](../../rust-primer.md#enums-and-match)

**Serde attributes.** `Follow::Layer { name, index, #[serde(skip)] uid }` saves the layer by name and number but
not the session-only `uid`. More: [Serde and file formats](../../rust-primer.md#serde-and-file-formats)

**Solving drags from the press state.** Like the selection box, `dragged(start_c, grab, start, now, …)` computes
the new corners from the corners at the press, so the result depends only on the current pointer position.

## Types

### `type P`
[src/tools/perspective.rs:30](../../../src/tools/perspective.rs#L30)

```rust
pub type P = [f32; 2];
```

A point or vector `(x, y)` in document space.

### `const FRONT_EPS`
[src/tools/perspective.rs:55](../../../src/tools/perspective.rs#L55)

```rust
const FRONT_EPS: f64 = 1e-6;
```

How far in front of the vanishing line a point must be, as a homogeneous `w`, to count as on the plane. The corners sit at `w` around 1; right at the line a point is infinitely far out along the plane.

### `struct Homography`
[src/tools/perspective.rs:60](../../../src/tools/perspective.rs#L60)

A projective map from the unit square onto a quad (see the intro).

| Field | Type | Meaning |
|---|---|---|
| `m` | `[f64; 9]` | The 3×3 matrix, row by row. |

### `enum Vp`
[src/tools/perspective.rs:184](../../../src/tools/perspective.rs#L184)

A vanishing point: finite, or a direction (point at infinity).

| Variant | Meaning |
|---|---|
| `Point(P)` | A finite vanishing point. |
| `Dir(P)` | A direction: the lines stay parallel (vanishing point at infinity). |

### `struct Line`
[src/tools/perspective.rs:201](../../../src/tools/perspective.rs#L201)

An infinite line: a point on it and a unit direction.

| Field | Type | Meaning |
|---|---|---|
| `p` | `P` | A point on the line. |
| `d` | `P` | Unit direction. |

### `struct Family`
[src/tools/perspective.rs:211](../../../src/tools/perspective.rs#L211)

One family of lines on a grid's plane: those where `alpha·u + beta·v` is a whole number. Rows are `(0, rows)`, columns `(cols, 0)`, and the two diagonals of every cell `(cols, ±rows)` — on an isometric grid the first of those is its verticals.

| Field | Type | Meaning |
|---|---|---|
| `alpha` | `f32` | Weight of `u` in `alpha·u + beta·v = k`. |
| `beta` | `f32` | Weight of `v`. |

### `enum Shown`
[src/tools/perspective.rs:268](../../../src/tools/perspective.rs#L268)

A line of a grid's plane as it shows: whole, or — where the plane recedes — a ray out of its vanishing point.

| Variant | Meaning |
|---|---|
| `Line { p: P, d: P }` | The whole line, through `p` along unit `d`. |
| `Ray { from: P, d: P }` | From vanishing point `from` along unit `d`, out to infinity. |

### `enum GridGrab`
[src/tools/perspective.rs:277](../../../src/tools/perspective.rs#L277)

What a press on a grid grabs.

| Variant | Meaning |
|---|---|
| `Corner(u8)` | One of the four corners (0 TL … 3 BL). |
| `Vp(u8)` | A finite vanishing point: 0 the rows', 1 the columns'. |
| `Extra(u8)` | One of the grid's extra vanishing points, by index. |
| `Horizon` | The horizon line itself: raises or lowers the eye level. |
| `Tilt` | A knob on the horizon: tilts it about the point above the grid. |
| `Rotate` | The ring around the grid: turns it. |
| `Move` | Inside the quad: moves the whole grid. |
| `Bend` | A pan's curve-shape handle, on its outermost arch. |

### `enum GridKind`
[src/tools/perspective.rs:317](../../../src/tools/perspective.rs#L317)

What kind of guide a grid is. All three are a four-corner quad; flat and isometric grids hold theirs to a square or a 60° rhombus, and tile it over the whole canvas.

| Variant | Meaning |
|---|---|
| `Perspective` | A free four-corner quad in perspective. |
| `Flat` | A square 2D grid: two families of lines at right angles. |
| `Isometric` | Three families at 60° to each other: the rhombus's two edge directions, and the vertical through its corners. |
| `Ruler` | A straight edge: a press on it draws along it. |
| `Parallel` | Strokes anywhere run parallel to its line. |
| `Point` | A vanishing point of its own, fitted by two helper lines. |
| `Ellipse` | A circle or ellipse — in perspective when its quad is. |
| `PanHorizontal` | A horizontal pan, three points: lines level with the horizon bow into arches through a point at each side, depth lines bend through the centre point, verticals stay straight. Panning across a background drawn on it reads as the camera turning. |
| `PanVertical` | The same on its side, for a tilt: verticals bow through a point above and one below, horizontals stay straight. |
| `PanLoop` | A horizontal pan all the way round, four points to a turn: the box is one whole turn, and its curves run on past it so the background loops. |
| `Fisheye` | Five points: horizontals arc through a point at each side, verticals through one above and one below, depth lines run straight to the centre. |

### `enum Follow`
[src/tools/perspective.rs:407](../../../src/tools/perspective.rs#L407)

What a grid's coordinates are relative to, and so what carries it along over the shot.

| Variant | Meaning |
|---|---|
| `Document` | The document: the grid stays on the drawing while the camera moves over it. |
| `Camera` | The camera's frame: the grid stays put in the shot as the camera pans, zooms and rolls. |
| `Layer { name: String, index: usize, #[serde(skip)] uid: u64, }` | A layer's transform. Named and numbered rather than by the layer's own id, which only lasts a session — grids outlive it. `uid` caches the match within a session, so a rename or a reorder keeps the link. |

### `struct Space`
[src/tools/perspective.rs:431](../../../src/tools/perspective.rs#L431)

Where a grid's frame-relative coordinates land in document space on one frame: the frame size, and the pose of what the grid follows — the camera's frame rect, or a layer's transform — as a similarity about the frame centre. Identity for a grid on the document.

| Field | Type | Meaning |
|---|---|---|
| `w` | `f32` | Frame width. |
| `h` | `f32` | Frame height. |
| `xf` | `Transform` | The pose of what the grid follows (camera frame or layer transform), as a similarity transform. |

### `struct GridPose`
[src/tools/perspective.rs:478](../../../src/tools/perspective.rs#L478)

The part of a grid that animates: where it is, and how strongly it shows.

| Field | Type | Meaning |
|---|---|---|
| `corners` | `[P; 4]` | TL, TR, BR, BL, frame-relative in the grid's `Space`. |
| `extras` | `Vec<P>` | Each extra vanishing point's spot, in order. |
| `opacity` | `f32` | How strongly the grid shows (0–1). |

### `struct GridKey`
[src/tools/perspective.rs:501](../../../src/tools/perspective.rs#L501)

A grid keyframe. `ease` shapes the segment running from this key to the next, as on camera keys; it is ignored on the last key.

| Field | Type | Meaning |
|---|---|---|
| `frame` | `usize` | The keyed frame. |
| `pose` | `GridPose` | The grid's pose there. |
| `ease` | `Ease` | Easing out of this key. |

### `struct WallLink`
[src/tools/perspective.rs:510](../../../src/tools/perspective.rs#L510)

A wall: a grid standing on an edge of another, its parent, and rebuilt from it whenever the parent changes.

| Field | Type | Meaning |
|---|---|---|
| `parent` | `u32` | The parent's `PerspectiveGrid::id`. |
| `edge` | `u8` | The parent's edge it stands on — see `EDGE_NAMES`. |
| `height` | `f32` | How tall: the fraction of the way to the vertical vanishing point, or — when the verticals stay parallel — a multiple of the edge's length. |

### `const EDGE_NAMES`
[src/tools/perspective.rs:522](../../../src/tools/perspective.rs#L522)

```rust
pub const EDGE_NAMES: [&str; 4] = ["Back", "Right", "Front", "Left"];
```

The quad's edges by where they sit on a floor: 0 TL–TR, 1 TR–BR, 2 BL–BR, 3 TL–BL.

### `struct PerspectiveGrid`
[src/tools/perspective.rs:526](../../../src/tools/perspective.rs#L526)

One grid: its kind, corners, divisions, look (colour, weight, extras, fans, ceiling), snapping switches, what it follows, keys and wall link.

| Field | Type | Meaning |
|---|---|---|
| `id` | `u32` | Unique among a file's grids, and how a wall names its parent. 0 until `PerspectiveConfig::ensure_ids` hands one out. |
| `kind` | `GridKind` | Perspective, flat or isometric. |
| `corners` | `[P; 4]` | TL, TR, BR, BL, frame-relative (see the module docs). With keys, this is what the current frame resolves to — or an edit not keyed yet. |
| `rows` | `u32` | Rows of cells. |
| `cols` | `u32` | Columns of cells. |
| `color` | `[u8; 3]` | Line colour. |
| `opacity` | `f32` | Line opacity. |
| `weight` | `f32` | Line weight in screen pixels. |
| `visible` | `bool` | Whether shown. |
| `locked` | `bool` | A locked grid can still be selected, but not dragged. |
| `extend` | `bool` | Carry every grid line out to its vanishing point. |
| `horizon` | `bool` | Whether to draw the horizon line. |
| `extra_vps` | `Vec<ExtraVp>` | Vanishing points beyond the two the rows and columns run to — at most `MAX_EXTRA_VPS`. |
| `infinite` | `bool` | Tile rows and columns on past the quad, out to the horizon and the canvas edge. |
| `fill_canvas` | `bool` | Cover the whole canvas: an even fan of rays round each vanishing point, and the level lines of the floor and of a ceiling over it. Takes over from `infinite` and `extend`. |
| `fan_rays` | `u32` | Rays in each fan, all the way round. |
| `ceiling` | `f32` | How far the ceiling sits above eye level, against how far the floor sits below it: 1 mirrors the floor. |
| `major_every` | `u32` | Draw every Nth line heavier, counting from the quad's first edge; 0 for none. |
| `diagonals` | `bool` | Both diagonals of every cell. |
| `centre_lines` | `bool` | The lines through the middle of the quad. |
| `snap_rows` | `bool` | Which families strokes may snap along: the rows, the columns, and an isometric grid's verticals. |
| `snap_cols` | `bool` | Strokes may snap along the columns. |
| `snap_third` | `bool` | Strokes may snap along an isometric grid's verticals. |
| `magnet` | `bool` | Strokes may snap to this guide at all — the magnet on its row. Every visible guide with it on is a snap source, not just the active one. |
| `infinite_ruler` | `bool` | A ruler runs on past its ends, across the whole canvas. |
| `rings` | `bool` | An ellipse also takes strokes that start inside it round its centre, on the ring through where they start. |
| `bend` | `f32` | A pan's curve shape: 0 plain arches; up to 1 fuller, square-shouldered ones that close in on their points steeply; down to flatter ones that close in at a shallow angle. Not keyed, like the rows: it is how the guide looks, not where it is. |
| `follow` | `Follow` | What the grid moves with: document, camera or a layer. |
| `keys` | `Vec<GridKey>` | Sorted by frame. None: the grid holds still in its space. |
| `wall` | `Option<WallLink>` | When this grid is a wall standing on another grid: the link to its parent. |

### `const MAX_DIVISIONS`
[src/tools/perspective.rs:592](../../../src/tools/perspective.rs#L592)

```rust
pub const MAX_DIVISIONS: u32 = 64;
```

The most rows or columns a grid may have.

### `const FAN_RAYS`
[src/tools/perspective.rs:595](../../../src/tools/perspective.rs#L595)

```rust
pub const FAN_RAYS: std::ops::RangeInclusive<u32> = 8..=180;
```

Fewest and most rays a fan may have.

### `const CEILING`
[src/tools/perspective.rs:599](../../../src/tools/perspective.rs#L599)

```rust
pub const CEILING: std::ops::RangeInclusive<f32> = 0.25..=4.0;
```

The range a ceiling may sit at.

### `const MAX_EXTRA_VPS`
[src/tools/perspective.rs:603](../../../src/tools/perspective.rs#L603)

```rust
pub const MAX_EXTRA_VPS: usize = 2;
```

Most vanishing points a grid adds to the two of its own plane, for four in all.

### `struct ExtraVp`
[src/tools/perspective.rs:611](../../../src/tools/perspective.rs#L611)

A vanishing point a grid carries besides its plane's own: somewhere else strokes can snap toward. On the horizon it is a second pair of directions on the same floor — a box turned another way. Off it, above or below, it is the third point verticals run to.

| Field | Type | Meaning |
|---|---|---|
| `pos` | `P` | Frame-relative, like the corners. A point on the horizon shows at this spot's foot on it, so it rides along as the grid is re-aimed. |
| `on_horizon` | `bool` | Whether it keeps to the horizon. |
| `rays` | `bool` | Fan guide lines from it across the grid. |
| `snap` | `bool` | Strokes may snap toward it. |

### `const RIGID_SIDE`
[src/tools/perspective.rs:673](../../../src/tools/perspective.rs#L673)

```rust
const RIGID_SIDE: f32 = 0.4;
```

Side of a fresh flat or isometric grid's home quad, in frame heights.

### `enum Preset`
[src/tools/perspective.rs:677](../../../src/tools/perspective.rs#L677)

A ready-made perspective setup, as the Add menu offers them.

| Variant | Meaning |
|---|---|
| `OnePoint` | A floor facing the viewer: one vanishing point. |
| `TwoPoint` | A floor turned to the viewer: two vanishing points out on the horizon, past the sides of the frame. |
| `BirdsEye` | Two-point from above: the horizon high, verticals running down to a third point below the frame. |
| `WormsEye` | Two-point from below: the horizon low, verticals running up to a third point above the frame. |

### `struct PoseFrame`
[src/tools/perspective.rs:1165](../../../src/tools/perspective.rs#L1165)

A quad's similarity frame: centroid, spread and heading. Tweening goes through it, so a grid turning between two keys turns rather than shrinking through the chord.

| Field | Type | Meaning |
|---|---|---|
| `o` | `P` | Centroid. |
| `s` | `f32` | Spread (size). |
| `a` | `f32` | Heading (angle). |

### `struct PerspectiveConfig`
[src/tools/perspective.rs:1265](../../../src/tools/perspective.rs#L1265)

All grids of the open file plus the global switches (show, snap, auto-key, ghosts). Kept by the app per file, never inside the `.anim`.

| Field | Type | Meaning |
|---|---|---|
| `grids` | `Vec<PerspectiveGrid>` | The open file's grids. Kept by the app per file, not in the file. |
| `active` | `usize` | Index into `grids` of the grid being edited, and the one strokes snap to. |
| `show` | `bool` | Draw the grids while other tools are active. The perspective tool always shows them. |
| `snap` | `bool` | Lock brush strokes to the active grid's lines. |
| `snap_strength` | `f32` | How hard a snapped stroke is pulled onto its line: 1 all the way, 0 not at all — a lighter pull keeps some of the hand in the line. |
| `snap_eraser` | `bool` | The eraser snaps as well as the pens. |
| `snap_vertical` | `bool` | Also offer the vertical as a snap direction. Off by default: near the middle of a one-point grid the columns are nearly vertical too, and a stroke meant to run to the vanishing point would lock straight up. |
| `auto_key` | `bool` | Once a grid has keys, an edit to its pose keys the current frame. |
| `ghosts` | `bool` | Faint outlines of the active grid at its keys either side. |
| `cursor_guides` | `bool` | While a stroke would snap, faint lines from the cursor along every way it could go. |

### `const WALL_COLOR`
[src/tools/perspective.rs:1428](../../../src/tools/perspective.rs#L1428)

```rust
const WALL_COLOR: [u8; 3] = [255, 150, 80];
```

A new wall's colour: warm, against the default grid's blue.

### `struct Plane`
[src/tools/perspective.rs:1590](../../../src/tools/perspective.rs#L1590)

Everything derived from a grid's corners that drawing and snapping need.

| Field | Type | Meaning |
|---|---|---|
| `h` | `Homography` | The quad's homography. |
| `vp_rows` | `Vp` | Where lines of constant v (the rows, running along u) converge. |
| `vp_cols` | `Vp` | Where lines of constant u (the columns, running along v) converge. |
| `near` | `P` | TL, and the quad's reach from it: what "far away" is measured against when deciding a vanishing point is at infinity. |
| `size` | `f32` | The quad's reach from `near`, the scale "far away" is judged against. |

### `const TILT_KNOB`
[src/tools/perspective.rs:1851](../../../src/tools/perspective.rs#L1851)

```rust
pub const TILT_KNOB: f32 = 12.0;
```

How far out along the horizon its tilt knobs sit from the point above the grid, in handle radii.

### `struct Tile`
[src/tools/perspective.rs:2106](../../../src/tools/perspective.rs#L2106)

One tiled line on screen: its index in its family, and its two ends.

| Field | Type | Meaning |
|---|---|---|
| `k` | `i32` | The line's index in its family. |
| `a` | `P` | One end on screen. |
| `b` | `P` | The other end. |

### `const MAX_TILES`
[src/tools/perspective.rs:2113](../../../src/tools/perspective.rs#L2113)

```rust
const MAX_TILES: i32 = 400;
```

Most lines tiled each way from the quad, per family.

### `const FADE_FROM_GAP`
[src/tools/perspective.rs:2278](../../../src/tools/perspective.rs#L2278)

```rust
const FADE_FROM_GAP: f32 = 1.5;
```

Where a fan's rays crowd closer than this many pixels they would only smudge together, so they don't show...

### `const FADE_TO_GAP`
[src/tools/perspective.rs:2280](../../../src/tools/perspective.rs#L2280)

```rust
const FADE_TO_GAP: f32 = 8.0;
```

...and from where they are this far apart they show in full.

### `const FADE_STEPS`
[src/tools/perspective.rs:2281](../../../src/tools/perspective.rs#L2281)

```rust
const FADE_STEPS: usize = 5;
```

How many pieces a fan ray's fade-in is cut into.

## Free functions

### `sub`
[src/tools/perspective.rs:32](../../../src/tools/perspective.rs#L32)

```rust
pub(crate) fn sub(a: P, b: P) -> P
```

Vector `a − b`.

### `add`
[src/tools/perspective.rs:35](../../../src/tools/perspective.rs#L35)

```rust
pub(crate) fn add(a: P, b: P) -> P
```

Vector `a + b`.

### `dot`
[src/tools/perspective.rs:38](../../../src/tools/perspective.rs#L38)

```rust
pub(crate) fn dot(a: P, b: P) -> f32
```

Dot product.

### `cross`
[src/tools/perspective.rs:41](../../../src/tools/perspective.rs#L41)

```rust
pub(crate) fn cross(a: P, b: P) -> f32
```

2-D cross product (`a.x·b.y − a.y·b.x`): positive if `b` turns left from `a`.

### `len`
[src/tools/perspective.rs:44](../../../src/tools/perspective.rs#L44)

```rust
pub(crate) fn len(a: P) -> f32
```

Vector length.

### `normalize`
[src/tools/perspective.rs:47](../../../src/tools/perspective.rs#L47)

```rust
pub(crate) fn normalize(a: P) -> Option<P>
```

The unit vector in `a`'s direction, or `None` for a zero vector.

## `impl Homography`

### `Homography::square_to_quad`
[src/tools/perspective.rs:67](../../../src/tools/perspective.rs#L67)

```rust
pub fn square_to_quad(c: &[P; 4]) -> Option<Self>
```

The map taking (0,0), (1,0), (1,1), (0,1) to TL, TR, BR, BL. Heckbert's closed form. `None` when three corners are collinear.

### `Homography::apply`
[src/tools/perspective.rs:105](../../../src/tools/perspective.rs#L105)

```rust
fn apply(&self, x: f64, y: f64, w: f64) -> [f64; 3]
```

Multiplies the matrix by `(x, y, w)`, returning the homogeneous result.

### `Homography::map`
[src/tools/perspective.rs:114](../../../src/tools/perspective.rs#L114)

```rust
pub fn map(&self, u: f32, v: f32) -> P
```

Maps `(u, v)` in the square to a document point (divide by `w`).

### `Homography::is_affine`
[src/tools/perspective.rs:121](../../../src/tools/perspective.rs#L121)

```rust
pub fn is_affine(&self) -> bool
```

Whether the map is affine — the quad a parallelogram, every family of lines parallel, nothing vanishing.

### `Homography::map_front`
[src/tools/perspective.rs:131](../../../src/tools/perspective.rs#L131)

```rust
pub fn map_front(&self, u: f32, v: f32) -> Option<P>
```

`map`, but `None` for a `(u, v)` on or past the plane's vanishing line — behind the viewer, where the map folds the plane back over the picture upside down. The corners of a convex quad are always in front.

### `Homography::unmap`
[src/tools/perspective.rs:139](../../../src/tools/perspective.rs#L139)

```rust
pub fn unmap(&self, p: P) -> Option<[f32; 2]>
```

The inverse: document point → `(u, v)`. `None` for a point on or past the vanishing line (above a floor's horizon), which no point of the plane in front of the viewer lands on.

### `Homography::vanish`
[src/tools/perspective.rs:166](../../../src/tools/perspective.rs#L166)

```rust
fn vanish(&self, du: f64, dv: f64, near: P, size: f32) -> Vp
```

Where the image of a direction in the square ends up: a finite vanishing point, or a direction when those lines stay parallel.

## `impl Vp`

### `Vp::dir_at`
[src/tools/perspective.rs:191](../../../src/tools/perspective.rs#L191)

```rust
pub fn dir_at(self, at: P) -> Option<P>
```

Unit direction of the family's line through `at`.

## `impl Family`

### `Family::rows`
[src/tools/perspective.rs:218](../../../src/tools/perspective.rs#L218)

```rust
pub fn rows(rows: u32) -> Self
```

Lines of constant v, running along u.

### `Family::cols`
[src/tools/perspective.rs:226](../../../src/tools/perspective.rs#L226)

```rust
pub fn cols(cols: u32) -> Self
```

Lines of constant u, running along v.

### `Family::diag`
[src/tools/perspective.rs:234](../../../src/tools/perspective.rs#L234)

```rust
pub fn diag(rows: u32, cols: u32) -> Self
```

`u·cols + v·rows = k`: through every cell's TR and BL corners.

### `Family::anti_diag`
[src/tools/perspective.rs:242](../../../src/tools/perspective.rs#L242)

```rust
pub fn anti_diag(rows: u32, cols: u32) -> Self
```

`u·cols − v·rows = k`: through every cell's TL and BR corners.

### `Family::dir`
[src/tools/perspective.rs:250](../../../src/tools/perspective.rs#L250)

```rust
fn dir(self) -> [f64; 2]
```

Along the lines, in (u, v).

### `Family::point`
[src/tools/perspective.rs:255](../../../src/tools/perspective.rs#L255)

```rust
fn point(self, k: f32) -> [f64; 2]
```

A point on line `k`, in (u, v).

## `impl GridKind`

### `GridKind::label`
[src/tools/perspective.rs:351](../../../src/tools/perspective.rs#L351)

```rust
pub fn label(self) -> &'static str
```

The kind's display name.

### `GridKind::row_name`
[src/tools/perspective.rs:368](../../../src/tools/perspective.rs#L368)

```rust
pub fn row_name(self) -> &'static str
```

What its row in the list is called, before its number.

### `GridKind::is_curved`
[src/tools/perspective.rs:383](../../../src/tools/perspective.rs#L383)

```rust
pub fn is_curved(self) -> bool
```

A curvilinear guide — the pans and the fisheye — whose lines bend through side points round a centre point.

### `GridKind::is_plane`
[src/tools/perspective.rs:393](../../../src/tools/perspective.rs#L393)

```rust
pub fn is_plane(self) -> bool
```

A plane with rows and columns: perspective, flat or isometric. Only these have walls, fill the canvas, and take shapes lying on them — and only these are what an older build can read.

### `GridKind::rigid`
[src/tools/perspective.rs:399](../../../src/tools/perspective.rs#L399)

```rust
pub fn rigid(self) -> bool
```

Whether the quad keeps its shape: flat and isometric grids only move, turn and scale.

## `impl Space`

### `Space::flat`
[src/tools/perspective.rs:439](../../../src/tools/perspective.rs#L439)

```rust
pub fn flat(w: f32, h: f32) -> Self
```

The document itself, for a `w`×`h` frame.

### `Space::unit`
[src/tools/perspective.rs:450](../../../src/tools/perspective.rs#L450)

```rust
pub fn unit() -> Self
```

A unit frame. Geometry worked out here is the frame-relative coordinates shifted by half a unit, which perspective constructions don't notice — walls are built in it.

### `Space::to_doc`
[src/tools/perspective.rs:454](../../../src/tools/perspective.rs#L454)

```rust
pub fn to_doc(self, n: P) -> P
```

Frame-relative coordinates to document space through the followed pose.

### `Space::to_rel`
[src/tools/perspective.rs:464](../../../src/tools/perspective.rs#L464)

```rust
pub fn to_rel(self, p: P) -> P
```

The inverse of `Self::to_doc`: a document point, frame-relative.

## `impl Default for GridPose`

### `GridPose::default`
[src/tools/perspective.rs:487](../../../src/tools/perspective.rs#L487)

```rust
fn default() -> Self
```

A default pose (the default grid's corners, no extras, full opacity).

## `impl Default for ExtraVp`

### `ExtraVp::default`
[src/tools/perspective.rs:623](../../../src/tools/perspective.rs#L623)

```rust
fn default() -> Self
```

A default extra vanishing point (on the horizon, with rays and snapping on).

## `impl Default for PerspectiveGrid`

### `PerspectiveGrid::default`
[src/tools/perspective.rs:636](../../../src/tools/perspective.rs#L636)

```rust
fn default() -> Self
```

A floor-like trapezoid in the middle of the frame: one-point perspective, so the grid reads as perspective the moment it appears.

## Free functions

### `line_quad`
[src/tools/perspective.rs:693](../../../src/tools/perspective.rs#L693)

```rust
pub fn line_quad(a: P, b: P) -> [P; 4]
```

A line from `a` to `b` as a guide keeps it: the flat quad `[a, b, b, a]`, whose centre is the line's middle and whose turn is the line's.

### `box_quad`
[src/tools/perspective.rs:699](../../../src/tools/perspective.rs#L699)

```rust
pub fn box_quad(o: P, w: f32, h: f32) -> [P; 4]
```

The level box `w` either side of `o` and `h` above and below it, TL, TR, BR, BL — how a curved guide keeps its centre point and side points.

## `impl PerspectiveGrid`

### `PerspectiveGrid::fresh`
[src/tools/perspective.rs:714](../../../src/tools/perspective.rs#L714)

```rust
pub fn fresh(kind: GridKind) -> Self
```

A fresh grid of `kind`: a floor trapezoid, a square, or a rhombus with its long diagonal level — each centred, four cells to a side for the tiled kinds. A ruler lies level across the middle (kept as `line_quad(a, b)`); a vanishing point sits above the middle with its two helper lines fanning down; an ellipse starts as a circle. Each non-plane kind gets its own default colour.

### `PerspectiveGrid::preset`
[src/tools/perspective.rs:794](../../../src/tools/perspective.rs#L794)

```rust
pub fn preset(preset: Preset) -> Self
```

A perspective grid set up as `preset` says: its floor aimed at two vanishing points out past the frame's sides, its near corner just right of the middle, and — for three-point — a third point for the verticals, off the horizon.

### `PerspectiveGrid::doc_corners`
[src/tools/perspective.rs:826](../../../src/tools/perspective.rs#L826)

```rust
pub fn doc_corners(&self, s: &Space) -> [P; 4]
```

Corners in document space.

### `PerspectiveGrid::set_doc_corners`
[src/tools/perspective.rs:830](../../../src/tools/perspective.rs#L830)

```rust
pub fn set_doc_corners(&mut self, c: [P; 4], s: &Space)
```

Sets the corners from document-space positions (converted to frame-relative).

### `PerspectiveGrid::is_wall`
[src/tools/perspective.rs:834](../../../src/tools/perspective.rs#L834)

```rust
pub fn is_wall(&self) -> bool
```

Whether this grid is a wall linked to a parent grid.

### `PerspectiveGrid::rotate`
[src/tools/perspective.rs:842](../../../src/tools/perspective.rs#L842)

```rust
pub fn rotate(&mut self, angle: f32)
```

Rotate about the corner centroid. Frame-relative storage is a uniform scale plus a translation of document space, so rotating it directly is the same as rotating in the document. The extra vanishing points turn with it.

### `PerspectiveGrid::extra_doc`
[src/tools/perspective.rs:853](../../../src/tools/perspective.rs#L853)

```rust
pub fn extra_doc(&self, s: &Space) -> Vec<P>
```

Where each extra vanishing point sits in document space: on the horizon at the foot of its spot, when it keeps to the horizon and the grid has one, else at the spot itself.

### `PerspectiveGrid::ceiling_doc`
[src/tools/perspective.rs:873](../../../src/tools/perspective.rs#L873)

```rust
pub fn ceiling_doc(&self, s: &Space) -> Option<[P; 4]>
```

The ceiling over the grid's floor, in document space: each corner carried up its vertical — toward the first extra vanishing point off the horizon, or else square to the horizon — to `Self::ceiling` times its depth below eye level, on the far side of the horizon. `None` without a horizon, or when the ceiling would reach round behind the viewer.

### `PerspectiveGrid::add_extra_vp`
[src/tools/perspective.rs:895](../../../src/tools/perspective.rs#L895)

```rust
pub fn add_extra_vp(&mut self, s: &Space) -> bool
```

Add an extra vanishing point, unless the grid has all it can take. On the horizon, well out to one side of the grid — the other side for the second — or straight above it when the horizon is at infinity. Every key gets it too, where it shows now.

### `PerspectiveGrid::remove_extra_vp`
[src/tools/perspective.rs:924](../../../src/tools/perspective.rs#L924)

```rust
pub fn remove_extra_vp(&mut self, i: usize)
```

Remove extra vanishing point `i`, from every key too.

### `PerspectiveGrid::families`
[src/tools/perspective.rs:938](../../../src/tools/perspective.rs#L938)

```rust
pub fn families(&self) -> Vec<Family>
```

The families of lines the grid is made of: rows and columns, and an isometric grid's verticals.

### `PerspectiveGrid::snap_targets`
[src/tools/perspective.rs:950](../../../src/tools/perspective.rs#L950)

```rust
pub fn snap_targets(&self, s: &Space, vertical: bool) -> Vec<Vp>
```

Everything strokes may snap toward, in document space: the families switched on, the vertical when asked (perspective grids only — square to the horizon, or to the rows when it is at infinity), and the extra vanishing points switched on.

### `PerspectiveGrid::snap_dirs`
[src/tools/perspective.rs:985](../../../src/tools/perspective.rs#L985)

```rust
pub fn snap_dirs(&self, s: &Space, at: P, vertical: bool) -> Vec<P>
```

Candidate snap directions at `at`: toward each of `Self::snap_targets`.

### `PerspectiveGrid::centre`
[src/tools/perspective.rs:998](../../../src/tools/perspective.rs#L998)

```rust
pub fn centre(&self) -> P
```

Where the grid sits: its corner centroid, from the frame centre.

### `PerspectiveGrid::translate`
[src/tools/perspective.rs:1002](../../../src/tools/perspective.rs#L1002)

```rust
pub fn translate(&mut self, d: P)
```

Moves the grid (and its extra vanishing points) by `d`.

### `PerspectiveGrid::scale`
[src/tools/perspective.rs:1010](../../../src/tools/perspective.rs#L1010)

```rust
pub fn scale(&self) -> f32
```

Size relative to a fresh grid of its kind, which reads 1.

### `PerspectiveGrid::scale_by`
[src/tools/perspective.rs:1016](../../../src/tools/perspective.rs#L1016)

```rust
pub fn scale_by(&mut self, k: f32)
```

Scale by `k` about the centroid. Ignores a `k` that would collapse or flip the grid.

### `PerspectiveGrid::angle`
[src/tools/perspective.rs:1031](../../../src/tools/perspective.rs#L1031)

```rust
pub fn angle(&self) -> f32
```

Heading of the grid, in radians, against a fresh grid of its kind (which reads 0): from the middle of its left edge to the middle of its right. `Self::rotate` turns it by exactly its angle.

### `PerspectiveGrid::pose`
[src/tools/perspective.rs:1038](../../../src/tools/perspective.rs#L1038)

```rust
pub fn pose(&self) -> GridPose
```

The live pose.

### `PerspectiveGrid::set_pose`
[src/tools/perspective.rs:1048](../../../src/tools/perspective.rs#L1048)

```rust
pub fn set_pose(&mut self, p: &GridPose)
```

Make `p` the live pose. Extra vanishing points it has no spot for stay where they are.

### `PerspectiveGrid::key_at`
[src/tools/perspective.rs:1056](../../../src/tools/perspective.rs#L1056)

```rust
pub fn key_at(&self, frame: usize) -> Option<&GridKey>
```

The key on `frame`, if any.

### `PerspectiveGrid::has_key`
[src/tools/perspective.rs:1060](../../../src/tools/perspective.rs#L1060)

```rust
pub fn has_key(&self, frame: usize) -> bool
```

Whether there's a key on `frame`.

### `PerspectiveGrid::set_key`
[src/tools/perspective.rs:1066](../../../src/tools/perspective.rs#L1066)

```rust
pub fn set_key(&mut self, frame: usize)
```

Key the live pose on `frame`, replacing a key there but keeping its ease.

### `PerspectiveGrid::delete_key`
[src/tools/perspective.rs:1081](../../../src/tools/perspective.rs#L1081)

```rust
pub fn delete_key(&mut self, frame: usize)
```

Removes the key on `frame`.

### `PerspectiveGrid::keys_insert_frames`
[src/tools/perspective.rs:1087](../../../src/tools/perspective.rs#L1087)

```rust
pub fn keys_insert_frames(&mut self, at: usize, n: usize)
```

Keep the keys on their frames after `n` frames are inserted at `at`, as the timeline's frame ops do for every other key.

### `PerspectiveGrid::keys_remove_frames`
[src/tools/perspective.rs:1097](../../../src/tools/perspective.rs#L1097)

```rust
pub fn keys_remove_frames(&mut self, at: usize, n: usize)
```

Keep the keys on their frames after `[at, at + n)` is removed. A key on a removed frame goes with it.

### `PerspectiveGrid::set_key_ease`
[src/tools/perspective.rs:1106](../../../src/tools/perspective.rs#L1106)

```rust
pub fn set_key_ease(&mut self, frame: usize, ease: Ease)
```

Sets the easing of the key on `frame`.

### `PerspectiveGrid::resolve`
[src/tools/perspective.rs:1114](../../../src/tools/perspective.rs#L1114)

```rust
pub fn resolve(&self, frame: usize) -> GridPose
```

The pose on `frame`: tweened from the keys, or the live pose when there are none.

### `PerspectiveGrid::restore_motion`
[src/tools/perspective.rs:1121](../../../src/tools/perspective.rs#L1121)

```rust
pub fn restore_motion(&mut self, snap: &PerspectiveGrid)
```

Take back everything a drag or a key step can change — the pose, the keys, a wall's height — from `snap`, leaving the look and the switches as they are now.

### `PerspectiveGrid::neighbour_keys`
[src/tools/perspective.rs:1131](../../../src/tools/perspective.rs#L1131)

```rust
pub fn neighbour_keys(&self, frame: usize) -> (Option<&GridKey>, Option<&GridKey>)
```

The nearest keys strictly before and strictly after `frame`.

## Free functions

### `spread`
[src/tools/perspective.rs:1139](../../../src/tools/perspective.rs#L1139)

```rust
fn spread(c: &[P; 4]) -> f32
```

Mean distance of the corners from their centroid.

### `heading`
[src/tools/perspective.rs:1146](../../../src/tools/perspective.rs#L1146)

```rust
fn heading(c: &[P; 4]) -> f32
```

From the middle of the quad's left edge to the middle of its right, in radians.

### `to_doc`
[src/tools/perspective.rs:1152](../../../src/tools/perspective.rs#L1152)

```rust
pub fn to_doc(n: P, w: f32, h: f32) -> P
```

Frame-relative point to document coordinates for a `w`×`h` frame.

### `from_doc`
[src/tools/perspective.rs:1156](../../../src/tools/perspective.rs#L1156)

```rust
pub fn from_doc(p: P, w: f32, h: f32) -> P
```

The reverse of `to_doc`.

## `impl PoseFrame`

### `PoseFrame::of`
[src/tools/perspective.rs:1172](../../../src/tools/perspective.rs#L1172)

```rust
fn of(c: &[P; 4]) -> Self
```

The similarity frame of a quad.

### `PoseFrame::local`
[src/tools/perspective.rs:1180](../../../src/tools/perspective.rs#L1180)

```rust
fn local(&self, p: P) -> P
```

A point expressed relative to the frame.

### `PoseFrame::world`
[src/tools/perspective.rs:1185](../../../src/tools/perspective.rs#L1185)

```rust
fn world(&self, l: P) -> P
```

A frame-relative point back in world coordinates.

## Free functions

### `lerp_p`
[src/tools/perspective.rs:1191](../../../src/tools/perspective.rs#L1191)

```rust
fn lerp_p(a: P, b: P, t: f32) -> P
```

Linear interpolation between two points.

### `tween`
[src/tools/perspective.rs:1201](../../../src/tools/perspective.rs#L1201)

```rust
pub fn tween(a: &GridPose, b: &GridPose, t: f32) -> GridPose
```

The pose `t` of the way from `a` to `b`. Like the camera: the centre moves in a straight line, the size at a steady rate, the heading the short way round. What is left of the corners' shape once that is taken out blends straight across, so a 90° turn keeps its size. A blend that would fold the quad falls back to sliding each corner straight, then to holding the nearer key.

### `resolve_keys`
[src/tools/perspective.rs:1243](../../../src/tools/perspective.rs#L1243)

```rust
pub fn resolve_keys(keys: &[GridKey], frame: usize) -> Option<GridPose>
```

The pose keys give `frame`, or `None` with no keys. Held flat before the first key and after the last, eased within a segment — the same rules as `Camera::resolve`.

## `impl Default for PerspectiveConfig`

### `PerspectiveConfig::default`
[src/tools/perspective.rs:1298](../../../src/tools/perspective.rs#L1298)

```rust
fn default() -> Self
```

Default settings: no grids, show on, snap off, vertical snap off, auto-key on, ghosts and cursor guides on.

## `impl PerspectiveConfig`

### `PerspectiveConfig::active_grid`
[src/tools/perspective.rs:1317](../../../src/tools/perspective.rs#L1317)

```rust
pub fn active_grid(&self) -> Option<&PerspectiveGrid>
```

The grid being edited, if any.

### `PerspectiveConfig::active_grid_mut`
[src/tools/perspective.rs:1321](../../../src/tools/perspective.rs#L1321)

```rust
pub fn active_grid_mut(&mut self) -> Option<&mut PerspectiveGrid>
```

The grid being edited, writable.

### `PerspectiveConfig::remove`
[src/tools/perspective.rs:1327](../../../src/tools/perspective.rs#L1327)

```rust
pub fn remove(&mut self, i: usize)
```

Remove grid `i`, keeping `active` on the same grid where it can. Walls standing on it come loose on the next `Self::relink`.

### `PerspectiveConfig::index_of`
[src/tools/perspective.rs:1337](../../../src/tools/perspective.rs#L1337)

```rust
pub fn index_of(&self, id: u32) -> Option<usize>
```

The index of the grid with id `id`.

### `PerspectiveConfig::next_id`
[src/tools/perspective.rs:1341](../../../src/tools/perspective.rs#L1341)

```rust
fn next_id(&self) -> u32
```

The next unused grid id.

### `PerspectiveConfig::ensure_ids`
[src/tools/perspective.rs:1347](../../../src/tools/perspective.rs#L1347)

```rust
pub fn ensure_ids(&mut self)
```

Give every grid an id, and make them unique — grids from before ids all read 0.

### `PerspectiveConfig::push`
[src/tools/perspective.rs:1359](../../../src/tools/perspective.rs#L1359)

```rust
pub fn push(&mut self, mut g: PerspectiveGrid) -> usize
```

Add `g` with a fresh id and make it the active grid. Returns its index.

### `PerspectiveConfig::add_wall`
[src/tools/perspective.rs:1369](../../../src/tools/perspective.rs#L1369)

```rust
pub fn add_wall(&mut self, i: usize, edge: u8) -> Option<usize>
```

Stand a wall on edge `edge` of grid `i`, which must be a perspective grid. It takes the edge's divisions across, so its lines meet the floor's, and becomes the active grid.

### `PerspectiveConfig::relink`
[src/tools/perspective.rs:1400](../../../src/tools/perspective.rs#L1400)

```rust
pub fn relink(&mut self)
```

Rebuild every wall from its parent as the parent is now. A wall whose parent is gone, or no longer a perspective grid, comes loose and keeps its last shape. Walls carry no keys of their own and follow what their parent follows: the parent's motion is theirs.

## Free functions

### `edge_ends`
[src/tools/perspective.rs:1432](../../../src/tools/perspective.rs#L1432)

```rust
fn edge_ends(c: &[P; 4], e: u8) -> (P, P)
```

The two ends of edge `e`, in the order the wall on it runs: its BL, then its BR.

### `edge_vp`
[src/tools/perspective.rs:1442](../../../src/tools/perspective.rs#L1442)

```rust
fn edge_vp(plane: &Plane, e: u8) -> Vp
```

Where the lines along edge `e` run.

### `floor_up`
[src/tools/perspective.rs:1452](../../../src/tools/perspective.rs#L1452)

```rust
fn floor_up(plane: &Plane, o: P) -> Option<P>
```

"Up" off a floor, in `Space::unit`: square to its horizon and toward it, or — horizon at infinity — square to the rows and up the screen.

### `wall_vertical`
[src/tools/perspective.rs:1470](../../../src/tools/perspective.rs#L1470)

```rust
fn wall_vertical(parent: &PerspectiveGrid) -> Option<(Vp, bool)>
```

What a wall on `parent` stands up toward, in `Space::unit`: the first extra vanishing point off the horizon, with whether it is above the floor (verticals rise to it) or below (they rise away from it, as from a bird's eye); or, with none, straight up — parallel verticals.

### `lift`
[src/tools/perspective.rs:1488](../../../src/tools/perspective.rs#L1488)

```rust
fn lift(p: P, hz: Line, v: Vp, k: f32) -> Option<P>
```

Floor point `p` carried along its vertical, through `v`, to the plane `k` times as far above eye level as the floor is below it: the homology about the horizon `hz` whose cross-ratio is −k. With parallel verticals that is a mirror across the horizon, stretched `k` times. `None` where the plane would pass behind the viewer.

### `rise`
[src/tools/perspective.rs:1508](../../../src/tools/perspective.rs#L1508)

```rust
fn rise(a: P, v: Vp, toward: bool, height: f32, edge_len: f32) -> P
```

The top of the vertical from `a`, `height` up (see `WallLink::height`).

### `height_of`
[src/tools/perspective.rs:1519](../../../src/tools/perspective.rs#L1519)

```rust
fn height_of(a: P, top: P, v: Vp, toward: bool, edge_len: f32) -> f32
```

The height a vertical from `a` reaching up to `top` stands for.

### `wall_corners`
[src/tools/perspective.rs:1535](../../../src/tools/perspective.rs#L1535)

```rust
pub fn wall_corners(parent: &PerspectiveGrid, edge: u8, height: f32) -> Option<[P; 4]>
```

A wall's corners (TL, TR, BR, BL, frame-relative) standing on edge `edge` of `parent`: verticals up from both ends toward the vertical vanishing point, and a top edge running to the same point as the base, so it is level in the scene.

### `default_wall_height`
[src/tools/perspective.rs:1549](../../../src/tools/perspective.rs#L1549)

```rust
fn default_wall_height(parent: &PerspectiveGrid, edge: u8) -> Option<f32>
```

A new wall's height: about six tenths of its edge's length.

### `wall_height_at`
[src/tools/perspective.rs:1570](../../../src/tools/perspective.rs#L1570)

```rust
pub fn wall_height_at(parent: &PerspectiveGrid, edge: u8, corner: u8, p: P) -> Option<f32>
```

The height a wall on `edge` of `parent` takes when its top corner `corner` (0 TL, 1 TR) is dragged to frame-relative point `p`.

## `impl Plane`

### `Plane::new`
[src/tools/perspective.rs:1603](../../../src/tools/perspective.rs#L1603)

```rust
pub fn new(corners: [P; 4]) -> Option<Self>
```

Builds the plane data (homography, both vanishing points, a reference corner and size) from four corners;
`None` for degenerate corners.

### `Plane::grid_lines`
[src/tools/perspective.rs:1621](../../../src/tools/perspective.rs#L1621)

```rust
pub fn grid_lines(&self, rows: u32, cols: u32) -> Vec<(P, P, Vp, u32)>
```

Row and column segments across the quad, borders included. Each is tagged with the vanishing point its line runs to, and its index from the quad's first edge — for picking out the major lines.

### `Plane::family_vp`
[src/tools/perspective.rs:1636](../../../src/tools/perspective.rs#L1636)

```rust
pub fn family_vp(&self, f: Family) -> Vp
```

Where a family's lines converge.

### `Plane::family_line`
[src/tools/perspective.rs:1645](../../../src/tools/perspective.rs#L1645)

```rust
pub fn family_line(&self, f: Family, k: f32) -> Option<Shown>
```

Line `k` of family `f` across the whole plane, as much of it as is in front of the viewer: all of it when the family stays parallel, else the ray from its vanishing point out through the plane. `None` when the line is wholly behind the viewer.

### `Plane::quad_family`
[src/tools/perspective.rs:1683](../../../src/tools/perspective.rs#L1683)

```rust
pub fn quad_family(&self, f: Family) -> Vec<(i32, P, P)>
```

The lines of family `f` inside the quad only, as document segments with their index `k`.

### `Plane::horizon`
[src/tools/perspective.rs:1711](../../../src/tools/perspective.rs#L1711)

```rust
pub fn horizon(&self) -> Option<Line>
```

The plane's vanishing line. `None` when both families stay parallel — a plane seen square-on has its horizon at infinity.

Its point is the one nearest the grid, not a vanishing point: a nearly parallel pair of edges puts its vanishing point hundreds of grid-widths away, and once the view zooms in that is far enough out for `f32` screen coordinates to lose the line's angle.

### `Plane::snap_dirs`
[src/tools/perspective.rs:1727](../../../src/tools/perspective.rs#L1727)

```rust
pub fn snap_dirs(&self, at: P, vertical: bool) -> Vec<P>
```

Candidate snap directions at `at`: toward each vanishing point, plus, with `vertical`, the vertical — perpendicular to the horizon, or to the rows when the horizon is at infinity.

### `Plane::vp_point`
[src/tools/perspective.rs:1744](../../../src/tools/perspective.rs#L1744)

```rust
pub fn vp_point(&self, which: u8) -> Option<P>
```

Vanishing point 0 (rows) or 1 (columns), when finite.

## Free functions

### `is_convex`
[src/tools/perspective.rs:1754](../../../src/tools/perspective.rs#L1754)

```rust
pub fn is_convex(c: &[P; 4]) -> bool
```

Whether the quad is strictly convex, in either winding. A corner drag that would fold it into a bow-tie or a dart is refused.

### `centroid`
[src/tools/perspective.rs:1773](../../../src/tools/perspective.rs#L1773)

```rust
pub fn centroid(c: &[P; 4]) -> P
```

The average of the four corners.

### `rotate`
[src/tools/perspective.rs:1781](../../../src/tools/perspective.rs#L1781)

```rust
pub fn rotate(c: &[P; 4], angle: f32) -> [P; 4]
```

Rotate the corners about their centroid.

### `rotate_about`
[src/tools/perspective.rs:1786](../../../src/tools/perspective.rs#L1786)

```rust
fn rotate_about(o: P, p: P, angle: f32) -> P
```

Rotates `p` by `angle` about `o`.

### `carry`
[src/tools/perspective.rs:1795](../../../src/tools/perspective.rs#L1795)

```rust
pub fn carry(from: &[P; 4], to: &[P; 4], p: P) -> P
```

`p` carried along by the move, turn or scale that took corners `from` to `to` — the similarity fixed by the first two corners. For a grid moved or rotated whole, which is when its extra vanishing points come along.

### `place_extra`
[src/tools/perspective.rs:1813](../../../src/tools/perspective.rs#L1813)

```rust
pub fn place_extra(
    horizon: Option<Line>,
    on_horizon: bool,
    to: P,
    detach: f32,
    attach: f32,
) -> (P, bool)
```

Where an extra vanishing point dragged to document point `to` lands, and whether it keeps to the horizon. One on the horizon slides along it until pulled more than `detach` off it, and is then free; a free one sticks back on once within `attach`. Without a horizon it goes where it is put.

### `inside`
[src/tools/perspective.rs:1833](../../../src/tools/perspective.rs#L1833)

```rust
pub(crate) fn inside(c: &[P; 4], p: P) -> bool
```

Whether `p` is inside the quad.

### `tilt_knobs`
[src/tools/perspective.rs:1854](../../../src/tools/perspective.rs#L1854)

```rust
pub fn tilt_knobs(hz: Line, tol: f32) -> [P; 2]
```

The two tilt knobs on horizon `hz`, for a handle radius of `tol`.

### `grab_at`
[src/tools/perspective.rs:1864](../../../src/tools/perspective.rs#L1864)

```rust
pub fn grab_at(c: &[P; 4], extra: &[P], p: P, tol: f32, eye_level: bool) -> Option<GridGrab>
```

What a press at `p` grabs on a grid with corners `c` and extra vanishing points at `extra`, with `tol` the handle radius in document units. Corners win, then the vanishing points, then — with `eye_level`, when the grid shows its horizon — the tilt knobs and the horizon line, then a ring just outside the corners rotates, then anywhere inside moves.

### `dragged`
[src/tools/perspective.rs:1908](../../../src/tools/perspective.rs#L1908)

```rust
pub fn dragged(
    start_c: &[P; 4],
    grab: GridGrab,
    start: P,
    now: P,
    snap: bool,
    rigid: bool,
) -> Option<[P; 4]>
```

The corners after dragging `grab` from `start` to `now`, solved against the corners at press time rather than accumulated. `snap` rounds a rotation to 15° steps, and a tilt to a horizon a whole 15° off level. A `rigid` grid (flat, isometric) keeps its shape: a corner drag turns and scales it about its centre instead. A drag that would break convexity returns `None`.

### `intersect`
[src/tools/perspective.rs:1978](../../../src/tools/perspective.rs#L1978)

```rust
pub(crate) fn intersect(p: P, d: P, q: P, e: P) -> Option<P>
```

Where `p + s·d` meets `q + r·e`, or `None` for parallel lines.

### `move_vp`
[src/tools/perspective.rs:1995](../../../src/tools/perspective.rs#L1995)

```rust
pub fn move_vp(c: &[P; 4], which: u8, v: P) -> Option<[P; 4]>
```

The corners re-aimed so vanishing point `which` (0 rows, 1 columns) sits at `v`, from corners whose vanishing point `which` is finite.

The two edges that run to it swing to meet at `v`. Of the two edges across them, the one farther from the vanishing point — the near edge of the plane — stays put. The far edge keeps its fraction of the way toward the vanishing point and keeps running to the *other* vanishing point, so dragging one never disturbs the other. `None` if that would fold the quad.

### `near_corner`
[src/tools/perspective.rs:2033](../../../src/tools/perspective.rs#L2033)

```rust
fn near_corner(c: &[P; 4], hz: Line) -> usize
```

The corner nearest the viewer: farthest from the horizon.

### `re_aim`
[src/tools/perspective.rs:2043](../../../src/tools/perspective.rs#L2043)

```rust
pub fn re_aim(c: &[P; 4], rows: Vp, cols: Vp) -> Option<[P; 4]>
```

The corners re-aimed so the rows run to `rows` and the columns to `cols`. The corner nearest the viewer stays put; its two edges swing to the new vanishing points keeping their share of the way there (or their length, when either end is at infinity), and the far corner closes the quad. `None` without a horizon, or if the quad would fold.

### `raise_horizon`
[src/tools/perspective.rs:2083](../../../src/tools/perspective.rs#L2083)

```rust
pub fn raise_horizon(c: &[P; 4], by: f32) -> Option<[P; 4]>
```

The corners re-aimed for an eye level moved `by` square to the horizon — along its left normal, the way `cross(d, ·)` measures.

### `tilt_horizon`
[src/tools/perspective.rs:2095](../../../src/tools/perspective.rs#L2095)

```rust
pub fn tilt_horizon(c: &[P; 4], pivot: P, angle: f32) -> Option<[P; 4]>
```

The corners re-aimed for a horizon turned `angle` about `pivot`.

### `tile_family`
[src/tools/perspective.rs:2125](../../../src/tools/perspective.rs#L2125)

```rust
pub fn tile_family(
    plane: &Plane,
    f: Family,
    to_screen: &dyn Fn(P) -> P,
    lo: P,
    hi: P,
    min_gap: f32,
) -> Vec<Tile>
```

Screen segments of family `f`'s lines across the whole plane, clipped to the screen rect `lo..hi`. `to_screen` must be a similarity — the canvas view — so that lines stay lines.

On a parallelogram every line is parallel and evenly spaced on screen, so the lines that cross the rect are found directly; packed closer than `min_gap` screen px, only every 2nd, 4th... is kept. On a receding plane the lines are walked out from the quad's first, both ways, and a way stops once they crowd closer than `min_gap` (they are running into the horizon), pass behind the viewer, or reach `MAX_TILES`.

### `vp_rays`
[src/tools/perspective.rs:2224](../../../src/tools/perspective.rs#L2224)

```rust
pub fn vp_rays(c: &[P; 4], v: P, n: u32) -> Vec<(P, P)>
```

Guide rays from vanishing point `v` fanned across a grid with corners `c`, as segments starting at `v`: `n + 1` of them, spread evenly in angle between the two outermost corners as seen from `v`, each running a little past the farthest corner. All the way round, `2n` of them, when `v` sits inside the grid.

### `fan`
[src/tools/perspective.rs:2260](../../../src/tools/perspective.rs#L2260)

```rust
pub fn fan(v: P, base: P, n: u32, lo: P, hi: P) -> Vec<(u32, P, P)>
```

Rays fanned evenly all the way round `v`, `n` of them, the first along `base`: each with its index, as the part of it inside the rect `lo..hi` — from `v`, or from where it comes in, out to the rect's edge. For a fan over the whole canvas, worked in screen space.

### `fade_pieces`
[src/tools/perspective.rs:2287](../../../src/tools/perspective.rs#L2287)

```rust
pub fn fade_pieces(ra: f32, rb: f32, n: u32) -> Vec<(f32, f32, f32)>
```

A ray of an `n`-ray fan, running from `ra` to `rb` pixels out of its vanishing point, cut into pieces `(from, to, strength)`: nothing where the rays are closer together than `FADE_FROM_GAP`, rising in steps to full strength where they are `FADE_TO_GAP` apart.

### `pick_dir`
[src/tools/perspective.rs:2309](../../../src/tools/perspective.rs#L2309)

```rust
pub fn pick_dir(dirs: &[P], motion: P) -> Option<P>
```

Of `dirs`, the one most nearly parallel to `motion` (either sense).

### `project`
[src/tools/perspective.rs:2317](../../../src/tools/perspective.rs#L2317)

```rust
pub fn project(origin: P, dir: P, p: P) -> P
```

`p` projected onto the line through `origin` along unit `dir`.

### `clip_line`
[src/tools/perspective.rs:2324](../../../src/tools/perspective.rs#L2324)

```rust
pub fn clip_line(a: P, b: P, lo: P, hi: P) -> Option<(f32, f32)>
```

Parameter range `[t0, t1]` of the infinite line `a + t·(b − a)` inside the rectangle `lo..hi`, or `None` when it misses. Liang–Barsky.

## Tests

### `mod tests`

Run with `cargo test tools::perspective::tests` · 61 tests

| Test | What it checks |
|---|---|
| [`presets_put_their_points_where_they_say`](../../../src/tools/perspective.rs#L2355) | Presets put their points where they say. |
| [`the_homography_hits_the_corners`](../../../src/tools/perspective.rs#L2379) | The homography hits the corners. |
| [`unmap_inverts_map`](../../../src/tools/perspective.rs#L2396) | Unmap inverts map. |
| [`nothing_past_the_horizon_is_on_the_plane`](../../../src/tools/perspective.rs#L2410) | Nothing past the horizon is on the plane. |
| [`collinear_corners_have_no_homography`](../../../src/tools/perspective.rs#L2423) | Collinear corners have no homography. |
| [`a_rectangle_has_both_vanishing_points_at_infinity`](../../../src/tools/perspective.rs#L2429) | A rectangle has both vanishing points at infinity. |
| [`a_trapezoid_is_one_point_perspective`](../../../src/tools/perspective.rs#L2437) | A trapezoid is one point perspective. |
| [`a_general_quad_is_two_point_and_its_horizon_joins_them`](../../../src/tools/perspective.rs#L2452) | A general quad is two point and its horizon joins them. |
| [`a_far_vanishing_point_still_anchors_the_horizon_near_the_grid`](../../../src/tools/perspective.rs#L2472) | A far vanishing point still anchors the horizon near the grid. |
| [`dragging_a_vanishing_point_re_aims_the_grid`](../../../src/tools/perspective.rs#L2482) | Dragging a vanishing point re aims the grid. |
| [`dragging_one_vanishing_point_keeps_the_other`](../../../src/tools/perspective.rs#L2495) | Dragging one vanishing point keeps the other. |
| [`a_vanishing_point_is_grabbable_and_drags`](../../../src/tools/perspective.rs#L2509) | A vanishing point is grabbable and drags. |
| [`grid_lines_include_the_borders`](../../../src/tools/perspective.rs#L2518) | Grid lines include the borders. |
| [`convexity_rejects_a_bow_tie`](../../../src/tools/perspective.rs#L2527) | Convexity rejects a bow tie. |
| [`rotation_keeps_the_centroid_and_edges`](../../../src/tools/perspective.rs#L2537) | Rotation keeps the centroid and edges. |
| [`grab_priority_is_corner_then_ring_then_inside`](../../../src/tools/perspective.rs#L2548) | Grab priority is corner then ring then inside. |
| [`grabbing_follows_a_rotation`](../../../src/tools/perspective.rs#L2560) | Grabbing follows a rotation. |
| [`corner_drags_move_one_corner_and_refuse_to_fold`](../../../src/tools/perspective.rs#L2572) | Corner drags move one corner and refuse to fold. |
| [`rotate_drags_snap_to_fifteen_degrees`](../../../src/tools/perspective.rs#L2583) | Rotate drags snap to fifteen degrees. |
| [`snapping_picks_the_nearest_family_and_projects_onto_it`](../../../src/tools/perspective.rs#L2598) | Snapping picks the nearest family and projects onto it. |
| [`frame_relative_corners_follow_the_frame`](../../../src/tools/perspective.rs#L2616) | Frame relative corners follow the frame. |
| [`a_fresh_grid_is_unscaled_and_level`](../../../src/tools/perspective.rs#L2638) | A fresh grid is unscaled and level. |
| [`each_pose_edit_moves_only_its_own_value`](../../../src/tools/perspective.rs#L2646) | Each pose edit moves only its own value. |
| [`clipping_an_infinite_line_to_a_rect`](../../../src/tools/perspective.rs#L2672) | Clipping an infinite line to a rect. |
| [`removing_a_grid_keeps_the_selection_sane`](../../../src/tools/perspective.rs#L2679) | Removing a grid keeps the selection sane. |
| [`an_extra_vp_lands_on_the_horizon_and_rides_it_when_the_grid_is_reaimed`](../../../src/tools/perspective.rs#L2710) | An extra vp lands on the horizon and rides it when the grid is reaimed. |
| [`a_grid_takes_two_extra_vps_one_to_each_side`](../../../src/tools/perspective.rs#L2724) | A grid takes two extra vps one to each side. |
| [`pulling_an_extra_vp_off_the_horizon_frees_it_and_near_it_sticks_again`](../../../src/tools/perspective.rs#L2740) | Pulling an extra vp off the horizon frees it and near it sticks again. |
| [`moving_turning_and_scaling_a_grid_carries_its_extra_vps`](../../../src/tools/perspective.rs#L2754) | Moving turning and scaling a grid carries its extra vps. |
| [`carry_follows_a_whole_grid_move_or_turn`](../../../src/tools/perspective.rs#L2776) | Carry follows a whole grid move or turn. |
| [`extra_vps_are_grabbed_after_the_planes_own`](../../../src/tools/perspective.rs#L2785) | Extra vps are grabbed after the planes own. |
| [`strokes_can_snap_toward_an_extra_vp`](../../../src/tools/perspective.rs#L2797) | Strokes can snap toward an extra vp. |
| [`rays_fan_between_the_outermost_corners`](../../../src/tools/perspective.rs#L2812) | Rays fan between the outermost corners. |
| [`a_quarter_turn_tweens_at_full_size`](../../../src/tools/perspective.rs#L2845) | A quarter turn tweens at full size. |
| [`a_tween_turns_the_short_way_round`](../../../src/tools/perspective.rs#L2857) | A tween turns the short way round. |
| [`scale_and_opacity_tween_like_the_camera`](../../../src/tools/perspective.rs#L2867) | Scale and opacity tween like the camera. |
| [`keys_hold_outside_their_range_and_ease_within`](../../../src/tools/perspective.rs#L2879) | Keys hold outside their range and ease within. |
| [`keys_carry_the_extra_vanishing_points`](../../../src/tools/perspective.rs#L2900) | Keys carry the extra vanishing points. |
| [`a_space_maps_there_and_back`](../../../src/tools/perspective.rs#L2914) | A space maps there and back. |
| [`raising_the_horizon_moves_both_vanishing_points_and_keeps_the_near_corner`](../../../src/tools/perspective.rs#L2940) | Raising the horizon moves both vanishing points and keeps the near corner. |
| [`tilting_the_horizon_turns_it_about_its_pivot`](../../../src/tools/perspective.rs#L2960) | Tilting the horizon turns it about its pivot. |
| [`the_horizon_and_its_knobs_are_grabbable_only_with_the_eye_level_on`](../../../src/tools/perspective.rs#L2977) | The horizon and its knobs are grabbable only with the eye level on. |
| [`a_tilt_snaps_to_level`](../../../src/tools/perspective.rs#L2995) | A tilt snaps to level. |
| [`a_rigid_corner_drag_turns_and_scales_but_keeps_the_square`](../../../src/tools/perspective.rs#L3009) | A rigid corner drag turns and scales but keeps the square. |
| [`a_fresh_flat_or_isometric_grid_is_level_unscaled_and_centred`](../../../src/tools/perspective.rs#L3023) | A fresh flat or isometric grid is level unscaled and centred. |
| [`an_isometric_grid_snaps_three_ways_and_the_third_is_vertical`](../../../src/tools/perspective.rs#L3034) | An isometric grid snaps three ways and the third is vertical. |
| [`a_flat_grid_tiles_the_whole_rect_and_thins_when_dense`](../../../src/tools/perspective.rs#L3052) | A flat grid tiles the whole rect and thins when dense. |
| [`an_infinite_floor_runs_to_the_horizon_and_stops_there`](../../../src/tools/perspective.rs#L3067) | An infinite floor runs to the horizon and stops there. |
| [`a_line_behind_the_viewer_is_not_shown`](../../../src/tools/perspective.rs#L3086) | A line behind the viewer is not shown. |
| [`cell_diagonals_cross_every_cell`](../../../src/tools/perspective.rs#L3096) | Cell diagonals cross every cell. |
| [`a_wall_stands_on_its_edge_and_rises_to_the_vertical_point`](../../../src/tools/perspective.rs#L3109) | A wall stands on its edge and rises to the vertical point. |
| [`a_wall_follows_its_floor_and_comes_loose_without_it`](../../../src/tools/perspective.rs#L3138) | A wall follows its floor and comes loose without it. |
| [`dragging_a_walls_top_corner_sets_its_height`](../../../src/tools/perspective.rs#L3159) | Dragging a walls top corner sets its height. |
| [`ids_are_handed_out_and_kept_unique`](../../../src/tools/perspective.rs#L3177) | Ids are handed out and kept unique. |
| [`the_ceiling_mirrors_a_one_point_floor_across_the_horizon`](../../../src/tools/perspective.rs#L3203) | The ceiling mirrors a one point floor across the horizon. |
| [`the_ceiling_rises_toward_a_vertical_vanishing_point`](../../../src/tools/perspective.rs#L3223) | The ceiling rises toward a vertical vanishing point. |
| [`a_floor_seen_square_on_has_no_ceiling`](../../../src/tools/perspective.rs#L3250) | A floor seen square on has no ceiling. |
| [`a_fan_goes_evenly_round_from_its_base`](../../../src/tools/perspective.rs#L3256) | A fan goes evenly round from its base. |
| [`a_fan_ray_fades_in_where_the_rays_crowd`](../../../src/tools/perspective.rs#L3274) | A fan ray fades in where the rays crowd. |
| [`grids_saved_before_fill_canvas_load_without_it`](../../../src/tools/perspective.rs#L3290) | Grids saved before fill canvas load without it. |
| [`a_curved_guide_is_a_level_box_round_the_middle_and_no_plane`](../../../src/tools/perspective.rs#L3296) | A curved guide is a level box round the middle and no plane. |

**Test helpers** (used only by the tests above):

- [`close`](../../../src/tools/perspective.rs#L2348) — Close.
- [`grid`](../../../src/tools/perspective.rs#L2697) — Grid.
- [`off_horizon`](../../../src/tools/perspective.rs#L2704) — How far `p` is off `g`'s horizon, in document units.
- [`pose`](../../../src/tools/perspective.rs#L2836) — Pose.
- [`room`](../../../src/tools/perspective.rs#L3194) — A one-point floor whose vanishing point is the middle of a 1000 px frame, its rows level, its near edge at y = 800.

**Test-only types and constants:** [`TRAP`](../../../src/tools/perspective.rs#L2352), [`QUAD`](../../../src/tools/perspective.rs#L2376), [`W`](../../../src/tools/perspective.rs#L2694), [`H`](../../../src/tools/perspective.rs#L2695), [`FLOOR2`](../../../src/tools/perspective.rs#L2937)
