# `src/tools/krita/spacing.rs` — dab spacing and the path walker

> **Source:** [src/tools/krita/spacing.rs](../../../../src/tools/krita/spacing.rs) · **Module:** `crate::tools::krita::spacing` · **Size:** 228 lines · Line links as of commit `36405ad`

## What this file is for

Decides **how far apart dabs sit** and **where along the stroke the next one falls** — Krita's
`effectiveSpacing`, `calcAutoSpacing` and `KisDistanceInformation::getNextPointPosition`.

Spacing is a fraction of the dab's size (e.g. 0.1 = every tenth of a diameter). With **auto spacing** it grows
sub-linearly with size, so big brushes don't need thousands of dabs. Spacing can be **anisotropic** (different
along x and y of a non-round dab, rotated with it).

`Distance` is the path walker: as each stroke segment `start → end` arrives, `next_point` returns the fraction
along it where the next dab goes (or a negative number if the segment ends first), carrying the leftover
distance into the next segment so spacing stays even across segments.

**Example.** Spacing 5 px, a 12 px segment, 2 px left over from before: dabs at 3 px and 8 px along (fractions
0.25 and 0.667), and 4 px carried into the next segment.

## How it connects

- **Used by:** [tools/krita/mod.rs](mod.md) (`KritaStroke::next_point`, spacing after each dab).

## Rust concepts in this file

**Plain data structs with methods.** `Spacing::isotropic(s)` is a named constructor (Rust has no overloading,
so alternative constructors get descriptive names).

## Types

### `const MIN_SPACING`
[src/tools/krita/spacing.rs:13](../../../../src/tools/krita/spacing.rs#L13)

```rust
const MIN_SPACING: f64 = 0.5;
```

The smallest spacing allowed (Krita's `MIN_DISTANCE_SPACING`).

### `struct Spacing`
[src/tools/krita/spacing.rs:17](../../../../src/tools/krita/spacing.rs#L17)

Spacing between dabs, possibly different along the dab's two axes (Krita's `KisSpacingInformation`).

| Field | Type | Meaning |
|---|---|---|
| `x` | `f64` | Spacing along the dab's x axis, in pixels. |
| `y` | `f64` | Spacing along its y axis. |
| `rotation` | `f64` | The dab's rotation (for anisotropic spacing). |
| `flipped` | `bool` | Whether the dab is mirrored. |

### `struct Distance`
[src/tools/krita/spacing.rs:82](../../../../src/tools/krita/spacing.rs#L82)

The part of `KisDistanceInformation` that decides where along a segment the next dab falls.

| Field | Type | Meaning |
|---|---|---|
| `spacing` | `Spacing` | The current spacing. |
| `accum` | `(f64, f64)` | Distance walked since the last dab (x, y). |

## `impl Spacing`

### `Spacing::isotropic`
[src/tools/krita/spacing.rs:25](../../../../src/tools/krita/spacing.rs#L25)

```rust
pub fn isotropic(s: f64) -> Self
```

The same spacing `s` in every direction.

### `Spacing::is_isotropic`
[src/tools/krita/spacing.rs:34](../../../../src/tools/krita/spacing.rs#L34)

```rust
pub fn is_isotropic(&self) -> bool
```

Whether x and y spacing are equal.

## Free functions

### `calc_auto_spacing`
[src/tools/krita/spacing.rs:40](../../../../src/tools/krita/spacing.rs#L40)

```rust
pub fn calc_auto_spacing(value: f64, coeff: f64) -> f64
```

`calcAutoSpacing(qreal value, qreal coeff)`.

### `effective_spacing`
[src/tools/krita/spacing.rs:47](../../../../src/tools/krita/spacing.rs#L47)

```rust
pub fn effective_spacing(
    dab_w: f64,
    dab_h: f64,
    isotropic: bool,
    rotation: f64,
    flipped: bool,
    spacing_val: f64,
    auto_spacing: bool,
    auto_coeff: f64,
) -> Spacing
```

`KisPaintOpUtils::effectiveSpacing` with distance spacing on, no extra scale and no level of detail.

## `impl Distance`

### `Distance::new`
[src/tools/krita/spacing.rs:88](../../../../src/tools/krita/spacing.rs#L88)

```rust
pub fn new(spacing: Spacing) -> Self
```

A walker starting with `spacing` and nothing accumulated.

### `Distance::set_spacing`
[src/tools/krita/spacing.rs:97](../../../../src/tools/krita/spacing.rs#L97)

```rust
pub fn set_spacing(&mut self, spacing: Spacing)
```

`registerPaintedDab`: the dab just painted sets the spacing to the next.

### `Distance::reset`
[src/tools/krita/spacing.rs:101](../../../../src/tools/krita/spacing.rs#L101)

```rust
fn reset(&mut self)
```

Clears the accumulated distance.

### `Distance::next_point`
[src/tools/krita/spacing.rs:107](../../../../src/tools/krita/spacing.rs#L107)

```rust
pub fn next_point(&mut self, start: (f64, f64), end: (f64, f64)) -> f64
```

`getNextPointPosition`: how far along `start → end` (0..=1) the next dab falls, or a negative number if the segment ends first.

### `Distance::next_isotropic`
[src/tools/krita/spacing.rs:115](../../../../src/tools/krita/spacing.rs#L115)

```rust
fn next_isotropic(&mut self, start: (f64, f64), end: (f64, f64)) -> f64
```

`next_point` for equal spacing: walks the segment's length.

### `Distance::next_anisotropic`
[src/tools/krita/spacing.rs:137](../../../../src/tools/krita/spacing.rs#L137)

```rust
fn next_anisotropic(&mut self, start: (f64, f64), end: (f64, f64)) -> f64
```

`next_point` for different x/y spacing: measures progress in the dab's rotated frame, as Krita does.

## Tests

### `mod tests`

Run with `cargo test tools::krita::spacing::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`auto_spacing_grows_with_the_root_of_the_size`](../../../../src/tools/krita/spacing.rs#L184) | Auto spacing grows with the root of the size. |
| [`isotropic_walk_steps_evenly`](../../../../src/tools/krita/spacing.rs#L193) | Isotropic walk steps evenly. |
| [`anisotropic_walk_uses_the_axis_it_travels_along`](../../../../src/tools/krita/spacing.rs#L213) | Anisotropic walk uses the axis it travels along. |
