# `src/io/frame_order.rs` — which timeline frame each exported frame shows

> **Source:** [src/io/frame_order.rs](../../../src/io/frame_order.rs) · **Module:** `crate::io::frame_order` · **Size:** 295 lines · Line links as of commit `36405ad`

## What this file is for

An export does not always write the timeline frames once, in order. This file turns the
export dialog's choices into a plain list of frame numbers — "output frame 0 shows timeline
frame 3, output frame 1 shows frame 4, …" — that every exporter then walks.

Three ideas live here:

- **Plain**: the chosen range once, e.g. `3..=6` → `[3, 4, 5, 6]`.
- **Ping-pong**: forward then back, without doubling the end frames so it loops seamlessly:
  `0..=4` → `[0, 1, 2, 3, 4, 3, 2, 1]`.
- **Loop to N seconds**: repeat that cycle until it fills at least N seconds, rounded **up**
  to whole cycles so the file ends on a cycle boundary (a player that repeats the file then
  repeats it seamlessly).

There is also a "shot" mode for when the **camera move lasts longer than the drawings**: the
camera's keyed move is stretched over several passes (loops) of the drawings. Then each output
frame needs two things — which drawings to show (`frames`) and *when along the camera move*
to look at them (`cam_times`). See [doc/camera.rs](../doc/camera.md) for the camera's `Fill`
option (what the drawings do during those passes: `Loop`, `PingPong` or `Hold`).

## How it connects

- **Used by:**
  - [app.rs](../app.md) — `ExportConfig::playback` builds a `Playback` (ping-pong for every
    format but sprite sheets; *Loop to* only for MP4 and PNG sequences, since a GIF or WebP
    repeats on its own); `export_info` calls
    `info` / `shot_info` for the dialog's summary; `export_order` calls `frame_order` / `shot`
    to get the list handed to the export worker thread.
  - [gif_export.rs](gif_export.md), [webp_export.rs](webp_export.md),
    [mp4_export.rs](mp4_export.md) — call `uses` to know when a cached frame can be dropped.
  - [timeline/playback.rs](../timeline/playback.md) — uses `shot_len`, `shot_frame`,
    `shot_cam_time` so on-screen playback of a stretched camera matches the export.
  - [ui/shell.rs](../ui/shell.md) — uses `shot_len`.
- **Uses:** [doc/camera.rs](../doc/camera.md) (`Fill`), `std::collections::HashMap`.

## Rust concepts in this file

