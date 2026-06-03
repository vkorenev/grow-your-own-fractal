use lsystem_core::{ColorConfig, HexColor, LineColorConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineColorMode {
    Solid,
    Gradient,
    HueCycle,
    DepthGradient,
}

impl LineColorMode {
    pub const ALL: &'static [Self] = &[
        Self::Solid,
        Self::Gradient,
        Self::HueCycle,
        Self::DepthGradient,
    ];

    pub fn from_line_color(line_color: &LineColorConfig) -> Self {
        match line_color {
            LineColorConfig::Solid { .. } => Self::Solid,
            LineColorConfig::Gradient { .. } => Self::Gradient,
            LineColorConfig::HueCycle { .. } => Self::HueCycle,
            LineColorConfig::DepthGradient { .. } => Self::DepthGradient,
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "solid" => Some(Self::Solid),
            "gradient" => Some(Self::Gradient),
            "hue_cycle" => Some(Self::HueCycle),
            "depth_gradient" => Some(Self::DepthGradient),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Gradient => "gradient",
            Self::HueCycle => "hue_cycle",
            Self::DepthGradient => "depth_gradient",
        }
    }

    pub(crate) fn default_line_color(self) -> LineColorConfig {
        match self {
            Self::Solid => LineColorConfig::DEFAULT_SOLID,
            Self::Gradient => LineColorConfig::DEFAULT_GRADIENT,
            Self::HueCycle => LineColorConfig::DEFAULT_HUE_CYCLE,
            Self::DepthGradient => LineColorConfig::DEFAULT_DEPTH_GRADIENT,
        }
    }
}

impl std::fmt::Display for LineColorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Solid => "Solid",
            Self::Gradient => "Gradient",
            Self::HueCycle => "Hue cycle",
            Self::DepthGradient => "Depth gradient",
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ColorControlMemory {
    background: HexColor,
    solid: Option<HexColor>,
    gradient: Option<(HexColor, HexColor)>,
    hue_cycle: Option<HexColor>,
    depth_gradient: Option<(HexColor, HexColor)>,
}

impl ColorControlMemory {
    pub fn from_colors(colors: &ColorConfig) -> Self {
        let mut memory = Self {
            background: colors.background.unwrap_or(ColorConfig::DEFAULT_BACKGROUND),
            solid: None,
            gradient: None,
            hue_cycle: None,
            depth_gradient: None,
        };
        memory.remember_line(colors.line);
        memory
    }

    pub fn background(&self) -> HexColor {
        self.background
    }

    pub fn remember_background(&mut self, background: HexColor) {
        self.background = background;
    }

    pub fn remember_line(&mut self, line_color: LineColorConfig) {
        match line_color {
            LineColorConfig::Solid { color } => self.solid = Some(color),
            LineColorConfig::Gradient { start, end } => self.gradient = Some((start, end)),
            LineColorConfig::HueCycle { initial } => self.hue_cycle = Some(initial),
            LineColorConfig::DepthGradient { start, end } => {
                self.depth_gradient = Some((start, end));
            }
        }
    }

    pub fn line_for(&self, mode: LineColorMode) -> LineColorConfig {
        match mode {
            LineColorMode::Solid => self
                .solid
                .map(|color| LineColorConfig::Solid { color })
                .unwrap_or_else(|| mode.default_line_color()),
            LineColorMode::Gradient => self
                .gradient
                .map(|(start, end)| LineColorConfig::Gradient { start, end })
                .unwrap_or_else(|| mode.default_line_color()),
            LineColorMode::HueCycle => self
                .hue_cycle
                .map(|initial| LineColorConfig::HueCycle { initial })
                .unwrap_or_else(|| mode.default_line_color()),
            LineColorMode::DepthGradient => self
                .depth_gradient
                .map(|(start, end)| LineColorConfig::DepthGradient { start, end })
                .unwrap_or_else(|| mode.default_line_color()),
        }
    }
}

#[cfg(test)]
mod tests {
    use lsystem_core::{ColorConfig, HexColor, LineColorConfig};

    use super::{ColorControlMemory, LineColorMode};

    #[test]
    fn line_color_mode_key_round_trip() {
        for mode in [
            LineColorMode::Solid,
            LineColorMode::Gradient,
            LineColorMode::HueCycle,
            LineColorMode::DepthGradient,
        ] {
            assert_eq!(LineColorMode::from_key(mode.key()), Some(mode));
        }
        assert_eq!(LineColorMode::from_key("unknown"), None);
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
            LineColorMode::from_line_color(&LineColorConfig::DEFAULT_DEPTH_GRADIENT),
            LineColorMode::DepthGradient
        );
    }

    #[test]
    fn color_memory_remembers_solid_across_mode_switch() {
        let solid = LineColorConfig::Solid {
            color: HexColor::new(0xff, 0x00, 0x00),
        };
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
            LineColorMode::Gradient.default_line_color()
        );
        assert_eq!(
            memory.line_for(LineColorMode::HueCycle),
            LineColorMode::HueCycle.default_line_color()
        );
        assert_eq!(
            memory.line_for(LineColorMode::DepthGradient),
            LineColorMode::DepthGradient.default_line_color()
        );
    }

    #[test]
    fn color_memory_background_remembered() {
        let bg = HexColor::new(0x1a, 0x33, 0x4d);
        let mut memory = ColorControlMemory::from_colors(&ColorConfig {
            background: Some(bg),
            line: LineColorConfig::DEFAULT_SOLID,
        });
        assert_eq!(memory.background(), bg);
        let new_bg = HexColor::new(0xe5, 0xcc, 0xb3);
        memory.remember_background(new_bg);
        assert_eq!(memory.background(), new_bg);
    }
}
