//! Everything that decides how a Source frame turns into the picture on screen, in one place
//! (docs/spec/v1.md §2.5, §2.6, §3.4):
//!
//! - **colour**: undoing any level squeeze the OS applied, Colour range, Colour space and the
//!   Picture adjustments, folded into one 3×4 matrix plus a gamma exponent;
//! - **geometry**: the Target ratio rectangle and how the Fill mode maps it back onto the Source.
//!
//! The GPU shaders (MSL and HLSL) only evaluate what [`plan`] produces. [`reference`] evaluates the
//! same plan on the CPU; the GPU tests on both OSes compare against it, which keeps the two
//! shaders in step with each other and with this file.

use super::fill::{self, Rect};

/// User choice for a setting that has an automatic value.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Choice<T> {
    #[default]
    Auto,
    Manual(T),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColourRange {
    Full,
    Limited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColourSpace {
    Bt709,
    Bt601,
}

/// xReveal's own Picture adjustments. Neutral values leave the picture unchanged.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Adjustments {
    /// Added to luma, −0.5..=0.5. Neutral 0.
    pub brightness: f32,
    /// Luma scale around mid-grey, 0..=2. Neutral 1.
    pub contrast: f32,
    /// Chroma scale, 0..=2. Neutral 1.
    pub saturation: f32,
    /// Mid-tone exponent, 0.5..=2 (higher is brighter). Neutral 1.
    pub gamma: f32,
    /// Hue rotation in degrees, −180..=180. Neutral 0.
    pub hue: f32,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self { brightness: 0.0, contrast: 1.0, saturation: 1.0, gamma: 1.0, hue: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FillMode {
    #[default]
    Fit,
    Stretch,
    Crop,
    SmartStretch,
}

/// The shape xReveal fills on screen, as width ÷ height.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TargetRatio(pub f64);

impl TargetRatio {
    pub const PRESETS: [(&'static str, TargetRatio); 5] = [
        ("4:3", TargetRatio(4.0 / 3.0)),
        ("16:9", TargetRatio(16.0 / 9.0)),
        ("16:10", TargetRatio(16.0 / 10.0)),
        ("21:9", TargetRatio(64.0 / 27.0)),
        ("32:9", TargetRatio(32.0 / 9.0)),
    ];

    #[cfg_attr(not(test), expect(dead_code, reason = "the Custom Target ratio in the Settings drawer (its own slice)"))]
    pub fn custom(width: f64, height: f64) -> Self {
        Self(width / height)
    }
}

impl Default for TargetRatio {
    fn default() -> Self {
        Self(16.0 / 9.0)
    }
}

/// The settings that shape the picture (part of the Remembered setup).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct PictureSettings {
    /// Per Source: saved in that Source's entry of the Remembered setup, not with the rest.
    #[serde(skip)]
    pub colour_range: Choice<ColourRange>,
    #[serde(skip)]
    pub colour_space: Choice<ColourSpace>,
    pub adjustments: Adjustments,
    pub target_ratio: TargetRatio,
    pub fill_mode: FillMode,
    /// 0..=1; 0 looks exactly like Stretch. Default 0.5.
    pub smart_stretch_strength: f32,
}

impl Default for PictureSettings {
    fn default() -> Self {
        Self {
            colour_range: Choice::Auto,
            colour_space: Choice::Auto,
            adjustments: Adjustments::default(),
            target_ratio: TargetRatio::default(),
            fill_mode: FillMode::Fit,
            smart_stretch_strength: 0.5,
        }
    }
}

/// How the frame's bytes are laid out in the textures the shader reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Luma plane (R8) + interleaved CbCr plane (RG8) at half width and half height.
    Nv12,
    /// One RGBA8 texture at half width: (Y0, Cb, Y1, Cr) per texel.
    Yuy2,
    /// One R8 texture three times as wide: Y, Cb, Cr per pixel (xReveal's own MJPEG decode).
    #[cfg_attr(
        all(target_os = "macos", not(test)),
        expect(dead_code, reason = "macOS decodes MJPEG in the OS; only Windows draws this layout")
    )]
    Ycbcr444,
}

