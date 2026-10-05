//! Changing a preset without rewriting it: an edit is a list of
//! `(param, value)` overrides set over the preset's own params, kept in the
//! brush settings. These helpers turn the editor's controls into overrides,
//! leaving out any that only restate what the preset already says, so an
//! untouched preset has none and Reset is clearing the list.

use super::kpp::Preset;

/// Set `key` to `value` in `overrides`, or drop the override when the
/// preset already says that.
pub fn set(base: &Preset, overrides: &mut Vec<(String, String)>, key: &str, value: String) {
    overrides.retain(|(k, _)| k != key);
    if base.string(key).as_deref() != Some(value.as_str()) {
        overrides.push((key.to_owned(), value));
    }
}

/// Set several at once.
pub fn set_all(base: &Preset, overrides: &mut Vec<(String, String)>, pairs: Vec<(String, String)>) {
    for (k, v) in pairs {
        set(base, overrides, &k, v);
    }
}

/// A number as Krita writes one (`QString::number`): shortest form, no
/// trailing zeros.
pub fn num(v: f64) -> String {
    let v = if v == 0.0 { 0.0 } else { v };
    let s = format!("{v}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// The first `<element>` start tag in `xml`: its byte range.
fn start_tag(xml: &str, element: &str) -> Option<(usize, usize)> {
    let open = format!("<{element}");
    let mut from = 0;
    while let Some(i) = xml[from..].find(&open) {
        let at = from + i;
        let after = xml[at + open.len()..].chars().next()?;
        if after.is_whitespace() || after == '>' || after == '/' {
            let end = at + xml[at..].find('>')?;
            return Some((at, end));
        }
        from = at + open.len();
    }
    None
}

/// `attr` of the first `<element>` in `xml`.
pub fn attr(xml: &str, element: &str, attr: &str) -> Option<String> {
    let (s, e) = start_tag(xml, element)?;
    let tag = &xml[s..e];
    let key = format!(" {attr}=\"");
    let i = tag.find(&key)? + key.len();
    let j = tag[i..].find('"')?;
    Some(tag[i..i + j].to_owned())
}

/// `xml` with `attr` of the first `<element>` set to `value`, added if
/// missing. Krita's tip definitions are one or two flat elements, so this
/// is all the XML editing they need.
pub fn set_attr(xml: &str, element: &str, attr: &str, value: &str) -> String {
    let Some((s, e)) = start_tag(xml, element) else { return xml.to_owned() };
    let tag = &xml[s..e];
    let key = format!(" {attr}=\"");
    let value = value.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
    let new_tag = match tag.find(&key) {
        Some(i) => {
            let v0 = i + key.len();
            let v1 = v0 + tag[v0..].find('"').unwrap_or(tag.len() - v0);
            format!("{}{value}{}", &tag[..v0], &tag[v1..])
        }
        None => {
            let body = tag.strip_suffix('/').map(|t| t.trim_end());
            match body {
                Some(b) => format!("{b} {attr}=\"{value}\"/"),
                None => format!("{tag} {attr}=\"{value}\""),
            }
        }
    };
    format!("{}{new_tag}{}", &xml[..s], &xml[e..])
}

/// A generated round tip of `diameter`, with Krita's defaults otherwise,
/// keeping the spacing the old tip had.
pub fn auto_tip_definition(diameter: f64, spacing: &str, auto_spacing: &str, coeff: &str) -> String {
    format!(
        "<Brush type=\"auto_brush\" useAutoSpacing=\"{auto_spacing}\" autoSpacingCoeff=\"{coeff}\" angle=\"0\" spacing=\"{spacing}\" BrushVersion=\"2\" density=\"1\" randomness=\"0\"> <MaskGenerator ratio=\"1\" vfade=\"0.5\" spikes=\"2\" type=\"circle\" hfade=\"0.5\" antialiasEdges=\"1\" diameter=\"{}\" id=\"default\"/> </Brush> ",
        num(diameter)
    )
}

/// A bitmap tip from `file`, keeping the old spacing.
pub fn bitmap_tip_definition(file: &str, spacing: &str, auto_spacing: &str, coeff: &str) -> String {
    let kind = if file.to_ascii_lowercase().ends_with(".png") { "png_brush" } else { "gbr_brush" };
    format!(
        "<Brush scale=\"1\" type=\"{kind}\" useAutoSpacing=\"{auto_spacing}\" autoSpacingCoeff=\"{coeff}\" angle=\"0\" spacing=\"{spacing}\" BrushVersion=\"2\" filename=\"{file}\"/> "
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEF: &str = r#"<Brush type="auto_brush" angle="0" spacing="0.1"> <MaskGenerator diameter="60" ratio="1" id="gauss"/> </Brush> "#;

    #[test]
    fn attributes_read_and_write_in_place() {
        assert_eq!(attr(DEF, "Brush", "spacing").as_deref(), Some("0.1"));
        assert_eq!(attr(DEF, "MaskGenerator", "id").as_deref(), Some("gauss"));
        assert_eq!(attr(DEF, "MaskGenerator", "spikes"), None);
        let d = set_attr(DEF, "MaskGenerator", "ratio", "0.5");
        assert_eq!(attr(&d, "MaskGenerator", "ratio").as_deref(), Some("0.5"));
        assert_eq!(attr(&d, "Brush", "spacing").as_deref(), Some("0.1"), "the other element untouched");
        let d = set_attr(&d, "MaskGenerator", "spikes", "5");
        assert!(d.contains(r#"id="gauss" spikes="5"/>"#), "{d}");
        let d = set_attr(&d, "Brush", "density", "0.8");
        assert!(d.starts_with(r#"<Brush type="auto_brush" angle="0" spacing="0.1" density="0.8">"#), "{d}");
        assert!(roxmltree::Document::parse(d.trim()).is_ok());
    }

    #[test]
    fn numbers_print_like_qt() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.25), "0.25");
        assert_eq!(num(156.25), "156.25");
        assert_eq!(num(-0.0), "0");
    }

    #[test]
    fn overrides_that_restate_the_preset_are_dropped() {
        let base = Preset::read(super::super::PENCIL5_KPP).unwrap();
        let mut o = Vec::new();
        set(&base, &mut o, "FlowValue", "0.5".into());
        assert_eq!(o, vec![("FlowValue".to_owned(), "0.5".to_owned())]);
        set(&base, &mut o, "FlowValue", base.string("FlowValue").unwrap());
        assert!(o.is_empty());
    }

    #[test]
    fn new_tip_definitions_parse() {
        for d in [auto_tip_definition(12.5, "0.1", "0", "1"), bitmap_tip_definition("a.gih", "0.25", "1", "0.8")] {
            assert!(roxmltree::Document::parse(d.trim()).is_ok(), "{d}");
        }
        assert_eq!(attr(&bitmap_tip_definition("a.gih", "0.25", "1", "0.8"), "Brush", "type").as_deref(), Some("gbr_brush"));
    }
}
