# `src/timeline/playback.rs` — the playback clock

> **Source:** [src/timeline/playback.rs](../../../src/timeline/playback.rs) · **Module:** `crate::timeline::playback` · **Size:** 376 lines · Line links as of commit `67a7cb9`

## What this file is for

When you press Play, something must decide which frame to show at each moment. That's `Playback`:
it remembers when the current frame started showing and, each time the app draws (every UI frame,
~60 per second), works out how many *animation* frames (at the project's fps, e.g. 24) have elapsed
and moves the playhead that many, wrapping around the loop range or stopping at its end.

It does **not** use a timer or `setInterval`. `AppState::update` calls `tick` with egui's clock every
frame and asks egui to repaint again after `1/fps` seconds; the frame shown is derived from elapsed
time, so a slow frame makes playback skip ahead rather than slow down.

A second mode handles a camera move that is stretched over several loops of the drawings ("a short
walk cycle under one long pan"): the drawings loop while the camera keeps a position of its own
(`shot_pos`).

## How it connects

- **Used by:** [app.rs](../app.md) — `AppState::advance_playback`, `play_pause`, `follow_stretched_camera`.
- **Uses:** [doc/project.rs](../doc/project.md) (frame count, loop range, fps, `current_frame`,
  camera loops), [io/frame_order.rs](../io/frame_order.md) (`shot_len`, `shot_cam_time`: shared with
  export so playback and exported files agree).

## Rust concepts in this file

**Time as `f64` seconds.** egui's clock (`ctx.input(|i| i.time)`) is a float of seconds, like
`performance.now() / 1000`.

**`isize` for maths that can go negative.** Frame indexes are `usize` (never negative), so the code
converts to `isize` (signed) to compute `current − loop_start + advance`, then uses `rem_euclid` to wrap
and converts back. More: [Numbers and casts](../../rust-primer.md#numbers-and-casts)

**`impl Default`.** `Playback::default()` is written out by hand; `Playback::default()` (or
`..Default::default()`) gives a stopped clock. More: [Traits and derive](../../rust-primer.md#traits-and-derive)

## Types

### `struct Playback`
[src/timeline/playback.rs:6](../../../src/timeline/playback.rs#L6)

The playback clock: whether it is playing, when the current frame started showing (egui clock seconds), and the position of a stretched camera move. One lives in `AppState::playback`.

| Field | Type | Meaning |
|---|---|---|
| `playing` | `bool` | Whether playback is running. |
| `last_advance_t` | `f64` | Wall-clock time (seconds) when the current frame started showing. |
| `unclocked` | `bool` | Started without a clock reading: the next `tick` takes its own time as `last_advance_t` instead of advancing. |
| `shot_pos` | `usize` | How far a camera move stretched over loops has got, in frames since it began. Only `Playback::tick_shot` moves it. |

## `impl Default for Playback`

### `Playback::default`
[src/timeline/playback.rs:19](../../../src/timeline/playback.rs#L19)

```rust
fn default() -> Self
```

A stopped clock at time 0.

## `impl Playback`

### `Playback::toggle`
[src/timeline/playback.rs:30](../../../src/timeline/playback.rs#L30)

```rust
pub fn toggle(&mut self, now: f64)
```

Play/pause with a clock reading `now`: flips `playing` and makes `now` the moment the current frame
started showing.

### `Playback::toggle_unclocked`
[src/timeline/playback.rs:39](../../../src/timeline/playback.rs#L39)

```rust
pub fn toggle_unclocked(&mut self)
```

Play/pause from a keyboard shortcut, which has no clock reading to hand. Marks the clock "unclocked"
so the first `tick` just records its time instead of advancing — otherwise it would jump by all the
time since playback last ran.

### `Playback::stop`
[src/timeline/playback.rs:44](../../../src/timeline/playback.rs#L44)

```rust
pub fn stop(&mut self)
```

Stops playback.

### `Playback::tick`
[src/timeline/playback.rs:54](../../../src/timeline/playback.rs#L54)

```rust
pub fn tick(&mut self, project: &mut Project, now: f64, looping: bool) -> bool
```

Advances the playhead by however many frames of wall-clock time have passed. Returns `true` if the
frame shown changed.

**How it works.**
1. Nothing to do if not playing, fps ≤ 0, or no frames. An unclocked start just takes `now` as its
   start time.
2. `frame_dur = 1/fps`; if less than one frame's time has passed, return.
3. `advance = floor(elapsed / frame_dur)`; the start time moves on by exactly that many frames (keeping
   the leftover fraction, so timing doesn't drift).
4. Compute the position within the loop range `[lo, hi)`. Looping: wrap with `rem_euclid`. Not looping:
   clamp to the last frame and stop.

**Example.** fps 24, loop frames 0–9, on frame 8, and 0.1 s passed: `advance = floor(0.1 / 0.0417) = 2`,
`raw = 8 + 2 = 10`, looping → `10 mod 10 = 0`, so the playhead wraps to frame 0.

### `Playback::range`
[src/timeline/playback.rs:97](../../../src/timeline/playback.rs#L97)

```rust
fn range(project: &Project) -> (usize, usize)
```

The first and last frame that plays: the loop range, clamped to the timeline (and at least one frame).

### `Playback::shot_at`
[src/timeline/playback.rs:108](../../../src/timeline/playback.rs#L108)

```rust
fn shot_at(&self, project: &Project) -> usize
```

How far a stretched camera move has got: the position the last tick left, or — if the playhead was moved
since (playback just started, a scrub) — the place in the move that the current frame stands for.

### `Playback::shot_cam_time`
[src/timeline/playback.rs:124](../../../src/timeline/playback.rs#L124)

```rust
pub fn shot_cam_time(&self, project: &Project) -> f32
```

Where the camera is in a stretched move, as a (fractional) time in timeline frames, using the same
formula as export.

### `Playback::tick_shot`
[src/timeline/playback.rs:137](../../../src/timeline/playback.rs#L137)

```rust
pub fn tick_shot(&mut self, project: &mut Project, now: f64, looping: bool) -> bool
```

`tick` for when the camera's move lasts more than one pass of the loop range: the move runs once over
several cycles of the drawings. Advances `shot_pos` by elapsed frames and sets `current_frame` to the
drawing for that position. Returns `true` when either moved. Without looping it stops on the move's last
frame.

## Tests

### `mod tests`

Run with `cargo test timeline::playback::tests` · 2 tests

| Test | What it checks |
|---|---|
| [`looping_playback_wraps_to_the_loop_start`](../../../src/timeline/playback.rs#L191) | Looping playback wraps to the loop start. |
| [`playback_stops_on_the_last_frame_when_not_looping`](../../../src/timeline/playback.rs#L202) | Playback stops on the last frame when not looping. |

**Test helpers** (used only by the tests above):

- [`scene`](../../../src/timeline/playback.rs#L176) — Scene.
- [`advance`](../../../src/timeline/playback.rs#L185) — One frame's worth of wall clock, so `tick` advances exactly once.

### `mod pins`

Run with `cargo test timeline::playback::pins` · 4 tests

| Test | What it checks |
|---|---|
| [`a_loop_range_plays_round_and_round`](../../../src/timeline/playback.rs#L249) | A loop range plays round and round. |
| [`without_looping_it_stops_on_the_last_frame_of_the_range`](../../../src/timeline/playback.rs#L256) | Without looping it stops on the last frame of the range. |
| [`a_slow_frame_skips_ahead_by_the_time_it_took`](../../../src/timeline/playback.rs#L263) | A slow frame skips ahead by the time it took. |
| [`a_playhead_past_the_range_wraps_into_it`](../../../src/timeline/playback.rs#L269) | A playhead past the range wraps into it. |

**Test helpers** (used only by the tests above):

- [`scene`](../../../src/timeline/playback.rs#L225) — Scene.
- [`run`](../../../src/timeline/playback.rs#L237) — The playhead after each of `steps` ticks, `frames` frames of wall clock apart — each a quarter frame late, so rounding in the clock can't land a tick just short of its frame.

### `mod shot_tests`

Run with `cargo test timeline::playback::shot_tests` · 5 tests

| Test | What it checks |
|---|---|
| [`the_drawings_loop_under_one_camera_move`](../../../src/timeline/playback.rs#L319) | The drawings go round three times while the camera goes from one key to the other once, and never back. |
| [`with_looping_on_the_whole_move_comes_round`](../../../src/timeline/playback.rs#L333) | With looping on the whole move comes round. |
| [`a_held_last_frame_stays_while_the_camera_carries_on`](../../../src/timeline/playback.rs#L343) | Holding, the last drawing stays up while the camera finishes its move. |
| [`ping_pong_goes_there_and_back_under_the_move`](../../../src/timeline/playback.rs#L351) | Ping pong goes there and back under the move. |
| [`play_takes_the_move_up_from_where_the_playhead_stands`](../../../src/timeline/playback.rs#L363) | Before the first tick lands, and after a scrub, the move is taken up from where the playhead stands in it — by the camera: the frame showing keeps the camera it had. |

**Test helpers** (used only by the tests above):

- [`scene`](../../../src/timeline/playback.rs#L284) — Five frames, the camera keyed from 0 on the first to 40 on the last, its move set to last three passes of them.
- [`run`](../../../src/timeline/playback.rs#L304) — The drawing showing and where the camera is after each of `steps` ticks a frame apart (each a quarter frame late, as in the pins).
