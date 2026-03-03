//! Step definitions for request context behavioural tests.
//!
//! All step phrases use a `request-context` prefix to avoid collisions
//! with other step definitions.

use rstest_bdd_macros::{given, then, when};
use wireframe::{
    context::{CorrelationId, RequestContext},
    tenant::{TenantId, UserId},
};

use crate::fixtures::request_context::{RequestContextWorld, TestResult};

#[given("a request-context with tenant id {tid:u64}")]
fn given_tenant_context(request_context_world: &mut RequestContextWorld, tid: u64) {
    let ctx = RequestContext::new().with_tenant_id(TenantId::new(tid));
    request_context_world.register_context(ctx);
}

#[given(
    "a request-context with tenant id {tid:u64} and user id {uid:u64} and correlation id {cid:u64}"
)]
fn given_full_context(
    request_context_world: &mut RequestContextWorld,
    tid: u64,
    uid: u64,
    cid: u64,
) {
    let ctx = RequestContext::new()
        .with_tenant_id(TenantId::new(tid))
        .with_user_id(UserId::new(uid))
        .with_correlation_id(CorrelationId::new(cid));
    request_context_world.register_context(ctx);
}

#[given("a request-context without a tenant id")]
fn given_context_without_tenant(request_context_world: &mut RequestContextWorld) {
    let ctx = RequestContext::new().with_user_id(UserId::new(1));
    request_context_world.register_context(ctx);
}

#[given("no request-context is registered")]
fn given_no_context(request_context_world: &mut RequestContextWorld) {
    // Intentionally do not register any context.
    let _ = request_context_world;
}

#[when("the request-context is extracted from the message request")]
fn when_extract(request_context_world: &mut RequestContextWorld) -> TestResult {
    request_context_world.extract()
}

#[when("require-tenant-id is called on the request-context")]
fn when_require_tenant_id(request_context_world: &mut RequestContextWorld) -> TestResult {
    request_context_world.extract()?;
    request_context_world.call_require_tenant_id();
    Ok(())
}

#[then("the extracted tenant id is {tid:u64}")]
fn then_tenant_id(request_context_world: &mut RequestContextWorld, tid: u64) -> TestResult {
    let actual = request_context_world.extracted_tenant_id();
    if actual != Some(TenantId::new(tid)) {
        return Err(format!("expected tenant id {tid}, got {actual:?}").into());
    }
    Ok(())
}

#[then("the extracted user id is {uid:u64}")]
fn then_user_id(request_context_world: &mut RequestContextWorld, uid: u64) -> TestResult {
    let actual = request_context_world.extracted_user_id();
    if actual != Some(UserId::new(uid)) {
        return Err(format!("expected user id {uid}, got {actual:?}").into());
    }
    Ok(())
}

#[then("the extracted correlation id is {cid:u64}")]
fn then_correlation_id(request_context_world: &mut RequestContextWorld, cid: u64) -> TestResult {
    let actual = request_context_world.extracted_correlation_id();
    if actual != Some(CorrelationId::new(cid)) {
        return Err(format!("expected correlation id {cid}, got {actual:?}").into());
    }
    Ok(())
}

#[then("a missing tenant error is returned")]
fn then_missing_tenant_error(request_context_world: &mut RequestContextWorld) -> TestResult {
    if !request_context_world.has_tenant_error() {
        return Err("expected a missing tenant error".into());
    }
    Ok(())
}

#[then("all extracted identifiers are absent")]
fn then_all_absent(request_context_world: &mut RequestContextWorld) -> TestResult {
    if !request_context_world.all_identifiers_absent() {
        return Err("expected all identifiers to be absent".into());
    }
    Ok(())
}
