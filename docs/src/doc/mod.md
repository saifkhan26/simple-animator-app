# `src/doc/mod.rs` — The document model's table of contents

> **Source:** [src/doc/mod.rs](../../../src/doc/mod.rs) · **Module:** `crate::doc` · **Size:** 14 lines · Line links as of commit `67a7cb9`

## What this file is for

**`doc` here means "document model", not documentation.** The folder `src/doc/` holds the
*data* of an animation — what gets saved into a `.anim` file, undone, exported and sent to
Krita — with no UI code in it. Think of it as the "store" or "state shape" of a web app:
plain data plus the methods that keep it consistent, while the UI (`src/ui/`, `src/app.rs`)
reads it and asks it to change.

This file itself has no code. It is the folder's index: every `pub mod x;` line tells the
Rust compiler "there is a file `src/doc/x.rs`; make it available as `crate::doc::x`". Its
first lines are a `//!` doc comment describing the folder. That comment is older than the
code — it says only `canvas` is wired and talks of "Phase A / Phase B" plans with `frame` and
`xsheet` modules that do not exist; in fact all eight modules below are present and used.

### The document model at a glance

```text
Project                         (project.rs)   one open animation
├── width, height, fps                          canvas size and playback speed
├── cells: Vec<Arc<Canvas>>     (canvas.rs)    pool of drawings, addressed by CellId = index
├── layers: Vec<Layer>          (layer.rs)     bottom-to-top stack
│   └── exposures: Vec<Option<CellId>>         one slot per frame: Some(id) = key, None = hold
│       transform / transform_keys (transform.rs)  where the layer's cells sit, per frame
├── frame_count, current_frame, current_layer  timeline length + editing cursor
└── camera / camera_keys        (camera.rs)    what part of the canvas is exported
```

Groups ([groups.rs](groups.md)) and clipping masks ([clip.rs](clip.md)) add behaviour on top
of the same structures; [orient.rs](orient.md) is a small helper for mirroring and
quarter-turning pixels without resampling.

## How it connects

- **Used by:** [main.rs](../main.md) declares it with `mod doc;`, which is what makes
  `crate::doc::…` paths work everywhere else. Nearly every other file imports something from
  `crate::doc`.
- **Uses:** nothing — it only declares the submodules:

| Line | Submodule | Page | In one line |
|---|---|---|---|
| [6](../../../src/doc/mod.rs#L6) | `camera` | [camera.md](camera.md) | the camera (pan / zoom / roll) and its keyframes: which part of document space each exported frame shows |
| [7](../../../src/doc/mod.rs#L7) | `canvas` | [canvas.md](canvas.md) | the pixels of one drawing (a cell), RGBA8 |
| [8](../../../src/doc/mod.rs#L8) | `clip` | [clip.md](clip.md) | clipping masks: a layer that only shows where the layer below has paint |
| [9](../../../src/doc/mod.rs#L9) | `groups` | [groups.md](groups.md) | layer groups (a header row plus the layers under it) |
| [10](../../../src/doc/mod.rs#L10) | `layer` | [layer.md](layer.md) | one layer: which cell shows on each frame, plus its settings |
| [11](../../../src/doc/mod.rs#L11) | `orient` | [orient.md](orient.md) | the eight lossless ways to turn pixels over: mirror and quarter turns |
| [12](../../../src/doc/mod.rs#L12) | `project` | [project.md](project.md) | the whole animation: cells, layers, timeline, camera, and every edit on them |
| [13](../../../src/doc/mod.rs#L13) | `transform` | [transform.md](transform.md) | a layer's move / scale / rotate, its keyframes, and ease curves |

## Rust concepts in this file

**`mod` declares a module; `pub` makes it visible outside.** In JavaScript a file *is* a
module as soon as something imports it. In Rust the parent has to declare each child:
```rust
pub mod canvas;   // "load src/doc/canvas.rs as crate::doc::canvas, and let others use it"
```
```js
// closest JS idea: an index.js that re-exports the folder
export * as canvas from "./canvas.js";
```
A folder module's index file is called `mod.rs` (like `index.js`). Without `pub` the
submodule would only be reachable from inside `doc`.
More: [Modules and visibility](../../rust-primer.md#modules-and-visibility)

**`//!` versus `///` comments.** `//!` documents the thing the comment is *inside* (here: the
whole `doc` module); `///` documents the item that comes right *after* it.
```rust
//! Document model — project, layers, frames, cells.
```
More: [Reading a Rust file](../../rust-primer.md#reading-a-rust-file)
