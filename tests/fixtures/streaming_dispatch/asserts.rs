//! Assertions over the observations recorded by a streaming dispatch world.

use std::{io, time::Duration};

use wireframe::{
    app::Envelope,
    serializer::{BincodeSerializer, Serializer},
};

use super::{
    COLLECT_TIMEOUT_MS,
    DispatchEvent,
    StreamingDispatchWorld,
    TestResult,
    frames::read_frame,
};

/// How long an assertion waits for the next reply frame.
const REPLY_TIMEOUT_MS: u64 = 400;
/// How long an absence assertion waits before concluding nothing arrived.
const ABSENCE_TIMEOUT_MS: u64 = 150;

impl StreamingDispatchWorld {
    /// Assert that the next chunk the handler received equals `expected`.
    ///
    /// Chunks are compared in delivery order, so successive calls pin the order
    /// the handler observed rather than merely the set of chunks it saw.
    ///
    /// # Errors
    ///
    /// Returns an error if the next chunk is missing or differs.
    pub fn assert_chunk_received(&mut self, expected: &str) -> TestResult {
        let index = self.chunks_seen;
        let wanted = expected.as_bytes();
        let done = |observed: &[DispatchEvent]| chunk_at(observed, index) == Some(wanted);
        self.collect_matching(Duration::from_millis(COLLECT_TIMEOUT_MS), &done)?;
        let actual = chunk_at(&self.observed, index);
        if actual == Some(expected.as_bytes()) {
            self.chunks_seen += 1;
            return Ok(());
        }
        let received = actual.map(|chunk| String::from_utf8_lossy(chunk).into_owned());
        Err(format!(
            "expected chunk {expected:?}, got {received:?}; {}",
            self.describe()
        )
        .into())
    }

    /// Assert that the handler observed a clean end of body.
    ///
    /// # Errors
    ///
    /// Returns an error if the body has not completed successfully.
    pub fn assert_body_completed(&mut self) -> TestResult {
        let done = |observed: &[DispatchEvent]| {
            has(observed, |event| matches!(event, DispatchEvent::Completed))
        };
        self.collect_matching(Duration::from_millis(COLLECT_TIMEOUT_MS), &done)?;
        if done(&self.observed) {
            return Ok(());
        }
        Err(format!("body did not complete; {}", self.describe()).into())
    }

    /// Assert that the handler observed `kind` as the body's terminal error.
    ///
    /// # Errors
    ///
    /// Returns an error if that error was not observed.
    pub fn assert_body_error(&mut self, kind: io::ErrorKind) -> TestResult {
        let done = |events: &[DispatchEvent]| {
            has(
                events,
                |event| matches!(event, DispatchEvent::Error { kind: seen } if *seen == kind),
            )
        };
        self.collect_matching(Duration::from_millis(COLLECT_TIMEOUT_MS), &done)?;
        if done(&self.observed) {
            return Ok(());
        }
        Err(format!("body error {kind:?} not observed; {}", self.describe()).into())
    }

    /// Assert that the body is still open, with neither completion nor error.
    ///
    /// # Errors
    ///
    /// Returns an error if the body terminated.
    pub fn assert_body_still_open(&mut self) -> TestResult {
        self.collect_for(Duration::from_millis(ABSENCE_TIMEOUT_MS))?;
        let terminated = self.has_event(|event| {
            matches!(
                event,
                DispatchEvent::Completed | DispatchEvent::Error { .. }
            )
        });
        if terminated {
            return Err(format!("body terminated unexpectedly; {}", self.describe()).into());
        }
        Ok(())
    }

    /// Assert that the buffered handler received `expected`.
    ///
    /// # Errors
    ///
    /// Returns an error if no matching dispatch was observed.
    pub fn assert_buffered_payload(&mut self, expected: &str) -> TestResult {
        let wanted = expected.as_bytes();
        let done = |observed: &[DispatchEvent]| buffered(observed).any(|body| body == wanted);
        self.collect_matching(Duration::from_millis(COLLECT_TIMEOUT_MS), &done)?;
        if done(&self.observed) {
            return Ok(());
        }
        Err(format!(
            "buffered payload {expected:?} not dispatched; {}",
            self.describe()
        )
        .into())
    }

