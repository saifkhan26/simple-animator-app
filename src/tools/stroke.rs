//! Stroke construction: input smoothing, spacing → stamps, per-stamp blit.
//!
//! Phase A uses CPU stamping into the canvas pixel buffer. Phase D moves stamping
//! to a wgpu compute shader, but the input model (`PointerSample` → spline →
//! evenly-spaced stamps) stays the same.

use crate::doc::canvas::Canvas;
use crate::input::pointer::PointerSample;
use crate::tools::{ActiveTool, BrushSettings};

/// Builds up a stroke as the pointer moves and emits stamps to the canvas.
pub struct StrokeBuilder {
    pub brush: BrushSettings,
    pub tool: ActiveTool,
    /// Raw input samples.
    samples: Vec<PointerSample>,
    /// Arc length covered up to the last emitted stamp.
    last_emit_s: f32,
}

impl StrokeBuilder {
    pub fn new(brush: BrushSettings, tool: ActiveTool) -> Self {
        Self {
            brush,
            tool,
            samples: Vec::with_capacity(64),
            last_emit_s: 0.0,
        }
    }

    pub fn push(&mut self, s: PointerSample) {
        // Drop duplicate position (egui may emit zero-delta moves).
        if let Some(last) = self.samples.last() {
            if (last.x - s.x).abs() < 0.01 && (last.y - s.y).abs() < 0.01 {
                return;
            }
        }
        self.samples.push(s);
    }

    /// Emit stamps for any newly available stroke segments.
    pub fn flush_to(&mut self, canvas: &mut Canvas) {
        if self.samples.is_empty() {
            return;
        }
        // First sample: drop a single stamp at the down point.
        if self.samples.len() == 1 {
            self.stamp(canvas, self.samples[0]);
            return;
        }
        // Use Catmull-Rom across the last 4 points (extend endpoints by repetition).
        for i in 1..self.samples.len() {
            let p0 = self.samples[i.saturating_sub(2).max(0)];
            let p1 = self.samples[i - 1];
            let p2 = self.samples[i];
            let p3 = self.samples[(i + 1).min(self.samples.len() - 1)];

            // Segment from p1 → p2; subdivide adaptively by chord length.
            let chord = ((p2.x - p1.x).powi(2) + (p2.y - p1.y).powi(2)).sqrt();
            let steps = (chord / (self.brush.radius * 0.25)).ceil().max(1.0) as usize;

            for step in 1..=steps {
                let t = step as f32 / steps as f32;
                let interp = catmull_rom(p0, p1, p2, p3, t);

                // Spacing-based emission.
                let dx = interp.x - last_pos(self, canvas).0;
                let dy = interp.y - last_pos(self, canvas).1;
                let ds = (dx * dx + dy * dy).sqrt();
                self.last_emit_s += ds;

                let stamp_spacing = (self.brush.radius * self.brush.spacing).max(0.5);
                while self.last_emit_s >= stamp_spacing {
                    self.last_emit_s -= stamp_spacing;
                    self.stamp(canvas, interp);
                }
            }
        }
        // Drop intermediate samples to keep memory small; keep last two for context.
        if self.samples.len() > 4 {
            let last2 = self.samples[self.samples.len() - 2..].to_vec();
            self.samples.clear();
            self.samples.extend(last2);
        }
    }

    pub fn finish(&mut self, canvas: &mut Canvas) {
        self.flush_to(canvas);
    }

    /// Rasterise a single circular stamp.
    fn stamp(&self, canvas: &mut Canvas, s: PointerSample) {
        let p = s.pressure.clamp(0.0, 1.0);
        let r = self.brush.radius * lerp(1.0 - self.brush.pressure_size, 1.0, p);
        let a = self.brush.opacity * lerp(1.0 - self.brush.pressure_opacity, 1.0, p);
        let r = r.max(0.5);

        let cx = s.x;
        let cy = s.y;
        let r_i = r.ceil() as i32 + 1;
        let x0 = (cx as i32 - r_i).max(0);
        let y0 = (cy as i32 - r_i).max(0);
        let x1 = (cx as i32 + r_i).min(canvas.width as i32 - 1);
        let y1 = (cy as i32 + r_i).min(canvas.height as i32 - 1);
        if x1 < x0 || y1 < y0 {
            return;
        }

        let hardness = self.brush.hardness.max(0.5);
        let inv_r = 1.0 / r;
        for py in y0..=y1 {
            for px in x0..=x1 {
                let dx = px as f32 + 0.5 - cx;
                let dy = py as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt() * inv_r;
                if d >= 1.0 {
                    continue;
                }
                let mask = (1.0 - d).powf(hardness);
                let cov = (mask * a).clamp(0.0, 1.0);
                if cov <= 0.001 {
                    continue;
                }
                match self.tool {
                    ActiveTool::Eraser => {
                        canvas.erase_pixel(px as u32, py as u32, cov);
                    }
                    _ => {
                        let mut c = self.brush.color;
                        c[3] = (c[3] as f32 * cov).round() as u8;
                        canvas.blend_pixel(px as u32, py as u32, c);
                    }
                }
            }
        }

        canvas.mark_dirty(
            x0 as u32,
            y0 as u32,
            (x1 - x0 + 1) as u32,
            (y1 - y0 + 1) as u32,
        );
    }
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Centripetal Catmull-Rom (alpha=0.5) — robust against cusps.
fn catmull_rom(
    p0: PointerSample,
    p1: PointerSample,
    p2: PointerSample,
    p3: PointerSample,
    t: f32,
) -> PointerSample {
    let t2 = t * t;
    let t3 = t2 * t;

    let interp = |a: f32, b: f32, c: f32, d: f32| -> f32 {
        0.5 * ((2.0 * b)
            + (-a + c) * t
            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
            + (-a + 3.0 * b - 3.0 * c + d) * t3)
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

/// Returns the previous emitted-position used to track stamp spacing.
/// Currently approximated by the most recent raw sample to avoid extra state.
fn last_pos(builder: &StrokeBuilder, _canvas: &Canvas) -> (f32, f32) {
    let last = builder
        .samples
        .iter()
        .rev()
        .nth(1)
        .or_else(|| builder.samples.last());
    match last {
        Some(s) => (s.x, s.y),
        None => (0.0, 0.0),
    }
}
