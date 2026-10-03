//! Two-way link with Krita.
//!
//! "Edit in Krita" writes the whole project as an animated `.kra` into a temp
//! folder, opens it in Krita, and watches the file. Each time Krita saves, the
//! file is read back on a worker and folded into the project as one undoable
//! edit — but only what Krita actually changed:
//!
//! * **Drawings** are matched by content hash against [`Baseline`], the record
//!   of what Krita held at the last sync. A drawing Krita didn't touch maps
//!   back to the local cell that stands for it, so edits made *here* in the
//!   meantime survive; one Krita changed becomes a new cell (Krita wins when
//!   both apps changed it). Existing cells are never mutated, which is what
//!   makes the cheap timeline-snapshot undo correct.
//! * **Timing** (a layer's keys) is only rebuilt when Krita's key list differs
//!   from the baseline; a retime done here on a layer Krita left alone stays.
//! * **Layer attributes** (name, opacity, visibility, lock) are applied field
//!   by field, only where Krita's value moved.
//! * **Layers** added in Krita are added; deleted in Krita are deleted;
//!   reordered in Krita are reordered. Layers added here since the send stay
//!   put above the linked layer they sat on.
//!
//! What Krita can't hold — layer transforms, camera, tracker points, the
//! reference flag — never travels, so it is never disturbed.
//!
//! The link lives until the app exits (or New/Open/Stop); nothing is stored
//! in the `.anim`.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{bail, Context, Result};

use crate::doc::canvas::Canvas;
use crate::doc::layer::{CellId, Layer};
use crate::doc::project::Project;
use crate::io::kra::{self, cell_origin, KraDoc};

/// Content hash of a cell: size + pixels. Both sides of the round trip hash
/// through here, so a drawing Krita didn't touch hashes the same coming back.
pub fn cell_hash(c: &Canvas) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    c.width.hash(&mut h);
    c.height.hash(&mut h);
    h.write(&c.pixels);
    h.finish()
}

/// How a pull that would leave no layers is refused.
pub const EVERYTHING_DELETED: &str = "Krita's save deletes every layer here";

/// A name that keeps a layer in the app it's in: it ends in "-x", ignoring
/// case and trailing spaces ("sketch-x", "BG -X "). The same rule both ways:
/// such a layer here is never sent, one in Krita is never brought back.
pub fn stays_put(name: &str) -> bool {
    let n = name.trim_end();
    n.len() > 2 && n.is_char_boundary(n.len() - 2) && n[n.len() - 2..].eq_ignore_ascii_case("-x")
}

/// A layer of ours that never goes to Krita: a reference (light-table) layer,
/// or one named to [`stays_put`].
pub fn stays_here(l: &Layer) -> bool {
    l.reference || stays_put(&l.name)
}

/// Whether a Krita layer comes back here: not named to [`stays_put`], and not
/// inside a group that is. The filter for [`kra::read_filtered`].
pub fn comes_back(name: &str, groups: &[String]) -> bool {
    !stays_put(name) && !groups.iter().any(|g| stays_put(g))
}

fn is_blank(c: &Canvas) -> bool {
    c.pixels.iter().all(|&b| b == 0)
}

/// A fresh `{8-4-4-4-12}` uuid in Krita's layer-id format. Random per process
/// (`RandomState` seeds from the OS) plus a counter, so two calls never meet.
pub fn new_uuid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let s = std::collections::hash_map::RandomState::new();
    let n = N.fetch_add(1, Ordering::Relaxed);
    let a = s.hash_one((n, SystemTime::now()));
    let b = s.hash_one((a, n));
    format!(
        "{{{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}}}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0x0fff,
        0x8000 | (b >> 48) & 0x3fff,
        b & 0xffff_ffff_ffff
    )
}

/// What Krita holds for one layer, as of the last sync.
#[derive(Clone)]
pub struct LayerBase {
    pub uuid: String,
    pub name: String,
    pub opacity: u8,
    pub visible: bool,
    pub locked: bool,
    /// `(time, content hash)` per key, leading blank keys dropped — Krita
    /// needs a blank key where we simply have nothing yet.
    pub keys: Vec<(usize, u64)>,
    /// Drawing size per key, parallel to `keys`.
    pub sizes: Vec<(u32, u32)>,
    /// Krita-side content hash → the local cell standing for it.
    pub by_hash: HashMap<u64, CellId>,
    /// Size for a drawing Krita adds with no neighbouring key to copy.
    pub cell_size: (u32, u32),
}

/// What Krita holds, as of the last sync: the send, or the last pull.
#[derive(Clone)]
pub struct Baseline {
    /// Frame size Krita's canvas was made from.
    pub pw: u32,
    pub ph: u32,
    pub range_end: usize,
    /// Bottom first, in Krita's stack order.
    pub layers: Vec<LayerBase>,
    /// Krita layers whose link ended from this side (the layer here became
    /// reference or `-x`): never brought back, until the next send rewrites
    /// Krita's file without them.
    pub ignored: HashSet<String>,
}

impl Baseline {
    /// Nothing sent yet (the stress harness starts from here).
    #[cfg(test)]
    pub fn empty(pw: u32, ph: u32) -> Self {
        Self { pw, ph, range_end: 0, layers: Vec::new(), ignored: HashSet::new() }
    }

    /// Record `project` as sent, `uuids[i]` naming layer `i`.
    pub fn capture(project: &Project, uuids: &[String]) -> Self {
        let (pw, ph) = (project.width, project.height);
        let mut hashes: HashMap<CellId, u64> = HashMap::new();
        let layers = project
            .layers
            .iter()
            .zip(uuids)
            .map(|(l, uuid)| {
                let mut b = LayerBase {
                    uuid: uuid.clone(),
                    name: l.name.clone(),
                    opacity: kra::opacity_u8(l.opacity),
                    visible: l.visible,
                    locked: l.locked,
                    keys: Vec::new(),
                    sizes: Vec::new(),
                    by_hash: HashMap::new(),
                    cell_size: l.cell_size(pw, ph),
                };
                for (f, e) in l.exposures.iter().enumerate().take(project.frame_count) {
                    let Some(id) = *e else { continue };
                    let Some(c) = project.cell(id) else { continue };
                    if b.keys.is_empty() && is_blank(c) {
                        continue;
                    }
                    let h = *hashes.entry(id).or_insert_with(|| cell_hash(c));
                    b.keys.push((f, h));
                    b.sizes.push((c.width, c.height));
                    b.by_hash.entry(h).or_insert(id);
                }
                b
            })
            .collect();
        Self { pw, ph, range_end: project.frame_count.saturating_sub(1), layers, ignored: HashSet::new() }
    }

    fn layer(&self, uuid: &str) -> Option<&LayerBase> {
        self.layers.iter().find(|l| l.uuid == uuid)
    }
}

/// One Krita drawing, rasterized into cell space and matched on the worker.
pub struct PulledDrawing {
    pub hash: u64,
    pub size: (u32, u32),
    /// The local cell whose Krita-side content this is. `None` = Krita
    /// changed or added it.
    pub matched: Option<CellId>,
    /// The pixels, kept only when unmatched.
    pub canvas: Option<Canvas>,
    pub blank: bool,
}

pub struct PulledLayer {
    pub uuid: String,
    pub name: String,
    pub opacity: u8,
    pub visible: bool,
    pub locked: bool,
    pub drawings: Vec<PulledDrawing>,
    /// `(time, drawing)`, leading blank keys dropped.
    pub keys: Vec<(usize, usize)>,
}

