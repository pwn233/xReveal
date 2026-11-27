//! The video thread on Windows: xReveal's own Direct3D 11 device draws the newest frame, in one
//! shader pass planned by `video::picture`, into a flip-model waitable swap chain held to one frame
//! of latency, presenting on arrival with vsync (docs/spec/v1.md §3.3, §3.4; ADR 0001).

use std::collections::VecDeque;
use std::sync::Arc;

use windows::Win32::Foundation::{HANDLE, HMODULE};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::WaitForSingleObjectEx;
use windows::core::{Interface, PCSTR, Result, s};

use windows::Win32::Media::MediaFoundation::MF_E_INVALIDMEDIATYPE;

use super::media::{Frame, PixelFormat};
use crate::video::flat;
use crate::video::mjpeg::MjpegDecoder;
use super::surface::Surface;
use crate::video::picture::{self, FrameInfo, Layout, Plan, ShaderParams};
use crate::video::timing::{self, Stamps};
use crate::video::{VideoShared, lock};
use crate::video::slot::FrameSlot;

const SHADER: &str = include_str!("video.hlsl");
const SWAP_CHAIN_FLAGS: DXGI_SWAP_CHAIN_FLAG = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT;

/// Runs until the slot is closed. Presents one black frame first, so the surface is proven working
/// even with no card (the CI smoke launch checks [`crate::video::FIRST_PRESENT`]).
pub fn run(surface: Surface, slot: &FrameSlot<Frame>, shared: Arc<VideoShared>) {
    let mut renderer = match Renderer::new(surface, shared.clone()) {
        Ok(renderer) => renderer,
        Err(error) => {
            eprintln!("video: {error}");
            return;
        }
    };
    if let Err(error) = renderer.draw(None) {
        eprintln!("video: first present failed: {error}");
        return;
    }
    crate::video::mark_first_present();

    let mut reported = false;
    timing::run_video_loop(slot, &shared.delay, &shared.stats, super::media::host_seconds, |frame| {
        if let Err(error) = renderer.draw(Some(&frame))
            && !reported
        {
            eprintln!("video: {error}");
            reported = true;
        }
    });
}

fn layout(format: PixelFormat) -> Layout {
    match format {
        PixelFormat::Nv12 => Layout::Nv12,
        PixelFormat::Yuy2 => Layout::Yuy2,
        PixelFormat::Mjpeg => Layout::Ycbcr444,
    }
}

/// The compiled video shader and its uniforms, ready to draw a planned frame into any target.
pub struct VideoPass {
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    params: ID3D11Buffer,
    // Culling off: Direct3D culls counter-clockwise triangles by default, and the fullscreen
    // triangle's winding is counter-clockwise (Metal doesn't cull, so the shaders share it).
    rasterizer: ID3D11RasterizerState,
    /// Linear filtering, clamped at the edges: the NV12 path's 5-fetch Catmull-Rom (#56, #118).
    bilinear: ID3D11SamplerState,
}

