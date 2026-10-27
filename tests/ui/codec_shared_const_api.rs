//! Verifies codec and shared-source APIs remain callable in const contexts.

use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};

use wireframe::{
    codec::{
        examples::{HotlineFrameCodec, MysqlFrameCodec},
        CodecError,
        CodecErrorContext,
        EofError,
        RecoveryConfig,
        RecoveryPolicy,
    },
    message_assembler::{
        AssembledMessage,
        CorrelationId,
        EnvelopeId,
        EnvelopeRouting,
        FirstFrameHeader,
        FrameHeader,
        MessageKey,
        ParsedFrameHeader,
    },
    request::RequestParts,
};

const CLEAN_CLOSE: CodecError = CodecError::Eof(EofError::CleanClose);
const RECOVERY_POLICY: RecoveryPolicy = CLEAN_CLOSE.default_recovery_policy();
const IS_CLEAN_CLOSE: bool = CLEAN_CLOSE.is_clean_close();
const ERROR_CATEGORY: &str = CLEAN_CLOSE.error_type();

const RECOVERY_CONFIG: RecoveryConfig = RecoveryConfig {
    max_consecutive_drops: 0,
    quarantine_duration: Duration::ZERO,
    log_dropped_frames: false,
}
.max_consecutive_drops(3)
.quarantine_duration(Duration::from_secs(5))
.log_dropped_frames(true);

const ERROR_CONTEXT: CodecErrorContext = CodecErrorContext {
    connection_id: None,
    peer_address: None,
    correlation_id: None,
    codec_state: None,
}
.with_connection_id(7)
.with_peer_address(SocketAddr::V4(SocketAddrV4::new(
    Ipv4Addr::LOCALHOST,
    8080,
)))
.with_correlation_id(9);

const HOTLINE_CODEC: HotlineFrameCodec = HotlineFrameCodec::new(1024);
const MYSQL_CODEC: MysqlFrameCodec = MysqlFrameCodec::new(2048);

const PARSED_HEADER: ParsedFrameHeader = ParsedFrameHeader::new(
    FrameHeader::First(FirstFrameHeader {
        message_key: MessageKey(11),
        metadata_len: 0,
        body_len: 4,
        total_body_len: None,
        is_last: true,
    }),
    12,
);
const FRAME_HEADER: FrameHeader = PARSED_HEADER.into_header();

const ASSEMBLED_MESSAGE: AssembledMessage = AssembledMessage::new(
    MessageKey(11),
    EnvelopeRouting {
        envelope_id: EnvelopeId(42),
        correlation_id: Some(CorrelationId(9)),
    },
    Vec::new(),
    Vec::new(),
);

const REQUEST_PARTS: RequestParts = RequestParts::new(3, Some(9), Vec::new());

const _: () = assert!(matches!(RECOVERY_POLICY, RecoveryPolicy::Disconnect));
const _: () = assert!(IS_CLEAN_CLOSE);
const _: &str = ERROR_CATEGORY;

const fn metadata_mut(parts: &mut RequestParts) -> &mut Vec<u8> { parts.metadata_mut() }

fn main() {
    let _ = (
        RECOVERY_CONFIG,
        ERROR_CONTEXT,
        HOTLINE_CODEC,
        MYSQL_CODEC,
        FRAME_HEADER,
        ASSEMBLED_MESSAGE,
        REQUEST_PARTS,
        metadata_mut,
    );
}
