//! Export the project as an animated WebP.
//!
//! Lossless, so every frame keeps its exact colours and its 8-bit alpha,
//! where a GIF has 256 colours and on/off transparency. `image-webp` encodes
//! each frame (VP8L); it writes still images only, so the animation container
//! around the frames — `VP8X`, `ANIM`, one `ANMF` per frame — is written here.
//!
//! Two things keep the file small, and neither changes a pixel:
//! - a frame stores only the rectangle that changed since the frame before,
//!   and replaces those pixels rather than blending over them, so a
//!   half-transparent pixel lands exactly as it is;
//! - a frame the same as the one before isn't stored: the one before shows
//!   for longer. A drawing held on twos costs one frame, not two.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use image_webp::{ColorType, WebPEncoder};

use crate::doc::camera::Camera;
use crate::doc::project::Project;
use crate::io::{composite, frame_order};

/// The widest and tallest a VP8L frame can be.
const MAX_SIDE: u32 = 16384;

/// Flattened frames kept for reuse by a ping-pong order, as MP4 export has
/// it. Past this, a repeat is flattened again instead.
const REUSE_BUDGET: usize = 1 << 30;

/// Longest a frame can show: the `ANMF` duration is 24 bits of milliseconds.
const MAX_DURATION: u32 = (1 << 24) - 1;

/// Write the timeline frames in `order` (see [`crate::io::frame_order`]) as
/// one animated WebP. `cams`, when the camera's move is stretched over loops,
/// is the camera each of them is seen through. `plays` is how many times a
/// player shows it; `0` = forever.
pub fn export_to(
    project: &Project,
    path: &Path,
    order: &[usize],
    cams: Option<&[Camera]>,
    plays: u16,
) -> Result<()> {
    let (w, h) = project.output_size();
    if w > MAX_SIDE || h > MAX_SIDE {
        bail!("a WebP frame is at most {MAX_SIDE} px a side; this export is {w} x {h}");
    }
    let file = File::create(path).with_context(|| format!("creating {path:?}"))?;
    let mut anim = AnimWriter::new(BufWriter::new(file), w, h, plays)?;

    // When output frame `i` starts showing, in ms. Rounded from the exact
    // time rather than adding up a rounded delay, so 24 fps doesn't drift.
    let fps = f64::from(project.fps.max(1.0));
    let at = |i: usize| (i as f64 * 1000.0 / fps).round() as u32;

    // A frame that comes round again (ping-pong) is flattened once and kept
    // until its last use.
    let frame_bytes = w as usize * h as usize * 4;
    let mut left = frame_order::uses(order);
    let mut kept: HashMap<usize, Vec<u8>> = HashMap::new();
    // Under a camera of its own a frame is its own picture: none is kept for
    // the next time its drawings show.
    let reuse = cams.is_none();
    for (i, &f) in order.iter().enumerate() {
        let remaining = left.get_mut(&f).map_or(0, |n| {
            *n -= 1;
            *n
        });
        match kept.get(&f).filter(|_| reuse) {
            Some(px) => anim.frame(px, at(i))?,
            None => {
                let flat = composite::flatten_through(project, f, cams.map(|c| c[i]));
                let px = flat.pixels();
                anim.frame(&px, at(i))?;
                if reuse && remaining > 0 && (kept.len() + 1) * frame_bytes <= REUSE_BUDGET {
                    kept.insert(f, px.into_owned());
                }
            }
        }
        if remaining == 0 {
            kept.remove(&f);
        }
    }
    anim.finish(at(order.len()))?;

    log::info!("Exported WebP → {}", path.display());
    Ok(())
}

/// A frame stored but not yet written: how long it shows is only known once
/// a frame that differs from it comes along, or the animation ends.
struct Pending {
    /// Where its rectangle sits on the canvas, and its size.
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    /// When it starts showing, in ms.
    start: u32,
    /// Its `VP8L` chunk, header and padding included.
    vp8l: Vec<u8>,
}

/// An animated WebP, written a frame at a time.
struct AnimWriter<W: Write + Seek> {
    out: W,
    w: u32,
    h: u32,
    /// The canvas as the stored frames so far leave it. Empty before the
    /// first frame.
    shown: Vec<u8>,
    pending: Option<Pending>,
    /// Whether any stored pixel is less than opaque — the `VP8X` alpha flag,
    /// which decoders read to decide whether to keep alpha at all.
    alpha: bool,
}