impl Layout {
    /// The value the shaders branch on (`misc.y`).
    fn shader_code(self) -> f32 {
        match self {
            Layout::Nv12 => 0.0,
            Layout::Yuy2 => 1.0,
            Layout::Ycbcr444 => 2.0,
        }
    }
}

/// Facts about the frame being drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameInfo {
    pub width: u32,
    pub height: u32,
    pub layout: Layout,
    /// The OS squeezed full-range levels into 16–235 / 16–240 before xReveal saw them (macOS does
    /// this to the Reference card's decoded `420v` frames). Undone first.
    pub os_squeezed: bool,
}

/// Auto Colour range: assume Full (docs/spec/v1.md §2.6).
pub fn resolve_range(choice: Choice<ColourRange>) -> ColourRange {
    match choice {
        Choice::Auto => ColourRange::Full,
        Choice::Manual(range) => range,
    }
}

/// Auto Colour space: BT.709 for pictures 720 lines and up, BT.601 below.
pub fn resolve_space(choice: Choice<ColourSpace>, height: u32) -> ColourSpace {
    match choice {
        Choice::Auto if height >= 720 => ColourSpace::Bt709,
        Choice::Auto => ColourSpace::Bt601,
        Choice::Manual(space) => space,
    }
}

/// Uniforms for the video shader. Layout is shared byte-for-byte by `video.metal` and
/// `video.hlsl` (16-byte rows).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShaderParams {
    /// R, G, B = row · (Y, Cb, Cr, 1), with Y, Cb, Cr the raw texture values in 0..=1.
    pub colour: [[f32; 4]; 3],
    /// Source UV = offset + screen UV × scale (affine mapping); also the vertical mapping in
    /// Smart stretch. xy = offset, zw = scale.
    pub mapping: [f32; 4],
    /// x = k (Target width ÷ Source width at equal height), y = strength t, z = exponent p,
    /// w = 1 when Smart stretch applies.
    pub smart: [f32; 4],
    /// xy = luma texture size in texels, zw = chroma texture size in texels.
    pub sizes: [f32; 4],
    /// x = 1 ÷ gamma, y = layout (0 NV12, 1 YUY2, 2 Y-Cb-Cr 4:4:4), zw unused.
    pub misc: [f32; 4],
}

/// What to draw this frame: the viewport (in surface pixels) and the shader's uniforms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub viewport: Rect,
    pub params: ShaderParams,
}

/// Plans one frame drawn onto a surface of `surface_size` pixels.
pub fn plan(frame: FrameInfo, settings: &PictureSettings, surface_size: (f64, f64)) -> Plan {
    let (sw, sh) = surface_size;
    let source_ratio = frame.width as f64 / frame.height.max(1) as f64;
    let target = fill::fit(settings.target_ratio.0, 1.0, sw, sh);
    let k = settings.target_ratio.0 / source_ratio;

    let full = [0.0, 0.0, 1.0, 1.0];
    let (viewport, mapping, smart) = match settings.fill_mode {
        FillMode::Fit => (fill::fit(frame.width as f64, frame.height as f64, target.width, target.height).offset(target), full, None),
        FillMode::Stretch => (target, full, None),
        FillMode::Crop => {
            let mapping = if k >= 1.0 {
                let visible = 1.0 / k; // fraction of the Source's height that fits
                [0.0, (1.0 - visible) / 2.0, 1.0, visible]
            } else {
                let visible = k; // fraction of the Source's width that fits
                [(1.0 - visible) / 2.0, 0.0, visible, 1.0]
            };
            (target, mapping, None)
        }
        FillMode::SmartStretch => {
            let p = smart_exponent(k);
            (target, full, Some([k, settings.smart_stretch_strength.clamp(0.0, 1.0) as f64, p, 1.0]))
        }
    };

    let (luma, chroma) = match frame.layout {
        Layout::Nv12 => ((frame.width, frame.height), (frame.width.div_ceil(2), frame.height.div_ceil(2))),
        Layout::Yuy2 => ((frame.width, frame.height), (frame.width.div_ceil(2), frame.height)),
        Layout::Ycbcr444 => ((frame.width, frame.height), (frame.width, frame.height)),
    };
    let colour = colour_matrix(frame, settings);
    let gamma = settings.adjustments.gamma.clamp(0.1, 10.0);
    Plan {
        viewport,
        params: ShaderParams {
            colour: colour.map(|row| row.map(|v| v as f32)),
            mapping: mapping.map(|v| v as f32),
            smart: smart.unwrap_or([1.0, 0.0, 3.0, 0.0]).map(|v| v as f32),
            sizes: [luma.0 as f32, luma.1 as f32, chroma.0 as f32, chroma.1 as f32],
            misc: [1.0 / gamma, frame.layout.shader_code(), 0.0, 0.0],
        },
    }
}

