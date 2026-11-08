# `src/input/pointer.rs` — one pointer sample (position, pressure, tilt, time)

> **Source:** [src/input/pointer.rs](../../../src/input/pointer.rs) · **Module:** `crate::input::pointer` · **Size:** 64 lines · Line links as of commit `36405ad`

## What this file is for

Every brush stroke in Animator is a list of *samples*: "at this moment the pen was
here, pressing this hard, tilted this way". This file defines that one record,
`PointerSample`, and nothing else. It is the common language between input
(mouse or pen tablet) and the drawing tools: whatever device produced the input,
the stroke code only ever sees `PointerSample`s.

If you know the browser's [`PointerEvent`](https://developer.mozilla.org/docs/Web/API/PointerEvent),
this is the same idea, trimmed to what a paint program needs:

| `PointerSample` field | Closest `PointerEvent` property |
|---|---|
| `x`, `y` | `offsetX`, `offsetY` (but here in *canvas pixels*, not CSS pixels) |
| `pressure` | `pressure` (0..1) |
| `tilt_x`, `tilt_y` | `tiltX`, `tiltY` (degrees, −90..90) |
| `rotation` | `twist` (degrees; here −180..180 instead of 0..359) |
| `tangential` | `tangentialPressure` (airbrush wheel) |
| `time_ms` | `timeStamp` |

A mouse has no pressure or tilt, so the mouse path fills in pressure = 1.0 and tilt
= (0, 0) — exactly what `PointerSample::default()` gives. The pen-tablet path (see
[tablet.rs](tablet.md)) fills the fields from the tablet's packets instead.

## How it connects

- **Used by:**
  - [app.rs](../app.md) — `AppState::make_sample` (mouse path / stroke start) and
    `AppState::make_pen_sample` (one sample per tablet packet) build these, and
    `pointer_down` / `pointer_move` take them.
  - [ui/shell.rs](../ui/shell.md) — `stroke_sample` picks which of the two builders
    to use for each point of a canvas drag.
  - [tools/stroke.rs](../tools/stroke.md), [tools/stabilizer.rs](../tools/stabilizer.md),
    [tools/krita/mod.rs](../tools/krita/mod.md), [tools/dab.rs](../tools/dab.md) —
    the stroke builder, the smoothing stabilizer and the brush engines consume,
    blend and smooth samples (the Krita engine is what sets `speed`).
  - [app/liquify.rs](../app/liquify.md) — the Liquify tool's `liquify_down` /
    `liquify_move` take samples.
  - [brush_library/mod.rs](../brush_library/mod.md) — builds synthetic samples with
    `PointerSample::at` to draw brush preview strokes.
  - [tools/golden_tests.rs](../tools/golden_tests.md) — builds samples for brush
    regression tests.
- **Uses:** nothing (plain data).

## Rust concepts in this file

**`#[derive(Clone, Copy, Debug)]` — free behaviour for a struct.** `derive` asks the
compiler to write standard trait implementations for you. `Copy` means assigning or
passing a sample copies its bytes (like a JS number), instead of *moving* it;
`Debug` lets you print it with `{:?}`.
```rust
#[derive(Clone, Copy, Debug)]
pub struct PointerSample { pub x: f32, pub y: f32, /* … */ }
```
```js
// JS objects are always shared by reference; the nearest thing to Copy is
const b = { ...a };  // an explicit shallow copy
```
More: [traits and derive](../../rust-primer.md#traits-and-derive),
[ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)

**Implementing the `Default` trait by hand.** `Default` is Rust's "what is an empty /
neutral value of this type" hook. Writing it yourself (instead of deriving it) lets
you choose non-zero defaults — here pressure 1.0.
```rust
impl Default for PointerSample {
    fn default() -> Self { Self { x: 0.0, y: 0.0, pressure: 1.0, /* … */ } }
}
```
More: [traits and derive](../../rust-primer.md#traits-and-derive)

**Struct update syntax `..Self::default()`.** Fill the named fields, take every other
field from another value. It is the Rust spelling of object spread.
```rust
Self { x, y, pressure, ..Self::default() }
```
```js
({ ...defaultSample(), x, y, pressure })
```
Note `x, y, pressure` uses *field init shorthand* (same as JS `{ x, y }`). More:
[structs](../../rust-primer.md#structs)

**`f32` vs `f64`.** `f32` is a 32-bit float (about 7 significant digits); `f64` is a
64-bit float, the same as every JavaScript `number`. Most fields here are `f32`
(fast, plenty for pixels); `time_ms` is `f64` because millisecond timestamps get
large. More: [numbers and casts](../../rust-primer.md#numbers-and-casts)

## impl Default for PointerSample

## impl PointerSample

## Types

### `struct PointerSample`
[src/input/pointer.rs:7](../../../src/input/pointer.rs#L7)

One input sample along a stroke. `Clone + Copy + Debug`.

| Field | Type | Meaning |
|---|---|---|
| `x` | `f32` | Horizontal position in canvas pixel space (the coordinate space of the cell being drawn, not the screen). |
| `y` | `f32` | Vertical position in canvas pixel space. |
| `pressure` | `f32` | Pen pressure, 0..=1. 1.0 for the mouse. |
| `tilt_x` | `f32` | Tilt away from straight-up on the x axis, degrees, −90..=90. 0 for the mouse. |
| `tilt_y` | `f32` | Tilt on the y axis, degrees, −90..=90. |
| `t` | `f32` | Seconds since the program started, at *frame* resolution: every packet in the same frame shares this value. The stroke's curve fitting keys on it, so the source comments warn its meaning must not change. |
| `time_ms` | `f64` | When the sample was taken, in milliseconds: the tablet's own packet time if it reports one, otherwise the frame time. Krita's *speed sensor* (brush dynamics that react to how fast you draw) measures against this. |
| `rotation` | `f32` | Barrel rotation ("twist") of the pen, degrees, −180..=180; 0 when the pen has no such sensor. |
| `tangential` | `f32` | Tangential pressure, 0..=1 — the finger wheel on an airbrush stylus. |
| `speed` | `f32` | Drawing speed for Krita's speed sensor, 0..=1. **Not** set by the input layer; the stroke code fills it in later. |

## `impl Default for PointerSample`

### `PointerSample::default`
[src/input/pointer.rs:36](../../../src/input/pointer.rs#L36)

```rust
fn default() -> Self
```

**What it does.** The neutral sample: at the origin `(0, 0)`, full pressure
(`1.0`), upright (no tilt), every other number `0`. This is what a mouse sample
looks like apart from its position and time.

**Used by:** `PointerSample::at` below, and anywhere code writes
`..Default::default()` / `..Self::default()` when building a sample (for example
`AppState::make_sample` and `make_pen_sample` in [app.rs](../app.md)).

## `impl PointerSample`

### `PointerSample::at`
[src/input/pointer.rs:55](../../../src/input/pointer.rs#L55)

```rust
pub fn at(x: f32, y: f32, pressure: f32) -> Self
```

**What it does.** Shortcut constructor: an upright sample at `(x, y)` with the given
`pressure`; every other field comes from `default()`.

**Parameters.**
- `x`, `y` — position in canvas pixels.
- `pressure` — 0..=1.

**Returns.** The new `PointerSample`.

**Example.**
```rust
let s = PointerSample::at(10.0, 20.0, 0.5);
assert_eq!(s.tilt_x, 0.0);   // the rest is the default
```

**Used by:** [brush_library/mod.rs](../brush_library/mod.md) (preview strokes); also
the test modules of [tools/krita/mod.rs](../tools/krita/mod.md) and
[tools/stabilizer.rs](../tools/stabilizer.md).
