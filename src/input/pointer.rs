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
    /// Tilt in radians from canvas normal on each axis.
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// Seconds since program start.
    pub t: f32,
}
