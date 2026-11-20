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

/// Runs every check in a fixed order.
///
/// The order matters: the harness self-check and the noise-floor probe come
/// first, because every later bound is meaningless if the instrument is not
/// observing the allocator.
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
    let checks: [(&'static str, fn()); 15] = [
        (
            "the allocation harness observes real traffic",
            checks::harness_observes_real_traffic,
        ),
        (
            "a window reports the bytes it allocated",
            checks::windows_report_their_bytes,
        ),
        (
            "a growing reallocation reports the growth",
            checks::growing_reallocation_reports_growth,
        ),
        (
            "idle windows report no traffic",
            checks::idle_windows_report_no_traffic,
        ),
        (
            "read-only middleware preserves the payload buffer",
            checks::middleware_preserves_buffer,
        ),
        (
            "read-only hooks preserve the payload buffer",
            checks::hooks_preserve_buffer,
        ),
        (
            "a mutating hook changes the buffer",
            checks::mutating_hook_changes_buffer,
        ),
        (
            "a copying hook relocates the buffer",
            checks::copying_hook_relocates_buffer,
        ),
        (
            "a copying middleware relocates the buffer",
            checks::copying_middleware_relocates_buffer,
        ),
        (
            "read-only middleware batches allocate the same across payload classes",
            checks::read_only_middleware_counts_match,
        ),
        (
            "one read-only middleware pass allocates less than the payload",
            checks::read_only_middleware_pass_stays_below_payload,
        ),
        (
            "a read-only hook batch allocates less than the payload",
            checks::read_only_hooks_stay_below_payload,
        ),
        (
            "the copying middleware keeps the count relation",
            checks::copying_middleware_counts_match,
        ),
        (
            "one copying middleware pass exceeds the payload bound",
            checks::copying_middleware_pass_exceeds_payload,
        ),
        (
            "a copying hook batch exceeds the payload bound",
            checks::copying_hooks_exceed_payload,
        ),
    ];
    for (name, check) in checks {
        writeln!(stdout, "{name} ...")?;
        check();
        writeln!(stdout, "    ok")?;
    }
    Ok(())
}
