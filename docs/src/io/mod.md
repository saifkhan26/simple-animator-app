# `src/io/mod.rs` — the list of file-format modules

> **Source:** [src/io/mod.rs](../../../src/io/mod.rs) · **Module:** `crate::io` · **Size:** 21 lines · Line links as of commit `36405ad`

## What this file is for

`src/io/` is the part of the app that talks to files on disk: saving and opening
Animator's own `.anim` projects, exporting PNG sequences / GIF / WebP / MP4 / sprite
sheets, importing images, GIFs, WebPs and videos, and reading and writing Krita `.kra`
documents.

`mod.rs` holds no code of its own. It is the folder's "table of contents": each
`pub mod name;` line tells the Rust compiler "there is a file `src/io/name.rs` (or a
folder `src/io/name/mod.rs`); compile it and make it visible as `crate::io::name`".
Think of it as an `index.js` that re-exports every file in a folder, except that in
Rust a file which is **not** listed here is not compiled at all.

The `//!` comment at the top still describes an early stage of the project ("Phase A:
only `png_save`", "ORA read"). The list of modules below is what the code actually
contains today; there is no ORA module.

## How it connects

- **Used by:** every file that writes `crate::io::…`, for example
  [app.rs](../app.md) (`crate::io::project_file`, `crate::io::kra`) and
  [ui/shell.rs](../ui/shell.md) (`use crate::io::{composite, png_import, png_save, project_file}`).
- **Uses:** nothing; it only declares the sub-modules listed below.

## Rust concepts in this file

**`mod` declares a file as part of the program.** A Rust crate is a tree of modules.
Writing `pub mod kra;` in `src/io/mod.rs` pulls in `src/io/kra/mod.rs` (a sub-folder)
and `pub mod legacy;` pulls in `src/io/legacy.rs`. `pub` makes the module reachable
from outside `io`.
```rust
pub mod kra;          // src/io/kra/mod.rs  -> crate::io::kra
pub mod project_file; // src/io/project_file.rs -> crate::io::project_file
```
```js
// index.js
export * as kra from './kra/index.js';
export * as project_file from './project_file.js';
```
Unlike JavaScript, there is no automatic discovery: a `.rs` file nobody declares with
`mod` is ignored by the compiler. (`src/io/fit.rs` is in the folder but is not listed
here, so it is not part of the build.)
More: [Modules and visibility](../../rust-primer.md#modules-and-visibility)

## Sub-modules

Each line in the file and the page that documents it. All are `pub`.

| Line | Module | Page | One-line job |
|---|---|---|---|
| [5](../../../src/io/mod.rs#L5) | `composite` | [composite.md](composite.md) | CPU composite: flattens a frame's layers into flat pixels for exports (onion skins and reference layers left out). |
| [6](../../../src/io/mod.rs#L6) | `frame_order` | [frame_order.md](frame_order.md) | Output frame order for exports: which timeline frame each written frame shows. |
| [7](../../../src/io/mod.rs#L7) | `gif_export` | [gif_export.md](gif_export.md) | Export the project as an animated GIF. |
| [8](../../../src/io/mod.rs#L8) | `gif_import` | [gif_import.md](gif_import.md) | Decode an animated GIF into one full-size canvas per frame. |
| [9](../../../src/io/mod.rs#L9) | `mp4_export` | [mp4_export.md](mp4_export.md) | Export the project as an H.264 MP4 by running `ffmpeg`. |
| [10](../../../src/io/mod.rs#L10) | `image_import` | [image_import.md](image_import.md) | Import a single still image as one project-sized canvas. |
| [11](../../../src/io/mod.rs#L11) | `kra` | [kra/mod.md](kra/mod.md) | Krita `.kra` read / write (a folder module with `tiles` and `xml` inside). |
| [12](../../../src/io/mod.rs#L12) | `legacy` | [legacy.md](legacy.md) | Frozen copies of old `.anim` layouts, so old project files still open. |
| [13](../../../src/io/mod.rs#L13) | `png_import` | [png_import.md](png_import.md) | Import a folder of PNGs (a sequence) as a new layer. |
| [14](../../../src/io/mod.rs#L14) | `png_save` | [png_save.md](png_save.md) | Save the current canvas as a PNG, or copy it to the clipboard. |
| [15](../../../src/io/mod.rs#L15) | `png_seq` | [png_seq.md](png_seq.md) | Export every frame as `frame_NNNN.png` into a folder. |
| [16](../../../src/io/mod.rs#L16) | `project_file` | [project_file.md](project_file.md) | Save / open Animator's own `.anim` project files. |
| [17](../../../src/io/mod.rs#L17) | `sprite_sheet` | [sprite_sheet.md](sprite_sheet.md) | Sprite-sheet export: a frame range laid out as a grid in one PNG. |
| [18](../../../src/io/mod.rs#L18) | `video_import` | [video_import.md](video_import.md) | Import frames from a video by running `ffmpeg` / `ffprobe`. |
| [19](../../../src/io/mod.rs#L19) | `webp_export` | [webp_export.md](webp_export.md) | Export the project as a lossless animated WebP. |
| [20](../../../src/io/mod.rs#L20) | `webp_import` | [webp_import.md](webp_import.md) | Decode an animated WebP into one full-size canvas per frame. |

The one-line jobs are paraphrased from the `//!` comment at the top of each file; see
each page for the details.
