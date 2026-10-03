//! Raster canvas — one cell's RGBA8 pixels.
//!
//! In Phase A this is the entire document. In Phase C this becomes one `Cell`
//! per layer per frame, owned by a `Project` and resolved via X-sheet.
//!
//! A canvas holds its pixels one of two ways. *Full*: every pixel, as a cell
//! being drawn on needs them. *Packed*: only the rect that has anything in
//! it. Drawings rarely fill their cell, so most of a full buffer is zeros: a
//! 2160×2880 project's cells came to 3.5 GB full and ~340 MB packed. Writing
//! unpacks ([`Canvas::pixels_mut`]); reading works on either; the app packs
//! the cells nobody is drawing on.

use std::borrow::Cow;

/// Serialised by hand (see the impls below) with exactly the wire layout the
/// derive gave it, so `.anim` files are unchanged.
#[derive(Clone)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    store: Store,
    /// Dirty rectangle (inclusive min, exclusive max). `None` = clean.
    /// Transient render bookkeeping — not persisted.
    pub dirty: Option<DirtyRect>,
}

#[derive(Clone)]
enum Store {
    /// Every pixel, row-major, RGBA8 unmultiplied.
    Full(Vec<u8>),
    /// Just `rect`, row-major: the bounds of every byte that isn't zero, so
    /// packing loses nothing. Everything outside it is zeros. An empty rect
    /// is a blank canvas and holds no buffer at all.
    Packed { rect: DirtyRect, pixels: Vec<u8> },
}

/// The pixels go out as one byte string rather than a sequence of bytes. In
/// postcard both are a length then the raw bytes — the same file — but the
/// byte string is written in one go instead of a call per byte. A packed
/// canvas is written out whole, one at a time.
impl serde::Serialize for Canvas {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        struct Bytes<'a>(&'a [u8]);
        impl serde::Serialize for Bytes<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_bytes(self.0)
            }
        }
        let mut st = s.serialize_struct("Canvas", 3)?;
        st.serialize_field("width", &self.width)?;
        st.serialize_field("height", &self.height)?;
        st.serialize_field("pixels", &Bytes(&self.pixels()))?;
        st.end()
    }
}

/// Read back as a byte string too, which hands the pixels over as one slice
/// that is packed as it arrives: a whole cell's buffer is never allocated.
impl<'de> serde::Deserialize<'de> for Canvas {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{DeserializeSeed, Error, SeqAccess, Visitor};
        use std::fmt;

        /// The pixels of a `width`×`height` canvas, packed.
        struct Pixels(u32, u32);
        impl<'de> DeserializeSeed<'de> for Pixels {
            type Value = Canvas;
            fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Canvas, D::Error> {
                d.deserialize_byte_buf(self)
            }
        }
        impl<'de> Visitor<'de> for Pixels {
            type Value = Canvas;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("RGBA8 pixels")
            }
            fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<Canvas, E> {
                let Pixels(w, h) = self;
                if v.len() as u64 != w as u64 * h as u64 * 4 {
                    return Err(Error::custom(format!("{w}×{h} canvas with {} bytes of pixels", v.len())));
                }
                let mut c = Canvas::packed_from(w, h, v);
                c.dirty = None;
                Ok(c)
            }
            fn visit_byte_buf<E: Error>(self, v: Vec<u8>) -> Result<Canvas, E> {
                self.visit_bytes(&v)
            }
        }

        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Canvas;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a canvas")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Canvas, A::Error> {
                let missing = |i| Error::invalid_length(i, &"width, height and pixels");
                let width: u32 = seq.next_element()?.ok_or_else(|| missing(0))?;
                let height: u32 = seq.next_element()?.ok_or_else(|| missing(1))?;
                seq.next_element_seed(Pixels(width, height))?.ok_or_else(|| missing(2))
            }
        }
        d.deserialize_struct("Canvas", &["width", "height", "pixels"], V)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRect {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

impl DirtyRect {
    fn is_empty(&self) -> bool {
        self.max_x <= self.min_x || self.max_y <= self.min_y
    }

    fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.min_x && x < self.max_x && y >= self.min_y && y < self.max_y
    }
}

const EMPTY: DirtyRect = DirtyRect {
    min_x: 0,
    min_y: 0,
    max_x: 0,
    max_y: 0,
};

/// A pixel word's alpha byte. RGBA8 read as a little-endian `u32`.
const ALPHA: u32 = u32::from_le_bytes([0, 0, 0, 255]);

