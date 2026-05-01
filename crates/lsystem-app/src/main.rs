mod camera;
mod input;
mod renderer;
mod ui;

use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::init();

    let event_loop = EventLoop::new().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = renderer::App::new();
    event_loop.run_app(&mut app).unwrap();
}
