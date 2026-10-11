//! The camera — which slab of document space each exported frame shows.
//!
//! The camera's frame rect is the output resolution. It is the size of the
//! canvas (`Project::width`/`height`) unless the camera is given a size of its
//! own — see [`CameraOpts::size`] and `Project::output_size`. Layers, however,
//! may sit anywhere in document space (see `Layer::transform`), so drawings can
//! live entirely off-frame and the camera pans/zooms/rolls over them across
//! the timeline.
//!
//! The identity camera with no size of its own puts the frame rect exactly on
//! the document rect, which is the pre-camera behaviour — so existing projects
//! export unchanged.

use crate::doc::transform::Transform;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    /// Offset of the frame center from the document center, in document pixels.
    pub tx: f32,
    pub ty: f32,
    /// How far in the camera is pushed. `2.0` = the frame covers half as much
    /// document as it does at rest (and the export upscales to compensate).
    pub zoom: f32,
    /// Camera roll in radians.
    pub rot: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            zoom: 1.0,
            rot: 0.0,
        }
    }
}

/// `Ease` now lives with the transform keys it is also used by. Re-exported
/// here so `crate::doc::camera::Ease` keeps resolving.
pub use crate::doc::transform::Ease;

/// A camera keyframe. `ease` shapes the segment running from this key to the
/// *next* one; it is ignored on the last key.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct CameraKey {
    pub frame: usize,
    pub camera: Camera,
    #[serde(default)]
    pub ease: Ease,
}

/// What the drawings do while a camera move that spans several passes of
/// them is still going — see [`CameraOpts::loops`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fill {
    /// Start again from the first frame each pass: a cycle.
    #[default]
    Loop,
    /// Forward, then back, then forward again.
    PingPong,
    /// Play once and stay on the last frame.
    Hold,
}

impl Fill {
    pub const ALL: [Fill; 3] = [Fill::Loop, Fill::PingPong, Fill::Hold];

    pub fn label(self) -> &'static str {
        match self {
            Fill::Loop => "Loop",
            Fill::PingPong => "Ping-pong",
            Fill::Hold => "Hold last frame",
        }
    }
}

/// Camera settings that last for the session. Not written to `.anim`, so a
/// project always opens with the defaults — and the defaults are the camera
/// as it has always been.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraOpts {
    /// Size of the camera's frame, which is the size of every exported
    /// frame. `None` = the canvas size, the frame a resting camera has always
    /// had. Smaller than the canvas, it is a window to pan around it.
    pub size: Option<(u32, u32)>,
    /// Hold the whole frame on the canvas: pushed in and moved over by the
    /// least that takes, so no frame — keyed, tweened or dragged — shows
    /// past the canvas's edge. See [`Camera::kept_inside`].
    pub keep_inside: bool,
    /// Curve through the keys instead of turning a corner at each: see
    /// [`Camera::resolve_at`]. Changes nothing until there are three keys.
    pub smooth: bool,
    /// How many passes of the animation the camera's move lasts. `1` = the
    /// two run together, frame for frame. More, and the move keyed along the
    /// timeline is stretched over that many passes of it: the drawings come
    /// round again (as `fill` has them) under one unbroken move, in playback
    /// and in exports. See `crate::io::frame_order::Shot`.
    pub loops: u32,
    pub fill: Fill,
}

impl Default for CameraOpts {
    fn default() -> Self {
        Self {
            size: None,
            keep_inside: false,
            smooth: false,
            loops: 1,
            fill: Fill::Loop,
        }
    }
}

/// Which part of the camera's frame a press landed on. The same grabs, by
/// the same rules, as a selection's transform box
/// (`crate::tools::selection::grab_at`) — less its one-axis handles, which a
/// frame of a fixed shape has no use for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CamGrab {
    /// Inside the frame: the drag moves the camera.
    Move,
    /// A corner: dragging it away from the centre widens the frame.
    Zoom,
    /// Just outside a corner: the drag rolls the camera about its centre.
    Roll,
}

