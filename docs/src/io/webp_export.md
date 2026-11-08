# `src/io/webp_export.rs` — animated WebP export

> **Source:** [src/io/webp_export.rs](../../../src/io/webp_export.rs) · **Module:** `crate::io::webp_export` · **Size:** 403 lines · Line links as of commit `1fa5845`

## What this file is for

Exports the animation as an **animated WebP** — lossless, so every frame keeps its exact colours and full
8-bit transparency (a GIF has 256 colours and on/off transparency). The `image-webp` crate encodes each
frame as a lossless `VP8L` image but only writes still images, so this file writes the **animation
container** itself, byte by byte:

```text
RIFF <size> WEBP
  VP8X  (flags: animation, alpha; canvas size)
  ANIM  (background colour, loop count)
  ANMF  (frame: x, y, w, h, duration, flags) + VP8L data      ← one per frame
  ANMF  …
```

Two size savings: a frame identical to the previous one isn't stored again (the previous frame's duration
grows instead), and a changed frame stores only the **rectangle that changed**.

## How it connects

- **Used by:** [app.rs](../app.md) (the export job, `ExportKind::Webp`).
- **Uses:** [io/composite.rs](composite.md) (`flatten_frame`), [io/frame_order.rs](frame_order.md), the
  `image-webp` crate.

## Rust concepts in this file

**Generic struct over a trait bound.** `struct AnimWriter<W>` with `impl<W: Write + Seek> AnimWriter<W>` works
with anything you can write to and seek in — a `File`, or an in-memory `Cursor<Vec<u8>>` in tests. Seeking
is needed to go back and patch the RIFF size once it's known. More: [Generics](../../rust-primer.md#generics)

**Little-endian bytes.** `v.to_le_bytes()` gives a number's bytes lowest first, as RIFF requires — like
`DataView.setUint32(off, v, true)`.

## Types

