use lsystem_core::{
    ColorDefaults, EditorColorConfig, EditorGenerationConfig, EditorLineColorConfig,
    LineColorConfig, LineColorDefaults, Rgb, normalize_line_color_for_render,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineColorMode {
    Solid,
    Gradient,
    HueCycle,
}

impl LineColorMode {
    pub const ALL: &'static [Self] = &[Self::Solid, Self::Gradient, Self::HueCycle];

    pub fn from_line_color(line_color: &LineColorConfig) -> Self {
        match line_color {
            LineColorConfig::Solid(_) => Self::Solid,
            LineColorConfig::Gradient { .. } => Self::Gradient,
            LineColorConfig::HueCycle { .. } => Self::HueCycle,
        }
    }

    pub fn from_editor_line_color(line_color: &EditorLineColorConfig) -> Self {
        match line_color {
            EditorLineColorConfig::Solid { .. } => Self::Solid,
            EditorLineColorConfig::Gradient { .. } => Self::Gradient,
            EditorLineColorConfig::HueCycle { .. } => Self::HueCycle,
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "solid" => Some(Self::Solid),
            "gradient" => Some(Self::Gradient),
            "hue_cycle" => Some(Self::HueCycle),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Gradient => "gradient",
            Self::HueCycle => "hue_cycle",
        }
    }
}

impl std::fmt::Display for LineColorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Solid => "Solid",
            Self::Gradient => "Gradient",
            Self::HueCycle => "Hue cycle",
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ColorControlMemory {
    background: Rgb,
    line_defaults: LineColorDefaults,
    solid: Option<Rgb>,
    gradient: Option<RememberedGradient>,
    hue_cycle: Option<Rgb>,
}

#[derive(Clone, Copy, Debug)]
struct RememberedGradient {
    start: Rgb,
    end: Rgb,
    topological_depth: bool,
}

impl ColorControlMemory {
    /// Initializes color picker memory from authored editor colors and defaults.
    ///
    /// Runtime-normalized render colors are intentionally excluded so inactive
    /// control slots preserve authored/default values rather than render-only
    /// optimizations.
    pub fn from_editor_config(editor: &EditorColorConfig, defaults: &ColorDefaults) -> Self {
        let mut memory = Self {
            background: editor.background.unwrap_or(defaults.background),
            line_defaults: defaults.line,
            solid: None,
            gradient: None,
            hue_cycle: None,
        };
        memory.remember_line(line_color_for_controls(editor, &defaults.line));
        memory
    }

    pub fn background(&self) -> Rgb {
        self.background
    }

    pub fn remember_background(&mut self, background: Rgb) {
        self.background = background;
    }

    pub fn solid_color(&self) -> Rgb {
        self.solid.unwrap_or(self.line_defaults.solid)
    }

    pub fn gradient_fields(&self) -> (Rgb, Rgb, bool) {
        let defaults = self.line_defaults.gradient;
        let gradient = self.gradient.unwrap_or(RememberedGradient {
            start: defaults.start,
            end: defaults.end,
            topological_depth: defaults.topological_depth,
        });
        (gradient.start, gradient.end, gradient.topological_depth)
    }

    pub fn hue_cycle_initial(&self) -> Rgb {
        self.hue_cycle
            .unwrap_or(self.line_defaults.hue_cycle.initial)
    }

    pub fn remember_line(&mut self, line_color: LineColorConfig) {
        match line_color {
            LineColorConfig::Solid(color) => self.solid = Some(color),
            LineColorConfig::Gradient {
                start,
                end,
                topological_depth,
            } => {
                self.gradient = Some(RememberedGradient {
                    start,
                    end,
                    topological_depth,
                });
            }
            LineColorConfig::HueCycle { initial } => self.hue_cycle = Some(initial),
        }
    }

    pub fn line_for(&self, mode: LineColorMode) -> LineColorConfig {
        match mode {
            LineColorMode::Solid => LineColorConfig::Solid(self.solid_color()),
            LineColorMode::Gradient => {
                let (start, end, topological_depth) = self.gradient_fields();
                LineColorConfig::Gradient {
                    start,
                    end,
                    topological_depth,
                }
            }
            LineColorMode::HueCycle => LineColorConfig::HueCycle {
                initial: self.hue_cycle_initial(),
            },
        }
    }
}

/// Returns the line color that controls should display and edit.
///
/// This resolves omitted mode parameters from defaults, but intentionally does
/// not apply runtime-only normalization such as disabling topological-depth
/// gradients for bracketless grammars. That keeps authored editor state intact
/// while the runtime `Config` remains optimized for rendering/export.
pub fn line_color_for_controls(
    editor: &EditorColorConfig,
    defaults: &LineColorDefaults,
) -> LineColorConfig {
    editor
        .line
        .map(|line| line.resolve(defaults))
        .unwrap_or_else(|| defaults.default_line_color())
}

/// Returns the normalized line color for render/export paths.
///
/// This starts from the same default-resolved line color that controls display,
/// then applies runtime-only normalization such as disabling topological-depth
/// gradients for bracketless grammars.
pub fn line_color_for_render(
    editor: &EditorColorConfig,
    generation: &EditorGenerationConfig,
    defaults: &LineColorDefaults,
) -> LineColorConfig {
    normalize_line_color_for_render(
        line_color_for_controls(editor, defaults),
        generation.has_stack_directives(),
    )
}

/// Returns the line-color picker mode represented by authored color state.
///
/// When the config omits `colors.line`, the selected mode follows the default
/// line-color mode rather than assuming solid.
pub fn selected_line_color_mode(
    editor: &EditorColorConfig,
    defaults: &LineColorDefaults,
) -> LineColorMode {
    editor
        .line
        .as_ref()
        .map(LineColorMode::from_editor_line_color)
        .unwrap_or_else(|| {
            LineColorMode::from_line_color(&line_color_for_controls(editor, defaults))
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use lsystem_core::{
        ColorDefaults, ConfigDefaults, Dimensions, EditorColorConfig, EditorGenerationConfig,
        EditorLineColorConfig, LineColorConfig, Rgb,
    };

    use super::{
        ColorControlMemory, LineColorMode, line_color_for_controls, line_color_for_render,
        selected_line_color_mode,
    };

    fn default_gradient() -> LineColorConfig {
        let defaults = ConfigDefaults::embedded().colors.line.gradient;
        LineColorConfig::Gradient {
            start: defaults.start,
            end: defaults.end,
            topological_depth: defaults.topological_depth,
        }
    }

    fn default_hue_cycle() -> LineColorConfig {
        LineColorConfig::HueCycle {
            initial: ConfigDefaults::embedded().colors.line.hue_cycle.initial,
        }
    }

    fn custom_color_defaults() -> ColorDefaults {
        let mut defaults = ConfigDefaults::embedded().colors;
        defaults.background = Rgb::new(0x08, 0x10, 0x18);
        defaults.line.default = lsystem_core::config::DefaultLineColorMode::Gradient;
        defaults.line.solid = Rgb::new(0x10, 0x20, 0x30);
        defaults.line.gradient.start = Rgb::new(0x40, 0x50, 0x60);
        defaults.line.gradient.end = Rgb::new(0x70, 0x80, 0x90);
        defaults.line.gradient.topological_depth = true;
        defaults.line.hue_cycle.initial = Rgb::new(0xa0, 0xb0, 0xc0);
        defaults
    }

    fn editor_generation(axiom: &str, rules: Vec<(char, &str)>) -> EditorGenerationConfig {
        EditorGenerationConfig {
            dimensions: Dimensions::TwoD,
            axiom: axiom.to_string(),
            iterations: 1,
            angle: 90.0,
            step: None,
            initial_heading: None,
            rules: rules
                .into_iter()
                .map(|(symbol, rhs)| (symbol, rhs.to_string()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn missing_editor_line_uses_default_line_color_for_controls() {
        let defaults = custom_color_defaults();
        assert_eq!(
            line_color_for_controls(&EditorColorConfig::default(), &defaults.line),
            defaults.line.default_line_color()
        );
    }

    #[test]
    fn selected_line_color_mode_uses_default_mode_when_line_is_absent() {
        let defaults = custom_color_defaults();

        assert_eq!(
            selected_line_color_mode(&EditorColorConfig::default(), &defaults.line),
            LineColorMode::Gradient
        );
    }

    #[test]
    fn selected_line_color_mode_uses_authored_mode_when_present() {
        let defaults = custom_color_defaults();
        let editor = EditorColorConfig {
            background: None,
            line: Some(EditorLineColorConfig::HueCycle { initial: None }),
        };

        assert_eq!(
            selected_line_color_mode(&editor, &defaults.line),
            LineColorMode::HueCycle
        );
    }

    #[test]
    fn line_color_for_render_normalizes_bracketless_topological_gradient() {
        let defaults = custom_color_defaults();
        let editor = EditorColorConfig {
            background: None,
            line: Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x01, 0x02, 0x03)),
                end: Some(Rgb::new(0x04, 0x05, 0x06)),
                topological_depth: Some(true),
            }),
        };

        assert_eq!(
            line_color_for_controls(&editor, &defaults.line),
            LineColorConfig::Gradient {
                start: Rgb::new(0x01, 0x02, 0x03),
                end: Rgb::new(0x04, 0x05, 0x06),
                topological_depth: true,
            }
        );
        assert_eq!(
            line_color_for_render(&editor, &editor_generation("F", vec![]), &defaults.line),
            LineColorConfig::Gradient {
                start: Rgb::new(0x01, 0x02, 0x03),
                end: Rgb::new(0x04, 0x05, 0x06),
                topological_depth: false,
            }
        );
    }

    #[test]
    fn line_color_for_render_preserves_topological_gradient_for_bracketed_grammar() {
        let defaults = custom_color_defaults();
        let editor = EditorColorConfig {
            background: None,
            line: Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x01, 0x02, 0x03)),
                end: Some(Rgb::new(0x04, 0x05, 0x06)),
                topological_depth: Some(true),
            }),
        };

        assert_eq!(
            line_color_for_render(
                &editor,
                &editor_generation("F", vec![('F', "F[+F]F")]),
                &defaults.line
            ),
            LineColorConfig::Gradient {
                start: Rgb::new(0x01, 0x02, 0x03),
                end: Rgb::new(0x04, 0x05, 0x06),
                topological_depth: true,
            }
        );
    }

    #[test]
    fn empty_authored_line_modes_resolve_parameters_from_defaults() {
        let defaults = custom_color_defaults();

        assert_eq!(
            line_color_for_controls(
                &EditorColorConfig {
                    background: None,
                    line: Some(EditorLineColorConfig::Solid { color: None }),
                },
                &defaults.line,
            ),
            LineColorConfig::Solid(defaults.line.solid)
        );
        assert_eq!(
            line_color_for_controls(
                &EditorColorConfig {
                    background: None,
                    line: Some(EditorLineColorConfig::Gradient {
                        start: None,
                        end: None,
                        topological_depth: None,
                    }),
                },
                &defaults.line,
            ),
            LineColorConfig::Gradient {
                start: defaults.line.gradient.start,
                end: defaults.line.gradient.end,
                topological_depth: defaults.line.gradient.topological_depth,
            }
        );
        assert_eq!(
            line_color_for_controls(
                &EditorColorConfig {
                    background: None,
                    line: Some(EditorLineColorConfig::HueCycle { initial: None }),
                },
                &defaults.line,
            ),
            LineColorConfig::HueCycle {
                initial: defaults.line.hue_cycle.initial,
            }
        );
    }

    #[test]
    fn color_memory_from_editor_config_initializes_background_from_editor_or_defaults() {
        let defaults = custom_color_defaults();
        let default_memory =
            ColorControlMemory::from_editor_config(&EditorColorConfig::default(), &defaults);
        assert_eq!(default_memory.background(), defaults.background);

        let override_background = Rgb::new(0xf0, 0xe0, 0xd0);
        let override_memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig {
                background: Some(override_background),
                line: None,
            },
            &defaults,
        );
        assert_eq!(override_memory.background(), override_background);
    }

    #[test]
    fn color_memory_inactive_line_slots_use_supplied_defaults() {
        let defaults = custom_color_defaults();
        let hue_active_memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig {
                background: None,
                line: Some(EditorLineColorConfig::HueCycle {
                    initial: Some(Rgb::new(0xf0, 0xe0, 0xd0)),
                }),
            },
            &defaults,
        );

        assert_eq!(
            hue_active_memory.line_for(LineColorMode::Solid),
            LineColorConfig::Solid(defaults.line.solid)
        );
        assert_eq!(
            hue_active_memory.line_for(LineColorMode::Gradient),
            LineColorConfig::Gradient {
                start: defaults.line.gradient.start,
                end: defaults.line.gradient.end,
                topological_depth: defaults.line.gradient.topological_depth,
            }
        );

        let solid_active_memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig {
                background: None,
                line: Some(EditorLineColorConfig::Solid {
                    color: Some(Rgb::new(0xd0, 0xe0, 0xf0)),
                }),
            },
            &defaults,
        );

        assert_eq!(
            solid_active_memory.line_for(LineColorMode::HueCycle),
            LineColorConfig::HueCycle {
                initial: defaults.line.hue_cycle.initial,
            }
        );
    }

    #[test]
    fn line_color_mode_key_round_trip() {
        for mode in [
            LineColorMode::Solid,
            LineColorMode::Gradient,
            LineColorMode::HueCycle,
        ] {
            assert_eq!(LineColorMode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(LineColorMode::ALL.len(), 3);
        assert_eq!(LineColorMode::from_key("unknown"), None);
        assert_eq!(LineColorMode::from_key("depth_gradient"), None);
    }

    #[test]
    fn line_color_mode_from_line_color() {
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::Solid(Rgb::new(0x1a, 0x33, 0x4d))),
            LineColorMode::Solid
        );
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::Gradient {
                start: Rgb::new(0x1a, 0x33, 0x4d),
                end: Rgb::new(0x80, 0x99, 0xb3),
                topological_depth: false,
            }),
            LineColorMode::Gradient
        );
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::HueCycle {
                initial: Rgb::new(0xe5, 0x1a, 0x33),
            }),
            LineColorMode::HueCycle
        );
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::Gradient {
                start: Rgb::new(0x1a, 0x33, 0x4d),
                end: Rgb::new(0x80, 0x99, 0xb3),
                topological_depth: true,
            }),
            LineColorMode::Gradient
        );
    }

    #[test]
    fn color_memory_remembers_solid_across_mode_switch() {
        let solid = LineColorConfig::Solid(Rgb::new(0xff, 0x00, 0x00));
        let mut memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig {
                background: None,
                line: Some(EditorLineColorConfig::Solid {
                    color: Some(Rgb::new(0xff, 0x00, 0x00)),
                }),
            },
            &ConfigDefaults::embedded().colors,
        );
        memory.remember_line(default_gradient());
        assert_eq!(memory.line_for(LineColorMode::Solid), solid);
    }

    #[test]
    fn color_memory_falls_back_to_default_when_slot_unset() {
        let memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig::default(),
            &ConfigDefaults::embedded().colors,
        );
        assert_eq!(memory.line_for(LineColorMode::Gradient), default_gradient());
        assert_eq!(
            memory.line_for(LineColorMode::HueCycle),
            default_hue_cycle()
        );
    }

    #[test]
    fn editor_line_color_preserves_authored_topological_depth() {
        let editor = EditorColorConfig {
            background: None,
            line: Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x1a, 0x33, 0x4d)),
                end: Some(Rgb::new(0x80, 0x99, 0xb3)),
                topological_depth: Some(true),
            }),
        };

        let expected = LineColorConfig::Gradient {
            start: Rgb::new(0x1a, 0x33, 0x4d),
            end: Rgb::new(0x80, 0x99, 0xb3),
            topological_depth: true,
        };
        assert_eq!(
            line_color_for_controls(&editor, &ConfigDefaults::embedded().colors.line),
            expected
        );
        assert_eq!(
            ColorControlMemory::from_editor_config(&editor, &ConfigDefaults::embedded().colors)
                .line_for(LineColorMode::Gradient),
            expected
        );
    }

    #[test]
    fn color_memory_preserves_gradient_topological_depth_flag() {
        let topological = LineColorConfig::Gradient {
            start: Rgb::new(0x1a, 0x33, 0x4d),
            end: Rgb::new(0xb3, 0xcc, 0xe6),
            topological_depth: true,
        };
        let mut memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig {
                background: None,
                line: Some(EditorLineColorConfig::Gradient {
                    start: Some(Rgb::new(0x1a, 0x33, 0x4d)),
                    end: Some(Rgb::new(0xb3, 0xcc, 0xe6)),
                    topological_depth: Some(true),
                }),
            },
            &ConfigDefaults::embedded().colors,
        );
        memory.remember_line(LineColorConfig::HueCycle {
            initial: Rgb::new(0xff, 0x00, 0x00),
        });

        assert_eq!(memory.line_for(LineColorMode::Gradient), topological);
    }

    #[test]
    fn color_memory_background_remembered() {
        let bg = Rgb::new(0x1a, 0x33, 0x4d);
        let mut memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig {
                background: Some(bg),
                line: None,
            },
            &ConfigDefaults::embedded().colors,
        );
        assert_eq!(memory.background(), bg);
        assert_eq!(
            memory.line_for(LineColorMode::Solid),
            LineColorConfig::Solid(ConfigDefaults::embedded().colors.line.solid)
        );
        let new_bg = Rgb::new(0xe5, 0xcc, 0xb3);
        memory.remember_background(new_bg);
        assert_eq!(memory.background(), new_bg);
    }
}