/// The Smart stretch curve's exponent: as steep as possible while staying monotonic.
pub fn smart_exponent(k: f64) -> f64 {
    if k > 1.0 { (0.95 * k / (k - 1.0)).min(3.0) } else { 3.0 }
}

/// Smart stretch: screen position `v` (distance from the centre, 0..=1 of the half-width) →
/// Source position, same units. `g(v) = (1−t)·v + t·(k·v − (k−1)·v^p)`; t = 0 is plain Stretch,
/// t = 1 keeps the centre undistorted. Narrower targets (k < 1) squeeze the edges instead.
#[cfg(test)]
pub fn smart_curve(v: f64, k: f64, t: f64, p: f64) -> f64 {
    (1.0 - t) * v + t * (k * v - (k - 1.0) * v.powf(p))
}

/// The 3×4 matrix taking raw texture values (Y, Cb, Cr, 1) to RGB, before gamma.
fn colour_matrix(frame: FrameInfo, settings: &PictureSettings) -> [[f64; 4]; 3] {
    // Each step is affine on (Y, Cb, Cr): y' = a·y + b; c' = s·c + o (same for both chroma).
    // 1. Undo the OS squeeze back to the Source's own levels.
    let (mut ya, mut yb, mut ca, mut cb) = (1.0, 0.0, 1.0, 0.0);
    if frame.os_squeezed {
        (ya, yb) = (255.0 / 219.0, -16.0 / 219.0);
        (ca, cb) = (255.0 / 224.0, 128.0 / 255.0 - 128.0 / 224.0);
    }
    // 2. The Source's own range → Y in 0..1, Cb/Cr centred on 0.
    let (ra, rb, rca, rcb) = match resolve_range(settings.colour_range) {
        ColourRange::Full => (1.0, 0.0, 1.0, -128.0 / 255.0),
        ColourRange::Limited => (255.0 / 219.0, -16.0 / 219.0, 255.0 / 224.0, -128.0 / 224.0),
    };
    let (ya, yb) = (ra * ya, ra * yb + rb);
    let (ca, cb) = (rca * ca, rca * cb + rcb);

    // 3. Picture adjustments in YCbCr: contrast + brightness on Y, saturation + hue on CbCr.
    let adj = settings.adjustments;
    let contrast = adj.contrast as f64;
    let (ya, yb) = (contrast * ya, contrast * yb + 0.5 * (1.0 - contrast) + adj.brightness as f64);
    let saturation = adj.saturation as f64;
    let (sin, cos) = (adj.hue as f64).to_radians().sin_cos();
    // [Cb'; Cr'] = saturation · R(hue) · [Cb; Cr], with Cb = ca·cb_raw + cb.
    let rot = [[saturation * cos, -saturation * sin], [saturation * sin, saturation * cos]];

    // 4. Colour space: Y, Cb, Cr → R, G, B.
    let (rcr, gcb, gcr, bcb) = match resolve_space(settings.colour_space, frame.height) {
        ColourSpace::Bt709 => (1.5748, -0.1873, -0.4681, 1.8556),
        ColourSpace::Bt601 => (1.402, -0.344136, -0.714136, 1.772),
    };
    let to_rgb = [[1.0, 0.0, rcr], [1.0, gcb, gcr], [1.0, bcb, 0.0]];

    // Compose: RGB = M · [Y'; Cb'; Cr'], Y' = ya·y + yb, [Cb';Cr'] = rot · (ca·[cb;cr] + cb).
    let mut out = [[0.0; 4]; 3];
    for (row, m) in out.iter_mut().zip(to_rgb) {
        let chroma_cb = m[1] * rot[0][0] + m[2] * rot[1][0];
        let chroma_cr = m[1] * rot[0][1] + m[2] * rot[1][1];
        row[0] = m[0] * ya;
        row[1] = chroma_cb * ca;
        row[2] = chroma_cr * ca;
        row[3] = m[0] * yb + (chroma_cb + chroma_cr) * cb;
    }
    out
}