impl Camera {
    /// How far the camera pulls out and pushes in.
    pub const ZOOM_MIN: f32 = 0.05;
    pub const ZOOM_MAX: f32 = 64.0;

    pub fn is_identity(&self) -> bool {
        self.tx == 0.0 && self.ty == 0.0 && (self.zoom - 1.0).abs() < 1e-6 && self.rot == 0.0
    }

    /// Interpolate between two cameras. Translation and roll move linearly;
    /// zoom moves *geometrically*, so a 1 → 4 push reads as a constant rate
    /// rather than racing at the start and crawling at the end.
    pub fn lerp(a: Camera, b: Camera, t: f32) -> Camera {
        let l = |x: f32, y: f32| x + (y - x) * t;
        let (za, zb) = (a.zoom.max(1e-6), b.zoom.max(1e-6));
        Camera {
            tx: l(a.tx, b.tx),
            ty: l(a.ty, b.ty),
            zoom: za * (zb / za).powf(t),
            rot: l(a.rot, b.rot),
        }
    }

    /// The camera shown on `frame`: `static_cam` when there are no keys,
    /// otherwise the eased interpolation of the keys (held flat before the
    /// first and after the last key). Mirrors `Layer::resolve_transform`.
    pub fn resolve(keys: &[CameraKey], static_cam: Camera, frame: usize) -> Camera {
        if keys.is_empty() {
            return static_cam;
        }
        if frame <= keys[0].frame {
            return keys[0].camera;
        }
        let last = keys.len() - 1;
        if frame >= keys[last].frame {
            return keys[last].camera;
        }
        for w in keys.windows(2) {
            let (a, b) = (w[0], w[1]);
            if frame >= a.frame && frame <= b.frame {
                let span = (b.frame - a.frame).max(1) as f32;
                let t = a.ease.apply((frame - a.frame) as f32 / span);
                return Camera::lerp(a.camera, b.camera, t);
            }
        }
        keys[last].camera
    }

    /// [`Camera::resolve`] at any time `t` in frames, not only on one — and
    /// on a whole frame, exactly what `resolve` gives there.
    ///
    /// With `smooth` and three keys or more, the frame's centre runs along a
    /// curve through the keys rather than a straight line from each to the
    /// next: a cubic Hermite segment between each pair, its tangent at a key
    /// the slope between that key's neighbours, so the path arrives at a
    /// middle key already heading for the one after. Zoom and roll, and each
    /// key's ease, are as ever. Two keys have nothing to curve round.
    pub fn resolve_at(keys: &[CameraKey], static_cam: Camera, t: f32, smooth: bool) -> Camera {
        if keys.is_empty() {
            return static_cam;
        }
        let last = keys.len() - 1;
        if t <= keys[0].frame as f32 {
            return keys[0].camera;
        }
        if t >= keys[last].frame as f32 {
            return keys[last].camera;
        }
        let i = keys
            .windows(2)
            .position(|w| t <= w[1].frame as f32)
            .unwrap_or(last - 1);
        let (a, b) = (keys[i], keys[i + 1]);
        let span = (b.frame - a.frame).max(1) as f32;
        let u = a.ease.apply((t - a.frame as f32) / span);
        let mut cam = Camera::lerp(a.camera, b.camera, u);
        if smooth && keys.len() >= 3 {
            // Slope of the path at key `k`, in pixels a frame: between its
            // neighbours, or along its one segment at either end.
            let slope = |k: usize| {
                let (p, n) = (keys[k.saturating_sub(1)], keys[(k + 1).min(last)]);
                let frames = n.frame.saturating_sub(p.frame).max(1) as f32;
                ((n.camera.tx - p.camera.tx) / frames, (n.camera.ty - p.camera.ty) / frames)
            };
            let (ma, mb) = (slope(i), slope(i + 1));
            let (u2, u3) = (u * u, u * u * u);
            let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
            let h10 = u3 - 2.0 * u2 + u;
            let h01 = -2.0 * u3 + 3.0 * u2;
            let h11 = u3 - u2;
            cam.tx = h00 * a.camera.tx + h10 * span * ma.0 + h01 * b.camera.tx + h11 * span * mb.0;
            cam.ty = h00 * a.camera.ty + h10 * span * ma.1 + h01 * b.camera.ty + h11 * span * mb.1;
        }
        cam
    }

