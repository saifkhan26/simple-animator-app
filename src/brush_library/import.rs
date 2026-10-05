//! Importing presets from an installed Krita.
//!
//! Krita keeps presets in two kinds of place: loose `.kpp` files in a
//! `paintoppresets` folder (the user's resource folder, and Krita's own
//! install), and `.bundle` files, which are zips with `paintoppresets/` and
//! `brushes/` inside. A preset names its tip by file name, and Krita finds
//! it wherever it is installed, so a preset in one bundle may use a tip from
//! another.
//!
//! The importer lists every preset it finds, with whether this app paints it
//! (and why not), on a background thread. Importing one copies the `.kpp`
//! and the tips it uses into `<brushes>/<name>/`, which is what the brush
//! library reads; nothing is ever read live from Krita.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use anyhow::{anyhow, bail, Context, Result};
use zip::ZipArchive;

use super::{display_name, kpp_thumbnail, FolderTips};
use crate::tools::krita::{defaults, PixelBrush, Resources};

/// Where Krita keeps its resources on this machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KritaPlaces {
    /// The user's resource folder: kritarc's `ResourceDirectory`, else
    /// `<data>/krita`.
    pub user: Option<PathBuf>,
    /// `share/krita` beside the Krita executable.
    pub install: Option<PathBuf>,
}

impl KritaPlaces {
    /// Krita's folders, given where its executable is (if known).
    pub fn find(krita_exe: Option<&Path>) -> Self {
        let user = kritarc_resource_dir()
            .filter(|d| d.is_dir())
            .or_else(|| dirs::data_dir().map(|d| d.join("krita")))
            .filter(|d| d.is_dir());
        let install = krita_exe
            .and_then(|exe| Some(exe.parent()?.parent()?.join("share").join("krita")))
            .filter(|d| d.is_dir());
        Self { user, install }
    }
}

/// kritarc lives in the local config folder (Windows, Linux) or the
/// preferences folder (macOS).
fn kritarc_resource_dir() -> Option<PathBuf> {
    [dirs::config_local_dir(), dirs::preference_dir()]
        .into_iter()
        .flatten()
        .find_map(|d| resource_dir_in(&std::fs::read_to_string(d.join("kritarc")).ok()?))
}

/// The `ResourceDirectory=` value in kritarc's text.
fn resource_dir_in(kritarc: &str) -> Option<PathBuf> {
    kritarc
        .lines()
        .find_map(|l| l.trim().strip_prefix("ResourceDirectory="))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Something that holds presets.
#[derive(Clone, Debug, PartialEq)]
pub enum Container {
    /// A folder of `.kpp` files.
    Folder(PathBuf),
    /// A `.bundle`, by its index in the importer's bundle list.
    Bundle(usize),
}

/// A heading in the import list: one folder or bundle.
#[derive(Clone, Debug)]
pub struct SourceGroup {
    pub label: String,
    pub path: PathBuf,
    pub container: Container,
}

/// Whether a preset is already here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Known {
    New,
    /// An identical `.kpp` is in the brushes folder.
    Imported,
    /// It is one of the presets the app ships.
    BuiltIn,
}

/// One preset found in Krita.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub group: usize,
    /// The file name in a folder, or the entry name in a bundle.
    pub file: String,
    pub name: String,
    /// Whether this app paints it, or why not.
    pub status: Result<(), String>,
    pub hash: u64,
    pub known: Known,
    /// The library entry it already is, if built in or imported.
    pub id: Option<String>,
    pub thumb: Option<egui::ColorImage>,
}

