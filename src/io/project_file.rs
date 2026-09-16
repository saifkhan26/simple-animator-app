//! Native project save / load (`.anim`).
//!
//! The whole `Project` (cells + layers + exposures + timeline metadata) is
//! serialised with `postcard` behind a small magic + version header so old/
//! foreign files are rejected cleanly and the format can evolve.
//!
//! Since v6 the postcard body is deflated. Cells are RGBA8 and line art is
//! mostly transparent — four zero bytes per pixel — so the payload is long runs
//! of zeros, which compresses by one to two orders of magnitude. Deflate is
//! lossless, so the pixels that come back are bit-identical to the ones saved.

use std::io::{BufWriter, Read};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::doc::project::Project;

const MAGIC: &[u8; 4] = b"ANIM";
// v2 adds per-layer transform + transform keyframes to each Layer. postcard is
// positional, so v1 files can't be read by this build — rejected cleanly below.
// v3 appends per-layer stabilization `track_points`; v2 files are migrated via
// the mirror structs in `crate::io::legacy`.
// v4 appends the project camera + camera keys, and the per-layer cell size.
// v5 appends `ease` to each layer transform key, matching camera keys.
// v6 changes the *framing* only — the body is now zlib-deflated. The postcard
// layout is identical to v5, so both versions decode into the same `Project`
// and only the inflate step differs.
const VERSION: u32 = 6;
const EXT: &str = "anim";

/// Compression level. Deliberately [`Compression::fast`] (level 1) rather than
/// the default 6: the payload is mostly runs of zeros, where the match finder
/// hits long matches straight away, so level 1 gets nearly all of level 6's
/// ratio for a fraction of the CPU. Saving a few hundred MB must not stall on
/// the compressor.
fn level() -> Compression {
    Compression::fast()
}

/// Input buffer between postcard's byte-at-a-time writes and the compressor.
/// Generous on purpose — the cost is one allocation per save, and the win is
/// the difference between a save you notice and one you don't.
const BUF: usize = 256 * 1024;

/// Encode a project into the on-disk byte layout:
/// `MAGIC | version_le | zlib(postcard)`.
///
/// Returns the bytes and the uncompressed body length, so the caller can log
/// the ratio it actually achieved.
fn encode(project: &Project) -> Result<(Vec<u8>, u64)> {
    // The header is written into the buffer *first* and the encoder appends
    // after it, so the compressed body lands in place with no second copy.
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());

    // `to_io` rather than `to_stdvec`: serialising to a `Vec` first would
    // materialise the whole uncompressed project — hundreds of MB on exactly
    // the files this compression exists for — before a single byte was
    // deflated. Streaming into the encoder keeps one buffer, not two.
    //
    // The `BufWriter` is not optional. postcard writes a `Vec<u8>` one byte at
    // a time, and `ZlibEncoder` does no input buffering of its own, so without
    // it every pixel byte makes its own trip through the deflate state machine
    // — measured at 3 MiB/s, about thirty times slower than with it.
    let enc = ZlibEncoder::new(out, level());
    let w = postcard::to_io(project, BufWriter::with_capacity(BUF, enc))
        .context("serialising project")?;
    // `into_inner` flushes on the way out, so the tail of the buffer reaches
    // the compressor; a write error surfaces here rather than being swallowed
    // by a `Drop`.
    let enc = w
        .into_inner()
        .map_err(|e| anyhow!("flushing project buffer: {}", e.error()))?;
    let raw_len = enc.total_in();
    let out = enc.finish().context("compressing project")?;
    Ok((out, raw_len))
}