impl PulledLayer {
    fn key_hashes(&self) -> Vec<(usize, u64)> {
        self.keys.iter().map(|&(t, d)| (t, self.drawings[d].hash)).collect()
    }
}

/// A Krita save, decoded and matched against the baseline — the heavy part of
/// a pull, done off the UI thread.
pub struct Pulled {
    pub layers: Vec<PulledLayer>,
    pub range_end: Option<usize>,
    pub warnings: Vec<String>,
    /// `(uuid, name)` of Krita layers that stay in Krita (see [`comes_back`]).
    pub held: Vec<(String, String)>,
}

/// The Krita uuids of the layers here that are linked — see [`prepare`].
pub fn present(project: &Project, links: &HashMap<u64, String>) -> HashSet<String> {
    project.layers.iter().filter_map(|l| links.get(&l.uid)).cloned().collect()
}

/// Rasterize and match every drawing in `doc` (worker side). `present` are
/// the uuids linked to a layer here ([`present`]): a Krita layer without one
/// comes back whole, as Krita has it, so its pixels are kept even where they
/// match — the matching cell here may have been edited since, or deleted.
pub fn prepare(doc: KraDoc, base: &Baseline, present: &HashSet<String>) -> Pulled {
    let (pw, ph) = (base.pw, base.ph);
    let mut warnings = doc.warnings;
    if (doc.width, doc.height) != (pw, ph) {
        warnings.push(format!(
            "Krita canvas is {}×{} (sent {pw}×{ph}); drawings kept centered",
            doc.width, doc.height
        ));
    }
    // Krita canvas → our frame: canvases are aligned by their centers.
    let (dx, dy) = cell_origin(pw, ph, doc.width, doc.height);
    let mut clipped = 0usize;

    let layers = doc
        .layers
        .into_iter()
        .map(|kl| {
            let b = base.layer(&kl.uuid);
            let whole = !present.contains(&kl.uuid);
            // Every size this layer's drawings had at the last sync; almost
            // always just one.
            let mut sizes: Vec<(u32, u32)> = Vec::new();
            if let Some(b) = b {
                for &s in b.sizes.iter().chain(std::iter::once(&b.cell_size)) {
                    if !sizes.contains(&s) {
                        sizes.push(s);
                    }
                }
            } else {
                sizes.push((pw, ph));
            }

            let drawings = kl
                .drawings
                .iter()
                .enumerate()
                .map(|(di, d)| {
                    let place = |(w, h): (u32, u32)| {
                        let (ox, oy) = cell_origin(pw, ph, w, h);
                        d.place(ox - dx, oy - dy, w, h)
                    };
                    let mut tried: Vec<((u32, u32), Canvas, bool, u64)> = Vec::new();
                    for &s in &sizes {
                        let (c, clip) = place(s);
                        let h = cell_hash(&c);
                        if let Some(&id) = b.and_then(|b| b.by_hash.get(&h)) {
                            let blank = is_blank(&c);
                            return PulledDrawing { hash: h, size: s, matched: Some(id), blank, canvas: whole.then_some(c) };
                        }
                        tried.push((s, c, clip, h));
                    }
                    // New content: take the size of the key it replaces.
                    let first_use = kl.keys.iter().find(|k| k.1 == di).map_or(0, |k| k.0);
                    let want = b
                        .and_then(|b| {
                            let i = b.keys.iter().rposition(|k| k.0 <= first_use).unwrap_or(0);
                            b.sizes.get(i).copied()
                        })
                        .unwrap_or(sizes[0]);
                    let (s, c, clip, h) = tried
                        .into_iter()
                        .find(|t| t.0 == want)
                        .unwrap_or_else(|| {
                            let (c, clip) = place(want);
                            let h = cell_hash(&c);
                            (want, c, clip, h)
                        });
                    clipped += clip as usize;
                    PulledDrawing { hash: h, size: s, matched: None, blank: is_blank(&c), canvas: Some(c) }
                })
                .collect::<Vec<_>>();

            let lead = kl.keys.iter().take_while(|k| drawings[k.1].blank).count();
            PulledLayer {
                uuid: if kl.uuid.is_empty() { new_uuid() } else { kl.uuid },
                name: kl.name,
                opacity: kl.opacity,
                visible: kl.visible,
                locked: kl.locked,
                keys: kl.keys[lead..].to_vec(),
                drawings,
            }
        })
        .collect();

    if clipped > 0 {
        warnings.push(format!(
            "{clipped} drawing{} had paint outside the layer's drawable area (clipped)",
            if clipped == 1 { "" } else { "s" }
        ));
    }
    Pulled { layers, range_end: doc.range_end, warnings, held: doc.held }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellRef {
    Old(CellId),
    New(usize),
}

/// Field-by-field changes to one layer; `None` = leave alone.
#[derive(Default)]
struct LayerEdit {
    name: Option<String>,
    opacity: Option<u8>,
    visible: Option<bool>,
    locked: Option<bool>,
    keys: Option<Vec<(usize, CellRef)>>,
}

impl LayerEdit {
    fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.opacity.is_none()
            && self.visible.is_none()
            && self.locked.is_none()
            && self.keys.is_none()
    }

    fn apply(self, l: &mut Layer, fc: usize, resolve: impl Fn(CellRef) -> CellId) {
        if let Some(n) = self.name {
            l.name = n;
        }
        if let Some(o) = self.opacity {
            l.opacity = o as f32 / 255.0;
        }
        if let Some(v) = self.visible {
            l.visible = v;
        }
        if let Some(v) = self.locked {
            l.locked = v;
        }
        if let Some(keys) = self.keys {
            l.exposures = vec![None; fc];
            for (t, r) in keys {
                if t < fc {
                    l.exposures[t] = Some(resolve(r));
                }
            }
        }
    }
}

enum Entry {
    /// An existing layer, possibly edited.
    Local { index: usize, edit: Option<LayerEdit> },
    /// A layer Krita added (or one deleted here that Krita then changed).
    New { uuid: String, edit: LayerEdit },
}

/// What a pull did, for the toast.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Report {
    pub drawings: usize,
    pub retimed: usize,
    pub added: usize,
    pub removed: usize,
    pub renamed_or_attrs: usize,
    pub reordered: bool,
    pub grew_to: Option<usize>,
}

impl Report {
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let n = |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
        if self.drawings > 0 {
            parts.push(n(self.drawings, "drawing", "drawings"));
        }
        if self.retimed > 0 {
            parts.push(format!("timing on {}", n(self.retimed, "layer", "layers")));
        }
        if self.added > 0 {
            parts.push(format!("{} added", n(self.added, "layer", "layers")));
        }
        if self.removed > 0 {
            parts.push(format!("{} removed", n(self.removed, "layer", "layers")));
        }
        if self.renamed_or_attrs > 0 {
            parts.push(format!("settings on {}", n(self.renamed_or_attrs, "layer", "layers")));
        }
        if self.reordered {
            parts.push("layer order".into());
        }
        if let Some(fc) = self.grew_to {
            parts.push(format!("{fc} frames"));
        }
        parts.join(", ")
    }
}

/// A pull, planned against the current project and ready to apply.
pub struct PullPlan {
    new_cells: Vec<Canvas>,
    /// `project.cells.len()` when planned; new cells get ids from here.
    cell_base: usize,
    stack: Vec<Entry>,
    frame_count: usize,
    noop: bool,
    /// The baseline once this plan is applied (or skipped as a no-op).
    pub next: Baseline,
    pub report: Report,
    pub warnings: Vec<String>,
    /// [`Layer::uid`]s whose link ends with this pull: renamed to stay in
    /// Krita there, or made reference / `-x` here. Their layers stay as they
    /// are; only the link goes. Applies even to a no-op plan.
    pub unlink: Vec<u64>,
}

