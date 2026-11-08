# `src/tools/dab.rs` — brush dabs (one stamp of a brush)

> **Source:** [src/tools/dab.rs](../../../src/tools/dab.rs) · **Module:** `crate::tools::dab` · **Size:** 171 lines · Line links as of commit `1fa5845`

## What this file is for

A **dab** is one stamp of the brush tip. Dab-mode brushes (pencils, airbrush) draw a stroke by stamping
dabs every short distance along it; overlapping dabs build up, which is what makes graphite look like
graphite. This file decides the shape of one dab from a pen sample and the brush settings — position,
radius, ellipse ratio and angle (from tilt), and how much alpha it lays down. Actually writing pixels is
[ribbon.rs](ribbon.md)'s job (it owns the coverage buffer the dabs and ribbons share).

```text
stroke path  ●───●───●───●───●     dabs every `spacing × diameter`
             ○ ○ ○ ○ ○ ○ ○ ○ ○     overlapping stamps → texture and build-up
```

Tilt matters: leaning the pen **widens** the dab and **flattens** it across the direction of lean, like a
pencil laid on its side.

## How it connects

- **Used by:** [tools/stroke.rs](stroke.md), [tools/ribbon.rs](ribbon.md).
- **Uses:** [tools/mod.rs](mod.md) (`BrushSettings`, `Dyn`), the pointer sample type.

## Rust concepts in this file

**`hypot` and `atan2` on `f32`.** `s.tilt_x.hypot(s.tilt_y)` is the tilt magnitude, `s.tilt_y.atan2(s.tilt_x)`
its direction.

## Types

### `const FULL_TILT_DEG`
[src/tools/dab.rs:18](../../../src/tools/dab.rs#L18)

```rust
const FULL_TILT_DEG: f32 = 60.0;
```

Tilt in degrees at which the tilt-driven dynamics reach full strength. Past about here a pen is lying nearly flat on the tablet.

### `struct Dab`
[src/tools/dab.rs:22](../../../src/tools/dab.rs#L22)

One stamp.

| Field | Type | Meaning |
|---|---|---|
| `x` | `f32` | Centre x, in cell pixels. |
| `y` | `f32` | Centre y, in cell pixels. |
| `radius` | `f32` | Semi-major radius, along `angle`. |
| `ratio` | `f32` | Semi-minor over semi-major, 0..=1. 1.0 is a circle. |
| `angle` | `f32` | Orientation of the major axis, radians. |
| `flow` | `f32` | Alpha this single stamp lays down at its solid core, 0..=1. |

## `impl Dab`

### `Dab::from_sample`
[src/tools/dab.rs:42](../../../src/tools/dab.rs#L42)

```rust
pub fn from_sample(brush: &BrushSettings, s: &PointerSample) -> Self
```

Builds the dab for a pointer sample: pressure scales radius and flow through the brush's `size` and
`flow_dyn` curves; tilt (via `tilt_of`) widens it by `tilt_size` and flattens it by `tilt_elongation`, with
the long axis across the lean direction.

### `Dab::spacing`
[src/tools/dab.rs:63](../../../src/tools/dab.rs#L63)

```rust
pub fn spacing(&self, brush: &BrushSettings) -> f32
```

Distance between dabs along the stroke: `spacing` × the dab's *narrow* diameter (so flattened dabs still
overlap across their thin side), at least half a pixel.

## Free functions

### `tilt_of`
[src/tools/dab.rs:73](../../../src/tools/dab.rs#L73)

```rust
fn tilt_of(s: &PointerSample) -> (f32, f32)
```

The pen's tilt as a 0–1 strength (full at `FULL_TILT_DEG` = 60°) and the lean direction in radians; `(0, 0)`
for an upright pen or mouse.

## Tests

### `mod tests`

Run with `cargo test tools::dab::tests` · 5 tests

| Test | What it checks |
|---|---|
| [`an_upright_pen_makes_a_round_dab`](../../../src/tools/dab.rs#L102) | An upright pen makes a round dab. |
| [`leaning_the_pen_flattens_the_dab_across_the_lean`](../../../src/tools/dab.rs#L111) | The tilted pencil's whole reason for existing: leaning it must both broaden the dab and flatten it, across the lean rather than along it. |
| [`tilt_dynamics_saturate`](../../../src/tools/dab.rs#L136) | Tilt past the saturation point must not keep growing the dab, or a pen laid flat would paint an unbounded blob. |
| [`pressure_drives_size_and_flow_through_their_curves`](../../../src/tools/dab.rs#L145) | Pressure drives size and flow through their curves. |
| [`spacing_scales_with_the_dab`](../../../src/tools/dab.rs#L162) | Spacing is a fraction of the dab, so it must track the dab's size — otherwise a big brush stamps far more dabs than it needs and a small one leaves gaps. |

**Test helpers** (used only by the tests above):

- [`sample`](../../../src/tools/dab.rs#L89) — Sample.
