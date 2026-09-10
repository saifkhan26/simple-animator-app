//! Stroke construction: input smoothing, cubic Bezier curve fitting, spine
//! node emission, ribbon capsule rasterization with per-stroke coverage
//! compositing (see `tools::ribbon`).
//!
//! The model is Krita's, ported from `kis_tool_freehand_helper.cpp`,
//! `kis_paintop.cc` and `kis_distance_information.cpp`:
//!
//! 1. Optionally smooth the incoming sample (gaussian over the recent
//!    history, keyed to distance). Krita's default does *not* do this.
//! 2. Fit a cubic Bezier through consecutive samples using the local tangents
//!    on either side, which is where the clean line quality actually comes
//!    from.
//! 3. Subdivide that Bezier until it is flat to within half a pixel, then walk
//!    the result at a fixed spacing, emitting one spine node per step.
//!
//! Step 3 replaces a fixed step count keyed to brush radius, which
//! tessellated *coarser* as the brush got bigger and was a direct source of
//! faceting on large brushes.
//!
//! The CPU rasterizer lives in `ribbon.rs`; a later wgpu compute port
//! replaces that module's internals while this input model stays the same.

use crate::doc::canvas::{Canvas, DirtyRect};
use crate::input::pointer::PointerSample;
use crate::tools::dab::Dab;
use crate::tools::ribbon::{union_rect, SpineNode, StrokeWorkspace};
use crate::tools::{ActiveTool, BrushMode, BrushSettings, Smoothing, SmoothingOptions};

/// A control point closer than this to the chord means the Bezier is flat
/// enough to draw as a line. Krita's `BEZIER_FLATNESS_THRESHOLD`.
const BEZIER_FLATNESS: f32 = 0.5;

/// Recursion cap for the subdivision above. The flatness test terminates on
/// its own for any sane curve; this is here so a NaN or a wildly out-of-range
/// control point cannot blow the stack.
const MAX_BEZIER_DEPTH: u32 = 24;

/// Floor on node spacing, in canvas pixels. Krita's `MIN_DISTANCE_SPACING`.
const MIN_SPACING: f32 = 0.5;

/// Distance between samples, in screen pixels, at which the drawing-speed
/// term saturates. See [`StrokeBuilder::drawing_speed`].
const SPEED_REF: f32 = 30.0;

/// Builds up a stroke as the pointer moves and rasterizes it incrementally.
pub struct StrokeBuilder {
    pub brush: BrushSettings,
    pub tool: ActiveTool,
    opts: SmoothingOptions,
    /// Smoothed input samples for the whole stroke. Doubles as the weighted
    /// filter's history.
    samples: Vec<PointerSample>,
    /// Distance from the preceding sample, parallel to `samples`. The
    /// weighted filter walks distance, not time.
    distance_history: Vec<f32>,
    /// Committed spine nodes — append-only. Under `max()` coverage combining
    /// already-rasterized geometry can never be retracted, so nodes are only
    /// emitted for curve segments whose shape is final.
    spine: Vec<SpineNode>,
    /// Stamps, in `Dab` mode. Parallel to `spine`, which that mode keeps
    /// only so the live-tail overlay has something to draw.
    dabs: Vec<Dab>,
    /// First spine (or dab) index not yet rasterized.
    raster_from: usize,
    /// The sample before the current one, and the one before that. Krita's
    /// `previousPaintInformation` / `olderPaintInformation`: the curve fit
    /// needs a point on each side, so painting always trails input by one
    /// sample. The gap is covered by the UI's live-tail overlay.
    previous: Option<PointerSample>,
    older: Option<PointerSample>,
    have_tangent: bool,
    previous_tangent: (f32, f32),
    /// Distance walked since the last spine node, carried across curve
    /// segments so spacing is uniform along the whole stroke.
    spacing_accum: f32,
    /// Distance between emitted points, in canvas pixels. Fixed for a
    /// ribbon; in `Dab` mode it is recomputed after every stamp, because
    /// spacing is a fraction of a dab whose size follows pressure and tilt.
    spacing: f32,
    /// The pen-down node is in `spine` but has not been rasterized yet.
    dot_pending: bool,
    /// Latest smoothed position and pressure, for the live-tail overlay.
    smooth_pos: Option<(f32, f32)>,
    smooth_pressure: Option<f32>,
    /// Screen pixels per canvas pixel at the time the stroke started. Samples
    /// arrive already mapped into canvas space, so this is what lets the
    /// filter specify its width in the units the hand and the device actually
    /// work in. Latched per stroke — see `AppState::cell_view_scale`.
    view_scale: f32,
}

impl StrokeBuilder {
    pub fn new(
        brush: BrushSettings,
        tool: ActiveTool,
        view_scale: f32,
        opts: SmoothingOptions,
    ) -> Self {
        let spacing = match brush.mode {
            // Fine enough that the capsule polyline reads as a curve.
            BrushMode::Ribbon => (brush.radius * 0.3).max(1.0),
            // A starting guess at full size; the first stamp replaces it.
            BrushMode::Dab => (brush.radius * 2.0 * brush.spacing).max(0.5),
        };
        Self {
            brush,
            tool,
            opts,
            view_scale: view_scale.max(1e-6),
            samples: Vec::with_capacity(256),
            distance_history: Vec::with_capacity(256),
            spine: Vec::with_capacity(256),
            dabs: Vec::new(),
            raster_from: 0,
            previous: None,
            older: None,
            have_tangent: false,
            previous_tangent: (0.0, 0.0),
            spacing_accum: 0.0,
            spacing,
            dot_pending: false,
            smooth_pos: None,
            smooth_pressure: None,
        }
    }

