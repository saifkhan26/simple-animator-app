//! Pen tablet input.
//!
//! Strategy mirrors Krita, which is really Qt's `qwindowstabletsupport.cpp`:
//! the tablet driver's own cursor emulation keeps driving egui's press and
//! release, but stroke *positions* come from the Wintab packet queue rather
//! than from the OS mouse.
//!
//! That distinction is the whole point. The mouse reports whole screen pixels
//! at the redraw rate; the pen reports thousands of counts per inch at
//! 133-266 Hz. Zoomed out, one screen pixel is several document pixels, so
//! mouse quantization turns a slow straight line into a visible staircase that
//! any interpolator then faithfully traces. Reading `pkXYZ` from *every*
//! queued packet is what removes it.
//!
//! Windows: Wintab via `wintab_lite`, dynamic-loaded so the app still runs
//! without a tablet driver. Linux/macOS: not yet implemented — no packets are
//! produced and the brush falls back to the egui pointer at pressure 1.0.

/// One tablet sample.
///
/// Position is in *virtual-desktop pixels* with sub-pixel precision, in the
/// same coordinate space as the OS cursor — see `Wintab::map_point` for why
/// the two agree by construction.
#[derive(Clone, Copy, Debug)]
pub struct PenPacket {
    pub x: f32,
    pub y: f32,
    /// Normalised 0..=1.
    pub pressure: f32,
    /// Tilt in degrees, -90..=90, x toward screen-right and y toward
    /// screen-down. Zero when the device reports no orientation.
    pub tilt_x: f32,
    pub tilt_y: f32,
}

/// One axis of the packet -> screen mapping: `(origin, packet_origin, scale)`,
/// applied as `origin + (v - packet_origin) * scale`.
///
/// `base_*` is the output area the driver defines for itself, `granted_*` the
/// one it actually gave us after we asked for a finer scale. Deriving the map
/// from the ratio of the two means neither this code nor its caller has to
/// know whether the request was honoured.
///
/// Crucially it also carries the *sign* through. A tablet's native Y axis
/// points up and a screen's points down, and the driver's own output extent is
/// where that flip is expressed — so an extent must only ever be scaled, never
/// substituted for the screen rectangle, which does not carry axis direction.
/// Getting that wrong mirrors every reported position about the middle of the
/// screen.
#[inline]
fn axis_map(base_org: i32, base_ext: i32, granted_org: i32, granted_ext: i32) -> (f32, f32, f32) {
    (
        base_org as f32,
        granted_org as f32,
        base_ext as f32 / granted_ext as f32,
    )
}

/// What the tablet backend is currently seeing.
///
/// This exists because the interesting failures here are invisible: a release
/// build is a GUI subsystem binary with no console, so `log::info!` goes
/// nowhere, and a mis-mapped axis draws confidently in the wrong place rather
/// than reporting an error. Everything below is read off the last poll; none
/// of it is computed on the drawing path.
#[derive(Clone, Debug, Default)]
pub struct PenDiagnostics {
    pub backend: bool,
    pub packets_this_frame: usize,
    pub total_packets: u64,
    /// Raw packet coordinates, exactly as the driver reported them.
    pub last_raw: Option<(i32, i32)>,
    /// The same point after mapping, in virtual-desktop pixels.
    pub last_mapped: Option<(f32, f32)>,
    pub client_origin: Option<(f32, f32)>,
    /// `(origin, packet_origin, scale)` per axis — see `axis_map`.
    pub map_x: (f32, f32, f32),
    pub map_y: (f32, f32, f32),
    pub pressure: f32,
    pub tilt: (f32, f32),
    pub queue_overflowed: bool,
}

/// Frames without a packet after which the pen is considered gone and pressure
/// falls back to 1.0. Only consulted while no pointer button is down, so a
/// stroke that pauses mid-line never loses its pressure.
const PEN_IDLE_FRAMES: u32 = 12;

