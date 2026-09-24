//! Real-socket and generated-order tests for prepared applications.

use super::*;

/// Bounded waits keep the real-socket test from hanging on a stalled peer.
const TCP_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Accepts one TCP connection and processes it with the prepared application.
async fn serve_one_prepared_tcp_connection(
    listener: TcpListener,
    prepared: TestPreparedApp,
) -> TestResult<()> {
    let (stream, _peer) = timeout(TCP_OPERATION_TIMEOUT, listener.accept())
        .await
        .map_err(|_| "prepared TCP accept timed out".to_string())??;
    timeout(
        TCP_OPERATION_TIMEOUT,
        prepared.handle_connection_result(stream),
    )
    .await
    .map_err(|_| "prepared TCP connection processing timed out".to_string())??;
    Ok(())
}

/// Sends one encoded frame to the server and returns the decoded response.
async fn exchange_one_tcp_frame(address: SocketAddr, frame: Vec<u8>) -> TestResult<Vec<u8>> {
    let mut client = timeout(TCP_OPERATION_TIMEOUT, TcpStream::connect(address))
        .await
        .map_err(|_| "prepared TCP client connect timed out".to_string())??;
    client.write_all(&frame).await?;
    // Signal end-of-input so the server can finish the request-response cycle.
    client.shutdown().await?;
    let mut response = Vec::new();
    timeout(TCP_OPERATION_TIMEOUT, client.read_to_end(&mut response))
        .await
        .map_err(|_| "prepared TCP response read timed out".to_string())??;
    response_payload(&response)
}

#[tokio::test]
async fn prepared_app_serves_real_tcp_connection_without_retransforming() -> TestResult<()> {
    let instrumentation = ConnectionStartupInstrumentation::new();
    let prepared: TestPreparedApp = counted_app_factory(instrumentation.clone())()?
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let prepared_counts = ConnectionStartupCounts {
        factory_calls: 1,
        transforms: ROUTES * MIDDLEWARE_LAYERS,
    };
    check_equal(
        &instrumentation.snapshot(),
        &prepared_counts,
        "preparation should transform each route once",
    )?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(serve_one_prepared_tcp_connection(listener, prepared));

    let payload = exchange_one_tcp_frame(address, build_frame(1, vec![b'X'])?).await?;
    // The request gains tags A then B in wrap order; the response unwinds them.
    check_equal(
        &payload,
        b"XABBA",
        "the real TCP response should follow middleware order",
    )?;

    let joined = timeout(TCP_OPERATION_TIMEOUT, server)
        .await
        .map_err(|_| "prepared TCP accept task did not finish".to_string())?;
    // Surfaces both a failed join and a failed connection-processing result.
    joined??;
    // Preparation transformed each route once; the connection added none.
    check_equal(
        &instrumentation.snapshot(),
        &prepared_counts,
        "the real TCP connection should not add transforms",
    )?;
    Ok(())
}

// Generate bounded prepared-application cases and preserve one-time transforms.
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prepared_app_transforms_once_and_reuses_services(
        route_count in 1usize..=4,
        middleware_layers in 0usize..=4,
        connection_count in 1usize..=4,
    ) {
        run_prepared_app_property_case(route_count, middleware_layers, connection_count)?;
    }
}

/// Exercise a generated preparation case on a deterministic Tokio runtime.
fn run_prepared_app_property_case(
    route_count: usize,
    middleware_layers: usize,
    connection_count: usize,
) -> TestCaseResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TestCaseError::fail(error.to_string()))?;
    runtime
        .block_on(exercise_prepared_app_property_case(
            route_count,
            middleware_layers,
            connection_count,
        ))
        .map_err(|error| TestCaseError::fail(error.to_string()))
}

/// Prepare a bounded generated application and verify every requested dispatch.
async fn exercise_prepared_app_property_case(
    route_count: usize,
    middleware_layers: usize,
    connection_count: usize,
) -> TestResult<()> {
    let transforms = Arc::new(AtomicUsize::new(0));
    let tags = middleware_tags(middleware_layers)?;
    let mut app = TestApp::new()?;
    for route_id in 1..=route_count {
        app = app.route(u32::try_from(route_id)?, handler())?;
    }
    for tag in &tags {
        app = app.wrap(TransformCountingMiddleware {
            tag: *tag,
            transforms: Arc::clone(&transforms),
        })?;
    }

    let prepared = app
        .prepare()
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { Box::new(error) })?;
    let expected_transforms = route_count * middleware_layers;
    if transforms.load(Ordering::SeqCst) != expected_transforms {
        return Err(format!(
            "preparation transformed {} route services, expected {expected_transforms}",
            transforms.load(Ordering::SeqCst)
        )
        .into());
    }

    for route_id in (1..=route_count)
        .cycle()
        .take(route_count + connection_count)
    {
        let route_id = u32::try_from(route_id)?;
        let payload = vec![u8::try_from(route_id)?];
        let response =
            drive_prepared_with_frames(&prepared, vec![build_frame(route_id, payload)?]).await?;
        let mut expected = vec![u8::try_from(route_id)?];
        expected.extend(tags.iter().copied());
        expected.extend(tags.iter().rev().copied());
        if response_payload(&response)? != expected {
            return Err(
                format!("route {route_id} did not preserve generated middleware order").into(),
            );
        }
    }
    if transforms.load(Ordering::SeqCst) != expected_transforms {
        return Err(format!(
            "prepared connections rebuilt middleware: observed {}, expected {expected_transforms}",
            transforms.load(Ordering::SeqCst)
        )
        .into());
    }
    Ok(())
}

/// Build distinct middleware tags for a bounded generated layer count.
fn middleware_tags(layer_count: usize) -> TestResult<Vec<u8>> {
    (0..layer_count)
        .map(|layer| Ok(b'A' + u8::try_from(layer)?))
        .collect()
}
