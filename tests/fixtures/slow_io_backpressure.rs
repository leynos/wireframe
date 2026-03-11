//! BDD world fixture for slow-I/O back-pressure scenarios.

use std::{fmt, future::Future, io, num::NonZeroUsize, str::FromStr, sync::Arc, time::Duration};

use futures::future::BoxFuture;
use rstest::fixture;
use wireframe::{
    app::{Envelope, WireframeApp},
    codec::examples::HotlineFrameCodec,
    serializer::{BincodeSerializer, Serializer},
};
pub use wireframe_testing::TestResult;
use wireframe_testing::{SlowIoConfig, SlowIoPacing, drive_with_slow_codec_payloads};

/// Runtime-backed world for behavioural tests covering slow reader and writer
/// simulation.
pub struct SlowIoBackpressureWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    max_frame_length: Option<usize>,
    task: Option<tokio::task::JoinHandle<io::Result<Vec<Vec<u8>>>>>,
    outputs: Option<Vec<Vec<u8>>>,
}

#[derive(Clone, Copy)]
pub(crate) struct ReaderDriveConfig {
    payload_len: usize,
    chunk_size: usize,
    delay_millis: u64,
    capacity: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct CombinedDriveConfig {
    payload_len: usize,
    writer_chunk_size: usize,
    writer_delay_millis: u64,
    reader_chunk_size: usize,
    reader_delay_millis: u64,
    capacity: usize,
}

impl FromStr for ReaderDriveConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('/');
        let Some(payload_len) = parts.next() else {
            return Err("missing payload_len".to_string());
        };
        let Some(chunk_size) = parts.next() else {
            return Err("missing chunk_size".to_string());
        };
        let Some(delay_millis) = parts.next() else {
            return Err("missing delay_millis".to_string());
        };
        let Some(capacity) = parts.next() else {
            return Err("missing capacity".to_string());
        };
        if parts.next().is_some() {
            return Err(format!(
                "expected reader config payload_len/chunk_size/delay_millis/capacity, got {s}"
            ));
        }
        Ok(Self {
            payload_len: payload_len
                .parse()
                .map_err(|error| format!("payload_len: {error}"))?,
            chunk_size: chunk_size
                .parse()
                .map_err(|error| format!("chunk_size: {error}"))?,
            delay_millis: delay_millis
                .parse()
                .map_err(|error| format!("delay_millis: {error}"))?,
            capacity: capacity
                .parse()
                .map_err(|error| format!("capacity: {error}"))?,
        })
    }
}

impl FromStr for CombinedDriveConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('/');
        let Some(payload_len) = parts.next() else {
            return Err("missing payload_len".to_string());
        };
        let Some(writer_chunk_size) = parts.next() else {
            return Err("missing writer_chunk_size".to_string());
        };
        let Some(writer_delay_millis) = parts.next() else {
            return Err("missing writer_delay_millis".to_string());
        };
        let Some(reader_chunk_size) = parts.next() else {
            return Err("missing reader_chunk_size".to_string());
        };
        let Some(reader_delay_millis) = parts.next() else {
            return Err("missing reader_delay_millis".to_string());
        };
        let Some(capacity) = parts.next() else {
            return Err("missing capacity".to_string());
        };
        if parts.next().is_some() {
            return Err(format!(
                "expected combined config \
                 payload_len/writer_chunk/writer_delay/reader_chunk/reader_delay/capacity, got {s}"
            ));
        }
        Ok(Self {
            payload_len: payload_len
                .parse()
                .map_err(|error| format!("payload_len: {error}"))?,
            writer_chunk_size: writer_chunk_size
                .parse()
                .map_err(|error| format!("writer_chunk_size: {error}"))?,
            writer_delay_millis: writer_delay_millis
                .parse()
                .map_err(|error| format!("writer_delay_millis: {error}"))?,
            reader_chunk_size: reader_chunk_size
                .parse()
                .map_err(|error| format!("reader_chunk_size: {error}"))?,
            reader_delay_millis: reader_delay_millis
                .parse()
                .map_err(|error| format!("reader_delay_millis: {error}"))?,
            capacity: capacity
                .parse()
                .map_err(|error| format!("capacity: {error}"))?,
        })
    }
}

