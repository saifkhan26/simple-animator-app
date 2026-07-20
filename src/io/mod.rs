//! Persistence: project save, PNG sequence export, GIF export, ORA read.
//!
//! Phase A: only `png_save` (single canvas → PNG file via rfd dialog).

pub mod composite;
pub mod gif_export;
pub mod gif_import;
pub mod mp4_export;
pub mod image_import;
pub mod legacy;
pub mod png_import;
pub mod png_save;
pub mod png_seq;
pub mod project_file;
pub mod video_import;