impl<W: Write + Seek> AnimWriter<W> {
    /// Writes the headers. The RIFF size and the `VP8X` flags are patched in
    /// by [`AnimWriter::finish`], once they are known.
    fn new(mut out: W, w: u32, h: u32, plays: u16) -> Result<Self> {
        out.write_all(b"RIFF")?;
        out.write_all(&0u32.to_le_bytes())?;
        out.write_all(b"WEBP")?;

        out.write_all(b"VP8X")?;
        out.write_all(&10u32.to_le_bytes())?;
        out.write_all(&[0; 4])?; // flags, then 3 reserved bytes
        out.write_all(&u24(w - 1))?;
        out.write_all(&u24(h - 1))?;

        out.write_all(b"ANIM")?;
        out.write_all(&6u32.to_le_bytes())?;
        // Background colour, BGRA: only a hint, and browsers show
        // transparent whatever it says. Say transparent too.
        out.write_all(&[0; 4])?;
        out.write_all(&plays.to_le_bytes())?;

        Ok(Self { out, w, h, shown: Vec::new(), pending: None, alpha: false })
    }

    /// Add `rgba`, a whole frame, starting to show at `start` ms.
    fn frame(&mut self, rgba: &[u8], start: u32) -> Result<()> {
        let (x, y, w, h) = if self.shown.is_empty() {
            self.shown.resize(rgba.len(), 0);
            (0, 0, self.w, self.h)
        } else {
            let Some((x0, y0, x1, y1)) = changed(&self.shown, rgba, self.w as usize) else {
                return Ok(()); // the frame before shows for longer
            };
            // A frame's offset is stored halved: it starts on an even pixel.
            let (x0, y0) = (x0 & !1, y0 & !1);
            (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32)
        };

        let mut rect = Vec::with_capacity(w as usize * h as usize * 4);
        let (stride, row) = (self.w as usize * 4, w as usize * 4);
        for r in y as usize..(y + h) as usize {
            let at = r * stride + x as usize * 4;
            rect.extend_from_slice(&rgba[at..at + row]);
            self.shown[at..at + row].copy_from_slice(&rgba[at..at + row]);
        }
        self.alpha = self.alpha || rect.chunks_exact(4).any(|p| p[3] != 255);
        let vp8l = vp8l(&rect, w, h)?;

        self.flush(start)?;
        self.pending = Some(Pending { x, y, w, h, start, vp8l });
        Ok(())
    }

    /// Write the pending frame, showing until `end` ms.
    fn flush(&mut self, end: u32) -> Result<()> {
        let Some(p) = self.pending.take() else {
            return Ok(());
        };
        let duration = end.saturating_sub(p.start).clamp(1, MAX_DURATION);
        let out = &mut self.out;
        out.write_all(b"ANMF")?;
        out.write_all(&(16 + p.vp8l.len() as u32).to_le_bytes())?;
        out.write_all(&u24(p.x / 2))?;
        out.write_all(&u24(p.y / 2))?;
        out.write_all(&u24(p.w - 1))?;
        out.write_all(&u24(p.h - 1))?;
        out.write_all(&u24(duration))?;
        // Don't blend: the rectangle replaces what was there, alpha and all.
        // Blending would let unchanged pixels go clear and pack smaller, as
        // libwebp does, but not every decoder keeps an opaque pixel as it is
        // (image-webp blends 255 over anything to 254).
        // Don't dispose: the next frame changes only what it stores.
        out.write_all(&[0b10])?;
        out.write_all(&p.vp8l)?;
        Ok(())
    }

    /// Write the last frame, showing until `end` ms, and patch the headers.
    fn finish(mut self, end: u32) -> Result<()> {
        ensure!(self.pending.is_some(), "no frames to export");
        self.flush(end)?;
        let len = self.out.stream_position()?;
        let riff = u32::try_from(len - 8).context("the WebP is over 4 GB")?;
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&riff.to_le_bytes())?;
        // VP8X flags: animation, and alpha when any pixel needs it.
        self.out.seek(SeekFrom::Start(20))?;
        self.out.write_all(&[0b10 | if self.alpha { 0b1_0000 } else { 0 }])?;
        self.out.flush()?;
        Ok(())
    }
}

