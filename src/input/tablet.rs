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
        cast_void, Packet, WTClose, WTDataGet, WTInfo, WTOpen, WTQueuePacketsEx, AXIS, CXO, DVC,
        HCTX, LOGCONTEXT, WTI, WTPKT,
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
        wt_queue: WTQueuePacketsEx<'static>,
        wt_data_get: WTDataGet<'static>,
        ctx_handle: *mut HCTX,
        hwnd: HWND,
        pressure_min: f32,
        pressure_span: f32,
        /// Affine packet → virtual-desktop mapping, per axis: the desktop
        /// origin, the packet-space origin, and desktop units per packet unit.
        map_x: (f32, f32, f32),
        map_y: (f32, f32, f32),
        /// Whether the device reports azimuth/altitude.
        tilt_support: bool,
        /// Refreshed every poll; see `PenInput::client_origin`.
        pub client_origin: Option<(f32, f32)>,
        queue_err_logged: bool,
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
            let wt_queue: WTQueuePacketsEx<'static> = unsafe {
                lib.get(c"WTQueuePacketsEx".to_bytes())
                    .context("WTQueuePacketsEx")?
            };
            let wt_data_get: WTDataGet<'static> =
                unsafe { lib.get(c"WTDataGet".to_bytes()).context("WTDataGet")? };

            let mut log_context = LOGCONTEXT::default();
            let r = unsafe { wt_info(WTI::DEFSYSCTX, 0, cast_void!(log_context)) };
            if r == 0 {
                return Err(anyhow!("WTInfo(DEFSYSCTX) failed"));
            }

            // The default *system* context carries the screen mapping the
            // driver uses to move the cursor. Capture it before we change
            // anything: expressing our output area as a multiple of it is what
            // makes packet positions land on the same spot as the OS pointer,
            // without having to re-derive the driver's axis orientation.
            let sys_org = log_context.lcSysOrgXY;
            let sys_ext = log_context.lcSysExtXY;
            if sys_ext.x == 0 || sys_ext.y == 0 {
                return Err(anyhow!("DEFSYSCTX has an empty screen mapping"));
            }

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

            // Kept so the fallback below can put the driver's own scale back.
            let default_out_org = log_context.lcOutOrgXYZ;
            let default_out_ext = log_context.lcOutExtXYZ;

            log_context.lcOutOrgXYZ.x = sys_org.x * SUBPIXEL;
            log_context.lcOutOrgXYZ.y = sys_org.y * SUBPIXEL;
            log_context.lcOutExtXYZ.x = sys_ext.x * SUBPIXEL;
            log_context.lcOutExtXYZ.y = sys_ext.y * SUBPIXEL;

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
                log_context.lcOutOrgXYZ = default_out_org;
                log_context.lcOutExtXYZ = default_out_ext;
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
            let map_x = (
                sys_org.x as f32,
                out_org.x as f32,
                sys_ext.x as f32 / out_ext.x as f32,
            );
            let map_y = (
                sys_org.y as f32,
                out_org.y as f32,
                sys_ext.y as f32 / out_ext.y as f32,
            );

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
                "Wintab opened: screen org ({}, {}) ext ({}, {}); granted output org ({}, {}) \
                 ext ({}, {}) = {:.4} x {:.4} desktop px per packet unit; \
                 pressure {pressure_min}..{pressure_max}; tilt {tilt_support}",
                sys_org.x,
                sys_org.y,
                sys_ext.x,
                sys_ext.y,
                out_org.x,
                out_org.y,
                out_ext.x,
                out_ext.y,
                map_x.2,
                map_y.2,
            );

            Ok(Self {
                wt_close,
                wt_queue,
                wt_data_get,
                ctx_handle,
                hwnd,
                pressure_min,
                pressure_span,
                map_x,
                map_y,
                tilt_support,
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

            let mut from = 0u32;
            let mut to = 0u32;
            let any = unsafe { (self.wt_queue)(self.ctx_handle, &mut from, &mut to) };
            if any == 0 {
                return;
            }
            const MAX: usize = QUEUE_SIZE as usize;
            let mut packets: [Packet; MAX] = core::array::from_fn(|_| Packet::default());
            let mut removed: i32 = 0;
            let _ = unsafe {
                (self.wt_data_get)(
                    self.ctx_handle,
                    from,
                    to,
                    MAX as i32,
                    cast_void!(packets),
                    &mut removed,
                )
            };
            let removed = (removed.max(0) as usize).min(MAX);

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
                // matched by value: TPS_QUEUE_ERR == 0b10.
                let status = p.pkStatus.bits();
                if status & 0b10 != 0 && !self.queue_err_logged {
                    self.queue_err_logged = true;
                    log::warn!("Wintab packet queue overflowed — pen samples were dropped");
                }
                let xy = p.pkXYZ;
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