    /// Fold this camera into a layer transform, producing the cell → *frame*
    /// transform to hand to `composite_layer`.
    ///
    /// Both the layer transform and the camera are similarity transforms
    /// (translate + uniform scale + rotate), and similarities compose into
    /// another similarity — so the camera costs nothing at composite time.
    ///
    /// cell → doc is `D = doc_c + t_L + R_L·s_L·(p − cell_c)`, and the frame
    /// rect sits in doc space as `D = doc_c + t_C + R_C·(1/z)·(f − frame_c)`.
    /// Eliminating `D` gives
    /// `f = frame_c + z·R₋ᵣ·(t_L − t_C) + z·s_L·R_(rot_L − rot_C)·(p − cell_c)`,
    /// which is exactly the `Transform` built below.
    pub fn apply(&self, l: &Transform) -> Transform {
        let z = self.zoom.max(1e-6);
        let (s, c) = (-self.rot).sin_cos();
        let (dx, dy) = (l.tx - self.tx, l.ty - self.ty);
        Transform {
            tx: z * (dx * c - dy * s),
            ty: z * (dx * s + dy * c),
            scale: l.scale * z,
            rot: l.rot - self.rot,
        }
    }

    /// Half the size of the box the frame takes up in the document at zoom
    /// 1: a frame `out`, rolled as this camera is.
    fn rolled_half(&self, out: (f32, f32)) -> (f32, f32) {
        let (s, c) = self.rot.sin_cos();
        let (s, c) = (s.abs(), c.abs());
        ((c * out.0 + s * out.1) * 0.5, (s * out.0 + c * out.1) * 0.5)
    }

    /// The least zoom at which a frame `out`, rolled as this camera is, lies
    /// wholly on a canvas `canvas` — the frame as big as it can be and still
    /// show nothing but canvas.
    pub fn fill_zoom(&self, canvas: (f32, f32), out: (f32, f32)) -> f32 {
        let (hx, hy) = self.rolled_half(out);
        (2.0 * hx / canvas.0.max(1e-6)).max(2.0 * hy / canvas.1.max(1e-6))
    }

    /// The greatest zoom at which a frame `out`, rolled as this camera is,
    /// takes in the whole of a canvas `canvas` when centred on it.
    pub fn fit_zoom(&self, canvas: (f32, f32), out: (f32, f32)) -> f32 {
        // The canvas inside the rolled frame is the canvas rolled the other
        // way inside a level one.
        let (hx, hy) = self.rolled_half(canvas);
        (out.0 / (2.0 * hx).max(1e-6)).min(out.1 / (2.0 * hy).max(1e-6))
    }

    /// This camera with its whole frame on the canvas: pushed in until the
    /// frame fits, then moved over until none of it hangs off an edge. A
    /// rolled frame is held by the box around it. A camera already inside
    /// comes back exactly as it was.
    pub fn kept_inside(self, canvas: (f32, f32), out: (f32, f32)) -> Camera {
        let zoom = self.zoom.max(self.fill_zoom(canvas, out));
        let (hx, hy) = self.rolled_half(out);
        let room_x = (canvas.0 * 0.5 - hx / zoom).max(0.0);
        let room_y = (canvas.1 * 0.5 - hy / zoom).max(0.0);
        Camera {
            tx: self.tx.clamp(-room_x, room_x),
            ty: self.ty.clamp(-room_y, room_y),
            zoom,
            rot: self.rot,
        }
    }

    /// The middle of the frame, in the document of a canvas `canvas`.
    fn centre(&self, canvas: (f32, f32)) -> (f32, f32) {
        (canvas.0 * 0.5 + self.tx, canvas.1 * 0.5 + self.ty)
    }

