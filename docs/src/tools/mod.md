# `src/tools/mod.rs` — the tools folder, tool list and brush settings

> **Source:** [src/tools/mod.rs](../../../src/tools/mod.rs) · **Module:** `crate::tools` · **Size:** 597 lines · Line links as of commit `67a7cb9`

## What this file is for

`src/tools/mod.rs` is the root of the **tools** folder. It declares every tool module and defines the
types every tool shares: which tool is active (`ActiveTool`), the brush settings each tool paints with
(`BrushSettings`: size, opacity, flow, hardness, grain, noise, pressure response, Krita preset, fill and
liquify options…), line smoothing options, and the brush presets offered in the Brush panel.

### Map of the tools folder

| Tool / feature | Implemented in |
|---|---|
| Pencil, Ink, Eraser — turning pen samples into strokes | [stroke.rs](stroke.md) (stroke builder), [ribbon.rs](ribbon.md) (continuous "ribbon" rasteriser), [dab.rs](dab.md) (stamped dabs), [stabilizer.rs](stabilizer.md) (Krita-style smoothing), [noise.rs](noise.md) (midtone speckle), [paper.rs](paper.md) (paper texture) |
| Krita brushes (Pencil-5 etc.) | [krita/mod.rs](krita/mod.md) and the [krita/](krita/mod.md) folder |
| Fill (bucket) | [fill.rs](fill.md) (flood fill, gap closing), [scan.rs](scan.md) (fast word scans) |
| Fill (lasso mode) | [app/lasso_fill.rs](../app/lasso_fill.md), using [lasso.rs](lasso.md) and [tap_lasso.rs](tap_lasso.md) |
| Shape | [shape.rs](shape.md) |
| Lasso select (freehand, box, ellipse, polygon, magic wand) | [selection.rs](selection.md) (selection mask and floating pixels), [select_mask.rs](select_mask.md) (masks per drawing), [lasso.rs](lasso.md), [tap_lasso.rs](tap_lasso.md), [wand.rs](wand.md) |
| Perspective grids | [perspective.rs](perspective.md) (+ [app/grids.rs](../app/grids.md)) |
| Liquify | [liquify.rs](liquify.md) (+ [app/liquify.rs](../app/liquify.md)) |
| Tracker | in [app.rs](../app.md) and [doc/layer.rs](../doc/layer.md) (`TrackSample`) |
| Shift axis lock | [axis_lock.rs](axis_lock.md) |
| Golden image tests for the Krita engine | [golden_tests.rs](golden_tests.md) |

## How it connects

- **Used by:** [app.rs](../app.md) (`AppState::tool`, `brush`, `tool_brushes`), [ui/shell.rs](../ui/shell.md)
  (Tools and Brush panels, brush settings window), [ui/brush_picker.rs](../ui/brush_picker.md).
- **Uses:** its submodules ([krita](krita/mod.md) for `KritaPreset`, [noise.rs](noise.md),
  [fill.rs](fill.md), [liquify.rs](liquify.md)).

## Rust concepts in this file

**Enums as tool ids, one carrying data.** `ActiveTool::Eraser`, `ActiveTool::Fill`… are plain ids, and
`idx()` maps each to an array slot for per-tool settings (`tool_brushes` in `AppState`).
`ActiveTool::Pen(u8)` carries a number — which pen slot — like a tagged union `{ kind: "pen", slot: 2 }`
in JS; `idx()` is `None` for it, because each slot keeps its own settings in `AppState::pens`.
`ActiveTool::PENCIL` and `ActiveTool::INK` are named constants for the first two slots.

