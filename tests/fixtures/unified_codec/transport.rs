//! Transport helpers for `UnifiedCodecWorld`.
//!
//! Contains the low-level send, receive, fragmentation, and reassembly
//! utilities used by the fixture's public async methods.

#[path = "../../common/unified_codec_transport.rs"]
mod unified_codec_transport;

use std::{num::NonZeroUsize, time::Duration};

use futures::{SinkExt, StreamExt};
use tokio::{io::AsyncWriteExt, time::timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use wireframe::{
    Serializer,
    app::{Envelope, Packet},
    fragment::{
        FragmentationConfig,
        Fragmenter,
        Reassembler,
        decode_fragment_payload,
        encode_fragment_payload,
    },
    serializer::BincodeSerializer,
};

use self::unified_codec_transport::{recv_one, send_one};
use super::{CORRELATION, ROUTE_ID, TestResult, UnifiedCodecWorld};

impl UnifiedCodecWorld {
    /// Send a single payload of the given size to the server.
    ///
    /// # Errors
    /// Returns an error if serialization or sending fails.
    pub async fn send_payload(&mut self, size: usize) -> TestResult {
        let payload = vec![b'P'; size];
        let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
        self.sent_payloads.push(payload);

        let client = self.client.as_mut().ok_or("client not started")?;
        send_one(client, &request).await?;
        client.get_mut().shutdown().await?;
        Ok(())
    }

    /// Send a fragmented payload of the given size to the server.
    ///
    /// # Errors
    /// Returns an error if fragmentation, serialization, or sending fails.
    pub async fn send_fragmented_payload(&mut self, size: usize) -> TestResult {
        let config = self.fragmentation.ok_or("fragmentation not configured")?;
        let payload = vec![b'F'; size];
        let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
        self.sent_payloads.push(payload);

        let fragmenter = Fragmenter::new(config.fragment_payload_cap);
        let envelopes = Self::fragment_envelope(&request, &fragmenter)?;

        let client = self.client.as_mut().ok_or("client not started")?;
        Self::send_envelopes(client, &envelopes).await?;
        client.flush().await?;
        client.get_mut().shutdown().await?;
        Ok(())
    }

    /// Send multiple sequential payloads.
    ///
    /// # Errors
    /// Returns an error if serialization or sending fails.
    pub async fn send_sequential_payloads(&mut self, count: usize, size: usize) -> TestResult {
        let client = self.client.as_mut().ok_or("client not started")?;

        for i in 0..count {
            let payload = vec![i.try_into().unwrap_or(0); size];
            let request = Envelope::new(ROUTE_ID, CORRELATION, payload.clone());
            self.sent_payloads.push(payload);
            send_one(client, &request).await?;
        }
        client.get_mut().shutdown().await?;
        Ok(())
    }

    /// Drain the handler channel and store observed payloads.
    ///
    /// # Errors
    /// Returns an error if the handler channel times out.
    pub async fn collect_handler_payloads(&mut self) -> TestResult {
        let rx = self.handler_rx.as_mut().ok_or("handler rx not started")?;
        let expected_count = self.sent_payloads.len();

        for _ in 0..expected_count {
            let observed = timeout(Duration::from_secs(2), rx.recv())
                .await?
                .ok_or("handler payload missing")?;
            self.handler_observed.push(observed);
        }
        Ok(())
    }

    /// Read a single unfragmented response.
    ///
    /// # Errors
    /// Returns an error if reading or deserialization fails.
    pub async fn collect_single_response(&mut self) -> TestResult {
        let client = self.client.as_mut().ok_or("client not started")?;
        let response = recv_one(client).await?;
        let payload = response.into_parts().into_payload();

        self.last_response_fragmented = Some(decode_fragment_payload(&payload)?.is_some());
        self.response_payloads.push(payload);
        Ok(())
    }

    /// Read a fragmented response and reassemble it.
    ///
    /// # Errors
    /// Returns an error if reading or reassembly fails.
    pub async fn collect_fragmented_response(&mut self) -> TestResult {
        let config = self.fragmentation.ok_or("fragmentation not configured")?;
        let client = self.client.as_mut().ok_or("client not started")?;
        let payload = Self::read_reassembled(client, &config).await?;
        self.last_response_fragmented = Some(true);
        self.response_payloads.push(payload);
        Ok(())
    }

    /// Read multiple sequential responses.
    ///
    /// # Errors
    /// Returns an error if reading or deserialization fails.
    pub async fn collect_sequential_responses(&mut self, count: usize) -> TestResult {
        let client = self.client.as_mut().ok_or("client not started")?;
        for _ in 0..count {
            let response = recv_one(client).await?;
            let payload = response.into_parts().into_payload();
            self.response_payloads.push(payload);
        }
        Ok(())
    }

    // -- low-level transport helpers ----------------------------------------

    pub(super) fn fragmentation_config(capacity: usize) -> TestResult<FragmentationConfig> {
        let message_limit = capacity
            .checked_mul(16)
            .and_then(NonZeroUsize::new)
            .ok_or("message limit overflow or zero")?;
        let config = FragmentationConfig::for_frame_budget(
            capacity,
            message_limit,
            Duration::from_millis(30),
        )
        .ok_or("frame budget must exceed fragment overhead")?;
        Ok(config)
    }

    async fn send_envelopes(
        client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
        envelopes: &[Envelope],
    ) -> TestResult {
        let serializer = BincodeSerializer;
        for env in envelopes {
            let bytes = serializer.serialize(env)?;
            client.send(bytes.into()).await?;
        }
        Ok(())
    }

    fn fragment_envelope(
        envelope: &Envelope,
        fragmenter: &Fragmenter,
    ) -> TestResult<Vec<Envelope>> {
        let parts = envelope.clone().into_parts();
        let id = parts.id();
        let correlation = parts.correlation_id();
        let payload = parts.into_payload();

        if payload.len() <= fragmenter.max_fragment_size().get() {
            return Ok(vec![Envelope::new(id, correlation, payload)]);
        }

        let envelopes = fragmenter
            .fragment_bytes(payload)?
            .into_iter()
            .map(|fragment| {
                let (header, frag_payload) = fragment.into_parts();
                encode_fragment_payload(header, &frag_payload)
                    .map(|encoded| Envelope::new(id, correlation, encoded))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(envelopes)
    }

    async fn read_reassembled(
        client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
        cfg: &FragmentationConfig,
    ) -> TestResult<Vec<u8>> {
        let serializer = BincodeSerializer;
        let mut reassembler = Reassembler::new(cfg.max_message_size, cfg.reassembly_timeout);

        let result: TestResult<Vec<u8>> = timeout(
            Duration::from_secs(2),
            Self::reassemble_loop(client, serializer, &mut reassembler),
        )
        .await?;

        result
    }

    async fn reassemble_loop(
        client: &mut Framed<tokio::io::DuplexStream, LengthDelimitedCodec>,
        serializer: BincodeSerializer,
        reassembler: &mut Reassembler,
    ) -> TestResult<Vec<u8>> {
        while let Some(frame) = client.next().await {
            let completed = Self::try_reassemble_frame(&frame?, serializer, reassembler)?;
            if let Some(payload) = completed {
                return Ok(payload);
            }
        }
        Err("stream ended before reassembly completed".into())
    }

    fn try_reassemble_frame(
        bytes: &bytes::BytesMut,
        serializer: BincodeSerializer,
        reassembler: &mut Reassembler,
    ) -> TestResult<Option<Vec<u8>>> {
        let (env, _) = serializer.deserialize::<Envelope>(bytes)?;
        let payload = env.into_parts().into_payload();
        match decode_fragment_payload(&payload)? {
            Some((header, fragment)) => match reassembler.push(header, fragment)? {
                Some(message) => Ok(Some(message.into_payload())),
                None => Ok(None),
            },
            None => Ok(Some(payload)),
        }
    }
}
