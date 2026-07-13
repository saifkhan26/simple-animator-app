//! Live screen sampling for the screen colour picker.
//!
//! Captures a small square region of the desktop around the OS cursor with GDI
//! `BitBlt` + `CAPTUREBLT` (the conventional reliable screen-grab). `CAPTUREBLT`
//! copies the *composited* on-screen image, so when our window's backdrop is
//! transparent this reads the app visible behind us — and the captured region
//! doubles as the zoom-loupe preview. Position is `GetCursorPos` (global,
//! physical pixels), so there is no window-client / DPI maths to get wrong.
//!
//! Windows-only (GDI). Other platforms return `None`, matching the pen backend.

/// A small RGBA snapshot of the screen around the cursor. `buf` is row-major
/// RGBA8, `w * h * 4` bytes, with the cursor pixel at the centre.
#[cfg(target_os = "windows")]
pub struct ScreenRegion {
    pub w: u32,
    pub h: u32,
    pub buf: Vec<u8>,
}

#[cfg(target_os = "windows")]
impl ScreenRegion {
    /// Colour of the centre pixel (the one under the cursor).
    pub fn center(&self) -> [u8; 4] {
        let cx = self.w / 2;
        let cy = self.h / 2;
        let i = ((cy * self.w + cx) * 4) as usize;
        [self.buf[i], self.buf[i + 1], self.buf[i + 2], 255]
    }
}

/// Current cursor position in physical screen pixels, or `None` off-Windows.
#[cfg(target_os = "windows")]
pub fn cursor_pos() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x, p.y))
}

/// Capture a `(2*half+1)` square of the desktop centred on `(cx, cy)`.
#[cfg(target_os = "windows")]
pub fn capture_region(cx: i32, cy: i32, half: i32) -> Option<ScreenRegion> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, CAPTUREBLT, DIB_RGB_COLORS, HGDIOBJ, ROP_CODE, SRCCOPY,
    };
    use windows::Win32::Graphics::Gdi::GetDC;

    let side = half * 2 + 1;
    if side <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(HWND::default());
        if screen.0 == 0 {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        if mem.0 == 0 {
            ReleaseDC(HWND::default(), screen);
            return None;
        }
        let bmp = CreateCompatibleBitmap(screen, side, side);
        if bmp.0 == 0 {
            let _ = DeleteDC(mem);
            ReleaseDC(HWND::default(), screen);
            return None;
        }
        let old = SelectObject(mem, HGDIOBJ(bmp.0));

        // CAPTUREBLT → include layered/transparent windows, i.e. the real
        // composited screen behind our transparent backdrop.
        let blt = BitBlt(
            mem,
            0,
            0,
            side,
            side,
            screen,
            cx - half,
            cy - half,
            ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0),
        );

        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<windows::Win32::Graphics::Gdi::BITMAPINFOHEADER>()
            as u32;
        bi.bmiHeader.biWidth = side;
        bi.bmiHeader.biHeight = -side; // negative → top-down rows
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = 0; // BI_RGB

        let mut buf = vec![0u8; (side * side * 4) as usize];
        let got = GetDIBits(
            mem,
            bmp,
            0,
            side as u32,
            Some(buf.as_mut_ptr().cast()),
            &mut bi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(HWND::default(), screen);

        if blt.is_err() || got == 0 {
            return None;
        }
        // 32bpp BI_RGB DIB is BGRX — swap to RGBA and force opaque.
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        Some(ScreenRegion {
            w: side as u32,
            h: side as u32,
            buf,
        })
    }
}

#[cfg(not(target_os = "windows"))]
pub struct ScreenRegion {
    pub w: u32,
    pub h: u32,
    pub buf: Vec<u8>,
}

#[cfg(not(target_os = "windows"))]
impl ScreenRegion {
    pub fn center(&self) -> [u8; 4] {
        [0, 0, 0, 255]
    }
}

#[cfg(not(target_os = "windows"))]
pub fn cursor_pos() -> Option<(i32, i32)> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn capture_region(_cx: i32, _cy: i32, _half: i32) -> Option<ScreenRegion> {
    None
}
