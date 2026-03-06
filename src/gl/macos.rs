use std::ffi::c_void;
use std::ptr::NonNull;
use std::str::FromStr;

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSOpenGLContext, NSOpenGLContextParameter, NSOpenGLPFAAccelerated, NSOpenGLPFAAlphaSize,
    NSOpenGLPFAColorSize, NSOpenGLPFADepthSize, NSOpenGLPFADoubleBuffer, NSOpenGLPFAMultisample,
    NSOpenGLPFAOpenGLProfile, NSOpenGLPFASampleBuffers, NSOpenGLPFASamples, NSOpenGLPFAStencilSize,
    NSOpenGLPixelFormat, NSOpenGLProfileVersion3_2Core, NSOpenGLProfileVersion4_1Core,
    NSOpenGLProfileVersionLegacy, NSOpenGLView, NSView,
};
use objc2_foundation::NSSize;
use raw_window_handle::RawWindowHandle;

use core_foundation::base::TCFType;
use core_foundation::bundle::{CFBundleGetBundleWithIdentifier, CFBundleGetFunctionPointerForName};
use core_foundation::string::CFString;

use super::{GlConfig, GlError, Profile};

pub type CreationFailedError = ();
pub struct GlContext {
    view: Retained<NSOpenGLView>,
    context: Retained<NSOpenGLContext>,
}

impl GlContext {
    pub unsafe fn create(
        parent: &RawWindowHandle, config: GlConfig, mtm: MainThreadMarker,
    ) -> Result<GlContext, GlError> {
        let handle = if let RawWindowHandle::AppKit(handle) = parent {
            handle
        } else {
            return Err(GlError::InvalidWindowHandle);
        };

        let parent_view = handle.ns_view.as_ptr() as *mut NSView;

        let version = if config.version < (3, 2) && config.profile == Profile::Compatibility {
            NSOpenGLProfileVersionLegacy
        } else if config.version == (3, 2) && config.profile == Profile::Core {
            NSOpenGLProfileVersion3_2Core
        } else if config.version > (3, 2) && config.profile == Profile::Core {
            NSOpenGLProfileVersion4_1Core
        } else {
            return Err(GlError::VersionNotSupported);
        };

        #[rustfmt::skip]
        let mut attrs = vec![
            NSOpenGLPFAOpenGLProfile as u32, version as u32,
            NSOpenGLPFAColorSize as u32, (config.red_bits + config.blue_bits + config.green_bits) as u32,
            NSOpenGLPFAAlphaSize as u32, config.alpha_bits as u32,
            NSOpenGLPFADepthSize as u32, config.depth_bits as u32,
            NSOpenGLPFAStencilSize as u32, config.stencil_bits as u32,
            NSOpenGLPFAAccelerated as u32,
        ];

        if config.samples.is_some() {
            #[rustfmt::skip]
            attrs.extend_from_slice(&[
                NSOpenGLPFAMultisample as u32,
                NSOpenGLPFASampleBuffers as u32, 1,
                NSOpenGLPFASamples as u32, config.samples.unwrap() as u32,
            ]);
        }

        if config.double_buffer {
            attrs.push(NSOpenGLPFADoubleBuffer as u32);
        }

        attrs.push(0);

        let Some(pixel_format) = NSOpenGLPixelFormat::initWithAttributes(
            NSOpenGLPixelFormat::alloc(),
            NonNull::new(attrs.as_ptr() as _).unwrap(),
        ) else {
            return Err(GlError::CreationFailed(()));
        };

        let Some(view) = NSOpenGLView::initWithFrame_pixelFormat(
            NSOpenGLView::alloc(mtm),
            (*parent_view).frame(),
            Some(&pixel_format),
        ) else {
            return Err(GlError::CreationFailed(()));
        };

        view.setWantsBestResolutionOpenGLSurface(true);

        view.display();
        (*parent_view).addSubview(&view);

        let Some(context) = view.openGLContext() else {
            return Err(GlError::CreationFailed(()));
        };
        let vsync = &config.vsync as *const bool;
        context.setValues_forParameter(
            NonNull::new(vsync as _).unwrap(),
            NSOpenGLContextParameter::SwapInterval,
        );

        Ok(GlContext { view, context })
    }

    pub unsafe fn make_current(&self) {
        self.context.makeCurrentContext();
    }

    pub unsafe fn make_not_current(&self) {
        NSOpenGLContext::clearCurrentContext();
    }

    pub fn get_proc_address(&self, symbol: &str) -> *const c_void {
        let symbol_name = CFString::from_str(symbol).unwrap();
        let framework_name = CFString::from_str("com.apple.opengl").unwrap();
        let framework =
            unsafe { CFBundleGetBundleWithIdentifier(framework_name.as_concrete_TypeRef()) };

        unsafe { CFBundleGetFunctionPointerForName(framework, symbol_name.as_concrete_TypeRef()) }
    }

    pub fn swap_buffers(&self) {
        unsafe {
            self.context.flushBuffer();
            self.view.setNeedsDisplay(true);
        }
    }

    /// On macOS the `NSOpenGLView` needs to be resized separtely from our main view.
    pub(crate) fn resize(&self, size: NSSize) {
        unsafe {
            self.view.setFrameSize(size);
            self.view.setNeedsDisplay(true);
        }
    }
}
