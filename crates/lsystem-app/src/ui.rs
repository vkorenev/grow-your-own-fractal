use std::sync::Arc;

use egui_wgpu::ScreenDescriptor;
use include_dir::{Dir, include_dir};
use lsystem_core::Config;
use wgpu::TextureFormat;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::camera::Camera;
use crate::fractal_renderer::{ColorParams, FractalCallback, FractalPipelineResources, Vertex};

impl egui_wgpu::CallbackTrait for FractalCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let res = callback_resources
            .get_mut::<FractalPipelineResources>()
            .unwrap();
        if self.needs_upload {
            res.upload(device, queue, &self.vertices, self.color_params);
        }
        res.write_transform(queue, self.transform);
        vec![]
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        // egui_wgpu sets the viewport to our allocated rect before calling paint().
        callback_resources
            .get::<FractalPipelineResources>()
            .unwrap()
            .draw(render_pass);
    }
}

static PRESETS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../presets");

fn load_presets() -> Vec<(String, &'static str)> {
    let mut files: Vec<_> = PRESETS_DIR
        .files()
        .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("toml"))
        .collect();
    files.sort_by_key(|f| f.path());
    files
        .into_iter()
        .filter_map(|f| {
            let stem = f.path().file_stem()?.to_str()?;
            let name = stem
                .split('_')
                .map(|w| {
                    let mut chars = w.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            Some((name, f.contents_utf8()?))
        })
        .collect()
}

pub struct UiState {
    presets: Vec<(String, &'static str)>,
    pub preset_idx: usize,
    pub toml_text: String,
    pub base_config: Option<Config>,
    pub iterations: u32,
    pub max_iterations: u32,
    pub angle: f32,
    pub step: f32,
    pub error: Option<String>,
    /// Set to true when geometry needs regenerating.
    pub dirty: bool,
}

impl UiState {
    pub fn new() -> Self {
        let presets = load_presets();
        let toml_text = presets
            .first()
            .expect("no preset TOML files found")
            .1
            .to_string();
        let mut s = Self {
            presets,
            preset_idx: 0,
            toml_text,
            base_config: None,
            iterations: 4,
            max_iterations: 10,
            angle: 60.0,
            step: 1.0,
            error: None,
            dirty: true,
        };
        s.apply();
        s
    }

    pub fn apply(&mut self) {
        match Config::parse(&self.toml_text) {
            Ok(cfg) => {
                self.max_iterations = lsystem_core::max_safe_iterations(
                    &cfg.axiom,
                    &cfg.rules,
                    crate::fractal_renderer::MAX_SEGMENTS,
                )
                .max(1);
                self.iterations = cfg.iterations.min(self.max_iterations);
                self.angle = cfg.angle;
                self.step = cfg.step;
                self.base_config = Some(cfg);
                self.error = None;
                self.dirty = true;
            }
            Err(e) => {
                self.error = Some(e.to_string());
            }
        }
    }

    pub fn effective_config(&self) -> Option<Config> {
        self.base_config.clone().map(|mut c| {
            c.iterations = self.iterations;
            c.angle = self.angle;
            c.step = self.step;
            c
        })
    }

    pub fn background_color(&self) -> wgpu::Color {
        let [r, g, b] = self
            .base_config
            .as_ref()
            .map(|c| c.colors.background)
            .unwrap_or_default();
        wgpu::Color {
            r: r as f64,
            g: g as f64,
            b: b as f64,
            a: 1.0,
        }
    }

    /// Draw the egui UI. Mutates `camera` based on pan/zoom interactions inside the
    /// central panel.
    #[allow(deprecated)]
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        vertices: Arc<Vec<Vertex>>,
        needs_upload: bool,
        color_params: ColorParams,
        camera: &mut Camera,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
    ) {
        egui::Panel::left("controls")
            .resizable(false)
            .default_size(280.0)
            .show(ctx, |ui| {
                ui.heading("Grow Your Own Fractal");
                ui.separator();

                ui.label("Preset");
                let prev = self.preset_idx;
                egui::ComboBox::from_id_salt("preset_combo")
                    .selected_text(self.presets[self.preset_idx].0.as_str())
                    .show_ui(ui, |ui| {
                        for (i, (name, _)) in self.presets.iter().enumerate() {
                            ui.selectable_value(&mut self.preset_idx, i, name.as_str());
                        }
                    });
                if self.preset_idx != prev {
                    self.toml_text = self.presets[self.preset_idx].1.to_string();
                    self.apply();
                }

                ui.separator();

                ui.label("Config (TOML)");
                ui.add(
                    egui::TextEdit::multiline(&mut self.toml_text)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(12)
                        .desired_width(f32::INFINITY),
                );

                if ui.button("Apply").clicked() {
                    self.apply();
                }

                match &self.error {
                    Some(err) => {
                        ui.colored_label(egui::Color32::RED, err);
                    }
                    None => {
                        ui.colored_label(egui::Color32::GREEN, "OK");
                    }
                }

                if self.base_config.is_some() {
                    ui.separator();
                    ui.label("Overrides");

                    let prev_iter = self.iterations;
                    ui.add(
                        egui::Slider::new(&mut self.iterations, 1..=self.max_iterations)
                            .text("Iterations"),
                    );
                    if self.iterations != prev_iter {
                        self.dirty = true;
                    }

                    let prev_angle = self.angle;
                    ui.add(
                        egui::Slider::new(&mut self.angle, 1.0..=180.0)
                            .text("Angle °")
                            .step_by(0.5),
                    );
                    if self.angle != prev_angle {
                        self.dirty = true;
                    }

                    let prev_step = self.step;
                    ui.add(
                        egui::Slider::new(&mut self.step, 0.1..=10.0)
                            .text("Step")
                            .step_by(0.1),
                    );
                    if self.step != prev_step {
                        self.dirty = true;
                    }
                }

                ui.separator();
                ui.small("Drag to pan · Scroll to zoom · F to fit");
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let (response, painter) =
                    ui.allocate_painter(ui.available_size(), egui::Sense::drag());

                let ppp = ui.ctx().pixels_per_point();
                let rect = response.rect;
                let physical_w = (rect.width() * ppp) as u32;
                let physical_h = (rect.height() * ppp) as u32;

                if response.dragged_by(egui::PointerButton::Primary) {
                    let delta = response.drag_delta();
                    camera.pan_by_pixels(
                        delta.x * ppp,
                        delta.y * ppp,
                        bounds_min,
                        bounds_max,
                        physical_w,
                        physical_h,
                    );
                }

                let scroll = ui.input(|i| i.smooth_scroll_delta);
                if scroll.y.abs() > 0.0 && response.hovered() {
                    let factor = 1.1_f32.powf(scroll.y / 20.0);
                    let hover_pos = response.hover_pos().unwrap_or(rect.center());
                    let local = hover_pos - rect.min;
                    let cursor_px = [local.x * ppp, local.y * ppp];
                    camera.zoom_toward_cursor(
                        factor, cursor_px, bounds_min, bounds_max, physical_w, physical_h,
                    );
                }

                let transform = camera.compute_transform(
                    bounds_min,
                    bounds_max,
                    physical_w.max(1),
                    physical_h.max(1),
                );

                painter.add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    FractalCallback {
                        vertices,
                        transform,
                        needs_upload,
                        color_params,
                    },
                ));
            });
    }
}

