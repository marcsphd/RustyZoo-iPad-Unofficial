//! The native MetalFX scaler presents a software framebuffer into SDL's Metal layer.

use std::ffi::c_void;
use std::ptr::{NonNull, null_mut};

use rustyzoo::Framebuffer;
use sdl3::render::Canvas;
use sdl3::sys::render::SDL_GetRenderMetalLayer;
use sdl3::video::Window;

unsafe extern "C" {
    fn rustyzoo_metalfx_create(layer: *mut c_void, frame_generation: bool) -> *mut c_void;
    fn rustyzoo_metalfx_destroy(context: *mut c_void);
    fn rustyzoo_metalfx_present(
        context: *mut c_void,
        pixels: *const u8,
        width: u32,
        height: u32,
        x: u32,
        y: u32,
        out_width: u32,
        out_height: u32,
        capture: *mut u8,
        capture_width: u32,
        capture_height: u32,
        interpolate: bool,
    ) -> i32;
}

pub(super) struct Scaler {
    context: NonNull<c_void>,
    announced: Option<((u32, u32), (u32, u32))>,
    generated: bool,
}

pub(super) enum Present {
    Drawn,
    Unavailable,
    Failed,
}

impl Scaler {
    pub fn new(canvas: &Canvas<Window>, frame_generation: bool) -> Option<Self> {
        // SAFETY: the renderer is live on the main thread; the native scaler retains its layer.
        let context = unsafe {
            NonNull::new(rustyzoo_metalfx_create(
                SDL_GetRenderMetalLayer(canvas.raw()),
                frame_generation,
            ))?
        };
        Some(Self {
            context,
            announced: None,
            generated: false,
        })
    }

    pub fn present(
        &mut self,
        frame: &Framebuffer,
        (x, y, w, h): (f32, f32, f32, f32),
        capture: Option<&mut Framebuffer>,
        interpolate: bool,
    ) -> Present {
        let (pixels, cw, ch) = capture.map_or((null_mut(), 0, 0), |frame| {
            (frame.pixels.as_mut_ptr(), frame.width, frame.height)
        });
        // SAFETY: both framebuffers contain width * height * 4 bytes. Native code copies input
        // during the call and waits for any requested capture before returning.
        let status = unsafe {
            rustyzoo_metalfx_present(
                self.context.as_ptr(),
                frame.pixels.as_ptr(),
                frame.width,
                frame.height,
                x as u32,
                y as u32,
                w as u32,
                h as u32,
                pixels,
                cw,
                ch,
                interpolate,
            )
        };
        match status {
            1 | 3 => {
                if status == 3 && !self.generated {
                    self.generated = true;
                    println!("MetalFX frame generation active");
                }
                let input = (frame.width, frame.height);
                let output = (w as u32, h as u32);
                if self.announced != Some((input, output)) {
                    self.announced = Some((input, output));
                    println!(
                        "MetalFX spatial upscaling {}x{} → {}x{}",
                        input.0, input.1, output.0, output.1
                    );
                }
                Present::Drawn
            }
            2 => Present::Unavailable,
            _ => Present::Failed,
        }
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        // SAFETY: this is the retained native object returned by create, released once.
        unsafe { rustyzoo_metalfx_destroy(self.context.as_ptr()) };
    }
}
