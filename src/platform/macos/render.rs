//! The video thread on macOS: takes the newest frame, wraps its planes as Metal textures with no
//! copy (made once per capture buffer from its IOSurface), draws them in one shader pass planned by
//! `video::picture`, and presents on arrival (docs/spec/v1.md §3.3, §3.4).

use std::ptr::NonNull;
use std::sync::Arc;

use block2::RcBlock;

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFRetained, CGSize};
use objc2_io_surface::IOSurfaceRef;
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetIOSurface,
    CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRow, CVPixelBufferGetBytesPerRowOfPlane,
    CVPixelBufferGetHeightOfPlane, CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidthOfPlane, CVPixelBufferIsPlanar,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVReturnSuccess,
};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLDevice, MTLLibrary, MTLLoadAction,
    MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
};
use objc2_metal::MTLDrawable;
use objc2_quartz_core::{CACurrentMediaTime, CAMetalDrawable};

use super::capture::{PixelBuffer, fourcc};
use super::surface::RenderTarget;
use crate::video::flat;
use crate::video::picture::{self, FrameInfo, Layout, Plan, ShaderParams};
use crate::video::{VideoShared, lock, timing};
use crate::video::slot::FrameSlot;

const SHADER: &str = include_str!("video.metal");

/// `420v`: bi-planar 4:2:0, video range (macOS squeezed the card's levels into it).
const NV12_VIDEO_RANGE: u32 = u32::from_be_bytes(*b"420v");
/// `420f`: bi-planar 4:2:0, full range.
const NV12_FULL_RANGE: u32 = u32::from_be_bytes(*b"420f");
/// `yuvs`: packed 4:2:2 in Y0 Cb Y1 Cr order (YUY2), as the card delivers it uncompressed.
const YUY2: u32 = u32::from_be_bytes(*b"yuvs");

type Texture = Retained<ProtocolObject<dyn MTLTexture>>;

/// Runs until the slot is closed.
pub fn run(target: RenderTarget, slot: &FrameSlot<PixelBuffer>, shared: Arc<VideoShared>) {
    let mut renderer = match Renderer::new(target, shared.clone()) {
        Ok(renderer) => renderer,
        Err(error) => {
            eprintln!("video: {error}");
            return;
        }
    };
    let mut in_flight: Option<InFlight> = None;
    let mut reported = false;
    timing::run_video_loop(slot, &shared.delay, &shared.stats, || CACurrentMediaTime(), |frame| {
        autoreleasepool(|_| match renderer.draw(frame) {
            Ok(Some(next)) => {
                crate::video::mark_first_present();
                // Keep the previous frame's buffers alive until the GPU is done with them.
                if let Some(previous) = in_flight.replace(next) {
                    previous.command_buffer.waitUntilCompleted();
                    record_gpu_time(&shared, &previous.command_buffer);
                }
            }
            Ok(None) => {}
            Err(error) => {
                if !reported {
                    eprintln!("video: {error}");
                    reported = true;
                }
            }
        })
    });
    if let Some(last) = in_flight {
        last.command_buffer.waitUntilCompleted();
    }
}

/// The compiled video shader, ready to draw a planned frame into any BGRA8 texture.
pub struct VideoPass {
    pub queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
}

