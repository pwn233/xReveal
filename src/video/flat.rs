//! A picture that has stayed one flat colour (docs/spec/v1.md §2.8). Some cards keep sending
//! frames when their input has no signal, a plain near-black picture, so the No-signal screen
//! (no frames for a second) never comes (#108). A flat picture is never called "No signal": a
//! dark game scene looks the same. After a while it only earns a hint over the picture.

use std::time::{Duration, Instant};

/// Columns and rows of the luma samples taken from each frame.
#[cfg_attr(all(target_os = "macos", not(test)), expect(dead_code, reason = "macOS doesn't sample frames for it yet"))]
pub const GRID: (usize, usize) = (16, 9);
/// How far apart (in 8-bit luma) the samples may be and still count as one flat colour: a card's
/// blank output can carry a little noise.
#[cfg_attr(all(target_os = "macos", not(test)), expect(dead_code, reason = "macOS doesn't sample frames for it yet"))]
pub const TOLERANCE: u8 = 3;
/// How long the picture must stay flat before the hint shows.
pub const HINT_AFTER: Duration = Duration::from_secs(10);

/// Luma sampled at the middle of each cell of a [`GRID`] over a `width`×`height` frame;
/// `luma(x, y)` reads one pixel.
#[cfg_attr(all(target_os = "macos", not(test)), expect(dead_code, reason = "macOS doesn't sample frames for it yet"))]
pub fn luma_grid(width: usize, height: usize, luma: impl Fn(usize, usize) -> u8) -> Vec<u8> {
    let (columns, rows) = GRID;
    (0..rows)
        .flat_map(|row| (0..columns).map(move |column| (column, row)))
        .map(|(column, row)| luma((2 * column + 1) * width / (2 * columns), (2 * row + 1) * height / (2 * rows)))
        .collect()
}

/// Whether the samples are all one colour, within [`TOLERANCE`].
#[cfg_attr(all(target_os = "macos", not(test)), expect(dead_code, reason = "macOS doesn't sample frames for it yet"))]
pub fn is_flat(samples: &[u8]) -> bool {
    match (samples.iter().min(), samples.iter().max()) {
        (Some(low), Some(high)) => high - low <= TOLERANCE,
        _ => false,
    }
}

/// Since when the picture has been flat.
#[derive(Debug, Default)]
pub struct FlatWatch {
    since: Option<Instant>,
}

impl FlatWatch {
    /// Takes in one frame's samples, seen at `now`.
    #[cfg_attr(all(target_os = "macos", not(test)), expect(dead_code, reason = "macOS doesn't sample frames for it yet"))]
    pub fn observe(&mut self, samples: &[u8], now: Instant) {
        if is_flat(samples) {
            self.since.get_or_insert(now);
        } else {
            self.since = None;
        }
    }

    /// How long the picture has been flat at `now` (zero when it isn't).
    pub fn flat_for(&self, now: Instant) -> Duration {
        self.since.map_or(Duration::ZERO, |since| now.saturating_duration_since(since))
    }

    /// Forgets the run, for a new Source or Video format.
    #[cfg_attr(all(target_os = "macos", not(test)), expect(dead_code, reason = "macOS doesn't sample frames for it yet"))]
    pub fn reset(&mut self) {
        self.since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{FlatWatch, GRID, luma_grid};
    use std::time::{Duration, Instant};

    #[test]
    fn samples_the_middle_of_each_cell() {
        let grid = luma_grid(160, 90, |x, y| if (x, y) == (5, 5) || (x, y) == (155, 85) { 200 } else { 16 });
        assert_eq!(grid.len(), GRID.0 * GRID.1);
        assert_eq!((grid[0], grid[grid.len() - 1]), (200, 200), "the first and last cells' middles");
    }

    #[test]
    fn a_flat_picture_is_timed_and_any_detail_resets_it() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut watch = FlatWatch::default();
        // Seen on the Windows test PC: the card's blank output, near-black with a little noise.
        let blank: Vec<u8> = (0..144).map(|i| 16 + (i % 3) as u8).collect();
        watch.observe(&blank, at(0));
        watch.observe(&blank, at(12));
        assert_eq!(watch.flat_for(at(12)), Duration::from_secs(12));
        // A dark scene with anything in it: a lit pixel, a dim logo.
        let mut scene = blank.clone();
        scene[70] = 40;
        watch.observe(&scene, at(13));
        assert_eq!(watch.flat_for(at(13)), Duration::ZERO);
        watch.observe(&blank, at(14));
        assert_eq!(watch.flat_for(at(20)), Duration::from_secs(6), "a new run starts");
        watch.reset();
        assert_eq!(watch.flat_for(at(20)), Duration::ZERO);
    }
}
