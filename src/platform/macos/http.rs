//! xReveal's few requests (#145, #163), through macOS's own `NSURLSession`: the system proxy and
//! trusted roots, no extra crates. An ephemeral session keeps no cookies or cache.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::{NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL, NSURLResponse, NSURLSession, NSURLSessionConfiguration};

use crate::update::{Answer, LATEST_RELEASE, Progress, READ_LIMIT, parse};

const TIMEOUT: Duration = Duration::from_secs(10);

fn session() -> Retained<NSURLSession> {
    let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
    config.setTimeoutIntervalForRequest(TIMEOUT.as_secs_f64());
    NSURLSession::sessionWithConfiguration(&config)
}

fn request(url: &str, accept: &str) -> Option<Retained<NSMutableURLRequest>> {
    let url = NSURL::URLWithString(&NSString::from_str(url))?;
    let request = NSMutableURLRequest::requestWithURL(&url);
    for (field, value) in [("User-Agent", "xReveal"), ("Accept", accept), ("Accept-Language", "en")] {
        request.setValue_forHTTPHeaderField(Some(&NSString::from_str(value)), &NSString::from_str(field));
    }
    Some(request)
}

fn status(response: *mut NSURLResponse) -> Option<isize> {
    // SAFETY: NSURLSession passes a valid (or nil) response, alive for the block's call.
    let response = unsafe { response.as_ref() }?;
    Some(response.downcast_ref::<NSHTTPURLResponse>()?.statusCode())
}

/// A GET of `url`: its status and body (at most [`READ_LIMIT`]), or `None` when there was no answer.
pub fn get(url: &str, accept: &str) -> Option<(isize, Vec<u8>)> {
    let request = request(url, accept)?;
    let session = session();
    let (tx, rx) = mpsc::channel::<Option<(isize, Vec<u8>)>>();
    let done = RcBlock::new(move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
        // SAFETY: as in `status`.
        let data = unsafe { data.as_ref() };
        let answer = (error.is_null()).then(|| status(response)).flatten().map(|code| (code, data.filter(|d| d.length() <= READ_LIMIT).map(|d| d.to_vec()).unwrap_or_default()));
        let _ = tx.send(answer);
    });
    // SAFETY: the block only sends on a channel, which may happen on any thread.
    let task = unsafe { session.dataTaskWithRequest_completionHandler(&request, &done) };
    task.resume();
    let answer = rx.recv_timeout(TIMEOUT + Duration::from_secs(5)).ok().flatten();
    session.finishTasksAndInvalidate();
    answer
}

/// Asks GitHub for the latest release. `None` when there was no usable answer (offline, a proxy
/// refusing, rate-limited): not recorded, so it's tried again later.
pub fn latest_release() -> Option<Answer> {
    match get(LATEST_RELEASE, "application/vnd.github+json")? {
        (200, body) => Some(parse(&body)),
        (404, _) => Some(Answer::Nothing),
        _ => None,
    }
}

/// Downloads `url` to `dest`, counting bytes into `progress` as they arrive. Blocks the calling
/// (background) thread until it's done, failed, or `progress` is cancelled.
pub fn download(url: &str, dest: &Path, progress: &Progress) -> Result<(), String> {
    let request = request(url, "application/octet-stream").ok_or("bad download address")?;
    let session = session();
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let dest = dest.to_owned();
    let done = RcBlock::new(move |file: *mut NSURL, response: *mut NSURLResponse, error: *mut NSError| {
        let result = if !error.is_null() {
            Err("the download failed".to_owned())
        } else if status(response) != Some(200) {
            Err("the download wasn't found".to_owned())
        } else {
            // The file only lives until this block returns: move it now.
            // SAFETY: as in `status`.
            let from = unsafe { file.as_ref() }.and_then(|f| f.path()).map(|p| p.to_string());
            match from {
                Some(from) => std::fs::rename(&from, &dest).or_else(|_| std::fs::copy(&from, &dest).map(|_| ())).map_err(|e| e.to_string()),
                None => Err("the download went missing".to_owned()),
            }
        };
        let _ = tx.send(result);
    });
    // SAFETY: as in `get`.
    let task = unsafe { session.downloadTaskWithRequest_completionHandler(&request, &done) };
    task.resume();
    let result = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(result) => break result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                progress.set(task.countOfBytesReceived().max(0) as u64, task.countOfBytesExpectedToReceive().max(0) as u64);
                if progress.cancelled() {
                    task.cancel();
                    break Err("cancelled".to_owned());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err("the download stopped".to_owned()),
        }
    };
    session.finishTasksAndInvalidate();
    result
}
