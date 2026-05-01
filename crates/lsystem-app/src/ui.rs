use lsystem_core::Config;

pub const PRESETS: &[(&str, &str)] = &[
    (
        "Koch Snowflake",
        include_str!("../../../presets/koch_snowflake.toml"),
    ),
    (
        "Dragon Curve",
        include_str!("../../../presets/dragon_curve.toml"),
    ),
    (
        "Sierpinski Triangle",
        include_str!("../../../presets/sierpinski_triangle.toml"),
    ),
    ("Plant A", include_str!("../../../presets/plant_a.toml")),
    (
        "Hilbert Curve",
        include_str!("../../../presets/hilbert_curve.toml"),
    ),
];

pub struct UiState {
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
        let toml_text = PRESETS[0].1.to_string();
        let mut s = Self {
            preset_idx: 0,
            toml_text: toml_text.clone(),
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
                    .selected_text(PRESETS[self.preset_idx].0)
                    .show_ui(ui, |ui| {
                        for (i, (name, _)) in PRESETS.iter().enumerate() {
                            ui.selectable_value(&mut self.preset_idx, i, *name);
                        }
                    });
                if self.preset_idx != prev {
                    self.toml_text = PRESETS[self.preset_idx].1.to_string();
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
