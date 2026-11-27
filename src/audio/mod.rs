//! Sound (docs/spec/v1.md §2.7, §3.6): which input and output play, and keeping them playing.

pub mod echo;
pub mod pacer;
pub mod pipeline;
pub mod ring;

use std::time::{Duration, Instant};

use crate::setup::Setup;
use crate::source::SourceIdentity;
pub use echo::DeviceKind;
use echo::EchoGuard;
use pipeline::{AudioDevice, AudioPipeline, MAX_BUFFER, Mix};

/// The key `setup.audio_buffers` uses for the system output.
const SYSTEM_OUTPUT: &str = "system-output";
/// Above this, an output's delay earns the Bluetooth hint (docs/spec/v1.md §2.7).
const HIGH_LATENCY_MS: f64 = 40.0;

/// What the Audio tab shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioModel {
    pub inputs: Vec<AudioDevice>,
    pub outputs: Vec<AudioDevice>,
    /// The input playing, and the one paired with the card (its own audio).
    pub input: Option<String>,
    pub paired: Option<String>,
    /// The input chosen in the Audio tab; `None` follows the card (its own audio).
    pub chosen_input: Option<String>,
    /// The chosen output; `None` is the system output.
    pub output: Option<String>,
    pub volume: f32,
    pub muted: bool,
    /// "Bluetooth adds about N ms…", when the output is Bluetooth or slow.
    pub latency_hint: Option<String>,
    /// Why there's no sound, or why it isn't the input chosen.
    pub notice: Option<String>,
}

/// The sound for whatever input is on screen.
#[derive(Default)]
pub struct Sound {
    pipeline: Option<AudioPipeline>,
    notice: Option<String>,
    paired: Option<String>,
    card: Option<SourceIdentity>,
    seen_underruns: u64,
    next_try: Option<Instant>,
    /// The echo guard (#112): who owns the mute while the computer's mic plays to its speakers.
    echo: EchoGuard,
    /// The mute the guard just set, for the app to remember.
    mute_change: Option<bool>,
}

impl Sound {
    /// Starts (or restarts) the sound for `card` with the setup's route, volume and offset.
    pub fn start_for(&mut self, card: Option<&SourceIdentity>, setup: &Setup) {
        self.pipeline = None;
        self.card = card.cloned();
        self.seen_underruns = 0;
        let inputs = pipeline::inputs();
        self.paired = card.and_then(|c| pipeline::paired_input(c, &inputs));
        let chosen = setup.audio.input.clone().filter(|id| inputs.iter().any(|d| &d.id == id));
        self.notice = None;
        if setup.audio.input.is_some() && chosen.is_none() && self.paired.is_some() {
            self.notice = Some("Your chosen sound input isn't connected, so the input device's own audio plays instead".into());
        }
        let Some(input) = chosen.or_else(|| self.paired.clone()) else {
            if card.is_some() {
                self.notice = Some("Choose where the sound comes from below".into());
            }
            return;
        };
        let output = setup.audio.output.clone().filter(|id| pipeline::outputs().iter().any(|d| &d.id == id));
        let key = output.clone().unwrap_or_else(|| SYSTEM_OUTPUT.into());
        let buffer = setup.audio_buffers.get(&key).copied();
        // The echo guard (#112): the computer's own mic through its own speakers starts muted,
        // before a single sample plays, unless the person already unmuted this route.
        let echoing = echo::echoes(pipeline::kind(Some(&input), true), pipeline::kind(output.as_deref(), false));
        let mut mix = mix(setup);
        let (muted, change) = self.echo.start((input.clone(), output.clone()), echoing, mix.muted);
        mix.muted = muted;
        if change.is_some() {
            self.mute_change = change;
        }
        match AudioPipeline::start(&input, output.as_deref(), buffer, mix) {
            Ok(p) => self.pipeline = Some(p),
            Err(error) => self.notice = Some(error),
        }
    }

    /// The mute the echo guard just set (once), for the app to remember: so the Audio tab shows it
    /// and the person can undo it.
    pub fn take_mute_change(&mut self) -> Option<bool> {
        self.mute_change.take()
    }

    /// The person set Mute to `muted`: from now on the mute is theirs (see [`EchoGuard::chosen`]).
    pub fn mute_chosen(&mut self, muted: bool) {
        self.echo.chosen(muted);
    }

    /// The echo guard is holding the sound muted: say why (Audio tab, and a bar over the picture).
    pub fn echo_muted(&self, setup: &Setup) -> bool {
        self.pipeline.is_some() && self.echo.holding(setup.audio.muted)
    }

    /// The input's loudest samples (left, right) since the last call, or `None` when nothing plays.
    pub fn take_peaks(&self) -> Option<(f32, f32)> {
        self.pipeline.as_ref().map(AudioPipeline::take_peaks)
    }

    /// Stops the sound (nothing on screen).
    pub fn stop(&mut self) {
        self.pipeline = None;
        self.card = None;
        self.echo.stop();
    }

    /// Volume, mute or the A/V sync offset changed.
    pub fn set_mix(&self, setup: &Setup) {
        if let Some(p) = &self.pipeline {
            p.set_mix(mix(setup));
        }
    }

    /// Twice a second. A dropout while sound is arriving doubles the output's buffer, which is
    /// returned to be remembered for that device (docs/spec/v1.md §3.6); a stream the OS stopped
    /// (a device unplugged) is tried again every 2 s.
    pub fn watch(&mut self, setup: &Setup) -> Option<(String, u32)> {
        let p = self.pipeline.as_ref()?;
        if p.failed() {
            let error = p.error();
            if self.next_try.is_none_or(|t| Instant::now() >= t) {
                self.next_try = Some(Instant::now() + Duration::from_secs(2));
                let card = self.card.clone();
                self.start_for(card.as_ref(), setup);
                if self.pipeline.is_none() && self.notice.is_none() {
                    self.notice = error;
                }
            }
            return None;
        }
        let underruns = p.underruns();
        if underruns <= self.seen_underruns {
            return None;
        }
        self.seen_underruns = underruns;
        let buffer = p.buffer?;
        if buffer >= MAX_BUFFER {
            return None;
        }
        let key = p.output_id.clone().unwrap_or_else(|| SYSTEM_OUTPUT.into());
        Some((key, (buffer * 2).min(MAX_BUFFER)))
    }

    /// The snapshot for the Audio tab.
    pub fn model(&self, setup: &Setup) -> AudioModel {
        let latency_hint = self.pipeline.as_ref().and_then(|p| {
            let ms = p.output_latency_ms()?;
            (p.bluetooth || ms > HIGH_LATENCY_MS).then(|| {
                let what = if p.bluetooth { "Bluetooth adds" } else { "This output adds" };
                format!("{what} about {ms:.0} ms of sound delay. Wired headphones or speakers keep sound in step with the picture.")
            })
        });
        AudioModel {
            inputs: pipeline::inputs(),
            outputs: pipeline::outputs(),
            input: self.pipeline.as_ref().map(|p| p.input_id.clone()),
            paired: self.paired.clone(),
            chosen_input: setup.audio.input.clone(),
            output: setup.audio.output.clone(),
            volume: setup.audio.volume,
            muted: setup.audio.muted,
            latency_hint,
            notice: if self.echo_muted(setup) { Some(echo::MUTED_NOTICE.into()) } else { self.notice.clone() },
        }
    }
}

fn mix(setup: &Setup) -> Mix {
    Mix { volume: setup.audio.volume, muted: setup.audio.muted, offset_ms: setup.delay.av_sync_offset_ms }
}

