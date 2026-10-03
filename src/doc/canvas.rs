//! Raster canvas — single RGBA8 pixel buffer.
//!
//! In Phase A this is the entire document. In Phase C this becomes one `Cell`
//! per layer per frame, owned by a `Project` and resolved via X-sheet.

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// Row-major, RGBA8 unmultiplied.
    pub pixels: Vec<u8>,
    /// Dirty rectangle (inclusive min, exclusive max). `None` = clean.
    /// Transient render bookkeeping — not persisted.
    #[serde(skip)]
    pub dirty: Option<DirtyRect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRect {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        let pixels = vec![0u8; (width * height * 4) as usize];
        Self {
            width,
            height,
            pixels,
            dirty: Some(DirtyRect {
                min_x: 0,
                min_y: 0,
                max_x: width,
                max_y: height,
            }),
        }
    }

    pub fn clear(&mut self) {
        for px in self.pixels.iter_mut() {
            *px = 0;
        }
        self.dirty = Some(DirtyRect {
            min_x: 0,
            min_y: 0,
            max_x: self.width,
            max_y: self.height,
        });
    }

    pub fn mark_dirty(&mut self, x: u32, y: u32, w: u32, h: u32) {
        let r = DirtyRect {
            min_x: x,
            min_y: y,
            max_x: (x + w).min(self.width),
            max_y: (y + h).min(self.height),
        };
        self.dirty = Some(match self.dirty {
            None => r,
            Some(d) => DirtyRect {
                min_x: d.min_x.min(r.min_x),
                min_y: d.min_y.min(r.min_y),
                max_x: d.max_x.max(r.max_x),
                max_y: d.max_y.max(r.max_y),
            },
        });
    }

    /// The smallest rect holding every pixel with any alpha, or `None` for a
    /// blank canvas. Rows above and below the drawing are skipped whole; the
    /// rows between only scan the span not yet known to be inside.
    pub fn content_bounds(&self) -> Option<DirtyRect> {
        let w = self.width as usize;
        if w == 0 {
            return None;
        }
        let rows: Vec<&[u8]> = self.pixels.chunks_exact(w * 4).collect();
        let inked = |px: &[u8]| px[3] != 0;
        // A row's pixels OR'd together a word at a time: no early exit, so it
        // vectorises — ~40% faster on the mostly-blank rows a big cell is made
        // of. A buffer that isn't word-aligned takes the byte scan instead.
        let blank = |row: &&[u8]| match bytemuck::try_cast_slice::<u8, u32>(row) {
            Ok(px) => px.iter().fold(0, |acc, &p| acc | p) & u32::from_le_bytes([0, 0, 0, 255]) == 0,
            Err(_) => !row.chunks_exact(4).any(inked),
        };
        let top = rows.iter().position(|r| !blank(r))?;
        let bottom = rows.iter().rposition(|r| !blank(r))? + 1;
        let (mut x0, mut x1) = (w, 0);
        for row in &rows[top..bottom] {
            if let Some(x) = row[..x0 * 4].chunks_exact(4).position(inked) {
                x0 = x;
            }
            if let Some(x) = row[x1 * 4..].chunks_exact(4).rposition(inked) {
                x1 += x + 1;
            }
        }
        Some(DirtyRect {
            min_x: x0 as u32,
            min_y: top as u32,
            max_x: x1 as u32,
            max_y: bottom as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_bounds_wraps_every_inked_pixel() {
        let mut c = Canvas::new(40, 30);
        assert_eq!(c.content_bounds(), None);
        // Colour with no alpha is nothing.
        c.pixels[(5 * 40 + 5) * 4] = 255;
        assert_eq!(c.content_bounds(), None);
        for (x, y) in [(7, 3), (30, 12), (12, 20)] {
            c.pixels[(y * 40 + x) * 4 + 3] = 1;
        }
        let r = DirtyRect {
            min_x: 7,
            min_y: 3,
            max_x: 31,
            max_y: 21,
        };
        assert_eq!(c.content_bounds(), Some(r));
    }
}