/// The bounds, in a `w`-pixel-wide RGBA8 buffer, of every pixel with a bit
/// of `mask` set — `ALPHA` for what's drawn, `!0` for anything at all.
/// Rows above and below are skipped whole; the rows between only scan the
/// span not yet known to be inside.
fn scan_bounds(buf: &[u8], w: usize, mask: u32) -> Option<DirtyRect> {
    if w == 0 {
        return None;
    }
    let word = |px: &[u8]| u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
    let hit = |px: &[u8]| word(px) & mask != 0;
    // A row's pixels OR'd together a word at a time: no early exit, so it
    // vectorises — ~40% faster on the mostly-blank rows a big cell is made
    // of. A buffer that isn't word-aligned takes the byte scan instead.
    let blank = |row: &[u8]| match bytemuck::try_cast_slice::<u8, u32>(row) {
        Ok(px) => px.iter().fold(0, |acc, &p| acc | p) & mask == 0,
        Err(_) => !row.chunks_exact(4).any(hit),
    };
    let rows: Vec<&[u8]> = buf.chunks_exact(w * 4).collect();
    let top = rows.iter().position(|r| !blank(r))?;
    let bottom = rows.iter().rposition(|r| !blank(r))? + 1;
    let (mut x0, mut x1) = (w, 0);
    for row in &rows[top..bottom] {
        if let Some(x) = row[..x0 * 4].chunks_exact(4).position(hit) {
            x0 = x;
        }
        if let Some(x) = row[x1 * 4..].chunks_exact(4).rposition(hit) {
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

/// Whether no pixel in straight-alpha RGBA8 has any coverage. OR-reduces the
/// buffer a word at a time, in blocks so a drawing stops at its first stroke:
/// a blank 100 MB cell is one pass at memory speed, not 25M branches.
pub fn no_alpha(px: &[u8]) -> bool {
    let (head, words, tail) = bytemuck::pod_align_to::<u8, u64>(px);
    if !head.is_empty() {
        // Words wouldn't line up with pixels. Allocations this size are
        // aligned in practice, so this is the rare path.
        return px.chunks_exact(4).all(|p| p[3] == 0);
    }
    const ALPHA: u64 = u64::from_ne_bytes([0, 0, 0, 0xff, 0, 0, 0, 0xff]);
    words
        .chunks(4096)
        .all(|block| block.iter().fold(0, |acc, w| acc | w) & ALPHA == 0)
        && tail.chunks_exact(4).all(|p| p[3] == 0)
}

/// `rect` of a `w`-pixel-wide buffer, row by row.
fn crop(buf: &[u8], w: u32, rect: DirtyRect) -> Vec<u8> {
    let (x0, x1, w) = (rect.min_x as usize, rect.max_x as usize, w as usize);
    let mut out = Vec::with_capacity((x1 - x0) * (rect.max_y - rect.min_y) as usize * 4);
    for y in rect.min_y as usize..rect.max_y as usize {
        out.extend_from_slice(&buf[(y * w + x0) * 4..(y * w + x1) * 4]);
    }
    out
}

impl Canvas {
    /// A blank canvas. It holds no buffer until something is drawn on it.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            store: Store::Packed {
                rect: EMPTY,
                pixels: Vec::new(),
            },
            dirty: Some(DirtyRect {
                min_x: 0,
                min_y: 0,
                max_x: width,
                max_y: height,
            }),
        }
    }

    /// A canvas holding `pixels`: every pixel, row-major RGBA8.
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(pixels.len(), width as usize * height as usize * 4, "{width}×{height} pixels");
        Self {
            width,
            height,
            store: Store::Full(pixels),
            dirty: Some(DirtyRect {
                min_x: 0,
                min_y: 0,
                max_x: width,
                max_y: height,
            }),
        }
    }

    /// [`Canvas::from_pixels`], packed — without ever holding a copy of the
    /// whole buffer.
    pub fn packed_from(width: u32, height: u32, full: &[u8]) -> Self {
        assert_eq!(full.len(), width as usize * height as usize * 4, "{width}×{height} pixels");
        let rect = scan_bounds(full, width as usize, !0).unwrap_or(EMPTY);
        let pixels = if rect.is_empty() { Vec::new() } else { crop(full, width, rect) };
        Self {
            store: Store::Packed { rect, pixels },
            ..Self::new(width, height)
        }
    }

    /// What a freed cell becomes: no size, no pixels. Its id stays taken so
    /// no other id shifts; a load drops it for good (`Project::compact_cells`).
    pub fn tombstone() -> Self {
        Self {
            dirty: None,
            ..Self::new(0, 0)
        }
    }

    pub fn is_tombstone(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Bytes of every pixel: `width × height × 4`, however many are held.
    pub fn byte_len(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Every byte is zero — colour under no alpha included, unlike
    /// [`Canvas::is_blank`]. Packed, that's a canvas holding nothing.
    pub fn is_all_zero(&self) -> bool {
        match &self.store {
            Store::Full(p) => scan_bounds(p, self.width as usize, !0).is_none(),
            Store::Packed { rect, .. } => rect.is_empty(),
        }
    }

    /// Every pixel, to read: borrowed from a full canvas, built for a packed
    /// one. Building costs a whole buffer, so call it once per use, never
    /// per pixel — [`Canvas::px`] and [`Canvas::row_span`] read in place.
    pub fn pixels(&self) -> Cow<'_, [u8]> {
        match &self.store {
            Store::Full(p) => Cow::Borrowed(p),
            Store::Packed { .. } => Cow::Owned(self.unpacked()),
        }
    }

    /// Every pixel, to write. A packed canvas is unpacked first, and stays
    /// so until it is packed again.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        if let Store::Packed { .. } = self.store {
            self.store = Store::Full(self.unpacked());
        }
        match &mut self.store {
            Store::Full(p) => p,
            Store::Packed { .. } => unreachable!("just unpacked"),
        }
    }

    /// Every pixel, by value.
    pub fn into_pixels(self) -> Vec<u8> {
        match self.store {
            Store::Full(p) => p,
            Store::Packed { .. } => self.unpacked(),
        }
    }

    fn unpacked(&self) -> Vec<u8> {
        match &self.store {
            Store::Full(p) => p.clone(),
            Store::Packed { rect, pixels } => {
                let mut full = vec![0u8; self.byte_len()];
                let (w, rw) = (self.width as usize, (rect.max_x - rect.min_x) as usize * 4);
                for (y, row) in (rect.min_y as usize..rect.max_y as usize).zip(pixels.chunks_exact(rw.max(1))) {
                    let at = (y * w + rect.min_x as usize) * 4;
                    full[at..at + rw].copy_from_slice(row);
                }
                full
            }
        }
    }

    pub fn is_packed(&self) -> bool {
        matches!(self.store, Store::Packed { .. })
    }

    /// Keep only what's drawn. Returns whether it was full before.
    pub fn pack(&mut self) -> bool {
        let Store::Full(full) = &self.store else {
            return false;
        };
        self.store = Self::packed_from(self.width, self.height, full).store;
        true
    }

    /// Bytes this canvas holds on the heap.
    #[cfg(test)]
    pub fn heap_bytes(&self) -> usize {
        match &self.store {
            Store::Full(p) | Store::Packed { pixels: p, .. } => p.capacity(),
        }
    }

    /// The pixel at `(x, y)`, which must be on the canvas.
    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        debug_assert!(x < self.width && y < self.height);
        let (buf, i) = match &self.store {
            Store::Full(p) => (p, (y * self.width + x) as usize * 4),
            Store::Packed { rect, pixels } => {
                if !rect.contains(x, y) {
                    return [0; 4];
                }
                let rw = rect.max_x - rect.min_x;
                (pixels, ((y - rect.min_y) * rw + (x - rect.min_x)) as usize * 4)
            }
        };
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    /// Row `y` as stored: the first pixel's x and the bytes from there. Every
    /// pixel of the row outside the span is zeros. A full canvas gives the
    /// whole row.
    pub fn row_span(&self, y: u32) -> (u32, &[u8]) {
        match &self.store {
            Store::Full(p) => {
                let w = self.width as usize * 4;
                (0, &p[y as usize * w..(y as usize + 1) * w])
            }
            Store::Packed { rect, pixels } => {
                if y < rect.min_y || y >= rect.max_y {
                    return (0, &[]);
                }
                let rw = (rect.max_x - rect.min_x) as usize * 4;
                let at = (y - rect.min_y) as usize * rw;
                (rect.min_x, &pixels[at..at + rw])
            }
        }
    }

    /// `rect`'s pixels row by row; `rect` must lie on the canvas.
    pub fn read_rect(&self, rect: DirtyRect) -> Vec<u8> {
        let (w, h) = ((rect.max_x - rect.min_x) as usize, (rect.max_y - rect.min_y) as usize);
        let mut out = vec![0u8; w * h * 4];
        if w == 0 {
            return out;
        }
        for (y, dst) in (rect.min_y..rect.max_y).zip(out.chunks_exact_mut(w * 4)) {
            self.read_row_into(y, rect.min_x, dst);
        }
        out
    }

    /// Row `y` from pixel `x` on, as many pixels as `out` holds — which must
    /// stay on the canvas.
    pub fn read_row_into(&self, y: u32, x: u32, out: &mut [u8]) {
        let (x0, row) = self.row_span(y);
        let x1 = x0 + (row.len() / 4) as u32;
        let end = x + (out.len() / 4) as u32;
        let (a, b) = (x0.max(x), x1.min(end));
        if a >= b {
            out.fill(0);
            return;
        }
        out[..(a - x) as usize * 4].fill(0);
        out[(a - x) as usize * 4..(b - x) as usize * 4]
            .copy_from_slice(&row[(a - x0) as usize * 4..(b - x0) as usize * 4]);
        out[(b - x) as usize * 4..].fill(0);
    }

    /// A `new_w`×`new_h` copy with the drawing kept centred: grown, it gains
    /// a clear margin; shrunk, the edges are cropped. Packed, whatever this
    /// one was — so re-sizing a layer's canvas never holds its cells whole.
    pub fn recentered(&self, new_w: u32, new_h: u32) -> Canvas {
        let mut out = Canvas::new(new_w, new_h);
        // The old origin inside the new canvas; negative when shrinking.
        let ox = (new_w as i64 - self.width as i64) / 2;
        let oy = (new_h as i64 - self.height as i64) / 2;
        let held = match &self.store {
            Store::Packed { rect, .. } => *rect,
            Store::Full(p) => scan_bounds(p, self.width as usize, !0).unwrap_or(EMPTY),
        };
        let clamp = |v: i64, hi: u32| v.clamp(0, hi as i64) as u32;
        let rect = DirtyRect {
            min_x: clamp(held.min_x as i64 + ox, new_w),
            min_y: clamp(held.min_y as i64 + oy, new_h),
            max_x: clamp(held.max_x as i64 + ox, new_w),
            max_y: clamp(held.max_y as i64 + oy, new_h),
        };
        if rect.is_empty() {
            return out;
        }
        let rw = (rect.max_x - rect.min_x) as usize * 4;
        let mut pixels = vec![0u8; rw * (rect.max_y - rect.min_y) as usize];
        for (y, row) in (rect.min_y..rect.max_y).zip(pixels.chunks_exact_mut(rw)) {
            let sy = (y as i64 - oy) as u32;
            self.read_row_into(sy, (rect.min_x as i64 - ox) as u32, row);
        }
        out.store = Store::Packed { rect, pixels };
        out
    }

    /// The smallest rect holding every pixel with any alpha, or `None` for a
    /// blank canvas. A packed canvas only scans what it holds.
    pub fn content_bounds(&self) -> Option<DirtyRect> {
        match &self.store {
            Store::Full(p) => scan_bounds(p, self.width as usize, ALPHA),
            Store::Packed { rect, pixels } => {
                let r = scan_bounds(pixels, (rect.max_x - rect.min_x) as usize, ALPHA)?;
                Some(DirtyRect {
                    min_x: r.min_x + rect.min_x,
                    min_y: r.min_y + rect.min_y,
                    max_x: r.max_x + rect.min_x,
                    max_y: r.max_y + rect.min_y,
                })
            }
        }
    }

    /// Nothing drawn: no pixel has any alpha.
    pub fn is_blank(&self) -> bool {
        match &self.store {
            Store::Full(p) | Store::Packed { pixels: p, .. } => no_alpha(p),
        }
    }

    /// Clear to transparent — which, packed, is nothing at all.
    pub fn clear(&mut self) {
        self.store = Store::Packed {
            rect: EMPTY,
            pixels: Vec::new(),
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `Canvas` as the derive used to write it — what every `.anim` holds.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Derived {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    }

    fn inked(w: u32, h: u32) -> Canvas {
        let mut c = Canvas::new(w, h);
        for (i, b) in c.pixels_mut().iter_mut().enumerate() {
            *b = (i * 7 % 251) as u8;
        }
        c
    }

    /// A drawing in one corner, with colour under zero alpha around it.
    fn sparse() -> Canvas {
        let mut c = Canvas::new(40, 30);
        let w = 40;
        let px = c.pixels_mut();
        for y in 5..9 {
            for x in 10..14 {
                px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[9, 8, 7, 255]);
            }
        }
        px[(20 * w + 30) * 4] = 200; // red, but clear
        c
    }

    #[test]
    fn writes_the_bytes_the_derive_wrote() {
        let c = inked(37, 5);
        let old = Derived {
            width: c.width,
            height: c.height,
            pixels: c.pixels().into_owned(),
        };
        let bytes = postcard::to_stdvec(&c).unwrap();
        assert_eq!(bytes, postcard::to_stdvec(&old).unwrap());
        // Shared in the project, written as if it weren't.
        assert_eq!(postcard::to_stdvec(&std::sync::Arc::new(c)).unwrap(), bytes);
        // Packed, it still writes every pixel.
        let mut s = sparse();
        let full = postcard::to_stdvec(&s).unwrap();
        s.pack();
        assert_eq!(postcard::to_stdvec(&s).unwrap(), full);
    }

    #[test]
    fn reads_what_the_derive_wrote_packed() {
        let c = sparse();
        let old = Derived {
            width: c.width,
            height: c.height,
            pixels: c.pixels().into_owned(),
        };
        let back: Canvas = postcard::from_bytes(&postcard::to_stdvec(&old).unwrap()).unwrap();
        assert_eq!((back.width, back.height), (40, 30));
        assert!(back.is_packed());
        assert_eq!(back.pixels(), c.pixels());
        // From the drawing's corner to the clear red pixel: 21×16.
        assert_eq!(back.heap_bytes(), 21 * 16 * 4);
        assert!(back.dirty.is_none());
    }

    #[test]
    fn packing_loses_nothing_and_reads_the_same() {
        let full = sparse();
        let mut packed = full.clone();
        assert!(packed.pack());
        assert!(!packed.pack(), "already packed");
        assert_eq!(packed.pixels(), full.pixels());
        assert_eq!(packed.content_bounds(), full.content_bounds());
        for (x, y) in [(0, 0), (10, 5), (13, 8), (14, 8), (30, 20), (39, 29)] {
            assert_eq!(packed.px(x, y), full.px(x, y), "({x}, {y})");
        }
        let r = DirtyRect { min_x: 8, min_y: 4, max_x: 33, max_y: 22 };
        assert_eq!(packed.read_rect(r), full.read_rect(r));
        for y in 0..30 {
            let (x0, span) = packed.row_span(y);
            let row = &full.pixels()[(y * 40) as usize * 4..(y + 1) as usize * 40 * 4];
            assert!(row[..x0 as usize * 4].iter().all(|&b| b == 0));
            assert_eq!(&row[x0 as usize * 4..x0 as usize * 4 + span.len()], span);
        }
        // Writing unpacks, and keeps what was there.
        let mut back = packed.clone();
        back.pixels_mut()[0] = 1;
        assert!(!back.is_packed());
        assert_eq!(back.px(10, 5), [9, 8, 7, 255]);
    }

    #[test]
    fn a_blank_canvas_holds_nothing() {
        let c = Canvas::new(2160, 2880);
        assert_eq!(c.heap_bytes(), 0);
        assert!(c.is_blank());
        assert_eq!(c.px(100, 100), [0; 4]);
        let mut d = sparse();
        d.clear();
        assert_eq!(d.heap_bytes(), 0);
    }

    #[test]
    fn pixels_that_dont_fit_the_size_are_an_error() {
        let bad = Derived {
            width: 4,
            height: 4,
            pixels: vec![0; 10],
        };
        assert!(postcard::from_bytes::<Canvas>(&postcard::to_stdvec(&bad).unwrap()).is_err());
    }

    #[test]
    fn content_bounds_wraps_every_inked_pixel() {
        let mut c = Canvas::new(40, 30);
        assert_eq!(c.content_bounds(), None);
        // Colour with no alpha is nothing.
        c.pixels_mut()[(5 * 40 + 5) * 4] = 255;
        assert_eq!(c.content_bounds(), None);
        for (x, y) in [(7, 3), (30, 12), (12, 20)] {
            c.pixels_mut()[(y * 40 + x) * 4 + 3] = 1;
        }
        let r = DirtyRect {
            min_x: 7,
            min_y: 3,
            max_x: 31,
            max_y: 21,
        };
        assert_eq!(c.content_bounds(), Some(r));
        c.pack();
        assert_eq!(c.content_bounds(), Some(r), "packed: the same, from less");
    }
}
