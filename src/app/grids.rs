//! Perspective grids on the app's side: which file's grids are live, what
//! each one follows, its keys, canvas drags on it, and their undo.
//!
//! Grids are never written to the `.anim` file. The app keeps one set per
//! project file, remembered by path in the workspace preferences: opening a
//! file brings its grids back, a new file starts with one default grid, and
//! Save As takes the grids along to the new path.
//!
//! A child of `app` so it can work on `AppState`'s private state directly;
//! split out only because `app.rs` is long enough already.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{AppState, GridDrag, HORIZON_ATTACH_PX, HORIZON_DETACH_PX};
use crate::doc::camera::Ease;
use crate::doc::transform::Transform;
use crate::tools::axis_lock;
use crate::tools::guides;
use crate::tools::perspective::{
    self, Follow, GridGrab, GridKind, PerspectiveConfig, PerspectiveGrid, Space, P,
};
use crate::undo;

/// One project file's grids, as the app keeps them between sessions.
///
/// Saved through [`GridFileRepr`], so an older build — which knows only the
/// plane kinds, and would throw away *every* preference over one guide kind
/// it can't read — still finds just planes where it looks.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(from = "GridFileRepr", into = "GridFileRepr")]
pub struct GridFile {
    pub path: PathBuf,
    /// Every guide, in list order: planes, rulers, points, ellipses.
    pub grids: Vec<PerspectiveGrid>,
    pub active: usize,
    /// Seconds since the Unix epoch it was last open. Past
    /// [`MAX_GRID_FILES`] the stalest are forgotten first.
    pub used: u64,
}

/// [`GridFile`] as written. `grids` and `active` keep their old shape and
/// hold only planes, so an older build reads them as it always did; the
/// other guides go in `guides`, each with its place in the whole list, where
/// an older build doesn't look.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct GridFileRepr {
    path: PathBuf,
    grids: Vec<PerspectiveGrid>,
    /// Into `grids` — what an older build makes active.
    active: usize,
    used: u64,
    guides: Vec<(usize, PerspectiveGrid)>,
    /// The active guide's place in the whole list. Only meant when
    /// `split` is set: a file from before has none, and `active` is it.
    active_all: usize,
    /// 1 once written this way; 0 (missing) in an older file.
    split: u8,
}

impl From<GridFile> for GridFileRepr {
    fn from(f: GridFile) -> Self {
        let mut grids = Vec::new();
        let mut guides = Vec::new();
        let mut active = 0;
        for (i, g) in f.grids.into_iter().enumerate() {
            if g.kind.is_plane() {
                if i <= f.active {
                    active = grids.len();
                }
                grids.push(g);
            } else {
                guides.push((i, g));
            }
        }
        Self {
            path: f.path,
            active: active.min(grids.len().saturating_sub(1)),
            grids,
            used: f.used,
            guides,
            active_all: f.active,
            split: 1,
        }
    }
}

impl From<GridFileRepr> for GridFile {
    fn from(r: GridFileRepr) -> Self {
        let n = r.grids.len() + r.guides.len();
        let mut slots: Vec<Option<PerspectiveGrid>> = vec![None; n];
        let mut late = Vec::new();
        for (i, g) in r.guides {
            match slots.get_mut(i) {
                Some(s @ None) => *s = Some(g),
                _ => late.push(g),
            }
        }
        let mut planes = r.grids.into_iter();
        for s in slots.iter_mut().filter(|s| s.is_none()) {
            *s = planes.next();
        }
        let mut grids: Vec<PerspectiveGrid> = slots.into_iter().flatten().collect();
        grids.extend(planes);
        grids.extend(late);
        let active = if r.split == 0 { r.active } else { r.active_all };
        Self {
            path: r.path,
            active: active.min(grids.len().saturating_sub(1)),
            grids,
            used: r.used,
        }
    }
}

/// Most files whose grids the app remembers.
const MAX_GRID_FILES: usize = 256;

