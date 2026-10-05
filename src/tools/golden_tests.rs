//! Golden strokes: every brush the app ships today, drawn through the real
//! stroke code over a fixed matrix of inputs, tools and layer conditions,
//! and hashed. The hashes were recorded before the Krita engine was
//! extended (tag `krita-baseline-0`); every later change must reproduce them
//! exactly. That is what "no regression" means for the existing brushes.
//!
//! The Krita path divides through `_mm_rcp_ss`, whose table differs between
//! CPU vendors, so its hashes are kept per [`qt::rcp_signature`]. Built with
//! the `exact-rcp` feature, that instruction becomes a true division and the
//! Krita cases get a second, CPU-independent hash:
//!
//! ```text
//! cargo test golden                         # this CPU's hashes
//! cargo test golden --features exact-rcp    # CPU-independent hashes
//! GOLDEN_RECORD=1 cargo test golden         # append rows for new cases
//! GOLDEN_STRICT=1 cargo test golden         # fail on an unknown CPU too
//! ```
//!
//! Recording only ever appends: a row that exists is never rewritten, so a
//! changed hash always fails. On a mismatch the stroke is written to
//! `target/golden-diff/<case>.png` for a look.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use crate::app::AppState;
use crate::doc::canvas::Canvas;
use crate::input::pointer::PointerSample;
use crate::input::shortcuts::Action;
use crate::tools::krita::qt;
use crate::tools::lasso::Mask;
use crate::tools::ribbon::StrokeWorkspace;
use crate::tools::shape;
use crate::tools::stroke::StrokeBuilder;
use crate::tools::{ActiveTool, BrushMode, BrushSettings, ShapeKind, Smoothing, SmoothingOptions, StrokeCap};

const W: u32 = 256;
const H: u32 = 160;
const TABLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tools/testdata/golden_strokes.tsv");

/// FNV-1a, 64-bit.
fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// What is on the layer before the stroke.
#[derive(Clone, Copy)]
enum Ground {
    Clear,
    Grey,
    /// Translucent red stripes over half the canvas, so the composite has
    /// partial alpha to work against.
    Stripes,
    /// Opaque dark ink everywhere: something for an eraser to take away.
    Ink,
}

/// What holds the stroke in.
#[derive(Clone, Copy)]
enum Hold {
    Free,
    /// A soft-edged selection over the middle of the canvas.
    Selection,
    AlphaLock,
}

fn canvas(ground: Ground) -> Canvas {
    let mut c = Canvas::new(W, H);
    let px = c.pixels_mut();
    for (i, p) in px.chunks_mut(4).enumerate() {
        let (x, y) = ((i as u32) % W, (i as u32) / W);
        let v: [u8; 4] = match ground {
            Ground::Clear => [0, 0, 0, 0],
            Ground::Grey => [205, 205, 205, 255],
            Ground::Stripes if x < W / 2 && (y / 6) % 2 == 0 => [200, 40, 40, 128],
            Ground::Stripes => [0, 0, 0, 0],
            Ground::Ink => [12, 14, 16, 255],
        };
        p.copy_from_slice(&v);
    }
    c
}

/// A selection over x 60..200, y 30..130 whose coverage ramps up over its
/// first 40 columns, so both hard and partial coverage are exercised.
fn selection() -> Arc<Mask> {
    let (x, y, w, h) = (60u32, 30u32, 140u32, 100u32);
    let cov = (0..w * h)
        .map(|i| {
            let col = i % w;
            ((col * 255) / 40).min(255) as u8
        })
        .collect();
    Arc::new(Mask { x, y, w, h, cov })
}

fn sample(x: f32, y: f32, pressure: f32, tilt: (f32, f32)) -> PointerSample {
    PointerSample {
        x,
        y,
        pressure,
        tilt_x: tilt.0,
        tilt_y: tilt.1,
        t: 0.0,
        ..Default::default()
    }
}