impl VideoPass {
    pub fn new(device: &ProtocolObject<dyn MTLDevice>) -> Result<Self, String> {
        let queue = device.newCommandQueue().ok_or("no Metal command queue")?;
        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(SHADER), None)
            .map_err(|e| format!("video shader failed to compile: {}", e.localizedDescription()))?;
        let function = |name: &str| library.newFunctionWithName(&NSString::from_str(name)).ok_or(format!("missing {name}"));
        let descriptor = MTLRenderPipelineDescriptor::new();
        let (vertex, fragment) = (function("fullscreen_vertex")?, function("video_fragment")?);
        descriptor.setVertexFunction(Some(&vertex));
        descriptor.setFragmentFunction(Some(&fragment));
        unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) }.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        let pipeline = device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .map_err(|e| format!("video pipeline failed: {}", e.localizedDescription()))?;
        Ok(Self { queue, pipeline })
    }

    /// Encodes one frame: clears `target` to black and draws `planes` through `plan`.
    pub fn encode(
        &self,
        command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
        target: &ProtocolObject<dyn MTLTexture>,
        planes: [&ProtocolObject<dyn MTLTexture>; 2],
        plan: &Plan,
    ) -> Result<(), String> {
        let pass = MTLRenderPassDescriptor::new();
        let attachment = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setTexture(Some(target));
        attachment.setLoadAction(MTLLoadAction::Clear);
        attachment.setClearColor(MTLClearColor { red: 0.0, green: 0.0, blue: 0.0, alpha: 1.0 });
        attachment.setStoreAction(MTLStoreAction::Store);

        let encoder = command_buffer.renderCommandEncoderWithDescriptor(&pass).ok_or("no render encoder")?;
        let vp = plan.viewport;
        encoder.setRenderPipelineState(&self.pipeline);
        encoder.setViewport(MTLViewport { originX: vp.x, originY: vp.y, width: vp.width, height: vp.height, znear: 0.0, zfar: 1.0 });
        unsafe {
            encoder.setFragmentTexture_atIndex(Some(planes[0]), 0);
            encoder.setFragmentTexture_atIndex(Some(planes[1]), 1);
            encoder.setFragmentBytes_length_atIndex(NonNull::from(&plan.params).cast(), size_of::<ShaderParams>(), 0);
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
        }
        encoder.endEncoding();
        Ok(())
    }
}

struct Renderer {
    target: RenderTarget,
    pass: VideoPass,
    /// Each capture buffer's planes as Metal textures, made the first time the buffer comes round
    /// (see [`Renderer::textures`]), by IOSurface ID. The surface is kept, so its ID stays its own.
    surfaces: Vec<(u32, CFRetained<IOSurfaceRef>, Vec<Texture>)>,
    shared: Arc<VideoShared>,
}

/// More buffers than any capture pool cycles through: past this the cache starts over (a new
/// Source or Video format brings a new pool).
const MAX_SURFACES: usize = 16;

/// Everything a submitted frame still needs until the GPU has finished with it.
/// Adds a completed frame's GPU time to the video pass's total (#218): what a Metal System Trace
/// shows for it, unlike macOS's per-process counter (#200).
fn record_gpu_time(shared: &VideoShared, command_buffer: &ProtocolObject<dyn MTLCommandBuffer>) {
    let busy = command_buffer.GPUEndTime() - command_buffer.GPUStartTime();
    if busy.is_finite() && busy > 0.0 {
        lock(&shared.gpu).add(std::time::Instant::now(), std::time::Duration::from_secs_f64(busy));
    }
}

struct InFlight {
    command_buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    /// The capture buffer stays retained while the GPU reads it, so the capture pool can't reuse it.
    _frame: PixelBuffer,
}

impl Renderer {
    fn new(target: RenderTarget, shared: Arc<VideoShared>) -> Result<Self, String> {
        let pass = VideoPass::new(&target.device)?;
        Ok(Self { target, pass, surfaces: Vec::new(), shared })
    }

