//! The video thread on macOS: takes the newest frame, wraps its planes as Metal textures with no
//! copy (`CVMetalTextureCache`), draws them in one shader pass planned by `video::picture`, and
//! presents on arrival (docs/spec/v1.md §3.3, §3.4).

use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex, PoisonError};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFRetained, CGSize};
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBufferGetHeightOfPlane,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidthOfPlane, CVPixelBufferIsPlanar, kCVReturnSuccess,
};
use objc2_foundation::NSString;
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLDevice, MTLLibrary, MTLLoadAction,
    MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLStoreAction, MTLTexture, MTLViewport,
};
use objc2_quartz_core::CAMetalDrawable;

use super::capture::{PixelBuffer, fourcc};
use super::surface::RenderTarget;
use crate::video::picture::{self, FrameInfo, Layout, PictureSettings, Plan, ShaderParams};
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
pub fn run(target: RenderTarget, slot: &FrameSlot<PixelBuffer>, settings: Arc<Mutex<PictureSettings>>) {
    let renderer = match Renderer::new(target, settings) {
        Ok(renderer) => renderer,
        Err(error) => {
            eprintln!("video: {error}");
            return;
        }
    };
    let mut in_flight: Option<InFlight> = None;
    let mut reported = false;
    while let Some(frame) = slot.take() {
        autoreleasepool(|_| match renderer.draw(frame) {
            Ok(Some(next)) => {
                crate::video::FIRST_PRESENT.store(true, std::sync::atomic::Ordering::SeqCst);
                // Keep the previous frame's buffers alive until the GPU is done with them.
                if let Some(previous) = in_flight.replace(next) {
                    previous.command_buffer.waitUntilCompleted();
                }
            }
            Ok(None) => {}
            Err(error) => {
                if !reported {
                    eprintln!("video: {error}");
                    reported = true;
                }
            }
        });
    }
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
    texture_cache: CFRetained<CVMetalTextureCache>,
    settings: Arc<Mutex<PictureSettings>>,
}

/// Everything a submitted frame still needs until the GPU has finished with it.
struct InFlight {
    command_buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    _frame: PixelBuffer,
    _planes: Vec<CFRetained<CVMetalTexture>>,
}

impl Renderer {
    fn new(target: RenderTarget, settings: Arc<Mutex<PictureSettings>>) -> Result<Self, String> {
        let pass = VideoPass::new(&target.device)?;
        let mut cache: *mut CVMetalTextureCache = ptr::null_mut();
        let status = unsafe { CVMetalTextureCache::create(None, None, &target.device, None, NonNull::from(&mut cache)) };
        let texture_cache = NonNull::new(cache)
            .filter(|_| status == kCVReturnSuccess)
            .map(|cache| unsafe { CFRetained::from_raw(cache) })
            .ok_or(format!("CVMetalTextureCache failed ({status})"))?;
        Ok(Self { target, pass, texture_cache, settings })
    }

    /// Draws one frame and presents it. `Ok(None)` when there is nothing to draw into yet.
    fn draw(&self, frame: PixelBuffer) -> Result<Option<InFlight>, String> {
        let image = &*frame.0;
        let (info, planes) = match CVPixelBufferGetPixelFormatType(image) {
            format @ (NV12_VIDEO_RANGE | NV12_FULL_RANGE) => {
                let luma = self.plane(&frame, 0, MTLPixelFormat::R8Unorm, 1)?;
                let chroma = self.plane(&frame, 1, MTLPixelFormat::RG8Unorm, 1)?;
                let size = (CVPixelBufferGetWidthOfPlane(image, 0) as u32, CVPixelBufferGetHeightOfPlane(image, 0) as u32);
                let info = FrameInfo { width: size.0, height: size.1, layout: Layout::Nv12, os_squeezed: format == NV12_VIDEO_RANGE };
                (info, vec![luma, chroma])
            }
            YUY2 if !CVPixelBufferIsPlanar(image) => {
                let width = objc2_core_video::CVPixelBufferGetWidth(image) as u32;
                let height = objc2_core_video::CVPixelBufferGetHeight(image) as u32;
                let packed = self.plane(&frame, 0, MTLPixelFormat::RGBA8Unorm, 2)?;
                (FrameInfo { width, height, layout: Layout::Yuy2, os_squeezed: false }, vec![packed])
            }
            other => return Err(format!("pixel format {} isn't drawn yet", fourcc(other))),
        };
        let textures: Vec<Texture> =
            planes.iter().map(|p| CVMetalTextureGetTexture(p).ok_or("no plane texture")).collect::<Result<_, _>>()?;

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

        let settings = *self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        let plan = picture::plan(info, &settings, (size.width, size.height));
        let command_buffer = self.pass.queue.commandBuffer().ok_or("no command buffer")?;
        let second = textures.get(1).unwrap_or(&textures[0]);
        self.pass.encode(&command_buffer, &drawable.texture(), [&textures[0], second], &plan)?;
        command_buffer.presentDrawable(ProtocolObject::from_ref(&*drawable));
        command_buffer.commit();

        Ok(Some(InFlight { command_buffer, _frame: frame, _planes: planes }))
    }

    /// Wraps one plane of the frame as a Metal texture without copying. `texel_pixels` is how many
    /// image pixels one texel holds across (2 for packed YUY2 viewed as RGBA).
    fn plane(&self, frame: &PixelBuffer, index: usize, format: MTLPixelFormat, texel_pixels: usize) -> Result<CFRetained<CVMetalTexture>, String> {
        let image = &*frame.0;
        let (width, height) = if CVPixelBufferIsPlanar(image) {
            (CVPixelBufferGetWidthOfPlane(image, index), CVPixelBufferGetHeightOfPlane(image, index))
        } else {
            (objc2_core_video::CVPixelBufferGetWidth(image), objc2_core_video::CVPixelBufferGetHeight(image))
        };
        let mut texture: *mut CVMetalTexture = ptr::null_mut();
        let status = unsafe {
            CVMetalTextureCache::create_texture_from_image(
                None,
                &self.texture_cache,
                image,
                None,
                format,
                width.div_ceil(texel_pixels),
                height,
                index,
                NonNull::from(&mut texture),
            )
        };
        NonNull::new(texture)
            .filter(|_| status == kCVReturnSuccess)
            .map(|texture| unsafe { CFRetained::from_raw(texture) })
            .ok_or(format!("plane {index} texture failed ({status})"))
    }
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

    #[test]
    fn msl_matches_reference() {
        let device = device();
        let pass = VideoPass::new(&device).expect("video pass");
        for layout in [Layout::Nv12, Layout::Yuy2, Layout::Ycbcr444] {
            gpu_check::assert_all_match(layout, |planes, plan| render(&device, &pass, planes, plan));
        }
    }
}
