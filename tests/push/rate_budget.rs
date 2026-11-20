//! Property coverage for the push rate budget.
//!
//! These cases live beside the routing tests rather than inside them so that
//! the parent file stays a readable set of examples.

use proptest::{prelude::*, test_runner::TestCaseError};
use tokio::runtime::Builder;
use wireframe::push::{PushError, PushPolicy, PushPriority};

use super::support;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        ..ProptestConfig::default()
    })]

    /// A rate budget admits exactly its initial allowance, however it is spent.
    ///
    /// The bucket starts full with one permit per configured unit per second,
    /// so the first `rate` non-blocking pushes are admitted and every later
    /// attempt is refused without spending budget. This pins the accounting for
    /// every combination of rate and attempt count, including no attempts and a
    /// rate above the number of attempts, which a table of named cases cannot
    /// cover without repeating the same assertion for each row. The clock is
    /// paused before the handle is built, so a real-clock stall between two
    /// attempts cannot refill a permit and admit an extra push.
    #[test]
    fn try_push_admits_exactly_the_initial_rate_budget(
        rate in 1usize..=16,
        attempts in 0usize..=24,
    ) {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(|error| TestCaseError::fail(format!("a paused runtime must build: {error}")))?;
        runtime.block_on(async move {
            // The lanes outgrow every generated attempt count, so capacity can
            // never be the reason a push is refused.
            let (_queues, handle) = support::builder::<u8>()
                .high_capacity(32)
                .low_capacity(32)
                .rate(Some(rate))
                .build()
                .map_err(|error| {
                    TestCaseError::fail(format!("a valid rate and capacity must build: {error:?}"))
                })?;

            for attempt in 0..attempts {
                let frame = u8::try_from(attempt).expect("the attempt count fits in u8");
                let outcome = handle.try_push(frame, PushPriority::High, PushPolicy::DropIfFull);
                if attempt < rate {
                    prop_assert_eq!(
                        outcome,
                        Ok(()),
                        "an attempt inside the initial budget must be admitted"
                    );
                } else {
                    prop_assert_eq!(
                        outcome,
                        Err(PushError::RateLimited),
                        "an attempt past the initial budget must be refused"
                    );
                }
            }
            Ok(())
        })?;
    }
}
