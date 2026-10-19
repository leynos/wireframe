//! Shared test support for client-pool bookkeeping modules.

use std::{
    io,
    net::SocketAddr,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bb8::Pool;
use tracing_subscriber::fmt::MakeWriter;

use super::{manager::WireframeConnectionManager, slot::PoolSlot, sync::lock_or_recover};
use crate::{
    client::{
        ClientCodecConfig,
        SocketOptions,
        connect_parts::ClientBuildParts,
        hooks::{LifecycleHooks, RequestHooks},
        tracing_config::TracingConfig,
    },
    serializer::BincodeSerializer,
};

/// Build a slot without connecting; `build_unchecked` creates its pool lazily.
pub(super) fn test_pool_slot(
    max_in_flight: usize,
    idle_timeout: Duration,
) -> PoolSlot<BincodeSerializer, (), ()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 1));
    let parts = ClientBuildParts {
        serializer: BincodeSerializer,
        codec_config: ClientCodecConfig::default(),
        socket_options: SocketOptions::default(),
        preamble_config: None,
        lifecycle_hooks: LifecycleHooks::default(),
        request_hooks: RequestHooks::default(),
        tracing_config: TracingConfig::default(),
    };
    let pool = Pool::builder()
        .max_size(1)
        .build_unchecked(WireframeConnectionManager::new(addr, parts));
    PoolSlot::new(pool, max_in_flight, idle_timeout)
}

/// Test-only lifetime witness shared by one core and its slots.
pub(super) struct DropProbe(Arc<AtomicUsize>);

impl DropProbe {
    /// Count one owner releasing its probed value.
    pub(super) fn new(drops: Arc<AtomicUsize>) -> Self { Self(drops) }
}

impl Drop for DropProbe {
    fn drop(&mut self) { self.0.fetch_add(1, Ordering::AcqRel); }
}

#[derive(Clone)]
pub(super) struct CaptureWriter {
    captured: Arc<Mutex<Vec<u8>>>,
}

impl CaptureWriter {
    pub(super) fn new(captured: Arc<Mutex<Vec<u8>>>) -> Self { Self { captured } }
}

impl<'a> MakeWriter<'a> for CaptureWriter {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer { self.clone() }
}

impl io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock_or_recover(&self.captured).extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