    /// Draws one frame and presents it. `Ok(None)` when there is nothing to draw into yet.
    fn draw(&mut self, frame: PixelBuffer) -> Result<Option<InFlight>, String> {
        let image = &*frame.image;
        let (info, textures) = match CVPixelBufferGetPixelFormatType(image) {
            format @ (NV12_VIDEO_RANGE | NV12_FULL_RANGE) => {
                let textures = self.textures(image, &[(0, MTLPixelFormat::R8Unorm, 1), (1, MTLPixelFormat::RG8Unorm, 1)])?;
                let size = (CVPixelBufferGetWidthOfPlane(image, 0) as u32, CVPixelBufferGetHeightOfPlane(image, 0) as u32);
                let info = FrameInfo { width: size.0, height: size.1, layout: Layout::Nv12, os_squeezed: format == NV12_VIDEO_RANGE };
                (info, textures)
            }
            YUY2 if !CVPixelBufferIsPlanar(image) => {
                let width = objc2_core_video::CVPixelBufferGetWidth(image) as u32;
                let height = objc2_core_video::CVPixelBufferGetHeight(image) as u32;
                let textures = self.textures(image, &[(0, MTLPixelFormat::RGBA8Unorm, 2)])?;
                (FrameInfo { width, height, layout: Layout::Yuy2, os_squeezed: false }, textures)
            }
            other => return Err(format!("pixel format {} isn't drawn yet", fourcc(other))),
        };
        // A few luma samples, to notice a picture that stays one flat colour (#108).
        let step = if matches!(info.layout, Layout::Yuy2) { 2 } else { 1 };
        if let Some(samples) = luma_samples(image, (info.width as usize, info.height as usize), step) {
            lock(&self.shared.flat).observe(&samples, std::time::Instant::now());
        }

        let layer = &self.target.layer;
        let bounds = layer.bounds();
        let scale = layer.contentsScale();
        let size = CGSize { width: (bounds.size.width * scale).round(), height: (bounds.size.height * scale).round() };
        if size.width < 1.0 || size.height < 1.0 {
            return Ok(None);
        }
        if layer.drawableSize() != size {
            layer.setDrawableSize(size);
        }
        let Some(drawable) = layer.nextDrawable() else {
            return Ok(None);
        };

        let settings = *lock(&self.shared.picture);
        let plan = picture::plan(info, &settings, (size.width, size.height));
        let command_buffer = self.pass.queue.commandBuffer().ok_or("no command buffer")?;
        let second = textures.get(1).unwrap_or(&textures[0]);
        self.pass.encode(&command_buffer, &drawable.texture(), [&textures[0], second], &plan)?;
        // Record when the frame actually reached the screen (Metal's host-time present time).
        let (shared, stamps) = (self.shared.clone(), frame.stamps);
        let on_presented = RcBlock::new(move |presented: NonNull<ProtocolObject<dyn MTLDrawable>>| {
            let at = unsafe { presented.as_ref() }.presentedTime();
            lock(&shared.stats).presented(stamps, at, false);
        });
        unsafe { drawable.addPresentedHandler(RcBlock::as_ptr(&on_presented)) };
        command_buffer.presentDrawable(ProtocolObject::from_ref(&*drawable));
        command_buffer.commit();

        Ok(Some(InFlight { command_buffer, _frame: frame }))
    }

