//! Sprite-sheet export — a frame range laid out as a grid in one PNG.
//!
//! Cells are the project frame size, composited exactly as every other export
//! is (`composite::flatten_frame`), so a sheet cell matches the same frame from
//! the PNG sequence pixel for pixel. Padding sits *between* cells only: an
//! outer margin would offset every cell by half a gutter and make the frame
//! stride disagree with the cell size, which is the number a game engine or
//! spritesheet importer actually wants.

use std::path::Path;

use anyhow::{bail, Context, Result};
use image::{ImageBuffer, Rgba};

use crate::doc::project::Project;
use crate::io::composite;

#[derive(Clone, Copy, Debug, Default)]
pub struct SheetOptions {
    /// Columns in the grid. `0` = pick a near-square layout.
    pub columns: usize,
    /// Transparent gutter between cells, in pixels.
    pub padding: u32,
}

/// Grid shape for `n` frames: `(columns, rows)`.
///
/// `columns` is clamped to `n` so asking for more columns than there are frames
/// doesn't pad the sheet with empty space.
pub fn grid(n: usize, columns: usize) -> (usize, usize) {
    if n == 0 {
        return (0, 0);
    }
    let cols = if columns == 0 {
        (n as f64).sqrt().ceil() as usize
    } else {
        columns
    }
    .clamp(1, n);
    let rows = n.div_ceil(cols);
    (cols, rows)
}

/// Pixel size of a sheet holding a `cols` × `rows` grid of `fw` × `fh` cells.
pub fn sheet_size(fw: u32, fh: u32, cols: usize, rows: usize, padding: u32) -> (u32, u32) {
    if cols == 0 || rows == 0 {
        return (0, 0);
    }
    let gaps = |count: usize| padding * (count as u32 - 1);
    (fw * cols as u32 + gaps(cols), fh * rows as u32 + gaps(rows))
}

/// Top-left pixel of cell `i` in the grid.
fn cell_origin(i: usize, cols: usize, fw: u32, fh: u32, padding: u32) -> (u32, u32) {
    let (col, row) = (i % cols, i / cols);
    (col as u32 * (fw + padding), row as u32 * (fh + padding))
}

/// Write frames `range.0..=range.1` as one PNG grid.
pub fn export_to(
    project: &Project,
    path: &Path,
    range: (usize, usize),
    opts: &SheetOptions,
) -> Result<()> {
    let last = project.frame_count.saturating_sub(1);
    let (start, end) = (range.0.min(last), range.1.min(last));
    if project.frame_count == 0 || end < start {
        bail!("empty frame range");
    }
    let n = end - start + 1;
    let (fw, fh) = (project.width, project.height);
    let (cols, rows) = grid(n, opts.columns);
    let (sw, sh) = sheet_size(fw, fh, cols, rows, opts.padding);

    // Transparent ground; cells never overlap, so a plain copy is enough and no
    // blend is involved.
    let mut sheet = vec![0u8; (sw as usize) * (sh as usize) * 4];
    for (i, f) in (start..=end).enumerate() {
        let flat = composite::flatten_frame(project, f);
        let (ox, oy) = cell_origin(i, cols, fw, fh, opts.padding);
        for y in 0..fh {
            let src = (y * fw * 4) as usize;
            let dst = (((oy + y) * sw + ox) * 4) as usize;
            let len = (fw * 4) as usize;
            sheet[dst..dst + len].copy_from_slice(&flat.pixels[src..src + len]);
        }
    }

    let buf: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(sw, sh, sheet).context("sheet buffer/dim mismatch")?;
    buf.save(path)
        .with_context(|| format!("writing {path:?}"))?;
    log::info!("Exported {n} frames as sprite sheet → {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_columns_are_near_square() {
        assert_eq!(grid(12, 0), (4, 3));
        assert_eq!(grid(16, 0), (4, 4));
        assert_eq!(grid(17, 0), (5, 4));
        assert_eq!(grid(1, 0), (1, 1));
    }

    #[test]
    fn explicit_columns_are_honoured_but_clamped_to_the_frame_count() {
        assert_eq!(grid(12, 6), (6, 2));
        assert_eq!(grid(12, 5), (5, 3));
        // Asking for more columns than frames would pad the sheet with dead
        // space; clamp instead.
        assert_eq!(grid(3, 8), (3, 1));
    }

    #[test]
    fn padding_only_sits_between_cells() {
        // 4x3 of 100x50 with a 10px gutter: 3 gutters across, 2 down.
        assert_eq!(sheet_size(100, 50, 4, 3, 10), (430, 170));
        // No padding, one row.
        assert_eq!(sheet_size(100, 50, 4, 1, 0), (400, 50));
        // A single cell is exactly the frame, whatever the padding.
        assert_eq!(sheet_size(100, 50, 1, 1, 10), (100, 50));
    }

    #[test]
    fn cells_are_laid_out_row_major_at_the_expected_stride() {
        let (cols, _) = grid(6, 3);
        assert_eq!(cell_origin(0, cols, 100, 50, 10), (0, 0));
        assert_eq!(cell_origin(2, cols, 100, 50, 10), (220, 0));
        // Frame 3 wraps to the second row.
        assert_eq!(cell_origin(3, cols, 100, 50, 10), (0, 60));
        assert_eq!(cell_origin(5, cols, 100, 50, 10), (220, 60));
    }

    #[test]
    fn every_cell_fits_inside_the_sheet() {
        let (fw, fh, pad) = (64u32, 48u32, 3u32);
        for n in 1..=20usize {
            let (cols, rows) = grid(n, 0);
            let (sw, sh) = sheet_size(fw, fh, cols, rows, pad);
            for i in 0..n {
                let (x, y) = cell_origin(i, cols, fw, fh, pad);
                assert!(x + fw <= sw && y + fh <= sh, "n={n} cell {i} overflows");
            }
        }
    }
}
