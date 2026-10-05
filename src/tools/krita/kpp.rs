//! Reading a Krita brush preset (`.kpp`).
//!
//! A `.kpp` is a PNG — the preset's thumbnail — whose `preset` text chunk
//! holds the settings as XML: a flat list of `<param type name>` entries,
//! the `KisPropertiesConfiguration` of the paintop. Values are read the way
//! Krita's `getBool` / `getDouble` / `getString` read them, defaults
//! included, since a preset only stores what its author's Krita wrote.
//! A version-5.0 preset also carries the files it needs (tips, patterns)
//! base64-encoded in a `<resources>` element.

use std::collections::HashMap;
use std::io::Read;

use anyhow::{anyhow, bail, Context, Result};

use super::curve::CubicCurve;
use super::options::OptionSpec;
use super::sensors::{CurveOption, Sensor, SensorKind};

pub const DEFAULT_CURVE: &str = "0,0;1,1;";

/// One `<param>`: its value, and its declared type (`string`, `bytearray`
/// …), kept so a preset can be written back the way Krita wrote it.
#[derive(Clone, Debug)]
pub struct Param {
    pub kind: String,
    pub value: Vec<u8>,
}

/// A file a version-5.0 preset carries inside itself.
#[derive(Clone, Debug)]
pub struct EmbeddedResource {
    /// `brushes`, `patterns`, …
    pub kind: String,
    pub name: String,
    pub filename: String,
    pub md5sum: String,
    pub data: Vec<u8>,
}

/// The flat settings of one preset.
#[derive(Clone, Debug)]
pub struct Preset {
    pub name: String,
    pub paintop: String,
    /// The `version` text chunk; `5.0` for a Krita 5 preset. Only the
    /// reader acts on it (embedded resources), so the app never reads it back.
    #[cfg_attr(not(test), allow(dead_code))]
    pub version: String,
    params: HashMap<String, Param>,
    /// Parameter names in the order the preset lists them.
    pub order: Vec<String>,
    pub resources: Vec<EmbeddedResource>,
}

