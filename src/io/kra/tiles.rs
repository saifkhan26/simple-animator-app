//! Krita's tiled paint-device files — the per-layer / per-drawing pixel data
//! inside a `.kra`.
//!
//! Layout ("VERSION 2", unchanged since Krita 2.x):
//!
//! ```text
//! VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA <count>\n
//! <x>,<y>,LZF,<len>\n <len bytes>      ← repeated <count> times
//! ```
//!
//! `x`,`y` are the tile's top-left in device pixels, 64-aligned and possibly
//! negative — a Krita layer is unbounded, so paint past the canvas survives.
//! The first payload byte is a flag: `1` = LZF-compressed *channel-planar*
//! data (every pixel's byte 0, then every byte 1, …), `0` = the raw
//! interleaved tile. Pixels are Krita's 8-bit RGBA colour space, which is
//! **BGRA** in memory, unpremultiplied.
//!
//! We only ever *write* raw tiles: the zip entry is deflated anyway, and it
//! spares us an LZF compressor. Krita writes compressed ones, so the reader
//! needs LZF decompression — which is small.

use anyhow::{bail, Context, Result};

pub const TILE: i32 = 64;
const PIXEL: usize = 4;
const TILE_BYTES: usize = (TILE * TILE) as usize * PIXEL;

const FLAG_RAW: u8 = 0;
const FLAG_LZF: u8 = 1;

