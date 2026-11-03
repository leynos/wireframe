//! Middleware pass-through measurement for the zero-copy benchmark matrix.
//!
//! The chain is built from the library's own [`Transform`] and [`Service`]
//! traits and terminated by a service that returns the request frame unchanged,
//! so the timed region is real middleware machinery: one `transform` at setup
//! time, then per-request policy application, continuation construction, and
//! terminal dispatch. The frame vector is recycled through `std::mem::take` and
//! the response, so no iteration allocates a payload and the number reflects
//! chain traversal alone.
//!
//! The chain is not built with [`wireframe::middleware::from_fn`] because that
//! adapter requires a single future type for every continuation lifetime
//! (`F: for<'a> Fn(ServiceRequest, Next<'a, S>) -> Fut`). A middleware that
//! forwards to `next.call` produces a future borrowing `Next`, so its type
//! varies with the lifetime and cannot satisfy that bound. This module therefore
//! mirrors the shape of the library's `FnService` instead: the same trait
//! objects, the same `Send + Sync` bounds, and the same `Next::new(&self.inner)`
//! dispatch, with only the callback erased behind `Policy`.

use std::{convert::Infallible, hint::black_box, time::Instant};

use async_trait::async_trait;
use wireframe::middleware::{Next, Service, ServiceRequest, ServiceResponse, Transform};

use super::{
    codec_benchmark_support::{BenchmarkWorkload, Measurement, payload_for_class},
    pipeline_benchmark_support::{AccessMode, empty_measurement},
};

/// Terminal service that returns the request frame unchanged.
struct EchoService;

#[async_trait]
impl Service for EchoService {
    type Error = Infallible;

    async fn call(&self, request: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        let _ = black_box(request.frame().len());
        let correlation_id = request.correlation_id();
        Ok(ServiceResponse::new(request.into_inner(), correlation_id))
    }
}

/// Middleware policy a measured chain applies before calling the next service.
trait Policy: Clone + Send + Sync + 'static {
    /// Applies the policy to one request.
    fn apply(&self, request: &mut ServiceRequest);
}

/// Pass-through policy: reads the frame and leaves it untouched.
#[derive(Clone, Copy)]
struct PassthroughPolicy;

impl Policy for PassthroughPolicy {
    fn apply(&self, request: &mut ServiceRequest) { let _ = black_box(request.frame().len()); }
}

/// Mutating policy: rewrites the first frame byte.
#[derive(Clone, Copy)]
struct MutatingPolicy;

impl Policy for MutatingPolicy {
    fn apply(&self, request: &mut ServiceRequest) {
        if let Some(first) = request.frame_mut().first_mut() {
            *first = first.wrapping_add(1);
        }
    }
}

/// Middleware that wraps a service with one [`Policy`].
struct PolicyMiddleware<P> {
    /// Policy applied to each request before continuation.
    policy: P,
}

/// Service produced by [`PolicyMiddleware::transform`].
struct PolicyService<S, P> {
    /// Wrapped service invoked by the continuation.
    inner: S,
    /// Policy applied to each request before continuation.
    policy: P,
}

#[async_trait]
impl<S, P> Transform<S> for PolicyMiddleware<P>
where
    S: Service + 'static,
    P: Policy,
{
    type Output = PolicyService<S, P>;

    async fn transform(&self, service: S) -> Self::Output {
        PolicyService {
            inner: service,
            policy: self.policy.clone(),
        }
    }
}

#[async_trait]
impl<S, P> Service for PolicyService<S, P>
where
    S: Service + 'static,
    P: Policy,
{
    type Error = S::Error;

    async fn call(&self, mut request: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        self.policy.apply(&mut request);
        Next::new(&self.inner).call(request).await
    }
}

/// Runs a measured middleware chain with one policy.
///
/// The frame vector is taken out of `buffer` and returned by the response, so
/// no iteration allocates a payload. Only the chain traversal and the
/// middleware's frame borrow are timed.
async fn run_middleware_iterations<P: Policy>(
    policy: P,
    payload_len: u64,
    mut buffer: Vec<u8>,
    iterations: u64,
) -> Measurement {
    let service = PolicyMiddleware { policy }.transform(EchoService).await;
    let started = Instant::now();
    for _ in 0..iterations {
        let request = ServiceRequest::new(std::mem::take(&mut buffer), Some(1));
        // The terminal service cannot fail: its error type is `Infallible`.
        let response = match service.call(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
        buffer = response.into_inner();
    }
    Measurement {
        operations: iterations,
        payload_bytes: iterations.saturating_mul(payload_len),
        elapsed: started.elapsed(),
    }
}

/// Measures middleware chain traversal for one workload and access mode.
///
/// # Examples
///
/// ```ignore
/// let measurement = measure_middleware_passthrough(workload, AccessMode::ReadOnly, 16).await;
/// assert_eq!(measurement.operations, 16);
/// ```
#[must_use]
pub async fn measure_middleware_passthrough(
    workload: BenchmarkWorkload,
    mode: AccessMode,
    iterations: u64,
) -> Measurement {
    if iterations == 0 {
        return empty_measurement();
    }
    let payload_len = workload.payload_class.len() as u64;
    let buffer = payload_for_class(workload.payload_class).to_vec();
    match mode {
        AccessMode::ReadOnly => {
            run_middleware_iterations(PassthroughPolicy, payload_len, buffer, iterations).await
        }
        AccessMode::Mutating => {
            run_middleware_iterations(MutatingPolicy, payload_len, buffer, iterations).await
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the middleware pass-through helper.

    use super::*;
    use crate::codec_benchmarks::{CodecUnderTest, PayloadClass, SMALL_PAYLOAD_BYTES};

    /// Small length-delimited workload used by the helper tests.
    fn small_workload() -> BenchmarkWorkload {
        BenchmarkWorkload {
            codec: CodecUnderTest::LengthDelimited,
            payload_class: PayloadClass::Small,
        }
    }

    #[test]
    fn middleware_passthrough_recycles_the_frame_buffer() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");
        let measurement = runtime.block_on(measure_middleware_passthrough(
            small_workload(),
            AccessMode::Mutating,
            4,
        ));
        assert_eq!(measurement.operations, 4);
        assert_eq!(measurement.payload_bytes, 4 * SMALL_PAYLOAD_BYTES as u64);
    }

    #[test]
    fn middleware_passthrough_reports_no_work_for_an_empty_batch() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("tokio runtime");
        let measurement = runtime.block_on(measure_middleware_passthrough(
            small_workload(),
            AccessMode::ReadOnly,
            0,
        ));
        assert_eq!(measurement.operations, 0);
    }
}
