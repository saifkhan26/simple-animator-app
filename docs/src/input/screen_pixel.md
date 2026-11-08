# `src/input/screen_pixel.rs` — read the OS cursor position and grab screen pixels

> **Source:** [src/input/screen_pixel.rs](../../../src/input/screen_pixel.rs) · **Module:** `crate::input::screen_pixel` · **Size:** 152 lines · Line links as of commit `67a7cb9`

## What this file is for

Two small Windows helpers that look *outside* the app window:

1. **Where is the mouse cursor on the whole desktop?** — `cursor_pos()` asks
   Windows (`GetCursorPos`) for the cursor position in *physical screen pixels*
   (real device pixels, not DPI-scaled "points"), across all monitors.
2. **What colours are on screen around it?** — `capture_region()` copies a small
   square of the desktop (for example 25×25 pixels) centred on a point, into an
   RGBA byte buffer.

Together they power the **screen colour picker** (an eyedropper that can sample any
pixel on your monitor, not just the canvas): the centre pixel of the captured square
is the picked colour, and the whole square is shown magnified as the *zoom loupe*.
Because the grab uses the `CAPTUREBLT` flag, it reads the final composited screen —
so when Animator's window backdrop is transparent, it sees the app *behind* Animator.
Using global screen coordinates means there is no window / DPI maths to get wrong.

`cursor_pos()` is also used by the pen-tablet code as the "ground truth" the pen
mapping is checked against (see [tablet.rs](tablet.md)).

On non-Windows systems every function here is a stub that returns `None` (or black),
like the pen backend.