    /// Push a pointer sample: smooth it, then extend the stroke.
    ///
    /// Painting trails input by one sample, because the Bezier through
    /// `older -> previous` is only determined once the sample after
    /// `previous` has arrived.
    pub fn push(&mut self, s: PointerSample) {
        // Drop duplicate positions (egui emits zero-delta moves, and a
        // stationary pen still reports pressure changes). Expressed in screen
        // pixels so it means the same thing at every zoom.
        let dup = 0.05 / self.view_scale;
        if let Some(p) = self.previous {
            if (p.x - s.x).abs() < dup && (p.y - s.y).abs() < dup {
                return;
            }
        }

        let info = self.smooth(s);
        self.smooth_pos = Some((info.x, info.y));
        self.smooth_pressure = Some(info.pressure);

        let Some(previous) = self.previous else {
            // Pen-down: ink immediately, exactly where the user pressed.
            self.emit(&info);
            self.dot_pending = true;
            self.previous = Some(info);
            return;
        };

        match self.opts.kind {
            Smoothing::None => self.paint_line(previous, info),
            Smoothing::Basic | Smoothing::Weighted => {
                if !self.have_tangent {
                    self.have_tangent = true;
                    self.previous_tangent = tangent(previous, info);
                } else {
                    let older = self.older.unwrap_or(previous);
                    let new_tangent = tangent(older, info);
                    if is_null(new_tangent) || is_null(self.previous_tangent) {
                        self.paint_line(previous, info);
                    } else {
                        self.paint_bezier_segment(
                            older,
                            previous,
                            self.previous_tangent,
                            new_tangent,
                        );
                    }
                    self.previous_tangent = new_tangent;
                }
                self.older = Some(previous);
            }
        }

        self.previous = Some(info);
    }

    /// Rasterize newly committed stroke geometry and composite it into the
    /// canvas. Returns the pixel rect updated this call (for partial texture
    /// upload), or `None` if nothing changed.
    pub fn flush(
        &mut self,
        canvas: &mut Canvas,
        ws: &mut StrokeWorkspace,
        pre: &[u8],
    ) -> Option<DirtyRect> {
        let mut acc: Option<DirtyRect> = None;

        // Pen-down dot: instant ink with zero latency. A lone disc rather
        // than a capsule, so it is rasterized here rather than by `drain`.
        if self.dot_pending {
            self.dot_pending = false;
            let first = match self.brush.mode {
                BrushMode::Dab => self.dabs.first().and_then(|&d| ws.raster_dab(d)),
                BrushMode::Ribbon => self.spine.first().and_then(|&n| ws.raster_dot(n)),
            };
            self.raster_from = 1;
            if let Some(r) = first {
                acc = Some(union_rect(acc, r));
            }
        }

        acc = self.drain(ws, acc);
        self.composite(canvas, ws, pre, acc);
        acc
    }

    /// Paint the trailing segment and rasterize everything outstanding.
    /// Called on pointer-up.
    pub fn finish(
        &mut self,
        canvas: &mut Canvas,
        ws: &mut StrokeWorkspace,
        pre: &[u8],
    ) -> Option<DirtyRect> {
        let mut acc = self.flush(canvas, ws, pre);

        // Krita's `finishStroke`: the segment `older -> previous` has been
        // held back waiting for a sample that will never come, so close it
        // with a tangent derived from the two points in hand.
        if self.have_tangent {
            self.have_tangent = false;
            if let (Some(older), Some(previous)) = (self.older, self.previous) {
                let new_tangent = tangent(older, previous);
                self.paint_bezier_segment(older, previous, self.previous_tangent, new_tangent);
            }
        }

        // Land the spine exactly on the stroke's end point. The spacing walker
        // stops at the last whole step, which on a short stroke can be well
        // short of where the pen actually lifted.
        if let Some(end) = self.previous {
            let needs_end_node = self
                .spine
                .last()
                .map(|n| (n.x - end.x).abs() > 0.01 || (n.y - end.y).abs() > 0.01)
                .unwrap_or(false);
            if needs_end_node {
                self.emit(&end);
                self.spacing_accum = 0.0;
            }
        }

        acc = self.drain(ws, acc);
        self.composite(canvas, ws, pre, acc);
        acc
    }

    /// The uncommitted stroke tail: last committed spine node plus the
    /// current smoothed pointer position. Drawn by the UI as an overlay so
    /// the one-sample paint lag is invisible.
    pub fn live_tail(&self) -> Option<(SpineNode, (f32, f32))> {
        Some((*self.spine.last()?, self.smooth_pos?))
    }

    /// Radius / flow modulation at the current smoothed pressure, for the
    /// live-tail overlay.
    pub fn current_node(&self) -> Option<SpineNode> {
        let (x, y) = self.smooth_pos?;
        let s = PointerSample {
            x,
            y,
            pressure: self.smooth_pressure.unwrap_or(1.0),
            tilt_x: 0.0,
            tilt_y: 0.0,
            t: 0.0,
        };
        Some(self.node_at(&s))
    }

