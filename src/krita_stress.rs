//! Stress tests for the Krita link.
//!
//! * [`random_sessions`] — hundreds of seeded two-sided editing sessions. Each
//!   round both sides may edit (paint, retime, new drawings, add / delete /
//!   reorder / rename layers, `-x` layers, reference toggles, timeline
//!   growth); Krita saves and the app pulls; sometimes the app sends again and
//!   Krita reloads, with or without saving its own edits first. After every
//!   pull and every send the promises the link makes are checked. A failure
//!   names its seed and round.
//! * `scale_*` (ignored; they print timings) — the user's real canvas, and a
//!   long timeline.
//!
//! "Krita" is a [`Project`] saved and opened through our own `.kra` code; the
//! reader and writer are checked against Krita-written files in `io::kra`.
//! The app side runs the same `send` / `prepare` / `plan` / `apply` the app
//! does.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::doc::canvas::Canvas;
use crate::doc::layer::Layer;
use crate::doc::project::Project;
use crate::io::kra;
use crate::krita_link::{self as kl, stays_put, Baseline};

/// xorshift64* — deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

const W: u32 = 16;
const H: u32 = 12;
const FRAMES: usize = 8;

fn doodle(rng: &mut Rng) -> Canvas {
    let mut c = Canvas::new(W, H);
    for _ in 0..1 + rng.below(6) {
        let i = rng.below((W * H) as usize) * 4;
        let px = [rng.next() as u8, rng.next() as u8, rng.next() as u8, 1 + rng.below(255) as u8];
        c.pixels_mut()[i..i + 4].copy_from_slice(&px);
    }
    c
}

fn scribble(c: &mut Canvas, rng: &mut Rng) {
    let i = rng.below((c.width * c.height) as usize) * 4;
    c.pixels_mut()[i..i + 4].copy_from_slice(&[rng.next() as u8, 200, rng.next() as u8, 255]);
}

fn blank(c: &Canvas) -> bool {
    c.is_all_zero()
}

/// What a layer shows, frame by frame: a content hash, 0 for nothing.
fn shows(p: &Project, l: &Layer, frames: usize) -> Vec<u64> {
    (0..frames)
        .map(|f| match l.resolve(f).and_then(|id| p.cell(id)) {
            Some(c) if !blank(c) => kl::cell_hash(c),
            _ => 0,
        })
        .collect()
}

fn attrs(l: &Layer) -> (String, u8, bool, bool) {
    (l.name.clone(), kra::opacity_u8(l.opacity), l.visible, l.locked)
}

/// Krita's document: its layers, each with its Krita uuid.
#[derive(Clone)]
struct Krita {
    p: Project,
    uuids: Vec<String>,
}

impl Krita {
    /// What Krita holds after opening `path`.
    fn open(path: &Path) -> Krita {
        let doc = kra::read(std::fs::read(path).unwrap()).unwrap();
        let (pw, ph) = (doc.width, doc.height);
        let frames = doc
            .layers
            .iter()
            .flat_map(|l| l.keys.iter().map(|k| k.0 + 1))
            .chain(doc.range_end.map(|e| e + 1))
            .max()
            .unwrap_or(1);
        let mut p = Project::new(pw, ph, 12.0);
        p.ensure_frame_count(frames);
        p.layers.clear();
        let mut uuids = Vec::new();
        let (x, y) = kra::cell_origin(pw, ph, pw, ph);
        for k in &doc.layers {
            let mut l = Layer::new(k.name.clone(), frames);
            l.opacity = k.opacity as f32 / 255.0;
            l.visible = k.visible;
            l.locked = k.locked;
            let base = p.cells.len();
            for d in &k.drawings {
                p.cells.push(d.place(x, y, pw, ph).0.into());
            }
            for &(t, di) in &k.keys {
                l.exposures[t] = Some(base + di);
            }
            p.layers.push(l);
            uuids.push(k.uuid.clone());
        }
        Krita { p, uuids }
    }

