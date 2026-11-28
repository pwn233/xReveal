//! `xreveal --report`: writes down what the hardware pass records about a machine and its inputs
//! (docs/spec/v1.md §7, step 1): every capture device with its formats and IDs, a few seconds of
//! real capture from each input device (what arrives and how evenly), the Device settings window,
//! and the audio devices with their pairing. Prints it and saves `xreveal-report.txt`.

use std::fmt::Write as _;

use crate::audio::pipeline;
use crate::source::{self, SourceInfo};

/// How long each input device is captured for the pacing figures.
const MEASURE_SECONDS: f64 = 5.0;

/// Frame pacing from the gaps between arrivals, in ms.
#[derive(Debug, PartialEq)]
pub struct Pacing {
    pub frames: usize,
    pub fps: f64,
    pub median: f64,
    pub p99: f64,
    pub max: f64,
    /// Gaps over 1.5× the median: a frame came late or went missing.
    pub late: usize,
}

pub fn pacing(gaps: &[f64]) -> Option<Pacing> {
    if gaps.is_empty() {
        return None;
    }
    let mut sorted = gaps.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    let median = at(0.5);
    let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
    Some(Pacing {
        frames: gaps.len() + 1,
        fps: if mean > 0.0 { 1000.0 / mean } else { 0.0 },
        median,
        p99: at(0.99),
        max: sorted[sorted.len() - 1],
        late: gaps.iter().filter(|g| **g > median * 1.5).count(),
    })
}

/// Prints the report, saves it next to where xReveal was started, and exits.
pub fn run() -> ! {
    #[cfg(target_os = "windows")]
    unsafe {
        // Release builds have no console of their own: write to the terminal that started us.
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
    let text = build();
    println!("{text}");
    match std::fs::write("xreveal-report.txt", &text) {
        Ok(()) => println!("Saved to xreveal-report.txt"),
        Err(error) => eprintln!("Couldn't save xreveal-report.txt: {error}"),
    }
    // A finished report isn't a crash (docs/spec/v1.md §2.11).
    crate::crash::end();
    std::process::exit(0);
}

fn build() -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# xReveal {} hardware report\n", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "- OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    if let Some(path) = crate::setup::default_path() {
        let _ = writeln!(out, "- Remembered setup: {}", path.display());
    }

    let sources = list_sources();
    let inputs = pipeline::inputs();
    let _ = writeln!(out, "\n## Capture devices ({})", sources.len());
    for info in &sources {
        source_section(&mut out, info, &inputs);
    }

    let _ = writeln!(out, "\n## Sound inputs ({})", inputs.len());
    for device in &inputs {
        let _ = writeln!(out, "- {} — `{}`{}", device.name, device.id, audio_container(&device.id));
    }
    let outputs = pipeline::outputs();
    let _ = writeln!(out, "\n## Sound outputs ({})", outputs.len());
    for device in &outputs {
        let _ = writeln!(out, "- {} — `{}`", device.name, device.id);
    }
    out
}