    // --- Smoothing ---------------------------------------------------------

    /// Krita's weighted smoothing, from `KisToolFreehandHelper::paint`.
    ///
    /// A gaussian average of the recent samples where the weight falls off
    /// with *distance travelled* rather than with sample count or elapsed
    /// time — time measurements from real devices are too unstable to key a
    /// filter on. `Basic` and `None` return the sample untouched, which is
    /// what Krita's own default does.
    fn smooth(&mut self, info: PointerSample) -> PointerSample {
        let weighted = self.opts.kind == Smoothing::Weighted
            && (self.opts.distance_min > 0.0 || self.opts.distance_max > 0.0);

        let prev_pos = self
            .samples
            .last()
            .map(|p| (p.x, p.y))
            .or_else(|| self.previous.map(|p| (p.x, p.y)));
        let travelled = match prev_pos {
            Some((px, py)) => (info.x - px).hypot(info.y - py),
            None => 0.0,
        };
        self.distance_history.push(travelled);
        self.samples.push(info);

        if !weighted || self.samples.len() <= 3 {
            return info;
        }

        // '3.0' for the (3 * sigma) range: the "distance" parameter names the
        // full width of the filter, not its standard deviation.
        let sigma = self.effective_smoothness(self.drawing_speed()) / 3.0;
        if sigma <= 0.0 {
            return info;
        }
        let gaussian_weight = 1.0 / ((2.0 * std::f32::consts::PI).sqrt() * sigma);
        let gaussian_weight2 = sigma * sigma;

        let mut distance_sum = 0.0f32;
        let mut scale_sum = 0.0f32;
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let mut pressure = 0.0f32;
        let mut base_rate = 0.0f32;

        let n = self.samples.len();
        for i in (0..n).rev() {
            let next = self.samples[i];
            let mut distance = self.distance_history[i];

            // A rising pressure at the head of a stroke drags the average
            // backwards and thins the tail. Inflating the distance for those
            // samples pushes them out of the filter instead.
            if i < n - 1 {
                let mut pressure_grad = next.pressure - self.samples[i + 1].pressure;
                if pressure_grad > 0.0 {
                    let tail = 40.0 * self.opts.tail_aggressiveness;
                    pressure_grad *= tail * (1.0 - next.pressure);
                    distance += pressure_grad * 3.0 * sigma;
                }
            }

            distance_sum += distance;
            let rate = gaussian_weight * (-distance_sum * distance_sum / (2.0 * gaussian_weight2)).exp();

            if n - i == 1 {
                base_rate = rate;
            } else if rate > 0.0 && base_rate / rate > 100.0 {
                // Everything older contributes less than a hundredth of the
                // newest sample. Bail rather than walk the whole stroke.
                break;
            }

            scale_sum += rate;
            x += rate * next.x;
            y += rate * next.y;
            if self.opts.smooth_pressure {
                pressure += rate * next.pressure;
            }
        }

        if scale_sum == 0.0 {
            return info;
        }
        x /= scale_sum;
        y /= scale_sum;
        if self.opts.smooth_pressure {
            pressure /= scale_sum;
        }

        // Krita's own guard against a degenerate average landing on the
        // origin, kept as written.
        if !((x != 0.0 && y != 0.0) || (x == info.x && y == info.y)) {
            return info;
        }
        let mut out = info;
        out.x = x;
        out.y = y;
        if self.opts.smooth_pressure {
            out.pressure = pressure.clamp(0.0, 1.0);
        }
        if let Some(last) = self.samples.last_mut() {
            *last = out;
        }
        out
    }

    /// Filter width for the current speed, in canvas pixels.
    ///
    /// With `scalable_distance` the configured width means *screen* pixels,
    /// so it is divided back out by the zoom. That is the whole trick behind
    /// a zoomed-out line staying as clean as a zoomed-in one: the same hand
    /// tremor covers more canvas pixels when zoomed out, and so must the
    /// filter.
    fn effective_smoothness(&self, speed: f32) -> f32 {
        let zoom_coeff = if self.opts.scalable_distance {
            1.0 / self.view_scale
        } else {
            1.0
        };
        zoom_coeff * ((1.0 - speed) * self.opts.distance_max + speed * self.opts.distance_min)
    }

    /// Stand-in for Krita's `KisPaintInformation::drawingSpeed()`, 0..1.
    ///
    /// Krita derives it from device timings; ours comes from the distance
    /// between the last two samples in screen pixels, saturating at
    /// `SPEED_REF`. With the default options `distance_min == distance_max`,
    /// so this term cancels out entirely and only matters once the two are
    /// dialled apart.
    fn drawing_speed(&self) -> f32 {
        let last = self.distance_history.last().copied().unwrap_or(0.0);
        ((last * self.view_scale) / SPEED_REF).clamp(0.0, 1.0)
    }

    // --- Curve fitting -----------------------------------------------------

