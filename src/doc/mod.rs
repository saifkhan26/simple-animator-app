//! Document model — project, layers, frames, cells.
//!
//! Phase A: only `canvas` is wired (single drawing surface).
//! Phase B+ will add `frame`, `layer`, `xsheet`, full `project`.

pub mod camera;
pub mod canvas;
pub mod clip;
pub mod groups;
pub mod layer;
pub mod orient;
pub mod project;
pub mod transform;