/// The input gestures.
fn gesture(name: &str) -> Vec<PointerSample> {
    match name {
        // A shallow S with pressure easing in and out and a little tremor:
        // the shape most strokes have.
        "s" => (0..=60)
            .map(|i| {
                let t = i as f32 / 60.0;
                let p = (t * std::f32::consts::PI).sin().powf(0.6).clamp(0.02, 1.0);
                let y = 80.0 + (t * 6.0).sin() * 30.0 + ((i % 3) as f32 - 1.0) * 0.4;
                sample(20.0 + t * 216.0, y, p, (0.0, 0.0))
            })
            .collect(),
        // Straight, constant pressure, on a diagonal.
        "line" => (0..=40)
            .map(|i| {
                let t = i as f32 / 40.0;
                sample(30.0 + t * 196.0, 40.0 + t * 80.0, 0.7, (0.0, 0.0))
            })
            .collect(),
        // A tap: one sample, no movement.
        "tap" => vec![sample(128.0, 80.0, 0.8, (0.0, 0.0))],
        // The pen leaning round a full circle while it travels.
        "tilt" => (0..=50)
            .map(|i| {
                let t = i as f32 / 50.0;
                let a = t * 5.0;
                sample(24.0 + t * 208.0, 70.0 + t * 20.0, 0.9, (55.0 * a.cos(), 55.0 * a.sin()))
            })
            .collect(),
        _ => unreachable!("gesture {name}"),
    }
}

/// One freehand stroke the way the app draws it: begin, hold, then a flush
/// after every sample and a finish at pen-up.
#[allow(clippy::too_many_arguments)]
fn stroke_on(
    c: &mut Canvas,
    ws: &mut StrokeWorkspace,
    brush: &BrushSettings,
    tool: ActiveTool,
    smoothing: SmoothingOptions,
    view_scale: f32,
    orient: (f64, (bool, bool)),
    hold: Hold,
    input: &[PointerSample],
) {
    let pre = c.pixels().into_owned();
    ws.begin(c.width, c.height, brush);
    match hold {
        Hold::Free => {}
        Hold::Selection => ws.set_clip(Some(selection())),
        Hold::AlphaLock => ws.set_alpha_lock(true),
    }
    let mut b = StrokeBuilder::new(brush.clone(), tool, view_scale, smoothing);
    b.orient(orient.0, orient.1);
    for &s in input {
        b.push(s);
        b.flush(c, ws, &pre);
    }
    b.finish(c, ws, &pre);
}

struct Case {
    name: String,
    /// Paints through `_mm_rcp_ss`, so its hash depends on the CPU.
    rcp: bool,
    run: Box<dyn Fn() -> Canvas>,
}

/// The brushes, with the tool each is used on.
fn brushes() -> Vec<(&'static str, BrushSettings, ActiveTool)> {
    let mut v: Vec<_> = BrushSettings::PRESETS
        .iter()
        .map(|&(label, _, build)| {
            let b = build();
            let tool = if label == "G-Pen" { ActiveTool::Ink } else { ActiveTool::Pencil };
            (label, b, tool)
        })
        .collect();
    v.push(("DefaultEraser", BrushSettings::default_eraser(), ActiveTool::Eraser));
    v
}

