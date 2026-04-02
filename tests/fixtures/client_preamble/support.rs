//! Connection and server orchestration helpers for `ClientPreambleWorld`.

use super::*;

impl ClientPreambleWorld {
    /// Start a preamble-aware echo server.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    pub async fn start_preamble_server(&mut self) -> TestResult {
        let (tx, rx) = oneshot::channel::<TestPreamble>();
        self.server_preamble_rx = Some(rx);
        self.spawn_server(|mut stream| async move {
            let (preamble, _) = read_preamble::<_, TestPreamble>(&mut stream)
                .await
                .map_err(preamble_decode_error)?;
            let _ = tx.send(preamble);
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(())
        })
        .await
    }

    /// Start a preamble-aware server that sends acknowledgement.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    pub async fn start_ack_server(&mut self) -> TestResult {
        self.spawn_server_after_preamble(|mut stream| async move {
            write_preamble(&mut stream, &ServerAck { accepted: true })
                .await
                .map_err(preamble_encode_error)?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(())
        })
        .await
    }

    /// Start a preamble-aware server that replies with invalid acknowledgement
    /// bytes.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    pub async fn start_invalid_ack_server(&mut self) -> TestResult {
        self.spawn_server_after_preamble(|mut stream| async move {
            stream.write_all(&INVALID_ACK_BYTES).await?;
            Ok(())
        })
        .await
    }

    /// Start a server that never responds (for timeout testing).
    ///
    /// # Errors
    /// Returns an error if binding fails.
    pub async fn start_slow_server(&mut self) -> TestResult {
        self.spawn_server(|_stream| async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        })
        .await
    }

    /// Start a standard echo server without preamble support.
    ///
    /// # Errors
    /// Returns an error if binding fails.
    pub async fn start_standard_server(&mut self) -> TestResult {
        self.spawn_server(|_stream| async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok(())
        })
        .await
    }

    /// Connect with preamble and success callback.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_preamble(&mut self, version: u16) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let (holder, rx) = create_signal_channel::<()>();

        let result = WireframeClient::builder()
            .with_preamble(TestPreamble::new(version))
            .on_preamble_success(move |_preamble, _stream| {
                let holder = holder.clone();
                async move {
                    send_signal(&holder, ());
                    Ok(Vec::new())
                }
                .boxed()
            })
            .connect(addr)
            .await;

        match result {
            Ok(client) => {
                self.client = Some(client);
                if matches!(
                    tokio::time::timeout(Duration::from_secs(1), rx).await,
                    Ok(Ok(()))
                ) {
                    self.success_callback_invoked = true;
                }
            }
            Err(error) => {
                self.last_error = Some(error);
            }
        }

        if let Some(preamble_rx) = self.server_preamble_rx.take()
            && let Ok(Ok(preamble)) =
                tokio::time::timeout(Duration::from_secs(1), preamble_rx).await
        {
            self.server_received_preamble = Some(preamble);
        }

        Ok(())
    }

    /// Connect with preamble and read acknowledgement.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_ack(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let (holder, rx) = create_signal_channel::<ServerAck>();

        let result = WireframeClient::builder()
            .with_preamble(TestPreamble::new(1))
            .on_preamble_success(move |_preamble, stream| {
                let holder = holder.clone();
                async move {
                    let (ack, leftover) = read_preamble::<_, ServerAck>(stream)
                        .await
                        .map_err(preamble_decode_error)?;
                    send_signal(&holder, ack);
                    Ok(leftover)
                }
                .boxed()
            })
            .connect(addr)
            .await;

        match result {
            Ok(client) => {
                self.client = Some(client);
                if let Ok(Ok(ack)) = tokio::time::timeout(Duration::from_secs(1), rx).await {
                    self.client_received_ack = Some(ack);
                    self.success_callback_invoked = true;
                }
            }
            Err(error) => {
                self.last_error = Some(error);
            }
        }
        Ok(())
    }

    /// Connect with a preamble timeout.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_timeout(&mut self, timeout_ms: u64) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let (failure_holder, failure_rx) = create_signal_channel::<()>();

        let result = WireframeClient::builder()
            .with_preamble(TestPreamble::new(1))
            .preamble_timeout(Duration::from_millis(timeout_ms))
            .on_preamble_success(|_preamble, stream| {
                async move {
                    use tokio::io::AsyncReadExt;

                    let mut buf = [0u8; 1];
                    stream.read_exact(&mut buf).await?;
                    Ok(Vec::new())
                }
                .boxed()
            })
            .on_preamble_failure(make_failure_signal_callback(failure_holder))
            .connect(addr)
            .await;

        self.store_connect_result_with_failure_signal(result, failure_rx)
            .await;
        Ok(())
    }

    /// Connect with a preamble and capture invalid acknowledgement read
    /// failures.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_with_invalid_ack(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let (failure_holder, failure_rx) = create_signal_channel::<()>();

        let result = WireframeClient::builder()
            .with_preamble(TestPreamble::new(1))
            .on_preamble_success(|_preamble, stream| {
                async move {
                    let (_ack, leftover) = read_preamble::<_, ServerAck>(stream)
                        .await
                        .map_err(preamble_decode_error)?;
                    Ok(leftover)
                }
                .boxed()
            })
            .on_preamble_failure(make_failure_signal_callback(failure_holder))
            .connect(addr)
            .await;

        self.store_connect_result_with_failure_signal(result, failure_rx)
            .await;
        Ok(())
    }

    /// Connect without a preamble.
    ///
    /// # Errors
    /// Returns an error if server address is missing.
    pub async fn connect_without_preamble(&mut self) -> TestResult {
        let addr = self.addr.ok_or("server address missing")?;
        let result = WireframeClient::builder().connect(addr).await;
        self.store_connect_result(result);
        Ok(())
    }
}
