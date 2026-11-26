//! Startup and shutdown error mapping for the client pair.

use wireframe::server::ServerError;

use crate::TestError;

/// Re-wrap a startup failure as the pair's historical message shape.
///
/// Only [`TestError::Server`] is re-wrapped: an address or binding failure is
/// also a server error, and Table 1 in the plan keeps those typed. Callers
/// therefore see this mapping applied to the second half of the startup
/// sequence only, never to the bind.
pub(super) fn map_pair_startup_error(error: TestError) -> TestError {
    match error {
        TestError::Server(server_error) => {
            TestError::Msg(format!("server failed to start: {server_error}"))
        }
        other => other,
    }
}

/// Re-wrap the handle's shutdown failure as the pair's historical shape.
///
/// The pair predates `ServerShutdown`, so a supervisor panic arrived as a
/// `JoinError` and any other server failure as a plain server error. The
/// handle reports both as `TestError::Server`; the source-less abnormal
/// termination is the panic class.
pub(super) fn map_pair_shutdown_error(error: TestError) -> TestError {
    match error {
        TestError::Server(ServerError::AbnormalTermination { source: None, .. }) => {
            TestError::Msg(format!("server task join error: {error}"))
        }
        TestError::Server(_) => TestError::Msg(format!("server error: {error}")),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pair's error mapping.
    //!
    //! Each case pins the variant and message prefix a caller observes, which
    //! is the part of the pair's contract the delegation must not change.

    use std::io;

    use wireframe::server::ServerError;

    use super::{map_pair_shutdown_error, map_pair_startup_error};
    use crate::TestError;

    /// The factory error the server reports when startup fails.
    fn factory_failure() -> ServerError {
        ServerError::FactoryBuild(Box::new(io::Error::other("boom")))
    }

    #[test]
    fn startup_mapping_rewraps_only_server_errors() {
        let mapped = map_pair_startup_error(TestError::Server(factory_failure()));
        match mapped {
            TestError::Msg(message) => assert!(
                message.starts_with("server failed to start: "),
                "unexpected message: {message}"
            ),
            other => panic!("a server error must be re-wrapped, got {other:?}"),
        }

        let io_error = TestError::Io(io::Error::other("reserve"));
        assert!(
            matches!(map_pair_startup_error(io_error), TestError::Io(_)),
            "an IO failure must keep its typed variant"
        );
        let message = TestError::Msg("custom".to_owned());
        assert!(
            matches!(map_pair_startup_error(message), TestError::Msg(text) if text == "custom"),
            "a message must pass through unchanged"
        );
    }

    #[test]
    fn shutdown_mapping_separates_the_panic_class() {
        let panicked = TestError::Server(ServerError::AbnormalTermination {
            message: "panic".to_owned(),
            source: None,
        });
        match map_pair_shutdown_error(panicked) {
            TestError::Msg(message) => assert!(
                message.starts_with("server task join error: "),
                "unexpected message: {message}"
            ),
            other => panic!("a source-less termination must map to the join error, got {other:?}"),
        }

        let returned = TestError::Server(ServerError::AbnormalTermination {
            message: "returned".to_owned(),
            source: Some(std::sync::Arc::new(factory_failure())),
        });
        match map_pair_shutdown_error(returned) {
            TestError::Msg(message) => assert!(
                message.starts_with("server error: "),
                "unexpected message: {message}"
            ),
            other => panic!("a returned server error must map to server error, got {other:?}"),
        }
    }
}
