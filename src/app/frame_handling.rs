//! Shared helpers for frame decoding, reassembly, and response forwarding.
//!
//! Extracted from `connection.rs` to keep modules small and focused.

use std::io;

use futures::SinkExt;
use log::warn;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use super::{
    Envelope,
    Packet,
    PacketParts,
    fragmentation_state::{FragmentProcessError, FragmentationState},
};
use crate::{
    middleware::{HandlerService, Service, ServiceRequest},
    serializer::Serializer,
};

/// Attempt to reassemble a potentially fragmented envelope.
pub(crate) fn reassemble_if_needed(
    fragmentation: &mut Option<FragmentationState>,
    deser_failures: &mut u32,
    env: Envelope,
    max_deser_failures: u32,
) -> io::Result<Option<Envelope>> {
    fn handle_fragment_error(
        deser_failures: &mut u32,
        max_deser_failures: u32,
        correlation_id: Option<u64>,
        context: &str,
        err: impl std::fmt::Debug,
    ) -> io::Result<Option<Envelope>> {
        *deser_failures += 1;
        warn!("{context}: correlation_id={correlation_id:?}, error={err:?}");
        crate::metrics::inc_deser_errors();
        if *deser_failures >= max_deser_failures {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "too many deserialization failures",
            ));
        }
        Ok(None)
    }

    if let Some(state) = fragmentation.as_mut() {
        let correlation_id = env.correlation_id;
        match state.reassemble(env) {
            Ok(Some(env)) => Ok(Some(env)),
            Ok(None) => Ok(None),
            Err(FragmentProcessError::Decode(err)) => handle_fragment_error(
                deser_failures,
                max_deser_failures,
                correlation_id,
                "failed to decode fragment header",
                err,
            ),
            Err(FragmentProcessError::Reassembly(err)) => handle_fragment_error(
                deser_failures,
                max_deser_failures,
                correlation_id,
                "fragment reassembly failed",
                err,
            ),
        }
    } else {
        Ok(Some(env))
    }
}

/// Forward a handler response, fragmenting if required, and write to the framed stream.
pub(crate) async fn forward_response<S, E, W>(
    serializer: &S,
    env: Envelope,
    service: &HandlerService<E>,
    framed: &mut Framed<W, LengthDelimitedCodec>,
    fragmentation: &mut Option<FragmentationState>,
) -> io::Result<()>
where
    S: Serializer + Send + Sync,
    E: Packet,
    W: AsyncRead + AsyncWrite + Unpin,
{
    let request = ServiceRequest::new(env.payload, env.correlation_id);
    match service.call(request).await {
        Ok(resp) => {
            let parts = PacketParts::new(env.id, resp.correlation_id(), resp.into_inner())
                .inherit_correlation(env.correlation_id);
            let correlation_id = parts.correlation_id();
            let responses = if let Some(state) = fragmentation.as_mut() {
                match state.fragment(Envelope::from_parts(parts)) {
                    Ok(fragmented) => fragmented,
                    Err(err) => {
                        warn!(
                            "failed to fragment response: id={}, correlation_id={:?}, \
                             error={err:?}",
                            env.id, correlation_id
                        );
                        crate::metrics::inc_handler_errors();
                        return Ok(());
                    }
                }
            } else {
                vec![Envelope::from_parts(parts)]
            };

            for response in responses {
                match serializer.serialize(&response) {
                    Ok(bytes) => {
                        if let Err(e) = framed.send(bytes.into()).await {
                            warn!(
                                "failed to send response: id={}, correlation_id={:?}, error={e:?}",
                                env.id, correlation_id
                            );
                            crate::metrics::inc_handler_errors();
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(
                            "failed to serialize response: id={}, correlation_id={:?}, error={e:?}",
                            env.id, correlation_id
                        );
                        crate::metrics::inc_handler_errors();
                        break;
                    }
                }
            }
        }
        Err(e) => {
            warn!(
                "handler error: id={}, correlation_id={:?}, error={e:?}",
                env.id, env.correlation_id
            );
            crate::metrics::inc_handler_errors();
        }
    }

    Ok(())
}