    /// Fit a cubic Bezier between two samples given the tangent at each, and
    /// paint it. Ported from `KisToolFreehandHelper::paintBezierSegment`.
    fn paint_bezier_segment(
        &mut self,
        pi1: PointerSample,
        pi2: PointerSample,
        tangent1: (f32, f32),
        tangent2: (f32, f32),
    ) {
        if is_null(tangent1) || is_null(tangent2) {
            return;
        }
        const MAX_SANE_POINT: f32 = 1e6;

        let p1 = (pi1.x, pi1.y);
        let p2 = (pi2.x, pi2.y);
        // Where the control points want to go, before their length is decided.
        let dir1 = (p1.0 + tangent1.0, p1.1 + tangent1.1);
        let dir2 = (p2.0 - tangent2.0, p2.1 - tangent2.1);

        let (target1, target2) = if segments_cross(dir1, dir2, p1, p2) {
            // The two tangents point across the chord at each other, so there
            // is no sensible single meeting point: give each control point
            // half the chord length along its own tangent instead.
            let control_length = (p2.0 - p1.0).hypot(p2.1 - p1.1) * 0.5;
            (
                extend(p1, dir1, control_length),
                extend(p2, dir2, control_length),
            )
        } else {
            let inter = line_intersection(p1, dir1, p2, dir2)
                .filter(|i| i.0.abs() + i.1.abs() <= MAX_SANE_POINT)
                .unwrap_or(((p1.0 + p2.0) * 0.5, (p1.1 + p2.1) * 0.5));
            (inter, inter)
        };

        // How near to the target the control point is allowed to rise.
        let mut coeff = 0.8;
        let v1 = len(tangent1).max(1e-6);
        let v2 = len(tangent2).max(1e-6);
        // The controls should not differ by more than 50%.
        let similarity = (v1 / v2).min(v2 / v1).max(0.5);
        // Symmetric controls want to be shorter, or the curve corners.
        coeff *= 1.0 - (similarity - 0.8).max(0.0);

        let (control1, control2) = if v1 > v2 {
            let c1 = lerp_pt(p1, target1, coeff);
            (c1, lerp_pt(p2, target2, coeff * similarity))
        } else {
            let c2 = lerp_pt(p2, target2, coeff);
            (lerp_pt(p1, target1, coeff * similarity), c2)
        };

        self.paint_bezier_curve(pi1, control1, control2, pi2, 0);
    }

    /// Midpoint subdivision until the curve is flat to within
    /// `BEZIER_FLATNESS`, then a straight walk. Foley & Van Dam p.508, via
    /// `kis_paintop.cc`.
    ///
    /// Subdividing on flatness rather than on a fixed step count is what
    /// keeps a big brush's curves smooth: the step count adapts to how
    /// curved the segment actually is, not to how wide the brush is.
    fn paint_bezier_curve(
        &mut self,
        pi1: PointerSample,
        control1: (f32, f32),
        control2: (f32, f32),
        pi2: PointerSample,
        depth: u32,
    ) {
        let p1 = (pi1.x, pi1.y);
        let p2 = (pi2.x, pi2.y);
        let d1 = line_distance(control1, p1, p2);
        let d2 = line_distance(control2, p1, p2);

        if depth >= MAX_BEZIER_DEPTH
            || d1.is_nan()
            || d2.is_nan()
            || (d1 < BEZIER_FLATNESS && d2 < BEZIER_FLATNESS)
        {
            self.paint_line(pi1, pi2);
            return;
        }

        let l2 = mid(p1, control1);
        let h = mid(control1, control2);
        let l3 = mid(l2, h);
        let r3 = mid(control2, p2);
        let r2 = mid(h, r3);
        let l4 = mid(l3, r2);

        let mut middle = mix(pi1, pi2, 0.5);
        middle.x = l4.0;
        middle.y = l4.1;

        self.paint_bezier_curve(pi1, l2, l3, middle, depth + 1);
        self.paint_bezier_curve(middle, r2, r3, pi2, depth + 1);
    }

    /// Walk a straight segment at the node spacing, emitting a spine node per
    /// step. The leftover distance carries into the next call, so spacing is
    /// uniform across segment and curve boundaries alike.
    fn paint_line(&mut self, from: PointerSample, to: PointerSample) {
        let mut cur = from;
        // The walker always makes progress or returns -1, but a cap costs
        // nothing and turns a hypothetical hang into a dropped segment.
        for _ in 0..100_000 {
            let t = self.next_point_position((cur.x, cur.y), (to.x, to.y));
            if t < 0.0 {
                return;
            }
            cur = mix(cur, to, t);
            self.emit(&cur);
        }
    }

    /// Krita's `getNextPointPositionIsotropic`: how far along `start -> end`
    /// the next node falls, or a negative number if the segment ends first.
    fn next_point_position(&mut self, start: (f32, f32), end: (f32, f32)) -> f32 {
        if start == end {
            return -1.0;
        }
        let spacing = self.spacing.max(MIN_SPACING);
        let drag_len = (end.0 - start.0).hypot(end.1 - start.1);
        let next_point_distance = spacing - self.spacing_accum;

        if next_point_distance <= 0.0 {
            // The accumulator is already past the spacing — paint immediately.
            self.spacing_accum = 0.0;
            0.0
        } else if next_point_distance <= drag_len {
            self.spacing_accum = 0.0;
            next_point_distance / drag_len
        } else {
            self.spacing_accum += drag_len;
            -1.0
        }
    }

    // --- Rasterization -----------------------------------------------------

