//! The checks the invariants binary runs, one function per claim.
//!
//! Each function either returns or panics with the measurement that broke it,
//! so a failure names the byte count or address rather than only the claim.
//! The functions are grouped by the instrument they exercise: the allocation
//! window itself (`windows`), the production hand-off (`handoff`), and the
//! pipeline stages (`stages`).

mod handoff;
mod stages;
mod windows;

use handoff::{
    exact_capacity_handoff_allocates_nothing,
    production_outbound_allocates_one_payload,
    spare_capacity_handoff_allocates_a_control_block,
};
use stages::{
    copying_hook_relocates_buffer,
    copying_hooks_exceed_payload,
    copying_middleware_counts_match,
    copying_middleware_pass_exceeds_payload,
    copying_middleware_relocates_buffer,
    hooks_preserve_buffer,
    middleware_preserves_buffer,
    mutating_hook_changes_buffer,
    read_only_hook_counts_match,
    read_only_hooks_hold_across_payload_sizes,
    read_only_hooks_stay_below_payload,
    read_only_middleware_batch_stays_below_payload,
    read_only_middleware_counts_match,
    read_only_middleware_pass_stays_below_payload,
};
use windows::{
    growing_reallocation_reports_growth,
    harness_observes_real_traffic,
    idle_windows_report_no_traffic,
    windows_report_their_bytes,
};

/// Every check, with the name the harness prints before running it.
///
/// The order matters: the harness self-check and the noise-floor probe come
/// first, because every later bound is meaningless if the instrument is not
/// observing the allocator.
pub(super) const CHECKS: [(&str, fn()); 21] = [
    (
        "the allocation harness observes real traffic",
        harness_observes_real_traffic,
    ),
    (
        "a window reports the bytes it allocated",
        windows_report_their_bytes,
    ),
    (
        "a growing reallocation reports the growth",
        growing_reallocation_reports_growth,
    ),
    (
        "idle windows report no traffic",
        idle_windows_report_no_traffic,
    ),
    (
        "the production outbound operation allocates one payload",
        production_outbound_allocates_one_payload,
    ),
    (
        "an exact-capacity hand-off allocates nothing",
        exact_capacity_handoff_allocates_nothing,
    ),
    (
        "a spare-capacity hand-off allocates only a control block",
        spare_capacity_handoff_allocates_a_control_block,
    ),
    (
        "read-only middleware preserves the payload buffer",
        middleware_preserves_buffer,
    ),
    (
        "read-only hooks preserve the payload buffer",
        hooks_preserve_buffer,
    ),
    (
        "a mutating hook changes the buffer",
        mutating_hook_changes_buffer,
    ),
    (
        "a copying hook relocates the buffer",
        copying_hook_relocates_buffer,
    ),
    (
        "a copying middleware relocates the buffer",
        copying_middleware_relocates_buffer,
    ),
    (
        "read-only middleware batches allocate the same across payload classes",
        read_only_middleware_counts_match,
    ),
    (
        "a read-only middleware batch allocates less than the payload",
        read_only_middleware_batch_stays_below_payload,
    ),
    (
        "one read-only middleware pass allocates less than the payload",
        read_only_middleware_pass_stays_below_payload,
    ),
    (
        "read-only hook batches allocate the same across payload classes",
        read_only_hook_counts_match,
    ),
    (
        "a read-only hook batch allocates less than the payload",
        read_only_hooks_stay_below_payload,
    ),
    (
        "read-only hooks hold across payload sizes",
        read_only_hooks_hold_across_payload_sizes,
    ),
    (
        "the copying middleware keeps the count relation",
        copying_middleware_counts_match,
    ),
    (
        "one copying middleware pass exceeds the payload bound",
        copying_middleware_pass_exceeds_payload,
    ),
    (
        "a copying hook batch exceeds the payload bound",
        copying_hooks_exceed_payload,
    ),
];
