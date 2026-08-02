#![cfg(target_arch = "wasm32")]

mod app;
mod export;
mod panels;
mod presets;
mod renderer;
// TODO(task-3): drop this `expect` once startup/autosave wiring calls into
// `storage`; until then its `pub` functions are unreachable from outside the
// crate and trigger `dead_code`.
#[expect(dead_code)]
mod storage;
pub(crate) mod ui;

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    let _ = console_log::init_with_level(log::Level::Info);
    leptos::mount::mount_to_body(app::App);
}
