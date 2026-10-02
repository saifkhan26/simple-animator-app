//! Reading a Krita brush preset (`.kpp`).
//!
//! A `.kpp` is a PNG — the preset's thumbnail — whose `preset` text chunk
//! holds the settings as XML: a flat list of `<param type name>` entries,
//! the `KisPropertiesConfiguration` of the paintop. Values are read the way
//! Krita's `getBool` / `getDouble` / `getString` read them, defaults
//! included, since a preset only stores what its author's Krita wrote.

use std::collections::HashMap;
use std::io::Read;

use anyhow::{anyhow, bail, Context, Result};

use super::curve::CubicCurve;
use super::sensors::{CurveOption, Sensor, SensorKind};

/// The flat settings of one preset.
#[derive(Clone, Debug)]
pub struct Preset {
    pub name: String,
    pub paintop: String,
    params: HashMap<String, Vec<u8>>,
}

/// The value of a PNG text chunk (`tEXt`, `zTXt` or `iTXt`) by keyword.
fn png_text(png: &[u8], key: &str) -> Result<Vec<u8>> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !png.starts_with(SIG) {
        bail!("not a PNG");
    }
    let mut i = SIG.len();
    while i + 8 <= png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        let kind = &png[i + 4..i + 8];
        let body = png
            .get(i + 8..i + 8 + len)
            .ok_or_else(|| anyhow!("truncated PNG chunk"))?;
        let split = body.iter().position(|&b| b == 0);
        if let (Some(z), b"tEXt" | b"zTXt" | b"iTXt") = (split, kind) {
            if &body[..z] == key.as_bytes() {
                let rest = &body[z + 1..];
                return match kind {
                    b"tEXt" => Ok(rest.to_vec()),
                    b"zTXt" => inflate(rest.get(1..).unwrap_or_default()),
                    _ => {
                        // iTXt: flag, method, language\0, translated\0, text.
                        let compressed = rest.first().copied().unwrap_or(0) == 1;
                        let mut r = rest.get(2..).unwrap_or_default();
                        for _ in 0..2 {
                            let n = r.iter().position(|&b| b == 0).ok_or_else(|| anyhow!("bad iTXt"))?;
                            r = &r[n + 1..];
                        }
                        if compressed {
                            inflate(r)
                        } else {
                            Ok(r.to_vec())
                        }
                    }
                };
            }
        }
        i += 12 + len;
    }
    bail!("no `{key}` chunk")
}

/// Krita's XML carries a `<!DOCTYPE>`, which roxmltree refuses unless asked.
fn parse_xml(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(text, opts)
}

fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .context("inflating the preset")?;
    Ok(out)
}

