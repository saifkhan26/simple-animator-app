# `src/tools/stabilizer.rs` — Krita's Stabilizer smoothing

> **Source:** [src/tools/stabilizer.rs](../../../src/tools/stabilizer.rs) · **Module:** `crate::tools::stabilizer` · **Size:** 302 lines · Line links as of commit `1fa5845`

## What this file is for

A port of Krita's **Stabilizer** line smoothing (`KisToolFreehandHelper`'s stabilizer functions). Instead of
painting pen events as they arrive, a timer polls every `poll_ms`; at each poll the pen's recent movement is
resampled at 1 ms steps and the line advances to the **average of the last N samples**. The line therefore
lags behind the pen and wobble averages out. With **delay** on, the line also stays put until the pen pulls
it — like dragging the line on a string `delay` pixels long.

**Example (averaging).** With 3 samples held at x = 0, 0, 0 and the pen jumping to x = 9: the next stabilized
point is the average of the newest three samples, `(0 + 0 + 9)/3 = 3`; next poll `(0 + 9 + 9)/3 = 6`; then 9 —
the jump becomes a smooth ramp.

Because the line keeps catching up while the pen is held still, the app calls the stroke's `tick` every
frame (`AppState::tick_stroke`).

## How it connects

- **Used by:** [tools/stroke.rs](stroke.md) (a stroke with `Smoothing::Stabilizer` owns a `Stabilizer`).
- **Uses:** the pointer sample type; settings from [tools/mod.rs](mod.md) (`SmoothingOptions`).

## Rust concepts in this file

