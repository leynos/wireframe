//! The workspace's single audited unsafe boundary: a counting global allocator.
//!
//! `GlobalAlloc` is an unsafe trait whose four methods are themselves unsafe,
//! and its bodies call the unsafe `System` allocator entry points. None of that
//! can be rewritten safely, and `unsafe_code = "forbid"` admits no exemption, so
//! the implementation is confined to this crate. Every other Cargo target root
//! in the workspace carries `#![forbid(unsafe_code)]` and is checked to do so.
//!
//! This crate deliberately contains nothing but the type and its one `unsafe
//! impl`. The counter, the measurement gate, and the scoped measurement helper
//! are safe code and live in `wireframe-bench-counting`, which is itself under
//! the prohibition. Keeping the audited surface this small is the point: a
//! reviewer can audit the whole crate in one screen.
//!
//! A benchmark binary installs the allocator and then measures with
//! `wireframe_bench_counting::count_allocations`:
//!
//! ```rust,ignore
//! use wireframe_bench_alloc::CountingAllocator;
//!
//! #[global_allocator]
//! static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;
//! ```
//!
//! # Safety obligations
//!
//! The impl adds one atomic increment and otherwise forwards every argument
//! unchanged to the system allocator. It therefore introduces no new pointer
//! ownership, layout, alignment, or lifetime obligation: a caller must satisfy
//! exactly the obligations `GlobalAlloc` and `System` already impose. The
//! `SAFETY` comments below state that per method rather than once for the block,
//! because `unsafe_op_in_unsafe_fn` is denied here.

#![deny(unsafe_op_in_unsafe_fn)]

use std::alloc::{GlobalAlloc, Layout, System};

use wireframe_bench_counting::record_allocation_if_enabled;

/// Forwards allocations to the system allocator while counting the operations
/// observed during an enabled measurement window.
///
/// Install one as the process `#[global_allocator]`; the type is otherwise inert
/// and holds no state, because the counter is process-global.
pub struct CountingAllocator;

// `reason` must be a single string literal: unlike `doc`, the attribute value is
// not macro-expanded, so `concat!` is rejected here.
#[expect(
    unsafe_code,
    reason = "GlobalAlloc is an unsafe trait with unsafe methods that call unsafe System APIs; \
              this crate is the workspace's single audited boundary"
)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: `layout` is forwarded unchanged to the system allocator, so the
        // caller obligation is exactly the one the `GlobalAlloc::alloc` contract
        // already places on this method.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` are forwarded unchanged, so the caller
        // obligation is exactly the one the `GlobalAlloc::dealloc` contract
        // already places on this method.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: `layout` is forwarded unchanged, so the caller obligation is
        // exactly the one the `GlobalAlloc::alloc_zeroed` contract already places
        // on this method.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation_if_enabled();
        // SAFETY: `ptr`, `layout`, and `new_size` are forwarded unchanged, so the
        // caller obligation is exactly the one the `GlobalAlloc::realloc`
        // contract already places on this method.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[cfg(test)]
// A single literal, for the same reason as the expectation above.
#[expect(
    unsafe_code,
    reason = "the GlobalAlloc methods are unsafe by definition; these tests call them directly so \
              every delegation is exercised, not just the ones a particular allocation happens to \
              reach"
)]
mod method_tests {
    //! Direct coverage of the four `GlobalAlloc` methods.
    //!
    //! Installing the allocator as `#[global_allocator]` proves the counting
    //! window works, but not that each method delegates correctly: which methods
    //! a test allocation reaches depends on the allocator's own behaviour. These
    //! tests therefore call the trait methods directly, so `alloc`,
    //! `alloc_zeroed`, `realloc`, and `dealloc` are each exercised on every run.
    //!
    //! Nothing here installs the allocator, so the counting gate stays closed and
    //! these tests assert only on the delegated memory, not on counts. Counting
    //! through a real installed allocator is covered by
    //! `tests/global_allocator.rs`.

    use std::alloc::{GlobalAlloc, Layout};

    use super::CountingAllocator;

