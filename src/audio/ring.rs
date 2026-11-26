//! The lock-free ring between the input and output audio callbacks (docs/spec/v1.md §3.6): one
//! writer (the input callback), one reader (the output callback), never a lock or an allocation.

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Shared {
    buffer: Box<[UnsafeCell<f32>]>,
    /// Total samples ever written / read; their difference is the fill.
    written: AtomicUsize,
    read: AtomicUsize,
}

// SAFETY: the writer only touches slots in [read + capacity - fill .. ), the reader only slots in
// [read .. written); the two ranges never overlap, and the counters are published with
// release/acquire ordering, so each slot has one owner at a time.
unsafe impl Sync for Shared {}

/// The input callback's end.
pub struct Writer(Arc<Shared>);
/// The output callback's end.
pub struct Reader(Arc<Shared>);

/// A ring holding `capacity` samples.
pub fn ring(capacity: usize) -> (Writer, Reader) {
    let buffer = (0..capacity.max(1)).map(|_| UnsafeCell::new(0.0)).collect();
    let shared = Arc::new(Shared { buffer, written: AtomicUsize::new(0), read: AtomicUsize::new(0) });
    (Writer(shared.clone()), Reader(shared))
}

impl Shared {
    fn fill(&self) -> usize {
        self.written.load(Ordering::Acquire).wrapping_sub(self.read.load(Ordering::Acquire))
    }
}

impl Writer {
    /// Appends as much of `samples` as fits; returns how many went in. The rest is dropped: a
    /// full ring means the output fell far behind, and the newest sound matters least to keep.
    pub fn push(&mut self, samples: &[f32]) -> usize {
        let s = &*self.0;
        let capacity = s.buffer.len();
        let written = s.written.load(Ordering::Relaxed);
        let free = capacity - written.wrapping_sub(s.read.load(Ordering::Acquire));
        let n = samples.len().min(free);
        for (i, &sample) in samples[..n].iter().enumerate() {
            // SAFETY: this slot is free (see `Shared`), and only the writer writes.
            unsafe { *s.buffer[(written + i) % capacity].get() = sample };
        }
        s.written.store(written.wrapping_add(n), Ordering::Release);
        n
    }
}

impl Reader {
    /// Samples waiting to be read.
    pub fn len(&self) -> usize {
        self.0.fill()
    }

    /// Fills `out` from the ring if it holds that many samples; otherwise leaves both alone.
    pub fn pop_exact(&mut self, out: &mut [f32]) -> bool {
        let s = &*self.0;
        let capacity = s.buffer.len();
        let read = s.read.load(Ordering::Relaxed);
        if s.written.load(Ordering::Acquire).wrapping_sub(read) < out.len() {
            return false;
        }
        for (i, slot) in out.iter_mut().enumerate() {
            // SAFETY: this slot holds a written sample (see `Shared`), and only the reader reads.
            *slot = unsafe { *s.buffer[(read + i) % capacity].get() };
        }
        s.read.store(read.wrapping_add(out.len()), Ordering::Release);
        true
    }

    /// Throws away up to `n` of the oldest samples (to cut the delay back after a stall).
    pub fn skip(&mut self, n: usize) {
        let s = &*self.0;
        let n = n.min(s.fill());
        s.read.store(s.read.load(Ordering::Relaxed).wrapping_add(n), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::ring;

    #[test]
    fn samples_come_out_in_order_and_wrap_around() {
        let (mut w, mut r) = ring(8);
        assert_eq!(w.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]), 6);
        let mut out = [0.0; 4];
        assert!(r.pop_exact(&mut out));
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(w.push(&[7.0, 8.0, 9.0, 10.0, 11.0]), 5, "wraps past the end");
        assert_eq!(r.len(), 7);
        let mut out = [0.0; 7];
        assert!(r.pop_exact(&mut out));
        assert_eq!(out, [5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0]);
    }

    #[test]
    fn a_full_ring_drops_the_rest_and_a_short_one_gives_nothing() {
        let (mut w, mut r) = ring(4);
        assert_eq!(w.push(&[1.0, 2.0, 3.0, 4.0, 5.0]), 4);
        let mut out = [0.0; 5];
        assert!(!r.pop_exact(&mut out), "only four waiting");
        r.skip(3);
        let mut one = [0.0; 1];
        assert!(r.pop_exact(&mut one));
        assert_eq!(one, [4.0]);
    }

    #[test]
    fn writer_and_reader_on_two_threads_keep_every_sample_in_order() {
        let (mut w, mut r) = ring(64);
        let writer = std::thread::spawn(move || {
            let mut next = 0.0f32;
            while next < 10_000.0 {
                let chunk: Vec<f32> = (0..7).map(|i| next + i as f32).collect();
                let n = w.push(&chunk);
                next += n as f32;
            }
        });
        let mut expected = 0.0f32;
        let mut buf = [0.0f32; 5];
        while expected < 9_995.0 {
            if r.pop_exact(&mut buf) {
                for v in buf {
                    assert_eq!(v, expected);
                    expected += 1.0;
                }
            }
        }
        writer.join().unwrap();
    }
}
