# `src/io/fit.rs` — fit an image inside the canvas, centred, without stretching

> **Source:** [src/io/fit.rs](../../../src/io/fit.rs) · **Module:** `crate::io::fit` · **Size:** 40 lines · Line links as of commit `67a7cb9`

## What this file is for

This tiny file takes an imported picture and places it in a canvas-sized RGBA buffer the
way CSS `object-fit: contain` would — but with one difference: it only ever shrinks. A
picture bigger than the canvas is scaled down until it fits; a smaller one keeps its own size.
Either way it is centred, and the leftover border is transparent.

The file's header comment explains the reason: cells (the drawings on a layer, see
[glossary](../../glossary.md)) had to be project-sized because the compositor copied them
1:1, so imported media had to be "fitted" into that size first.

> **Note — not currently compiled.** [`src/io/mod.rs`](../../../src/io/mod.rs) has no
> `pub mod fit;` line, and nothing in `src/` mentions `fit_centered`, so this file is not part
> of the build at commit `4c83de8`. Rust only compiles files that a `mod` line reaches. Image
> import today keeps the image at its own size and places it with a layer transform instead
> (see [image_import.rs](image_import.md)).

## How it connects

- **Used by:** nothing at this commit (the module is not declared in [mod.rs](mod.md)).
- **Uses:** the external `image` crate (`RgbaImage`, `imageops::resize` with the Lanczos3 filter).

## Rust concepts in this file

**A file is only compiled if a `mod` line names it.** Unlike JS, where any file can be
`import`ed by path, Rust builds a tree of modules starting at `main.rs`; each `mod name;`
pulls in `name.rs`. A file no `mod` reaches is simply ignored.
```rust
// src/io/mod.rs
pub mod composite;   // compiles src/io/composite.rs
// (no `pub mod fit;` → src/io/fit.rs is never compiled)
```
More: [Modules and visibility](../../rust-primer.md#modules-and-visibility)

**`vec![value; n]` — a filled array.**
```rust
let mut buf = vec![0u8; (cw * ch * 4) as usize];
```
```js
const buf = new Uint8Array(cw * ch * 4); // zero-filled
```
More: [Macros](../../rust-primer.md#macros)

**Copying a slice range into another slice.**
```rust
buf[dst_off..dst_off + row_bytes].copy_from_slice(&src[src_off..src_off + row_bytes]);
```
```js
buf.set(src.subarray(srcOff, srcOff + rowBytes), dstOff);
```
More: [Strings and slices](../../rust-primer.md#strings-and-slices)

## Free functions

### `fit_centered`
[src/io/fit.rs:9](../../../src/io/fit.rs#L9)

```rust
pub fn fit_centered(img: &RgbaImage, cw: u32, ch: u32) -> Vec<u8>
```

**What it does.** Returns a `cw × ch` RGBA8 buffer with `img` scaled down to fit (if needed),
centred, and surrounded by transparent pixels.

**Parameters.**
- `img` — the decoded source image.
- `cw`, `ch` — canvas width and height.

**Returns.** `cw × ch × 4` bytes, row-major RGBA.

**How it works.**
1. An empty (0-pixel) image gives an all-transparent buffer.
2. Scale `s = min(cw / iw, ch / ih, 1.0)` — the largest scale that fits both ways, capped at 1
   so small images are never enlarged.
3. Target size `dw × dh` = image size × `s`, rounded, clamped to 1..canvas size.
4. If the size did not change the image is cloned; otherwise it is resized with the
   high-quality Lanczos3 filter.
5. Offsets `ox = (cw − dw) / 2`, `oy = (ch − dh) / 2` centre it; each row is copied into the
   right place in a zeroed buffer.

**Example.** A 4000×1000 photo into a 1920×1080 canvas: `s = min(0.48, 1.08, 1) = 0.48`,
so it becomes 1920×480, placed at `oy = (1080 − 480) / 2 = 300`.
