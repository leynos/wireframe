//! Startup preparation and bounded startup observability.

use std::{sync::Arc, time::Instant};

use futures::Future;
use tokio::select;
use tracing::error;

use super::{AppFactory, ServerError};
use crate::{
    app::{Envelope, Packet, PrepareError, PreparedApp},
    codec::FrameCodec,
    frame::FrameMetadata,
    message::{DecodeWith, EncodeWith},
    metrics::{
        ServerStartupFailureStage,
        ServerStartupOutcome,
        inc_server_startup_failure,
        record_server_startup_duration,
    },
    serializer::Serializer,
};

/// Build and prepare an application unless shutdown wins the startup race.
pub(super) async fn prepare_application_or_shutdown<F, S, Ser, Ctx, E, Codec>(
    factory: F,
    shutdown: S,
    startup_started: Instant,
) -> Result<Option<(Arc<PreparedApp<Ser, Ctx, E, Codec>>, std::pin::Pin<Box<S>>)>, ServerError>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    S: Future<Output = ()> + Send,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    let app = build_application(factory, startup_started)?;
    let prepared = prepare_or_shutdown(app.prepare(), shutdown, startup_started).await?;
    Ok(prepared.map(|(app, shutdown)| (Arc::new(app), shutdown)))
}

/// Await preparation unless shutdown completes first, recording its outcome.
pub(super) async fn prepare_or_shutdown<T, P, S>(
    preparation: P,
    shutdown: S,
    startup_started: Instant,
) -> Result<Option<(T, std::pin::Pin<Box<S>>)>, ServerError>
where
    P: Future<Output = Result<T, PrepareError>>,
    S: Future<Output = ()> + Send,
{
    let mut shutdown = Box::pin(shutdown);
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! expands to modulus internally"
    )]
    let startup_result = select! {
        () = &mut shutdown => {
            record_server_startup_duration(
                ServerStartupOutcome::Cancelled,
                startup_started.elapsed(),
            );
            Ok(None)
        },
        result = preparation => Ok(Some((record_preparation_result(result, startup_started)?, shutdown))),
    };
    startup_result
}

/// Build and prepare an application before starting a controlled server.
pub(super) async fn prepare_application<F, Ser, Ctx, E, Codec>(
    factory: F,
    startup_started: Instant,
) -> Result<Arc<PreparedApp<Ser, Ctx, E, Codec>>, ServerError>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    prepare_built_application(
        build_application(factory, startup_started)?,
        startup_started,
    )
    .await
}

/// Build an application and record a bounded failure outcome when needed.
fn build_application<F, Ser, Ctx, E, Codec>(
    factory: F,
    startup_started: Instant,
) -> Result<crate::app::WireframeApp<Ser, Ctx, E, Codec>, ServerError>
where
    F: AppFactory<Ser, Ctx, E, Codec>,
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
{
    factory.build().map_err(|error| {
        record_startup_failure(ServerStartupFailureStage::FactoryBuild, &error);
        record_server_startup_duration(
            ServerStartupOutcome::FactoryBuild,
            startup_started.elapsed(),
        );
        ServerError::FactoryBuild(Box::new(error))
    })
}

/// Prepare a built application and record a bounded failure outcome when needed.
async fn prepare_built_application<Ser, Ctx, E, Codec>(
    app: crate::app::WireframeApp<Ser, Ctx, E, Codec>,
    startup_started: Instant,
) -> Result<Arc<PreparedApp<Ser, Ctx, E, Codec>>, ServerError>
where
    Ser: Serializer + FrameMetadata<Frame = Envelope> + Send + Sync + 'static,
    Ctx: Send + 'static,
    E: Packet,
    Codec: FrameCodec,
    Envelope: DecodeWith<Ser> + EncodeWith<Ser>,
{
    record_preparation_result(app.prepare().await, startup_started).map(Arc::new)
}

/// Record the common typed error and duration for a preparation result.
fn record_preparation_result<T>(
    result: Result<T, PrepareError>,
    startup_started: Instant,
) -> Result<T, ServerError> {
    result.map_err(|error| {
        record_startup_failure(ServerStartupFailureStage::Preparation, &error);
        record_server_startup_duration(
            ServerStartupOutcome::Preparation,
            startup_started.elapsed(),
        );
        ServerError::Prepare(error)
    })
}

/// Emit bounded observability for a startup failure without accepting traffic.
fn record_startup_failure<E>(error_stage: ServerStartupFailureStage, _error: &E)
where
    E: std::error::Error,
{
    inc_server_startup_failure(error_stage);
    error!(
        event = "server_startup_failure",
        stage = error_stage.as_str(),
        error_type = std::any::type_name::<E>(),
        "server start-up failed before readiness"
    );
}
