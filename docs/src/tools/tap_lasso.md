# `src/tools/tap_lasso.rs` — the lasso outline (drag or tap corners)

> **Source:** [src/tools/tap_lasso.rs](../../../src/tools/tap_lasso.rs) · **Module:** `crate::tools::tap_lasso` · **Size:** 183 lines · Line links as of commit `36405ad`

## What this file is for

The outline both lassos draw — the Lasso selection's and the Fill tool's lasso mode — as a small state
machine:

- **drag** → a freehand outline, closed when you let go;
- **tap** → starts a polygon, one corner per tap; a tap on the first corner, a double-click, or Enter closes
  it; while tapping corners, a press that drags adds a freehand stretch from the last corner.

```text
press ──(moved < tap distance)──► release = tap  ──► polygon: corner added
  └──(moved ≥ tap distance)──► drawing ──► release = freehand outline done
```

## How it connects

- **Used by:** [app/select.rs](../app/select.md) (Lasso selection), [app/lasso_fill.rs](../app/lasso_fill.md)
  (Fill lasso), [ui/shell.rs](../ui/shell.md) (drawing the outline).
- Closed outlines become masks in [tools/lasso.rs](lasso.md).

## Rust concepts in this file

**Methods that consume `self`.** `finish(mut self)` takes the lasso by value: after finishing, the outline is
gone (the compiler forbids using it again) and its points are returned. More:
[Impl blocks and methods](../../rust-primer.md#impl-blocks-and-methods)

## Types

### `struct TapLasso`
[src/tools/tap_lasso.rs:18](../../../src/tools/tap_lasso.rs#L18)

An outline being drawn.

| Field | Type | Meaning |
|---|---|---|
| `pts` | `Vec<(f32, f32)>` | Every point so far: corners, and freehand samples after them. |
| `corners` | `Vec<usize>` | Which of `pts` are tapped corners. |
| `polygon` | `bool` | Tapped out corner by corner, not drawn in one drag. |
| `pressed` | `bool` | The pen is down. |
| `press` | `(f32, f32)` | Where the pen went down. |
| `drawing` | `bool` | The pen has gone far enough from `press` to be drawing. |

## Free functions

### `dist`
[src/tools/tap_lasso.rs:12](../../../src/tools/tap_lasso.rs#L12)

```rust
fn dist(a: (f32, f32), b: (f32, f32)) -> f32
```

Distance between two points.

## `impl TapLasso`

### `TapLasso::start`
[src/tools/tap_lasso.rs:35](../../../src/tools/tap_lasso.rs#L35)

```rust
pub fn start(at: (f32, f32)) -> Self
```

Starts an outline with the pen down at `at`.

### `TapLasso::closable`
[src/tools/tap_lasso.rs:47](../../../src/tools/tap_lasso.rs#L47)

```rust
pub fn closable(&self) -> bool
```

Whether the polygon has enough corners that a press near the first one would close it.

### `TapLasso::press`
[src/tools/tap_lasso.rs:54](../../../src/tools/tap_lasso.rs#L54)

```rust
pub fn press(&mut self, at: (f32, f32), corner: (f32, f32), close: f32) -> bool
```

A press while tapping a polygon: within `close` of the first corner it closes the polygon (`true`; the
caller then calls `finish`); otherwise it adds a corner at `corner` (the press point, possibly snapped).

### `TapLasso::drag`
[src/tools/tap_lasso.rs:68](../../../src/tools/tap_lasso.rs#L68)

```rust
pub fn drag(&mut self, at: (f32, f32), tap: f32, step: f32)
```

The pen moved to `at` while down: still a tap until it has moved `tap` from the press; after that it draws,
adding a point every `step`.

### `TapLasso::release`
[src/tools/tap_lasso.rs:81](../../../src/tools/tap_lasso.rs#L81)

```rust
pub fn release(&mut self) -> bool
```

The pen came up: returns `true` if that completed a freehand outline (the caller calls `finish`); a tap that
started the outline turns it into a polygon with its first corner there.

### `TapLasso::finish`
[src/tools/tap_lasso.rs:97](../../../src/tools/tap_lasso.rs#L97)

```rust
pub fn finish(mut self) -> Option<Vec<(f32, f32)>>
```

The closed outline's points, if they make a shape (at least three distinct points). Consumes the lasso.

### `TapLasso::back`
[src/tools/tap_lasso.rs:104](../../../src/tools/tap_lasso.rs#L104)

```rust
pub fn back(&mut self) -> bool
```

Removes the newest corner and any freehand stretch drawn from it; `false` when nothing is left.

### `TapLasso::doubled`
[src/tools/tap_lasso.rs:115](../../../src/tools/tap_lasso.rs#L115)

```rust
pub fn doubled(&self, near: f32) -> bool
```

Whether the two newest corners are within `near` of each other — i.e. a double-click (egui counts any two
quick clicks as a double-click, so fast taps must be told apart by distance).

## Tests

### `mod tests`

Run with `cargo test tools::tap_lasso::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`a_drag_closes_on_letting_go`](../../../src/tools/tap_lasso.rs#L144) | A drag closes on letting go. |
| [`taps_make_a_polygon_closed_on_its_first_corner`](../../../src/tools/tap_lasso.rs#L154) | Taps make a polygon closed on its first corner. |
| [`a_drag_between_corners_adds_a_freehand_stretch`](../../../src/tools/tap_lasso.rs#L165) | A drag between corners adds a freehand stretch. |

**Test helpers** (used only by the tests above):

- [`tap`](../../../src/tools/tap_lasso.rs#L128) — A press and a release where it went down.
