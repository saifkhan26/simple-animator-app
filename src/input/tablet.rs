//! Pen pressure input.
//!
//! Strategy: egui drives pointer XY (it already accepts pen events as mouse
//! on every desktop OS we care about). We use the platform tablet API only to
//! read the *latest* normalised pressure value, then inject it into the
//! PointerSample seen by the brush engine.
//!
//! Windows: Wintab via `wintab_lite`, dynamic-loaded so the app still runs
//! without a tablet driver. Linux/macOS: not yet implemented — `current()`
//! returns `None` and the brush falls back to pressure = 1.0 from the mouse
//! synth.

pub struct PenInput {
    #[cfg(target_os = "windows")]
    backend: Option<windows_backend::Wintab>,
    #[cfg(not(target_os = "windows"))]
    backend: (),
}

impl PenInput {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "windows")]
            backend: None,
            #[cfg(not(target_os = "windows"))]
            backend: (),
        }
    }

    /// Called once per frame. Initialises the backend lazily (once the window
    /// exists) and drains any pending packets.
    pub fn poll(&mut self) {
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
                w.poll();
            }
        }
    }

    /// Returns latest reported pen pressure (0..=1) if available.
    pub fn current_pressure(&self) -> Option<f32> {
        #[cfg(target_os = "windows")]
        {
            return self.backend.as_ref().map(|w| w.last_pressure);
        }
        #[cfg(not(target_os = "windows"))]
        {
            return None;
        }
    }

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

    use anyhow::{anyhow, Context, Result};
    use libloading::Library;
    use windows::core::PCSTR;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowA;
    use wintab_lite::{
        cast_void, Packet, WTClose, WTDataGet, WTInfo, WTOpen, WTQueuePacketsEx, AXIS, CXO, DVC,
        LOGCONTEXT, WTI, WTPKT,
    };

    pub struct Wintab {
        wt_close: WTClose<'static>,
        wt_queue: WTQueuePacketsEx<'static>,
        wt_data_get: WTDataGet<'static>,
        ctx_handle: *mut wintab_lite::HCTX,
        pressure_max: f32,
        pub last_pressure: f32,
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

            log_context.lcName.write_str("animator-app");
            log_context.lcOptions |= CXO::SYSTEM;
            log_context.lcPktData = WTPKT::all();
            log_context.lcPktMode = WTPKT::empty();
            log_context.lcMoveMask = WTPKT::X | WTPKT::Y | WTPKT::NORMAL_PRESSURE;

            let mut pressure_axis = AXIS::default();
            let pr = unsafe {
                wt_info(WTI::DEVICES, DVC::NPRESSURE as u32, cast_void!(pressure_axis))
            };
            let pressure_max = if pr as usize == std::mem::size_of::<AXIS>() {
                pressure_axis.axMax as f32
            } else {
                1023.0
            };
            if pressure_max < 1.0 {
                return Err(anyhow!("invalid pressure_max: {pressure_max}"));
            }

            let ctx_handle = unsafe { wt_open(hwnd, &mut log_context, 1) };
            if ctx_handle.is_null() {
                return Err(anyhow!("WTOpen returned null"));
            }

            log::info!("Wintab opened: pressure_max = {pressure_max}");

            Ok(Self {
                wt_close,
                wt_queue,
                wt_data_get,
                ctx_handle,
                pressure_max,
                last_pressure: 1.0,
            })
        }

        pub fn poll(&mut self) {
            let mut from = 0u32;
            let mut to = 0u32;
            let any = unsafe { (self.wt_queue)(self.ctx_handle, &mut from, &mut to) };
            if any == 0 {
                return;
            }
            const MAX: usize = 64;
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
            let removed = removed as usize;
            if removed == 0 {
                return;
            }
            // Use the most recent packet's pressure as the "current" value.
            let last = &packets[removed - 1];
            self.last_pressure =
                (last.pkNormalPressure as f32 / self.pressure_max).clamp(0.0, 1.0);
        }
    }

    impl Drop for Wintab {
        fn drop(&mut self) {
            let _ = unsafe { (self.wt_close)(self.ctx_handle) };
        }
    }
}
