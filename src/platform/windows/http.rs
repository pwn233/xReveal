//! The Update available check's one request (#145), through Windows' own WinHTTP: the system and
//! per-user proxy settings and trusted roots, no extra crates. Synchronous, so it runs on a
//! background thread.

use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::update::{Answer, LATEST_RELEASE, READ_LIMIT, parse};

/// Closes a WinHTTP handle when dropped.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// Asks GitHub for the latest release (blocking, at most about 10 s). `None` when there was no
/// usable answer (offline, a proxy refusing, rate-limited): not recorded, so it's tried again.
pub fn latest_release() -> Option<Answer> {
    unsafe {
        let session = Handle(WinHttpOpen(w!("xReveal"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return None;
        }
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 10_000).ok()?;
        let connection = Handle(WinHttpConnect(session.0, w!("api.github.com"), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if connection.0.is_null() {
            return None;
        }
        // The same address the macOS side asks, split into host and path for WinHTTP.
        let path = HSTRING::from(LATEST_RELEASE.strip_prefix("https://api.github.com").unwrap_or(LATEST_RELEASE));
        let request = Handle(WinHttpOpenRequest(connection.0, w!("GET"), &path, PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if request.0.is_null() {
            return None;
        }
        let headers: Vec<u16> = "Accept: application/vnd.github+json\r\nAccept-Language: en\r\n".encode_utf16().collect();
        WinHttpAddRequestHeaders(request.0, &headers, WINHTTP_ADDREQ_FLAG_ADD).ok()?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).ok()?;
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some((&raw mut status).cast()), &mut size, std::ptr::null_mut()).ok()?;
        match status {
            200 => {}
            404 => return Some(Answer::Nothing),
            _ => return None,
        }
        let mut body = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, chunk.as_mut_ptr().cast(), chunk.len() as u32, &mut read).ok()?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > READ_LIMIT {
                return Some(Answer::Nothing);
            }
        }
        Some(parse(&body))
    }
}
