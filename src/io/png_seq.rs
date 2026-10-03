//! Export each frame of the project as `frame_NNNN.png` into a chosen folder.
//! Flattens visible non-reference layers per frame.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use image::{ImageBuffer, Rgba};

use crate::doc::project::Project;
use crate::io::composite;

/// Whether `order` shows some timeline frame more than once — a loop or a
/// ping-pong. Then files are numbered by position, not by timeline frame.
pub fn repeats(order: &[usize]) -> bool {
    let mut seen = std::collections::HashSet::new();
    !order.iter().all(|f| seen.insert(*f))
}

/// Write the timeline frames in `order` (see [`crate::io::frame_order`]).
///
/// A plain range keeps the **absolute** frame index in the filename, so a
/// range starting at 10 writes `frame_0010.png` — renumbering from zero would
/// silently misalign a partial re-export against files from an earlier full
/// one. An order that shows a frame twice (loop, ping-pong) can't: those files
/// are numbered from `frame_0000.png` in playing order, and each repeat is a
/// copy of the file already written for that frame.
pub fn export_to(project: &Project, dir: &PathBuf, order: &[usize]) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {dir:?}"))?;
    let sequential = repeats(order);
    let mut written: HashMap<usize, PathBuf> = HashMap::new();
    for (i, &f) in order.iter().enumerate() {
        let path = dir.join(format!("frame_{:04}.png", if sequential { i } else { f }));
        if let Some(first) = written.get(&f) {
            std::fs::copy(first, &path).with_context(|| format!("writing {path:?}"))?;
            continue;
        }
        let flat = composite::flatten_frame(project, f);
        let buf: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(flat.width, flat.height, flat.pixels())
                .context("buffer/dim mismatch")?;
        buf.save(&path)
            .with_context(|| format!("writing {path:?}"))?;
        written.insert(f, path);
    }
    log::info!("Exported {} PNG frames → {}", order.len(), dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::frame_order::{frame_order, Playback};

    fn project() -> Project {
        let mut p = Project::new(8, 6, 24.0);
        p.ensure_frame_count(3);
        for f in 0..3 {
            let id = p.cells.len();
            let mut c = crate::doc::canvas::Canvas::new(8, 6);
            c.pixels_mut()[0..4].copy_from_slice(&[f as u8 * 80, 0, 0, 255]);
            p.cells.push(c.into());
            p.layers[0].exposures[f] = Some(id);
        }
        p
    }

    fn names(dir: &PathBuf) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("animator-pngseq-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn plain_range_keeps_absolute_names() {
        let dir = tmp("plain");
        let order = frame_order(1, 2, Playback::default(), 24.0);
        export_to(&project(), &dir, &order).unwrap();
        assert_eq!(names(&dir), ["frame_0001.png", "frame_0002.png"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn looped_order_numbers_sequentially_and_copies_repeats() {
        let dir = tmp("loop");
        // Ping-pong of 0..=2 is 0 1 2 1; two cycles fill 8 frames (1/3 s).
        let pb = Playback { ping_pong: true, loop_secs: Some(8.0 / 24.0) };
        let order = frame_order(0, 2, pb, 24.0);
        assert_eq!(order, [0, 1, 2, 1, 0, 1, 2, 1]);
        export_to(&project(), &dir, &order).unwrap();
        let n = names(&dir);
        assert_eq!(n.len(), 8);
        assert_eq!(n[0], "frame_0000.png");
        assert_eq!(n[7], "frame_0007.png");
        let read = |i: usize| std::fs::read(dir.join(format!("frame_{i:04}.png"))).unwrap();
        assert_eq!(read(1), read(3));
        assert_eq!(read(0), read(4));
        assert_ne!(read(0), read(2));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
