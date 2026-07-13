//! Windows-specific window chrome tweaks.
//!
//! The app runs frameless (no caption buttons / title). On Windows 11 we re-add
//! the *default* rounded corners and window border via DWM so a borderless
//! window still looks like a normal rounded, bordered window.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DWM_WINDOW_CORNER_PREFERENCE,
};

/// `DWMWA_COLOR_DEFAULT` — let the system pick the default border colour.
const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

/// Apply default rounded corners + a default border to the given window.
/// Safe to call repeatedly; failures (e.g. pre-Win11) are ignored.
pub fn round_window(hwnd: isize) {
    let hwnd = HWND(hwnd);
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const DWM_WINDOW_CORNER_PREFERENCE as *const core::ffi::c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
        let color = DWMWA_COLOR_DEFAULT;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &color as *const u32 as *const core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}
