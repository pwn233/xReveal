//! xReveal's few requests (#145, #163), through Windows' own WinHTTP: the system and per-user
//! proxy settings and trusted roots, no extra crates. Synchronous, so they run on a background
//! thread. Redirects (GitHub's downloads move to its storage host) are followed by WinHTTP.

use std::io::Write;
use std::path::Path;

use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpAddRequestHeaders,
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR, w};

use crate::update::{Answer, LATEST_RELEASE, Progress, READ_LIMIT, parse};

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

/// An open request to `url` whose response has arrived: the handles (kept alive together) and the
/// status code.
struct Response {
    request: Handle,
    _connection: Handle,
    _session: Handle,
    status: u32,
}

fn open(url: &str, accept: &str) -> Option<Response> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}")))?;
    unsafe {
        let session = Handle(WinHttpOpen(w!("xReveal"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return None;
        }
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 30_000).ok()?;
        let connection = Handle(WinHttpConnect(session.0, &HSTRING::from(host), INTERNET_DEFAULT_HTTPS_PORT, 0));
        if connection.0.is_null() {
            return None;
        }
        let request = Handle(WinHttpOpenRequest(connection.0, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if request.0.is_null() {
            return None;
        }
        let headers: Vec<u16> = format!("Accept: {accept}\r\nAccept-Language: en\r\n").encode_utf16().collect();
        WinHttpAddRequestHeaders(request.0, &headers, WINHTTP_ADDREQ_FLAG_ADD).ok()?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).ok()?;
        let status = number(&request, WINHTTP_QUERY_STATUS_CODE)?;
        Some(Response { request, _connection: connection, _session: session, status })
    }
}

fn number(request: &Handle, query: u32) -> Option<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    unsafe { WinHttpQueryHeaders(request.0, query | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some((&raw mut value).cast()), &mut size, std::ptr::null_mut()).ok()? };
    Some(value)
}

/// Reads the body in chunks, handing each to `chunk`; false to stop early.
fn read(response: &Response, mut chunk: impl FnMut(&[u8]) -> bool) -> Option<()> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        unsafe { WinHttpReadData(response.request.0, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut read).ok()? };
        if read == 0 || !chunk(&buffer[..read as usize]) {
            return Some(());
        }
    }
}

/// A GET of `url`: its status and body (at most [`READ_LIMIT`]), or `None` when there was no answer.
pub fn get(url: &str, accept: &str) -> Option<(isize, Vec<u8>)> {
    let response = open(url, accept)?;
    let mut body = Vec::new();
    read(&response, |bytes| {
        body.extend_from_slice(bytes);
        body.len() <= READ_LIMIT
    })?;
    if body.len() > READ_LIMIT {
        body.clear();
    }
    Some((response.status as isize, body))
}

/// Asks GitHub for the latest release (blocking, at most about 10 s). `None` when there was no
/// usable answer (offline, a proxy refusing, rate-limited): not recorded, so it's tried again.
pub fn latest_release() -> Option<Answer> {
    match get(LATEST_RELEASE, "application/vnd.github+json")? {
        (200, body) => Some(parse(&body)),
        (404, _) => Some(Answer::Nothing),
        _ => None,
    }
}

/// Downloads `url` to `dest`, counting bytes into `progress`. Blocks until done, failed or
/// cancelled.
pub fn download(url: &str, dest: &Path, progress: &Progress) -> Result<(), String> {
    let response = open(url, "application/octet-stream").ok_or("couldn't reach GitHub")?;
    if response.status != 200 {
        return Err("the download wasn't found".into());
    }
    let total = number(&response.request, WINHTTP_QUERY_CONTENT_LENGTH).unwrap_or(0) as u64;
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut received = 0u64;
    let mut failed = None;
    read(&response, |bytes| {
        if let Err(e) = file.write_all(bytes) {
            failed = Some(e.to_string());
            return false;
        }
        received += bytes.len() as u64;
        progress.set(received, total);
        !progress.cancelled()
    })
    .ok_or("the download stopped")?;
    if let Some(error) = failed {
        return Err(error);
    }
    if progress.cancelled() {
        return Err("cancelled".into());
    }
    Ok(())
}