/// Decode bytes produced by [`encode`] back into a `Project`.
fn decode(bytes: &[u8]) -> Result<Project> {
    if bytes.len() < 8 || &bytes[0..4] != MAGIC {
        bail!("not an Animator project file");
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let body = &bytes[8..];
    let project: Project = match version {
        6 => {
            let mut raw = Vec::new();
            ZlibDecoder::new(body)
                .read_to_end(&mut raw)
                .context("decompressing project body")?;
            postcard::from_bytes(&raw).context("parsing project body")?
        }
        // v5 shares v6's postcard layout; it just isn't compressed.
        5 => postcard::from_bytes(body).context("parsing project body")?,
        4 => postcard::from_bytes::<crate::io::legacy::ProjectV4>(body)
            .context("parsing v4 project body")?
            .into(),
        3 => postcard::from_bytes::<crate::io::legacy::ProjectV3>(body)
            .context("parsing v3 project body")?
            .into(),
        2 => postcard::from_bytes::<crate::io::legacy::ProjectV2>(body)
            .context("parsing v2 project body")?
            .into(),
        v => bail!("unsupported project version {v} (this build reads 2–{VERSION})"),
    };
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::legacy::{
        LayerV2, LayerV3, LayerV4, ProjectV2, ProjectV3, ProjectV4, TransformKeyV4,
    };

    /// A v2-era file (no `track_points`) must decode and migrate cleanly.
    #[test]
    fn decodes_v2_project() {
        let v2 = ProjectV2 {
            width: 4,
            height: 3,
            fps: 12.0,
            cells: vec![crate::doc::canvas::Canvas::new(4, 3)],
            layers: vec![LayerV2 {
                name: "L1".into(),
                opacity: 0.5,
                visible: true,
                locked: false,
                reference: false,
                exposures: vec![Some(0), None, None],
                transform: Default::default(),
                transform_keys: Vec::new(),
            }],
            frame_count: 3,
            current_frame: 1,
            current_layer: 0,
            loop_start: 0,
            loop_end: 3,
        };
        let body = postcard::to_stdvec(&v2).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&body);

        let p = decode(&bytes).expect("v2 decode");
        assert_eq!(p.frame_count, 3);
        assert_eq!(p.layers.len(), 1);
        assert_eq!(p.layers[0].name, "L1");
        assert_eq!(p.layers[0].exposures, vec![Some(0), None, None]);
        assert!(p.layers[0].track_points.is_empty());
    }

    /// A v3-era file (no camera, no per-layer cell size) must decode and
    /// migrate to an identity camera, which composites exactly as v3 did.
    #[test]
    fn decodes_v3_project() {
        let v3 = ProjectV3 {
            width: 4,
            height: 3,
            fps: 12.0,
            cells: vec![crate::doc::canvas::Canvas::new(4, 3)],
            layers: vec![LayerV3 {
                name: "L1".into(),
                opacity: 1.0,
                visible: true,
                locked: false,
                reference: false,
                exposures: vec![Some(0), None],
                transform: Default::default(),
                transform_keys: Vec::new(),
                track_points: vec![crate::doc::layer::TrackSample {
                    a: Some([3.0, 4.0]),
                    b: None,
                }],
            }],
            frame_count: 2,
            current_frame: 0,
            current_layer: 0,
            loop_start: 0,
            loop_end: 2,
        };
        let body = postcard::to_stdvec(&v3).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&body);

        let p = decode(&bytes).expect("v3 decode");
        assert_eq!(p.layers[0].track_points[0].a, Some([3.0, 4.0]));
        assert!(p.camera.is_identity());
        assert!(p.camera_keys.is_empty());
        assert_eq!((p.layers[0].cell_w, p.layers[0].cell_h), (0, 0));
    }

    /// A v4-era file (transform keys without `ease`) must decode with its keys
    /// intact and default to Linear, which reproduces v4 playback exactly.
    #[test]
    fn decodes_v4_project() {
        let cam = crate::doc::camera::Camera {
            tx: 7.0,
            ty: 0.0,
            zoom: 2.0,
            rot: 0.0,
        };
        let v4 = ProjectV4 {
            width: 4,
            height: 3,
            fps: 12.0,
            cells: vec![crate::doc::canvas::Canvas::new(8, 6)],
            layers: vec![LayerV4 {
                name: "L1".into(),
                opacity: 1.0,
                visible: true,
                locked: false,
                reference: false,
                exposures: vec![Some(0), None],
                transform: Default::default(),
                transform_keys: vec![TransformKeyV4 {
                    frame: 1,
                    transform: crate::doc::transform::Transform {
                        tx: 12.0,
                        ty: 0.0,
                        scale: 2.0,
                        rot: 0.0,
                    },
                }],
                track_points: Vec::new(),
                cell_w: 8,
                cell_h: 6,
            }],
            frame_count: 2,
            current_frame: 0,
            current_layer: 0,
            loop_start: 0,
            loop_end: 2,
            camera: cam,
            camera_keys: Vec::new(),
        };
        let body = postcard::to_stdvec(&v4).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&body);

        let p = decode(&bytes).expect("v4 decode");
        assert_eq!((p.layers[0].cell_w, p.layers[0].cell_h), (8, 6));
        assert_eq!(p.layers[0].transform_keys.len(), 1);
        assert_eq!(p.layers[0].transform_keys[0].transform.tx, 12.0);
        assert_eq!(
            p.layers[0].transform_keys[0].ease,
            crate::doc::transform::Ease::Linear
        );
        // The camera landed in v4, so it must survive rather than reset.
        assert_eq!(p.camera, cam);
    }

    /// Current-version round trip through encode/decode.
    #[test]
    fn round_trips_current_project() {
        let mut p = Project::new(4, 3, 12.0);
        p.layers[0].track_points = vec![crate::doc::layer::TrackSample {
            a: Some([1.0, 2.0]),
            b: None,
        }];
        p.layers[0].cell_w = 8;
        p.layers[0].cell_h = 6;
        p.camera = crate::doc::camera::Camera {
            tx: 10.0,
            ty: -5.0,
            zoom: 1.5,
            rot: 0.25,
        };
        p.camera_keys = vec![crate::doc::camera::CameraKey {
            frame: 1,
            camera: p.camera,
            ease: crate::doc::camera::Ease::Both,
        }];
        p.layers[0].set_transform_key(2, Default::default());
        p.layers[0].set_transform_key_ease(2, crate::doc::transform::Ease::Out);
        let (bytes, _) = encode(&p).unwrap();
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            VERSION,
            "encode must stamp the current version"
        );
        let q = decode(&bytes).expect("current decode");
        assert_eq!(q.layers[0].track_points.len(), 1);
        assert_eq!(q.layers[0].track_points[0].a, Some([1.0, 2.0]));
        assert_eq!((q.layers[0].cell_w, q.layers[0].cell_h), (8, 6));
        assert_eq!(q.camera, p.camera);
        assert_eq!(q.camera_keys.len(), 1);
        assert_eq!(q.camera_keys[0].ease, crate::doc::camera::Ease::Both);
        assert_eq!(
            q.layers[0].transform_keys[0].ease,
            crate::doc::transform::Ease::Out
        );
    }

    /// Fill a cell with a pattern no compressor can fake its way through, and
    /// demand every byte back. Compression is lossless, so a drawing that
    /// survives a save is bit-identical — this is the test that keeps it that
    /// way if the codec is ever swapped.
    #[test]
    fn pixels_survive_the_round_trip_exactly() {
        let mut p = Project::new(23, 17, 12.0);
        for (i, b) in p.cells[0].pixels.iter_mut().enumerate() {
            // Deliberately not a flat fill: a stride-coprime pattern with no
            // long runs would expose any lossy or truncating step.
            *b = ((i * 37 + i / 23 * 11) % 251) as u8;
        }
        let expect = p.cells[0].pixels.clone();

        let (bytes, _) = encode(&p).unwrap();
        let q = decode(&bytes).expect("decode");
        assert_eq!(q.cells.len(), 1);
        assert_eq!(
            q.cells[0].pixels, expect,
            "pixels came back changed — the round trip is not lossless"
        );
        assert_eq!((q.cells[0].width, q.cells[0].height), (23, 17));
    }

    /// An uncompressed v5 file — everything written before compression landed —
    /// must still open. Hand-built rather than produced by `encode`, because
    /// `encode` only writes the current version now.
    #[test]
    fn decodes_v5_project() {
        let mut p = Project::new(4, 3, 24.0);
        p.layers[0].name = "Old".into();
        p.layers[0].cell_w = 8;
        p.layers[0].cell_h = 6;
        p.camera_keys = vec![crate::doc::camera::CameraKey {
            frame: 0,
            camera: p.camera,
            ease: crate::doc::camera::Ease::In,
        }];
        // v5 layout is v6's; the difference is purely that it isn't deflated.
        let body = postcard::to_stdvec(&p).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&body);

        let q = decode(&bytes).expect("v5 decode");
        assert_eq!(q.layers[0].name, "Old");
        assert_eq!((q.layers[0].cell_w, q.layers[0].cell_h), (8, 6));
        assert_eq!(q.camera_keys.len(), 1);
        assert_eq!(q.fps, 24.0);
    }

    /// The point of the whole format bump. Blank cells are runs of zero bytes,
    /// so they must collapse to a tiny fraction of the raw postcard — if this
    /// ever fails, something has stopped compressing.
    #[test]
    fn compression_actually_shrinks_sparse_cells() {
        let mut p = Project::new(256, 256, 12.0);
        for _ in 0..4 {
            p.cells.push(crate::doc::canvas::Canvas::new(256, 256));
        }
        let raw = postcard::to_stdvec(&p).unwrap().len();
        let (packed, reported_raw) = encode(&p).unwrap();

        assert_eq!(
            reported_raw as usize, raw,
            "the logged raw size must be the real body size"
        );
        assert!(
            packed.len() * 100 < raw,
            "5 blank 256x256 cells packed to {} from {raw} — under 100x is not compression",
            packed.len()
        );
        // And it still reads back.
        assert_eq!(decode(&packed).unwrap().cells.len(), 5);
    }

    /// A half-copied file must fail, not panic or hand back a half-built
    /// project. Truncation is the realistic corruption for a save interrupted
    /// partway through.
    #[test]
    fn rejects_a_truncated_body() {
        let p = Project::new(8, 8, 12.0);
        let (bytes, _) = encode(&p).unwrap();
        let cut = &bytes[..bytes.len() - bytes.len().clamp(1, 16)];
        assert!(decode(cut).is_err(), "a truncated body must be an error");
        // Header intact but nothing after it at all.
        assert!(decode(&bytes[..8]).is_err(), "an empty body must be an error");
    }

    #[test]
    #[ignore = "measurement harness, not an assertion"]
    fn measure_ratio_and_time() {
        use std::time::Instant;
        // 12 cells of 1080p line art: mostly transparent, with sparse dark
        // strokes laid down in bands, which is what a real drawing looks like
        // to a compressor.
        let mut p = Project::new(1920, 1080, 24.0);
        for c in 0..12 {
            let mut canvas = crate::doc::canvas::Canvas::new(1920, 1080);
            for y in 0..1080usize {
                for x in 0..1920usize {
                    let on = (y.wrapping_mul(7) + x.wrapping_mul(3) + c * 131) % 977 < 40;
                    if on {
                        let i = (y * 1920 + x) * 4;
                        let a = ((x * 31 + y * 17) % 256) as u8;
                        canvas.pixels[i..i + 4].copy_from_slice(&[20, 20, 25, a]);
                    }
                }
            }
            p.cells.push(canvas);
        }
        let t0 = Instant::now();
        let raw = postcard::to_stdvec(&p).unwrap();
        let t_raw = t0.elapsed();

        let t1 = Instant::now();
        let (packed, reported) = encode(&p).unwrap();
        let t_pack = t1.elapsed();

        let t2 = Instant::now();
        let back = decode(&packed).unwrap();
        let t_load = t2.elapsed();
        assert_eq!(back.cells.len(), p.cells.len());

        let mib = |n: usize| n as f64 / (1024.0 * 1024.0);
        println!(
            "\n  raw      {:8.1} MiB   serialize {:?}\
             \n  packed   {:8.1} MiB   encode    {:?}   ({:.0}x smaller)\
             \n  decode                          {:?}\n",
            mib(raw.len()),
            t_raw,
            mib(packed.len()),
            t_pack,
            raw.len() as f64 / packed.len() as f64,
            t_load,
        );
        assert_eq!(reported as usize, raw.len());
    }

    /// The whole thing through a real file: write it, read it back, and demand
    /// the pixels byte for byte. `encode`/`decode` alone would not catch a
    /// framing or truncation bug that only shows up once bytes hit the disk.
    #[test]
    fn saves_and_loads_a_real_file_losslessly() {
        let mut p = Project::new(64, 48, 12.0);
        for (i, b) in p.cells[0].pixels.iter_mut().enumerate() {
            *b = ((i * 29 + i / 64 * 7) % 251) as u8;
        }
        p.layers[0].name = "Round trip".into();
        let expect = p.cells[0].pixels.clone();

        let path = std::env::temp_dir().join(format!(
            "animator-round-trip-{}.anim",
            std::process::id()
        ));
        save_to(&p, &path).expect("save");

        let on_disk = std::fs::metadata(&path).expect("stat").len();
        let raw = postcard::to_stdvec(&p).unwrap().len() as u64;

        let q = load_from(&path).expect("load");
        let _ = std::fs::remove_file(&path);

        assert_eq!(q.layers[0].name, "Round trip");
        assert_eq!(
            q.cells[0].pixels, expect,
            "pixels changed on the way through a real file"
        );
        assert!(
            on_disk < raw,
            "{on_disk} bytes on disk is not smaller than the {raw}-byte body"
        );
    }

    /// A file from a future build is refused with the supported range, rather
    /// than being fed to a parser that would read it as garbage.
    #[test]
    fn rejects_unsupported_version() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(VERSION + 1).to_le_bytes());
        bytes.extend_from_slice(&[0u8; 32]);
        // Not `expect_err`: `Project` has no `Debug`, and deriving one on a
        // struct that owns every pixel buffer is not worth it for a test.
        let err = match decode(&bytes) {
            Ok(_) => panic!("a future version must be refused"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("unsupported project version"),
            "got {err:#}"
        );
        // And something that isn't ours at all.
        assert!(decode(b"NOPE\x06\x00\x00\x00junk").is_err());
    }
}