impl VideoPass {
    pub fn new(device: &ID3D11Device) -> Result<Self> {
        unsafe {
            let vertex_bytes = compile(s!("vs_main"), s!("vs_4_0"))?;
            let pixel_bytes = compile(s!("ps_main"), s!("ps_4_0"))?;
            let (mut vertex_shader, mut pixel_shader, mut params, mut rasterizer, mut bilinear) = (None, None, None, None, None);
            device.CreateVertexShader(&vertex_bytes, None, Some(&mut vertex_shader))?;
            device.CreatePixelShader(&pixel_bytes, None, Some(&mut pixel_shader))?;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: size_of::<ShaderParams>() as u32,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut params),
            )?;
            device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    DepthClipEnable: true.into(),
                    ..Default::default()
                },
                Some(&mut rasterizer),
            )?;
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                },
                Some(&mut bilinear),
            )?;
            Ok(Self {
                vertex_shader: vertex_shader.expect("created"),
                pixel_shader: pixel_shader.expect("created"),
                params: params.expect("created"),
                rasterizer: rasterizer.expect("created"),
                bilinear: bilinear.expect("created"),
            })
        }
    }

    /// Clears `target` to black and draws `planes` through `plan`.
    pub fn encode(&self, context: &ID3D11DeviceContext, target: &ID3D11RenderTargetView, planes: &Planes, plan: &Plan) -> Result<()> {
        unsafe {
            context.ClearRenderTargetView(target, &[0.0, 0.0, 0.0, 1.0]);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&self.params, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(
                &plan.params as *const ShaderParams as *const u8,
                mapped.pData as *mut u8,
                size_of::<ShaderParams>(),
            );
            context.Unmap(&self.params, 0);

            let vp = plan.viewport;
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: vp.x as f32,
                TopLeftY: vp.y as f32,
                Width: vp.width as f32,
                Height: vp.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            context.RSSetState(&self.rasterizer);
            context.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            context.VSSetShader(&self.vertex_shader, None);
            context.PSSetShader(&self.pixel_shader, None);
            let second = planes.views.get(1).unwrap_or(&planes.views[0]);
            context.PSSetShaderResources(0, Some(&[planes.views[0].clone(), second.clone()]));
            context.PSSetConstantBuffers(0, Some(&[Some(self.params.clone())]));
            context.PSSetSamplers(0, Some(&[Some(self.bilinear.clone())]));
            context.Draw(3, 0);
        }
        Ok(())
    }
}

/// Upload textures for one frame size and layout: NV12 = luma (R8) + CbCr (R8G8);
/// YUY2 = one packed RGBA8 texture at half width.
pub struct Planes {
    key: (Layout, u32, u32),
    pub textures: Vec<ID3D11Texture2D>,
    views: Vec<Option<ID3D11ShaderResourceView>>,
}

impl Planes {
    pub fn new(device: &ID3D11Device, layout: Layout, w: u32, h: u32) -> Result<Self> {
        let specs: &[(DXGI_FORMAT, u32, u32)] = match layout {
            Layout::Nv12 => &[(DXGI_FORMAT_R8_UNORM, w, h), (DXGI_FORMAT_R8G8_UNORM, w.div_ceil(2), h.div_ceil(2))],
            Layout::Yuy2 => &[(DXGI_FORMAT_R8G8B8A8_UNORM, w.div_ceil(2), h)],
            Layout::Ycbcr444 => &[(DXGI_FORMAT_R8_UNORM, w * 3, h)],
        };
        let mut textures = Vec::new();
        let mut views = Vec::new();
        for &(format, width, height) in specs {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: format,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                MiscFlags: 0,
            };
            let (mut texture, mut view) = (None, None);
            unsafe {
                device.CreateTexture2D(&desc, None, Some(&mut texture))?;
                let texture = texture.expect("created");
                device.CreateShaderResourceView(&texture, None, Some(&mut view))?;
                textures.push(texture);
            }
            views.push(view);
        }
        Ok(Self { key: (layout, w, h), textures, views })
    }
}

struct Renderer {
    surface: Surface,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain2,
    frame_latency: HANDLE,
    target: Option<ID3D11RenderTargetView>,
    size: (u32, u32),
    pass: VideoPass,
    planes: Option<Planes>,
    shared: Arc<VideoShared>,
    mjpeg: MjpegDecoder,
    /// Presents not yet seen in the frame statistics: (present count, stamps).
    pending: VecDeque<(u32, Stamps)>,
}