#[allow(clippy::too_many_arguments)]
fn freehand_case(
    name: String,
    brush: BrushSettings,
    tool: ActiveTool,
    ground: Ground,
    hold: Hold,
    input: &'static str,
    smoothing: SmoothingOptions,
    view_scale: f32,
    orient: (f64, (bool, bool)),
) -> Case {
    let rcp = brush.mode == BrushMode::Krita;
    Case {
        name,
        rcp,
        run: Box::new(move || {
            let mut c = canvas(ground);
            let mut ws = StrokeWorkspace::new();
            stroke_on(&mut c, &mut ws, &brush, tool, smoothing, view_scale, orient, hold, &gesture(input));
            c
        }),
    }
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    let basic = SmoothingOptions::default();
    let upright = (0.0, (false, false));

    // Every brush × gesture × condition.
    for (label, brush, tool) in brushes() {
        let painting_tool = if tool == ActiveTool::Eraser { ActiveTool::Pencil } else { tool };
        for input in ["s", "line", "tap", "tilt"] {
            for (cond, ground, hold) in [
                ("clear", Ground::Clear, Hold::Free),
                ("grey", Ground::Grey, Hold::Free),
                ("stripes", Ground::Stripes, Hold::Free),
                ("sel", Ground::Grey, Hold::Selection),
                ("lock", Ground::Stripes, Hold::AlphaLock),
            ] {
                out.push(freehand_case(
                    format!("{label}/{}/{input}/{cond}", tool_name(painting_tool)),
                    brush.clone(),
                    painting_tool,
                    ground,
                    hold,
                    input,
                    basic,
                    1.0,
                    upright,
                ));
            }
        }
        // The same brush on the eraser, over ink, free and held.
        for (cond, hold) in [("ink", Hold::Free), ("sel", Hold::Selection), ("lock", Hold::AlphaLock)] {
            out.push(freehand_case(
                format!("{label}/eraser/s/{cond}"),
                brush.clone(),
                ActiveTool::Eraser,
                Ground::Ink,
                hold,
                "s",
                basic,
                1.0,
                upright,
            ));
        }
    }

    // Smoothing modes and zoom, on one brush of each engine.
    let weighted = SmoothingOptions {
        kind: Smoothing::Weighted,
        ..SmoothingOptions::default()
    };
    let none = SmoothingOptions {
        kind: Smoothing::None,
        ..SmoothingOptions::default()
    };
    for (label, brush, tool) in brushes() {
        if !matches!(label, "Pencil" | "G-Pen" | "Pencil-5") {
            continue;
        }
        for (sname, opts, scale) in [("none", none, 1.0), ("weighted", weighted, 1.0), ("weighted-zoom05", weighted, 0.5)] {
            out.push(freehand_case(
                format!("{label}/{}/s/clear/{sname}", tool_name(tool)),
                brush.clone(),
                tool,
                Ground::Clear,
                Hold::Free,
                "s",
                opts,
                scale,
                upright,
            ));
        }
    }

    // Krita's Pencil-5 on a turned and mirrored view.
    for input in ["s", "tilt"] {
        out.push(freehand_case(
            format!("Pencil-5/pencil/{input}/clear/rot90-mirror"),
            BrushSettings::pencil5_krita(),
            ActiveTool::Pencil,
            Ground::Clear,
            Hold::Free,
            input,
            basic,
            1.0,
            (90.0, (true, false)),
        ));
    }

    // Flat ink ends.
    for input in ["s", "line", "tap"] {
        let mut b = BrushSettings::default_ink();
        b.cap = StrokeCap::Flat;
        out.push(freehand_case(
            format!("G-Pen-flat/ink/{input}/clear"),
            b,
            ActiveTool::Ink,
            Ground::Clear,
            Hold::Free,
            input,
            basic,
            1.0,
            upright,
        ));
    }

    // Two strokes through one workspace: the second `begin()` must clear
    // what the first left behind.
    for (label, brush, tool) in brushes() {
        if label == "DefaultEraser" {
            continue;
        }
        let rcp = brush.mode == BrushMode::Krita;
        out.push(Case {
            name: format!("{label}/{}/line+s/reused-workspace", tool_name(tool)),
            rcp,
            run: Box::new(move || {
                let mut c = canvas(Ground::Grey);
                let mut ws = StrokeWorkspace::new();
                let o = (0.0, (false, false));
                let s = SmoothingOptions::default();
                stroke_on(&mut c, &mut ws, &brush, tool, s, 1.0, o, Hold::Free, &gesture("line"));
                stroke_on(&mut c, &mut ws, &brush, tool, s, 1.0, o, Hold::Selection, &gesture("s"));
                c
            }),
        });
    }

    // Krita's shipped presets, each as it loads: added as the engine learns
    // to paint them, pinned from then on. Random features are seeded.
    for name in crate::tools::krita::defaults::PRESETS {
        let Ok(brush) = crate::tools::krita::defaults::brush(name) else {
            continue;
        };
        let short = name.trim_end_matches(".kpp");
        let settings = BrushSettings {
            radius: (brush.diameter() / 2.0) as f32,
            color: [30, 60, 90, 255],
            mode: BrushMode::Krita,
            ..BrushSettings::default()
        };
        let erases = brush.erase;
        let conds: &[(&str, Ground, Hold, &str)] = if erases {
            &[("ink", Ground::Ink, Hold::Free, "s"), ("ink-sel", Ground::Ink, Hold::Selection, "s"), ("ink", Ground::Ink, Hold::Free, "tilt")]
        } else {
            &[
                ("clear", Ground::Clear, Hold::Free, "s"),
                ("clear", Ground::Clear, Hold::Free, "tilt"),
                ("clear", Ground::Clear, Hold::Free, "tap"),
                ("stripes", Ground::Stripes, Hold::Free, "line"),
                ("sel", Ground::Grey, Hold::Selection, "s"),
                ("lock", Ground::Stripes, Hold::AlphaLock, "s"),
            ]
        };
        for &(cond, ground, hold, input) in conds {
            let (brush, settings) = (brush.clone(), settings.clone());
            out.push(Case {
                name: format!("krita/{short}/{input}/{cond}"),
                rcp: true,
                run: Box::new(move || {
                    let mut c = canvas(ground);
                    let pre = c.pixels().into_owned();
                    let mut ws = StrokeWorkspace::new();
                    ws.begin(W, H, &settings);
                    match hold {
                        Hold::Free => {}
                        Hold::Selection => ws.set_clip(Some(selection())),
                        Hold::AlphaLock => ws.set_alpha_lock(true),
                    }
                    let mut b = StrokeBuilder::new_with_krita(
                        settings.clone(),
                        ActiveTool::Pencil,
                        1.0,
                        SmoothingOptions::default(),
                        brush.clone(),
                    );
                    b.seed_random(1);
                    for s in gesture(input) {
                        b.push(s);
                        b.flush(&mut c, &mut ws, &pre);
                    }
                    b.finish(&mut c, &mut ws, &pre);
                    c
                }),
            });
        }
    }

    // The Shape tool shares the ribbon workspace.
    for kind in [ShapeKind::Line, ShapeKind::Rect, ShapeKind::Ellipse] {
        for (cond, hold) in [("clear", Hold::Free), ("sel", Hold::Selection)] {
            out.push(Case {
                name: format!("Shape/{kind:?}/{cond}"),
                rcp: false,
                run: Box::new(move || {
                    let brush = BrushSettings::default_shape();
                    let mut c = canvas(Ground::Clear);
                    let pre = c.pixels().into_owned();
                    let mut ws = StrokeWorkspace::new();
                    ws.begin(W, H, &brush);
                    if let Hold::Selection = hold {
                        ws.set_clip(Some(selection()));
                    }
                    let spine = shape::outline(kind, (40.0, 30.0), (210.0, 135.0), brush.radius);
                    shape::rasterize(&mut c, &mut ws, &pre, &spine, &brush);
                    c
                }),
            });
        }
    }

    // Through the app: pen down, moves, up, then undo. The hash covers the
    // painted drawing and the drawing after undo, which must be blank again.
    for (label, action, preset) in [
        ("app/Pencil", Action::ToolPencil, BrushSettings::default_pencil as fn() -> BrushSettings),
        ("app/G-Pen", Action::ToolInk, BrushSettings::default_ink),
        ("app/Pencil-5", Action::ToolPencil, BrushSettings::pencil5_krita),
        ("app/Airbrush", Action::ToolPencil, BrushSettings::airbrush),
    ] {
        let rcp = preset().mode == BrushMode::Krita;
        out.push(Case {
            name: label.to_owned(),
            rcp,
            run: Box::new(move || app_stroke(action, preset())),
        });
    }

    out
}