    /// The frame's planes as Metal textures, without copying: `planes` lists each plane's index,
    /// texel format and how many image pixels one texel holds across (2 for packed YUY2 viewed as
    /// RGBA). The capture card cycles through a small pool of buffers, so each buffer's textures
    /// are made the first time it comes round and reused after that. Wrapping the planes anew every
    /// frame (`CVMetalTextureCache`) was half of the video thread's CPU time (#118).
    fn textures(&mut self, image: &CVPixelBuffer, planes: &[(usize, MTLPixelFormat, usize)]) -> Result<Vec<Texture>, String> {
        let surface = CVPixelBufferGetIOSurface(Some(image)).ok_or("the frame isn't backed by an IOSurface")?;
        // By ID, not by object: frames come from macOS's camera service, and the same buffer can
        // arrive as a new IOSurfaceRef each time.
        let id = surface.id();
        if let Some((_, _, textures)) = self.surfaces.iter().find(|(known, _, _)| *known == id) {
            return Ok(textures.clone());
        }
        let textures = planes
            .iter()
            .map(|&(index, format, texel_pixels)| {
                let (width, height) = if CVPixelBufferIsPlanar(image) {
                    (CVPixelBufferGetWidthOfPlane(image, index), CVPixelBufferGetHeightOfPlane(image, index))
                } else {
                    (objc2_core_video::CVPixelBufferGetWidth(image), objc2_core_video::CVPixelBufferGetHeight(image))
                };
                let desc = unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(format, width.div_ceil(texel_pixels), height, false) };
                desc.setUsage(MTLTextureUsage::ShaderRead);
                desc.setStorageMode(MTLStorageMode::Shared);
                self.target.device.newTextureWithDescriptor_iosurface_plane(&desc, &surface, index).ok_or(format!("plane {index} texture failed"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if self.surfaces.len() >= MAX_SURFACES {
            self.surfaces.clear();
        }
        self.surfaces.push((id, surface, textures.clone()));
        Ok(textures)
    }
}

/// The [`flat::luma_grid`] samples of a `width`×`height` frame, read from its first plane, where
/// neighbouring luma values sit `step` bytes apart: 1 for NV12, 2 for YUY2 (Y0 Cb Y1 Cr). The
/// buffer is only locked for reading, which doesn't copy it.
fn luma_samples(image: &CVPixelBuffer, (width, height): (usize, usize), step: usize) -> Option<Vec<u8>> {
    let read_only = CVPixelBufferLockFlags::ReadOnly;
    if unsafe { CVPixelBufferLockBaseAddress(image, read_only) } != kCVReturnSuccess {
        return None;
    }
    let (base, row) = if CVPixelBufferIsPlanar(image) {
        (CVPixelBufferGetBaseAddressOfPlane(image, 0), CVPixelBufferGetBytesPerRowOfPlane(image, 0))
    } else {
        (CVPixelBufferGetBaseAddress(image), CVPixelBufferGetBytesPerRow(image))
    };
    let samples = (!base.is_null() && row >= width * step).then(|| {
        // SAFETY: while the buffer is locked its first plane is `row` bytes for each of its `height` rows.
        let bytes = unsafe { std::slice::from_raw_parts(base as *const u8, row * height) };
        flat::luma_grid(width, height, |x, y| bytes[y * row + x * step])
    });
    unsafe { CVPixelBufferUnlockBaseAddress(image, read_only) };
    samples
}

#[cfg(test)]
mod gpu_tests {
    //! The MSL shader against the CPU reference (`video::picture::gpu_check`), offscreen.
    use super::*;
    use crate::video::picture::{gpu_check, reference};
    use objc2_metal::{
        MTLBlitCommandEncoder, MTLBuffer, MTLCopyAllDevices, MTLCreateSystemDefaultDevice, MTLOrigin, MTLRegion,
        MTLResourceOptions, MTLSize, MTLStorageMode, MTLTextureDescriptor, MTLTextureUsage,
    };

    fn device() -> Retained<ProtocolObject<dyn MTLDevice>> {
        // CI's paravirtual GPU isn't always the "default" device for a command-line process.
        MTLCreateSystemDefaultDevice()
            .or_else(|| MTLCopyAllDevices().firstObject())
            .expect("a Metal device (local GPU or CI's paravirtual one)")
    }

    fn texture(device: &ProtocolObject<dyn MTLDevice>, format: MTLPixelFormat, (width, height): (usize, usize), bytes: &[u8], bpp: usize) -> Texture {
        let desc = unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(format, width, height, false) };
        desc.setUsage(MTLTextureUsage::ShaderRead);
        let t = device.newTextureWithDescriptor(&desc).expect("texture");
        let region = MTLRegion { origin: MTLOrigin { x: 0, y: 0, z: 0 }, size: MTLSize { width, height, depth: 1 } };
        unsafe { t.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, NonNull::new(bytes.as_ptr() as *mut _).unwrap(), width * bpp) };
        t
    }

    fn render(device: &ProtocolObject<dyn MTLDevice>, pass: &VideoPass, planes: &reference::Planes, plan: &Plan) -> Vec<[f32; 3]> {
        let (w, h) = (gpu_check::FRAME.0 as usize, gpu_check::FRAME.1 as usize);
        let (a, b) = match planes.layout {
            Layout::Nv12 => (
                texture(device, MTLPixelFormat::R8Unorm, (w, h), planes.first, 1),
                texture(device, MTLPixelFormat::RG8Unorm, (w.div_ceil(2), h.div_ceil(2)), planes.second, 2),
            ),
            Layout::Yuy2 => {
                let t = texture(device, MTLPixelFormat::RGBA8Unorm, (w.div_ceil(2), h), planes.first, 4);
                (t.clone(), t)
            }
            Layout::Ycbcr444 => {
                let t = texture(device, MTLPixelFormat::R8Unorm, (w * 3, h), planes.first, 1);
                (t.clone(), t)
            }
        };
        let (out_w, out_h) = (gpu_check::OUTPUT.0 as usize, gpu_check::OUTPUT.1 as usize);
        let desc = unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(MTLPixelFormat::BGRA8Unorm, out_w, out_h, false) };
        desc.setUsage(MTLTextureUsage::RenderTarget);
        desc.setStorageMode(MTLStorageMode::Private);
        let target = device.newTextureWithDescriptor(&desc).expect("target");
        let row_bytes = out_w * 4;
        let readback = device.newBufferWithLength_options(row_bytes * out_h, MTLResourceOptions::StorageModeShared).expect("buffer");

        let command_buffer = pass.queue.commandBuffer().expect("command buffer");
        pass.encode(&command_buffer, &target, [&a, &b], plan).expect("encode");
        let blit = command_buffer.blitCommandEncoder().expect("blit");
        unsafe {
            blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
                &target,
                0,
                0,
                MTLOrigin { x: 0, y: 0, z: 0 },
                MTLSize { width: out_w, height: out_h, depth: 1 },
                &readback,
                0,
                row_bytes,
                row_bytes * out_h,
            );
        }
        blit.endEncoding();
        command_buffer.commit();
        command_buffer.waitUntilCompleted();