    fn save(&self, path: &Path) {
        let opts = kra::WriteOpts { layer_uuids: &self.uuids, selected_layer: 0, carried: &[] };
        std::fs::write(path, kra::write(&self.p, &opts).unwrap()).unwrap();
    }

    fn index_of(&self, uuid: &str) -> Option<usize> {
        self.uuids.iter().position(|u| u == uuid)
    }
}

/// The app side: the project and the link state the app keeps.
struct App {
    p: Project,
    links: HashMap<u64, String>,
    base: Baseline,
    path: PathBuf,
}

impl App {
    /// `start_send`, minus the worker thread.
    fn send(&mut self) -> kl::Sent {
        let keep: Vec<bool> = self.p.layers.iter().map(|l| !kl::stays_here(l)).collect();
        let mut uuids = Vec::new();
        for (l, &k) in self.p.layers.iter().zip(&keep) {
            if k {
                uuids.push(self.links.entry(l.uid).or_insert_with(kl::new_uuid).clone());
            } else {
                self.links.remove(&l.uid);
            }
        }
        let sent = kl::send(self.p.clone(), &keep, &uuids, Some(0), &self.path).unwrap();
        self.base = sent.baseline.clone();
        sent
    }

    /// `pull` + `plan`, as the app's worker and UI thread do.
    fn try_plan(&self) -> anyhow::Result<kl::PullPlan> {
        let bytes = std::fs::read(&self.path).unwrap();
        let present = kl::present(&self.p, &self.links);
        let pulled = kl::prepare(kra::read_filtered(bytes, &kl::comes_back).unwrap(), &self.base, &present);
        kl::plan(&self.p, &self.base, &self.links, pulled)
    }

    fn plan(&self) -> kl::PullPlan {
        self.try_plan().unwrap()
    }

    fn linked(&self, uuid: &str) -> Option<usize> {
        self.p.layers.iter().position(|l| self.links.get(&l.uid).is_some_and(|u| u == uuid))
    }
}

/// What changed on each side since the two last agreed.
#[derive(Default)]
struct Dirty {
    local_content: HashSet<u64>,
    local_attr: HashSet<u64>,
    local_order: bool,
    krita_content: HashSet<String>,
    krita_attr: HashSet<String>,
    krita_unsaved: bool,
}

struct Session {
    seed: u64,
    round: usize,
    rng: Rng,
    app: App,
    krita: Krita,
    d: Dirty,
    stats: Stats,
    /// Every step, for a failure report.
    log: Vec<String>,
}

#[derive(Default, Debug)]
struct Stats {
    pulls: usize,
    noops: usize,
    sends: usize,
    new_cells: usize,
    unlinks: usize,
    carried: usize,
    refused: usize,
}

