//! Stroke construction: input smoothing, centripetal Catmull-Rom spline,
//! spine-node emission, ribbon capsule rasterization with per-stroke
//! coverage compositing (see `tools::ribbon`).
//!
//! The CPU rasterizer lives in `ribbon.rs`; a later wgpu compute port
//! replaces that module's internals while this input model stays the same.

use crate::doc::canvas::{Canvas, DirtyRect};
use crate::input::pointer::PointerSample;
use crate::tools::ribbon::{union_rect, SpineNode, StrokeWorkspace};
use crate::tools::{ActiveTool, BrushSettings};

/// Builds up a stroke as the pointer moves and rasterizes it incrementally.
pub struct StrokeBuilder {
    pub brush: BrushSettings,
    pub tool: ActiveTool,
    /// Raw (smoothed) input samples, retained for the whole stroke.
    samples: Vec<PointerSample>,
    /// Committed spine nodes — append-only. Under `max()` coverage combining
    /// already-rasterized geometry can never be retracted, so nodes are only
    /// emitted for curve segments whose shape is final (see `flush`).
    spine: Vec<SpineNode>,
    /// First spine index whose incoming segment is not yet rasterized.
    raster_from: usize,
    /// Curve segments `samples[i-1] -> samples[i]` for `i <= committed_seg`
    /// have been committed to the spine.
    committed_seg: usize,
    /// Arc length accumulated on the curve since the last emitted node.
    arc_since_node: f32,
    /// Curve direction at the last emitted node (normalized).
    last_dir: Option<(f32, f32)>,
    /// Last dense curve evaluation point, carried across segments/flushes.
    last_curve: Option<PointerSample>,
    /// The pen-down dot has been rasterized.
    dot_emitted: bool,
    /// Smoothed pointer position (exponential moving average).
    smooth_pos: Option<(f32, f32)>,
    /// Smoothed pressure (exponential moving average).
    smooth_pressure: Option<f32>,
}

/// Smoothing factor for exponential moving average of pointer input.
/// 0.0 = maximum smoothing (sluggish), 1.0 = no smoothing.
const SMOOTH_FACTOR: f32 = 0.45;

/// Emit a spine node whenever the curve direction has turned this much since
/// the last node, regardless of arc length (keeps tight curls smooth with
/// large brushes). cos(10 degrees).
const ANGLE_COS: f32 = 0.984_807_75;

impl StrokeBuilder {
    pub fn new(brush: BrushSettings, tool: ActiveTool) -> Self {
        Self {
            brush,
            tool,
            samples: Vec::with_capacity(256),
            spine: Vec::with_capacity(256),
            raster_from: 0,
            committed_seg: 0,
            arc_since_node: 0.0,
            last_dir: None,
            last_curve: None,
            dot_emitted: false,
            smooth_pos: None,
            smooth_pressure: None,
        }
    }

    /// Push a raw pointer sample, applying input smoothing.
    pub fn push(&mut self, s: PointerSample) {
        let f = SMOOTH_FACTOR;
        let s = match self.smooth_pos {
            Some((sx, sy)) => PointerSample {
                x: sx * (1.0 - f) + s.x * f,
                y: sy * (1.0 - f) + s.y * f,
                ..s
            },
            None => s,
        };
        self.smooth_pos = Some((s.x, s.y));

        // Smooth pressure separately to avoid sudden thickness jumps.
        let s = match self.smooth_pressure {
            Some(sp) => PointerSample {
                pressure: sp * (1.0 - f) + s.pressure * f,
                ..s
            },
            None => s,
        };
        self.smooth_pressure = Some(s.pressure);

        // Drop duplicate position (egui may emit zero-delta moves).
        if let Some(last) = self.samples.last() {
            if (last.x - s.x).abs() < 0.01 && (last.y - s.y).abs() < 0.01 {
                return;
            }
        }
        self.samples.push(s);
    }

