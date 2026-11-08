# `src/tools/stroke.rs` — building a stroke from pen samples

> **Source:** [src/tools/stroke.rs](../../../src/tools/stroke.rs) · **Module:** `crate::tools::stroke` · **Size:** 2127 lines · Line links as of commit `1fa5845`

## What this file is for

`StrokeBuilder` turns the stream of pen samples of one pen-down…pen-up into a painted line. Its model is
ported from Krita's freehand tool (`kis_tool_freehand_helper.cpp`):

```text
pen samples ──► smoothing ──► curve fit ──► spine nodes / dabs ──► rasterise ──► composite onto canvas
 (x, y,          (Basic /       (cubic        (points every         (ribbon.rs    (over the pre-stroke
 pressure,        Weighted /     Bezier        `spacing` along       coverage)     snapshot, opacity)
 tilt, time)      Stabilizer)    between       the curve, each
                                 samples)      with radius/flow)
```

1. **Smoothing** (`smooth`): Krita's weighted filter averages recent samples by *distance travelled*; the
   Stabilizer (see [stabilizer.rs](stabilizer.md)) instead paints on a timer.
2. **Curve fitting** (`paint_bezier_segment`): between consecutive samples a cubic Bezier is fitted to the
   tangents, then subdivided until flat (`paint_bezier_curve`). Because a segment's shape needs the sample
   *after* it, painting always trails input by one sample — hidden by the "live tail" overlay.
3. **Walking** (`paint_line`, `next_point_position`): the curve is walked at fixed spacing, emitting **spine
   nodes** (ribbon mode) or **dabs** (dab mode), each with pressure-driven radius and flow.
4. **Rasterising and compositing** (`flush`, `drain`, `composite`): new geometry is rendered into a coverage
   buffer ([ribbon.rs](ribbon.md)) and composited over the snapshot of the drawing from before the stroke, so
   re-compositing never double-darkens. `flush` returns the changed rectangle for a partial texture upload.
5. **Ends** (`finish`, `recut`): at pen-up the last segment is painted; with flat caps the start and end are
   re-cut square to the stroke.

In **Krita mode** the builder instead feeds the ported Krita engine ([tools/krita/mod.rs](krita/mod.md)),
which paints dabs straight onto the layer.

## How it connects

- **Used by:** [app.rs](../app.md) (`AppState::stroke: Option<StrokeBuilder>`; pointer_down/move/up,
  `flush_stroke`, `tick_stroke`, `stream_stroke`), [ui/shell.rs](../ui/shell.md) (`live_tail`, stabilizer circle).
- **Uses:** [tools/ribbon.rs](ribbon.md) (`SpineNode`, `StrokeWorkspace`, rasterising, `CapPlane`),
  [tools/dab.rs](dab.md), [tools/stabilizer.rs](stabilizer.md), [tools/krita/mod.rs](krita/mod.md),
  [tools/mod.rs](mod.md) (`BrushSettings`, `SmoothingOptions`), [doc/canvas.rs](../doc/canvas.md).

## Rust concepts in this file

