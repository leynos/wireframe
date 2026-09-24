//! Verifies public client configuration methods remain callable in const contexts.

use std::time::Duration;

use tracing::Level;
use wireframe::{
    client::{
        ClientCodecConfig,
        SendStreamingConfig,
        SendStreamingOutcome,
        SocketOptions,
        TracingConfig,
        WireframeClientBuilder,
    },
    frame::LengthFormat,
};

const STREAMING_OUTCOME: SendStreamingOutcome = SendStreamingOutcome::new(3);
const STREAMING_FRAMES_SENT: u64 = STREAMING_OUTCOME.frames_sent();

// Both public receiver types must compile the same const socket setters.
macro_rules! configure_socket_settings {
    ($target:expr, $keepalive:expr) => {{
        let configured = $target
            .nodelay(true)
            .keepalive(Some($keepalive))
            .send_buffer_size(2048)
            .recv_buffer_size(4096)
            .reuseaddr(true);

        #[cfg(all(
            unix,
            not(target_os = "solaris"),
            not(target_os = "illumos"),
            not(target_os = "cygwin"),
        ))]
        let configured = configured.reuseport(true);

        configured
    }};
}

const fn configure_socket_options(options: SocketOptions, keepalive: Duration) -> SocketOptions {
    configure_socket_settings!(options, keepalive)
}

const fn configure_codec(
    config: ClientCodecConfig,
    length_format: LengthFormat,
) -> (ClientCodecConfig, usize, LengthFormat) {
    let configured = config.length_format(length_format);
    (
        configured,
        configured.max_frame_length_value(),
        configured.length_format_value(),
    )
}

const fn configure_tracing(config: TracingConfig, level: Level) -> TracingConfig {
    config
        .with_connect_level(level)
        .with_connect_timing(true)
        .with_send_level(level)
        .with_send_timing(true)
        .with_receive_level(level)
        .with_receive_timing(true)
        .with_call_level(level)
        .with_call_timing(true)
        .with_streaming_level(level)
        .with_streaming_timing(true)
        .with_close_level(level)
        .with_close_timing(true)
        .with_all_levels(level)
        .with_all_timing(true)
}

const fn configure_streaming(
    config: SendStreamingConfig,
    timeout: Duration,
) -> (SendStreamingConfig, Option<usize>, Option<Duration>) {
    let configured = config.with_chunk_size(1024).with_timeout(timeout);
    (configured, configured.chunk_size(), configured.timeout())
}

const fn configure_builder_socket(
    builder: WireframeClientBuilder,
    socket_options: SocketOptions,
    keepalive: Duration,
) -> WireframeClientBuilder {
    configure_socket_settings!(builder.socket_options(socket_options), keepalive)
}

const fn configure_builder_codec_and_tracing(
    builder: WireframeClientBuilder,
    codec_config: ClientCodecConfig,
    tracing_config: TracingConfig,
    length_format: LengthFormat,
) -> WireframeClientBuilder {
    builder
        .codec_config(codec_config)
        .length_format(length_format)
        .tracing_config(tracing_config)
}

fn main() {
    let _ = (
        STREAMING_FRAMES_SENT,
        configure_socket_options,
        configure_codec,
        configure_tracing,
        configure_streaming,
        configure_builder_socket,
        configure_builder_codec_and_tracing,
    );
}