/// FNV-1a over a preset's bytes: identical files are the same preset.
pub(super) fn hash(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

struct Bundle {
    archive: RefCell<ZipArchive<File>>,
    /// Tip file name → entry name.
    tips: HashMap<String, String>,
}

/// Every place a tip can come from, opened once.
pub struct Pool {
    /// `brushes` folders: the user's, then the install's.
    folders: Vec<PathBuf>,
    /// `None` for a bundle that would not open.
    bundles: Vec<Option<Bundle>>,
}

impl Pool {
    pub fn open(places: &KritaPlaces, bundles: &[PathBuf]) -> Self {
        let folders = [&places.user, &places.install]
            .into_iter()
            .flatten()
            .map(|d| d.join("brushes"))
            .filter(|d| d.is_dir())
            .collect();
        let bundles = bundles
            .iter()
            .map(|p| {
                let archive = ZipArchive::new(File::open(p).ok()?).ok()?;
                let tips = archive
                    .file_names()
                    .filter_map(|n| Some((n.strip_prefix("brushes/")?.to_owned(), n.to_owned())))
                    .filter(|(base, _)| !base.is_empty() && !base.contains('/'))
                    .collect();
                Some(Bundle {
                    archive: RefCell::new(archive),
                    tips,
                })
            })
            .collect();
        Self { folders, bundles }
    }

    fn bundle_file(&self, b: usize, entry: &str) -> Option<Vec<u8>> {
        use std::io::Read;
        let bundle = self.bundles.get(b)?.as_ref()?;
        let mut archive = bundle.archive.borrow_mut();
        let mut f = archive.by_name(entry).ok()?;
        let mut out = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut out).ok()?;
        Some(out)
    }

    /// The `.kpp` files a bundle holds, sorted.
    fn bundle_presets(&self, b: usize) -> Vec<String> {
        let Some(Some(bundle)) = self.bundles.get(b) else {
            return Vec::new();
        };
        let mut v: Vec<String> = bundle
            .archive
            .borrow()
            .file_names()
            .filter(|n| n.starts_with("paintoppresets/") && n.to_ascii_lowercase().ends_with(".kpp"))
            .map(str::to_owned)
            .collect();
        v.sort();
        v
    }

    /// A tip as Krita would find it: in the preset's own bundle, then the
    /// `brushes` folders, then any other bundle.
    fn tip(&self, own: Option<usize>, filename: &str) -> Option<Vec<u8>> {
        let base = Path::new(filename).file_name()?.to_str()?;
        let from_bundle = |i: usize| {
            let entry = self.bundles.get(i)?.as_ref()?.tips.get(base)?;
            self.bundle_file(i, entry)
        };
        if let Some(d) = own.and_then(from_bundle) {
            return Some(d);
        }
        if let Some(d) = self.folders.iter().find_map(|dir| std::fs::read(dir.join(base)).ok()) {
            return Some(d);
        }
        (0..self.bundles.len()).filter(|&i| Some(i) != own).find_map(from_bundle)
    }

    /// A candidate's `.kpp` bytes.
    fn kpp(&self, group: &SourceGroup, file: &str) -> Option<Vec<u8>> {
        match &group.container {
            Container::Folder(dir) => std::fs::read(dir.join(file)).ok(),
            Container::Bundle(b) => self.bundle_file(*b, file),
        }
    }
}

/// Tips from the pool, remembering which were used so an import can copy
/// them.
struct Recording<'a> {
    pool: &'a Pool,
    own: Option<usize>,
    used: RefCell<Vec<(String, Vec<u8>)>>,
}

impl Resources for Recording<'_> {
    fn tip(&self, filename: &str) -> Option<Cow<'_, [u8]>> {
        let data = self.pool.tip(self.own, filename)?;
        let base = Path::new(filename).file_name()?.to_string_lossy().into_owned();
        self.used.borrow_mut().push((base, data.clone()));
        Some(Cow::Owned(data))
    }
}

fn own_bundle(group: &SourceGroup) -> Option<usize> {
    match group.container {
        Container::Bundle(b) => Some(b),
        Container::Folder(_) => None,
    }
}

/// `Krita_4_Default_Resources.bundle` → `Krita 4 Default Resources`.
fn bundle_label(path: &Path) -> String {
    path.file_stem().unwrap_or_default().to_string_lossy().replace('_', " ")
}

/// What a scan will read: Krita's folders and bundles, the user's first.
/// Bundles of the same file name in both places are listed once.
pub fn plan(places: &KritaPlaces) -> (Vec<SourceGroup>, Vec<PathBuf>) {
    let mut groups = Vec::new();
    let mut bundles: Vec<PathBuf> = Vec::new();
    let sorted = |dir: &Path, ext: &str| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        v.retain(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)));
        v.sort();
        v
    };
    for (root, label, bundle_dir) in [
        (&places.user, "Your Krita presets", None),
        (&places.install, "Krita's install", Some("bundles")),
    ] {
        let Some(root) = root else { continue };
        let presets = root.join("paintoppresets");
        if presets.is_dir() {
            groups.push(SourceGroup {
                label: label.to_owned(),
                path: presets.clone(),
                container: Container::Folder(presets),
            });
        }
        let dir = bundle_dir.map_or_else(|| root.clone(), |d| root.join(d));
        for b in sorted(&dir, "bundle") {
            if bundles.iter().any(|seen| seen.file_name() == b.file_name()) {
                continue;
            }
            groups.push(SourceGroup {
                label: bundle_label(&b),
                path: b.clone(),
                container: Container::Bundle(bundles.len()),
            });
            bundles.push(b);
        }
    }
    (groups, bundles)
}

