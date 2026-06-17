//! Shared client send pipeline helpers.
//!
//! This module centralizes the common serialize, request-hook, framed-send,
//! timing, and error-hook flow used by the client's send APIs.

use std::time::Instant;

use bytes::Bytes;
use futures::SinkExt;
use tracing::{Instrument, Span};

use super::{ClientError, WireframeClient, runtime::ClientStream, tracing_config::TracingConfig};
use crate::{message::EncodeWith, serializer::Serializer};

impl<S, T, C> WireframeClient<S, T, C>
where
    S: Serializer + Send + Sync,
    T: ClientStream,
{
    /// Serialize a value, run outbound hooks, and send the resulting frame.
    pub(crate) async fn serialize_and_send<M, F>(
        &mut self,
        value: &M,
        timing_start: Option<Instant>,
        span_for_frame: F,
    ) -> Result<(), ClientError>
    where
        M: EncodeWith<S>,
        F: FnOnce(&TracingConfig, usize) -> Span,
    {
        let mut bytes = match self.serializer.serialize(value) {
            Ok(bytes) => bytes,
            Err(e) => {
                let err = ClientError::Serialize(e);
                super::tracing_helpers::emit_timing_event(timing_start);
                self.invoke_error_hook(&err).await;
                return Err(err);
            }
        };

        self.invoke_before_send_hooks(&mut bytes);
        let span = span_for_frame(&self.tracing_config, bytes.len());
        let send_result = async {
            let result = self.framed.send(Bytes::from(bytes)).await;
            super::tracing_helpers::emit_timing_event(timing_start);
            result
        }
        .instrument(span)
        .await;

        if let Err(e) = send_result {
            let err = ClientError::from(e);
            self.invoke_error_hook(&err).await;
            return Err(err);
        }

        Ok(())
    }
}
