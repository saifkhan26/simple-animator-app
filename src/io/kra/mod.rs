//! Krita `.kra` — write the whole project as an animated Krita document, and
//! read one back.
//!
//! A `.kra` is a zip: `mimetype` (stored, first), `maindoc.xml` (image size +
//! layer stack), `<image>/animation/index.xml` (fps, range, playhead), and per
//! paint layer a `*.keyframes.xml` plus one tile file per drawing
//! (`layerN.fK`, see [`tiles`]). A Krita raster keyframe holds until the next
//! one — exactly our key / hold exposure model — and a keyframe file used at
//! several times is a Krita 5 cloned frame, which is how a cell repeated
//! within a layer travels.
//!
//! What Krita can't hold (layer transforms, camera, tracker points, the
//! reference flag) is not written: Krita edits the drawings, this app keeps
//! the staging. Cells are placed the way the identity [`Transform`] places
//! them — centered on the frame — so what Krita shows lines up with the
//! drawing as it was painted.
//!
//! [`Transform`]: crate::doc::transform::Transform

pub mod tiles;
mod xml;

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use anyhow::{anyhow, bail, Context, Result};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::doc::canvas::Canvas;
use crate::doc::layer::CellId;
use crate::doc::project::Project;
use crate::io::composite;

/// Folder name inside the zip. Krita takes it from `IMAGE@name`; any name works.
const IMAGE: &str = "image";
const MIMETYPE: &[u8] = b"application/x-krita";

/// The sRGB profile every layer and the image carry — our pixels are sRGB.
/// Byte-for-byte what Krita embeds for its "sRGB built-in" profile, which is
/// LittleCMS's `cmsCreate_sRGBProfile()` (its copyright tag reads "No
/// copyright, use freely"). Named in `maindoc.xml` as [`xml::PROFILE`].
const SRGB_ICC: &[u8] = include_bytes!("srgb-built-in.icc");

pub struct WriteOpts<'a> {
    /// One `{uuid}` per project layer, bottom first.
    pub layer_uuids: &'a [String],
    /// The layer Krita opens with selected.
    pub selected_layer: usize,
    /// Layers of the file being replaced that go back in untouched (see
    /// [`carry`]).
    pub carried: &'a [Carried],
}

/// Our layers' file names: `anim1`, `anim2`, … Krita names its own
/// `layer<N>`, so layers carried over from a Krita save (which keep Krita's
/// names) can never clash with ours.
const OUR_PREFIX: &str = "anim";

/// A layer — or a whole group — of a Krita file, carried untouched into a
/// rewrite of that file: its `<layer>` element exactly as Krita wrote it, and
/// every file under `layers/` it names (pixels, frames, keyframes, profile,
/// vector shapes, masks…). Nothing is decoded, so any layer type survives.
pub struct Carried {
    pub name: String,
    xml: String,
    /// `(path under layers/, bytes)`.
    files: Vec<(String, Vec<u8>)>,
    /// Uuids of the layers and groups under it in Krita's stack, nearest
    /// first (a group before what's in it): it goes back in just above the
    /// first of them the rewrite still has beside it.
    below: Vec<String>,
    /// The uuid of the group it sat in — the outermost one, as groups fold
    /// on reading — so it goes back inside it while the rewrite has it.
    group: Option<String>,
}

/// The layers of a Krita file that `keep(name, groups)` rejects — the same
/// filter as [`read_filtered`] — lifted out whole, ready for
/// [`WriteOpts::carried`]. A rejected group comes out as one piece.
pub fn carry(bytes: Vec<u8>, keep: &dyn Fn(&str, &[String]) -> bool) -> Result<Vec<Carried>> {
    let zip = ZipArchive::new(Cursor::new(bytes)).context("not a zip file")?;
    let mut r = Reader { zip };
    let maindoc = r.text("maindoc.xml")?;
    let doc = xml::parse(&maindoc).context("parsing maindoc.xml")?;
    let image = doc
        .descendants()
        .find(|n| n.has_tag_name("IMAGE"))
        .context("maindoc.xml has no IMAGE")?;
    let dir = format!("{}/layers/", image.attribute("name").unwrap_or(""));

    // The stack top first, as the XML lists it: each rejected subtree once,
    // and the paint layers and groups that stay, in order (a group before
    // what's in it).
    enum Seen {
        Carried(usize),
        Kept(String),
    }
    struct Walk<'a> {
        keep: &'a dyn Fn(&str, &[String]) -> bool,
        src: &'a str,
        groups: Vec<String>,
        /// The uuid of each group in `groups`.
        group_uuids: Vec<String>,
        seen: Vec<Seen>,
        out: Vec<(String, String, Vec<String>, Option<String>)>,
    }
    fn walk(layers: roxmltree::Node, w: &mut Walk) {
        for n in layers.children().filter(|n| n.has_tag_name("layer")) {
            let name = n.attribute("name").unwrap_or("").to_string();
            let uuid = n.attribute("uuid").unwrap_or("").to_string();
            if !(w.keep)(&name, &w.groups) {
                let files =
                    n.descendants().filter_map(|d| d.attribute("filename")).map(str::to_string).collect();
                w.out.push((name, w.src[n.range()].to_string(), files, w.group_uuids.first().cloned()));
                w.seen.push(Seen::Carried(w.out.len() - 1));
                continue;
            }
            match n.attribute("nodetype").unwrap_or("") {
                "paintlayer" => w.seen.push(Seen::Kept(uuid)),
                "grouplayer" => {
                    w.seen.push(Seen::Kept(uuid.clone()));
                    if let Some(inner) = n.children().find(|c| c.has_tag_name("layers")) {
                        w.groups.push(name);
                        w.group_uuids.push(uuid);
                        walk(inner, w);
                        w.groups.pop();
                        w.group_uuids.pop();
                    }
                }
                _ => {}
            }
        }
    }
    let top = image
        .children()
        .find(|n| n.has_tag_name("layers"))
        .context("maindoc.xml has no layers")?;
    let mut w =
        Walk { keep, src: &maindoc, groups: Vec::new(), group_uuids: Vec::new(), seen: Vec::new(), out: Vec::new() };
    walk(top, &mut w);
    let Walk { seen, out, .. } = w;

    let entries: Vec<String> = r.zip.file_names().map(str::to_string).collect();
    let mut carried = Vec::with_capacity(out.len());
    for (i, (name, xml, filenames, group)) in out.into_iter().enumerate() {
        let at = seen.iter().position(|s| matches!(s, Seen::Carried(j) if *j == i)).unwrap_or(0);
        let below = seen[at + 1..]
            .iter()
            .filter_map(|s| match s {
                Seen::Kept(u) => Some(u.clone()),
                Seen::Carried(_) => None,
            })
            .collect();
        // A layer named `layer5` owns `layer5`, `layer5.f3`,
        // `layer5.keyframes.xml`, `layer5.shapelayer/…` — not `layer50`.
        let owns = |rel: &str| {
            filenames.iter().any(|f| {
                rel == f || rel.strip_prefix(f.as_str()).is_some_and(|t| t.starts_with('.') || t.starts_with('/'))
            })
        };
        let mut files = Vec::new();
        for e in &entries {
            if let Some(rel) = e.strip_prefix(&dir).filter(|rel| owns(rel)) {
                files.push((rel.to_string(), r.bytes(e)?));
            }
        }
        carried.push(Carried { name, xml, files, below, group });
    }
    Ok(carried)
}

