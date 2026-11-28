//! The video pass's own GPU time (#218): what a Metal System Trace shows for xReveal, read from each
//! frame's command buffer once it has completed. macOS's per-process GPU counter (the IORegistry's
//! `accumulatedGPUTime`) reads about 1.5× that (#200), so the verification card uses this instead.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The share is taken over this much wall time, the samplers' interval (#191).
pub const WINDOW: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct GpuTime {
    /// When the first frame's time came in: there's no share until a whole window has passed.
    first: Option<Instant>,
    /// Each completed frame's GPU time, newest last, as far back as the window.
    frames: VecDeque<(Instant, Duration)>,
}

impl GpuTime {
    /// One frame's command buffer completed at `now` after `busy` on the GPU.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code, reason = "only macOS's render loop times its frames (#218)"))]
    pub fn add(&mut self, now: Instant, busy: Duration) {
        self.first.get_or_insert(now);
        self.frames.push_back((now, busy));
        while self.frames.front().is_some_and(|(at, _)| now.duration_since(*at) > WINDOW) {
            self.frames.pop_front();
        }
    }

    /// The GPU's busy share over the last [`WINDOW`], in percent. `None` until a whole window has
    /// passed since the first frame.
    pub fn share(&self, now: Instant) -> Option<f64> {
        if now.duration_since(self.first?) < WINDOW {
            return None;
        }
        let busy: Duration = self.frames.iter().filter(|(at, _)| now.duration_since(*at) <= WINDOW).map(|(_, busy)| *busy).sum();
        Some(busy.as_secs_f64() / WINDOW.as_secs_f64() * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(20);

    /// 50 frames a second for `seconds`, each `busy` on the GPU.
    fn run(time: &mut GpuTime, start: Instant, seconds: u32, busy: Duration) -> Instant {
        let mut now = start;
        for _ in 0..seconds * 50 {
            now += FRAME;
            time.add(now, busy);
        }
        now
    }

    #[test]
    fn nothing_until_a_whole_window_has_passed() {
        let mut time = GpuTime::default();
        let start = Instant::now();
        assert_eq!(time.share(start), None);
        let now = run(&mut time, start, 4, Duration::from_millis(1));
        assert_eq!(time.share(now), None);
        let now = run(&mut time, now, 2, Duration::from_millis(1));
        assert!(time.share(now).is_some());
    }

    #[test]
    fn share_of_wall_time_over_the_last_window() {
        let mut time = GpuTime::default();
        let start = Instant::now();
        // 1 ms of every 20 ms frame is 5%.
        let now = run(&mut time, start, 10, Duration::from_millis(1));
        let share = time.share(now).unwrap();
        assert!((share - 5.0).abs() < 0.05, "{share}");
        // Heavier frames replace the lighter ones once a whole window has gone by.
        let now = run(&mut time, now, 5, Duration::from_millis(2));
        let share = time.share(now).unwrap();
        assert!((share - 10.0).abs() < 0.05, "{share}");
    }

    #[test]
    fn no_frames_lately_is_zero() {
        let mut time = GpuTime::default();
        let start = Instant::now();
        let now = run(&mut time, start, 6, Duration::from_millis(1));
        assert_eq!(time.share(now + WINDOW + FRAME), Some(0.0));
    }
}
