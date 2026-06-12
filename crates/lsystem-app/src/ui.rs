mod app_state;
mod controls;
mod fractal_canvas;

use iced::Size;
use lsystem_renderer::png_export::{MAX_DIMENSION as PNG_MAX_WIDTH, MIN_WIDTH as PNG_MIN_WIDTH};

use app_state::FractalApp;

const TITLE: &str = "Grow Your Own Fractal";
const CONTROL_WIDTH: f32 = 320.0;

pub fn run() -> iced::Result {
    iced::application(FractalApp::new, FractalApp::update, FractalApp::view)
        .title(app_title)
        .subscription(FractalApp::subscription)
        .window_size(Size::new(1200.0, 800.0))
        .run()
}

fn app_title(_app: &FractalApp) -> String {
    TITLE.to_string()
}