        let bytes = unsafe { std::slice::from_raw_parts(readback.contents().as_ptr() as *const u8, row_bytes * out_h) };
        bytes.as_chunks::<4>().0.iter().map(|p| [p[2] as f32 / 255.0, p[1] as f32 / 255.0, p[0] as f32 / 255.0]).collect()
    }

    /// GPU time of one frame at the real sizes: a 1920×1080 NV12 frame drawn to a 3024×1964
    /// Retina drawable (#56). Run by hand: `cargo test --release -- --ignored shader_gpu_time --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn shader_gpu_time() {
        use crate::video::picture::{FrameInfo, PictureSettings, plan};
        let device = device();
        let pass = VideoPass::new(&device).expect("video pass");
        let (w, h) = (1920usize, 1080usize);
        let luma: Vec<u8> = (0..w * h).map(|i| (i % 251) as u8).collect();
        let chroma: Vec<u8> = (0..w.div_ceil(2) * h.div_ceil(2) * 2).map(|i| (i % 241) as u8).collect();
        let a = texture(&device, MTLPixelFormat::R8Unorm, (w, h), &luma, 1);
        let b = texture(&device, MTLPixelFormat::RG8Unorm, (w.div_ceil(2), h.div_ceil(2)), &chroma, 2);
        let (out_w, out_h) = (3024usize, 1964usize);
        let desc = unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(MTLPixelFormat::BGRA8Unorm, out_w, out_h, false) };
        desc.setUsage(MTLTextureUsage::RenderTarget);
        desc.setStorageMode(MTLStorageMode::Private);
        let target = device.newTextureWithDescriptor(&desc).expect("target");
        let info = FrameInfo { width: w as u32, height: h as u32, layout: Layout::Nv12, os_squeezed: false };
        for (what, settings) in [
            ("fit", PictureSettings::default()),
            ("smart stretch to 21:9", PictureSettings { fill_mode: crate::video::picture::FillMode::SmartStretch, target_ratio: crate::video::picture::TargetRatio(64.0 / 27.0), ..Default::default() }),
        ] {
            let plan = plan(info, &settings, (out_w as f64, out_h as f64));
            let mut times = Vec::new();
            for _ in 0..120 {
                let command_buffer = pass.queue.commandBuffer().expect("command buffer");
                pass.encode(&command_buffer, &target, [&a, &b], &plan).expect("encode");
                command_buffer.commit();
                command_buffer.waitUntilCompleted();
                times.push((command_buffer.GPUEndTime() - command_buffer.GPUStartTime()) * 1000.0);
            }
            times.sort_by(f64::total_cmp);
            eprintln!("{what}: GPU {:.2} ms median, {:.2} ms p90 per frame", times[times.len() / 2], times[times.len() * 9 / 10]);
        }
    }

    #[test]
    fn msl_matches_reference() {
        let device = device();
        let pass = VideoPass::new(&device).expect("video pass");
        for layout in [Layout::Nv12, Layout::Yuy2, Layout::Ycbcr444] {
            gpu_check::assert_all_match(layout, |planes, plan| render(&device, &pass, planes, plan));
        }
    }
}