pub struct PenInput {
    #[cfg(target_os = "windows")]
    backend: Option<windows_backend::Wintab>,
    #[cfg(not(target_os = "windows"))]
    backend: (),
    /// This frame's packets, oldest first. Cleared by every `poll`.
    packets: Vec<PenPacket>,
    last_pressure: f32,
    idle_frames: u32,
}

impl PenInput {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "windows")]
            backend: None,
            #[cfg(not(target_os = "windows"))]
            backend: (),
            packets: Vec::new(),
            last_pressure: 1.0,
            idle_frames: u32::MAX,
        }
    }

    /// Called once per frame. Initialises the backend lazily (once the window
    /// exists) and drains the whole packet queue into `packets`.
    ///
    /// `pointer_down` gates the idle reset only: a pen held still mid-stroke
    /// produces no packets, and dropping its pressure back to 1.0 there would
    /// swell the line.
    pub fn poll(&mut self, pointer_down: bool) {
        self.packets.clear();
        #[cfg(target_os = "windows")]
        {
            if self.backend.is_none() {
                match windows_backend::Wintab::try_init() {
                    Ok(w) => {
                        log::info!("Wintab pen backend ready");
                        self.backend = Some(w);
                    }
                    Err(e) => {
                        // Don't spam — only log once per second-ish would be
                        // nicer, but a single missing-DLL message is fine.
                        log::debug!("Wintab init pending/failed: {e}");
                    }
                }
            }
            if let Some(w) = &mut self.backend {
                w.poll(&mut self.packets);
            }
        }

        if let Some(last) = self.packets.last() {
            self.last_pressure = last.pressure;
            self.idle_frames = 0;
        } else {
            self.idle_frames = self.idle_frames.saturating_add(1);
            // Idle with nothing pressed: assume the mouse took over. Without
            // this the pressure left behind by a pen lift (~0.0) would make
            // every subsequent mouse stroke a hairline.
            if !pointer_down && self.idle_frames > PEN_IDLE_FRAMES {
                self.last_pressure = 1.0;
            }
        }
    }

    /// This frame's tablet packets, oldest first.
    pub fn packets(&self) -> &[PenPacket] {
        &self.packets
    }

    /// A snapshot for the diagnostics readout. See [`PenDiagnostics`].
    pub fn diagnostics(&self) -> PenDiagnostics {
        let mut d = PenDiagnostics {
            backend: self.is_active(),
            packets_this_frame: self.packets.len(),
            pressure: self.last_pressure,
            ..Default::default()
        };
        if let Some(p) = self.packets.last() {
            d.tilt = (p.tilt_x, p.tilt_y);
        }
        #[cfg(target_os = "windows")]
        if let Some(w) = self.backend.as_ref() {
            d.total_packets = w.total_packets;
            d.last_raw = w.last_raw;
            d.client_origin = w.client_origin;
            d.map_x = w.map_x;
            d.map_y = w.map_y;
            d.queue_overflowed = w.queue_err_logged;
        }
        // Mapped from the retained raw value rather than from this frame's
        // packets, which are empty whenever the pen holds still — the readout
        // would otherwise blink out exactly while being read.
        d.last_mapped = d.last_raw.map(|(x, y)| {
            (
                d.map_x.0 + (x as f32 - d.map_x.1) * d.map_x.2,
                d.map_y.0 + (y as f32 - d.map_y.1) * d.map_y.2,
            )
        });
        d
    }

    /// True while the pen is the live input device: a backend exists and it
    /// has produced packets recently. False for mouse input, so callers know
    /// to ignore `current_pressure` and the packet positions.
    pub fn pen_active(&self) -> bool {
        self.is_active() && self.idle_frames <= PEN_IDLE_FRAMES
    }

    /// Returns latest reported pen pressure (0..=1) if available.
    pub fn current_pressure(&self) -> Option<f32> {
        if self.pen_active() {
            Some(self.last_pressure)
        } else {
            None
        }
    }

    /// Top-left of the window's client area in virtual-desktop pixels, for
    /// turning packet positions into window-local ones. `None` without a
    /// backend.
    pub fn client_origin(&self) -> Option<(f32, f32)> {
        #[cfg(target_os = "windows")]
        {
            return self.backend.as_ref().and_then(|w| w.client_origin);
        }
        #[cfg(not(target_os = "windows"))]
        {
            return None;
        }
    }

    /// Whether a tablet backend is present at all — drives the UI's
    /// pen/mouse status label, independent of recent activity.
    pub fn is_active(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            return self.backend.is_some();
        }
        #[cfg(not(target_os = "windows"))]
        {
            return false;
        }
    }
}

