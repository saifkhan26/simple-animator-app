//! The eight lossless ways to turn a raster over: mirror, quarter turns, and
//! their mixes. Each one only moves pixels to other pixel positions, so it
//! never resamples, and doing one and then its inverse gives back every byte.
//!
//! An [`Orient`] is a horizontal mirror (or not) followed by some clockwise
//! quarter turns, in image space: x right, y down.

/// A mirror, then `quarter` clockwise quarter turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Orient {
    pub quarter: u8,
    pub mirror: bool,
}

impl Orient {
    pub const IDENTITY: Orient = Orient {
        quarter: 0,
        mirror: false,
    };
    /// Left and right swap.
    pub const FLIP_H: Orient = Orient {
        quarter: 0,
        mirror: true,
    };
    /// Top and bottom swap: a left-right mirror turned half way round.
    pub const FLIP_V: Orient = Orient {
        quarter: 2,
        mirror: true,
    };
    pub const CW: Orient = Orient {
        quarter: 1,
        mirror: false,
    };
    pub const CCW: Orient = Orient {
        quarter: 3,
        mirror: false,
    };
    pub const HALF: Orient = Orient {
        quarter: 2,
        mirror: false,
    };

    pub fn is_identity(self) -> bool {
        self.quarter % 4 == 0 && !self.mirror
    }

    /// `self` and then `next`. A mirror reverses the turns done before it
    /// (mirroring a clockwise turn makes it anticlockwise), which is all the
    /// arithmetic there is to it.
    pub fn then(self, next: Orient) -> Orient {
        if next.mirror {
            Orient {
                quarter: (next.quarter % 4 + 4 - self.quarter % 4) % 4,
                mirror: !self.mirror,
            }
        } else {
            Orient {
                quarter: (self.quarter + next.quarter) % 4,
                mirror: self.mirror,
            }
        }
    }

    /// Whether width and height trade places.
    pub fn swaps_axes(self) -> bool {
        self.quarter % 2 == 1
    }

    /// The size a `w`×`h` image comes out at.
    pub fn dims(self, w: u32, h: u32) -> (u32, u32) {
        if self.swaps_axes() {
            (h, w)
        } else {
            (w, h)
        }
    }

    /// Where pixel `(x, y)` of a `w`×`h` image lands.
    pub fn map(self, x: u32, y: u32, w: u32, h: u32) -> (u32, u32) {
        let (mut x, mut y, mut w, mut h) = (x, y, w, h);
        if self.mirror {
            x = w - 1 - x;
        }
        for _ in 0..self.quarter % 4 {
            (x, y) = (h - 1 - y, x);
            (w, h) = (h, w);
        }
        (x, y)
    }

    /// Where the `w`×`h` rect at `(x, y)` inside a `cw`×`ch` image lands, as
    /// its new top-left. Its size is [`Orient::dims`].
    pub fn map_rect(self, x: u32, y: u32, w: u32, h: u32, cw: u32, ch: u32) -> (u32, u32) {
        let a = self.map(x, y, cw, ch);
        let b = self.map(x + w - 1, y + h - 1, cw, ch);
        (a.0.min(b.0), a.1.min(b.1))
    }

    /// `src`, a `w`×`h` image of `bpp`-byte pixels, turned over.
    pub fn permute(self, src: &[u8], w: u32, h: u32, bpp: usize) -> Vec<u8> {
        debug_assert_eq!(src.len(), w as usize * h as usize * bpp);
        if self.is_identity() {
            return src.to_vec();
        }
        let (nw, _) = self.dims(w, h);
        let mut out = vec![0u8; src.len()];
        for y in 0..h {
            let row = &src[(y * w) as usize * bpp..((y + 1) * w) as usize * bpp];
            for (x, px) in row.chunks_exact(bpp).enumerate() {
                let (dx, dy) = self.map(x as u32, y, w, h);
                let at = (dy * nw + dx) as usize * bpp;
                out[at..at + bpp].copy_from_slice(px);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3×2 image whose every pixel differs, one byte each.
    const IMG: [u8; 6] = [1, 2, 3, 4, 5, 6];

    fn all() -> Vec<Orient> {
        (0..4)
            .flat_map(|quarter| [false, true].map(|mirror| Orient { quarter, mirror }))
            .collect()
    }

    fn apply(o: Orient, img: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
        let (nw, nh) = o.dims(w, h);
        (o.permute(img, w, h, 1), nw, nh)
    }

    #[test]
    fn four_quarters_and_two_flips_are_identity() {
        let mut img = (IMG.to_vec(), 3, 2);
        for _ in 0..4 {
            img = apply(Orient::CW, &img.0, img.1, img.2);
        }
        assert_eq!(img, (IMG.to_vec(), 3, 2));
        for o in [Orient::FLIP_H, Orient::FLIP_V] {
            let once = apply(o, &IMG, 3, 2);
            assert_ne!(once.0, IMG.to_vec());
            assert_eq!(apply(o, &once.0, once.1, once.2), (IMG.to_vec(), 3, 2));
        }
    }

    #[test]
    fn the_named_ones_do_what_they_say() {
        // 1 2 3
        // 4 5 6
        assert_eq!(apply(Orient::FLIP_H, &IMG, 3, 2), (vec![3, 2, 1, 6, 5, 4], 3, 2));
        assert_eq!(apply(Orient::FLIP_V, &IMG, 3, 2), (vec![4, 5, 6, 1, 2, 3], 3, 2));
        assert_eq!(apply(Orient::CW, &IMG, 3, 2), (vec![4, 1, 5, 2, 6, 3], 2, 3));
        assert_eq!(apply(Orient::CCW, &IMG, 3, 2), (vec![3, 6, 2, 5, 1, 4], 2, 3));
        assert_eq!(apply(Orient::HALF, &IMG, 3, 2), (vec![6, 5, 4, 3, 2, 1], 3, 2));
    }

    #[test]
    fn then_composes_like_applying_twice() {
        for a in all() {
            for b in all() {
                let (once, w, h) = apply(a, &IMG, 3, 2);
                let twice = apply(b, &once, w, h);
                assert_eq!(apply(a.then(b), &IMG, 3, 2), twice, "{a:?} then {b:?}");
            }
        }
    }

    #[test]
    fn half_turn_is_both_flips() {
        assert_eq!(Orient::FLIP_H.then(Orient::FLIP_V), Orient::HALF);
        assert_eq!(Orient::CW.then(Orient::CCW), Orient::IDENTITY);
    }

    #[test]
    fn a_rect_lands_where_its_pixels_do() {
        // A 2×1 rect at (1, 0) in a 4×3 image.
        for o in all() {
            let (x, y) = o.map_rect(1, 0, 2, 1, 4, 3);
            let (rw, rh) = o.dims(2, 1);
            let ends = [o.map(1, 0, 4, 3), o.map(2, 0, 4, 3)];
            for (px, py) in ends {
                assert!(px >= x && px < x + rw && py >= y && py < y + rh, "{o:?}");
            }
        }
    }
}
