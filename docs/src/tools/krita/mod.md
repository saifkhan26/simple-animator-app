# `src/tools/krita/mod.rs` — Krita's pixel brush engine, ported

> **Source:** [src/tools/krita/mod.rs](../../../../src/tools/krita/mod.rs) · **Module:** `crate::tools::krita` · **Size:** 1589 lines · Line links as of commit `1fa5845`

## What this file is for

A port of **Krita's pixel brush** (the `paintbrush` paint operation of the C++ painting program Krita 5.2),
faithful enough that a Krita preset (`.kpp`) paints here the way it paints in Krita — down to the bit, which
the [golden tests](../golden_tests.md) check. Only the features the shipped and importable presets use are
ported; a preset needing anything else is refused with a reason (`MUST_BE_OFF`).

This file holds the two central types:

- **`PixelBrush`** — a loaded, checked preset: its tip (bitmap, generated "auto" tip, or image pipe), the
  sensor curves for size/rotation/opacity/flow/ratio/softness/scatter/texture, spacing, precision, wash vs
  build-up mode.
- **`KritaStroke`** — one stroke painting with a `PixelBrush`: for each dab position the path walker reaches,
  it evaluates sensors (pressure, tilt, speed, distance…), decides the dab's size, angle, opacity, scatter and
  texture (`dab_at`), builds or reuses the dab's alpha mask (`dab`, with Krita's cache rules), and paints it
  (`render` for build-up straight onto the layer, or `darken` into a stroke buffer then `merge` for wash).

```text
pen samples ──► StrokeBuilder (Krita mode) ──► KritaStroke::paint_at(s)  (one per spacing step)
                                                  │  sensors → size, angle, opacity, scatter
                                                  ▼
                                               DabRequest queued
   each frame: KritaStroke::drain ──► dab mask (tip + texture) ──► render / darken+merge ──► canvas
```

### Module map of `tools/krita/`

| Page | Ports |
|---|---|
| [kpp.md](kpp.md) | reading `.kpp` preset files |
| [kpp_write.md](kpp_write.md) | writing them back |
| [edit.md](edit.md) | the brush editor's parameter overrides |
| [defaults.md](defaults.md) | Krita's default option values |
| [options.md](options.md) | the option names this port knows |
| [curve.md](curve.md) | Krita's cubic-spline sensor curves |
| [sensors.md](sensors.md) | pressure, tilt, speed, distance… sensors and `CurveOption` |
| [spacing.md](spacing.md) | dab spacing |
| [distance.md](distance.md) | the path walker that places dabs |
| [speed.md](speed.md) | the speed sensor's smoothing |
| [random.md](random.md) | Krita's random source |
| [texture.md](texture.md) | paper textures |
| [composite.md](composite.md) | blending dabs onto pixels |
| [qt.md](qt.md) | Qt's painting/rounding maths Krita relies on |
| [tip/mod.md](tip/mod.md) | brush tips: bitmap ([gbr](tip/gbr.md)), auto ([auto](tip/auto.md)), fast maths ([simd_math](tip/simd_math.md)) |

## How it connects

- **Used by:** [tools/stroke.rs](../stroke.md) (Krita-mode strokes own a `KritaStroke`), [tools/mod.rs](../mod.md)
  (`KritaPreset`, `BrushMode::Krita`), [brush_library/mod.rs](../../brush_library/mod.md) (loads presets),
  [app.rs](../../app.md) (`krita_brush`), [ui/krita_editor.rs](../../ui/krita_editor.md).
- **Uses:** every file in this folder, [tools/ribbon.rs](../ribbon.md) (`StrokeWorkspace` wash buffer, clip).

## Rust concepts in this file

**Embedded files.** `PENCIL5_KPP` and `GRADIENT_PNG` are `include_bytes!` constants — the preset and its tip are
compiled into the app.

