//! `InterleavedPushWorld` fixture for rstest-bdd tests.
//!
//! Provides test fixtures exercising interleaved high- and low-priority
//! push queue behaviour under various fairness and rate-limit
//! configurations.

use futures::FutureExt;
use rstest::fixture;
use tokio::time::{self, Duration};
use wireframe::{connection::FairnessConfig, push::PushQueues};
/// Re-export `TestResult` from `wireframe_testing` for use in steps.
pub use wireframe_testing::TestResult;
use wireframe_testing::push_expect;

#[path = "../common/interleaved_push_helpers.rs"]
mod interleaved_push_helpers;

/// Test world capturing push queue output and rate-limiter state for
/// interleaved queue scenarios.
#[derive(Debug, Default)]
pub struct InterleavedPushWorld {
    frames: Vec<u8>,
    rate_limit_blocked: bool,
    rate_limit_recovered: bool,
}

// rustfmt collapses simple fixtures into one line, which triggers
// unused_braces.
#[rustfmt::skip]
#[fixture]
pub fn interleaved_push_world() -> InterleavedPushWorld {
    InterleavedPushWorld::default()
}

impl InterleavedPushWorld {
    /// Build unlimited queues, load frames via `setup`, run the actor
    /// with the given `fairness` config, and collect output into
    /// `self.frames`.
    ///
    /// Delegates to `interleaved_push_helpers::run_actor_with_fairness`
    /// to share setup logic with the unit tests.
    async fn run_actor_with_fairness<F, Fut>(
        &mut self,
        fairness: FairnessConfig,
        setup: F,
    ) -> TestResult
    where
        F: FnOnce(wireframe::push::PushHandle<u8>) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        self.frames = interleaved_push_helpers::run_actor_with_fairness(fairness, setup).await?;
        Ok(())
    }

    /// Run the actor with fairness disabled and both queues loaded.
    ///
    /// # Errors
    ///
    /// Returns an error if queue construction or actor execution fails.
    pub async fn run_strict_priority(&mut self) -> TestResult {
        self.run_actor_with_fairness(
            FairnessConfig {
                max_high_before_low: 0,
                time_slice: None,
            },
            |handle| async move {
                let _ = push_expect!(handle.push_low_priority(101));
                let _ = push_expect!(handle.push_low_priority(102));
                let _ = push_expect!(handle.push_high_priority(1));
                let _ = push_expect!(handle.push_high_priority(2));
                let _ = push_expect!(handle.push_high_priority(3));
            },
        )
        .await
    }

    /// Run the actor with counter-based fairness.
    ///
    /// # Errors
    ///
    /// Returns an error if queue construction or actor execution fails.
    pub async fn run_fairness_threshold(&mut self) -> TestResult {
        self.run_actor_with_fairness(
            FairnessConfig {
                max_high_before_low: 3,
                time_slice: None,
            },
            |handle| async move {
                for n in 1..=6 {
                    let _ = push_expect!(handle.push_high_priority(n));
                }
                let _ = push_expect!(handle.push_low_priority(101));
                let _ = push_expect!(handle.push_low_priority(102));
            },
        )
        .await
    }

    /// Test rate-limit symmetry: a high-priority push blocks a
    /// subsequent low-priority push until the token bucket refills.
    ///
    /// # Errors
    ///
    /// Returns an error if queue construction fails.
    pub async fn run_rate_limit_symmetry(&mut self) -> TestResult {
        time::pause();
        let (_queues, handle) = PushQueues::<u8>::builder()
            .high_capacity(4)
            .low_capacity(4)
            .rate(Some(1))
            .build()?;

        let _ = push_expect!(handle.push_high_priority(1));

        // The low-priority push should be blocked: the single token was
        // consumed by the high-priority push above.
        let mut blocked = handle.push_low_priority(2).boxed();
        tokio::task::yield_now().await;
        self.rate_limit_blocked = blocked.as_mut().now_or_never().is_none();

        // Advance past the refill window and verify the push unblocks,
        // proving the shared bucket recovers for both priorities.
        time::advance(Duration::from_secs(1)).await;
        self.rate_limit_recovered = matches!(blocked.as_mut().now_or_never(), Some(Ok(())));
        Ok(())
    }

    /// Run the actor with fairness enabled and verify all frames are
    /// delivered.
    ///
    /// # Errors
    ///
    /// Returns an error if queue construction or actor execution fails.
    pub async fn run_interleaved_delivery(&mut self) -> TestResult {
        self.run_actor_with_fairness(
            FairnessConfig {
                max_high_before_low: 2,
                time_slice: None,
            },
            |handle| async move {
                for n in 1..=5 {
                    let _ = push_expect!(handle.push_high_priority(n));
                }
                for n in 101..=105 {
                    let _ = push_expect!(handle.push_low_priority(n));
                }
            },
        )
        .await
    }

    /// Assert all high-priority frames precede all low-priority frames.
    ///
    /// # Panics
    ///
    /// Panics if the ordering constraint is violated.
    pub fn verify_strict_priority(&self) {
        assert_eq!(
            self.frames,
            vec![1, 2, 3, 101, 102],
            "all high frames should precede all low frames"
        );
    }

    /// Assert low-priority frames are interleaved every 3 high frames.
    ///
    /// # Panics
    ///
    /// Panics if the interleaving pattern is incorrect.
    pub fn verify_fairness_threshold(&self) {
        assert_eq!(
            self.frames,
            vec![1, 2, 3, 101, 4, 5, 6, 102],
            "low frames should be interleaved every 3 high frames"
        );
    }

    /// Assert the low-priority push was blocked by the rate limiter and
    /// recovered after the refill window.
    ///
    /// # Panics
    ///
    /// Panics if the low-priority push was not blocked or did not
    /// recover.
    pub fn verify_rate_limit_blocked(&self) {
        assert!(
            self.rate_limit_blocked,
            "low-priority push should be blocked after high consumed the token"
        );
        assert!(
            self.rate_limit_recovered,
            "low-priority push should succeed after the refill window"
        );
    }

    /// Assert all 10 frames (5 high + 5 low) are present.
    ///
    /// # Panics
    ///
    /// Panics if any frames are missing.
    pub fn verify_all_delivered(&self) {
        assert_eq!(self.frames.len(), 10, "all 10 frames should be delivered");
        let mut high: Vec<u8> = self.frames.iter().copied().filter(|&f| f <= 5).collect();
        let mut low: Vec<u8> = self.frames.iter().copied().filter(|&f| f >= 101).collect();
        high.sort_unstable();
        low.sort_unstable();
        assert_eq!(high, vec![1, 2, 3, 4, 5], "all high frames present");
        assert_eq!(low, vec![101, 102, 103, 104, 105], "all low frames present");
    }
}
