//! Allocator-level invariants for the zero-copy baseline.
//!
//! The copied-byte counters report what the seams count; this binary reports
//! what the allocator saw. The two instruments fail differently, which is why
//! both exist: a recorder can fire without a materialization, a materialization
//! can happen at an uncounted site, and a stage that starts copying the payload
//! need not change any timing.
//!
//! # Why there is no libtest harness
//!
//! The window's statistics are process-global, and libtest spawns a thread per
//! test; the allocation that spawn makes lands inside whichever window happens
//! to be open. Stage A's noise-floor probe found exactly that: two of a
//! thousand idle windows reported a single small allocation under the default
//! harness, which is enough to break the count relation between the two payload
//! classes even though it is far too small to break a byte bound. The plan's
//! rule for that result is a `harness = false` target, so this binary runs its
//! checks from `main` on one thread and nothing else allocates while a window
//! is open. The probe is kept as one of the checks rather than thrown away, so
//! a future harness change cannot silently reintroduce the interference.
//!
//! Every bound is a relation with a wide margin against the 64 KiB payload
//! class, not an absolute count, so dependency churn does not re-tune it.

#![forbid(unsafe_code)]

mod checks;
mod support;

use std::io::{self, Write};

use wireframe_testing::benchmark_alloc_support::{INSTRUMENTED_SYSTEM, StatsAlloc, System};

/// Process allocator for this binary, so the counting window sees the same
/// allocator the code under test uses. `stats_alloc` supplies the instrumented
/// wrapper, so this crate needs no `unsafe` code of its own.
#[global_allocator]
static GLOBAL_ALLOCATOR: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// Runs every check in the order [`checks::CHECKS`] states.
///
/// Progress goes to stdout through `Write` rather than `println!`, because the
/// workspace denies `clippy::print_stdout` and this binary has no libtest
/// harness to report through instead. A failing check panics, so the process
/// exits non-zero and CI sees the failure without a summary line.
///
/// # Errors
///
/// Returns the write failure when stdout is closed, which is the only error the
/// harness itself can produce.
fn main() -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    for (name, check) in checks::CHECKS {
        writeln!(stdout, "{name} ...")?;
        check();
        writeln!(stdout, "    ok")?;
    }
    Ok(())
}