/// Top-left, in frame coordinates, of a `cw`×`ch` cell under the identity
/// transform (centered on a `pw`×`ph` frame). Floored, so an odd size
/// difference lands on the same pixel both ways through the round trip.
pub fn cell_origin(pw: u32, ph: u32, cw: u32, ch: u32) -> (i32, i32) {
    (
        (pw as i32 - cw as i32).div_euclid(2),
        (ph as i32 - ch as i32).div_euclid(2),
    )
}

/// Serialize `project` as an animated `.kra`.
pub fn write(project: &Project, opts: &WriteOpts) -> Result<Vec<u8>> {
    if opts.layer_uuids.len() != project.layers.len() {
        bail!("one uuid per layer expected");
    }
    let (pw, ph) = (project.width, project.height);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    // Level 1: the payload is raw tiles that are mostly zeros, same trade as
    // the `.anim` writer.
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));

    // Krita sniffs the first entry, uncompressed, like ODF.
    zip.start_file("mimetype", stored)?;
    zip.write_all(MIMETYPE)?;

    // Our file names, stepping over any a carried layer already owns — a
    // Krita save names its own `layerN`, but nothing else guarantees it.
    let taken = |name: &str| {
        opts.carried.iter().flat_map(|c| &c.files).any(|(rel, _)| {
            rel == name || rel.strip_prefix(name).is_some_and(|t| t.starts_with('.') || t.starts_with('/'))
        })
    };
    let mut names: Vec<String> = Vec::with_capacity(project.layers.len());
    let mut n = 0;
    while names.len() < project.layers.len() {
        n += 1;
        let name = format!("{OUR_PREFIX}{n}");
        if !taken(&name) {
            names.push(name);
        }
    }
    // Carried layers go back where they were: inside their group while it's
    // still a group here, else among the top-level items; there, above the
    // nearest item under them that's still beside them — or above the group
    // that item has since gone into — else at the bottom.
    let index_of = |u: &String| opts.layer_uuids.iter().position(|x| x == u);
    let home = |c: &Carried| c.group.as_ref().and_then(index_of).filter(|&g| project.is_group(g));
    let anchor = |c: &Carried| {
        let beside = |i: usize| match home(c) {
            Some(g) => (project.group_of(i) == Some(g)).then_some(i),
            None => Some(project.group_of(i).unwrap_or(i)),
        };
        c.below.iter().filter_map(index_of).find_map(beside)
    };
    let above = |i: usize| opts.carried.iter().filter(move |k| anchor(k) == Some(i)).map(|k| xml::Item::Raw(&k.xml));
    let own: Vec<xml::LayerXml> = project
        .layers
        .iter()
        .enumerate()
        .map(|(i, l)| xml::LayerXml {
            filename: &names[i],
            name: &l.name,
            uuid: &opts.layer_uuids[i],
            opacity: opacity_u8(l.opacity),
            visible: l.visible,
            locked: l.locked,
            selected: i == opts.selected_layer,
        })
        .collect();
    // Each layer is preceded by whatever carried layers sat just above it —
    // in the same group as it, if it's in one.
    let mut stack: Vec<xml::Item> = Vec::new();
    let mut i = project.layers.len();
    while i > 0 {
        i -= 1;
        stack.extend(above(i));
        if project.is_group(i) {
            let kids = project.children(i);
            let mut inner = Vec::new();
            for c in kids.clone().rev() {
                inner.extend(above(c));
                inner.push(xml::Item::Layer(own[c]));
            }
            inner.extend(
                opts.carried.iter().filter(|k| home(k) == Some(i) && anchor(k).is_none()).map(|k| xml::Item::Raw(&k.xml)),
            );
            stack.push(xml::Item::Group(own[i], project.layers[i].collapsed, inner));
            i = kids.start;
            continue;
        }
        stack.push(xml::Item::Layer(own[i]));
    }
    stack.extend(
        opts.carried.iter().filter(|c| home(c).is_none() && anchor(c).is_none()).map(|c| xml::Item::Raw(&c.xml)),
    );
    zip.start_file("maindoc.xml", deflated)?;
    zip.write_all(xml::maindoc(IMAGE, pw, ph, &stack).as_bytes())?;
    zip.start_file("documentinfo.xml", deflated)?;
    zip.write_all(xml::documentinfo().as_bytes())?;

    let empty = tiles::encode(&[], 0, 0, 0, 0);
    for (i, layer) in project.layers.iter().enumerate() {
        // A group has no pixels of its own to write.
        if layer.is_group() {
            continue;
        }
        let base = &names[i];
        // Each distinct cell becomes one frame file; a repeat reuses it.
        // `None` is the blank drawing.
        //
        // The first drawing *is* the layer's main file (`layerN`), as Krita
        // writes it: a layer with a single keyframe is loaded from the main
        // file alone, whatever its keyframe names — a drawing kept anywhere
        // else opens as an empty layer.
        let name = |n: usize| if n == 0 { base.clone() } else { format!("{base}.f{n}") };
        let mut frames: Vec<(String, Option<CellId>)> = Vec::new();
        let mut file_of: HashMap<CellId, usize> = HashMap::new();
        let mut keys: Vec<(usize, String)> = Vec::new();
        for (f, e) in layer.exposures.iter().enumerate().take(project.frame_count) {
            let Some(id) = *e else { continue };
            if project.cell(id).is_none() {
                continue;
            }
            if keys.is_empty() && f > 0 {
                // Nothing shows before our first key; say so explicitly
                // rather than rely on how Krita treats time before its first
                // keyframe.
                frames.push((name(0), None));
                keys.push((0, name(0)));
            }
            let i = *file_of.entry(id).or_insert_with(|| {
                frames.push((name(frames.len()), Some(id)));
                frames.len() - 1
            });
            keys.push((f, frames[i].0.clone()));
        }
        if keys.is_empty() {
            frames.push((name(0), None));
            keys.push((0, name(0)));
        }

        for (file, id) in &frames {
            let data = match id.and_then(|id| project.cell(id)) {
                Some(c) => {
                    let (ox, oy) = cell_origin(pw, ph, c.width, c.height);
                    tiles::encode(&c.pixels(), c.width, c.height, ox, oy)
                }
                None => empty.clone(),
            };
            zip.start_file(format!("{IMAGE}/layers/{file}"), deflated)?;
            zip.write_all(&data)?;
            zip.start_file(format!("{IMAGE}/layers/{file}.defaultpixel"), deflated)?;
            zip.write_all(&[0; 4])?;
        }
        // Krita reports a layer without its own profile as a load problem.
        zip.start_file(format!("{IMAGE}/layers/{base}.icc"), deflated)?;
        zip.write_all(SRGB_ICC)?;
        zip.start_file(format!("{IMAGE}/layers/{base}.keyframes.xml"), deflated)?;
        zip.write_all(xml::keyframes(&keys).as_bytes())?;
    }
    let mut copied = std::collections::HashSet::new();
    for (rel, bytes) in opts.carried.iter().flat_map(|c| &c.files) {
        if copied.insert(rel.as_str()) {
            zip.start_file(format!("{IMAGE}/layers/{rel}"), deflated)?;
            zip.write_all(bytes)?;
        }
    }
    zip.start_file(format!("{IMAGE}/annotations/icc"), deflated)?;
    zip.write_all(SRGB_ICC)?;

    let last = project.frame_count.saturating_sub(1);
    zip.start_file(format!("{IMAGE}/animation/index.xml"), deflated)?;
    zip.write_all(xml::animation_index(project.fps, last, project.current_frame.min(last)).as_bytes())?;

    // Thumbnails for file browsers and other apps; Krita itself reads layers.
    let merged = composite::flatten_frame(project, project.current_frame.min(last));
    let img = image::RgbaImage::from_raw(merged.width, merged.height, merged.into_pixels())
        .context("flattened frame has the wrong size")?;
    zip.start_file("mergedimage.png", deflated)?;
    zip.write_all(&png_bytes(&img)?)?;
    let thumb = image::imageops::thumbnail(&img, 256.min(img.width()), 256.min(img.height()));
    zip.start_file("preview.png", deflated)?;
    zip.write_all(&png_bytes(&thumb)?)?;

    Ok(zip.finish()?.into_inner())
}