The web equivalent of `capture_region` would be the
[EyeDropper API](https://developer.mozilla.org/docs/Web/API/EyeDropper_API) or
`getDisplayMedia()` + drawing the frame into a canvas and calling `getImageData`.

## How it connects

- **Used by:**
  - [ui/shell.rs](../ui/shell.md) — `screen_pick_live` captures a region around the
    cursor every frame while the screen picker is active (colour + loupe texture);
    `display_diagnostics` shows the raw cursor position in the tablet readout.
  - [tablet.rs](tablet.md) — `Wintab::poll` reads `cursor_pos()` right after
    draining pen packets, to learn the pen-to-screen mapping (`CursorFit`).
  - [pen_record.rs](pen_record.md) — `PenRecord::frame` logs the cursor position
    in each row of a pen recording.
- **Uses:** nothing else in the crate; Windows GDI and `GetCursorPos` through the
  `windows` crate.

## Rust concepts in this file

**Two versions of the same function, chosen at compile time.** `#[cfg(...)]` keeps or
drops an item when compiling. This file defines `ScreenRegion`, `cursor_pos` and
`capture_region` twice: once for Windows, once for everything else. Only one copy
exists in any given build, so callers never need their own `#[cfg]`.
```rust
#[cfg(target_os = "windows")]
pub fn cursor_pos() -> Option<(i32, i32)> { /* real Windows call */ }

#[cfg(not(target_os = "windows"))]
pub fn cursor_pos() -> Option<(i32, i32)> { None }
```
```js
// Closest JS idea: a build-time define that a bundler strips
export const cursorPos = process.platform === "win32" ? realCursorPos : () => null;
```
More: [attributes and cfg](../../rust-primer.md#attributes-and-cfg)

**`unsafe` + "out-parameters".** Many Windows functions write their answer into a
variable you pass by mutable reference, and return only success/failure:
```rust
let mut p = POINT::default();          // { x: 0, y: 0 }
unsafe { GetCursorPos(&mut p).ok()? }; // Windows fills p in
Some((p.x, p.y))
```
`&mut p` lends `p` to the function so it may change it. `.ok()?` turns the Windows
`Result` into an `Option` and returns `None` from `cursor_pos` early on failure.
More: [unsafe and FFI](../../rust-primer.md#unsafe-and-ffi),
[Option and Result](../../rust-primer.md#option-and-result)

**Manual cleanup of OS handles.** GDI hands out *handles* (numbers that name OS
objects: a device context, a bitmap). The OS does not garbage-collect them, so the
code releases each one by hand (`DeleteObject`, `DeleteDC`, `ReleaseDC`) on every
path, including the early `return None`s. It is like having to call
`URL.revokeObjectURL` yourself, but for every object.

**Walking a byte buffer in fixed-size pieces.** `chunks_exact_mut(4)` gives one
mutable 4-byte slice per pixel; `swap(0, 2)` exchanges the blue and red bytes.
```rust
for px in buf.chunks_exact_mut(4) {
    px.swap(0, 2);   // BGRX -> RGBX
    px[3] = 255;     // force opaque
}
```
```js
for (let i = 0; i < buf.length; i += 4) {
  [buf[i], buf[i + 2]] = [buf[i + 2], buf[i]];
  buf[i + 3] = 255;
}
```
More: [strings and slices](../../rust-primer.md#strings-and-slices),
[iterators](../../rust-primer.md#iterators)

## Windows versions

## Other-platform stubs

## Types

### `struct ScreenRegion`
[src/input/screen_pixel.rs:15](../../../src/input/screen_pixel.rs#L15)

Same three fields, so code that uses the type compiles everywhere; nothing on these
platforms ever creates one.

| Field | Type | Meaning |
|---|---|---|
| `w` | `u32` | Width in pixels. |
| `h` | `u32` | Height in pixels. |
| `buf` | `Vec<u8>` | Pixels, row by row from the top, 4 bytes each (R, G, B, A), `w * h * 4` bytes in total. Alpha is always 255. |

### `struct ScreenRegion`
[src/input/screen_pixel.rs:130](../../../src/input/screen_pixel.rs#L130)

Same three fields, so code that uses the type compiles everywhere; nothing on these
platforms ever creates one.

| Field | Type | Meaning |
|---|---|---|
| `w` | `u32` | Width in pixels. |
| `h` | `u32` | Height in pixels. |
| `buf` | `Vec<u8>` | Pixels, row by row from the top, 4 bytes each (R, G, B, A), `w * h * 4` bytes in total. Alpha is always 255. |

## `impl ScreenRegion`

### `ScreenRegion::center`
[src/input/screen_pixel.rs:24](../../../src/input/screen_pixel.rs#L24)

```rust
pub fn center(&self) -> [u8; 4]
```

**What it does.** Returns the colour of the middle pixel (the one under the cursor)
as `[r, g, b, 255]`. It finds the pixel at `(w/2, h/2)` and reads its byte offset
`(cy * w + cx) * 4`.

**Used by:** `screen_pick_live` in [ui/shell.rs](../ui/shell.md) (the picked colour).

## Free functions

### `cursor_pos`
[src/input/screen_pixel.rs:34](../../../src/input/screen_pixel.rs#L34)

```rust
pub fn cursor_pos() -> Option<(i32, i32)>
```

**What it does.** The OS cursor's position in physical screen pixels on the virtual
desktop (all monitors together; can be negative for monitors left of / above the
primary one).

**Returns.** `Some((x, y))`, or `None` if `GetCursorPos` fails.

**Used by:** `screen_pick_live` and `display_diagnostics` in
[ui/shell.rs](../ui/shell.md); `Wintab::poll` in [tablet.rs](tablet.md);
`PenRecord::frame` in [pen_record.rs](pen_record.md).

### `capture_region`
[src/input/screen_pixel.rs:44](../../../src/input/screen_pixel.rs#L44)

```rust
pub fn capture_region(cx: i32, cy: i32, half: i32) -> Option<ScreenRegion>
```

**What it does.** Copies a square of the desktop, `2*half + 1` pixels on each side,
centred on `(cx, cy)`, and returns it as RGBA bytes.

**Parameters.**
- `cx`, `cy` — centre in physical screen pixels (normally from `cursor_pos`).
- `half` — how many pixels to take on each side of the centre (`12` gives 25×25).

**Returns.** `Some(ScreenRegion)`, or `None` if the size is not positive or any GDI
step fails.

**How it works.** (GDI = *Graphics Device Interface*, Windows' classic 2D drawing
API. A *DC*, device context, is a handle to something you can draw on or copy from.)
1. `side = half * 2 + 1`; give up if it is not positive.
2. `GetDC(null)` gets a DC for the whole screen.
3. `CreateCompatibleDC` makes an in-memory DC, and `CreateCompatibleBitmap` a
   `side × side` bitmap for it; `SelectObject` attaches the bitmap (remembering the
   old object). Each failure releases what was already acquired and returns `None`.
4. `BitBlt` ("bit block transfer", a rectangle copy) copies the screen square at
   `(cx - half, cy - half)` into the memory bitmap. The raster operation is
   `SRCCOPY | CAPTUREBLT`: plain copy, *including* layered / transparent windows, so
   it captures the real composited image.
5. Fills a `BITMAPINFO` header asking for 32 bits per pixel, uncompressed
   (`BI_RGB`), with a **negative height** so rows come out top-down.
6. `GetDIBits` copies the bitmap's pixels into a `Vec<u8>` of `side * side * 4`
   bytes.
7. Restores the old object and frees the bitmap, the memory DC and the screen DC.
8. If the copy or read failed, returns `None`.
9. The bytes are in BGRX order (blue, green, red, unused); each pixel's bytes 0 and 2
   are swapped and byte 3 set to 255, giving opaque RGBA.

**Used by:** `screen_pick_live` in [ui/shell.rs](../ui/shell.md).

## `impl ScreenRegion`

### `ScreenRegion::center`
[src/input/screen_pixel.rs:138](../../../src/input/screen_pixel.rs#L138)

```rust
pub fn center(&self) -> [u8; 4]
```

Always opaque black `[0, 0, 0, 255]`.

## Free functions

### `cursor_pos`
[src/input/screen_pixel.rs:144](../../../src/input/screen_pixel.rs#L144)

```rust
pub fn cursor_pos() -> Option<(i32, i32)>
```

Always `None`.

### `capture_region`
[src/input/screen_pixel.rs:149](../../../src/input/screen_pixel.rs#L149)

```rust
pub fn capture_region(_cx: i32, _cy: i32, _half: i32) -> Option<ScreenRegion>
```

Always `None`. The leading `_` on parameter names tells the compiler they are
unused on purpose (no warning).