    /// Assert that no buffered message was dispatched.
    ///
    /// # Errors
    ///
    /// Returns an error if a buffered dispatch was observed.
    pub fn assert_no_buffered_payload(&mut self) -> TestResult {
        self.collect_for(Duration::from_millis(ABSENCE_TIMEOUT_MS))?;
        if buffered(&self.observed).next().is_none() {
            return Ok(());
        }
        Err(format!("unexpected buffered dispatch; {}", self.describe()).into())
    }

    /// Read the next reply frame and assert its payload.
    ///
    /// Replies are consumed in arrival order, so consecutive calls assert
    /// response ordering.
    ///
    /// # Errors
    ///
    /// Returns an error if no reply arrives or its payload differs.
    pub fn assert_reply(&mut self, expected: &str) -> TestResult {
        let reply = self.next_reply(Duration::from_millis(REPLY_TIMEOUT_MS))?;
        let Some(payload) = reply else {
            return Err(format!("expected reply {expected:?}, got none").into());
        };
        if payload == expected.as_bytes() {
            return Ok(());
        }
        Err(format!(
            "expected reply {expected:?}, got {:?}",
            String::from_utf8_lossy(&payload)
        )
        .into())
    }

    /// Assert that the connection ended without error.
    ///
    /// Closes the connection first when a scenario did not close it already.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection produced an error or did not finish.
    pub fn assert_server_ok(&mut self) -> TestResult {
        if self.server_is_open() {
            self.close_connection()?;
        }
        match self.server_outcome_text() {
            Some("ok") => Ok(()),
            other => Err(format!("connection did not close cleanly: {other:?}").into()),
        }
    }

    fn collect_for(&mut self, wait: Duration) -> TestResult { self.collect_events_for(wait) }

    fn has_event(&self, predicate: impl Fn(&DispatchEvent) -> bool) -> bool {
        has(&self.observed, predicate)
    }

    fn next_reply(&mut self, wait: Duration) -> TestResult<Option<Vec<u8>>> {
        let Some(mut client) = self.take_client() else {
            return Ok(None);
        };
        let read =
            self.block_on(async { tokio::time::timeout(wait, read_frame(&mut client)).await });
        self.restore_client(client);
        match read? {
            Err(_) => Ok(None),
            Ok(Ok(frame)) => {
                let (envelope, _): (Envelope, usize) = BincodeSerializer.deserialize(&frame)?;
                Ok(Some(envelope.payload_bytes().to_vec()))
            }
            Ok(Err(error)) => Err(format!("reading reply failed: {error}").into()),
        }
    }

    /// Render the recorded events for assertion failure messages.
    fn describe(&self) -> String {
        let events: Vec<String> = self
            .observed
            .iter()
            .map(|event| match event {
                DispatchEvent::Chunk { body } => {
                    format!("chunk({:?})", String::from_utf8_lossy(body))
                }
                DispatchEvent::Error { kind } => format!("error({kind:?})"),
                DispatchEvent::Completed => "completed".to_owned(),
                DispatchEvent::Buffered { body } => {
                    format!("buffered({:?})", String::from_utf8_lossy(body))
                }
            })
            .collect();
        format!("observed=[{}]", events.join(", "))
    }
}

/// Return whether any observed event satisfies `predicate`.
fn has(observed: &[DispatchEvent], predicate: impl Fn(&DispatchEvent) -> bool) -> bool {
    observed.iter().any(predicate)
}

/// Return the chunk delivered at `index`, counting only chunk events.
fn chunk_at(observed: &[DispatchEvent], index: usize) -> Option<&[u8]> {
    chunks(observed).nth(index)
}

/// Iterate the chunk events in delivery order.
fn chunks(observed: &[DispatchEvent]) -> impl Iterator<Item = &[u8]> {
    observed.iter().filter_map(|event| match event {
        DispatchEvent::Chunk { body } => Some(body.as_slice()),
        _ => None,
    })
}

/// Iterate the payloads dispatched to the buffered route.
fn buffered(observed: &[DispatchEvent]) -> impl Iterator<Item = &[u8]> {
    observed.iter().filter_map(|event| match event {
        DispatchEvent::Buffered { body } => Some(body.as_slice()),
        _ => None,
    })
}
