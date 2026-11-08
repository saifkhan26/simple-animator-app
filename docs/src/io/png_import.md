# `src/io/png_import.rs` — importing a PNG sequence as a layer

> **Source:** [src/io/png_import.rs](../../../src/io/png_import.rs) · **Module:** `crate::io::png_import` · **Size:** 113 lines · Line links as of commit `67a7cb9`

## What this file is for

Imports a folder of PNG files (an image sequence like `frame_0001.png`, `frame_0002.png`, …) as a **new
layer** with one drawing per file, keyed on consecutive frames — the reverse of PNG-sequence export
([io/png_seq.rs](png_seq.md)).

## How it connects

- **Used by:** [app.rs](../app.md) / [ui/shell.rs](../ui/shell.md) (File → Import PNG sequence).
- **Uses:** [doc/project.rs](../doc/project.md), [doc/canvas.rs](../doc/canvas.md), the `image` crate, `rfd`.

## Rust concepts in this file

**`let … else` for an early return.**
```rust
let Some(dir) = rfd::FileDialog::new().pick_folder() else { return Ok(()); };
```
If the dialog was cancelled (`None`), leave the function; otherwise `dir` holds the folder.
More: [Enums and match](../../rust-primer.md#enums-and-match)

**`anyhow::bail!`** returns an error immediately: `bail!("No PNG files found in {:?}", dir)` is like
`throw new Error(...)`.

## Free functions

### `import_dialog`
[src/io/png_import.rs:11](../../../src/io/png_import.rs#L11)

```rust
pub fn import_dialog(project: &mut Project) -> Result<()>
```

Asks for a folder with a folder-picker dialog and imports it with `import_from` (cancelling does nothing).

### `import_from`
[src/io/png_import.rs:21](../../../src/io/png_import.rs#L21)

```rust
pub fn import_from(project: &mut Project, dir: &PathBuf) -> Result<()>
```

Imports every PNG in `dir`, sorted by file name:
1. Errors if there are none.
2. Extends the timeline (appending holds to every layer) if the sequence is longer, without moving the
   playhead.
3. Adds a new layer on top named after the folder, decodes each PNG into a new cell and keys it on frame
   `i`.
4. Makes the new layer active and logs a summary.
