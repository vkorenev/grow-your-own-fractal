#[cfg(feature = "png")]
pub mod animation_export;
pub mod camera;
#[allow(dead_code, unused_imports, clippy::all)]
mod generated_shader_2d {
    include!(concat!(env!("OUT_DIR"), "/shader_2d_bindings.rs"));
}
#[allow(dead_code, unused_imports, clippy::all)]
mod generated_shader_3d {
    include!(concat!(env!("OUT_DIR"), "/shader_3d_bindings.rs"));
}
pub mod line_renderer;
pub mod lsystem_bridge;
#[cfg(feature = "png")]
mod offscreen;
#[cfg(feature = "png")]
pub mod png_export;
pub mod scene_upload;
pub mod wgpu_util;
