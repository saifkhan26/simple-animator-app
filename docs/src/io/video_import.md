# `src/io/video_import.rs` — importing frames from a video (ffmpeg)

> **Source:** [src/io/video_import.rs](../../../src/io/video_import.rs) · **Module:** `crate::io::video_import` · **Size:** 241 lines · Line links as of commit `1fa5845`

## What this file is for

Imports frames from a video file (MP4, MOV, MKV, …) for rotoscoping or reference, by running the external
programs **ffprobe** (to read frame count and fps) and **ffmpeg** (to extract frames). The flow:

1. `probe` (on a worker thread) reads the frame count and fps → the range dialog opens.
2. While choosing the range, `extract_preview` pulls single frames for the preview.
3. `extract_frames` writes the chosen range as PNGs into a temporary folder, decodes them into canvases,
   and deletes the folder (even on error).

## How it connects

- **Used by:** [app.rs](../app.md) (`open_video_import`, background jobs `BgJob::VideoProbe` etc.,
  `request_preview`).
- **Uses:** [doc/canvas.rs](../doc/canvas.md), `std::process::Command`, the `image` crate.

## Rust concepts in this file

**RAII cleanup with `Drop`.** `struct TempDir(PathBuf)` implements `Drop`, whose `drop` deletes the folder.
Rust calls `drop` automatically when the value goes out of scope — on normal return *and* on early return
from an error (`?`). Like a `try { … } finally { rmdir() }` you can't forget.
```rust
let _guard = TempDir(dir.clone());   // folder deleted whenever this function exits
```
More: [Traits and derive](../../rust-primer.md#traits-and-derive)

**Tuple struct.** `TempDir(PathBuf)` has one unnamed field, accessed as `self.0`.

## Types

### `struct TempDir`
[src/io/video_import.rs:234](../../../src/io/video_import.rs#L234)

RAII guard that removes the temp extraction directory when dropped.

| Field | Type | Meaning |
|---|---|---|
| `0` | `PathBuf` |  |

## Free functions

### `cmd`
[src/io/video_import.rs:16](../../../src/io/video_import.rs#L16)

```rust
fn cmd(program: &str) -> Command
```

Builds a `Command` that doesn't flash a console window on Windows (one ffmpeg run per preview frame would
otherwise blink a terminal).

### `probe`
[src/io/video_import.rs:30](../../../src/io/video_import.rs#L30)

```rust
pub fn probe(path: &Path) -> Result<(usize, f64)>
```

Reads the first video stream's frame count and fps with ffprobe: tries fast metadata (`nb_frames`, or
duration × frame rate) and only falls back to an exact (slow) count. Run off the UI thread.

### `parse_rational`
[src/io/video_import.rs:79](../../../src/io/video_import.rs#L79)

```rust
fn parse_rational(s: &str) -> f64
```

Parses ffprobe's rational numbers like `"30000/1001"` (→ 29.97) or plain `"25"`; 0 when invalid.

### `count_frames`
[src/io/video_import.rs:93](../../../src/io/video_import.rs#L93)

```rust
fn count_frames(path: &Path) -> Result<usize>
```

Exact frame count by decoding the whole stream (`-count_frames`) — slow, the fallback.

### `extract_preview`
[src/io/video_import.rs:125](../../../src/io/video_import.rs#L125)

```rust
pub fn extract_preview(path: &Path, frame: usize, fps: f64) -> Result<(u32, u32, Vec<u8>)>
```

Extracts one frame near index `frame` (fast seek by timestamp) for the range dialog, downscaled; returns
`(width, height, rgba)`. Run off the UI thread.

### `extract_frames`
[src/io/video_import.rs:169](../../../src/io/video_import.rs#L169)

```rust
pub fn extract_frames(path: &Path, start: usize, end: usize) -> Result<Vec<Canvas>>
```

Extracts frames `start..=end` with ffmpeg into a unique temp folder as PNGs, decodes them in order into
canvases at native resolution, and returns them; the folder is removed on return (by `TempDir`).

### `unique_temp_dir`
[src/io/video_import.rs:222](../../../src/io/video_import.rs#L222)

```rust
fn unique_temp_dir() -> Result<PathBuf>
```

Creates a new temp folder named with the current time in nanoseconds (so concurrent imports don't collide).

## `impl Drop for TempDir`

### `TempDir::drop`
[src/io/video_import.rs:237](../../../src/io/video_import.rs#L237)

```rust
fn drop(&mut self)
```

Deletes the temp folder (ignoring errors) when the guard goes away.
