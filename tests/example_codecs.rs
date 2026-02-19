//! Tests for shared example codecs.
#![cfg(not(loom))]

use std::{io, sync::Arc};

use bytes::{BufMut, Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::codec::{Decoder, Encoder};
use wireframe::{
    app::{Envelope, Packet, WireframeApp},
    codec::{
        FrameCodec,
        examples::{HotlineFrame, HotlineFrameCodec, MysqlFrame, MysqlFrameCodec},
    },
    correlation::CorrelatableFrame,
    serializer::{BincodeSerializer, Serializer},
};

#[test]
fn hotline_codec_round_trips_payload_and_correlation() {
    let codec = HotlineFrameCodec::new(8);
    let payload = Bytes::from(vec![1_u8, 2, 3, 4, 5, 6, 7, 8]);
    let frame = HotlineFrame {
        transaction_id: 42,
        payload: payload.clone(),
    };

    let mut encoder = codec.encoder();
    let mut buf = BytesMut::new();
    encoder.encode(frame, &mut buf).expect("encode frame");

    let mut decoder = codec.decoder();
    let decoded_frame = decoder
        .decode(&mut buf)
        .expect("decode frame")
        .expect("missing frame");

    assert!(buf.is_empty(), "unexpected trailing bytes");
    assert_eq!(decoded_frame.transaction_id, 42);
    assert_eq!(decoded_frame.payload, payload);
    assert_eq!(HotlineFrameCodec::correlation_id(&decoded_frame), Some(42));
}

#[test]
fn hotline_codec_rejects_invalid_total_size() {
    let codec = HotlineFrameCodec::new(16);
    let mut decoder = codec.decoder();
    let mut buf = BytesMut::new();
    buf.put_u32(1);
    buf.put_u32(1);
    buf.put_u32(7);
    buf.extend_from_slice(&[0_u8; 8]);

    let err = decoder
        .decode(&mut buf)
        .expect_err("expected invalid total size error");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn mysql_codec_round_trips_payload_and_correlation() {
    let codec = MysqlFrameCodec::new(5);
    let payload = Bytes::from(vec![9_u8, 8, 7, 6, 5]);
    let frame = MysqlFrame {
        sequence_id: 3,
        payload: payload.clone(),
    };

    let mut encoder = codec.encoder();
    let mut buf = BytesMut::new();
    encoder.encode(frame, &mut buf).expect("encode frame");

    let mut decoder = codec.decoder();
    let decoded_frame = decoder
        .decode(&mut buf)
        .expect("decode frame")
        .expect("missing frame");

    assert!(buf.is_empty(), "unexpected trailing bytes");
    assert_eq!(decoded_frame.sequence_id, 3);
    assert_eq!(decoded_frame.payload, payload);
    assert_eq!(MysqlFrameCodec::correlation_id(&decoded_frame), Some(3));
}

#[test]
fn mysql_codec_rejects_oversized_payload() {
    let codec = MysqlFrameCodec::new(3);
    let mut encoder = codec.encoder();
    let mut buf = BytesMut::new();

    let err = encoder
        .encode(
            MysqlFrame {
                sequence_id: 0,
                payload: Bytes::from(vec![0_u8; 4]),
            },
            &mut buf,
        )
        .expect_err("expected oversized payload error");
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[tokio::test]
async fn hotline_codec_round_trips_through_app() {
    let codec = HotlineFrameCodec::new(64);
    let app = WireframeApp::<BincodeSerializer, (), Envelope>::new()
        .expect("build app")
        .with_codec(codec.clone())
        .route(1, Arc::new(|_: &Envelope| Box::pin(async {})))
        .expect("route handler");

    let (mut client, server) = tokio::io::duplex(256);
    let server_task = tokio::spawn(async move {
        app.handle_connection_result(server)
            .await
            .expect("server should exit cleanly");
    });

    let request = Envelope::new(1, None, b"ping".to_vec());
    let payload = BincodeSerializer
        .serialize(&request)
        .expect("serialize request");

    let mut encoder = codec.encoder();
    let mut buf = BytesMut::new();
    encoder
        .encode(
            HotlineFrame {
                transaction_id: 7,
                payload: Bytes::from(payload),
            },
            &mut buf,
        )
        .expect("encode request");

    client.write_all(&buf).await.expect("write request");
    client.shutdown().await.expect("shutdown client");

    let mut output = Vec::new();
    client
        .read_to_end(&mut output)
        .await
        .expect("read response");

    server_task.await.expect("join server task");

    let mut decoder = codec.decoder();
    let mut response_buf = BytesMut::from(&output[..]);
    let response_frame = decoder
        .decode(&mut response_buf)
        .expect("decode response")
        .expect("response frame");
    assert!(response_buf.is_empty(), "unexpected trailing bytes");

    let (response_env, _) = BincodeSerializer
        .deserialize::<Envelope>(&response_frame.payload)
        .expect("deserialize response");
    assert_eq!(response_env.correlation_id(), Some(7));
    let response_payload = response_env.into_parts().into_payload();
    assert_eq!(response_payload, b"ping".to_vec());
}