impl PullPlan {
    pub fn is_noop(&self) -> bool {
        self.noop
    }

    /// Apply to the project the plan was made against, in the same frame —
    /// new cell ids were assigned from its cell count. Returns the
    /// `(uid, uuid)` link of every layer it created.
    pub fn apply(self, project: &mut Project) -> Vec<(u64, String)> {
        assert_eq!(project.cells.len(), self.cell_base, "project changed between plan and apply");
        project.ensure_frame_count(self.frame_count);
        let fc = project.frame_count;
        let base = self.cell_base;
        project.cells.extend(self.new_cells.into_iter().map(std::sync::Arc::new));
        let resolve = |r: CellRef| match r {
            CellRef::Old(id) => id,
            CellRef::New(i) => base + i,
        };

        let mut old: Vec<Option<Layer>> = std::mem::take(&mut project.layers).into_iter().map(Some).collect();
        let mut moved: Vec<Option<usize>> = vec![None; old.len()];
        let mut layers = Vec::with_capacity(self.stack.len());
        let mut linked = Vec::new();
        for e in self.stack {
            match e {
                Entry::Local { index, edit } => {
                    let Some(mut l) = old[index].take() else { continue };
                    if let Some(edit) = edit {
                        edit.apply(&mut l, fc, resolve);
                    }
                    moved[index] = Some(layers.len());
                    layers.push(l);
                }
                Entry::New { uuid, edit } => {
                    let mut l = Layer::new(edit.name.clone().unwrap_or_default(), fc);
                    linked.push((l.uid, uuid));
                    edit.apply(&mut l, fc, resolve);
                    layers.push(l);
                }
            }
        }
        for l in &mut layers {
            l.lines_from = l.lines_from.and_then(|i| moved.get(i).copied().flatten());
        }
        let cur = moved.get(project.current_layer).copied().flatten();
        project.layers = layers;
        project.current_layer = cur.unwrap_or(0).min(project.layers.len().saturating_sub(1));
        linked
    }
}