**A long-lived builder object with `&mut self` methods.** Like a JS class instance that accumulates state across
events: `builder.push(sample)` on each move, `builder.flush(...)` each frame, `builder.finish(...)` at pen-up.
More: [Impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

**Recursion with a depth cap.** `paint_bezier_curve(…, depth + 1)` subdivides; `MAX_BEZIER_DEPTH` stops a NaN
from recursing forever.

**Small free helper functions** (`lerp`, `mid`, `len`, …) operate on `(f32, f32)` tuples — the file's 2-D
vector type.

## Types

### `const BEZIER_FLATNESS`
[src/tools/stroke.rs:37](../../../src/tools/stroke.rs#L37)

```rust
const BEZIER_FLATNESS: f32 = 0.5;
```

A control point closer than this to the chord means the Bezier is flat enough to draw as a line. Krita's `BEZIER_FLATNESS_THRESHOLD`.

### `const MAX_BEZIER_DEPTH`
[src/tools/stroke.rs:42](../../../src/tools/stroke.rs#L42)

```rust
const MAX_BEZIER_DEPTH: u32 = 24;
```

Recursion cap for the subdivision above. The flatness test terminates on its own for any sane curve; this is here so a NaN or a wildly out-of-range control point cannot blow the stack.

### `const MIN_SPACING`
[src/tools/stroke.rs:45](../../../src/tools/stroke.rs#L45)

```rust
const MIN_SPACING: f32 = 0.5;
```

Floor on node spacing, in canvas pixels. Krita's `MIN_DISTANCE_SPACING`.

### `const SPEED_REF`
[src/tools/stroke.rs:49](../../../src/tools/stroke.rs#L49)

```rust
const SPEED_REF: f32 = 30.0;
```

Distance between samples, in screen pixels, at which the drawing-speed term saturates. See `StrokeBuilder::drawing_speed`.

### `struct StrokeBuilder`
[src/tools/stroke.rs:52](../../../src/tools/stroke.rs#L52)

Builds up a stroke as the pointer moves and rasterizes it incrementally.

| Field | Type | Meaning |
|---|---|---|
| `brush` | `BrushSettings` | The brush this stroke paints with (latched at pen-down). |
| `tool` | `ActiveTool` | The tool (decides e.g. eraser behaviour). |
| `opts` | `SmoothingOptions` | Smoothing options for this stroke. |
| `samples` | `Vec<PointerSample>` | Smoothed input samples for the whole stroke. Doubles as the weighted filter's history. |
| `distance_history` | `Vec<f32>` | Distance from the preceding sample, parallel to `samples`. The weighted filter walks distance, not time. |
| `spine` | `Vec<SpineNode>` | Committed spine nodes — append-only. Under `max()` coverage combining already-rasterized geometry can never be retracted, so nodes are only emitted for curve segments whose shape is final. |
| `arc` | `Vec<f32>` | Arc length from the first node to each spine node, parallel to `spine`. Says which segments are close enough to an end to be cut. |
| `start_cut` | `Option<CapPlane>` | The flat cuts, once known: the start's as soon as the stroke has run far enough to have a direction, the end's at pen-up. |
| `end_cut` | `Option<CapPlane>` | The flat cut at the end, set at pen-up. |
| `dabs` | `Vec<Dab>` | Stamps, in `Dab` mode. Parallel to `spine`, which that mode keeps only so the live-tail overlay has something to draw. |
| `raster_from` | `usize` | First spine (or dab) index not yet rasterized. |
| `previous` | `Option<PointerSample>` | The sample before the current one, and the one before that. Krita's `previousPaintInformation` / `olderPaintInformation`: the curve fit needs a point on each side, so painting always trails input by one sample. The gap is covered by the UI's live-tail overlay. |
| `older` | `Option<PointerSample>` | The sample before `previous` (see `previous`). |
| `have_tangent` | `bool` | Whether `previous_tangent` is valid yet. |
| `previous_tangent` | `(f32, f32)` | The tangent at `previous`, for the next Bezier segment. |
| `spacing_accum` | `f32` | Distance walked since the last spine node, carried across curve segments so spacing is uniform along the whole stroke. |
| `spacing` | `f32` | Distance between emitted points, in canvas pixels. Fixed for a ribbon; in `Dab` mode it is recomputed after every stamp, because spacing is a fraction of a dab whose size follows pressure and tilt. |
| `dot_pending` | `bool` | The pen-down node is in `spine` but has not been rasterized yet. |
| `smooth_pos` | `Option<(f32, f32)>` | Latest smoothed position and pressure, for the live-tail overlay. |
| `smooth_pressure` | `Option<f32>` | Latest smoothed pressure, for the live-tail overlay. |
| `view_scale` | `f32` | Screen pixels per canvas pixel at the time the stroke started. Samples arrive already mapped into canvas space, so this is what lets the filter specify its width in the units the hand and the device actually work in. Latched per stroke — see `AppState::cell_view_scale`. |
| `krita` | `Option<KritaStroke>` | The Krita engine's side of a `Krita`-mode stroke: its own spacing walker and the dabs waiting to be painted. |
| `painted_once` | `bool` | Some segment has been handed to the walker: Krita's `hasPaintAtLeastOnce`, which decides how a stroke ends. |
| `stabilizer` | `Option<Stabilizer>` | The Stabilizer's timer-driven line, once the pen is down. |
| `clock` | `f64` | The stroke's clock, milliseconds, as `tick` last gave it. |

## `impl StrokeBuilder`

### `StrokeBuilder::new`
[src/tools/stroke.rs:116](../../../src/tools/stroke.rs#L116)

```rust
pub fn new(
    brush: BrushSettings,
    tool: ActiveTool,
    view_scale: f32,
    opts: SmoothingOptions,
) -> Self
```

A new stroke for `brush` and `tool` at the given view scale and smoothing (Krita mode uses the brush's built-in
preset).

### `StrokeBuilder::new_with_krita`
[src/tools/stroke.rs:129](../../../src/tools/stroke.rs#L129)

```rust
pub fn new_with_krita(
    brush: BrushSettings,
    tool: ActiveTool,
    view_scale: f32,
    opts: SmoothingOptions,
    krita_brush: Arc<PixelBrush>,
) -> Self
```

`new` with the Krita preset given rather than taken from `brush.krita`: for presets the app loads at run time. Ignored unless `brush.mode` is `Krita`.

### `StrokeBuilder::orient`
[src/tools/stroke.rs:188](../../../src/tools/stroke.rs#L188)

```rust
pub fn orient(&mut self, rotation_deg: f64, mirrored: (bool, bool))
```

Tell a Krita-mode stroke how the view is turned: the rotation of canvas space on screen, degrees clockwise, and whether it is mirrored. Krita's tilt-direction rotation is measured on screen and corrected by this. Call before the first sample.

### `StrokeBuilder::seed_random`
[src/tools/stroke.rs:205](../../../src/tools/stroke.rs#L205)

```rust
pub fn seed_random(&mut self, seed: u32)
```

Fix a Krita-mode stroke's random source (scatter, fuzzy sensors): for tests, which need the same stroke twice.

### `StrokeBuilder::push`
[src/tools/stroke.rs:216](../../../src/tools/stroke.rs#L216)

```rust
pub fn push(&mut self, mut s: PointerSample)
```

Push a pointer sample: smooth it, then extend the stroke.

Painting trails input by one sample, because the Bezier through `older -> previous` is only determined once the sample after `previous` has arrived.

### `StrokeBuilder::stabilizer_settings`
[src/tools/stroke.rs:294](../../../src/tools/stroke.rs#L294)

```rust
fn stabilizer_settings(&self) -> StabilizerSettings
```

The stabilizer's settings for this stroke, from Krita's options.

### `StrokeBuilder::stabilized`
[src/tools/stroke.rs:310](../../../src/tools/stroke.rs#L310)

```rust
pub fn stabilized(&self) -> bool
```

Whether this stroke paints on the stabilizer's timer, so wants `tick` every frame while the pen is down.

### `StrokeBuilder::stabilizer_delay`
[src/tools/stroke.rs:316](../../../src/tools/stroke.rs#L316)

```rust
pub fn stabilizer_delay(&self) -> Option<f32>
```

The stabilizer's string length, screen pixels, when this stroke holds its line back on one.

### `StrokeBuilder::stabilizer_end`
[src/tools/stroke.rs:321](../../../src/tools/stroke.rs#L321)

```rust
pub fn stabilizer_end(&self) -> Option<(f32, f32)>
```

The stabilized line's end, where Krita draws the delay circle.

### `StrokeBuilder::tick`
[src/tools/stroke.rs:330](../../../src/tools/stroke.rs#L330)

```rust
pub fn tick(&mut self, now: f64)
```

Tell the stroke the time, milliseconds on any steady clock. A stabilized stroke paints whatever its timer has made due since.

### `StrokeBuilder::flush`
[src/tools/stroke.rs:349](../../../src/tools/stroke.rs#L349)

```rust
pub fn flush(
    &mut self,
    canvas: &mut Canvas,
    ws: &mut StrokeWorkspace,
    pre: &[u8],
) -> Option<DirtyRect>
```

Rasterize newly committed stroke geometry and composite it into the canvas. Returns the pixel rect updated this call (for partial texture upload), or `None` if nothing changed.

### `StrokeBuilder::finish`
[src/tools/stroke.rs:398](../../../src/tools/stroke.rs#L398)

```rust
pub fn finish(
    &mut self,
    canvas: &mut Canvas,
    ws: &mut StrokeWorkspace,
    pre: &[u8],
) -> Option<DirtyRect>
```

Paint the trailing segment and rasterize everything outstanding. Called on pointer-up.

### `StrokeBuilder::live_tail`
[src/tools/stroke.rs:487](../../../src/tools/stroke.rs#L487)

```rust
pub fn live_tail(&self) -> Option<(SpineNode, (f32, f32))>
```

The uncommitted stroke tail: last committed spine node plus the current smoothed pointer position. Drawn by the UI as an overlay so the one-sample paint lag is invisible.

### `StrokeBuilder::current_node`
[src/tools/stroke.rs:493](../../../src/tools/stroke.rs#L493)

```rust
pub fn current_node(&self) -> Option<SpineNode>
```

Radius / flow modulation at the current smoothed pressure, for the live-tail overlay.

### `StrokeBuilder::smooth`
[src/tools/stroke.rs:516](../../../src/tools/stroke.rs#L516)

```rust
fn smooth(&mut self, info: PointerSample) -> PointerSample
```

Krita's weighted smoothing, from `KisToolFreehandHelper::paint`.

A gaussian average of the recent samples where the weight falls off with *distance travelled* rather than with sample count or elapsed time — time measurements from real devices are too unstable to key a filter on. `Basic` and `None` return the sample untouched, which is what Krita's own default does.

### `StrokeBuilder::effective_smoothness`
[src/tools/stroke.rs:621](../../../src/tools/stroke.rs#L621)

```rust
fn effective_smoothness(&self, speed: f32) -> f32
```

Filter width for the current speed, in canvas pixels.

With `scalable_distance` the configured width means *screen* pixels, so it is divided back out by the zoom. That is the whole trick behind a zoomed-out line staying as clean as a zoomed-in one: the same hand tremor covers more canvas pixels when zoomed out, and so must the filter.

### `StrokeBuilder::drawing_speed`
[src/tools/stroke.rs:637](../../../src/tools/stroke.rs#L637)

```rust
fn drawing_speed(&self) -> f32
```

Stand-in for Krita's `KisPaintInformation::drawingSpeed()`, 0..1.

Krita derives it from device timings; ours comes from the distance between the last two samples in screen pixels, saturating at `SPEED_REF`. With the default options `distance_min == distance_max`, so this term cancels out entirely and only matters once the two are dialled apart.

### `StrokeBuilder::paint_bezier_segment`
[src/tools/stroke.rs:646](../../../src/tools/stroke.rs#L646)

```rust
fn paint_bezier_segment(
    &mut self,
    pi1: PointerSample,
    pi2: PointerSample,
    tangent1: (f32, f32),
    tangent2: (f32, f32),
)
```

Fit a cubic Bezier between two samples given the tangent at each, and paint it. Ported from `KisToolFreehandHelper::paintBezierSegment`.

### `StrokeBuilder::paint_bezier_curve`
[src/tools/stroke.rs:707](../../../src/tools/stroke.rs#L707)

```rust
fn paint_bezier_curve(
    &mut self,
    pi1: PointerSample,
    control1: (f32, f32),
    control2: (f32, f32),
    pi2: PointerSample,
    depth: u32,
)
```

Midpoint subdivision until the curve is flat to within `BEZIER_FLATNESS`, then a straight walk. Foley & Van Dam p.508, via `kis_paintop.cc`.

Subdividing on flatness rather than on a fixed step count is what keeps a big brush's curves smooth: the step count adapts to how curved the segment actually is, not to how wide the brush is.

### `StrokeBuilder::paint_line`
[src/tools/stroke.rs:748](../../../src/tools/stroke.rs#L748)

```rust
fn paint_line(&mut self, from: PointerSample, to: PointerSample)
```

Walk a straight segment at the node spacing, emitting a spine node per step. The leftover distance carries into the next call, so spacing is uniform across segment and curve boundaries alike.

### `StrokeBuilder::next_point_position`
[src/tools/stroke.rs:768](../../../src/tools/stroke.rs#L768)

```rust
fn next_point_position(&mut self, start: (f32, f32), end: (f32, f32)) -> f32
```

Krita's `getNextPointPositionIsotropic`: how far along `start -> end` the next node falls, or a negative number if the segment ends first.

### `StrokeBuilder::emit`
[src/tools/stroke.rs:794](../../../src/tools/stroke.rs#L794)

```rust
fn emit(&mut self, s: &PointerSample)
```

Emit one point of the walked curve: a spine node always, plus a dab when the brush stamps. The dab also sets the next spacing, since spacing is a fraction of a dab whose size follows pressure and tilt.

### `StrokeBuilder::drain`
[src/tools/stroke.rs:812](../../../src/tools/stroke.rs#L812)

```rust
fn drain(&mut self, ws: &mut StrokeWorkspace, mut acc: Option<DirtyRect>) -> Option<DirtyRect>
```

Rasterize geometry appended since the last drain.

### `StrokeBuilder::composite`
[src/tools/stroke.rs:837](../../../src/tools/stroke.rs#L837)

```rust
fn composite(
    &self,
    canvas: &mut Canvas,
    ws: &StrokeWorkspace,
    pre: &[u8],
    rect: Option<DirtyRect>,
)
```

Composite the coverage inside `rect` over the pre-stroke snapshot.

### `StrokeBuilder::flat_ends`
[src/tools/stroke.rs:867](../../../src/tools/stroke.rs#L867)

```rust
fn flat_ends(&self) -> bool
```

Whether this stroke's ends are cut flat (ribbon mode with `StrokeCap::Flat`).

### `StrokeBuilder::cap_zone`
[src/tools/stroke.rs:875](../../../src/tools/stroke.rs#L875)

```rust
fn cap_zone(&self) -> f32
```

Arc length from an end within which a node's round cap can reach past that end: the full-pressure radius, plus its anti-aliasing rim, plus a margin. Anything further along that crosses back over an end is the stroke returning, and stays.

### `StrokeBuilder::total_arc`
[src/tools/stroke.rs:879](../../../src/tools/stroke.rs#L879)

```rust
fn total_arc(&self) -> f32
```

The stroke's total arc length so far.

### `StrokeBuilder::start_plane`
[src/tools/stroke.rs:886](../../../src/tools/stroke.rs#L886)

```rust
fn start_plane(&self) -> Option<CapPlane>
```

The start's cut, square to the direction the stroke leaves in: from the first node towards the first one past the cap zone (or the last, on a short stroke). `None` for a tap, which has no direction.

### `StrokeBuilder::end_plane`
[src/tools/stroke.rs:899](../../../src/tools/stroke.rs#L899)

```rust
fn end_plane(&self) -> Option<CapPlane>
```

The end's cut, the mirror image of `start_plane`: facing back along the direction the stroke arrived from.

### `StrokeBuilder::recut`
[src/tools/stroke.rs:910](../../../src/tools/stroke.rs#L910)

```rust
fn recut(
    &self,
    canvas: &mut Canvas,
    ws: &mut StrokeWorkspace,
    pre: &[u8],
    redraw_start: bool,
    redraw_end: bool,
) -> Option<DirtyRect>
```

Clear and redraw the neighbourhood of the start and/or end, applying each known cut to the segments within its zone. Returns the rect recomposited.

### `StrokeBuilder::node_at`
[src/tools/stroke.rs:974](../../../src/tools/stroke.rs#L974)

```rust
fn node_at(&self, s: &PointerSample) -> SpineNode
```

Spine node at a curve sample: pressure-modulated radius and flow. `brush.opacity` is applied per-stroke at composite time, not here.

## Free functions

### `lerp`
[src/tools/stroke.rs:988](../../../src/tools/stroke.rs#L988)

```rust
fn lerp(a: f32, b: f32, t: f32) -> f32
```

Linear interpolation between two numbers: `a + (b − a)·t`.

### `lerp_pt`
[src/tools/stroke.rs:993](../../../src/tools/stroke.rs#L993)

```rust
fn lerp_pt(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32)
```

Linear interpolation between two points.

### `mid`
[src/tools/stroke.rs:998](../../../src/tools/stroke.rs#L998)

```rust
fn mid(a: (f32, f32), b: (f32, f32)) -> (f32, f32)
```

The midpoint of two points.

### `len`
[src/tools/stroke.rs:1003](../../../src/tools/stroke.rs#L1003)

```rust
fn len(v: (f32, f32)) -> f32
```

The length of a 2-D vector.

### `is_null`
[src/tools/stroke.rs:1008](../../../src/tools/stroke.rs#L1008)

```rust
fn is_null(v: (f32, f32)) -> bool
```

Whether a vector is (nearly) zero.

### `tangent`
[src/tools/stroke.rs:1019](../../../src/tools/stroke.rs#L1019)

```rust
fn tangent(from: PointerSample, to: PointerSample) -> (f32, f32)
```

Velocity between two samples. Time is in seconds here and milliseconds in Krita, but `max(1, dt)` means both collapse to a plain position delta for any realistic sample rate — and every packet drained in one frame shares that frame's timestamp, so the delta is all there is to go on anyway. Only the ratio of two tangent lengths is ever used, so a constant factor is harmless.

### `cut_through`
[src/tools/stroke.rs:1026](../../../src/tools/stroke.rs#L1026)

```rust
fn cut_through(at: SpineNode, toward: SpineNode) -> Option<CapPlane>
```

The cut through `at`, keeping the side `toward` is on. `None` when the two are too close to say which way that is.

### `overlaps`
[src/tools/stroke.rs:1041](../../../src/tools/stroke.rs#L1041)

```rust
fn overlaps(a: DirtyRect, b: DirtyRect) -> bool
```

Whether two rectangles overlap.

### `extend`
[src/tools/stroke.rs:1047](../../../src/tools/stroke.rs#L1047)

```rust
fn extend(from: (f32, f32), toward: (f32, f32), length: f32) -> (f32, f32)
```

`from` moved `length` along the direction of `toward`.

### `line_distance`
[src/tools/stroke.rs:1059](../../../src/tools/stroke.rs#L1059)

```rust
fn line_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32
```

Distance from `p` to the infinite line through `a` and `b`. NaN when the line is degenerate, which the flatness test treats as "draw it straight".

### `line_intersection`
[src/tools/stroke.rs:1071](../../../src/tools/stroke.rs#L1071)

```rust
fn line_intersection(
    a1: (f32, f32),
    a2: (f32, f32),
    b1: (f32, f32),
    b2: (f32, f32),
) -> Option<(f32, f32)>
```

Intersection of the infinite lines `a1->a2` and `b1->b2`, or `None` when they are parallel.

### `segments_cross`
[src/tools/stroke.rs:1090](../../../src/tools/stroke.rs#L1090)

```rust
fn segments_cross(a1: (f32, f32), a2: (f32, f32), b1: (f32, f32), b2: (f32, f32)) -> bool
```

Whether the two *segments* properly cross, i.e. Qt's `QLineF::BoundedIntersection`.

### `mix`
[src/tools/stroke.rs:1105](../../../src/tools/stroke.rs#L1105)

```rust
pub(crate) fn mix(a: PointerSample, b: PointerSample, t: f32) -> PointerSample
```

Blend two samples. Position is included, so callers that want a different position (the Bezier midpoint) overwrite it afterwards.

### `mix_rotation`
[src/tools/stroke.rs:1122](../../../src/tools/stroke.rs#L1122)

```rust
fn mix_rotation(a: f32, b: f32, t: f32) -> f32
```

Barrel rotation between two samples the short way round, as `KisPaintInformation::mixOtherImpl` does it, degrees in -180..=180.

## Tests

### `mod tests`

Run with `cargo test tools::stroke::tests` · 27 tests

| Test | What it checks |
|---|---|
| [`weighted_smoothing_is_zoom_independent`](../../../src/tools/stroke.rs#L1231) | The same gesture at two zooms must be smoothed identically.  This guards the `1 / view_scale` term in `effective_smoothness`: without it the weighted filter would measure its width in canvas pixels, which means a different amount of smoothing at every zoom — and a line that gets wavier the further out you go.  Measured on the filtered input rather than on the emitted spine. The spine also depends on Bezier subdivision, whose flatness threshold is deliberately a canvas-pixel tolerance (that is the space being rasterized), so it is finer relative to the gesture when zoomed out and would blur what this test is asking about. |
| [`weighted_smoothing_rejects_one_pixel_jitter`](../../../src/tools/stroke.rs#L1269) | The reported bug, as a test: a straight line drawn with one screen pixel of jitter — the pointer's own quantization, not hand motion — must come out straighter than it went in, at any zoom. |
| [`a_straight_gesture_stays_straight_at_any_zoom`](../../../src/tools/stroke.rs#L1316) | The reported bug, end to end: a perfectly straight gesture must produce a perfectly straight stroke, at any zoom, with the default (Basic) smoothing that does no positional filtering at all.  This is the shape of the failure the user saw — a slow straight line coming out wavy when zoomed out. What made it wave was the input, not the curve fit: whole-screen-pixel mouse positions become a staircase several canvas pixels wide once one screen pixel covers several canvas pixels. Fed clean samples, the fit is exact. |
| [`quarter_pixel_pen_samples_stay_straight_zoomed_out`](../../../src/tools/stroke.rs#L1341) | What the tablet actually delivers: dense packets on a quarter-pixel grid. Fed only those — no whole-pixel cursor positions slipped in between — a slow straight line must stay straight however far out the view is zoomed. |
| [`dense_nearly_collinear_samples_do_not_bulge`](../../../src/tools/stroke.rs#L1372) | A tablet delivers several packets per frame, a fraction of a pixel apart. Consecutive tangents are then nearly collinear, and the meeting point of two nearly-parallel lines runs off towards infinity — so a control point placed at that meeting point drags the curve with it.  The stroke must stay where it was drawn no matter how densely it is sampled. |
| [`the_stabilizer_trails_then_lands_on_the_pen`](../../../src/tools/stroke.rs#L1406) | Krita's stabilizer: the line follows on its timer, trailing the pen, keeps catching up while the pen is held, and pen-up finishes it on the point where the pen lifted. |
| [`the_stabilizer_string_holds_a_short_stroke`](../../../src/tools/stroke.rs#L1441) | With the delay, a stroke that never leaves the string is one dot. |
| [`basic_smoothing_leaves_positions_alone`](../../../src/tools/stroke.rs#L1461) | Basic smoothing does not touch positions — that is the point of it, and of Krita choosing it as the default. Clean lines come from the curve fit and from the sample fidelity underneath it. |
| [`first_sample_is_exact`](../../../src/tools/stroke.rs#L1470) | Pen-down must land exactly where the user pressed — no filter warm-up. |
| [`spacing_is_uniform_across_segment_boundaries`](../../../src/tools/stroke.rs#L1483) | The spacing walker must emit at a uniform interval, and must carry its remainder across segment boundaries rather than restarting at each one. |
| [`bezier_subdivision_terminates_on_flatness`](../../../src/tools/stroke.rs#L1507) | Subdivision must stop once the control points are within the flatness threshold of the chord — that is what bounds the work, and a curve that never flattens would recurse to the depth cap on every segment. |
| [`large_brushes_do_not_facet`](../../../src/tools/stroke.rs#L1531) | A curved stroke drawn with a large brush must still be tessellated finely. The old fixed step count was `chord / (radius * 0.25)`, which bottomed out at two evaluations per segment once the brush got wide — visible faceting exactly where a wide brush shows it most. |
| [`flat_ends_stop_square_at_the_pen`](../../../src/tools/stroke.rs#L1603) | A flat end stops at the pen, square across. Beyond it the canvas must be exactly what it was — the round end the live stroke showed before the cut must not be left behind. |
| [`round_ends_reach_past_the_pen`](../../../src/tools/stroke.rs#L1618) | Round ends reach past the pen. |
| [`a_flat_tap_is_still_a_dot`](../../../src/tools/stroke.rs#L1632) | A tap has no direction to cut across, and a tap with a pixel or two of tremor must not become a sliver: both stay round dots. |
| [`a_stroke_returning_past_its_start_keeps_that_ink`](../../../src/tools/stroke.rs#L1648) | Only the stroke's own end is cut. A later stretch of the same stroke that runs back past the start is ink, and stays. |
| [`a_dab_brush_ignores_the_cap`](../../../src/tools/stroke.rs#L1659) | Dab coverage builds up, so it cannot be redrawn after the fact; the setting must leave a dab brush alone. |
| [`krita_pencil_paints_only_its_colour_and_is_deterministic`](../../../src/tools/stroke.rs#L1729) | Krita pencil paints only its colour and is deterministic. |
| [`krita_pencil_grows_as_the_pen_tilts`](../../../src/tools/stroke.rs#L1761) | Pencil-5's size follows tilt elevation: a quarter size upright, full size laid flat. A tap shows it without the stroke's own length. |
| [`krita_pencil_turns_with_the_lean`](../../../src/tools/stroke.rs#L1786) | The tip is a thin bar that turns with the lean: leaning towards x and towards y lays the bar at right angles to each other. |
| [`krita_pencil_starts_and_ends_like_krita`](../../../src/tools/stroke.rs#L1819) | Krita puts no dab down at pen-down: the first comes half a pixel along the first segment, and a stroke that never moves gets its one dab at pen-up. |
| [`krita_pencil_honours_selection_and_alpha_lock`](../../../src/tools/stroke.rs#L1843) | Krita pencil honours selection and alpha lock. |
| [`krita_eraser_takes_alpha_away`](../../../src/tools/stroke.rs#L1888) | Krita eraser takes alpha away. |
| [`krita_pencil_across_1080p_is_quick`](../../../src/tools/stroke.rs#L1912) | A full-tilt diagonal across a 1080p cell, timed. Spacing and the dab cache keep it well inside a frame's budget per flush. |
| [`brush_sheet`](../../../src/tools/stroke.rs#L1955) | Render every preset as a stroke on a grey ground, for eyeballing brush feel against a reference. Ignored by default — it writes a file:  ```text PREVIEW_OUT=sheet.png cargo test brush_sheet -- --ignored --nocapture ```  Tuning a brush by slider means a round trip through the GUI and a tablet. This is the short loop for the part you can judge from a picture — density, tooth, taper, how far tilt opens the dab up. |
| [`krita_preset_sheet`](../../../src/tools/stroke.rs#L2002) | Krita's shipped presets, one S-stroke each with pressure easing in and out, for comparing against the same presets in Krita. Ignored by default — it writes a file:  ```text PREVIEW_OUT=krita.png cargo test krita_preset_sheet -- --ignored ``` |
| [`the_airbrush_preset_builds_a_soft_translucent_band`](../../../src/tools/stroke.rs#L2062) | The airbrush preset sprays: one pass leaves a soft band that is densest along the stroke, fades toward its edge, and is still far from opaque — the rest comes from going over it again. |

**Test helpers** (used only by the tests above):

- [`sample`](../../../src/tools/stroke.rs#L1146) — Sample.
- [`builder`](../../../src/tools/stroke.rs#L1158) — Builder.
- [`weighted`](../../../src/tools/stroke.rs#L1167) — Weighted.
- [`spine_in_screen_px`](../../../src/tools/stroke.rs#L1181) — Run a screen-space path through the builder at a given view scale, returning the emitted spine mapped back into screen pixels.
- [`zigzag`](../../../src/tools/stroke.rs#L1209) — A zigzag with a sharp reversal — the shape whose corners are the first thing a badly-scaled filter rounds off.
- [`smoothed_in_screen_px`](../../../src/tools/stroke.rs#L1250) — Run a screen-space path through the filter at a given view scale and return the smoothed positions, back in screen pixels. Positions are divided by the scale on the way in, which is the same gesture drawn with the same on-screen pen at two different zooms.
- [`draw`](../../../src/tools/stroke.rs#L1566) — Draw `path` at full pressure with `brush` on a grey canvas, flushing after every sample as the app does. Returns the canvas and the pre-stroke pixels.
- [`ink`](../../../src/tools/stroke.rs#L1583) — Ink.
- [`px`](../../../src/tools/stroke.rs#L1590) — Px.
- [`horizontal`](../../../src/tools/stroke.rs#L1595) — Horizontal.
- [`px_of`](../../../src/tools/stroke.rs#L1624) — Px of.
- [`krita_stroke`](../../../src/tools/stroke.rs#L1676) — One straight stroke with a Krita brush: `n` samples from `from` to `to`, flushed after each like the app does. A zero-length stroke is a tap.
- [`spread`](../../../src/tools/stroke.rs#L1711) — Total alpha, and its variance along x and along y.
- [`sheet_stroke`](../../../src/tools/stroke.rs#L2097) — One stroke for `brush_sheet`: a shallow S with pressure ramping in and out, plus a little tremor so grain and smoothing both have something to work on.

**Test-only types and constants:** [`GROUND`](../../../src/tools/stroke.rs#L1561)
