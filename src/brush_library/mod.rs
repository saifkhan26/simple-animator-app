//! The brushes the brush picker offers, in three groups: the app's own
//! presets, the Krita presets the app ships, and Krita presets the user has
//! imported (copied into `<config>/brushes/<name>/`).
//!
//! Loading a Krita preset decodes its tip and texture, which is too slow to
//! do for a whole group in one frame, so a background thread loads every
//! Krita entry and decodes every thumbnail, and the picker shows them as
//! they arrive. A stroke that needs a preset before then loads it at once.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use crate::tools::krita::kpp::Preset;
use crate::tools::krita::{defaults, PixelBrush, Resources};
use crate::tools::BrushSettings;

pub mod import;

/// Thumbnails are this many pixels on their long side.
pub const THUMB: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    App,
    Krita,
    Imported,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::App => "App",
            Group::Krita => "Krita",
            Group::Imported => "Imported",
        }
    }
}

/// Where an entry's brush comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// `BrushSettings::PRESETS[i]`.
    App(usize),
    /// A shipped Krita preset, by its file name.
    Embedded(&'static str),
    /// An imported preset: its folder and the `.kpp` inside it.
    Folder(PathBuf, String),
}

/// Whether a Krita entry paints.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Loading,
    Ready,
    /// Why not, in the loader's words.
    Refused(String),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub group: Group,
    pub source: Source,
    pub status: Status,
    pub tooltip: String,
}

/// What the loader thread sends back for one entry.
struct Loaded {
    id: String,
    brush: Result<Arc<PixelBrush>, String>,
    thumb: Option<egui::ColorImage>,
}

/// Brushes built from edited presets lately, newest last.
type Edited = Vec<(String, Vec<(String, String)>, Result<Arc<PixelBrush>, String>)>;
/// The last edited preset asked for.
type EditedPreset = Option<(String, Vec<(String, String)>, Arc<Preset>)>;

pub struct BrushLibrary {
    root: Option<PathBuf>,
    pub entries: Vec<Entry>,
    brushes: RefCell<HashMap<String, Arc<PixelBrush>>>,
    presets: RefCell<HashMap<String, Arc<Preset>>>,
    edited: RefCell<Edited>,
    edited_preset: RefCell<EditedPreset>,
    thumbs: HashMap<String, egui::ColorImage>,
    incoming: Option<Receiver<Loaded>>,
}

/// `c)_Pencil-5_Tilted.kpp` → `Pencil-5 Tilted`.
pub fn display_name(file: &str) -> String {
    let stem = file.strip_suffix(".kpp").unwrap_or(file);
    let stem = match stem.split_once(")_") {
        Some((prefix, rest)) if prefix.len() <= 2 => rest,
        _ => stem,
    };
    stem.replace('_', " ")
}

/// Tips for a preset imported into its own folder: the files beside it.
struct FolderTips<'a>(&'a Path);

impl Resources for FolderTips<'_> {
    fn tip(&self, filename: &str) -> Option<std::borrow::Cow<'_, [u8]>> {
        // A preset names its tip by file name only; never look outside.
        let name = Path::new(filename).file_name()?;
        std::fs::read(self.0.join(name)).ok().map(std::borrow::Cow::Owned)
    }
}

/// One source of tips, then another.
struct Chain<'a>(&'a dyn Resources, &'a dyn Resources);

impl Resources for Chain<'_> {
    fn tip(&self, filename: &str) -> Option<std::borrow::Cow<'_, [u8]>> {
        self.0.tip(filename).or_else(|| self.1.tip(filename))
    }
}

/// Build `preset` with the tips `source` can reach, and the shipped ones
/// (an edit may switch to one of those).
fn build_edited(source: &Source, preset: &Preset) -> Result<PixelBrush, String> {
    let r = match source {
        Source::Folder(dir, _) => PixelBrush::from_preset(preset, &Chain(&FolderTips(dir), &defaults::DefaultTips)),
        _ => PixelBrush::from_preset(preset, &defaults::DefaultTips),
    };
    r.map_err(|e| format!("{e:#}"))
}