fn png_bytes(img: &image::RgbaImage) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

pub fn opacity_u8(o: f32) -> u8 {
    (o.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A Krita drawing: its tiles' bounding box in Krita canvas coordinates, as
/// RGBA8. `default` fills everything outside the box (Krita's "fill layer
/// with colour" background is a non-transparent default pixel, not tiles).
pub struct Drawing {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
    pub default: [u8; 4],
}

impl Drawing {
    /// Rasterize into a `cw`×`ch` cell whose top-left sits at Krita canvas
    /// `(cx, cy)`. The flag is true when non-transparent paint fell outside
    /// the cell and was clipped.
    pub fn place(&self, cx: i32, cy: i32, cw: u32, ch: u32) -> (Canvas, bool) {
        let mut c = Canvas::new(cw, ch);
        if self.default != [0; 4] {
            for px in c.pixels_mut().chunks_exact_mut(4) {
                px.copy_from_slice(&self.default);
            }
        }
        let mut clipped = false;
        for y in 0..self.h as i32 {
            let ty = self.y + y - cy;
            let row = &self.pixels[(y as usize * self.w as usize * 4)..][..self.w as usize * 4];
            if !(0..ch as i32).contains(&ty) {
                clipped |= row.chunks_exact(4).any(|p| p[3] != 0);
                continue;
            }
            let x0 = (cx - self.x).clamp(0, self.w as i32);
            let x1 = (cx + cw as i32 - self.x).clamp(0, self.w as i32);
            if x0 > 0 {
                clipped |= row[..x0 as usize * 4].chunks_exact(4).any(|p| p[3] != 0);
            }
            if (x1 as u32) < self.w {
                clipped |= row[x1 as usize * 4..].chunks_exact(4).any(|p| p[3] != 0);
            }
            if x1 > x0 {
                let dst = ((ty as usize * cw as usize) + (self.x + x0 - cx) as usize) * 4;
                let n = (x1 - x0) as usize * 4;
                c.pixels_mut()[dst..dst + n].copy_from_slice(&row[x0 as usize * 4..x0 as usize * 4 + n]);
            }
        }
        (c, clipped)
    }
}

pub struct KraLayer {
    pub uuid: String,
    pub name: String,
    /// Krita's 0–255 opacity.
    pub opacity: u8,
    pub visible: bool,
    pub locked: bool,
    pub drawings: Vec<Drawing>,
    /// `(time, index into drawings)`, sorted by time.
    pub keys: Vec<(usize, usize)>,
    /// A group's header — no drawings — with its layers listed just below
    /// it in [`KraDoc::layers`].
    pub group: bool,
    /// One of a group's layers. Groups inside groups are folded into the
    /// outermost one.
    pub in_group: bool,
    /// A group shown folded in Krita.
    pub collapsed: bool,
}

pub struct KraDoc {
    pub width: u32,
    pub height: u32,
    pub fps: Option<f32>,
    /// Last frame of Krita's playback range.
    pub range_end: Option<usize>,
    /// Layers, bottom first: paint layers, and a group's header just above
    /// its layers.
    pub layers: Vec<KraLayer>,
    /// Things read but not carried over — unsupported layer types, blend
    /// modes, masks — worded for a toast.
    pub warnings: Vec<String>,
    /// `(uuid, name)` of paint layers the caller's filter kept out (see
    /// [`read_filtered`]). Not decoded.
    pub held: Vec<(String, String)>,
}

struct Reader {
    zip: ZipArchive<Cursor<Vec<u8>>>,
}

impl Reader {
    fn bytes(&mut self, name: &str) -> Result<Vec<u8>> {
        let mut f = self.zip.by_name(name).with_context(|| format!("missing {name}"))?;
        let mut out = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut out)?;
        Ok(out)
    }

    fn opt_bytes(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        if self.zip.index_for_name(name).is_none() {
            return Ok(None);
        }
        self.bytes(name).map(Some)
    }

    fn text(&mut self, name: &str) -> Result<String> {
        String::from_utf8(self.bytes(name)?).with_context(|| format!("{name} is not UTF-8"))
    }

    /// Default pixel file (BGRA) as RGBA; transparent when absent.
    fn default_pixel(&mut self, name: &str) -> Result<[u8; 4]> {
        Ok(match self.opt_bytes(name)? {
            Some(b) if b.len() >= 4 => [b[2], b[1], b[0], b[3]],
            _ => [0; 4],
        })
    }

    fn drawing(&mut self, file: &str, fallback_default: [u8; 4], dx: i32, dy: i32) -> Result<Drawing> {
        let default = match self.opt_bytes(&format!("{file}.defaultpixel"))? {
            Some(b) if b.len() >= 4 => [b[2], b[1], b[0], b[3]],
            _ => fallback_default,
        };
        let data = self.bytes(file)?;
        Ok(match tiles::decode(&data, default).with_context(|| format!("decoding {file}"))? {
            Some(d) => Drawing { x: d.x + dx, y: d.y + dy, w: d.w, h: d.h, pixels: d.pixels, default },
            None => Drawing { x: 0, y: 0, w: 0, h: 0, pixels: Vec::new(), default },
        })
    }
}

fn attr_i32(n: roxmltree::Node, name: &str) -> i32 {
    n.attribute(name).and_then(|v| v.trim().parse().ok()).unwrap_or(0)
}

/// Read a `.kra`, every layer. The app always reads through a filter (see
/// `krita_link::comes_back`); this is the unfiltered view tests check against.
#[cfg(test)]
pub fn read(bytes: Vec<u8>) -> Result<KraDoc> {
    read_filtered(bytes, &|_, _| true)
}

/// Read a `.kra`, keeping only the layers `keep(name, groups)` accepts —
/// `groups` being the names of the groups it sits in, outermost first. A
/// group `keep` rejects takes everything inside it along. Rejected layers
/// are listed in [`KraDoc::held`], never decoded, and raise no warnings.
pub fn read_filtered(bytes: Vec<u8>, keep: &dyn Fn(&str, &[String]) -> bool) -> Result<KraDoc> {
    let zip = ZipArchive::new(Cursor::new(bytes)).context("not a zip file")?;
    let mut r = Reader { zip };
    let maindoc = r.text("maindoc.xml")?;
    let doc = xml::parse(&maindoc).context("parsing maindoc.xml")?;
    let image = doc
        .descendants()
        .find(|n| n.has_tag_name("IMAGE"))
        .context("maindoc.xml has no IMAGE")?;
    let cs = image.attribute("colorspacename").unwrap_or("RGBA");
    if cs != "RGBA" {
        bail!(
            "This Krita file is in the {cs} colour space. Convert it to 8-bit RGBA in Krita \
             (Image ▸ Convert Image Color Space) and save again."
        );
    }
    let width = attr_i32(image, "width").max(1) as u32;
    let height = attr_i32(image, "height").max(1) as u32;
    let name = image.attribute("name").unwrap_or("").to_string();
    let dir = format!("{name}/layers");

    // Layers in XML order (top first). A group at the top of the stack comes
    // through as a header entry above its layers; a group inside a group
    // folds into the outermost one, since ours hold layers, never groups.
    struct Found {
        uuid: String,
        name: String,
        filename: String,
        opacity: u8,
        visible: bool,
        locked: bool,
        x: i32,
        y: i32,
        keyframes: Option<String>,
        group: bool,
        in_group: bool,
        collapsed: bool,
    }
    struct Walk<'a> {
        keep: &'a dyn Fn(&str, &[String]) -> bool,
        groups: Vec<String>,
        found: Vec<Found>,
        warnings: Vec<String>,
        held: Vec<(String, String)>,
    }
    /// `visible` and `opacity` are what folded-in groups pass down: a layer
    /// directly in a top-level group keeps its own.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        layers: roxmltree::Node,
        visible: bool,
        opacity: f32,
        holding: bool,
        dx: i32,
        dy: i32,
        w: &mut Walk,
    ) -> Result<()> {
        for n in layers.children().filter(|n| n.has_tag_name("layer")) {
            let name = n.attribute("name").unwrap_or("").to_string();
            let own_vis = n.attribute("visible") != Some("0");
            let (x, y) = (dx + attr_i32(n, "x"), dy + attr_i32(n, "y"));
            let hold = holding || !(w.keep)(&name, &w.groups);
            if !hold && n.children().any(|c| c.has_tag_name("masks")) {
                w.warnings.push(format!("masks on \"{name}\" ignored"));
            }
            let own_opacity = attr_i32(n, "opacity").clamp(0, 255) as f32 / 255.0;
            match n.attribute("nodetype").unwrap_or("") {
                "paintlayer" if hold => {
                    w.held.push((n.attribute("uuid").unwrap_or("").to_string(), name));
                }
                "paintlayer" => {
                    let cs = n.attribute("colorspacename").unwrap_or("RGBA");
                    if cs != "RGBA" {
                        bail!(
                            "Layer \"{name}\" is in the {cs} colour space. Convert it to 8-bit \
                             RGBA in Krita and save again."
                        );
                    }
                    let op = n.attribute("compositeop").unwrap_or("normal");
                    if op != "normal" {
                        w.warnings.push(format!("\"{name}\" blend mode {op} ignored"));
                    }
                    w.found.push(Found {
                        uuid: n.attribute("uuid").unwrap_or("").to_string(),
                        filename: n.attribute("filename").context("layer without filename")?.to_string(),
                        opacity: (own_opacity * opacity * 255.0).round().clamp(0.0, 255.0) as u8,
                        visible: visible && own_vis,
                        locked: n.attribute("locked") == Some("1"),
                        x,
                        y,
                        keyframes: n.attribute("keyframes").map(str::to_string),
                        name,
                        group: false,
                        in_group: !w.groups.is_empty(),
                        collapsed: false,
                    });
                }
                "grouplayer" => {
                    let Some(inner) = n.children().find(|c| c.has_tag_name("layers")) else {
                        continue;
                    };
                    let top_level = w.groups.is_empty();
                    if top_level && !hold {
                        // Ours: a header, its layers keeping their own state.
                        w.found.push(Found {
                            uuid: n.attribute("uuid").unwrap_or("").to_string(),
                            filename: String::new(),
                            opacity: (own_opacity * 255.0).round() as u8,
                            visible: own_vis,
                            locked: n.attribute("locked") == Some("1"),
                            x,
                            y,
                            keyframes: None,
                            name: name.clone(),
                            group: true,
                            in_group: false,
                            collapsed: n.attribute("collapsed") == Some("1"),
                        });
                    } else if !hold && own_opacity < 1.0 {
                        w.warnings.push(format!(
                            "group \"{name}\" is inside another group: its opacity went into its layers"
                        ));
                    }
                    // A top-level group's own state stays on its header; a
                    // nested one's passes down into its layers.
                    let (vis, op) = if top_level { (visible, opacity) } else { (visible && own_vis, opacity * own_opacity) };
                    w.groups.push(name);
                    walk(inner, vis, op, hold, x, y, w)?;
                    w.groups.pop();
                }
                _ if hold => {}
                other => w.warnings.push(format!("\"{name}\" ({other}) skipped")),
            }
        }
        Ok(())
    }
    let top = image
        .children()
        .find(|n| n.has_tag_name("layers"))
        .context("maindoc.xml has no layers")?;
    let mut w = Walk { keep, groups: Vec::new(), found: Vec::new(), warnings: Vec::new(), held: Vec::new() };
    walk(top, true, 1.0, false, 0, 0, &mut w)?;
    let Walk { found, mut warnings, held, .. } = w;

    let mut layers = Vec::with_capacity(found.len());
    for f in found.into_iter().rev() {
        if f.group {
            layers.push(KraLayer {
                uuid: f.uuid,
                name: f.name,
                opacity: f.opacity,
                visible: f.visible,
                locked: f.locked,
                drawings: Vec::new(),
                keys: Vec::new(),
                group: true,
                in_group: false,
                collapsed: f.collapsed,
            });
            continue;
        }
        let layer_default = r.default_pixel(&format!("{dir}/{}.defaultpixel", f.filename))?;
        let mut drawings: Vec<Drawing> = Vec::new();
        let mut keys: Vec<(usize, usize)> = Vec::new();
        match &f.keyframes {
            Some(kf) => {
                let text = r.text(&format!("{dir}/{kf}"))?;
                let kdoc = xml::parse(&text).with_context(|| format!("parsing {kf}"))?;
                let mut by_file: HashMap<String, usize> = HashMap::new();
                for ch in kdoc.descendants().filter(|n| n.has_tag_name("channel")) {
                    let cname = ch.attribute("name").unwrap_or("");
                    if cname != "content" {
                        warnings.push(format!("\"{}\" animated {cname} ignored", f.name));
                        continue;
                    }
                    for k in ch.children().filter(|n| n.has_tag_name("keyframe")) {
                        let time = attr_i32(k, "time").max(0) as usize;
                        let file = k.attribute("frame").context("keyframe without frame")?.to_string();
                        let (ox, oy) = k
                            .children()
                            .find(|c| c.has_tag_name("offset"))
                            .map(|o| (attr_i32(o, "x"), attr_i32(o, "y")))
                            .unwrap_or((0, 0));
                        let idx = match by_file.get(&file) {
                            Some(&i) => i,
                            None => {
                                let d = r.drawing(&format!("{dir}/{file}"), layer_default, f.x + ox, f.y + oy)?;
                                drawings.push(d);
                                by_file.insert(file, drawings.len() - 1);
                                drawings.len() - 1
                            }
                        };
                        keys.push((time, idx));
                    }
                }
            }
            None => {
                drawings.push(r.drawing(&format!("{dir}/{}", f.filename), layer_default, f.x, f.y)?);
                keys.push((0, 0));
            }
        }
        keys.sort_by_key(|k| k.0);
        keys.dedup_by_key(|k| k.0);
        layers.push(KraLayer {
            uuid: f.uuid,
            name: f.name,
            opacity: f.opacity,
            visible: f.visible,
            locked: f.locked,
            drawings,
            keys,
            group: false,
            in_group: f.in_group,
            collapsed: false,
        });
    }

    // Frame rate and range: Krita 5 keeps them in animation/index.xml; older
    // files put the same elements in maindoc's <animation>.
    let index = r.opt_bytes(&format!("{name}/animation/index.xml"))?;
    let index_text = index.map(String::from_utf8).transpose().map_err(|e| anyhow!("{e}"))?;
    let index_doc = index_text.as_deref().map(xml::parse).transpose()?;
    let anim_root = index_doc
        .as_ref()
        .map(|d| d.root_element())
        .or_else(|| image.children().find(|n| n.has_tag_name("animation")));
    let (mut fps, mut range_end) = (None, None);
    if let Some(a) = anim_root {
        for n in a.children().filter(|n| n.is_element()) {
            if n.has_tag_name("framerate") {
                fps = n.attribute("value").and_then(|v| v.parse::<f32>().ok()).filter(|v| *v > 0.0);
            } else if n.has_tag_name("range") {
                range_end = n.attribute("to").and_then(|v| v.parse::<usize>().ok());
            }
        }
    }

    Ok(KraDoc { width, height, fps, range_end, layers, warnings, held })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::doc::layer::Layer;

    /// A cell with a few coloured pixels, distinct per `seed`.
    fn cell(w: u32, h: u32, seed: u8) -> Canvas {
        let mut c = Canvas::new(w, h);
        for i in 0..5u32 {
            let (x, y) = ((i * 7 + seed as u32) % w, (i * 3 + seed as u32) % h);
            let p = ((y * w + x) * 4) as usize;
            c.pixels_mut()[p..p + 4].copy_from_slice(&[seed, 10 * i as u8, 255 - seed, 200]);
        }
        c
    }

    pub(crate) fn uuids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("{{00000000-0000-4000-8000-{i:012}}}")).collect()
    }

    /// Two layers: one with a key, a hold and a repeated cell; one with its
    /// first key late and a bigger-than-frame cell.
    pub(crate) fn sample_project() -> Project {
        let mut p = Project::new(100, 60, 12.0);
        p.ensure_frame_count(6);
        let a = p.cells.len();
        p.cells.push(cell(100, 60, 1).into());
        let b = p.cells.len();
        p.cells.push(cell(100, 60, 2).into());
        let l = &mut p.layers[0];
        l.exposures = vec![Some(a), None, Some(b), None, Some(a), None];
        l.name = "Ink".into();
        l.opacity = 0.5;

        let mut top = Layer::new("Big", 6);
        top.cell_w = 140;
        top.cell_h = 81;
        let c = p.cells.len();
        p.cells.push(cell(140, 81, 3).into());
        top.exposures[3] = Some(c);
        top.locked = true;
        p.layers.push(top);
        p
    }

    /// The frame-space placement read back into a cell of the original size.
    pub(crate) fn read_cell(d: &Drawing, pw: u32, ph: u32, w: u32, h: u32) -> Canvas {
        let (x, y) = cell_origin(pw, ph, w, h);
        let (c, clipped) = d.place(x, y, w, h);
        assert!(!clipped);
        c
    }

    #[test]
    fn write_then_read_keeps_keys_holds_repeats_and_pixels() {
        let p = sample_project();
        let bytes = write(&p, &WriteOpts { layer_uuids: &uuids(2), selected_layer: 1, carried: &[] }).unwrap();
        let doc = read(bytes).unwrap();
        assert_eq!((doc.width, doc.height), (100, 60));
        assert_eq!(doc.fps, Some(12.0));
        assert_eq!(doc.range_end, Some(5));
        assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
        assert_eq!(doc.layers.len(), 2);

        let ink = &doc.layers[0];
        assert_eq!(ink.name, "Ink");
        assert_eq!(ink.uuid, uuids(2)[0]);
        assert_eq!(ink.opacity, 128);
        // Frames 0 and 4 share one drawing (a cloned keyframe).
        assert_eq!(ink.keys.iter().map(|k| k.0).collect::<Vec<_>>(), vec![0, 2, 4]);
        assert_eq!(ink.keys[0].1, ink.keys[2].1);
        assert_eq!(ink.drawings.len(), 2);
        let a = read_cell(&ink.drawings[ink.keys[0].1], 100, 60, 100, 60);
        assert_eq!(a.pixels(), p.cells[p.layers[0].exposures[0].unwrap()].pixels());

        let big = &doc.layers[1];
        assert!(big.locked);
        // A blank key at 0 stands in for "nothing before the first key".
        assert_eq!(big.keys.iter().map(|k| k.0).collect::<Vec<_>>(), vec![0, 3]);
        assert_eq!(big.drawings[big.keys[0].1].w, 0);
        let c = read_cell(&big.drawings[big.keys[1].1], 100, 60, 140, 81);
        assert_eq!(c.pixels(), p.cells[p.layers[1].exposures[3].unwrap()].pixels());
    }

    /// `sample_project` with drawings big enough to see — the project behind
    /// `testdata/krita-5.2.9-resaved.kra`.
    pub(crate) fn fixture_project() -> Project {
        let mut p = sample_project();
        for (i, c) in p.cells.iter_mut().map(std::sync::Arc::make_mut).enumerate() {
            let (w, h) = (c.width, c.height);
            for y in 0..h {
                for x in 0..w {
                    if (x as i32 - (20 + 15 * i as i32)).abs() < 12 && (y as i32 - 30).abs() < 25 {
                        let o = ((y * w + x) * 4) as usize;
                        c.pixels_mut()[o..o + 4].copy_from_slice(&[(60 * i) as u8, 200, 90, 255]);
                    }
                }
            }
        }
        // One drawing on its own layer: Krita loads a single-keyframe layer
        // from the layer's main file, not the keyframe's.
        let mut solo = Layer::new("Solo", p.frame_count);
        let id = p.cells.len();
        let mut c = Canvas::new(100, 60);
        for y in 40..55 {
            for x in 60..90 {
                let o = ((y * 100 + x) * 4) as usize;
                c.pixels_mut()[o..o + 4].copy_from_slice(&[30, 60, 220, 255]);
            }
        }
        p.cells.push(c.into());
        solo.exposures[0] = Some(id);
        p.layers.push(solo);
        p
    }

    /// The reader against a file Krita itself wrote: `fixture_project` went
    /// through our writer, then `krita --export resaved.kra` (Krita 5.2.9).
    /// Regenerate with `KRA_FIXTURE_DIR=… cargo test kra_fixture -- --ignored`
    /// and that export.
    /// Krita re-encodes every tile (LZF, channel-planar), renames the frame
    /// files and keeps a cloned keyframe — every drawing must still come back
    /// byte-identical, or the link would see changes that were never made.
    #[test]
    fn reads_krita_written_file_byte_identical() {
        let p = fixture_project();
        let doc = read(include_bytes!("testdata/krita-5.2.9-resaved.kra").to_vec()).unwrap();
        assert_eq!((doc.width, doc.height, doc.fps, doc.range_end), (100, 60, Some(12.0), Some(5)));
        assert_eq!(doc.layers.len(), p.layers.len());
        for (li, kl) in doc.layers.iter().enumerate() {
            let l = &p.layers[li];
            assert_eq!(kl.uuid, uuids(p.layers.len())[li]);
            assert_eq!(kl.name, l.name);
            for &(time, di) in &kl.keys {
                let d = &kl.drawings[di];
                match l.exposures[time] {
                    Some(id) => {
                        let c = &p.cells[id];
                        assert_eq!(read_cell(d, 100, 60, c.width, c.height).pixels(), c.pixels(), "layer {li} t{time}");
                    }
                    // Our explicit blank lead-in key.
                    None => assert!(d.pixels.iter().all(|&b| b == 0)),
                }
            }
        }
        // Frames 0 and 4 of the ink layer are still one drawing.
        let ink = &doc.layers[0];
        assert_eq!(ink.keys.iter().map(|k| k.0).collect::<Vec<_>>(), vec![0, 2, 4]);
        assert_eq!(ink.keys[0].1, ink.keys[2].1);
    }

    /// Carry a layer out of a file Krita itself wrote — Krita's folder name,
    /// Krita's `layerN` file names, LZF tiles — into a rewrite that doesn't
    /// have it: it goes back in the same place, drawing for drawing.
    #[test]
    fn carries_a_krita_written_layer_into_a_rewrite() {
        let krita = include_bytes!("testdata/krita-5.2.9-resaved.kra").to_vec();
        // Pretend "Big" is Krita-only.
        let keep = |name: &str, _: &[String]| name != "Big";
        let carried = carry(krita.clone(), &keep).unwrap();
        assert_eq!(carried.len(), 1);
        assert_eq!(carried[0].name, "Big");
        assert!(carried[0].files.iter().any(|(f, _)| f.ends_with(".keyframes.xml")));
        assert_eq!(carried[0].below, [uuids(3)[0].clone()], "Ink is under it");

        // The rewrite from this app: Ink and Solo only, under our own names.
        let mut q = fixture_project();
        q.layers.remove(1);
        let u = uuids(3);
        let qu = [u[0].clone(), u[2].clone()];
        let out = write(&q, &WriteOpts { layer_uuids: &qu, selected_layer: 0, carried: &carried }).unwrap();

        let back = read(out).unwrap();
        let names: Vec<&str> = back.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Ink", "Big", "Solo"]);
        let orig = read(krita).unwrap();
        let (a, b) = (&orig.layers[1], &back.layers[1]);
        assert_eq!(a.uuid, b.uuid);
        assert_eq!(a.keys.iter().map(|k| k.0).collect::<Vec<_>>(), b.keys.iter().map(|k| k.0).collect::<Vec<_>>());
        for (ka, kb) in a.keys.iter().zip(&b.keys) {
            let (da, db) = (&a.drawings[ka.1], &b.drawings[kb.1]);
            assert_eq!((da.x, da.y, da.w, da.h), (db.x, db.y, db.w, db.h));
            assert_eq!(da.pixels, db.pixels);
        }
    }

    /// A rejected group comes out whole — any layer type inside — with the
    /// files its layers name, and only those (`layer9…`, not `layer90`).
    #[test]
    fn carries_a_whole_group_with_its_files() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let o = SimpleFileOptions::default();
        zip.start_file("maindoc.xml", o).unwrap();
        zip.write_all(
            br#"<DOC><IMAGE name="Unnamed" width="4" height="4" colorspacename="RGBA"><layers>
                 <layer nodetype="grouplayer" name="refs-x" filename="layer8" uuid="{g}"><layers>
                   <layer nodetype="shapelayer" name="notes" filename="layer9" uuid="{v}"/>
                 </layers></layer>
                 <layer nodetype="paintlayer" name="Ink" filename="layer2" uuid="{ink}"/>
               </layers></IMAGE></DOC>"#,
        )
        .unwrap();
        for (f, b) in [
            ("Unnamed/layers/layer9.shapelayer/content.svg", &b"<svg/>"[..]),
            ("Unnamed/layers/layer90", b"not ours"),
            ("Unnamed/layers/layer2", b"ink"),
        ] {
            zip.start_file(f, o).unwrap();
            zip.write_all(b).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let keep = |name: &str, _: &[String]| !name.ends_with("-x");
        let c = carry(bytes, &keep).unwrap();
        assert_eq!(c.len(), 1);
        assert!(c[0].xml.starts_with("<layer nodetype=\"grouplayer\"") && c[0].xml.contains("name=\"notes\""));
        let files: Vec<&str> = c[0].files.iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(files, ["layer9.shapelayer/content.svg"]);
        assert_eq!(c[0].below, ["{ink}"]);
    }

    const SKETCH_UUID: &str = "{00000000-0000-4000-8000-00000000abcd}";

    /// `fixture_project` with a Krita-only `sketch-x` layer between Ink and
    /// Big, and the uuids to write it with (`uuids(3)` for the fixture's own
    /// layers, in order, so a rewrite without the sketch lines up).
    fn with_sketch() -> (Project, Vec<String>) {
        let mut p = fixture_project();
        let mut sketch = Layer::new("sketch-x", p.frame_count);
        let id = p.cells.len();
        let mut c = Canvas::new(100, 60);
        for y in 5..20 {
            for x in 5..95 {
                let o = ((y * 100 + x) * 4) as usize;
                c.pixels_mut()[o..o + 4].copy_from_slice(&[220, 40, 160, 255]);
            }
        }
        p.cells.push(c.into());
        sketch.exposures[1] = Some(id);
        p.layers.insert(1, sketch);
        let u = uuids(3);
        (p, vec![u[0].clone(), SKETCH_UUID.to_string(), u[1].clone(), u[2].clone()])
    }

    /// Not a unit test: step 1 of the carry check — writes `carry_src.kra`
    /// (the project with `sketch-x`) into `$KRA_FIXTURE_DIR`. Then
    /// `krita --export --export-filename carry_krita.kra carry_src.kra`.
    #[test]
    #[ignore]
    fn kra_carry_src() {
        let dir = std::path::PathBuf::from(std::env::var("KRA_FIXTURE_DIR").unwrap());
        let (p, u) = with_sketch();
        let bytes = write(&p, &WriteOpts { layer_uuids: &u, selected_layer: 0, carried: &[] }).unwrap();
        std::fs::write(dir.join("carry_src.kra"), bytes).unwrap();
    }

    /// `sample_project` with its two layers in a half-opaque, folded group
    /// under a third layer.
    pub(crate) fn grouped_project() -> Project {
        let mut p = sample_project();
        p.add_layer();
        let top = p.layers.len() - 1;
        p.layers[top].name = "Over".into();
        let id = p.cells.len();
        p.cells.push(cell(100, 60, 9).into());
        p.layers[top].set_key(0, id);
        let g = p.group_layers(&[0, 1]).unwrap();
        p.layers[g].name = "Character".into();
        p.layers[g].opacity = 0.5;
        p.layers[g].collapsed = true;
        p
    }

    #[test]
    fn a_group_round_trips_through_kra() {
        let p = grouped_project();
        let u = uuids(p.layers.len());
        let bytes = write(&p, &WriteOpts { layer_uuids: &u, selected_layer: 0, carried: &[] }).unwrap();
        let doc = read(bytes).unwrap();
        let rows: Vec<(String, bool, bool, bool, u8)> = doc
            .layers
            .iter()
            .map(|l| (l.name.clone(), l.group, l.in_group, l.collapsed, l.opacity))
            .collect();
        let row = |i: usize| (p.layers[i].name.clone(), false, true, false, opacity_u8(p.layers[i].opacity));
        assert_eq!(
            rows,
            vec![
                row(0),
                row(1),
                ("Character".to_string(), true, false, true, 128),
                ("Over".to_string(), false, false, false, 255),
            ]
        );
        assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
        // Same uuids both ways, the group's included.
        assert_eq!(doc.layers.iter().map(|l| l.uuid.clone()).collect::<Vec<_>>(), u);
    }

    /// Krita-only layers beside a group go back on the same side of its
    /// edge: one just above the group stays out of it, one at the bottom of
    /// it stays in.
    #[test]
    fn carried_layers_keep_their_side_of_a_group() {
        let mut krita = grouped_project();
        let mut inside = Layer::new("bottom-x", krita.frame_count);
        inside.in_group = true;
        krita.layers.insert(0, inside);
        krita.layers.insert(4, Layer::new("above-x", krita.frame_count));
        assert!(krita.groups_ok());
        let u = uuids(krita.layers.len());
        let bytes = write(&krita, &WriteOpts { layer_uuids: &u, selected_layer: 0, carried: &[] }).unwrap();
        let carried = carry(bytes, &|name: &str, _: &[String]| !name.ends_with("-x")).unwrap();
        assert_eq!(carried.len(), 2);

        let p = grouped_project();
        let pu = [u[1].clone(), u[2].clone(), u[3].clone(), u[5].clone()];
        let out = write(&p, &WriteOpts { layer_uuids: &pu, selected_layer: 0, carried: &carried }).unwrap();
        let rows: Vec<(String, bool)> = read(out).unwrap().layers.iter().map(|l| (l.name.clone(), l.in_group)).collect();
        let want: Vec<(String, bool)> =
            krita.layers.iter().map(|l| (l.name.clone(), l.in_group)).collect();
        assert_eq!(rows, want);
    }

    /// Rewrite `bytes`' `maindoc.xml` with `f`, every other file as it was.
    fn with_maindoc(bytes: Vec<u8>, f: impl Fn(&str) -> String) -> Vec<u8> {
        let mut src = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut out = ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..src.len() {
            let mut e = src.by_index(i).unwrap();
            let name = e.name().to_string();
            let mut data = Vec::new();
            e.read_to_end(&mut data).unwrap();
            if name == "maindoc.xml" {
                data = f(std::str::from_utf8(&data).unwrap()).into_bytes();
            }
            out.start_file(name, SimpleFileOptions::default()).unwrap();
            out.write_all(&data).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    /// Krita groups can nest; ours can't. A group inside a group folds into
    /// the outer one, its own visibility and opacity going into its layers.
    #[test]
    fn a_nested_krita_group_folds_into_the_outer() {
        let p = grouped_project();
        let u = uuids(p.layers.len());
        let bytes = write(&p, &WriteOpts { layer_uuids: &u, selected_layer: 0, carried: &[] }).unwrap();
        // Wrap our group in an outer one, and make the inner one half opaque
        // and hidden.
        let nested = with_maindoc(bytes, |xml| {
            let start = xml.find("<layer nodetype=\"grouplayer\"").unwrap();
            let end = xml[start..].find("</layers>\n   </layer>").unwrap() + start + "</layers>\n   </layer>".len();
            // The group's own tag comes first: hide just it.
            let inner = xml[start..end].replacen("visible=\"1\"", "visible=\"0\"", 1);
            let outer = format!(
                "<layer nodetype=\"grouplayer\" filename=\"outer\" name=\"Outer\" uuid=\"{{11111111-0000-4000-8000-000000000000}}\" opacity=\"255\" visible=\"1\" locked=\"0\" x=\"0\" y=\"0\" compositeop=\"normal\" collapsed=\"0\" passthrough=\"0\" intimeline=\"1\"><layers>{inner}</layers></layer>"
            );
            format!("{}{}{}", &xml[..start], outer, &xml[end..])
        });
        let doc = read(nested).unwrap();
        let rows: Vec<(String, bool, bool, bool, u8)> =
            doc.layers.iter().map(|l| (l.name.clone(), l.group, l.in_group, l.visible, l.opacity)).collect();
        // Outer header; its layers — the inner group's — folded in: hidden
        // with it, at its half opacity.
        assert_eq!(rows.len(), 4, "{rows:?}");
        assert_eq!(rows[2].0, "Outer");
        assert!(rows[2].1 && !rows[2].2);
        for (i, r) in rows[..2].iter().enumerate() {
            assert!(!r.1 && r.2 && !r.3, "{r:?}");
            assert_eq!(r.4, opacity_u8(p.layers[i].opacity * 128.0 / 255.0), "{r:?}");
        }
        assert!(doc.warnings.iter().any(|w| w.contains("inside another group")), "{:?}", doc.warnings);
    }

    /// Not a unit test: writes `group_src.kra` (see [`grouped_project`])
    /// into `$KRA_FIXTURE_DIR`, for checking against Krita:
    /// `krita --export-sequence --export-filename seq.png group_src.kra`
    /// should match our own flatten, and `krita --export --export-filename
    /// group_krita.kra group_src.kra` should read back with its group.
    #[test]
    #[ignore]
    fn kra_group_src() {
        let dir = std::path::PathBuf::from(std::env::var("KRA_FIXTURE_DIR").unwrap());
        let p = grouped_project();
        let u = uuids(p.layers.len());
        let bytes = write(&p, &WriteOpts { layer_uuids: &u, selected_layer: 0, carried: &[] }).unwrap();
        std::fs::write(dir.join("group_src.kra"), bytes).unwrap();
        // The same layers with no group, to tell group effects from anything
        // Krita's export does to every file.
        let mut plain = p.clone();
        plain.ungroup(plain.layers.iter().position(|l| l.is_group()).unwrap());
        let u = uuids(plain.layers.len());
        let bytes = write(&plain, &WriteOpts { layer_uuids: &u, selected_layer: 0, carried: &[] }).unwrap();
        std::fs::write(dir.join("plain_src.kra"), bytes).unwrap();
        for f in 0..p.frame_count {
            for (proj, name) in [(&p, "ours"), (&plain, "oursplain")] {
                let flat = crate::io::composite::flatten_frame(proj, f);
                let img = image::RgbaImage::from_raw(flat.width, flat.height, flat.into_pixels()).unwrap();
                img.save(dir.join(format!("{name}{f:04}.png"))).unwrap();
            }
        }
    }

    /// Not a unit test: reads Krita's re-save of `group_src.kra` and checks
    /// the group came through.
    #[test]
    #[ignore]
    fn kra_group_back() {
        let dir = std::path::PathBuf::from(std::env::var("KRA_FIXTURE_DIR").unwrap());
        let doc = read(std::fs::read(dir.join("group_krita.kra")).unwrap()).unwrap();
        let rows: Vec<(String, bool, bool, bool, u8)> =
            doc.layers.iter().map(|l| (l.name.clone(), l.group, l.in_group, l.collapsed, l.opacity)).collect();
        println!("{rows:?}\n{:?}", doc.warnings);
        assert!(rows.iter().any(|r| r.0 == "Character" && r.1 && r.4 == 128));
        assert_eq!(rows.iter().filter(|r| r.2).count(), 2);
        // Krita's own render of the group (`seq*.png`) against ours. Krita's
        // PNG export converts colour (it does the same to `plain*.png`, the
        // file without a group), so colour is checked against that render,
        // and coverage — what the group's opacity decides — against ours,
        // wherever our render and Krita's already agree without the group
        // (an odd-sized cell lands half a pixel apart in the two, groups or
        // not).
        for f in 0..6 {
            let open = |name: String| image::open(dir.join(name)).unwrap().to_rgba8();
            let ours = open(format!("ours{f:04}.png"));
            let ours_plain = open(format!("oursplain{f:04}.png"));
            let theirs = open(format!("seq{f:04}.png"));
            let plain = open(format!("plain{f:04}.png"));
            let (mut alpha, mut colour, mut compared) = (0, 0, 0);
            for (((a, b), c), d) in ours.pixels().zip(theirs.pixels()).zip(plain.pixels()).zip(ours_plain.pixels()) {
                if d[3] == c[3] {
                    compared += 1;
                    alpha = alpha.max((a[3] as i32 - b[3] as i32).abs());
                }
                if b[3] > 0 && c[3] > 0 {
                    colour = colour.max((0..3).map(|k| (b[k] as i32 - c[k] as i32).abs()).max().unwrap());
                }
            }
            println!("frame {f}: alpha off by {alpha} over {compared} pixels, colour off by {colour}");
            assert!(alpha <= 1 && colour <= 2, "frame {f}");
        }
    }

    /// Not a unit test: step 2 — carries `sketch-x` out of Krita's
    /// `carry_krita.kra` into a rewrite of the project *without* it,
    /// `carry_out.kra`. Krita should render that exactly like `carry_src.kra`.
    #[test]
    #[ignore]
    fn kra_carry_out() {
        let dir = std::path::PathBuf::from(std::env::var("KRA_FIXTURE_DIR").unwrap());
        let krita = std::fs::read(dir.join("carry_krita.kra")).unwrap();
        let keep = |name: &str, _: &[String]| !name.ends_with("-x");
        let carried = carry(krita, &keep).unwrap();
        let p = fixture_project();
        let opts = WriteOpts { layer_uuids: &uuids(3), selected_layer: 0, carried: &carried };
        std::fs::write(dir.join("carry_out.kra"), write(&p, &opts).unwrap()).unwrap();
    }

    /// Not a unit test: writes `ours.kra` plus our own flattened frames
    /// (`ours_NNNN.png`) into `$KRA_FIXTURE_DIR`, to check the writer against
    /// a real Krita (`krita --export-sequence`, `krita --export`).
    #[test]
    #[ignore]
    fn kra_fixture() {
        let dir = std::path::PathBuf::from(std::env::var("KRA_FIXTURE_DIR").unwrap());
        let p = fixture_project();
        let bytes = write(&p, &WriteOpts { layer_uuids: &uuids(p.layers.len()), selected_layer: 1, carried: &[] }).unwrap();
        std::fs::write(dir.join("ours.kra"), bytes).unwrap();
        for f in 0..p.frame_count {
            let flat = composite::flatten_frame(&p, f);
            image::RgbaImage::from_raw(flat.width, flat.height, flat.into_pixels())
                .unwrap()
                .save(dir.join(format!("ours_{f:04}.png")))
                .unwrap();
        }
    }

    /// Krita loads a single-keyframe layer from the layer's main file alone,
    /// and reports a layer without its own `.icc` as a load problem — both
    /// found the hard way in the Krita GUI, which the CLI export stays quiet
    /// about.
    #[test]
    fn first_drawing_lives_in_the_main_file_next_to_a_profile() {
        let p = sample_project();
        let bytes = write(&p, &WriteOpts { layer_uuids: &uuids(2), selected_layer: 0, carried: &[] }).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        for n in ["anim1", "anim2"] {
            let mut kf = String::new();
            zip.by_name(&format!("image/layers/{n}.keyframes.xml")).unwrap().read_to_string(&mut kf).unwrap();
            let doc = xml::parse(&kf).unwrap();
            let first = doc.descendants().find(|k| k.has_tag_name("keyframe")).unwrap();
            assert_eq!(first.attribute("frame"), Some(n));
            let mut icc = Vec::new();
            zip.by_name(&format!("image/layers/{n}.icc")).unwrap().read_to_end(&mut icc).unwrap();
            assert_eq!(icc, SRGB_ICC);
        }
        // Ink's first drawing sits in `anim1` itself.
        let mut main = Vec::new();
        zip.by_name("image/layers/anim1").unwrap().read_to_end(&mut main).unwrap();
        assert!(tiles::decode(&main, [0; 4]).unwrap().is_some());
    }

    #[test]
    fn place_reports_paint_outside_the_cell() {
        let d = Drawing { x: -1, y: 0, w: 2, h: 1, pixels: vec![1, 2, 3, 255, 4, 5, 6, 255], default: [0; 4] };
        let (c, clipped) = d.place(0, 0, 1, 1);
        assert!(clipped);
        assert_eq!(&c.pixels()[..], &[4, 5, 6, 255]);
    }

    /// A group the filter rejects takes its layers along: they come back as
    /// `held`, undecoded — this file has no pixel data for them at all.
    #[test]
    fn a_rejected_group_holds_its_layers_undecoded() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("maindoc.xml", SimpleFileOptions::default()).unwrap();
        zip.write_all(
            br#"<DOC><IMAGE name="x" width="4" height="4" colorspacename="RGBA"><layers>
                 <layer nodetype="grouplayer" name="refs-x" opacity="128" filename="g"><layers>
                   <layer nodetype="paintlayer" name="photo" uuid="{p}" filename="missing"/>
                   <layer nodetype="vectorlayer" name="notes" filename="v"/>
                 </layers></layer>
               </layers></IMAGE></DOC>"#,
        )
        .unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        // Shaped like the link's filter: the layer's own name, and its groups'.
        let keep = |name: &str, groups: &[String]| {
            !name.ends_with("-x") && !groups.iter().any(|g| g.ends_with("-x"))
        };
        let doc = read_filtered(bytes, &keep).unwrap();
        assert!(doc.layers.is_empty());
        assert_eq!(doc.held, [("{p}".to_string(), "photo".to_string())]);
        // No "skipped" / "opacity ignored" noise for what was kept out on purpose.
        assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
    }

    #[test]
    fn rejects_non_rgba_colour_space() {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("maindoc.xml", SimpleFileOptions::default()).unwrap();
        zip.write_all(br#"<DOC><IMAGE name="x" width="4" height="4" colorspacename="RGBA16"><layers/></IMAGE></DOC>"#)
            .unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let err = read(bytes).err().unwrap().to_string();
        assert!(err.contains("RGBA16"), "{err}");
    }
}
