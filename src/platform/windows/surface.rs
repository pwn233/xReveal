//! xReveal's own video surface on Windows: a child window over GPUI's window that owns a swap chain
//! (ADR 0001, docs/spec/v1.md §3.4). It lets the mouse through, so GPUI keeps all input.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, BLACK_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Result, w};

const CLASS: windows::core::PCWSTR = w!("xRevealVideoSurface");

/// The child window's handle, moved to the video thread.
#[derive(Clone, Copy)]
pub struct Surface {
    pub parent: HWND,
    pub child: HWND,
}

// SAFETY: window handles are plain identifiers; the video thread only reads the parent's client size
// and repositions the child asynchronously (SWP_ASYNCWINDOWPOS), never blocking on the UI thread.
unsafe impl Send for Surface {}

/// Creates the child window covering `parent`'s client area. Must run on the parent's (UI) thread.
pub fn attach_over(parent: HWND) -> Result<Surface> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(surface_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: CLASS,
            ..Default::default()
        };
        // Registering twice fails harmlessly; the class is the same.
        RegisterClassExW(&class);

        let mut client = RECT::default();
        GetClientRect(parent, &mut client)?;
        let child = CreateWindowExW(
            // The swap chain is the only content: no redirection bitmap (better flip-model presents).
            WS_EX_NOREDIRECTIONBITMAP,
            CLASS,
            w!("xReveal video"),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            0,
            0,
            client.right - client.left,
            client.bottom - client.top,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )?;
        Ok(Surface { parent, child })
    }
}

impl Surface {
    /// The parent's client size in pixels.
    pub fn parent_size(&self) -> (u32, u32) {
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.parent, &mut client);
        }
        ((client.right - client.left).max(0) as u32, (client.bottom - client.top).max(0) as u32)
    }

    /// Resizes the child to `size` without waiting for the UI thread.
    pub fn resize(&self, (width, height): (u32, u32)) {
        unsafe {
            let _ = SetWindowPos(
                self.child,
                None,
                0,
                0,
                width as i32,
                height as i32,
                SWP_ASYNCWINDOWPOS | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE,
            );
        }
    }
}

unsafe extern "system" fn surface_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // Let clicks fall through to GPUI's window underneath.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
