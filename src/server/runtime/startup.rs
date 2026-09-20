//! Startup preparation and bounded startup observability.

use std::{sync::Arc, time::Instant};

use futures::Future;
use tokio::select;
use tracing::error;

use super::{AppFactory, ServerError};
use crate::{
    app::{Envelope, Packet, PreparedApp},
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
    let app = factory.build().map_err(|error| {
        record_startup_failure(ServerStartupFailureStage::FactoryBuild, &error);
        record_server_startup_duration(
            ServerStartupOutcome::FactoryBuild,
            startup_started.elapsed(),
        );
        ServerError::FactoryBuild(Box::new(error))
    })?;
    let mut shutdown = Box::pin(shutdown);
    #[expect(
        clippy::integer_division_remainder_used,
        reason = "tokio::select! expands to modulus internally"
    )]
    let startup_result = select! {
        () = &mut shutdown => Ok(None),
        result = app.prepare() => Ok(Some((Arc::new(result.map_err(|error| {
            record_startup_failure(ServerStartupFailureStage::Preparation, &error);
            record_server_startup_duration(ServerStartupOutcome::Preparation, startup_started.elapsed());
            ServerError::Prepare(error)
        })?), shutdown))),
    };
    startup_result
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
