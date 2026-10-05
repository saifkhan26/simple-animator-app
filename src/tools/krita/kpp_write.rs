//! Writing a `.kpp`, as `KisPaintOpPreset::saveToDevice` does: a PNG of the
//! preset's picture, with the settings XML in a `preset` text chunk and a
//! `version` chunk of `5.0`, which tells Krita to read the tips the preset
//! carries in its `<resources>`.

use std::io::Write;

use anyhow::{Context, Result};

use super::kpp::{EmbeddedResource, Preset};

/// Carry `data` as the tip `filename`, replacing a copy already carried.
pub fn embed_tip(preset: &mut Preset, filename: &str, data: Vec<u8>) {
    preset.resources.retain(|r| !(r.kind == "brushes" && r.filename == filename));
    let name = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem).to_owned();
    preset.resources.push(EmbeddedResource {
        kind: "brushes".to_owned(),
        name,
        filename: filename.to_owned(),
        md5sum: md5_hex(&data),
        data,
    });
}

/// The `.kpp` bytes: `picture` with the preset's chunks.
pub fn write(preset: &Preset, picture: &image::RgbaImage) -> Result<Vec<u8>> {
    let mut png = Vec::new();
    picture
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .context("encoding the preset's picture")?;
    // iTXt, compressed: the XML is UTF-8 (a name may not be Latin-1) and
    // carries base64 textures that deflate well.
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(preset.to_xml().as_bytes())?;
    let mut itxt = b"preset\0\x01\x00\0\0".to_vec();
    itxt.extend_from_slice(&z.finish()?);
    let chunks = [chunk(b"tEXt", b"version\x005.0"), chunk(b"iTXt", &itxt)];
    // After IHDR, which is always the first chunk: 8 signature bytes, then
    // a 13-byte body in 25 bytes of chunk.
    let at = 8 + 25;
    let mut out = png[..at].to_vec();
    for c in chunks {
        out.extend_from_slice(&c);
    }
    out.extend_from_slice(&png[at..]);
    Ok(out)
}

fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut c = (body.len() as u32).to_be_bytes().to_vec();
    c.extend_from_slice(kind);
    c.extend_from_slice(body);
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(body);
    c.extend_from_slice(&crc.sum().to_be_bytes());
    c
}

/// MD5 (RFC 1321), as hex: what Krita names an embedded resource's
/// contents by.
pub fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16,
        23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64).map(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32).collect();
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    let mut h = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    for block in msg.chunks_exact(64) {
        let m: Vec<u32> = block.chunks_exact(4).map(|w| u32::from_le_bytes(w.try_into().unwrap())).collect();
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::super::{defaults, PixelBrush};
    use super::*;

    #[test]
    fn md5_matches_rfc_1321() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    /// A written preset carries its tip, so it loads with nothing else
    /// supplied, and it is still a PNG an image reader opens.
    #[test]
    fn a_written_preset_loads_on_its_own() {
        let mut p = Preset::read(&defaults::kpp("c)_Pencil-4_Soft.kpp").unwrap()).unwrap();
        p.name = "Grainy ünïcode".into();
        let tip = defaults::file("brushes/graphite_grain.gih").unwrap();
        embed_tip(&mut p, "graphite_grain.gih", tip);
        let picture = image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 200, 200, 255]));
        let kpp = write(&p, &picture).unwrap();
        assert!(image::load_from_memory(&kpp).is_ok());
        let back = Preset::read(&kpp).unwrap();
        assert_eq!(back.version, "5.0");
        assert_eq!(back.name, "Grainy ünïcode");
        assert_eq!(back.resources.len(), 1);
        assert_eq!(back.resources[0].md5sum, md5_hex(&back.resources[0].data));
        let no_tips: &[(&str, &[u8])] = &[];
        let b = PixelBrush::load(&kpp, no_tips).expect("loads from its own resources");
        let shipped = defaults::load("c)_Pencil-4_Soft.kpp").unwrap();
        assert_eq!(b.diameter(), shipped.diameter());
        assert_eq!(b.wash, shipped.wash);
    }
}