/// The rectangle of pixels that differ between two RGBA frames `width`
/// pixels wide, as `(x0, y0, x1, y1)`, exclusive at the far end. `None` when
/// the frames are the same.
fn changed(a: &[u8], b: &[u8], width: usize) -> Option<(usize, usize, usize, usize)> {
    let stride = width * 4;
    let (mut x0, mut x1) = (width, 0);
    let (mut y0, mut y1) = (usize::MAX, 0);
    for (y, (ra, rb)) in a.chunks_exact(stride).zip(b.chunks_exact(stride)).enumerate() {
        if ra == rb {
            continue;
        }
        let differs = |&i: &usize| ra[i * 4..i * 4 + 4] != rb[i * 4..i * 4 + 4];
        let first = (0..width).find(differs).unwrap_or(0);
        let last = (0..width).rev().find(differs).unwrap_or(0);
        x0 = x0.min(first);
        x1 = x1.max(last + 1);
        y0 = y0.min(y);
        y1 = y + 1;
    }
    (y0 != usize::MAX).then_some((x0, y0, x1, y1))
}

/// `rgba` as a lossless `VP8L` chunk, header and padding included.
fn vp8l(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let mut file = Vec::new();
    WebPEncoder::new(&mut file)
        .encode(rgba, w, h, ColorType::Rgba8)
        .context("encoding a WebP frame")?;
    // With no metadata image-webp writes the simple container: a 12-byte
    // RIFF header, then the one chunk.
    let chunk = file.split_off(12);
    ensure!(chunk.starts_with(b"VP8L"), "image-webp wrote an unexpected container");
    Ok(chunk)
}

