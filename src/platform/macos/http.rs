//! The Update available check's one request (#145), through macOS's own `NSURLSession`: the system
//! proxy and trusted roots, no extra crates. An ephemeral session keeps no cookies or cache.

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::{NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSString, NSURL, NSURLResponse, NSURLSession, NSURLSessionConfiguration};

use crate::update::{Answer, LATEST_RELEASE, READ_LIMIT, parse};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Asks GitHub for the latest release, blocking the calling (background) thread for at most a few
/// seconds past the timeout. `None` when there was no usable answer (offline, a proxy refusing,
/// rate-limited): not recorded, so it's tried again later.
pub fn latest_release() -> Option<Answer> {
    let url = NSURL::URLWithString(&NSString::from_str(LATEST_RELEASE))?;
    let request = NSMutableURLRequest::requestWithURL(&url);
    for (field, value) in [("User-Agent", "xReveal"), ("Accept", "application/vnd.github+json"), ("Accept-Language", "en")] {
        request.setValue_forHTTPHeaderField(Some(&NSString::from_str(value)), &NSString::from_str(field));
    }
    let config = NSURLSessionConfiguration::ephemeralSessionConfiguration();
    config.setTimeoutIntervalForRequest(TIMEOUT.as_secs_f64());
    let session = NSURLSession::sessionWithConfiguration(&config);

    let (tx, rx) = mpsc::channel::<Option<Answer>>();
    let done = RcBlock::new(move |data: *mut NSData, response: *mut NSURLResponse, error: *mut NSError| {
        // SAFETY: NSURLSession passes valid (or nil) objects, alive for the block's call.
        let (data, response) = unsafe { (data.as_ref(), response.as_ref()) };
        let answer = if !error.is_null() {
            None
        } else {
            let status = response.and_then(|r| r.downcast_ref::<NSHTTPURLResponse>()).map(|r| r.statusCode());
            match status {
                Some(200) => Some(data.filter(|d| d.length() <= READ_LIMIT).map_or(Answer::Nothing, |d| parse(&d.to_vec()))),
                Some(404) => Some(Answer::Nothing),
                _ => None,
            }
        };
        let _ = tx.send(answer);
    });
    // SAFETY: the block only sends on a channel, which may happen on any thread.
    let task = unsafe { session.dataTaskWithRequest_completionHandler(&request, &done) };
    task.resume();
    let answer = rx.recv_timeout(TIMEOUT + Duration::from_secs(5)).ok().flatten();
    session.finishTasksAndInvalidate();
    let _: Retained<_> = task;
    answer
}
