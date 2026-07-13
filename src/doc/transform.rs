//! Per-layer 2D transform (translate / uniform scale / rotate) and its
//! keyframes.
//!
//! A transform maps a cell's local pixel space into document (canvas) space.
//! Identity places the cell centered on the canvas at native scale, so a
//! project-sized drawn cell with the identity transform overlays the canvas
//! pixel-for-pixel (matching the pre-transform behavior).

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transform {
    /// Offset of the cell center from the canvas center, in document pixels.
    pub tx: f32,
    pub ty: f32,
    /// Uniform scale.
    pub scale: f32,
    /// Rotation in radians.
    pub rot: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
            rot: 0.0,
        }
    }
}

impl Transform {
    pub fn is_identity(&self) -> bool {
        self.tx == 0.0 && self.ty == 0.0 && (self.scale - 1.0).abs() < 1e-6 && self.rot == 0.0
    }

    /// Linear interpolation of every component.
    pub fn lerp(a: Transform, b: Transform, t: f32) -> Transform {
        let l = |x: f32, y: f32| x + (y - x) * t;
        Transform {
            tx: l(a.tx, b.tx),
            ty: l(a.ty, b.ty),
            scale: l(a.scale, b.scale),
            rot: l(a.rot, b.rot),
        }
    }

    /// Map a cell-local pixel `(u, v)` to document coordinates. `(cw, ch)` is the
    /// cell size, `(pw, ph)` the canvas size.
    pub fn cell_to_doc(&self, u: f32, v: f32, cw: f32, ch: f32, pw: f32, ph: f32) -> (f32, f32) {
        let lx = (u - cw * 0.5) * self.scale;
        let ly = (v - ch * 0.5) * self.scale;
        let (s, c) = self.rot.sin_cos();
        let rx = lx * c - ly * s;
        let ry = lx * s + ly * c;
        (pw * 0.5 + self.tx + rx, ph * 0.5 + self.ty + ry)
    }

    /// Inverse of [`cell_to_doc`]: map a document point to cell-local pixels.
    pub fn doc_to_cell(&self, x: f32, y: f32, cw: f32, ch: f32, pw: f32, ph: f32) -> (f32, f32) {
        let dx = x - pw * 0.5 - self.tx;
        let dy = y - ph * 0.5 - self.ty;
        let (s, c) = self.rot.sin_cos();
        // Rotate by -rot.
        let rx = dx * c + dy * s;
        let ry = -dx * s + dy * c;
        let sc = if self.scale.abs() < 1e-9 { 1e-9 } else { self.scale };
        (rx / sc + cw * 0.5, ry / sc + ch * 0.5)
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct TransformKey {
    pub frame: usize,
    pub transform: Transform,
}
