//! The XML parts of a `.kra`: `maindoc.xml`, per-layer `*.keyframes.xml`,
//! `animation/index.xml` and `documentinfo.xml`. Written with `format!` —
//! the shapes are fixed and small — and read back with `roxmltree` in
//! `super::read`.

use std::fmt::Write;

/// One entry of the layer stack.
pub enum Item<'a> {
    /// A paint layer of ours.
    Layer(LayerXml<'a>),
    /// A `<layer>` element copied verbatim from a Krita file (see
    /// `super::Carried`).
    Raw(&'a str),
}

/// One `<layer>` element of ours. Only paint layers are ever written.
pub struct LayerXml<'a> {
    pub filename: &'a str,
    pub name: &'a str,
    pub uuid: &'a str,
    pub opacity: u8,
    pub visible: bool,
    pub locked: bool,
    pub selected: bool,
}

/// Parse XML the way Krita writes it: every part carries a `<!DOCTYPE>`,
/// which roxmltree refuses unless asked.
pub fn parse(text: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error> {
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(text, opts)
}

/// Escape text for use inside a double-quoted attribute.
pub fn attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            c => out.push(c),
        }
    }
    out
}

/// The image profile's name. Krita registers LittleCMS's sRGB under this name
/// at startup, and the bytes `super::SRGB_ICC` carries are that profile.
pub const PROFILE: &str = "sRGB built-in";

/// The kritaVersion we claim. Old enough that any Krita 5 reads it without a
/// "newer version" warning, new enough to promise cloned keyframes.
const KRITA_VERSION: &str = "5.0.0";

/// `layers` is in XML order: **top of the stack first**.
pub fn maindoc(image: &str, width: u32, height: u32, layers: &[Item]) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<!DOCTYPE DOC PUBLIC '-//KDE//DTD krita 2.0//EN' 'http://www.calligra.org/DTD/krita-2.0.dtd'>\n");
    let _ = writeln!(
        s,
        "<DOC xmlns=\"http://www.calligra.org/DTD/krita\" editor=\"Krita\" kritaVersion=\"{KRITA_VERSION}\" syntaxVersion=\"2.0\">"
    );
    let _ = writeln!(
        s,
        " <IMAGE mime=\"application/x-kra\" name=\"{}\" width=\"{width}\" height=\"{height}\" x-res=\"72\" y-res=\"72\" description=\"\" colorspacename=\"RGBA\" profile=\"{PROFILE}\">",
        attr(image)
    );
    s.push_str("  <layers>\n");
    for item in layers {
        let l = match item {
            Item::Layer(l) => l,
            Item::Raw(xml) => {
                let _ = writeln!(s, "   {xml}");
                continue;
            }
        };
        let _ = writeln!(
            s,
            "   <layer nodetype=\"paintlayer\" filename=\"{}\" name=\"{}\" uuid=\"{}\" opacity=\"{}\" visible=\"{}\" locked=\"{}\" x=\"0\" y=\"0\" compositeop=\"normal\" colorspacename=\"RGBA\" channelflags=\"\" channellockflags=\"\" collapsed=\"0\" colorlabel=\"0\" onionskin=\"0\" intimeline=\"1\" keyframes=\"{}.keyframes.xml\"{}/>",
            attr(l.filename),
            attr(l.name),
            attr(l.uuid),
            l.opacity,
            l.visible as u8,
            l.locked as u8,
            attr(l.filename),
            if l.selected { " selected=\"true\"" } else { "" },
        );
    }
    s.push_str("  </layers>\n");
    s.push_str(" </IMAGE>\n</DOC>\n");
    s
}

/// A raster layer's content channel. `keys` is `(time, frame file name)`; a
/// file named twice is a cloned keyframe (Krita 5).
pub fn keyframes(keys: &[(usize, String)]) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<!DOCTYPE keyframes PUBLIC '-//KDE//DTD krita-keyframes 1.0//EN' 'http://www.calligra.org/DTD/krita-keyframes-1.0.dtd'>\n");
    s.push_str("<keyframes>\n <channel name=\"content\">\n");
    for (time, frame) in keys {
        let _ = writeln!(
            s,
            "  <keyframe time=\"{time}\" color-label=\"0\" frame=\"{}\">\n   <offset type=\"point\" x=\"0\" y=\"0\"/>\n  </keyframe>",
            attr(frame)
        );
    }
    s.push_str(" </channel>\n</keyframes>\n");
    s
}

pub fn animation_index(fps: f32, last_frame: usize, current: usize) -> String {
    // Krita stores an integer frame rate.
    let fps = fps.round().max(1.0) as u32;
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE animation-metadata PUBLIC '-//KDE//DTD krita 1.1//EN' 'http://www.calligra.org/DTD/krita-1.1.dtd'>\n\
<animation-metadata xmlns=\"http://www.calligra.org/DTD/krita\">\n \
<framerate type=\"value\" value=\"{fps}\"/>\n \
<range type=\"timerange\" from=\"0\" to=\"{last_frame}\"/>\n \
<currentTime type=\"value\" value=\"{current}\"/>\n\
</animation-metadata>\n"
    )
}

pub fn documentinfo() -> &'static str {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE document-info PUBLIC '-//KDE//DTD document-info 1.1//EN' 'http://www.calligra.org/DTD/document-info-1.1.dtd'>\n\
<document-info xmlns=\"http://www.calligra.org/DTD/document-info\">\n \
<about>\n  <title></title>\n  <editing-cycles>1</editing-cycles>\n </about>\n \
<author/>\n\
</document-info>\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attr_escapes_quotes_and_markup() {
        assert_eq!(attr(r#"a "b" <c> & 'd'"#), "a &quot;b&quot; &lt;c&gt; &amp; &apos;d&apos;");
    }

    #[test]
    fn maindoc_parses_and_marks_selection() {
        let layers = [Item::Layer(LayerXml {
            filename: "layer1",
            name: "Ink & \"line\"",
            uuid: "{00000000-0000-4000-8000-000000000001}",
            opacity: 200,
            visible: true,
            locked: false,
            selected: true,
        })];
        let xml = maindoc("image", 640, 360, &layers);
        let doc = parse(&xml).unwrap();
        let layer = doc.descendants().find(|n| n.has_tag_name("layer")).unwrap();
        assert_eq!(layer.attribute("name"), Some("Ink & \"line\""));
        assert_eq!(layer.attribute("selected"), Some("true"));
        assert_eq!(layer.attribute("keyframes"), Some("layer1.keyframes.xml"));
    }
}
