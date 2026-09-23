//! Loom models of the active-connection gauge.
//!
//! Every connection actor holds an `ActiveConnection` guard for its
//! lifetime: creating one increments a process-wide gauge and dropping it
//! decrements the gauge. Actors start and stop on different threads, so the
//! increments and decrements interleave. Under `cfg(loom)` the gauge is a
//! Loom atomic, and these models hold the guard through
//! `LoomConnectionGuard`, the same type the actor uses.
#![cfg(loom)]

use loom::{model, thread};
use wireframe::connection::{LoomConnectionGuard, active_connection_count};

#[test]
fn concurrent_guards_return_the_gauge_to_zero() {
    // Two actors start and stop concurrently. However their increments and
    // decrements interleave, the gauge must read zero once both are gone.
    model(|| {
        let actors: Vec<_> = (0..2)
            .map(|_| thread::spawn(|| drop(LoomConnectionGuard::new())))
            .collect();
        for actor in actors {
            actor.join().expect("actor thread panicked");
        }
        assert_eq!(
            active_connection_count(),
            0,
            "every guard dropped must leave the gauge at zero"
        );
    });
}

#[test]
fn live_guards_are_all_counted() {
    // Two actors start concurrently and are still alive when the gauge is
    // read: it must count both, whatever order they started in.
    model(|| {
        let actors: Vec<_> = (0..2)
            .map(|_| thread::spawn(LoomConnectionGuard::new))
            .collect();
        let guards: Vec<_> = actors
            .into_iter()
            .map(|actor| actor.join().expect("actor thread panicked"))
            .collect();
        assert_eq!(
            active_connection_count(),
            2,
            "two live guards must count two"
        );
        drop(guards);
        assert_eq!(
            active_connection_count(),
            0,
            "dropping both guards must return the gauge to zero"
        );
    });
}