impl Rect {
    fn offset(self, by: Rect) -> Rect {
        Rect { x: self.x + by.x, y: self.y + by.y, ..self }
    }
}

/// The shaders' maths on the CPU, for tests: same mapping, same Catmull-Rom taps, same colour.
#[cfg(test)]
pub mod reference {
    use super::{Layout, Plan, ShaderParams};

    /// The textures a frame is uploaded into, as raw bytes.
    pub struct Planes<'a> {
        pub layout: Layout,
        /// NV12: the luma plane (1 byte per texel). YUY2: the packed plane (4 bytes per texel).
        pub first: &'a [u8],
        /// NV12: the CbCr plane (2 bytes per texel). YUY2: unused.
        pub second: &'a [u8],
    }

    /// Renders `plan` into a `width`×`height` RGB image (0..=1 per channel); black outside the
    /// viewport, like the GPU's clear colour.
    pub fn render(planes: &Planes, plan: &Plan, width: u32, height: u32) -> Vec<[f32; 3]> {
        let mut out = vec![[0.0; 3]; (width * height) as usize];
        let vp = plan.viewport;
        for py in 0..height {
            for px in 0..width {
                let (x, y) = (px as f64 + 0.5, py as f64 + 0.5);
                if x < vp.x || y < vp.y || x >= vp.x + vp.width || y >= vp.y + vp.height {
                    continue;
                }
                let uv = [((x - vp.x) / vp.width) as f32, ((y - vp.y) / vp.height) as f32];
                out[(py * width + px) as usize] = shade(planes, &plan.params, uv);
            }
        }
        out
    }

    /// One screen pixel at viewport UV `uv`.
    pub fn shade(planes: &Planes, p: &ShaderParams, uv: [f32; 2]) -> [f32; 3] {
        let src = source_uv(p, uv);
        // NV12's planes are filtered in hardware, in 5 fetches that leave out the corner taps.
        let corners = planes.layout != Layout::Nv12;
        let luma = bicubic(src, [p.sizes[0], p.sizes[1]], corners, |x, y| fetch_luma(planes, p, x, y));
        let cb = bicubic(src, [p.sizes[2], p.sizes[3]], corners, |x, y| fetch_chroma(planes, p, x, y)[0]);
        let cr = bicubic(src, [p.sizes[2], p.sizes[3]], corners, |x, y| fetch_chroma(planes, p, x, y)[1]);
        let ycc = [luma, cb, cr, 1.0];
        let rgb = p.colour.map(|row| (row[0] * ycc[0] + row[1] * ycc[1] + row[2] * ycc[2] + row[3] * ycc[3]).clamp(0.0, 1.0));
        rgb.map(|c| c.powf(p.misc[0]))
    }

    pub fn source_uv(p: &ShaderParams, uv: [f32; 2]) -> [f32; 2] {
        let mut u = p.mapping[0] + uv[0] * p.mapping[2];
        let v = p.mapping[1] + uv[1] * p.mapping[3];
        if p.smart[3] > 0.5 {
            let (k, t, e) = (p.smart[0], p.smart[1], p.smart[2]);
            let s = uv[0] * 2.0 - 1.0;
            let a = s.abs();
            let g = (1.0 - t) * a + t * (k * a - (k - 1.0) * a.powf(e));
            u = 0.5 + 0.5 * s.signum() * g;
        }
        [u, v]
    }

    /// Catmull-Rom bicubic over 4×4 texels, clamped at the edges (same taps as the shaders).
    /// Without `corners`, the four corner taps (both outside the middle two on each axis) are left
    /// out and the rest renormalised, as the shaders' 5-fetch NV12 path does (#118).
    fn bicubic(uv: [f32; 2], size: [f32; 2], corners: bool, fetch: impl Fn(i32, i32) -> f32) -> f32 {
        let px = uv[0] * size[0] - 0.5;
        let py = uv[1] * size[1] - 0.5;
        let (ix, iy) = (px.floor(), py.floor());
        let (wx, wy) = (weights(px - ix), weights(py - iy));
        let (max_x, max_y) = (size[0] as i32 - 1, size[1] as i32 - 1);
        let outer = |i: usize| i == 0 || i == 3;
        let (mut sum, mut total) = (0.0, 0.0);
        for (j, wy) in wy.into_iter().enumerate() {
            let y = (iy as i32 + j as i32 - 1).clamp(0, max_y);
            for (i, wx) in wx.into_iter().enumerate() {
                if !corners && outer(i) && outer(j) {
                    continue;
                }
                let x = (ix as i32 + i as i32 - 1).clamp(0, max_x);
                sum += wx * wy * fetch(x, y);
                total += wx * wy;
            }
        }
        sum / total
    }

    pub fn weights(t: f32) -> [f32; 4] {
        let (t2, t3) = (t * t, t * t * t);
        [
            0.5 * (-t3 + 2.0 * t2 - t),
            0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
            0.5 * (-3.0 * t3 + 4.0 * t2 + t),
            0.5 * (t3 - t2),
        ]
    }

    fn fetch_luma(planes: &Planes, p: &ShaderParams, x: i32, y: i32) -> f32 {
        let width = p.sizes[0] as usize;
        let (x, y) = (x as usize, y as usize);
        match planes.layout {
            Layout::Nv12 => planes.first[y * width + x] as f32 / 255.0,
            Layout::Yuy2 => {
                let texel = (y * width.div_ceil(2) + x / 2) * 4;
                planes.first[texel + if x % 2 == 0 { 0 } else { 2 }] as f32 / 255.0
            }
            Layout::Ycbcr444 => planes.first[(y * width + x) * 3] as f32 / 255.0,
        }
    }

    fn fetch_chroma(planes: &Planes, p: &ShaderParams, x: i32, y: i32) -> [f32; 2] {
        let width = p.sizes[2] as usize;
        let (x, y) = (x as usize, y as usize);
        match planes.layout {
            Layout::Nv12 => {
                let at = (y * width + x) * 2;
                [planes.second[at] as f32 / 255.0, planes.second[at + 1] as f32 / 255.0]
            }
            Layout::Yuy2 => {
                let at = (y * width + x) * 4;
                [planes.first[at + 1] as f32 / 255.0, planes.first[at + 3] as f32 / 255.0]
            }
            Layout::Ycbcr444 => {
                let at = (y * width + x) * 3;
                [planes.first[at + 1] as f32 / 255.0, planes.first[at + 2] as f32 / 255.0]
            }
        }
    }
}