/// A `.kpp` holding `xml`: just the text chunk, which is all the reader
/// looks at.
#[cfg(test)]
pub fn with_xml(xml: &str) -> Vec<u8> {
    let mut body = b"preset\0".to_vec();
    body.extend_from_slice(xml.as_bytes());
    let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
    v.extend_from_slice(&(body.len() as u32).to_be_bytes());
    v.extend_from_slice(b"tEXt");
    v.extend_from_slice(&body);
    v.extend_from_slice(&[0; 4]);
    v
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
pub(crate) fn base64(text: &[u8]) -> Vec<u8> {
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

/// One sensor slot of an option, as `KisSensorData` keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub id: &'static str,
    pub active: bool,
    pub curve: String,
    // KisDrawingAngleSensorData
    pub fan_corners_enabled: bool,
    pub fan_corners_step: i32,
    pub angle_offset: i32,
    pub locked_angle_mode: bool,
    // KisSensorWithLengthData (fade)
    pub length: i32,
    pub periodic: bool,
}

/// One curve option as `KisCurveOptionData` holds it: what a preset says,
/// before it becomes sensors. The editor changes this and writes it back.
#[derive(Clone, Debug, PartialEq)]
pub struct OptionData {
    pub checked: bool,
    pub use_curve: bool,
    pub use_same_curve: bool,
    pub strength: f64,
    pub curve_mode: i32,
    pub common_curve: String,
    /// Every sensor Krita knows, in its order; the active ones apply.
    pub slots: Vec<Slot>,
}

/// Every sensor `KisKritaSensorData` holds, in `detail::sensors` order.
pub const SLOTS: [&str; 16] = [
    "pressure",
    "pressurein",
    "tangentialpressure",
    "drawingangle",
    "xtilt",
    "ytilt",
    "ascension",
    "declination",
    "rotation",
    "fuzzy",
    "fuzzystroke",
    "speed",
    "fade",
    "distance",
    "time",
    "perspective",
];

impl Slot {
    /// `KisSensorData(id)` / `reset()`.
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            active: false,
            curve: DEFAULT_CURVE.to_owned(),
            fan_corners_enabled: false,
            fan_corners_step: 30,
            angle_offset: 0,
            locked_angle_mode: false,
            length: if id == "fade" { 1000 } else { 30 },
            periodic: false,
        }
    }

    /// `KisSensorData::read` and its subclasses' (which reset first).
    fn read(&mut self, e: roxmltree::Node) {
        let active = self.active;
        *self = Slot::new(self.id);
        self.active = active;
        if let Some(c) = e.children().find(|c| c.has_tag_name("curve")) {
            self.curve = c.text().unwrap_or_default().to_owned();
        }
        let int = |k: &str| e.attribute(k).map(|v| v.trim().parse::<i32>().unwrap_or(0));
        if self.id == "drawingangle" {
            if let Some(v) = int("fanCornersEnabled") {
                self.fan_corners_enabled = v != 0;
            }
            if let Some(v) = int("fanCornersStep") {
                self.fan_corners_step = v;
            }
            if let Some(v) = int("angleOffset") {
                self.angle_offset = v;
            }
            if let Some(v) = int("lockedAngleMode") {
                self.locked_angle_mode = v != 0;
            }
        }
        if self.id == "fade" {
            if let Some(v) = int("periodic") {
                self.periodic = v != 0;
            }
            if let Some(v) = int("length") {
                self.length = v;
            }
        }
    }

    /// `KisSensorData::write` and its subclasses', into `<tag …>…</tag>`
    /// with QDom's line breaks folded to spaces, as Krita's presets hold it.
    fn write(&self, tag: &str) -> String {
        let mut attrs = format!(" id=\"{}\"", self.id);
        if self.id == "drawingangle" {
            attrs += &format!(
                " fanCornersEnabled=\"{}\" fanCornersStep=\"{}\" angleOffset=\"{}\" lockedAngleMode=\"{}\"",
                self.fan_corners_enabled as i32, self.fan_corners_step, self.angle_offset, self.locked_angle_mode as i32
            );
        }
        if self.id == "fade" {
            attrs += &format!(" periodic=\"{}\" length=\"{}\"", self.periodic as i32, self.length);
        }
        if self.curve == DEFAULT_CURVE {
            format!("<{tag}{attrs}/>")
        } else {
            format!("<{tag}{attrs}> <curve>{}</curve> </{tag}>", self.curve)
        }
    }

    /// The sensor this slot paints with, or why it can't.
    pub fn kind(&self, option: &str) -> Result<SensorKind> {
        if self.id == "drawingangle" {
            if self.locked_angle_mode {
                bail!("{option}: a locked drawing angle is not supported yet");
            }
            return Ok(SensorKind::DrawingAngle {
                offset: self.angle_offset,
            });
        }
        if self.id == "fade" {
            return Ok(SensorKind::Fade {
                length: self.length,
                periodic: self.periodic,
            });
        }
        SensorKind::from_id(self.id).ok_or_else(|| anyhow!("{option}: the `{}` sensor is not supported yet", self.id))
    }
}

impl Preset {
    pub fn read(kpp: &[u8]) -> Result<Self> {
        // KisPaintOpPreset::loadFromDevice: a workaround for presets saved
        // with a CDATA section nested in another.
        let xml = preset_xml(kpp)?
            .replace("<curve><![CDATA[", "<curve>")
            .replace("]]></curve>", "</curve>");
        let version = png_text(kpp, "version")
            .map(|v| String::from_utf8_lossy(&v).trim().to_owned())
            .unwrap_or_default();
        let doc = parse_xml(&xml).context("parsing the preset XML")?;
        let root = doc.root_element();
        if root.tag_name().name() != "Preset" {
            bail!("not a brush preset");
        }
        let mut params = HashMap::new();
        let mut order = Vec::new();
        for p in root.children().filter(|n| n.has_tag_name("param")) {
            let Some(name) = p.attribute("name") else { continue };
            let text: String = p.children().filter_map(|c| c.text()).collect();
            let kind = p.attribute("type").unwrap_or("string").to_owned();
            let value = if kind == "bytearray" {
                base64(text.as_bytes())
            } else {
                text.into_bytes()
            };
            if params.insert(name.to_owned(), Param { kind, value }).is_none() {
                order.push(name.to_owned());
            }
        }
        let mut resources = Vec::new();
        if version == "5.0" {
            if let Some(list) = root.children().find(|n| n.has_tag_name("resources")) {
                for r in list.children().filter(|n| n.has_tag_name("resource")) {
                    let text: String = r.children().filter_map(|c| c.text()).collect();
                    let attr = |k: &str| r.attribute(k).unwrap_or_default().to_owned();
                    resources.push(EmbeddedResource {
                        kind: attr("type"),
                        name: attr("name"),
                        filename: attr("filename"),
                        md5sum: attr("md5sum"),
                        data: base64(text.as_bytes()),
                    });
                }
            }
        }
        Ok(Self {
            name: root.attribute("name").unwrap_or_default().to_owned(),
            paintop: root.attribute("paintopid").unwrap_or_default().to_owned(),
            version,
            params,
            order,
            resources,
        })
    }