/// Plan folding `pulled` into `project`. `links` maps [`Layer::uid`] to the
/// Krita uuid it was sent as. Pure: nothing changes until [`PullPlan::apply`].
pub fn plan(project: &Project, base: &Baseline, links: &HashMap<u64, String>, pulled: Pulled) -> Result<PullPlan> {
    let (pw, ph) = (project.width, project.height);
    let base_uuids: HashSet<&str> = base.layers.iter().map(|l| l.uuid.as_str()).collect();
    let krita_uuids: HashSet<String> = pulled.layers.iter().map(|l| l.uuid.clone()).collect();

    let uuid_of = |l: &Layer| links.get(&l.uid);

    // Links that end here, in both directions: a Krita layer renamed to stay
    // in Krita, and a layer here made reference / `-x` since the send. Neither
    // side is touched — the Krita layer isn't added back, ours isn't removed.
    let mut ignore: HashSet<String> = pulled.held.iter().map(|(u, _)| u.clone()).collect();
    ignore.extend(base.ignored.iter().cloned());
    let mut ignored = base.ignored.clone();
    let mut unlink: Vec<u64> = Vec::new();
    let mut warnings = pulled.warnings;
    for l in &project.layers {
        let Some(u) = uuid_of(l) else { continue };
        let linked = base_uuids.contains(u.as_str());
        if stays_here(l) {
            ignore.insert(u.clone());
            ignored.insert(u.clone());
            unlink.push(l.uid);
            if linked {
                warnings.push(format!("\"{}\" stays here now, no longer synced", l.name));
            }
        } else if let Some((_, kname)) = pulled.held.iter().find(|(h, _)| h == u) {
            unlink.push(l.uid);
            if linked {
                warnings.push(format!("\"{kname}\" now stays in Krita; \"{}\" kept here as it was", l.name));
            }
        }
    }

    // The local layer standing for each linked uuid — the first one carrying
    // it, should a layer ever have been copied along with its uid.
    let mut local_of: HashMap<String, usize> = HashMap::new();
    for (i, l) in project.layers.iter().enumerate() {
        if let Some(u) = uuid_of(l) {
            if base_uuids.contains(u.as_str()) && !ignore.contains(u) {
                local_of.entry(u.clone()).or_insert(i);
            }
        }
    }

    let mut report = Report::default();
    let mut new_cells: Vec<Canvas> = Vec::new();
    let cell_base = project.cells.len();
    let mut frame_count = project.frame_count;
    if let Some(re) = pulled.range_end {
        if re != base.range_end {
            frame_count = frame_count.max(re + 1);
        }
    }

    // Decide every Krita layer, in Krita order (bottom first).
    enum Slot {
        Keep(usize, LayerEdit),
        Add(LayerEdit),
        /// Deleted here, untouched in Krita: stays deleted.
        Skip,
    }
    let mut decided: Vec<(String, Slot)> = Vec::new();
    let mut next_layers: Vec<LayerBase> = Vec::new();
    for mut pl in pulled.layers {
        if ignore.contains(&pl.uuid) {
            continue;
        }
        let b = base.layer(&pl.uuid);
        let key_hashes = pl.key_hashes();
        let keys_changed = b.map_or(true, |b| b.keys != key_hashes);

        let mut edit = LayerEdit::default();
        if b.map_or(true, |b| b.name != pl.name) {
            edit.name = Some(pl.name.clone());
        }
        if b.map_or(true, |b| b.opacity != pl.opacity) {
            edit.opacity = Some(pl.opacity);
        }
        if b.map_or(true, |b| b.visible != pl.visible) {
            edit.visible = Some(pl.visible);
        }
        if b.map_or(true, |b| b.locked != pl.locked) {
            edit.locked = Some(pl.locked);
        }
        let attrs_changed = !edit.is_empty();

        let local = local_of.get(&pl.uuid).copied();
        let keep = b.is_some() && local.is_some();
        let add = !keep && (b.is_none() || keys_changed || attrs_changed);
        let rebuild = (keep && keys_changed) || add;

        // Resolve drawings into cells only for a layer whose keys are rebuilt.
        let mut refs: Vec<Option<CellRef>> = vec![None; pl.drawings.len()];
        if rebuild {
            let mut keys = Vec::with_capacity(pl.keys.len());
            for &(t, di) in &pl.keys {
                let r = match refs[di] {
                    Some(r) => r,
                    None => {
                        let d = &mut pl.drawings[di];
                        let r = match d.matched {
                            // A layer that's here keeps its cells: edits made
                            // here survive where Krita didn't change the drawing.
                            Some(id) if keep => CellRef::Old(id),
                            // A layer coming back (new, or deleted here) is
                            // Krita's exactly: a cell is reused only while it
                            // still holds that very content.
                            Some(id) if project.cell(id).is_some_and(|c| cell_hash(c) == d.hash) => CellRef::Old(id),
                            matched => match d.canvas.take() {
                                Some(c) => {
                                    new_cells.push(c);
                                    CellRef::New(new_cells.len() - 1)
                                }
                                // Pixels weren't kept (`present` said the layer
                                // was here): the old cell is the best there is.
                                None => CellRef::Old(matched.context("unmatched drawing without pixels")?),
                            },
                        };
                        refs[di] = Some(r);
                        r
                    }
                };
                keys.push((t, r));
                frame_count = frame_count.max(t + 1);
            }
            if keep {
                // What changed, for the toast: the times, and the drawings
                // Krita holds now that it didn't before.
                let old = b.map(|b| &b.keys);
                let old_times: Vec<usize> = old.map(|k| k.iter().map(|k| k.0).collect()).unwrap_or_default();
                if old_times != pl.keys.iter().map(|k| k.0).collect::<Vec<_>>() {
                    report.retimed += 1;
                }
                let old: HashSet<u64> = old.map(|k| k.iter().map(|k| k.1).collect()).unwrap_or_default();
                let mut seen = HashSet::new();
                report.drawings += key_hashes
                    .iter()
                    .filter(|(_, h)| !old.contains(h) && seen.insert(*h))
                    .count();
            }
            edit.keys = Some(keys);
        }

        // The baseline for this layer after the pull: Krita's state.
        let lb = match (b, rebuild) {
            (Some(b), false) => LayerBase {
                name: pl.name.clone(),
                opacity: pl.opacity,
                visible: pl.visible,
                locked: pl.locked,
                ..b.clone()
            },
            (b, _) => {
                // Old entries stay: if Krita later undoes back to an earlier
                // drawing, it maps onto the cell that already holds it.
                let mut by_hash = b.map(|b| b.by_hash.clone()).unwrap_or_default();
                for (di, r) in refs.iter().enumerate() {
                    let id = match r {
                        Some(CellRef::Old(id)) => *id,
                        Some(CellRef::New(i)) => cell_base + i,
                        None => continue,
                    };
                    by_hash.insert(pl.drawings[di].hash, id);
                }
                LayerBase {
                    uuid: pl.uuid.clone(),
                    name: pl.name.clone(),
                    opacity: pl.opacity,
                    visible: pl.visible,
                    locked: pl.locked,
                    keys: key_hashes,
                    sizes: pl.keys.iter().map(|k| pl.drawings[k.1].size).collect(),
                    by_hash,
                    cell_size: match local {
                        Some(i) => project.layers[i].cell_size(pw, ph),
                        None => (pw, ph),
                    },
                }
            }
        };
        next_layers.push(lb);

        let slot = match local {
            Some(i) if keep => {
                if attrs_changed {
                    report.renamed_or_attrs += 1;
                }
                Slot::Keep(i, edit)
            }
            _ if add => {
                // A (re-)added layer needs every attribute, not only the
                // changed ones.
                edit.name = Some(pl.name.clone());
                edit.opacity = Some(pl.opacity);
                edit.visible = Some(pl.visible);
                edit.locked = Some(pl.locked);
                report.added += 1;
                Slot::Add(edit)
            }
            _ => Slot::Skip,
        };
        decided.push((pl.uuid, slot));
    }

    // Linked layers Krita deleted. (`local_of` already leaves out the ones
    // whose link just ended.)
    let removed: HashSet<usize> = local_of
        .iter()
        .filter(|(u, _)| !krita_uuids.contains(*u))
        .map(|(_, &i)| i)
        .collect();
    report.removed = removed.len();

    // Order of the linked layers. Krita's, if Krita reordered them; else the
    // local order (keeping any reorder done here), with Krita's additions
    // slotted in above their Krita neighbour.
    let kept_local: HashMap<String, usize> = decided
        .iter()
        .filter_map(|(u, s)| match s {
            Slot::Keep(i, _) => Some((u.clone(), *i)),
            _ => None,
        })
        .collect();
    let base_order: Vec<&str> = base
        .layers
        .iter()
        .map(|l| l.uuid.as_str())
        .filter(|u| kept_local.contains_key(*u))
        .collect();
    let krita_order: Vec<&str> = decided
        .iter()
        .map(|(u, _)| u.as_str())
        .filter(|u| kept_local.contains_key(*u))
        .collect();
    report.reordered = base_order != krita_order;

    let linked: Vec<String> = if report.reordered {
        decided
            .iter()
            .filter(|(_, s)| !matches!(s, Slot::Skip))
            .map(|(u, _)| u.clone())
            .collect()
    } else {
        let mut v: Vec<String> = project
            .layers
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                let u = uuid_of(l)?;
                (kept_local.get(u) == Some(&i)).then(|| u.clone())
            })
            .collect();
        for (pos, (u, s)) in decided.iter().enumerate() {
            if !matches!(s, Slot::Add(_)) {
                continue;
            }
            let at = decided[..pos]
                .iter()
                .rev()
                .find_map(|(p, _)| v.iter().position(|x| x == p))
                .map_or(0, |i| i + 1);
            v.insert(at, u.clone());
        }
        v
    };

    // Local-only layers (added here since the send), anchored to the kept
    // linked layer beneath them.
    let mut anchored: HashMap<Option<String>, Vec<usize>> = HashMap::new();
    let mut last: Option<String> = None;
    for (i, l) in project.layers.iter().enumerate() {
        match uuid_of(l) {
            Some(u) if kept_local.get(u) == Some(&i) => last = Some(u.clone()),
            _ if removed.contains(&i) => {}
            _ => anchored.entry(last.clone()).or_default().push(i),
        }
    }

    let mut slots: HashMap<String, Slot> = decided.into_iter().collect();
    let mut stack: Vec<Entry> = Vec::new();
    let mut changed = report.added > 0 || report.removed > 0 || report.reordered;
    let push_local = |stack: &mut Vec<Entry>, list: Option<&Vec<usize>>| {
        for &index in list.into_iter().flatten() {
            stack.push(Entry::Local { index, edit: None });
        }
    };
    push_local(&mut stack, anchored.get(&None));
    for u in linked {
        match slots.remove(&u) {
            Some(Slot::Keep(index, edit)) => {
                changed |= !edit.is_empty();
                stack.push(Entry::Local { index, edit: (!edit.is_empty()).then_some(edit) });
            }
            Some(Slot::Add(edit)) => stack.push(Entry::New { uuid: u.clone(), edit }),
            _ => {}
        }
        push_local(&mut stack, anchored.get(&Some(u)));
    }
    if stack.is_empty() {
        // Krita deleted every layer here and brings none back. A project
        // needs a layer, and wiping the whole thing from one save is the
        // wrong default — refuse, and say how to go on.
        bail!(
            "{EVERYTHING_DELETED}: nothing was changed here. Send to Krita again to put them              back there, or delete them here yourself."
        );
    }

    if frame_count > project.frame_count {
        report.grew_to = Some(frame_count);
        changed = true;
    }
    let next = Baseline {
        pw: base.pw,
        ph: base.ph,
        range_end: pulled.range_end.unwrap_or(base.range_end),
        layers: next_layers,
        ignored,
    };
    Ok(PullPlan {
        new_cells,
        cell_base,
        stack,
        frame_count,
        noop: !changed,
        next,
        report,
        warnings,
        unlink,
    })
}

// ---------------------------------------------------------------------------
// Session: the temp file, Krita's process, and the save watcher.

/// Minimum quiet time after the file changes before it is read, so a save
/// still being written isn't read half-done.
const SETTLE: Duration = Duration::from_millis(500);
/// How often the file is stat'ed.
const POLL: Duration = Duration::from_secs(1);
/// How long the helper plugin gets to say it's ready (it ticks every 400 ms,
/// and may have a save to make first).
const READY_TIMEOUT: Duration = Duration::from_secs(4);
/// How long it gets to reload — reopening a big animated document takes a
/// while.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Modification time and length: enough to notice a save.
pub type Stamp = (SystemTime, u64);

