# `src/tools/krita/distance.rs` — what a Krita stroke remembers between dabs

> **Source:** [src/tools/krita/distance.rs](../../../../src/tools/krita/distance.rs) · **Module:** `crate::tools::krita::distance` · **Size:** 112 lines · Line links as of commit `67a7cb9`

## What this file is for

Some Krita sensors need the stroke's history: the **drawing angle** (direction from the last dab), how far the
stroke has run (fade, distance), how many dabs have been painted, the hardest pressure so far. `DabHistory`
keeps those, updated as each dab is painted, plus a few angle helpers that reproduce Krita's exact
arithmetic (including its single-precision `atan2f`).

## How it connects

- **Used by:** [tools/krita/mod.rs](mod.md) (`KritaStroke::history`), [tools/krita/sensors.rs](sensors.md)
  (`PaintInfo::drawing_angle`, `dab_seq_no`).

## Rust concepts in this file

**Matching float precision on purpose.** `(dy as f32).atan2(dx as f32) as f64` computes in `f32` because Krita's
`QVector2D` does — a deliberate precision loss to match results.

## Types

### `struct DabHistory`
[src/tools/krita/distance.rs:50](../../../../src/tools/krita/distance.rs#L50)

`KisDistanceInformation`'s dab history.

| Field | Type | Meaning |
|---|---|---|
| `last_position` | `(f64, f64)` | Where the last dab was painted. |
| `last_angle` | `f64` | The drawing angle at the last dab. |
| `total_distance` | `f64` | Distance travelled so far. |
| `dab_seq_no` | `i32` | Dabs painted so far. |
| `last_max_pressure` | `f64` | The highest pressure so far in the stroke. |

## Free functions

### `direction_between_points`
[src/tools/krita/distance.rs:19](../../../../src/tools/krita/distance.rs#L19)

```rust
pub fn direction_between_points(p1: (f64, f64), p2: (f64, f64), default_angle: f64) -> f64
```

`KisAlgebra2D::directionBetweenPoints`. Krita takes the difference as a `QVector2D`, whose components are single precision, so the angle is `atan2f` of floats.

### `shortest_angular_distance`
[src/tools/krita/distance.rs:29](../../../../src/tools/krita/distance.rs#L29)

```rust
pub fn shortest_angular_distance(a: f64, b: f64) -> f64
```

`shortestAngularDistance`.

### `increment_in_direction`
[src/tools/krita/distance.rs:38](../../../../src/tools/krita/distance.rs#L38)

```rust
pub fn increment_in_direction(a: f64, inc: f64, direction: f64) -> f64
```

`incrementInDirection`.

## `impl DabHistory`

### `DabHistory::start`
[src/tools/krita/distance.rs:61](../../../../src/tools/krita/distance.rs#L61)

```rust
pub fn start(pos: (f64, f64), start_angle: f64) -> Self
```

`KisToolFreehandHelper::initPaintImpl`: the stroke starts at the pen-down point, facing `start_angle`, with no dabs painted.

### `DabHistory::drawing_angle`
[src/tools/krita/distance.rs:73](../../../../src/tools/krita/distance.rs#L73)

```rust
pub fn drawing_angle(&self, pos: (f64, f64)) -> f64
```

`KisPaintInformation::drawingAngle`: the direction from the last dab to `pos`, or the last dab's own when they coincide.

### `DabHistory::register_painted_dab`
[src/tools/krita/distance.rs:78](../../../../src/tools/krita/distance.rs#L78)

```rust
pub fn register_painted_dab(&mut self, pos: (f64, f64), pressure: f64)
```

`registerPaintedDab`.

## Tests

### `mod tests`

Run with `cargo test tools::krita::distance::tests` · 2 tests

| Test | What it checks |
|---|---|
| [`the_angle_follows_the_dabs_and_holds_when_they_coincide`](../../../../src/tools/krita/distance.rs#L93) | The angle follows the dabs and holds when they coincide. |
| [`increments_go_the_short_way_round`](../../../../src/tools/krita/distance.rs#L106) | Increments go the short way round. |
