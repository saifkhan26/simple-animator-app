# `src/app/brush_sizes.rs` — remembered brush sizes (recent, pinned, swap, detents)

> **Source:** [src/app/brush_sizes.rs](../../../src/app/brush_sizes.rs) · **Module:** `crate::app::brush_sizes` · **Size:** 266 lines · Line links as of commit `1fa5845`

## What this file is for

Every brush tool (Pencil, Ink, Eraser, …) remembers the sizes it was used at. This file
keeps that memory and the small features built on it:

- **Recent sizes** — the last 5 sizes a stroke was actually drawn at, newest first.
- **Pinned sizes** — up to 5 sizes the artist chose to keep, smallest first. Together with
  the recent ones they are shown as a clickable strip under the Size slider (drawn by
  [ui/shell.rs](../ui/shell.md)).
- **Size swap** — one key (`SizeSwap`) jumps to the size you drew at last, and pressing it
  again jumps back. Think of it like the "previous channel" button on a TV remote.
- **Detents** — when you change the brush size by dragging with a pen button ("tool
  adjust", see [app/button_drag.rs](button_drag.md)), the size briefly *sticks* on each
  remembered size before carrying on. A *detent* is that little "click" a physical knob
  makes at certain positions: it holds you for a moment but does not stop you going
  past it.

The memory is a workspace preference (saved with the app's preferences, not inside the
`.anim` project), like the colour palette.

The file is a child module of [src/app.rs](../app.md). It only exists to keep `app.rs`
shorter: most of its functions are extra methods on the app's big state struct,
`AppState`.

## How it connects

- **Used by:**
  - [src/app.rs](../app.md) — declares this module (`pub mod brush_sizes;`), stores one
    `SizeMemory` per tool in `AppState::brush_sizes` and in the saved preferences,
    calls `remember_size` at the end of a stroke in `pointer_up`, and calls `swap_size`
    when the `SizeSwap` shortcut fires in `dispatch`.
  - [src/app/button_drag.rs](button_drag.md) — tool adjust's size drag uses
    `size_with_detents`, `SIZE_DETENT_PX`, `size_detents` and `size_note`.
  - [src/ui/shell.rs](../ui/shell.md) — the Size slider and the size strip under it use
    `size_memory`, `same_size`, `size_label` and `toggle_pinned_size`.
- **Uses:** [src/app.rs](../app.md) (`AppState`, `ActiveTool`). Nothing else.

```text
 stroke ends (app.rs pointer_up) ──► remember_size ──┐
 right-click a size in the strip ──► toggle_pinned_size ──► SizeMemory { recent, pinned }
 SizeSwap key (app.rs dispatch) ──► swap_size ─────────┘          │
                                                                  ▼
 tool adjust drag (button_drag.rs) ◄── size_detents + size_with_detents + size_note
 size strip / slider marks (ui/shell.rs) ◄── size_memory + same_size + size_label
```

## Rust concepts in this file

**Adding methods to a type from another file.** `AppState` is *defined* in `src/app.rs`,
but this file adds more methods to it with its own `impl AppState { … }` block. Rust lets
a type's methods be spread over several `impl` blocks, even in different files of the same
crate. A JS class body has to be in one place; the closest JS idea is attaching methods to
the prototype from another module.
```rust
use super::{ActiveTool, AppState}; // `super` = the parent module, crate::app
impl AppState {
    pub fn size_slot(&self) -> usize { /* … */ }
}
```
```js
import { AppState } from "../app.js";
AppState.prototype.sizeSlot = function () { /* … */ };
```
Because this file is a *child* module of `app`, it may read and write `AppState`'s private
fields (`self.brush_sizes`, `self.size_swap`) — in Rust, privacy is per module, and child
modules can see their parent's private items. More:
[impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods),
[modules and visibility](../../rust-primer.md#modules-and-visibility)

**`pub(super)` — visible to the parent only.** `remember_size`, `size_detents` and
`size_note` are marked `pub(super)`: code in `crate::app` (and its other children such as
`button_drag`) can call them, but the UI code in `crate::ui` cannot. Plain `pub` methods
(`size_memory`, `toggle_pinned_size`, `swap_size`) can be called from anywhere.
```rust
pub(super) fn remember_size(&mut self) { /* … */ }
```
More: [modules and visibility](../../rust-primer.md#modules-and-visibility)

**Saved with serde.** `SizeMemory` derives `Serialize`/`Deserialize`, so it can be written
into the preferences file automatically. `#[serde(default)]` means "if a field is missing
when loading, use its default value" (an empty list), so older preference files still load.
```rust
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SizeMemory { pub recent: Vec<f32>, pub pinned: Vec<f32> }
```
Roughly `JSON.stringify` / `JSON.parse` with `{ recent: [], pinned: [], ...saved }`.
More: [serde and file formats](../../rust-primer.md#serde-and-file-formats),
[traits and derive](../../rust-primer.md#traits-and-derive)

**Iterator chains with patterns in closures.** The code filters and maps lists much like
JS array methods. The `|&s|` / `|&&s|` in a closure parameter *unpacks a reference*:
`iter()` hands out references (`&f32`) to the numbers, and `filter` adds one more layer
(`&&f32`), so the `&` patterns strip them to get a plain `f32`.
```rust
m.recent.retain(|&s| !same_size(s, r));          // keep only sizes different from r
let i = m.pinned.iter().position(|&s| same_size(s, size)); // Option<usize>
```
```js
m.recent = m.recent.filter(s => !sameSize(s, r));
const i = m.pinned.findIndex(s => sameSize(s, size)); // -1 instead of None
```
More: [closures](../../rust-primer.md#closures), [iterators](../../rust-primer.md#iterators)

**`match` with a guard.** `swap_size` matches on an `Option` holding a tuple, and adds an
`if` condition (a *guard*) to the arm. The arm only matches when the value has that shape
*and* the condition is true; otherwise the catch-all `_` arm runs.
```rust
let to = match self.size_swap {
    Some((s, to, from)) if s == slot && same_size(to, r) => Some(from),
    _ => /* first recent size that differs from r */,
};
```
More: [enums and match](../../rust-primer.md#enums-and-match)

**Returning a `&mut` into the struct.** `size_memory_mut` returns `&mut SizeMemory` — a
mutable reference straight into the `Vec` inside `AppState`, so the caller can edit that
entry in place. JS objects are always shared by reference, so this happens implicitly
there; in Rust it must be spelled out, and while that reference is alive nothing else may
touch `self`. More: [ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)

## impl AppState

These methods are added to `AppState` (defined in [src/app.rs](../app.md)).

## Types

### `const RECENT_SIZES`
[src/app/brush_sizes.rs:11](../../../src/app/brush_sizes.rs#L11)

```rust
pub const RECENT_SIZES: usize = 5;
```

`usize = 5`. How many recent sizes each brush keeps.

### `const MAX_PINNED_SIZES`
[src/app/brush_sizes.rs:14](../../../src/app/brush_sizes.rs#L14)

```rust
pub const MAX_PINNED_SIZES: usize = 5;
```

`usize = 5`. How many sizes each brush can pin — as many as the strip under the Size
slider always has room for.

### `const SIZE_DETENT_PX`
[src/app/brush_sizes.rs:18](../../../src/app/brush_sizes.rs#L18)

```rust
pub const SIZE_DETENT_PX: f32 = 10.0;
```

`f32 = 10.0`. How far, in screen points, tool adjust's size drag stays held on a
remembered size before moving on past it. It is a detent, not a snap: sizes on both sides
of it can still be reached.

### `struct SizeMemory`
[src/app/brush_sizes.rs:24](../../../src/app/brush_sizes.rs#L24)

One brush's remembered sizes. `AppState::brush_sizes` is a `Vec<SizeMemory>` with one entry
per tool, indexed by `ActiveTool::idx()` (a number for each tool). Saved in the
preferences.

Sizes are brush radii in canvas pixels (`brush.radius`), as `f32` (a 32-bit float, like a
JS `number` with less precision).

| Field | Type | Meaning |
|---|---|---|
| `recent` | `Vec<f32>` | Sizes strokes were drawn at, newest first, at most `RECENT_SIZES`, no size twice. |
| `pinned` | `Vec<f32>` | Sizes the artist pinned to keep, smallest first, at most `MAX_PINNED_SIZES`. |

## Free functions

### `same_size`
[src/app/brush_sizes.rs:30](../../../src/app/brush_sizes.rs#L30)

```rust
pub fn same_size(a: f32, b: f32) -> bool
```

**What it does.** Says whether two sizes are equal *to one decimal place* — the precision
the size strip shows. It multiplies both by 10 and compares the rounded results, so `8.0`
and `8.04` count as the same size. Used everywhere sizes are compared, so the strip never
shows "8" twice.

**Used by:** this file (almost every method), [ui/shell.rs](../ui/shell.md) (`size_marks`,
`size_strip`).

### `size_label`
[src/app/brush_sizes.rs:35](../../../src/app/brush_sizes.rs#L35)

```rust
pub fn size_label(s: f32) -> String
```

**What it does.** Formats a size for display: one decimal at most, and no trailing `.0`.
`12.0` → `"12"`, `3.45` → `"3.5"`, `0.5` → `"0.5"`. It formats with `{:.1}` (like JS
`toFixed(1)`) and then strips a `.0` suffix if there is one.

**Used by:** `AppState::size_note` (this file), [ui/shell.rs](../ui/shell.md) (`size_strip`).

### `size_with_detents`
[src/app/brush_sizes.rs:44](../../../src/app/brush_sizes.rs#L44)

```rust
pub fn size_with_detents(from: f32, d: f32, per_doubling: f32, stops: &[f32], detent: f32) -> (f32, Option<f32>)
```

**What it does.** Works out the brush size during tool adjust's size drag. Dragging
sideways scales the size *exponentially*: every `per_doubling` points doubles it (or halves
it when dragging the other way). On the way, each remembered size in `stops` *holds* the
drag for `detent` points. Past a stop the size carries on smoothly from that stop, so a
size just past a stop is exactly as easy to reach as one just short of it.

**Parameters.**
- `from` — the size when the drag started.
- `d` — how far the pen has moved sideways, in screen points, with the dead zone (a small
  area around the press where nothing happens) already removed. Positive = bigger.
- `per_doubling` — screen points of drag that double the size (150 in practice).
- `stops` — remembered sizes to stick on (see `AppState::size_detents`).
- `detent` — how many points each stop holds the drag for (`SIZE_DETENT_PX`).

**Returns.** A tuple `(size, held)`: the new size, and `Some(stop)` if the drag is
currently being held on a stop, or `None` otherwise.

**How it works.** Everything is measured in *doublings* (powers of two), because that is
how the size scales.
1. Convert the detent width and the drag into doublings: `w = detent / per_doubling`,
   `u = d / per_doubling`.
2. For each positive stop, compute how many doublings away from `from` it is:
   `log2(stop / from)`. Keep only stops on the side the drag is going (bigger stops for a
   drag right, smaller for a drag left), and sort them nearest first.
3. Walk the stops in order. If the drag has not reached this stop yet, stop looking. If it
   is within `w` past the stop, return the stop itself — the drag is held there. Otherwise
   the drag has gone through the detent: subtract `w` from `u` (the time spent held does not
   count as movement) and look at the next stop.
4. With no stop holding it, the size is `from * 2^u`.

**Example.** The same idea in JavaScript:
```js
function sizeWithDetents(from, d, perDoubling, stops, detent) {
  const w = detent / perDoubling;
  let u = d / perDoubling;
  const sign = u < 0 ? -1 : 1;              // Rust's 0.0.signum() is +1 too
  const ahead = stops
    .filter(s => s > 0)
    .map(s => [Math.log2(s / from), s])
    .filter(([us]) => us !== 0 && Math.sign(us) === sign)
    .sort((a, b) => Math.abs(a[0]) - Math.abs(b[0]));
  for (const [us, s] of ahead) {
    if (Math.abs(u) <= Math.abs(us)) break;            // not there yet
    if (Math.abs(u) <= Math.abs(us) + w) return [s, s]; // held on the stop
    u -= w * sign;                                      // passed it: drop the hold
  }
  return [from * 2 ** u, null];
}
// from 10, stop 20, 150 pt per doubling, 10 pt detent:
// d = 151 → [20, 20] (held)   d = 161 → [~20.09, null] (just past)
```

**Used by:** `AppState::adjust_tool` in [app/button_drag.rs](button_drag.md).

## `impl AppState`

### `AppState::size_slot`
[src/app/brush_sizes.rs:71](../../../src/app/brush_sizes.rs#L71)

```rust
pub fn size_slot(&self) -> usize
```

**What it does.** Returns which tool's size memory the active tool uses — the index into
`brush_sizes`. Normally that is the active tool's own index (`self.tool.idx()`). The Shape
tool is the exception: it draws at the size of the pen it follows (`self.last_pen`, the
last pen tool used), so it shares that pen's sizes.

**Used by:** the other methods in this file; tests in [ui/shell.rs](../ui/shell.md).

### `AppState::size_memory`
[src/app/brush_sizes.rs:80](../../../src/app/brush_sizes.rs#L80)

```rust
pub fn size_memory(&self) -> SizeMemory
```

**What it does.** Returns a copy of the active brush's remembered sizes. If there is no
entry for that tool yet, it returns an empty `SizeMemory` (`unwrap_or_default`). Returning
a copy (`.cloned()`) means callers can read it freely while still changing `self`.

**Used by:** `size_detents`, `size_note`, `swap_size` (this file);
[ui/shell.rs](../ui/shell.md) (`size_slider`, `size_strip`).

### `AppState::size_memory_mut`
[src/app/brush_sizes.rs:84](../../../src/app/brush_sizes.rs#L84)

```rust
fn size_memory_mut(&mut self) -> &mut SizeMemory
```

**What it does.** Returns a mutable reference to the active brush's `SizeMemory` so it can
be edited in place. If the `brush_sizes` list is too short to have an entry for this tool,
it first grows the list with empty entries (`resize`). Private to this file.

**Used by:** `remember_size`, `toggle_pinned_size`.

### `AppState::remember_size`
[src/app/brush_sizes.rs:93](../../../src/app/brush_sizes.rs#L93)

```rust
pub(super) fn remember_size(&mut self)
```

**What it does.** Called when a stroke has landed at the brush's current size
(`self.brush.radius`): moves that size to the front of the recent list. It removes any
entry that is the same size (to a tenth), inserts the size at position 0, and cuts the list
down to `RECENT_SIZES`. Only drawing a stroke calls this — just moving the size slider does
not count.

**Used by:** `AppState::pointer_up` in [src/app.rs](../app.md), when the stroke drew
something.

### `AppState::toggle_pinned_size`
[src/app/brush_sizes.rs:103](../../../src/app/brush_sizes.rs#L103)

```rust
pub fn toggle_pinned_size(&mut self, size: f32)
```

**What it does.** Pins `size` if it is not pinned, or unpins it if it is (compared with
`same_size`). Pinning is refused once `MAX_PINNED_SIZES` sizes are pinned. After a pin the
list is re-sorted smallest first (`sort_by(f32::total_cmp)` — floats need an explicit
comparison function in Rust, like `arr.sort((a, b) => a - b)` in JS).

**Parameters.**
- `size` — the size to pin or unpin.

**Used by:** [ui/shell.rs](../ui/shell.md) (`size_strip`: right-click a size, or the pin
button).

### `AppState::swap_size`
[src/app/brush_sizes.rs:115](../../../src/app/brush_sizes.rs#L115)

```rust
pub fn swap_size(&mut self)
```

**What it does.** The `SizeSwap` shortcut. Switches the brush to the size drawn at last,
or — if the previous swap was just made in this tool and the brush is still at the size it
swapped to — back to the size it swapped from. If the brush is already at the size drawn
at last, it goes to the most recent size that is *different* from the current one.

**How it works.**
1. Read the current slot and radius.
2. `self.size_swap` stores the last swap as `(slot, to, from)`. If it is for this slot and
   the brush is still at `to`, the target is `from` (swap back).
3. Otherwise the target is the first entry in `recent` that differs from the current size.
4. If there is a target, record `(slot, target, current)` as the new swap and set the
   brush radius to the target. With no recent sizes, nothing happens.

**Used by:** `AppState::dispatch` in [src/app.rs](../app.md) (`Action::SizeSwap`).

### `AppState::size_detents`
[src/app/brush_sizes.rs:129](../../../src/app/brush_sizes.rs#L129)

```rust
pub(super) fn size_detents(&self, from: f32) -> Vec<f32>
```

**What it does.** The sizes tool adjust's size drag should stick on: all pinned and recent
sizes, except the starting size `from` itself, sorted smallest first with duplicates (to a
tenth) removed (`dedup_by`).

**Parameters.**
- `from` — the brush size when the drag started.

**Returns.** The list of stop sizes, ready to pass to `size_with_detents`.

**Used by:** `AppState::adjust_tool` in [app/button_drag.rs](button_drag.md).

### `AppState::size_note`
[src/app/brush_sizes.rs:139](../../../src/app/brush_sizes.rs#L139)

```rust
pub(super) fn size_note(&self) -> String
```

**What it does.** A short note shown after the size in tool adjust's on-screen readout,
saying how the current size relates to the remembered ones. Checked in this order:
`" (pinned)"`, `" (drawn last)"` (the newest recent size), `" (recent)"`, otherwise
`" (last 12)"` naming the size drawn at last, or an empty string if there are no recent
sizes at all.

**Used by:** `AppState::button_drag_readout` in [app/button_drag.rs](button_drag.md).

## Tests

Run with `cargo test app::brush_sizes`.

| Test | What it checks |
|---|---|
| [`a_stop_holds_the_drag_then_lets_it_go_on_from_there`](../../../src/app/brush_sizes.rs#L166) | A stop holds the size for the detent, a size just past the stop is reachable, and the size never goes backwards as the drag goes on. |
| [`stops_below_hold_a_drag_down`](../../../src/app/brush_sizes.rs#L187) | Smaller stops hold a drag to the left; stops on the other side are ignored; a second stop is reached one detent later than its size alone says. |
| [`each_brush_remembers_the_sizes_it_drew_at`](../../../src/app/brush_sizes.rs#L207) | Each tool keeps its own recent sizes, newest first, five at most, no repeats; only drawing remembers; Shape shares the pen's list. |
| [`swapping_goes_to_the_size_drawn_at_last_and_back`](../../../src/app/brush_sizes.rs#L227) | `SizeSwap` goes to the size drawn at last and back again, and at that size goes to the one before it. |
| [`pins_toggle_and_stay_sorted`](../../../src/app/brush_sizes.rs#L246) | Pins toggle on and off (same to a tenth counts), stay sorted, and stop at `MAX_PINNED_SIZES`. |
| [`labels_drop_a_trailing_zero`](../../../src/app/brush_sizes.rs#L260) | `size_label` shows `12`, `3.5` and `0.5`. |

**Test helpers:** `drawn` ([line 197](../../../src/app/brush_sizes.rs#L197)) — sets the brush
size and draws a short stroke with `pointer_down` / `pointer_move` / `pointer_up`, so the
size gets remembered. `PER` (line 160) — the 150-point-per-doubling value the detent tests
use.

### `mod tests`

Run with `cargo test app::brush_sizes::tests` · 6 tests

| Test | What it checks |
|---|---|
| [`a_stop_holds_the_drag_then_lets_it_go_on_from_there`](../../../src/app/brush_sizes.rs#L166) | A stop holds the drag for the detent, then the drag carries on from it: a size just past the stop is reachable, every size is, and the size never goes back as the drag goes on. |
| [`stops_below_hold_a_drag_down`](../../../src/app/brush_sizes.rs#L187) | Smaller stops hold a drag the other way, and stops behind the drag are passed by. |
| [`each_brush_remembers_the_sizes_it_drew_at`](../../../src/app/brush_sizes.rs#L207) | Each brush keeps the sizes it was drawn at, newest first, five of them, no size twice; Shape keeps the pen's. |
| [`swapping_goes_to_the_size_drawn_at_last_and_back`](../../../src/app/brush_sizes.rs#L227) | The swap goes to the size drawn at last, and back again. |
| [`pins_toggle_and_stay_sorted`](../../../src/app/brush_sizes.rs#L246) | Pins toggle and stay sorted. |
| [`labels_drop_a_trailing_zero`](../../../src/app/brush_sizes.rs#L260) | Labels drop a trailing zero. |

**Test helpers** (used only by the tests above):

- [`drawn`](../../../src/app/brush_sizes.rs#L197) — Drawn.

**Test-only types and constants:** [`PER`](../../../src/app/brush_sizes.rs#L160)
