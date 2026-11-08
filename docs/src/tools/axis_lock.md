# `src/tools/axis_lock.rs` — Shift-lock geometry for drags

> **Source:** [src/tools/axis_lock.rs](../../../src/tools/axis_lock.rs) · **Module:** `crate::tools::axis_lock` · **Size:** 121 lines · Line links as of commit `36405ad`

## What this file is for

Small geometry helpers for what holding **Shift** (pressed partway through a drag) does: a stroke or move is
held to the horizontal or vertical through where it started, a shape's line snaps to 15° steps, and a box
becomes a square. With the centre key (Alt) a shape grows from its middle (`mirror`). Each function works in
whatever coordinate space it is given; `basis` is the angle of "horizontal" in that space (non-zero when
the canvas view is rotated and locking follows the screen axes).

## How it connects

- **Used by:** [app.rs](../app.md) (stroke axis lock, shape and selection drags, camera drags),
  [app/lasso_fill.rs](../app/lasso_fill.md), [app/select.rs](../app/select.md).

## Rust concepts in this file

**A type alias for points.** `type P = (f32, f32);` keeps signatures short.

**`sin_cos()`** returns both at once: `let (s, c) = a.sin_cos();`.

## Types

### `type P`
[src/tools/axis_lock.rs:12](../../../src/tools/axis_lock.rs#L12)

```rust
type P = (f32, f32);
```

A 2-D point `(x, y)`.

### `const LINE_STEP`
[src/tools/axis_lock.rs:15](../../../src/tools/axis_lock.rs#L15)

```rust
pub const LINE_STEP: f32 = PI / 12.0;
```

The step a Shift line turns in.

## Free functions

### `snap_dir`
[src/tools/axis_lock.rs:20](../../../src/tools/axis_lock.rs#L20)

```rust
pub fn snap_dir(from: P, to: P, basis: f32, step: f32) -> P
```

Holds `to` to the nearest allowed direction from `from` (whole multiples of `step` away from `basis`). The
end is *projected* onto that line, not swung onto it, so it slides along the line with the pointer.

**Example.** `from = (0,0)`, `to = (10, 1)`, `basis = 0`, `step = 90°`: nearest direction is 0°, so the
result is `(10, 0)`.

### `lock_hv`
[src/tools/axis_lock.rs:33](../../../src/tools/axis_lock.rs#L33)

```rust
pub fn lock_hv(from: P, to: P, basis: f32) -> P
```

`to` held to the horizontal or vertical through `from`, whichever the drag is nearer (`snap_dir` with 90°
steps).

### `hv_angle`
[src/tools/axis_lock.rs:39](../../../src/tools/axis_lock.rs#L39)

```rust
pub fn hv_angle(from: P, to: P, basis: f32) -> f32
```

The angle of the horizontal or vertical through `from` that `to` is nearer — a stroke picks one and keeps
it.

### `onto`
[src/tools/axis_lock.rs:45](../../../src/tools/axis_lock.rs#L45)

```rust
pub fn onto(from: P, a: f32, p: P) -> P
```

`p` projected onto the line through `from` at angle `a` (dot product with the line's direction).

### `mirror`
[src/tools/axis_lock.rs:53](../../../src/tools/axis_lock.rs#L53)

```rust
pub fn mirror(centre: P, p: P) -> P
```

The far corner of a drag grown from `centre`: `p` reflected through it (`2·centre − p`).

### `square`
[src/tools/axis_lock.rs:59](../../../src/tools/axis_lock.rs#L59)

```rust
pub fn square(from: P, to: P) -> P
```

Moves `to` so the box from `from` is square, keeping the longer side and each side's drag direction.

## Tests

### `mod tests`

Run with `cargo test tools::axis_lock::tests` · 6 tests

| Test | What it checks |
|---|---|
| [`a_drag_keeps_its_longer_axis`](../../../src/tools/axis_lock.rs#L75) | A drag keeps its longer axis. |
| [`a_turned_basis_locks_along_its_own_axes`](../../../src/tools/axis_lock.rs#L81) | A turned basis locks along its own axes. |
| [`a_line_turns_in_fifteen_degree_steps`](../../../src/tools/axis_lock.rs#L91) | A line turns in fifteen degree steps. |
| [`a_box_squares_up_the_way_it_was_dragged`](../../../src/tools/axis_lock.rs#L102) | A box squares up the way it was dragged. |
| [`a_centred_box_reaches_as_far_the_other_way`](../../../src/tools/axis_lock.rs#L108) | A centred box reaches as far the other way. |
| [`a_stroke_picks_the_axis_it_heads_along`](../../../src/tools/axis_lock.rs#L116) | A stroke picks the axis it heads along. |

**Test helpers** (used only by the tests above):

- [`near`](../../../src/tools/axis_lock.rs#L70) — Near.
