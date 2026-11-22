//! Coverage for core builder parameter propagation and defaults.

use std::{error::Error, io, num::NonZeroUsize};

use rstest::{fixture, rstest};

use super::WireframeApp;
use crate::{
    app::{BudgetBytes, Envelope, MemoryBudgets},
    codec::LengthDelimitedFrameCodec,
    serializer::BincodeSerializer,
};

type TestApp = WireframeApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>;

/// Fail the test unless `condition` holds, naming what was expected.
///
/// These cases return `Result` so fixture errors propagate with `?`; this helper
/// keeps that style without repeating the error conversion in every case.
fn check(condition: bool, message: &str) -> Result<(), Box<dyn Error>> {
    if condition {
        return Ok(());
    }
    Err(io::Error::other(message).into())
}

#[fixture]
fn app_builder() -> Result<TestApp, Box<dyn Error>> {
    let app = TestApp::new()
        .map_err(|error| io::Error::other(format!("failed to construct test app: {error}")))?;
    if app.fragmentation.is_some() {
        return Err(io::Error::other("fixture expects default fragmentation disabled").into());
    }
    if app.memory_budgets.is_some() {
        return Err(io::Error::other("fixture expects default memory budgets disabled").into());
    }
    if app.max_concurrent_assemblies.is_some() {
        return Err(
            io::Error::other("fixture expects the derived assembly cap, not an override").into(),
        );
    }
    Ok(app)
}

#[fixture]
fn memory_budgets() -> Result<MemoryBudgets, Box<dyn Error>> {
    let per_message = NonZeroUsize::new(1024).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "message budget must be non-zero",
        )
    })?;
    let per_connection = NonZeroUsize::new(4096).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "connection budget must be non-zero",
        )
    })?;
    let in_flight = NonZeroUsize::new(2048).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "in-flight budget must be non-zero",
        )
    })?;
    Ok(MemoryBudgets::new(
        BudgetBytes::new(per_message),
        BudgetBytes::new(per_connection),
        BudgetBytes::new(in_flight),
    ))
}

#[rstest]
fn builder_defaults_fragmentation_to_disabled(
    app_builder: Result<TestApp, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    check(
        app_builder?.fragmentation.is_none(),
        "fragmentation should be disabled by default",
    )
}

#[rstest]
fn builder_defaults_memory_budgets_to_disabled(
    app_builder: Result<TestApp, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    check(
        app_builder?.memory_budgets.is_none(),
        "memory budgets should be disabled by default",
    )
}

#[rstest]
fn fragmentation_survives_only_until_the_builder_is_reconfigured(
    app_builder: Result<TestApp, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let fragmented = app_builder?.enable_fragmentation();
    check(
        fragmented.fragmentation.is_some(),
        "enable_fragmentation should configure fragmentation",
    )?;

    let recoded = fragmented.with_codec(LengthDelimitedFrameCodec::new(2048));
    check(
        recoded.fragmentation.is_none(),
        "with_codec should clear fragmentation configuration",
    )?;

    let resized = recoded.enable_fragmentation().buffer_capacity(2048);
    check(
        resized.fragmentation.is_none(),
        "buffer_capacity should clear fragmentation configuration",
    )
}

#[rstest]
fn memory_budgets_survive_every_reconfiguration(
    app_builder: Result<TestApp, Box<dyn Error>>,
    memory_budgets: Result<MemoryBudgets, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let configured = memory_budgets?;
    let stored = app_builder?.memory_budgets(configured);
    check(
        stored.memory_budgets == Some(configured),
        "memory_budgets should store the configured value",
    )?;

    let recoded = stored.with_codec(LengthDelimitedFrameCodec::new(4096));
    check(
        recoded.memory_budgets == Some(configured),
        "with_codec should preserve configured memory budgets",
    )?;

    let reserialized = recoded.serializer(BincodeSerializer);
    check(
        reserialized.memory_budgets == Some(configured),
        "serializer should preserve configured memory budgets",
    )
}

#[rstest]
fn max_concurrent_assemblies_survives_every_reconfiguration(
    app_builder: Result<TestApp, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let limit = NonZeroUsize::new(32).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "assembly cap must be non-zero")
    })?;
    let stored = app_builder?.max_concurrent_assemblies(limit);
    check(
        stored.max_concurrent_assemblies == Some(limit),
        "max_concurrent_assemblies should store the configured cap",
    )?;

    let recoded = stored.with_codec(LengthDelimitedFrameCodec::new(4096));
    check(
        recoded.max_concurrent_assemblies == Some(limit),
        "with_codec should preserve the configured cap",
    )?;

    let reserialized = recoded.serializer(BincodeSerializer);
    check(
        reserialized.max_concurrent_assemblies == Some(limit),
        "serializer should preserve the configured cap",
    )?;

    let resized = reserialized.buffer_capacity(2048);
    check(
        resized.max_concurrent_assemblies == Some(limit),
        "buffer_capacity should preserve the configured cap",
    )
}