/// `QByteArray::fromBase64`: forgiving of whitespace and missing padding.
fn base64(text: &[u8]) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in text {
        if c == b'=' {
            break;
        }
        let Some(v) = val(c) else { continue };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// The settings XML of a `.kpp`.
pub fn preset_xml(kpp: &[u8]) -> Result<String> {
    let xml = png_text(kpp, "preset")?;
    String::from_utf8(xml).context("preset XML is not UTF-8")
}

impl Preset {
    pub fn read(kpp: &[u8]) -> Result<Self> {
        let xml = preset_xml(kpp)?;
        let doc = parse_xml(&xml).context("parsing the preset XML")?;
        let root = doc.root_element();
        if root.tag_name().name() != "Preset" {
            bail!("not a brush preset");
        }
        let mut params = HashMap::new();
        for p in root.children().filter(|n| n.has_tag_name("param")) {
            let Some(name) = p.attribute("name") else { continue };
            let text: String = p.children().filter_map(|c| c.text()).collect();
            let value = if p.attribute("type") == Some("bytearray") {
                base64(text.as_bytes())
            } else {
                text.into_bytes()
            };
            params.insert(name.to_owned(), value);
        }
        Ok(Self {
            name: root.attribute("name").unwrap_or_default().to_owned(),
            paintop: root.attribute("paintopid").unwrap_or_default().to_owned(),
            params,
        })
    }

    pub fn string(&self, key: &str) -> Option<String> {
        self.params.get(key).map(|v| String::from_utf8_lossy(v).into_owned())
    }

    pub fn bytes(&self, key: &str) -> Option<&[u8]> {
        self.params.get(key).map(|v| v.as_slice())
    }

    /// `getBool`: `QVariant(QString).toBool()`.
    pub fn bool(&self, key: &str, default: bool) -> bool {
        match self.string(key) {
            None => default,
            Some(s) => {
                let s = s.trim().to_ascii_lowercase();
                !(s.is_empty() || s == "0" || s == "false")
            }
        }
    }

    pub fn double(&self, key: &str, default: f64) -> f64 {
        self.string(key)
            .and_then(|s| s.trim().parse::<f64>().ok())
            .unwrap_or(default)
    }

    pub fn int(&self, key: &str, default: i32) -> i32 {
        self.string(key)
            .and_then(|s| s.trim().parse::<i32>().ok())
            .unwrap_or(default)
    }

    /// A curve option, read the way `KisKritaSensorPack::read` reads it.
    ///
    /// `checkable` is the option's checkability: a non-checkable option
    /// (Opacity, Flow) is always on.
    pub fn curve_option(&self, id: &str, checkable: bool) -> Result<CurveOption> {
        const DEFAULT_CURVE: &str = "0,0;1,1;";
        let checked = !checkable || self.bool(&format!("Pressure{id}"), false);
        let definition = self.string(&format!("{id}Sensor")).unwrap_or_default();

        let mut active: Vec<(SensorKind, String)> = Vec::new();
        let mut common = DEFAULT_CURVE.to_owned();
        if !definition.is_empty() {
            if definition.contains("sensorslist") {
                bail!("{id}: several sensors on one option are not supported yet");
            }
            let doc = parse_xml(&definition).with_context(|| format!("{id}Sensor"))?;
            let e = doc.root_element();
            let sid = e.attribute("id").unwrap_or_default();
            let kind = SensorKind::from_id(sid).ok_or_else(|| anyhow!("{id}: sensor `{sid}` is not supported yet"))?;
            let curve = e
                .children()
                .find(|c| c.has_tag_name("curve"))
                .and_then(|c| c.text())
                .map(str::to_owned)
                .unwrap_or_else(|| DEFAULT_CURVE.to_owned());
            common = curve.clone();
            active.push((kind, curve));
        }
        let use_same = self.bool(&format!("{id}UseSameCurve"), true);
        if !definition.contains("curve") {
            if self.bool(&format!("Custom{id}"), false) {
                common = self.string(&format!("Curve{id}")).unwrap_or_else(|| DEFAULT_CURVE.to_owned());
                for s in &mut active {
                    s.1 = common.clone();
                }
            } else {
                common = DEFAULT_CURVE.to_owned();
            }
        }
        if use_same {
            if let Some(c) = self.string(&format!("{id}commonCurve")) {
                common = c;
            }
            if common.is_empty() {
                common = DEFAULT_CURVE.to_owned();
            }
        }
        // At least one sensor is always active: pressure.
        if active.is_empty() {
            active.push((SensorKind::Pressure, DEFAULT_CURVE.to_owned()));
        }

        let mut sensors = Vec::new();
        for (kind, curve) in active {
            let text = if use_same { &common } else { &curve };
            let curve = CubicCurve::parse(text).ok_or_else(|| anyhow!("{id}: bad curve `{text}`"))?;
            sensors.push(Sensor::new(kind, &curve));
        }
        sensors.sort_by_key(|s| s.kind.order());
        Ok(CurveOption {
            checked,
            use_curve: self.bool(&format!("{id}UseCurve"), true),
            curve_mode: self.int(&format!("{id}curveMode"), 0),
            strength: self.double(&format!("{id}Value"), 1.0),
            min: 0.0,
            max: 1.0,
            sensors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_lenient() {
        assert_eq!(base64(b"aGVsbG8="), b"hello");
        assert_eq!(base64(b"aGVs\nbG8"), b"hello");
    }

    #[test]
    fn reads_the_embedded_pencil() {
        let p = Preset::read(super::super::PENCIL5_KPP).unwrap();
        assert_eq!(p.paintop, "paintbrush");
        assert_eq!(p.name, "c)_Pencil-5_Tilted");
        assert_eq!(p.int("PaintOpAction", 2), 1);
        assert_eq!(p.double("Texture/Pattern/Scale", 1.0), 0.6);
        assert!(p.bool("Texture/Pattern/Enabled", false));
        // The pattern is stored as base64 text inside a base64 bytearray.
        let inner = base64(p.bytes("Texture/Pattern/Pattern").unwrap());
        assert!(inner.starts_with(b"\x89PNG"));
    }
}