/// A small synthetic frame for GPU tests: colour bars over a luma ramp, so mapping and colour
/// mistakes both show. Returns (first plane, second plane) bytes for `layout`.
#[cfg(test)]
pub fn test_frame(width: u32, height: u32, layout: Layout) -> (Vec<u8>, Vec<u8>) {
    // BT.709 limited-range bars (white, red, green, blue, black) on the top half, ramp below.
    const BARS: [(u8, u8, u8); 5] = [(235, 128, 128), (63, 102, 240), (173, 42, 26), (32, 240, 118), (16, 128, 128)];
    let (w, h) = (width as usize, height as usize);
    let pixel = |x: usize, y: usize| -> (u8, u8, u8) {
        if y < h / 2 {
            BARS[(x * 5 / w).min(4)]
        } else {
            let ramp = 16 + (x * 219 / (w - 1)) as u8;
            (ramp, 128 + (y % 7) as u8 * 4, 128 - (x % 5) as u8 * 6)
        }
    };
    match layout {
        Layout::Nv12 => {
            let luma = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| pixel(x, y).0).collect();
            let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
            let chroma = (0..ch)
                .flat_map(|y| (0..cw).map(move |x| (x, y)))
                .flat_map(|(x, y)| {
                    let (_, cb, cr) = pixel(x * 2, y * 2);
                    [cb, cr]
                })
                .collect();
            (luma, chroma)
        }
        Layout::Yuy2 => {
            let packed = (0..h)
                .flat_map(|y| (0..w.div_ceil(2)).map(move |x| (x, y)))
                .flat_map(|(x, y)| {
                    let (y0, cb, cr) = pixel(x * 2, y);
                    let (y1, _, _) = pixel((x * 2 + 1).min(w - 1), y);
                    [y0, cb, y1, cr]
                })
                .collect();
            (packed, Vec::new())
        }
        Layout::Ycbcr444 => {
            let packed = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .flat_map(|(x, y)| {
                    let (l, cb, cr) = pixel(x, y);
                    [l, cb, cr]
                })
                .collect();
            (packed, Vec::new())
        }
    }
}

