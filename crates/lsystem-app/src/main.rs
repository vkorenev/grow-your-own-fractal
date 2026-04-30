mod camera;
mod input;
mod renderer;

use lsystem_core::Config;
use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::init();

    let config = Config::parse(include_str!("../../../presets/koch_snowflake.toml"))
        .expect("bundled preset is valid");
    let geometry = lsystem_core::generate(&config);

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = renderer::App::new(geometry);
    event_loop.run_app(&mut app).unwrap();
}
