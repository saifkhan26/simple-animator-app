# `src/input/button_drag.rs` — what middle / right button (and pen barrel button) drags do

> **Source:** [src/input/button_drag.rs](../../../src/input/button_drag.rs) · **Module:** `crate::input::button_drag` · **Size:** 218 lines · Line links as of commit `67a7cb9`

## What this file is for

On the canvas the **left** mouse button (or the pen tip) always draws with the active tool. The
**middle** and **right** buttons are different: what dragging with them does is a preference the
user picks in Settings — pan the view, zoom, rotate, scrub the timeline, flip through drawings,
adjust the brush, or open a "swatch pie" of colours. Pen tablets report their side ("barrel")
buttons as those same middle/right buttons, so this is also what the pen's side switches do.
The app's defaults are middle = Scrub and right = Swatch pie (set in `UiPrefs` in
[app.rs](../app.md)).

This file holds only the **vocabulary and the maths**, not the behaviour:

- the `ButtonDrag` enum — the list of things a button drag can do, with a label and a tooltip
  for each;
- a **dead zone** and **step** helper, so a value-changing drag ignores the small wobble of a
  plain press and then changes in whole steps;
- the scrub / flip speeds (screen points per frame or per drawing);
- the **swatch pie** geometry: given how far and in which direction the pen is from where it
  was pressed, which colour swatch is it over?

The code that actually runs a drag (moves the playhead, resizes the brush, picks the colour)
lives in [app/button_drag.rs](../app/button_drag.md); the pie is drawn in
[ui/shell.rs](../ui/shell.md).

"Screen points" are egui's logical pixels (physical pixels divided by the display scale), so the
distances feel the same on any monitor.

## How it connects

- **Used by:**
  - [app/button_drag.rs](../app/button_drag.md) — runs the drags: uses `ButtonDrag`,
    `drag_steps`, `past_dead_zone`, `pie_slot`, `SCRUB_PX_PER_FRAME`, `FLIP_PX_PER_KEY`.
  - [app.rs](../app.md) — stores the two choices (`middle_drag`, `right_drag`) in `UiPrefs` and
    `AppState`; `fill_drag_to` uses `drag_steps` for the Fill tool's press-drag (gap / expand);
    `pie_colors` uses `PIE_INNER`.
  - [app/select.rs](../app/select.md) — `wand_drag_to` uses `drag_steps` to retune the magic
    wand's tolerance and gap.
  - [ui/shell.rs](../ui/shell.md) — the Settings picker lists `ButtonDrag::ALL` with `label` /
    `hint`; `draw_swatch_pie` uses `pie_rings`, `PIE_HOLE`, `PIE_RING`, `PIE_OUTER`.
  - [shortcuts.rs](shortcuts.md) — `Action::hold_drag` maps a held key (e.g. "hold to scrub")
    to the same `ButtonDrag` values, so a key can start any of these drags too.
- **Uses:** only `serde` (to save the choice) and the standard library's float maths.

## Rust concepts in this file