fn tool_name(t: ActiveTool) -> &'static str {
    match t {
        ActiveTool::Pencil => "pencil",
        ActiveTool::Ink => "ink",
        ActiveTool::Eraser => "eraser",
        _ => "other",
    }
}

/// A stroke through `AppState`, returned as one canvas: the drawing as
/// painted on top, the drawing after undo underneath.
fn app_stroke(tool: Action, brush: BrushSettings) -> Canvas {
    let mut st = AppState::for_test();
    st.dispatch(tool);
    let color = st.brush.color;
    st.brush = brush;
    st.brush.color = color;
    let path = gesture("s");
    st.pointer_down(st.make_sample(path[0].x + 100.0, path[0].y + 100.0, 0.0));
    for (i, s) in path.iter().enumerate().skip(1) {
        let mut p = st.make_sample(s.x + 100.0, s.y + 100.0, i as f32 * 0.01);
        p.pressure = s.pressure;
        st.pointer_move(p);
    }
    st.pointer_up();
    let grab = |st: &AppState| {
        let layer = &st.project.layers[st.project.current_layer];
        let mut out = Vec::with_capacity((W * H * 4) as usize);
        let Some(cell) = layer.resolve(st.project.current_frame).and_then(|id| st.project.cell(id)) else {
            // No drawing left: undo took the whole cell away.
            out.resize((W * H * 4) as usize, 0);
            return out;
        };
        // Crop the stroke's neighbourhood so the hash stays cheap.
        for y in 100..100 + H {
            let row = ((y * cell.width + 100) * 4) as usize;
            out.extend_from_slice(&cell.pixels()[row..row + (W * 4) as usize]);
        }
        out
    };
    let painted = grab(&st);
    st.dispatch(Action::Undo);
    let undone = grab(&st);
    let mut c = Canvas::new(W, H * 2);
    let px = c.pixels_mut();
    px[..painted.len()].copy_from_slice(&painted);
    px[painted.len()..].copy_from_slice(&undone);
    c
}