/// What the scan thread sends.
pub enum ScanMsg {
    Found(Candidate),
    Done,
}

/// The library ids of the presets the app ships and of those in `root`,
/// by the hash of their `.kpp`.
fn known_hashes(root: Option<&Path>) -> (HashMap<u64, String>, HashMap<u64, String>) {
    let built_in = defaults::PRESETS
        .iter()
        .filter_map(|n| Some((hash(&defaults::kpp(n)?), format!("krita/{}", n.trim_end_matches(".kpp")))))
        .collect();
    let mut imported = HashMap::new();
    for dir in root.and_then(|r| std::fs::read_dir(r).ok()).into_iter().flatten().flatten() {
        let folder = dir.file_name().to_string_lossy().into_owned();
        if folder.starts_with('.') {
            continue;
        }
        for f in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            if f.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".kpp") {
                if let Ok(b) = std::fs::read(f.path()) {
                    imported.entry(hash(&b)).or_insert_with(|| format!("user/{folder}"));
                }
            }
        }
    }
    (built_in, imported)
}

/// List the presets in `groups[first..]` on a background thread. A preset
/// identical to one already listed (Krita often has the same file in two
/// places) is left out.
pub fn scan(
    places: KritaPlaces,
    groups: Vec<SourceGroup>,
    first: usize,
    bundles: Vec<PathBuf>,
    root: Option<PathBuf>,
    mut seen: HashSet<u64>,
) -> Receiver<ScanMsg> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("krita-import".into()).spawn(move || {
        let pool = Pool::open(&places, &bundles);
        let (built_in, imported) = known_hashes(root.as_deref());
        for (gi, group) in groups.iter().enumerate().skip(first) {
            let files: Vec<String> = match &group.container {
                Container::Folder(dir) => {
                    let mut v: Vec<String> = std::fs::read_dir(dir)
                        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                        .unwrap_or_default();
                    v.retain(|n| n.to_ascii_lowercase().ends_with(".kpp"));
                    v.sort();
                    v
                }
                Container::Bundle(b) => pool.bundle_presets(*b),
            };
            for file in files {
                let Some(bytes) = pool.kpp(group, &file) else { continue };
                let h = hash(&bytes);
                if !seen.insert(h) {
                    continue;
                }
                let rec = Recording {
                    pool: &pool,
                    own: own_bundle(group),
                    used: RefCell::new(Vec::new()),
                };
                let status = PixelBrush::load(&bytes, &rec).map(|_| ()).map_err(|e| format!("{e:#}"));
                let base = file.rsplit('/').next().unwrap_or(&file);
                let (known, id) = if let Some(id) = built_in.get(&h) {
                    (Known::BuiltIn, Some(id.clone()))
                } else if let Some(id) = imported.get(&h) {
                    (Known::Imported, Some(id.clone()))
                } else {
                    (Known::New, None)
                };
                let c = Candidate {
                    group: gi,
                    name: display_name(base),
                    file,
                    status,
                    hash: h,
                    known,
                    id,
                    thumb: kpp_thumbnail(&bytes),
                };
                if tx.send(ScanMsg::Found(c)).is_err() {
                    return;
                }
            }
        }
        tx.send(ScanMsg::Done).ok();
    });
    if spawned.is_err() {
        let (tx, rx) = mpsc::channel();
        tx.send(ScanMsg::Done).ok();
        return rx;
    }
    rx
}

/// A folder name from a preset's name: no path separators or characters
/// Windows refuses, and not one of its reserved device names.
pub(super) fn folder_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_()+,.'".contains(c) { c } else { '_' })
        .take(60)
        .collect();
    s = s.trim_matches(|c| c == ' ' || c == '.').to_owned();
    if s.is_empty() {
        s = "Brush".to_owned();
    }
    let stem = s.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        s.insert(0, '_');
    }
    s
}

