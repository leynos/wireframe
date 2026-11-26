//! BDD world for protocol-agnostic server lifecycle harness scenarios.

use std::{
    io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use rstest::fixture;
use tokio::net::TcpStream;
use wireframe::server::{ServerError, WireframeServer as Server};
pub use wireframe_testing::TestResult;
use wireframe_testing::{
    RunningWireframeServer,
    ShutdownOutcome,
    TestError,
    echo_app_factory,
    spawn_wireframe_server,
    wait_for_listener_release,
};

/// BDD world for server lifecycle harness scenarios.
pub struct ServerHarnessLifecycleWorld {
    /// Runtime executing every asynchronous world operation.
    runtime: tokio::runtime::Runtime,
    /// Invocation counter shared with the echo factory.
    counter: Arc<AtomicUsize>,
    /// Running handle, present between a successful start and its end.
    handle: Option<RunningWireframeServer>,
    /// Address reported by the handle at startup.
    addr: Option<SocketAddr>,
    /// Failure reported by the start step, when it failed.
    start_error: Option<TestError>,
    /// Whether the configured server's factory should fail.
    factory_fails: bool,
    /// Results of the stop attempts, in call order.
    stop_results: Vec<TestResult<()>>,
}

impl std::fmt::Debug for ServerHarnessLifecycleWorld {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerHarnessLifecycleWorld")
            .field("runtime", &"..")
            .field("counter", &self.counter.load(Ordering::SeqCst))
            .field("handle", &self.handle.as_ref().map(|_| ".."))
            .field("addr", &self.addr)
            .field("start_error", &self.start_error)
            .field("factory_fails", &self.factory_fails)
            .field("stop_results", &self.stop_results)
            .finish()
    }
}

/// Fixture for [`ServerHarnessLifecycleWorld`].
#[rustfmt::skip]
#[fixture]
pub fn server_harness_lifecycle_world() -> TestResult<ServerHarnessLifecycleWorld> {
    Ok(ServerHarnessLifecycleWorld {
        runtime: tokio::runtime::Runtime::new().map_err(|error| {
            format!("server harness lifecycle fixture setup failed: {error}")
        })?,
        counter: Arc::new(AtomicUsize::new(0)),
        handle: None,
        addr: None,
        start_error: None,
        factory_fails: false,
        stop_results: Vec::new(),
    })
}

impl ServerHarnessLifecycleWorld {
    /// Configure an echo server that has not been bound.
    pub const fn given_echo_server(&mut self) { self.factory_fails = false; }

    /// Configure a server whose application factory fails.
    pub const fn given_failing_factory(&mut self) { self.factory_fails = true; }

    /// Build the configured server and start it through the harness.
    ///
    /// A failed start is stored so the matching `Then` step can assert it.
    pub fn start_server(&mut self) {
        let started = if self.factory_fails {
            let failing = || -> Result<wireframe::app::WireframeApp, io::Error> {
                Err(io::Error::other("refused by the behavioural factory"))
            };
            self.runtime
                .block_on(spawn_wireframe_server(Server::new(failing).workers(1)))
        } else {
            let counter = Arc::clone(&self.counter);
            let server = Server::new(echo_app_factory(&counter)).workers(1);
            self.runtime.block_on(spawn_wireframe_server(server))
        };
        match started {
            Ok(handle) => {
                self.addr = Some(handle.local_addr());
                self.handle = Some(handle);
            }
            Err(error) => self.start_error = Some(error),
        }
    }

    /// Stop the running handle twice, recording both results.
    ///
    /// # Errors
    ///
    /// Returns an error when no handle is running.
    pub fn stop_twice(&mut self) -> TestResult {
        for _ in 0..2 {
            let handle = self.handle.as_mut().ok_or("no running server handle")?;
            self.stop_results
                .push(self.runtime.block_on(handle.shutdown()));
        }
        Ok(())
    }

    /// Drop the running handle without stopping it explicitly.
    ///
    /// # Errors
    ///
    /// Returns an error when no handle is running.
    pub fn drop_without_stopping(&mut self) -> TestResult {
        let handle = self.handle.take().ok_or("no running server handle")?;
        drop(handle);
        Ok(())
    }

    /// Assert the reported address is a loopback endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is missing or not loopback.
    pub fn assert_loopback_address(&self) -> TestResult {
        let addr = self.addr.ok_or("no server address recorded")?;
        if !addr.ip().is_loopback() {
            return Err(format!("expected a loopback address, got {addr}").into());
        }
        if addr.port() == 0 {
            return Err("expected a bound port".into());
        }
        Ok(())
    }

    /// Assert a client can open a connection to the reported address.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is missing or the connect fails.
    pub fn assert_client_can_connect(&self) -> TestResult {
        let addr = self.addr.ok_or("no server address recorded")?;
        self.runtime.block_on(TcpStream::connect(addr))?;
        Ok(())
    }

    /// Assert both stop attempts succeeded.
    ///
    /// # Errors
    ///
    /// Returns an error when either attempt failed or was not recorded.
    pub fn assert_both_stops_succeeded(&self) -> TestResult {
        if self.stop_results.len() != 2 {
            return Err("expected exactly two stop attempts".into());
        }
        for (index, result) in self.stop_results.iter().enumerate() {
            if result.is_err() {
                return Err(format!("stop attempt {index} failed: {result:?}").into());
            }
        }
        Ok(())
    }

    /// Assert the handle recorded a completed shutdown.
    ///
    /// # Errors
    ///
    /// Returns an error when no handle exists or the outcome differs.
    pub fn assert_completed(&self) -> TestResult {
        let handle = self.handle.as_ref().ok_or("no running server handle")?;
        if handle.shutdown_outcome() != Some(ShutdownOutcome::Completed) {
            return Err(format!(
                "expected a completed shutdown, got {:?}",
                handle.shutdown_outcome()
            )
            .into());
        }
        Ok(())
    }

    /// Assert the address refuses new connections immediately.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is missing or still accepts.
    pub fn assert_refuses_connections(&self) -> TestResult {
        let addr = self.addr.ok_or("no server address recorded")?;
        match self.runtime.block_on(TcpStream::connect(addr)) {
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => Ok(()),
            Ok(stream) => {
                drop(stream);
                Err(format!("the listener at {addr} still accepts connections").into())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Assert the address stops accepting within one second.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is missing or never released.
    pub fn assert_released_within_one_second(&self) -> TestResult {
        let addr = self.addr.ok_or("no server address recorded")?;
        self.runtime.block_on(wait_for_listener_release(addr))
    }

    /// Assert the start failed with a typed application startup error.
    ///
    /// # Errors
    ///
    /// Returns an error when no failure was recorded or it has another shape.
    pub fn assert_application_startup_error(&self) -> TestResult {
        match self.start_error.as_ref() {
            Some(TestError::Server(ServerError::FactoryBuild(_))) => Ok(()),
            Some(other) => Err(format!("expected a factory build error, got {other:?}").into()),
            None => Err("expected the start to fail, but it succeeded".into()),
        }
    }
}

impl Drop for ServerHarnessLifecycleWorld {
    fn drop(&mut self) {
        if let Some(mut handle) = self.handle.take() {
            drop(self.runtime.block_on(handle.shutdown()));
        }
    }
}
