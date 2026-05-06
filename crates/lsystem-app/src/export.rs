use std::sync::Arc;

use lsystem_core::Config;

pub(crate) enum ExportRequest {
    Svg(Config),
    Png { config: Config, width: u32 },
}

pub(crate) fn handle_export(
    request: ExportRequest,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
) {
    match request {
        ExportRequest::Svg(cfg) => {
            let filename = sanitize_filename(&cfg.name, "svg");
            let svg = lsystem_core::svg_export::export_svg(&cfg);
            save_svg(svg, filename);
        }
        ExportRequest::Png { config, width } => {
            let filename = sanitize_filename(&config.name, "png");
            save_png(device, queue, config, width, filename);
        }
    }
}

fn sanitize_filename(name: &str, extension: &str) -> String {
    let base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{base}.{extension}")
}

#[cfg(not(target_arch = "wasm32"))]
fn save_svg(svg: String, suggested_name: String) {
    if let Some(path) = rfd::FileDialog::new()
        .set_file_name(&suggested_name)
        .add_filter("SVG Image", &["svg"])
        .save_file()
        && let Err(e) = std::fs::write(&path, svg.as_bytes())
    {
        log::error!("Failed to write SVG: {e}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn save_png(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    cfg: Config,
    width: u32,
    suggested_name: String,
) {
    if let Some(path) = rfd::FileDialog::new()
        .set_file_name(&suggested_name)
        .add_filter("PNG Image", &["png"])
        .save_file()
    {
        match pollster::block_on(lsystem_renderer::png_export::render_png(
            &device, &queue, &cfg, width,
        )) {
            Ok(png) => {
                if let Err(e) = std::fs::write(&path, png.bytes) {
                    log::error!("Failed to write PNG: {e}");
                }
            }
            Err(e) => {
                log::error!("Failed to export PNG: {e}");
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn save_svg(svg: String, suggested_name: String) {
    let array = js_sys::Array::new();
    array.push(&wasm_bindgen::JsValue::from_str(&svg));
    let props = web_sys::BlobPropertyBag::new();
    props.set_type("image/svg+xml");
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&array, &props) else {
        return;
    };
    download_blob(blob, suggested_name);
}

#[cfg(target_arch = "wasm32")]
fn save_png(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    cfg: Config,
    width: u32,
    suggested_name: String,
) {
    wasm_bindgen_futures::spawn_local(async move {
        match lsystem_renderer::png_export::render_png(&device, &queue, &cfg, width).await {
            Ok(png) => {
                let array = js_sys::Array::new();
                let bytes = js_sys::Uint8Array::from(png.bytes.as_slice());
                array.push(&bytes);
                let props = web_sys::BlobPropertyBag::new();
                props.set_type("image/png");
                let Ok(blob) =
                    web_sys::Blob::new_with_u8_array_sequence_and_options(&array, &props)
                else {
                    return;
                };
                download_blob(blob, suggested_name);
            }
            Err(e) => {
                log::error!("Failed to export PNG: {e}");
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn download_blob(blob: web_sys::Blob, suggested_name: String) {
    use wasm_bindgen::JsCast;

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };

    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };

    let Ok(el) = document.create_element("a") else {
        return;
    };
    let Ok(anchor) = el.dyn_into::<web_sys::HtmlAnchorElement>() else {
        return;
    };
    anchor.set_href(&url);
    anchor.set_download(&suggested_name);
    // Append to body so click() works in Firefox, then remove immediately after.
    if let Some(body) = document.body() {
        let _ = body.append_child(&anchor);
        anchor.click();
        let _ = body.remove_child(&anchor);
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}