**`VecDeque` as a sliding window.** `deque.pop_front(); deque.push_back(s);` keeps the last N samples — like
`arr.shift(); arr.push(s)` but O(1). More: [Collections](../../rust-primer.md#collections)

## Types

### `struct StabilizerSettings`
[src/tools/stabilizer.rs:26](../../../src/tools/stabilizer.rs#L26)

What the stabilizer does, resolved for one stroke.

| Field | Type | Meaning |
|---|---|---|
| `sample_size` | `usize` | How many 1 ms samples each painted point averages, at least 3. |
| `delay` | `Option<f32>` | The delay radius, in canvas pixels; `None` paints from the start. |
| `finish` | `bool` | Draw the line on to where the pen lifted. |
| `sensors` | `bool` | Average pressure, tilt and the rest along with position. |
| `poll_ms` | `f64` | How often the line catches up, milliseconds. |

### `struct Sampler`
[src/tools/stabilizer.rs:41](../../../src/tools/stabilizer.rs#L41)

`KisStabilizedEventsSampler` with Krita's 1 ms sample time.

| Field | Type | Meaning |
|---|---|---|
| `real` | `Vec<PointerSample>` | Pen events received since the last poll. |
| `last_paint_time` | `f64` | Time (ms) of the last poll. |
| `override_ms` | `i64` | Extra milliseconds to generate (used to finish the line at pen-up). |
| `last` | `PointerSample` | The last event, repeated when none arrived. |

### `struct Stabilizer`
[src/tools/stabilizer.rs:85](../../../src/tools/stabilizer.rs#L85)

One stroke's stabilizer.

| Field | Type | Meaning |
|---|---|---|
| `settings` | `StabilizerSettings` | This stroke's stabilizer settings. |
| `deque` | `VecDeque<PointerSample>` | The last `sample_size` samples, averaged for each painted point. |
| `sampler` | `Sampler` | Turns irregular pen events into 1 ms samples. |
| `previous` | `PointerSample` | The line's end: Krita's `previousPaintInformation`. |
| `last_poll` | `f64` | Time of the last poll. |

## `impl Sampler`

### `Sampler::clear`
[src/tools/stabilizer.rs:49](../../../src/tools/stabilizer.rs#L49)

```rust
fn clear(&mut self, now: f64)
```

Forgets received events and restarts the clock at `now`.

### `Sampler::add_finishing`
[src/tools/stabilizer.rs:57](../../../src/tools/stabilizer.rs#L57)

```rust
fn add_finishing(&mut self, samples: usize, now: f64)
```

Asks the next `range` to produce `samples` extra samples (repeating the last event) — used at pen-up to pull
the averaged line onto where the pen lifted.

### `Sampler::range`
[src/tools/stabilizer.rs:68](../../../src/tools/stabilizer.rs#L68)

```rust
fn range(&mut self, now: f64) -> Vec<PointerSample>
```

One sample per whole millisecond since the last call, spread over the events received meanwhile (the last
event repeated when none came).

## Free functions

### `mix_other`
[src/tools/stabilizer.rs:96](../../../src/tools/stabilizer.rs#L96)

```rust
fn mix_other(this: PointerSample, t: f32, other: &PointerSample, position_only: bool) -> PointerSample
```

Krita's `mixOtherImpl`: a sample moved from `other` toward `this` by `t` in every property except time (or
only in position with `position_only`).

## `impl Stabilizer`

### `Stabilizer::start`
[src/tools/stabilizer.rs:111](../../../src/tools/stabilizer.rs#L111)

```rust
pub fn start(first: PointerSample, settings: StabilizerSettings, now: f64) -> Self
```

Starts the stabilizer at pen-down: the deque filled with the first event.

### `Stabilizer::add_event`
[src/tools/stabilizer.rs:127](../../../src/tools/stabilizer.rs#L127)

```rust
pub fn add_event(&mut self, s: PointerSample)
```

Records a pen event for the next poll.

### `Stabilizer::last_event`
[src/tools/stabilizer.rs:132](../../../src/tools/stabilizer.rs#L132)

```rust
pub fn last_event(&self) -> PointerSample
```

The newest event the pen sent.

### `Stabilizer::end_point`
[src/tools/stabilizer.rs:137](../../../src/tools/stabilizer.rs#L137)

```rust
pub fn end_point(&self) -> PointerSample
```

Where the stabilized line has got to.

### `Stabilizer::due`
[src/tools/stabilizer.rs:142](../../../src/tools/stabilizer.rs#L142)

```rust
pub fn due(&self, now: f64) -> bool
```

Whether a poll is due at `now`.

### `Stabilizer::stabilized`
[src/tools/stabilizer.rs:148](../../../src/tools/stabilizer.rs#L148)

```rust
fn stabilized(&self, last: PointerSample) -> PointerSample
```

`getStabilizedPaintInfo`: the average of the deque with `last` replacing its oldest entry — position (and
pressure/tilt if `sensors`).

### `Stabilizer::poll`
[src/tools/stabilizer.rs:160](../../../src/tools/stabilizer.rs#L160)

```rust
pub fn poll(&mut self, now: f64) -> Vec<(PointerSample, PointerSample)>
```

`stabilizerPollAndPaint`: for each new 1 ms sample, slides the window, computes the averaged point, applies
the delay string, and returns the segments to paint (from the line's end to each next point).

### `Stabilizer::end`
[src/tools/stabilizer.rs:186](../../../src/tools/stabilizer.rs#L186)

```rust
pub fn end(&mut self, now: f64) -> Vec<(PointerSample, PointerSample)>
```

`stabilizerEnd`: paints what is pending, then (if finishing) feeds the last event enough times to bring the
average exactly onto where the pen lifted.

## Tests

### `mod tests`

Run with `cargo test tools::stabilizer::tests` · 5 tests

| Test | What it checks |
|---|---|
| [`the_sampler_spreads_a_poll_s_events_over_its_milliseconds`](../../../src/tools/stabilizer.rs#L228) | The sampler spreads a poll s events over its milliseconds. |
| [`the_line_trails_the_pen_by_its_sample_size`](../../../src/tools/stabilizer.rs#L245) | The line trails the pen by its sample size. |
| [`a_held_pen_lets_the_line_catch_up`](../../../src/tools/stabilizer.rs#L260) | A held pen lets the line catch up. |
| [`the_delay_holds_the_line_on_a_string`](../../../src/tools/stabilizer.rs#L271) | The delay holds the line on a string. |
| [`sensors_average_only_when_asked`](../../../src/tools/stabilizer.rs#L288) | Sensors average only when asked. |

**Test helpers** (used only by the tests above):

- [`at`](../../../src/tools/stabilizer.rs#L201) — At.
- [`settings`](../../../src/tools/stabilizer.rs#L205) — Settings.
- [`drive`](../../../src/tools/stabilizer.rs#L216) — A pen that moves 1 px per ms from `from` to `to`, polled every 50 ms.
