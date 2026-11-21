//! Recovery hook and configuration for `WireframeApp`.

use super::WireframeApp;
use crate::{
    app::Packet,
    codec::{FrameCodec, RecoveryConfig, RecoveryPolicyHook},
    serializer::Serializer,
};

impl<S, C, E, F> WireframeApp<S, C, E, F>
where
    S: Serializer + Send + Sync,
    C: Send + 'static,
    E: Packet,
    F: FrameCodec,
{
    /// Install a [`RecoveryPolicyHook`] consulted when a decoder fails.
    ///
    /// Without a hook, a decoder failure on a live connection is terminal: the
    /// connection stops reading and the original error reaches the teardown
    /// path unchanged. Installing a hook lets the application choose between
    /// [`Drop`](crate::codec::RecoveryPolicy::Drop),
    /// [`Quarantine`](crate::codec::RecoveryPolicy::Quarantine), and
    /// [`Disconnect`](crate::codec::RecoveryPolicy::Disconnect) per error.
    ///
    /// The hook is shared by every connection built from this application and
    /// is wrapped in an [`Arc`](std::sync::Arc), so all its methods take
    /// `&self`. Per-connection state belongs in a connection setup hook, not in
    /// the recovery hook.
    ///
    /// # Callback order
    ///
    /// For each decoder failure the connection loop performs these steps in
    /// order:
    ///
    /// 1. Call [`recovery_policy`](RecoveryPolicyHook::recovery_policy) to select a policy.
    /// 2. Apply the fallbacks described below, which may replace the selection.
    /// 3. Call [`quarantine_duration`](RecoveryPolicyHook::quarantine_duration) when the applied
    ///    policy is `Quarantine`, and pause for that duration.
    /// 4. Call [`on_error`](RecoveryPolicyHook::on_error) with the applied policy, immediately
    ///    before the loop acts on it.
    /// 5. Increment `wireframe_codec_errors_total` once, labelled with the error type and the
    ///    applied policy.
    ///
    /// # Fallbacks
    ///
    /// The connection loop overrides the hook's selection in three cases,
    /// because continuing would either repeat forever or cannot help:
    ///
    /// - A failure from `decode_eof` always disconnects: the peer has stopped sending, so no later
    ///   frame can arrive on this connection.
    /// - `Drop` or `Quarantine` selected for a failure that consumed no bytes disconnects: the same
    ///   malformed prefix would be decoded again.
    /// - A `Drop` beyond
    ///   [`RecoveryConfig::max_consecutive_drops`](crate::codec::RecoveryConfig::max_consecutive_drops)
    ///   disconnects. The counter restarts whenever a frame decodes successfully.
    ///
    /// `on_error` and the metric report the applied policy, so they never claim
    /// a drop or quarantine that was escalated to a disconnect.
    ///
    /// # Decoder contract
    ///
    /// Recovery only works if the decoder consumes the malformed frame and
    /// leaves its framing state valid before returning the error. A decoder
    /// that returns an error without consuming any bytes would be offered the
    /// same input again, so the connection loop escalates that case to a
    /// disconnect.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::{
    ///     app::WireframeApp,
    ///     codec::{CodecError, CodecErrorContext, RecoveryPolicy, RecoveryPolicyHook},
    /// };
    ///
    /// struct DisconnectOnEveryError;
    ///
    /// impl RecoveryPolicyHook for DisconnectOnEveryError {
    ///     fn recovery_policy(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
    ///         RecoveryPolicy::Disconnect
    ///     }
    /// }
    ///
    /// let app: WireframeApp = WireframeApp::new()
    ///     .expect("builder")
    ///     .with_recovery_hook(DisconnectOnEveryError);
    /// let _ = app;
    /// ```
    #[must_use]
    pub fn with_recovery_hook<H>(self, hook: H) -> Self
    where
        H: RecoveryPolicyHook + 'static,
    {
        Self {
            recovery_hook: Some(std::sync::Arc::new(hook)),
            ..self
        }
    }

    /// Replace the global recovery settings.
    ///
    /// These settings are independent of the installed hook: the drop limit
    /// bounds how many consecutive frames a hook's `Drop` may discard, and
    /// [`RecoveryConfig::log_dropped_frames`](crate::codec::RecoveryConfig::log_dropped_frames)
    /// controls the log line emitted for an applied drop.
    ///
    /// [`RecoveryConfig::quarantine_duration`](crate::codec::RecoveryConfig::quarantine_duration)
    /// is only a fallback. When a hook is installed, its
    /// [`quarantine_duration`](RecoveryPolicyHook::quarantine_duration) method
    /// decides how long a quarantine lasts, because the hook may vary the pause
    /// per error.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use wireframe::{app::WireframeApp, codec::RecoveryConfig};
    ///
    /// let app: WireframeApp = WireframeApp::new().expect("builder").recovery_config(
    ///     RecoveryConfig::default()
    ///         .max_consecutive_drops(3)
    ///         .quarantine_duration(Duration::from_millis(250)),
    /// );
    /// let _ = app;
    /// ```
    #[must_use]
    pub const fn recovery_config(mut self, config: RecoveryConfig) -> Self {
        self.recovery_config = config;
        self
    }
}

#[cfg(test)]
mod tests {
    //! Coverage for recovery hook installation and propagation.

    use std::{error::Error, io, time::Duration};

    use rstest::{fixture, rstest};

