# `src/input/mod.rs` — the input folder's table of contents

> **Source:** [src/input/mod.rs](../../../src/input/mod.rs) · **Module:** `crate::input` · **Size:** 14 lines · Line links as of commit `67a7cb9`

## What this file is for

`mod.rs` is the "index file" of the `src/input/` folder. It holds no code of its own: it
only declares the eight files inside the folder as sub-modules, so the rest of the app can
reach them as `crate::input::shortcuts`, `crate::input::tablet` and so on. Think of it as an
`index.js` that re-exports the files in its folder.

The folder as a whole is everything about **how the user's hands reach the app**: the
keyboard (shortcuts), the mouse buttons and wheel, the pen tablet, raw Windows messages that
egui would otherwise hide, and a couple of diagnostic helpers. The top comment (`//!`) still
names the original plan — "Phase A: mouse only", "Phase D: tablet injects pressure and tilt" —
which is how the pointer and tablet files came to exist.

| Sub-module | Declared at | Source file | What it does | Docs page |
|---|---|---|---|---|
| `button_drag` | [mod.rs:6](../../../src/input/mod.rs#L6) | [button_drag.rs](../../../src/input/button_drag.rs) | What a middle- or right-button (or pen barrel button) drag does on the canvas, plus the maths for "steps" and the swatch pie | [button_drag.md](button_drag.md) |
| `clipboard_keys` | [mod.rs:7](../../../src/input/mod.rs#L7) | [clipboard_keys.rs](../../../src/input/clipboard_keys.rs) | Puts back the Ctrl+C / Ctrl+X / Ctrl+V key presses that egui turns into clipboard events, so they can be shortcuts | [clipboard_keys.md](clipboard_keys.md) |
| `msg_hook` | [mod.rs:8](../../../src/input/mod.rs#L8) | [msg_hook.rs](../../../src/input/msg_hook.rs) | The single hook that sees every raw Windows message before egui does | [msg_hook.md](msg_hook.md) |
| `pen_record` | [mod.rs:9](../../../src/input/mod.rs#L9) | [pen_record.rs](../../../src/input/pen_record.rs) | Records a few seconds of pen / cursor / pointer positions to a file, to debug tablet problems | [pen_record.md](pen_record.md) |
| `pointer` | [mod.rs:10](../../../src/input/mod.rs#L10) | [pointer.rs](../../../src/input/pointer.rs) | One pointer sample: position, pressure, tilt, time — the same shape for mouse and pen | [pointer.md](pointer.md) |
| `screen_pixel` | [mod.rs:11](../../../src/input/mod.rs#L11) | [screen_pixel.rs](../../../src/input/screen_pixel.rs) | Grabs a small square of the screen around the cursor, for the screen colour picker | [screen_pixel.md](screen_pixel.md) |
| `shortcuts` | [mod.rs:12](../../../src/input/mod.rs#L12) | [shortcuts.rs](../../../src/input/shortcuts.rs) | The user's key bindings: every action, its key combo, the mouse-wheel map, and the `shortcuts.toml` file | [shortcuts.md](shortcuts.md) |
| `tablet` | [mod.rs:13](../../../src/input/mod.rs#L13) | [tablet.rs](../../../src/input/tablet.rs) | Pen tablet input through Wintab: every pen packet with pressure and tilt | [tablet.md](tablet.md) |

## How it connects

- **Used by:** [main.rs](../main.md) declares it with `mod input;` and reaches
  `input::shortcuts::config_dir` and `input::msg_hook::install` directly. Most of the app then
  uses the sub-modules by full path — for example [app.rs](../app.md),
  [app/button_drag.rs](../app/button_drag.md), [ui/shell.rs](../ui/shell.md) and
  [ui/tracks.rs](../ui/tracks.md) all `use crate::input::…`.
- **Uses:** nothing itself; it only lists the eight files above.

```text
main.rs ── mod input; ──► input/mod.rs ──► pub mod button_drag;   (button_drag.rs)
                                       ├──► pub mod clipboard_keys; (clipboard_keys.rs)
                                       ├──► pub mod msg_hook;       (msg_hook.rs)
                                       ├──► pub mod pen_record;     …
                                       ├──► pub mod pointer;
                                       ├──► pub mod screen_pixel;
                                       ├──► pub mod shortcuts;
                                       └──► pub mod tablet;
```

## Rust concepts in this file

**`mod.rs` and `pub mod`.** In Rust a folder becomes a module when it has a `mod.rs` file, and
each file inside only exists for the compiler once some parent says `mod name;`. Adding `pub`
makes it reachable from outside the folder.
```rust
pub mod shortcuts; // compiles src/input/shortcuts.rs as crate::input::shortcuts
pub mod tablet;    // compiles src/input/tablet.rs   as crate::input::tablet
```
```js
// input/index.js
export * as shortcuts from "./shortcuts.js";
export * as tablet from "./tablet.js";
```
Unlike JavaScript, a `.rs` file that no `mod` line names is simply not part of the program.
More: [Modules and visibility](../../rust-primer.md#modules-and-visibility)

**`//!` comments.** Lines starting with `//!` document the module they sit in (here, the whole
`input` folder), while `///` documents the item right below it.
More: [Reading a Rust file](../../rust-primer.md#reading-a-rust-file)