/// Shared by the MSL and HLSL GPU tests: the same cases, compared the same way against
/// [`reference`]. Each platform only supplies how to render on its GPU.
#[cfg(test)]
pub mod gpu_check {
    use super::*;

    /// Test frame size and output size: small, but with bars, a ramp and an off-ratio target.
    pub const FRAME: (u32, u32) = (96, 54);
    pub const OUTPUT: (u32, u32) = (160, 72);

    pub fn cases() -> Vec<(&'static str, PictureSettings)> {
        let base = PictureSettings { target_ratio: TargetRatio(160.0 / 72.0), ..Default::default() };
        vec![
            ("fit, auto colour", base),
            ("stretch, limited BT.709", PictureSettings { fill_mode: FillMode::Stretch, colour_range: Choice::Manual(ColourRange::Limited), ..base }),
            ("crop, BT.601", PictureSettings { fill_mode: FillMode::Crop, colour_space: Choice::Manual(ColourSpace::Bt601), ..base }),
            ("smart 0%", PictureSettings { fill_mode: FillMode::SmartStretch, smart_stretch_strength: 0.0, ..base }),
            ("smart 50%", PictureSettings { fill_mode: FillMode::SmartStretch, smart_stretch_strength: 0.5, ..base }),
            ("smart 100%", PictureSettings { fill_mode: FillMode::SmartStretch, smart_stretch_strength: 1.0, ..base }),
            ("smart narrower (4:3)", PictureSettings { fill_mode: FillMode::SmartStretch, smart_stretch_strength: 1.0, target_ratio: TargetRatio(4.0 / 3.0), ..base }),
            (
                "adjustments",
                PictureSettings { adjustments: Adjustments { brightness: 0.1, contrast: 1.2, saturation: 1.4, gamma: 1.3, hue: 25.0 }, ..base },
            ),
        ]
    }

    /// Renders every case on the GPU via `gpu` (planes, plan → RGB image of [`OUTPUT`] size) and
    /// asserts it matches the CPU reference within 3/255 per channel.
    pub fn assert_all_match(layout: Layout, gpu: impl Fn(&reference::Planes, &Plan) -> Vec<[f32; 3]>) {
        let (w, h) = FRAME;
        let (first, second) = test_frame(w, h, layout);
        let planes = reference::Planes { layout, first: &first, second: &second };
        for (what, settings) in cases() {
            let info = FrameInfo { width: w, height: h, layout, os_squeezed: false };
            let plan = plan(info, &settings, (OUTPUT.0 as f64, OUTPUT.1 as f64));
            let gpu_image = gpu(&planes, &plan);
            let cpu_image = reference::render(&planes, &plan, OUTPUT.0, OUTPUT.1);
            let mut worst = (0.0f32, 0usize);
            for (i, (g, c)) in gpu_image.iter().zip(&cpu_image).enumerate() {
                for ch in 0..3 {
                    let d = (g[ch] - c[ch]).abs();
                    if d > worst.0 {
                        worst = (d, i);
                    }
                }
            }
            assert!(
                worst.0 <= 3.0 / 255.0,
                "{layout:?}, {what}: GPU differs from the reference by {:.1}/255 at pixel {} (gpu {:?}, cpu {:?})",
                worst.0 * 255.0,
                worst.1,
                gpu_image[worst.1],
                cpu_image[worst.1]
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32) -> FrameInfo {
        FrameInfo { width, height, layout: Layout::Nv12, os_squeezed: false }
    }

    fn apply(m: &[[f32; 4]; 3], ycc: [f32; 3]) -> [f32; 3] {
        m.map(|r| r[0] * ycc[0] + r[1] * ycc[1] + r[2] * ycc[2] + r[3])
    }

    fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tolerance)
    }

