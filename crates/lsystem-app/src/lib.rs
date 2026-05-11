mod export;
mod ui;

#[cfg(not(target_arch = "wasm32"))]
pub fn run_native() {
    env_logger::init();
    ui::run().unwrap();
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    let _ = console_log::init_with_level(log::Level::Info);
    ui::run().unwrap();
}
