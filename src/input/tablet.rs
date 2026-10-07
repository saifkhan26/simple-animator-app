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
//! Positions are asked for at `SUBPIXEL` times screen resolution and mapped
//! back down, so they land between screen pixels; see the note at the output
//! area in `try_init`. A driver that ignores the request still maps correctly,
//! just on whole pixels.
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
    /// The packet's own timestamp, milliseconds on the driver's clock (0
    /// when it reports none). Only differences between packets mean
    /// anything.
    pub time_ms: f64,
    /// Barrel rotation (twist), degrees, -180..=180, as Krita maps it.
    pub rotation: f32,
    /// Tangential pressure, 0..=1: an airbrush's finger wheel. Zero for
    /// every other pen, as in Krita.
    pub tangential: f32,
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
    /// The granted field mask this was derived from.
    pub mask: u32,
    /// Stride between packets, in bytes.
    pub size: usize,
    pub status: Option<usize>,
    pub x: Option<usize>,
    pub y: Option<usize>,
    pub pressure: Option<usize>,
    /// Offset of the ORIENTATION triple: azimuth, altitude, twist.
    pub orientation: Option<usize>,
    pub time: Option<usize>,
    pub cursor: Option<usize>,
    pub tangent: Option<usize>,
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
        let mut out = Self {
            mask: granted,
            ..Self::default()
        };
        let mut at = 0usize;
        for (bit, size) in PACKET_FIELDS {
            if granted & bit == 0 {
                continue;
            }
            match bit {
                b if b == 1 << 1 => out.status = Some(at),
                b if b == 1 << 2 => out.time = Some(at),
                b if b == 1 << 5 => out.cursor = Some(at),
                b if b == 1 << 11 => out.tangent = Some(at),
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

/// One axis of a fitted packet -> screen mapping: `origin + raw * scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisFit {
    pub origin: f64,
    pub scale: f64,
    /// Root-mean-square distance of the fitted pairs from the line, in
    /// screen pixels. Whole-pixel cursor rounding alone gives about 0.3.
    pub rms: f64,
}

impl AxisFit {
    #[inline]
    pub fn map(&self, raw: i32) -> f32 {
        (self.origin + raw as f64 * self.scale) as f32
    }
}

/// Pairs pulled into a fit. Enough to average whole-pixel rounding down to a
/// small fraction of a pixel, few enough to refit every frame.
const FIT_WINDOW: usize = 256;
/// Fewest pairs a fit is trusted on.
const FIT_MIN_PAIRS: usize = 16;
/// Cursor travel, in screen pixels, the pairs must span on an axis before
/// that axis's slope means anything.
const FIT_MIN_SPAN: f64 = 48.0;
/// Scatter above which the pairs are not describing a straight line — a
/// mouse moved alongside the pen, say — and the fit is not used.
const FIT_MAX_RMS: f64 = 1.5;
/// Cursor travel in one frame above which a pair is left out: moving that
/// fast, the gap between reading the queue and reading the cursor matters.
const FIT_MAX_STEP: i32 = 8;
/// A new pair this far off a trusted fit is a miss. A miss is left out of
/// the fit — it is usually the cursor lagging the pen for a moment — but a
/// long enough run of them means the mapping itself has changed (a monitor
/// rearranged, the tablet remapped in its driver), and the fit starts over.
const FIT_MISS_PX: f64 = 6.0;
const FIT_MISSES_TO_RESET: u32 = 24;

/// The packet -> screen mapping, learned from where the driver puts the
/// cursor.
///
/// The output area a driver reports back from `WTOpen` cannot be taken at
/// its word. One seen here accepted a Y-flipped area, reported it back
/// flipped, and delivered top-down coordinates anyway: every position landed
/// a screen-height off, every batch failed the cursor check, and every
/// stroke fell back to whole-pixel cursor positions — a staircase, several
/// canvas pixels deep once zoomed out.
///
/// The cursor is the one mapping that is always right, because the driver
/// moves it from these same packets. It is only rounded to a whole pixel, and
/// a straight-line fit over many pairs averages that rounding away. So the
/// fit recovers flip, offset and scale alike, to a fraction of a pixel, with
/// no assumptions about what the driver does with the area it was asked for.
#[derive(Clone, Debug, Default)]
pub struct CursorFit {
    pairs: std::collections::VecDeque<((i32, i32), (i32, i32))>,
    last_cursor: Option<(i32, i32)>,
    misses: u32,
    pub x: Option<AxisFit>,
    pub y: Option<AxisFit>,
}

impl CursorFit {
    /// Record the newest packet of a poll against the cursor read straight
    /// after it.
    pub fn observe(&mut self, raw: (i32, i32), cursor: (i32, i32)) {
        let last = self.last_cursor.replace(cursor);
        if let Some(last) = last {
            let step = (cursor.0 - last.0).abs().max((cursor.1 - last.1).abs());
            // A pen held still would fill the window with one point, and one
            // moving fast pairs a packet with a cursor it has already left.
            if step == 0 || step > FIT_MAX_STEP {
                return;
            }
        }

        if let (Some(fx), Some(fy)) = (self.x, self.y) {
            let off = (fx.map(raw.0) as f64 - cursor.0 as f64)
                .abs()
                .max((fy.map(raw.1) as f64 - cursor.1 as f64).abs());
            if off > FIT_MISS_PX {
                self.misses += 1;
                if self.misses < FIT_MISSES_TO_RESET {
                    return;
                }
                *self = Self {
                    last_cursor: Some(cursor),
                    ..Self::default()
                };
            } else {
                self.misses = 0;
            }
        }

        self.pairs.push_back((raw, cursor));
        if self.pairs.len() > FIT_WINDOW {
            self.pairs.pop_front();
        }
        // A window that has turned noisy keeps the last good line rather than
        // dropping it. Losing the fit means falling back to the driver's own
        // mapping, which may be a screen-height out — and that stops every
        // stroke from using the pen at all.
        if let Some(f) = fit_axis(self.pairs.iter().map(|&(r, c)| (r.0, c.0))) {
            self.x = Some(f);
        }
        if let Some(f) = fit_axis(self.pairs.iter().map(|&(r, c)| (r.1, c.1))) {
            self.y = Some(f);
        }
    }

    /// Stop learning until the next `observe`, which then starts its step
    /// check afresh. Called for every poll made with the pen down: a stroke
    /// is drawn through one mapping from end to end, and drawing is also when
    /// the cursor trails the pen furthest.
    pub fn pause(&mut self) {
        self.last_cursor = None;
    }

    pub fn pairs(&self) -> usize {
        self.pairs.len()
    }
}

/// Least-squares line through `(raw, cursor)` pairs, or `None` when there
/// are too few, they span too little, or they do not lie on a line.
fn fit_axis(pairs: impl Iterator<Item = (i32, i32)> + Clone) -> Option<AxisFit> {
    let n = pairs.clone().count();
    if n < FIT_MIN_PAIRS {
        return None;
    }
    let nf = n as f64;
    let (mut sr, mut sc) = (0.0f64, 0.0f64);
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for (r, c) in pairs.clone() {
        sr += r as f64;
        sc += c as f64;
        lo = lo.min(c);
        hi = hi.max(c);
    }
    if ((hi - lo) as f64) < FIT_MIN_SPAN {
        return None;
    }
    let (mr, mc) = (sr / nf, sc / nf);
    let (mut srr, mut src) = (0.0f64, 0.0f64);
    for (r, c) in pairs.clone() {
        let (dr, dc) = (r as f64 - mr, c as f64 - mc);
        srr += dr * dr;
        src += dr * dc;
    }
    if srr <= 0.0 {
        return None;
    }
    let scale = src / srr;
    let origin = mc - scale * mr;
    let sq: f64 = pairs
        .map(|(r, c)| {
            let e = origin + scale * r as f64 - c as f64;
            e * e
        })
        .sum();
    let rms = (sq / nf).sqrt();
    (rms <= FIT_MAX_RMS).then_some(AxisFit { origin, scale, rms })
}

// --- keeping the context in step with the desktop ---
//
// The Wintab context and its startup mapping describe the desktop as it was
// when they were made. Plugging in a monitor, rearranging them, or changing
// the work area in the tablet driver leaves both describing a desktop that no
// longer exists, and nothing on the drawing path notices: the cursor fit only
// relearns after two dozen misses, and only while hovering slowly. So the
// layout is watched and the context reopened when it changes.

/// A rectangle in virtual-desktop pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl ScreenRect {
    /// The same rectangle with a non-negative size, whichever way round its
    /// extent ran (Wintab's can be negative).
    fn normalized(self) -> Self {
        let (x, w) = if self.w < 0 { (self.x + self.w, -self.w) } else { (self.x, self.w) };
        let (y, h) = if self.h < 0 { (self.y + self.h, -self.h) } else { (self.y, self.h) };
        Self { x, y, w, h }
    }

    /// Whether the two share any area — touching edges don't count.
    fn overlaps(self, o: Self) -> bool {
        let (a, b) = (self.normalized(), o.normalized());
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }
}