/// Where an imported preset came from, kept beside it.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SourceInfo {
    pub origin: String,
    pub file: String,
    pub hash: String,
}

/// Copy a preset and the tips it uses into a new folder under `root`.
/// Returns the library id of the imported entry.
pub fn import(pool: &Pool, group: &SourceGroup, c: &Candidate, root: &Path) -> Result<String> {
    let bytes = pool.kpp(group, &c.file).ok_or_else(|| anyhow!("{} is gone", c.file))?;
    let rec = Recording {
        pool,
        own: own_bundle(group),
        used: RefCell::new(Vec::new()),
    };
    PixelBrush::load(&bytes, &rec).with_context(|| c.name.clone())?;
    let kpp_name = c.file.rsplit('/').next().unwrap_or(&c.file).to_owned();
    let mut files = vec![(kpp_name, bytes)];
    files.extend(rec.used.into_inner());
    let info = SourceInfo {
        origin: group.path.display().to_string(),
        file: c.file.clone(),
        hash: format!("{:016x}", c.hash),
    };
    install(root, &c.name, files, info)
}

/// Put a preset's `files` (the `.kpp` first) and its `source.toml` into a
/// new folder under `root` named after `name`, and return its library id.
/// The folder is built aside and renamed into place, so the library never
/// sees half of it, and must load on its own before it is.
pub(super) fn install(root: &Path, name: &str, files: Vec<(String, Vec<u8>)>, info: SourceInfo) -> Result<String> {
    std::fs::create_dir_all(root).with_context(|| root.display().to_string())?;
    let tmp = root.join(format!(".import-{}", std::process::id()));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).ok();
    }
    std::fs::create_dir(&tmp).with_context(|| tmp.display().to_string())?;
    let written = (|| -> Result<()> {
        for (file, data) in &files {
            std::fs::write(tmp.join(file), data)?;
        }
        std::fs::write(tmp.join("source.toml"), toml::to_string(&info)?)?;
        let kpp = &files.first().ok_or_else(|| anyhow!("no preset to install"))?.1;
        PixelBrush::load(kpp, &FolderTips(&tmp)).context("the copy does not load")?;
        Ok(())
    })();
    if let Err(e) = written {
        std::fs::remove_dir_all(&tmp).ok();
        return Err(e);
    }
    let base = folder_name(name);
    let mut folder = base.clone();
    let mut n = 2;
    while root.join(&folder).exists() {
        folder = format!("{base}-{n}");
        n += 1;
    }
    if let Err(e) = std::fs::rename(&tmp, root.join(&folder)) {
        std::fs::remove_dir_all(&tmp).ok();
        bail!("moving the brush into place: {e}");
    }
    Ok(format!("user/{folder}"))
}

/// An import dialog's state: what was found, and the scans still running.
pub struct KritaImport {
    pub places: KritaPlaces,
    pub groups: Vec<SourceGroup>,
    bundles: Vec<PathBuf>,
    pub candidates: Vec<Candidate>,
    scans: Vec<Receiver<ScanMsg>>,
    root: Option<PathBuf>,
    pool: Option<Pool>,
    pub search: String,
    pub show_unsupported: bool,
    /// The last import's outcome, shown under the list.
    pub notice: Option<String>,
    /// Candidates' thumbnails, uploaded by the window as it draws them.
    pub thumbs: HashMap<usize, egui::TextureHandle>,
}

impl KritaImport {
    /// Start reading Krita's presets. Imports go into `root`.
    pub fn start(places: KritaPlaces, root: Option<PathBuf>) -> Self {
        let (groups, bundles) = plan(&places);
        let mut s = Self {
            places,
            groups: Vec::new(),
            bundles: Vec::new(),
            candidates: Vec::new(),
            scans: Vec::new(),
            root,
            pool: None,
            search: String::new(),
            show_unsupported: false,
            notice: None,
            thumbs: HashMap::new(),
        };
        s.add_groups(groups, bundles);
        s
    }

    fn add_groups(&mut self, groups: Vec<SourceGroup>, bundles: Vec<PathBuf>) {
        let first = self.groups.len();
        let offset = self.bundles.len();
        self.groups.extend(groups.into_iter().map(|mut g| {
            if let Container::Bundle(b) = &mut g.container {
                *b += offset;
            }
            g
        }));
        self.bundles.extend(bundles);
        self.pool = None;
        let seen = self.candidates.iter().map(|c| c.hash).collect();
        self.scans.push(scan(
            self.places.clone(),
            self.groups.clone(),
            first,
            self.bundles.clone(),
            self.root.clone(),
            seen,
        ));
    }

