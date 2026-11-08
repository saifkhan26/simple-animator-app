# `src/io/png_save.rs` — saving one frame as PNG, or copying it

> **Source:** [src/io/png_save.rs](../../../src/io/png_save.rs) · **Module:** `crate::io::png_save` · **Size:** 70 lines · Line links as of commit `67a7cb9`

## What this file is for

Two small output paths for a single flattened frame: save it as a PNG file through a save dialog, or put it
on the system clipboard as an image to paste into another app ("Copy frame").

## How it connects

- **Used by:** [app.rs](../app.md) (`copy_frame`, export of the current frame).
- **Uses:** [doc/canvas.rs](../doc/canvas.md), the `image` crate, `arboard`, `rfd`.

## Rust concepts in this file

**`Cow` from `canvas.pixels()`.** `pixels()` returns a borrowed-or-owned view (`Cow<[u8]>`);
`.into_owned()` gets an owned `Vec<u8>` (copying only if it was borrowed). More:
[Smart pointers](../../rust-primer.md#smart-pointers)

**`?` with `.context(...)`.** Errors from creating the image buffer or saving get a description attached on
the way up. More: [Option and Result](../../rust-primer.md#option-and-result)

## Free functions

### `save_dialog`
[src/io/png_save.rs:9](../../../src/io/png_save.rs#L9)

```rust
pub fn save_dialog(canvas: &Canvas) -> Result<()>
```

Shows a save dialog (default name `frame.png`) and writes `canvas` as a PNG there. Cancelling does nothing.

### `copy_to_clipboard`
[src/io/png_save.rs:28](../../../src/io/png_save.rs#L28)

```rust
pub fn copy_to_clipboard(canvas: &Canvas) -> Result<()>
```

Puts `canvas` on the system clipboard as an image.

### `clipboard_pixels`
[src/io/png_save.rs:42](../../../src/io/png_save.rs#L42)

```rust
fn clipboard_pixels(canvas: &Canvas) -> Vec<u8>
```

The pixels as the clipboard gets them: the canvas's own, but fully transparent pixels made white
underneath (alpha stays 0). Apps that read alpha see no difference; apps that ignore alpha show white paper
instead of black, which would hide dark lines.

## Tests

### `mod tests`

Run with `cargo test io::png_save::tests` · 1 tests

| Test | What it checks |
|---|---|
| [`clipboard_pixels_put_white_under_clear_pixels_only`](../../../src/io/png_save.rs#L59) | Clipboard pixels put white under clear pixels only. |
