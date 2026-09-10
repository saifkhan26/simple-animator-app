//! Pen tablet input.
//!
//! Strategy mirrors Krita, which is really Qt's `qwindowstabletsupport.cpp`:
//! the tablet driver's own cursor emulation keeps driving egui's press and
//! release, but stroke *positions* come from the Wintab packet queue rather
//! than from the OS mouse.
//!
//! That distinction is the whole point. The mouse gives one position per
//! redraw, so most of what a 133-266 Hz pen reports is thrown away and what
//! survives is spaced two to four device samples apart. Zoomed out, the gaps
//! are several document pixels wide and any interpolator traces the resulting
//! staircase faithfully. Reading *every* queued packet is what removes it.
//!
//! Positions arrive in the driver's own screen convention rather than at a
//! finer scale of our asking; see the note at the output area in `try_init`.
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

/// Byte offsets of the packet fields this module reads, and the packet's
/// stride, derived from the field mask the driver actually granted.
///
/// `WTOpen` rewrites `lcPktData` with what it is willing to report, which need
/// not be what was asked for — most pens have no rotation, for instance. Every
/// granted field is present, in the fixed order of the `WTPKT` bits, and
/// nothing else is. So a packet's size depends on the device.
///
/// Reading a device's packets through a fixed-size struct is therefore only
/// correct when every field happened to be granted. Otherwise the first packet
/// of a batch reads fine and every one after it is picked up at the wrong
/// offset — which is why the symptom was a stroke that looked perfect while
/// the pen hovered, one packet to a frame, and fell apart the moment it was
/// moved fast enough to deliver several.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PacketLayout {
    /// Stride between packets, in bytes.
    pub size: usize,
    pub status: Option<usize>,
    pub x: Option<usize>,
    pub y: Option<usize>,
    pub pressure: Option<usize>,
    /// Offset of the ORIENTATION triple; azimuth first, then altitude.
    pub orientation: Option<usize>,
}

/// The packet fields, in `WTPKT` bit order with their sizes. The order is the
/// layout: Wintab emits exactly the granted fields, in this sequence.
///
/// `CONTEXT` is a handle, so it is pointer-sized; the rest are 32-bit, and the
/// two triples are three of those each.
const PACKET_FIELDS: [(u32, usize); 14] = [
    (1 << 0, std::mem::size_of::<usize>()), // CONTEXT
    (1 << 1, 4),                            // STATUS
    (1 << 2, 4),                            // TIME
    (1 << 3, 4),                            // CHANGED
    (1 << 4, 4),                            // SERIAL_NUMBER
    (1 << 5, 4),                            // CURSOR
    (1 << 6, 4),                            // BUTTONS
    (1 << 7, 4),                            // X
    (1 << 8, 4),                            // Y
    (1 << 9, 4),                            // Z
    (1 << 10, 4),                           // NORMAL_PRESSURE
    (1 << 11, 4),                           // TANGENT_PRESSURE
    (1 << 12, 12),                          // ORIENTATION
    (1 << 13, 12),                          // ROTATION
];

impl PacketLayout {
    /// Walk the granted mask, accumulating offsets.
    pub fn from_mask(granted: u32) -> Self {
        let mut out = Self::default();
        let mut at = 0usize;
        for (bit, size) in PACKET_FIELDS {
            if granted & bit == 0 {
                continue;
            }
            match bit {
                b if b == 1 << 1 => out.status = Some(at),
                b if b == 1 << 7 => out.x = Some(at),
                b if b == 1 << 8 => out.y = Some(at),
                b if b == 1 << 10 => out.pressure = Some(at),
                b if b == 1 << 12 => out.orientation = Some(at),
                _ => {}
            }
            at += size;
        }
        out.size = at;
        out
    }

    /// Whether the granted fields are enough to place a stroke at all.
    pub fn usable(&self) -> bool {
        self.size > 0 && self.x.is_some() && self.y.is_some()
    }
}