    /// Add `.kpp` files and `.bundle`s the user picked.
    pub fn add_files(&mut self, paths: &[PathBuf]) {
        let mut groups = Vec::new();
        let mut bundles = Vec::new();
        let mut loose = std::collections::BTreeSet::new();
        for p in paths {
            let ext = p.extension().map(|e| e.to_ascii_lowercase());
            if ext.as_deref() == Some("bundle".as_ref()) {
                groups.push(SourceGroup {
                    label: bundle_label(p),
                    path: p.clone(),
                    container: Container::Bundle(bundles.len()),
                });
                bundles.push(p.clone());
            } else if ext.as_deref() == Some("kpp".as_ref()) {
                if let Some(dir) = p.parent() {
                    loose.insert(dir.to_path_buf());
                }
            }
        }
        // A picked `.kpp` brings its folder: its tips are usually there.
        for dir in loose {
            groups.push(SourceGroup {
                label: dir.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                path: dir.clone(),
                container: Container::Folder(dir),
            });
        }
        if !groups.is_empty() {
            self.add_groups(groups, bundles);
        }
    }

    /// Take in what the scans found. Returns whether anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        self.scans.retain(|rx| loop {
            match rx.try_recv() {
                Ok(ScanMsg::Found(c)) => {
                    changed = true;
                    self.candidates.push(c);
                }
                Ok(ScanMsg::Done) | Err(mpsc::TryRecvError::Disconnected) => {
                    changed = true;
                    break false;
                }
                Err(mpsc::TryRecvError::Empty) => break true,
            }
        });
        changed
    }

    pub fn scanning(&self) -> bool {
        !self.scans.is_empty()
    }

    /// Import candidate `i`. Returns the new library id.
    pub fn import(&mut self, i: usize) -> Result<String> {
        let root = self.root.clone().ok_or_else(|| anyhow!("there is no brushes folder to import into"))?;
        let pool = self.pool.get_or_insert_with(|| Pool::open(&self.places, &self.bundles));
        let c = self.candidates.get(i).ok_or_else(|| anyhow!("no such preset"))?;
        let id = import(pool, &self.groups[c.group], c, &root)?;
        self.candidates[i].known = Known::Imported;
        self.candidates[i].id = Some(id.clone());
        Ok(id)
    }

    /// The candidates `group` shows under the current search and filter.
    pub fn visible(&self, group: usize) -> Vec<usize> {
        let q = self.search.trim().to_lowercase();
        self.candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.group == group)
            .filter(|(_, c)| self.show_unsupported || c.status.is_ok())
            .filter(|(_, c)| q.is_empty() || c.name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("animator-import-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn bundle(path: &Path, files: &[(&str, Vec<u8>)]) {
        let mut z = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    /// A shipped preset, changed so it is not the shipped one.
    fn edited(name: &str) -> Vec<u8> {
        let xml = crate::tools::krita::kpp::preset_xml(&defaults::kpp(name).unwrap()).unwrap();
        crate::tools::krita::kpp::with_xml(&xml.replacen("</Preset>", "<!-- edited --></Preset>", 1))
    }

    /// A Krita resource folder with:
    /// - `paintoppresets/`: the shipped Basic-2 (built in), a hairy brush
    ///   (refused), and My Rake, whose tip is in `brushes/`;
    /// - `Pencils.bundle`: My Grain, whose `.gih` tip is in the bundle, and
    ///   the shipped Pencil-5.
    pub(crate) fn fake_krita(tag: &str) -> (PathBuf, KritaPlaces) {
        let root = tmp(tag);
        let user = root.join("krita");
        let presets = user.join("paintoppresets");
        std::fs::create_dir_all(&presets).unwrap();
        std::fs::create_dir_all(user.join("brushes")).unwrap();
        let kpp = |n: &str| defaults::kpp(n).unwrap();
        std::fs::write(presets.join("b)_Basic-2_Opacity.kpp"), kpp("b)_Basic-2_Opacity.kpp")).unwrap();
        std::fs::write(presets.join("My_Rake.kpp"), edited("b)_Basic-6_Details.kpp")).unwrap();
        let hairy = r#"<Preset paintopid="hairybrush" name="Hairy"><param name="x" type="string">1</param></Preset>"#;
        std::fs::write(presets.join("Hairy.kpp"), crate::tools::krita::kpp::with_xml(hairy)).unwrap();
        std::fs::write(user.join("brushes").join("plain_rake.png"), defaults::file("brushes/plain_rake.png").unwrap()).unwrap();
        bundle(
            &user.join("Pencils.bundle"),
            &[
                ("paintoppresets/My_Grain.kpp", edited("c)_Pencil-4_Soft.kpp")),
                ("paintoppresets/c)_Pencil-5_Tilted.kpp", kpp("c)_Pencil-5_Tilted.kpp")),
                ("brushes/graphite_grain.gih", defaults::file("brushes/graphite_grain.gih").unwrap()),
                ("brushes/gradient.png", defaults::file("brushes/gradient.png").unwrap()),
            ],
        );
        let places = KritaPlaces {
            user: Some(user),
            install: None,
        };
        (root, places)
    }

    pub(crate) fn settle(imp: &mut KritaImport) {
        let t0 = std::time::Instant::now();
        while imp.scanning() && t0.elapsed().as_secs() < 30 {
            imp.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn find<'a>(imp: &'a KritaImport, name: &str) -> (usize, &'a Candidate) {
        imp.candidates
            .iter()
            .enumerate()
            .find(|(_, c)| c.name == name)
            .unwrap_or_else(|| panic!("no {name} in {:?}", imp.candidates.iter().map(|c| &c.name).collect::<Vec<_>>()))
    }

    #[test]
    fn kritarc_names_the_resource_folder() {
        let rc = "[General]\nfoo=1\nResourceDirectory=D:/Art/krita\n";
        assert_eq!(resource_dir_in(rc), Some(PathBuf::from("D:/Art/krita")));
        assert_eq!(resource_dir_in("ResourceDirectory=\n"), None);
    }

    #[test]
    fn folder_names_are_safe() {
        assert_eq!(folder_name("Pencil-4 Soft"), "Pencil-4 Soft");
        assert_eq!(folder_name("a/b\\c:d"), "a_b_c_d");
        assert_eq!(folder_name("..."), "Brush");
        assert_eq!(folder_name("con"), "_con");
        assert_eq!(folder_name("COM1.x"), "_COM1.x");
        assert_eq!(folder_name("Combo"), "Combo");
    }

    #[test]
    fn the_scan_lists_folders_and_bundles_and_says_what_paints() {
        let (root, places) = fake_krita("scan");
        let mut imp = KritaImport::start(places, Some(root.join("brushes")));
        settle(&mut imp);
        assert_eq!(imp.groups.len(), 2, "{:?}", imp.groups);
        assert_eq!(imp.groups[0].label, "Your Krita presets");
        assert_eq!(imp.groups[1].label, "Pencils");
        assert_eq!(find(&imp, "Basic-2 Opacity").1.status, Ok(()));
        assert_eq!(find(&imp, "Basic-2 Opacity").1.known, Known::BuiltIn);
        assert_eq!(find(&imp, "Basic-2 Opacity").1.id.as_deref(), Some("krita/b)_Basic-2_Opacity"));
        assert_eq!(find(&imp, "Pencil-5 Tilted").1.known, Known::BuiltIn);
        assert!(find(&imp, "Pencil-5 Tilted").1.thumb.is_some(), "a .kpp's own picture");
        let hairy = find(&imp, "Hairy").1;
        assert!(matches!(&hairy.status, Err(why) if why.contains("hairybrush")), "{:?}", hairy.status);
        // The pipe tip comes from the bundle; the rake from the folder.
        assert_eq!(find(&imp, "My Grain").1.status, Ok(()));
        assert_eq!(find(&imp, "My Grain").1.known, Known::New);
        assert_eq!(find(&imp, "My Rake").1.status, Ok(()));
        // Unpaintable ones are hidden until asked for.
        assert_eq!(imp.visible(0).len(), 2);
        imp.show_unsupported = true;
        assert_eq!(imp.visible(0).len(), 3);
        imp.search = "hair".into();
        assert_eq!(imp.visible(0).len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn importing_copies_the_preset_and_its_tips() {
        let (root, places) = fake_krita("copy");
        let brushes = root.join("brushes");
        let mut imp = KritaImport::start(places.clone(), Some(brushes.clone()));
        settle(&mut imp);
        let (i, _) = find(&imp, "My Grain");
        let id = imp.import(i).unwrap();
        assert_eq!(id, "user/My Grain");
        assert_eq!(imp.candidates[i].known, Known::Imported);
        assert_eq!(imp.candidates[i].id.as_deref(), Some("user/My Grain"));
        let dir = brushes.join("My Grain");
        for f in ["My_Grain.kpp", "graphite_grain.gih", "source.toml"] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        let info: SourceInfo = toml::from_str(&std::fs::read_to_string(dir.join("source.toml")).unwrap()).unwrap();
        assert!(info.origin.ends_with("Pencils.bundle"), "{}", info.origin);
        assert_eq!(info.file, "paintoppresets/My_Grain.kpp");
        // A second import of the same preset gets its own folder.
        imp.import(i).unwrap();
        assert!(brushes.join("My Grain-2").is_dir());
        // A folder preset brings the tip from Krita's `brushes`.
        let (r, _) = find(&imp, "My Rake");
        imp.import(r).unwrap();
        assert!(brushes.join("My Rake").join("plain_rake.png").is_file());
        // No temporary folder is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&brushes)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty());

        // The library reads it, and a new scan knows it is here.
        let mut lib = super::super::BrushLibrary::new(Some(brushes.clone()));
        let t0 = std::time::Instant::now();
        while lib.loading() && t0.elapsed().as_secs() < 30 {
            lib.poll();
        }
        assert_eq!(lib.entry("user/My Grain").unwrap().status, super::super::Status::Ready);
        assert_eq!(lib.entry("user/My Rake").unwrap().status, super::super::Status::Ready);
        let mut again = KritaImport::start(places, Some(brushes));
        settle(&mut again);
        assert_eq!(find(&again, "My Grain").1.known, Known::Imported);
        assert_eq!(find(&again, "My Grain").1.id.as_deref(), Some("user/My Grain"));
        // A refused preset cannot be imported.
        again.show_unsupported = true;
        let (h, _) = find(&again, "Hairy");
        assert!(again.import(h).is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn picked_files_join_the_list() {
        let (root, places) = fake_krita("pick");
        let mut imp = KritaImport::start(KritaPlaces::default(), Some(root.join("brushes")));
        settle(&mut imp);
        assert!(imp.candidates.is_empty());
        let user = places.user.unwrap();
        imp.add_files(&[user.join("Pencils.bundle"), user.join("paintoppresets").join("Hairy.kpp")]);
        settle(&mut imp);
        assert_eq!(imp.groups.len(), 2);
        assert_eq!(find(&imp, "My Grain").1.status, Ok(()));
        assert!(find(&imp, "Hairy").1.status.is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    /// What this machine's Krita offers, per folder or bundle.
    ///
    /// ```text
    /// cargo test krita_import_survey -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn krita_import_survey() {
        let exe = crate::krita_link::find_krita(None);
        let places = KritaPlaces::find(exe.as_deref());
        println!("{places:?}");
        let mut imp = KritaImport::start(places, None);
        settle(&mut imp);
        let mut reasons: HashMap<String, usize> = HashMap::new();
        for (gi, g) in imp.groups.iter().enumerate() {
            let all: Vec<&Candidate> = imp.candidates.iter().filter(|c| c.group == gi).collect();
            let ok = all.iter().filter(|c| c.status.is_ok()).count();
            println!("{:40} {ok:3} of {:3} paint", g.label, all.len());
            for c in &all {
                if let Err(why) = &c.status {
                    let why = why.split(": ").last().unwrap_or(why);
                    // Names elided, except the setting a refusal turns on.
                    let key = if why.contains("turns on") {
                        why.split('`').nth(3).unwrap_or(why).to_owned()
                    } else {
                        why.split('`').enumerate().map(|(i, s)| if i % 2 == 1 { "…" } else { s }).collect()
                    };
                    *reasons.entry(key).or_default() += 1;
                }
            }
        }
        let mut reasons: Vec<_> = reasons.into_iter().collect();
        reasons.sort_by(|a, b| b.1.cmp(&a.1));
        for (why, n) in reasons {
            println!("{n:4}  {why}");
        }
    }
}