    fn bytes(y: u8, cb: u8, cr: u8) -> [f32; 3] {
        [y as f32 / 255.0, cb as f32 / 255.0, cr as f32 / 255.0]
    }

    #[test]
    fn auto_rules() {
        assert_eq!(resolve_range(Choice::Auto), ColourRange::Full);
        assert_eq!(resolve_space(Choice::Auto, 720), ColourSpace::Bt709);
        assert_eq!(resolve_space(Choice::Auto, 480), ColourSpace::Bt601);
        assert_eq!(resolve_space(Choice::Manual(ColourSpace::Bt601), 1080), ColourSpace::Bt601);
    }

    #[test]
    fn limited_bt709_bars_become_pure_colours() {
        let settings = PictureSettings { colour_range: Choice::Manual(ColourRange::Limited), ..Default::default() };
        let m = plan(frame(1280, 720), &settings, (1280.0, 720.0)).params.colour;
        for (ycc, rgb) in [
            (bytes(235, 128, 128), [1.0, 1.0, 1.0]),
            (bytes(63, 102, 240), [1.0, 0.0, 0.0]),
            (bytes(173, 42, 26), [0.0, 1.0, 0.0]),
            (bytes(32, 240, 118), [0.0, 0.0, 1.0]),
            (bytes(16, 128, 128), [0.0, 0.0, 0.0]),
        ] {
            let got = apply(&m, ycc);
            assert!(close(got, rgb, 0.02), "{ycc:?} → {got:?}, expected {rgb:?}");
        }
    }

    #[test]
    fn full_range_bars_become_pure_colours() {
        let m = plan(frame(1280, 720), &PictureSettings::default(), (1280.0, 720.0)).params.colour;
        for (ycc, rgb) in [(bytes(255, 128, 128), [1.0, 1.0, 1.0]), (bytes(54, 98, 255), [1.0, 0.0, 0.0]), (bytes(0, 128, 128), [0.0, 0.0, 0.0])] {
            let got = apply(&m, ycc);
            assert!(close(got, rgb, 0.02), "{ycc:?} → {got:?}, expected {rgb:?}");
        }
    }

    #[test]
    fn os_squeeze_is_undone_before_the_source_range() {
        // macOS squeezed a full-range white (255) to 235: with Auto (Full) it must be white again.
        let info = FrameInfo { os_squeezed: true, ..frame(1280, 720) };
        let m = plan(info, &PictureSettings::default(), (1280.0, 720.0)).params.colour;
        assert!(close(apply(&m, bytes(235, 128, 128)), [1.0, 1.0, 1.0], 0.01));
        assert!(close(apply(&m, bytes(16, 128, 128)), [0.0, 0.0, 0.0], 0.01));
    }

    #[test]
    fn adjustments_act_as_named() {
        let grey = bytes(128, 128, 128);
        let with = |adjustments| {
            let s = PictureSettings { adjustments, ..Default::default() };
            apply(&plan(frame(1280, 720), &s, (1280.0, 720.0)).params.colour, grey)
        };
        let neutral = with(Adjustments::default());
        assert!(with(Adjustments { brightness: 0.2, ..Default::default() })[0] > neutral[0] + 0.15);
        let red = bytes(63, 102, 240);
        let s = PictureSettings { adjustments: saturation_zero(), ..Default::default() };
        let m = plan(frame(1280, 720), &s, (1280.0, 720.0)).params.colour;
        let grey_red = apply(&m, red);
        assert!(close([grey_red[0]; 3], grey_red, 0.001), "saturation 0 is grey: {grey_red:?}");
    }

    fn saturation_zero() -> Adjustments {
        Adjustments { saturation: 0.0, ..Default::default() }
    }

