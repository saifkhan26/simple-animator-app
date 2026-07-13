//! Pointer / tablet input abstraction.
//!
//! Phase A: mouse only via egui::Response.
//! Phase D: tablet (octotablet / wintab) injects PointerSample with pressure + tilt.

pub mod pointer;
pub mod screen_pixel;
pub mod shortcuts;
pub mod tablet;
