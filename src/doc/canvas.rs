//! Raster canvas — single RGBA8 pixel buffer.
//!
//! In Phase A this is the entire document. In Phase C this becomes one `Cell`
//! per layer per frame, owned by a `Project` and resolved via X-sheet.

#[derive(Clone)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// Row-major, RGBA8 unmultiplied.
    pub pixels: Vec<u8>,
    /// Dirty rectangle (inclusive min, exclusive max). `None` = clean.
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

    /// Blend a single RGBA8 source pixel onto destination using premultiplied
    /// "normal" blend (src-over). `src` is unmultiplied.
    #[inline]
    pub fn blend_pixel(&mut self, x: u32, y: u32, src: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = ((y * self.width + x) * 4) as usize;
        let dst = &mut self.pixels[idx..idx + 4];

        let sa = src[3] as f32 / 255.0;
        let da = dst[3] as f32 / 255.0;
        let out_a = sa + da * (1.0 - sa);
        if out_a <= 0.0 {
            dst.copy_from_slice(&[0, 0, 0, 0]);
            return;
        }
        for c in 0..3 {
            let s = src[c] as f32 / 255.0;
            let d = dst[c] as f32 / 255.0;
            let o = (s * sa + d * da * (1.0 - sa)) / out_a;
            dst[c] = (o * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    /// Erase = punch alpha (set to zero), not paint white.
    #[inline]
    pub fn erase_pixel(&mut self, x: u32, y: u32, strength: f32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = ((y * self.width + x) * 4) as usize;
        let dst = &mut self.pixels[idx..idx + 4];
        let s = strength.clamp(0.0, 1.0);
        let cur_a = dst[3] as f32 / 255.0;
        let new_a = (cur_a * (1.0 - s)).max(0.0);
        dst[3] = (new_a * 255.0).round() as u8;
        if new_a == 0.0 {
            dst[0] = 0;
            dst[1] = 0;
            dst[2] = 0;
        }
    }
}