    /// Bytes requested by the probe allocations.
    ///
    /// Any small non-zero size works; a value with no alignment requirement keeps
    /// `realloc` free to move the block, which is the case worth testing.
    const PROBE_BYTES: usize = 64;

    /// Layout used by every probe: `PROBE_BYTES` bytes aligned to `u64`.
    fn probe_layout() -> Layout {
        Layout::from_size_align(PROBE_BYTES, 8).expect("the probe layout is valid by construction")
    }

    /// Read `len` bytes of the probe allocation as a slice.
    ///
    /// The lifetime is the caller's to choose, mirroring
    /// [`std::slice::from_raw_parts`]: only the caller knows how long the
    /// allocation outlives the returned slice.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a live allocation of at least `len` bytes that is not
    /// mutated for the returned slice's lifetime.
    unsafe fn probe_bytes<'a>(ptr: *mut u8, len: usize) -> &'a [u8] {
        // SAFETY: the caller guarantees a live `len`-byte allocation; the
        // returned slice borrows it without permitting mutation.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }

    #[test]
    fn alloc_and_dealloc_round_trip_through_the_system_allocator() {
        let allocator = CountingAllocator;
        let layout = probe_layout();

        // SAFETY: `layout` is a valid layout and the returned block is freed with
        // the same layout before the test ends.
        let ptr = unsafe { allocator.alloc(layout) };
        assert!(!ptr.is_null(), "the system allocator must supply the block");

        // SAFETY: `ptr` came from `alloc` with exactly this layout and is not
        // used afterwards.
        unsafe { allocator.dealloc(ptr, layout) };
    }

    #[test]
    fn alloc_zeroed_returns_a_zeroed_block() {
        let allocator = CountingAllocator;
        let layout = probe_layout();

        // SAFETY: `layout` is a valid layout and the block is freed with the same
        // layout before the test ends.
        let ptr = unsafe { allocator.alloc_zeroed(layout) };
        assert!(!ptr.is_null(), "the system allocator must supply the block");

        // SAFETY: `ptr` is live for `PROBE_BYTES` and is only read here.
        let bytes = unsafe { probe_bytes(ptr, PROBE_BYTES) };
        assert!(
            bytes.iter().all(|byte| *byte == 0),
            "alloc_zeroed must return zeroed memory"
        );

        // SAFETY: `ptr` came from `alloc_zeroed` with exactly this layout and is
        // not used afterwards.
        unsafe { allocator.dealloc(ptr, layout) };
    }

    #[test]
    fn realloc_preserves_the_prefix_and_frees_with_the_new_layout() {
        let allocator = CountingAllocator;
        let layout = probe_layout();
        let new_size = PROBE_BYTES * 2;

        // SAFETY: `layout` is a valid layout and the block is reallocated rather
        // than freed directly, so exactly one live allocation exists at the end.
        let ptr = unsafe { allocator.alloc(layout) };
        assert!(!ptr.is_null(), "the system allocator must supply the block");

        // SAFETY: `ptr` is live for `PROBE_BYTES` and nothing else borrows it.
        unsafe { std::ptr::write_bytes(ptr, 0xa5, PROBE_BYTES) };

        // SAFETY: `ptr` came from `alloc` with `layout` and has not been freed;
        // `new_size` is non-zero and larger than the current size.
        let grown = unsafe { allocator.realloc(ptr, layout, new_size) };
        assert!(!grown.is_null(), "growing the block must succeed");

        // SAFETY: `grown` is live for `new_size` bytes, and the original
        // `PROBE_BYTES` prefix is still readable.
        let prefix = unsafe { probe_bytes(grown, PROBE_BYTES) };
        assert!(
            prefix.iter().all(|byte| *byte == 0xa5),
            "realloc must preserve the existing contents"
        );

        let grown_layout = Layout::from_size_align(new_size, 8)
            .expect("the grown layout is valid by construction");
        // SAFETY: `grown` came from `realloc` with `grown_layout` and is not used
        // afterwards.
        unsafe { allocator.dealloc(grown, grown_layout) };
    }
}