/// A tip `source` can reach, or a shipped one.
fn source_tip(source: &Source, file: &str) -> Option<Vec<u8>> {
    let found = match source {
        Source::Folder(dir, _) => Chain(&FolderTips(dir), &defaults::DefaultTips).tip(file).map(|c| c.into_owned()),
        _ => defaults::DefaultTips.tip(file).map(|c| c.into_owned()),
    };
    found
}

/// A Krita brush's S-stroke on paper: a saved preset's picture.
fn krita_picture(brush: Arc<PixelBrush>, size: u32) -> image::RgbaImage {
    use crate::doc::canvas::Canvas;
    use crate::input::pointer::PointerSample;
    use crate::tools::ribbon::StrokeWorkspace;
    use crate::tools::stroke::StrokeBuilder;
    use crate::tools::{ActiveTool, BrushMode, SmoothingOptions};
    let mut c = Canvas::new(size, size);
    let b = BrushSettings {
        mode: BrushMode::Krita,
        radius: (brush.diameter() / 2.0).min(size as f64 / 8.0) as f32,
        color: [20, 20, 20, 255],
        ..BrushSettings::default()
    };
    let pre = c.pixels().into_owned();
    let mut ws = StrokeWorkspace::new();
    ws.begin(size, size, &b);
    let mut s = StrokeBuilder::new_with_krita(b, ActiveTool::Pencil, 1.0, SmoothingOptions::default(), brush);
    let f = size as f32;
    for i in 0..=60 {
        let t = i as f32 / 60.0;
        let p = (t * std::f32::consts::PI).sin().powf(0.6).clamp(0.05, 1.0);
        let mut sample = PointerSample::at(f * 0.12 + t * f * 0.76, f / 2.0 + (t * 6.0).sin() * f * 0.22, p);
        sample.tilt_x = 35.0;
        sample.time_ms = i as f64 * 8.0;
        s.push(sample);
        s.flush(&mut c, &mut ws, &pre);
    }
    s.finish(&mut c, &mut ws, &pre);
    // Over paper, so it reads in Krita's chooser as in ours.
    let px = c.pixels();
    image::RgbaImage::from_fn(size, size, |x, y| {
        let i = ((y * size + x) * 4) as usize;
        let a = px[i + 3] as u32;
        let over = |ch: u32, paper: u32| ((ch * a + paper * (255 - a) + 127) / 255) as u8;
        image::Rgba([over(px[i] as u32, 226), over(px[i + 1] as u32, 226), over(px[i + 2] as u32, 222), 255])
    })
}

/// Tip files that sit beside an imported preset.
const TIP_EXTENSIONS: [&str; 3] = ["png", "gbr", "gih"];

fn load_source(source: &Source) -> Result<PixelBrush, String> {
    match source {
        Source::Embedded(name) => defaults::load(name).map_err(|e| format!("{e:#}")),
        Source::Folder(dir, kpp) => {
            let bytes = std::fs::read(dir.join(kpp)).map_err(|e| format!("{kpp}: {e}"))?;
            PixelBrush::load(&bytes, &FolderTips(dir)).map_err(|e| format!("{e:#}"))
        }
        Source::App(_) => Err("an app preset is not a Krita preset".into()),
    }
}

fn kpp_bytes(source: &Source) -> Option<Vec<u8>> {
    match source {
        Source::Embedded(name) => defaults::kpp(name),
        Source::Folder(dir, kpp) => std::fs::read(dir.join(kpp)).ok(),
        Source::App(_) => None,
    }
}

/// An RGBA image as egui wants it, premultiplied the way the app's theme
/// does it (in gamma space, so translucent edges don't come out bright).
fn color_image(size: [usize; 2], rgba: &[u8]) -> egui::ColorImage {
    egui::ColorImage {
        size,
        pixels: rgba.chunks_exact(4).map(|p| crate::ui::theme::premul(p[0], p[1], p[2], p[3])).collect(),
    }
}

/// A `.kpp` is a PNG: its own picture, scaled to a thumbnail.
fn kpp_thumbnail(kpp: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory_with_format(kpp, image::ImageFormat::Png).ok()?;
    let img = img.resize(THUMB, THUMB, image::imageops::FilterType::Triangle).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(color_image(size, img.as_raw()))
}