/// Write `project` to `path`, overwriting whatever is there.
///
/// Deliberately separate from [`ask_save_path`]: the caller keeps the path so it
/// can save to the same file again without prompting.
pub fn save_to(project: &Project, path: &Path) -> Result<()> {
    let (bytes, raw_len) = encode(project)?;
    let packed = bytes.len() as u64;
    std::fs::write(path, bytes).with_context(|| format!("writing {path:?}"))?;
    // Log the ratio: a format change that quietly stopped compressing would
    // otherwise only show up as a surprise in the file manager.
    let mib = |n: u64| n as f64 / (1024.0 * 1024.0);
    log::info!(
        "Saved project → {} ({:.1} MiB from {:.1} MiB, {:.0}x)",
        path.display(),
        mib(packed),
        mib(raw_len),
        raw_len as f64 / packed.max(1) as f64,
    );
    Ok(())
}

/// Prompt for a destination. When `current` is set the dialog opens on that
/// file's folder and name, so "Save As" starts from where the project already
/// lives. `None` = the user cancelled.
pub fn ask_save_path(current: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter("Animator Project", &[EXT]);
    match current {
        Some(p) => {
            if let Some(dir) = p.parent() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                dialog = dialog.set_file_name(name);
            }
        }
        None => dialog = dialog.set_file_name("project.anim"),
    }
    dialog.save_file()
}

/// Read a project from `path`.
///
/// Split out of [`load_dialog`] so the on-disk round trip can be exercised
/// without a file picker — the read half of [`save_to`], which has always taken
/// a path.
pub fn load_from(path: &Path) -> Result<Project> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {path:?}"))?;
    let project = decode(&bytes)?;
    log::info!("Loaded project ← {}", path.display());
    Ok(project)
}

/// Prompt for a file and load it, returning the path it came from so the caller
/// can remember it. `Ok(None)` if the user cancels.
pub fn load_dialog() -> Result<Option<(Project, PathBuf)>> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Animator Project", &[EXT])
        .pick_file()
    else {
        return Ok(None);
    };
    let project = load_from(&path)?;
    Ok(Some((project, path)))
}
