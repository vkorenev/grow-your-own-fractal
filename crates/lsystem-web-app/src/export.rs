use std::sync::Arc;

use lsystem_app_model::sanitize_filename;
use lsystem_core::Config;
use lsystem_renderer::animation_export::AnimationParams;
use lsystem_renderer::camera::Camera;
use wasm_bindgen::JsCast;

pub(crate) fn export_svg(config: Config) {
    let filename = sanitize_filename(&config.name, "svg");
    let svg = lsystem_core::svg_export::export_svg(&config);
    let blob = gloo_file::Blob::new_with_options(svg.as_str(), Some("image/svg+xml"));
    download_blob(blob, filename);
}

pub(crate) fn export_png(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    camera: Camera,
    config: Config,
    width: u32,
    height: u32,
    on_error: impl Fn(String) + 'static,
) {
    let filename = sanitize_filename(&config.name, "png");
    wasm_bindgen_futures::spawn_local(async move {
        match lsystem_renderer::png_export::render_png(
            &device, &queue, &config, width, height, &camera,
        )
        .await
        {
            Ok(png) => {
                let blob =
                    gloo_file::Blob::new_with_options(png.bytes.as_slice(), Some("image/png"));
                download_blob(blob, filename);
            }
            Err(err) => {
                let error = format!("Failed to export PNG: {err}");
                log::error!("{error}");
                on_error(error);
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn export_animation(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    camera: Camera,
    config: Config,
    width: u32,
    height: u32,
    params: AnimationParams,
    on_progress: impl Fn(u32, u32) + 'static,
    on_done: impl Fn(Option<String>) + 'static,
) {
    let filename = sanitize_filename(&config.name, "apng");
    wasm_bindgen_futures::spawn_local(async move {
        match lsystem_renderer::animation_export::render_animation(
            &device,
            &queue,
            &config,
            width,
            height,
            &camera,
            &params,
            &on_progress,
        )
        .await
        {
            Ok(png) => {
                let blob =
                    gloo_file::Blob::new_with_options(png.bytes.as_slice(), Some("image/apng"));
                download_blob(blob, filename);
                on_done(None);
            }
            Err(err) => {
                let msg = format!("Failed to export APNG: {err}");
                log::error!("{msg}");
                on_done(Some(msg));
            }
        }
    });
}

pub(crate) fn download_toml(name: &str, text: &str) {
    let filename = sanitize_filename(name, "toml");
    let blob = gloo_file::Blob::new_with_options(text, Some("application/toml"));
    download_blob(blob, filename);
}

fn download_blob(blob: gloo_file::Blob, suggested_name: String) {
    let Some(window) = web_sys::window() else {
        log::error!("Cannot download export: window is unavailable");
        return;
    };
    let Some(document) = window.document() else {
        log::error!("Cannot download export: document is unavailable");
        return;
    };
    let url = gloo_file::ObjectUrl::from(blob);
    let el = match document.create_element("a") {
        Ok(el) => el,
        Err(err) => {
            log::error!("Failed to create export download link: {err:?}");
            return;
        }
    };
    let anchor = match el.dyn_into::<web_sys::HtmlAnchorElement>() {
        Ok(anchor) => anchor,
        Err(err) => {
            log::error!("Export download link was not an anchor element: {err:?}");
            return;
        }
    };
    anchor.set_href(&url);
    anchor.set_download(&suggested_name);
    if let Some(body) = document.body() {
        let _ = body.append_child(&anchor);
        anchor.click();
        let _ = body.remove_child(&anchor);
    }
}
