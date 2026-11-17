//! Verifies a recovery hook survives the builder's type-changing methods.

use std::time::Duration;

use wireframe::{
    app::WireframeApp,
    codec::{
        CodecError,
        CodecErrorContext,
        LengthDelimitedFrameCodec,
        RecoveryConfig,
        RecoveryPolicy,
        RecoveryPolicyHook,
    },
    serializer::BincodeSerializer,
    server::WireframeServer,
};

/// Recovery hook that keeps the built-in policy for every error.
struct ExampleHook;

impl RecoveryPolicyHook for ExampleHook {
    fn recovery_policy(&self, error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
        error.default_recovery_policy()
    }

    fn quarantine_duration(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> Duration {
        Duration::from_millis(10)
    }
}

fn main() {
    // `serializer` returns a new application built from its argument alone, so
    // pinning the serializer here is the only way to leave the receiver
    // inferable while still exercising the type-changing transition.
    let app: WireframeApp = WireframeApp::<BincodeSerializer>::new()
        .expect("build application")
        .with_recovery_hook(ExampleHook)
        .recovery_config(RecoveryConfig::default().max_consecutive_drops(2))
        .with_codec(LengthDelimitedFrameCodec::new(2048))
        .serializer(BincodeSerializer)
        .on_connection_setup(|| async {})
        .expect("register setup hook");

    let _: WireframeServer<_> = WireframeServer::from_app(app);
}