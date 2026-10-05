//! GIMP brush tips: a single `.gbr` (`KisGbrBrush::init`, libs/brush/
//! kis_gbr_brush.cpp) and the `.gih` image pipe that strings several
//! together and picks one per dab (`KisImagePipeBrush`,
//! kis_imagepipe_brush.cpp, kis_pipebrush_parasite.cpp,
//! kis_brushes_pipe.h).
//!
//! Only grayscale cells are taken: Krita turns those into a mask. A colour
//! `.gbr` is painted as an image stamp or lightness map, which this engine
//! does not do.

use anyhow::{anyhow, bail, Result};

/// A grayscale cell as Krita holds it: `255 - byte`, so the mask a dab
/// takes from it (`255 - gray`) is the file's own byte.
#[derive(Clone, Debug, PartialEq)]
pub struct GbrCell {
    pub width: u32,
    pub height: u32,
    pub gray: Vec<u8>,
}

fn be32(d: &[u8], at: usize) -> Result<u32> {
    let b = d.get(at..at + 4).ok_or_else(|| anyhow!("truncated GIMP brush"))?;
    Ok(u32::from_be_bytes(b.try_into().unwrap()))
}

/// `KisGbrBrush::init` on `data`: the cell, and how many bytes it took.
pub fn parse_gbr(data: &[u8]) -> Result<(GbrCell, usize)> {
    // sizeof(GimpBrushHeader): seven 32-bit fields.
    if data.len() < 28 {
        bail!("GIMP brush shorter than its header");
    }
    let header_size = be32(data, 0)? as usize;
    let version = be32(data, 4)?;
    let width = be32(data, 8)?;
    let height = be32(data, 12)?;
    let bytes = be32(data, 16)?;
    if version != 1 && be32(data, 24)? > 1000 {
        bail!("GIMP brush spacing above 1000");
    }
    if header_size == 0 || header_size > data.len() {
        bail!("GIMP brush header size {header_size} out of range");
    }
    if width == 0 || height == 0 {
        bail!("GIMP brush of zero size");
    }
    let n = width as usize * height as usize;
    match bytes {
        1 => {
            let px = data
                .get(header_size..header_size + n)
                .ok_or_else(|| anyhow!("GIMP brush smaller than its {width}x{height} image"))?;
            let gray = px.iter().map(|&b| 255 - b).collect();
            Ok((GbrCell { width, height, gray }, header_size + n))
        }
        4 => bail!("colour GIMP brush tips are not supported yet"),
        b => bail!("GIMP brushes with {b} bytes per pixel are not supported"),
    }
}

/// `KisParasite::SelectionMode`: how a pipe dimension picks its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Constant,
    Incremental,
    Angular,
    Random,
    Pressure,
    TiltX,
    TiltY,
    Velocity,
}

const MAX_DIM: usize = 4;

/// `KisPipeBrushParasite`, parsed from the `.gih` header.
#[derive(Clone, Debug, PartialEq)]
pub struct Parasite {
    pub dim: usize,
    pub ncells: i32,
    pub rank: [i32; MAX_DIM],
    pub selection: [Selection; MAX_DIM],
    pub brushes_count: [i32; MAX_DIM],
}