macro_rules! check {
    ($s:expr, $cond:expr, $($msg:tt)+) => {
        assert!(
            $cond,
            "seed {} round {}: {}
--- what happened ---
{}",
            $s.seed,
            $s.round,
            format!($($msg)+),
            $s.log.join("
")
        )
    };
}

impl Session {
    fn new(seed: u64, dir: &Path) -> Session {
        let mut rng = Rng::new(seed);
        let mut p = Project::new(W, H, 12.0);
        p.ensure_frame_count(FRAMES);
        p.layers.clear();
        for n in 0..2 + rng.below(3) {
            let mut l = Layer::new(format!("L{n}"), FRAMES);
            for f in 0..FRAMES {
                if rng.chance(35) {
                    l.exposures[f] = Some(p.cells.len());
                    p.cells.push(doodle(&mut rng).into());
                }
            }
            p.layers.push(l);
        }
        // Layers that stay here: a reference background and a `-x` layer.
        let mut bg = Layer::new("BG", FRAMES);
        bg.reference = true;
        bg.exposures[0] = Some(p.cells.len());
        p.cells.push(doodle(&mut rng).into());
        p.layers.insert(0, bg);
        let mut notes = Layer::new("notes-x", FRAMES);
        notes.exposures[0] = Some(p.cells.len());
        p.cells.push(doodle(&mut rng).into());
        let at = rng.below(p.layers.len() + 1);
        p.layers.insert(at, notes);

        let path = dir.join(format!("s{seed}.kra"));
        let _ = std::fs::remove_file(&path);
        let mut app = App { p, links: HashMap::new(), base: Baseline::empty(W, H), path };
        app.send();
        let krita = Krita::open(&app.path);
        Session { seed, round: 0, rng, app, krita, d: Dirty::default(), stats: Stats::default(), log: Vec::new() }
    }

    // --- edits -------------------------------------------------------------

    fn local_edit(&mut self) {
        let rng = &mut self.rng;
        let p = &mut self.app.p;
        let fc = p.frame_count;
        let li = rng.below(p.layers.len());
        let uid = p.layers[li].uid;
        let op = rng.below(12);
        self.log.push(format!("r{} here  op{op} on {} ({:?})", self.round, p.layers[li].name, p.layers[li].exposures));
        match op {
            0..=2 => {
                let keys: Vec<usize> = p.layers[li].exposures.iter().flatten().copied().collect();
                if let Some(&id) = keys.get(rng.below(keys.len())) {
                    scribble(p.cell_mut(id).unwrap(), rng);
                    self.d.local_content.insert(uid);
                }
            }
            3 => {
                let l = &mut p.layers[li];
                let (f, g) = (rng.below(fc), rng.below(fc));
                if l.exposures[f].is_some() && l.exposures[g].is_none() {
                    l.exposures[g] = l.exposures[f].take();
                    self.d.local_content.insert(uid);
                }
            }
            4 => {
                let id = p.cells.len();
                p.cells.push(doodle(rng).into());
                p.layers[li].exposures[rng.below(fc)] = Some(id);
                self.d.local_content.insert(uid);
            }
            5 => {
                if !stays_put(&p.layers[li].name) {
                    p.layers[li].name = format!("R{}", rng.below(1000));
                    self.d.local_attr.insert(uid);
                }
            }
            6 => {
                p.layers[li].opacity = rng.below(256) as f32 / 255.0;
                p.layers[li].visible = rng.chance(80);
                self.d.local_attr.insert(uid);
            }
            7 => {
                let mut l = Layer::new(format!("new{}", rng.below(1000)), fc);
                l.exposures[rng.below(fc)] = Some(p.cells.len());
                p.cells.push(doodle(rng).into());
                p.layers.insert(rng.below(p.layers.len() + 1), l);
            }
            8 if rng.chance(30) => p.layers[li].reference ^= true,
            9 if rng.chance(30) && !stays_put(&p.layers[li].name) => p.layers[li].name.push_str("-x"),
            10 if p.layers.len() > 2 => {
                p.layers.remove(li);
            }
            11 if li + 1 < p.layers.len() => {
                p.layers.swap(li, li + 1);
                self.d.local_order = true;
            }
            _ => {}
        }
    }

    fn krita_edit(&mut self) {
        let rng = &mut self.rng;
        let k = &mut self.krita;
        if k.p.layers.is_empty() {
            return;
        }
        let fc = k.p.frame_count;
        let li = rng.below(k.p.layers.len());
        let u = k.uuids[li].clone();
        self.d.krita_unsaved = true;
        let op = rng.below(12);
        self.log.push(format!("r{} krita op{op} on {} ({:?})", self.round, k.p.layers[li].name, k.p.layers[li].exposures));
        match op {
            0..=2 => {
                let keys: Vec<usize> = k.p.layers[li].exposures.iter().flatten().copied().collect();
                if let Some(&id) = keys.get(rng.below(keys.len())) {
                    scribble(k.p.cell_mut(id).unwrap(), rng);
                    self.d.krita_content.insert(u);
                }
            }
            3 => {
                let l = &mut k.p.layers[li];
                let (f, g) = (rng.below(fc), rng.below(fc));
                if l.exposures[f].is_some() && l.exposures[g].is_none() {
                    l.exposures[g] = l.exposures[f].take();
                    self.d.krita_content.insert(u);
                }
            }
            4 => {
                // Sometimes past the end: the timeline grows.
                let g = if rng.chance(15) { fc + rng.below(3) } else { rng.below(fc) };
                k.p.ensure_frame_count(g + 1);
                let id = k.p.cells.len();
                k.p.cells.push(doodle(rng).into());
                k.p.layers[li].exposures[g] = Some(id);
                self.d.krita_content.insert(u);
            }
            5 | 6 => {
                let name = if rng.chance(50) { format!("sk{}-x", rng.below(1000)) } else { format!("K{}", rng.below(1000)) };
                let mut l = Layer::new(name, k.p.frame_count);
                l.exposures[rng.below(k.p.frame_count)] = Some(k.p.cells.len());
                k.p.cells.push(doodle(rng).into());
                let at = rng.below(k.p.layers.len() + 1);
                k.p.layers.insert(at, l);
                k.uuids.insert(at, kl::new_uuid());
            }
            7 if k.p.layers.len() > 1 => {
                k.p.layers.remove(li);
                k.uuids.remove(li);
            }
            8 if rng.chance(30) && !stays_put(&k.p.layers[li].name) => {
                k.p.layers[li].name.push_str("-x");
                self.d.krita_attr.insert(u);
            }
            9 if li + 1 < k.p.layers.len() => {
                k.p.layers.swap(li, li + 1);
                k.uuids.swap(li, li + 1);
            }
            10 => {
                k.p.layers[li].opacity = rng.below(256) as f32 / 255.0;
                k.p.layers[li].locked = rng.chance(20);
                self.d.krita_attr.insert(u);
            }
            11 => {
                if !stays_put(&k.p.layers[li].name) {
                    k.p.layers[li].name = format!("KR{}", rng.below(1000));
                    self.d.krita_attr.insert(u);
                }
            }
            _ => {}
        }
    }

    // --- pull ----------------------------------------------------------------

    fn save_and_pull(&mut self) {
        self.log.push(format!("r{} krita saves; pull", self.round));
        self.krita.save(&self.app.path);
        self.d.krita_unsaved = false;
        self.pull();
    }

    fn pull(&mut self) {
        let before = self.app.p.clone();
        let links_before = self.app.links.clone();
        let base_before: HashSet<String> = self.app.base.layers.iter().map(|l| l.uuid.clone()).collect();
        let ignored_before = self.app.base.ignored.clone();
        let plan = match self.app.try_plan() {
            Ok(plan) => plan,
            Err(e) => {
                // Refused only when the result would have no layers at all:
                // everything here linked, and nothing of it left in Krita.
                let text = e.to_string();
                check!(self, text.starts_with(kl::EVERYTHING_DELETED), "pull failed: {text}");
                let app = &self.app;
                let comes_back: Vec<&String> = self
                    .krita
                    .p
                    .layers
                    .iter()
                    .zip(&self.krita.uuids)
                    .filter(|(l, _)| kl::comes_back(&l.name, &[]))
                    .map(|(_, u)| u)
                    .filter(|u| !app.base.ignored.contains(*u))
                    .collect();
                let all_gone = app.p.layers.iter().all(|l| {
                    !kl::stays_here(l) && app.links.get(&l.uid).is_some_and(|u| !comes_back.contains(&u))
                });
                check!(self, all_gone, "refused a pull that leaves layers: {text}");
                let same = |a: &Project, b: &Project| {
                    a.layers.iter().map(|l| (l.uid, &l.exposures)).eq(b.layers.iter().map(|l| (l.uid, &l.exposures)))
                };
                check!(self, same(&app.p, &before), "refusal changed the project");
                self.stats.refused += 1;
                return;
            }
        };
        self.stats.pulls += 1;
        self.stats.noops += plan.is_noop() as usize;
        self.stats.unlinks += plan.unlink.len();
        let n_cells = self.app.p.cells.len();
        let unlink = plan.unlink.clone();
        let next = plan.next.clone();
        let added = plan.apply(&mut self.app.p);
        self.app.links.extend(added);
        self.app.links.retain(|uid, _| !unlink.contains(uid));
        self.app.base = next;
        self.stats.new_cells += self.app.p.cells.len() - n_cells;
        let (app, k) = (&self.app, &self.krita);
        let fc = app.p.frame_count;

        // The timeline is whole.
        for l in &app.p.layers {
            check!(self, l.exposures.len() == fc, "layer {} has {} of {fc} frames", l.name, l.exposures.len());
            check!(self, l.exposures.iter().flatten().all(|&id| id < app.p.cells.len()), "dangling cell in {}", l.name);
        }
        for (kl_, u) in k.p.layers.iter().zip(&k.uuids) {
            if let (Some(last), true) = (kl_.exposures.iter().rposition(Option::is_some), kl::comes_back(&kl_.name, &[])) {
                if app.linked(u).is_some() {
                    check!(self, last < fc, "Krita key at {last} past the end ({fc})");
                }
            }
        }

        // Undo only has to restore the timeline: no existing cell changed.
        for i in 0..n_cells {
            check!(self, app.p.cells[i].pixels() == before.cells[i].pixels(), "cell {i} mutated by the pull");
        }

        // Layers that stay here are untouched and still here.
        for l in before.layers.iter().filter(|l| kl::stays_here(l)) {
            let now = app.p.layers.iter().find(|x| x.uid == l.uid);
            check!(self, now.is_some(), "stay-here layer {} vanished", l.name);
            let now = now.unwrap();
            check!(self, now.exposures[..l.exposures.len()] == l.exposures[..], "stay-here {} retimed", l.name);
            check!(self, attrs(now) == attrs(l), "stay-here {} changed", l.name);
        }

        // Krita's `-x` layers never come here.
        let before_uids: HashSet<u64> = before.layers.iter().map(|l| l.uid).collect();
        for l in app.p.layers.iter().filter(|l| !before_uids.contains(&l.uid)) {
            check!(self, !stays_put(&l.name), "Krita-only {} came in", l.name);
        }

        for (ki, u) in k.uuids.iter().enumerate() {
            let kl_ = &k.p.layers[ki];
            if !kl::comes_back(&kl_.name, &[]) {
                check!(self, app.linked(u).is_none(), "{} is -x in Krita but still linked", kl_.name);
                continue;
            }
            let Some(li) = app.linked(u) else {
                // Only a layer deleted here, or made to stay here, may be missing.
                check!(
                    self,
                    base_before.contains(u) || ignored_before.contains(u) || app.base.ignored.contains(u),
                    "new Krita layer {} not added",
                    kl_.name
                );
                continue;
            };
            let l = &app.p.layers[li];
            // Krita wins on what only Krita changed: an untouched layer here
            // shows exactly what Krita shows.
            if !self.d.local_content.contains(&l.uid) {
                let (here, there) = (shows(&app.p, l, fc), shows(&k.p, kl_, fc));
                check!(
                    self,
                    here == there,
                    "{} doesn't show Krita's frames
 here  {:?}
 there {:?}
 here exposures {:?}
 there exposures {:?}",
                    l.name,
                    here,
                    there,
                    l.exposures,
                    kl_.exposures
                );
            }
            if !self.d.local_attr.contains(&l.uid) {
                check!(self, attrs(l) == attrs(kl_), "{} attrs {:?} vs Krita {:?}", l.name, attrs(l), attrs(kl_));
            }
            // Local edits survive where Krita didn't touch the layer.
            if !self.d.krita_content.contains(u) {
                if let Some(b) = before.layers.iter().find(|b| b.uid == l.uid) {
                    check!(self, l.exposures[..b.exposures.len()] == b.exposures[..], "{} retimed though Krita left it", l.name);
                }
            }
        }

        // Linked layers Krita deleted are gone here.
        for b in &before.layers {
            if let Some(u) = links_before.get(&b.uid) {
                if base_before.contains(u) && !kl::stays_here(b) && k.index_of(u).is_none() {
                    check!(self, !app.p.layers.iter().any(|l| l.uid == b.uid), "{} deleted in Krita but kept", b.name);
                }
            }
        }

        // Krita's order wins unless this side reordered.
        if !self.d.local_order {
            let here: Vec<&String> = app.p.layers.iter().filter_map(|l| app.links.get(&l.uid)).collect();
            let there: Vec<&String> = k.uuids.iter().filter(|u| here.contains(u)).collect();
            check!(self, here == there, "layer order differs from Krita's");
        }

        // The same file again changes nothing.
        let again = app.plan();
        check!(self, again.is_noop() && again.unlink.is_empty(), "second pull not quiet: {:?}", again.report);

        // Agreed again, except where edits here survived.
        self.d.krita_content.clear();
        self.d.krita_attr.clear();
        let (app, k) = (&self.app, &self.krita);
        // A reorder here that Krita didn't make stays a difference.
        let here: Vec<&String> = app.p.layers.iter().filter_map(|l| app.links.get(&l.uid)).collect();
        let there: Vec<&String> = k.uuids.iter().filter(|u| here.contains(u)).collect();
        self.d.local_order = here != there;
        let mut lc = HashSet::new();
        let mut la = HashSet::new();
        for l in &app.p.layers {
            let Some(ki) = app.links.get(&l.uid).and_then(|u| k.index_of(u)) else { continue };
            if shows(&app.p, l, fc) != shows(&k.p, &k.p.layers[ki], fc) {
                lc.insert(l.uid);
            }
            if attrs(l) != attrs(&k.p.layers[ki]) {
                la.insert(l.uid);
            }
        }
        self.d.local_content = lc;
        self.d.local_attr = la;
    }

    // --- send ------------------------------------------------------------------

    fn resend(&mut self) {
        if self.d.krita_unsaved {
            if self.rng.chance(70) {
                // The helper: Krita's edits are saved and come in first.
                self.save_and_pull();
            } else {
                // No helper: the file is closed in Krita without saving.
                self.d.krita_unsaved = false;
                self.d.krita_content.clear();
                self.d.krita_attr.clear();
            }
        }
        // What Krita had on disk: its `-x` layers, and the kept layer under each.
        let on_disk = Krita::open(&self.app.path);
        let held: Vec<(String, String, Vec<u64>, Option<String>)> = on_disk
            .p
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| stays_put(&l.name))
            .map(|(i, l)| {
                let under = (0..i).rev().find(|&j| !stays_put(&on_disk.p.layers[j].name)).map(|j| on_disk.uuids[j].clone());
                (on_disk.uuids[i].clone(), l.name.clone(), shows(&on_disk.p, l, on_disk.p.frame_count), under)
            })
            .collect();

        self.log.push(format!("r{} send again", self.round));
        let sent = self.app.send();
        self.stats.sends += 1;
        self.stats.carried += sent.kept.len();
        let k2 = Krita::open(&self.app.path);
        let app = &self.app;
        let fc = app.p.frame_count;

        // Krita holds exactly what's here, layer for layer…
        let sendable: Vec<&Layer> = app.p.layers.iter().filter(|l| !kl::stays_here(l)).collect();
        let theirs: Vec<usize> = (0..k2.p.layers.len()).filter(|&i| !stays_put(&k2.p.layers[i].name)).collect();
        check!(self, sendable.len() == theirs.len(), "sent {} layers, Krita has {}", sendable.len(), theirs.len());
        for (l, &i) in sendable.iter().zip(&theirs) {
            check!(self, app.links.get(&l.uid) == Some(&k2.uuids[i]), "{} sent out of order", l.name);
            check!(self, attrs(l) == attrs(&k2.p.layers[i]), "{} attrs lost in the send", l.name);
            check!(self, shows(&app.p, l, fc) == shows(&k2.p, &k2.p.layers[i], fc), "{} frames lost in the send", l.name);
        }
        // …plus its own `-x` layers, untouched and in place.
        let mut kept_names: Vec<&str> = sent.kept.iter().map(String::as_str).collect();
        kept_names.sort_unstable();
        let mut held_names: Vec<&str> = held.iter().map(|h| h.1.as_str()).collect();
        held_names.sort_unstable();
        check!(self, kept_names == held_names, "kept {kept_names:?}, Krita had {held_names:?}");
        for (u, name, frames, under) in &held {
            let i = k2.index_of(u);
            check!(self, i.is_some(), "Krita-only {name} dropped by the send");
            let i = i.unwrap();
            let fc2 = frames.len().max(k2.p.frame_count);
            let mut want = frames.clone();
            want.resize(fc2, *frames.last().unwrap_or(&0));
            check!(self, shows(&k2.p, &k2.p.layers[i], fc2) == want, "Krita-only {name} changed by the send");
            if let Some(under) = under.as_ref().filter(|u| k2.index_of(u).is_some()) {
                let now = (0..i).rev().find(|&j| !stays_put(&k2.p.layers[j].name)).map(|j| &k2.uuids[j]);
                check!(self, now == Some(under), "Krita-only {name} moved");
            }
        }

        // Nothing to pull back from our own send.
        let again = app.plan();
        check!(self, again.is_noop() && again.unlink.is_empty(), "pull after send not quiet: {:?}", again.report);

        self.krita = k2;
        self.d = Dirty::default();
    }

    fn run(&mut self, rounds: usize) {
        for round in 0..rounds {
            self.round = round;
            for _ in 0..self.rng.below(3) {
                self.local_edit();
            }
            for _ in 0..self.rng.below(3) {
                self.krita_edit();
            }
            if self.d.krita_unsaved && self.rng.chance(60) {
                self.save_and_pull();
            }
            if self.rng.chance(25) {
                self.resend();
            }
        }
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("animator-stress-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn panic_text(e: &Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

/// Seeded two-sided sessions. `STRESS_SEEDS` / `STRESS_ROUNDS` scale it up.
#[test]
fn random_sessions() {
    let seeds: u64 = std::env::var("STRESS_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(25);
    let rounds: usize = std::env::var("STRESS_ROUNDS").ok().and_then(|s| s.parse().ok()).unwrap_or(15);
    let dir = scratch("random");
    let t = Instant::now();
    let mut total = Stats::default();
    // `STRESS_SEED=n` replays one session.
    let only: Option<u64> = std::env::var("STRESS_SEED").ok().and_then(|s| s.parse().ok());
    for seed in only.map_or(0..seeds, |n| n..n + 1) {
        let mut s = std::panic::catch_unwind(|| Session::new(seed, &dir))
            .unwrap_or_else(|e| panic!("seed {seed} setup: {}", panic_text(&e)));
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.run(rounds))) {
            panic!("seed {seed} round {}: {}
--- what happened ---
{}", s.round, panic_text(&e), s.log.join("
"));
        }
        total.pulls += s.stats.pulls;
        total.noops += s.stats.noops;
        total.sends += s.stats.sends;
        total.new_cells += s.stats.new_cells;
        total.unlinks += s.stats.unlinks;
        total.carried += s.stats.carried;
        total.refused += s.stats.refused;
    }
    eprintln!("{seeds} sessions x {rounds} rounds in {:.2?}: {total:?}", t.elapsed());
    if only.is_some() {
        return;
    }
    // The mix actually exercised what it's meant to.
    assert!(total.pulls > seeds as usize && total.sends > seeds as usize);
    assert!(total.new_cells > 0 && total.unlinks > 0 && total.carried > 0, "{total:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

// --- scale -------------------------------------------------------------------

/// A drawing-like cell: a filled blob with a soft edge over `frac` of it.
fn blob(w: u32, h: u32, seed: u64, frac: f32) -> Canvas {
    let mut c = Canvas::new(w, h);
    let mut rng = Rng::new(seed);
    let (cx, cy) = (rng.below(w as usize) as f32, rng.below(h as usize) as f32);
    let r = (w.min(h) as f32) * frac;
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            if d < r {
                let a = (((r - d) / 4.0).min(1.0) * 255.0) as u8;
                let o = ((y * w + x) * 4) as usize;
                c.pixels_mut()[o..o + 4].copy_from_slice(&[(x % 251) as u8, (y % 241) as u8, seed as u8, a]);
            }
        }
    }
    c
}

/// Send, pull untouched, Krita edits one drawing, pull, re-send carrying a
/// full-size `-x` layer — each timed.
fn scale(tag: &str, w: u32, h: u32, frames: usize, drawings: &[usize]) {
    let dir = scratch(tag);
    let mut p = Project::new(w, h, 24.0);
    p.ensure_frame_count(frames);
    p.layers.clear();
    for (li, &n) in drawings.iter().enumerate() {
        let mut l = Layer::new(format!("L{li}"), frames);
        for k in 0..n {
            l.exposures[k * frames / n] = Some(p.cells.len());
            p.cells.push(blob(w, h, (li * 100 + k) as u64, 0.35).into());
        }
        p.layers.push(l);
    }
    let cells = p.cells.len();
    let mb = cells as f64 * (w * h * 4) as f64 / 1e6;
    let mut app = App { p, links: HashMap::new(), base: Baseline::empty(w, h), path: dir.join("big.kra") };

    let t = Instant::now();
    app.send();
    let send = t.elapsed();
    let size = std::fs::metadata(&app.path).unwrap().len() as f64 / 1e6;

    let t = Instant::now();
    let plan = app.plan();
    let pull_noop = t.elapsed();
    assert!(plan.is_noop(), "{:?}", plan.report);

    // Krita paints on one drawing, and adds a full-size sketch-x.
    let mut k = Krita::open(&app.path);
    let id = k.p.layers[0].exposures.iter().flatten().next().copied().unwrap();
    for px in k.p.cell_mut(id).unwrap().pixels_mut().chunks_exact_mut(4).take(5000) {
        px.copy_from_slice(&[255, 0, 0, 255]);
    }
    let mut sketch = Layer::new("sketch-x", k.p.frame_count);
    sketch.exposures[0] = Some(k.p.cells.len());
    k.p.cells.push(blob(w, h, 999, 0.45).into());
    k.p.layers.push(sketch);
    k.uuids.push(kl::new_uuid());
    k.save(&app.path);

    let t = Instant::now();
    let plan = app.plan();
    let pull_edit = t.elapsed();
    assert_eq!(plan.report.drawings, 1, "{:?}", plan.report);
    let next = plan.next.clone();
    plan.apply(&mut app.p);
    app.base = next;

    let t = Instant::now();
    let sent = app.send();
    let resend = t.elapsed();
    assert_eq!(sent.kept, ["sketch-x"]);
    let k2 = Krita::open(&app.path);
    assert!(k2.p.layers.iter().any(|l| l.name == "sketch-x"));

    eprintln!(
        "{tag}: {w}x{h}, {} layers, {frames} frames, {cells} drawings ({mb:.0} MB raw) -> .kra {size:.1} MB\n  \
         send {send:.2?} | pull untouched {pull_noop:.2?} | pull 1 changed {pull_edit:.2?} | re-send carrying a {w}x{h} -x layer {resend:.2?}",
        drawings.len()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The user's shot: 2160x3840, six layers, one with seven drawings.
#[test]
#[ignore]
fn scale_users_shot() {
    scale("shot", 2160, 3840, 21, &[1, 1, 1, 7, 1, 1]);
}

/// A long timeline at 720p: three layers of 48 drawings over 192 frames.
#[test]
#[ignore]
fn scale_long_timeline() {
    scale("long", 1280, 720, 192, &[48, 48, 48]);
}