    pub fn has(&self, key: &str) -> bool {
        self.params.contains_key(key)
    }

    #[cfg(test)]
    pub fn param(&self, key: &str) -> Option<&Param> {
        self.params.get(key)
    }

    pub fn string(&self, key: &str) -> Option<String> {
        self.params.get(key).map(|v| String::from_utf8_lossy(&v.value).into_owned())
    }

    pub fn bytes(&self, key: &str) -> Option<&[u8]> {
        self.params.get(key).map(|v| v.value.as_slice())
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

    /// A curve option, read the way `KisKritaSensorPack::read` reads it,
    /// down to its legacy quirks: a sensor the preset does not mention is
    /// reset, except that the old `Custom<id>` curve format keeps every
    /// sensor's state, which leaves pressure on beside whatever was read.
    pub fn curve_option(&self, spec: &OptionSpec) -> Result<CurveOption> {
        self.option_data(spec)?.curve_option(spec)
    }

    /// An option's data, without judging its sensors: see
    /// [`Preset::curve_option`].
    pub fn option_data(&self, spec: &OptionSpec) -> Result<OptionData> {
        let id = spec.id;
        let checked = !spec.checkable || self.bool(&format!("Pressure{id}"), false);

        let mut slots: Vec<Slot> = SLOTS.iter().map(|&s| Slot::new(s)).collect();
        // KisKritaSensorData(): pressure starts active.
        slots[0].active = true;
        let mut reset = vec![true; slots.len()];
        let mut common = String::new();

        let definition = self.string(&format!("{id}Sensor")).unwrap_or_default();
        let read_one = |e: roxmltree::Node, slots: &mut Vec<Slot>, reset: &mut Vec<bool>| -> Result<String> {
            let sid = e.attribute("id").unwrap_or_default();
            let i = SLOTS
                .iter()
                .position(|&s| s == sid)
                .ok_or_else(|| anyhow!("{id}: unknown sensor `{sid}`"))?;
            slots[i].read(e);
            slots[i].active = true;
            reset[i] = false;
            Ok(slots[i].curve.clone())
        };
        if definition.is_empty() {
            // noop
        } else if !definition.contains("sensorslist") {
            let doc = parse_xml(&definition).with_context(|| format!("{id}Sensor"))?;
            common = read_one(doc.root_element(), &mut slots, &mut reset)?;
        } else {
            let doc = parse_xml(&definition).with_context(|| format!("{id}Sensor"))?;
            let mut proposed = String::new();
            for child in doc.root_element().children().filter(|c| c.has_tag_name("ChildSensor")) {
                // Due to legacy reasons the common curve starts as the
                // last curve read.
                proposed = read_one(child, &mut slots, &mut reset)?;
            }
            common = if proposed.is_empty() { DEFAULT_CURVE.to_owned() } else { proposed };
        }

        let use_same = self.bool(&format!("{id}UseSameCurve"), true);
        // Only load the old curve format if the sensor didn't save its own.
        if !definition.contains("curve") {
            if self.bool(&format!("Custom{id}"), false) {
                common = self.string(&format!("Curve{id}")).unwrap_or_else(|| DEFAULT_CURVE.to_owned());
                for (s, r) in slots.iter_mut().zip(reset.iter_mut()) {
                    s.curve = common.clone();
                    *r = false;
                }
            } else {
                common = DEFAULT_CURVE.to_owned();
            }
        }
        if use_same {
            let fallback = if common.is_empty() { DEFAULT_CURVE.to_owned() } else { common.clone() };
            common = self.string(&format!("{id}commonCurve")).unwrap_or(fallback);
            if common.is_empty() {
                common = DEFAULT_CURVE.to_owned();
            }
        }
        for (s, r) in slots.iter_mut().zip(reset.iter()) {
            if *r {
                *s = Slot::new(s.id);
            }
        }
        // At least one sensor is always active.
        if !slots.iter().any(|s| s.active) {
            slots[0].active = true;
        }

        let mut strength = self.double(&format!("{id}Value"), spec.max);
        // KisScatterOptionData's fix-up for presets from before ScatterValue.
        if id == "Scatter" && self.has("Scattering/Amount") && !self.has("ScatterValue") {
            strength = self.double("Scattering/Amount", 0.0);
        }
        Ok(OptionData {
            checked,
            use_curve: self.bool(&format!("{id}UseCurve"), true),
            use_same_curve: use_same,
            strength,
            curve_mode: self.int(&format!("{id}curveMode"), 0),
            common_curve: common,
            slots,
        })
    }

    /// Set one parameter, keeping its declared type (a new one is typed
    /// as Krita would: numbers and booleans `internal`, text `string`).
    pub fn set(&mut self, key: &str, value: &str) {
        if let Some(p) = self.params.get_mut(key) {
            p.value = value.as_bytes().to_vec();
            return;
        }
        let internal = value == "true" || value == "false" || value.parse::<f64>().is_ok();
        let kind = if internal { "internal" } else { "string" };
        self.params.insert(
            key.to_owned(),
            Param {
                kind: kind.to_owned(),
                value: value.as_bytes().to_vec(),
            },
        );
        self.order.push(key.to_owned());
    }

    /// This preset with `overrides` (key, value) set over it.
    pub fn with_overrides(&self, overrides: &[(String, String)]) -> Preset {
        let mut p = self.clone();
        for (k, v) in overrides {
            p.set(k, v);
        }
        p
    }

    /// The `preset` XML, as `KisPaintOpPreset` saves it: each param in
    /// order, text in CDATA, byte arrays in base64, and (for a version 5.0
    /// preset) the files it carries.
    pub fn to_xml(&self) -> String {
        fn esc(s: &str) -> String {
            s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
        }
        let mut out = format!("<Preset paintopid=\"{}\" name=\"{}\">", esc(&self.paintop), esc(&self.name));
        for key in &self.order {
            let Some(p) = self.params.get(key) else { continue };
            let value = match p.kind.as_str() {
                "bytearray" => base64_encode(&p.value),
                "string" => format!("<![CDATA[{}]]>", String::from_utf8_lossy(&p.value)),
                _ => esc(&String::from_utf8_lossy(&p.value)),
            };
            out += &format!(" <param type=\"{}\" name=\"{}\">{value}</param>", esc(&p.kind), esc(key));
        }
        if !self.resources.is_empty() {
            out += " <resources>";
            for r in &self.resources {
                out += &format!(
                    " <resource type=\"{}\" md5sum=\"{}\" name=\"{}\" filename=\"{}\"><![CDATA[{}]]></resource>",
                    esc(&r.kind),
                    esc(&r.md5sum),
                    esc(&r.name),
                    esc(&r.filename),
                    base64_encode(&r.data)
                );
            }
            out += " </resources>";
        }
        out += " </Preset>";
        out
    }
}

impl OptionData {
    /// The option's sensors, or why one can't paint.
    pub fn curve_option(&self, spec: &OptionSpec) -> Result<CurveOption> {
        let id = spec.id;
        let mut sensors = Vec::new();
        let mut fan_corners = None;
        for s in self.slots.iter().filter(|s| s.active) {
            let kind = s.kind(id)?;
            let text = if self.use_same_curve { &self.common_curve } else { &s.curve };
            let curve = CubicCurve::parse(text).ok_or_else(|| anyhow!("{id}: bad curve `{text}`"))?;
            sensors.push(Sensor::new(kind, &curve));
            if s.id == "drawingangle" && s.fan_corners_enabled && !s.locked_angle_mode {
                fan_corners = Some(s.fan_corners_step as f64);
            }
        }
        Ok(CurveOption {
            checked: self.checked,
            use_curve: self.use_curve,
            curve_mode: self.curve_mode,
            strength: self.strength,
            min: spec.min,
            max: spec.max,
            sensors,
            fan_corners,
        })
    }

