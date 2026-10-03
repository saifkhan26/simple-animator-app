//! Export the project as an H.264 MP4 by shelling out to `ffmpeg`.
//!
//! Requires `ffmpeg` on the PATH (the same dependency as video import). Each
//! frame is flattened, composited over black into a packed RGB24 buffer, and
//! piped to ffmpeg's stdin as raw video; ffmpeg encodes it with libx264.
//!
//! MP4 has no alpha, so transparent areas become black. The flatten buffer is
//! *unmultiplied* RGBA, so we premultiply (`rgb * a / 255`) before dropping the
//! alpha — otherwise semi-transparent pixels would encode too bright.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};

use crate::doc::project::Project;
use crate::io::{composite, frame_order};

/// Encoder settings chosen in the export dialog.
pub struct Mp4Settings {
    /// libx264 Constant Rate Factor: 0..=51, lower = better quality / bigger file.
    pub crf: u32,
    /// libx264 preset (`ultrafast`..`veryslow`): speed/compression trade-off.
    pub preset: &'static str,
}

/// Build a `Command` that does not pop up a console window on Windows
/// (`CREATE_NO_WINDOW`), so the ffmpeg subprocess doesn't flash a terminal.
fn cmd(program: &str) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Flattened frames kept for reuse by a looping or ping-pong order. Past
/// this, a repeat is flattened again instead — slower, never out of memory.
const REUSE_BUDGET: usize = 1 << 30;

/// Encode the timeline frames in `order` (see [`crate::io::frame_order`]) to an
/// MP4 at `path`. Blocking — run on a worker thread (see
/// `AppState::start_export`).
pub fn export_to(
    project: &Project,
    path: &Path,
    settings: &Mp4Settings,
    order: &[usize],
) -> Result<()> {
    let w = project.width;
    let h = project.height;
    // Even dimensions are required by yuv420p; pad odd sizes up by a pixel.
    let pad = "pad=ceil(iw/2)*2:ceil(ih/2)*2";
    let size = format!("{w}x{h}");
    let fps = format!("{}", project.fps.max(1.0));
    let crf = settings.crf.to_string();

    let mut child = cmd("ffmpeg")
        .args(["-y", "-f", "rawvideo", "-pixel_format", "rgb24"])
        .args(["-video_size", &size, "-framerate", &fps])
        .args(["-i", "-", "-an", "-vf", pad])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .args(["-crf", &crf, "-preset", settings.preset])
        .args(["-movflags", "+faststart"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning ffmpeg (is it installed and on PATH?)")?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("ffmpeg stdin unavailable"))?;

    // Drain stderr on a separate thread so a full pipe can't deadlock us while
    // we're busy writing frames to stdin.
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("ffmpeg stderr unavailable"))?;
    let err_handle = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    // Stream frames. Feed rgb24 (3 bytes/px) composited over black.
    let frame_bytes = (w as usize) * (h as usize) * 3;
    let mut rgb = vec![0u8; frame_bytes];
    let write_result = (|| -> Result<()> {
        // A frame that comes round again (loop, ping-pong) is flattened once
        // and kept until its last use.
        let mut left = frame_order::uses(order);
        let mut kept: HashMap<usize, Vec<u8>> = HashMap::new();
        for &f in order {
            let remaining = left.get_mut(&f).map_or(0, |n| {
                *n -= 1;
                *n
            });
            let res = match kept.get(&f) {
                Some(buf) => stdin.write_all(buf),
                None => {
                    let flat = composite::flatten_frame(project, f);
                    rgba_over_black(&flat.pixels(), &mut rgb);
                    if remaining > 0 && (kept.len() + 1) * frame_bytes <= REUSE_BUDGET {
                        kept.insert(f, rgb.clone());
                    }
                    stdin.write_all(&rgb)
                }
            };
            res.with_context(|| format!("writing frame {f} to ffmpeg"))?;
            if remaining == 0 {
                kept.remove(&f);
            }
        }
        Ok(())
    })();
    // Close stdin so ffmpeg flushes and exits, even if a write failed midway.
    drop(stdin);

    let status = child.wait().context("waiting for ffmpeg")?;
    let log = err_handle.join().unwrap_or_default();

    // Surface a frame-write error (e.g. ffmpeg died early) with ffmpeg's own log.
    write_result.with_context(|| format!("ffmpeg log:\n{}", log.trim()))?;

    if !status.success() {
        return Err(anyhow!("ffmpeg failed: {}", log.trim()));
    }

    log::info!(
        "Exported MP4 ({} frames, {w}x{h} @ {fps}fps) → {}",
        order.len(),
        path.display()
    );
    Ok(())
}

/// Composite an unmultiplied RGBA buffer over black into a packed RGB24 buffer.
/// `dst.len()` must be `src.len() / 4 * 3`.
fn rgba_over_black(src: &[u8], dst: &mut [u8]) {
    for (px, out) in src.chunks_exact(4).zip(dst.chunks_exact_mut(3)) {
        let a = px[3] as u32;
        out[0] = (px[0] as u32 * a / 255) as u8;
        out[1] = (px[1] as u32 * a / 255) as u8;
        out[2] = (px[2] as u32 * a / 255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::frame_order::{frame_order, Playback};

    /// Not a unit test: needs ffmpeg on PATH. Writes `loop.mp4` (21 frames
    /// looped to 10 s) and `pingpong.mp4` (ping-ponged to 10 s) into
    /// `$MP4_OUT`, for ffprobe to count.
    #[test]
    #[ignore]
    fn mp4_loop_fixture() {
        let dir = std::path::PathBuf::from(std::env::var("MP4_OUT").unwrap());
        let mut p = Project::new(64, 36, 24.0);
        p.ensure_frame_count(21);
        for f in 0..21usize {
            let id = p.cells.len();
            let mut c = crate::doc::canvas::Canvas::new(64, 36);
            // A block that walks right, one column per frame.
            for y in 10..26 {
                for x in f * 2..f * 2 + 8 {
                    let o = (y * 64 + x) * 4;
                    c.pixels_mut()[o..o + 4].copy_from_slice(&[240, 200, 40, 255]);
                }
            }
            p.cells.push(c.into());
            p.layers[0].exposures[f] = Some(id);
        }
        let settings = Mp4Settings { crf: 18, preset: "ultrafast" };
        for (name, ping_pong) in [("loop", false), ("pingpong", true)] {
            let pb = Playback { ping_pong, loop_secs: Some(10.0) };
            let order = frame_order(0, 20, pb, 24.0);
            export_to(&p, &dir.join(format!("{name}.mp4")), &settings, &order).unwrap();
        }
    }
}
