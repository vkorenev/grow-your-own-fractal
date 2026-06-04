use lsystem_core::{ColorConfig, LineColorConfig, Rgb};

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
    pub fn from_colors(colors: &ColorConfig) -> Self {
        let mut memory = Self {
            background: colors.background.unwrap_or(ColorConfig::DEFAULT_BACKGROUND),
            solid: None,
            gradient: None,
            hue_cycle: None,
        };
        memory.remember_line(colors.line);
        memory
    }

    pub fn background(&self) -> Rgb {
        self.background
    }

    pub fn remember_background(&mut self, background: Rgb) {
        self.background = background;
    }

    pub fn solid_color(&self) -> Rgb {
        self.solid.unwrap_or(Rgb::DEFAULT_SOLID_LINE)
    }

    pub fn gradient_fields(&self) -> (Rgb, Rgb, bool) {
        let gradient = self.gradient.unwrap_or(RememberedGradient {
            start: Rgb::DEFAULT_GRADIENT_START,
            end: Rgb::DEFAULT_GRADIENT_END,
            topological_depth: false,
        });
        (gradient.start, gradient.end, gradient.topological_depth)
    }

    pub fn hue_cycle_initial(&self) -> Rgb {
        self.hue_cycle.unwrap_or(Rgb::DEFAULT_HUE_CYCLE_INITIAL)
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

#[cfg(test)]
mod tests {
    use lsystem_core::{ColorConfig, LineColorConfig, Rgb};

    use super::{ColorControlMemory, LineColorMode};

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
            LineColorMode::from_line_color(&LineColorConfig::DEFAULT_SOLID),
            LineColorMode::Solid
        );
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::DEFAULT_GRADIENT),
            LineColorMode::Gradient
        );
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::DEFAULT_HUE_CYCLE),
            LineColorMode::HueCycle
        );
        assert_eq!(
            LineColorMode::from_line_color(&LineColorConfig::DEFAULT_TOPOLOGICAL_GRADIENT),
            LineColorMode::Gradient
        );
    }

    #[test]
    fn color_memory_remembers_solid_across_mode_switch() {
        let solid = LineColorConfig::Solid(Rgb::new(0xff, 0x00, 0x00));
        let mut memory = ColorControlMemory::from_colors(&ColorConfig {
            background: None,
            line: solid,
        });
        memory.remember_line(LineColorConfig::DEFAULT_GRADIENT);
        assert_eq!(memory.line_for(LineColorMode::Solid), solid);
    }

    #[test]
    fn color_memory_falls_back_to_default_when_slot_unset() {
        let memory = ColorControlMemory::from_colors(&ColorConfig {
            background: None,
            line: LineColorConfig::DEFAULT_SOLID,
        });
        assert_eq!(
            memory.line_for(LineColorMode::Gradient),
            LineColorConfig::DEFAULT_GRADIENT
        );
        assert_eq!(
            memory.line_for(LineColorMode::HueCycle),
            LineColorConfig::DEFAULT_HUE_CYCLE
        );
    }

    #[test]
    fn color_memory_preserves_gradient_topological_depth_flag() {
        let topological = LineColorConfig::Gradient {
            start: Rgb::new(0x1a, 0x33, 0x4d),
            end: Rgb::new(0xb3, 0xcc, 0xe6),
            topological_depth: true,
        };
        let mut memory = ColorControlMemory::from_colors(&ColorConfig {
            background: None,
            line: topological,
        });
        memory.remember_line(LineColorConfig::HueCycle {
            initial: Rgb::new(0xff, 0x00, 0x00),
        });

        assert_eq!(memory.line_for(LineColorMode::Gradient), topological);
    }

    #[test]
    fn color_memory_background_remembered() {
        let bg = Rgb::new(0x1a, 0x33, 0x4d);
        let mut memory = ColorControlMemory::from_colors(&ColorConfig {
            background: Some(bg),
            line: LineColorConfig::DEFAULT_SOLID,
        });
        assert_eq!(memory.background(), bg);
        let new_bg = Rgb::new(0xe5, 0xcc, 0xb3);
        memory.remember_background(new_bg);
        assert_eq!(memory.background(), new_bg);
    }
}
