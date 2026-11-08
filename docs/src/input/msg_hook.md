# `src/input/msg_hook.rs` — peeking at raw Windows messages before egui does

> **Source:** [src/input/msg_hook.rs](../../../src/input/msg_hook.rs) · **Module:** `crate::input::msg_hook` · **Size:** 23 lines · Line links as of commit `36405ad`

## What this file is for

On Windows, every key press, mouse move and system notice reaches a program as a **window
message**: a small C struct called `MSG` that holds a message number (for example
`WM_KEYDOWN`) and two number arguments. The app never reads these itself. The windowing library
**winit** reads them, and **egui-winit** turns them into egui events (`Event::Key`,
`Event::Copy`, …) — the way a browser turns OS input into DOM events before your JavaScript
ever sees it.

Sometimes that translation throws information away, so two parts of the app need to see the
raw message first:

- [clipboard_keys.rs](clipboard_keys.md) — egui-winit swallows the Ctrl+C / Ctrl+X / Ctrl+V key
  presses and turns them into clipboard events, so shortcuts never see them.
- [tablet.rs](tablet.md) — the pen tablet driver sends its own "something changed" messages.

winit lets a program register **one** message hook (`with_msg_hook`). This file is that one
hook: it calls both listeners for every message, then always lets the message carry on
normally. On other operating systems it does nothing.

It is a bit like adding a capture-phase listener on `window` in the browser that only *looks*
at events and never calls `stopPropagation()`.

## How it connects

- **Used by:** [main.rs](../main.md) passes `input::msg_hook::install` to eframe as its
  `event_loop_builder`, so it runs once while the event loop is being built.
- **Uses:** [clipboard_keys.rs](clipboard_keys.md) (`on_msg`) and [tablet.rs](tablet.md)
  (`on_msg`), plus winit's Windows-only `EventLoopBuilderExtWindows::with_msg_hook` and the
  `windows` crate's `MSG` type.

```text
Windows ──MSG──► winit event loop ──► msg hook (this file)
                                       ├─► clipboard_keys::on_msg(msg)   (record C/X/V/Insert/Delete downs)
                                       ├─► tablet::on_msg(msg)           (driver change notices)
                                       └─► return false  ("don't eat it")
                                 └──► egui-winit ──► egui events ──► the app
```

## Rust concepts in this file

**Compiling different code per operating system with `#[cfg]`.** The same function name is
written twice; the compiler keeps only the copy whose condition is true for the platform it is
building for.
```rust
#[cfg(target_os = "windows")]
pub fn install(builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>) { /* real work */ }

#[cfg(not(target_os = "windows"))]
pub fn install(_builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {}
```
```js
// Closest JS idea: a build-time flag that strips one branch.
export const install = process.platform === "win32" ? realInstall : () => {};
```
The leading `_` in `_builder` tells the compiler "unused on purpose, don't warn".
More: [Attributes and cfg](../../rust-primer.md#attributes-and-cfg)

**Closures as callbacks.** `|msg| { … }` is an anonymous function, like `(msg) => { … }`.
winit stores it and calls it for every message.
```rust
builder.with_msg_hook(|msg| {
    // …
    false
});
```
The last expression without a `;` (`false`) is the return value.
More: [Closures](../../rust-primer.md#closures)

**`unsafe` and raw pointers.** winit hands the hook a *raw pointer* (`*const c_void`, an
address with no type and no guarantee it is valid). Turning it into a normal Rust reference is
something the compiler cannot check, so it must be wrapped in `unsafe { }`. The `// SAFETY:`
comment above it records why it is fine: winit promises the pointer points at the `MSG` it just
read.
```rust
let msg = unsafe { &*(msg as *const MSG) };
//                   │   └── "treat this address as pointing at a MSG"
//                   └────── "*" follows the pointer, "&" borrows what is there
```
```js
// No JS equivalent: JS never hands you raw memory addresses.
// Closest feeling: trusting that `event.detail` really has the shape you expect.
```
More: [unsafe and FFI](../../rust-primer.md#unsafe-and-ffi)

**`use` inside a function.** The `use` lines sit inside `install` rather than at the top of the
file, so the Windows-only names are only imported in the Windows-only copy.
More: [Reading a Rust file](../../rust-primer.md#reading-a-rust-file)

## Free functions

### `install`
[src/input/msg_hook.rs:7](../../../src/input/msg_hook.rs#L7)

```rust
pub fn install(builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>)
```

**What it does.** Registers the app's single winit message hook. For every raw Windows message
the event loop reads, the hook passes it to the clipboard-key recorder and to the tablet code,
then returns `false` so the message is **not** consumed — winit and egui-winit still handle it
exactly as before.

**Parameters.**
- `builder` — eframe's event-loop builder, borrowed mutably (`&mut`) so the hook can be added
  to it before the event loop is created.

**How it works.**
1. Imports the `MSG` struct (from the `windows` crate) and winit's Windows extension trait
   `EventLoopBuilderExtWindows`, which is what adds the `with_msg_hook` method to the builder.
2. Calls `builder.with_msg_hook(...)` with a closure.
3. Inside the closure, casts the raw pointer winit provides into a `&MSG` (the `unsafe` step
   explained above).
4. Calls [`clipboard_keys::on_msg`](clipboard_keys.md#on_msg) and
   `tablet::on_msg` (see [tablet.md](tablet.md)) with that message.
5. Returns `false`: "I did not handle this; keep going".

**Example.** The shape of the idea in JavaScript:
```js
window.addEventListener("keydown", (e) => {
  clipboardKeys.onMsg(e);
  tablet.onMsg(e);
  // never e.preventDefault() / stopPropagation(): everyone else still sees it
}, { capture: true });
```

**Used by:** [main.rs](../main.md) — `event_loop_builder: Some(Box::new(input::msg_hook::install))`.

### `install`
[src/input/msg_hook.rs:22](../../../src/input/msg_hook.rs#L22)

```rust
pub fn install(_builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>)
```

**What it does.** Does nothing. It exists so [main.rs](../main.md) can pass `install` to eframe
on every platform without its own `#[cfg]`; there are no raw Windows messages to watch
elsewhere.

**Used by:** [main.rs](../main.md) on non-Windows builds.