impl Renderer {
    fn new(surface: Surface, shared: Arc<VideoShared>) -> Result<Self> {
        unsafe {
            let (device, context) = create_device()?;
            let factory: IDXGIFactory2 = device.cast::<IDXGIDevice>()?.GetAdapter()?.GetParent()?;
            let size = surface.parent_size();
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: size.0.max(1),
                Height: size.1.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: SWAP_CHAIN_FLAGS.0 as u32,
                ..Default::default()
            };
            let swap_chain: IDXGISwapChain2 =
                factory.CreateSwapChainForHwnd(&device, surface.child, &desc, None, None)?.cast()?;
            let _ = factory.MakeWindowAssociation(surface.child, DXGI_MWA_NO_ALT_ENTER);
            // At most one frame in flight (docs/spec/v1.md §3.3).
            swap_chain.SetMaximumFrameLatency(1)?;
            let frame_latency = swap_chain.GetFrameLatencyWaitableObject();
            let pass = VideoPass::new(&device)?;
            Ok(Self {
                surface,
                device,
                context,
                swap_chain,
                frame_latency,
                target: None,
                size: (desc.Width, desc.Height),
                pass,
                planes: None,
                shared,
                mjpeg: MjpegDecoder::default(),
                pending: VecDeque::new(),
            })
        }
    }

    /// Draws `frame` (or just black) and presents it at the next refresh.
    fn draw(&mut self, frame: Option<&Frame>) -> Result<()> {
        unsafe {
            WaitForSingleObjectEx(self.frame_latency, 1000, true);
            self.follow_window_size()?;
            if self.target.is_none() {
                let back_buffer: ID3D11Texture2D = self.swap_chain.GetBuffer(0)?;
                let mut target = None;
                self.device.CreateRenderTargetView(&back_buffer, None, Some(&mut target))?;
                self.target = target;
            }
            let target = self.target.clone().expect("created above");
            match frame {
                Some(frame) => {
                    let (width, height) = self.upload(frame)?;
                    // Windows passes the card's levels through untouched: nothing to undo.
                    let info = FrameInfo { width, height, layout: layout(frame.format.pixel_format), os_squeezed: false };
                    let settings = *lock(&self.shared.picture);
                    let plan = picture::plan(info, &settings, (self.size.0 as f64, self.size.1 as f64));
                    self.pass.encode(&self.context, &target, self.planes.as_ref().expect("uploaded"), &plan)?;
                }
                None => self.context.ClearRenderTargetView(&target, &[0.0, 0.0, 0.0, 1.0]),
            }
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
            if let Some(frame) = frame {
                self.record_present(frame.stamps);
            }
            Ok(())
        }
    }

    /// Matches presents to DXGI's frame statistics (the refresh each present reached the screen,
    /// in QPC time). Where Windows gives no statistics (e.g. a composed window), records the time
    /// of the Present call instead, marked as estimated.
    fn record_present(&mut self, stamps: Stamps) {
        let mut stats = lock(&self.shared.stats);
        let Ok(count) = (unsafe { self.swap_chain.GetLastPresentCount() }) else {
            stats.presented(stamps, super::media::host_seconds(), true);
            return;
        };
        self.pending.push_back((count, stamps));
        let mut frame_stats = DXGI_FRAME_STATISTICS::default();
        if unsafe { self.swap_chain.GetFrameStatistics(&mut frame_stats) }.is_err() {
            // No statistics: report this present now, as estimated.
            self.pending.clear();
            stats.presented(stamps, super::media::host_seconds(), true);
            return;
        }
        let frequency = {
            let mut f = 0i64;
            let _ = unsafe { windows::Win32::System::Performance::QueryPerformanceFrequency(&mut f) };
            f.max(1) as f64
        };
        let displayed_at = frame_stats.SyncQPCTime as f64 / frequency;
        while let Some(&(id, earlier)) = self.pending.front() {
            if id > frame_stats.PresentCount {
                break;
            }
            self.pending.pop_front();
            if id == frame_stats.PresentCount {
                stats.presented(earlier, displayed_at, false);
            }
        }
        // Statistics that stop advancing mustn't grow this without bound.
        while self.pending.len() > 8 {
            self.pending.pop_front();
        }
    }

    /// Keeps the child window and swap chain the size of GPUI's window.
    fn follow_window_size(&mut self) -> Result<()> {
        let size = self.surface.parent_size();
        if size == self.size || size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        self.surface.resize(size);
        self.target = None;
        unsafe {
            self.context.OMSetRenderTargets(None, None);
            self.swap_chain.ResizeBuffers(0, size.0, size.1, DXGI_FORMAT_UNKNOWN, SWAP_CHAIN_FLAGS)?;
        }
        self.size = size;
        Ok(())
    }

    /// Copies the frame's planes into the upload textures: the one CPU→GPU copy per frame.
    /// MJPEG frames are decoded first (only the newest frame ever reaches here). Returns the
    /// picture size actually uploaded.
    fn upload(&mut self, frame: &Frame) -> Result<(u32, u32)> {
        let f = frame.format;
        let (device, planes, mjpeg) = (&self.device, &mut self.planes, &mut self.mjpeg);
        let context = &self.context;
        // A few luma samples, to notice a picture that stays one flat colour (#108).
        let observe = |samples: Vec<u8>| lock(&self.shared.flat).observe(&samples, std::time::Instant::now());
        frame.with_pixels(|bytes, pitch| -> Result<(u32, u32)> {
            if f.pixel_format == PixelFormat::Mjpeg {
                let decoded = mjpeg.decode(bytes).map_err(|e| windows::core::Error::new(MF_E_INVALIDMEDIATYPE, e))?;
                let (w, h) = (decoded.width, decoded.height);
                let planes = ensure_planes(device, planes, Layout::Ycbcr444, w, h)?;
                let row = w as usize * 3;
                write_rows(context, &planes.textures[0], decoded.ycbcr, row, row, h as usize)?;
                observe(flat::luma_grid(w as usize, h as usize, |x, y| decoded.ycbcr[y * row + x * 3]));
                return Ok((w, h));
            }
            let planes = ensure_planes(device, planes, layout(f.pixel_format), f.width, f.height)?;
            let (w, h) = (f.width as usize, f.height as usize);
            match f.pixel_format {
                PixelFormat::Nv12 => {
                    write_rows(context, &planes.textures[0], &bytes[..pitch * h], pitch, w, h)?;
                    write_rows(context, &planes.textures[1], &bytes[pitch * h..], pitch, w, h.div_ceil(2))?;
                    observe(flat::luma_grid(w, h, |x, y| bytes[y * pitch + x]));
                }
                PixelFormat::Yuy2 => {
                    write_rows(context, &planes.textures[0], bytes, pitch, w * 2, h)?;
                    observe(flat::luma_grid(w, h, |x, y| bytes[y * pitch + x * 2]));
                }
                PixelFormat::Mjpeg => unreachable!("handled above"),
            }
            Ok((f.width, f.height))
        })?
    }
}

