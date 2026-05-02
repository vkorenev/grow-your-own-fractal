use std::sync::Arc;

use egui_wgpu::ScreenDescriptor;
use include_dir::{Dir, include_dir};
use lsystem_core::Config;
use wgpu::TextureFormat;
use winit::event::WindowEvent;
use winit::window::Window;

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
    pub angle: f32,
    pub step: f32,
    pub error: Option<String>,
    /// Set to true when geometry needs regenerating.
    pub dirty: bool,
    /// Width of the side panel in egui logical pixels, updated each frame.
    pub panel_width: f32,
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
            angle: 60.0,
            step: 1.0,
            error: None,
            dirty: true,
            panel_width: 280.0,
        };
        s.apply();
        s
    }

    pub fn apply(&mut self) {
        match Config::parse(&self.toml_text) {
            Ok(cfg) => {
                self.iterations = cfg.iterations;
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

    #[allow(deprecated)]
    pub fn draw(&mut self, ctx: &egui::Context) {
        let panel_response = egui::Panel::left("controls")
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
                    ui.add(egui::Slider::new(&mut self.iterations, 1..=10).text("Iterations"));
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
        self.panel_width = panel_response.response.rect.width();
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
        self.renderer = Some(egui_wgpu::Renderer::new(
            &device,
            format,
            egui_wgpu::RendererOptions::default(),
        ));
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

    pub fn is_pointer_over_egui(&self) -> bool {
        self.ctx.is_pointer_over_egui()
    }

    pub fn render(
        &mut self,
        window: &Window,
        ui: &mut UiState,
        view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        surface_size: (u32, u32),
    ) -> (bool, std::time::Duration) {
        let (renderer, device, queue) = match (
            self.renderer.as_mut(),
            self.device.as_ref(),
            self.queue.as_ref(),
        ) {
            (Some(r), Some(d), Some(q)) => (r, d, q),
            _ => return (false, std::time::Duration::MAX),
        };

        let raw_input = self.winit_state.take_egui_input(window);
        self.ctx.begin_pass(raw_input);
        ui.draw(&self.ctx);
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
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
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

        (false, repaint_delay)
    }
}
