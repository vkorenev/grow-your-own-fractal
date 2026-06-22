use std::sync::Arc;

#[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
use browser_wasm as platform;
#[cfg(any(not(target_arch = "wasm32"), target_os = "emscripten"))]
use non_browser_wasm as platform;

pub(crate) use platform::{MAX_BUFFER_SIZE_BYTES, new_instance};

pub(crate) fn device_descriptor(
    label: &'static str,
    adapter: &wgpu::Adapter,
) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: Some(label),
        required_limits: platform::device_limits(adapter),
        ..Default::default()
    }
}

#[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
mod browser_wasm {
    pub(crate) const MAX_BUFFER_SIZE_BYTES: u64 =
        wgpu::Limits::downlevel_webgl2_defaults().max_buffer_size;

    pub(crate) async fn new_instance() -> wgpu::Instance {
        let descriptor = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(WebDisplay));
        wgpu::util::new_instance_with_webgpu_detection(descriptor).await
    }

    pub(crate) fn device_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
        wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
    }

    #[derive(Debug)]
    struct WebDisplay;

    impl wgpu::rwh::HasDisplayHandle for WebDisplay {
        fn display_handle(&self) -> Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
            Ok(wgpu::rwh::DisplayHandle::web())
        }
    }
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "emscripten"))]
mod non_browser_wasm {
    pub(crate) const MAX_BUFFER_SIZE_BYTES: u64 = wgpu::Limits::defaults().max_buffer_size;

    pub(crate) async fn new_instance() -> wgpu::Instance {
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle())
    }

    pub(crate) fn device_limits(_adapter: &wgpu::Adapter) -> wgpu::Limits {
        wgpu::Limits::default()
    }
}

#[cfg(feature = "png")]
#[derive(Debug)]
pub(crate) enum CreateDeviceError {
    NoAdapter,
    RequestDevice(wgpu::RequestDeviceError),
}

#[cfg(feature = "png")]
pub(crate) async fn create_headless_device(
    label: &'static str,
    context: &'static str,
) -> Result<(wgpu::Device, wgpu::Queue), CreateDeviceError> {
    let instance = new_instance().await;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .map_err(|_| CreateDeviceError::NoAdapter)?;
    let adapter_info = adapter.get_info();
    log::info!(
        "Selected {context} GPU adapter: {} ({})",
        adapter_info.name,
        adapter_info.backend
    );
    let (device, queue) = adapter
        .request_device(&device_descriptor(label, &adapter))
        .await
        .map_err(CreateDeviceError::RequestDevice)?;
    install_uncaptured_error_handler(&device, context);
    Ok((device, queue))
}

pub(crate) fn install_uncaptured_error_handler(device: &wgpu::Device, context: &'static str) {
    device.on_uncaptured_error(Arc::new(move |error| match error {
        wgpu::Error::OutOfMemory { .. } => {
            log::error!("{context}: uncaptured wgpu out-of-memory error: {error}");
        }
        wgpu::Error::Internal { .. } => {
            log::error!("{context}: uncaptured internal wgpu error: {error}");
        }
        wgpu::Error::Validation { .. } => {
            log::error!("{context}: uncaptured wgpu validation error: {error}");
        }
    }));
}
