pub mod endpoint;
pub mod file;
pub mod picture;
pub mod stream;
pub mod wake;
pub mod wire;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod errands;
#[cfg(feature = "client")]
pub mod pool;

pub use file::FileKey;
pub use picture::PictureKey;
pub use stream::{Eraser, SealError, StreamId, StreamKeys, StreamSecret};
pub use wire::{ClientFrame, ErrorCode, ServerFrame, WireEntry};

#[cfg(feature = "client")]
pub use client::{AppendError, SyncClient, SyncEvent};
#[cfg(feature = "client")]
pub use pool::Pool;
