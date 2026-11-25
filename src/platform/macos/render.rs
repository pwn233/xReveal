//! The video thread on macOS: takes the newest frame, wraps its planes as Metal textures with no
//! copy (`CVMetalTextureCache`), draws them and presents on arrival (docs/spec/v1.md §3.3, §3.4).

use std::ptr::{self, NonNull};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFRetained, CGSize};
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBufferGetHeightOfPlane,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidthOfPlane, kCVReturnSuccess,
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
use crate::video::fill;
use crate::video::slot::FrameSlot;

const SHADER: &str = include_str!("video.metal");

/// `420v`: bi-planar 4:2:0, video range.
const NV12_VIDEO_RANGE: u32 = u32::from_be_bytes(*b"420v");
/// `420f`: bi-planar 4:2:0, full range.
const NV12_FULL_RANGE: u32 = u32::from_be_bytes(*b"420f");

/// Runs until the slot is closed.
pub fn run(target: RenderTarget, slot: &FrameSlot<PixelBuffer>) {
    let renderer = match Renderer::new(target) {
        Ok(renderer) => renderer,
        Err(error) => {
            eprintln!("video: {error}");
            return;
        }
    };
    let mut in_flight: Option<InFlight> = None;
    let mut unsupported_reported = false;
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
                if !unsupported_reported {
                    eprintln!("video: {error}");
                    unsupported_reported = true;
                }
            }
        });
    }
    if let Some(last) = in_flight {
        last.command_buffer.waitUntilCompleted();
    }
}

struct Renderer {
    target: RenderTarget,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    texture_cache: CFRetained<CVMetalTextureCache>,
}

/// Everything a submitted frame still needs until the GPU has finished with it.
struct InFlight {
    command_buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    _frame: PixelBuffer,
    _planes: [CFRetained<CVMetalTexture>; 2],
}

impl Renderer {
    fn new(target: RenderTarget) -> Result<Self, String> {
        let device = &target.device;
        let queue = device.newCommandQueue().ok_or("no Metal command queue")?;
        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(SHADER), None)
            .map_err(|e| format!("video shader failed to compile: {}", e.localizedDescription()))?;
        let vertex = library
            .newFunctionWithName(&NSString::from_str("fullscreen_vertex"))
            .ok_or("missing fullscreen_vertex")?;
        let fragment = library.newFunctionWithName(&NSString::from_str("nv12_fragment")).ok_or("missing nv12_fragment")?;

        let descriptor = MTLRenderPipelineDescriptor::new();
        descriptor.setVertexFunction(Some(&vertex));
        descriptor.setFragmentFunction(Some(&fragment));
        unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) }.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
        let pipeline = device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .map_err(|e| format!("video pipeline failed: {}", e.localizedDescription()))?;

        let mut cache: *mut CVMetalTextureCache = ptr::null_mut();
        let status = unsafe { CVMetalTextureCache::create(None, None, device, None, NonNull::from(&mut cache)) };
        let texture_cache = NonNull::new(cache)
            .filter(|_| status == kCVReturnSuccess)
            .map(|cache| unsafe { CFRetained::from_raw(cache) })
            .ok_or(format!("CVMetalTextureCache failed ({status})"))?;

        Ok(Self { target, queue, pipeline, texture_cache })
    }

    /// Draws one frame and presents it. `Ok(None)` when there is nothing to draw into yet.
    fn draw(&self, frame: PixelBuffer) -> Result<Option<InFlight>, String> {
        let image = &*frame.0;
        let pixel_format = CVPixelBufferGetPixelFormatType(image);
        let video_range = match pixel_format {
            NV12_VIDEO_RANGE => 1u32,
            NV12_FULL_RANGE => 0u32,
            other => return Err(format!("pixel format {} isn't drawn yet (NV12 only)", fourcc(other))),
        };
        let luma = self.plane(&frame, 0, MTLPixelFormat::R8Unorm)?;
        let chroma = self.plane(&frame, 1, MTLPixelFormat::RG8Unorm)?;
        let luma_texture = CVMetalTextureGetTexture(&luma).ok_or("no luma texture")?;
        let chroma_texture = CVMetalTextureGetTexture(&chroma).ok_or("no chroma texture")?;

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

        let pass = MTLRenderPassDescriptor::new();
        let attachment = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setTexture(Some(&drawable.texture()));
        attachment.setLoadAction(MTLLoadAction::Clear);
        attachment.setClearColor(MTLClearColor { red: 0.0, green: 0.0, blue: 0.0, alpha: 1.0 });
        attachment.setStoreAction(MTLStoreAction::Store);

        let command_buffer = self.queue.commandBuffer().ok_or("no command buffer")?;
        let encoder = command_buffer.renderCommandEncoderWithDescriptor(&pass).ok_or("no render encoder")?;
        let rect = fill::fit(luma_texture.width() as f64, luma_texture.height() as f64, size.width, size.height);
        encoder.setRenderPipelineState(&self.pipeline);
        encoder.setViewport(MTLViewport {
            originX: rect.x,
            originY: rect.y,
            width: rect.width,
            height: rect.height,
            znear: 0.0,
            zfar: 1.0,
        });
        unsafe {
            encoder.setFragmentTexture_atIndex(Some(&luma_texture), 0);
            encoder.setFragmentTexture_atIndex(Some(&chroma_texture), 1);
            encoder.setFragmentBytes_length_atIndex(
                NonNull::from(&video_range).cast(),
                size_of::<u32>(),
                0,
            );
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
        }
        encoder.endEncoding();
        command_buffer.presentDrawable(ProtocolObject::from_ref(&*drawable));
        command_buffer.commit();

        Ok(Some(InFlight { command_buffer, _frame: frame, _planes: [luma, chroma] }))
    }

    /// Wraps one plane of the frame as a Metal texture without copying.
    fn plane(&self, frame: &PixelBuffer, index: usize, format: MTLPixelFormat) -> Result<CFRetained<CVMetalTexture>, String> {
        let image = &*frame.0;
        let width = CVPixelBufferGetWidthOfPlane(image, index);
        let height = CVPixelBufferGetHeightOfPlane(image, index);
        let mut texture: *mut CVMetalTexture = ptr::null_mut();
        let status = unsafe {
            CVMetalTextureCache::create_texture_from_image(
                None,
                &self.texture_cache,
                image,
                None,
                format,
                width,
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