**Presets as functions.** `BrushSettings::default_pencil()`, `pencil_4b()`, … are constructors that start
from `Self::default()` and override a few fields with struct-update syntax:
```rust
pub fn grainy_airbrush() -> Self {
    Self { noise: 0.7, ..Self::airbrush() }
}
```
`PRESETS` is a constant array of `(label, tooltip, fn() -> Self)` — function pointers, so the Brush panel can
build any preset on demand. More: [Structs](../../rust-primer.md#structs), [Closures](../../rust-primer.md#closures)

**`const fn`.** `Dyn::new` is `pub const fn`, so it can be called when initialising constants at compile time.

**`#[serde(default)]` for old preference files.** `BrushSettings` is saved in `ui.ron`; a field missing in an
older file takes its value from `Default`, so adding settings never breaks old prefs.
More: [Serde and file formats](../../rust-primer.md#serde-and-file-formats)

## Types

### `enum ActiveTool`
[src/tools/mod.rs:25](../../../src/tools/mod.rs#L25)

Which tool is active.

| Variant | Meaning |
|---|---|
| `Pen(u8)` | A pen slot, by its place in the row: each holds any brush, with its own size, smoothing, colour, name and icon (`AppState::pens`). The first two start as the pencil and the ink. |
| `Eraser` | The eraser (removes alpha). |
| `Fill` | Fill — bucket or lasso mode. |
| `Shape` | Lines, rectangles and ellipses. |
| `Tracker` | Stabilization tracker — places per-frame tracking points, draws nothing. |
| `Lasso` | Selection — freehand, rectangle, ellipse or polygon. Draws nothing itself; while a selection exists, every paint edit stays inside it. |
| `Perspective` | Perspective grids — edits the viewport guides, draws nothing. |
| `Liquify` | Push the current drawing's pixels around inside the selection. |

### `const ActiveTool::PENCIL`
[src/tools/mod.rs:46](../../../src/tools/mod.rs#L46)

```rust
pub const PENCIL: ActiveTool = ActiveTool::Pen(0);
```

The first pen slot, which starts as the pencil.

### `const ActiveTool::INK`
[src/tools/mod.rs:49](../../../src/tools/mod.rs#L49)

```rust
pub const INK: ActiveTool = ActiveTool::Pen(1);
```

The second pen slot, which starts as the ink.

### `const ActiveTool::OTHERS`
[src/tools/mod.rs:52](../../../src/tools/mod.rs#L52)

```rust
pub const OTHERS: [ActiveTool; 7] = [
    ActiveTool::Eraser,
    ActiveTool::Fill,
    ActiveTool::Shape,
    ActiveTool::Tracker,
    ActiveTool::Lasso,
// …
```

The tools that are not pen slots, in `Self::idx` order.

### `enum ShapeKind`
[src/tools/mod.rs:92](../../../src/tools/mod.rs#L92)

Outline shape drawn by the Shape tool.

| Variant | Meaning |
|---|---|
| `Line` | A straight line between press and release. |
| `Rect` | A rectangle outline (Shift makes it square). |
| `Ellipse` | An ellipse outline (Shift makes it a circle). |

### `enum Smoothing`
[src/tools/mod.rs:100](../../../src/tools/mod.rs#L100)

Line smoothing, following Krita's `KisSmoothingOptions`.

| Variant | Meaning |
|---|---|
| `None` | Straight lines between raw samples. |
| `Basic` | Krita's "Basic": no positional filtering at all, but consecutive samples are joined by a cubic Bezier fitted to the local tangents. This is Krita's own default, and most of what makes its lines clean — its straight lines come from input fidelity and curve fitting rather than from averaging the hand's tremor away. |
| `Weighted` | Krita's "Weighted": a gaussian average over the recent samples, keyed to distance travelled rather than to time (timings are too unstable to key a filter on), then the same Bezier fit. |
| `Stabilizer` | Krita's "Stabilizer": the line follows the pen on a timer, averaging the last `distance_max` milliseconds of movement, and with the delay on it trails the pen on a string `delay_distance` long. |

### `struct SmoothingOptions`
[src/tools/mod.rs:124](../../../src/tools/mod.rs#L124)

Smoothing parameters. Defaults are Krita's own, read from `kis_config.cc`: Basic smoothing, a 50-pixel filter width, 0.15 tail aggressiveness, pressure unsmoothed, distances scaled by zoom.

| Field | Type | Meaning |
|---|---|---|
| `kind` | `Smoothing` | Which smoothing method (None, Basic, Weighted, Stabilizer). |
| `distance_max` | `f32` | Filter width at low speed, in screen pixels. Krita's `LineSmoothingDistanceMax`. |
| `distance_min` | `f32` | Filter width at high speed. Krita's `LineSmoothingDistanceMin`. Equal to `distance_max` by default, which makes speed irrelevant. |
| `tail_aggressiveness` | `f32` | How hard the filter resists the position drift caused by pressure rising at the start of a stroke. Krita's `LineSmoothingTailAggressiveness`. |
| `smooth_pressure` | `bool` | Run pressure through the same filter as position. |
| `scalable_distance` | `bool` | Interpret the distances above in *screen* pixels rather than canvas pixels, so the filter feels identical at every zoom. This is what keeps a zoomed-out line as clean as a zoomed-in one. |
| `delay_distance` | `f32` | Stabilizer: the string's length, screen pixels. Krita's `LineSmoothingDelayDistance`. |
| `use_delay` | `bool` | Stabilizer: hold the line back on the string at all. |
| `finish_stabilized` | `bool` | Stabilizer: at pen-up, draw the line on to where the pen lifted. |
| `stabilize_sensors` | `bool` | Stabilizer: average pressure and tilt with position. |
| `stabilizer_poll_ms` | `u32` | Stabilizer: how often the line catches up, milliseconds. Krita's `stabilizerSampleSize`. |

### `enum BrushMode`
[src/tools/mod.rs:177](../../../src/tools/mod.rs#L177)

How a stroke is turned into pixels.

| Variant | Meaning |
|---|---|
| `Ribbon` | One variable-radius capsule per spine segment, coverage combined with `max()`. Uniform density however slowly the stroke is drawn, and no darkening where it crosses itself — what an ink line wants. |
| `Dab` | Krita's pixel brush: a dab stamped every `spacing` of a diameter, each one accumulating into the coverage already there. Overlapping dabs darken, which is what makes graphite read as graphite rather than as flat fill. |
| `Krita` | A real Krita preset on Krita's own pixel-brush engine (`krita`): its tip, sensors, texture and 8-bit build-up, painted dab by dab straight onto the layer. Which preset is `krita`. |

### `enum StrokeCap`
[src/tools/mod.rs:195](../../../src/tools/mod.rs#L195)

How a ribbon stroke ends, at pen-down and at pen-up.

| Variant | Meaning |
|---|---|
| `Round` | A half-disc, the natural end of a capsule. |
| `Flat` | Cut straight across, square to the stroke, flush with where the pen touched down and lifted. |

### `struct Dyn`
[src/tools/mod.rs:206](../../../src/tools/mod.rs#L206)

Response of a brush property to pressure or tilt.

| Field | Type | Meaning |
|---|---|---|
| `amount` | `f32` | How much of the property the input controls. 0 leaves it constant; 1 lets the input take it all the way down to nothing. |
| `gamma` | `f32` | Curve shape. Below 1 the property rises early, like a blunt nib that reaches full width the moment it touches the paper. Above 1 it holds back until you lean on it, which is what gives a pencil its long taper. |

### `struct BrushSettings`
[src/tools/mod.rs:232](../../../src/tools/mod.rs#L232)

Everything about how a tool paints. One per tool is kept in `AppState::tool_brushes`; the active tool's is
`AppState::brush`. Many fields only matter to some tools (marked in their descriptions).

| Field | Type | Meaning |
|---|---|---|
| `radius` | `f32` | Brush radius in pixels at pressure = 1.0. |
| `opacity` | `f32` | Whole-stroke opacity, 0..=1, applied once at composite time. |
| `flow` | `f32` | Alpha of a single dab at full pressure. Below 1 the stroke builds its density out of overlapping dabs instead of laying it down in one pass, which is where a pencil's mottled darkness comes from. Ignored in `Ribbon` mode, where coverage cannot accumulate. |
| `hardness` | `f32` | Edge hardness, 0..=1: fraction of the radius that is fully solid. 1.0 = crisp edge (~1px AA rim only), 0.0 = airbrush falloff. |
| `softness` | `f32` | Exponent on the edge falloff. 1.0 is a plain smoothstep; higher values pull the taper inwards so the dab fades from well inside its rim — the difference between a hard pencil and a soft one. |
| `spacing` | `f32` | Distance between dabs as a fraction of the dab's *diameter*, which is Krita's convention. `Dab` mode only. |
| `grain` | `f32` | Paper-grain strength, 0..=1. Canvas-position noise eats into the coverage — pencil tooth. 0 = smooth ink. |
| `grain_scale` | `f32` | Canvas pixels per grain texel. Larger values magnify the tooth. |
| `noise` | `f32` | Midtone noise, 0..=1: the stroke's soft falloff broken into specks while its solid core and the bare paper stay clean — see `noise`. 0 = smooth. Not for `Krita` brushes, which paint past where it applies. |
| `noise_size` | `f32` | How big the noise's specks are: canvas pixels per fine speck, or the paper at that many times Pencil-5's scale. |
| `noise_source` | `noise::NoiseSource` | Where the noise comes from. |
| `noise_stipple` | `bool` | Each pixel fully inked or bare by the noise: pure dots. |
| `color` | `[u8; 4]` | Ink color (unmultiplied RGBA). |
| `size` | `Dyn` | Pressure → radius. |
| `flow_dyn` | `Dyn` | Pressure → per-dab alpha. |
| `tilt_elongation` | `f32` | How far tilting the pen flattens the dab into an ellipse, 0..=1. This is what turns a pencil onto its side for shading. |
| `tilt_size` | `f32` | How much tilting the pen widens the dab, 0..=1. The side of a lead covers more paper than its point does. |
| `mode` | `BrushMode` | Rasterization model. See `BrushMode`. |
| `krita` | `krita::KritaPreset` | The Krita preset `Krita` mode paints with. Its own curves, tip and texture replace everything above except radius (Krita's size), opacity (its opacity slider) and colour. |
| `krita_id` | `Option<String>` | A Krita preset from the brush library (`krita/…` shipped, `user/…` imported), which takes the place of `krita` when set. Kept as an id so a preferences file names the brush, not its contents; an id the library no longer has falls back to `krita`. |
| `krita_overrides` | `Vec<(String, String)>` | The brush editor's changes to that preset: `(param, value)` pairs set over its own. Empty for the preset as it comes. |
| `cap` | `StrokeCap` | Shape of the stroke's two ends. `Ribbon` mode only: a dab brush's coverage builds up, so its ends cannot be re-cut after the fact. |
| `fill_tolerance` | `u8` | Flood-fill tolerance per channel (0..=255). Only used by Fill tool. |
| `fill_expand` | `u8` | Grow the filled region by this many pixels after the flood, so colour tucks under the anti-aliased edge of lines living on another layer. Only used by the Fill tool. |
| `fill_gap` | `u8` | Treat breaks in the lines up to this many pixels wide as shut. Only used by the Fill tool. |
| `fill_all_visible` | `bool` | Read the lines from every visible layer at once, instead of the active layer's own pixels or its `lines_from` link. Only used by the Fill tool. |
| `fill_grain` | `f32` | Paper-grain strength on fills, 0..=1 — see `fill::Grain`. Apart from `grain`, the pens' own tooth, which is a different paper on a different scale. Only used by the Fill tool. |
| `fill_grain_size` | `f32` | Size of the fills' paper, 1 = Pencil-5 Tilted's. Only used by the Fill tool. |
| `fill_grain_contrast` | `f32` | Contrast of the fills' paper, 1 = as scanned. Only used by the Fill tool. |
| `fill_grain_boil` | `bool` | Each drawing fills on its own patch of paper. Only used by the Fill tool. |
| `shape_kind` | `ShapeKind` | Outline shape to draw. Only used by the Shape tool. |
| `liquify_strength` | `f32` | How far the pixels follow the pen, 0..=1, at full pressure. Only used by Liquify. |
| `liquify_mode` | `liquify::LiquifyMode` | Push or smooth. Only used by Liquify. |
| `liquify_edge` | `f32` | How many pixels in from a selection's edge the warp eases out over. Only used by Liquify. |
| `liquify_mesh` | `bool` | Draw the warp as a deformed grid. Only used by Liquify. |

### `const BrushSettings::PRESETS`
[src/tools/mod.rs:554](../../../src/tools/mod.rs#L554)

```rust
pub const PRESETS: &'static [(&'static str, &'static str, fn() -> Self)] = &[
    (
        "Pencil",
        "Soft round pencil with visible tooth. After Krita's Pencil-4_Soft.",
        BrushSettings::default_pencil,
    ),
// …
```

Presets offered in the brush panel, as (label, tooltip, builder). Most are starting points matched to the Krita brushes they are named after, not copies of them; "Pencil-5" is the Krita preset itself, on the Krita engine.

## `impl ActiveTool`

### `ActiveTool::idx`
[src/tools/mod.rs:64](../../../src/tools/mod.rs#L64)

```rust
pub fn idx(self) -> Option<usize>
```

The tool's place (0–6) in the per-tool arrays of the tools that are not pens, like `tool_brushes`; `None` for a
pen slot, which keeps its own. The order must stay stable, because saved preferences are indexed by it (after
the pencil and the ink, in the prefs' layout).

### `ActiveTool::pen`
[src/tools/mod.rs:78](../../../src/tools/mod.rs#L78)

```rust
pub fn pen(self) -> Option<usize>
```

The pen slot this is, if it is one.

### `ActiveTool::is_pen`
[src/tools/mod.rs:85](../../../src/tools/mod.rs#L85)

```rust
pub fn is_pen(self) -> bool
```

Whether this is a pen slot — `self.pen().is_some()`.

## `impl Default for SmoothingOptions`

### `SmoothingOptions::default`
[src/tools/mod.rs:156](../../../src/tools/mod.rs#L156)

```rust
fn default() -> Self
```

Krita's own smoothing defaults: Basic smoothing, 50-pixel filter width, 0.15 tail aggressiveness, pressure not
smoothed, distances in screen pixels, stabilizer delay 50 px; the stabilizer polls every 50 ms on Windows (whose
timers are coarse) and 15 ms elsewhere.

## `impl Dyn`

### `Dyn::new`
[src/tools/mod.rs:218](../../../src/tools/mod.rs#L218)

```rust
pub const fn new(amount: f32, gamma: f32) -> Self
```

A pressure/tilt response with the given `amount` and `gamma`.

### `Dyn::apply`
[src/tools/mod.rs:224](../../../src/tools/mod.rs#L224)

```rust
pub fn apply(&self, p: f32) -> f32
```

Turns an input `p` (0–1, e.g. pressure) into a multiplier between `1 − amount` and 1, curved by `gamma`:
`lo + (1 − lo) × p^gamma`.

**Example.** `amount = 0.8`, `gamma = 2`, pressure 0.5 → `lo = 0.2`, `0.2 + 0.8 × 0.25 = 0.4`: the brush is
40% of its full size at half pressure.

## `impl Default for BrushSettings`

### `BrushSettings::default`
[src/tools/mod.rs:341](../../../src/tools/mod.rs#L341)

```rust
fn default() -> Self
```

The plain ribbon brush every preset starts from (and that fills in missing fields of old preference files).

## `impl BrushSettings`

### `BrushSettings::default_pencil`
[src/tools/mod.rs:386](../../../src/tools/mod.rs#L386)

```rust
pub fn default_pencil() -> Self
```

The everyday pencil, modelled on Krita's `Pencil-4_Soft`: soft round tip, visible tooth, tapering at both ends.

### `BrushSettings::pencil_4b`
[src/tools/mod.rs:406](../../../src/tools/mod.rs#L406)

```rust
pub fn pencil_4b() -> Self
```

Modelled on Krita's `Pencil-3_Large_4B`: broad, dark soft graphite with strong grain.

### `BrushSettings::pencil_tilted`
[src/tools/mod.rs:427](../../../src/tools/mod.rs#L427)

```rust
pub fn pencil_tilted() -> Self
```

Modelled on Krita's `Pencil-5_Tilted`: tilting the pen flattens the dab into a broad ellipse for shading.

### `BrushSettings::pencil5_krita`
[src/tools/mod.rs:449](../../../src/tools/mod.rs#L449)

```rust
pub fn pencil5_krita() -> Self
```

Krita's real `Pencil-5_Tilted` preset on the ported Krita engine (`BrushMode::Krita`), with radius half of
Krita's 40 px size.

### `BrushSettings::default_ink`
[src/tools/mod.rs:463](../../../src/tools/mod.rs#L463)

```rust
pub fn default_ink() -> Self
```

Modelled on Krita's `Ink-3_G-Pen`: opaque, crisp, sharp pressure taper, drawn as a ribbon so it never darkens
where it crosses itself.

### `BrushSettings::airbrush`
[src/tools/mod.rs:483](../../../src/tools/mod.rs#L483)

```rust
pub fn airbrush() -> Self
```

Modelled on Krita's `Airbrush_Soft`: a wide, soft dab with low flow that builds up over passes.

### `BrushSettings::grainy_airbrush`
[src/tools/mod.rs:499](../../../src/tools/mod.rs#L499)

```rust
pub fn grainy_airbrush() -> Self
```

The airbrush with midtone noise (fine specks in its soft falloff).

### `BrushSettings::default_eraser`
[src/tools/mod.rs:506](../../../src/tools/mod.rs#L506)

```rust
pub fn default_eraser() -> Self
```

The default eraser: radius 16, transparent colour, half the size and some of the strength follow pressure.

### `BrushSettings::default_fill`
[src/tools/mod.rs:516](../../../src/tools/mod.rs#L516)

```rust
pub fn default_fill() -> Self
```

Fill tool defaults: no pressure dynamics, tolerance 24, no expansion (expanding would eat into the user's own
lines on a same-layer fill).

### `BrushSettings::default_liquify`
[src/tools/mod.rs:532](../../../src/tools/mod.rs#L532)

```rust
pub fn default_liquify() -> Self
```

Liquify brush defaults: radius 60, big enough to move a limb at once.

### `BrushSettings::default_shape`
[src/tools/mod.rs:540](../../../src/tools/mod.rs#L540)

```rust
pub fn default_shape() -> Self
```

Shape tool defaults: radius 3, hard edge, no pressure dynamics.
