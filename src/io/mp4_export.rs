//! Export the project as an H.264 MP4 by shelling out to `ffmpeg`.
//!
//! Requires `ffmpeg` on the PATH (the same dependency as video import). Each
//! frame is flattened, composited over black into a packed RGB24 buffer, and
//! piped to ffmpeg's stdin as raw video; ffmpeg encodes it with libx264.
//!
//! MP4 has no alpha, so transparent areas become black. The flatten buffer is
//! *unmultiplied* RGBA, so we premultiply (`rgb * a / 255`) before dropping the
//! alpha — otherwise semi-transparent pixels would encode too bright.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};

use crate::doc::project::Project;
use crate::io::composite;

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

/// Encode every frame of `project` to an MP4 at `path`. Blocking — run on a
/// worker thread (see `AppState::start_mp4_export`).
pub fn export_to(project: &Project, path: &Path, settings: &Mp4Settings) -> Result<()> {
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
    let mut rgb = vec![0u8; (w as usize) * (h as usize) * 3];
    let write_result = (|| -> Result<()> {
        for f in 0..project.frame_count {
            let flat = composite::flatten_frame(project, f);
            rgba_over_black(&flat.pixels, &mut rgb);
            stdin
                .write_all(&rgb)
                .with_context(|| format!("writing frame {f} to ffmpeg"))?;
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
        project.frame_count,
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