    use super::*;
    use crate::{
        app::Envelope,
        codec::{
            CodecError,
            CodecErrorContext,
            LengthDelimitedFrameCodec,
            RecoveryConfig,
            RecoveryPolicy,
            RecoveryPolicyHook,
        },
        serializer::BincodeSerializer,
    };

    type TestApp = WireframeApp<BincodeSerializer, (), Envelope, LengthDelimitedFrameCodec>;

    /// Recovery hook whose only observable property is that it is installed.
    struct FixedPolicyHook;

    impl RecoveryPolicyHook for FixedPolicyHook {
        fn recovery_policy(&self, _error: &CodecError, _ctx: &CodecErrorContext) -> RecoveryPolicy {
            RecoveryPolicy::Drop
        }
    }

    #[fixture]
    fn app_builder() -> Result<TestApp, Box<dyn Error>> {
        TestApp::new().map_err(|error| {
            io::Error::other(format!("failed to construct test app: {error}")).into()
        })
    }

    #[fixture]
    fn custom_config() -> RecoveryConfig {
        RecoveryConfig::default()
            .max_consecutive_drops(3)
            .quarantine_duration(Duration::from_millis(25))
            .log_dropped_frames(false)
    }

    fn assert_custom_config_preserved(config: &RecoveryConfig, expected: &RecoveryConfig) {
        assert_eq!(
            config, expected,
            "a type-changing builder method should preserve the recovery config"
        );
    }

    /// Assert that `app` still carries the hook a builder step was given.
    ///
    /// The assertion lives outside the tests because
    /// `clippy::panic_in_result_fn` rejects panicking assertions in
    /// `Result`-returning functions, and the tests return `Result` so fixture
    /// failures can propagate with `?`.
    fn assert_hook_installed(app: &TestApp, step: &str) {
        assert!(
            app.recovery_hook.is_some(),
            "{step} should preserve the recovery hook"
        );
    }

    /// Assert that `app` is in the state a freshly built application has.
    fn assert_default_recovery_state(app: &TestApp) {
        assert!(
            app.recovery_hook.is_none(),
            "no recovery hook should be installed by default"
        );
        assert_eq!(app.recovery_config, RecoveryConfig::default());
    }

    #[rstest]
    fn builder_defaults_to_no_hook_and_default_config(
        app_builder: Result<TestApp, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let app = app_builder?;
        assert_default_recovery_state(&app);
        Ok(())
    }

    #[rstest]
    fn with_recovery_hook_installs_hook(
        app_builder: Result<TestApp, Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let app = app_builder?.with_recovery_hook(FixedPolicyHook);
        assert_hook_installed(&app, "with_recovery_hook");
        Ok(())
    }

    #[rstest]
    fn recovery_config_replaces_settings(
        app_builder: Result<TestApp, Box<dyn Error>>,
        custom_config: RecoveryConfig,
    ) -> Result<(), Box<dyn Error>> {
        let app = app_builder?.recovery_config(custom_config.clone());
        assert_custom_config_preserved(&app.recovery_config, &custom_config);
        Ok(())
    }

    /// A builder step that changes one of the application's type parameters.
    ///
    /// Every variant ends at [`TestApp`] again, which lets one parameterized
    /// test cover the transitions instead of repeating the same body per step.
    #[derive(Clone, Copy)]
    enum TypeTransition {
        /// Replace the frame codec.
        Codec,
        /// Replace the serializer.
        Serializer,
        /// Register a setup hook, which changes the connection-state type.
        ConnectionSetup,
    }

    /// Apply `transition` and return the application with its original types.
    fn apply_transition(
        app: TestApp,
        transition: TypeTransition,
    ) -> Result<TestApp, Box<dyn Error>> {
        match transition {
            TypeTransition::Codec => Ok(app.with_codec(LengthDelimitedFrameCodec::new(4096))),
            TypeTransition::Serializer => Ok(app.serializer(BincodeSerializer)),
            TypeTransition::ConnectionSetup => Ok(app.on_connection_setup(|| async {})?),
        }
    }

    #[rstest]
    #[case::with_codec(TypeTransition::Codec, "with_codec")]
    #[case::serializer(TypeTransition::Serializer, "serializer")]
    #[case::connection_setup(TypeTransition::ConnectionSetup, "on_connection_setup")]
    fn transition_preserves_recovery_state(
        app_builder: Result<TestApp, Box<dyn Error>>,
        custom_config: RecoveryConfig,
        #[case] transition: TypeTransition,
        #[case] step: &str,
    ) -> Result<(), Box<dyn Error>> {
        let app = apply_transition(
            app_builder?
                .with_recovery_hook(FixedPolicyHook)
                .recovery_config(custom_config.clone()),
            transition,
        )?;
        assert_hook_installed(&app, step);
        assert_custom_config_preserved(&app.recovery_config, &custom_config);
        Ok(())
    }

    #[rstest]
    #[case::with_codec(TypeTransition::Codec, "with_codec")]
    #[case::serializer(TypeTransition::Serializer, "serializer")]
    #[case::connection_setup(TypeTransition::ConnectionSetup, "on_connection_setup")]
    fn transition_then_recovery_hook_installs_hook(
        app_builder: Result<TestApp, Box<dyn Error>>,
        #[case] transition: TypeTransition,
        #[case] step: &str,
    ) -> Result<(), Box<dyn Error>> {
        let app = apply_transition(app_builder?, transition)?.with_recovery_hook(FixedPolicyHook);
        assert_hook_installed(&app, step);
        Ok(())
    }
}
