//! The single slot that hands the newest captured frame to the video thread.
//!
//! The capture callback puts each frame in; the video thread takes it out. A frame that hasn't
//! been taken yet is replaced (and dropped) by the next one, so the video thread always draws the
//! newest frame and nothing ever queues up behind a slow frame (docs/spec/v1.md §3.3).

use std::sync::{Condvar, Mutex, PoisonError};

pub struct FrameSlot<T> {
    state: Mutex<State<T>>,
    wake: Condvar,
}

struct State<T> {
    frame: Option<T>,
    closed: bool,
}

impl<T> Default for FrameSlot<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State { frame: None, closed: false }),
            wake: Condvar::new(),
        }
    }
}

impl<T> FrameSlot<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts `frame` in the slot and wakes the video thread. Returns `true` if it replaced a frame
    /// that was never taken (a dropped frame). Frames put after [`close`](Self::close) are dropped.
    pub fn put(&self, frame: T) -> bool {
        let replaced = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.closed {
                return false;
            }
            state.frame.replace(frame)
        };
        self.wake.notify_one();
        // Dropped outside the lock: releasing a frame may return it to the capture pool.
        replaced.is_some()
    }

    /// Waits for the next frame. Returns `None` once the slot is closed.
    pub fn take(&self) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if state.closed {
                return None;
            }
            if let Some(frame) = state.frame.take() {
                return Some(frame);
            }
            state = self.wake.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Wakes the video thread so it can stop. Any frame still in the slot is dropped.
    pub fn close(&self) {
        let pending = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.closed = true;
            state.frame.take()
        };
        self.wake.notify_all();
        drop(pending);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn newest_frame_wins() {
        let slot = FrameSlot::new();
        assert!(!slot.put(1));
        assert!(slot.put(2));
        assert!(slot.put(3));
        assert_eq!(slot.take(), Some(3));
    }

    #[test]
    fn replaced_frames_are_dropped() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let slot = FrameSlot::new();
        slot.put(Counted(drops.clone()));
        slot.put(Counted(drops.clone()));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        drop(slot.take());
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn take_waits_for_a_frame() {
        let slot = Arc::new(FrameSlot::new());
        let producer = {
            let slot = slot.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                slot.put(7);
            })
        };
        assert_eq!(slot.take(), Some(7));
        producer.join().unwrap();
    }

    #[test]
    fn close_wakes_the_taker_and_stops_puts() {
        let slot = Arc::new(FrameSlot::<u32>::new());
        let taker = {
            let slot = slot.clone();
            thread::spawn(move || slot.take())
        };
        thread::sleep(Duration::from_millis(20));
        slot.close();
        assert_eq!(taker.join().unwrap(), None);
        assert!(!slot.put(1));
        assert_eq!(slot.take(), None);
    }
}