pub struct EguiRenderer {
    pub ctx: egui::Context,
    winit_state: egui_winit::State,
    renderer: Option<egui_wgpu::Renderer>,
    device: Option<Arc<wgpu::Device>>,
    queue: Option<Arc<wgpu::Queue>>,
}

impl EguiRenderer {
    pub fn new(window: &Window) -> Self {
        let ctx = egui::Context::default();
        let winit_state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        Self {
            ctx,
            winit_state,
            renderer: None,
            device: None,
            queue: None,
        }
    }

    pub fn attach_gpu(
        &mut self,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        format: TextureFormat,
    ) {
        let mut renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        renderer
            .callback_resources
            .insert(FractalPipelineResources::new(&device, format));
        self.renderer = Some(renderer);
        self.device = Some(device);
        self.queue = Some(queue);
    }

    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &WindowEvent,
    ) -> egui_winit::EventResponse {
        self.winit_state.on_window_event(window, event)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        window: &Window,
        ui: &mut UiState,
        vertices: Arc<Vec<Vertex>>,
        needs_upload: bool,
        color_params: ColorParams,
        camera: &mut Camera,
        bounds_min: [f32; 2],
        bounds_max: [f32; 2],
        view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        surface_size: (u32, u32),
    ) -> std::time::Duration {
        let (renderer, device, queue) = match (
            self.renderer.as_mut(),
            self.device.as_ref(),
            self.queue.as_ref(),
        ) {
            (Some(r), Some(d), Some(q)) => (r, d, q),
            _ => return std::time::Duration::MAX,
        };

        let raw_input = self.winit_state.take_egui_input(window);
        self.ctx.begin_pass(raw_input);
        ui.draw(
            &self.ctx,
            vertices,
            needs_upload,
            color_params,
            camera,
            bounds_min,
            bounds_max,
        );
        let full_output = self.ctx.end_pass();
        self.winit_state
            .handle_platform_output(window, full_output.platform_output);

        let repaint_delay = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|vo| vo.repaint_delay)
            .unwrap_or(std::time::Duration::MAX);

        let screen_desc = ScreenDescriptor {
            size_in_pixels: [surface_size.0, surface_size.1],
            pixels_per_point: full_output.pixels_per_point,
        };
        let tris = self
            .ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        for (id, delta) in &full_output.textures_delta.set {
            renderer.update_texture(device, queue, *id, delta);
        }

        renderer.update_buffers(device, queue, encoder, &tris, &screen_desc);
        {
            let mut egui_pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            // Clearing here also serves as the fractal viewport background,
                            // which is why CentralPanel uses Frame::NONE (no own fill).
                            load: wgpu::LoadOp::Clear(ui.background_color()),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                })
                // forget_lifetime is safe here: egui_pass is dropped before
                // encoder.finish(), which is the invariant wgpu requires.
                .forget_lifetime();
            renderer.render(&mut egui_pass, &tris, &screen_desc);
        }

        for id in &full_output.textures_delta.free {
            renderer.free_texture(id);
        }

        repaint_delay
    }
}