#[inline]
fn read_i32(buf: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

#[inline]
fn read_u32(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
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
    /// The packet layout the driver granted. `size` is the giveaway: a device
    /// that grants every field gives 76 bytes on a 64-bit build.
    pub layout: PacketLayout,
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
            d.layout = w.layout;
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
    use super::{read_i32, read_u32, PacketLayout};
    use wintab_lite::{
        cast_void, WTClose, WTInfo, WTOpen, WTPacketsGet, AXIS, CXO, DVC, HCTX, LOGCONTEXT, WTI,
        WTPKT,
    };

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
        /// Where each field sits in a packet, and how long one is. Derived
        /// from what the driver granted, not from what was asked for.
        pub layout: PacketLayout,
        /// Scratch for one poll's worth of packets. Held rather than allocated
        /// per frame; this runs on every frame of every stroke.
        buf: Vec<u8>,
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

            // The output area is deliberately left exactly as the driver
            // defines it. Asking for a finer one, to get sub-pixel positions,
            // inverted the Y axis twice — scaling lcOut* is evidently not the
            // whole story for a system context, and where the axis direction
            // actually lives is not something to keep guessing at against a
            // device that cannot be tested here. Packets therefore arrive in
            // the driver's own screen convention, agreeing with the cursor by
            // construction.
            //
            // Little is lost. The wave this module set out to fix came mostly
            // from taking one position per redraw instead of every packet; the
            // sub-pixel grid was the smaller half, and it can come back
            // through the route Qt actually uses — a non-system context on a
            // window of its own, scaled against the device axes — once there
            // is a tablet to verify it against.

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

            let ctx_handle = unsafe { wt_open(hwnd, &mut log_context, 1) };
            if ctx_handle.is_null() {
                return Err(anyhow!("WTOpen returned null"));
            }

            // WTOpen rewrites the struct with what the driver settled on.
            // Nothing here asked it to change the output area, so this is
            // normally the identity — but it is derived rather than assumed,
            // because a driver is free to adjust it and the failure mode of
            // guessing wrong is a stroke drawn confidently in the wrong place.
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
            // What the driver will actually report, which WTOpen has just
            // rewritten. Never assume this matches the request.
            let layout = PacketLayout::from_mask(log_context.lcPktData.bits());
            if !layout.usable() {
                let _ = unsafe { wt_close(ctx_handle) };
                return Err(anyhow!(
                    "Wintab granted no position fields (packet mask {:#x})",
                    log_context.lcPktData.bits()
                ));
            }

            log::info!(
                "Wintab opened: screen org ({}, {}) ext ({}, {}); default output org \
                 ({}, {}) ext ({}, {}); granted output org ({}, {}) ext ({}, {}) \
                 = {:.4} x {:.4} screen px per packet unit; \
                 packet mask {:#x} = {} bytes, offsets x {:?} y {:?} pressure {:?} \
                 orientation {:?}; pressure {pressure_min}..{pressure_max}; \
                 tilt {tilt_support}",
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
                log_context.lcPktData.bits(),
                layout.size,
                layout.x,
                layout.y,
                layout.pressure,
                layout.orientation,
            );

            Ok(Self {
                wt_close,
                wt_packets_get,
                ctx_handle,
                layout,
                buf: vec![0u8; QUEUE_SIZE as usize * layout.size],
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
            // Bytes, walked at the driver's own stride. A fixed-size struct
            // would only be right for a device that granted every field.
            let copied = unsafe {
                (self.wt_packets_get)(
                    self.ctx_handle,
                    MAX as i32,
                    self.buf.as_mut_ptr() as *mut std::ffi::c_void,
                )
            };
            let removed = (copied.max(0) as usize).min(MAX);
            if removed == 0 {
                return;
            }

            let layout = self.layout;
            let (Some(x_at), Some(y_at)) = (layout.x, layout.y) else {
                return;
            };

            out.reserve(removed);
            for i in 0..removed {
                let p = &self.buf[i * layout.size..(i + 1) * layout.size];

                // TPS_QUEUE_ERR == 0b10. TPS_PROXIMITY is deliberately not
                // filtered on: the spec calls it "cursor is out of the
                // context", but Qt never tests it in its packet loop, and
                // getting the sense backwards would silently discard every
                // packet. A stale coordinate that lands far from the cursor is
                // caught downstream by the distance guard in
                // `ui::shell::pen_stroke_points` instead.
                if let Some(at) = layout.status {
                    if read_u32(p, at) & 0b10 != 0 && !self.queue_err_logged {
                        self.queue_err_logged = true;
                        log::warn!("Wintab packet queue overflowed — pen samples were dropped");
                    }
                }

                let (rx, ry) = (read_i32(p, x_at), read_i32(p, y_at));
                let (x, y) = self.map_point(rx, ry);
                let pressure = match layout.pressure {
                    Some(at) => ((read_u32(p, at) as f32 - self.pressure_min)
                        / self.pressure_span)
                        .clamp(0.0, 1.0),
                    // Nothing to go on, so draw at full strength rather than
                    // at nothing.
                    None => 1.0,
                };
                let (tilt_x, tilt_y) = match layout.orientation {
                    Some(at) => self.map_tilt(read_i32(p, at), read_i32(p, at + 4)),
                    None => (0.0, 0.0),
                };

                if !self.seen_packets {
                    self.seen_packets = true;
                    log::info!(
                        "Wintab first packet: raw ({rx}, {ry}) -> desktop ({x:.2}, {y:.2})"
                    );
                }
                self.last_raw = Some((rx, ry));
                self.total_packets = self.total_packets.saturating_add(1);

                out.push(PenPacket {
                    x,
                    y,
                    pressure,
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

    /// The field table has to agree with the layout `wintab_lite`'s own
    /// `Packet` struct describes, or the offsets computed from it are fiction.
    /// Checking the full mask against that struct's size is what pins the two
    /// together.
    #[test]
    fn the_full_mask_matches_the_reference_packet_struct() {
        let full = PacketLayout::from_mask(0x3FFF);
        assert_eq!(
            full.size,
            std::mem::size_of::<wintab_lite::Packet>(),
            "field table disagrees with wintab_lite::Packet"
        );
        assert!(full.usable());
    }

    /// The case that was breaking: a pen with no rotation. Every field before
    /// the missing one keeps its offset, and the packet is shorter — so a
    /// fixed-stride read finds the first packet and loses every one after it.
    #[test]
    fn a_missing_trailing_field_only_shortens_the_packet() {
        let full = PacketLayout::from_mask(0x3FFF);
        let no_rotation = PacketLayout::from_mask(0x3FFF & !(1 << 13));
        assert_eq!(no_rotation.size, full.size - 12);
        assert_eq!(no_rotation.x, full.x);
        assert_eq!(no_rotation.y, full.y);
        assert_eq!(no_rotation.pressure, full.pressure);
        assert_eq!(no_rotation.orientation, full.orientation);
    }

    /// A field dropped from the middle shifts everything after it, which is
    /// the case a size check alone would not catch.
    #[test]
    fn a_missing_middle_field_shifts_what_follows() {
        let full = PacketLayout::from_mask(0x3FFF);
        let no_z = PacketLayout::from_mask(0x3FFF & !(1 << 9));
        assert_eq!(no_z.x, full.x, "x is before z");
        assert_eq!(
            no_z.pressure.unwrap(),
            full.pressure.unwrap() - 4,
            "pressure is after z"
        );
    }

    /// The minimum a tablet has to give us to be worth listening to.
    #[test]
    fn position_alone_is_usable_and_nothing_is_not() {
        let xy = PacketLayout::from_mask((1 << 7) | (1 << 8));
        assert!(xy.usable());
        assert_eq!(xy.size, 8);
        assert_eq!((xy.x, xy.y), (Some(0), Some(4)));
        assert_eq!(xy.pressure, None);

        assert!(!PacketLayout::from_mask(0).usable());
        assert!(!PacketLayout::from_mask(1 << 7).usable(), "x without y");
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
