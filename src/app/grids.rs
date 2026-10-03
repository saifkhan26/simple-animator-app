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
use crate::tools::perspective::{
    self, Follow, GridGrab, GridKind, PerspectiveConfig, PerspectiveGrid, Space, P,
};
use crate::undo;

/// One project file's grids, as the app keeps them between sessions.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GridFile {
    pub path: PathBuf,
    pub grids: Vec<PerspectiveGrid>,
    pub active: usize,
    /// Seconds since the Unix epoch it was last open. Past
    /// [`MAX_GRID_FILES`] the stalest are forgotten first.
    pub used: u64,
}

/// Most files whose grids the app remembers.
const MAX_GRID_FILES: usize = 256;

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
            let Some(grab) = perspective::grab_at(&corners, &extra_at, p, tol, eye_level) else {
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
    pub fn perspective_move(&mut self, p: P) {
        let Some(d) = self.grid_drag else {
            return;
        };
        let s = d.space;
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
        let snap = self.shift_held;
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
        let rigid = g.kind.rigid();
        let Some(c) = perspective::dragged(&d.corners, d.grab, d.start, p, snap, rigid) else {
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