/// The upload textures for `layout` at `w`×`h`, recreated only when that changes.
fn ensure_planes<'a>(device: &ID3D11Device, planes: &'a mut Option<Planes>, layout: Layout, w: u32, h: u32) -> Result<&'a Planes> {
    if planes.as_ref().map(|p| p.key) != Some((layout, w, h)) {
        *planes = Some(Planes::new(device, layout, w, h)?);
    }
    Ok(planes.as_ref().expect("set above"))
}

/// Copies `rows` rows of `row_bytes` from `source` (with `pitch`) into a dynamic texture.
pub fn write_rows(
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    source: &[u8],
    pitch: usize,
    row_bytes: usize,
    rows: usize,
) -> Result<()> {
    unsafe {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(texture, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
        let destination = mapped.pData as *mut u8;
        for row in 0..rows {
            let from = &source[row * pitch..row * pitch + row_bytes];
            std::ptr::copy_nonoverlapping(from.as_ptr(), destination.add(row * mapped.RowPitch as usize), row_bytes);
        }
        context.Unmap(texture, 0);
    }
    Ok(())
}

/// A hardware device, or Microsoft's software renderer (WARP) where there is no GPU, e.g. on CI.
fn create_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut last_error = None;
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        match create_device_of(driver) {
            Ok(pair) => return Ok(pair),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(windows::core::Error::empty))
}

