use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum ReadbackError {
    Map(wgpu::BufferAsyncError),
    ChannelClosed,
    // Only constructed on non-wasm; wasm uses a polling loop that doesn't return PollError.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Poll(wgpu::PollError),
}

impl Display for ReadbackError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Map(e) => write!(f, "failed to map readback buffer: {e}"),
            Self::ChannelClosed => write!(f, "readback callback was dropped"),
            Self::Poll(e) => write!(f, "failed to poll GPU device: {e}"),
        }
    }
}

impl Error for ReadbackError {}

pub(crate) async fn map_read_buffer(
    device: &wgpu::Device,
    buffer: &wgpu::Buffer,
) -> Result<(), ReadbackError> {
    let (sender, receiver) = futures_channel::oneshot::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });

    #[cfg(not(target_arch = "wasm32"))]
    {
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(ReadbackError::Poll)?;
        receiver
            .await
            .map_err(|_| ReadbackError::ChannelClosed)?
            .map_err(ReadbackError::Map)?;
    }

    #[cfg(target_arch = "wasm32")]
    {
        let mut receiver = receiver;
        loop {
            let _ = device.poll(wgpu::PollType::Poll);
            match receiver.try_recv() {
                Ok(Some(result)) => {
                    result.map_err(ReadbackError::Map)?;
                    break;
                }
                Ok(None) => {
                    gloo_timers::future::TimeoutFuture::new(0).await;
                }
                Err(_) => return Err(ReadbackError::ChannelClosed),
            }
        }
    }

    Ok(())
}