/// Where the tablet driver says it puts the pen: the screen area its system
/// context moves the cursor over (`lcSys*`) and the size of the tablet area
/// it reads (`lcInExt`), in tablet counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriverArea {
    pub screen: ScreenRect,
    pub tablet: (i32, i32),
}

/// The desktop a context was opened against.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DisplaySig {
    /// The virtual desktop's bounding box.
    pub desktop: ScreenRect,
    /// Every monitor, `true` for the primary. Sorted, so enumeration order
    /// can never pass for a change.
    pub monitors: Vec<(ScreenRect, bool)>,
    /// `None` when there is no driver to ask.
    pub driver: Option<DriverArea>,
}

/// Why the context was last reopened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReopenReason {
    Monitors,
    Driver,
    Requested,
}

impl ReopenReason {
    pub fn label(self) -> &'static str {
        match self {
            ReopenReason::Monitors => "monitors changed",
            ReopenReason::Driver => "driver area changed",
            ReopenReason::Requested => "reconnect pressed",
        }
    }
}

/// How often the layout is re-read. Time, not frames: the app only redraws
/// when something happens, and the first frame after a quiet spell is the
/// one that should notice.
const DISPLAY_CHECK: std::time::Duration = std::time::Duration::from_millis(500);

/// Watches the desktop and the driver's area for changes since the context
/// opened.
#[derive(Clone, Debug, Default)]
pub struct DisplayWatch {
    at_open: Option<DisplaySig>,
    last_check: Option<std::time::Instant>,
    /// A change seen mid-stroke, held until the pen lifts: reopening under a
    /// stroke would remap the rest of it.
    pending: Option<ReopenReason>,
}

impl DisplayWatch {
    /// Note what a freshly opened context was built against.
    pub fn opened(&mut self, sig: DisplaySig, now: std::time::Instant) {
        *self = Self {
            at_open: Some(sig),
            last_check: Some(now),
            pending: None,
        };
    }

    /// Whether the context should be reopened now. `read` is only called when
    /// a check is due, or straight away when `nudged` (the driver posted a
    /// change message).
    pub fn tick(
        &mut self,
        now: std::time::Instant,
        read: impl FnOnce() -> DisplaySig,
        pointer_down: bool,
        nudged: bool,
    ) -> Option<ReopenReason> {
        if self.pending.is_none() {
            let due = nudged
                || self
                    .last_check
                    .map_or(true, |t| now.saturating_duration_since(t) >= DISPLAY_CHECK);
            if !due {
                return None;
            }
            self.last_check = Some(now);
            let sig = read();
            let old = self.at_open.as_ref()?;
            self.pending = if sig.desktop != old.desktop || sig.monitors != old.monitors {
                Some(ReopenReason::Monitors)
            } else if sig.driver.is_some() && sig.driver != old.driver {
                Some(ReopenReason::Driver)
            } else {
                None
            };
        }
        if pointer_down {
            return None;
        }
        self.pending.take()
    }
}

/// Why opening the context failed, which decides when to try again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitFail {
    /// The window isn't up yet. Cheap to retry, so every frame.
    NoWindow,
    /// No Wintab32.dll. Nothing will change that until the user installs a
    /// driver and presses Reconnect.
    NoDriver,
    /// The driver is there but refused the context. Backed off, so a tablet
    /// plugged in later is still picked up without retrying every frame.
    Context,
}

/// Backoff for opening the context.
#[derive(Clone, Copy, Debug, Default)]
pub struct Retry {
    failures: u32,
    next_at: Option<std::time::Instant>,
    /// Waiting for Reconnect rather than for a time.
    parked: bool,
}

/// First and longest wait after a refused context.
const RETRY_FIRST: f32 = 0.5;
const RETRY_MAX: f32 = 10.0;

impl Retry {
    pub fn due(&self, now: std::time::Instant) -> bool {
        !self.parked && self.next_at.map_or(true, |t| now >= t)
    }

    pub fn failed(&mut self, kind: InitFail, now: std::time::Instant) {
        match kind {
            InitFail::NoWindow => self.next_at = None,
            InitFail::NoDriver => self.parked = true,
            InitFail::Context => {
                self.failures += 1;
                let wait = (RETRY_FIRST * 2f32.powi(self.failures as i32 - 1)).min(RETRY_MAX);
                self.next_at = Some(now + std::time::Duration::from_secs_f32(wait));
            }
        }
    }

    /// How long until the next attempt; `None` when it is due now or parked.
    pub fn wait(&self, now: std::time::Instant) -> Option<std::time::Duration> {
        self.next_at
            .filter(|_| !self.parked)
            .map(|t| t.saturating_duration_since(now))
            .filter(|d| !d.is_zero())
    }

    pub fn parked(&self) -> bool {
        self.parked
    }
}

/// How many monitors `area` reaches onto. More than one means the tablet is
/// spread over several screens, so any one of them gets only part of it.
pub fn monitors_spanned(area: ScreenRect, monitors: &[(ScreenRect, bool)]) -> usize {
    monitors.iter().filter(|(m, _)| m.overlaps(area)).count()
}

/// How much screen, in desktop pixels, the whole tablet covers: the packet
/// range the context reports over, times the scale the cursor fit measured.
/// `None` until both axes of the fit are trusted.
///
/// The driver's own word for this (`lcSys*` of its default context) can't be
/// taken: XP-Pen's always names the whole desktop, whatever work area is set,
/// while the cursor it moves keeps to the one monitor it was told to. Only the
/// size is measured, not where it sits — the same drivers misreport where the
/// packet range starts, too (the fit's origin absorbs that).
pub fn pen_reach(span: (i32, i32), fit_x: Option<AxisFit>, fit_y: Option<AxisFit>) -> Option<(f32, f32)> {
    let (fx, fy) = (fit_x?, fit_y?);
    Some(((fx.scale * span.0 as f64).abs() as f32, (fy.scale * span.1 as f64).abs() as f32))
}

/// Pixels a measured reach may run past a monitor before it counts as
/// reaching off it: the fit is good to a fraction of a pixel, but the packet
/// range and the screen can disagree by one at each edge.
const REACH_SLACK: f32 = 8.0;

