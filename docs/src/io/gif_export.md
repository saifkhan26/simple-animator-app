# `src/io/gif_export.rs` — write the animation as an animated GIF

> **Source:** [src/io/gif_export.rs](../../../src/io/gif_export.rs) · **Module:** `crate::io::gif_export` · **Size:** 116 lines · Line links as of commit `67a7cb9`

## What this file is for

This file writes an **animated GIF** that repeats forever. For each output frame it:

1. flattens the frame into one RGBA image ([composite.rs](composite.md)),
2. reduces it to a **256-colour palette** — a GIF frame can only hold 256 different colours —
   using the *NeuQuant* algorithm (a small neural-network colour quantiser from the
   `color_quant` crate; see [glossary](../../glossary.md)),
3. writes it with a delay that matches the project's frame rate.

GIF has no partial transparency: a pixel is either fully see-through or fully solid. Pixels
that are almost clear (alpha below 8) become the GIF's "transparent" colour, palette index 0;
everything else is solid.

The frame list comes from [frame_order.rs](frame_order.md). Because ping-pong comes back
through the same frames, each distinct frame is quantised once and kept until its last use.

The export runs on a **worker thread** started by `start_export` in [app.rs](../app.md), so
the UI stays responsive; the result (`Ok` or an error) is sent back over a channel.

## How it connects

- **Used by:** [app.rs](../app.md) — `start_export` (on a worker thread) for `ExportKind::Gif`;
  a test in [webp_export.rs](webp_export.md) writes a GIF to compare sizes.
- **Uses:** [composite.rs](composite.md) (`flatten_through`), [frame_order.rs](frame_order.md)
  (`uses`), [doc/project.rs](../doc/project.md) (`output_size`, `fps`),
  [doc/camera.rs](../doc/camera.md) (`Camera`); external crates `gif` (encoder) and
  `color_quant` (NeuQuant), `anyhow` (errors).

## Rust concepts in this file

**`Result` and `?` with `anyhow::Context`.** Every step that can fail returns a `Result`.
`?` means "if this is an error, return it from the function now". `.context(...)` /
`.with_context(|| ...)` wraps the error with a human message, like adding a note to a thrown
error.
```rust
let file = File::create(path).with_context(|| format!("creating {path:?}"))?;
encoder.write_frame(&frame).context("write_frame")?;
```
```js
let file;
try { file = fs.openSync(path, "w"); } catch (e) { throw new Error(`creating ${path}`, { cause: e }); }
```
More: [Option and Result](../../rust-primer.md#option-and-result)

**`Option::filter` and `map_or`.** `kept.remove(&f)` returns `Option<Frame>`; `.filter(|_| reuse)`
throws the value away (turns it into `None`) when `reuse` is false.
```rust
let frame = match kept.remove(&f).filter(|_| reuse) {
    Some(frame) => frame,
    None => { /* flatten and encode */ }
};
```
More: [Option and Result](../../rust-primer.md#option-and-result)

**`Cow::Owned`.** The `gif` crate's `Frame` holds its pixels as a `Cow` ("clone on write"), a
value that is either borrowed or owned. Here it is given an owned `Vec`, so the frame can be
stored and reused (`Frame<'static>` — it borrows nothing).
More: [Smart pointers](../../rust-primer.md#smart-pointers), [Lifetimes](../../rust-primer.md#lifetimes)

## Types

### `const NQ_SAMPLE_FACTOR`
[src/io/gif_export.rs:19](../../../src/io/gif_export.rs#L19)

```rust
const NQ_SAMPLE_FACTOR: i32 = 10; // 1..=30; 10 = quality/speed sweet spot.
```

NeuQuant's sampling factor, allowed range 1..=30. Lower looks at more pixels (better palette,
slower). The comment calls 10 the quality/speed sweet spot.

## Free functions

### `export_to`
[src/io/gif_export.rs:24](../../../src/io/gif_export.rs#L24)

```rust
pub fn export_to(project: &Project, path: &PathBuf, order: &[usize], cams: Option<&[Camera]>) -> Result<()>
```

**What it does.** Writes the timeline frames listed in `order` as one infinitely repeating GIF
at `path`.

**Parameters.**
- `project` — the document to export (a clone, when called from the worker thread).
- `path` — output file.
- `order` — one timeline frame index per output frame (from `frame_order`).
- `cams` — `Some(list)` when the camera move is stretched over loops: the camera for each
  output frame (same length as `order`); `None` otherwise.

**Returns.** `Ok(())`, or an error with context (file creation, encoder init, writing a frame).

**How it works.**
1. Creates the file and a GIF encoder at the camera frame size (`output_size`), set to repeat
   forever.
2. Frame delay in centiseconds (GIF's time unit): `round(100 / fps)`, at least 1. At 24 fps
   that is 4 cs = 40 ms (GIF cannot express 41.67 ms exactly).
3. `left = frame_order::uses(order)` counts how often each frame is still needed.
4. For each output frame `i` showing timeline frame `f`: decrements its remaining count;
   reuses a kept encoded frame if there is one (only when `cams` is `None` — under a
   per-frame camera every output frame is its own picture); otherwise flattens with
   `flatten_through` and calls `encode_frame`.
5. Writes the frame; if `f` is needed again later, keeps it in the `kept` map.
6. Logs and returns `Ok(())`.

**Used by:** [app.rs](../app.md) (`start_export`); test in [webp_export.rs](webp_export.md).

### `encode_frame`
[src/io/gif_export.rs:62](../../../src/io/gif_export.rs#L62)

```rust
fn encode_frame(rgba: &[u8], w: u16, h: u16, delay_cs: u16) -> Frame<'static>
```

**What it does.** Turns one flat RGBA image into a GIF frame with its own 256-colour palette.

**How it works.**
1. Trains NeuQuant on the pixels and takes its palette (`256 × [r, g, b]`).
2. For each pixel: if alpha < 8, writes index 0 and remembers that the frame has transparency;
   otherwise looks up the nearest palette index. Index 0 is reserved for transparency, so a
   colour that maps to 0 is bumped to 1 (the nearest-looking neighbour slot, not an exact
   match).
3. Builds a `gif::Frame` with size, delay, palette and the index buffer. If any pixel was
   transparent, sets `transparent = Some(0)` and disposal `Background` (clear the area before
   the next frame, so transparent parts don't show the previous frame).

**Example.** The palette idea in JS terms:

```js
// palette: 256 [r,g,b] entries; each pixel stores just its palette index (1 byte)
const indices = pixels.map(([r, g, b, a]) => a < 8 ? 0 : Math.max(1, nearest(palette, r, g, b)));
```

## Tests

Run with `cargo test io::gif_export`.

| Test | What it checks |
|---|---|
| [`a_gif_is_the_camera_s_size`](../../../src/io/gif_export.rs#L105) | The GIF header's size is the canvas size, or the camera's own size when it has one. |

**Test helpers:**

### `mod camera_size_tests`

Run with `cargo test io::gif_export::camera_size_tests` · 1 tests

| Test | What it checks |
|---|---|
| [`a_gif_is_the_camera_s_size`](../../../src/io/gif_export.rs#L105) | A gif is the camera s size. |

**Test helpers** (used only by the tests above):

- [`size`](../../../src/io/gif_export.rs#L99) — The logical screen size a GIF's header declares.