/// An app preset drawn as a short S-stroke, dark on clear.
fn app_thumbnail(brush: BrushSettings) -> egui::ColorImage {
    use crate::doc::canvas::Canvas;
    use crate::input::pointer::PointerSample;
    use crate::tools::ribbon::StrokeWorkspace;
    use crate::tools::stroke::StrokeBuilder;
    use crate::tools::{ActiveTool, SmoothingOptions};
    let (w, h) = (THUMB, THUMB * 3 / 4);
    let mut c = Canvas::new(w, h);
    let mut b = brush;
    b.color = [20, 20, 20, 255];
    b.radius = b.radius.min(7.0);
    let pre = c.pixels().into_owned();
    let mut ws = StrokeWorkspace::new();
    ws.begin(w, h, &b);
    let mut s = StrokeBuilder::new(b, ActiveTool::Pencil, 1.0, SmoothingOptions::default());
    for i in 0..=40 {
        let t = i as f32 / 40.0;
        let p = (t * std::f32::consts::PI).sin().powf(0.6).clamp(0.05, 1.0);
        let mut sample = PointerSample::at(6.0 + t * (w as f32 - 12.0), h as f32 / 2.0 + (t * 6.0).sin() * h as f32 * 0.25, p);
        sample.tilt_x = 35.0;
        s.push(sample);
        s.flush(&mut c, &mut ws, &pre);
    }
    s.finish(&mut c, &mut ws, &pre);
    color_image([w as usize, h as usize], &c.pixels())
}

