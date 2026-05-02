use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::Key;
use winit::window::{Window, WindowAttributes, WindowId};

use crate::camera::Camera;
use crate::fractal_renderer::{FractalRenderer, FrameOutcome, geometry_to_vertices};
use crate::input::InputState;
use crate::ui::{EguiRenderer, UiState};

/// Events raised outside the winit event loop and routed back through
/// `ApplicationHandler::user_event`. On wasm the GPU device is acquired
/// asynchronously and delivered this way; on native the device is built
/// synchronously and this variant is unused.
pub enum UserEvent {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    GpuReady(Box<FractalRenderer>),
}

pub struct App {
    window: Option<Arc<Window>>,
    state: Option<FractalRenderer>,
    bounds_min: [f32; 2],
    bounds_max: [f32; 2],
    camera: Camera,
    input: InputState,
    egui: Option<EguiRenderer>,
    ui: UiState,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    proxy: EventLoopProxy<UserEvent>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let ui = UiState::new();
        Self {
            window: None,
            state: None,
            bounds_min: [-1.0, -1.0],
            bounds_max: [1.0, 1.0],
            camera: Camera::new(),
            input: InputState::new(),
            egui: None,
            ui,
            proxy,
        }
    }

    /// Run after `state` is populated to upload the initial geometry.
    fn finish_init(&mut self) {
        self.ui.dirty = true;
        self.regenerate_if_dirty();
    }

    /// Physical pixel width of the egui side panel, used to restrict the camera viewport.
    fn panel_px(&self) -> u32 {
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        self.ui.panel_width.mul_add(scale, 0.5) as u32
    }

    fn regenerate_if_dirty(&mut self) {
        if !self.ui.dirty {
            return;
        }
        self.ui.dirty = false;
        let Some(cfg) = self.ui.effective_config() else {
            return;
        };
        let geometry = lsystem_core::generate(&cfg);
        let (vertices, bounds_min, bounds_max) = geometry_to_vertices(&geometry);
        self.bounds_min = bounds_min;
        self.bounds_max = bounds_max;
        self.camera.reset();
        if let Some(state) = &mut self.state {
            state.upload_vertices(&vertices);
        }
        self.upload_camera_transform();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn upload_camera_transform(&self) {
        if let Some(state) = &self.state {
            let (w, h) = state.size();
            let effective_w = w.saturating_sub(self.panel_px()).max(1);
            let transform =
                self.camera
                    .compute_transform(self.bounds_min, self.bounds_max, effective_w, h);
            state.upload_transform(&transform);
        }
    }
}

