//! Frame timing (docs/spec/v1.md §3.3, §4): per-frame timestamps on one host clock, the
//! picture-delay stage, and the video thread's loop that ties them to the frame slot.
//!
//! Every time here is in seconds on the platform's host clock (macOS `CACurrentMediaTime`,
//! Windows QPC), so capture, arrival and present times can be subtracted directly.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use super::slot::{FrameSlot, Taken};

/// When a frame was captured (the OS's timestamp, converted to host time) and when it reached
/// xReveal (the capture callback).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamps {
    pub capture: f64,
    pub arrival: f64,
}

/// Something the video thread draws that knows its own stamps.
pub trait Timed {
    fn stamps(&self) -> Stamps;
}

/// How much the picture is held back (docs/spec/v1.md §3.3): Buffering in whole frames plus a
/// negative A/V sync offset. Both zero (the default) bypasses the delay stage entirely.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct DelaySettings {
    /// Off (0), 1 or 2 frames.
    pub buffering_frames: u8,
    /// −500..=500 ms. Only a negative offset delays the picture; a positive one delays the sound.
    pub av_sync_offset_ms: i32,
}

impl DelaySettings {
    /// The picture delay in seconds, given the Source's measured frame interval.
    pub fn seconds(&self, frame_interval: f64) -> f64 {
        self.buffering_frames.min(2) as f64 * frame_interval + (-self.av_sync_offset_ms).max(0) as f64 / 1000.0
    }
}

/// Frames held back until `capture + delay`. When several are due at once, only the newest is
/// shown; the older ones count as dropped.
pub struct PictureDelay<T> {
    queue: VecDeque<T>,
}

impl<T: Timed> Default for PictureDelay<T> {
    fn default() -> Self {
        Self { queue: VecDeque::new() }
    }
}

impl<T: Timed> PictureDelay<T> {
    pub fn push(&mut self, frame: T) {
        self.queue.push_back(frame);
    }

    /// The time the oldest held frame becomes due.
    pub fn next_due(&self, delay: f64) -> Option<f64> {
        self.queue.front().map(|f| f.stamps().capture + delay)
    }

    /// The newest frame due at `now`, plus how many older due frames were skipped.
    pub fn take_due(&mut self, now: f64, delay: f64) -> (Option<T>, u64) {
        let mut newest = None;
        let mut skipped = 0;
        while self.queue.front().is_some_and(|f| f.stamps().capture + delay <= now) {
            if newest.replace(self.queue.pop_front().expect("checked")).is_some() {
                skipped += 1;
            }
        }
        (newest, skipped)
    }

    /// Everything held, oldest first (used when the delay is turned off: show the newest).
    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.queue.drain(..)
    }
}

/// A snapshot for the Stats overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    /// Capture → on screen, milliseconds (p50, p99).
    pub capture_to_screen: Option<(f64, f64)>,
    /// Arrival in xReveal → on screen, milliseconds (p50, p99): xReveal's own share.
    pub arrival_to_screen: Option<(f64, f64)>,
    /// Frames per second, from capture timestamps.
    pub fps: Option<f64>,
    pub dropped: u64,
    /// Frames shown more than one frame interval after they arrived.
    pub late: u64,
    /// Present times are when xReveal asked to present, not when the display showed it.
    pub present_time_estimated: bool,
}

const WINDOW: usize = 240;

/// Recent frames' timings.
#[derive(Default)]
pub struct FrameStats {
    samples: VecDeque<(Stamps, f64)>,
    dropped: u64,
    late: u64,
    estimated: bool,
}

impl FrameStats {
    /// Records that a frame with `stamps` was on screen at `present` (host seconds).
    /// `estimated` marks a present time taken when presenting rather than from the display.
    pub fn presented(&mut self, stamps: Stamps, present: f64, estimated: bool) {
        if present <= 0.0 {
            return; // never reached the screen (e.g. the drawable was dropped)
        }
        if let Some(interval) = self.frame_interval()
            && present - stamps.arrival > interval
        {
            self.late += 1;
        }
        self.estimated = estimated;
        if self.samples.len() == WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back((stamps, present));
    }

    pub fn dropped(&mut self, frames: u64) {
        self.dropped += frames;
    }