    /// Rasterize newly committed stroke geometry and composite it into the
    /// canvas. Returns the pixel rect updated this call (for partial texture
    /// upload), or `None` if nothing changed.
    ///
    /// Curve segment `samples[i-1] -> samples[i]` is only committed once
    /// `samples[i+1]` exists: Catmull-Rom needs the following point, and with
    /// a clamped placeholder the segment's shape would change when the next
    /// sample arrives — which `max()` coverage cannot undo. `finish()`
    /// commits the final segment with the clamped endpoint, exactly like the
    /// old stamper's last segment. The uncommitted tail is drawn by the UI as
    /// a live overlay (see `live_tail`).
    pub fn flush(
        &mut self,
        canvas: &mut Canvas,
        ws: &mut StrokeWorkspace,
        pre: &[u8],
    ) -> Option<DirtyRect> {
        if self.samples.is_empty() {
            return None;
        }
        let mut acc: Option<DirtyRect> = None;

        // Pen-down dot: instant ink with zero latency.
        if !self.dot_emitted {
            self.dot_emitted = true;
            let s = self.samples[0];
            let n = self.node_at(&s);
            self.spine.push(n);
            self.raster_from = 1;
            self.last_curve = Some(s);
            if let Some(r) = ws.raster_dot(n) {
                acc = Some(union_rect(acc, r));
            }
        }

        // Commit every segment that has a real following sample.
        while self.committed_seg + 2 <= self.samples.len().saturating_sub(1) {
            let i = self.committed_seg + 1;
            self.commit_segment(i);
            self.committed_seg = i;
        }

        acc = self.drain(ws, acc);
        self.composite(canvas, ws, pre, acc);
        acc
    }

    /// Commit the remaining tail segments (clamped endpoint) and rasterize
    /// everything outstanding. Called on pointer-up.
    pub fn finish(
        &mut self,
        canvas: &mut Canvas,
        ws: &mut StrokeWorkspace,
        pre: &[u8],
    ) -> Option<DirtyRect> {
        let mut acc = self.flush(canvas, ws, pre);

        for i in (self.committed_seg + 1)..self.samples.len() {
            self.commit_segment(i);
            self.committed_seg = i;
        }
        // Land the spine exactly on the stroke's end point.
        if let Some(end) = self.last_curve {
            let needs_end_node = self
                .spine
                .last()
                .map(|n| (n.x - end.x).abs() > 0.01 || (n.y - end.y).abs() > 0.01)
                .unwrap_or(false);
            if needs_end_node {
                let n = self.node_at(&end);
                self.spine.push(n);
                self.arc_since_node = 0.0;
            }
        }

        acc = self.drain(ws, acc);
        self.composite(canvas, ws, pre, acc);
        acc
    }

    /// The uncommitted stroke tail: last committed spine node plus the
    /// current smoothed pointer position. Drawn by the UI as an overlay so
    /// the one-sample commit lag is invisible.
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

    /// Rasterize spine segments appended since the last drain.
    fn drain(&mut self, ws: &mut StrokeWorkspace, mut acc: Option<DirtyRect>) -> Option<DirtyRect> {
        for j in self.raster_from.max(1)..self.spine.len() {
            if let Some(r) = ws.raster_capsule(self.spine[j - 1], self.spine[j]) {
                acc = Some(union_rect(acc, r));
            }
        }
        self.raster_from = self.spine.len().max(self.raster_from);
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

    /// Evaluate the Catmull-Rom curve for segment `samples[i-1] -> samples[i]`
    /// densely and greedily decimate it into spine nodes: a node is emitted
    /// when the arc length since the last node exceeds the node spacing, or
    /// when the curve direction has turned past `ANGLE_COS`.
    fn commit_segment(&mut self, i: usize) {
        let p0 = self.samples[i.saturating_sub(2)];
        let p1 = self.samples[i - 1];
        let p2 = self.samples[i];
        let p3 = self.samples[(i + 1).min(self.samples.len() - 1)];

        let chord = dist(p1, p2);
        if chord <= 0.0 {
            return;
        }
        let steps = (chord / (self.brush.radius * 0.25)).ceil().max(2.0) as usize;
        let spacing = (self.brush.radius * 0.3).max(1.0);

        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let interp = catrom_centripetal(p0, p1, p2, p3, t);
            if let Some(prev) = self.last_curve {
                let dx = interp.x - prev.x;
                let dy = interp.y - prev.y;
                let ds = (dx * dx + dy * dy).sqrt();
                if ds > 1e-6 {
                    self.arc_since_node += ds;
                    let dir = (dx / ds, dy / ds);
                    let turned = match self.last_dir {
                        Some((lx, ly)) => lx * dir.0 + ly * dir.1 < ANGLE_COS,
                        None => false,
                    };
                    if self.arc_since_node >= spacing || turned {
                        let n = self.node_at(&interp);
                        self.spine.push(n);
                        self.arc_since_node = 0.0;
                        self.last_dir = Some(dir);
                    }
                }
            }
            self.last_curve = Some(interp);
        }
    }

