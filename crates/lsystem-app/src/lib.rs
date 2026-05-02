mod camera;
mod fractal_renderer;
mod input;
mod renderer;
mod ui;

use winit::event_loop::{ControlFlow, EventLoop};

use crate::renderer::{App, UserEvent};

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() {
    env_logger::init();
    let event_loop = build_event_loop();
    let mut app = App::new(event_loop.create_proxy());
    event_loop.run_app(&mut app).unwrap();
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    let _ = console_log::init_with_level(log::Level::Info);
    spawn_event_loop();
}

#[cfg(target_arch = "wasm32")]
fn spawn_event_loop() {
    use winit::platform::web::EventLoopExtWebSys;

    let event_loop = build_event_loop();
    let app = App::new(event_loop.create_proxy());
    event_loop.spawn_app(app);
}

fn build_event_loop() -> EventLoop<UserEvent> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().unwrap();
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop
}

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::JsCast;

    pub fn show_unsupported_overlay() {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        if let Some(canvas) = document
            .get_element_by_id("lsystem-canvas")
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = canvas.style().set_property("display", "none");
        }
        if let Some(overlay) = document
            .get_element_by_id("unsupported")
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = overlay.style().set_property("display", "flex");
        }
    }
}
