use std::sync::Arc;

#[cfg(all(target_arch = "wasm32", not(target_os = "emscripten")))]
use browser_wasm as platform;
#[cfg(any(not(target_arch = "wasm32"), target_os = "emscripten"))]
use non_browser_wasm as platform;

pub(crate) use platform::new_instance;

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
    pub(crate) async fn new_instance() -> wgpu::Instance {
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle())
    }

    pub(crate) fn device_limits(_adapter: &wgpu::Adapter) -> wgpu::Limits {
        wgpu::Limits::default()
    }
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