    /// Spine node at a curve sample: pressure-modulated radius and flow.
    /// `brush.opacity` is applied per-stroke at composite time, not here.
    fn node_at(&self, s: &PointerSample) -> SpineNode {
        let p = s.pressure.clamp(0.0, 1.0);
        SpineNode {
            x: s.x,
            y: s.y,
            radius: (self.brush.radius * lerp(1.0 - self.brush.pressure_size, 1.0, p)).max(0.1),
            flow: lerp(1.0 - self.brush.pressure_opacity, 1.0, p),
        }
    }
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Centripetal Catmull-Rom interpolation (alpha = 0.5).
///
/// Unlike uniform Catmull-Rom (which can produce cusps and loops when
/// control points are irregularly spaced), centripetal parameterization
/// computes knot intervals based on control-point distances raised to
/// alpha=0.5. This prevents cusps and follows the control points more
/// faithfully.
///
/// `t` is in [0, 1] and maps to the curve segment between `p1` and `p2`.
fn catrom_centripetal(
    p0: PointerSample,
    p1: PointerSample,
    p2: PointerSample,
    p3: PointerSample,
    t: f32,
) -> PointerSample {
    let alpha = 0.5;

    // Knot intervals based on Euclidean distance^alpha.
    let d01 = dist(p0, p1).powf(alpha);
    let d12 = dist(p1, p2).powf(alpha);
    let d23 = dist(p2, p3).powf(alpha);

    let t0 = 0.0;
    let t1 = t0 + d01;
    let t2 = t1 + d12;
    let t3 = t2 + d23;

    // Remap from normalised [0,1] to the chord-length parameter space [t1, t2].
    let t_remap = t1 + t * (t2 - t1);

    // Barry-Goldman algorithm for centripetal Catmull-Rom evaluation.
    let interp = |a: f32, b: f32, c: f32, d: f32| -> f32 {
        let a1 = lerp(a, b, (t1 - t_remap) / (t1 - t0));
        let a2 = lerp(b, c, (t2 - t_remap) / (t2 - t1));
        let a3 = lerp(c, d, (t3 - t_remap) / (t3 - t2));

        let b1 = lerp(a1, a2, (t2 - t_remap) / (t2 - t0));
        let b2 = lerp(a2, a3, (t3 - t_remap) / (t3 - t1));

        // Final point C = B1 when evaluated at t=t1... but the formula that
        // gives us the point on the curve at t_remap uses a different weight:
        lerp(b1, b2, (t2 - t_remap) / (t2 - t1))
    };

    PointerSample {
        x: interp(p0.x, p1.x, p2.x, p3.x),
        y: interp(p0.y, p1.y, p2.y, p3.y),
        pressure: interp(p0.pressure, p1.pressure, p2.pressure, p3.pressure).clamp(0.0, 1.0),
        tilt_x: interp(p0.tilt_x, p1.tilt_x, p2.tilt_x, p3.tilt_x),
        tilt_y: interp(p0.tilt_y, p1.tilt_y, p2.tilt_y, p3.tilt_y),
        t: interp(p0.t, p1.t, p2.t, p3.t),
    }
}

/// Euclidean distance between two pointer samples (x, y only).
fn dist(a: PointerSample, b: PointerSample) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}
