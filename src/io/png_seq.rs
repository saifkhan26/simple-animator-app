//! Export each frame of the project as `frame_NNNN.png` into a chosen folder.
//! Flattens visible non-reference layers per frame.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use image::{ImageBuffer, Rgba};

use crate::doc::camera::Camera;
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
///
/// `cams`, when the camera's move is stretched over loops, is the camera each
/// frame of `order` is seen through. A frame that comes round again is then
/// a picture of its own: numbered by position, and no copy of the last one.
pub fn export_to(project: &Project, dir: &PathBuf, order: &[usize], cams: Option<&[Camera]>) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {dir:?}"))?;
    let sequential = cams.is_some() || repeats(order);
    let mut written: HashMap<usize, PathBuf> = HashMap::new();
    for (i, &f) in order.iter().enumerate() {
        let path = dir.join(format!("frame_{:04}.png", if sequential { i } else { f }));
        if let Some(first) = written.get(&f).filter(|_| cams.is_none()) {
            std::fs::copy(first, &path).with_context(|| format!("writing {path:?}"))?;
            continue;
        }
        let flat = composite::flatten_through(project, f, cams.map(|c| c[i]));
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
        export_to(&project(), &dir, &order, None).unwrap();
        assert_eq!(names(&dir), ["frame_0001.png", "frame_0002.png"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frames_come_out_at_the_camera_s_size() {
        let dir = tmp("camsize");
        let mut p = project();
        p.camera_opts.size = Some((4, 2));
        export_to(&p, &dir, &[0], None).unwrap();
        let img = image::open(dir.join("frame_0000.png")).unwrap();
        assert_eq!((img.width(), img.height()), (4, 2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A camera a frame: the same drawing coming round again is a different
    /// picture, written out in full rather than copied.
    #[test]
    fn frames_under_their_own_cameras_are_all_their_own_pictures() {
        use crate::doc::camera::Camera;
        let dir = tmp("cams");
        // Frames 1 and 2 twice over, the camera a pixel further right each time.
        let order = [1, 2, 1, 2];
        let cams: Vec<Camera> = (0..4)
            .map(|i| Camera {
                tx: i as f32,
                ..Default::default()
            })
            .collect();
        export_to(&project(), &dir, &order, Some(&cams)).unwrap();
        assert_eq!(
            names(&dir),
            ["frame_0000.png", "frame_0001.png", "frame_0002.png", "frame_0003.png"]
        );
        let read = |i: usize| image::open(dir.join(format!("frame_{i:04}.png"))).unwrap().to_rgba8();
        // The project's one painted pixel is its first; a camera to the right
        // of it has it off the frame's left edge.
        assert_eq!(read(0).get_pixel(0, 0).0, [80, 0, 0, 255], "frame 1, camera at rest");
        assert_eq!(read(1).get_pixel(0, 0).0[3], 0, "frame 2, a pixel over");
        assert_ne!(read(0), read(2), "frame 1 again, but two pixels over");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn looped_order_numbers_sequentially_and_copies_repeats() {
        let dir = tmp("loop");
        // Ping-pong of 0..=2 is 0 1 2 1; two cycles fill 8 frames (1/3 s).
        let pb = Playback { ping_pong: true, loop_secs: Some(8.0 / 24.0) };
        let order = frame_order(0, 2, pb, 24.0);
        assert_eq!(order, [0, 1, 2, 1, 0, 1, 2, 1]);
        export_to(&project(), &dir, &order, None).unwrap();
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