/// Pixels decoded from a tile file: the bounding box of every tile present,
/// in device coordinates, as unpremultiplied RGBA8. Space no tile covers holds
/// the device's default pixel.
pub struct Decoded {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

/// Encode an RGBA8 `w`×`h` buffer whose top-left sits at device `(ox, oy)`.
/// Fully transparent tiles are left out — they are the default pixel.
pub fn encode(pixels: &[u8], w: u32, h: u32, ox: i32, oy: i32) -> Vec<u8> {
    let (w, h) = (w as i32, h as i32);
    let mut body = Vec::new();
    let mut count = 0usize;
    let mut tile = vec![0u8; TILE_BYTES];
    let (tx0, ty0) = (ox.div_euclid(TILE) * TILE, oy.div_euclid(TILE) * TILE);
    let mut ty = ty0;
    while ty < oy + h {
        let mut tx = tx0;
        while tx < ox + w {
            tile.fill(0);
            let mut any = false;
            for row in 0..TILE {
                let sy = ty + row - oy;
                if !(0..h).contains(&sy) {
                    continue;
                }
                // Clip the tile row against the source row.
                let sx0 = (tx - ox).max(0);
                let sx1 = (tx + TILE - ox).min(w);
                for sx in sx0..sx1 {
                    let s = ((sy * w + sx) as usize) * PIXEL;
                    let d = ((row * TILE + (sx + ox - tx)) as usize) * PIXEL;
                    let px = &pixels[s..s + PIXEL];
                    if px[3] == 0 && px[0] == 0 && px[1] == 0 && px[2] == 0 {
                        continue;
                    }
                    any = true;
                    tile[d] = px[2];
                    tile[d + 1] = px[1];
                    tile[d + 2] = px[0];
                    tile[d + 3] = px[3];
                }
            }
            if any {
                body.extend_from_slice(format!("{tx},{ty},LZF,{}\n", TILE_BYTES + 1).as_bytes());
                body.push(FLAG_RAW);
                body.extend_from_slice(&tile);
                count += 1;
            }
            tx += TILE;
        }
        ty += TILE;
    }
    let mut out = format!(
        "VERSION 2\nTILEWIDTH {TILE}\nTILEHEIGHT {TILE}\nPIXELSIZE {PIXEL}\nDATA {count}\n"
    )
    .into_bytes();
    out.extend_from_slice(&body);
    out
}

/// Decode a tile file. `default` is the device's default pixel (RGBA), used for
/// any gap between tiles inside the bounding box. `None` when the file holds
/// no tiles at all.
pub fn decode(bytes: &[u8], default: [u8; 4]) -> Result<Option<Decoded>> {
    let mut pos = 0usize;
    let mut version = 0;
    let mut pixel_size = 0;
    let count;
    loop {
        let l = next_line(bytes, &mut pos)?;
        let (key, val) = l.split_once(' ').unwrap_or((l, ""));
        match key {
            "VERSION" => version = val.trim().parse().unwrap_or(0),
            "TILEWIDTH" | "TILEHEIGHT" if val.trim() != "64" => bail!("unsupported tile size {val}"),
            "PIXELSIZE" => pixel_size = val.trim().parse().unwrap_or(0),
            "DATA" => {
                count = val.trim().parse::<usize>().context("bad tile count")?;
                break;
            }
            _ => {}
        }
    }
    if version != 2 {
        bail!("unsupported Krita tile version {version}");
    }
    if pixel_size != PIXEL {
        bail!("layer is not 8-bit RGBA ({pixel_size} bytes per pixel)");
    }

    // Parse every tile first: the bounding box has to be known before the
    // output buffer can be sized.
    let mut tiles: Vec<(i32, i32, Vec<u8>)> = Vec::with_capacity(count);
    let mut planar = vec![0u8; TILE_BYTES];
    for _ in 0..count {
        let l = next_line(bytes, &mut pos)?;
        let mut it = l.split(',');
        let (Some(x), Some(y), Some(_codec), Some(len)) = (it.next(), it.next(), it.next(), it.next())
        else {
            bail!("bad tile header {l:?}");
        };
        let x: i32 = x.parse().context("tile x")?;
        let y: i32 = y.parse().context("tile y")?;
        let len: usize = len.parse().context("tile length")?;
        let data = bytes.get(pos..pos + len).context("truncated tile data")?;
        pos += len;

        let mut bgra = vec![0u8; TILE_BYTES];
        match data.first() {
            Some(&FLAG_LZF) => {
                let n = lzf_decompress(&data[1..], &mut planar)?;
                if n != TILE_BYTES {
                    bail!("tile inflated to {n} bytes, expected {TILE_BYTES}");
                }
                delinearize(&planar, &mut bgra);
            }
            Some(&FLAG_RAW) if data.len() > TILE_BYTES => bgra.copy_from_slice(&data[1..=TILE_BYTES]),
            _ => bail!("bad tile payload at {x},{y}"),
        }
        tiles.push((x, y, bgra));
    }
    if tiles.is_empty() {
        return Ok(None);
    }

    let x0 = tiles.iter().map(|t| t.0).min().unwrap_or(0);
    let y0 = tiles.iter().map(|t| t.1).min().unwrap_or(0);
    let x1 = tiles.iter().map(|t| t.0 + TILE).max().unwrap_or(0);
    let y1 = tiles.iter().map(|t| t.1 + TILE).max().unwrap_or(0);
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let mut pixels = Vec::with_capacity((w * h) as usize * PIXEL);
    for _ in 0..w * h {
        pixels.extend_from_slice(&default);
    }
    for (tx, ty, bgra) in tiles {
        for row in 0..TILE as usize {
            let dy = (ty - y0) as usize + row;
            for col in 0..TILE as usize {
                let s = (row * TILE as usize + col) * PIXEL;
                let d = (dy * w as usize + (tx - x0) as usize + col) * PIXEL;
                pixels[d] = bgra[s + 2];
                pixels[d + 1] = bgra[s + 1];
                pixels[d + 2] = bgra[s];
                pixels[d + 3] = bgra[s + 3];
            }
        }
    }
    Ok(Some(Decoded { x: x0, y: y0, w, h, pixels }))
}

/// The text line starting at `*pos`, advancing past its newline.
fn next_line<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a str> {
    let rest = &bytes[*pos..];
    let end = rest.iter().position(|&b| b == b'\n').context("truncated tile header")?;
    *pos += end + 1;
    std::str::from_utf8(&rest[..end]).context("tile header is not text")
}

/// Undo Krita's channel-planar layout: `planar` holds every pixel's byte 0,
/// then every byte 1, and so on.
fn delinearize(planar: &[u8], out: &mut [u8]) {
    let n = planar.len() / PIXEL;
    for c in 0..PIXEL {
        for i in 0..n {
            out[i * PIXEL + c] = planar[c * n + i];
        }
    }
}

/// Plain LZF (liblzf's format, no header), as Krita's `KisLzfCompression`
/// writes it. Returns the number of bytes produced.
fn lzf_decompress(input: &[u8], out: &mut [u8]) -> Result<usize> {
    let (mut ip, mut op) = (0usize, 0usize);
    while ip < input.len() {
        let ctrl = input[ip] as usize;
        ip += 1;
        if ctrl < 32 {
            // Literal run of ctrl + 1 bytes.
            let n = ctrl + 1;
            let src = input.get(ip..ip + n).context("LZF literal overruns input")?;
            out.get_mut(op..op + n).context("LZF literal overruns output")?.copy_from_slice(src);
            ip += n;
            op += n;
        } else {
            // Back reference: length in the top three bits (7 = extended),
            // distance in the low five plus the next byte.
            let mut len = ctrl >> 5;
            if len == 7 {
                len += *input.get(ip).context("LZF truncated length")? as usize;
                ip += 1;
            }
            len += 2;
            let lo = *input.get(ip).context("LZF truncated offset")? as usize;
            ip += 1;
            let dist = ((ctrl & 0x1f) << 8) + lo + 1;
            if dist > op || op + len > out.len() {
                bail!("LZF back reference out of range");
            }
            // Byte by byte: the source may overlap what is being written.
            for _ in 0..len {
                out[op] = out[op - dist];
                op += 1;
            }
        }
    }
    Ok(op)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny LZF *compressor* for tests only — enough to produce the planar,
    /// compressed tiles Krita writes, so the decoder is exercised on the real
    /// code path rather than only on raw tiles.
    fn lzf_literal_only(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in input.chunks(32) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
        out
    }

    #[test]
    fn lzf_back_reference_repeats_overlapping_run() {
        // Literal "ab", then a back reference of length 6 at distance 2:
        // ctrl = (6-2)<<5 | 0, offset byte = 1 (distance = 0 + 1 + 1).
        let input = [1, b'a', b'b', (4 << 5), 1];
        let mut out = [0u8; 8];
        let n = lzf_decompress(&input, &mut out).unwrap();
        assert_eq!(&out[..n], b"abababab");
    }

    #[test]
    fn lzf_extended_length() {
        // One literal, then len = 7 + 3 + 2 = 12 at distance 1.
        let input = [0, b'z', (7 << 5), 3, 0];
        let mut out = [0u8; 13];
        let n = lzf_decompress(&input, &mut out).unwrap();
        assert_eq!(n, 13);
        assert!(out.iter().all(|&b| b == b'z'));
    }

    #[test]
    fn lzf_rejects_reference_before_start() {
        let mut out = [0u8; 8];
        assert!(lzf_decompress(&[(1 << 5), 4], &mut out).is_err());
    }

    fn sample(w: u32, h: u32) -> Vec<u8> {
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                if (x + y) % 3 == 0 {
                    let i = ((y * w + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[x as u8, y as u8, 200, 128 + (x % 100) as u8]);
                }
            }
        }
        px
    }

