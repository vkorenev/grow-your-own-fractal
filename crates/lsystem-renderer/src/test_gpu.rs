//! Shared headless GPU device for unit tests.
//!
//! GPU tests run in parallel threads of one test process. Creating and
//! dropping a separate device per test has crashed the process with SIGSEGV
//! on Mesa's lavapipe software renderer in CI, so all GPU tests share one
//! device that is created on first use and never dropped.
//!
//! Sharing is safe for the current tests because wgpu error scopes are
//! thread-local when wgpu's `std` feature is enabled (it is, via wgpu's
//! default features), so validation checks stay isolated per test.

use std::sync::OnceLock;

/// Returns handles to the process-wide test device and queue.
pub(crate) fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static SHARED: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            pollster::block_on(crate::wgpu_util::create_headless_device(
                "shared_test_device",
                "shared test device",
            ))
            .expect("failed to create shared headless test device")
        })
        .clone()
}