    #[test]
    fn hue_rotation_keeps_luma_and_moves_colour() {
        let red = bytes(63, 102, 240);
        let s = PictureSettings { adjustments: Adjustments { hue: 120.0, ..Default::default() }, ..Default::default() };
        let rotated = apply(&plan(frame(1280, 720), &s, (1280.0, 720.0)).params.colour, red);
        assert!(rotated[1] > rotated[0], "red rotated by 120° leans green: {rotated:?}");
    }

    #[test]
    fn fit_crop_stretch_geometry() {
        // 16:9 Source, 21:9 surface, Target ratio 21:9.
        let settings = |fill_mode| PictureSettings { fill_mode, target_ratio: TargetRatio(64.0 / 27.0), ..Default::default() };
        let surface = (2560.0, 1080.0);
        let fit = plan(frame(1920, 1080), &settings(FillMode::Fit), surface);
        assert_eq!((fit.viewport.width, fit.viewport.height), (1920.0, 1080.0));
        assert_eq!(fit.viewport.x, 320.0);
        let stretch = plan(frame(1920, 1080), &settings(FillMode::Stretch), surface);
        assert_eq!((stretch.viewport.width, stretch.viewport.height), (2560.0, 1080.0));
        let crop = plan(frame(1920, 1080), &settings(FillMode::Crop), surface);
        assert_eq!(crop.viewport, stretch.viewport);
        let visible = crop.params.mapping[3];
        assert!((visible - 0.75).abs() < 1e-6, "crop shows 3/4 of the height: {visible}");
        assert!((crop.params.mapping[1] - 0.125).abs() < 1e-6);
    }

    #[test]
    fn target_ratio_narrower_than_the_screen_leaves_side_bars() {
        let settings = PictureSettings { fill_mode: FillMode::Stretch, target_ratio: TargetRatio(4.0 / 3.0), ..Default::default() };
        let p = plan(frame(1920, 1080), &settings, (1920.0, 1080.0));
        assert_eq!((p.viewport.x, p.viewport.width, p.viewport.height), (240.0, 1440.0, 1080.0));
    }

    #[test]
    fn smart_stretch_curve() {
        for k in [64.0 / 27.0 / (16.0 / 9.0), 2.0, 0.75] {
            let p = smart_exponent(k);
            for t in [0.0, 0.5, 1.0] {
                assert!((smart_curve(0.0, k, t, p)).abs() < 1e-12);
                assert!((smart_curve(1.0, k, t, p) - 1.0).abs() < 1e-12, "edges map to edges");
                let mut last = 0.0;
                for i in 1..=100 {
                    let g = smart_curve(i as f64 / 100.0, k, t, p);
                    assert!(g >= last, "monotonic for k={k} t={t}");
                    last = g;
                }
            }
            // t = 0 is plain Stretch; t = 1 keeps the centre undistorted (slope k).
            assert!((smart_curve(0.3, k, 0.0, p) - 0.3).abs() < 1e-12);
            let slope = smart_curve(1e-6, k, 1.0, p) / 1e-6;
            assert!((slope - k).abs() < 1e-3);
        }
    }

    #[test]
    fn catmull_rom_weights_interpolate() {
        for t in [0.0, 0.25, 0.5, 0.9] {
            let w = reference::weights(t);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        }
        assert_eq!(reference::weights(0.0), [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn reference_renders_bars_through_fit() {
        let (w, h) = (100, 40);
        let (first, second) = test_frame(w, h, Layout::Nv12);
        let info = FrameInfo { width: w, height: h, layout: Layout::Nv12, os_squeezed: false };
        let settings = PictureSettings {
            colour_range: Choice::Manual(ColourRange::Limited),
            colour_space: Choice::Manual(ColourSpace::Bt709),
            target_ratio: TargetRatio::custom(100.0, 40.0),
            ..Default::default()
        };
        let p = plan(info, &settings, (100.0, 40.0));
        let image = reference::render(&reference::Planes { layout: Layout::Nv12, first: &first, second: &second }, &p, 100, 40);
        let at = |x: u32, y: u32| image[(y * 100 + x) as usize];
        assert!(close(at(10, 5), [1.0, 1.0, 1.0], 0.03), "white bar: {:?}", at(10, 5));
        assert!(close(at(70, 5), [0.0, 0.0, 1.0], 0.03), "blue bar: {:?}", at(70, 5));
    }
}
