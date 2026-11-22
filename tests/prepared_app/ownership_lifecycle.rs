//! Lifecycle-state proofs across overlapping prepared connections.
//!
//! Two connections are alive at the same time. Each setup hook produces its
//! own state, and each teardown must observe its own connection's state
//! exactly once. This is the lifecycle counterpart to the shared assembler and
//! serializer probe in `ownership_interleaved`.

use std::{
    error::Error,
    sync::{
        Arc,
        Mutex,
        PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{
    io::{AsyncWriteExt, duplex},
    time::{sleep, timeout},
};
use wireframe::{
    app::{Envelope, Handler, WireframeApp},
    serializer::BincodeSerializer,
};
use wireframe_testing::TestResult;

use super::{
    fallible_check_equal::check_equal,
    ownership_interleaved::{finish_connection, spawn_connection},
};

/// Per-connection state identified so its teardown can be matched.
struct ConnectionState {
    /// Identifier assigned by the setup hook.
    id: u64,
}

/// Handler that ignores every request; this test only needs live routes.
fn no_op_handler() -> Handler<Envelope> { Arc::new(|_env: &Envelope| Box::pin(async {})) }

/// Box a preparation or lifecycle error for the test result.
fn boxed_error(error: impl Error + Send + Sync + 'static) -> Box<dyn Error + Send + Sync> {
    Box::new(error)
}

/// Snapshot a lifecycle log.
fn events(log: &Arc<Mutex<Vec<u64>>>) -> Vec<u64> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Wait until a lifecycle log holds at least `expected` entries.
async fn wait_for_events(log: &Arc<Mutex<Vec<u64>>>, expected: usize) -> TestResult<()> {
    timeout(Duration::from_secs(2), async {
        while events(log).len() < expected {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| {
        format!(
            "expected {expected} lifecycle events, saw {:?}",
            events(log)
        )
    })?;
    Ok(())
}

/// Build the setup hook that gives each connection its own state.
///
/// The identifier is assigned when setup runs, and the same call records it so
/// both setups can be observed before either client closes.
fn connection_setup(
    next_id: Arc<AtomicU64>,
    setups: Arc<Mutex<Vec<u64>>>,
) -> impl Fn() -> std::future::Ready<ConnectionState> + Send + Sync + 'static {
    move || {
        let id = next_id.fetch_add(1, Ordering::SeqCst);
        setups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(id);
        std::future::ready(ConnectionState { id })
    }
}

/// Build the teardown hook that records the state it was given.
fn connection_teardown(
    teardowns: Arc<Mutex<Vec<u64>>>,
) -> impl Fn(ConnectionState) -> std::future::Ready<()> + Send + Sync + 'static {
    move |state| {
        teardowns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(state.id);
        std::future::ready(())
    }
}

/// Two live connections receive distinct setup state, each torn down once.
///
/// Connection A starts alone, so the first setup belongs to A by construction.
/// B then starts while A is still live, so the two states overlap. Each
/// teardown is checked against its own connection's setup state before the
/// other connection closes.
#[tokio::test]
async fn overlapping_connections_receive_distinct_setup_state() -> TestResult<()> {
    let setups = Arc::new(Mutex::new(Vec::new()));
    let teardowns = Arc::new(Mutex::new(Vec::new()));
    let next_id = Arc::new(AtomicU64::new(1));
    let prepared = WireframeApp::<BincodeSerializer, (), Envelope>::new()?
        .on_connection_setup(connection_setup(Arc::clone(&next_id), Arc::clone(&setups)))
        .map_err(boxed_error)?
        .on_connection_teardown(connection_teardown(Arc::clone(&teardowns)))
        .map_err(boxed_error)?
        .route(1, no_op_handler())?
        .prepare()
        .await
        .map_err(boxed_error)?;
    let root = Arc::new(prepared);

    let (mut client_a, server_a) = duplex(64);
    let (mut client_b, server_b) = duplex(64);
    let task_a = spawn_connection(Arc::clone(&root), server_a);
    wait_for_events(&setups, 1).await?;
    let a_id = events(&setups)
        .first()
        .copied()
        .ok_or("connection A should set up")?;

    let task_b = spawn_connection(Arc::clone(&root), server_b);
    wait_for_events(&setups, 2).await?;
    let setup_ids = events(&setups);
    let b_id = setup_ids
        .get(1)
        .copied()
        .ok_or("connection B should set up")?;
    check_equal(
        &(a_id != b_id),
        &true,
        "each connection received its own setup state",
    )?;

    client_a.shutdown().await?;
    finish_connection(task_a).await?;
    wait_for_events(&teardowns, 1).await?;
    check_equal(
        &events(&teardowns),
        &vec![a_id],
        "connection A's teardown received its own setup state",
    )?;

    client_b.shutdown().await?;
    finish_connection(task_b).await?;
    wait_for_events(&teardowns, 2).await?;
    check_equal(
        &events(&teardowns),
        &vec![a_id, b_id],
        "connection B's teardown received its own setup state, once",
    )?;
    Ok(())
}
