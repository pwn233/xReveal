//! Live signal states (docs/spec/v1.md §2.8): what xReveal can know for certain about the Source.
//! It never judges the picture's content: a flat or black frame is a frame. (A picture that stays
//! flat only earns a hint over it: see [`super::flat`].)

use std::time::Duration;

/// No frame for this long shows the No-signal screen.
pub const NO_SIGNAL_AFTER: Duration = Duration::from_secs(1);
/// How often a busy card is tried again.
pub const BUSY_RETRY: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(clippy::enum_variant_names, reason = "the spec's own name: the No-signal screen")]
pub enum Signal {
    /// Just opened; the first frame hasn't had time to arrive.
    Starting,
    Live,
    /// Open, but no frame for [`NO_SIGNAL_AFTER`].
    NoSignal,
    /// The card is gone. The picture resumes when the same Source returns.
    Unplugged,
    /// The card refused to open because another app has it.
    Busy,
}

/// What the app can see of the Source it wants to show.
#[derive(Clone, Copy, Debug, Default)]
pub struct Observed {
    pub connected: bool,
    pub busy: bool,
    /// Since the capture last started.
    pub since_start: Duration,
    /// Since the last frame of this capture, if one arrived.
    pub since_frame: Option<Duration>,
}

/// Why a card didn't open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Another app has it (docs/spec/v1.md §2.8 Source busy): try again every [`BUSY_RETRY`].
    Busy(String),
    /// Windows' privacy switch blocks camera access for desktop apps (docs/spec/v1.md §2.1).
    #[cfg_attr(not(target_os = "windows"), expect(dead_code, reason = "only Windows has the switch"))]
    Blocked(String),
    Failed(String),
}

impl OpenError {
    pub fn message(&self) -> &str {
        match self {
            OpenError::Busy(m) | OpenError::Blocked(m) | OpenError::Failed(m) => m,
        }
    }
}

pub fn judge(o: Observed) -> Signal {
    if !o.connected {
        return Signal::Unplugged;
    }
    if o.busy {
        return Signal::Busy;
    }
    match o.since_frame {
        Some(age) if age < NO_SIGNAL_AFTER => Signal::Live,
        Some(_) => Signal::NoSignal,
        None if o.since_start < NO_SIGNAL_AFTER => Signal::Starting,
        None => Signal::NoSignal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn no_signal_after_a_second_without_frames_and_cleared_by_the_next() {
        let open = Observed { connected: true, ..Default::default() };
        assert_eq!(judge(Observed { since_start: ms(300), ..open }), Signal::Starting);
        assert_eq!(judge(Observed { since_start: ms(1000), ..open }), Signal::NoSignal, "nothing ever arrived");
        assert_eq!(judge(Observed { since_start: ms(5000), since_frame: Some(ms(16)), ..open }), Signal::Live);
        assert_eq!(judge(Observed { since_start: ms(5000), since_frame: Some(ms(999)), ..open }), Signal::Live);
        assert_eq!(judge(Observed { since_start: ms(5000), since_frame: Some(ms(1000)), ..open }), Signal::NoSignal, "a frame gap");
        assert_eq!(judge(Observed { since_start: ms(6000), since_frame: Some(ms(10)), ..open }), Signal::Live, "the next frame clears it");
    }

    #[test]
    fn a_missing_card_outranks_everything_then_busy() {
        let gone = Observed { connected: false, busy: true, since_start: ms(5000), since_frame: Some(ms(10)) };
        assert_eq!(judge(gone), Signal::Unplugged);
        assert_eq!(judge(Observed { connected: true, ..gone }), Signal::Busy);
    }
}