**A plain enum as a list of choices.** `ButtonDrag` is like a TypeScript string-union type:
a value is exactly one of the listed names, and the compiler checks every `match` covers all of
them.
```rust
pub enum ButtonDrag { None, Pan, Zoom, Rotate, Scrub, Flip, ToolAdjust, SwatchPie }
```
```ts
type ButtonDrag = "None" | "Pan" | "Zoom" | "Rotate" | "Scrub" | "Flip" | "ToolAdjust" | "SwatchPie";
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

**`#[derive(...)]` for free behaviour.** The line above the enum asks the compiler to generate
code: `Clone, Copy` (pass it around by value like a number), `Debug` (printable), `PartialEq,
Eq` (`==` works), `Serialize, Deserialize` (it can be written to / read from the prefs file by
serde).
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonDrag { /* … */ }
```
More: [Traits and derive](../../rust-primer.md#traits-and-derive),
[Serde and file formats](../../rust-primer.md#serde-and-file-formats)

**`&'static str`.** `label` returns text that is baked into the program and lives forever, so it
can be handed out without copying. In JS every string literal is like this.
More: [Strings and slices](../../rust-primer.md#strings-and-slices),
[Lifetimes](../../rust-primer.md#lifetimes)

**Associated constants on a type.** `ButtonDrag::ALL` is a constant array that belongs to the
type, like a `static readonly ALL` field on a JS class.
```rust
impl ButtonDrag {
    pub const ALL: [ButtonDrag; 8] = [ButtonDrag::Pan, /* … */ ButtonDrag::None];
}
```
`[ButtonDrag; 8]` means "an array of exactly 8 `ButtonDrag`s" — the length is part of the type.
More: [Constants and statics](../../rust-primer.md#constants-and-statics),
[Collections](../../rust-primer.md#collections)

**Float helpers and casts.** `f32` is a 32-bit float. Methods like `abs()`, `signum()`,
`trunc()`, `hypot()`, `atan2()` and `rem_euclid()` are called on the number itself
(`d.abs()` rather than `Math.abs(d)`), and `as i32` / `as usize` convert to integers.
```rust
(past_dead_zone(d) / STEP_PX).trunc() as i32
```
```js
Math.trunc(pastDeadZone(d) / STEP_PX) | 0
```
More: [Numbers and casts](../../rust-primer.md#numbers-and-casts)

**Returning a pair (tuple).** `pie_rings` returns `(usize, usize)` — two numbers at once,
unpacked by the caller with `let (inner, outer) = …`, like returning `[a, b]` in JS and
destructuring it.
More: [Variables and mutability](../../rust-primer.md#variables-and-mutability)

## impl ButtonDrag

## Types

### `enum ButtonDrag`
[src/input/button_drag.rs:11](../../../src/input/button_drag.rs#L11)

What one middle- or right-button drag on the canvas does. Saved in the app's UI preferences, so
the user's choice survives restarts.

| Variant | Meaning |
|---|---|
| `None` | Nothing: the button is ignored on the canvas. |
| `Pan` | Move the view. |
| `Zoom` | Drag up and down to zoom the view. |
| `Rotate` | Drag sideways to rotate the view. |
| `Scrub` | Sideways moves the playhead a frame at a time; with Shift held, one **key drawing** (a frame where a new drawing starts, skipping held frames) at a time. The playhead stays where you let go. |
| `Flip` | Sideways flips through the active layer's drawings, and on release goes back to the frame it started from — the animator's "flip" to check motion without losing the drawing being worked on. |
| `ToolAdjust` | Sideways and up/down change the active tool's two main values: size and opacity for a brush; gap and expand for the Fill bucket. |
| `SwatchPie` | Opens a ring of the pinned colour swatches around the pen; let go over one to pick it, or in the middle to keep the current colour. |

### `const ButtonDrag::ALL`
[src/input/button_drag.rs:34](../../../src/input/button_drag.rs#L34)

```rust
pub const ALL: [ButtonDrag; 8] = [
    ButtonDrag::Pan,
    ButtonDrag::Zoom,
    ButtonDrag::Rotate,
    ButtonDrag::Scrub,
    ButtonDrag::Flip,
// …
```

All eight variants in the order the Settings picker lists them: Pan, Zoom, Rotate, Scrub, Flip,
ToolAdjust, SwatchPie, None.

### `const DEAD_ZONE`
[src/input/button_drag.rs:86](../../../src/input/button_drag.rs#L86)

```rust
const DEAD_ZONE: f32 = 8.0;
```

`8.0` screen points. A value-adjusting drag must move this far before it starts counting, so the
small wander of a plain press never changes a value. Private to this file.

### `const STEP_PX`
[src/input/button_drag.rs:88](../../../src/input/button_drag.rs#L88)

```rust
const STEP_PX: f32 = 12.0;
```

`12.0` screen points of drag per step of a stepped value (one step past the dead zone every 12
points). Private to this file.

### `const SCRUB_PX_PER_FRAME`
[src/input/button_drag.rs:108](../../../src/input/button_drag.rs#L108)

```rust
pub const SCRUB_PX_PER_FRAME: f32 = 10.0;
```

`10.0` screen points of sideways drag per frame when scrubbing.

### `const FLIP_PX_PER_KEY`
[src/input/button_drag.rs:111](../../../src/input/button_drag.rs#L111)

```rust
pub const FLIP_PX_PER_KEY: f32 = 24.0;
```

`24.0` screen points of sideways drag per key drawing when flipping, or when scrubbing with Shift.

### `const PIE_HOLE`
[src/input/button_drag.rs:116](../../../src/input/button_drag.rs#L116)

```rust
pub const PIE_HOLE: f32 = 26.0;
```

The pie's geometry, in screen points from where the button was pressed.

| Constant | Line | Value | Meaning |
|---|---|---|---|
| `PIE_HOLE` | [116](../../../src/input/button_drag.rs#L116) | `26.0` | Radius of the centre "cancel" spot: let go inside it to keep the colour. |
| `PIE_RING` | [117](../../../src/input/button_drag.rs#L117) | `78.0` | Outer edge of the inner ring of swatches. |
| `PIE_OUTER` | [118](../../../src/input/button_drag.rs#L118) | `128.0` | Outer edge of the outer ring (used when there are more swatches than fit on the inner ring). |
| `PIE_INNER` | [121](../../../src/input/button_drag.rs#L121) | `8` | How many swatches usually go on the inner ring. When the colour wheel is pinned, its own colour set goes there instead, however many that is (the caller passes a different `cap`). |

```text
            outer ring: swatches 8.. (only if there are more than `cap`)
        ┌──────────────────────────────┐   ← PIE_OUTER (128)
        │   inner ring: swatches 0..cap │   ← PIE_RING (78)
        │        ┌──────────┐           │
        │        │  cancel  │ ← PIE_HOLE (26)
        │        └──────────┘           │
        └──────────────────────────────┘
   wedge 0 is centred straight up; numbers grow clockwise
```

### `const PIE_RING`
[src/input/button_drag.rs:117](../../../src/input/button_drag.rs#L117)

```rust
pub const PIE_RING: f32 = 78.0;
```

The pie's geometry, in screen points from where the button was pressed.

| Constant | Line | Value | Meaning |
|---|---|---|---|
| `PIE_HOLE` | [116](../../../src/input/button_drag.rs#L116) | `26.0` | Radius of the centre "cancel" spot: let go inside it to keep the colour. |
| `PIE_RING` | [117](../../../src/input/button_drag.rs#L117) | `78.0` | Outer edge of the inner ring of swatches. |
| `PIE_OUTER` | [118](../../../src/input/button_drag.rs#L118) | `128.0` | Outer edge of the outer ring (used when there are more swatches than fit on the inner ring). |
| `PIE_INNER` | [121](../../../src/input/button_drag.rs#L121) | `8` | How many swatches usually go on the inner ring. When the colour wheel is pinned, its own colour set goes there instead, however many that is (the caller passes a different `cap`). |

```text
            outer ring: swatches 8.. (only if there are more than `cap`)
        ┌──────────────────────────────┐   ← PIE_OUTER (128)
        │   inner ring: swatches 0..cap │   ← PIE_RING (78)
        │        ┌──────────┐           │
        │        │  cancel  │ ← PIE_HOLE (26)
        │        └──────────┘           │
        └──────────────────────────────┘
   wedge 0 is centred straight up; numbers grow clockwise
```

### `const PIE_OUTER`
[src/input/button_drag.rs:118](../../../src/input/button_drag.rs#L118)

```rust
pub const PIE_OUTER: f32 = 128.0;
```

The pie's geometry, in screen points from where the button was pressed.

| Constant | Line | Value | Meaning |
|---|---|---|---|
| `PIE_HOLE` | [116](../../../src/input/button_drag.rs#L116) | `26.0` | Radius of the centre "cancel" spot: let go inside it to keep the colour. |
| `PIE_RING` | [117](../../../src/input/button_drag.rs#L117) | `78.0` | Outer edge of the inner ring of swatches. |
| `PIE_OUTER` | [118](../../../src/input/button_drag.rs#L118) | `128.0` | Outer edge of the outer ring (used when there are more swatches than fit on the inner ring). |
| `PIE_INNER` | [121](../../../src/input/button_drag.rs#L121) | `8` | How many swatches usually go on the inner ring. When the colour wheel is pinned, its own colour set goes there instead, however many that is (the caller passes a different `cap`). |

```text
            outer ring: swatches 8.. (only if there are more than `cap`)
        ┌──────────────────────────────┐   ← PIE_OUTER (128)
        │   inner ring: swatches 0..cap │   ← PIE_RING (78)
        │        ┌──────────┐           │
        │        │  cancel  │ ← PIE_HOLE (26)
        │        └──────────┘           │
        └──────────────────────────────┘
   wedge 0 is centred straight up; numbers grow clockwise
```

### `const PIE_INNER`
[src/input/button_drag.rs:121](../../../src/input/button_drag.rs#L121)

```rust
pub const PIE_INNER: usize = 8;
```

The pie's geometry, in screen points from where the button was pressed.

| Constant | Line | Value | Meaning |
|---|---|---|---|
| `PIE_HOLE` | [116](../../../src/input/button_drag.rs#L116) | `26.0` | Radius of the centre "cancel" spot: let go inside it to keep the colour. |
| `PIE_RING` | [117](../../../src/input/button_drag.rs#L117) | `78.0` | Outer edge of the inner ring of swatches. |
| `PIE_OUTER` | [118](../../../src/input/button_drag.rs#L118) | `128.0` | Outer edge of the outer ring (used when there are more swatches than fit on the inner ring). |
| `PIE_INNER` | [121](../../../src/input/button_drag.rs#L121) | `8` | How many swatches usually go on the inner ring. When the colour wheel is pinned, its own colour set goes there instead, however many that is (the caller passes a different `cap`). |

```text
            outer ring: swatches 8.. (only if there are more than `cap`)
        ┌──────────────────────────────┐   ← PIE_OUTER (128)
        │   inner ring: swatches 0..cap │   ← PIE_RING (78)
        │        ┌──────────┐           │
        │        │  cancel  │ ← PIE_HOLE (26)
        │        └──────────┘           │
        └──────────────────────────────┘
   wedge 0 is centred straight up; numbers grow clockwise
```

## `impl ButtonDrag`

### `ButtonDrag::label`
[src/input/button_drag.rs:45](../../../src/input/button_drag.rs#L45)

```rust
pub fn label(self) -> &'static str
```

The short name shown in the Settings drop-down: "Nothing", "Pan canvas", "Zoom canvas",
"Rotate canvas", "Scrub timeline", "Flip drawings", "Adjust tool", "Swatch pie".

**Used by:** [ui/shell.rs](../ui/shell.md) — the "Middle button drag" / "Right button drag"
pickers in Settings.

### `ButtonDrag::hint`
[src/input/button_drag.rs:58](../../../src/input/button_drag.rs#L58)

```rust
pub fn hint(self) -> &'static str
```

The tooltip sentence for each choice, e.g. Scrub: "Drag sideways to move through the frames.
Hold Shift to step from one key drawing to the next." The texts match the variant descriptions
in the table above.

**Used by:** [ui/shell.rs](../ui/shell.md) — hover text on the Settings pickers and their
options.

## Free functions

### `past_dead_zone`
[src/input/button_drag.rs:92](../../../src/input/button_drag.rs#L92)

```rust
pub fn past_dead_zone(d: f32) -> f32
```

**What it does.** Takes a drag distance `d` along one axis (in screen points, positive or
negative) and removes the 8-point dead zone from it, keeping the sign. Anything inside the dead
zone, and `NaN`, gives `0.0`.

**Parameters.**
- `d` — how far the pointer has moved along one axis since the press.

**Returns.** `(|d| - 8) * sign(d)` when `|d| > 8`, else `0.0`. For example `-20.0` → `-12.0`,
`5.0` → `0.0`.

**Used by:** `drag_steps`; [app/button_drag.rs](../app/button_drag.md) (`adjust_tool` — smooth
brush size, opacity and liquify strength changes).

### `drag_steps`
[src/input/button_drag.rs:103](../../../src/input/button_drag.rs#L103)

```rust
pub fn drag_steps(d: f32) -> i32
```

**What it does.** Turns a drag distance into a whole number of steps: none inside the dead
zone, then one more every 12 points, with the sign of `d`.

**Returns.** `trunc(past_dead_zone(d) / 12)` as an integer. So `0..19.9` → `0`, `20` → `1`,
`-20` → `-1`, `40` → `2`.

**Example.**
```js
const DEAD_ZONE = 8, STEP_PX = 12;
function dragSteps(d) {
  const past = Math.abs(d) - DEAD_ZONE;
  if (!(past > 0)) return 0;          // also catches NaN
  return Math.trunc(past * Math.sign(d) / STEP_PX);
}
```

**Used by:** [app/button_drag.rs](../app/button_drag.md) (`adjust_tool` — bucket gap and
expand); [app.rs](../app.md) (`fill_drag_to`); [app/select.rs](../app/select.md)
(`wand_drag_to`).

### `pie_rings`
[src/input/button_drag.rs:125](../../../src/input/button_drag.rs#L125)

```rust
pub fn pie_rings(n: usize, cap: usize) -> (usize, usize)
```

**What it does.** Splits `n` swatches between the inner ring (which takes up to `cap`) and the
outer ring (the rest).

**Returns.** `(inner, outer)` with `inner = min(n, cap)` and `outer = n - inner`. E.g.
`pie_rings(20, 8)` → `(8, 12)`; `pie_rings(5, 2)` → `(2, 3)`.

**Used by:** `pie_slot`; [ui/shell.rs](../ui/shell.md) (`draw_swatch_pie`).

### `pie_heading`
[src/input/button_drag.rs:132](../../../src/input/button_drag.rs#L132)

```rust
pub fn pie_heading(off: [f32; 2]) -> f32
```

**What it does.** The direction of the offset `off = [x, y]` (screen coordinates, so y grows
downwards) as an angle in radians: `0` straight up, growing clockwise, always in `0..2π`.

**How it works.** `atan2(x, -y)` — swapping the usual arguments and flipping y makes "up" the
zero angle and "right" a quarter turn (π/2). `rem_euclid(TAU)` then wraps negative angles into
`0..2π` (like a `%` that never returns a negative number). `TAU` is 2π.

**Used by:** `pie_slot`.

### `wedge`
[src/input/button_drag.rs:137](../../../src/input/button_drag.rs#L137)

```rust
fn wedge(heading: f32, n: usize) -> usize
```

**What it does.** Which of `n` equal wedges a heading falls in, with wedge 0 **centred** on
straight up (so it reaches half a wedge either side of the top).

**How it works.** `step = 2π / n`; the result is `floor((heading + step/2) / step) % n`. Adding
half a step shifts the boundaries so the top is the middle of wedge 0; the `% n` folds the last
half-wedge (just left of the top) back into wedge 0.

**Used by:** `pie_slot`. Private.

### `pie_slot`
[src/input/button_drag.rs:145](../../../src/input/button_drag.rs#L145)

```rust
pub fn pie_slot(n: usize, cap: usize, off: [f32; 2]) -> Option<usize>
```

**What it does.** Which swatch the pen is over, given its offset `off` from the pie's centre.
Returns `None` over the cancel spot (or when there are no swatches). Beyond the rings still
counts, by direction alone, so a quick flick picks as well as a careful aim.

**Parameters.**
- `n` — how many swatches the pie shows.
- `cap` — how many of them go on the inner ring (`PIE_INNER` normally, or the pinned wheel's
  set size).
- `off` — pen position minus press position, in screen points.

**Returns.** `Some(index)` into the swatch list, or `None` for "keep the colour".

**How it works.**
1. Distance `d = hypot(x, y)`. If `n == 0` or `d < PIE_HOLE` → `None`.
2. Split into `(inner, outer)` with `pie_rings`.
3. Find the heading with `pie_heading`.
4. If there is no outer ring, or `d < PIE_RING`, the answer is `wedge(heading, inner)`.
5. Otherwise it is `inner + wedge(heading, outer)` — an outer-ring swatch, however far out.

**Example.** With 20 swatches and `cap = 8`, the pen 100 points to the right:
```text
d = 100  (past PIE_RING, and there is an outer ring of 12)
heading = π/2 (a quarter turn clockwise from up)
wedge(π/2, 12) = floor((π/2 + π/12) / (π/6)) % 12 = 3
result = 8 + 3 = Some(11)
```

**Used by:** [app/button_drag.rs](../app/button_drag.md) (`button_drag_to`, to set the
hovered swatch while the pie is open).

## Tests

Run with `cargo test input::button_drag`.

| Test | What it checks |
|---|---|
| [`the_hole_cancels_and_an_empty_pie_picks_nothing`](../../../src/input/button_drag.rs#L164) | The centre and a point inside `PIE_HOLE` give `None`, and so does any point when there are no swatches. |
| [`inner_wedges_run_clockwise_from_the_top`](../../../src/input/button_drag.rs#L171) | With 8 swatches: up = 0, right = 2, down = 4, left = 6; a little left of up is still 0; with 3 swatches, right is wedge 1. |
| [`the_outer_ring_holds_the_rest_and_a_flick_past_it_still_counts`](../../../src/input/button_drag.rs#L187) | 20 swatches split 8 + 12; outer-ring indices start at 8; a point far past the outer edge still picks by direction; with no outer ring, far out still means the inner ring. |
| [`a_smaller_cap_moves_the_rest_out`](../../../src/input/button_drag.rs#L198) | A `cap` of 2 (a pinned wheel's pair) puts 2 swatches inside and 3 on the outer ring, and indexes them accordingly. |
| [`drag_steps_wait_out_the_dead_zone`](../../../src/input/button_drag.rs#L206) | `drag_steps` is 0 up to 19.9 points, then ±1 at ±20 and 2 at 40, 0 for `NaN`; `past_dead_zone(-20)` is `-12` and `past_dead_zone(5)` is `0`. |

### `mod tests`

Run with `cargo test input::button_drag::tests` · 5 tests

| Test | What it checks |
|---|---|
| [`the_hole_cancels_and_an_empty_pie_picks_nothing`](../../../src/input/button_drag.rs#L164) | The hole cancels and an empty pie picks nothing. |
| [`inner_wedges_run_clockwise_from_the_top`](../../../src/input/button_drag.rs#L171) | Inner wedges run clockwise from the top. |
| [`the_outer_ring_holds_the_rest_and_a_flick_past_it_still_counts`](../../../src/input/button_drag.rs#L187) | The outer ring holds the rest and a flick past it still counts. |
| [`a_smaller_cap_moves_the_rest_out`](../../../src/input/button_drag.rs#L198) | A smaller cap moves the rest out. |
| [`drag_steps_wait_out_the_dead_zone`](../../../src/input/button_drag.rs#L206) | Drag steps wait out the dead zone. |
