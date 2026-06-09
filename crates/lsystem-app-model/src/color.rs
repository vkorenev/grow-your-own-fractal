use lsystem_core::{LineColorConfig, Rgb};

use crate::config_defaults::{ColorDefaults, LineColorDefaults};
use crate::editor_config::{EditorColorConfig, EditorLineColorConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineColorMode {
    Solid,
    Gradient,
    HueCycle,
}

impl LineColorMode {
    pub const ALL: &'static [Self] = &[Self::Solid, Self::Gradient, Self::HueCycle];

    pub fn from_line_color(line_color: &EditorLineColorConfig) -> Self {
        match line_color {
            EditorLineColorConfig::Solid(_) => Self::Solid,
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

#[derive(Clone, Copy, Debug, Default)]
struct RememberedGradient {
    start: Option<Rgb>,
    end: Option<Rgb>,
    topological_depth: Option<bool>,
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
        memory.remember_line(editor.line);
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
        let d = self.line_defaults.gradient;
        let rg = self.gradient.unwrap_or_default();
        (
            rg.start.unwrap_or(d.start),
            rg.end.unwrap_or(d.end),
            rg.topological_depth.unwrap_or(d.topological_depth),
        )
    }

    pub fn hue_cycle_initial(&self) -> Rgb {
        self.hue_cycle
            .unwrap_or(self.line_defaults.hue_cycle.initial)
    }

    pub fn remember_line(&mut self, line_color: Option<EditorLineColorConfig>) {
        match line_color {
            None => self.solid = None,
            Some(EditorLineColorConfig::Solid(color)) => self.solid = Some(color),
            Some(EditorLineColorConfig::Gradient {
                start,
                end,
                topological_depth,
            }) => {
                self.gradient = Some(RememberedGradient {
                    start,
                    end,
                    topological_depth,
                });
            }
            Some(EditorLineColorConfig::HueCycle { initial }) => self.hue_cycle = initial,
        }
    }

    pub fn line_for(&self, mode: LineColorMode) -> Option<EditorLineColorConfig> {
        match mode {
            LineColorMode::Solid => self.solid.map(EditorLineColorConfig::Solid),
            LineColorMode::Gradient => {
                let rg = self.gradient.unwrap_or_default();
                Some(EditorLineColorConfig::Gradient {
                    start: rg.start,
                    end: rg.end,
                    topological_depth: rg.topological_depth,
                })
            }
            LineColorMode::HueCycle => Some(EditorLineColorConfig::HueCycle {
                initial: self.hue_cycle,
            }),
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
        .unwrap_or(LineColorConfig::Solid(defaults.solid))
}

/// Returns the line-color picker mode represented by authored color state.
///
/// When the config omits `colors.line`, the selected mode defaults to `Solid`.
pub fn selected_line_color_mode(editor: &EditorColorConfig) -> LineColorMode {
    editor
        .line
        .as_ref()
        .map(LineColorMode::from_line_color)
        .unwrap_or(LineColorMode::Solid)
}

#[cfg(test)]
mod tests {
    use lsystem_core::{LineColorConfig, Rgb};

    use crate::config_defaults::{ColorDefaults, ConfigDefaults};
    use crate::editor_config::{EditorColorConfig, EditorLineColorConfig};

    use super::{
        ColorControlMemory, LineColorMode, line_color_for_controls, selected_line_color_mode,
    };

    fn custom_color_defaults() -> ColorDefaults {
        let mut defaults = ConfigDefaults::embedded().colors;
        defaults.background = Rgb::new(0x08, 0x10, 0x18);
        defaults.line.solid = Rgb::new(0x10, 0x20, 0x30);
        defaults.line.gradient.start = Rgb::new(0x40, 0x50, 0x60);
        defaults.line.gradient.end = Rgb::new(0x70, 0x80, 0x90);
        defaults.line.gradient.topological_depth = true;
        defaults.line.hue_cycle.initial = Rgb::new(0xa0, 0xb0, 0xc0);
        defaults
    }

    #[test]
    fn missing_editor_line_uses_solid_color_for_controls() {
        let defaults = custom_color_defaults();
        assert_eq!(
            line_color_for_controls(&EditorColorConfig::default(), &defaults.line),
            LineColorConfig::Solid(defaults.line.solid)
        );
    }

    #[test]
    fn selected_line_color_mode_uses_solid_when_line_is_absent() {
        assert_eq!(
            selected_line_color_mode(&EditorColorConfig::default()),
            LineColorMode::Solid
        );
    }

    #[test]
    fn selected_line_color_mode_uses_authored_mode_when_present() {
        let editor = EditorColorConfig {
            background: None,
            line: Some(EditorLineColorConfig::HueCycle { initial: None }),
        };

        assert_eq!(selected_line_color_mode(&editor), LineColorMode::HueCycle);
    }

    #[test]
    fn empty_authored_line_modes_resolve_parameters_from_defaults() {
        let defaults = custom_color_defaults();

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
    fn color_memory_inactive_line_slots_use_none_for_defaults() {
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

        assert_eq!(hue_active_memory.line_for(LineColorMode::Solid), None);
        assert_eq!(
            hue_active_memory.line_for(LineColorMode::Gradient),
            Some(EditorLineColorConfig::Gradient {
                start: None,
                end: None,
                topological_depth: None,
            })
        );

        let solid_active_memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig {
                background: None,
                line: Some(EditorLineColorConfig::Solid(Rgb::new(0xd0, 0xe0, 0xf0))),
            },
            &defaults,
        );

        assert_eq!(
            solid_active_memory.line_for(LineColorMode::HueCycle),
            Some(EditorLineColorConfig::HueCycle { initial: None })
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
    fn color_memory_remembers_solid_across_mode_switch() {
        let solid = Some(EditorLineColorConfig::Solid(Rgb::new(0xff, 0x00, 0x00)));
        let mut memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig {
                background: None,
                line: Some(EditorLineColorConfig::Solid(Rgb::new(0xff, 0x00, 0x00))),
            },
            &ConfigDefaults::embedded().colors,
        );
        memory.remember_line(Some(EditorLineColorConfig::Gradient {
            start: None,
            end: None,
            topological_depth: None,
        }));
        assert_eq!(memory.line_for(LineColorMode::Solid), solid);
    }

    #[test]
    fn color_memory_unvisited_slots_return_all_none() {
        let memory = ColorControlMemory::from_editor_config(
            &EditorColorConfig::default(),
            &ConfigDefaults::embedded().colors,
        );
        assert_eq!(memory.line_for(LineColorMode::Solid), None);
        assert_eq!(
            memory.line_for(LineColorMode::Gradient),
            Some(EditorLineColorConfig::Gradient {
                start: None,
                end: None,
                topological_depth: None,
            })
        );
        assert_eq!(
            memory.line_for(LineColorMode::HueCycle),
            Some(EditorLineColorConfig::HueCycle { initial: None })
        );
    }

    #[test]
    fn line_color_preserves_authored_topological_depth() {
        let editor = EditorColorConfig {
            background: None,
            line: Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x1a, 0x33, 0x4d)),
                end: Some(Rgb::new(0x80, 0x99, 0xb3)),
                topological_depth: Some(true),
            }),
        };

        assert_eq!(
            line_color_for_controls(&editor, &ConfigDefaults::embedded().colors.line),
            LineColorConfig::Gradient {
                start: Rgb::new(0x1a, 0x33, 0x4d),
                end: Rgb::new(0x80, 0x99, 0xb3),
                topological_depth: true,
            }
        );
        assert_eq!(
            ColorControlMemory::from_editor_config(&editor, &ConfigDefaults::embedded().colors)
                .line_for(LineColorMode::Gradient),
            Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x1a, 0x33, 0x4d)),
                end: Some(Rgb::new(0x80, 0x99, 0xb3)),
                topological_depth: Some(true),
            })
        );
    }

    #[test]
    fn color_memory_preserves_gradient_topological_depth_flag() {
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
        memory.remember_line(Some(EditorLineColorConfig::HueCycle {
            initial: Some(Rgb::new(0xff, 0x00, 0x00)),
        }));

        assert_eq!(
            memory.line_for(LineColorMode::Gradient),
            Some(EditorLineColorConfig::Gradient {
                start: Some(Rgb::new(0x1a, 0x33, 0x4d)),
                end: Some(Rgb::new(0xb3, 0xcc, 0xe6)),
                topological_depth: Some(true),
            })
        );
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
        assert_eq!(memory.line_for(LineColorMode::Solid), None);
        let new_bg = Rgb::new(0xe5, 0xcc, 0xb3);
        memory.remember_background(new_bg);
        assert_eq!(memory.background(), new_bg);
    }
}
