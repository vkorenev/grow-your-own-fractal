#![cfg(target_arch = "wasm32")]

mod app;
mod color_input;
mod export;
mod presets;
mod renderer;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    let _ = console_log::init_with_level(log::Level::Info);
    leptos::mount::mount_to_body(app::App);
}