    /// Emit one point of the walked curve: a spine node always, plus a dab
    /// when the brush stamps. The dab also sets the next spacing, since
    /// spacing is a fraction of a dab whose size follows pressure and tilt.
    fn emit(&mut self, s: &PointerSample) {
        let n = self.node_at(s);
        self.spine.push(n);
        if self.brush.mode == BrushMode::Dab {
            let d = Dab::from_sample(&self.brush, s);
            self.spacing = d.spacing(&self.brush);
            self.dabs.push(d);
        }
    }

    /// Rasterize geometry appended since the last drain.
    fn drain(&mut self, ws: &mut StrokeWorkspace, mut acc: Option<DirtyRect>) -> Option<DirtyRect> {
        match self.brush.mode {
            BrushMode::Dab => {
                for i in self.raster_from..self.dabs.len() {
                    if let Some(r) = ws.raster_dab(self.dabs[i]) {
                        acc = Some(union_rect(acc, r));
                    }
                }
                self.raster_from = self.dabs.len().max(self.raster_from);
            }
            BrushMode::Ribbon => {
                for j in self.raster_from.max(1)..self.spine.len() {
                    if let Some(r) = ws.raster_capsule(self.spine[j - 1], self.spine[j]) {
                        acc = Some(union_rect(acc, r));
                    }
                }
                self.raster_from = self.spine.len().max(self.raster_from);
            }
        }
        acc
    }

    /// Composite the coverage inside `rect` over the pre-stroke snapshot.
    fn composite(
        &self,
        canvas: &mut Canvas,
        ws: &StrokeWorkspace,
        pre: &[u8],
        rect: Option<DirtyRect>,
    ) {
        let Some(rect) = rect else { return };
        match self.tool {
            ActiveTool::Eraser => ws.composite_erase(canvas, pre, rect, self.brush.opacity),
            _ => ws.composite_paint(canvas, pre, rect, self.brush.color, self.brush.opacity),
        }
        canvas.mark_dirty(
            rect.min_x,
            rect.min_y,
            rect.max_x - rect.min_x,
            rect.max_y - rect.min_y,
        );
    }

    /// Spine node at a curve sample: pressure-modulated radius and flow.
    /// `brush.opacity` is applied per-stroke at composite time, not here.
    fn node_at(&self, s: &PointerSample) -> SpineNode {
        let p = s.pressure.clamp(0.0, 1.0);
        SpineNode {
            x: s.x,
            y: s.y,
            radius: (self.brush.radius * self.brush.size.apply(p)).max(0.1),
            flow: self.brush.flow * self.brush.flow_dyn.apply(p),
        }
    }
}

// --- Geometry helpers ------------------------------------------------------

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
fn lerp_pt(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t))
}

#[inline]
fn mid(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

#[inline]
fn len(v: (f32, f32)) -> f32 {
    v.0.hypot(v.1)
}

#[inline]
fn is_null(v: (f32, f32)) -> bool {
    v.0 == 0.0 && v.1 == 0.0
}

/// Velocity between two samples. Time is in seconds here and milliseconds in
/// Krita, but `max(1, dt)` means both collapse to a plain position delta for
/// any realistic sample rate — and every packet drained in one frame shares
/// that frame's timestamp, so the delta is all there is to go on anyway. Only
/// the ratio of two tangent lengths is ever used, so a constant factor is
/// harmless.
#[inline]
fn tangent(from: PointerSample, to: PointerSample) -> (f32, f32) {
    let dt = ((to.t - from.t) * 1000.0).max(1.0);
    ((to.x - from.x) / dt, (to.y - from.y) / dt)
}

/// `from` moved `length` along the direction of `toward`.
#[inline]
fn extend(from: (f32, f32), toward: (f32, f32), length: f32) -> (f32, f32) {
    let d = (toward.0 - from.0, toward.1 - from.1);
    let l = len(d);
    if l < 1e-9 {
        return from;
    }
    (from.0 + d.0 / l * length, from.1 + d.1 / l * length)
}

/// Distance from `p` to the infinite line through `a` and `b`. NaN when the
/// line is degenerate, which the flatness test treats as "draw it straight".
#[inline]
fn line_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let l = dx.hypot(dy);
    if l == 0.0 {
        return f32::NAN;
    }
    ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / l
}

/// Intersection of the infinite lines `a1->a2` and `b1->b2`, or `None` when
/// they are parallel.
fn line_intersection(
    a1: (f32, f32),
    a2: (f32, f32),
    b1: (f32, f32),
    b2: (f32, f32),
) -> Option<(f32, f32)> {
    let r = (a2.0 - a1.0, a2.1 - a1.1);
    let s = (b2.0 - b1.0, b2.1 - b1.1);
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom == 0.0 {
        return None;
    }
    let d = (b1.0 - a1.0, b1.1 - a1.1);
    let t = (d.0 * s.1 - d.1 * s.0) / denom;
    Some((a1.0 + r.0 * t, a1.1 + r.1 * t))
}