**A trait with blanket-ish impls.** `trait Resources { fn tip(&self, name) -> Option<Cow<[u8]>> }` is
implemented for slices and fixed arrays of `(name, bytes)` pairs (`impl<const N: usize> Resources for [...; N]`
— generic over the array *length*), and by the brush library for folders and bundles.
More: [Traits and derive](../../../rust-primer.md#traits-and-derive)

**`?Sized` bounds.** `load<R: Resources + ?Sized>` accepts unsized types like slices directly.

**Shared mutable state with `Rc<RefCell<…>>`.** `random: Rc<RefCell<RandomSource>>` lets several sensors share
one random generator and mutate it (checked at run time) — single-threaded shared ownership.
More: [Smart pointers](../../../rust-primer.md#smart-pointers)

## Types

### `const PENCIL5_KPP`
[src/tools/krita/mod.rs:66](../../../../src/tools/krita/mod.rs#L66)

```rust
pub(crate) const PENCIL5_KPP: &[u8] = include_bytes!("assets/Pencil-5_Tilted.kpp");
```

Krita's `Pencil-5_Tilted.kpp` preset bytes, embedded.

### `const GRADIENT_PNG`
[src/tools/krita/mod.rs:67](../../../../src/tools/krita/mod.rs#L67)

```rust
pub(crate) const GRADIENT_PNG: &[u8] = include_bytes!("assets/gradient.png");
```

The gradient tip image that preset uses, embedded.

### `enum KritaPreset`
[src/tools/krita/mod.rs:71](../../../../src/tools/krita/mod.rs#L71)

The Krita presets this app carries.

| Variant | Meaning |
|---|---|
| `Pencil5Tilted` | Krita's Pencil-5 Tilted. |

### `struct Precision`
[src/tools/krita/mod.rs:97](../../../../src/tools/krita/mod.rs#L97)

`precisionLevels` in kis_dab_cache_base.cpp: how far a dab may drift from the last one generated before its mask is made afresh.

| Field | Type | Meaning |
|---|---|---|
| `angle` | `f64` | Angle drift allowed before a new mask (radians). |
| `size_frac` | `f64` | Size drift allowed, as a fraction. |
| `sub_pixel` | `f64` | Sub-pixel position drift allowed. |
| `softness` | `f64` | Softness drift allowed. |
| `ratio` | `f64` | Aspect-ratio drift allowed. |

### `const PRECISION_LEVELS`
[src/tools/krita/mod.rs:105](../../../../src/tools/krita/mod.rs#L105)

```rust
const PRECISION_LEVELS: [Precision; 5] = [
    Precision {
        angle: std::f64::consts::PI / 180.0,
        size_frac: 0.05,
        sub_pixel: 1.0,
        softness: 0.01,
// …
```

Krita's table of the five precision levels.

### `struct PrecisionOption`
[src/tools/krita/mod.rs:146](../../../../src/tools/krita/mod.rs#L146)

`KisPrecisionOption`: the preset's precision level, or with auto precision one chosen per dab from its size.

| Field | Type | Meaning |
|---|---|---|
| `level` | `i32` | 1..=5. |
| `auto` | `bool` | Pick the level per dab from its size. |
| `imprecise` | `bool` | Scatter, rotation or a texture is on: dab positions are imprecise anyway, so auto precision may reuse masks more freely. |

### `enum Tip`
[src/tools/krita/mod.rs:171](../../../../src/tools/krita/mod.rs#L171)

A preset's brush tip.

| Variant | Meaning |
|---|---|
| `Png(PngTip)` | A bitmap, scaled to size. |
| `Auto(AutoDef)` | Generated: the size slider resizes the generator itself, so each stroke builds its own `AutoTip` from this. |
| `Pipe { cells: Vec<PngTip>, parasite: Parasite }` | A GIMP image pipe: several bitmaps, one picked per dab. |

### `struct PixelBrush`
[src/tools/krita/mod.rs:182](../../../../src/tools/krita/mod.rs#L182)

A pixel-brush preset, loaded and checked.

| Field | Type | Meaning |
|---|---|---|
| `tip` | `Tip` | The brush tip. |
| `tip_scale` | `f64` | The preset's own `KisBrush::scale`. |
| `size` | `CurveOption` | Sensor curve for size. |
| `rotation` | `CurveOption` | Sensor curve for rotation. |
| `opacity` | `CurveOption` | Sensor curve for opacity. |
| `flow` | `CurveOption` | Sensor curve for flow. |
| `ratio` | `CurveOption` | Sensor curve for the tip's aspect ratio. |
| `softness` | `CurveOption` | Sensor curve for an auto tip's softness. |
| `scatter` | `CurveOption` | Sensor curve for scatter amount. |
| `scatter_axes` | `(bool, bool)` | `Scattering/AxisX`, `AxisY`: which ways scatter throws a dab. Both is in any direction; one is along or across the stroke. |
| `texture_strength` | `CurveOption` | Sensor curve for texture strength. |
| `texture` | `Option<TextureMask>` | The paper texture, if the preset uses one. |
| `spacing` | `f64` | Dab spacing as a fraction of the tip size. |
| `auto_spacing` | `Option<f64>` | Krita's auto spacing coefficient, if on. |
| `isotropic_spacing` | `bool` | Spacing measured the same in x and y (ignoring tip shape). |
| `precision` | `PrecisionOption` | Mask reuse precision. |
| `wash` | `bool` | Wash mode: dabs build up in a stroke buffer with alpha darken, and the buffer is merged onto the layer. Otherwise build-up: each dab goes straight onto the layer. |
| `erase` | `bool` | The preset erases (its blend is `erase`, or it was saved in eraser mode), whatever tool paints with it. |

### `const MUST_BE_OFF`
[src/tools/krita/mod.rs:213](../../../../src/tools/krita/mod.rs#L213)

```rust
const MUST_BE_OFF: &[&str] = &[
    "PressureMirror",
    "PressureSharpness",
    "PressureDarken",
    "PressureMix",
    "Pressureh",
// …
```

Krita options a preset may only carry switched off (features this port doesn't paint).

### `trait Resources`
[src/tools/krita/mod.rs:234](../../../../src/tools/krita/mod.rs#L234)

Where a preset's tip files come from if the preset doesn't embed them.

### `struct DabRequest`
[src/tools/krita/mod.rs:528](../../../../src/tools/krita/mod.rs#L528)

What the stroke asked for at one dab position, decided when the path walker reaches it and painted when the stroke next flushes.

| Field | Type | Meaning |
|---|---|---|
| `x` | `f64` | Dab position x (canvas pixels). |
| `y` | `f64` | Dab position y. |
| `shape` | `DabShape` | Size, angle and ratio of the dab. |
| `opacity` | `f64` | Dab opacity. |
| `flow` | `f64` | Wash mode's alpha-darken flow; build-up ignores it. |
| `cell` | `usize` | A pipe tip's cell for this dab. |
| `softness` | `f64` | The Softness option's factor for an auto tip's fade. |
| `texture_strength` | `f64` | How strongly the texture applies to this dab. |

### `struct DabParams`
[src/tools/krita/mod.rs:544](../../../../src/tools/krita/mod.rs#L544)

`SavedDabParameters`: what decides whether the last mask can be reused.

| Field | Type | Meaning |
|---|---|---|
| `angle` | `f64` | Dab angle. |
| `w` | `i32` | Mask width. |
| `h` | `i32` | Mask height. |
| `sub_x` | `f64` | Sub-pixel x offset. |
| `sub_y` | `f64` | Sub-pixel y offset. |
| `softness` | `f64` | Softness factor. |
| `ratio` | `f64` | Aspect ratio. |
| `index` | `i32` | A pipe tip's cell. |

### `struct CachedDab`
[src/tools/krita/mod.rs:570](../../../../src/tools/krita/mod.rs#L570)

The last mask generated, before texture: Krita's "original device".

| Field | Type | Meaning |
|---|---|---|
| `params` | `DabParams` | The parameters it was made for. |
| `alpha` | `Vec<u8>` | The mask's alpha bytes. |
| `w` | `usize` | Mask width. |
| `h` | `usize` | Mask height. |

### `struct KritaStroke`
[src/tools/krita/mod.rs:578](../../../../src/tools/krita/mod.rs#L578)

One stroke with a Krita brush.

| Field | Type | Meaning |
|---|---|---|
| `brush` | `Arc<PixelBrush>` | The preset painting this stroke. |
| `brush_scale` | `f64` | Krita's tip scale for this stroke's size. |
| `color` | `[u8; 3]` | Paint colour. |
| `opacity` | `CurveOption` | The preset's opacity option with the app's opacity slider as its strength, which is what Krita's own opacity slider sets. |
| `erase` | `bool` | Whether this stroke erases. |
| `average_opacity` | `f64` | Wash mode's running `averageOpacity`, dab to dab. |
| `canvas_rotation` | `f64` | View rotation (degrees) for tilt-direction correction. |
| `mirrored` | `(bool, bool)` | Whether the view is mirrored on each axis. |
| `distance` | `Distance` | The path walker placing dabs (spacing). |
| `history` | `DabHistory` | The dabs painted so far, for the sensors that read the stroke's history. Set by `start` at pen-down. |
| `last_dab` | `Option<PointerSample>` | The last dab painted: where a fan round a corner starts from. |
| `speed` | `speed::SpeedSmoother` | The speed sensor's input, fed every raw sample. |
| `random` | `Rc<RefCell<RandomSource>>` | Shared random source for sensors and scatter. |
| `auto_tip` | `Option<AutoTip>` | An auto tip, built at this stroke's size. |
| `pipe` | `PipeState` | A pipe tip's walk through its cells, and the dabs queued so far (`KisDabRenderingQueue`'s sequence number). |
| `queued` | `i32` | How many dabs have been queued so far. |
| `pending` | `Vec<DabRequest>` | Dabs decided but not yet painted. |
| `cache` | `Option<CachedDab>` | The last generated mask, for reuse. |

## `impl KritaPreset`

### `KritaPreset::brush`
[src/tools/krita/mod.rs:79](../../../../src/tools/krita/mod.rs#L79)

```rust
pub fn brush(self) -> Arc<PixelBrush>
```

The loaded preset. Built once; the embedded files are checked by the tests, so a failure here is a build that should never have shipped.

## `impl PrecisionOption`

### `PrecisionOption::for_dab`
[src/tools/krita/mod.rs:158](../../../../src/tools/krita/mod.rs#L158)

```rust
fn for_dab(&self, dab_size: i32) -> &'static Precision
```

`effectivePrecisionLevel`, as the index `SavedDabParameters::compare` takes. `dab_size` is the smaller side of the dab's mask.

## `trait Resources`

### `Resources::tip`
[src/tools/krita/mod.rs:236](../../../../src/tools/krita/mod.rs#L236)

```rust
fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>>
```

A tip file's bytes by the name the preset gives.

## `impl Resources for [(&str, &[u8])]`

### `Resources::tip`
[src/tools/krita/mod.rs:240](../../../../src/tools/krita/mod.rs#L240)

```rust
fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>>
```

For a slice of `(name, bytes)` pairs: finds the entry with that name.

## `impl<const N: usize> Resources for [(&str, &[u8]); N]`

### `Resources::tip`
[src/tools/krita/mod.rs:246](../../../../src/tools/krita/mod.rs#L246)

```rust
fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>>
```

The same for a fixed-size array of pairs.

## `impl PixelBrush`

### `PixelBrush::load`
[src/tools/krita/mod.rs:254](../../../../src/tools/krita/mod.rs#L254)

```rust
pub fn load<R: Resources + ?Sized>(kpp: &[u8], resources: &R) -> Result<Self>
```

Load a `.kpp`. A tip the preset embeds (a Krita 5 preset carries its own) is used first; otherwise `resources` supplies it.

### `PixelBrush::from_preset`
[src/tools/krita/mod.rs:259](../../../../src/tools/krita/mod.rs#L259)

```rust
pub fn from_preset<R: Resources + ?Sized>(p: &Preset, resources: &R) -> Result<Self>
```

Builds a `PixelBrush` from a parsed preset: checks every option is supported (or switched off), loads the tip
(embedded, or via `resources`), reads each sensor curve option, the texture, spacing and precision settings, and
wash/erase modes. Errors explain what isn't supported.

### `PixelBrush::diameter`
[src/tools/krita/mod.rs:433](../../../../src/tools/krita/mod.rs#L433)

```rust
pub fn diameter(&self) -> f64
```

The preset's own brush size, in pixels.

### `PixelBrush::brush_scale`
[src/tools/krita/mod.rs:452](../../../../src/tools/krita/mod.rs#L452)

```rust
fn brush_scale(&self, diameter: f64) -> f64
```

`KisBrush::scale` for a brush `diameter` pixels across. At the preset's own size this is the preset's scale, not one recomputed from a rounded diameter, so an untouched brush is the preset. An auto tip is resized through its generator instead, so its scale stays 1.

### `PixelBrush::outline`
[src/tools/krita/mod.rs:470](../../../../src/tools/krita/mod.rs#L470)

```rust
pub fn outline(
    &self,
    diameter: f64,
    pressure: f32,
    tilt: (f32, f32),
    rotation_deg: f64,
    mirrored: (bool, bool),
) -> Vec<(f64, f64)>
```

The brush outline for the pen as it is now: the tip's footprint, sized and turned by the same sensors a dab would be, as corners around the pointer in canvas pixels. Krita's cursor does the same (`KisCurrentOutlineFetcher`), so leaning the pen turns and grows it.

## Free functions

### `kpp_base64`
[src/tools/krita/mod.rs:501](../../../../src/tools/krita/mod.rs#L501)

```rust
fn kpp_base64(text: &[u8]) -> Vec<u8>
```

Decodes base64 text embedded in a preset (Krita stores some resources that way).

## `impl DabParams`

### `DabParams::matches`
[src/tools/krita/mod.rs:557](../../../../src/tools/krita/mod.rs#L557)

```rust
fn matches(&self, last: &DabParams, prec: &Precision) -> bool
```

Whether the last mask can be reused for these parameters under the precision limits.

## Free functions

### `mix_krita`
[src/tools/krita/mod.rs:612](../../../../src/tools/krita/mod.rs#L612)

```rust
fn mix_krita(t: f64, a: &PointerSample, b: &PointerSample) -> PointerSample
```

`KisPaintInformation::mix(t, pi1, pi2)`: pressure, tilt, tangential pressure, speed and time lerped in double precision, barrel rotation the short way round.

### `qt_tilt`
[src/tools/krita/mod.rs:637](../../../../src/tools/krita/mod.rs#L637)

```rust
pub fn qt_tilt(t: f32) -> f32
```

Qt hands tilt to Krita in whole degrees, truncated.

### `split`
[src/tools/krita/mod.rs:642](../../../../src/tools/krita/mod.rs#L642)

```rust
fn split(c: f64) -> (i32, f64)
```

`KisPaintOp::splitCoordinate`.

## `impl KritaStroke`

### `KritaStroke::new`
[src/tools/krita/mod.rs:650](../../../../src/tools/krita/mod.rs#L650)

```rust
pub fn new(
    brush: Arc<PixelBrush>,
    diameter: f64,
    color: [u8; 4],
    opacity: f64,
    erase: bool,
    rotation_deg: f64,
    mirrored: (bool, bool),
) -> Self
```

`diameter` is the brush size in canvas pixels, `rotation_deg` the view's rotation, positive clockwise on screen.

### `KritaStroke::start`
[src/tools/krita/mod.rs:693](../../../../src/tools/krita/mod.rs#L693)

```rust
pub fn start(&mut self, s: &PointerSample)
```

Pen-down at `s`: the history starts there. Krita's starting angle comes from the hover path before the press, which the app does not keep; it only shows if the first dab lands exactly on the press.

### `KritaStroke::seed`
[src/tools/krita/mod.rs:699](../../../../src/tools/krita/mod.rs#L699)

```rust
pub fn seed(&mut self, seed: u32)
```

Fix the stroke's random source, so a test sees the same stroke twice.

### `KritaStroke::png`
[src/tools/krita/mod.rs:709](../../../../src/tools/krita/mod.rs#L709)

```rust
fn png(&self) -> &PngTip
```

The bitmap tip in use: the tip, or the pipe's current cell.

### `KritaStroke::png_cell`
[src/tools/krita/mod.rs:713](../../../../src/tools/krita/mod.rs#L713)

```rust
fn png_cell(&self, cell: usize) -> &PngTip
```

The bitmap tip for pipe cell `cell` (or the single tip).

### `KritaStroke::tip_size`
[src/tools/krita/mod.rs:722](../../../../src/tools/krita/mod.rs#L722)

```rust
fn tip_size(&self) -> (i32, i32)
```

`KisBrush::width` / `height`: a pipe's are its first cell's.

### `KritaStroke::characteristic_size`
[src/tools/krita/mod.rs:729](../../../../src/tools/krita/mod.rs#L729)

```rust
fn characteristic_size(&self, shape: DabShape) -> (f64, f64)
```

The tip's characteristic size for a dab shape (Krita's `characteristicSize`), used for spacing.

### `KritaStroke::hot_spot`
[src/tools/krita/mod.rs:736](../../../../src/tools/krita/mod.rs#L736)

```rust
fn hot_spot(&self, shape: DabShape) -> (f64, f64)
```

The tip's hot spot (anchor point) for a dab shape.

### `KritaStroke::mask_size`
[src/tools/krita/mod.rs:743](../../../../src/tools/krita/mod.rs#L743)

```rust
fn mask_size(&self, shape: DabShape, sub_x: f64, sub_y: f64) -> (i32, i32)
```

The pixel size of the dab mask for a shape and sub-pixel offset.

### `KritaStroke::tip_mask`
[src/tools/krita/mod.rs:750](../../../../src/tools/krita/mod.rs#L750)

```rust
fn tip_mask(&self, shape: DabShape, sub_x: f64, sub_y: f64, softness: f64) -> (Vec<u8>, usize, usize)
```

Generates the dab's alpha mask from the tip for a shape, sub-pixel offset and softness.

### `KritaStroke::supports_caching`
[src/tools/krita/mod.rs:757](../../../../src/tools/krita/mod.rs#L757)

```rust
fn supports_caching(&self) -> bool
```

Whether masks may be reused (not for pipes or anything random per dab).

### `KritaStroke::brush`
[src/tools/krita/mod.rs:762](../../../../src/tools/krita/mod.rs#L762)

```rust
pub fn brush(&self) -> &Arc<PixelBrush>
```

The preset this stroke paints with.

### `KritaStroke::note_speed`
[src/tools/krita/mod.rs:769](../../../../src/tools/krita/mod.rs#L769)

```rust
pub fn note_speed(&mut self, s: &mut PointerSample, view_scale: f32)
```

The speed sensor's reading for a raw pen sample (`view_scale` screen pixels per canvas pixel), as `KisPaintingInformationBuilder` takes it before smoothing.

### `KritaStroke::info_with`
[src/tools/krita/mod.rs:778](../../../../src/tools/krita/mod.rs#L778)

```rust
fn info_with(&self, s: &PointerSample, angle: Option<f64>) -> PaintInfo
```

`angle`: a fan dab's drawing angle, overriding the stroke's.

### `KritaStroke::next_point`
[src/tools/krita/mod.rs:798](../../../../src/tools/krita/mod.rs#L798)

```rust
pub fn next_point(&mut self, start: (f32, f32), end: (f32, f32)) -> f32
```

Where along `start → end` the next dab falls (0..=1), or negative.

### `KritaStroke::paint_at`
[src/tools/krita/mod.rs:807](../../../../src/tools/krita/mod.rs#L807)

```rust
pub fn paint_at(&mut self, s: &PointerSample)
```

`KisBrushOp::paintAt`: decide the dab at `s` and the spacing to the next one. The dab is painted by the next `drain`.

### `KritaStroke::paint_one`
[src/tools/krita/mod.rs:819](../../../../src/tools/krita/mod.rs#L819)

```rust
fn paint_one(&mut self, s: &PointerSample, angle: Option<f64>)
```

`KisPaintInformation::paintAt`: decide the dab, then register it, even one too small to paint.

### `KritaStroke::paint_fan`
[src/tools/krita/mod.rs:831](../../../../src/tools/krita/mod.rs#L831)

```rust
fn paint_fan(&mut self, pi1: PointerSample, pi2: PointerSample, step: f64)
```

`KisPaintOpUtils::paintFan`: dabs every `step` radians from the last dab's angle round to the new one, walked from the last dab towards the new one.

### `KritaStroke::scatter`
[src/tools/krita/mod.rs:848](../../../../src/tools/krita/mod.rs#L848)

```rust
fn scatter(&self, info: &PaintInfo, w: i32, h: i32) -> (f64, f64)
```

`KisScatterOption::apply`: where scatter throws the dab, given its unscattered mask size.

### `KritaStroke::dab_at`
[src/tools/krita/mod.rs:873](../../../../src/tools/krita/mod.rs#L873)

```rust
fn dab_at(&mut self, s: &PointerSample, info: PaintInfo)
```

Decides one dab at sample `s`: evaluates every sensor curve (size, rotation with tilt direction and canvas
rotation, ratio, opacity, flow, softness, scatter, texture strength), computes the dab shape, applies scatter,
queues a `DabRequest`, records it in the history, and updates spacing for the next dab.

### `KritaStroke::drain`
[src/tools/krita/mod.rs:952](../../../../src/tools/krita/mod.rs#L952)

```rust
pub fn drain(&mut self, canvas: &mut Canvas, ws: &mut StrokeWorkspace, pre: &[u8]) -> Option<DirtyRect>
```

Paint every dab decided since the last drain onto `canvas`. A wash stroke builds them up in `ws`'s buffer and merges the buffer over `pre`, the layer as it was before the stroke.

### `KritaStroke::dab`
[src/tools/krita/mod.rs:977](../../../../src/tools/krita/mod.rs#L977)

```rust
fn dab(&mut self, req: &DabRequest) -> Option<(Vec<u8>, i32, i32, usize, usize)>
```

The dab's alpha, textured, and where it lands: `(alpha, x, y, w, h)`.

### `KritaStroke::darken`
[src/tools/krita/mod.rs:1027](../../../../src/tools/krita/mod.rs#L1027)

```rust
fn darken(&mut self, req: DabRequest, ws: &mut StrokeWorkspace) -> Option<DirtyRect>
```

Wash: alpha-darken one dab into the stroke buffer, in the order Krita's dab queue hands them over. Returns the buffer rect touched.

### `KritaStroke::merge`
[src/tools/krita/mod.rs:1081](../../../../src/tools/krita/mod.rs#L1081)

```rust
fn merge(&self, canvas: &mut Canvas, ws: &mut StrokeWorkspace, pre: &[u8], r: DirtyRect) -> DirtyRect
```

Wash: the stroke buffer merged onto the pre-stroke layer within `r` (`KisIndirectPaintingSupport::mergeToLayer`): the preset's blend at the opacity slider's strength, held to the selection and the alpha lock. A pure function of `pre` and the buffer, so redoing it every flush is exact.

### `KritaStroke::render`
[src/tools/krita/mod.rs:1110](../../../../src/tools/krita/mod.rs#L1110)

```rust
fn render(&mut self, req: DabRequest, canvas: &mut Canvas, clip: Option<&Arc<Mask>>, alpha_lock: bool) -> Option<DirtyRect>
```

Build-up: one dab straight onto the layer.

## Tests

### `mod tests`

Run with `cargo test tools::krita::tests` · 16 tests

| Test | What it checks |
|---|---|
| [`embedded_pencil_loads_as_krita_configures_it`](../../../../src/tools/krita/mod.rs#L1162) | Embedded pencil loads as krita configures it. |
| [`outline_is_the_bar_sized_and_turned_like_a_dab`](../../../../src/tools/krita/mod.rs#L1216) | The cursor outline is the bar: long and thin, a quarter size with the pen upright and full size laid flat, and it turns with the lean. |
| [`a_krita5_preset_brings_its_own_tip`](../../../../src/tools/krita/mod.rs#L1247) | A Krita 5 preset (`version` 5.0) carries its tip in `<resources>`, so it loads with nothing else supplied. |
| [`unsupported_settings_are_refused_by_name`](../../../../src/tools/krita/mod.rs#L1283) | Unsupported settings are refused by name. |
| [`the_wash_presets_are_wash`](../../../../src/tools/krita/mod.rs#L1355) | The wash presets are wash. |
| [`a_wash_stroke_does_not_darken_where_it_crosses_itself`](../../../../src/tools/krita/mod.rs#L1364) | Going back over the same place within one wash stroke adds nothing: the buffer's alpha darken stops at the dab's opacity. |
| [`the_opacity_slider_sets_the_merge`](../../../../src/tools/krita/mod.rs#L1381) | The opacity slider is the merge's opacity: a stroke at half opacity lands at about half the alpha. |
| [`a_wash_stroke_stays_in_the_selection`](../../../../src/tools/krita/mod.rs#L1394) | The dabs build up unclipped; the selection holds the merge. |
| [`a_wash_stroke_honours_the_alpha_lock`](../../../../src/tools/krita/mod.rs#L1413) | Alpha locked, a wash recolours paint and adds none. |
| [`the_eraser_presets_erase`](../../../../src/tools/krita/mod.rs#L1436) | Krita's erasers carry an erase blend: they erase on any tool, in wash mode, softly at a soft edge. |
| [`the_wash_buffer_is_cleared_between_strokes`](../../../../src/tools/krita/mod.rs#L1452) | Two strokes through one workspace: the second starts with a clear buffer, so it is the same as on a fresh one. |
| [`an_auto_tip_outline_is_its_ellipse`](../../../../src/tools/krita/mod.rs#L1465) | An auto tip's cursor is its ellipse at the dab's size. |
| [`a_sharp_turn_fans_its_dabs`](../../../../src/tools/krita/mod.rs#L1488) | A sharp turn on a drawing-angle preset is filled with a fan of dabs, one every fan step of the turn. |
| [`scatter_throws_dabs_off_the_path`](../../../../src/tools/krita/mod.rs#L1509) | Scatter throws dabs off the path by up to the dab's size times the strength, both ways, the same for the same seed. |
| [`speed_changes_a_speed_driven_preset`](../../../../src/tools/krita/mod.rs#L1528) | Ink-2's size follows speed: the same line drawn faster comes out differently. |
| [`subtract_textured_presets_paint_with_grain`](../../../../src/tools/krita/mod.rs#L1560) | The subtract-textured presets paint, grainy: some pixels inside the stroke are taken back by the paper. |

**Test helpers** (used only by the tests above):

- [`b64`](../../../../src/tools/krita/mod.rs#L1248) — B64.
- [`wash_stroke`](../../../../src/tools/krita/mod.rs#L1317) — Draw `path` with a shipped preset at full pressure, the app's way.
- [`line`](../../../../src/tools/krita/mod.rs#L1346) — Line.
- [`alpha_at`](../../../../src/tools/krita/mod.rs#L1350) — Alpha at.
- [`stroke_for`](../../../../src/tools/krita/mod.rs#L1477) — Stroke for.
