fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    lsystem_app::run_native();
}
