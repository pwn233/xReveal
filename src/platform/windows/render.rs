//! The video thread on Windows: xReveal's own Direct3D 11 device draws the newest frame into a
//! flip-model, waitable swap chain held to one frame of latency, presenting on arrival with vsync
//! (docs/spec/v1.md §3.3, §3.4; ADR 0001).

use std::sync::atomic::Ordering;

use windows::Win32::Foundation::{HANDLE, HMODULE};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::WaitForSingleObjectEx;
use windows::core::{Interface, PCSTR, Result, s};

use super::media::{Frame, PixelFormat};
use super::surface::Surface;
use crate::video::fill;
use crate::video::slot::FrameSlot;

const SHADER: &str = include_str!("video.hlsl");
const SWAP_CHAIN_FLAGS: DXGI_SWAP_CHAIN_FLAG = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT;

/// Runs until the slot is closed. Presents one black frame first, so the surface is proven working
/// even with no card (the CI smoke launch checks [`crate::video::FIRST_PRESENT`]).
pub fn run(surface: Surface, slot: &FrameSlot<Frame>) {
    let mut renderer = match Renderer::new(surface) {
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
    crate::video::FIRST_PRESENT.store(true, Ordering::SeqCst);

    let mut reported = false;
    while let Some(frame) = slot.take() {
        if let Err(error) = renderer.draw(Some(&frame))
            && !reported
        {
            eprintln!("video: {error}");
            reported = true;
        }
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
    vertex_shader: ID3D11VertexShader,
    nv12_shader: ID3D11PixelShader,
    yuy2_shader: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    params: ID3D11Buffer,
    planes: Option<Planes>,
}

/// Upload textures for the current frame size and pixel format.
struct Planes {
    key: (PixelFormat, u32, u32),
    textures: Vec<ID3D11Texture2D>,
    views: Vec<Option<ID3D11ShaderResourceView>>,
}

#[repr(C)]
struct Params {
    video_range: u32,
    picture_width: u32,
    picture_height: u32,
    pad: u32,
}

impl Renderer {
    fn new(surface: Surface) -> Result<Self> {
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

            let vertex_bytes = compile(s!("vs_main"), s!("vs_4_0"))?;
            let mut vertex_shader = None;
            device.CreateVertexShader(&vertex_bytes, None, Some(&mut vertex_shader))?;
            let nv12_shader = pixel_shader(&device, s!("ps_nv12"))?;
            let yuy2_shader = pixel_shader(&device, s!("ps_yuy2"))?;

            let mut sampler = None;
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    MaxLOD: f32::MAX,
                    ..Default::default()
                },
                Some(&mut sampler),
            )?;
            let mut params = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: size_of::<Params>() as u32,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut params),
            )?;

            Ok(Self {
                surface,
                device,
                context,
                swap_chain,
                frame_latency,
                target: None,
                size: (desc.Width, desc.Height),
                vertex_shader: vertex_shader.expect("created"),
                nv12_shader,
                yuy2_shader,
                sampler: sampler.expect("created"),
                params: params.expect("created"),
                planes: None,
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
            self.context.ClearRenderTargetView(&target, &[0.0, 0.0, 0.0, 1.0]);

            if let Some(frame) = frame {
                self.upload(frame)?;
                let f = frame.format;
                let rect = fill::fit(f.width as f64, f.height as f64, self.size.0 as f64, self.size.1 as f64);
                self.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                    TopLeftX: rect.x as f32,
                    TopLeftY: rect.y as f32,
                    Width: rect.width as f32,
                    Height: rect.height as f32,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]));
                // Windows passes the card's levels through untouched; Auto Colour range = Full
                // (docs/spec/v1.md §2.6). Range/space settings arrive with the full shader.
                self.write_params(&Params { video_range: 0, picture_width: f.width, picture_height: f.height, pad: 0 })?;
                let planes = self.planes.as_ref().expect("uploaded above");
                self.context.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
                self.context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                self.context.VSSetShader(&self.vertex_shader, None);
                let shader = match f.pixel_format {
                    PixelFormat::Nv12 => &self.nv12_shader,
                    PixelFormat::Yuy2 => &self.yuy2_shader,
                };
                self.context.PSSetShader(shader, None);
                let views = match f.pixel_format {
                    PixelFormat::Nv12 => [planes.views[0].clone(), planes.views[1].clone(), None],
                    PixelFormat::Yuy2 => [None, None, planes.views[0].clone()],
                };
                self.context.PSSetShaderResources(0, Some(&views));
                self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
                self.context.PSSetConstantBuffers(0, Some(&[Some(self.params.clone())]));
                self.context.Draw(3, 0);
            }
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
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
    fn upload(&mut self, frame: &Frame) -> Result<()> {
        let f = frame.format;
        let key = (f.pixel_format, f.width, f.height);
        if self.planes.as_ref().map(|p| p.key) != Some(key) {
            self.planes = Some(self.create_planes(key)?);
        }
        let planes = self.planes.as_ref().expect("created above");
        let context = &self.context;
        let (w, h) = (f.width as usize, f.height as usize);
        frame.with_pixels(|bytes, pitch| -> Result<()> {
            match f.pixel_format {
                PixelFormat::Nv12 => {
                    write_rows(context, &planes.textures[0], &bytes[..pitch * h], pitch, w, h)?;
                    write_rows(context, &planes.textures[1], &bytes[pitch * h..], pitch, w, h.div_ceil(2))
                }
                PixelFormat::Yuy2 => write_rows(context, &planes.textures[0], bytes, pitch, w * 2, h),
            }
        })?
    }

    fn create_planes(&self, key: (PixelFormat, u32, u32)) -> Result<Planes> {
        let (format, w, h) = key;
        let layout: &[(DXGI_FORMAT, u32, u32)] = match format {
            PixelFormat::Nv12 => &[(DXGI_FORMAT_R8_UNORM, w, h), (DXGI_FORMAT_R8G8_UNORM, w.div_ceil(2), h.div_ceil(2))],
            PixelFormat::Yuy2 => &[(DXGI_FORMAT_R8G8B8A8_UNORM, w.div_ceil(2), h)],
        };
        let mut textures = Vec::new();
        let mut views = Vec::new();
        for &(format, width, height) in layout {
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
            let mut texture = None;
            let mut view = None;
            unsafe {
                self.device.CreateTexture2D(&desc, None, Some(&mut texture))?;
                let texture = texture.expect("created");
                self.device.CreateShaderResourceView(&texture, None, Some(&mut view))?;
                textures.push(texture);
            }
            views.push(view);
        }
        Ok(Planes { key, textures, views })
    }

    fn write_params(&self, params: &Params) -> Result<()> {
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&self.params, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(params as *const Params as *const u8, mapped.pData as *mut u8, size_of::<Params>());
            self.context.Unmap(&self.params, 0);
        }
        Ok(())
    }
}

/// Copies `rows` rows of `row_bytes` from `source` (with `pitch`) into a dynamic texture.
fn write_rows(
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
    let levels = [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0];
    let mut last_error = None;
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        let mut context = None;
        let result = unsafe {
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
            )
        };
        match (result, device, context) {
            (Ok(()), Some(device), Some(context)) => return Ok((device, context)),
            (Err(error), _, _) => last_error = Some(error),
            _ => {}
        }
    }
    Err(last_error.unwrap_or_else(windows::core::Error::empty))
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

fn pixel_shader(device: &ID3D11Device, entry: PCSTR) -> Result<ID3D11PixelShader> {
    let bytes = compile(entry, s!("ps_4_0"))?;
    let mut shader = None;
    unsafe { device.CreatePixelShader(&bytes, None, Some(&mut shader))? };
    Ok(shader.expect("created"))
}
