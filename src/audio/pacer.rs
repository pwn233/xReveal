//! Keeps the ring at its target fill (docs/spec/v1.md §3.6): silence until it has filled, then a
//! slow PI controller nudges the resampler's ratio so clock drift between the input and output
//! devices never drains or floods the ring. A positive A/V sync offset just raises the target.

/// How far the controller may move the ratio: ±1% covers any real clock drift.
const MAX_ADJUST: f64 = 0.01;
/// Proportional and integral gains, per millisecond of fill error.
const KP: f64 = 0.0005;
const KI: f64 = 0.0001;
/// How much of each new fill reading the smoothed fill takes (callbacks arrive with jitter).
const SMOOTHING: f64 = 0.05;
/// More than this far above the target (after a stall), the excess is skipped at once.
const SNAP_MS: f64 = 80.0;

/// What the output callback should do this time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// Not enough waiting yet: play silence.
    Silence,
    /// Play, resampling at the base ratio times this.
    Play { relative_ratio: f64 },
    /// Throw away this many frames first, then play at the base ratio.
    Skip(usize),
}

pub struct Pacer {
    /// Frames per millisecond at the input's rate.
    frames_per_ms: f64,
    target_ms: f64,
    smoothed_ms: f64,
    integral: f64,
    primed: bool,
}

impl Pacer {
    pub fn new(input_rate: u32, target_ms: f64) -> Self {
        Self { frames_per_ms: input_rate as f64 / 1000.0, target_ms, smoothed_ms: 0.0, integral: 0.0, primed: false }
    }

    /// The fill to hold, in milliseconds: the base cushion plus any positive A/V sync offset.
    pub fn set_target_ms(&mut self, target_ms: f64) {
        self.target_ms = target_ms;
    }

    /// The ring ran dry: start over with silence until it fills again.
    pub fn underrun(&mut self) {
        self.primed = false;
    }

    /// One output callback: `fill` frames waiting, `dt` seconds since the last call.
    pub fn step(&mut self, fill: usize, dt: f64) -> Step {
        let fill_ms = fill as f64 / self.frames_per_ms;
        if !self.primed {
            if fill_ms < self.target_ms {
                return Step::Silence;
            }
            self.primed = true;
            self.smoothed_ms = self.target_ms;
            self.integral = 0.0;
        }
        if fill_ms > self.target_ms + SNAP_MS {
            self.smoothed_ms = self.target_ms;
            self.integral = 0.0;
            return Step::Skip(((fill_ms - self.target_ms) * self.frames_per_ms) as usize);
        }
        self.smoothed_ms += SMOOTHING * (fill_ms - self.smoothed_ms);
        let error = self.smoothed_ms - self.target_ms;
        self.integral = (self.integral + error * dt).clamp(-MAX_ADJUST / KI, MAX_ADJUST / KI);
        // Too full: take more input per output frame, i.e. a lower output/input ratio.
        let adjust = (KP * error + KI * self.integral).clamp(-MAX_ADJUST, MAX_ADJUST);
        Step::Play { relative_ratio: 1.0 - adjust }
    }
}

#[cfg(test)]
mod tests {
    use super::{Pacer, Step};

    /// Runs a ring between an input `drift` faster than nominal and an output that consumes
    /// according to the pacer, for `seconds`. Returns the fill the pacer last saw, in ms, and the
    /// last ratio.
    fn simulate(drift: f64, seconds: f64, target_ms: f64) -> (f64, f64) {
        let rate = 48_000.0;
        let dt = 128.0 / rate;
        let mut pacer = Pacer::new(48_000, target_ms);
        let mut fill = 0.0f64;
        let mut ratio = 1.0;
        let mut seen = 0.0;
        for _ in 0..(seconds / dt) as usize {
            fill += rate * (1.0 + drift) * dt;
            seen = fill;
            match pacer.step(fill as usize, dt) {
                Step::Silence => {}
                Step::Skip(n) => fill -= n as f64,
                Step::Play { relative_ratio } => {
                    ratio = relative_ratio;
                    fill -= rate * dt / relative_ratio;
                }
            }
            assert!(fill > 0.0, "the ring never runs dry");
        }
        (seen / 48.0, ratio)
    }

    #[test]
    fn holds_the_target_against_drift_either_way() {
        for drift in [0.0005, -0.0005, 0.002, -0.002] {
            let (fill_ms, ratio) = simulate(drift, 120.0, 8.0);
            assert!((fill_ms - 8.0).abs() < 1.0, "drift {drift}: fill {fill_ms:.2} ms");
            assert!((ratio - 1.0 / (1.0 + drift)).abs() < 0.0003, "drift {drift}: ratio {ratio}");
        }
    }

    #[test]
    fn silent_until_primed_and_again_after_an_underrun() {
        let mut pacer = Pacer::new(48_000, 10.0);
        assert_eq!(pacer.step(100, 0.003), Step::Silence);
        assert!(matches!(pacer.step(480, 0.003), Step::Play { .. }));
        pacer.underrun();
        assert_eq!(pacer.step(200, 0.003), Step::Silence);
    }

    #[test]
    fn a_long_stall_is_skipped_back_to_the_target() {
        let mut pacer = Pacer::new(48_000, 10.0);
        pacer.step(480, 0.003);
        assert_eq!(pacer.step(48_000, 0.003), Step::Skip(48_000 - 480), "a second piled up");
    }

    #[test]
    fn a_positive_av_offset_raises_the_fill_it_holds() {
        let (fill_ms, _) = simulate(0.0, 30.0, 8.0 + 120.0);
        assert!((fill_ms - 128.0).abs() < 1.5, "{fill_ms}");
    }
}