    /// The Source's frame interval in seconds (median of recent capture intervals).
    pub fn frame_interval(&self) -> Option<f64> {
        let mut captures: Vec<f64> = self.samples.iter().map(|(s, _)| s.capture).collect();
        captures.sort_by(f64::total_cmp);
        let mut intervals: Vec<f64> = captures.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).collect();
        percentile(&mut intervals, 0.5)
    }

    pub fn summary(&self, slot_dropped: u64) -> Summary {
        let mut capture: Vec<f64> = self.samples.iter().map(|(s, p)| (p - s.capture) * 1000.0).collect();
        let mut arrival: Vec<f64> = self.samples.iter().map(|(s, p)| (p - s.arrival) * 1000.0).collect();
        let pair = |v: &mut Vec<f64>| Some((percentile(v, 0.5)?, percentile(v, 0.99)?));
        Summary {
            capture_to_screen: pair(&mut capture),
            arrival_to_screen: pair(&mut arrival),
            fps: self.frame_interval().map(|i| 1.0 / i),
            dropped: self.dropped + slot_dropped,
            late: self.late,
            present_time_estimated: self.estimated,
        }
    }
}

fn percentile(values: &mut [f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() - 1) as f64 * p).round() as usize;
    Some(values[index])
}

/// The video thread's loop: takes frames from `slot`, passes them straight to `draw` when there is
/// no delay, otherwise holds them in a [`PictureDelay`] and draws each when it is due. `draw` is
/// responsible for reporting present times to the stats. Returns when the slot is closed.
pub fn run_video_loop<T: Timed>(
    slot: &FrameSlot<T>,
    delay: &Mutex<DelaySettings>,
    stats: &Mutex<FrameStats>,
    now: impl Fn() -> f64,
    mut draw: impl FnMut(T),
) {
    let mut held = PictureDelay::default();
    // How long to wait for a frame when nothing is held: long, but not forever, so a change of
    // delay settings is noticed even if the Source stops.
    const IDLE: Duration = Duration::from_millis(250);
    loop {
        let interval = lock(stats).frame_interval().unwrap_or(1.0 / 60.0);
        let delay_s = lock(delay).seconds(interval);

        if delay_s <= 0.0 {
            // Bypass: no queue, the newest frame goes straight to the screen.
            if let Some(newest) = held.drain().last() {
                draw(newest);
            }
            match slot.take() {
                Some(frame) => draw(frame),
                None => return,
            }
            continue;
        }

        let wait = held.next_due(delay_s).map_or(IDLE, |due| Duration::from_secs_f64((due - now()).clamp(0.0, 1.0)));
        match slot.take_timeout(wait) {
            Taken::Frame(frame) => held.push(frame),
            Taken::TimedOut => {}
            Taken::Closed => return,
        }
        let (due, skipped) = held.take_due(now(), delay_s);
        if skipped > 0 {
            lock(stats).dropped(skipped);
        }
        if let Some(frame) = due {
            draw(frame);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Instant;

    #[derive(Debug, PartialEq)]
    struct F(f64);
    impl Timed for F {
        fn stamps(&self) -> Stamps {
            Stamps { capture: self.0, arrival: self.0 }
        }
    }

    #[test]
    fn delay_settings() {
        let off = DelaySettings::default();
        assert_eq!(off.seconds(1.0 / 60.0), 0.0);
        let two_frames = DelaySettings { buffering_frames: 2, av_sync_offset_ms: 0 };
        assert!((two_frames.seconds(0.02) - 0.04).abs() < 1e-12);
        // A positive offset delays the sound, not the picture.
        assert_eq!(DelaySettings { buffering_frames: 0, av_sync_offset_ms: 120 }.seconds(0.02), 0.0);
        let negative = DelaySettings { buffering_frames: 1, av_sync_offset_ms: -30 };
        assert!((negative.seconds(0.02) - 0.05).abs() < 1e-12);
    }

    #[test]
    fn picture_delay_releases_frames_when_due_newest_first() {
        let mut d = PictureDelay::default();
        d.push(F(1.0));
        d.push(F(1.02));
        d.push(F(1.04));
        assert_eq!(d.next_due(0.05), Some(1.05));
        assert_eq!(d.take_due(1.049, 0.05), (None, 0));
        assert_eq!(d.take_due(1.075, 0.05), (Some(F(1.02)), 1), "two due: show the newer, skip the older");
        assert_eq!(d.take_due(2.0, 0.05), (Some(F(1.04)), 0));
        assert_eq!(d.next_due(0.05), None);
    }

    #[test]
    fn stats_percentiles_fps_and_late_frames() {
        let mut s = FrameStats::default();
        for i in 0..120 {
            let capture = i as f64 / 60.0;
            let latency = if i == 100 { 0.040 } else { 0.010 };
            s.presented(Stamps { capture, arrival: capture + 0.002 }, capture + latency, false);
        }
        let summary = s.summary(3);
        let (p50, p99) = summary.capture_to_screen.unwrap();
        assert!((p50 - 10.0).abs() < 1e-6, "median latency 10 ms: {p50}");
        assert!(p99 >= p50);
        assert!((summary.fps.unwrap() - 60.0).abs() < 0.01);
        let (a50, _) = summary.arrival_to_screen.unwrap();
        assert!((a50 - 8.0).abs() < 1e-6);
        assert_eq!(summary.late, 1, "the 40 ms frame was shown after the next one arrived");
        assert_eq!(summary.dropped, 3);
        assert!(!summary.present_time_estimated);
    }

    #[test]
    fn frames_that_never_reached_the_screen_are_ignored() {
        let mut s = FrameStats::default();
        s.presented(Stamps { capture: 1.0, arrival: 1.0 }, 0.0, false);
        assert_eq!(s.summary(0).capture_to_screen, None);
    }

    /// A clock the test can read from any thread.
    fn clock() -> impl Fn() -> f64 + Clone {
        let start = Instant::now();
        move || start.elapsed().as_secs_f64()
    }

    #[test]
    fn loop_bypasses_the_delay_at_zero() {
        let slot = Arc::new(FrameSlot::new());
        let delay = Mutex::new(DelaySettings::default());
        let stats = Mutex::new(FrameStats::default());
        let now = clock();
        let drawn = Arc::new(AtomicU64::new(0));
        let producer = {
            let (slot, now) = (slot.clone(), now.clone());
            thread::spawn(move || {
                for _ in 0..5 {
                    slot.put(F(now()));
                    thread::sleep(Duration::from_millis(5));
                }
                slot.close();
            })
        };
        let d = drawn.clone();
        run_video_loop(&slot, &delay, &stats, now, move |_| {
            d.fetch_add(1, Ordering::SeqCst);
        });
        producer.join().unwrap();
        assert!(drawn.load(Ordering::SeqCst) >= 4);
    }

    #[test]
    fn loop_holds_frames_back_by_the_delay() {
        let slot = Arc::new(FrameSlot::new());
        let delay = Mutex::new(DelaySettings { buffering_frames: 0, av_sync_offset_ms: -40 });
        let stats = Mutex::new(FrameStats::default());
        let now = clock();
        let lags = Arc::new(Mutex::new(Vec::new()));
        let producer = {
            let (slot, now) = (slot.clone(), now.clone());
            thread::spawn(move || {
                for _ in 0..5 {
                    slot.put(F(now()));
                    thread::sleep(Duration::from_millis(10));
                }
                thread::sleep(Duration::from_millis(80));
                slot.close();
            })
        };
        let (l, n) = (lags.clone(), now.clone());
        run_video_loop(&slot, &delay, &stats, now, move |f: F| l.lock().unwrap().push(n() - f.0));
        producer.join().unwrap();
        let mut lags = lags.lock().unwrap().clone();
        assert!(!lags.is_empty());
        // The guarantee: never shown before the delay.
        for lag in &lags {
            assert!(*lag >= 0.039, "shown {:.1} ms after capture, expected ≥ 40 ms", lag * 1000.0);
        }
        // Lateness depends on thread scheduling (busy CI machines wake threads late), so check the
        // typical frame tightly and individual frames only loosely.
        lags.sort_by(f64::total_cmp);
        let median = lags[lags.len() / 2];
        assert!(median < 0.040 + 0.030, "median frame shown {:.1} ms after capture: the delay stage is late", median * 1000.0);
        let worst = lags[lags.len() - 1];
        assert!(worst < 0.040 + 0.250, "a frame shown {:.1} ms after capture", worst * 1000.0);
    }
}