#[cfg(target_os = "windows")]
mod windows_backend {
    //! Wintab backend. We find our HWND by window title to avoid plumbing a
    //! raw window handle through eframe's CreationContext.

    use super::PenPacket;
    use anyhow::{anyhow, Context, Result};
    use libloading::{Library, Symbol};
    use windows::core::PCSTR;
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, GetClientRect};
    use wintab_lite::{
        cast_void, Packet, WTClose, WTInfo, WTOpen, WTPacketsGet, AXIS, CXO, DVC, HCTX, LOGCONTEXT,
        WTI, WTPKT,
    };

    /// Sub-pixel factor applied to the context's output extents. Packet
    /// coordinates come back scaled by this, so a 1/32-pixel grid replaces the
    /// mouse's whole-pixel one. Well past the tablet's own resolution on any
    /// desktop-sized mapping, which is the point — the quantization that
    /// remains is the hardware's, not ours.
    const SUBPIXEL: i32 = 32;

    /// Polls after which a context that has never produced a packet is
    /// called out. Roughly a few seconds of drawing.
    const SILENCE_WARN_POLLS: u32 = 300;

    /// Packet queue depth, matching Qt's `TabletPacketQSize`. The driver
    /// default is often 8, which a 200 Hz pen overflows in 40 ms — well inside
    /// one frame at 60 fps.
    const QUEUE_SIZE: i32 = 128;

    /// `WTQueueSizeGet` / `WTQueueSizeSet` have no `wintab_lite` binding, so
    /// they are resolved by name like the rest.
    type WtQueueSizeGet<'a> = Symbol<'a, unsafe extern "C" fn(*mut HCTX) -> i32>;
    type WtQueueSizeSet<'a> = Symbol<'a, unsafe extern "C" fn(*mut HCTX, i32) -> i32>;

    pub struct Wintab {
        wt_close: WTClose<'static>,
        wt_packets_get: WTPacketsGet,
        ctx_handle: *mut HCTX,
        hwnd: HWND,
        pressure_min: f32,
        pressure_span: f32,
        /// Raw coordinates of the most recent packet, for diagnostics.
        pub last_raw: Option<(i32, i32)>,
        /// Packets delivered since the context opened.
        pub total_packets: u64,
        /// Affine packet → virtual-desktop mapping, per axis: the desktop
        /// origin, the packet-space origin, and desktop units per packet unit.
        pub map_x: (f32, f32, f32),
        pub map_y: (f32, f32, f32),
        /// Whether the device reports azimuth/altitude.
        tilt_support: bool,
        /// Refreshed every poll; see `PenInput::client_origin`.
        pub client_origin: Option<(f32, f32)>,
        pub queue_err_logged: bool,
        /// Polls since the context opened, and whether any packet has ever
        /// arrived. A context that opens but never delivers looks exactly like
        /// having no tablet at all, so say so rather than fail silently.
        polls: u32,
        seen_packets: bool,
    }

    impl Wintab {
        pub fn try_init() -> Result<Self> {
            // Find our window by title. The title must match what we set via
            // ViewportBuilder::with_title in main.rs.
            let title = b"Animator\0";
            let hwnd = unsafe { FindWindowA(None, PCSTR(title.as_ptr())) };
            if hwnd.0 == 0 {
                return Err(anyhow!("FindWindowA returned null (window not up yet)"));
            }

            let lib: &'static Library = Box::leak(Box::new(
                unsafe { Library::new("Wintab32.dll") }
                    .context("Wintab32.dll not found (no tablet driver?)")?,
            ));

            let wt_open: WTOpen<'static> =
                unsafe { lib.get(c"WTOpenA".to_bytes()).context("WTOpenA")? };
            let wt_info: WTInfo<'static> =
                unsafe { lib.get(c"WTInfoA".to_bytes()).context("WTInfoA")? };
            let wt_close: WTClose<'static> =
                unsafe { lib.get(c"WTClose".to_bytes()).context("WTClose")? };
            // WTPacketsGet rather than WTQueuePacketsEx + WTDataGet, which is
            // what Qt uses. Its *return value* is the number of packets copied,
            // where WTDataGet reports that through an out-parameter alongside a
            // separate return value, and needs a serial-number range that has to
            // stay valid across a 32-bit wrap. One number that cannot
            // disagree with itself is worth having here — see `poll`.
            let wt_packets_get: WTPacketsGet =
                unsafe { lib.get(c"WTPacketsGet".to_bytes()).context("WTPacketsGet")? };

            let mut log_context = LOGCONTEXT::default();
            let r = unsafe { wt_info(WTI::DEFSYSCTX, 0, cast_void!(log_context)) };
            if r == 0 {
                return Err(anyhow!("WTInfo(DEFSYSCTX) failed"));
            }

            // The driver's own output mapping, which is the *only* thing
            // here that knows how the tablet's axes relate to the screen's.
            // A tablet's native Y points up and a screen's points down, and
            // this is where that flip lives. So it may be scaled, to ask for
            // finer resolution, and it may be restored — but it must never be
            // rebuilt out of lcSys*, which carries the screen rectangle
            // without the axis directions. Doing that mirrors Y about the
            // middle of the screen.
            let base_out_org = log_context.lcOutOrgXYZ;
            let base_out_ext = log_context.lcOutExtXYZ;
            if base_out_ext.x == 0 || base_out_ext.y == 0 {
                return Err(anyhow!("DEFSYSCTX has an empty output area"));
            }
            // Read only to be logged: every problem in this file has come down
            // to what the driver granted versus what it was asked for.
            let sys_org = log_context.lcSysOrgXY;
            let sys_ext = log_context.lcSysExtXY;

            log_context.lcName.write_str("animator-app");
            // Stay a system context. Qt only *adds* flags to the default
            // system context, and a system context still honours lcOut* for
            // the coordinates it reports — lcSys* is what drives the cursor,
            // separately. Clearing CXO_SYSTEM here stopped packets arriving
            // at all, which is worth a comment because the fix looks like a
            // no-op otherwise.
            log_context.lcOptions |= CXO::SYSTEM;
            log_context.lcPktData = WTPKT::all();
            log_context.lcPktMode = WTPKT::empty();
            log_context.lcMoveMask = WTPKT::X | WTPKT::Y | WTPKT::NORMAL_PRESSURE;

            // A pure scale of the driver's own area about the origin, so
            // the mapping is unchanged in every respect but resolution — signs
            // and all.
            log_context.lcOutOrgXYZ.x = base_out_org.x * SUBPIXEL;
            log_context.lcOutOrgXYZ.y = base_out_org.y * SUBPIXEL;
            log_context.lcOutExtXYZ.x = base_out_ext.x * SUBPIXEL;
            log_context.lcOutExtXYZ.y = base_out_ext.y * SUBPIXEL;

            let mut pressure_axis = AXIS::default();
            let pr =
                unsafe { wt_info(WTI::DEVICES, DVC::NPRESSURE as u32, cast_void!(pressure_axis)) };
            let (pressure_min, pressure_max) = if pr as usize == std::mem::size_of::<AXIS>() {
                (pressure_axis.axMin as f32, pressure_axis.axMax as f32)
            } else {
                (0.0, 1023.0)
            };
            let pressure_span = pressure_max - pressure_min;
            if pressure_span < 1.0 {
                return Err(anyhow!(
                    "invalid pressure range: {pressure_min}..{pressure_max}"
                ));
            }

            // Azimuth/altitude/twist. A device without tilt reports zero
            // resolution on the first two axes; asking anyway and checking is
            // cheaper than maintaining a device table.
            let mut orientation = [AXIS::default(); 3];
            let or =
                unsafe { wt_info(WTI::DEVICES, DVC::ORIENTATION as u32, cast_void!(orientation)) };
            let azimuth_res: f64 = orientation[0].axResolution.into();
            let altitude_res: f64 = orientation[1].axResolution.into();
            let tilt_support = or as usize == std::mem::size_of::<[AXIS; 3]>()
                && azimuth_res != 0.0
                && altitude_res != 0.0;

            let mut ctx_handle = unsafe { wt_open(hwnd, &mut log_context, 1) };
            if ctx_handle.is_null() {
                // Sub-pixel precision is worth asking for, but not worth
                // losing the tablet over if a driver dislikes an output area
                // that large.
                log::warn!(
                    "Wintab refused a {SUBPIXEL}x output area; retrying at the driver's own scale"
                );
                log_context.lcOutOrgXYZ = base_out_org;
                log_context.lcOutExtXYZ = base_out_ext;
                ctx_handle = unsafe { wt_open(hwnd, &mut log_context, 1) };
            }
            if ctx_handle.is_null() {
                return Err(anyhow!("WTOpen returned null"));
            }

            // WTOpen rewrites the struct with what the driver actually granted,
            // which may not be what we asked for. Build the mapping from the
            // granted values so a clamped output area still lands correctly.
            let out_org = log_context.lcOutOrgXYZ;
            let out_ext = log_context.lcOutExtXYZ;
            if out_ext.x == 0 || out_ext.y == 0 {
                let _ = unsafe { wt_close(ctx_handle) };
                return Err(anyhow!("Wintab granted an empty output area"));
            }
            // Packet space back to the driver's own output space, which is
            // screen coordinates. Identity when the scale request was ignored,
            // a plain divide when it was granted, and correct either way
            // without this code needing to know which happened.
            let map_x = super::axis_map(base_out_org.x, base_out_ext.x, out_org.x, out_ext.x);
            let map_y = super::axis_map(base_out_org.y, base_out_ext.y, out_org.y, out_ext.y);

            // Deepen the packet queue. Qt restores the old size on failure and
            // treats a failed restore as fatal; a shallow queue silently drops
            // exactly the samples this module exists to collect.
            unsafe {
                let get: Result<WtQueueSizeGet<'static>, _> = lib.get(c"WTQueueSizeGet".to_bytes());
                let set: Result<WtQueueSizeSet<'static>, _> = lib.get(c"WTQueueSizeSet".to_bytes());
                if let (Ok(get), Ok(set)) = (get, set) {
                    let current = get(ctx_handle);
                    if current != QUEUE_SIZE && set(ctx_handle, QUEUE_SIZE) == 0 {
                        log::warn!("Wintab refused queue size {QUEUE_SIZE}, keeping {current}");
                        let _ = set(ctx_handle, current);
                    }
                } else {
                    log::warn!("WTQueueSize{{Get,Set}} unavailable; using the driver's queue depth");
                }
            }

            // Logged in full because every past tablet problem here has come
            // down to what the driver actually granted, not what was asked for.
            log::info!(
                "Wintab opened: screen org ({}, {}) ext ({}, {}); default output org \
                 ({}, {}) ext ({}, {}); granted output org ({}, {}) ext ({}, {}) \
                 = {:.4} x {:.4} screen px per packet unit; \
                 pressure {pressure_min}..{pressure_max}; tilt {tilt_support}",
                sys_org.x,
                sys_org.y,
                sys_ext.x,
                sys_ext.y,
                base_out_org.x,
                base_out_org.y,
                base_out_ext.x,
                base_out_ext.y,
                out_org.x,
                out_org.y,
                out_ext.x,
                out_ext.y,
                map_x.2,
                map_y.2,
            );

            Ok(Self {
                wt_close,
                wt_packets_get,
                ctx_handle,
                hwnd,
                pressure_min,
                pressure_span,
                map_x,
                map_y,
                tilt_support,
                last_raw: None,
                total_packets: 0,
                client_origin: None,
                queue_err_logged: false,
                polls: 0,
                seen_packets: false,
            })
        }

        /// Packet coordinate → virtual-desktop pixels.
        #[inline]
        fn map_point(&self, x: i32, y: i32) -> (f32, f32) {
            let (sx, ox, kx) = self.map_x;
            let (sy, oy, ky) = self.map_y;
            (sx + (x as f32 - ox) * kx, sy + (y as f32 - oy) * ky)
        }

        /// Azimuth/altitude → x/y tilt in degrees. Straight from Qt's
        /// `qwindowstabletsupport.cpp`, which derives it from
        /// `X = sin(azimuth) * cos(altitude)`, `Z = sin(altitude)`,
        /// `xTilt = atan(X / Z)`. Both angles arrive in tenths of a degree.
        #[inline]
        fn map_tilt(&self, azimuth: i32, altitude: i32) -> (f32, f32) {
            if !self.tilt_support {
                return (0.0, 0.0);
            }
            let rad_azim = (azimuth as f32 / 10.0).to_radians();
            let tan_alt = (altitude as f32 / 10.0).abs().to_radians().tan();
            if tan_alt.abs() < 1e-6 {
                // Pen dead upright: no tilt, and the ratio below would blow up.
                return (0.0, 0.0);
            }
            let x = (rad_azim.sin() / tan_alt).atan().to_degrees();
            let y = (rad_azim.cos() / tan_alt).atan().to_degrees();
            (x, -y)
        }

        /// Drain the whole packet queue into `out`, oldest first, and refresh
        /// the cached client origin.
        pub fn poll(&mut self, out: &mut Vec<PenPacket>) {
            self.client_origin = client_origin(self.hwnd);

            self.polls = self.polls.saturating_add(1);
            if self.polls == SILENCE_WARN_POLLS && !self.seen_packets {
                log::warn!(
                    "Wintab context is open but has never delivered a packet — \
                     drawing will fall back to the mouse. Check that the tablet \
                     driver has Wintab support enabled."
                );
            }

            const MAX: usize = QUEUE_SIZE as usize;
            let mut packets: [Packet; MAX] = core::array::from_fn(|_| Packet::default());
            // The buffer starts as default packets, whose pkXYZ is the tablet
            // origin. Trusting a count that overstates what was written
            // therefore does not produce noise, it produces a stream of
            // *identical* points at one spot on the canvas — and a stroke
            // stretched between the real path and a fixed point rasterizes as
            // a cone. Hence a count that comes straight back from the call.
            let copied = unsafe {
                (self.wt_packets_get)(self.ctx_handle, MAX as i32, cast_void!(packets))
            };
            let removed = (copied.max(0) as usize).min(MAX);
            if removed == 0 {
                return;
            }

            if removed > 0 && !self.seen_packets {
                self.seen_packets = true;
                let xy = packets[0].pkXYZ;
                let mapped = self.map_point(xy.x, xy.y);
                log::info!(
                    "Wintab first packet: raw ({}, {}) -> desktop ({:.2}, {:.2})",
                    xy.x,
                    xy.y,
                    mapped.0,
                    mapped.1
                );
            }

            out.reserve(removed);
            for p in packets.iter().take(removed) {
                // Fields are read through locals: `Packet` is `packed(4)`, so
                // taking a reference to a field is unaligned.
                // `TPS` is not re-exported by `wintab_lite`, so the flag is
                // matched by value: TPS_QUEUE_ERR == 0b10. TPS_PROXIMITY is
                // deliberately not filtered on: the spec calls it "cursor is
                // out of the context", but Qt never tests it in its packet
                // loop, and getting the sense backwards would silently discard
                // every packet. A stale coordinate that lands far from the
                // cursor is caught downstream by the distance guard in
                // `ui::shell::pen_stroke_points` instead.
                let status = p.pkStatus.bits();
                if status & 0b10 != 0 && !self.queue_err_logged {
                    self.queue_err_logged = true;
                    log::warn!("Wintab packet queue overflowed — pen samples were dropped");
                }
                let xy = p.pkXYZ;
                self.last_raw = Some((xy.x, xy.y));
                self.total_packets = self.total_packets.saturating_add(1);
                let (x, y) = self.map_point(xy.x, xy.y);
                let orientation = p.pkOrientation;
                let (tilt_x, tilt_y) = self.map_tilt(orientation.orAzimuth, orientation.orAltitude);
                let raw_pressure = p.pkNormalPressure as f32;
                out.push(PenPacket {
                    x,
                    y,
                    pressure: ((raw_pressure - self.pressure_min) / self.pressure_span)
                        .clamp(0.0, 1.0),
                    tilt_x,
                    tilt_y,
                });
            }
        }
    }

    /// Top-left of the window's client area, in virtual-desktop pixels.
    fn client_origin(hwnd: HWND) -> Option<(f32, f32)> {
        unsafe {
            let mut rect = RECT::default();
            GetClientRect(hwnd, &mut rect).ok()?;
            let mut origin = POINT {
                x: rect.left,
                y: rect.top,
            };
            if !ClientToScreen(hwnd, &mut origin).as_bool() {
                return None;
            }
            Some((origin.x as f32, origin.y as f32))
        }
    }

    impl Drop for Wintab {
        fn drop(&mut self) {
            let _ = unsafe { (self.wt_close)(self.ctx_handle) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply what `axis_map` returns, the way `Wintab::map_point` does.
    fn apply(map: (f32, f32, f32), v: i32) -> f32 {
        map.0 + (v as f32 - map.1) * map.2
    }

    /// A driver that ignores the finer-scale request leaves packets in its own
    /// output space, and the mapping has to be the identity.
    #[test]
    fn an_ignored_scale_request_maps_one_to_one() {
        let map = axis_map(0, 1920, 0, 1920);
        assert_eq!(apply(map, 0), 0.0);
        assert_eq!(apply(map, 1920), 1920.0);
        assert_eq!(apply(map, 733), 733.0);
    }

    /// A granted request divides back down, and lands between whole pixels.
    #[test]
    fn a_granted_scale_request_divides_back_down() {
        let map = axis_map(0, 1920, 0, 1920 * 32);
        assert_eq!(apply(map, 0), 0.0);
        assert_eq!(apply(map, 1920 * 32), 1920.0);
        assert!((apply(map, 733 * 32 + 16) - 733.5).abs() < 1e-3);
    }

    /// The regression this function exists for.
    ///
    /// A tablet's Y axis points up, so the driver's output extent for it is
    /// negative. Rebuilding the map from the screen rectangle instead — which
    /// is positive — mirrored every position about the middle of the screen.
    /// Near that line the mirrored point is close enough to the real one to
    /// pass a sanity check and be drawn, which is why the artefact appeared as
    /// a band across the middle of the window rather than as everything being
    /// upside down.
    #[test]
    fn a_flipped_axis_stays_flipped() {
        // Driver maps the tablet onto y = 0..1080 with an inverted axis.
        let map = axis_map(1080, -1080, 1080 * 32, -1080 * 32);
        assert_eq!(apply(map, 1080 * 32), 1080.0, "origin end");
        assert_eq!(apply(map, 0), 0.0, "far end");
        // The midpoint is the mirror line: the one place a lost flip looks
        // correct, and the reason the bug hid there.
        assert_eq!(apply(map, 540 * 32), 540.0);
        // A quarter of the way along must not come back three quarters.
        assert!((apply(map, 270 * 32) - 270.0).abs() < 1e-3);
    }

    /// A driver free to grant a different origin as well as a different extent
    /// must still land correctly.
    #[test]
    fn an_offset_output_origin_is_carried_through() {
        let map = axis_map(-1920, 1920, -1920 * 8, 1920 * 8);
        assert_eq!(apply(map, -1920 * 8), -1920.0);
        assert_eq!(apply(map, 0), 0.0);
    }
}