**`#[derive(...)]` gives a struct free abilities.** `Copy` lets it be passed by value like a
number; `Default` gives an all-zero/`false`/`None` value; `PartialEq` allows `==`.
```rust
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Playback { pub ping_pong: bool, pub loop_secs: Option<f32> }
let plain = Playback::default(); // { ping_pong: false, loop_secs: None }
```
More: [Traits and derive](../../rust-primer.md#traits-and-derive)

**Inclusive ranges and reversed ranges.** `a..=b` includes `b`; `a..b` excludes it.
`.rev()` walks backwards.
```rust
let mut c: Vec<usize> = (a..=b).collect();   // [a, ..., b]
c.extend((a + 1..b).rev());                  // b-1 down to a+1
```
```js
const c = []; for (let i = a; i <= b; i++) c.push(i);
for (let i = b - 1; i > a; i--) c.push(i);
```
More: [Iterators](../../rust-primer.md#iterators)

**`match` with a guard.** `Fill::PingPong if pass >= 3 => …` only matches when the condition
holds; `_` catches everything else.
```rust
match fill {
    Fill::PingPong if pass >= 3 => 2 * pass - 2,
    _ => pass,
}
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

**`HashMap::entry` for counting.**
```rust
*m.entry(f).or_insert(0) += 1;
```
```js
m.set(f, (m.get(f) ?? 0) + 1);
```
More: [Collections](../../rust-primer.md#collections)

**`self::loops` — naming the function, not the variable.** Inside `shot_info` a parameter is
also called `loops`, so the code writes `self::loops(...)` to mean "the function `loops` in
this module". More: [Modules and visibility](../../rust-primer.md#modules-and-visibility)

## Types

### `struct Playback`
[src/io/frame_order.rs:15](../../../src/io/frame_order.rs#L15)

How an export plays its frame range. (Not to be confused with the on-screen playback clock
`timeline::playback::Playback`.)

| Field | Type | Meaning |
|---|---|---|
| `ping_pong` | `bool` | Play forward, then back; the end frames are not doubled so the wrap is seamless. |
| `loop_secs` | `Option<f32>` | Fill at least this many seconds with whole cycles. `None` = one cycle. |

### `struct OrderInfo`
[src/io/frame_order.rs:24](../../../src/io/frame_order.rs#L24)

What an order adds up to, shown as a summary in the export dialog.

| Field | Type | Meaning |
|---|---|---|
| `cycle` | `usize` | Frames in one cycle. |
| `loops` | `usize` | How many times the cycle repeats. |
| `frames` | `usize` | Total frames written (`cycle × loops`). |
| `secs` | `f32` | Total length in seconds (`frames / fps`). |

### `struct Shot`
[src/io/frame_order.rs:121](../../../src/io/frame_order.rs#L121)

An export whose camera move spans several passes of the drawings. Both vectors have one entry
per written frame. *Loop to* repeats the whole move, camera and all.

| Field | Type | Meaning |
|---|---|---|
| `frames` | `Vec<usize>` | Which timeline frame's drawings each output frame shows. |
| `cam_times` | `Vec<f32>` | The camera time (in timeline frames, may be fractional) each output frame is seen at. |

## Free functions

### `cycle`
[src/io/frame_order.rs:36](../../../src/io/frame_order.rs#L36)

```rust
fn cycle(start: usize, end: usize, ping_pong: bool) -> Vec<usize>
```

**What it does.** One pass over `start..=end` (either can be the larger), ping-ponged if asked.
With fewer than three frames there is nothing between the ends to come back through, so
ping-pong gives the plain pass.

**Example.** `cycle(0, 2, true)` → `[0, 1, 2, 1]`; `cycle(0, 1, true)` → `[0, 1]`;
`cycle(6, 3, false)` → `[3, 4, 5, 6]`.

### `loops`
[src/io/frame_order.rs:45](../../../src/io/frame_order.rs#L45)

```rust
fn loops(cycle_len: usize, pb: Playback, fps: f32) -> usize
```

**What it does.** How many cycles to write. With no `loop_secs`, 1. Otherwise the target frame
count `round(secs × fps)` is computed **first** (so 10 s at 24 fps is exactly 240, and float
noise cannot add an extra loop), then divided by the cycle length rounding up, with a minimum
of 1.

**Example.** 21-frame cycle, 10 s at 24 fps: target 240, `ceil(240 / 21) = 12` loops = 252
frames (10.5 s).

### `info`
[src/io/frame_order.rs:57](../../../src/io/frame_order.rs#L57)

```rust
pub fn info(start: usize, end: usize, pb: Playback, fps: f32) -> OrderInfo
```

**What it does.** The summary (`cycle`, `loops`, `frames`, `secs`) of a plain or ping-pong
export, without building the list.

**Used by:** [app.rs](../app.md) (`export_info`).

### `frame_order`
[src/io/frame_order.rs:65](../../../src/io/frame_order.rs#L65)

```rust
pub fn frame_order(start: usize, end: usize, pb: Playback, fps: f32) -> Vec<usize>
```

**What it does.** The full list of timeline frames to write, in order: `cycle` repeated
`loops` times.

**Returns.** One timeline frame index per output frame.

**Used by:** [app.rs](../app.md) (`export_order`), tests in [mp4_export.rs](mp4_export.md) and
[png_seq.rs](png_seq.md).

### `fill_cycle`
[src/io/frame_order.rs:76](../../../src/io/frame_order.rs#L76)

```rust
fn fill_cycle(pass: usize, fill: Fill) -> usize
```

**What it does.** Frames in one cycle of `fill` over a range `pass` frames long: `2·pass − 2`
for ping-pong of 3+ frames (forward then back, ends not doubled — same rule as `cycle`),
otherwise `pass`.

### `shot_len`
[src/io/frame_order.rs:86](../../../src/io/frame_order.rs#L86)

```rust
pub fn shot_len(start: usize, end: usize, loops: u32, fill: Fill) -> usize
```

**What it does.** How many frames a camera move spanning `loops` passes of `start..=end`
lasts: `fill_cycle(pass) × loops` (with `loops` at least 1).

**Example.** Range `0..=99`, 3 loops: `Loop` → 300, `PingPong` → `3 × 198 = 594`.

**Used by:** `shot_info`; [timeline/playback.rs](../timeline/playback.md);
[ui/shell.rs](../ui/shell.md).

### `shot_frame`
[src/io/frame_order.rs:92](../../../src/io/frame_order.rs#L92)

```rust
pub fn shot_frame(start: usize, end: usize, fill: Fill, i: usize) -> usize
```

**What it does.** The timeline frame whose drawings frame `i` of such a move shows.

**How it works.** With `a` the lower end and `pass` the range length:
- `Loop` → `a + i % pass` (wrap round).
- `PingPong` → `k = i % cycle`; going out while `k < pass`, coming back as `cycle − k`.
- `Hold` → `a + min(i, pass − 1)` (stay on the last frame).

**Example.** Range `2..=4`: `Loop` gives `2,3,4,2,3,4…`; `PingPong` gives `2,3,4,3,2,3,4,3…`;
`Hold` gives `2,3,4,4,4,4…`.

**Used by:** `shot`; [timeline/playback.rs](../timeline/playback.md).

### `shot_cam_time`
[src/io/frame_order.rs:110](../../../src/io/frame_order.rs#L110)

```rust
pub fn shot_cam_time(start: usize, end: usize, len: usize, i: usize) -> f32
```

**What it does.** Where the camera is on frame `i` of a move `len` frames long, as a time in
timeline frames: exactly on `start` for the first frame, on `end` for the last, evenly spaced
between. The camera keys stay where they are on the timeline; only the journey through them
is stretched. With `len < 2` it returns `start`.

**Example.** Range `0..=99` over 300 frames: frame 0 → 0.0, frame 299 → 99.0, frame 150 →
`99 × 150 / 299 ≈ 49.67`.

**Used by:** `shot`; [timeline/playback.rs](../timeline/playback.md).

### `shot_info`
[src/io/frame_order.rs:128](../../../src/io/frame_order.rs#L128)

```rust
pub fn shot_info(start: usize, end: usize, loops: u32, fill: Fill, loop_secs: Option<f32>, fps: f32) -> OrderInfo
```

**What it does.** Like `info` but for a stretched camera move: here one "cycle" is the
*whole move* (`shot_len`), and *Loop to* repeats whole moves (never ping-pong).

**Used by:** `shot`; [app.rs](../app.md) (`export_info`).

### `shot`
[src/io/frame_order.rs:135](../../../src/io/frame_order.rs#L135)

```rust
pub fn shot(start: usize, end: usize, loops: u32, fill: Fill, loop_secs: Option<f32>, fps: f32) -> Shot
```

**What it does.** Builds the `frames` and `cam_times` lists for a stretched camera move: for
each repeat of the move, for each frame `i` of it, pushes `shot_frame(i)` and
`shot_cam_time(i)`.

**Used by:** [app.rs](../app.md) (`export_order`, which turns each `cam_time` into a `Camera`
with `resolve_camera_at`).

### `uses`
[src/io/frame_order.rs:150](../../../src/io/frame_order.rs#L150)

```rust
pub fn uses(order: &[usize]) -> HashMap<usize, usize>
```

**What it does.** Counts how many times each timeline frame appears in `order`. An exporter
flattens each distinct frame once, keeps it in a cache, and counts this number down as it
writes — when it reaches zero the cached image can be dropped to save memory.

**Example.** `uses(&[0, 1, 2, 1])` → `{0: 1, 1: 2, 2: 1}`.

**Used by:** [gif_export.rs](gif_export.md), [webp_export.rs](webp_export.md),
[mp4_export.rs](mp4_export.md).

## Tests

Run with `cargo test io::frame_order`.

| Test | What it checks |
|---|---|
| [`plain_range_is_played_once`](../../../src/io/frame_order.rs#L167) | A plain range plays once, in order, whichever end is given first. |
| [`ping_pong_does_not_double_the_ends`](../../../src/io/frame_order.rs#L173) | Ping-pong of 1, 2, 3 and 5 frames never repeats the end frames. |
| [`loop_rounds_up_to_whole_cycles`](../../../src/io/frame_order.rs#L182) | 21 frames looped to 10 s at 24 fps gives 12 whole cycles (252 frames, 10.5 s). |
| [`exact_fit_adds_no_extra_loop`](../../../src/io/frame_order.rs#L193) | A 40-frame ping-pong cycle fits 10 s exactly in 6 loops, no extra. |
| [`short_length_still_plays_one_cycle`](../../../src/io/frame_order.rs#L201) | A tiny or zero loop length still writes one cycle. |
| [`fractional_frame_rate`](../../../src/io/frame_order.rs#L207) | 23.976 fps rounds the target to 240 frames → 12 cycles. |
| [`uses_counts_repeats`](../../../src/io/frame_order.rs#L214) | `uses` counts the middle frame of a ping-pong twice. |
| [`one_loop_is_the_plain_order`](../../../src/io/frame_order.rs#L230) | A one-loop shot gives the same list as a plain/ping-pong `frame_order`. |
| [`the_drawings_come_round_as_the_fill_has_them`](../../../src/io/frame_order.rs#L240) | `Loop`, `PingPong` and `Hold` produce the expected drawing sequences. |
| [`the_camera_makes_its_move_once_over_all_the_passes`](../../../src/io/frame_order.rs#L251) | Camera time goes 0 → 99 across all 300 frames, always increasing. |
| [`with_one_loop_the_camera_keeps_time_with_the_frames`](../../../src/io/frame_order.rs#L265) | With one loop, each frame's camera time equals its own frame number. |
| [`loop_to_repeats_the_whole_move`](../../../src/io/frame_order.rs#L275) | *Loop to* repeats the whole move; the camera jumps back to its start. |
| [`a_shot_s_length_is_its_cycles`](../../../src/io/frame_order.rs#L288) | `shot_len` per fill mode, and never fewer than one loop. |

**Test helpers:**

### `mod tests`

Run with `cargo test io::frame_order::tests` · 7 tests

| Test | What it checks |
|---|---|
| [`plain_range_is_played_once`](../../../src/io/frame_order.rs#L167) | Plain range is played once. |
| [`ping_pong_does_not_double_the_ends`](../../../src/io/frame_order.rs#L173) | Ping pong does not double the ends. |
| [`loop_rounds_up_to_whole_cycles`](../../../src/io/frame_order.rs#L182) | The user's shot: 21 frames at 24 fps, looped to 10 s. |
| [`exact_fit_adds_no_extra_loop`](../../../src/io/frame_order.rs#L193) | Exact fit adds no extra loop. |
| [`short_length_still_plays_one_cycle`](../../../src/io/frame_order.rs#L201) | Short length still plays one cycle. |
| [`fractional_frame_rate`](../../../src/io/frame_order.rs#L207) | Fractional frame rate. |
| [`uses_counts_repeats`](../../../src/io/frame_order.rs#L214) | Uses counts repeats. |

**Test helpers** (used only by the tests above):

- [`pb`](../../../src/io/frame_order.rs#L162) — Pb.

### `mod shot_tests`

Run with `cargo test io::frame_order::shot_tests` · 6 tests

| Test | What it checks |
|---|---|
| [`one_loop_is_the_plain_order`](../../../src/io/frame_order.rs#L230) | One loop is the range once — the order a plain export has always had. |
| [`the_drawings_come_round_as_the_fill_has_them`](../../../src/io/frame_order.rs#L240) | The drawings come round as the fill has them. |
| [`the_camera_makes_its_move_once_over_all_the_passes`](../../../src/io/frame_order.rs#L251) | The whole move, once: the first frame on the first key's frame, the last on the last, however many passes it takes. |
| [`with_one_loop_the_camera_keeps_time_with_the_frames`](../../../src/io/frame_order.rs#L265) | With one loop the camera is on each frame's own time: the frames and the camera run together, as they do without any of this. |
| [`loop_to_repeats_the_whole_move`](../../../src/io/frame_order.rs#L275) | *Loop to* repeats the whole move: the camera goes back to its start with the drawings. |
| [`a_shot_s_length_is_its_cycles`](../../../src/io/frame_order.rs#L288) | A shot s length is its cycles. |

**Test helpers** (used only by the tests above):

- [`frames`](../../../src/io/frame_order.rs#L224) — Frames.
