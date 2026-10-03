//! Raster canvas — single RGBA8 pixel buffer.
//!
//! In Phase A this is the entire document. In Phase C this becomes one `Cell`
//! per layer per frame, owned by a `Project` and resolved via X-sheet.

/// Serialised by hand (see the impls below) with exactly the wire layout the
/// derive gave it, so `.anim` files are unchanged.
#[derive(Clone)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// Row-major, RGBA8 unmultiplied.
    pub pixels: Vec<u8>,
    /// Dirty rectangle (inclusive min, exclusive max). `None` = clean.
    /// Transient render bookkeeping — not persisted.
    pub dirty: Option<DirtyRect>,
}

/// The pixels go out as one byte string rather than a sequence of bytes. In
/// postcard both are a length then the raw bytes — the same file — but the
/// byte string is written in one go instead of a call per byte.
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
        st.serialize_field("pixels", &Bytes(&self.pixels))?;
        st.end()
    }
}

/// Read back as a byte string too, which hands the pixels over as one slice:
/// they land in a buffer of exactly their size. Read as a sequence they grew
/// by doubling, leaving a 2160×2880 cell's 24 MB in a 32 MB allocation.
impl<'de> serde::Deserialize<'de> for Canvas {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{Error, SeqAccess, Visitor};
        use std::fmt;

        struct Pixels(Vec<u8>);
        impl<'de> serde::Deserialize<'de> for Pixels {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl<'de> Visitor<'de> for V {
                    type Value = Pixels;
                    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                        f.write_str("RGBA8 pixels")
                    }
                    fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<Pixels, E> {
                        Ok(Pixels(v.to_vec()))
                    }
                    fn visit_byte_buf<E: Error>(self, v: Vec<u8>) -> Result<Pixels, E> {
                        Ok(Pixels(v))
                    }
                }
                d.deserialize_byte_buf(V)
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
                let Pixels(pixels) = seq.next_element()?.ok_or_else(|| missing(2))?;
                if pixels.len() as u64 != width as u64 * height as u64 * 4 {
                    return Err(Error::custom(format!(
                        "{width}×{height} canvas with {} bytes of pixels",
                        pixels.len()
                    )));
                }
                Ok(Canvas {
                    width,
                    height,
                    pixels,
                    dirty: None,
                })
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

    /// What a freed cell becomes: no size, no pixels. Its id stays taken so
    /// no other id shifts; a load drops it for good (`Project::compact_cells`).
    pub fn tombstone() -> Self {
        Self {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            dirty: None,
        }
    }

    pub fn is_tombstone(&self) -> bool {
        self.pixels.is_empty()
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

    /// `Canvas` as the derive used to write it — what every `.anim` holds.
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Derived {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    }

    fn inked(w: u32, h: u32) -> Canvas {
        let mut c = Canvas::new(w, h);
        for (i, b) in c.pixels.iter_mut().enumerate() {
            *b = (i * 7 % 251) as u8;
        }
        c
    }

    #[test]
    fn writes_the_bytes_the_derive_wrote() {
        let c = inked(37, 5);
        let old = Derived {
            width: c.width,
            height: c.height,
            pixels: c.pixels.clone(),
        };
        let bytes = postcard::to_stdvec(&c).unwrap();
        assert_eq!(bytes, postcard::to_stdvec(&old).unwrap());
        // Shared in the project, written as if it weren't.
        assert_eq!(postcard::to_stdvec(&std::sync::Arc::new(c)).unwrap(), bytes);
    }

    #[test]
    fn reads_what_the_derive_wrote_into_an_exact_buffer() {
        let c = inked(300, 200);
        let old = Derived {
            width: c.width,
            height: c.height,
            pixels: c.pixels.clone(),
        };
        let back: Canvas = postcard::from_bytes(&postcard::to_stdvec(&old).unwrap()).unwrap();
        assert_eq!((back.width, back.height), (300, 200));
        assert_eq!(back.pixels, c.pixels);
        assert_eq!(back.pixels.capacity(), back.pixels.len(), "no growth slack");
        assert!(back.dirty.is_none());
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
