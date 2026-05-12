use std::sync::Arc;

pub(crate) fn new_instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle())
}

pub(crate) fn device_descriptor(label: &'static str) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: Some(label),
        ..Default::default()
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
