//! A zero fixture budget must fail in the same guard used by unit tests.

include!("../support/non_zero.rs");

const ZERO_BUDGET: std::num::NonZeroUsize = nz!(0);

fn main() {
    let _ = ZERO_BUDGET;
}