### `const MAX_SIDE`
[src/io/webp_export.rs:28](../../../src/io/webp_export.rs#L28)

```rust
const MAX_SIDE: u32 = 16384;
```

The widest and tallest a VP8L frame can be.

### `const REUSE_BUDGET`
[src/io/webp_export.rs:32](../../../src/io/webp_export.rs#L32)

```rust
const REUSE_BUDGET: usize = 1 << 30;
```

Flattened frames kept for reuse by a ping-pong order, as MP4 export has it. Past this, a repeat is flattened again instead.

### `const MAX_DURATION`
[src/io/webp_export.rs:35](../../../src/io/webp_export.rs#L35)

```rust
const MAX_DURATION: u32 = (1 << 24) - 1;
```

Longest a frame can show: the `ANMF` duration is 24 bits of milliseconds.

### `struct Pending`
[src/io/webp_export.rs:96](../../../src/io/webp_export.rs#L96)

A frame stored but not yet written: how long it shows is only known once a frame that differs from it comes along, or the animation ends.

| Field | Type | Meaning |
|---|---|---|
| `x` | `u32` | Where its rectangle sits on the canvas, and its size. |
| `y` | `u32` | Top of its rectangle on the canvas. |
| `w` | `u32` | Width of its rectangle. |
| `h` | `u32` | Height of its rectangle. |
| `start` | `u32` | When it starts showing, in ms. |
| `vp8l` | `Vec<u8>` | Its `VP8L` chunk, header and padding included. |

### `struct AnimWriter`
[src/io/webp_export.rs:109](../../../src/io/webp_export.rs#L109)

An animated WebP, written a frame at a time.

| Field | Type | Meaning |
|---|---|---|
| `out` | `W` | Where the file is being written. |
| `w` | `u32` | Canvas width. |
| `h` | `u32` | Canvas height. |
| `shown` | `Vec<u8>` | The canvas as the stored frames so far leave it. Empty before the first frame. |
| `pending` | `Option<Pending>` | The frame stored but not yet written (its duration is known only when the next different frame arrives). |
| `alpha` | `bool` | Whether any stored pixel is less than opaque — the `VP8X` alpha flag, which decoders read to decide whether to keep alpha at all. |

## Free functions

### `export_to`
[src/io/webp_export.rs:41](../../../src/io/webp_export.rs#L41)

```rust
pub fn export_to(
    project: &Project,
    path: &Path,
    order: &[usize],
    cams: Option<&[Camera]>,
    plays: u16,
) -> Result<()>
```

Writes the frames in `order` as one animated WebP at `path`: flattens each (reusing frames for ping-pong
orders up to `REUSE_BUDGET`), feeds it to an `AnimWriter` with its start time (`index × 1000 / fps` ms), and
finishes with the total length. `plays` is the loop count (0 = forever). Refuses canvases larger than
`MAX_SIDE`.

## `impl<W: Write + Seek> AnimWriter<W>`

### `AnimWriter::new`
[src/io/webp_export.rs:125](../../../src/io/webp_export.rs#L125)

```rust
fn new(mut out: W, w: u32, h: u32, plays: u16) -> Result<Self>
```

Writes the RIFF/WEBP header, a `VP8X` chunk and the `ANIM` chunk (transparent background, loop count); the
RIFF size and flags are placeholders patched by `finish`.

### `AnimWriter::frame`
[src/io/webp_export.rs:147](../../../src/io/webp_export.rs#L147)

```rust
fn frame(&mut self, rgba: &[u8], start: u32) -> Result<()>
```

Adds a whole frame starting at `start` ms: if it's identical to what's shown, nothing is stored (the
pending frame just lasts longer); otherwise the pending frame is flushed and this frame's changed rectangle
is encoded and becomes pending.

### `AnimWriter::flush`
[src/io/webp_export.rs:176](../../../src/io/webp_export.rs#L176)

```rust
fn flush(&mut self, end: u32) -> Result<()>
```

Writes the pending frame as an `ANMF` chunk lasting until `end` ms (no blending, so the stored rectangle
replaces what was there).

### `AnimWriter::finish`
[src/io/webp_export.rs:200](../../../src/io/webp_export.rs#L200)

```rust
fn finish(mut self, end: u32) -> Result<()>
```

Flushes the last frame, then seeks back to patch the RIFF size and the `VP8X` alpha flag.

## Free functions

### `changed`
[src/io/webp_export.rs:218](../../../src/io/webp_export.rs#L218)

```rust
fn changed(a: &[u8], b: &[u8], width: usize) -> Option<(usize, usize, usize, usize)>
```

The bounding rectangle of pixels that differ between two RGBA frames, or `None` if identical.

### `vp8l`
[src/io/webp_export.rs:238](../../../src/io/webp_export.rs#L238)

```rust
fn vp8l(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>>
```

Encodes an RGBA buffer as a lossless `VP8L` chunk (with the `image-webp` encoder), including chunk header
and padding.

### `u24`
[src/io/webp_export.rs:250](../../../src/io/webp_export.rs#L250)

```rust
fn u24(v: u32) -> [u8; 3]
```

The lowest three bytes of `v`, little-endian — WebP stores some fields (sizes, durations) as 24-bit
numbers.

## Tests

### `mod tests`

Run with `cargo test io::webp_export::tests` · 6 tests

| Test | What it checks |
|---|---|
| [`frames_come_back_exactly_with_holds_merged`](../../../src/io/webp_export.rs#L306) | Frames come back exactly with holds merged. |
| [`a_later_frame_stores_only_what_changed`](../../../src/io/webp_export.rs#L329) | A later frame stores only what changed. |
| [`ping_pong_and_play_count`](../../../src/io/webp_export.rs#L339) | Ping pong and play count. |
| [`a_webp_is_the_camera_s_size`](../../../src/io/webp_export.rs#L352) | A webp is the camera s size. |
| [`probe_webp_against_gif`](../../../src/io/webp_export.rs#L368) | Size and time against GIF on a real project. Not a pass/fail test — run it for the numbers: `ANIM_PROBE=path/to/project.anim cargo test probe_webp_against_gif -- --ignored --nocapture` Leaves `animator-probe.webp` and `.gif` in the temp folder. |
| [`an_opaque_animation_says_so`](../../../src/io/webp_export.rs#L390) | An opaque animation says so. |

**Test helpers** (used only by the tests above):

- [`tmp`](../../../src/io/webp_export.rs#L260) — Tmp.
- [`project`](../../../src/io/webp_export.rs#L266) — An 8×6 project, `frames` long, with a drawing keyed on each of `keys`: opaque, half-transparent and clear pixels, different on every key.
- [`decode`](../../../src/io/webp_export.rs#L287) — Every frame of the WebP at `path`, as players show them, with how long each shows; and how many times it plays (0 = forever).
