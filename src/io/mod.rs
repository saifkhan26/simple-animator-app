//! Persistence: project save, PNG sequence export, GIF export, ORA read.
//!
//! Phase A: only `png_save` (single canvas → PNG file via rfd dialog).

pub mod composite;
pub mod gif_export;
pub mod png_save;
pub mod png_seq;
