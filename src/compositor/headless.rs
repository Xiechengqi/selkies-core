//! Headless backend using Pixman software renderer
//!
//! Replaces winit backend with a headless renderer that exports
//! framebuffer pixels for GStreamer appsrc ingestion.

use smithay::{
    backend::allocator::Fourcc as DrmFourcc,
    backend::renderer::{
        damage::OutputDamageTracker,
        element::surface::WaylandSurfaceRenderElement,
        pixman::PixmanRenderer,
        ExportMem, Bind, Offscreen,
    },
    desktop::space::render_output,
    output::{Mode, Output, PhysicalProperties, Subpixel},
    utils::{Rectangle, Size},
};
use log::{info, warn};
use pixman::Image;

/// Headless backend that renders to an in-memory Pixman buffer
pub struct HeadlessBackend {
    renderer: PixmanRenderer,
    buffer: Image<'static, 'static>,
    output: Output,
    damage_tracker: OutputDamageTracker,
    width: u32,
    height: u32,
}

impl HeadlessBackend {
    /// Create a new headless backend with the given dimensions
    pub fn new(width: u32, height: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let mut renderer = PixmanRenderer::new()
            .map_err(|e| format!("Failed to create Pixman renderer: {:?}", e))?;

        let size = Size::from((width as i32, height as i32));
        let buffer: Image<'static, 'static> = renderer.create_buffer(DrmFourcc::Xrgb8888, size)
            .map_err(|e| format!("Failed to create offscreen buffer: {:?}", e))?;

        let output = Output::new(
            "ivnc-headless".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "iVnc".into(),
                model: "Virtual".into(),
                serial_number: "0".into(),
            },
        );

        let mode = Mode {
            size: (width as i32, height as i32).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
        output.set_preferred(mode);

        let damage_tracker = OutputDamageTracker::from_output(&output);

        info!("Headless backend created: {}x{} @ 60Hz (Pixman)", width, height);

        Ok(Self { renderer, buffer, output, damage_tracker, width, height })
    }

    pub fn output(&self) -> &Output {
        &self.output
    }

    /// Send frame callbacks to all mapped windows so clients keep submitting.
    pub fn send_frame_callbacks(&self, state: &super::Compositor) {
        state.space.elements().for_each(|window| {
            window.send_frame(
                &self.output,
                state.start_time.elapsed(),
                None,
                |_, _| Some(self.output.clone()),
            );
        });
    }

    /// Render the compositor space and return raw pixel data.
    /// Caller is responsible for only calling this when there is work to do.
    pub fn render_frame(
        &mut self,
        state: &mut super::Compositor,
    ) -> Option<Vec<u8>> {
        let mut framebuffer = match self.renderer.bind(&mut self.buffer) {
            Ok(fb) => fb,
            Err(e) => {
                warn!("Failed to bind framebuffer: {:?}", e);
                return None;
            }
        };

        // age=0: always full render. Skipping logic is handled by the
        // caller via Compositor::needs_redraw so we don't rely on the
        // damage tracker's broken skip path.
        let render_result = render_output::<
            _,
            WaylandSurfaceRenderElement<PixmanRenderer>,
            _,
            _,
        >(
            &self.output,
            &mut self.renderer,
            &mut framebuffer,
            1.0,
            0,
            [&state.space],
            &[],
            &mut self.damage_tracker,
            [0.1, 0.1, 0.1, 1.0],
        );

        match render_result {
            Ok(_result) => {
                let size = Size::from((self.width as i32, self.height as i32));
                let region = Rectangle::new((0, 0).into(), size);

                let mapping = match self.renderer.copy_framebuffer(
                    &framebuffer, region, DrmFourcc::Xrgb8888,
                ) {
                    Ok(m) => m,
                    Err(e) => { warn!("Failed to copy framebuffer: {:?}", e); return None; }
                };

                match self.renderer.map_texture(&mapping) {
                    Ok(data) => Some(data.to_vec()),
                    Err(e) => { warn!("Failed to map texture: {:?}", e); None }
                }
            }
            Err(e) => { warn!("Render output failed: {:?}", e); None }
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Box<dyn std::error::Error>> {
        let size = Size::from((width as i32, height as i32));
        self.buffer = self.renderer.create_buffer(DrmFourcc::Xrgb8888, size)
            .map_err(|e| format!("Failed to create buffer: {:?}", e))?;

        let mode = Mode {
            size: (width as i32, height as i32).into(),
            refresh: 60_000,
        };
        self.output.change_current_state(Some(mode), None, None, None);
        self.damage_tracker = OutputDamageTracker::from_output(&self.output);
        self.width = width;
        self.height = height;
        info!("Headless backend resized to {}x{}", width, height);
        Ok(())
    }

    pub fn reset_damage(&mut self) {
        self.damage_tracker = OutputDamageTracker::from_output(&self.output);
    }
}
