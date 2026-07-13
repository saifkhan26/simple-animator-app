//! Import frames from a video by shelling out to ffmpeg / ffprobe.
//!
//! Requires `ffmpeg` and `ffprobe` to be installed and on the PATH. Frames in
//! the chosen `[start, end]` range are extracted to PNGs in a temp directory,
//! decoded into project-sized canvases, and the temp directory is removed.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use crate::doc::canvas::Canvas;

/// Build a `Command` that does not pop up a console window on Windows
/// (`CREATE_NO_WINDOW`), so per-frame ffmpeg calls don't flash a terminal.
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

/// Probe the first video stream for `(frame_count, fps)`. Tries fast metadata
/// (`nb_frames`, or `duration × r_frame_rate`) and only falls back to the slow
/// exact frame count if metadata is missing. Run this off the UI thread.
pub fn probe(path: &Path) -> Result<(usize, f64)> {
    let out = cmd("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=r_frame_rate,nb_frames,duration",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path)
        .output()
        .context("running ffprobe (is ffmpeg installed and on PATH?)")?;

    if !out.status.success() {
        return Err(anyhow!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let s = String::from_utf8_lossy(&out.stdout);
    let mut fps = 0.0f64;
    let mut nb = 0usize;
    let mut dur = 0.0f64;
    for line in s.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("r_frame_rate=") {
            fps = parse_rational(v);
        } else if let Some(v) = line.strip_prefix("nb_frames=") {
            nb = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("duration=") {
            dur = v.trim().parse().unwrap_or(0.0);
        }
    }

    let count = if nb > 0 {
        nb
    } else if dur > 0.0 && fps > 0.0 {
        (dur * fps).round() as usize
    } else {
        // Metadata insufficient — fall back to the exact (slow) count.
        count_frames(path)?
    };
    Ok((count, fps))
}

fn parse_rational(s: &str) -> f64 {
    let s = s.trim();
    if let Some((n, d)) = s.split_once('/') {
        let n: f64 = n.parse().unwrap_or(0.0);
        let d: f64 = d.parse().unwrap_or(1.0);
        if d != 0.0 {
            return n / d;
        }
        return 0.0;
    }
    s.parse().unwrap_or(0.0)
}

/// Exact frame count via `-count_frames` (decodes the whole stream — slow).
fn count_frames(path: &Path) -> Result<usize> {
    let out = cmd("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=nb_read_frames",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .context("running ffprobe -count_frames")?;
    if !out.status.success() {
        return Err(anyhow!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let first = stdout.lines().next().unwrap_or("").trim();
    first
        .parse()
        .with_context(|| format!("parsing frame count from ffprobe output: {stdout:?}"))
}

/// Extract a single frame near index `frame` (fast timestamp seek) for the range
/// dialog preview. Returns `(width, height, rgba8)` downscaled for display. Run
/// this off the UI thread.
pub fn extract_preview(path: &Path, frame: usize, fps: f64) -> Result<(u32, u32, Vec<u8>)> {
    let secs = if fps > 0.0 { frame as f64 / fps } else { 0.0 };
    let tmp = unique_temp_dir()?;
    let _cleanup = TempDir(tmp.clone());
    let out_png = tmp.join("preview.png");

    let out = cmd("ffmpeg")
        .args(["-y", "-ss", &format!("{secs}")])
        .arg("-i")
        .arg(path)
        .args(["-frames:v", "1"])
        .arg(&out_png)
        .output()
        .context("running ffmpeg for preview")?;
    if !out.status.success() {
        return Err(anyhow!(
            "ffmpeg preview failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let img = image::ImageReader::open(&out_png)
        .context("opening preview frame")?
        .decode()
        .context("decoding preview frame")?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let max = 360u32;
    let (pw, ph) = if w > max {
        (max, (h * max / w).max(1))
    } else {
        (w, h)
    };
    let buf = if pw != w {
        image::imageops::resize(&img, pw, ph, image::imageops::FilterType::Triangle).into_raw()
    } else {
        img.into_raw()
    };
    Ok((pw, ph, buf))
}

/// Extract source frames `[start, end]` (inclusive) via ffmpeg, decode them into
/// native-resolution canvases, and return them in order. The temp directory is
/// cleaned up on return (including on error).
pub fn extract_frames(path: &Path, start: usize, end: usize) -> Result<Vec<Canvas>> {
    let tmp = unique_temp_dir()?;
    let _cleanup = TempDir(tmp.clone());

    let select = format!("select=between(n\\,{start}\\,{end})");
    let out_pat = tmp.join("f_%06d.png");

    let out = cmd("ffmpeg")
        .arg("-i")
        .arg(path)
        .args(["-vf", &select, "-vsync", "0"])
        .arg(&out_pat)
        .output()
        .context("running ffmpeg (is it installed and on PATH?)")?;

    if !out.status.success() {
        return Err(anyhow!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&tmp)
        .with_context(|| format!("reading {tmp:?}"))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        })
        .collect();
    entries.sort();

    if entries.is_empty() {
        return Err(anyhow!(
            "ffmpeg produced no frames for range {start}..={end}"
        ));
    }

    let mut cells = Vec::with_capacity(entries.len());
    for p in &entries {
        let img = image::ImageReader::open(p)
            .with_context(|| format!("opening {p:?}"))?
            .decode()
            .with_context(|| format!("decoding {p:?}"))?
            .to_rgba8();
        let (w, h) = img.dimensions();
        let mut canvas = Canvas::new(w, h);
        canvas.pixels = img.into_raw();
        cells.push(canvas);
    }

    Ok(cells)
}

fn unique_temp_dir() -> Result<PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("animator_vidimport_{nanos}"));
    std::fs::create_dir_all(&dir).with_context(|| format!("creating temp dir {dir:?}"))?;
    Ok(dir)
}

/// RAII guard that removes the temp extraction directory when dropped.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