/// A `.guides` file: guides exported from one project, to bring into
/// another. Text, so it can be read and mailed about.
#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct GuidesFile {
    /// 1 for now; a later layout bumps it.
    version: u32,
    guides: Vec<PerspectiveGrid>,
}

/// Write `guides` to a `.guides` file at `path`.
pub fn write_guides(path: &Path, guides: &[PerspectiveGrid]) -> anyhow::Result<()> {
    let file = GuidesFile {
        version: 1,
        guides: guides.to_vec(),
    };
    let text = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())?;
    std::fs::write(path, text)?;
    Ok(())
}

/// The guides in the `.guides` file at `path`.
pub fn read_guides(path: &Path) -> anyhow::Result<Vec<PerspectiveGrid>> {
    let text = std::fs::read_to_string(path)?;
    let file: GuidesFile = ron::from_str(&text)?;
    Ok(file.guides)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether two paths name the same file as far as the store is concerned:
/// case-blind on Windows, as its file system is.
fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

impl AppState {
    // --- Which file's grids ---

    /// File the live grids under the open project's path, if it has one.
    pub(super) fn stash_grids(&mut self) {
        if let Some(path) = self.project_path.clone() {
            self.store_grids_as(&path);
        }
    }

    fn store_grids_as(&mut self, path: &Path) {
        let entry = GridFile {
            path: path.to_path_buf(),
            grids: self.perspective.grids.clone(),
            active: self.perspective.active,
            used: now_secs(),
        };
        match self
            .grid_files
            .iter_mut()
            .find(|f| same_path(&f.path, path))
        {
            Some(f) => *f = entry,
            None => self.grid_files.push(entry),
        }
        if self.grid_files.len() > MAX_GRID_FILES {
            self.grid_files.sort_by_key(|f| std::cmp::Reverse(f.used));
            self.grid_files.truncate(MAX_GRID_FILES);
        }
    }

    /// Make `path`'s grids the live ones: those it had, or — for a file the
    /// app has never kept grids for — the grids from before they were kept
    /// per file, so nothing set up then goes missing. A new, unsaved project
    /// (`None`) starts with one default grid.
    pub(super) fn unstash_grids(&mut self, path: Option<&Path>) {
        let kept = path.and_then(|p| self.grid_files.iter().find(|f| same_path(&f.path, p)));
        let (grids, active) = match (kept, path) {
            (Some(f), _) => (f.grids.clone(), f.active),
            (None, Some(_)) => (self.grid_seed.clone(), 0),
            (None, None) => (PerspectiveConfig::default().grids, 0),
        };
        self.perspective.grids = grids;
        self.perspective.active = active.min(self.perspective.grids.len().saturating_sub(1));
        self.perspective.ensure_ids();
        self.grid_drag = None;
        self.grid_drag_before = None;
        self.grid_sync_last = None;
    }

    /// A save just landed at `path`. Save As takes the grids along to the new
    /// path, and the old file keeps the ones it had.
    pub(super) fn grids_saved_as(&mut self, path: &Path) {
        self.stash_grids();
        self.store_grids_as(path);
    }

    /// The store as the preferences should save it, with the live grids
    /// filed first.
    pub(super) fn grid_files_for_prefs(&mut self) -> Vec<GridFile> {
        self.stash_grids();
        self.grid_files.clone()
    }

    // --- Guide files ---

    /// Export every guide of the open file to a `.guides` file the artist
    /// picks.
    pub fn export_guides(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Guides", &["guides"])
            .set_file_name("guides.guides")
            .set_title("Export guides")
            .save_file()
        else {
            return;
        };
        let n = self.perspective.grids.len();
        self.guide_file_note = Some(match write_guides(&path, &self.perspective.grids) {
            Ok(()) => format!("Exported {n} guide{}", if n == 1 { "" } else { "s" }),
            Err(e) => format!("Couldn't export the guides: {e:#}"),
        });
    }

    /// Add the guides in a `.guides` file the artist picks to the open
    /// file's own.
    pub fn import_guides(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Guides", &["guides"])
            .set_title("Import guides")
            .pick_file()
        else {
            return;
        };
        self.guide_file_note = Some(match read_guides(&path) {
            Ok(guides) => {
                let n = self.merge_guides(guides);
                format!("Imported {n} guide{}", if n == 1 { "" } else { "s" })
            }
            Err(e) => format!("Couldn't read {}: {e:#}", path.display()),
        });
    }

    /// Append `incoming` to the live guides under fresh ids, keeping each
    /// wall on its own floor — or freeing it, if its floor didn't come too.
    /// The last one added becomes active. How many came in.
    pub(super) fn merge_guides(&mut self, incoming: Vec<PerspectiveGrid>) -> usize {
        let cfg = &mut self.perspective;
        let first = cfg.grids.len();
        let mut ids = std::collections::HashMap::new();
        for g in incoming {
            let old = g.id;
            let i = cfg.push(g);
            if old != 0 {
                ids.insert(old, cfg.grids[i].id);
            }
        }
        for g in &mut cfg.grids[first..] {
            if let Some(w) = g.wall.as_mut() {
                match ids.get(&w.parent) {
                    Some(&id) => w.parent = id,
                    None => g.wall = None,
                }
            }
        }
        cfg.relink();
        cfg.grids.len() - first
    }

    // --- What a grid follows ---

    /// The layer `f` follows, if it is a layer and that layer is still there.
    fn followed_layer(&self, f: &Follow) -> Option<usize> {
        let Follow::Layer { uid, .. } = f else {
            return None;
        };
        self.project.layers.iter().position(|l| l.uid == *uid)
    }

    /// Re-find every followed layer: by its session id, or — fresh from the
    /// store, where ids don't survive — by name, its old position first. The
    /// name and position are kept current, so the next session finds it too.
    fn refresh_follow_links(&mut self) {
        let layers = &self.project.layers;
        for g in &mut self.perspective.grids {
            let Follow::Layer { name, index, uid } = &mut g.follow else {
                continue;
            };
            let found = layers
                .iter()
                .position(|l| *uid != 0 && l.uid == *uid)
                .or_else(|| {
                    layers
                        .get(*index)
                        .filter(|l| l.name == *name)
                        .map(|_| *index)
                })
                .or_else(|| layers.iter().position(|l| l.name == *name));
            if let Some(i) = found {
                *uid = layers[i].uid;
                *index = i;
                name.clone_from(&layers[i].name);
            }
        }
    }

    /// Where grid `g`'s coordinates land in the document on `frame`: carried
    /// by the camera or the layer it follows, as they stand on that frame.
    /// A grid whose layer is gone stays on the document.
    pub fn grid_space(&self, g: &PerspectiveGrid, frame: usize) -> Space {
        let (w, h) = self.frame_size();
        let xf = match &g.follow {
            Follow::Document => Transform::default(),
            Follow::Camera => self.display_camera(frame).as_frame_transform(),
            Follow::Layer { .. } => self
                .followed_layer(&g.follow)
                .map(|i| self.display_transform(i, frame))
                .unwrap_or_default(),
        };
        Space { w, h, xf }
    }

    /// The follow link for layer `i`, ready to hand to [`Self::set_grid_follow`].
    pub fn follow_layer(&self, i: usize) -> Option<Follow> {
        let l = self.project.layers.get(i)?;
        Some(Follow::Layer {
            name: l.name.clone(),
            index: i,
            uid: l.uid,
        })
    }

    /// Point grid `i` at a new parent, keeping it where it shows on this
    /// frame: its live pose and every key are carried across through this
    /// frame's two spaces. A wall follows whatever its floor does.
    pub fn set_grid_follow(&mut self, i: usize, follow: Follow) {
        let f = self.project.current_frame;
        let Some(g) = self.perspective.grids.get(i) else {
            return;
        };
        if g.follow == follow || g.is_wall() {
            return;
        }
        let old = self.grid_space(g, f);
        let mut moved = g.clone();
        moved.follow = follow;
        let new = self.grid_space(&moved, f);
        let carry = |n: P| new.to_rel(old.to_doc(n));
        moved.corners = moved.corners.map(carry);
        for v in &mut moved.extra_vps {
            v.pos = carry(v.pos);
        }
        for k in &mut moved.keys {
            k.pose.corners = k.pose.corners.map(carry);
            for e in &mut k.pose.extras {
                *e = carry(*e);
            }
        }
        self.perspective.grids[i] = moved;
        self.perspective.relink();
    }

    /// Once a frame: re-find followed layers, bring keyed grids to the
    /// current frame when the cursor has moved (never mid-drag — the same
    /// contract as the camera's buffer), and rebuild walls from their floors.
    pub(super) fn sync_grid_buffers(&mut self) {
        self.refresh_follow_links();
        let f = self.project.current_frame;
        if self.grid_drag.is_none() && self.grid_sync_last != Some(f) {
            self.grid_sync_last = Some(f);
            for g in &mut self.perspective.grids {
                if !g.keys.is_empty() {
                    let p = g.resolve(f);
                    g.set_pose(&p);
                }
            }
        }
        self.perspective.relink();
    }

    // --- Keys ---

    /// Run `edit` on grid `i` (given the current frame) as one undo step, or
    /// none if it changed nothing.
    pub fn grid_edit(&mut self, i: usize, edit: impl FnOnce(&mut PerspectiveGrid, usize)) {
        let f = self.project.current_frame;
        let Some(g) = self.perspective.grids.get_mut(i) else {
            return;
        };
        let before = g.clone();
        edit(g, f);
        if *g != before {
            let after = g.clone();
            self.history.push(undo::Command::Grid {
                before: Box::new(before),
                after: Box::new(after),
            });
        }
        self.perspective.relink();
    }

    /// Key the active grid's live pose on the current frame. Walls take
    /// their motion from their floor and have no keys. Undoable.
    pub fn add_grid_key(&mut self) {
        let i = self.perspective.active;
        self.grid_edit(i, |g, f| {
            if !g.is_wall() {
                g.set_key(f);
            }
        });
    }

    /// Delete the active grid's key on the current frame, and show what the
    /// keys left give this frame. Undoable.
    pub fn delete_grid_key(&mut self) {
        let i = self.perspective.active;
        self.grid_edit(i, |g, f| {
            if g.has_key(f) {
                g.delete_key(f);
                let p = g.resolve(f);
                g.set_pose(&p);
            }
        });
    }

    /// Set the ease out of the active grid's key on the current frame.
    /// Undoable.
    pub fn set_grid_key_ease(&mut self, ease: Ease) {
        let i = self.perspective.active;
        self.grid_edit(i, |g, f| g.set_key_ease(f, ease));
    }

    /// Grid `i`'s pose was just changed by hand. With auto-key on, a grid
    /// that already has keys takes one here, so the change outlasts a frame
    /// change; a grid with none just moves.
    pub fn grid_pose_edited(&mut self, i: usize) {
        if !self.perspective.auto_key {
            return;
        }
        let f = self.project.current_frame;
        if let Some(g) = self.perspective.grids.get_mut(i) {
            if !g.keys.is_empty() && !g.is_wall() {
                g.set_key(f);
            }
        }
    }

    /// Put back what an undo or redo of a grid step recorded. A keyed grid
    /// then shows its keys' pose for the frame the cursor is on now.
    pub(super) fn restore_grid(&mut self, snap: &PerspectiveGrid) {
        let f = self.project.current_frame;
        let Some(i) = self.perspective.index_of(snap.id) else {
            return;
        };
        let g = &mut self.perspective.grids[i];
        g.restore_motion(snap);
        if !g.keys.is_empty() {
            let p = g.resolve(f);
            g.set_pose(&p);
        }
        self.grid_drag = None;
        self.grid_drag_before = None;
        self.perspective.relink();
    }

    // --- The perspective tool on the canvas ---

    /// Give the perspective tool something to edit: selecting it with every
    /// grid deleted brings a default one back.
    pub fn ensure_perspective_grid(&mut self) {
        if self.perspective.grids.is_empty() {
            self.perspective.push(PerspectiveGrid::default());
        }
        self.perspective.active = self
            .perspective
            .active
            .min(self.perspective.grids.len() - 1);
    }

    /// Make the next visible grid after the active one active, wrapping
    /// round — how to switch the grid strokes snap to without opening the
    /// grid list. Hidden grids are skipped: nothing can snap to them.
    pub fn cycle_perspective_grid(&mut self) {
        let cfg = &mut self.perspective;
        let n = cfg.grids.len();
        if let Some(next) = (1..n)
            .map(|k| (cfg.active + k) % n)
            .find(|&i| cfg.grids[i].visible)
        {
            cfg.active = next;
        }
    }

    /// Perspective tool press at document point `p`. The active grid gets
    /// first pick; a press on another visible grid makes it active and grabs
    /// it. A locked grid is selected but never grabbed, and a wall only by
    /// its top corners — the rest of it is its floor's.
    pub fn perspective_down(&mut self, p: P) {
        self.grid_drag = None;
        self.grid_drag_before = None;
        let f = self.project.current_frame;
        let tol = crate::tools::selection::HANDLE_PX / self.view_scale.max(1e-6);
        let n = self.perspective.grids.len();
        let active = self.perspective.active;
        let order = std::iter::once(active).chain((0..n).filter(|&i| i != active));
        for i in order {
            let Some(g) = self.perspective.grids.get(i) else {
                continue;
            };
            if !g.visible {
                continue;
            }
            let space = self.grid_space(g, f);
            let corners = g.doc_corners(&space);
            let extra_at = g.extra_doc(&space);
            let eye_level = g.horizon && g.kind == GridKind::Perspective && !g.is_wall();
            let grab = if g.kind.is_plane() {
                perspective::grab_at(&corners, &extra_at, p, tol, eye_level)
            } else {
                guides::grab_at(g.kind, &corners, p, tol)
            };
            let Some(grab) = grab else {
                continue;
            };
            self.perspective.active = i;
            let grabbable = !g.locked && (!g.is_wall() || matches!(grab, GridGrab::Corner(0 | 1)));
            if grabbable {
                let mut extra = [None; perspective::MAX_EXTRA_VPS];
                for (slot, &v) in extra.iter_mut().zip(&extra_at) {
                    *slot = Some(v);
                }
                self.grid_drag_before = Some(g.clone());
                self.grid_drag = Some(GridDrag {
                    grid: i,
                    grab,
                    start: p,
                    corners,
                    extra,
                    space,
                });
            }
            return;
        }
    }

    /// Perspective tool drag to document point `p`. A drag that would fold
    /// the quad leaves it where it last was. Moving or turning the whole
    /// grid carries its extra vanishing points along; reshaping it leaves them
    /// be, and those on the horizon ride it wherever it goes. A wall's top
    /// corner sets how tall it stands.
    ///
    /// Shift, pressed partway, holds whatever the drag slides — the grid, a
    /// corner, a vanishing point — to the horizontal or vertical through
    /// the press, and snaps what it turns to 15° steps.
    pub fn perspective_move(&mut self, p: P) {
        let Some(d) = self.grid_drag else {
            return;
        };
        let s = d.space;
        let kind = self.perspective.grids.get(d.grid).map_or(GridKind::Perspective, |g| g.kind);
        let rigid = kind.rigid();
        // A ruler's end and a helper line's end turn in 15° steps about the
        // other end instead (see `guides::dragged`).
        let turns = matches!(kind, GridKind::Ruler | GridKind::Parallel | GridKind::Point);
        let slides = match d.grab {
            GridGrab::Move | GridGrab::Vp(_) | GridGrab::Extra(_) => true,
            GridGrab::Corner(_) => !rigid && !turns,
            _ => false,
        };
        let snap = self.axis_lock_on();
        let centred = self.centre_on();
        let p = if snap && slides {
            let (x, y) = axis_lock::lock_hv((d.start[0], d.start[1]), (p[0], p[1]), self.lock_basis_doc());
            [x, y]
        } else {
            p
        };
        let moved = |from: P| [from[0] + p[0] - d.start[0], from[1] + p[1] - d.start[1]];
        if let Some(link) = self.perspective.grids.get(d.grid).and_then(|g| g.wall) {
            if let (GridGrab::Corner(c @ (0 | 1)), Some(pi)) =
                (d.grab, self.perspective.index_of(link.parent))
            {
                let to = s.to_rel(moved(d.corners[c as usize]));
                let parent = &self.perspective.grids[pi];
                if let Some(h) = perspective::wall_height_at(parent, link.edge, c, to) {
                    if let Some(w) = self.perspective.grids[d.grid].wall.as_mut() {
                        w.height = h;
                    }
                    self.perspective.relink();
                }
            }
            return;
        }
        let px = 1.0 / self.view_scale.max(1e-6);
        let Some(g) = self.perspective.grids.get_mut(d.grid) else {
            return;
        };
        if let GridGrab::Extra(i) = d.grab {
            let (Some(from), Some(v)) = (d.extra[i as usize], g.extra_vps.get(i as usize)) else {
                return;
            };
            let horizon = perspective::Plane::new(g.doc_corners(&s)).and_then(|pl| pl.horizon());
            let (at, on_horizon) = perspective::place_extra(
                horizon,
                v.on_horizon,
                moved(from),
                HORIZON_DETACH_PX * px,
                HORIZON_ATTACH_PX * px,
            );
            g.extra_vps[i as usize].pos = s.to_rel(at);
            g.extra_vps[i as usize].on_horizon = on_horizon;
            return;
        }
        // The centre key (Alt) on a two-point grid's vanishing point moves
        // both, evenly about the middle between them, as in Harmony.
        let both = match d.grab {
            GridGrab::Vp(which) if kind == GridKind::Perspective && centred => {
                mirror_vps(&d.corners, which, [p[0] - d.start[0], p[1] - d.start[1]])
            }
            _ => None,
        };
        let c = if both.is_some() {
            both
        } else if kind.is_plane() {
            perspective::dragged(&d.corners, d.grab, d.start, p, snap, rigid)
        } else {
            guides::dragged(kind, &d.corners, d.grab, d.start, p, snap)
        };
        let Some(c) = c else {
            return;
        };
        let whole = matches!(d.grab, GridGrab::Move | GridGrab::Rotate)
            || (rigid && matches!(d.grab, GridGrab::Corner(_)));
        if whole {
            for (v, from) in g.extra_vps.iter_mut().zip(d.extra) {
                if let Some(from) = from {
                    v.pos = s.to_rel(perspective::carry(&d.corners, &c, from));
                }
            }
        }
        g.set_doc_corners(c, &s);
        self.perspective.relink();
    }

    /// What the grid drag in hand is turning, in degrees, for beside the pen:
    /// a ruler's or a helper line's heading, a grid turned whole, a horizon
    /// tilted. Up the screen counts positive, as on a protractor.
    pub fn grid_drag_readout(&self) -> Option<String> {
        let d = self.grid_drag?;
        let g = self.perspective.grids.get(d.grid)?;
        let c = g.doc_corners(&d.space);
        // A line has no head or tail: (-90°, 90°].
        let line = |v: P| {
            let mut a = (-v[1]).atan2(v[0]).to_degrees();
            if a > 90.0 {
                a -= 180.0;
            } else if a <= -90.0 {
                a += 180.0;
            }
            format!("{a:.1}°")
        };
        match (g.kind, d.grab) {
            (GridKind::Ruler | GridKind::Parallel, GridGrab::Corner(_)) => Some(line(perspective::sub(c[1], c[0]))),
            (GridKind::Point, GridGrab::Corner(i)) => {
                let (a, b) = if matches!(i, 0 | 3) { (c[0], c[3]) } else { (c[1], c[2]) };
                Some(line(perspective::sub(a, b)))
            }
            (_, GridGrab::Rotate) => Some(format!("{:.1}°", -g.angle().to_degrees())),
            (GridKind::Perspective, GridGrab::Tilt) => {
                let hz = perspective::Plane::new(c)?.horizon()?;
                Some(line(hz.d))
            }
            _ => None,
        }
    }

    /// The pen lifted off a grid drag: key it where auto-key says to, and
    /// record the whole drag as one undo step.
    pub(super) fn finish_grid_drag(&mut self) {
        let drag = self.grid_drag.take();
        let (Some(d), Some(before)) = (drag, self.grid_drag_before.take()) else {
            return;
        };
        self.grid_pose_edited(d.grid);
        let Some(after) = self.perspective.grids.get(d.grid) else {
            return;
        };
        if *after != before {
            self.history.push(undo::Command::Grid {
                before: Box::new(before),
                after: Box::new(after.clone()),
            });
        }
    }
}

/// A grid's corners re-aimed with its vanishing point `which` moved by
/// `delta` and the other moved the opposite way, so both stay as far from the
/// middle between them. `None` unless both are finite, or if the quad would
/// fold.
fn mirror_vps(c: &[P; 4], which: u8, delta: P) -> Option<[P; 4]> {
    let plane = perspective::Plane::new(*c)?;
    let (rows, cols) = (plane.vp_point(0)?, plane.vp_point(1)?);
    let (rows, cols) = if which == 0 {
        (perspective::add(rows, delta), perspective::sub(cols, delta))
    } else {
        (perspective::sub(rows, delta), perspective::add(cols, delta))
    };
    perspective::re_aim(c, perspective::Vp::Point(rows), perspective::Vp::Point(cols))
}

#[cfg(test)]
mod tests {
    use super::super::tests::MemStorage;
    use super::*;

    const KEY: &str = "grid_files";

    fn file() -> GridFile {
        let kinds = [
            GridKind::Perspective,
            GridKind::Ruler,
            GridKind::Flat,
            GridKind::Ellipse,
            GridKind::Point,
        ];
        GridFile {
            path: PathBuf::from("shot.anim"),
            grids: kinds.map(PerspectiveGrid::fresh).to_vec(),
            active: 3,
            used: 7,
        }
    }

    /// The store as an older build declares it: planes only.
    #[derive(serde::Serialize, serde::Deserialize, Default)]
    #[serde(default)]
    struct OldGridFile {
        path: PathBuf,
        grids: Vec<OldGrid>,
        active: usize,
        used: u64,
    }

    #[derive(serde::Serialize, serde::Deserialize, Default)]
    #[serde(default)]
    struct OldGrid {
        kind: OldKind,
        corners: [P; 4],
    }

    #[derive(serde::Serialize, serde::Deserialize, Default, Debug, PartialEq)]
    enum OldKind {
        #[default]
        Perspective,
        Flat,
        Isometric,
    }

    #[test]
    fn exported_guides_come_back_in_another_file_with_their_walls() {
        let mut from = AppState::for_test();
        from.perspective.grids = vec![PerspectiveGrid::default(), PerspectiveGrid::fresh(GridKind::Ruler)];
        from.perspective.ensure_ids();
        assert!(from.perspective.add_wall(0, 1).is_some());
        let path = std::env::temp_dir().join(format!("animator_guides_{}.guides", std::process::id()));
        write_guides(&path, &from.perspective.grids).unwrap();
        let read = read_guides(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(read, from.perspective.grids);

        let mut to = AppState::for_test();
        to.perspective.grids = vec![PerspectiveGrid::fresh(GridKind::Flat)];
        to.perspective.ensure_ids();
        assert_eq!(to.merge_guides(read), 3);
        let g = &to.perspective.grids;
        assert_eq!(g.len(), 4);
        assert_eq!(to.perspective.active, 3);
        let mut ids: Vec<u32> = g.iter().map(|g| g.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), 4, "every id its own");
        // The wall stands on the floor that came with it.
        let wall = g[3].wall.expect("still a wall");
        assert_eq!(wall.parent, g[1].id);
    }

    #[test]
    fn a_wall_without_its_floor_comes_in_free() {
        let mut from = AppState::for_test();
        from.perspective.grids = vec![PerspectiveGrid::default()];
        from.perspective.ensure_ids();
        let w = from.perspective.add_wall(0, 2).unwrap();
        let alone = vec![from.perspective.grids[w].clone()];
        let mut to = AppState::for_test();
        to.merge_guides(alone);
        assert!(to.perspective.grids.last().unwrap().wall.is_none());
    }

    #[test]
    fn the_centre_key_moves_both_vanishing_points_evenly() {
        let g = PerspectiveGrid::preset(perspective::Preset::TwoPoint);
        let plane = perspective::Plane::new(g.corners).unwrap();
        let (r0, c0) = (plane.vp_point(0).unwrap(), plane.vp_point(1).unwrap());
        let c = mirror_vps(&g.corners, 0, [0.1, 0.0]).unwrap();
        let plane = perspective::Plane::new(c).unwrap();
        let (r1, c1) = (plane.vp_point(0).unwrap(), plane.vp_point(1).unwrap());
        let near = |a: P, b: P| (a[0] - b[0]).abs() < 2e-3 && (a[1] - b[1]).abs() < 2e-3;
        assert!(near(r1, [r0[0] + 0.1, r0[1]]), "{r1:?}");
        assert!(near(c1, [c0[0] - 0.1, c0[1]]), "{c1:?}");
    }

    #[test]
    fn the_store_keeps_every_guide_kind_in_order() {
        let mut storage = MemStorage::default();
        eframe::set_value(&mut storage, KEY, &vec![file()]);
        let back: Vec<GridFile> = eframe::get_value(&storage, KEY).expect("reads back");
        assert_eq!(back, vec![file()]);
    }

    #[test]
    fn an_older_build_reads_only_the_planes() {
        let mut storage = MemStorage::default();
        eframe::set_value(&mut storage, KEY, &vec![file()]);
        let old: Vec<OldGridFile> = eframe::get_value(&storage, KEY).expect("an older build still reads it");
        let kinds: Vec<_> = old[0].grids.iter().map(|g| &g.kind).collect();
        assert_eq!(kinds, [&OldKind::Perspective, &OldKind::Flat]);
        // The ellipse was active: the plane before it stands in.
        assert_eq!(old[0].active, 1);
        assert_eq!(old[0].used, 7);
    }

    #[test]
    fn a_store_an_older_build_wrote_still_loads() {
        let mut storage = MemStorage::default();
        let written = vec![OldGridFile {
            path: PathBuf::from("old.anim"),
            grids: vec![
                OldGrid { kind: OldKind::Flat, corners: PerspectiveGrid::fresh(GridKind::Flat).corners },
                OldGrid { kind: OldKind::Perspective, corners: PerspectiveGrid::default().corners },
            ],
            active: 1,
            used: 3,
        }];
        eframe::set_value(&mut storage, KEY, &written);
        let back: Vec<GridFile> = eframe::get_value(&storage, KEY).expect("reads");
        let kinds: Vec<_> = back[0].grids.iter().map(|g| g.kind).collect();
        assert_eq!(kinds, [GridKind::Flat, GridKind::Perspective]);
        assert_eq!(back[0].active, 1);
    }
}