pub fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// One "Send to Krita again": get Krita's latest onto disk and in here,
/// write the new file, and — with the helper plugin — have Krita reload it.
pub struct Round {
    /// Mailbox sequence number; 0 when the helper isn't in play.
    pub seq: u64,
    pub stage: Stage,
    /// The layer Krita should open on.
    pub selected: usize,
    /// Krita to start if it has to be (no helper answered, Krita closed).
    pub launch: Option<PathBuf>,
    /// The helper said it's ready, so it will reload the file itself.
    pub helper_ready: bool,
    /// The helper is installed but didn't answer — worth a hint.
    pub helper_silent: bool,
    /// What the send toast would have said (layers kept out here, Krita-only
    /// layers kept there), for the toast once Krita confirms the reload.
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Asked the helper to get ready (saving Krita's edits if it has any).
    AwaitReady { until: Instant },
    /// Bring in any Krita save not pulled yet, then send.
    Sync,
    /// The send worker is writing the file.
    Sending,
    /// Told the helper the file is there; waiting for it to reload.
    AwaitReloaded { until: Instant },
}

pub struct KritaLink {
    pub path: PathBuf,
    /// [`Layer::uid`] → the Krita uuid it travels as.
    pub uuids: HashMap<u64, String>,
    /// `None` until the send worker reports back.
    pub baseline: Option<Baseline>,
    /// The file as last read (or written by us). A different stamp is a save.
    last_seen: Option<Stamp>,
    /// A change noticed, and when; read once it has held still for `SETTLE`.
    changed: Option<(Instant, Stamp)>,
    last_poll: Option<Instant>,
    /// Krita as we launched it. Still running = don't launch again.
    child: Option<Child>,
    /// Pull in flight: the stamp being read, and the worker's answer.
    pub pending: Option<(Stamp, Receiver<Result<Pulled>>)>,
    /// How often the file is stat'ed, and how long a change must hold still
    /// before it is read. Fields so tests can run them at zero.
    pub poll_every: Duration,
    pub settle: Duration,
    /// The re-send in progress, if any.
    pub round: Option<Round>,
    /// Last mailbox sequence number used.
    pub helper_seq: u64,
    /// Whether to go through the helper plugin. `None` = if it's installed;
    /// tests pin it.
    pub use_helper: Option<bool>,
    /// How long to wait for the helper to answer, and to reload.
    pub ready_timeout: Duration,
    pub reload_timeout: Duration,
}

impl KritaLink {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            uuids: HashMap::new(),
            baseline: None,
            last_seen: None,
            changed: None,
            last_poll: None,
            child: None,
            pending: None,
            poll_every: POLL,
            settle: SETTLE,
            round: None,
            helper_seq: 0,
            use_helper: None,
            ready_timeout: READY_TIMEOUT,
            reload_timeout: RELOAD_TIMEOUT,
        }
    }

    /// The file's stamp if it changed since we last read or wrote it — a
    /// Krita save not pulled yet.
    pub fn unseen_change(&self) -> Option<Stamp> {
        stamp(&self.path).filter(|s| Some(*s) != self.last_seen)
    }

    /// Read the file on a worker, now, against the current baseline. `false`
    /// when there's no baseline to match against (a send in flight).
    pub fn start_pull(&mut self, at: Stamp, present: HashSet<String>) -> bool {
        let Some(base) = self.baseline.clone() else {
            return false;
        };
        let path = self.path.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(pull(&path, &base, &present));
        });
        self.pending = Some((at, rx));
        true
    }

    /// The temp file for this app instance.
    pub fn temp_path(project_path: Option<&Path>) -> PathBuf {
        let stem = project_path
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into());
        std::env::temp_dir()
            .join("animator-krita")
            .join(format!("{stem}-{}.kra", std::process::id()))
    }

    /// The file now on disk is ours (just written or just pulled).
    pub fn mark_seen(&mut self, s: Option<Stamp>) {
        self.last_seen = s;
        self.changed = None;
    }

    /// Called every frame. Returns the stamp to read when a Krita save has
    /// settled and no read is already running.
    pub fn poll(&mut self, now: Instant) -> Option<Stamp> {
        if self.pending.is_some() || self.baseline.is_none() {
            return None;
        }
        if self.last_poll.is_some_and(|t| now.duration_since(t) < self.poll_every) {
            return None;
        }
        self.last_poll = Some(now);
        let s = stamp(&self.path)?;
        if Some(s) == self.last_seen {
            self.changed = None;
            return None;
        }
        match self.changed {
            Some((t, c)) if c == s && now.duration_since(t) >= self.settle => Some(s),
            Some((_, c)) if c == s => None,
            _ => {
                self.changed = Some((now, s));
                None
            }
        }
    }

    /// Whether the Krita we launched is still open.
    pub fn krita_running(&mut self) -> bool {
        self.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    pub fn launch(&mut self, krita: &Path) -> Result<()> {
        let child = Command::new(krita)
            .arg(&self.path)
            .spawn()
            .with_context(|| format!("starting {}", krita.display()))?;
        self.child = Some(child);
        Ok(())
    }
}

/// Where Krita is: the saved preference, then the usual install locations,
/// then `PATH`. `None` = ask the user.
pub fn find_krita(pref: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = pref.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    let mut guesses: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(var) {
            let root = PathBuf::from(root);
            guesses.push(root.join("Krita (x64)").join("bin").join("krita.exe"));
            guesses.push(root.join("Programs").join("Krita (x64)").join("bin").join("krita.exe"));
        }
    }
    #[cfg(target_os = "macos")]
    guesses.push(PathBuf::from("/Applications/krita.app/Contents/MacOS/krita"));
    if let Some(found) = guesses.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    let exe = if cfg!(windows) { "krita.exe" } else { "krita" };
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).map(|d| d.join(exe)).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .find(|p| p.is_file())
}

/// What a send wrote.
pub struct Sent {
    pub baseline: Baseline,
    pub stamp: Option<Stamp>,
    /// Krita-only (`-x`) layers carried over from the file it replaced.
    pub kept: Vec<String>,
}

/// Worker side of a send: write the `.kra` and record the baseline. Only the
/// layers `keep` marks go; `uuids` and `selected` count those layers alone.
/// Takes the worker's own copy, so leaving layers out clones no cells.
///
/// Krita's own `-x` layers in the file being replaced are carried into the
/// new one untouched, in their place in the stack — the rewrite updates what
/// came from here and nothing else.
///
/// Krita has no clipping to match ours, so clipped layers go cut to their base
/// (see [`crate::doc::clip::bake_clipping`]). The baseline records the cut
/// pixels, which is what Krita holds, but each one stands for the whole
/// clipped drawing here: a round trip that leaves them alone changes nothing,
/// and the layer stays clipped.
pub fn send(
    mut project: Project,
    keep: &[bool],
    uuids: &[String],
    selected: Option<usize>,
    path: &Path,
) -> Result<Sent> {
    let origin = crate::doc::clip::bake_clipping(&mut project);
    let mut i = 0;
    project.layers.retain(|_| {
        i += 1;
        keep.get(i - 1).copied().unwrap_or(false)
    });
    let project = &project;
    let carried = match std::fs::read(path) {
        Ok(bytes) => kra::carry(bytes, &comes_back).unwrap_or_else(|e| {
            // Better a send that loses Krita-only layers than no send at all.
            log::warn!("Couldn't read Krita-only layers from {}: {e:#}", path.display());
            Vec::new()
        }),
        Err(_) => Vec::new(),
    };
    let opts = kra::WriteOpts {
        layer_uuids: uuids,
        selected_layer: selected.unwrap_or(usize::MAX),
        carried: &carried,
    };
    let bytes = kra::write(project, &opts)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    let mut baseline = Baseline::capture(project, uuids);
    for id in baseline.layers.iter_mut().flat_map(|l| l.by_hash.values_mut()) {
        if let Some(&clipped) = origin.get(id) {
            *id = clipped;
        }
    }
    Ok(Sent {
        baseline,
        stamp: stamp(path),
        kept: carried.into_iter().map(|c| c.name).collect(),
    })
}

