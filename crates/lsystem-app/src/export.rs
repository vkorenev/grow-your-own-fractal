use lsystem_app_model::sanitize_filename;
use lsystem_core::Config;
use lsystem_renderer::camera::Camera;

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ExportKind {
    Svg,
    Png,
}

impl ExportKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn label(self) -> &'static str {
        match self {
            Self::Svg => "SVG",
            Self::Png => "PNG",
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn filter_name(self) -> &'static str {
        match self {
            Self::Svg => "SVG Image",
            Self::Png => "PNG Image",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ExportRequest {
    #[cfg(not(target_arch = "wasm32"))]
    Svg { config: Config, path: PathBuf },
    #[cfg(not(target_arch = "wasm32"))]
    Png {
        config: Config,
        width: u32,
        path: PathBuf,
        camera: Camera,
    },
    #[cfg(target_arch = "wasm32")]
    Svg(Config),
    #[cfg(target_arch = "wasm32")]
    Png {
        config: Config,
        width: u32,
        camera: Camera,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum ExportOutcome {
    Saved(&'static str),
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Cancelled,
    Failed(String),
}

pub(crate) async fn handle_export(request: ExportRequest) -> ExportOutcome {
    match request {
        #[cfg(not(target_arch = "wasm32"))]
        ExportRequest::Svg { config, path } => {
            let svg = lsystem_core::svg_export::export_svg(&config);
            save_svg(svg, path)
        }
        #[cfg(not(target_arch = "wasm32"))]
        ExportRequest::Png {
            config,
            width,
            path,
            camera,
        } => save_png(config, width, path, camera).await,
        #[cfg(target_arch = "wasm32")]
        ExportRequest::Svg(config) => {
            let filename = suggested_filename(&config.name, ExportKind::Svg);
            let svg = lsystem_core::svg_export::export_svg(&config);
            save_svg(svg, filename)
        }
        #[cfg(target_arch = "wasm32")]
        ExportRequest::Png {
            config,
            width,
            camera,
        } => {
            let filename = suggested_filename(&config.name, ExportKind::Png);
            save_png(config, width, camera, filename).await
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn choose_export_path(name: &str, kind: ExportKind) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_file_name(suggested_filename(name, kind))
        .add_filter(kind.filter_name(), &[kind.extension()])
        .save_file()
}

fn suggested_filename(name: &str, kind: ExportKind) -> String {
    sanitize_filename(name, kind.extension())
}

#[cfg(not(target_arch = "wasm32"))]
fn save_svg(svg: String, path: PathBuf) -> ExportOutcome {
    match std::fs::write(&path, svg.as_bytes()) {
        Ok(()) => ExportOutcome::Saved(ExportKind::Svg.label()),
        Err(e) => ExportOutcome::Failed(e.to_string()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
async fn save_png(cfg: Config, width: u32, path: PathBuf, camera: Camera) -> ExportOutcome {
    match lsystem_renderer::png_export::render_png_standalone(&cfg, width, &camera).await {
        Ok(png) => match std::fs::write(&path, png.bytes) {
            Ok(()) => ExportOutcome::Saved(ExportKind::Png.label()),
            Err(e) => ExportOutcome::Failed(e.to_string()),
        },
        Err(e) => {
            log::error!("Failed to export PNG: {e}");
            ExportOutcome::Failed(e.to_string())
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn save_svg(svg: String, suggested_name: String) -> ExportOutcome {
    let blob = gloo_file::Blob::new_with_options(svg.as_str(), Some("image/svg+xml"));
    download_blob(blob, suggested_name);
    ExportOutcome::Saved("SVG")
}

#[cfg(target_arch = "wasm32")]
async fn save_png(
    cfg: Config,
    width: u32,
    camera: Camera,
    suggested_name: String,
) -> ExportOutcome {
    match lsystem_renderer::png_export::render_png_standalone(&cfg, width, &camera).await {
        Ok(png) => {
            let blob = gloo_file::Blob::new_with_options(png.bytes.as_slice(), Some("image/png"));
            download_blob(blob, suggested_name);
            ExportOutcome::Saved("PNG")
        }
        Err(e) => {
            log::error!("Failed to export PNG: {e}");
            ExportOutcome::Failed(e.to_string())
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn download_blob(blob: gloo_file::Blob, suggested_name: String) {
    use wasm_bindgen::JsCast;

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };

    let url = gloo_file::ObjectUrl::from(blob);

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
}
