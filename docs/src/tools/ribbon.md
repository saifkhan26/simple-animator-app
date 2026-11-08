# `src/tools/ribbon.rs` — the stroke rasteriser (ribbons and dabs)

> **Source:** [src/tools/ribbon.rs](../../../src/tools/ribbon.rs) · **Module:** `crate::tools::ribbon` · **Size:** 1306 lines · Line links as of commit `67a7cb9`

## What this file is for

This is where a stroke's geometry becomes pixels. The stroke (from [stroke.rs](stroke.md)) arrives as a
**spine**: a polyline of `SpineNode`s, each with a position, a pressure-driven radius and a flow. Two ways to
turn it into coverage:

- **Ribbon** (ink): each spine segment is drawn as a variable-radius **capsule** (a rounded segment), and
  capsules combine with `max()` — so where the line overlaps itself it does *not* get darker, and density is
  uniform however slowly you draw.
- **Dabs** (pencil, airbrush): elliptical stamps at a fixed spacing, combined with "build-up"
  (`a + b(1 − a)`), so overlaps darken like graphite.

```text
ribbon: capsules, max()                dabs: stamps, build-up
 ╭──────────────────╮                   ○○○○○○○○○○○○○○
 │●────●────●────●  │  one smooth       overlapping circles
 ╰──────────────────╯  band             darken where they overlap
```

Coverage accumulates in a per-stroke 16-bit buffer (`StrokeWorkspace::cov`), shaped by the brush's hardness,
softness, paper grain and midtone noise, and limited by the selection and alpha lock. It is then **composited
over the snapshot of the drawing taken at pen-down** (`pre`), never over the canvas as it currently is — so
compositing the same area again after more of the stroke arrives gives exactly the same result (no double
darkening). Flat stroke ends are made by drawing round caps and slicing them off with a `CapPlane`.

## How it connects

- **Used by:** [tools/stroke.rs](stroke.md) (all rasterising and compositing), [tools/shape.rs](shape.md),
  [tools/krita/mod.rs](krita/mod.md) (wash buffer, clip), [app.rs](../app.md) (`AppState::stroke_ws`).
- **Uses:** [tools/dab.rs](dab.md), [tools/noise.rs](noise.md), [tools/paper.rs](paper.md),
  [tools/lasso.rs](lasso.md) (`Mask`), [doc/canvas.rs](../doc/canvas.md).

## Rust concepts in this file