/// Worker side of a pull: read, decode and match a Krita save.
pub fn pull(path: &Path, base: &Baseline, present: &HashSet<String>) -> Result<Pulled> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(prepare(kra::read_filtered(bytes, &comes_back)?, base, present))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(w: u32, h: u32, seed: u8) -> Canvas {
        let mut c = Canvas::new(w, h);
        for i in 0..6u32 {
            let (x, y) = ((i * 5 + seed as u32) % w, (i * 3 + seed as u32 * 2) % h);
            let p = ((y * w + x) * 4) as usize;
            c.pixels[p..p + 4].copy_from_slice(&[seed, 40 + i as u8, 7, 255]);
        }
        c
    }

    /// Two layers over 6 frames: "Ink" keys 0,2,4 (4 repeats 0), "Color" key 0.
    fn project() -> Project {
        let mut p = Project::new(40, 30, 12.0);
        p.ensure_frame_count(6);
        let a = p.cells.len();
        p.cells.push(cell(40, 30, 1).into());
        let b = p.cells.len();
        p.cells.push(cell(40, 30, 2).into());
        p.layers[0].name = "Ink".into();
        p.layers[0].exposures = vec![Some(a), None, Some(b), None, Some(a), None];
        let mut color = Layer::new("Color", 6);
        let c = p.cells.len();
        p.cells.push(cell(40, 30, 3).into());
        color.exposures[0] = Some(c);
        p.layers.push(color);
        p
    }

    type Links = HashMap<u64, String>;

    /// Link `p`: a uuid per layer, and the baseline of what Krita now holds.
    fn link(p: &Project) -> (Links, Baseline) {
        let links: Links = p.layers.iter().map(|l| (l.uid, new_uuid())).collect();
        (links.clone(), base_of(p, &links))
    }

    /// A baseline of `p`'s linked layers as they stand.
    fn base_of(p: &Project, links: &Links) -> Baseline {
        let mut q = p.clone();
        q.layers.retain(|l| links.contains_key(&l.uid));
        let uuids: Vec<String> = q.layers.iter().map(|l| links[&l.uid].clone()).collect();
        Baseline::capture(&q, &uuids)
    }

    /// "Krita saves" `krita` and we plan the pull. `krita`'s layers are named
    /// through `links`; one Krita made up gets a fresh uuid, as Krita would.
    fn pull_from(p: &Project, base: &Baseline, links: &mut Links, krita: &Project) -> PullPlan {
        let uuids: Vec<String> =
            krita.layers.iter().map(|l| links.entry(l.uid).or_insert_with(new_uuid).clone()).collect();
        let bytes = kra::write(krita, &kra::WriteOpts { layer_uuids: &uuids, selected_layer: 0, carried: &[] }).unwrap();
        let pulled = prepare(kra::read_filtered(bytes, &comes_back).unwrap(), base, &present(p, links));
        plan(p, base, links, pulled).unwrap()
    }

    fn paint(p: &mut Project, id: CellId, seed: u8) {
        let px = &mut p.cell_mut(id).unwrap().pixels;
        px[0..4].copy_from_slice(&[seed, seed, seed, 255]);
    }

    /// The whole premise, against real Krita output: what we sent, loaded and
    /// re-saved by Krita 5.2.9 untouched, must pull back as no change at all.
    #[test]
    fn real_krita_resave_of_what_we_sent_is_a_noop() {
        let p = crate::io::kra::tests::fixture_project();
        let uuids = crate::io::kra::tests::uuids(p.layers.len());
        let links: Links = p.layers.iter().map(|l| l.uid).zip(uuids.iter().cloned()).collect();
        let base = Baseline::capture(&p, &uuids);
        let doc = kra::read(include_bytes!("io/kra/testdata/krita-5.2.9-resaved.kra").to_vec()).unwrap();
        let plan = plan(&p, &base, &links, prepare(doc, &base, &present(&p, &links))).unwrap();
        assert!(plan.is_noop(), "{:?}", plan.report);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    #[test]
    fn uuids_are_krita_shaped_and_distinct() {
        let (a, b) = (new_uuid(), new_uuid());
        assert_ne!(a, b);
        assert_eq!(a.len(), 38);
        assert!(a.starts_with('{') && a.ends_with('}'));
        assert_eq!(&a[15..16], "4");
    }

    #[test]
    fn untouched_krita_file_is_a_noop() {
        let p = project();
        let (mut links, base) = link(&p);
        let plan = pull_from(&p, &base, &mut links, &p.clone());
        assert!(plan.is_noop(), "{:?}", plan.report);
    }

    #[test]
    fn only_the_drawing_krita_changed_is_replaced() {
        let mut p = project();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        let b = k.layers[0].exposures[2].unwrap();
        paint(&mut k, b, 99);
        let before = p.layers[0].exposures.clone();
        let old_pixels = p.cells[b].pixels.clone();

        let plan = pull_from(&p, &base, &mut links, &k);
        assert_eq!(plan.report.drawings, 1);
        assert_eq!(plan.report.retimed, 0);
        plan.apply(&mut p);
        let after = &p.layers[0].exposures;
        assert_eq!(after[0], before[0]);
        assert_eq!(after[4], before[4]);
        let new_b = after[2].unwrap();
        assert_ne!(new_b, b);
        assert_eq!(p.cells[new_b].pixels, k.cells[b].pixels);
        // The old cell is untouched — undo only has to restore exposures.
        assert_eq!(p.cells[b].pixels, old_pixels);
        // The other layer wasn't touched at all.
        assert_eq!(p.layers[1].exposures, project().layers[1].exposures);
    }

    #[test]
    fn local_edit_krita_did_not_touch_survives() {
        let mut p = project();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        // Here: edit drawing A. Krita: edit drawing B (same layer).
        let a = p.layers[0].exposures[0].unwrap();
        paint(&mut p, a, 50);
        let b = k.layers[0].exposures[2].unwrap();
        paint(&mut k, b, 99);

        pull_from(&p, &base, &mut links, &k).apply(&mut p);
        assert_eq!(p.layers[0].exposures[0], Some(a));
        assert_eq!(p.cells[a].pixels[0], 50);
    }

    #[test]
    fn both_edited_krita_wins() {
        let mut p = project();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        let b = p.layers[0].exposures[2].unwrap();
        paint(&mut p, b, 50);
        paint(&mut k, b, 99);
        pull_from(&p, &base, &mut links, &k).apply(&mut p);
        let now = p.layers[0].exposures[2].unwrap();
        assert_eq!(p.cells[now].pixels[0], 99);
    }

    #[test]
    fn krita_retime_reuses_cells() {
        let mut p = project();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        let b = k.layers[0].exposures[2].take();
        k.layers[0].exposures[3] = b;
        let plan = pull_from(&p, &base, &mut links, &k);
        assert_eq!(plan.report.drawings, 0);
        assert_eq!(plan.report.retimed, 1);
        plan.apply(&mut p);
        assert_eq!(p.layers[0].exposures[2], None);
        assert_eq!(p.layers[0].exposures[3], b);
    }

    #[test]
    fn local_retime_on_a_layer_krita_left_alone_stays() {
        let mut p = project();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        k.layers[1].name = "Paint".into(); // Krita changes something else
        let b = p.layers[0].exposures[2].take();
        p.layers[0].exposures[1] = b;
        pull_from(&p, &base, &mut links, &k).apply(&mut p);
        assert_eq!(p.layers[0].exposures[1], b);
        assert_eq!(p.layers[1].name, "Paint");
    }

    /// Undo restores layer snapshots taken before the send; the link must
    /// survive that, which is why it keys on `uid` rather than a field set
    /// at send time.
    #[test]
    fn link_survives_undo_to_before_the_send() {
        let mut p = project();
        let before_send = p.layers.clone();
        let (mut links, base) = link(&p);
        p.layers = before_send; // what undo does
        let mut k = p.clone();
        let b = k.layers[0].exposures[2].unwrap();
        paint(&mut k, b, 99);
        let plan = pull_from(&p, &base, &mut links, &k);
        assert_eq!((plan.report.added, plan.report.drawings), (0, 1));
    }

    #[test]
    fn layers_added_removed_and_reordered_in_krita() {
        let mut p = project();
        let (mut links, base) = link(&p);
        // Added here after the send: sits above Ink.
        p.layers.insert(1, Layer::new("Local", 6));

        let mut k = p.clone();
        k.layers.remove(1); // Krita never saw "Local"
        let mut fresh = Layer::new("Krita new", 6);
        let id = k.cells.len();
        k.cells.push(cell(40, 30, 9).into());
        fresh.exposures[1] = Some(id);
        k.layers.push(fresh);

        let plan = pull_from(&p, &base, &mut links, &k);
        assert_eq!(plan.report.added, 1);
        links.extend(plan.apply(&mut p));
        let names: Vec<&str> = p.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Ink", "Local", "Color", "Krita new"]);
        assert!(links.contains_key(&p.layers[3].uid));
        let got = p.layers[3].exposures[1].unwrap();
        assert_eq!(p.cells[got].pixels, k.cells[id].pixels);

        // Now Krita deletes Color and swaps the other two.
        let base = base_of(&p, &links);
        let mut k = p.clone();
        k.layers.remove(1); // Local
        k.layers.remove(1); // Color
        k.layers.swap(0, 1);
        let plan = pull_from(&p, &base, &mut links, &k);
        assert_eq!(plan.report.removed, 1);
        assert!(plan.report.reordered);
        plan.apply(&mut p);
        let names: Vec<&str> = p.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Krita new", "Ink", "Local"]);
    }

    #[test]
    fn krita_key_past_the_end_grows_the_timeline() {
        let mut p = project();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        k.ensure_frame_count(10);
        let a = k.layers[0].exposures[0];
        k.layers[0].exposures[9] = a;
        let plan = pull_from(&p, &base, &mut links, &k);
        assert_eq!(plan.report.grew_to, Some(10));
        plan.apply(&mut p);
        assert_eq!(p.frame_count, 10);
        assert!(p.layers.iter().all(|l| l.exposures.len() == 10));
        assert_eq!(p.layers[0].exposures[9], a);
    }

    #[test]
    fn second_pull_after_first_is_a_noop() {
        let mut p = project();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        let b = k.layers[0].exposures[2].unwrap();
        paint(&mut k, b, 99);
        let plan = pull_from(&p, &base, &mut links, &k);
        let next = plan.next.clone();
        plan.apply(&mut p);
        // Krita saves again without changes.
        let again = pull_from(&p, &next, &mut links, &k);
        assert!(again.is_noop(), "{:?}", again.report);
    }

    /// A clipped layer goes to Krita cut to its base — one drawing per base
    /// drawing under it — and an untouched round trip brings nothing back.
    #[test]
    fn a_clipped_layer_goes_cut_and_an_untouched_round_trip_is_a_noop() {
        let mut p = project();
        // Color, solid and held all the way, clipped to Ink, whose drawings
        // cover the left half and then the right, changing at 2 and 4.
        p.layers[1].clip = true;
        let color = p.layers[1].exposures[0].unwrap();
        let (a, b) = (p.layers[0].exposures[0].unwrap(), p.layers[0].exposures[2].unwrap());
        for (id, left) in [(a, true), (b, false), (color, true)] {
            let c = p.cell_mut(id).unwrap();
            for y in 0..c.height {
                for x in 0..c.width {
                    let i = ((y * c.width + x) * 4) as usize;
                    let on = id == color || (x < c.width / 2) == left;
                    c.pixels[i..i + 4].copy_from_slice(if on { &[9, 9, 9, 255] } else { &[0; 4] });
                }
            }
        }
        let links: Links = p.layers.iter().map(|l| (l.uid, new_uuid())).collect();
        let uuids: Vec<String> = p.layers.iter().map(|l| links[&l.uid].clone()).collect();
        let dir = std::env::temp_dir().join(format!("animator-clip-{}-{}", std::process::id(), new_uuid()));
        let path = dir.join("c.kra");
        let keep = vec![true; p.layers.len()];
        let base = send(p.clone(), &keep, &uuids, Some(0), &path).unwrap().baseline;
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let color_base = base.layer(&uuids[1]).unwrap();
        let times: Vec<usize> = color_base.keys.iter().map(|k| k.0).collect();
        assert_eq!(times, [0, 2, 4], "a drawing per base drawing");
        assert_ne!(color_base.keys[0].1, color_base.keys[1].1, "cut differently");
        assert!(color_base.by_hash.values().all(|&id| id == color), "each stands for the whole drawing");

        let pulled = prepare(kra::read_filtered(bytes, &comes_back).unwrap(), &base, &present(&p, &links));
        let plan = plan(&p, &base, &links, pulled).unwrap();
        assert!(plan.is_noop(), "{:?}", plan.report);
    }

    #[test]
    fn leading_blank_keys_do_not_count_as_changes() {
        let mut p = project();
        // Color's first key moves to frame 2: the writer adds a blank lead-in.
        let c = p.layers[1].exposures[0].take();
        p.layers[1].exposures[2] = c;
        let (mut links, base) = link(&p);
        assert!(pull_from(&p, &base, &mut links, &p.clone()).is_noop());
    }

    #[test]
    fn oversized_layer_cells_keep_their_size() {
        let mut p = project();
        p.layers[1].cell_w = 60;
        p.layers[1].cell_h = 51;
        let id = p.layers[1].exposures[0].unwrap();
        p.cells[id] = cell(60, 51, 3).into();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        paint(&mut k, id, 77);
        pull_from(&p, &base, &mut links, &k).apply(&mut p);
        let now = p.layers[1].exposures[0].unwrap();
        assert_ne!(now, id);
        assert_eq!((p.cells[now].width, p.cells[now].height), (60, 51));
        assert_eq!(p.cells[now].pixels, k.cells[id].pixels);
    }

    #[test]
    fn stays_put_is_a_trailing_dash_x() {
        for yes in ["sketch-x", "BG -X ", "a-x", "notes-X"] {
            assert!(stays_put(yes), "{yes:?}");
        }
        for no in ["x", "-x", "box", "ax", "sketch-x2", "-x-y", ""] {
            assert!(!stays_put(no), "{no:?}");
        }
        assert!(comes_back("Ink", &[]));
        assert!(!comes_back("Ink", &["refs-x".into()]));
        assert!(!comes_back("sketch-x", &[]));
    }

    /// A project whose bottom layer is a reference and top layer is named -x:
    /// only "Ink" goes.
    fn project_with_stay_here_layers() -> Project {
        let mut p = project();
        p.layers[1].reference = true; // Color
        let mut notes = Layer::new("notes-x", 6);
        let id = p.cells.len();
        p.cells.push(cell(40, 30, 7).into());
        notes.exposures[0] = Some(id);
        p.layers.push(notes);
        p
    }

    fn send_linked(p: &Project) -> (Links, Baseline, Vec<u8>) {
        let keep: Vec<bool> = p.layers.iter().map(|l| !stays_here(l)).collect();
        let mut links = Links::new();
        let mut uuids = Vec::new();
        for (l, &k) in p.layers.iter().zip(&keep) {
            if k {
                let u = new_uuid();
                links.insert(l.uid, u.clone());
                uuids.push(u);
            }
        }
        let dir = std::env::temp_dir().join(format!("animator-stay-{}-{}", std::process::id(), new_uuid()));
        let path = dir.join("s.kra");
        let base = send(p.clone(), &keep, &uuids, Some(0), &path).unwrap().baseline;
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        (links, base, bytes)
    }

    #[test]
    fn reference_and_dash_x_layers_are_not_sent() {
        let p = project_with_stay_here_layers();
        let (links, base, bytes) = send_linked(&p);
        let doc = kra::read(bytes.clone()).unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Ink"]);
        assert_eq!(base.layers.len(), 1);
        // Pulling the untouched file changes nothing here.
        let pulled = prepare(kra::read_filtered(bytes, &comes_back).unwrap(), &base, &present(&p, &links));
        let plan = plan(&p, &base, &links, pulled).unwrap();
        assert!(plan.is_noop(), "{:?}", plan.report);
        assert!(plan.unlink.is_empty() && plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    #[test]
    fn a_dash_x_layer_made_in_krita_stays_there() {
        let p = project();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        let mut sketch = Layer::new("sketch-x", 6);
        let id = k.cells.len();
        k.cells.push(cell(40, 30, 9).into());
        sketch.exposures[0] = Some(id);
        k.layers.push(sketch);
        let plan = pull_from(&p, &base, &mut links, &k);
        assert!(plan.is_noop(), "{:?}", plan.report);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    #[test]
    fn renamed_dash_x_in_krita_stops_syncing_and_keeps_ours() {
        let p = project();
        let (mut links, base) = link(&p);
        let before = p.layers[0].exposures.clone();
        let mut k = p.clone();
        k.layers[0].name = "Ink-x".into();
        let b = k.layers[0].exposures[2].unwrap();
        paint(&mut k, b, 99); // and Krita changes it too
        let plan = pull_from(&p, &base, &mut links, &k);
        assert!(plan.is_noop(), "{:?}", plan.report);
        assert_eq!(plan.unlink, [p.layers[0].uid]);
        assert!(plan.warnings.iter().any(|w| w.contains("now stays in Krita")), "{:?}", plan.warnings);
        assert!(plan.next.layers.iter().all(|l| l.name != "Ink-x" && l.name != "Ink"));

        // What the app does with it: drop the link, keep the new baseline.
        // Krita keeps its own uuids, so remember them before the link goes.
        let krita_uuids: Vec<String> = k.layers.iter().map(|l| links[&l.uid].clone()).collect();
        links.retain(|uid, _| !plan.unlink.contains(uid));
        let next = plan.next.clone();
        let mut p2 = p.clone();
        plan.apply(&mut p2);
        assert_eq!(p2.layers[0].name, "Ink");
        assert_eq!(p2.layers[0].exposures, before);

        // The same Krita file again: quiet.
        let bytes = kra::write(&k, &kra::WriteOpts { layer_uuids: &krita_uuids, selected_layer: 0, carried: &[] }).unwrap();
        let pulled = prepare(kra::read_filtered(bytes, &comes_back).unwrap(), &next, &present(&p2, &links));
        let again = super::plan(&p2, &next, &links, pulled).unwrap();
        assert!(again.is_noop() && again.unlink.is_empty() && again.warnings.is_empty(), "{:?}", again.warnings);
    }

    /// Quotes, markup characters and non-Latin scripts in layer names go out
    /// and come back unchanged, both ways.
    #[test]
    fn odd_layer_names_survive_the_round_trip() {
        let mut p = project();
        p.layers[0].name = r#"Ink "&<>' 线稿"#.into();
        p.layers[1].name = "Hair -X2".into(); // ends in X2, not -x: synced
        let (mut links, base) = link(&p);
        assert!(pull_from(&p, &base, &mut links, &p.clone()).is_noop());

        let mut k = p.clone();
        k.layers[1].name = "スケッチ «final»".into();
        let mut p2 = p.clone();
        pull_from(&p, &base, &mut links, &k).apply(&mut p2);
        assert_eq!(p2.layers[0].name, p.layers[0].name);
        assert_eq!(p2.layers[1].name, "スケッチ «final»");
    }

    /// Krita adds `sketch-x` between two synced layers and saves; the next
    /// send from here rewrites the file but leaves the sketch in its place.
    #[test]
    fn resend_keeps_krita_only_layers_in_place() {
        let p = project();
        let keep = vec![true; p.layers.len()];
        let uuids: Vec<String> = p.layers.iter().map(|_| new_uuid()).collect();
        let dir = std::env::temp_dir().join(format!("animator-keep-{}-{}", std::process::id(), new_uuid()));
        let path = dir.join("k.kra");
        assert!(send(p.clone(), &keep, &uuids, Some(0), &path).unwrap().kept.is_empty());

        // Krita's save: the sketch sits between Ink and Color.
        let mut k = p.clone();
        let mut sketch = Layer::new("sketch-x", 6);
        let id = k.cells.len();
        k.cells.push(cell(40, 30, 9).into());
        sketch.exposures[0] = Some(id);
        k.layers.insert(1, sketch);
        let ku = vec![uuids[0].clone(), new_uuid(), uuids[1].clone()];
        std::fs::write(&path, kra::write(&k, &kra::WriteOpts { layer_uuids: &ku, selected_layer: 0, carried: &[] }).unwrap())
            .unwrap();

        // Send again from here, where the sketch never existed.
        let mut p2 = p.clone();
        p2.layers[1].name = "Paint".into();
        let sent = send(p2, &keep, &uuids, Some(0), &path).unwrap();
        assert_eq!(sent.kept, ["sketch-x"]);
        let doc = kra::read(std::fs::read(&path).unwrap()).unwrap();
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Ink", "sketch-x", "Paint"]);
        let sk = &doc.layers[1];
        let (c, _) = sk.drawings[sk.keys[0].1].place(0, 0, 40, 30);
        assert_eq!(c.pixels, cell(40, 30, 9).pixels);
        // Baseline: only what came from here.
        assert_eq!(sent.baseline.layers.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_layer_made_reference_here_after_the_send_unlinks() {
        let mut p = project();
        let (mut links, base) = link(&p);
        let mut k = p.clone();
        p.layers[1].reference = true; // Color, here, after the send
        let c = k.layers[1].exposures[0].unwrap();
        paint(&mut k, c, 99); // Krita edits it anyway
        let before = p.layers[1].exposures.clone();
        let plan = pull_from(&p, &base, &mut links, &k);
        assert!(plan.is_noop(), "{:?}", plan.report);
        assert_eq!(plan.unlink, [p.layers[1].uid]);
        assert_eq!(plan.report.added, 0);
        plan.apply(&mut p);
        assert_eq!(p.layers.len(), 2);
        assert_eq!(p.layers[1].exposures, before);
    }
}
