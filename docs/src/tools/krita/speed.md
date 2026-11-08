# `src/tools/krita/speed.rs` — the speed sensor's input

> **Source:** [src/tools/krita/speed.rs](../../../../src/tools/krita/speed.rs) · **Module:** `crate::tools::krita::speed` · **Size:** 176 lines · Line links as of commit `36405ad`

## What this file is for

Krita's **Speed** sensor makes a brush respond to how fast you draw (e.g. thinner when fast). Raw speed from
a tablet is noisy, so Krita smooths it (`KisSpeedSmoother`): it keeps the last few distances travelled and
estimates the tablet's sample interval with a **trimmed rolling mean** (the average of recent intervals with
the most extreme values dropped). Speed = distance over time, in view pixels per millisecond, normalised to
0–1 against a maximum. This port reproduces that so speed-driven presets match Krita.

**Trimmed mean example.** Intervals `[4, 5, 5, 6, 40]` ms with the extremes trimmed → mean of `[5, 5, 6]` ≈ 5.3,
so one hiccup (40 ms) doesn't make the pen look slow.

## How it connects

- **Used by:** [tools/krita/mod.rs](mod.md) (`KritaStroke::note_speed` feeds every raw sample).

## Rust concepts in this file

**`VecDeque` rolling windows.** `push_back` new, `pop_front` oldest. More:
[Collections](../../../rust-primer.md#collections)

## Types

### `const MAX_SMOOTH_HISTORY`
[src/tools/krita/speed.rs:13](../../../../src/tools/krita/speed.rs#L13)

```rust
const MAX_SMOOTH_HISTORY: usize = 512;
```

How many recent distances are kept.

### `const NUM_SMOOTHING_SAMPLES`
[src/tools/krita/speed.rs:14](../../../../src/tools/krita/speed.rs#L14)

```rust
const NUM_SMOOTHING_SAMPLES: usize = 3;
```

How many samples the speed estimate averages over.

### `const MIN_TRACKING_DISTANCE`
[src/tools/krita/speed.rs:15](../../../../src/tools/krita/speed.rs#L15)

```rust
const MIN_TRACKING_DISTANCE: f64 = 5.0;
```

The minimum travelled distance before speed is measured.

### `const MAX_ALLOWED_SPEED`
[src/tools/krita/speed.rs:17](../../../../src/tools/krita/speed.rs#L17)

```rust
pub const MAX_ALLOWED_SPEED: f64 = 30.0;
```

`KisConfig::maxAllowedSpeedValue`'s default.

### `struct FilteredRollingMean`
[src/tools/krita/speed.rs:31](../../../../src/tools/krita/speed.rs#L31)

`KisFilteredRollingMean`: the mean of a rolling window with the most extreme values trimmed off both ends.

| Field | Type | Meaning |
|---|---|---|
| `values` | `VecDeque<f64>` | The window's values. |
| `capacity` | `usize` | Window size. |
| `rolling_sum` | `f64` | Running sum of the window. |
| `effective_portion` | `f64` | Fraction of values kept after trimming the extremes. |

### `struct SpeedSmoother`
[src/tools/krita/speed.rs:80](../../../../src/tools/krita/speed.rs#L80)

`KisSpeedSmoother`.

| Field | Type | Meaning |
|---|---|---|
| `distances` | `VecDeque<(f64, f64)>` | Recent (distance, time) steps. |
| `mean` | `FilteredRollingMean` | The rolling estimate of the sample interval. |
| `last_point` | `(f64, f64)` | Previous position. |
| `last_time` | `f64` | Previous time (ms). |
| `last_speed` | `f64` | Last computed speed. |

## Free functions

### `qround`
[src/tools/krita/speed.rs:20](../../../../src/tools/krita/speed.rs#L20)

```rust
fn qround(d: f64) -> i64
```

`qRound` for doubles (halves round up).

## `impl FilteredRollingMean`

### `FilteredRollingMean::new`
[src/tools/krita/speed.rs:39](../../../../src/tools/krita/speed.rs#L39)

```rust
pub fn new(window: usize, effective_portion: f64) -> Self
```

A window of `window` values keeping `effective_portion` of them.

### `FilteredRollingMean::add_value`
[src/tools/krita/speed.rs:48](../../../../src/tools/krita/speed.rs#L48)

```rust
pub fn add_value(&mut self, v: f64)
```

Adds a value, dropping the oldest when full.

### `FilteredRollingMean::filtered_mean`
[src/tools/krita/speed.rs:58](../../../../src/tools/krita/speed.rs#L58)

```rust
pub fn filtered_mean(&self) -> f64
```

The mean of the window with the extreme values trimmed from both ends.

## `impl Default for SpeedSmoother`

### `SpeedSmoother::default`
[src/tools/krita/speed.rs:89](../../../../src/tools/krita/speed.rs#L89)

```rust
fn default() -> Self
```

A fresh smoother.

## `impl SpeedSmoother`

### `SpeedSmoother::next`
[src/tools/krita/speed.rs:103](../../../../src/tools/krita/speed.rs#L103)

```rust
pub fn next(&mut self, pt: (f64, f64), time: f64) -> f64
```

`getNextSpeedImpl`: `pt` in view pixels, `time` in milliseconds. Returns view pixels per millisecond.

### `SpeedSmoother::normalized`
[src/tools/krita/speed.rs:135](../../../../src/tools/krita/speed.rs#L135)

```rust
pub fn normalized(&mut self, pt: (f64, f64), time: f64) -> f64
```

The speed sensor's value: `qMin(1.0, speed / maxAllowedSpeedValue)`.

## Tests

### `mod tests`

Run with `cargo test tools::krita::speed::tests` · 3 tests

| Test | What it checks |
|---|---|
| [`the_rolling_mean_trims_its_outliers`](../../../../src/tools/krita/speed.rs#L145) | The rolling mean trims its outliers. |
| [`a_steady_pen_reads_its_speed`](../../../../src/tools/krita/speed.rs#L158) | A pen moving 2 view px every 4 ms is going half a pixel per ms. |
| [`short_moves_hold_the_last_speed`](../../../../src/tools/krita/speed.rs#L169) | Short moves hold the last speed. |
