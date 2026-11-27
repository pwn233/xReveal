//! The echo guard (#112, docs/spec/v1.md §2.7): the computer's own microphone played through its
//! own speakers makes a feedback loop that doesn't stop. Such a route starts muted, with a notice.
//! It goes only by which devices play, never by listening: game sound can look like feedback.

/// What a sound device is, as far as the echo guard needs to know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// The computer's own microphone (not one on a headset or in the jack).
    BuiltInMicrophone,
    /// The computer's own speakers (not headphones).
    BuiltInSpeakers,
    Other,
}

/// Whether playing `input` through `output` makes a feedback loop.
pub fn echoes(input: DeviceKind, output: DeviceKind) -> bool {
    input == DeviceKind::BuiltInMicrophone && output == DeviceKind::BuiltInSpeakers
}

/// What the Audio tab and the bar over the picture say while the guard holds the sound muted.
pub const MUTED_NOTICE: &str = if cfg!(target_os = "macos") {
    "Sound muted: the Mac's microphone would echo through its own speakers. Pick the input device's own audio in Settings › Audio, or use headphones."
} else {
    "Sound muted: the computer's microphone would echo through its own speakers. Pick the input device's own audio in Settings › Audio, or use headphones."
};

/// An input and output played together (`None` output: the system's).
pub type Route = (String, Option<String>);

/// Who owns the mute, so the guard never fights the person.
#[derive(Debug, Default)]
pub struct EchoGuard {
    /// The route playing echoes.
    echoing: Option<Route>,
    /// An echoing route the person unmuted anyway (for this session): left alone until it changes.
    accepted: Option<Route>,
    /// The mute on now is the guard's: it goes again when the route stops echoing.
    guard_muted: bool,
    /// The person turned Mute on themselves: never the guard's to lift.
    person_muted: bool,
}

impl EchoGuard {
    /// A route starts playing: `echoing` says whether it makes a loop, `muted` is the remembered
    /// mute. Returns the mute to play with and, when the guard changes it, the new remembered mute.
    pub fn start(&mut self, route: Route, echoing: bool, muted: bool) -> (bool, Option<bool>) {
        self.echoing = echoing.then_some(route);
        if self.accepted != self.echoing {
            self.accepted = None;
        }
        if self.echoing.is_some() && self.accepted.is_none() {
            if !muted {
                self.guard_muted = true;
                return (true, Some(true));
            }
            // Already muted, e.g. by the guard before a relaunch: the guard's too, unless the
            // person muted it themselves.
            self.guard_muted |= !self.person_muted;
        } else if self.echoing.is_none() && self.guard_muted {
            // The route no longer echoes (the card's own audio, headphones): the guard's mute goes.
            self.guard_muted = false;
            return (false, Some(false));
        }
        (muted, None)
    }

    /// Nothing plays any more.
    pub fn stop(&mut self) {
        self.echoing = None;
    }

    /// The person set Mute to `muted`: from now on the mute is theirs. Unmuting an echoing route is
    /// their choice, kept until the route changes.
    pub fn chosen(&mut self, muted: bool) {
        self.guard_muted = false;
        self.person_muted = muted;
        if !muted {
            self.accepted = self.echoing.clone();
        }
    }

    /// The guard is holding the sound muted (the notice shows), given the remembered mute.
    pub fn holding(&self, muted: bool) -> bool {
        muted && self.echoing.is_some() && self.accepted.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::{DeviceKind, EchoGuard, Route, echoes};

    fn mic() -> Route {
        ("BuiltInMicrophoneDevice".into(), None)
    }
    fn card() -> Route {
        ("USB2 Digital Audio".into(), None)
    }

    #[test]
    fn only_the_computers_own_mic_through_its_own_speakers_echoes() {
        use DeviceKind::*;
        assert!(echoes(BuiltInMicrophone, BuiltInSpeakers));
        // Headphones, a USB or Bluetooth output, the card's own audio: no loop.
        assert!(!echoes(BuiltInMicrophone, Other));
        assert!(!echoes(Other, BuiltInSpeakers));
        assert!(!echoes(BuiltInSpeakers, BuiltInMicrophone));
    }

    #[test]
    fn an_echoing_route_starts_muted_and_the_mute_goes_with_it() {
        let mut guard = EchoGuard::default();
        // Seen on the MacBook: the remembered mic through the speakers, unmuted.
        assert_eq!(guard.start(mic(), true, false), (true, Some(true)), "muted before any sound plays");
        assert!(guard.holding(true));
        // A restart of the same route (a dropout, a device back) changes nothing.
        assert_eq!(guard.start(mic(), true, true), (true, None));
        // The card's own audio: the guard's mute goes by itself.
        assert_eq!(guard.start(card(), false, true), (false, Some(false)));
        assert!(!guard.holding(false));
        // And a safe route never touches the mute again.
        assert_eq!(guard.start(card(), false, false), (false, None));
    }

    #[test]
    fn unmuting_an_echoing_route_is_the_persons_choice_until_the_route_changes() {
        let mut guard = EchoGuard::default();
        guard.start(mic(), true, false);
        guard.chosen(false);
        assert!(!guard.holding(false));
        assert_eq!(guard.start(mic(), true, false), (false, None), "the same route stays unmuted");
        // Away and back: a new route, so the guard mutes again.
        guard.start(card(), false, false);
        assert_eq!(guard.start(mic(), true, false), (true, Some(true)));
    }

    #[test]
    fn the_persons_own_mute_is_never_lifted() {
        let mut guard = EchoGuard::default();
        guard.start(mic(), true, false);
        guard.chosen(false);
        guard.chosen(true);
        assert_eq!(guard.start(mic(), true, true), (true, None));
        assert_eq!(guard.start(card(), false, true), (true, None), "muted by the person: stays muted");
    }

    #[test]
    fn a_mute_kept_from_before_a_relaunch_counts_as_the_guards() {
        let mut guard = EchoGuard::default();
        assert_eq!(guard.start(mic(), true, true), (true, None));
        assert!(guard.holding(true));
        assert_eq!(guard.start(card(), false, true), (false, Some(false)));
    }
}
