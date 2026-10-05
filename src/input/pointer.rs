//! Unified pointer sample: position, pressure, tilt, timestamp.
//!
//! Mouse path synthesises pressure = 1.0 and tilt = (0, 0).
//! Tablet path (Phase D) fills these from device packets.

#[derive(Clone, Copy, Debug)]
pub struct PointerSample {
    /// Position in canvas pixel space.
    pub x: f32,
    pub y: f32,
    /// Pen pressure, 0..=1.
    pub pressure: f32,
    /// Tilt from the canvas normal on each axis, degrees, -90..=90.
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// Seconds since program start, at frame resolution: every packet in a
    /// frame shares it. The stroke's curve fitting keys on this, so it must
    /// keep meaning exactly what it always has.
    pub t: f32,
    /// When the sample was taken, milliseconds: the tablet's own packet
    /// time where it reports one, otherwise the frame time. What Krita's
    /// speed sensor measures against.
    pub time_ms: f64,
    /// Barrel rotation (twist), degrees, -180..=180, 0 when the pen has no
    /// such axis.
    pub rotation: f32,
    /// Tangential pressure (an airbrush wheel), 0..=1.
    pub tangential: f32,
    /// Drawing speed for Krita's speed sensor, 0..=1. Filled in by the
    /// stroke, not by the input layer.
    pub speed: f32,
}

impl Default for PointerSample {
    /// Full pressure, upright, at the origin.
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            pressure: 1.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            t: 0.0,
            time_ms: 0.0,
            rotation: 0.0,
            tangential: 0.0,
            speed: 0.0,
        }
    }
}