/// Whether the two *segments* properly cross, i.e. Qt's
/// `QLineF::BoundedIntersection`.
fn segments_cross(a1: (f32, f32), a2: (f32, f32), b1: (f32, f32), b2: (f32, f32)) -> bool {
    let r = (a2.0 - a1.0, a2.1 - a1.1);
    let s = (b2.0 - b1.0, b2.1 - b1.1);
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom == 0.0 {
        return false;
    }
    let d = (b1.0 - a1.0, b1.1 - a1.1);
    let t = (d.0 * s.1 - d.1 * s.0) / denom;
    let u = (d.0 * r.1 - d.1 * r.0) / denom;
    (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
}

/// Blend two samples. Position is included, so callers that want a different
/// position (the Bezier midpoint) overwrite it afterwards.
fn mix(a: PointerSample, b: PointerSample, t: f32) -> PointerSample {
    PointerSample {
        x: lerp(a.x, b.x, t),
        y: lerp(a.y, b.y, t),
        pressure: lerp(a.pressure, b.pressure, t).clamp(0.0, 1.0),
        tilt_x: lerp(a.tilt_x, b.tilt_x, t),
        tilt_y: lerp(a.tilt_y, b.tilt_y, t),
        t: lerp(a.t, b.t, t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(x: f32, y: f32) -> PointerSample {
        PointerSample {
            x,
            y,
            pressure: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            t: 0.0,
        }
    }

    fn builder(view_scale: f32, opts: SmoothingOptions) -> StrokeBuilder {
        StrokeBuilder::new(
            BrushSettings::default_ink(),
            ActiveTool::Ink,
            view_scale,
            opts,
        )
    }

    fn weighted() -> SmoothingOptions {
        SmoothingOptions {
            kind: Smoothing::Weighted,
            ..SmoothingOptions::default()
        }
    }

    /// Run a screen-space path through the builder at a given view scale,
    /// returning the emitted spine mapped back into screen pixels.
    ///
    /// Both the sample positions and the brush radius are divided by the
    /// scale, which is exactly what the screen-size brush lock does — so the
    /// two runs describe the same gesture drawn with the same on-screen pen at
    /// two different zooms.
    fn spine_in_screen_px(
        view_scale: f32,
        opts: SmoothingOptions,
        screen_path: &[(f32, f32)],
    ) -> Vec<(f32, f32)> {
        let mut brush = BrushSettings::default_ink();
        brush.radius = 8.0 / view_scale;

        let mut canvas = Canvas::new(512, 512);
        let pre = canvas.pixels.clone();
        let mut ws = StrokeWorkspace::new();
        ws.begin(512, 512, &brush);

        let mut b = StrokeBuilder::new(brush, ActiveTool::Ink, view_scale, opts);
        for &(x, y) in screen_path {
            b.push(sample(x / view_scale, y / view_scale));
            b.flush(&mut canvas, &mut ws, &pre);
        }
        b.finish(&mut canvas, &mut ws, &pre);

        b.spine
            .iter()
            .map(|n| (n.x * view_scale, n.y * view_scale))
            .collect()
    }

    /// A zigzag with a sharp reversal — the shape whose corners are the first
    /// thing a badly-scaled filter rounds off.
    fn zigzag() -> Vec<(f32, f32)> {
        let mut p = Vec::new();
        for i in 0..40 {
            let t = i as f32;
            p.push((60.0 + t * 3.0, 120.0 + if i % 2 == 0 { 0.0 } else { 14.0 }));
        }
        p
    }

    /// The same gesture at two zooms must be smoothed identically.
    ///
    /// This guards the `1 / view_scale` term in `effective_smoothness`:
    /// without it the weighted filter would measure its width in canvas
    /// pixels, which means a different amount of smoothing at every zoom —
    /// and a line that gets wavier the further out you go.
    ///
    /// Measured on the filtered input rather than on the emitted spine. The
    /// spine also depends on Bezier subdivision, whose flatness threshold is
    /// deliberately a canvas-pixel tolerance (that is the space being
    /// rasterized), so it is finer relative to the gesture when zoomed out and
    /// would blur what this test is asking about.
    #[test]
    fn weighted_smoothing_is_zoom_independent() {
        let path = zigzag();
        let out = smoothed_in_screen_px(0.5, weighted(), &path);
        let zin = smoothed_in_screen_px(2.0, weighted(), &path);

        assert!(out.len() > 20, "test is vacuous on {} samples", out.len());
        assert_eq!(out.len(), zin.len());
        for (i, (a, b)) in out.iter().zip(zin.iter()).enumerate() {
            assert!(
                (a.0 - b.0).abs() < 0.02 && (a.1 - b.1).abs() < 0.02,
                "sample {i} smoothed differently between zooms: {a:?} vs {b:?}"
            );
        }
    }

    /// Run a screen-space path through the filter at a given view scale and
    /// return the smoothed positions, back in screen pixels. Positions are
    /// divided by the scale on the way in, which is the same gesture drawn
    /// with the same on-screen pen at two different zooms.
    fn smoothed_in_screen_px(
        view_scale: f32,
        opts: SmoothingOptions,
        screen_path: &[(f32, f32)],
    ) -> Vec<(f32, f32)> {
        let mut b = builder(view_scale, opts);
        let mut out = Vec::new();
        for &(x, y) in screen_path {
            b.push(sample(x / view_scale, y / view_scale));
            let (sx, sy) = b.smooth_pos.unwrap();
            out.push((sx * view_scale, sy * view_scale));
        }
        out
    }

    /// The reported bug, as a test: a straight line drawn with one screen
    /// pixel of jitter — the pointer's own quantization, not hand motion —
    /// must come out straighter than it went in, at any zoom.
    #[test]
    fn weighted_smoothing_rejects_one_pixel_jitter() {
        for view_scale in [0.25, 1.0, 4.0] {
            let mut b = builder(view_scale, weighted());
            // Peak-to-peak, not distance from the baseline: a one-sided filter
            // settles on the jitter's *mean*, half a pixel off the line, and
            // that offset is not wobble.
            let mut ys: Vec<f32> = Vec::new();
            for i in 0..60 {
                let x = 100.0 + i as f32 * 4.0 / view_scale;
                // Alternating one screen pixel off the line y = 100.
                let jitter = if i % 2 == 0 { 0.0 } else { 1.0 / view_scale };
                b.push(sample(x, 100.0 + jitter));
                // Skip the filter's warm-up, where there is not yet a window
                // of history to average over.
                if i >= 20 {
                    ys.push(b.smooth_pos.unwrap().1 * view_scale);
                }
            }
            let lo = ys.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let wobble = hi - lo;
            // Roughly 2.4x attenuation, not the 10x the nominal 50-pixel
            // window suggests. That is Krita's own behaviour, and the reason
            // is worth knowing: the filter measures distance from its *own*
            // lagging output back to the raw incoming sample, so its lag
            // inflates every distance in the history and shrinks the effective
            // window. It settles at an equilibrium well short of the
            // configured width. Weighted smoothing is a taste option; the
            // actual fix for a wavy line is sub-pixel input, which is why
            // Krita ships Basic as the default.
            assert!(
                wobble < 0.5,
                "1px jitter survived at scale {view_scale}: {wobble} screen px peak-to-peak,                  against 1.0 raw"
            );
        }
    }

    /// The reported bug, end to end: a perfectly straight gesture must
    /// produce a perfectly straight stroke, at any zoom, with the default
    /// (Basic) smoothing that does no positional filtering at all.
    ///
    /// This is the shape of the failure the user saw — a slow straight line
    /// coming out wavy when zoomed out. What made it wave was the input, not
    /// the curve fit: whole-screen-pixel mouse positions become a staircase
    /// several canvas pixels wide once one screen pixel covers several canvas
    /// pixels. Fed clean samples, the fit is exact.
    #[test]
    fn a_straight_gesture_stays_straight_at_any_zoom() {
        let path: Vec<(f32, f32)> = (0..80)
            .map(|i| (60.0 + i as f32 * 3.0, 120.0 + i as f32 * 1.5))
            .collect();
        for scale in [0.25, 1.0, 4.0] {
            let spine = spine_in_screen_px(scale, SmoothingOptions::default(), &path);
            assert!(spine.len() > 10, "vacuous at scale {scale}");
            let a = *spine.first().unwrap();
            let b = *spine.last().unwrap();
            let worst = spine
                .iter()
                .map(|p| line_distance(*p, a, b))
                .fold(0.0f32, f32::max);
            assert!(
                worst < 0.1,
                "straight gesture bowed by {worst} screen px at scale {scale}"
            );
        }
    }

    /// Basic smoothing does not touch positions — that is the point of it,
    /// and of Krita choosing it as the default. Clean lines come from the
    /// curve fit and from the sample fidelity underneath it.
    #[test]
    fn basic_smoothing_leaves_positions_alone() {
        let mut b = builder(1.0, SmoothingOptions::default());
        b.push(sample(10.0, 10.0));
        b.push(sample(20.0, 31.0));
        assert_eq!(b.smooth_pos, Some((20.0, 31.0)));
    }

    /// Pen-down must land exactly where the user pressed — no filter warm-up.
    #[test]
    fn first_sample_is_exact() {
        for opts in [SmoothingOptions::default(), weighted()] {
            let mut b = builder(1.0, opts);
            b.push(sample(42.0, 77.0));
            assert_eq!(b.smooth_pos, Some((42.0, 77.0)));
            assert_eq!(b.spine.len(), 1, "the pen-down dot is a spine node");
            assert_eq!((b.spine[0].x, b.spine[0].y), (42.0, 77.0));
        }
    }

    /// The spacing walker must emit at a uniform interval, and must carry its
    /// remainder across segment boundaries rather than restarting at each one.
    #[test]
    fn spacing_is_uniform_across_segment_boundaries() {
        let mut b = builder(1.0, SmoothingOptions::default());
        b.spacing = 4.0;
        b.spacing_accum = 0.0;
        // Three collinear segments walked one after another.
        b.paint_line(sample(0.0, 0.0), sample(10.0, 0.0));
        b.paint_line(sample(10.0, 0.0), sample(20.0, 0.0));
        b.paint_line(sample(20.0, 0.0), sample(30.0, 0.0));

        let xs: Vec<f32> = b.spine.iter().map(|n| n.x).collect();
        assert!(xs.len() >= 7, "expected ~7 nodes over 30px at 4px, got {xs:?}");
        for w in xs.windows(2) {
            let gap = w[1] - w[0];
            assert!(
                (gap - 4.0).abs() < 1e-3,
                "uneven spacing across a boundary: {xs:?}"
            );
        }
    }

    /// Subdivision must stop once the control points are within the flatness
    /// threshold of the chord — that is what bounds the work, and a curve
    /// that never flattens would recurse to the depth cap on every segment.
    #[test]
    fn bezier_subdivision_terminates_on_flatness() {
        let a = (0.0, 0.0);
        let b = (100.0, 0.0);
        // A control point 30px off the chord is not flat...
        assert!(line_distance((50.0, 30.0), a, b) >= BEZIER_FLATNESS);
        // ...but one midpoint subdivision halves its offset, so a handful of
        // levels is enough to reach the threshold.
        let mut off = 30.0f32;
        let mut levels = 0;
        while off >= BEZIER_FLATNESS {
            off *= 0.5;
            levels += 1;
        }
        assert!(
            levels < MAX_BEZIER_DEPTH,
            "flatness should be reached well inside the depth cap, took {levels}"
        );
    }

    /// A curved stroke drawn with a large brush must still be tessellated
    /// finely. The old fixed step count was `chord / (radius * 0.25)`, which
    /// bottomed out at two evaluations per segment once the brush got wide —
    /// visible faceting exactly where a wide brush shows it most.
    #[test]
    fn large_brushes_do_not_facet() {
        let mut brush = BrushSettings::default_ink();
        brush.radius = 60.0;
        let mut b = StrokeBuilder::new(
            brush,
            ActiveTool::Ink,
            1.0,
            SmoothingOptions::default(),
        );
        // A quarter circle of radius 200, sampled every ~10 degrees.
        for i in 0..10 {
            let a = i as f32 * std::f32::consts::FRAC_PI_2 / 9.0;
            b.push(sample(256.0 + 200.0 * a.cos(), 256.0 + 200.0 * a.sin()));
        }

        // Every emitted node must sit on the circle: a faceted spine cuts the
        // corner and lands inside it.
        let mut worst: f32 = 0.0;
        for n in b.spine.iter().skip(1) {
            let r = (n.x - 256.0).hypot(n.y - 256.0);
            worst = worst.max((r - 200.0).abs());
        }
        assert!(
            worst < 2.0,
            "spine deviates from the arc by {worst}px — the curve is faceted"
        );
    }

    /// Render every preset as a stroke on a grey ground, for eyeballing brush
    /// feel against a reference. Ignored by default — it writes a file:
    ///
    /// ```text
    /// PREVIEW_OUT=sheet.png cargo test brush_sheet -- --ignored --nocapture
    /// ```
    ///
    /// Tuning a brush by slider means a round trip through the GUI and a
    /// tablet. This is the short loop for the part you can judge from a
    /// picture — density, tooth, taper, how far tilt opens the dab up.
    #[test]
    #[ignore]
    fn brush_sheet() {
        let Ok(path) = std::env::var("PREVIEW_OUT") else {
            panic!("set PREVIEW_OUT to the .png to write");
        };
        let (w, h) = (900u32, 560u32);
        let mut canvas = Canvas::new(w, h);
        for px in canvas.pixels.chunks_mut(4) {
            px.copy_from_slice(&[205, 205, 205, 255]);
        }

        // (brush, tilt in degrees, peak pressure)
        let rows: [(BrushSettings, (f32, f32), f32); 6] = [
            (BrushSettings::default_pencil(), (0.0, 0.0), 1.0),
            (BrushSettings::default_pencil(), (0.0, 0.0), 0.45),
            (BrushSettings::pencil_4b(), (0.0, 0.0), 1.0),
            (BrushSettings::pencil_tilted(), (0.0, 0.0), 1.0),
            (BrushSettings::pencil_tilted(), (55.0, 20.0), 1.0),
            (BrushSettings::default_ink(), (0.0, 0.0), 1.0),
        ];
        for (i, (brush, tilt, peak)) in rows.into_iter().enumerate() {
            sheet_stroke(&mut canvas, brush, 60.0 + i as f32 * 85.0, tilt, peak);
        }

        image::RgbaImage::from_raw(w, h, canvas.pixels.clone())
            .expect("canvas is RGBA8")
            .save(&path)
            .expect("write preview");
        println!("wrote {path}");
    }

    /// One stroke for `brush_sheet`: a shallow S with pressure ramping in and
    /// out, plus a little tremor so grain and smoothing both have something to
    /// work on.
    fn sheet_stroke(
        canvas: &mut Canvas,
        brush: BrushSettings,
        y0: f32,
        tilt: (f32, f32),
        peak: f32,
    ) {
        let pre = canvas.pixels.clone();
        let mut ws = StrokeWorkspace::new();
        ws.begin(canvas.width, canvas.height, &brush);
        let mut b = StrokeBuilder::new(brush, ActiveTool::Pencil, 1.0, SmoothingOptions::default());

        let n = 90;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let p = (peak * (t * std::f32::consts::PI).sin().powf(0.6)).clamp(0.02, 1.0);
            b.push(PointerSample {
                x: 40.0 + t * 800.0,
                y: y0 + (t * 6.0).sin() * 10.0 + ((i % 3) as f32 - 1.0) * 0.4,
                pressure: p,
                tilt_x: tilt.0,
                tilt_y: tilt.1,
                t: 0.0,
            });
            b.flush(canvas, &mut ws, &pre);
        }
        b.finish(canvas, &mut ws, &pre);
    }
}
