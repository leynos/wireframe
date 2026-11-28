//! `ServerTaskGuard` reaps a spawned task on both of its exit paths.
//!
//! The pass case pins the public surface: construction, the natural join for a
//! client-bounded server, the abort-and-join for a long-lived one, and the
//! value each runner returns.
use wireframe::test_helpers::ServerTaskGuard;

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let (body, server) = ServerTaskGuard::new(tokio::spawn(async { 7_u32 }))
            .run_and_join(async { Ok::<_, Box<dyn std::error::Error + Send + Sync>>(5_u32) })
            .await?;
        assert_eq!((body, server), (5, 7));

        let ((), aborted) = ServerTaskGuard::new(tokio::spawn(async {
            std::future::pending::<()>().await;
        }))
        .run_and_abort(async { Ok::<_, Box<dyn std::error::Error + Send + Sync>>(()) })
        .await?;
        assert!(aborted.is_none(), "an aborted server reports no value");
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })?;
    Ok(())
}