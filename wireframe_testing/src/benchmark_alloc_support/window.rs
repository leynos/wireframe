//! The pure bookkeeping model behind an allocation-measurement window.
//!
//! [`WindowState`] holds the three values the parent module keeps in atomics:
//! what earlier windows accumulated, the allocator total at the last boundary,
//! and whether the counting gate is open. Expressing the transition rules as a
//! plain value makes them testable without touching process globals, so the
//! properties below check the arithmetic directly rather than inferring it from
//! a running allocator.
//!
//! The parent module's global helpers only load a snapshot, apply one
//! transition, and publish the result; this file is where the meaning of those
//! transitions is defined.

/// Bookkeeping for one allocation-measurement window.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct WindowState {
    /// Allocations folded in by windows that have already closed.
    pub(super) accumulated: usize,
    /// Allocator total at the last boundary: window open, reset, or close.
    pub(super) baseline: usize,
    /// Whether the counting gate is currently open.
    pub(super) is_open: bool,
}

impl WindowState {
    /// Open a fresh window at `total`, discarding earlier accumulation.
    ///
    /// [`count_allocations`](super::count_allocations) opens its window with
    /// this transition.
    pub(super) const fn opened_at(total: usize) -> Self {
        Self {
            accumulated: 0,
            baseline: total,
            is_open: true,
        }
    }

    /// Clear the accumulated count and re-baseline at `total`.
    ///
    /// The gate is left as it is: a reset inside an open window keeps counting
    /// from `total`, and a reset outside one prepares the next window.
    pub(super) const fn reset_at(self, total: usize) -> Self {
        Self {
            accumulated: 0,
            baseline: total,
            is_open: self.is_open,
        }
    }

    /// Allocations observed since `baseline`, up to `total`.
    ///
    /// Saturating rather than wrapping: a total below the baseline can only
    /// mean the allocator's counters were sampled out of order, and reporting
    /// zero is honest where a wrapped difference would invent allocations.
    pub(super) const fn delta(baseline: usize, total: usize) -> usize {
        total.saturating_sub(baseline)
    }

    /// Count visible at allocator total `total`.
    ///
    /// An open window adds its live delta to the accumulated count; a closed
    /// one reports only what it folded in.
    pub(super) const fn count_at(self, total: usize) -> usize {
        if self.is_open {
            self.accumulated
                .saturating_add(Self::delta(self.baseline, total))
        } else {
            self.accumulated
        }
    }

    /// Close the window at `window_total`, folding its contribution in.
    ///
    /// The baseline advances to the sampled total, so folding the same window
    /// again adds nothing. The production close path folds with an atomic
    /// `swap` plus `fetch_add` so concurrent closers cannot double-count; this
    /// transition states the same arithmetic as one value, which is what the
    /// properties below exercise.
    #[cfg(test)]
    pub(super) const fn closed_at(self, window_total: usize) -> Self {
        Self {
            accumulated: self.count_at(window_total),
            baseline: window_total,
            is_open: false,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Properties of the window arithmetic.
    //!
    //! These are the assertions the installed-allocator probe cannot make
    //! cheaply: it can show that a real window counts, but not that an open
    //! window reports exactly its own delta for arbitrary totals, that later
    //! allocations never inflate a closed window, or that out-of-order samples
    //! saturate instead of wrapping.

    use proptest::prelude::*;

    use super::WindowState;

    proptest! {
        /// An open window reports exactly the allocations since it opened.
        #[test]
        fn open_window_reports_its_own_delta(
            baseline in any::<usize>(),
            observed in any::<usize>(),
        ) {
            let window = WindowState::opened_at(baseline);
            prop_assert_eq!(
                window.count_at(observed),
                observed.saturating_sub(baseline),
                "an open window must report the delta since its baseline"
            );
        }

        /// A closed window keeps only what it folded in, whatever happens next.
        #[test]
        fn closed_window_ignores_later_allocations(
            opened in any::<usize>(),
            closed in any::<usize>(),
            later in any::<usize>(),
        ) {
            let window = WindowState::opened_at(opened).closed_at(closed);
            let expected = closed.saturating_sub(opened);
            prop_assert_eq!(
                window.count_at(later),
                expected,
                "a closed window must not absorb allocations after its close"
            );
        }

        /// Closing the same window twice adds its delta only once.
        #[test]
        fn closing_is_idempotent(
            opened in any::<usize>(),
            closed in any::<usize>(),
            next_total in any::<usize>(),
        ) {
            let once = WindowState::opened_at(opened).closed_at(closed);
            let twice = once.closed_at(next_total);
            prop_assert_eq!(
                twice.count_at(next_total),
                once.count_at(next_total),
                "folding an already-closed window must add nothing"
            );
        }

        /// Resetting discards what earlier windows folded in.
        #[test]
        fn reset_clears_accumulation_for_a_closed_window(
            opened in any::<usize>(),
            closed in any::<usize>(),
            reset_at in any::<usize>(),
            sampled in any::<usize>(),
        ) {
            let window = WindowState::opened_at(opened)
                .closed_at(closed)
                .reset_at(reset_at);
            prop_assert_eq!(
                window.count_at(sampled),
                0,
                "a reset must discard the accumulated count"
            );
        }

        /// Resetting inside an open window re-baselines but keeps counting.
        #[test]
        fn reset_rebaselines_an_open_window(
            opened in any::<usize>(),
            reset_at in any::<usize>(),
            observed in any::<usize>(),
        ) {
            let window = WindowState::opened_at(opened).reset_at(reset_at);
            prop_assert_eq!(
                window.count_at(observed),
                observed.saturating_sub(reset_at),
                "an open window must count from the reset total"
            );
        }

        /// Out-of-order samples saturate rather than wrapping.
        #[test]
        fn delta_saturates_below_the_baseline(baseline in any::<usize>(), lower in any::<usize>()) {
            prop_assume!(lower < baseline);
            prop_assert_eq!(
                WindowState::delta(baseline, lower),
                0,
                "a sample below the baseline must not wrap into a huge delta"
            );
        }
    }
}