    /// `KisKritaSensorPack::write`: the params that say this.
    pub fn write(&self, spec: &OptionSpec) -> Vec<(String, String)> {
        let id = spec.id;
        let active: Vec<&Slot> = self.slots.iter().filter(|s| s.active).collect();
        let body = if active.len() == 1 {
            active[0].write("params")
        } else {
            let children: Vec<String> = active.iter().map(|s| s.write("ChildSensor")).collect();
            format!("<params id=\"sensorslist\"> {} </params>", children.join(" "))
        };
        vec![
            (format!("Pressure{id}"), (self.checked || !spec.checkable).to_string()),
            (format!("{id}Sensor"), format!("<!DOCTYPE params> {body} ")),
            (format!("{id}UseCurve"), self.use_curve.to_string()),
            (format!("{id}UseSameCurve"), self.use_same_curve.to_string()),
            (format!("{id}Value"), self.strength.to_string()),
            (format!("{id}curveMode"), self.curve_mode.to_string()),
            (format!("{id}commonCurve"), self.common_curve.clone()),
            // A default curve is left out of the sensor XML, which would
            // send a preset that still has the legacy `Custom<id>` key down
            // the old-format path on the next read.
            (format!("Custom{id}"), "false".to_owned()),
        ]
    }
}

/// `QByteArray::toBase64`.
pub(crate) fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

impl Preset {
}

#[cfg(test)]
mod tests {
    use super::super::options;
    use super::*;