    /// What a press at document point `p` grabs of a frame `out`, if
    /// anything. `tol` is a handle's half-size in document pixels, so the
    /// handles are a constant size on screen at any view zoom.
    ///
    /// Tested in the frame's own space, where it is a plain rect whatever
    /// the roll: a corner first, then the ring diagonally outside one, then
    /// the inside.
    pub fn grab_at(&self, p: (f32, f32), canvas: (f32, f32), out: (f32, f32), tol: f32) -> Option<CamGrab> {
        let z = self.zoom.max(1e-6);
        let (cx, cy) = self.centre(canvas);
        let (dx, dy) = (p.0 - cx, p.1 - cy);
        let (s, c) = self.rot.sin_cos();
        // Distance from the centre along each of the frame's axes, in its
        // own (exported) pixels.
        let (u, v) = (((dx * c + dy * s) * z).abs(), ((dy * c - dx * s) * z).abs());
        let (hw, hh) = (out.0 * 0.5, out.1 * 0.5);
        // Capped so the corners of a small frame cannot swallow its inside.
        let (tu, tv) = ((tol * z).min(out.0 * 0.2), (tol * z).min(out.1 * 0.2));

        if (u - hw).abs() <= tu && (v - hh).abs() <= tv {
            return Some(CamGrab::Zoom);
        }
        if u > hw && v > hh && u - hw <= tu * 3.0 && v - hh <= tv * 3.0 {
            return Some(CamGrab::Roll);
        }
        (u <= hw && v <= hh).then_some(CamGrab::Move)
    }

    /// This camera after its frame is dragged by `grab` from document point
    /// `from` to `to`: moved by as much, or zoomed or rolled about its
    /// centre so the grabbed corner follows the cursor.
    pub fn dragged(self, grab: CamGrab, from: (f32, f32), to: (f32, f32), canvas: (f32, f32)) -> Camera {
        let (cx, cy) = self.centre(canvas);
        let (ax, ay) = (from.0 - cx, from.1 - cy);
        let (bx, by) = (to.0 - cx, to.1 - cy);
        let mut cam = self;
        match grab {
            CamGrab::Move => {
                cam.tx += to.0 - from.0;
                cam.ty += to.1 - from.1;
            }
            CamGrab::Zoom => {
                // The frame covers 1/zoom of the document: a corner twice
                // as far out is half the zoom.
                let (was, now) = (ax.hypot(ay), bx.hypot(by));
                if was > 1e-3 && now > 1e-3 {
                    cam.zoom = (self.zoom * was / now).clamp(Self::ZOOM_MIN, Self::ZOOM_MAX);
                }
            }
            CamGrab::Roll => {
                if ax.hypot(ay) > 1e-3 && bx.hypot(by) > 1e-3 {
                    cam.rot += by.atan2(bx) - ay.atan2(ax);
                }
            }
        }
        cam
    }