/// The folders under `root` that hold a `.kpp`, sorted by name.
fn scan_imported(root: &Path) -> Vec<Entry> {
    let Ok(dirs) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for d in dirs.flatten() {
        let dir = d.path();
        // `.import-*` is an import still being written.
        if !dir.is_dir() || d.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else { continue };
        let kpp = files
            .flatten()
            .map(|f| f.file_name().to_string_lossy().into_owned())
            .find(|n| n.to_ascii_lowercase().ends_with(".kpp"));
        let Some(kpp) = kpp else { continue };
        let folder = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        out.push(Entry {
            id: format!("user/{folder}"),
            name: display_name(&kpp),
            group: Group::Imported,
            tooltip: format!("Imported from Krita: {kpp}"),
            source: Source::Folder(dir, kpp),
            status: Status::Loading,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

impl BrushLibrary {
    /// The library, with imported presets read from `root` (none if
    /// `None`), and a thread started to load the Krita entries.
    pub fn new(root: Option<PathBuf>) -> Self {
        let mut entries: Vec<Entry> = BrushSettings::PRESETS
            .iter()
            .enumerate()
            .map(|(i, (label, tip, _))| Entry {
                id: format!("app/{label}"),
                name: (*label).to_owned(),
                group: Group::App,
                source: Source::App(i),
                status: Status::Ready,
                tooltip: (*tip).to_owned(),
            })
            .collect();
        entries.extend(defaults::PRESETS.iter().map(|&file| Entry {
            id: format!("krita/{}", file.trim_end_matches(".kpp")),
            name: display_name(file),
            group: Group::Krita,
            source: Source::Embedded(file),
            status: Status::Loading,
            tooltip: format!("Krita's own {}, from its default brushes.", display_name(file)),
        }));
        if let Some(root) = &root {
            entries.extend(scan_imported(root));
        }
        let mut lib = Self {
            root,
            entries,
            brushes: RefCell::new(HashMap::new()),
            presets: RefCell::new(HashMap::new()),
            edited: RefCell::new(Vec::new()),
            edited_preset: RefCell::new(None),
            thumbs: HashMap::new(),
            incoming: None,
        };
        lib.start_loading();
        lib
    }

    /// Load, on a thread, every entry not yet loaded or thumbnailed.
    fn start_loading(&mut self) {
        let jobs: Vec<(String, Source)> = self
            .entries
            .iter()
            .filter(|e| e.status == Status::Loading || !self.thumbs.contains_key(&e.id))
            .map(|e| (e.id.clone(), e.source.clone()))
            .collect();
        let (tx, rx) = mpsc::channel();
        self.incoming = Some(rx);
        let spawned = std::thread::Builder::new()
            .name("brush-library".into())
            .spawn(move || {
                for (id, source) in jobs {
                    let (brush, thumb) = match &source {
                        Source::App(i) => (Err(String::new()), Some(app_thumbnail((BrushSettings::PRESETS[*i].2)()))),
                        _ => (
                            load_source(&source).map(Arc::new),
                            kpp_bytes(&source).and_then(|b| kpp_thumbnail(&b)),
                        ),
                    };
                    if tx.send(Loaded { id, brush, thumb }).is_err() {
                        return;
                    }
                }
            });
        if spawned.is_err() {
            self.incoming = None;
        }
    }

    /// Take in whatever the loader has finished. Returns whether anything
    /// changed, so the caller can repaint.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.incoming else { return false };
        let mut changed = false;
        while let Ok(l) = rx.try_recv() {
            changed = true;
            if let Some(t) = l.thumb {
                self.thumbs.insert(l.id.clone(), t);
            }
            let Some(e) = self.entries.iter_mut().find(|e| e.id == l.id) else { continue };
            if e.group == Group::App {
                continue;
            }
            match l.brush {
                Ok(b) => {
                    e.status = Status::Ready;
                    self.brushes.borrow_mut().entry(l.id).or_insert(b);
                }
                Err(why) => e.status = Status::Refused(why),
            }
        }
        changed
    }

    /// Whether the loader is still working.
    pub fn loading(&self) -> bool {
        self.entries.iter().any(|e| e.status == Status::Loading)
    }

    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// A Krita entry's brush, loading it now if the thread has not yet.
    pub fn brush(&self, id: &str) -> Option<Arc<PixelBrush>> {
        if let Some(b) = self.brushes.borrow().get(id) {
            return Some(b.clone());
        }
        let e = self.entry(id)?;
        let b = Arc::new(load_source(&e.source).ok()?);
        self.brushes.borrow_mut().insert(id.to_owned(), b.clone());
        Some(b)
    }

    /// A Krita entry's preset, as its file says it.
    pub fn preset(&self, id: &str) -> Option<Arc<Preset>> {
        if let Some(p) = self.presets.borrow().get(id) {
            return Some(p.clone());
        }
        let bytes = kpp_bytes(&self.entry(id)?.source)?;
        let p = Arc::new(Preset::read(&bytes).ok()?);
        self.presets.borrow_mut().insert(id.to_owned(), p.clone());
        Some(p)
    }

    /// A Krita entry's brush with the editor's `overrides` set over its
    /// preset, or why that does not build.
    pub fn brush_with(&self, id: &str, overrides: &[(String, String)]) -> Result<Arc<PixelBrush>, String> {
        if overrides.is_empty() {
            return self.brush(id).ok_or_else(|| format!("no brush `{id}`"));
        }
        if let Some((_, _, b)) = self.edited.borrow().iter().find(|(i, o, _)| i == id && o == overrides) {
            return b.clone();
        }
        let built = match (self.entry(id), self.preset(id)) {
            (Some(e), Some(p)) => build_edited(&e.source, &p.with_overrides(overrides)).map(Arc::new),
            _ => Err(format!("no brush `{id}`")),
        };
        let mut edited = self.edited.borrow_mut();
        edited.push((id.to_owned(), overrides.to_vec(), built.clone()));
        // A slider drag builds one per frame; only the last few matter.
        if edited.len() > 4 {
            edited.drain(..1);
        }
        built
    }

    /// Entry `id`'s preset with `overrides` set over it: what the editor
    /// shows. Rebuilt only when the overrides change.
    pub fn edited_preset(&self, id: &str, overrides: &[(String, String)]) -> Option<Arc<Preset>> {
        if let Some((i, o, p)) = &*self.edited_preset.borrow() {
            if i == id && o == overrides {
                return Some(p.clone());
            }
        }
        let p = Arc::new(self.preset(id)?.with_overrides(overrides));
        *self.edited_preset.borrow_mut() = Some((id.to_owned(), overrides.to_vec(), p.clone()));
        Some(p)
    }

    /// Save entry `id` with `overrides` as a new imported preset called
    /// `name`, carrying its tip, and return the new entry's id.
    pub fn save_as_new(&mut self, id: &str, overrides: &[(String, String)], name: &str) -> anyhow::Result<String> {
        use crate::tools::krita::{edit, kpp_write};
        let root = self.root.clone().ok_or_else(|| anyhow::anyhow!("there is no brushes folder to save into"))?;
        let source = self.entry(id).ok_or_else(|| anyhow::anyhow!("no brush `{id}`"))?.source.clone();
        let mut p = self.preset(id).ok_or_else(|| anyhow::anyhow!("no preset for `{id}`"))?.with_overrides(overrides);
        p.name = name.trim().to_owned();
        let def = p.string("brush_definition").unwrap_or_default();
        if let Some(file) = edit::attr(&def, "Brush", "filename").filter(|f| !f.is_empty()) {
            if !p.resources.iter().any(|r| r.kind == "brushes" && r.filename == file) {
                let data = source_tip(&source, &file).ok_or_else(|| anyhow::anyhow!("the tip `{file}` is missing"))?;
                kpp_write::embed_tip(&mut p, &file, data);
            }
        }
        let brush = build_edited(&source, &p).map_err(|e| anyhow::anyhow!(e))?;
        let kpp = kpp_write::write(&p, &krita_picture(Arc::new(brush), 128))?;
        let file = format!("{}.kpp", import::folder_name(&p.name));
        let info = import::SourceInfo {
            origin: format!("edited from {id}"),
            file: file.clone(),
            hash: format!("{:016x}", import::hash(&kpp)),
        };
        let new_id = import::install(&root, &p.name, vec![(file, kpp)], info)?;
        self.rescan();
        Ok(new_id)
    }

    /// The tip files an edit of entry `id` can switch to: the shipped
    /// ones, and any beside an imported preset.
    pub fn tips_for(&self, id: &str) -> Vec<String> {
        let mut tips: Vec<String> = defaults::TIPS.iter().map(|s| s.to_string()).collect();
        if let Some(Entry { source: Source::Folder(dir, _), .. }) = self.entry(id) {
            for f in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let ext = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
                if TIP_EXTENSIONS.contains(&ext.as_str()) && !tips.contains(&name) {
                    tips.push(name);
                }
            }
        }
        tips
    }

    pub fn thumbnail(&self, id: &str) -> Option<&egui::ColorImage> {
        self.thumbs.get(id)
    }

    /// Re-read the imported folder, after an import.
    pub fn rescan(&mut self) {
        self.entries.retain(|e| e.group != Group::Imported);
        self.brushes.borrow_mut().retain(|id, _| !id.starts_with("user/"));
        self.presets.borrow_mut().retain(|id, _| !id.starts_with("user/"));
        self.edited.borrow_mut().retain(|(id, _, _)| !id.starts_with("user/"));
        *self.edited_preset.borrow_mut() = None;
        self.thumbs.retain(|id, _| !id.starts_with("user/"));
        if let Some(root) = &self.root {
            self.entries.extend(scan_imported(root));
        }
        self.start_loading();
    }

    /// Where imported presets live.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn settle(lib: &mut BrushLibrary) {
        let t0 = std::time::Instant::now();
        while lib.loading() && t0.elapsed().as_secs() < 30 {
            lib.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        lib.poll();
    }

    #[test]
    fn names_read_like_krita_s_picker() {
        assert_eq!(display_name("c)_Pencil-5_Tilted.kpp"), "Pencil-5 Tilted");
        assert_eq!(display_name("b)_Basic-5_Size_default.kpp"), "Basic-5 Size default");
        assert_eq!(display_name("My_Brush.kpp"), "My Brush");
    }

    #[test]
    fn the_library_lists_three_groups_and_loads_the_krita_ones() {
        let mut lib = BrushLibrary::new(None);
        assert_eq!(lib.entries.iter().filter(|e| e.group == Group::App).count(), BrushSettings::PRESETS.len());
        assert_eq!(lib.entries.iter().filter(|e| e.group == Group::Krita).count(), 24);
        settle(&mut lib);
        let pencil = lib.entry("krita/c)_Pencil-5_Tilted").unwrap();
        assert_eq!(pencil.status, Status::Ready);
        assert!(lib.thumbnail("krita/c)_Pencil-5_Tilted").is_some());
        assert!(lib.thumbnail("app/Pencil").is_some());
        assert!(lib.entries.iter().filter(|e| e.group == Group::Krita).all(|e| e.status == Status::Ready));
    }

    /// An imported preset this engine cannot paint: a Krita hairy brush.
    pub(crate) fn write_hairy(root: &Path) {
        let dir = root.join("Hairy");
        std::fs::create_dir_all(&dir).unwrap();
        let xml = r#"<Preset paintopid="hairybrush" name="Hairy"><param name="x" type="string">1</param></Preset>"#;
        std::fs::write(dir.join("Hairy.kpp"), crate::tools::krita::kpp::with_xml(xml)).unwrap();
    }

    /// The shipped Pencil-5 is the same preset as the app's built-in one,
    /// so its strokes hash the same.
    #[test]
    fn the_library_s_pencil5_is_the_built_in_one() {
        let lib = BrushLibrary::new(None);
        let a = lib.brush("krita/c)_Pencil-5_Tilted").unwrap();
        let b = crate::tools::krita::KritaPreset::Pencil5Tilted.brush();
        assert_eq!(a.diameter(), b.diameter());
        assert_eq!(a.wash, b.wash);
        assert!(lib.brush("krita/nope").is_none());
    }

    /// An edit paints differently, and Save as new keeps it: the saved
    /// preset carries its tip and loads as the edited brush.
    #[test]
    fn edits_build_and_save_as_new_presets() {
        let root = crate::brush_library::import::tests::tmp("save-as");
        let mut lib = BrushLibrary::new(Some(root.clone()));
        let id = "krita/c)_Pencil-5_Tilted";
        let edit = vec![("FlowValue".to_owned(), "0.3".to_owned()), ("PaintOpAction".to_owned(), "2".to_owned())];
        let b = lib.brush_with(id, &edit).unwrap();
        assert!(b.wash, "the edit applies");
        assert!(!lib.brush(id).unwrap().wash, "the preset itself is untouched");
        assert!(Arc::ptr_eq(&b, &lib.brush_with(id, &edit).unwrap()), "built once");
        assert_eq!(lib.edited_preset(id, &edit).unwrap().string("FlowValue").as_deref(), Some("0.3"));
        let bad = vec![("brush_definition".to_owned(), "<Brush type=\"nope\"/>".to_owned())];
        assert!(lib.brush_with(id, &bad).is_err());

        let new_id = lib.save_as_new(id, &edit, "Soft wash").unwrap();
        assert_eq!(new_id, "user/Soft wash");
        let dir = root.join("Soft wash");
        assert!(dir.join("Soft wash.kpp").is_file());
        assert!(!dir.join("gradient.png").exists(), "the tip travels inside the preset");
        settle(&mut lib);
        assert_eq!(lib.entry(&new_id).unwrap().status, Status::Ready);
        let saved = lib.brush(&new_id).unwrap();
        assert!(saved.wash);
        assert_eq!(saved.diameter(), b.diameter());
        assert!(lib.thumbnail(&new_id).is_some(), "its picture is the engine's stroke");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn imported_presets_are_read_from_their_folders() {
        let root = std::env::temp_dir().join(format!("animator-brushes-{}", std::process::id()));
        let dir = root.join("Basic");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b)_Basic-2_Opacity.kpp"), defaults::kpp("b)_Basic-2_Opacity.kpp").unwrap()).unwrap();
        let mut lib = BrushLibrary::new(Some(root.clone()));
        settle(&mut lib);
        let e = lib.entry("user/Basic").expect("imported entry");
        assert_eq!(e.group, Group::Imported);
        assert_eq!(e.status, Status::Ready);
        assert!(lib.brush("user/Basic").unwrap().wash);
        // Refusals say why, and have no brush.
        write_hairy(&root);
        lib.rescan();
        settle(&mut lib);
        let hairy = lib.entry("user/Hairy").expect("listed even though refused");
        assert!(matches!(&hairy.status, Status::Refused(why) if why.contains("hairybrush")), "{:?}", hairy.status);
        std::fs::remove_dir_all(&root).ok();
    }
}