/// Whether a tablet covering `reach` pixels fits on one of `monitors`. When
/// it doesn't, the work area is spread over several, and the window gets only
/// part of the tablet.
pub fn reach_fits_one_monitor(reach: (f32, f32), monitors: &[(ScreenRect, bool)]) -> bool {
    monitors.iter().any(|(m, _)| {
        let m = m.normalized();
        reach.0 <= m.w as f32 + REACH_SLACK && reach.1 <= m.h as f32 + REACH_SLACK
    })
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
    /// Most packets seen in a single frame. A stride error only shows itself
    /// when this is above one, so it says whether the case was even exercised.
    pub max_packets_per_frame: usize,
    /// Polls that filled the read buffer, meaning the queue had backed up.
    pub drained_full: u32,
    /// The packet layout the driver granted. `size` is the giveaway: a device
    /// that grants every field gives 76 bytes on a 64-bit build.
    pub layout: PacketLayout,
    /// The mapping learned from the cursor, per axis, once trusted, and how
    /// many pairs it is drawn from. See [`CursorFit`].
    pub fit_x: Option<AxisFit>,
    pub fit_y: Option<AxisFit>,
    pub fit_pairs: usize,
    /// Desktop pixels the whole tablet covers, as the cursor shows it — see
    /// [`pen_reach`].
    pub reach: Option<(f32, f32)>,
    /// The screen area the context was opened with (`lcSys*`).
    pub context_area: Option<ScreenRect>,
    /// The window the context is bound to, and the one `FindWindowA` finds
    /// by title — how the context was bound before it took the handle from
    /// the app. Recorded to tell the two apart, should they differ.
    pub hwnd: Option<isize>,
    pub hwnd_by_title: Option<isize>,
    /// The desktop and the driver's area, read when the snapshot is taken.
    pub display: DisplaySig,
    /// Times the context was reopened, and the latest reason.
    pub reopens: u32,
    pub last_reopen: Option<ReopenReason>,
    /// Why the last attempt to open the context failed, while there is none.
    pub init_error: Option<String>,
    /// Until the next attempt; `None` when due now or waiting for Reconnect.
    pub retry_in: Option<std::time::Duration>,
    pub waiting_for_reconnect: bool,
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
    /// The newest packet ever drained. Unlike `packets` it survives a poll
    /// that comes back empty, which is what a stroke's first sample needs:
    /// the frame that sees the press has often already drained its packet a
    /// frame earlier.
    last_packet: Option<PenPacket>,
    last_pressure: f32,
    idle_frames: u32,
    /// The app window, once known. Looked up by title only until then.
    hwnd: Option<isize>,
    retry: Retry,
    watch: DisplayWatch,
    /// Reconnect was pressed; honoured on the next poll with the pen up.
    reconnect: bool,
    reopens: u32,
    last_reopen: Option<ReopenReason>,
    init_error: Option<String>,
}

