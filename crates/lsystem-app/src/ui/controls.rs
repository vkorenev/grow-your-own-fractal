use iced::widget::{
    button, column, container, pick_list, row, scrollable, slider, text, text_editor, text_input,
};
use iced::{Color, Element, Length, Theme};

use super::app_state::{FractalApp, Message};
use super::{CONTROL_WIDTH, TITLE};

impl FractalApp {
    pub(super) fn controls(&self) -> Element<'_, Message> {
        let preset_names: Vec<String> = self
            .presets
            .iter()
            .map(|preset| preset.name.clone())
            .collect();

        let mut controls = column![
            text(TITLE).size(24),
            text("Preset").size(13),
            pick_list(self.selected_preset.clone(), preset_names, String::clone)
                .on_select(Message::PresetSelected)
                .width(Length::Fill),
            text("Config (TOML)").size(13),
            text_editor(&self.toml)
                .height(260)
                .on_action(Message::TomlEdited),
            button("Apply").on_press(Message::ApplyConfig),
            self.status_text(),
        ]
        .spacing(10);

        let is_3d = self.scene.is_3d();

        if self.base_config.is_some() {
            controls = controls
                .push(text("Overrides").size(13))
                .push(text(format!("Iterations: {}", self.iterations)))
                .push(slider(
                    0..=self.max_iterations,
                    self.iterations,
                    Message::IterationsChanged,
                ))
                .push(text(format!("Angle: {:.1}", self.angle)))
                .push(slider(1.0..=180.0, self.angle, Message::AngleChanged).step(0.5))
                .push(text("PNG width").size(13))
                .push(
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