fn create_device_of(driver: D3D_DRIVER_TYPE) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let levels = [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0];
    let (mut device, mut context) = (None, None);
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(windows::core::Error::empty()),
    }
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<Vec<u8>> {
    unsafe {
        let mut blob = None;
        let mut errors = None;
        let result = D3DCompile(
            SHADER.as_ptr() as *const _,
            SHADER.len(),
            s!("video.hlsl"),
            None,
            None,
            entry,
            target,
            0,
            0,
            &mut blob,
            Some(&mut errors),
        );
        if let Err(error) = result {
            let message = errors
                .map(|e| String::from_utf8_lossy(std::slice::from_raw_parts(e.GetBufferPointer() as *const u8, e.GetBufferSize())).into_owned())
                .unwrap_or_default();
            eprintln!("video: shader compile failed: {message}");
            return Err(error);
        }
        let blob = blob.expect("compiled");
        Ok(std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()).to_vec())
    }
}

#[cfg(test)]
mod gpu_tests {
    //! The HLSL shader against the CPU reference (`video::picture::gpu_check`), offscreen on WARP.
    use super::*;
    use crate::video::picture::{gpu_check, reference};

    fn render(device: &ID3D11Device, context: &ID3D11DeviceContext, pass: &VideoPass, src: &reference::Planes, plan: &Plan) -> Vec<[f32; 3]> {
        unsafe {
            let (w, h) = gpu_check::FRAME;
            let planes = Planes::new(device, src.layout, w, h).expect("planes");
            match src.layout {
                Layout::Ycbcr444 => {
                    let row = w as usize * 3;
                    write_rows(context, &planes.textures[0], src.first, row, row, h as usize).unwrap();
                }
                Layout::Nv12 => {
                    write_rows(context, &planes.textures[0], src.first, w as usize, w as usize, h as usize).unwrap();
                    let row = w.div_ceil(2) as usize * 2;
                    write_rows(context, &planes.textures[1], src.second, row, row, h.div_ceil(2) as usize).unwrap();
                }
                Layout::Yuy2 => {
                    let row = w.div_ceil(2) as usize * 4;
                    write_rows(context, &planes.textures[0], src.first, row, row, h as usize).unwrap();
                }
            }
            let (out_w, out_h) = gpu_check::OUTPUT;
            let mut desc = D3D11_TEXTURE2D_DESC {
                Width: out_w,
                Height: out_h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut target = None;
            device.CreateTexture2D(&desc, None, Some(&mut target)).unwrap();
            let target = target.unwrap();
            let mut view = None;
            device.CreateRenderTargetView(&target, None, Some(&mut view)).unwrap();
            pass.encode(context, &view.unwrap(), &planes, plan).unwrap();

            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            device.CreateTexture2D(&desc, None, Some(&mut staging)).unwrap();
            let staging = staging.unwrap();
            context.CopyResource(&staging, &target);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).unwrap();
            let mut image = Vec::with_capacity((out_w * out_h) as usize);
            for y in 0..out_h as usize {
                let row = std::slice::from_raw_parts((mapped.pData as *const u8).add(y * mapped.RowPitch as usize), out_w as usize * 4);
                image.extend(row.as_chunks::<4>().0.iter().map(|p| [p[2] as f32 / 255.0, p[1] as f32 / 255.0, p[0] as f32 / 255.0]));
            }
            context.Unmap(&staging, 0);
            image
        }
    }

    #[test]
    fn hlsl_matches_reference() {
        let (device, context) = create_device_of(D3D_DRIVER_TYPE_WARP).expect("WARP device");
        let pass = VideoPass::new(&device).expect("video pass");
        for layout in [Layout::Nv12, Layout::Yuy2, Layout::Ycbcr444] {
            gpu_check::assert_all_match(layout, |planes, plan| render(&device, &context, &pass, planes, plan));
        }
    }
}
