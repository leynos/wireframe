//! Scenario tests for request context feature.

use rstest_bdd_macros::scenario;

use crate::fixtures::request_context::*;

#[scenario(
    path = "tests/features/request_context.feature",
    name = "Request context carries tenant identifier"
)]
fn tenant_id_only(request_context_world: RequestContextWorld) { let _ = request_context_world; }

#[scenario(
    path = "tests/features/request_context.feature",
    name = "Request context carries all cross-cutting identifiers"
)]
fn all_identifiers(request_context_world: RequestContextWorld) { let _ = request_context_world; }

#[scenario(
    path = "tests/features/request_context.feature",
    name = "Missing tenant identifier fails require check"
)]
fn missing_tenant(request_context_world: RequestContextWorld) { let _ = request_context_world; }

#[scenario(
    path = "tests/features/request_context.feature",
    name = "Default context has no identifiers"
)]
fn default_context(request_context_world: RequestContextWorld) { let _ = request_context_world; }
