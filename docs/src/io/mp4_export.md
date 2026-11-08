# `src/io/mp4_export.rs` — MP4 export through ffmpeg

> **Source:** [src/io/mp4_export.rs](../../../src/io/mp4_export.rs) · **Module:** `crate::io::mp4_export` · **Size:** 197 lines · Line links as of commit `67a7cb9`

## What this file is for

Exports the animation as an H.264 MP4 video by running the external program **ffmpeg** (it must be on the
PATH — the same requirement as video import). Each frame is flattened (all layers composited through the
camera), composited over black into plain RGB bytes, and **piped into ffmpeg's standard input** as raw
video; ffmpeg encodes it with libx264.

```text
for each frame:  flatten ─► RGBA ─► over black ─► RGB24 bytes ─► ffmpeg stdin ─► out.mp4
```

It runs on a worker thread (the caller's job), so the UI stays responsive.

## How it connects

- **Used by:** [app.rs](../app.md) (the export job, `ExportKind::Mp4`).
- **Uses:** [io/composite.rs](composite.md) (flattening frames), [io/frame_order.rs](frame_order.md) (the
  `order` of frames and stretched cameras), [doc/project.rs](../doc/project.md), `std::process::Command`.

## Rust concepts in this file

**Running another program.** `std::process::Command::new("ffmpeg").args([...]).stdin(Stdio::piped()).spawn()`
— like Node's `child_process.spawn("ffmpeg", args, { stdio: ["pipe", …] })`. Writing to `child.stdin`
streams frames in.

**Platform-specific extension traits.** On Windows, `use std::os::windows::process::CommandExt;` adds a
`creation_flags` method so ffmpeg runs without flashing a console window.

**`chunks_exact` + `zip`.** `src.chunks_exact(4).zip(dst.chunks_exact_mut(3))` walks the RGBA input four
bytes at a time alongside the RGB output three bytes at a time. More: [Iterators](../../rust-primer.md#iterators)

## Types

### `struct Mp4Settings`
[src/io/mp4_export.rs:23](../../../src/io/mp4_export.rs#L23)

Encoder settings chosen in the export dialog.

| Field | Type | Meaning |
|---|---|---|
| `crf` | `u32` | libx264 Constant Rate Factor: 0..=51, lower = better quality / bigger file. |
| `preset` | `&'static str` | libx264 preset (`ultrafast`..`veryslow`): speed/compression trade-off. |

### `const REUSE_BUDGET`
[src/io/mp4_export.rs:45](../../../src/io/mp4_export.rs#L45)

```rust
const REUSE_BUDGET: usize = 1 << 30;
```

Flattened frames kept for reuse by a looping or ping-pong order. Past this, a repeat is flattened again instead — slower, never out of memory.

## Free functions

### `cmd`
[src/io/mp4_export.rs:32](../../../src/io/mp4_export.rs#L32)

```rust
fn cmd(program: &str) -> Command
```

Builds a `Command` for `program` that, on Windows, won't pop up a console window.

### `export_to`
[src/io/mp4_export.rs:51](../../../src/io/mp4_export.rs#L51)

```rust
pub fn export_to(
    project: &Project,
    path: &Path,
    settings: &Mp4Settings,
    order: &[usize],
    cams: Option<&[Camera]>,
) -> Result<()>
```

Encodes the frames in `order` to an MP4 at `path`. Starts ffmpeg with raw-RGB input at the project fps and
the chosen CRF/preset, then for each frame flattens it (reusing already-flattened frames for loops/ping-pong
up to `REUSE_BUDGET`), converts it with `rgba_over_black`, and writes it to ffmpeg's stdin; finally waits for
ffmpeg and reports its error output if it failed. Blocking — call it from a worker thread.

### `rgba_over_black`
[src/io/mp4_export.rs:153](../../../src/io/mp4_export.rs#L153)

```rust
fn rgba_over_black(src: &[u8], dst: &mut [u8])
```

Composites straight-alpha RGBA over black into RGB24: each channel becomes `c × a / 255`
(e.g. red 200 at alpha 128 → 100). Video has no alpha, so transparent areas become black.

## Tests

### `mod tests`

Run with `cargo test io::mp4_export::tests` · 1 tests

| Test | What it checks |
|---|---|
| [`mp4_loop_fixture`](../../../src/io/mp4_export.rs#L172) | Not a unit test: needs ffmpeg on PATH. Writes `loop.mp4` (21 frames looped to 10 s) and `pingpong.mp4` (ping-ponged to 10 s) into `$MP4_OUT`, for ffprobe to count. |
