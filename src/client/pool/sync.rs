//! Pool lock helper shared across client pool internals.
//!
//! [`lock_or_recover`] recovers poisoned mutexes for pool bookkeeping instead
//! of propagating poison. Use it only where a subsequent scheduler or slot
//! operation re-establishes consistency; see the function docs for detailed
//! constraints.

use std::sync::{Mutex, MutexGuard};

use tracing::warn;

/// Lock pool bookkeeping state, recovering the inner value after poison.
///
/// # When to use
///
/// Use this helper only for pool-internal bookkeeping whose consistency is
/// re-established by the next scheduler or slot operation. Current callers are
/// the pool scheduler waiter table and slot-return timestamp state.
///
/// Do not use this helper for connection state, protocol state, serializer
/// state, lifecycle hook state, or user-provided state. Those locks must
/// propagate poison so callers cannot continue after a panic-interrupted
/// mutation.
///
/// # Observability
///
/// Poison recovery is logged at warning level before the guarded value is
/// returned, so unexpected panic recovery remains visible to operators.
pub(super) fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            crate::metrics::inc_pool_bookkeeping_poison_recoveries();
            warn!("recovering poisoned client pool bookkeeping lock");
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for pool lock poison recovery.

    use std::{
        io,
        sync::{Arc, Mutex},
        thread,
    };

    use googletest::{gtest, prelude::*};
    use tracing::Level;
    use tracing_subscriber::fmt::MakeWriter;
    use wireframe_testing::ObservabilityHandle;

    use super::lock_or_recover;

    #[gtest]
    fn lock_or_recover_reads_unpoisoned_mutex() {
        let mutex = Mutex::new(42);

        let guard = lock_or_recover(&mutex);

        expect_that!(*guard, eq(42));
    }

    #[gtest]
    fn lock_or_recover_reads_poisoned_mutex() {
        // This covers the local recovery primitive. Issue #539 tracks the
        // broader scheduler/slot integration case where a later pool operation
        // must prove bookkeeping consistency is re-established after poison.
        let mutex = Arc::new(Mutex::new(42));
        let poisoned_mutex = Arc::clone(&mutex);
        let join_result = thread::spawn(move || {
            let _guard = lock_or_recover(&poisoned_mutex);
            panic!("poison pool lock for recovery coverage");
        })
        .join();

        expect_that!(join_result, err(anything()));
        expect_that!(mutex.is_poisoned(), eq(true));

        let captured_logs = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(Level::WARN)
            .with_writer(CaptureWriter::new(Arc::clone(&captured_logs)))
            .finish();
        let mut observability = ObservabilityHandle::new();
        metrics::with_local_recorder(observability.recorder(), || {
            tracing::subscriber::with_default(subscriber, || {
                let guard = lock_or_recover(&mutex);

                expect_that!(*guard, eq(42));
            });
        });
        observability.snapshot();
        let log_bytes = lock_or_recover(&captured_logs).clone();
        let logs = String::from_utf8_lossy(&log_bytes);

        expect_that!(
            logs.as_ref(),
            contains_substring("recovering poisoned client pool bookkeeping lock")
        );
        expect_that!(
            observability
                .counter_without_labels(crate::metrics::POOL_BOOKKEEPING_POISON_RECOVERIES),
            eq(1)
        );
    }

    #[derive(Clone)]
    struct CaptureWriter {
        captured: Arc<Mutex<Vec<u8>>>,
    }

    impl CaptureWriter {
        fn new(captured: Arc<Mutex<Vec<u8>>>) -> Self { Self { captured } }
    }

    impl<'a> MakeWriter<'a> for CaptureWriter {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer { self.clone() }
    }

    impl io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            lock_or_recover(&self.captured).extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
}
