use std::sync::Arc;

use lsystem_app_model::sanitize_filename;
use lsystem_core::Config;
use lsystem_renderer::camera::Camera;
use wasm_bindgen::JsCast;

pub(crate) fn export_svg(config: Config) {
    let filename = sanitize_filename(&config.name, "svg");
    let svg = lsystem_core::svg_export::export_svg(&config);
    let array = js_sys::Array::new();
    array.push(&wasm_bindgen::JsValue::from_str(&svg));
    let props = web_sys::BlobPropertyBag::new();
    props.set_type("image/svg+xml");
    match web_sys::Blob::new_with_str_sequence_and_options(&array, &props) {
        Ok(blob) => download_blob(blob, filename),
        Err(err) => log::error!("Failed to create SVG export blob: {err:?}"),
    }
}

pub(crate) fn export_png(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    camera: Camera,
    config: Config,
    width: u32,
    on_error: impl Fn(String) + 'static,
) {
    let filename = sanitize_filename(&config.name, "png");
    wasm_bindgen_futures::spawn_local(async move {
        match lsystem_renderer::png_export::render_png(&device, &queue, &config, width, &camera)
            .await
        {
            Ok(png) => {
                let array = js_sys::Array::new();
                let bytes = js_sys::Uint8Array::from(png.bytes.as_slice());
                array.push(&bytes);
                let props = web_sys::BlobPropertyBag::new();
                props.set_type("image/png");
                match web_sys::Blob::new_with_u8_array_sequence_and_options(&array, &props) {
                    Ok(blob) => download_blob(blob, filename),
                    Err(err) => log::error!("Failed to create PNG export blob: {err:?}"),
                }
            }
            Err(err) => {
                let error = format!("Failed to export PNG: {err}");
                log::error!("{error}");
                on_error(error);
            }
        }
    });
}

fn download_blob(blob: web_sys::Blob, suggested_name: String) {
    let Some(window) = web_sys::window() else {
        log::error!("Cannot download export: window is unavailable");
        return;
    };
    let Some(document) = window.document() else {
        log::error!("Cannot download export: document is unavailable");
        return;
    };
    let url = match web_sys::Url::create_object_url_with_blob(&blob) {
        Ok(url) => url,
        Err(err) => {
            log::error!("Failed to create export object URL: {err:?}");
            return;
        }
    };
    let el = match document.create_element("a") {
        Ok(el) => el,
        Err(err) => {
            log::error!("Failed to create export download link: {err:?}");
            let _ = web_sys::Url::revoke_object_url(&url);
            return;
        }
    };
    let anchor = match el.dyn_into::<web_sys::HtmlAnchorElement>() {
        Ok(anchor) => anchor,
        Err(err) => {
            log::error!("Export download link was not an anchor element: {err:?}");
            let _ = web_sys::Url::revoke_object_url(&url);
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
    let _ = web_sys::Url::revoke_object_url(&url);
}
