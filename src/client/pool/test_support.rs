//! Shared test support for client-pool bookkeeping modules.

use std::{
    io,
    sync::{Arc, Mutex},
};

use tracing_subscriber::fmt::MakeWriter;

use super::sync::lock_or_recover;

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