    /// The camera's frame rect expressed as a transform placing a "cell" the
    /// size of the frame into document space. Lets the editor reuse the
    /// cell-corner helpers to draw the camera guide.
    pub fn as_frame_transform(&self) -> Transform {
        Transform {
            tx: self.tx,
            ty: self.ty,
            scale: 1.0 / self.zoom.max(1e-6),
            rot: self.rot,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }

    /// The identity camera must leave layer transforms untouched, so existing
    /// projects composite through the exact same path as before.
    #[test]
    fn identity_camera_is_a_noop() {
        let l = Transform {
            tx: 12.0,
            ty: -30.0,
            scale: 0.75,
            rot: 0.4,
        };
        assert_eq!(Camera::default().apply(&l), l);
    }

    /// A pure pan shifts every layer by exactly minus the camera offset.
    #[test]
    fn pan_shifts_layers_opposite() {
        let cam = Camera {
            tx: 200.0,
            ty: -50.0,
            ..Default::default()
        };
        let out = cam.apply(&Transform::default());
        approx(out.tx, -200.0);
        approx(out.ty, 50.0);
        approx(out.scale, 1.0);
    }

    /// `apply` and `as_frame_transform` must describe the same camera: mapping
    /// a frame corner to doc space and back through `apply` returns it.
    #[test]
    fn apply_agrees_with_frame_transform() {
        let (pw, ph) = (320.0f32, 200.0f32);
        let cam = Camera {
            tx: 90.0,
            ty: -40.0,
            zoom: 1.8,
            rot: 0.6,
        };
        // A layer whose cell is frame-sized and identity-placed.
        let eff = cam.apply(&Transform::default());
        let frame_t = cam.as_frame_transform();
        for (u, v) in [(0.0, 0.0), (pw, 0.0), (pw, ph), (37.0, 91.0)] {
            // Where does frame pixel (u,v) live in doc space?
            let (dx, dy) = frame_t.cell_to_doc(u, v, pw, ph, pw, ph);
            // The identity layer's cell pixel at that doc point...
            let (cu, cv) = Transform::default().doc_to_cell(dx, dy, pw, ph, pw, ph);
            // ...must land back on (u,v) once folded through the camera.
            let (fu, fv) = eff.cell_to_doc(cu, cv, pw, ph, pw, ph);
            approx(fu, u);
            approx(fv, v);
        }
    }

    /// A frame of its own size is still placed by its centre: one of its
    /// pixels mapped into the document, onto a layer's cell and back through
    /// the fold lands where it started, with the frame's size standing where
    /// the canvas's would.
    #[test]
    fn a_frame_of_its_own_size_folds_about_its_centre() {
        let (pw, ph) = (320.0f32, 200.0f32);
        let (ow, oh) = (120.0f32, 90.0f32);
        let (cw, ch) = (200.0f32, 150.0f32);
        let cam = Camera {
            tx: 40.0,
            ty: -25.0,
            zoom: 1.6,
            rot: 0.5,
        };
        let layer = Transform {
            tx: 12.0,
            ty: 7.0,
            scale: 0.8,
            rot: -0.3,
        };
        let eff = cam.apply(&layer);
        let frame_t = cam.as_frame_transform();
        for (u, v) in [(0.0, 0.0), (ow, 0.0), (ow, oh), (37.0, 61.0)] {
            let (dx, dy) = frame_t.cell_to_doc(u, v, ow, oh, pw, ph);
            let (cu, cv) = layer.doc_to_cell(dx, dy, cw, ch, pw, ph);
            let (fu, fv) = eff.cell_to_doc(cu, cv, cw, ch, ow, oh);
            approx(fu, u);
            approx(fv, v);
        }
    }

    const CANVAS: (f32, f32) = (400.0, 200.0);
    const FRAME: (f32, f32) = (100.0, 50.0);

    /// A camera already showing nothing but canvas is not touched — to the
    /// bit, so turning the hold on changes no frame that didn't need it.
    #[test]
    fn a_camera_inside_the_canvas_is_left_exactly_as_it_was() {
        for cam in [
            Camera::default(),
            Camera {
                tx: 150.0,
                ty: -75.0,
                zoom: 1.0,
                rot: 0.0,
            },
            Camera {
                tx: 31.7,
                ty: 12.3,
                zoom: 2.4,
                rot: 0.7,
            },
        ] {
            assert_eq!(cam.kept_inside(CANVAS, FRAME), cam);
        }
    }

    #[test]
    fn a_camera_past_the_edge_is_moved_back_onto_the_canvas() {
        let cam = Camera {
            tx: 900.0,
            ty: -900.0,
            ..Default::default()
        };
        let held = cam.kept_inside(CANVAS, FRAME);
        // Half the canvas less half the frame.
        approx(held.tx, 150.0);
        approx(held.ty, -75.0);
        approx(held.zoom, 1.0);
    }

    #[test]
    fn a_camera_pulled_out_too_far_is_pushed_in_until_it_fits() {
        let cam = Camera {
            zoom: 0.1,
            tx: 40.0,
            ..Default::default()
        };
        let held = cam.kept_inside(CANVAS, FRAME);
        // 100 wide in 400 and 50 tall in 200: both bind at a quarter.
        approx(held.zoom, 0.25);
        approx(held.tx, 0.0);
        approx(held.ty, 0.0);
        approx(cam.fill_zoom(CANVAS, FRAME), 0.25);
    }

    /// A rolled frame is held by the box around it: on its side, a 100×50
    /// frame is 50 wide and 100 tall.
    #[test]
    fn a_rolled_frame_is_held_by_the_box_around_it() {
        let cam = Camera {
            tx: 900.0,
            ty: 900.0,
            rot: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        let held = cam.kept_inside(CANVAS, FRAME);
        approx(held.tx, 200.0 - 25.0);
        approx(held.ty, 100.0 - 50.0);
        approx(held.rot, cam.rot);
    }

    /// Fit takes in the whole canvas; fill shows nothing but canvas. On a
    /// frame the canvas's shape they are the same zoom.
    #[test]
    fn fit_takes_in_the_canvas_and_fill_stays_on_it() {
        let cam = Camera::default();
        approx(cam.fit_zoom(CANVAS, FRAME), 0.25);
        // A square frame on the 2:1 canvas: fit by its width, fill by its height.
        approx(cam.fit_zoom(CANVAS, (100.0, 100.0)), 0.25);
        approx(cam.fill_zoom(CANVAS, (100.0, 100.0)), 0.5);
    }

    /// A level 100×50 frame at zoom 1 in the middle of the 400×200 canvas
    /// covers x 150..250, y 75..125.
    #[test]
    fn a_press_grabs_the_inside_a_corner_or_the_ring_past_one() {
        let cam = Camera::default();
        let at = |x, y| cam.grab_at((x, y), CANVAS, FRAME, 4.0);
        assert_eq!(at(200.0, 100.0), Some(CamGrab::Move));
        assert_eq!(at(160.0, 120.0), Some(CamGrab::Move));
        assert_eq!(at(150.0, 75.0), Some(CamGrab::Zoom));
        assert_eq!(at(252.0, 127.0), Some(CamGrab::Zoom), "a corner reaches a little past itself");
        assert_eq!(at(258.0, 133.0), Some(CamGrab::Roll));
        assert_eq!(at(142.0, 67.0), Some(CamGrab::Roll));
        assert_eq!(at(258.0, 100.0), None, "beside an edge is not the frame");
        assert_eq!(at(300.0, 180.0), None);
    }

    /// The handles are found where the frame is drawn, however it is moved,
    /// pushed in and rolled.
    #[test]
    fn grabs_follow_a_moved_zoomed_and_rolled_frame() {
        let cam = Camera {
            tx: 60.0,
            ty: -20.0,
            zoom: 2.0,
            rot: 0.6,
        };
        let t = cam.as_frame_transform();
        let doc = |u: f32, v: f32| t.cell_to_doc(u, v, FRAME.0, FRAME.1, CANVAS.0, CANVAS.1);
        let at = |p| cam.grab_at(p, CANVAS, FRAME, 2.0);
        assert_eq!(at(doc(50.0, 25.0)), Some(CamGrab::Move));
        for corner in [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)] {
            assert_eq!(at(doc(corner.0, corner.1)), Some(CamGrab::Zoom), "{corner:?}");
        }
        assert_eq!(at(doc(-8.0, -8.0)), Some(CamGrab::Roll));
        assert_eq!(at(doc(-40.0, 25.0)), None);
    }

    #[test]
    fn dragging_moves_zooms_and_rolls_about_the_centre() {
        let cam = Camera {
            tx: 10.0,
            ty: 5.0,
            zoom: 2.0,
            rot: 0.3,
        };
        let moved = cam.dragged(CamGrab::Move, (100.0, 100.0), (130.0, 80.0), CANVAS);
        approx(moved.tx, 40.0);
        approx(moved.ty, -15.0);
        assert_eq!((moved.zoom, moved.rot), (cam.zoom, cam.rot));

        // The centre is at (210, 105). A corner pulled twice as far from it
        // doubles the frame: half the zoom.
        let wider = cam.dragged(CamGrab::Zoom, (220.0, 105.0), (230.0, 105.0), CANVAS);
        approx(wider.zoom, 1.0);
        assert_eq!((wider.tx, wider.ty, wider.rot), (cam.tx, cam.ty, cam.rot));

        // A quarter turn round the centre.
        let rolled = cam.dragged(CamGrab::Roll, (260.0, 105.0), (210.0, 155.0), CANVAS);
        approx(rolled.rot, 0.3 + std::f32::consts::FRAC_PI_2);
        assert_eq!((rolled.tx, rolled.ty, rolled.zoom), (cam.tx, cam.ty, cam.zoom));
    }

    /// The corner under the cursor stays under it: after a zoom drag the
    /// grabbed corner is where the cursor went.
    #[test]
    fn a_dragged_corner_follows_the_cursor() {
        let cam = Camera {
            tx: -30.0,
            ty: 12.0,
            zoom: 1.5,
            rot: 0.0,
        };
        let corner = |c: &Camera| {
            c.as_frame_transform()
                .cell_to_doc(FRAME.0, FRAME.1, FRAME.0, FRAME.1, CANVAS.0, CANVAS.1)
        };
        let from = corner(&cam);
        let (cx, cy) = (CANVAS.0 * 0.5 + cam.tx, CANVAS.1 * 0.5 + cam.ty);
        // Straight out along the diagonal, half as far again.
        let to = (cx + (from.0 - cx) * 1.5, cy + (from.1 - cy) * 1.5);
        let after = cam.dragged(CamGrab::Zoom, from, to, CANVAS);
        let now = corner(&after);
        approx(now.0, to.0);
        approx(now.1, to.1);
    }

    fn key(frame: usize, tx: f32, ty: f32, ease: Ease) -> CameraKey {
        CameraKey {
            frame,
            camera: Camera {
                tx,
                ty,
                ..Default::default()
            },
            ease,
        }
    }

    /// The straight path at a time between frames is, on every whole frame,
    /// bit for bit what `resolve` has always given there.
    #[test]
    fn resolving_at_a_whole_frame_is_resolve() {
        let mut seed = 0x2545_f491u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for _ in 0..200 {
            let n = next() % 6;
            let mut frame = 0usize;
            let mut keys = Vec::new();
            for _ in 0..n {
                frame += 1 + (next() % 9) as usize;
                keys.push(CameraKey {
                    frame,
                    camera: Camera {
                        tx: (next() % 2000) as f32 * 0.37 - 300.0,
                        ty: (next() % 2000) as f32 * 0.11 - 90.0,
                        zoom: 0.2 + (next() % 500) as f32 * 0.013,
                        rot: (next() % 600) as f32 * 0.01 - 3.0,
                    },
                    ease: Ease::ALL[(next() % 4) as usize],
                });
            }
            let still = Camera {
                tx: 3.0,
                ty: -4.0,
                zoom: 1.5,
                rot: 0.25,
            };
            for f in 0..frame + 4 {
                assert_eq!(
                    Camera::resolve_at(&keys, still, f as f32, false),
                    Camera::resolve(&keys, still, f),
                    "frame {f} of {keys:?}"
                );
            }
        }
    }

    /// Between frames the straight path is the straight line it is on them.
    #[test]
    fn resolving_between_frames_follows_the_segment() {
        let keys = [key(0, 0.0, 0.0, Ease::Linear), key(10, 100.0, -50.0, Ease::Linear)];
        let cam = Camera::resolve_at(&keys, Camera::default(), 2.5, false);
        approx(cam.tx, 25.0);
        approx(cam.ty, -12.5);
        approx(Camera::resolve_at(&keys, Camera::default(), 99.0, false).tx, 100.0);
        approx(Camera::resolve_at(&keys, Camera::default(), -3.0, false).tx, 0.0);
    }

    /// Right, then down: three keys with a square corner on the middle one.
    fn corner() -> [CameraKey; 3] {
        [
            key(0, 0.0, 0.0, Ease::Linear),
            key(10, 100.0, 0.0, Ease::Linear),
            key(20, 100.0, 100.0, Ease::Linear),
        ]
    }

    /// The smooth path still goes exactly through every key.
    #[test]
    fn a_smooth_path_passes_through_its_keys() {
        let keys = corner();
        for k in keys {
            let cam = Camera::resolve_at(&keys, Camera::default(), k.frame as f32, true);
            assert_eq!((cam.tx, cam.ty), (k.camera.tx, k.camera.ty), "key on {}", k.frame);
        }
    }

    /// The straight path turns the corner on the middle key in one frame; the
    /// smooth one is already turning as it arrives, and carries on the same
    /// way as it leaves.
    #[test]
    fn a_smooth_path_rounds_the_corner_a_straight_one_turns() {
        let keys = corner();
        let at = |t: f32, smooth| {
            let c = Camera::resolve_at(&keys, Camera::default(), t, smooth);
            (c.tx, c.ty)
        };
        // Heading just before and just after the middle key.
        let heading = |smooth| {
            let (a, k, b) = (at(9.9, smooth), at(10.0, smooth), at(10.1, smooth));
            ((k.1 - a.1).atan2(k.0 - a.0), (b.1 - k.1).atan2(b.0 - k.0))
        };
        let (into, out_of) = heading(false);
        approx(into, 0.0);
        approx(out_of, std::f32::consts::FRAC_PI_2);
        let (into, out_of) = heading(true);
        assert!((into - out_of).abs() < 0.05, "{into} then {out_of}");
        assert!((into - std::f32::consts::FRAC_PI_4).abs() < 0.05, "diagonally through it: {into}");

        // It swings wide to do it: off the straight line on the way in.
        assert_eq!(at(5.0, false).1, 0.0);
        approx(at(5.0, true).0, 56.25);
        approx(at(5.0, true).1, -6.25);
    }

    /// Two keys have nothing to curve round: smooth or not, the same camera,
    /// to the bit. Nor does a camera with no keys, or a time outside them.
    #[test]
    fn smoothing_changes_nothing_with_fewer_than_three_keys() {
        let keys = [key(3, -40.0, 12.0, Ease::Both), key(17, 260.0, -75.0, Ease::In)];
        let still = Camera::default();
        for i in 0..=80 {
            let t = i as f32 * 0.25;
            assert_eq!(Camera::resolve_at(&keys, still, t, true), Camera::resolve_at(&keys, still, t, false));
        }
        assert_eq!(Camera::resolve_at(&[], still, 5.0, true), still);
        let three = corner();
        assert_eq!(Camera::resolve_at(&three, still, 40.0, true), three[2].camera);
    }

    /// Keys hold flat outside their range and ease within a segment.
    #[test]
    fn resolve_holds_and_eases() {
        let keys = vec![
            CameraKey {
                frame: 10,
                camera: Camera {
                    tx: 0.0,
                    ..Default::default()
                },
                ease: Ease::Both,
            },
            CameraKey {
                frame: 20,
                camera: Camera {
                    tx: 100.0,
                    ..Default::default()
                },
                ease: Ease::Linear,
            },
        ];
        let st = Camera::default();
        approx(Camera::resolve(&keys, st, 0).tx, 0.0);
        approx(Camera::resolve(&keys, st, 30).tx, 100.0);
        // Midpoint is the same for smoothstep, but the quarter point is slower.
        approx(Camera::resolve(&keys, st, 15).tx, 50.0);
        let q = Camera::resolve(&keys, st, 12).tx;
        assert!(q < 20.0, "smoothstep should lag a linear 20.0, got {q}");
    }

    /// Zoom interpolates geometrically: the halfway point of 1 → 4 is 2, not
    /// 2.5.
    #[test]
    fn zoom_is_geometric() {
        let a = Camera {
            zoom: 1.0,
            ..Default::default()
        };
        let b = Camera {
            zoom: 4.0,
            ..Default::default()
        };
        approx(Camera::lerp(a, b, 0.5).zoom, 2.0);
    }
}