fn variant(rcp: bool) -> String {
    if !rcp {
        "portable".into()
    } else if cfg!(feature = "exact-rcp") {
        "exact".into()
    } else {
        format!("rcp-{:016x}", qt::rcp_signature())
    }
}

fn load_table() -> HashMap<(String, String), u64> {
    let text = std::fs::read_to_string(TABLE).unwrap_or_default();
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (case, var, hash) = (f.next()?, f.next()?, f.next()?);
            Some(((case.to_owned(), var.to_owned()), u64::from_str_radix(hash, 16).ok()?))
        })
        .collect()
}

fn dump(name: &str, c: &Canvas) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target/golden-diff");
    let _ = std::fs::create_dir_all(dir);
    let file = format!("{dir}/{}.png", name.replace(['/', ' '], "_"));
    if let Some(img) = image::RgbaImage::from_raw(c.width, c.height, c.pixels().into_owned()) {
        let _ = img.save(&file);
    }
}

#[test]
fn golden_strokes() {
    let record = std::env::var_os("GOLDEN_RECORD").is_some();
    let strict = std::env::var_os("GOLDEN_STRICT").is_some();
    let table = load_table();
    let mut failures = Vec::new();
    let mut unknown_cpu = 0;
    let mut new_rows = String::new();
    let cases = cases();
    let mut seen = std::collections::HashSet::new();
    for case in &cases {
        assert!(seen.insert(case.name.clone()), "duplicate case {}", case.name);
        let c = (case.run)();
        let hash = fnv(&c.pixels());
        let var = variant(case.rcp);
        match table.get(&(case.name.clone(), var.clone())) {
            Some(&want) if want == hash => {}
            Some(&want) => {
                dump(&case.name, &c);
                failures.push(format!("{} [{var}]: {hash:016x}, recorded {want:016x}", case.name));
            }
            None if record => {
                let _ = writeln!(new_rows, "{}\t{var}\t{hash:016x}", case.name);
            }
            None if var.starts_with("rcp-") && !strict => unknown_cpu += 1,
            None => failures.push(format!("{} [{var}]: no recorded hash (GOLDEN_RECORD=1 adds it)", case.name)),
        }
    }
    if !new_rows.is_empty() {
        let mut text = std::fs::read_to_string(TABLE).unwrap_or_else(|_| {
            "# Golden stroke hashes: case, variant, FNV-1a of the pixels. Append-only.\n".into()
        });
        text.push_str(&new_rows);
        std::fs::write(TABLE, text).expect("write the golden table");
        println!("recorded {} rows", new_rows.lines().count());
    }
    if unknown_cpu > 0 {
        println!(
            "{unknown_cpu} Krita cases skipped: no hashes for this CPU's rcp signature {:016x}; \
             the exact-rcp run still covers them",
            qt::rcp_signature()
        );
    }
    println!("{} golden cases", cases.len());
    assert!(failures.is_empty(), "{} golden strokes changed:\n{}", failures.len(), failures.join("\n"));
}