fn source_section(out: &mut String, info: &SourceInfo, inputs: &[pipeline::AudioDevice]) {
    let id = &info.identity;
    let _ = writeln!(out, "\n### {}{}", id.name, if info.external { "" } else { " (built in)" });
    let _ = writeln!(out, "- OS ID: `{}`", id.os_id);
    let _ = writeln!(out, "- Serial: {}", id.serial.as_deref().unwrap_or("none"));
    let _ = writeln!(out, "- Model: {}", id.model.map_or("unknown".into(), |(v, p)| format!("{v:04x}:{p:04x}")));
    let _ = writeln!(out, "{}", video_extras(&id.os_id));
    let paired = pipeline::paired_input(id, inputs);
    let paired_name = paired.as_ref().and_then(|p| inputs.iter().find(|d| &d.id == p)).map(|d| d.name.clone());
    let _ = writeln!(out, "- Its own sound: {}", paired_name.unwrap_or_else(|| "not found".into()));
    let _ = writeln!(out, "- Formats the device lists:");
    for f in &info.formats {
        let _ = writeln!(out, "  - {}×{} @ {} {:?}", f.width, f.height, f.fps, f.pixel);
    }
    let choices: Vec<String> = source::choices(&info.formats).into_iter().map(crate::drawer::readout).collect();
    let _ = writeln!(out, "- xReveal offers: {}", choices.join(", "));

    if !info.external {
        return;
    }
    let Some(choice) = source::default_choice(&info.formats) else { return };
    let Some(native) = source::best_native(&info.formats, choice) else { return };
    let _ = writeln!(out, "- Capture for {MEASURE_SECONDS:.0} s at {}:", crate::drawer::readout(choice));
    match measure(&id.os_id, native) {
        Ok((description, gaps)) => {
            let _ = writeln!(out, "  - Delivered: {description}");
            match pacing(&gaps) {
                Some(p) => {
                    let _ = writeln!(
                        out,
                        "  - {} frames, {:.2} fps; gap median {:.2} ms, p99 {:.2} ms, max {:.2} ms; {} late or missing",
                        p.frames, p.fps, p.median, p.p99, p.max, p.late
                    );
                }
                None => {
                    let _ = writeln!(out, "  - No frames arrived (no signal? HDCP on?)");
                }
            }
        }
        Err(error) => {
            let _ = writeln!(out, "  - Couldn't capture: {error}");
        }
    }
}

fn list_sources() -> Vec<SourceInfo> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    return crate::platform::VideoPipeline::list_sources();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Vec::new()
}

fn measure(os_id: &str, native: source::NativeFormat) -> Result<(String, Vec<f64>), String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    return crate::platform::VideoPipeline::measure(os_id, native, MEASURE_SECONDS);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (os_id, native);
        Err("capture isn't implemented on this OS".into())
    }
}

/// Windows only: the device's container ID and whether it has a Device settings window.
fn video_extras(os_id: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        let container = crate::platform::windows::container::of_video(os_id).map_or("none".into(), |g| format!("{g:?}"));
        let settings = crate::platform::VideoPipeline::offers_device_settings(os_id);
        format!("- Container ID: {container}\n- Device settings window: {}", if settings { "yes" } else { "no" })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = os_id;
        "- Device settings window: none on macOS".into()
    }
}

fn audio_container(id: &str) -> String {
    #[cfg(target_os = "windows")]
    return crate::platform::windows::container::of_audio_endpoint(id).map_or(String::new(), |g| format!(" (container {g:?})"));
    #[cfg(not(target_os = "windows"))]
    {
        let _ = id;
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::pacing;

    /// Real hardware: 2 s of capture from the first device (on a Mac, its own camera). Run by hand:
    /// `cargo test -- --ignored measure_first_device --nocapture`.
    #[test]
    #[ignore = "needs a camera or capture device"]
    fn measure_first_device() {
        use crate::source;
        let info = super::list_sources().into_iter().next().expect("a capture device");
        let choice = source::default_choice(&info.formats).expect("a format");
        let native = source::best_native(&info.formats, choice).expect("its native format");
        let (description, gaps) = crate::platform::VideoPipeline::measure(&info.identity.os_id, native, 2.0).expect("captures");
        let p = pacing(&gaps).expect("frames arrived");
        eprintln!("{description}: {p:?}");
        assert!(p.frames > 10, "{p:?}");
    }

    #[test]
    fn pacing_from_arrival_gaps() {
        assert_eq!(pacing(&[]), None);
        let mut gaps = vec![16.7; 98];
        gaps.push(33.4); // one frame missing
        gaps.push(16.6);
        let p = pacing(&gaps).unwrap();
        assert_eq!(p.frames, 101);
        assert!((p.fps - 59.4).abs() < 0.2, "{}", p.fps);
        assert_eq!(p.median, 16.7);
        assert_eq!(p.max, 33.4);
        assert_eq!(p.late, 1);
    }
}
