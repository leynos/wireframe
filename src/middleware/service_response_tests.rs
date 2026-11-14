//! Unit tests for the deferred response body on `ServiceResponse`.
//!
//! A response route stores its body out of band so middleware can inspect the
//! frame bytes without owning a live stream. These tests pin the resulting
//! contract: clones share one take-once slot, equality and `Debug` ignore the
//! body, and mutating the frame never rewrites the streamed frames.

use std::sync::{Arc, Mutex};

use super::service_response::{DeferredBody, ServiceResponse};
use crate::{app::Envelope, response::Response};

/// Build a deferred slot holding one multi-frame response.
fn deferred_body() -> DeferredBody {
    Arc::new(Mutex::new(Some(Response::Vec(vec![Envelope::new(
        1,
        None,
        vec![9],
    )]))))
}

/// Clones share the slot, so the first consumer takes the body exactly once.
#[test]
fn clones_share_the_deferred_body_and_take_it_once() {
    let response = ServiceResponse::with_deferred(Vec::new(), Some(3), deferred_body());
    let clone = response.clone();

    assert!(
        response.take_deferred().is_some(),
        "the first consumer takes the body"
    );
    assert!(
        clone.take_deferred().is_none(),
        "a clone sees the body already taken"
    );
    assert!(
        response.take_deferred().is_none(),
        "the body is taken exactly once"
    );
}

/// Equality compares the frame bytes and correlation id, not the body.
#[test]
fn equality_ignores_the_deferred_body() {
    let with_body = ServiceResponse::with_deferred(vec![1, 2], Some(3), deferred_body());
    let without_body = ServiceResponse::new(vec![1, 2], Some(3));

    assert_eq!(
        with_body, without_body,
        "the deferred body must be excluded from equality"
    );
    assert_ne!(
        with_body,
        ServiceResponse::new(vec![1, 2], Some(4)),
        "the correlation id must still be compared"
    );
}

/// `Debug` reports that a body is present without rendering its contents.
#[test]
fn debug_reports_body_presence_without_its_contents() {
    let with_body = ServiceResponse::with_deferred(Vec::new(), None, deferred_body());
    let without_body = ServiceResponse::new(Vec::new(), None);

    let rendered = format!("{with_body:?}");
    assert!(
        rendered.contains("has_deferred_body: true"),
        "a present body must be reported: {rendered}"
    );
    assert!(
        !rendered.contains("Envelope"),
        "the body's contents must not be rendered: {rendered}"
    );
    assert!(
        format!("{without_body:?}").contains("has_deferred_body: false"),
        "an absent body must be reported"
    );
}

/// Editing the frame bytes must not reach the streamed frames.
#[test]
fn frame_mutation_does_not_change_the_deferred_body() {
    let mut response = ServiceResponse::with_deferred(Vec::new(), None, deferred_body());
    response.frame_mut().push(b'!');

    let body = response.take_deferred().expect("the body is still present");
    match body {
        Response::Vec(frames) => {
            assert_eq!(frames.len(), 1, "the body keeps its own frames");
            let frame = frames.first().expect("the body holds one frame");
            assert_eq!(
                frame.payload,
                vec![9],
                "the streamed frame must be unaffected by a frame edit"
            );
        }
        _ => panic!("unexpected response variant"),
    }
}

/// A response built without a route body has nothing to take.
#[test]
fn a_fresh_response_has_no_deferred_body() {
    let response = ServiceResponse::new(vec![1], Some(2));

    assert!(
        response.take_deferred().is_none(),
        "only a response route installs a deferred body"
    );
}
