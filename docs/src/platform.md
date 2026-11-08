# `src/platform.rs` — rounded corners and a border for the frameless window (Windows)

> **Source:** [src/platform.rs](../../src/platform.rs) · **Module:** `crate::platform` · **Size:** 37 lines · Line links as of commit `36405ad`

## What this file is for

Animator's main window is *frameless*: it has no title bar and no minimise /
maximise / close buttons drawn by the operating system (the app draws its own
menu strip instead — see [main.rs](main.md), where the window is created with
`.with_decorations(false)`). On Windows 11 a frameless window loses the normal
rounded corners and the thin window border, so it looks like a sharp-edged
rectangle pasted on the screen.

This tiny file puts those two things back. It asks **DWM** (the *Desktop Window
Manager*, the part of Windows that composites every window onto the screen) to
draw the system's default rounded corners and default border colour around the
app window. It is the only thing in the module.

The whole module only exists on Windows: [main.rs](main.md) declares it as
`#[cfg(target_os = "windows")] mod platform;`, so on Linux/macOS the file is not
even compiled. A web analogy: it is like setting `border-radius` and `border` on
the browser window itself — something a page normally cannot do, so it has to
ask the host (here, Windows) directly.

## How it connects

- **Used by:** [app.rs](app.md) — on the first frame, `AppState::update` gets the
  native window handle from eframe and calls `crate::platform::round_window(...)`
  once (it remembers it did so in a `window_styled` flag).
- **Uses:** nothing else in the crate. It calls the Windows API through the
  [`windows`](https://crates.io/crates/windows) crate (`DwmSetWindowAttribute`).

## Rust concepts in this file

**`unsafe` blocks for calling the operating system.** Rust cannot check what a C /
Windows function does with the memory you hand it, so such calls must sit inside
an `unsafe { … }` block. The block is a promise from the programmer: "I checked the
rules by hand here." Outside the block the code is normal, checked Rust.
```rust
unsafe {
    let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, /* pointer */, /* size */);
}
```
JavaScript has no equivalent — everything in JS is memory-safe and there is no way
to call raw OS functions. The closest idea is a Node native addon: the call leaves
the safe world. More: [unsafe and FFI](../rust-primer.md#unsafe-and-ffi)

**Raw pointers and `as` casts.** The Windows function takes "a pointer to some bytes,
plus how many bytes". The code makes that pointer from a normal variable:
```rust
let color = DWMWA_COLOR_DEFAULT;                     // a u32 on the stack
let ptr = &color as *const u32 as *const core::ffi::c_void;
let len = std::mem::size_of::<u32>() as u32;         // 4
```
`&color` is a safe reference; `as *const u32` turns it into a *raw pointer* (just
an address, no safety guarantees); `as *const c_void` forgets the type, like C's
`void*`. `std::mem::size_of::<T>()` is the size of a type in bytes — a little like
`Uint32Array.BYTES_PER_ELEMENT` in JS. More:
[numbers and casts](../rust-primer.md#numbers-and-casts),
[unsafe and FFI](../rust-primer.md#unsafe-and-ffi)

**`let _ = …` to ignore a result on purpose.** `DwmSetWindowAttribute` returns a
`Result` (success or an error code). Rust warns if you silently drop a `Result`;
assigning it to `_` says "I know, and I do not care". Here a failure (for example on
Windows 10, which does not know these attributes) simply leaves the window square.
```rust
let _ = DwmSetWindowAttribute(/* … */);
```
```js
try { setWindowAttribute(/* … */); } catch {}  // deliberately ignored
```
More: [Option and Result](../rust-primer.md#option-and-result)

**Tuple structs as type-safe wrappers.** `HWND(hwnd)` wraps a plain number (`isize`)
in the `HWND` type ("handle to a window"), so a window handle cannot be confused
with any other integer.
```rust
let hwnd = HWND(hwnd);   // isize -> HWND
```
More: [structs](../rust-primer.md#structs)

## Types

### `const DWMWA_COLOR_DEFAULT`
[src/platform.rs:14](../../src/platform.rs#L14)

```rust
const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;
```

`u32` value `0xFFFF_FFFF`. A special "colour" that tells DWM *use the system's
default border colour* instead of a specific one. The `_` in the number is just a
digit separator for readability (like `1_000_000` in modern JS).

## Free functions

### `round_window`
[src/platform.rs:18](../../src/platform.rs#L18)

```rust
pub fn round_window(hwnd: isize)
```

**What it does.** Gives the window the default Windows 11 rounded corners and the
default window border. Safe to call more than once; if Windows refuses (for
example on versions before Windows 11), nothing happens and no error is reported.

**Parameters.**
- `hwnd` — the native window handle as a plain number. The caller gets it from
  eframe's `raw_window_handle` (`RawWindowHandle::Win32(w)` → `w.hwnd.get()`).

**How it works.**
1. Wraps the number in the Windows `HWND` type.
2. Calls `DwmSetWindowAttribute` with `DWMWA_WINDOW_CORNER_PREFERENCE` set to
   `DWMWCP_ROUND` (round corners), passing a pointer to that value and its size.
3. Calls `DwmSetWindowAttribute` again with `DWMWA_BORDER_COLOR` set to
   `DWMWA_COLOR_DEFAULT` (system default border colour).
4. Both results are ignored with `let _ =`.

**Used by:** `AppState::update` in [app.rs](app.md), once, on the first frame.
