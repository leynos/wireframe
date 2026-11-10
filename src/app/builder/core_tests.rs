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
    let app = app_builder?;
    if app.fragmentation.is_some() {
        return Err(io::Error::other("fragmentation should be disabled by default").into());
    }
    Ok(())
}

#[rstest]
fn builder_defaults_memory_budgets_to_disabled(
    app_builder: Result<TestApp, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let app = app_builder?;
    if app.memory_budgets.is_some() {
        return Err(io::Error::other("memory budgets should be disabled by default").into());
    }
    Ok(())
}

#[rstest]
fn enable_fragmentation_requires_explicit_opt_in(
    app_builder: Result<TestApp, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let app = app_builder?.enable_fragmentation();
    if app.fragmentation.is_none() {
        return Err(io::Error::other("enable_fragmentation should configure fragmentation").into());
    }
    Ok(())
}

#[rstest]
fn with_codec_clears_fragmentation_to_require_reconfiguration(
    app_builder: Result<TestApp, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let app = app_builder?
        .enable_fragmentation()
        .with_codec(LengthDelimitedFrameCodec::new(2048));
    if app.fragmentation.is_some() {
        return Err(io::Error::other("with_codec should clear fragmentation configuration").into());
    }
    Ok(())
}

#[rstest]
fn buffer_capacity_clears_fragmentation_to_require_reconfiguration(
    app_builder: Result<TestApp, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let app = app_builder?.enable_fragmentation().buffer_capacity(2048);
    if app.fragmentation.is_some() {
        return Err(
            io::Error::other("buffer_capacity should clear fragmentation configuration").into(),
        );
    }
    Ok(())
}

#[rstest]
fn memory_budgets_builder_stores_configuration(
    app_builder: Result<TestApp, Box<dyn Error>>,
    memory_budgets: Result<MemoryBudgets, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let configured_budgets = memory_budgets?;
    let app = app_builder?.memory_budgets(configured_budgets);
    if app.memory_budgets != Some(configured_budgets) {
        return Err(io::Error::other("memory_budgets should store the configured value").into());
    }
    Ok(())
}

#[rstest]
fn with_codec_preserves_memory_budgets(
    app_builder: Result<TestApp, Box<dyn Error>>,
    memory_budgets: Result<MemoryBudgets, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let configured_budgets = memory_budgets?;
    let app = app_builder?
        .memory_budgets(configured_budgets)
        .with_codec(LengthDelimitedFrameCodec::new(4096));
    if app.memory_budgets != Some(configured_budgets) {
        return Err(
            io::Error::other("with_codec should preserve configured memory budgets").into(),
        );
    }
    Ok(())
}

#[rstest]
fn serializer_preserves_memory_budgets(
    app_builder: Result<TestApp, Box<dyn Error>>,
    memory_budgets: Result<MemoryBudgets, Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    let configured_budgets = memory_budgets?;
    let app = app_builder?
        .memory_budgets(configured_budgets)
        .serializer(BincodeSerializer);
    if app.memory_budgets != Some(configured_budgets) {
        return Err(
            io::Error::other("serializer should preserve configured memory budgets").into(),
        );
    }
    Ok(())
}
