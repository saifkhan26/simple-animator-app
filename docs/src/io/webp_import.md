# `src/io/webp_import.rs` — animated WebP import

> **Source:** [src/io/webp_import.rs](../../../src/io/webp_import.rs) · **Module:** `crate::io::webp_import` · **Size:** 98 lines · Line links as of commit `67a7cb9`

## What this file is for

Decodes a WebP file (animated or still) into one **full-size canvas per frame**. Animated WebP frames are
often small rectangles blended over earlier frames; the decoder composites them the way a browser does, so
each canvas is the complete picture at that frame. A still WebP comes in as one frame. The frames then go
through the same range dialog as GIF and video imports.

## How it connects

- **Used by:** [app.rs](../app.md) (`open_webp_import` → `open_frames_import`, run on a worker thread).
- **Uses:** the `image-webp` crate, [doc/canvas.rs](../doc/canvas.md).

## Rust concepts in this file

**Looping until a specific error.**
```rust
loop {
    match decoder.read_frame(&mut buf) {
        Ok(_) => frames.push(to_canvas(&buf, w, h, alpha)),
        Err(DecodingError::NoMoreFrames) => break,      // the normal end
        Err(e) => return Err(e).context("decoding WebP frame"),
    }
}
```
"No more frames" is reported as an error variant; matching it specifically ends the loop.
More: [Enums and match](../../rust-primer.md#enums-and-match)

**`flat_map`.** `buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255])` turns RGB into RGBA by emitting
four bytes per three — like JS `flatMap`. More: [Iterators](../../rust-primer.md#iterators)

## Free functions

### `decode_all`
[src/io/webp_import.rs:17](../../../src/io/webp_import.rs#L17)

```rust
pub fn decode_all(path: &Path) -> Result<Vec<Canvas>>
```

Opens the file, and either decodes the single still image, or — for an animation — sets disposed areas to
clear to transparent (what browsers show) and reads every frame until `NoMoreFrames`, converting each to a
canvas.

### `to_canvas`
[src/io/webp_import.rs:46](../../../src/io/webp_import.rs#L46)

```rust
fn to_canvas(buf: &[u8], w: u32, h: u32, alpha: bool) -> Canvas
```

Makes a canvas from a decoded frame buffer: as is when it has alpha, or RGB expanded to opaque RGBA when
the file has none.

## Tests

### `mod tests`

Run with `cargo test io::webp_import::tests` · 2 tests

| Test | What it checks |
|---|---|
| [`an_exported_webp_comes_back_frame_for_frame`](../../../src/io/webp_import.rs#L61) | An exported webp comes back frame for frame. |
| [`a_still_webp_is_one_frame`](../../../src/io/webp_import.rs#L84) | A still webp is one frame. |
