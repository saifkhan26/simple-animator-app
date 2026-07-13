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

#[derive(Clone, Copy, Debug)]
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

}
