//! Smooth window moves between monitors with different display scaling.
//!
//! When a window crosses to a monitor with another scale, Windows sends
//! `WM_DPICHANGED` with the rectangle the window should take: scaled around the
//! cursor, so the window stays under the pointer and on the new monitor. The
//! windowing library computes its own rectangle instead and nudges the window
//! until it sits on the new monitor, which makes the window jump while it is
//! being dragged onto a screen such as a pen display. This module lets the
//! library process the message (so the interface scale updates) and then
//! places the window where Windows asked.

#[cfg(windows)]
mod imp {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER, WM_DPICHANGED,
    };

    const SUBCLASS_ID: usize = 0x5348_4152; // "SHAR"

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if message == WM_DPICHANGED && lparam != 0 {
            // SAFETY: for WM_DPICHANGED, lparam points to the suggested RECT.
            let suggested = unsafe { *(lparam as *const RECT) };
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            return result;
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    /// Hooks the application window. Returns whether the hook is in place.
    pub fn install(frame: &eframe::Frame) -> bool {
        let Ok(handle) = frame.window_handle() else { return false };
        let RawWindowHandle::Win32(window) = handle.as_raw() else { return false };
        let hwnd = window.hwnd.get() as HWND;
        // SAFETY: called on the thread that owns the window, with a procedure
        // that forwards every message to the previous one.
        unsafe { SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, 0) != 0 }
    }
}

#[cfg(windows)]
pub use imp::install;

/// Other systems need no fix.
#[cfg(not(windows))]
pub fn install(_frame: &eframe::Frame) -> bool {
    true
}