impl PenInput {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "windows")]
            backend: None,
            #[cfg(not(target_os = "windows"))]
            backend: (),
            packets: Vec::new(),
            last_packet: None,
            last_pressure: 1.0,
            idle_frames: u32::MAX,
            hwnd: None,
            retry: Retry::default(),
            watch: DisplayWatch::default(),
            reconnect: false,
            reopens: 0,
            last_reopen: None,
            init_error: None,
        }
    }

    /// The app window's handle, so the context binds to this window rather
    /// than to the first one titled "Animator" — which, with two instances
    /// open, need not be this one.
    pub fn set_hwnd(&mut self, hwnd: isize) {
        self.hwnd = Some(hwnd);
    }

    pub fn has_hwnd(&self) -> bool {
        self.hwnd.is_some()
    }

    /// Close and reopen the context on the next poll with the pen up, and try
    /// again at once if opening had failed.
    pub fn reconnect(&mut self) {
        self.reconnect = true;
    }

    /// Called once per frame. Opens the context lazily (once the window
    /// exists), reopens it when the desktop or the driver's area has changed,
    /// and drains the whole packet queue into `packets`.
    ///
    /// `pointer_down` freezes the idle count: a pen held still mid-stroke
    /// produces no packets, and handing the stroke to the mouse there — at
    /// pressure 1.0 — stamps a full-size blob into the line. It also holds
    /// off any reopen until the stroke is over.
    pub fn poll(&mut self, pointer_down: bool) {
        self.packets.clear();
        #[cfg(target_os = "windows")]
        {
            let now = std::time::Instant::now();
            let nudged = windows_backend::take_nudge();
            let reason = if self.reconnect && !pointer_down {
                Some(ReopenReason::Requested)
            } else if self.backend.is_some() {
                let backend = self.backend.as_ref();
                self.watch
                    .tick(now, || windows_backend::display_sig(backend), pointer_down, nudged)
            } else {
                None
            };
            if let Some(reason) = reason {
                if self.backend.take().is_some() {
                    log::info!("Wintab context reopened: {}", reason.label());
                    self.reopens += 1;
                    self.last_reopen = Some(reason);
                    // Positions from the old mapping mean nothing now.
                    self.last_packet = None;
                }
                self.reconnect = false;
                self.retry = Retry::default();
            }
            if self.backend.is_none() && self.retry.due(now) {
                match windows_backend::Wintab::try_init(self.hwnd) {
                    Ok(w) => {
                        log::info!("Wintab pen backend ready");
                        self.watch.opened(windows_backend::display_sig(Some(&w)), now);
                        self.retry = Retry::default();
                        self.init_error = None;
                        self.backend = Some(w);
                    }
                    Err((kind, e)) => {
                        if kind != InitFail::NoWindow {
                            log::debug!("Wintab init failed: {e}");
                        }
                        self.retry.failed(kind, now);
                        self.init_error = Some(e);
                    }
                }
            }
            if let Some(w) = &mut self.backend {
                w.poll(&mut self.packets, pointer_down);
            }
        }
        self.track_idle(pointer_down);
    }

    /// Idle bookkeeping for the packets `poll` just drained.
    ///
    /// The count only runs while nothing is pressed. It used to run always,
    /// with only the pressure reset gated — but `pen_active` reads the count
    /// directly, so a pen paused for `PEN_IDLE_FRAMES` mid-stroke (a live
    /// stroke repaints every frame) went inactive anyway, and the next sample
    /// came from the mouse path at pressure 1.0.
    fn track_idle(&mut self, pointer_down: bool) {
        if let Some(&last) = self.packets.last() {
            self.last_pressure = last.pressure;
            self.last_packet = Some(last);
            self.idle_frames = 0;
        } else if !pointer_down {
            self.idle_frames = self.idle_frames.saturating_add(1);
            // Idle with nothing pressed: assume the mouse took over. Without
            // this the pressure left behind by a pen lift (~0.0) would make
            // every subsequent mouse stroke a hairline.
            if self.idle_frames > PEN_IDLE_FRAMES {
                self.last_pressure = 1.0;
            }
        }
    }

    /// This frame's tablet packets, oldest first.
    pub fn packets(&self) -> &[PenPacket] {
        &self.packets
    }

    /// The most recent packet, from this frame or any earlier one. `None`
    /// until the pen has reported at all.
    pub fn last_packet(&self) -> Option<PenPacket> {
        self.last_packet
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
        d.reopens = self.reopens;
        d.last_reopen = self.last_reopen;
        d.init_error = self.init_error.clone().filter(|_| !self.is_active());
        d.retry_in = self.retry.wait(std::time::Instant::now());
        d.waiting_for_reconnect = self.retry.parked();
        d.hwnd = self.hwnd;
        #[cfg(target_os = "windows")]
        {
            d.display = windows_backend::display_sig(self.backend.as_ref());
            d.hwnd_by_title = windows_backend::window_by_title();
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
            d.max_packets_per_frame = w.max_packets_per_frame;
            d.drained_full = w.drained_full;
            d.fit_x = w.fit.x;
            d.fit_y = w.fit.y;
            d.fit_pairs = w.fit.pairs();
            d.reach = pen_reach(w.out_span, w.fit.x, w.fit.y);
            d.context_area = Some(w.sys_area);
            // Mapped from the retained raw value rather than from this frame's
            // packets, which are empty whenever the pen holds still — the
            // readout would otherwise blink out exactly while being read.
            d.last_mapped = w.last_raw.map(|(x, y)| w.map_point(x, y));
        }
        d
    }

    /// True while the pen is the live input device: a backend exists and it
    /// has produced packets recently. False for mouse input, so callers know
    /// to ignore `current_pressure` and the packet positions.
    pub fn pen_active(&self) -> bool {
        self.is_active() && self.recently_seen()
    }

    /// The backend-independent half of `pen_active`.
    fn recently_seen(&self) -> bool {
        self.idle_frames <= PEN_IDLE_FRAMES
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
    //! Wintab backend. The app hands over its window handle on the first
    //! frame; until then the window is found by title.

    use super::{InitFail, PenPacket, ScreenRect};
    use anyhow::{anyhow, Context, Result};
    use libloading::{Library, Symbol};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::OnceLock;
    use windows::core::PCSTR;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
    use windows::Win32::Graphics::Gdi::{
        ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowA, GetClientRect, GetSystemMetrics, MONITORINFOF_PRIMARY, MSG,
        SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };
    use super::{read_i32, read_u32, PacketLayout};
    use wintab_lite::{
        cast_void, WTClose, WTInfo, WTOpen, WTPacketsGet, AXIS, CSR, CXO, DVC, HCTX, LOGCONTEXT, WTI,
        WTPKT,
    };

    /// Wintab's twist (tenths of a degree, clockwise) as Krita reads it:
    /// `360 - twist / 10`, folded into -180..=180.
    pub(super) fn map_twist(twist: i32) -> f32 {
        let mut r = 360.0 - twist as f64 / 10.0;
        if r > 180.0 {
            r -= 360.0;
        }
        r as f32
    }

    /// Polls after which a context that has never produced a packet is
    /// called out. Roughly a few seconds of drawing.
    const SILENCE_WARN_POLLS: u32 = 300;

    /// How much finer than a screen pixel to ask for. Past the tablet's own
    /// resolution on any desktop-sized mapping, deliberately: the quantization
    /// that remains should be the hardware's, not ours.
    const SUBPIXEL: i32 = 32;

    /// Upper bound on one packet, for sizing the read buffer. The documented
    /// fields come to 76 bytes; this leaves room for a driver that reports
    /// more than was asked for.
    const MAX_PACKET_BYTES: usize = 256;

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
        /// Most packets delivered in one poll.
        pub max_packets_per_frame: usize,
        /// Polls that came back with a completely full buffer, meaning the
        /// queue had backed up past what one call can take.
        pub drained_full: u32,
        /// Affine packet → virtual-desktop mapping, per axis: the desktop
        /// origin, the packet-space origin, and desktop units per packet unit.
        pub map_x: (f32, f32, f32),
        pub map_y: (f32, f32, f32),
        /// The size of the packet range the whole tablet reports over, per
        /// axis — the granted output extent, made positive.
        pub out_span: (i32, i32),
        /// The screen area the context was opened with (`lcSys*`).
        pub sys_area: ScreenRect,
        /// Whether the device reports azimuth/altitude.
        tilt_support: bool,
        /// The tangential-pressure axis's span, 0 without one.
        tangent_span: f32,
        /// Each cursor's `CSR_TYPE`, asked once: only an airbrush reports
        /// tangential pressure.
        cursor_types: std::collections::HashMap<u32, u32>,
        /// `WTInfoA` with its category as a plain number, for the
        /// multiplexed cursor categories.
        wt_info_raw: libloading::Symbol<'static, unsafe extern "C" fn(u32, u32, *mut std::ffi::c_void) -> u32>,
        /// Refreshed every poll; see `PenInput::client_origin`.
        pub client_origin: Option<(f32, f32)>,
        /// The mapping as the cursor shows it to be. Preferred over
        /// `map_x`/`map_y` per axis once trusted.
        pub fit: super::CursorFit,
        pub queue_err_logged: bool,
        /// Polls since the context opened, and whether any packet has ever
        /// arrived. A context that opens but never delivers looks exactly like
        /// having no tablet at all, so say so rather than fail silently.
        polls: u32,
        seen_packets: bool,
    }

    /// Wintab32.dll, loaded once for the life of the process. Every symbol
    /// borrows from it, which is what lets `Wintab` hold them as `'static`;
    /// reopening a context must not load (and leak) another copy.
    static LIB: OnceLock<Library> = OnceLock::new();

    fn library() -> Result<&'static Library> {
        if let Some(lib) = LIB.get() {
            return Ok(lib);
        }
        let lib = unsafe { Library::new("Wintab32.dll") }
            .context("Wintab32.dll not found (no tablet driver?)")?;
        Ok(LIB.get_or_init(|| lib))
    }

    /// The message base the open context was given, for spotting the
    /// driver's change notices in [`on_msg`].
    static MSG_BASE: AtomicU32 = AtomicU32::new(0x7FF0);
    /// Set by [`on_msg`], taken by the next poll.
    static NUDGE: AtomicBool = AtomicBool::new(false);

    /// See every message the event loop reads. `WT_CTXUPDATE` and
    /// `WT_INFOCHANGE` (base + 3, + 6) say the driver changed something, so
    /// the next poll checks the layout at once instead of within half a
    /// second. A hint only: `WM_DISPLAYCHANGE` is sent rather than posted and
    /// never passes through here, which is why the poll checks regardless.
    pub fn on_msg(msg: &MSG) {
        let base = MSG_BASE.load(Ordering::Relaxed);
        if msg.message == base + 3 || msg.message == base + 6 {
            NUDGE.store(true, Ordering::Relaxed);
        }
    }

    pub fn take_nudge() -> bool {
        NUDGE.swap(false, Ordering::Relaxed)
    }

    /// The window titled "Animator" — the title main.rs sets — which is how
    /// the context is bound until the app hands its own handle over.
    pub fn window_by_title() -> Option<isize> {
        let h = unsafe { FindWindowA(None, PCSTR(c"Animator".as_ptr().cast())) };
        (h.0 != 0).then_some(h.0)
    }

    /// The desktop as it stands, and the driver's area when there is a
    /// context to ask through.
    pub fn display_sig(w: Option<&Wintab>) -> super::DisplaySig {
        let metric = |i| unsafe { GetSystemMetrics(i) };
        let desktop = ScreenRect {
            x: metric(SM_XVIRTUALSCREEN),
            y: metric(SM_YVIRTUALSCREEN),
            w: metric(SM_CXVIRTUALSCREEN),
            h: metric(SM_CYVIRTUALSCREEN),
        };
        unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(m, &mut info).as_bool() {
                let r = info.rcMonitor;
                let out = &mut *(out.0 as *mut Vec<(super::ScreenRect, bool)>);
                out.push((
                    super::ScreenRect {
                        x: r.left,
                        y: r.top,
                        w: r.right - r.left,
                        h: r.bottom - r.top,
                    },
                    info.dwFlags & MONITORINFOF_PRIMARY != 0,
                ));
            }
            true.into()
        }
        let mut monitors: Vec<(ScreenRect, bool)> = Vec::new();
        unsafe {
            let _ = EnumDisplayMonitors(
                HDC::default(),
                None,
                Some(each),
                LPARAM(&mut monitors as *mut _ as isize),
            );
        }
        monitors.sort();
        let driver = w.and_then(|w| {
            let mut lc = LOGCONTEXT::default();
            let n = unsafe { (w.wt_info_raw)(WTI::DEFSYSCTX as u32, 0, cast_void!(lc)) };
            (n != 0).then_some(super::DriverArea {
                screen: ScreenRect {
                    x: lc.lcSysOrgXY.x,
                    y: lc.lcSysOrgXY.y,
                    w: lc.lcSysExtXY.x,
                    h: lc.lcSysExtXY.y,
                },
                tablet: (lc.lcInExtXYZ.x, lc.lcInExtXYZ.y),
            })
        });
        super::DisplaySig {
            desktop,
            monitors,
            driver,
        }
    }

    impl Wintab {
        /// Open a context on `hwnd`, or on the window found by title when the
        /// app hasn't handed its handle over yet. A failure says which kind it
        /// was, which decides when to try again.
        pub fn try_init(hwnd: Option<isize>) -> std::result::Result<Self, (InitFail, String)> {
            let hwnd = match hwnd {
                Some(h) => HWND(h),
                None => match window_by_title() {
                    Some(h) => HWND(h),
                    None => return Err((InitFail::NoWindow, "window not up yet".into())),
                },
            };
            let lib = library().map_err(|e| (InitFail::NoDriver, format!("{e:#}")))?;
            Self::open(hwnd, lib).map_err(|e| (InitFail::Context, format!("{e:#}")))
        }

        fn open(hwnd: HWND, lib: &'static Library) -> Result<Self> {
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

            let base_out_org = log_context.lcOutOrgXYZ;
            let base_out_ext = log_context.lcOutExtXYZ;
            // The driver's own area, as reported, and left as it is: while the
            // app is active its context is on top and steers the pointer, and
            // with XP-Pen any other area here moved the pointer off the work
            // area — a monitor-sized one shrank the pen to 61% of the monitor
            // (2560 / 4160), and a non-system context got no packets at all.
            let sys_org = log_context.lcSysOrgXY;
            let sys_ext = log_context.lcSysExtXY;
            if sys_ext.x == 0 || sys_ext.y == 0 || base_out_ext.x == 0 || base_out_ext.y == 0 {
                return Err(anyhow!("DEFSYSCTX has an empty mapping"));
            }

            // The output area we want packets reported in: screen coordinates.
            //
            // Wintab maps the tablet linearly onto this area, and a tablet's Y
            // axis increases *upward* where the screen's increases downward.
            // So the Y extent has to be negative — the top of the tablet has
            // to land on the smallest screen y. The default area has a
            // positive extent, which is why packets arrived mirrored about the
            // middle of the screen: exactly right along that line and wrong by
            // twice the distance from it everywhere else.
            //
            // This is what Qt's `lcOutExtY = -lcInExtY` does. The flip does
            // not live in the default output area, and the cursor's own
            // correctness says nothing about it: the driver moves the cursor
            // through lcSys*, which is a separate path.
            let want_org_x = sys_org.x;
            let want_ext_x = sys_ext.x;
            let want_org_y = sys_org.y + sys_ext.y;
            let want_ext_y = -sys_ext.y;

            log_context.lcName.write_str("animator-app");
            // Stay a system context. Qt only *adds* flags to the default
            // system context, and a system context still honours lcOut* for
            // the coordinates it reports — lcSys* is what drives the cursor,
            // separately. Clearing CXO_SYSTEM here stopped packets arriving
            // at all, which is worth a comment because the fix looks like a
            // no-op otherwise.
            log_context.lcOptions |= CXO::SYSTEM;
            // Only the fields actually read. Asking for everything invites a
            // packet whose length depends on what the device happens to
            // support, and the length is the one thing that must be right:
            // every packet after the first in a batch is found by it. Five
            // documented fields, none of them optional extensions, is a packet
            // this code can size exactly.
            // Time, cursor and tangential pressure feed Krita's speed sensor
            // and airbrush wheel; the layout below is built from whatever is
            // actually granted, so a driver that refuses them costs nothing.
            log_context.lcPktData = WTPKT::STATUS
                | WTPKT::TIME
                | WTPKT::CURSOR
                | WTPKT::X
                | WTPKT::Y
                | WTPKT::NORMAL_PRESSURE
                | WTPKT::TANGENT_PRESSURE
                | WTPKT::ORIENTATION;
            log_context.lcPktMode = WTPKT::empty();
            // Twisting the barrel or turning the wheel with the pen held
            // still is input too.
            log_context.lcMoveMask = WTPKT::X
                | WTPKT::Y
                | WTPKT::NORMAL_PRESSURE
                | WTPKT::TANGENT_PRESSURE
                | WTPKT::ORIENTATION;

            // The same area, scaled up, so positions arrive on a fraction
            // of a screen pixel instead of on whole ones.
            //
            // A tablet resolves far finer than a screen pixel — roughly five
            // device counts to one on a desktop-sized mapping — and whole-pixel
            // positions turn a shallow line into a staircase, which is what a
            // zoomed-out or high-resolution canvas magnifies into visible
            // wobble. Asking for a bigger output area is how that resolution
            // is kept.
            //
            // Safe to ask for because the map below is built from what was
            // wanted against what was granted: a driver that refuses this, or
            // grants something else entirely, is corrected rather than
            // believed. An earlier version of this scaled the *wrong* rectangle
            // and had no such correction, which is how it inverted Y.
            log_context.lcOutOrgXYZ.x = want_org_x * SUBPIXEL;
            log_context.lcOutExtXYZ.x = want_ext_x * SUBPIXEL;
            log_context.lcOutOrgXYZ.y = want_org_y * SUBPIXEL;
            log_context.lcOutExtXYZ.y = want_ext_y * SUBPIXEL;

            let mut pressure_axis = AXIS::default();
            let pr =
                unsafe { wt_info(WTI::DEVICES, DVC::NPRESSURE as u32, cast_void!(pressure_axis)) };
            let (pressure_min, pressure_max) = if pr as usize == std::mem::size_of::<AXIS>() {
                (pressure_axis.axMin as f32, pressure_axis.axMax as f32)
            } else {
                (0.0, 1023.0)
            };
            // The airbrush wheel's range. Krita scales by its span alone.
            let mut tangent_axis = AXIS::default();
            let tr = unsafe { wt_info(WTI::DEVICES, DVC::TPRESSURE as u32, cast_void!(tangent_axis)) };
            let tangent_span = if tr as usize == std::mem::size_of::<AXIS>() {
                (tangent_axis.axMax as f32 - tangent_axis.axMin as f32).max(0.0)
            } else {
                0.0
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
                // Finer positions are worth asking for, not worth losing the
                // tablet over.
                log::warn!("Wintab refused a {SUBPIXEL}x output area; retrying at screen scale");
                log_context.lcOutOrgXYZ.x = want_org_x;
                log_context.lcOutExtXYZ.x = want_ext_x;
                log_context.lcOutOrgXYZ.y = want_org_y;
                log_context.lcOutExtXYZ.y = want_ext_y;
                ctx_handle = unsafe { wt_open(hwnd, &mut log_context, 1) };
            }
            if ctx_handle.is_null() {
                return Err(anyhow!("WTOpen returned null"));
            }
            MSG_BASE.store(log_context.lcMsgBase, Ordering::Relaxed);

            // WTOpen rewrites the struct with the area the driver settled
            // on, which need not be the one asked for.
            let out_org = log_context.lcOutOrgXYZ;
            let out_ext = log_context.lcOutExtXYZ;
            if out_ext.x == 0 || out_ext.y == 0 {
                let _ = unsafe { wt_close(ctx_handle) };
                return Err(anyhow!("Wintab granted an empty output area"));
            }
            // Built from what we *want* against what was *granted*, so it
            // corrects the difference — including a refused Y flip, which
            // simply comes back as a negative scale here instead. Getting the
            // mapping right must not depend on the request being honoured.
            let map_x = super::axis_map(want_org_x, want_ext_x, out_org.x, out_ext.x);
            let map_y = super::axis_map(want_org_y, want_ext_y, out_org.y, out_ext.y);

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
                // Deliberately far larger than the layout says it needs to be.
                // If this code ever *under*-estimates a packet, the driver
                // writes past the end of the buffer, and a wrong stroke is a
                // much better failure than a corrupted heap.
                buf: vec![0u8; QUEUE_SIZE as usize * MAX_PACKET_BYTES],
                hwnd,
                pressure_min,
                pressure_span,
                tangent_span,
                cursor_types: std::collections::HashMap::new(),
                wt_info_raw: unsafe { lib.get(c"WTInfoA".to_bytes()).context("WTInfoA")? },
                map_x,
                map_y,
                out_span: (out_ext.x.abs(), out_ext.y.abs()),
                sys_area: ScreenRect { x: sys_org.x, y: sys_org.y, w: sys_ext.x, h: sys_ext.y },
                tilt_support,
                last_raw: None,
                total_packets: 0,
                max_packets_per_frame: 0,
                drained_full: 0,
                client_origin: None,
                fit: super::CursorFit::default(),
                queue_err_logged: false,
                polls: 0,
                seen_packets: false,
            })
        }

        /// Packet coordinate → virtual-desktop pixels. Each axis goes through
        /// the cursor fit once it is trusted, and through the area the driver
        /// reported until then.
        #[inline]
        pub fn map_point(&self, x: i32, y: i32) -> (f32, f32) {
            let (sx, ox, kx) = self.map_x;
            let (sy, oy, ky) = self.map_y;
            (
                self.fit.x.map_or(sx + (x as f32 - ox) * kx, |f| f.map(x)),
                self.fit.y.map_or(sy + (y as f32 - oy) * ky, |f| f.map(y)),
            )
        }

        /// Whether cursor `index` is an airbrush (`QTabletEvent::Airbrush`:
        /// `CSR_TYPE & 0x0F06 == 0x0902`).
        fn is_airbrush(&mut self, index: u32) -> bool {
            let ty = *self.cursor_types.entry(index).or_insert_with(|| {
                let mut ty: u32 = 0;
                // WTI_CURSORS + n is a category of its own, outside the enum.
                let n = unsafe { (self.wt_info_raw)(WTI::CURSORS as u32 + index, CSR::TYPE as u32, cast_void!(ty)) };
                if n as usize == std::mem::size_of::<u32>() {
                    ty
                } else {
                    0
                }
            });
            ty & 0x0F06 == 0x0902
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
        ///
        /// One call to `WTPacketsGet` takes at most a bufferful, so a queue
        /// that has backed up past that — which happens whenever a frame runs
        /// long, or the window goes without redrawing — would otherwise never
        /// catch up, and would keep handing back positions from further and
        /// further in the past.
        /// `pointer_down` holds the cursor fit still; see `CursorFit::pause`.
        pub fn poll(&mut self, out: &mut Vec<PenPacket>, pointer_down: bool) {
            self.client_origin = client_origin(self.hwnd);
            let seen = self.total_packets;
            // Bounded: a pen that could outrun this is not one we can draw
            // with anyway, and an unbounded loop here would stall a frame.
            for _ in 0..8 {
                let before = out.len();
                self.poll_once(out);
                if out.len() - before < QUEUE_SIZE as usize {
                    break;
                }
            }
            // The cursor is read straight after the drain, so it stands where
            // the newest packet put it.
            if pointer_down {
                self.fit.pause();
            } else if self.total_packets != seen {
                if let (Some(raw), Some(cursor)) =
                    (self.last_raw, crate::input::screen_pixel::cursor_pos())
                {
                    self.fit.observe(raw, cursor);
                }
            }
        }

        fn poll_once(&mut self, out: &mut Vec<PenPacket>) {

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
            if removed == MAX {
                // A full buffer means more is queued behind it. Counted, so a
                // backlog is visible rather than merely suspected; `poll` is
                // called again below until the queue is actually empty.
                self.drained_full = self.drained_full.saturating_add(1);
            }

            self.max_packets_per_frame = self.max_packets_per_frame.max(removed);

            let layout = self.layout;
            let (Some(x_at), Some(y_at)) = (layout.x, layout.y) else {
                return;
            };
            debug_assert!(layout.size <= MAX_PACKET_BYTES);

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
                let rotation = match (layout.orientation, self.tilt_support) {
                    (Some(at), true) => map_twist(read_i32(p, at + 8)),
                    _ => 0.0,
                };
                let time_ms = layout.time.map_or(0.0, |at| read_u32(p, at) as f64);
                let cursor = layout.cursor.map(|at| read_u32(p, at));
                let tangent = layout.tangent.map(|at| read_u32(p, at));

                if !self.seen_packets {
                    self.seen_packets = true;
                    log::info!(
                        "Wintab first packet: raw ({rx}, {ry}) -> desktop ({x:.2}, {y:.2})"
                    );
                }
                self.last_raw = Some((rx, ry));
                self.total_packets = self.total_packets.saturating_add(1);
                let tangential = match (tangent, cursor) {
                    (Some(t), Some(c)) if self.tangent_span > 0.0 && self.is_airbrush(c) => t as f32 / self.tangent_span,
                    _ => 0.0,
                };

                out.push(PenPacket {
                    x,
                    y,
                    pressure,
                    tilt_x,
                    tilt_y,
                    time_ms,
                    rotation,
                    tangential,
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

#[cfg(target_os = "windows")]
pub use windows_backend::on_msg;

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

    /// The fields asked for now: time, cursor and tangential pressure
    /// land at their places in the bit order, around the old ones.
    #[test]
    fn time_cursor_and_tangent_have_their_offsets() {
        let mask = (1 << 1) | (1 << 2) | (1 << 5) | (1 << 7) | (1 << 8) | (1 << 10) | (1 << 11) | (1 << 12);
        let l = PacketLayout::from_mask(mask);
        assert_eq!(l.status, Some(0));
        assert_eq!(l.time, Some(4));
        assert_eq!(l.cursor, Some(8));
        assert_eq!((l.x, l.y), (Some(12), Some(16)));
        assert_eq!(l.pressure, Some(20));
        assert_eq!(l.tangent, Some(24));
        assert_eq!(l.orientation, Some(28));
        assert_eq!(l.size, 40);
        // A driver that refuses the new ones leaves the old offsets where
        // the old request had them.
        let old = PacketLayout::from_mask((1 << 1) | (1 << 7) | (1 << 8) | (1 << 10) | (1 << 12));
        assert_eq!((old.time, old.cursor, old.tangent), (None, None, None));
        assert_eq!((old.x, old.pressure, old.orientation), (Some(4), Some(12), Some(16)));
    }

    #[cfg(windows)]
    #[test]
    fn twist_reads_like_krita() {
        use super::windows_backend::map_twist;
        assert_eq!(map_twist(0), 0.0, "360 folds to 0");
        assert_eq!(map_twist(900), -90.0);
        assert_eq!(map_twist(2700), 90.0);
        assert_eq!(map_twist(1800), 180.0);
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

    /// The live configuration: a Y axis that must come out top-down, on an
    /// output area scaled up for sub-pixel resolution. Both at once, because
    /// each was got wrong in the presence of the other.
    #[test]
    fn a_flipped_axis_scaled_up_lands_on_screen_coordinates() {
        // Screen y runs 0..1440, so the tablet's upward axis is asked for as
        // origin 1440, extent -1440, times 32.
        let map = axis_map(1440, -1440, 1440 * 32, -1440 * 32);
        assert_eq!(apply(map, 1440 * 32), 1440.0, "device bottom -> screen bottom");
        assert_eq!(apply(map, 0), 0.0, "device top -> screen top");
        assert_eq!(apply(map, 720 * 32), 720.0, "and the middle stays put");
        // The point of the exercise: positions between whole pixels.
        assert!((apply(map, 720 * 32 - 16) - 719.5).abs() < 1e-3);
    }

    /// A driver that ignores both the scale and the flip hands back its own
    /// bottom-up, screen-scaled area. The map has to correct both, or every
    /// position comes out mirrored about the middle of the screen — which is
    /// the failure this whole arrangement exists to make impossible.
    #[test]
    fn a_refused_flip_is_corrected_rather_than_believed() {
        let map = axis_map(1440, -1440, 0, 1440);
        assert_eq!(apply(map, 0), 1440.0, "packet 0 is the bottom of the screen");
        assert_eq!(apply(map, 1440), 0.0, "packet 1440 is the top");
        assert_eq!(apply(map, 720), 720.0);
    }

    fn packet(pressure: f32) -> PenPacket {
        PenPacket {
            x: 0.0,
            y: 0.0,
            pressure,
            tilt_x: 0.0,
            tilt_y: 0.0,
            time_ms: 0.0,
            rotation: 0.0,
            tangential: 0.0,
        }
    }

    /// The blob regression. A pen held still mid-stroke sends no packets while
    /// the stroke keeps repainting; it must stay the live device, at its own
    /// pressure, however long the pause.
    #[test]
    fn a_pen_paused_mid_stroke_keeps_its_pressure() {
        let mut pen = PenInput::new();
        pen.packets.push(packet(0.3));
        pen.track_idle(true);
        pen.packets.clear();
        for _ in 0..PEN_IDLE_FRAMES * 10 {
            pen.track_idle(true);
        }
        assert!(pen.recently_seen());
        assert_eq!(pen.last_pressure, 0.3);
    }

    /// A pen hovering around the screen, as `(true x, true y)` in screen
    /// pixels, a few pixels per step.
    fn hover_path() -> Vec<(f64, f64)> {
        (0..600)
            .map(|i| {
                let t = i as f64 * 0.01;
                (700.0 + 300.0 * (t * 1.3).sin(), 500.0 + 250.0 * (t * 0.9).cos())
            })
            .collect()
    }

    /// Feed `path` through a driver whose packets are `raw(true position)`,
    /// with the cursor on the whole pixel nearest the truth.
    fn learn(fit: &mut CursorFit, path: &[(f64, f64)], raw: impl Fn(f64, f64) -> (i32, i32)) {
        for &(x, y) in path {
            fit.observe(raw(x, y), (x.round() as i32, y.round() as i32));
        }
    }

    fn worst_error(fit: &CursorFit, path: &[(f64, f64)], raw: impl Fn(f64, f64) -> (i32, i32)) -> f64 {
        let (fx, fy) = (fit.x.expect("x fitted"), fit.y.expect("y fitted"));
        path.iter()
            .map(|&(x, y)| {
                let (rx, ry) = raw(x, y);
                (fx.map(rx) as f64 - x).abs().max((fy.map(ry) as f64 - y).abs())
            })
            .fold(0.0, f64::max)
    }

    /// The driver seen in the field: accepted a Y-flipped output area,
    /// reported it back flipped, and delivered top-down coordinates offset
    /// by the flipped origin anyway. The fit must land on the cursor to a
    /// fraction of a pixel regardless.
    #[test]
    fn a_driver_that_misreports_its_area_is_fitted_from_the_cursor() {
        let raw = |x: f64, y: f64| ((x * 32.0).round() as i32, 46048 + (y * 32.0).round() as i32);
        let mut fit = CursorFit::default();
        let path = hover_path();
        learn(&mut fit, &path, raw);
        let worst = worst_error(&fit, &path, raw);
        assert!(worst < 0.35, "fitted positions off by up to {worst} px");
    }

    /// And one that really does flip, at a scale nobody asked for.
    #[test]
    fn a_flipped_axis_at_any_scale_is_fitted() {
        let raw = |x: f64, y: f64| {
            (
                (x * 7.3).round() as i32 + 1200,
                ((1439.0 - y) * 11.0).round() as i32,
            )
        };
        let mut fit = CursorFit::default();
        let path = hover_path();
        learn(&mut fit, &path, raw);
        assert!(fit.y.unwrap().scale < 0.0);
        let worst = worst_error(&fit, &path, raw);
        assert!(worst < 0.35, "fitted positions off by up to {worst} px");
    }

    /// Until the pen has moved enough to pin a slope down, there is no fit,
    /// and the driver's own mapping stays in charge.
    #[test]
    fn too_little_travel_is_not_trusted() {
        let mut fit = CursorFit::default();
        let path: Vec<(f64, f64)> = (0..200)
            .map(|i| (300.0 + (i % 20) as f64, 300.0 + (i % 17) as f64))
            .collect();
        learn(&mut fit, &path, |x, y| ((x * 32.0) as i32, (y * 32.0) as i32));
        assert!(fit.x.is_none() && fit.y.is_none());
    }

    /// A pen resting in place must not flood the window with one point.
    #[test]
    fn a_still_pen_adds_one_pair() {
        let mut fit = CursorFit::default();
        for _ in 0..50 {
            fit.observe((3200, 3200), (100, 100));
        }
        assert_eq!(fit.pairs(), 1);
    }

    /// Rearrange the monitors and the old fit is simply wrong. It must be
    /// dropped and relearned, not averaged into the new one.
    #[test]
    fn a_changed_mapping_is_relearned() {
        let before = |x: f64, y: f64| ((x * 32.0).round() as i32, (y * 32.0).round() as i32);
        let after = |x: f64, y: f64| (((x - 1920.0) * 32.0).round() as i32, (y * 32.0).round() as i32);
        let mut fit = CursorFit::default();
        let path = hover_path();
        learn(&mut fit, &path, before);
        learn(&mut fit, &path, after);
        let worst = worst_error(&fit, &path, after);
        assert!(worst < 0.35, "still mapping the old layout: off by {worst} px");
    }

    /// The regression from the field: a stroke fast enough that the cursor
    /// trails the pen used to push the fit out of trust mid-stroke, and the
    /// stroke fell back to the cursor. A run of lagging pairs must neither
    /// move a trusted fit nor lose it.
    #[test]
    fn a_lagging_cursor_neither_bends_nor_drops_the_fit() {
        let raw = |x: f64, y: f64| ((x * 32.0).round() as i32, 46048 + (y * 32.0).round() as i32);
        let mut fit = CursorFit::default();
        let path = hover_path();
        learn(&mut fit, &path, raw);
        let (x0, y0) = (fit.x, fit.y);

        // The cursor ten pixels behind the pen, then three behind — outliers,
        // then pairs merely noisy enough to push the window's scatter up.
        for &(x, y) in path.iter().take(FIT_MISSES_TO_RESET as usize - 1) {
            fit.observe(raw(x, y), ((x - 10.0).round() as i32, (y - 10.0).round() as i32));
        }
        assert_eq!((fit.x, fit.y), (x0, y0), "outliers moved the fit");
        for &(x, y) in path.iter().skip(100).take(200) {
            fit.observe(raw(x, y), ((x - 3.0).round() as i32, (y + 3.0).round() as i32));
        }
        assert!(fit.x.is_some() && fit.y.is_some(), "noise dropped the fit");
    }

    /// A stroke's first sample comes from the newest packet, and the press is
    /// often seen a frame after that packet was drained. An empty poll must
    /// not forget it.
    #[test]
    fn the_last_packet_survives_an_empty_poll() {
        let mut pen = PenInput::new();
        assert!(pen.last_packet().is_none());
        pen.packets.push(packet(0.4));
        pen.track_idle(false);
        pen.packets.clear();
        pen.track_idle(true);
        assert_eq!(pen.last_packet().map(|p| p.pressure), Some(0.4));
    }

    /// And once lifted and left alone, the mouse takes over at full pressure.
    #[test]
    fn an_idle_pen_hands_over_to_the_mouse() {
        let mut pen = PenInput::new();
        pen.packets.push(packet(0.0));
        pen.track_idle(false);
        pen.packets.clear();
        for _ in 0..=PEN_IDLE_FRAMES {
            pen.track_idle(false);
        }
        assert!(!pen.recently_seen());
        assert_eq!(pen.last_pressure, 1.0);
    }

    /// A driver free to grant a different origin as well as a different extent
    /// must still land correctly.
    #[test]
    fn an_offset_output_origin_is_carried_through() {
        let map = axis_map(-1920, 1920, -1920 * 8, 1920 * 8);
        assert_eq!(apply(map, -1920 * 8), -1920.0);
        assert_eq!(apply(map, 0), 0.0);
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> ScreenRect {
        ScreenRect { x, y, w, h }
    }

    /// The reported setup: a 2560×1440 primary with a 1600×900 display to its
    /// right, and a Deco Pro M (16:9) mapped over both.
    fn reported() -> DisplaySig {
        DisplaySig {
            desktop: rect(0, 0, 4160, 1440),
            monitors: vec![(rect(0, 0, 2560, 1440), true), (rect(2560, 0, 1600, 900), false)],
            driver: Some(DriverArea {
                screen: rect(0, 0, 4160, 1440),
                tablet: (50800, 28575),
            }),
        }
    }

    #[test]
    fn the_watch_reopens_only_on_a_real_change() {
        let t0 = std::time::Instant::now();
        let later = |ms| t0 + std::time::Duration::from_millis(ms);
        let mut w = DisplayWatch::default();
        // Nothing opened yet: nothing to compare against.
        assert_eq!(w.tick(t0, reported, false, false), None);
        w.opened(reported(), t0);
        assert_eq!(w.tick(later(600), reported, false, false), None);
        // Not due yet: the layout isn't even read.
        let unread = || -> DisplaySig { panic!("read before it was due") };
        assert_eq!(w.tick(later(700), unread, false, false), None);
        // A monitor moved.
        let moved = || {
            let mut s = reported();
            s.monitors[1].0.y = 200;
            s
        };
        assert_eq!(w.tick(later(1200), moved, false, false), Some(ReopenReason::Monitors));
        // The work area narrowed to the primary in the driver.
        w.opened(reported(), later(1200));
        let narrowed = || {
            let mut s = reported();
            s.driver.as_mut().unwrap().screen = rect(0, 0, 2560, 1440);
            s
        };
        assert_eq!(w.tick(later(1300), narrowed, false, true), Some(ReopenReason::Driver));
        // A driver that can't be asked is no change.
        w.opened(reported(), later(1300));
        let silent = || DisplaySig { driver: None, ..reported() };
        assert_eq!(w.tick(later(2000), silent, false, false), None);
    }

    #[test]
    fn a_change_during_a_stroke_waits_for_pen_up() {
        let t0 = std::time::Instant::now();
        let mut w = DisplayWatch::default();
        w.opened(reported(), t0);
        let unplugged = || DisplaySig {
            desktop: rect(0, 0, 2560, 1440),
            monitors: vec![(rect(0, 0, 2560, 1440), true)],
            ..reported()
        };
        let t1 = t0 + std::time::Duration::from_secs(1);
        assert_eq!(w.tick(t1, unplugged, true, false), None);
        // Still down: held, and the layout isn't read again.
        assert_eq!(w.tick(t1, || panic!("re-read while held"), true, false), None);
        assert_eq!(w.tick(t1, || panic!("re-read while held"), false, false), Some(ReopenReason::Monitors));
    }

    #[test]
    fn failed_inits_back_off_and_a_missing_driver_waits_for_reconnect() {
        let t0 = std::time::Instant::now();
        let at = |s: f32| t0 + std::time::Duration::from_secs_f32(s);
        let mut r = Retry::default();
        assert!(r.due(t0));
        r.failed(InitFail::NoWindow, t0);
        assert!(r.due(t0), "no window yet: try again next frame");
        r.failed(InitFail::Context, t0);
        assert!(!r.due(at(0.4)));
        assert!(r.due(at(0.5)));
        r.failed(InitFail::Context, at(0.5));
        assert!(!r.due(at(1.4)));
        assert!(r.due(at(1.5)));
        for _ in 0..10 {
            r.failed(InitFail::Context, t0);
        }
        assert!(r.due(at(10.0)), "capped at ten seconds");
        r.failed(InitFail::NoDriver, t0);
        assert!(r.parked());
        assert!(!r.due(at(3600.0)));
        assert_eq!(r.wait(t0), None);
    }

    #[test]
    fn your_desktop_spans_two_monitors() {
        let s = reported();
        let area = s.driver.unwrap().screen;
        assert_eq!(monitors_spanned(area, &s.monitors), 2);
        assert_eq!(monitors_spanned(rect(0, 0, 2560, 1440), &s.monitors), 1);
        // Touching the second monitor's edge isn't reaching onto it.
        assert_eq!(monitors_spanned(rect(0, 0, 2560, 900), &s.monitors), 1);
        // Extents can run negative in Wintab's rectangles.
        assert_eq!(monitors_spanned(rect(4160, 1440, -4160, -1440), &s.monitors), 2);
    }

    /// The user's readout after mapping the tablet to the main monitor: the
    /// driver still says 4159×1439, but the cursor puts the tablet's width on
    /// 2558 pixels — one monitor.
    #[test]
    fn the_cursor_measures_what_the_pen_reaches() {
        let s = reported();
        let fit = |scale: f64, origin: f64| Some(AxisFit { origin, scale, rms: 0.3 });
        let span = (4159 * 32, 1439 * 32);
        let reach = pen_reach(span, fit(0.01922, 0.0), fit(0.03126, -1439.5)).unwrap();
        assert!((reach.0 - 2558.0).abs() < 1.0 && (reach.1 - 1439.5).abs() < 1.0, "{reach:?}");
        assert!(reach_fits_one_monitor(reach, &s.monitors));
        // Mapped over both monitors, the cursor shows it too.
        let both = pen_reach(span, fit(1.0 / 32.0, 0.0), fit(-1.0 / 32.0, 1439.0)).unwrap();
        assert!(!reach_fits_one_monitor(both, &s.monitors));
        // The small monitor alone fits; nothing is measured without a fit.
        assert!(reach_fits_one_monitor((1600.0, 900.0), &s.monitors));
        assert_eq!(pen_reach(span, fit(0.02, 0.0), None), None);
    }
}