impl Parasite {
    /// `KisPipeBrushParasite(const QString&)` then `sanitize()`.
    pub fn parse(source: &str) -> Self {
        let mut p = Parasite {
            dim: 1,
            ncells: 0,
            rank: [0; MAX_DIM],
            selection: [Selection::Constant; MAX_DIM],
            brushes_count: [0; MAX_DIM],
        };
        let to_int = |s: &str| s.trim().parse::<i32>().unwrap_or(0);
        for item in source.split(' ').filter(|s| !s.is_empty()) {
            let parts: Vec<&str> = item.split(':').filter(|s| !s.is_empty()).collect();
            if parts.len() != 2 {
                continue;
            }
            let (key, value) = (parts[0], parts[1]);
            if key == "dim" {
                let d = to_int(value);
                p.dim = if (1..=MAX_DIM as i32).contains(&d) { d as usize } else { 1 };
            } else if let Some(i) = key.strip_prefix("sel") {
                let i = to_int(i);
                if i >= 0 && (i as usize) < p.dim {
                    p.selection[i as usize] = match value {
                        "incremental" => Selection::Incremental,
                        "angular" => Selection::Angular,
                        "random" => Selection::Random,
                        "pressure" => Selection::Pressure,
                        "xtilt" => Selection::TiltX,
                        "ytilt" => Selection::TiltY,
                        "velocity" => Selection::Velocity,
                        _ => Selection::Constant,
                    };
                }
            } else if let Some(i) = key.strip_prefix("rank") {
                let i = to_int(i);
                if i >= 0 && (i as usize) <= p.dim && (i as usize) < MAX_DIM {
                    p.rank[i as usize] = to_int(value);
                }
            } else if key == "ncells" {
                p.ncells = to_int(value).max(1);
            }
        }
        // setBrushesCount.
        p.brushes_count[0] = if p.rank[0] != 0 { p.ncells / p.rank[0] } else { p.ncells };
        for i in 1..p.dim {
            p.brushes_count[i] = if p.rank[i] == 0 {
                p.brushes_count[i - 1]
            } else {
                p.brushes_count[i - 1] / p.rank[i]
            };
        }
        // sanitize: these two would divide by a zero rank.
        for i in 0..p.dim {
            if p.rank[i] == 0 && matches!(p.selection[i], Selection::Incremental | Selection::Angular) {
                p.selection[i] = Selection::Constant;
            }
        }
        p
    }
}

/// A `.gih`: the header's parasite and its cells.
pub fn parse_gih(data: &[u8]) -> Result<(Parasite, Vec<GbrCell>)> {
    let nl1 = data.iter().position(|&b| b == b'\n').ok_or_else(|| anyhow!("not an image pipe"))?;
    let rest = &data[nl1 + 1..];
    let nl2 = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
    let line2 = String::from_utf8_lossy(&rest[..nl2]);
    let (count, parasite) = match line2.find(' ') {
        Some(i) => (&line2[..i], &line2[i + 1..]),
        None => (&line2[..], ""),
    };
    let count: usize = count.trim().parse().unwrap_or(0);
    let parasite = Parasite::parse(parasite);
    let mut at = nl1 + 1 + nl2 + 1;
    let mut cells = Vec::new();
    while cells.len() < count && at < data.len() {
        let (cell, used) = parse_gbr(&data[at..])?;
        cells.push(cell);
        at += used;
    }
    if cells.is_empty() {
        bail!("an image pipe with no cells");
    }
    Ok((parasite, cells))
}

/// What a pipe's selection reads from the dab.
#[derive(Clone, Copy, Debug, Default)]
pub struct PipeInput {
    pub pressure: f64,
    pub drawing_angle: f64,
    pub x_tilt: f64,
    pub y_tilt: f64,
    pub speed: f64,
}

/// One stroke's walk through a pipe's cells.
#[derive(Clone, Debug)]
pub struct PipeState {
    index: [i32; MAX_DIM],
    pub current: usize,
}

impl PipeState {
    pub fn new() -> Self {
        Self {
            index: [0; MAX_DIM],
            current: 0,
        }
    }

    /// `prepareForSeqNo`: advance the indices for dab `seq_no` (drawing
    /// from `random` for a random dimension), then pick the cell.
    pub fn prepare(&mut self, p: &Parasite, cells: usize, input: &PipeInput, seq_no: i32, random: &mut impl FnMut(i32, i32) -> i32) -> usize {
        // updateBrushIndexes / selectPost.
        for i in 0..p.dim {
            let rank = p.rank[i];
            self.index[i] = match p.selection[i] {
                Selection::Incremental => (if seq_no >= 0 { seq_no } else { self.index[i] + 1 }) % rank,
                Selection::Random => random(0, rank - 1),
                _ => self.index[i],
            };
        }
        // chooseNextBrush / selectPre.
        let mut brush_index: u32 = 0;
        for i in 0..p.dim {
            let rank = p.rank[i];
            let idx = match p.selection[i] {
                Selection::Constant | Selection::Incremental | Selection::Random => self.index[i],
                Selection::Pressure => (input.pressure * (rank - 1) as f64 + 0.5) as i32,
                Selection::Angular => {
                    let a = super::super::sensors::normalize_angle(input.drawing_angle + std::f64::consts::FRAC_PI_2 + std::f64::consts::FRAC_PI_4);
                    (a / (2.0 * std::f64::consts::PI) * rank as f64) as i32
                }
                Selection::TiltX => qround(input.x_tilt / 2.0 * rank as f64) + rank / 2,
                Selection::TiltY => qround(input.y_tilt / 2.0 * rank as f64) + rank / 2,
                Selection::Velocity => {
                    let cap = 3.0;
                    let v = (input.speed + 1.0).ln().min(cap) / cap * ((rank - 1) as f64 + 0.5);
                    qround(v)
                }
            };
            brush_index = brush_index.wrapping_add((p.brushes_count[i] as u32).wrapping_mul(idx as u32));
        }
        self.current = (brush_index % cells as u32) as usize;
        self.current
    }
}

