//! The Krita presets the app ships: the eraser, basic, pencil, ink and
//! marker pixel-brush presets of `Krita_4_Default_Resources.bundle` (CC-0),
//! with the tips they name, zipped into the binary (`assets/defaults.zip`).
//! `regen_default_brushes` rebuilds the zip from an installed Krita.

use std::borrow::Cow;
use std::io::{Cursor, Read};
#[cfg(test)]
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use zip::ZipArchive;

use super::{PixelBrush, Resources};

const ZIP: &[u8] = include_bytes!("assets/defaults.zip");

/// The shipped presets, by file name inside the bundle, in the order the
/// brush picker lists them.
pub const PRESETS: [&str; 24] = [
    "a)_Eraser_Circle.kpp",
    "a)_Eraser_Small.kpp",
    "a)_Eraser_Soft.kpp",
    "b)_Airbrush_Soft.kpp",
    "b)_Basic-2_Opacity.kpp",
    "b)_Basic-3_Flow.kpp",
    "b)_Basic-4_Flow_Opacity.kpp",
    "b)_Basic-5_Size_Opacity.kpp",
    "b)_Basic-5_Size_default.kpp",
    "b)_Basic-6_Details.kpp",
    "c)_Pencil-1_Hard.kpp",
    "c)_Pencil-2.kpp",
    "c)_Pencil-3_Large_4B.kpp",
    "c)_Pencil-4_Soft.kpp",
    "c)_Pencil-5_Tilted.kpp",
    "c)_Pencil-6_Quick_Shade.kpp",
    "d)_Ink-1_Precision.kpp",
    "d)_Ink-2_Fineliner.kpp",
    "d)_Ink-3_Gpen.kpp",
    "d)_Ink-4_Pen_Rough.kpp",
    "d)_Ink-7_Brush_Rough.kpp",
    "e)_Marker_Chisel_Smooth.kpp",
    "e)_Marker_Details.kpp",
    "e)_Marker_Dry.kpp",
];

/// The tips those presets name.
#[cfg(test)]
pub const TIPS: [&str; 3] = ["gradient.png", "plain_rake.png", "graphite_grain.gih"];

type Archive = Mutex<ZipArchive<Cursor<&'static [u8]>>>;

fn archive() -> &'static Archive {
    static A: OnceLock<Archive> = OnceLock::new();
    A.get_or_init(|| Mutex::new(ZipArchive::new(Cursor::new(ZIP)).expect("embedded defaults.zip")))
}

/// A file from the zip: `paintoppresets/<name>` or `brushes/<name>`.
pub fn file(path: &str) -> Option<Vec<u8>> {
    let mut a = archive().lock().ok()?;
    let mut f = a.by_name(path).ok()?;
    let mut out = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut out).ok()?;
    Some(out)
}

/// Tips for the shipped presets.
pub struct DefaultTips;

impl Resources for DefaultTips {
    fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>> {
        file(&format!("brushes/{filename}")).map(Cow::Owned)
    }
}

/// The `.kpp` bytes of a shipped preset.
pub fn kpp(name: &str) -> Option<Vec<u8>> {
    file(&format!("paintoppresets/{name}"))
}

/// Load a shipped preset.
pub fn load(name: &str) -> Result<PixelBrush> {
    let bytes = kpp(name).ok_or_else(|| anyhow!("no default preset `{name}`"))?;
    PixelBrush::load(&bytes, &DefaultTips).with_context(|| name.to_owned())
}

/// A shipped preset, loaded once and shared.
#[cfg(test)]
pub fn brush(name: &str) -> Result<Arc<PixelBrush>> {
    type Cache = Mutex<Vec<(String, Arc<PixelBrush>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Some((_, b)) = cache.lock().unwrap().iter().find(|(n, _)| n == name) {
        return Ok(b.clone());
    }
    let b = Arc::new(load(name)?);
    cache.lock().unwrap().push((name.to_owned(), b.clone()));
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Which shipped presets the engine paints so far. Each phase of the
    /// port moves names from the refused list to the loading one.
    #[test]
    fn the_shipped_presets_load_or_say_why_not() {
        let mut loading = Vec::new();
        let mut refused = Vec::new();
        for name in PRESETS {
            match load(name) {
                Ok(_) => loading.push(name),
                Err(e) => refused.push((name, format!("{e:#}"))),
            }
        }
        let want_loading = [
            "a)_Eraser_Circle.kpp",
            "a)_Eraser_Small.kpp",
            "a)_Eraser_Soft.kpp",
            "b)_Airbrush_Soft.kpp",
            "b)_Basic-2_Opacity.kpp",
            "b)_Basic-3_Flow.kpp",
            "b)_Basic-4_Flow_Opacity.kpp",
            "b)_Basic-5_Size_Opacity.kpp",
            "b)_Basic-5_Size_default.kpp",
            "c)_Pencil-5_Tilted.kpp",
            "d)_Ink-3_Gpen.kpp",
            "d)_Ink-4_Pen_Rough.kpp",
            "e)_Marker_Details.kpp",
            "e)_Marker_Dry.kpp",
        ];
        for (name, why) in &refused {
            println!("refused {name}: {why}");
        }
        assert_eq!(loading, want_loading);
        // Every refusal names its reason.
        assert!(refused.iter().all(|(_, why)| why.contains("not supported")), "{refused:#?}");
    }

    /// Rebuild `assets/defaults.zip` from an installed Krita's CC-0 bundle.
    /// Reads the bundle only.
    ///
    /// ```text
    /// KRITA_BUNDLE="C:/Program Files/Krita (x64)/share/krita/bundles/Krita_4_Default_Resources.bundle" \
    ///     cargo test regen_default_brushes -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn regen_default_brushes() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let path = std::env::var("KRITA_BUNDLE").unwrap_or_else(|_| {
            "C:/Program Files/Krita (x64)/share/krita/bundles/Krita_4_Default_Resources.bundle".into()
        });
        let bundle = std::fs::read(&path).expect("read the bundle");
        let mut src = ZipArchive::new(Cursor::new(bundle)).unwrap();
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        let files = PRESETS
            .iter()
            .map(|n| format!("paintoppresets/{n}"))
            .chain(TIPS.iter().map(|t| format!("brushes/{t}")));
        for name in files {
            let mut data = Vec::new();
            src.by_name(&name).unwrap_or_else(|_| panic!("{name}")).read_to_end(&mut data).unwrap();
            out.start_file(name.as_str(), opts).unwrap();
            out.write_all(&data).unwrap();
        }
        let zip = out.finish().unwrap().into_inner();
        let dst = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tools/krita/assets/defaults.zip");
        std::fs::write(dst, &zip).unwrap();
        println!("wrote {} bytes", zip.len());
    }
}

