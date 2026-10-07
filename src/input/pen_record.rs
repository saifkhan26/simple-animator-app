//! A few seconds of where the pen, the OS cursor and egui's pointer each are,
//! frame by frame, written to a file.
//!
//! For tablet problems that only show while the app is the active window: a
//! screenshot of the tablet readout is taken with the screenshot tool active
//! instead, so the readout itself can never show them.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::tablet::PenInput;

/// How long a recording runs: time to click, then sweep the pen across the
/// tablet edge to edge and back.
pub const LENGTH: Duration = Duration::from_secs(6);

pub struct PenRecord {
    started: Instant,
    text: String,
}

impl PenRecord {
    /// Start recording, with what the tablet backend was opened against as a
    /// header.
    pub fn start(pen: &PenInput) -> Self {
        let d = pen.diagnostics();
        let mut text = String::new();
        let _ = writeln!(text, "# area {:?}, context area {:?}", d.area, d.context_area);
        let _ = writeln!(text, "# monitors {:?}", d.display.monitors);
        let _ = writeln!(
            text,
            "# desktop {:?}, driver {:?}, window monitor {:?}",
            d.display.desktop, d.display.driver, d.display.window_monitor
        );
        let _ = writeln!(text, "# map x {:?}, map y {:?}", d.map_x, d.map_y);
        let _ = writeln!(text, "# fit x {:?}, fit y {:?}, {} pairs", d.fit_x, d.fit_y, d.fit_pairs);
        let _ = writeln!(text, "# reopens {} ({:?}), backend {}", d.reopens, d.last_reopen, d.backend);
        let _ = writeln!(
            text,
            "t_ms,focused,cursor_x,cursor_y,egui_x,egui_y,client_x,client_y,ppp,\
             raw_x,raw_y,pen_x,pen_y,packets,pressure,down,touch_x,touch_y"
        );
        Self { started: Instant::now(), text }
    }

    /// One frame's row. True once the recording has run its length.
    pub fn frame(&mut self, ctx: &egui::Context, pen: &PenInput) -> bool {
        let t = self.started.elapsed();
        let d = pen.diagnostics();
        let (focused, egui_pos, ppp, down, touch) = ctx.input(|i| {
            let touch = i.events.iter().rev().find_map(|e| match e {
                egui::Event::Touch { pos, .. } => Some(*pos),
                _ => None,
            });
            (i.focused, i.pointer.latest_pos(), i.pixels_per_point, i.pointer.any_down(), touch)
        });
        let cursor = super::screen_pixel::cursor_pos();
        let last = pen.packets().last().copied();
        let opt = |v: Option<f32>| v.map_or(String::new(), |v| format!("{v:.2}"));
        let _ = writeln!(
            self.text,
            "{},{},{},{},{},{},{},{},{:.3},{},{},{},{},{},{},{},{},{}",
            t.as_millis(),
            focused as u8,
            cursor.map_or(String::new(), |c| c.0.to_string()),
            cursor.map_or(String::new(), |c| c.1.to_string()),
            opt(egui_pos.map(|p| p.x)),
            opt(egui_pos.map(|p| p.y)),
            opt(d.client_origin.map(|c| c.0)),
            opt(d.client_origin.map(|c| c.1)),
            ppp,
            d.last_raw.map_or(String::new(), |r| r.0.to_string()),
            d.last_raw.map_or(String::new(), |r| r.1.to_string()),
            opt(last.map(|p| p.x)),
            opt(last.map(|p| p.y)),
            pen.packets().len(),
            opt(last.map(|p| p.pressure)),
            down as u8,
            opt(touch.map(|p| p.x)),
            opt(touch.map(|p| p.y)),
        );
        t >= LENGTH
    }

    /// Write the recording to the temp folder, returning where.
    pub fn finish(self) -> std::io::Result<PathBuf> {
        let path = std::env::temp_dir().join("animator-pen-record.csv");
        std::fs::write(&path, self.text)?;
        Ok(path)
    }
}