fn u24(v: u32) -> [u8; 3] {
    let b = v.to_le_bytes();
    [b[0], b[1], b[2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::canvas::Canvas;

    fn tmp(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("animator-webp-{tag}-{}.webp", std::process::id()))
    }

    /// An 8×6 project, `frames` long, with a drawing keyed on each of `keys`:
    /// opaque, half-transparent and clear pixels, different on every key.
    fn project(frames: usize, keys: &[usize]) -> Project {
        let mut p = Project::new(8, 6, 24.0);
        p.ensure_frame_count(frames);
        for (n, &f) in keys.iter().enumerate() {
            let mut c = Canvas::new(8, 6);
            let px = c.pixels_mut();
            for (i, p) in px.chunks_exact_mut(4).enumerate() {
                let a = [255, 128, 0][(i + n) % 3];
                p.copy_from_slice(&[(i * 9 + n * 40) as u8, (n * 70) as u8, i as u8, a]);
            }
            // Something that moves, so frames differ in a small rectangle.
            px[(n * 4 + 8) * 4..(n * 4 + 8) * 4 + 4].copy_from_slice(&[1, 2, 3, 255]);
            let id = p.cells.len();
            p.cells.push(c.into());
            p.layers[0].set_key(f, id);
        }
        p
    }

    /// Every frame of the WebP at `path`, as players show them, with how
    /// long each shows; and how many times it plays (0 = forever).
    fn decode(path: &Path) -> (Vec<(Vec<u8>, u32)>, u16) {
        let file = std::io::BufReader::new(File::open(path).unwrap());
        let mut d = image_webp::WebPDecoder::new(file).unwrap();
        assert!(d.is_animated());
        assert!(d.has_alpha(), "the alpha flag, or decoders drop alpha");
        d.set_background_color([0; 4]).unwrap();
        let plays = match d.loop_count() {
            image_webp::LoopCount::Forever => 0,
            image_webp::LoopCount::Times(n) => n.get(),
        };
        let mut frames = Vec::new();
        let mut buf = vec![0; d.output_buffer_size().unwrap()];
        while let Ok(ms) = d.read_frame(&mut buf) {
            frames.push((buf.clone(), ms));
        }
        (frames, plays)
    }

    #[test]
    fn frames_come_back_exactly_with_holds_merged() {
        // Keys on 0, 1 and 3: frame 2 holds frame 1's drawing.
        let p = project(4, &[0, 1, 3]);
        let path = tmp("exact");
        export_to(&p, &path, &[0, 1, 2, 3], None, 0).unwrap();
        let (frames, plays) = decode(&path);
        let _ = std::fs::remove_file(&path);

        assert_eq!(plays, 0, "forever");
        let want: Vec<Vec<u8>> =
            [0, 1, 3].iter().map(|&f| composite::flatten_frame(&p, f).pixels().into_owned()).collect();
        assert_eq!(frames.len(), 3, "the held frame folds into the one before");
        for (i, ((got, _), want)) in frames.iter().zip(&want).enumerate() {
            assert!(got == want, "frame {i} differs");
        }
        // 24 fps: frames start at 0, 42, 83 and 125 ms and the last ends at
        // 167. Frame 1 shows on through the hold, until 125.
        let ms: Vec<u32> = frames.iter().map(|f| f.1).collect();
        assert_eq!(ms, [42, 83, 42]);
        assert_eq!(ms.iter().sum::<u32>(), 167, "4 frames at 24 fps, no drift");
    }

    #[test]
    fn a_later_frame_stores_only_what_changed() {
        let a = vec![0u8; 8 * 6 * 4];
        let mut b = a.clone();
        b[(2 * 8 + 3) * 4] = 9; // (3, 2)
        b[(4 * 8 + 5) * 4 + 3] = 9; // (5, 4)
        assert_eq!(changed(&a, &b, 8), Some((3, 2, 6, 5)));
        assert_eq!(changed(&a, &a, 8), None);
    }

    #[test]
    fn ping_pong_and_play_count() {
        let p = project(3, &[0, 1, 2]);
        let path = tmp("pingpong");
        export_to(&p, &path, &[0, 1, 2, 1], None, 3).unwrap();
        let (frames, plays) = decode(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(plays, 3);
        assert_eq!(frames.len(), 4);
        let back = composite::flatten_frame(&p, 1);
        assert!(frames[3].0 == *back.pixels(), "the way back shows frame 1 again");
    }

    #[test]
    fn a_webp_is_the_camera_s_size() {
        let mut p = project(1, &[0]);
        p.camera_opts.size = Some((4, 2));
        let path = tmp("cam");
        export_to(&p, &path, &[0], None, 0).unwrap();
        let d = image_webp::WebPDecoder::new(std::io::BufReader::new(File::open(&path).unwrap())).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(d.dimensions(), (4, 2));
    }

    /// Size and time against GIF on a real project. Not a pass/fail test —
    /// run it for the numbers:
    /// `ANIM_PROBE=path/to/project.anim cargo test probe_webp_against_gif -- --ignored --nocapture`
    /// Leaves `animator-probe.webp` and `.gif` in the temp folder.
    #[test]
    #[ignore]
    fn probe_webp_against_gif() {
        use std::time::Instant;
        let src = std::env::var("ANIM_PROBE").expect("ANIM_PROBE=<path to a .anim>");
        let p = crate::io::project_file::load_from(Path::new(&src)).unwrap();
        let order: Vec<usize> = (0..p.frame_count).collect();
        let (w, h) = p.output_size();
        eprintln!("{src}: {w} x {h}, {} frames at {} fps", order.len(), p.fps);
        let dir = std::env::temp_dir();
        let size = |path: &Path| std::fs::metadata(path).unwrap().len();

        let webp = dir.join("animator-probe.webp");
        let start = Instant::now();
        export_to(&p, &webp, &order, None, 0).unwrap();
        eprintln!("WebP {:>12} bytes {:>8.0} ms", size(&webp), start.elapsed().as_secs_f64() * 1e3);

        let gif = dir.join("animator-probe.gif");
        let start = Instant::now();
        crate::io::gif_export::export_to(&p, &gif, &order, None).unwrap();
        eprintln!("GIF  {:>12} bytes {:>8.0} ms", size(&gif), start.elapsed().as_secs_f64() * 1e3);
    }

    #[test]
    fn an_opaque_animation_says_so() {
        let mut p = Project::new(4, 4, 12.0);
        let mut c = Canvas::new(4, 4);
        c.pixels_mut().chunks_exact_mut(4).for_each(|px| px.copy_from_slice(&[10, 20, 30, 255]));
        p.cells[0] = c.into();
        let path = tmp("opaque");
        export_to(&p, &path, &[0], None, 0).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(bytes[20], 0b10, "animation, no alpha");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len() - 8);
    }
}