    #[test]
    fn base64_is_lenient() {
        assert_eq!(base64(b"aGVsbG8="), b"hello");
        assert_eq!(base64(b"aGVs\nbG8"), b"hello");
        for s in [&b""[..], b"h", b"he", b"hel", b"hello"] {
            assert_eq!(base64(base64_encode(s).as_bytes()), s);
        }
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

    /// Writing an option back and reading it again changes nothing, for
    /// every option of every shipped preset: the editor's edits round-trip.
    #[test]
    fn options_write_back_as_they_read() {
        use options::*;
        for name in super::super::defaults::PRESETS {
            let p = Preset::read(&super::super::defaults::kpp(name).unwrap()).unwrap();
            for spec in [SIZE, RATIO, ROTATION, OPACITY, FLOW, SOFTNESS, SCATTER, TEXTURE_STRENGTH] {
                let data = p.option_data(&spec).unwrap();
                let written = p.with_overrides(&data.write(&spec));
                let again = written.option_data(&spec).unwrap();
                // Inactive slots are not written, so only the active ones,
                // and the curves that apply, have to survive.
                let active = |d: &OptionData| -> Vec<Slot> { d.slots.iter().filter(|s| s.active).cloned().collect() };
                assert_eq!(active(&again), active(&data), "{name} {}", spec.id);
                assert_eq!(
                    (again.checked || !spec.checkable, again.use_curve, again.use_same_curve, again.strength, again.curve_mode),
                    (data.checked || !spec.checkable, data.use_curve, data.use_same_curve, data.strength, data.curve_mode),
                    "{name} {}",
                    spec.id
                );
                if data.use_same_curve {
                    assert_eq!(again.common_curve, data.common_curve, "{name} {}", spec.id);
                }
            }
        }
    }

    /// The XML writer's output reads back as the same preset.
    #[test]
    fn the_preset_xml_reads_back() {
        let p = Preset::read(super::super::PENCIL5_KPP).unwrap();
        let mut edited = p.with_overrides(&[("FlowValue".into(), "0.5".into()), ("NewThing".into(), "a<b".into())]);
        edited.resources.push(EmbeddedResource {
            kind: "brushes".into(),
            name: "tip".into(),
            filename: "tip.png".into(),
            md5sum: "x".into(),
            data: vec![1, 2, 3],
        });
        let xml = edited.to_xml();
        let mut kpp = with_xml(&xml);
        let body = b"version\x005.0";
        let mut chunk = (body.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(body);
        chunk.extend_from_slice(&[0; 4]);
        kpp.splice(8..8, chunk);
        let back = Preset::read(&kpp).unwrap();
        assert_eq!(back.order, edited.order);
        for k in &back.order {
            assert_eq!(back.param(k).unwrap().value, edited.param(k).unwrap().value, "{k}");
            assert_eq!(back.param(k).unwrap().kind, edited.param(k).unwrap().kind, "{k}");
        }
        assert_eq!(back.param("NewThing").unwrap().kind, "string");
        assert_eq!(back.param("FlowValue").unwrap().kind, "internal");
        assert_eq!(back.resources[0].data, vec![1, 2, 3]);
        assert_eq!(back.name, p.name);
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
        assert_eq!(p.param("Texture/Pattern/Pattern").unwrap().kind, "bytearray");
        assert!(p.order.len() > 50);
    }

    /// A preset holding just `params`, for exercising the option reader.
    fn preset(params: &[(&str, &str)]) -> Preset {
        let mut map = HashMap::new();
        for (k, v) in params {
            map.insert(
                k.to_string(),
                Param {
                    kind: "string".into(),
                    value: v.as_bytes().to_vec(),
                },
            );
        }
        Preset {
            name: "t".into(),
            paintop: "paintbrush".into(),
            version: String::new(),
            params: map,
            order: Vec::new(),
            resources: Vec::new(),
        }
    }

    fn kinds(o: &CurveOption) -> Vec<SensorKind> {
        o.sensors.iter().map(|s| s.kind).collect()
    }

    #[test]
    fn a_sensor_list_reads_every_child_and_the_common_curve() {
        let p = preset(&[
            ("PressureSize", "true"),
            (
                "SizeSensor",
                r#"<params id="sensorslist"><ChildSensor id="speed"><curve>0,1;1,0;</curve></ChildSensor><ChildSensor id="pressure"/></params>"#,
            ),
            ("SizecurveMode", "1"),
        ]);
        let o = p.curve_option(&options::SIZE).unwrap();
        assert!(o.checked);
        assert_eq!(o.curve_mode, 1);
        // Sorted into generateSensors order, pressure first.
        assert_eq!(kinds(&o), vec![SensorKind::Pressure, SensorKind::Speed]);
        assert_eq!(o.strength, 1.0, "missing strength reads as the top of the range");
    }

    #[test]
    fn scatter_defaults_to_five_and_honours_the_old_amount() {
        let p = preset(&[("PressureScatter", "true")]);
        assert_eq!(p.curve_option(&options::SCATTER).unwrap().strength, 5.0);
        let p = preset(&[("PressureScatter", "true"), ("Scattering/Amount", "1.5")]);
        assert_eq!(p.curve_option(&options::SCATTER).unwrap().strength, 1.5);
    }

    #[test]
    fn the_drawing_angle_brings_its_offset_and_fan_corners() {
        let p = preset(&[
            ("PressureRotation", "true"),
            (
                "RotationSensor",
                r#"<params id="drawingangle" fanCornersEnabled="1" fanCornersStep="90" angleOffset="30" lockedAngleMode="0"/>"#,
            ),
        ]);
        let o = p.curve_option(&options::ROTATION).unwrap();
        assert_eq!(kinds(&o), vec![SensorKind::DrawingAngle { offset: 30 }]);
        assert_eq!(o.fan_corners, Some(90.0));

        let locked = preset(&[(
            "RotationSensor",
            r#"<params id="drawingangle" lockedAngleMode="1"/>"#,
        )]);
        let err = locked.curve_option(&options::ROTATION).err().unwrap();
        assert!(err.to_string().contains("locked"), "{err}");
    }

    #[test]
    fn the_old_custom_curve_format_leaves_pressure_on() {
        // Krita's legacy branch keeps every slot's state, so the sensor
        // read sits beside the pressure that was active from the start.
        let p = preset(&[
            ("SizeSensor", r#"<params id="speed"/>"#),
            ("CustomSize", "true"),
            ("CurveSize", "0,0;1,0.5;"),
        ]);
        let o = p.curve_option(&options::SIZE).unwrap();
        assert_eq!(kinds(&o), vec![SensorKind::Pressure, SensorKind::Speed]);
    }

    #[test]
    fn unsupported_sensors_are_refused_by_name() {
        let p = preset(&[("SizeSensor", r#"<params id="distance"/>"#)]);
        let err = p.curve_option(&options::SIZE).err().unwrap();
        assert!(err.to_string().contains("distance"), "{err}");
    }
}
