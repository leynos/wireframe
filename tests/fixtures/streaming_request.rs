//! Behavioural fixture for streaming request body scenarios.

use std::{fmt, future::Future, io, time::Duration};

use bytes::Bytes;
use futures::StreamExt;
use rstest::fixture;
use tokio::{io::AsyncReadExt, sync::mpsc};
use wireframe::{
    extractor::StreamingBody,
    request::{RequestBodyReader, RequestBodyStream, body_channel},
};
pub use wireframe_testing::TestResult;

/// Runtime-backed world for streaming request body behaviour.
pub struct StreamingRequestWorld {
    runtime: Option<tokio::runtime::Runtime>,
    runtime_error: Option<String>,
    sender: Option<mpsc::Sender<Result<Bytes, io::Error>>>,
    stream: Option<RequestBodyStream>,
    collected_body: Vec<u8>,
    collected_chunks: usize,
    last_error_kind: Option<io::ErrorKind>,
    send_blocked_by_backpressure: Option<bool>,
}

impl Default for StreamingRequestWorld {
    fn default() -> Self {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => Self {
                runtime: Some(runtime),
                runtime_error: None,
                sender: None,
                stream: None,
                collected_body: Vec::new(),
                collected_chunks: 0,
                last_error_kind: None,
                send_blocked_by_backpressure: None,
            },
            Err(err) => Self {
                runtime: None,
                runtime_error: Some(format!("failed to create runtime: {err}")),
                sender: None,
                stream: None,
                collected_body: Vec::new(),
                collected_chunks: 0,
                last_error_kind: None,
                send_blocked_by_backpressure: None,
            },
        }
    }
}

impl fmt::Debug for StreamingRequestWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamingRequestWorld")
            .field("sender_initialized", &self.sender.is_some())
            .field("stream_initialized", &self.stream.is_some())
            .field("collected_body_len", &self.collected_body.len())
            .field("collected_chunks", &self.collected_chunks)
            .field("last_error_kind", &self.last_error_kind)
            .field(
                "send_blocked_by_backpressure",
                &self.send_blocked_by_backpressure,
            )
            .finish_non_exhaustive()
    }
}

/// Fixture for streaming request body behavioural tests.
#[rustfmt::skip]
#[fixture]
pub fn streaming_request_world() -> StreamingRequestWorld {
    StreamingRequestWorld::default()
}