impl Default for SlowIoBackpressureWorld {
    fn default() -> Self {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
        {
            Ok(runtime) => Self {
                runtime: Some(runtime),
                runtime_error: None,
                max_frame_length: None,
                task: None,
                outputs: None,
            },
            Err(error) => Self {
                runtime: None,
                runtime_error: Some(format!("failed to create runtime: {error}")),
                max_frame_length: None,
                task: None,
                outputs: None,
            },
        }
    }
}

impl fmt::Debug for SlowIoBackpressureWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlowIoBackpressureWorld")
            .field("max_frame_length", &self.max_frame_length)
            .field("task_started", &self.task.is_some())
            .field("outputs", &self.outputs.as_ref().map(std::vec::Vec::len))
            .finish_non_exhaustive()
    }
}

/// Construct the default world used by slow-I/O behavioural tests.
#[rustfmt::skip]
#[fixture]
pub fn slow_io_backpressure_world() -> SlowIoBackpressureWorld {
    SlowIoBackpressureWorld::default()
}

impl SlowIoBackpressureWorld {
    fn runtime(&self) -> TestResult<&tokio::runtime::Runtime> {
        self.runtime.as_ref().ok_or_else(|| {
            self.runtime_error
                .clone()
                .unwrap_or_else(|| "runtime unavailable".to_string())
                .into()
        })
    }

