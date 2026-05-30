//! Document model — project, layers, frames, cells.
//!
//! Phase A: only `canvas` is wired (single drawing surface).
//! Phase B+ will add `frame`, `layer`, `xsheet`, full `project`.

pub mod canvas;
pub mod layer;
pub mod project;
