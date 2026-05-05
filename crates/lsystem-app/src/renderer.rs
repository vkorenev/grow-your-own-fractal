use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::Key;
use winit::window::{Window, WindowAttributes, WindowId};

use crate::camera::Camera;
use crate::fractal_renderer::{
    ColorParams, FractalRenderer, FrameOutcome, Vertex, color_params_from_config,
    geometry_to_vertices,
};
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
    egui: Option<EguiRenderer>,
    ui: UiState,
    vertices: Arc<Vec<Vertex>>,
    color_params: ColorParams,
    geometry_version: u64,
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
            egui: None,
            ui,
            vertices: Arc::new(vec![]),
            color_params: ColorParams::default(),
            geometry_version: 0,
            proxy,
        }
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
        let total_segments = (vertices.len() / 2) as u32;
        self.color_params = color_params_from_config(&cfg.colors.line, total_segments);
        self.vertices = Arc::new(vertices);
        self.geometry_version += 1;
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

        self.window = Some(window.clone());

        #[cfg(not(target_arch = "wasm32"))]
        {
            let fractal =
                pollster::block_on(FractalRenderer::new(window)).expect("no GPU adapter found");
            if let Some(egui) = &mut self.egui {
                egui.attach_gpu(
                    fractal.device.clone(),
                    fractal.queue.clone(),
                    fractal.surface_format(),
                );
            }
            self.state = Some(fractal);
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            // wgpu's adapter/device requests are JS Promises and must be awaited
            // on the JS event loop. Hand the result back via a UserEvent so the
            // App regains exclusive ownership before installing the FractalRenderer.
            let proxy = self.proxy.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match FractalRenderer::new(window).await {
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
                // The canvas's CSS layout typically settles during async device init,
                // so a Resized event may have fired while `state` was still None and
                // been dropped. Re-sync the surface to the window's current size so the
                // first frame isn't stuck at the stale init size.
                if let (Some(state), Some(window)) = (&mut self.state, &self.window) {
                    state.resize(window.inner_size());
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // egui-winit returns `repaint = true` for `WindowEvent::RedrawRequested` itself
        // (see the catch-all in egui-winit/src/lib.rs), so feeding it through the
        // egui-feed branch below would queue another RedrawRequested every frame.
        // Handle it directly instead. Mirrors eframe (crates/eframe/src/native/run.rs).
        if matches!(event, WindowEvent::RedrawRequested) {
            self.handle_redraw();
            return;
        }

        // `response.repaint` is the single trigger for redraws on input; nothing below
        // needs to call `request_redraw` again. `response.consumed` means egui handled
        // the event and we should skip our own processing.
        let egui_consumed = if let (Some(egui), Some(window)) = (&mut self.egui, &self.window) {
            let response = egui.on_window_event(window, &event);
            if response.repaint {
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
            }

            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                if let Key::Character(ch) = &event.logical_key
                    && ch.eq_ignore_ascii_case("f")
                {
                    self.camera.reset();
                }
            }

            _ => {}
        }
    }
}

impl App {
    fn handle_redraw(&mut self) {
        self.regenerate_if_dirty();
        if let (Some(fractal), Some(egui), Some(window)) =
            (&mut self.state, &mut self.egui, &self.window)
        {
            match fractal.begin_frame() {
                FrameOutcome::Skip => {}
                FrameOutcome::Reconfigured => {
                    window.request_redraw();
                }
                FrameOutcome::Ready(frame, view, mut encoder, reconfigure_after) => {
                    let surface_size = fractal.size();
                    let repaint_delay = egui.render(
                        window,
                        &mut self.ui,
                        Arc::clone(&self.vertices),
                        self.geometry_version,
                        self.color_params,
                        &mut self.camera,
                        self.bounds_min,
                        self.bounds_max,
                        &view,
                        &mut encoder,
                        surface_size,
                    );
                    fractal.end_frame(*frame, encoder, reconfigure_after);
                    // `repaint_delay == 0` covers active drags, scrolls, and
                    // animations; egui itself drives them.
                    if reconfigure_after || repaint_delay.is_zero() {
                        window.request_redraw();
                    }
                }
            }
        }
        // A widget interaction during draw may have set `ui.dirty`; regenerate on the
        // next frame. `repaint_delay` alone isn't reliable: a slider release can settle
        // it to MAX even when the geometry still needs to be rebuilt.
        if self.ui.dirty
            && let Some(window) = &self.window
        {
            window.request_redraw();
        }
    }
}