impl StreamingRequestWorld {
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
            return Err("nested Tokio runtime detected in streaming request fixture".into());
        }
        Ok(self.runtime()?.block_on(future))
    }

    /// Create a fresh request body channel.
    pub fn create_channel(&mut self, capacity: usize) {
        let (sender, stream) = body_channel(capacity);
        self.sender = Some(sender);
        self.stream = Some(stream);
        self.collected_body.clear();
        self.collected_chunks = 0;
        self.last_error_kind = None;
        self.send_blocked_by_backpressure = None;
    }

    /// Send a body chunk into the request stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel is unavailable or the send fails.
    pub fn send_chunk(&mut self, chunk: &str) -> TestResult {
        let sender = self.sender.clone().ok_or("request body sender missing")?;
        self.block_on(async move { sender.send(Ok(Bytes::from(chunk.to_owned()))).await })??;
        Ok(())
    }

    /// Attempt to send a body chunk with a timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel is unavailable.
    pub fn send_chunk_with_timeout(&mut self, chunk: &str, timeout_ms: u64) -> TestResult {
        let sender = self.sender.clone().ok_or("request body sender missing")?;
        let blocked = self.block_on(async move {
            tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                sender.send(Ok(Bytes::from(chunk.to_owned()))),
            )
            .await
            .is_err()
        })?;
        self.send_blocked_by_backpressure = Some(blocked);
        Ok(())
    }

    /// Send an I/O error into the request stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel is unavailable or the send fails.
    pub fn send_error(&mut self, kind: io::ErrorKind) -> TestResult {
        let sender = self.sender.clone().ok_or("request body sender missing")?;
        self.block_on(async move {
            sender
                .send(Err(io::Error::new(
                    kind,
                    format!("request body error: {kind}"),
                )))
                .await
        })??;
        Ok(())
    }

    /// Read the request body through `StreamingBody` and `RequestBodyReader`.
    ///
    /// # Errors
    ///
    /// Returns an error if the stream is unavailable or the read fails.
    pub fn drain_with_reader(&mut self) -> TestResult {
        self.sender = None;
        let stream = self.stream.take().ok_or("request body stream missing")?;
        let body = StreamingBody::new(stream);
        let mut reader: RequestBodyReader = body.into_reader();
        let mut buffer = Vec::new();
        self.block_on(async { reader.read_to_end(&mut buffer).await })??;
        self.collected_chunks = usize::from(!buffer.is_empty());
        self.collected_body = buffer;
        Ok(())
    }

    /// Drain the request stream directly and retain any observed error.
    ///
    /// # Errors
    ///
    /// Returns an error if the stream is unavailable.
    pub fn drain_stream(&mut self) -> TestResult {
        let mut stream = self.stream.take().ok_or("request body stream missing")?;
        let observed = self.block_on(async move {
            let mut chunks = Vec::new();
            let mut error_kind = None;
            loop {
                let next_chunk = stream.next().await.transpose();
                match classify_next_chunk(next_chunk, &mut error_kind) {
                    NextChunk::Chunk(chunk) => chunks.push(chunk),
                    NextChunk::End => break,
                }
            }
            (chunks, error_kind)
        })?;

        let (chunks, error_kind) = observed;
        self.collected_chunks = chunks.len();
        self.collected_body = chunks
            .into_iter()
            .flat_map(|chunk| chunk.to_vec())
            .collect();
        self.last_error_kind = error_kind;
        Ok(())
    }

    /// Assert the collected body matches the expected bytes.
    pub fn assert_collected_body(&self, expected: &str) -> TestResult {
        assert_field_eq("body", expected.as_bytes(), self.collected_body.as_slice())
    }

    /// Assert that back-pressure blocked the timed send.
    pub fn assert_send_blocked_by_backpressure(&self) -> TestResult {
        match self.send_blocked_by_backpressure {
            Some(true) => Ok(()),
            Some(false) => Err("expected back-pressure to block the send".into()),
            None => Err("back-pressure result was not recorded".into()),
        }
    }

    /// Assert the number of successful chunks seen before termination.
    pub fn assert_collected_chunks(&self, expected: usize) -> TestResult {
        assert_field_eq("collected chunks count", expected, self.collected_chunks)
    }

    /// Assert the last observed error kind.
    pub fn assert_last_error_kind(&self, expected: io::ErrorKind) -> TestResult {
        match self.last_error_kind {
            Some(kind) if kind == expected => Ok(()),
            Some(kind) => {
                Err(format!("expected error kind {expected:?}, observed {kind:?}").into())
            }
            None => Err("no stream error was observed".into()),
        }
    }
}

/// Parse a human-readable error kind used in feature files.
pub fn parse_error_kind(value: &str) -> Result<io::ErrorKind, String> {
    match value {
        "invalid data" => Ok(io::ErrorKind::InvalidData),
        "broken pipe" => Ok(io::ErrorKind::BrokenPipe),
        "timed out" => Ok(io::ErrorKind::TimedOut),
        other => Err(format!("unsupported error kind: {other}")),
    }
}

/// Newtype that lets BDD macros capture an `io::ErrorKind` directly from a
/// Gherkin step string via `FromStr`.
pub struct ErrorKindArg(pub io::ErrorKind);

impl std::str::FromStr for ErrorKindArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_error_kind(s).map(ErrorKindArg)
    }
}

impl From<ErrorKindArg> for io::ErrorKind {
    fn from(arg: ErrorKindArg) -> Self {
        arg.0
    }
}

fn assert_field_eq<T>(label: &str, expected: T, observed: T) -> TestResult
where
    T: PartialEq + fmt::Debug,
{
    if expected == observed {
        return Ok(());
    }
    Err(format!("expected {label} {expected:?}, observed {observed:?}").into())
}

enum NextChunk {
    Chunk(Bytes),
    End,
}

fn classify_next_chunk(
    next_chunk: Result<Option<Bytes>, io::Error>,
    error_kind: &mut Option<io::ErrorKind>,
) -> NextChunk {
    match next_chunk {
        Ok(Some(chunk)) => NextChunk::Chunk(chunk),
        Ok(None) => NextChunk::End,
        Err(err) => {
            *error_kind = Some(err.kind());
            NextChunk::End
        }
    }
}