    /// Crop a decoded device back to a `w`×`h` window at `(ox, oy)`.
    fn crop(d: &Decoded, ox: i32, oy: i32, w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (dx, dy) = (ox + x - d.x, oy + y - d.y);
                if dx < 0 || dy < 0 || dx >= d.w as i32 || dy >= d.h as i32 {
                    continue;
                }
                let s = ((dy as u32 * d.w + dx as u32) * 4) as usize;
                let t = ((y as u32 * w + x as u32) * 4) as usize;
                out[t..t + 4].copy_from_slice(&d.pixels[s..s + 4]);
            }
        }
        out
    }

    #[test]
    fn raw_round_trip_with_negative_unaligned_origin() {
        let (w, h) = (150, 90);
        let px = sample(w, h);
        let file = encode(&px, w, h, -37, -70);
        let d = decode(&file, [0; 4]).unwrap().unwrap();
        assert_eq!(d.x % TILE, 0);
        assert!(d.x <= -37 && d.y <= -70);
        assert_eq!(crop(&d, -37, -70, w, h), px);
    }

    #[test]
    fn empty_buffer_writes_no_tiles() {
        let file = encode(&vec![0u8; 64 * 64 * 4], 64, 64, 0, 0);
        assert!(file.ends_with(b"DATA 0\n"));
        assert!(decode(&file, [0; 4]).unwrap().is_none());
    }

    #[test]
    fn decodes_compressed_planar_tile() {
        // One tile the way Krita writes it: BGRA, channel-planar, LZF.
        let mut bgra = vec![0u8; TILE_BYTES];
        for (i, p) in bgra.chunks_mut(4).enumerate() {
            p.copy_from_slice(&[10, 20, 30, (i % 256) as u8]);
        }
        let n = TILE_BYTES / 4;
        let mut planar = vec![0u8; TILE_BYTES];
        for i in 0..n {
            for c in 0..4 {
                planar[c * n + i] = bgra[i * 4 + c];
            }
        }
        let mut payload = vec![FLAG_LZF];
        payload.extend(lzf_literal_only(&planar));
        let mut file = b"VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA 1\n".to_vec();
        file.extend_from_slice(format!("-64,128,LZF,{}\n", payload.len()).as_bytes());
        file.extend_from_slice(&payload);

        let d = decode(&file, [0; 4]).unwrap().unwrap();
        assert_eq!((d.x, d.y, d.w, d.h), (-64, 128, 64, 64));
        // BGRA (10,20,30) comes back as RGBA (30,20,10).
        assert_eq!(&d.pixels[4..8], &[30, 20, 10, 1]);
    }

    #[test]
    fn gaps_between_tiles_take_the_default_pixel() {
        let mut px = vec![0u8; (192 * 64 * 4) as usize];
        px[0..4].copy_from_slice(&[1, 2, 3, 255]);
        let last = ((192 * 64 - 1) * 4) as usize;
        px[last..last + 4].copy_from_slice(&[4, 5, 6, 255]);
        let file = encode(&px, 192, 64, 0, 0);
        let d = decode(&file, [9, 9, 9, 9]).unwrap().unwrap();
        assert_eq!(d.w, 192);
        // The middle tile was never written, so it is the default pixel.
        let mid = ((192 * 32 + 96) * 4) as usize;
        assert_eq!(&d.pixels[mid..mid + 4], &[9, 9, 9, 9]);
    }
}