    fn block_on<F, T>(&self, future: F) -> TestResult<T>
    where
        F: Future<Output = T>,
    {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("nested Tokio runtime detected in slow-io fixture".into());
        }
        Ok(self.runtime()?.block_on(future))
    }

    fn build_app(
        &self,
    ) -> Result<WireframeApp<BincodeSerializer, (), Envelope, HotlineFrameCodec>, String> {
        let max_frame_length = self
            .max_frame_length
            .ok_or_else(|| "max frame length not configured".to_string())?;
        let codec = HotlineFrameCodec::new(max_frame_length);
        WireframeApp::<BincodeSerializer, (), Envelope>::new()
            .map_err(|e| format!("app init: {e}"))?
            .with_codec(codec)
            .route(
                1,
                Arc::new(|_: &Envelope| -> BoxFuture<'static, ()> { Box::pin(async {}) }),
            )
            .map_err(|e| format!("route: {e}"))
    }

    fn serialize_request(payload_len: usize) -> Result<Vec<u8>, String> {
        BincodeSerializer
            .serialize(&Envelope::new(1, Some(7), vec![b'x'; payload_len]))
            .map_err(|e| format!("serialize: {e}"))
    }

    fn start_drive(&mut self, payload_len: usize, config: SlowIoConfig) -> TestResult {
        if self.task.as_ref().is_some_and(|task| !task.is_finished()) {
            return Err("slow-io drive is already running".into());
        }

        let app = self.build_app()?;
        let codec = HotlineFrameCodec::new(
            self.max_frame_length
                .ok_or("max frame length not configured")?,
        );
        let payload = Self::serialize_request(payload_len)?;

        self.outputs = None;
        self.task = Some(self.runtime()?.spawn(async move {
            drive_with_slow_codec_payloads(app, &codec, vec![payload], config).await
        }));
        Ok(())
    }

    fn take_outputs_from_task(&mut self) -> TestResult<()> {
        if self.outputs.is_some() {
            return Ok(());
        }
        let task = self
            .task
            .take()
            .ok_or("slow-io drive has not been started")?;
        if !task.is_finished() {
            self.task = Some(task);
            return Err(
                "slow-io drive is still pending; advance Tokio time before collecting outputs"
                    .into(),
            );
        }
        let join_result = self.block_on(task)?;
        let outputs = join_result.map_err(|error| format!("join failed: {error}"))??;
        self.outputs = Some(outputs);
        Ok(())
    }

    /// Configure the app under test.
    pub fn configure_app(&mut self, max_frame_length: usize) -> TestResult {
        if max_frame_length == 0 {
            return Err("max frame length must be greater than zero".into());
        }
        self.max_frame_length = Some(max_frame_length);
        Ok(())
    }

    /// Start a drive using slow writer pacing.
    pub fn start_slow_writer(
        &mut self,
        payload_len: usize,
        chunk_size: usize,
        delay_millis: u64,
    ) -> TestResult {
        let chunk_size = NonZeroUsize::new(chunk_size).ok_or("chunk size must be non-zero")?;
        let config = SlowIoConfig::new().with_writer_pacing(SlowIoPacing::new(
            chunk_size,
            Duration::from_millis(delay_millis),
        ));
        self.start_drive(payload_len, config)
    }

    /// Start a drive using slow reader pacing.
    pub fn start_slow_reader(&mut self, config: ReaderDriveConfig) -> TestResult {
        let chunk_size =
            NonZeroUsize::new(config.chunk_size).ok_or("chunk size must be non-zero")?;
        let drive_config = SlowIoConfig::new()
            .with_reader_pacing(SlowIoPacing::new(
                chunk_size,
                Duration::from_millis(config.delay_millis),
            ))
            .with_capacity(config.capacity);
        self.start_drive(config.payload_len, drive_config)
    }

    /// Start a drive using both slow writer and slow reader pacing.
    pub fn start_combined(&mut self, config: CombinedDriveConfig) -> TestResult {
        let writer_chunk_size = NonZeroUsize::new(config.writer_chunk_size)
            .ok_or("writer chunk size must be non-zero")?;
        let reader_chunk_size = NonZeroUsize::new(config.reader_chunk_size)
            .ok_or("reader chunk size must be non-zero")?;
        let drive_config = SlowIoConfig::new()
            .with_writer_pacing(SlowIoPacing::new(
                writer_chunk_size,
                Duration::from_millis(config.writer_delay_millis),
            ))
            .with_reader_pacing(SlowIoPacing::new(
                reader_chunk_size,
                Duration::from_millis(config.reader_delay_millis),
            ))
            .with_capacity(config.capacity);
        self.start_drive(config.payload_len, drive_config)
    }

    /// Assert that the drive has not completed yet.
    pub fn assert_pending(&mut self) -> TestResult {
        self.block_on(async { tokio::task::yield_now().await })?;
        let task = self
            .task
            .as_ref()
            .ok_or("slow-io drive has not been started")?;
        if task.is_finished() {
            return Err("expected slow-io drive to remain pending".into());
        }
        Ok(())
    }

    /// Advance Tokio virtual time.
    pub fn advance_millis(&mut self, millis: u64) -> TestResult {
        self.block_on(async {
            tokio::time::advance(Duration::from_millis(millis)).await;
            tokio::task::yield_now().await;
        })?;
        Ok(())
    }

    /// Assert that the drive completed with one echoed payload of `expected_len`
    /// bytes.
    pub fn assert_completed_payload_len(&mut self, expected_len: usize) -> TestResult {
        self.take_outputs_from_task()?;
        let outputs = self.outputs.as_ref().ok_or("slow-io outputs missing")?;
        if outputs.len() != 1 {
            return Err(format!("expected exactly 1 output payload, got {}", outputs.len()).into());
        }
        let raw = outputs
            .first()
            .ok_or("missing echoed payload after length check")?;
        let (env, consumed) = BincodeSerializer
            .deserialize::<Envelope>(raw)
            .map_err(|e| format!("deserialize: {e}"))?;
        if consumed != raw.len() {
            return Err(format!(
                "deserialize: trailing bytes after envelope: consumed {consumed} of {}",
                raw.len()
            )
            .into());
        }
        let actual_len = env.payload_bytes().len();
        if actual_len != expected_len {
            return Err(
                format!("expected echoed payload length {expected_len}, got {actual_len}").into(),
            );
        }
        Ok(())
    }
}
