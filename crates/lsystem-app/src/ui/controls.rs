use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, slider, text, text_editor,
    text_input,
};
use iced::{Color, Element, Length, Theme};
use lsystem_core::LineColorConfig;

use super::app_state::{FractalApp, LineColorMode, Message};
use super::{CONTROL_WIDTH, TITLE};

impl FractalApp {
    pub(super) fn controls(&self) -> Element<'_, Message> {
        let preset_names: Vec<String> = self.config_workspace.names().map(str::to_string).collect();
        let selected_entry = self.config_workspace.selected();
        let selected_preset = Some(selected_entry.name().to_string());
        let is_dirty = selected_entry.is_dirty();
        let can_reset = !is_dirty && self.config_workspace.can_reset();

        let mut controls = column![
            text(TITLE).size(24),
            text("Config").size(13),
            pick_list(selected_preset, preset_names, String::clone)
                .on_select(Message::PresetSelected)
                .width(Length::Fill),
            button("Copy").on_press(Message::CopyConfig),
            text("Config (TOML)").size(13),
            text_editor(&self.toml)
                .height(260)
                .on_action(Message::TomlEdited),
            row![
                button("Apply").on_press_maybe(is_dirty.then_some(Message::ApplyConfig)),
                button("Revert").on_press_maybe(is_dirty.then_some(Message::RevertConfig)),
                button("Reset").on_press_maybe(can_reset.then_some(Message::ResetConfig)),
            ]
            .spacing(8),
            self.status_text(),
        ]
        .spacing(10);

        let is_3d = self.effective_is_3d();

        if is_dirty {
            controls = controls
                .push(text("Apply or Revert the edited config before using controls.").size(13));
        } else {
            let config = self.selected_config();
            controls = controls
                .push(text("Overrides").size(13))
                .push(text(format!("Iterations: {}", self.iterations)))
                .push(slider(
                    0..=self.max_iterations,
                    self.iterations,
                    Message::IterationsChanged,
                ))
                .push(text(format!("Angle: {:.1}", config.generation.angle)))
                .push(
                    slider(1.0..=180.0, config.generation.angle, Message::AngleChanged).step(0.5),
                );

            controls = push_color_controls(controls, config);

            controls = controls.push(text("PNG width").size(13)).push(
                text_input("2048", &self.png_width_text)
                    .on_input(Message::PngWidthChanged)
                    .width(Length::Fill),
            );

            let mut export_row = row![button("Export PNG").on_press(Message::ExportPng)].spacing(8);
            if !is_3d {
                export_row = export_row.push(button("Export SVG").on_press(Message::ExportSvg));
            }
            controls = controls.push(export_row);

            if is_3d {
                let auto_rotate_label = if self.auto_rotate {
                    "Auto-rotate: On"
                } else {
                    "Auto-rotate: Off"
                };
                controls = controls
                    .push(button(auto_rotate_label).on_press(Message::ToggleAutoRotate))
                    .push(text(format!("Speed: {:.0} °/s", self.auto_rotate_speed)).size(13))
                    .push(
                        slider(
                            10.0..=360.0,
                            self.auto_rotate_speed,
                            Message::SetAutoRotateSpeed,
                        )
                        .step(10.0),
                    );
            }
        }

        if let Some(status) = &self.export_status {
            controls = controls.push(text(status).size(13));
        }

        if self.scene_pending {
            controls = controls.push(text("Rendering...").size(13));
        }

        let hint = if is_3d {
            "Drag to orbit · Scroll to zoom · F to fit\nArrows to rotate · Q/E to roll"
        } else {
            "Drag to pan · Scroll to zoom · F to fit"
        };
        controls = controls.push(text(hint).size(12));

        container(scrollable(controls.padding(16).spacing(12)))
            .width(CONTROL_WIDTH)
            .height(Length::Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(theme.palette().background.base.color.into()),
                ..Default::default()
            })
            .into()
    }

    fn status_text(&self) -> Element<'_, Message> {
        match &self.error {
            Some(error) => text(error)
                .size(13)
                .color(Color::from_rgb(0.9, 0.2, 0.2))
                .into(),
            None => text("OK")
                .size(13)
                .color(Color::from_rgb(0.2, 0.65, 0.25))
                .into(),
        }
    }
}

fn push_color_controls<'a>(
    mut controls: iced::widget::Column<'a, Message>,
    config: &'a lsystem_core::Config,
) -> iced::widget::Column<'a, Message> {
    let background = config.colors.background;
    controls = controls.push(
        checkbox(background.is_some())
            .label("Background")
            .on_toggle(Message::BackgroundOverrideToggled),
    );
    if let Some(color) = background {
        controls = controls.push(rgb_controls(
            "Background RGB",
            color,
            Message::BackgroundColorChanged,
        ));
    }

    let line_color = &config.colors.line;
    let selected_mode = Some(LineColorMode::from_line_color(line_color));
    controls = controls.push(text("Line color").size(13)).push(
        pick_list(selected_mode, LineColorMode::ALL, |choice| {
            choice.to_string()
        })
        .on_select(Message::LineColorModeSelected)
        .width(Length::Fill),
    );

    match *line_color {
        LineColorConfig::Solid { color } => {
            controls.push(rgb_controls("Line RGB", color, |color| {
                Message::LineColorChanged(LineColorConfig::Solid { color })
            }))
        }
        LineColorConfig::Gradient { start, end } => controls
            .push(rgb_controls("Gradient start", start, move |start| {
                Message::LineColorChanged(LineColorConfig::Gradient { start, end })
            }))
            .push(rgb_controls("Gradient end", end, move |end| {
                Message::LineColorChanged(LineColorConfig::Gradient { start, end })
            })),
        LineColorConfig::HueCycle { initial } => {
            controls.push(rgb_controls("Initial RGB", initial, |initial| {
                Message::LineColorChanged(LineColorConfig::HueCycle { initial })
            }))
        }
    }
}

fn rgb_controls<'a>(
    label: &'a str,
    color: [f32; 3],
    message: impl Fn([f32; 3]) -> Message + Clone + 'a,
) -> Element<'a, Message> {
    column![
        row![text(label).size(13), color_swatch(color)].spacing(8),
        color_slider("R", color, 0, message.clone()),
        color_slider("G", color, 1, message.clone()),
        color_slider("B", color, 2, message),
    ]
    .spacing(6)
    .into()
}

fn color_slider<'a>(
    label: &'a str,
    color: [f32; 3],
    component: usize,
    message: impl Fn([f32; 3]) -> Message + 'a,
) -> Element<'a, Message> {
    row![
        text(label).size(12).width(Length::Fixed(16.0)),
        slider(0.0..=1.0, color[component], move |value| {
            let mut next = color;
            next[component] = value;
            message(next)
        })
        .step(0.01),
        text(format!("{:.0}", color[component] * 255.0))
            .size(12)
            .width(Length::Fixed(34.0)),
    ]
    .spacing(6)
    .align_y(iced::alignment::Vertical::Center)
    .into()
}

fn color_swatch(color: [f32; 3]) -> Element<'static, Message> {
    let [r, g, b] = color;
    container(text(""))
        .width(Length::Fixed(28.0))
        .height(Length::Fixed(18.0))
        .style(move |_theme: &Theme| container::Style {
            background: Some(Color::from_rgb(r, g, b).into()),
            border: iced::Border {
                color: Color::from_rgb(0.45, 0.5, 0.58),
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        })
        .into()
}
