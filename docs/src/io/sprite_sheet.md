# `src/io/sprite_sheet.rs` — sprite-sheet export

> **Source:** [src/io/sprite_sheet.rs](../../../src/io/sprite_sheet.rs) · **Module:** `crate::io::sprite_sheet` · **Size:** 170 lines · Line links as of commit `1fa5845`

## What this file is for

Exports a range of frames laid out as a **grid in one PNG** (a sprite sheet, as games and CSS animations
use). Each cell is the camera's frame size and is flattened exactly like every other export, so a cell
matches the same frame of the PNG sequence pixel for pixel. Transparent padding goes *between* cells only.

```text
cols = 3, padding = p
┌────┐p┌────┐p┌────┐
│ f0 │ │ f1 │ │ f2 │
└────┘ └────┘ └────┘
 p
┌────┐ ┌────┐
│ f3 │ │ f4 │
└────┘ └────┘
```

## How it connects

- **Used by:** [app.rs](../app.md) (the export job, `ExportKind::SpriteSheet`), [ui/shell.rs](../ui/shell.md)
  (dialog shows the resulting size).
- **Uses:** [io/composite.rs](composite.md) (`flatten_frame`), the `image` crate.

## Rust concepts in this file

**Integer division and remainder for grid positions.** `(i % cols, i / cols)` gives column and row — same as
in JS, except Rust's `/` on integers already truncates (no `Math.floor` needed).

## Types

### `struct SheetOptions`
[src/io/sprite_sheet.rs:19](../../../src/io/sprite_sheet.rs#L19)

The sprite-sheet dialog's options.

| Field | Type | Meaning |
|---|---|---|
| `columns` | `usize` | Columns in the grid. `0` = pick a near-square layout. |
| `padding` | `u32` | Transparent gutter between cells, in pixels. |

## Free functions

### `grid`
[src/io/sprite_sheet.rs:30](../../../src/io/sprite_sheet.rs#L30)

```rust
pub fn grid(n: usize, columns: usize) -> (usize, usize)
```

The grid shape `(columns, rows)` for `n` frames: the requested columns (clamped to `n`), or with 0 a
near-square layout (`columns = ceil(√n)`); rows = `ceil(n / columns)`.

### `sheet_size`
[src/io/sprite_sheet.rs:45](../../../src/io/sprite_sheet.rs#L45)

```rust
pub fn sheet_size(fw: u32, fh: u32, cols: usize, rows: usize, padding: u32) -> (u32, u32)
```

The sheet's pixel size: cells times count plus padding between them (`count − 1` gaps per axis).

### `cell_origin`
[src/io/sprite_sheet.rs:54](../../../src/io/sprite_sheet.rs#L54)

```rust
fn cell_origin(i: usize, cols: usize, fw: u32, fh: u32, padding: u32) -> (u32, u32)
```

The top-left pixel of cell `i`.

### `export_to`
[src/io/sprite_sheet.rs:60](../../../src/io/sprite_sheet.rs#L60)

```rust
pub fn export_to(
    project: &Project,
    path: &Path,
    range: (usize, usize),
    opts: &SheetOptions,
) -> Result<()>
```

Flattens frames `range.0..=range.1`, copies each into its cell of one transparent image, and saves it as a
PNG at `path`. Blocking; runs on the export worker thread.

## Tests

### `mod tests`

Run with `cargo test io::sprite_sheet::tests` · 5 tests

| Test | What it checks |
|---|---|
| [`auto_columns_are_near_square`](../../../src/io/sprite_sheet.rs#L103) | Auto columns are near square. |
| [`explicit_columns_are_honoured_but_clamped_to_the_frame_count`](../../../src/io/sprite_sheet.rs#L111) | Explicit columns are honoured but clamped to the frame count. |
| [`padding_only_sits_between_cells`](../../../src/io/sprite_sheet.rs#L120) | Padding only sits between cells. |
| [`cells_are_laid_out_row_major_at_the_expected_stride`](../../../src/io/sprite_sheet.rs#L130) | Cells are laid out row major at the expected stride. |
| [`every_cell_fits_inside_the_sheet`](../../../src/io/sprite_sheet.rs#L140) | Every cell fits inside the sheet. |

### `mod camera_size_tests`

Run with `cargo test io::sprite_sheet::camera_size_tests` · 1 tests

| Test | What it checks |
|---|---|
| [`cells_are_the_camera_s_frame`](../../../src/io/sprite_sheet.rs#L158) | Cells are the camera s frame. |
