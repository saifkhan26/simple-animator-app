# `src/io/gif_import.rs` — decode an animated GIF into one canvas per frame

> **Source:** [src/io/gif_import.rs](../../../src/io/gif_import.rs) · **Module:** `crate::io::gif_import` · **Size:** 97 lines · Line links as of commit `67a7cb9`

## What this file is for

*File → Import GIF…* reads an animated GIF and puts each of its frames on its own timeline
frame of a new layer. This file does the decoding.

GIF frames are often **not** full pictures: to save space, a frame may only store a small
rectangle that changed, plus a *disposal method* saying what to do with that rectangle before
the next frame is drawn (leave it, clear it to transparent, or restore what was there before).
So this file keeps a running full-size RGBA buffer, draws each frame's rectangle onto it,
takes a full snapshot as a `Canvas`, then applies the disposal rule — giving one complete
picture per frame, the way a browser would show it.

It is a pure function: given a path, return all frames. [app.rs](../app.md) runs it on a
worker thread and opens a range dialog when it finishes.

## How it connects

- **Used by:** [app.rs](../app.md) — `open_gif_import` passes `decode_all` to
  `open_frames_import`, which runs it on a worker thread.
- **Uses:** [doc/canvas.rs](../doc/canvas.md) (`Canvas::from_pixels`); external crates `gif`
  (decoder) and `anyhow`.

## Rust concepts in this file

**`while let` — loop while a value is present.** `read_next_frame()` returns
`Result<Option<Frame>>`; `?` handles the error, then `while let Some(frame)` keeps looping
until it returns `None` (no more frames).
```rust
while let Some(frame) = decoder.read_next_frame().context("decoding GIF frame")? {
    // ...
}
```
```js
let frame;
while ((frame = decoder.readNextFrame()) != null) { /* ... */ }
```
More: [Enums and match](../../rust-primer.md#enums-and-match)

**`clone()` makes a real copy.** `let prev = buf.clone();` copies the whole buffer so it can
be restored later. Plain `let prev = buf;` would *move* `buf` (it could no longer be used).
More: [Ownership and borrowing](../../rust-primer.md#ownership-and-borrowing)

**`usize` for indexes.** Sizes are converted to `usize` (Rust's index type) up front with
`as usize`, because only `usize` can index a `Vec`.
More: [Numbers and casts](../../rust-primer.md#numbers-and-casts)

## Free functions

### `decode_all`
[src/io/gif_import.rs:15](../../../src/io/gif_import.rs#L15)

```rust
pub fn decode_all(path: &Path) -> Result<Vec<Canvas>>
```

**What it does.** Opens the GIF at `path` and returns one full-size canvas (the GIF's own
"logical screen" size) per frame.

**Returns.** All frames in order, or an error (open failed, bad header, bad frame data).

**How it works.**
1. Opens the file and asks the `gif` decoder for RGBA output.
2. Makes a zeroed `gw × gh` RGBA buffer.
3. For each frame:
   - copies the buffer to `prev` (needed if this frame's disposal is "previous");
   - draws the frame's rectangle (`left`, `top`, `width`, `height`) onto the buffer,
     clipped to the screen; a transparent source pixel (alpha 0) leaves what is underneath;
   - pushes a snapshot of the whole buffer as a `Canvas`;
   - applies disposal: `Background` clears the rectangle to transparent; `Previous`
     restores `prev`; anything else (`Keep` / `Any`) leaves the buffer as it is.
4. Returns the list.

**Used by:** [app.rs](../app.md) (`open_gif_import`).

### `to_canvas`
[src/io/gif_import.rs:94](../../../src/io/gif_import.rs#L94)

```rust
fn to_canvas(buf: &[u8], gw: u32, gh: u32) -> Canvas
```

**What it does.** Copies the running buffer into a new canvas at the GIF's own resolution.
The doc comment notes that placing and scaling it on the project is left to the layer
transform.