const PERF_TABLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tools/testdata/perf_baseline.txt");

/// Stroke timings on a 1080p cell, against this machine's recorded baseline.
/// Release only means anything:
///
/// ```text
/// cargo test --release perf_ -- --ignored --nocapture     # report
/// PERF_RECORD=1 cargo test --release perf_ -- --ignored   # add missing rows
/// PERF_RERECORD=1 cargo test --release perf_ -- --ignored # re-baseline this machine
/// PERF_STRICT=1 cargo test --release perf_ -- --ignored   # fail beyond +15%
/// ```
#[test]
#[ignore]
fn perf_brushes_1080p() {
    let machine = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".into());
    let mut rows: Vec<(String, f64, f64)> = Vec::new();
    for (label, brush, tool) in brushes() {
        if label == "DefaultEraser" {
            continue;
        }
        // Best of three, so one scheduler hiccup does not fail the run.
        let mut best = (f64::MAX, f64::MAX);
        for _ in 0..3 {
            let mut c = Canvas::new(1920, 1080);
            let pre = c.pixels().into_owned();
            let mut ws = StrokeWorkspace::new();
            ws.begin(c.width, c.height, &brush);
            let mut b = StrokeBuilder::new(brush.clone(), tool, 1.0, SmoothingOptions::default());
            let t0 = std::time::Instant::now();
            let mut worst = std::time::Duration::ZERO;
            let n = 400;
            for i in 0..=n {
                let t = i as f32 / n as f32;
                b.push(sample(
                    40.0 + t * 1840.0,
                    40.0 + t * 1000.0 + (t * 20.0).sin() * 30.0,
                    0.3 + 0.7 * (t * std::f32::consts::PI).sin(),
                    (50.0 * (t * 9.0).cos(), 50.0 * (t * 9.0).sin()),
                ));
                let f0 = std::time::Instant::now();
                b.flush(&mut c, &mut ws, &pre);
                worst = worst.max(f0.elapsed());
            }
            b.finish(&mut c, &mut ws, &pre);
            let total = t0.elapsed().as_secs_f64() * 1e3;
            best = (best.0.min(total), best.1.min(worst.as_secs_f64() * 1e3));
        }
        rows.push((label.to_owned(), best.0, best.1));
    }
    // Krita's shipped presets in wash and build-up, auto and bitmap tips.
    for name in ["b)_Basic-5_Size_default.kpp", "a)_Eraser_Soft.kpp", "b)_Airbrush_Soft.kpp", "e)_Marker_Dry.kpp"] {
        let Ok(kb) = crate::tools::krita::defaults::brush(name) else { continue };
        let settings = BrushSettings {
            radius: (kb.diameter() / 2.0) as f32,
            mode: BrushMode::Krita,
            ..BrushSettings::default()
        };
        let mut best = (f64::MAX, f64::MAX);
        for _ in 0..3 {
            let mut c = Canvas::new(1920, 1080);
            let pre = c.pixels().into_owned();
            let mut ws = StrokeWorkspace::new();
            ws.begin(c.width, c.height, &settings);
            let mut b = StrokeBuilder::new_with_krita(settings.clone(), ActiveTool::Pencil, 1.0, SmoothingOptions::default(), kb.clone());
            b.seed_random(1);
            let t0 = std::time::Instant::now();
            let mut worst = std::time::Duration::ZERO;
            let n = 400;
            for i in 0..=n {
                let t = i as f32 / n as f32;
                b.push(sample(
                    40.0 + t * 1840.0,
                    40.0 + t * 1000.0 + (t * 20.0).sin() * 30.0,
                    0.3 + 0.7 * (t * std::f32::consts::PI).sin(),
                    (50.0 * (t * 9.0).cos(), 50.0 * (t * 9.0).sin()),
                ));
                let f0 = std::time::Instant::now();
                b.flush(&mut c, &mut ws, &pre);
                worst = worst.max(f0.elapsed());
            }
            b.finish(&mut c, &mut ws, &pre);
            let total = t0.elapsed().as_secs_f64() * 1e3;
            best = (best.0.min(total), best.1.min(worst.as_secs_f64() * 1e3));
        }
        rows.push((format!("krita/{}", name.trim_end_matches(".kpp")), best.0, best.1));
    }
    {
        let brush = BrushSettings::default_shape();
        let mut best = f64::MAX;
        for _ in 0..3 {
            let mut c = Canvas::new(1920, 1080);
            let pre = c.pixels().into_owned();
            let mut ws = StrokeWorkspace::new();
            ws.begin(c.width, c.height, &brush);
            let t0 = std::time::Instant::now();
            let spine = shape::outline(ShapeKind::Ellipse, (40.0, 40.0), (1880.0, 1040.0), brush.radius);
            shape::rasterize(&mut c, &mut ws, &pre, &spine, &brush);
            best = best.min(t0.elapsed().as_secs_f64() * 1e3);
        }
        rows.push(("Shape-ellipse".into(), best, best));
    }

    let text = std::fs::read_to_string(PERF_TABLE).unwrap_or_default();
    let baseline: HashMap<String, f64> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<_> = l.split('\t').collect();
            if f.len() != 4 || f[0] != machine {
                return None;
            }
            Some((f[1].to_owned(), f[2].parse().ok()?))
        })
        .collect();
    let mut slow = Vec::new();
    for (label, total, worst) in &rows {
        let base = baseline.get(label).copied();
        let note = match base {
            Some(b) if b > 0.0 => format!("{:+.0}% vs {b:.1} ms", (total / b - 1.0) * 100.0),
            _ => "no baseline".into(),
        };
        println!("{label:16} {total:8.1} ms total  {worst:6.2} ms worst flush  ({note})");
        if let Some(b) = base {
            if *total > b * 1.15 && total - b > 2.0 {
                slow.push(format!("{label}: {total:.1} ms vs {b:.1} ms"));
            }
        }
    }
    // PERF_RECORD adds rows this machine has none for; PERF_RERECORD
    // replaces all of this machine's rows.
    let rerecord = std::env::var_os("PERF_RERECORD").is_some();
    if rerecord || std::env::var_os("PERF_RECORD").is_some() {
        let mut out: String = text
            .lines()
            .filter(|l| !(rerecord && l.starts_with(&format!("{machine}\t"))))
            .map(|l| format!("{l}\n"))
            .collect();
        if out.is_empty() {
            out.push_str("# machine\tcase\ttotal_ms\tworst_flush_ms (release, best of 3)\n");
        }
        for (label, total, worst) in &rows {
            if rerecord || !baseline.contains_key(label) {
                let _ = writeln!(out, "{machine}\t{label}\t{total:.2}\t{worst:.3}");
            }
        }
        std::fs::write(PERF_TABLE, out).expect("write perf baseline");
    }
    if std::env::var_os("PERF_STRICT").is_some() {
        assert!(slow.is_empty(), "slower than baseline:\n{}", slow.join("\n"));
    }
}
