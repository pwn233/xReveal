//! The Input level meter in the Audio tab: whether sound is coming in, at a glance. Two segmented
//! bars (left, right) on a −60…0 dBFS scale, green through yellow and orange to red, that rise at
//! once, fall back smoothly, and hold their peak for a moment.

use std::time::Instant;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme;

/// Segments per bar.
pub const SEGMENTS: usize = 24;
/// The quietest level the meter shows, in dBFS.
const FLOOR_DB: f32 = -60.0;
/// How fast the bars fall back, in parts of the meter per second (20 dB/s).
const FALL: f32 = 20.0 / -FLOOR_DB;
/// How long a peak stays marked before it falls too, in seconds.
const HOLD: f32 = 1.0;

/// A linear level (0 to 1) as a part of the meter.
pub fn fraction(level: f32) -> f32 {
    if level <= 0.0 { 0.0 } else { ((20.0 * level.log10() - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0) }
}

/// Segment `i`'s colour: green for most of the range, then yellow, orange, and red at the top.
fn colour(i: usize) -> Hsla {
    let at = (i + 1) as f32 / SEGMENTS as f32;
    let hex = if at <= 0.6 {
        0x3fb950
    } else if at <= 0.8 {
        0xd4b62a
    } else if at <= 0.92 {
        0xe8833a
    } else {
        0xe5484d
    };
    rgb(hex).into()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Meter {
    /// What each bar shows, and its held peak, as parts of the meter.
    shown: [f32; 2],
    peak: [f32; 2],
    peak_age: [f32; 2],
    /// Sound is playing (an input is open).
    live: bool,
    last: Option<Instant>,
}

impl Meter {
    /// A new reading: the loudest samples since the last one, or `None` when nothing plays.
    pub fn update(&mut self, peaks: Option<(f32, f32)>) {
        let now = Instant::now();
        let dt = self.last.map_or(0.0, |t| now.duration_since(t).as_secs_f32());
        self.last = Some(now);
        self.step(peaks, dt);
    }

    fn step(&mut self, peaks: Option<(f32, f32)>, dt: f32) {
        self.live = peaks.is_some();
        let (left, right) = peaks.unwrap_or_default();
        for (side, level) in [fraction(left), fraction(right)].into_iter().enumerate() {
            // Rise at once; fall back smoothly.
            self.shown[side] = level.max(self.shown[side] - FALL * dt).max(0.0);
            if level >= self.peak[side] {
                self.peak[side] = level;
                self.peak_age[side] = 0.0;
            } else {
                self.peak_age[side] += dt;
                if self.peak_age[side] > HOLD {
                    self.peak[side] = (self.peak[side] - FALL * dt).max(self.shown[side]);
                }
            }
        }
    }

    pub fn render(&self) -> AnyElement {
        let pal = theme::palette();
        let bar = |side: usize, label: &'static str| {
            let lit = (self.shown[side] * SEGMENTS as f32).round() as usize;
            let peak = (self.peak[side] * SEGMENTS as f32).ceil() as usize;
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(px(10.)).text_xs().font_family(theme::mono_family()).text_color(pal.muted).child(label))
                .child(div().flex_1().flex().gap(px(2.)).children((0..SEGMENTS).map(|i| {
                    let on = i < lit || (peak > 0 && i + 1 == peak);
                    div().flex_1().h(px(6.)).rounded(px(1.5)).bg(if on { colour(i) } else { pal.raised })
                })))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(bar(0, "L"))
            .child(bar(1, "R"))
            .when(!self.live, |d| d.child(div().text_xs().text_color(pal.muted).child("No sound playing")))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Meter, fraction};

    #[test]
    fn levels_on_a_decibel_scale() {
        assert_eq!(fraction(0.0), 0.0);
        assert_eq!(fraction(1.0), 1.0);
        assert!((fraction(0.1) - 2.0 / 3.0).abs() < 1e-4, "−20 dBFS is two thirds up");
        assert_eq!(fraction(0.0001), 0.0, "below −60 dBFS is off the bottom");
    }

    #[test]
    fn bars_rise_at_once_fall_back_and_hold_their_peak() {
        let mut m = Meter::default();
        m.step(Some((1.0, 0.1)), 0.05);
        assert_eq!(m.shown[0], 1.0);
        m.step(Some((0.0, 0.0)), 0.5);
        assert!((m.shown[0] - (1.0 - 20.0 / 60.0 * 0.5)).abs() < 1e-4, "falls 20 dB/s");
        assert_eq!(m.peak[0], 1.0, "the peak holds for a second");
        m.step(Some((0.0, 0.0)), 1.0);
        assert!(m.peak[0] < 1.0, "then falls too");
        m.step(None, 0.05);
        assert!(!m.live);
    }
}