fn qround(d: f64) -> i32 {
    if d >= 0.0 {
        (d + 0.5) as i32
    } else {
        (d - 0.5) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gbr(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
        let name = b"t\0";
        let header = 28 + name.len() as u32;
        let mut v = Vec::new();
        for f in [header, 2, w, h, 1, 0x4749_4d50, 25] {
            v.extend_from_slice(&f.to_be_bytes());
        }
        v.extend_from_slice(name);
        v.extend_from_slice(px);
        v
    }

    #[test]
    fn a_gray_gbr_is_its_inverse() {
        let (c, used) = parse_gbr(&gbr(2, 1, &[0, 200])).unwrap();
        assert_eq!((c.width, c.height), (2, 1));
        assert_eq!(c.gray, vec![255, 55]);
        assert_eq!(used, 30 + 2);
    }

    #[test]
    fn the_shipped_pipe_parses_as_seven_random_cells() {
        let data = super::super::super::defaults::file("brushes/graphite_grain.gih").unwrap();
        let (p, cells) = parse_gih(&data).unwrap();
        assert_eq!(cells.len(), 7);
        assert!(cells.iter().all(|c| (c.width, c.height) == (128, 128)));
        assert_eq!(p.selection[0], Selection::Random);
        assert_eq!((p.rank[0], p.ncells, p.brushes_count[0]), (7, 7, 1));
    }

    /// Krita's own test brushes (libs/brush/tests/data).
    #[test]
    fn krita_s_test_brushes_read_as_krita_reads_them() {
        let (c, used) = parse_gbr(include_bytes!("testdata/brush.gbr")).unwrap();
        assert_eq!((c.width, c.height, used), (7, 7, 89), "\"Circle (07)\"");
        // A filled disc: the file's corner byte is 0 (nothing), so Krita's
        // image holds 255 there and 0 at the solid top and centre.
        assert_eq!((c.gray[0], c.gray[3], c.gray[24]), (255, 0, 0));

        let e = parse_gbr(include_bytes!("testdata/pepper.gbr")).unwrap_err();
        assert!(format!("{e}").contains("not supported"), "{e}");

        let (p, cells) = parse_gih(include_bytes!("testdata/brush.gih")).unwrap();
        assert_eq!(cells.len(), 5);
        assert!(cells.iter().all(|c| (c.width, c.height) == (64, 64)));
        assert_eq!((p.dim, p.rank[0], p.selection[0], p.brushes_count[0]), (1, 5, Selection::Random, 1));
    }

    #[test]
    fn selection_walks_like_krita() {
        let p = Parasite::parse("ncells:6 dim:1 rank0:6 sel0:incremental");
        let mut s = PipeState::new();
        let mut never = |_: i32, _: i32| -> i32 { panic!("no randomness here") };
        let picks: Vec<usize> = (0..8).map(|seq| s.prepare(&p, 6, &PipeInput::default(), seq, &mut never)).collect();
        assert_eq!(picks, vec![0, 1, 2, 3, 4, 5, 0, 1]);

        let p = Parasite::parse("ncells:5 dim:1 rank0:5 sel0:pressure");
        let input = PipeInput { pressure: 0.6, ..Default::default() };
        assert_eq!(PipeState::new().prepare(&p, 5, &input, 0, &mut never), 2, "0.6 · 4 + 0.5");

        let p = Parasite::parse("ncells:7 dim:1 rank0:7 sel0:random");
        let mut n = 0;
        let mut seq = |lo: i32, hi: i32| {
            assert_eq!((lo, hi), (0, 6));
            n += 1;
            n % 7
        };
        assert_eq!(PipeState::new().prepare(&p, 7, &PipeInput::default(), 0, &mut seq), 1);
    }
}
