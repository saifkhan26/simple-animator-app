# `src/tools/krita/curve.rs` — Krita's sensor curves (cubic splines)

> **Source:** [src/tools/krita/curve.rs](../../../../src/tools/krita/curve.rs) · **Module:** `crate::tools::krita::curve` · **Size:** 292 lines · Line links as of commit `36405ad`

## What this file is for

In Krita every sensor (pressure, tilt, speed…) goes through a **curve** the user draws: a few points from
(0,0) to (1,1), joined by a smooth **natural cubic spline**. The curve maps the raw sensor value to the
option's strength. Krita never evaluates the spline directly while painting: it precomputes a
**transfer table** (`floatTransfer`, 256 entries here) and linearly interpolates in it — this port does
exactly the same, because evaluating the spline directly would give slightly different values than Krita.

**Example.** Curve points `0,0; 0.5,0.2; 1,1` (a "hold back" curve): pressure 0.5 → table lookup → 0.2, so
at half pressure the brush is at 20% of its size; at full pressure 100%.

```text
out 1 |            ●
      |          ╱
  0.2 |   ____●╱          a curve that holds back until you press harder
      | ╱
    0 ●─────────────── in
      0     0.5      1
```

Curves are stored as strings like `"0,0;0.5,0.2;1,1;"`.

## How it connects

- **Used by:** [tools/krita/sensors.rs](sensors.md) (each sensor's curve), [tools/krita/kpp.rs](kpp.md),
  [ui/curve_editor.rs](../../ui/curve_editor.md) (editing curves).

## Rust concepts in this file

**Solving a tridiagonal system.** Fitting a cubic spline means solving a banded linear system — the classic
Thomas algorithm in `tridiagonal`. Plain `Vec<f64>` arithmetic.

**Formatting numbers like Qt.** `qnum` reproduces `QString::number` (`%g`, six significant digits) so written
curves are byte-identical to Krita's.

## Types

### `const TRANSFER_SIZE`
[src/tools/krita/curve.rs:11](../../../../src/tools/krita/curve.rs#L11)

```rust
pub const TRANSFER_SIZE: usize = 256;
```

The transfer table size Krita's sensors ask for (256).

### `struct CubicCurve`
[src/tools/krita/curve.rs:14](../../../../src/tools/krita/curve.rs#L14)

A sensor curve: its points, sorted by x.

| Field | Type | Meaning |
|---|---|---|
| `points` | `Vec<(f64, f64)>` | The control points, each `(x, y)` in 0–1. |

### `struct Spline`
[src/tools/krita/curve.rs:19](../../../../src/tools/krita/curve.rs#L19)

`KisCubicSpline<QPointF, qreal>`.

| Field | Type | Meaning |
|---|---|---|
| `a` | `Vec<f64>` | Spline coefficient per interval (value). |
| `b` | `Vec<f64>` | Spline coefficient per interval (slope). |
| `c` | `Vec<f64>` | Spline coefficient per interval (curvature). |
| `d` | `Vec<f64>` | Spline coefficient per interval (cubic term). |
| `h` | `Vec<f64>` | Interval widths. |
| `begin` | `f64` | First x. |
| `end` | `f64` | Last x. |
| `intervals` | `usize` | Number of intervals. |

## Free functions

### `tridiagonal`
[src/tools/krita/curve.rs:31](../../../../src/tools/krita/curve.rs#L31)

```rust
fn tridiagonal(a: &[f64], b: &[f64], c: &[f64], f: &[f64]) -> Vec<f64>
```

`KisTridiagonalSystem::calculate`, with `c == a` as the spline passes it.

## `impl Spline`

### `Spline::new`
[src/tools/krita/curve.rs:55](../../../../src/tools/krita/curve.rs#L55)

```rust
fn new(pts: &[(f64, f64)]) -> Self
```

Builds the natural cubic spline through the points (solving for the second derivatives with `tridiagonal`).

### `Spline::find_region`
[src/tools/krita/curve.rs:103](../../../../src/tools/krita/curve.rs#L103)

```rust
fn find_region(&self, x: f64) -> (usize, f64)
```

Which interval `x` falls in, and the offset within it.

### `Spline::value`
[src/tools/krita/curve.rs:115](../../../../src/tools/krita/curve.rs#L115)

```rust
fn value(&self, x: f64) -> f64
```

The spline's value at `x`.

## `impl CubicCurve`

### `CubicCurve::identity`
[src/tools/krita/curve.rs:125](../../../../src/tools/krita/curve.rs#L125)

```rust
pub fn identity() -> Self
```

The identity, Krita's `DEFAULT_CURVE_STRING`.

### `CubicCurve::parse`
[src/tools/krita/curve.rs:133](../../../../src/tools/krita/curve.rs#L133)

```rust
pub fn parse(s: &str) -> Option<Self>
```

Parse Krita's `"x,y;x,y;"` form. Pairs without a comma are skipped; the points are sorted by x, as `setPoints` does.

### `CubicCurve::from_points`
[src/tools/krita/curve.rs:152](../../../../src/tools/krita/curve.rs#L152)

```rust
pub fn from_points(mut points: Vec<(f64, f64)>) -> Self
```

`KisCubicCurve(QList<QPointF>)`: the points, sorted by x.

### `CubicCurve::points`
[src/tools/krita/curve.rs:158](../../../../src/tools/krita/curve.rs#L158)

```rust
pub fn points(&self) -> &[(f64, f64)]
```

The curve's points, sorted by x.

### `CubicCurve::to_krita_string`
[src/tools/krita/curve.rs:164](../../../../src/tools/krita/curve.rs#L164)

```rust
pub fn to_krita_string(&self) -> String
```

`KisCubicCurve::toString`: `"x,y;x,y;"`, each number as `QString::number` prints it (six significant digits).

### `CubicCurve::is_identity`
[src/tools/krita/curve.rs:170](../../../../src/tools/krita/curve.rs#L170)

```rust
pub fn is_identity(&self) -> bool
```

`KisCubicCurve::isIdentity`. A sensor with an identity curve skips the transfer table entirely and reports its raw value.

### `CubicCurve::float_transfer`
[src/tools/krita/curve.rs:181](../../../../src/tools/krita/curve.rs#L181)

```rust
pub fn float_transfer(&self, size: usize) -> Vec<f64>
```

`KisCubicCurve::floatTransfer(size)`.

## Free functions

### `qnum`
[src/tools/krita/curve.rs:195](../../../../src/tools/krita/curve.rs#L195)

```rust
fn qnum(v: f64) -> String
```

`QString::number(double)`: `%g` with six significant digits.

### `interpolate_linear`
[src/tools/krita/curve.rs:225](../../../../src/tools/krita/curve.rs#L225)

```rust
pub fn interpolate_linear(normalized: f64, transfer: &[f64]) -> f64
```

`KisCubicCurve::interpolateLinear`.

## Tests

### `mod tests`

Run with `cargo test tools::krita::curve::tests` · 4 tests

| Test | What it checks |
|---|---|
| [`curves_print_as_krita_prints_them`](../../../../src/tools/krita/curve.rs#L255) | Curves Krita wrote read back to the same text. |
| [`identity_is_recognised`](../../../../src/tools/krita/curve.rs#L265) | Identity is recognised. |
| [`two_point_curve_is_linear`](../../../../src/tools/krita/curve.rs#L272) | Two points make a straight line: the spline has no curvature to add. |
| [`spline_passes_through_its_knots`](../../../../src/tools/krita/curve.rs#L283) | Pencil-5's opacity curve passes through its middle knot. |