fn window_attributes() -> WindowAttributes {
    let attrs = Window::default_attributes().with_title("Grow Your Own Fractal");
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        use winit::platform::web::WindowAttributesExtWebSys;
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("lsystem-canvas"))
            .and_then(|el| el.dyn_into::<web_sys::HtmlCanvasElement>().ok());
        attrs.with_canvas(canvas)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        attrs
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            // Reentrant resume (e.g. mobile app foreground): keep existing state.
            return;
        }

        let window = Arc::new(event_loop.create_window(window_attributes()).unwrap());

        self.egui = Some(EguiRenderer::new(&window));

        let size = window.inner_size();
        let initial_vertices = vec![];
        let initial_transform = self.camera.compute_transform(
            self.bounds_min,
            self.bounds_max,
            size.width.max(1),
            size.height.max(1),
        );

        self.window = Some(window.clone());

        #[cfg(not(target_arch = "wasm32"))]
        {
            let fractal = pollster::block_on(FractalRenderer::new(
                window,
                &initial_vertices,
                &initial_transform,
            ))
            .expect("no GPU adapter found");
            if let Some(egui) = &mut self.egui {
                egui.attach_gpu(
                    fractal.device.clone(),
                    fractal.queue.clone(),
                    fractal.surface_format(),
                );
            }
            self.state = Some(fractal);
            self.finish_init();
        }
        #[cfg(target_arch = "wasm32")]
        {
            // wgpu's adapter/device requests are JS Promises and must be awaited
            // on the JS event loop. Hand the result back via a UserEvent so the
            // App regains exclusive ownership before installing the FractalRenderer.
            let proxy = self.proxy.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match FractalRenderer::new(window, &initial_vertices, &initial_transform).await {
                    Ok(state) => {
                        let _ = proxy.send_event(UserEvent::GpuReady(Box::new(state)));
                    }
                    Err(()) => {
                        crate::web::show_unsupported_overlay();
                    }
                }
            });
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::GpuReady(fractal) => {
                if let Some(egui) = &mut self.egui {
                    egui.attach_gpu(
                        fractal.device.clone(),
                        fractal.queue.clone(),
                        fractal.surface_format(),
                    );
                }
                self.state = Some(*fractal);
                // A Resized event may have arrived between resumed() and now
                // (the canvas's CSS layout typically completes during async
                // device init) and been dropped because `state` was None.
                // Re-sync the surface to the window's current size before
                // rendering so the first frame isn't stuck at the stale init
                // size.
                if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
                    state.resize(window.inner_size());
                }
                self.finish_init();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Unconditionally clear drag on left-button release so it can never get stuck,
        // even when egui consumes the release event.
        if matches!(
            &event,
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            }
        ) {
            self.input.on_left_button(false);
        }

        // Feed event to egui first.
        let egui_consumed = if let (Some(egui), Some(window)) = (&mut self.egui, &self.window) {
            let response = egui.on_window_event(window, &event);
            // response.repaint keeps is_pointer_over_egui() fresh between clicks.
            if response.repaint || response.consumed {
                window.request_redraw();
            }
            response.consumed
        } else {
            false
        };
        if egui_consumed {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.resize(size);
                }
                self.upload_camera_transform();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            // Only start a drag when the cursor is not over the egui panel.
            // Releases are already handled unconditionally above.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !self.egui.as_ref().is_some_and(|e| e.is_pointer_over_egui()) => {
                self.input.on_left_button(true);
            }

            WindowEvent::CursorMoved { position, .. } => {
                let x = position.x as f32;
                let y = position.y as f32;
                if let Some([dx, dy]) = self.input.on_cursor_moved(x, y) {
                    if let Some(state) = &self.state {
                        let (w, h) = state.size();
                        let effective_w = w.saturating_sub(self.panel_px()).max(1);
                        self.camera.pan_by_pixels(
                            dx,
                            dy,
                            self.bounds_min,
                            self.bounds_max,
                            effective_w,
                            h,
                        );
                    }
                    self.upload_camera_transform();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. }
                if !self.egui.as_ref().is_some_and(|e| e.is_pointer_over_egui()) =>
            {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 20.0,
                };
                let factor = 1.1_f32.powf(lines);
                if let Some(state) = &self.state {
                    let (w, h) = state.size();
                    let panel_px = self.panel_px();
                    let effective_w = w.saturating_sub(panel_px).max(1);
                    // Cursor position relative to the canvas viewport.
                    let cursor = [
                        (self.input.cursor_pos[0] - panel_px as f32).max(0.0),
                        self.input.cursor_pos[1],
                    ];
                    self.camera.zoom_toward_cursor(
                        factor,
                        cursor,
                        self.bounds_min,
                        self.bounds_max,
                        effective_w,
                        h,
                    );
                }
                self.upload_camera_transform();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                if let Key::Character(ch) = &event.logical_key
                    && ch.eq_ignore_ascii_case("f")
                {
                    self.camera.reset();
                    self.upload_camera_transform();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }

            WindowEvent::RedrawRequested => {
                self.regenerate_if_dirty();
                let panel_px = self.panel_px();
                if let (Some(fractal), Some(egui), Some(window)) =
                    (&mut self.state, &mut self.egui, &self.window)
                {
                    match fractal.begin_frame(panel_px) {
                        FrameOutcome::Skip => {}
                        FrameOutcome::Reconfigured => {
                            window.request_redraw();
                        }
                        FrameOutcome::Ready(frame, view, mut encoder, reconfigure_after) => {
                            let surface_size = fractal.size();
                            let (_, repaint_delay) = egui.render(
                                window,
                                &mut self.ui,
                                &view,
                                &mut encoder,
                                surface_size,
                            );
                            fractal.end_frame(*frame, encoder, reconfigure_after);
                            // Redraw immediately on surface reconfiguration or when egui
                            // needs the very next frame (repaint_delay == ZERO). Deferred
                            // egui animations (repaint_delay > ZERO) are fine to skip —
                            // they will resume on the next user-input event.
                            if reconfigure_after || repaint_delay.is_zero() {
                                window.request_redraw();
                            }
                        }
                    }
                }
                // Dirty flag may have been set by slider interaction during draw.
                if self.ui.dirty
                    && let Some(window) = &self.window
                {
                    window.request_redraw();
                }
            }

            _ => {}
        }
    }
}