**Traits for zero-cost variation.** `trait Shaper { fn skip(&mut self); fn shape(&mut self, c: f32) -> f32; }`
has two implementations: `Plain` (does nothing) and `NoiseRow` (adds noise). `paint_with<S: Shaper>` is
*generic*, so the compiler generates a separate copy of the loop for each — the `Plain` version compiles to the
same code as if noise didn't exist. More: [Traits and derive](../../rust-primer.md#traits-and-derive),
[Generics](../../rust-primer.md#generics)

**Fixed-point coverage.** Coverage is stored as `u16` where 65535 means 1.0 — half the memory of `f32` on a
canvas-sized buffer.

**Lazy clearing.** `begin()` only clears the rectangle the previous stroke touched, not the whole buffer.

## Types

### `trait Shaper`
[src/tools/ribbon.rs:34](../../../src/tools/ribbon.rs#L34)

Turns coverage into ink one pixel at a time along a row (`skip` for pixels that take none). Implemented by
`Plain` and by the noise row walker.

### `struct Plain`
[src/tools/ribbon.rs:41](../../../src/tools/ribbon.rs#L41)

The do-nothing shaper: coverage as is.

### `struct SpineNode`
[src/tools/ribbon.rs:65](../../../src/tools/ribbon.rs#L65)

One vertex of the stroke spine.

| Field | Type | Meaning |
|---|---|---|
| `x` | `f32` | Position x, in cell pixels. |
| `y` | `f32` | Position y. |
| `radius` | `f32` | Brush radius at this node in pixels (pressure-modulated). |
| `flow` | `f32` | Falloff peak at this node, 0..=1 (pressure -> opacity modulation). Final pixel alpha = coverage * per-stroke opacity. |

### `const AA`
[src/tools/ribbon.rs:76](../../../src/tools/ribbon.rs#L76)

```rust
const AA: f32 = 0.5;
```

Half-pixel anti-aliasing band added to the local radius.

### `struct CapPlane`
[src/tools/ribbon.rs:82](../../../src/tools/ribbon.rs#L82)

A straight cut across one end of a stroke: a point on the cut and the unit normal pointing *into* the stroke. Coverage on the far side is removed, with a one-pixel anti-aliased edge centred on the line.

| Field | Type | Meaning |
|---|---|---|
| `x` | `f32` | A point on the cut line (x). |
| `y` | `f32` | A point on the cut line (y). |
| `ux` | `f32` | Unit normal pointing into the stroke (x). |
| `uy` | `f32` | Unit normal (y). |

### `struct StrokeWorkspace`
[src/tools/ribbon.rs:100](../../../src/tools/ribbon.rs#L100)

Persistent, reusable per-stroke scratch state. One per app; `begin()` is called at stroke start and clears only the region the previous stroke touched.

| Field | Type | Meaning |
|---|---|---|
| `cov` | `Vec<u16>` | Canvas-sized coverage, row-major, u16 fixed point (65535 == 1.0). |
| `w` | `u32` | Canvas width the buffers are sized for. |
| `h` | `u32` | Canvas height. |
| `stroke_rect` | `Option<DirtyRect>` | Union of everything rasterized this stroke. Cleared lazily by the next `begin()`. |
| `hardness` | `f32` | Edge hardness, 0..=1: fraction of the radius that is fully solid. 1.0 = crisp edge (falloff only in the ~1px AA rim), 0.0 = airbrush (falloff across the whole radius). |
| `softness` | `f32` | Exponent on the falloff; see `BrushSettings::softness`. |
| `grain` | `f32` | Paper-grain strength, 0..=1: how deeply canvas-position noise eats into the coverage (0 = smooth ink). |
| `grain_scale` | `f32` | Canvas pixels per grain texel. |
| `noise` | `Option<Noise>` | Midtone noise, applied as coverage turns into ink. |
| `build_up` | `bool` | How a new stamp combines with the coverage already there. |
| `clip` | `Option<Arc<Mask>>` | The selection, in this canvas's pixel space, that the stroke is held to — `None` paints freely. Set after `begin()`, which clears it, and constant for the whole stroke: that is what keeps incremental compositing exact under a clip. |
| `alpha_lock` | `bool` | The layer's alpha is locked: paint recolours pixels that already have paint and adds none, and the eraser does nothing. Set after `begin()`, which clears it. |
| `wash` | `Vec<u8>` | A Krita wash stroke's buffer: the alpha its dabs have built up, in the brush colour, before it is merged onto the layer. Canvas-sized, allocated on the first wash stroke only. |
| `wash_rect` | `Option<DirtyRect>` | What the last wash stroke touched, cleared lazily by `begin()`. |

## `trait Shaper`

### `Shaper::skip`
[src/tools/ribbon.rs:35](../../../src/tools/ribbon.rs#L35)

```rust
fn skip(&mut self)
```

Step over a pixel that takes no ink.

### `Shaper::shape`
[src/tools/ribbon.rs:36](../../../src/tools/ribbon.rs#L36)

```rust
fn shape(&mut self, c: f32) -> f32
```

The ink for this pixel's coverage `c`, then move to the next pixel.

## `impl Shaper for Plain`

### `Plain::skip`
[src/tools/ribbon.rs:45](../../../src/tools/ribbon.rs#L45)

```rust
fn skip(&mut self)
```

Does nothing.

### `Plain::shape`
[src/tools/ribbon.rs:47](../../../src/tools/ribbon.rs#L47)

```rust
fn shape(&mut self, c: f32) -> f32
```

Returns `c` unchanged.

## `impl Shaper for NoiseRow<'_>`

### `NoiseRow::skip`
[src/tools/ribbon.rs:54](../../../src/tools/ribbon.rs#L54)

```rust
fn skip(&mut self)
```

Advances the noise walker past a pixel.

### `NoiseRow::shape`
[src/tools/ribbon.rs:58](../../../src/tools/ribbon.rs#L58)

```rust
fn shape(&mut self, c: f32) -> f32
```

Applies midtone noise to coverage `c` (see [noise.rs](noise.md)).

## `impl CapPlane`

### `CapPlane::keep`
[src/tools/ribbon.rs:92](../../../src/tools/ribbon.rs#L92)

```rust
fn keep(&self, fx: f32, fy: f32) -> f32
```

Fraction of a pixel centred at `(fx, fy)` that lies on the kept side.

## `impl StrokeWorkspace`

### `StrokeWorkspace::new`
[src/tools/ribbon.rs:141](../../../src/tools/ribbon.rs#L141)

```rust
pub fn new() -> Self
```

An empty workspace (buffers grow on the first `begin`).

### `StrokeWorkspace::begin`
[src/tools/ribbon.rs:162](../../../src/tools/ribbon.rs#L162)

```rust
pub fn begin(&mut self, w: u32, h: u32, brush: &BrushSettings)
```

Prepare for a new stroke: size the coverage buffer to the target canvas, zero the previous stroke's footprint, latch brush profile.

### `StrokeWorkspace::clear_last`
[src/tools/ribbon.rs:187](../../../src/tools/ribbon.rs#L187)

```rust
pub fn clear_last(&mut self)
```

Clear what the last stroke left in the buffers, so the next `begin()` has nothing to clear: after a stroke across an 8K cell that's ~5 ms, better spent while the app is idle than at pen-down.

### `StrokeWorkspace::is_clear`
[src/tools/ribbon.rs:203](../../../src/tools/ribbon.rs#L203)

```rust
pub fn is_clear(&self) -> bool
```

Whether the last stroke has been cleared away. See `clear_last`.

### `StrokeWorkspace::set_alpha_lock`
[src/tools/ribbon.rs:209](../../../src/tools/ribbon.rs#L209)

```rust
pub fn set_alpha_lock(&mut self, on: bool)
```

Lock the stroke begun by the last `begin()` to the alpha already on the canvas. See `StrokeWorkspace::alpha_lock`.

### `StrokeWorkspace::set_clip`
[src/tools/ribbon.rs:215](../../../src/tools/ribbon.rs#L215)

```rust
pub fn set_clip(&mut self, clip: Option<Arc<Mask>>)
```

Hold the stroke begun by the last `begin()` to `clip`. An empty mask (`w == 0`) means nothing on this canvas may change.

### `StrokeWorkspace::clip`
[src/tools/ribbon.rs:221](../../../src/tools/ribbon.rs#L221)

```rust
pub fn clip(&self) -> Option<&Arc<Mask>>
```

The selection the current stroke is held to, for painters that write the canvas themselves rather than through the coverage buffer.

### `StrokeWorkspace::wash`
[src/tools/ribbon.rs:227](../../../src/tools/ribbon.rs#L227)

```rust
pub fn wash(&mut self) -> (&mut [u8], u32)
```

The wash buffer for the stroke begun by the last `begin()`, all clear at the start of the stroke, and its row width.

### `StrokeWorkspace::note_wash`
[src/tools/ribbon.rs:236](../../../src/tools/ribbon.rs#L236)

```rust
pub fn note_wash(&mut self, r: DirtyRect)
```

Record that the wash buffer was written within `r`, so the next `begin()` clears it.

### `StrokeWorkspace::alpha_lock`
[src/tools/ribbon.rs:241](../../../src/tools/ribbon.rs#L241)

```rust
pub fn alpha_lock(&self) -> bool
```

Whether the current stroke's layer has its alpha locked.

### `StrokeWorkspace::clipped`
[src/tools/ribbon.rs:247](../../../src/tools/ribbon.rs#L247)

```rust
fn clipped(&self, rect: DirtyRect) -> Option<DirtyRect>
```

`rect` cut down to the clip's box — outside it nothing is composited, so those pixels still equal `pre`. `None` when nothing is left.

### `StrokeWorkspace::clip_at`
[src/tools/ribbon.rs:262](../../../src/tools/ribbon.rs#L262)

```rust
fn clip_at(&self, x: u32, y: u32) -> f32
```

Selection coverage at a pixel as a 0..=1 factor on the stroke's alpha.

### `StrokeWorkspace::mask`
[src/tools/ribbon.rs:273](../../../src/tools/ribbon.rs#L273)

```rust
fn mask(&self, d: f32, outer: f32) -> f32
```

Falloff mask for distance `d` from the spine given the local outer radius: fully solid core, smoothstep rim of width `(1 - hardness) * outer`, never narrower than 1px (anti-aliasing).

### `StrokeWorkspace::mask_w`
[src/tools/ribbon.rs:281](../../../src/tools/ribbon.rs#L281)

```rust
fn mask_w(&self, d: f32, outer: f32, min_width: f32) -> f32
```

As `mask`, but with an explicit floor on the falloff width. The dab rasterizer works in normalised ellipse coordinates, where one canvas pixel of anti-aliasing is not one unit.

### `StrokeWorkspace::combine`
[src/tools/ribbon.rs:295](../../../src/tools/ribbon.rs#L295)

```rust
fn combine(&mut self, idx: usize, c16: u16)
```

Combine a new stamp's coverage into the buffer: `max()` for a ribbon, `a + b(1 - a)` for dabs.

### `StrokeWorkspace::pixel_cov`
[src/tools/ribbon.rs:310](../../../src/tools/ribbon.rs#L310)

```rust
fn pixel_cov(&self, mask: f32, flow: f32, x: u32, y: u32) -> u16
```

Grain-attenuated coverage for a pixel. Noise is a pure hash of the canvas position (paper texture): deterministic, so overlapping strokes and `max()` re-rasterization see identical values.

### `StrokeWorkspace::raster_capsule`
[src/tools/ribbon.rs:329](../../../src/tools/ribbon.rs#L329)

```rust
pub fn raster_capsule(&mut self, a: SpineNode, b: SpineNode) -> Option<DirtyRect>
```

Rasterize a variable-radius capsule from `a` to `b` into the coverage buffer with `max()` combining. Returns the clipped pixel bbox touched (exclusive max), or `None` if fully off-canvas.

The projection method is used: each pixel takes the radius/flow lerped at its clamped projection onto the segment. Clamping yields round caps at both ends for free; interior joints are covered by adjacent segments' caps and deduplicated by `max()`. The shape is well-formed for any taper, so no |r0-r1| special-casing is needed.

### `StrokeWorkspace::raster_capsule_clipped`
[src/tools/ribbon.rs:336](../../../src/tools/ribbon.rs#L336)

```rust
pub fn raster_capsule_clipped(
    &mut self,
    a: SpineNode,
    b: SpineNode,
    clips: &[CapPlane],
) -> Option<DirtyRect>
```

`raster_capsule`, with coverage beyond each of `clips` cut away. This is how a flat end is made: the round caps are drawn as usual and then sliced off square.

### `StrokeWorkspace::capsule_box`
[src/tools/ribbon.rs:387](../../../src/tools/ribbon.rs#L387)

```rust
fn capsule_box(&self, a: SpineNode, b: SpineNode) -> Option<(i32, i32, i32, i32)>
```

Inclusive, canvas-clipped pixel box a capsule from `a` to `b` can touch, or `None` if it lies wholly off the canvas. A dot is the capsule from a node to itself.

### `StrokeWorkspace::capsule_rect`
[src/tools/ribbon.rs:397](../../../src/tools/ribbon.rs#L397)

```rust
pub fn capsule_rect(&self, a: SpineNode, b: SpineNode) -> Option<DirtyRect>
```

The rect `raster_capsule(a, b)` would return, without drawing it.

### `StrokeWorkspace::raster_dot`
[src/tools/ribbon.rs:409](../../../src/tools/ribbon.rs#L409)

```rust
pub fn raster_dot(&mut self, n: SpineNode) -> Option<DirtyRect>
```

Rasterize a single dot (tap / first sample): a plain disc, the degenerate case of `raster_capsule`.

### `StrokeWorkspace::raster_dot_clipped`
[src/tools/ribbon.rs:414](../../../src/tools/ribbon.rs#L414)

```rust
pub fn raster_dot_clipped(&mut self, n: SpineNode, clips: &[CapPlane]) -> Option<DirtyRect>
```

`raster_dot` with coverage beyond each of `clips` cut away.

### `StrokeWorkspace::raster_dab`
[src/tools/ribbon.rs:452](../../../src/tools/ribbon.rs#L452)

```rust
pub fn raster_dab(&mut self, d: Dab) -> Option<DirtyRect>
```

Stamp one elliptical dab.

Pixels are tested in the dab's own frame, where it is a unit circle, so a rotated or flattened dab costs no more than a round one and lands exactly on its sub-pixel position. The anti-alias band is derived from the *minor* axis, since that is the direction in which a flattened dab has the least room for a soft edge.

### `StrokeWorkspace::composite_paint`
[src/tools/ribbon.rs:508](../../../src/tools/ribbon.rs#L508)

```rust
pub fn composite_paint(
    &self,
    canvas: &mut Canvas,
    pre: &[u8],
    rect: DirtyRect,
    color: [u8; 4],
    opacity: f32,
)
```

Composite `coverage * opacity` src-over the pre-stroke snapshot into the canvas, only inside `rect`.

Correctness of incremental compositing: the result is a pure function of (pre-stroke pixels, current coverage), never of the canvas's current value, and coverage is monotone non-decreasing under `max()`. Pixels outside `rect` have unchanged coverage and already hold the correct value; pixels inside are recomputed from `pre`. Hence compositing only each flush's new-segment bbox union is exact, even where a new segment overlaps previously composited regions.

Midtone noise, when the brush has it, shapes each pixel's coverage by its canvas position alone, so the result stays a pure function of `pre` and coverage.

### `StrokeWorkspace::paint_with`
[src/tools/ribbon.rs:524](../../../src/tools/ribbon.rs#L524)

```rust
fn paint_with<'a, S: Shaper>(
    &'a self,
    canvas: &mut Canvas,
    pre: &[u8],
    rect: DirtyRect,
    color: [u8; 4],
    opacity: f32,
    row_shaper: impl Fn(u32, u32) -> S + 'a,
)
```

`composite_paint`, with `row(x, y)` shaping the coverage along the row from `(x, y)`.

### `StrokeWorkspace::composite_erase`
[src/tools/ribbon.rs:594](../../../src/tools/ribbon.rs#L594)

```rust
pub fn composite_erase(
    &self,
    canvas: &mut Canvas,
    pre: &[u8],
    rect: DirtyRect,
    strength: f32,
)
```

Eraser composite: `out_alpha = pre_alpha * (1 - coverage * strength)`, RGB carried from the snapshot, zeroed when fully erased. Midtone noise shapes the coverage as it does for paint.

### `StrokeWorkspace::erase_with`
[src/tools/ribbon.rs:609](../../../src/tools/ribbon.rs#L609)

```rust
fn erase_with<'a, S: Shaper>(
    &'a self,
    canvas: &mut Canvas,
    pre: &[u8],
    rect: DirtyRect,
    strength: f32,
    row_shaper: impl Fn(u32, u32) -> S + 'a,
)
```

`composite_erase`, shaped as `paint_with` is.

### `StrokeWorkspace::clear_coverage`
[src/tools/ribbon.rs:659](../../../src/tools/ribbon.rs#L659)

```rust
pub fn clear_coverage(&mut self, rect: DirtyRect)
```

Zero the coverage inside `rect`, ready for it to be redrawn. Only for re-cutting a stroke's ends: it breaks the monotonicity the composite relies on, so follow it with `restore_pre` over the same rect.

### `StrokeWorkspace::restore_pre`
[src/tools/ribbon.rs:671](../../../src/tools/ribbon.rs#L671)

```rust
pub fn restore_pre(&self, canvas: &mut Canvas, pre: &[u8], rect: DirtyRect)
```

Put the pre-stroke pixels back inside `rect`. The composite skips zero-coverage pixels on the grounds that they were never painted; after `clear_coverage` that is no longer true, and without this the stroke's old round ends would stay on the canvas beyond the cut.

### `StrokeWorkspace::touched`
[src/tools/ribbon.rs:682](../../../src/tools/ribbon.rs#L682)

```rust
fn touched(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) -> Option<DirtyRect>
```

Record a touched bbox (inclusive pixel coords) into the stroke rect and return it as an exclusive-max `DirtyRect`.

## Free functions

### `union_rect`
[src/tools/ribbon.rs:695](../../../src/tools/ribbon.rs#L695)

```rust
pub fn union_rect(acc: Option<DirtyRect>, r: DirtyRect) -> DirtyRect
```

Union two dirty rects (the accumulator may be empty).

## Tests

### `mod tests`

Run with `cargo test tools::ribbon::tests` · 29 tests

| Test | What it checks |
|---|---|
| [`zero_length_capsule_matches_dot`](../../../src/tools/ribbon.rs#L741) | Zero length capsule matches dot. |
| [`a_capsule_with_no_cuts_is_unchanged`](../../../src/tools/ribbon.rs#L754) | A capsule with no cuts is unchanged. |
| [`a_cut_removes_everything_beyond_it`](../../../src/tools/ribbon.rs#L768) | A cut through the middle keeps one side, with a one-pixel edge on the line itself. |
| [`hard_brush_has_solid_core`](../../../src/tools/ribbon.rs#L783) | Hard brush has solid core. |
| [`grain_is_deterministic_per_canvas_position`](../../../src/tools/ribbon.rs#L802) | Grain is deterministic per canvas position. |
| [`max_combine_is_monotonic_and_idempotent`](../../../src/tools/ribbon.rs#L819) | Max combine is monotonic and idempotent. |
| [`composite_matches_reference_src_over`](../../../src/tools/ribbon.rs#L831) | Composite matches reference src over. |
| [`noise_breaks_up_the_soft_edge_and_leaves_the_core_and_paper_clean`](../../../src/tools/ribbon.rs#L884) | Noise breaks up the soft edge and leaves the core and paper clean. |
| [`stipple_inks_each_pixel_fully_or_not_at_all`](../../../src/tools/ribbon.rs#L903) | Stipple inks each pixel fully or not at all. |
| [`the_eraser_takes_noise_too`](../../../src/tools/ribbon.rs#L912) | The eraser takes noise too. |
| [`noise_is_anchored_to_the_canvas`](../../../src/tools/ribbon.rs#L929) | The noise belongs to the canvas: compositing a stroke in pieces, as flushes do, gives what compositing it whole does, and a second stroke over the same spot lands on the same specks. |
| [`paints_over_opaque_pixels`](../../../src/tools/ribbon.rs#L950) | Paints over opaque pixels. |
| [`erase_reduces_alpha_proportionally`](../../../src/tools/ribbon.rs#L971) | Erase reduces alpha proportionally. |
| [`begin_clears_previous_stroke_footprint`](../../../src/tools/ribbon.rs#L991) | Begin clears previous stroke footprint. |
| [`dabs_build_up_where_a_ribbon_would_not`](../../../src/tools/ribbon.rs#L1003) | Build-up is the whole reason dabs exist: a second stamp on the same ground must darken it, where a ribbon's `max()` would not. |
| [`build_up_saturates`](../../../src/tools/ribbon.rs#L1023) | Build-up must saturate rather than wrap, or a long stroke over its own path would suddenly go transparent. |
| [`a_flattened_dab_is_an_ellipse_about_its_angle`](../../../src/tools/ribbon.rs#L1039) | A flattened dab must cover ground along its major axis and none across the minor one — that is what puts a tilted pencil on its side. |
| [`rotating_a_dab_rotates_its_footprint`](../../../src/tools/ribbon.rs#L1053) | The same dab rotated a quarter turn must cover the transpose of what it covered before, or the rotation is being applied the wrong way round. |
| [`softness_fades_the_dab_from_inside_its_rim`](../../../src/tools/ribbon.rs#L1077) | Softness must actually soften: raising it has to pull coverage down inside the dab, not only at the rim. |
| [`grain_follows_the_canvas_not_the_stroke`](../../../src/tools/ribbon.rs#L1104) | Grain has to be a function of canvas position, not of stroke position: `max()` and build-up both re-visit pixels, and a grain that moved would make the result depend on visit order. |
| [`alpha_lock_recolours_paint_and_leaves_bare_canvas_bare`](../../../src/tools/ribbon.rs#L1163) | Alpha lock recolours paint and leaves bare canvas bare. |
| [`alpha_lock_stops_the_eraser`](../../../src/tools/ribbon.rs#L1181) | Alpha lock stops the eraser. |
| [`a_selection_holds_paint_to_its_inside`](../../../src/tools/ribbon.rs#L1202) | A selection holds paint to its inside. |
| [`a_full_selection_paints_like_no_selection`](../../../src/tools/ribbon.rs#L1209) | A full selection paints like no selection. |
| [`a_half_selected_pixel_takes_half_the_paint`](../../../src/tools/ribbon.rs#L1222) | A half selected pixel takes half the paint. |
| [`an_empty_selection_changes_nothing`](../../../src/tools/ribbon.rs#L1229) | An empty selection changes nothing. |
| [`the_eraser_is_held_to_the_selection_too`](../../../src/tools/ribbon.rs#L1241) | The eraser is held to the selection too. |
| [`begin_forgets_the_last_strokes_selection`](../../../src/tools/ribbon.rs#L1257) | Begin forgets the last strokes selection. |
| [`clipped_incremental_compositing_matches_one_shot`](../../../src/tools/ribbon.rs#L1268) | Compositing each flush's rect as it comes must land exactly where one composite of the whole stroke does, clip or no clip. |

**Test helpers** (used only by the tests above):

- [`node`](../../../src/tools/ribbon.rs#L711) — Node.
- [`ribbon`](../../../src/tools/ribbon.rs#L716) — A ribbon brush with the given edge profile and grain.
- [`dabs`](../../../src/tools/ribbon.rs#L726) — A dab brush that stamps at full flow, so build-up is visible.
- [`dab`](../../../src/tools/ribbon.rs#L736) — Dab.
- [`soft_dot`](../../../src/tools/ribbon.rs#L858) — A soft dot (hardness 0, so coverage runs from 1 at the centre to 0 at the rim) painted on a bare 64×64 canvas, with `noise`.
- [`half_clip`](../../../src/tools/ribbon.rs#L1124) — A left-half selection on a 64x64 canvas: coverage `k` for x < 32.
- [`painted_through`](../../../src/tools/ribbon.rs#L1135) — Paint a dot through `clip` onto a transparent canvas and return it.
- [`half_blue`](../../../src/tools/ribbon.rs#L1148) — A 64×64 canvas: left half opaque blue, a half-alpha blue column at x = 40, bare elsewhere.
- [`alpha`](../../../src/tools/ribbon.rs#L1197) — Alpha.
