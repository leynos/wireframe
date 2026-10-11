//! Counters and fixtures for measuring connection-startup preparation work.
//!
//! The prepared-connection path exists to evaluate an application's route and
//! middleware chains once per server run rather than once per connection. That
//! claim is about *how many times code ran*, which timing cannot establish, so
//! this module supplies counting fixtures instead: an application factory that
//! records its own invocations, and a middleware layer that records its
//! one-time transformations.
//!
//! Both counters are plain atomics shared through [`Arc`], readable at any
//! point with [`ConnectionStartupInstrumentation::snapshot`], so a caller can
//! compare counts before and after preparation, after serving N connections, or
//! after preparing a second application instance.
//!
//! ```rust,no_run
//! use wireframe::test_helpers::connection_startup::{
//!     ConnectionStartupInstrumentation,
//!     StartupFixturePlan,
//!     counted_app_factory,
//! };
//!
//! # async fn example() -> wireframe::Result<()> {
//! let instrumentation = ConnectionStartupInstrumentation::new();
//! let plan = StartupFixturePlan::default();
//! let factory = counted_app_factory(instrumentation.clone(), plan);
//! let app = factory()?;
//! let prepared = app.prepare().await?;
//! assert_eq!(instrumentation.snapshot().factory_calls, 1);
//! assert_eq!(
//!     instrumentation.snapshot().transforms,
//!     plan.route_transforms()
//! );
//! # drop(prepared);
//! # Ok(())
//! # }
//! ```

use std::{
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;

use crate::{
    Result as WireframeResult,
    app::{Envelope, Handler, WireframeApp},
    middleware::{HandlerService, Service, ServiceRequest, ServiceResponse, Transform},
    serializer::BincodeSerializer,
};

/// Concrete application type used by the connection-startup fixtures.
pub type StartupTestApp = WireframeApp<BincodeSerializer, (), Envelope>;

/// Shared counters recording how much one-time work preparation performed.
///
/// Cloning shares the same underlying counters, so a factory can hold one clone
/// while the caller keeps another to read snapshots from.
#[derive(Clone, Debug, Default)]
pub struct ConnectionStartupInstrumentation {
    /// Number of times the application factory has been invoked.
    factory_calls: Arc<AtomicUsize>,
    /// Number of one-time middleware transformations performed.
    transforms: Arc<AtomicUsize>,
}

impl ConnectionStartupInstrumentation {
    /// Create counters initialized to zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::test_helpers::connection_startup::ConnectionStartupInstrumentation;
    ///
    /// let instrumentation = ConnectionStartupInstrumentation::new();
    /// assert_eq!(instrumentation.snapshot().factory_calls, 0);
    /// ```
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Read the current counter values.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::test_helpers::connection_startup::ConnectionStartupInstrumentation;
    ///
    /// let instrumentation = ConnectionStartupInstrumentation::new();
    /// let counts = instrumentation.snapshot();
    /// assert_eq!((counts.factory_calls, counts.transforms), (0, 0));
    /// ```
    #[must_use]
    pub fn snapshot(&self) -> ConnectionStartupCounts {
        ConnectionStartupCounts {
            factory_calls: self.factory_calls.load(Ordering::SeqCst),
            transforms: self.transforms.load(Ordering::SeqCst),
        }
    }

    /// Record one application-factory invocation.
    fn record_factory_call(&self) { self.factory_calls.fetch_add(1, Ordering::SeqCst); }

    /// A counter handle incremented once per middleware transformation.
    fn transform_counter(&self) -> Arc<AtomicUsize> { Arc::clone(&self.transforms) }
}

/// Snapshot of [`ConnectionStartupInstrumentation`] counter values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectionStartupCounts {
    /// Application-factory invocations observed so far.
    pub factory_calls: usize,
    /// Middleware transformations observed so far.
    pub transforms: usize,
}

/// Route and middleware counts for a connection-startup fixture.
///
/// Varying these parameters proves that the recorded transformation count is
/// derived from the application's shape rather than a hard-coded constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupFixturePlan {
    /// Number of routes registered on the fixture application.
    routes: usize,
    /// Number of middleware layers wrapped around every route.
    middleware_layers: usize,
}

impl Default for StartupFixturePlan {
    fn default() -> Self {
        Self {
            routes: 2,
            middleware_layers: 2,
        }
    }
}

impl StartupFixturePlan {
    /// Build a plan with the given route and middleware-layer counts.
    ///
    /// Both counts are floored at one, because a fixture with no routes or no
    /// middleware would produce a transformation count indistinguishable from
    /// "preparation did nothing".
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::test_helpers::connection_startup::StartupFixturePlan;
    ///
    /// let plan = StartupFixturePlan::new(3, 2);
    /// assert_eq!((plan.routes(), plan.middleware_layers()), (3, 2));
    /// assert_eq!(plan.route_transforms(), 6);
    /// ```
    #[must_use]
    pub const fn new(routes: usize, middleware_layers: usize) -> Self {
        Self {
            routes: if routes == 0 { 1 } else { routes },
            middleware_layers: if middleware_layers == 0 {
                1
            } else {
                middleware_layers
            },
        }
    }

    /// Number of routes in this plan.
    #[must_use]
    pub const fn routes(self) -> usize { self.routes }

    /// Number of middleware layers in this plan.
    #[must_use]
    pub const fn middleware_layers(self) -> usize { self.middleware_layers }

    /// Transformations one full preparation of this plan must perform.
    ///
    /// Every layer wraps every route exactly once during preparation, so the
    /// expected count is the product of the two plan parameters.
    #[must_use]
    pub const fn route_transforms(self) -> usize { self.routes * self.middleware_layers }

    /// Route identifiers registered by this plan, in registration order.
    ///
    /// # Examples
    ///
    /// ```
    /// use wireframe::test_helpers::connection_startup::StartupFixturePlan;
    ///
    /// let plan = StartupFixturePlan::new(3, 1);
    /// assert_eq!(plan.route_ids(), vec![1, 2, 3]);
    /// ```
    #[must_use]
    pub fn route_ids(self) -> Vec<u32> {
        let routes = u32::try_from(self.routes).unwrap_or(u32::MAX);
        (1..=routes).collect()
    }
}

/// Middleware that counts how often it is transformed and tags frame payloads.
///
/// Each instance wraps one route during preparation, so counting here measures
/// the one-time cost the prepared path is meant to pay exactly once.
#[derive(Clone)]
pub struct TransformCountingMiddleware {
    /// Byte pushed around each handled frame to make the layer observable.
    tag: u8,
    /// Shared counter incremented by every one-time transformation.
    transforms: Arc<AtomicUsize>,
}

impl TransformCountingMiddleware {
    /// Build a layer that tags frames with `tag` and counts its transforms.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::{
    ///     convert::Infallible,
    ///     sync::{Arc, atomic::AtomicUsize},
    /// };
    ///
    /// use wireframe::test_helpers::connection_startup::TransformCountingMiddleware;
    ///
    /// let layer = TransformCountingMiddleware::new(b'A', Arc::new(AtomicUsize::new(0)));
    /// assert_eq!(layer.tag(), b'A');
    /// ```
    #[must_use]
    pub fn new(tag: u8, transforms: Arc<AtomicUsize>) -> Self { Self { tag, transforms } }
}

impl TransformCountingMiddleware {
    /// Byte this layer pushes around handled frames.
    #[must_use]
    pub const fn tag(&self) -> u8 { self.tag }
}

/// Service that tags requests and responses around its delegate.
struct TagService<S> {
    /// The middleware-transformed service this layer delegates to.
    inner: S,
    /// Byte pushed to the front of requests and the back of responses.
    tag: u8,
}

#[async_trait]
impl<S> Service for TagService<S>
where
    S: Service<Error = Infallible> + Send + Sync + 'static,
{
    type Error = Infallible;

    /// Push this layer's tag around the delegated call.
    async fn call(&self, mut request: ServiceRequest) -> Result<ServiceResponse, Self::Error> {
        request.frame_mut().push(self.tag);
        let mut response = self.inner.call(request).await?;
        response.frame_mut().push(self.tag);
        Ok(response)
    }
}

#[async_trait]
impl Transform<HandlerService<Envelope>> for TransformCountingMiddleware {
    type Output = HandlerService<Envelope>;

    /// Count this one-time transformation and wrap the route service.
    async fn transform(&self, service: HandlerService<Envelope>) -> Self::Output {
        self.transforms.fetch_add(1, Ordering::SeqCst);
        let id = service.id();
        HandlerService::from_service(
            id,
            TagService {
                inner: service,
                tag: self.tag,
            },
        )
    }
}

/// Build an application matching `plan`, recording transforms as it goes.
///
/// # Errors
///
/// Returns [`WireframeError::DuplicateRoute`](crate::WireframeError::DuplicateRoute)
/// when the same route identifier is registered twice, which cannot happen for
/// the identifiers [`StartupFixturePlan::route_ids`] produces.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::test_helpers::connection_startup::{
///     ConnectionStartupInstrumentation,
///     StartupFixturePlan,
///     build_counted_app,
/// };
///
/// let instrumentation = ConnectionStartupInstrumentation::new();
/// let app = build_counted_app(&instrumentation, StartupFixturePlan::new(2, 2))?;
/// drop(app);
/// # Ok::<(), wireframe::WireframeError>(())
/// ```
pub fn build_counted_app(
    instrumentation: &ConnectionStartupInstrumentation,
    plan: StartupFixturePlan,
) -> WireframeResult<StartupTestApp> {
    let mut app = StartupTestApp::new()?;
    for route_id in plan.route_ids() {
        app = app.route(route_id, noop_handler())?;
    }
    for layer in 0..plan.middleware_layers() {
        // Tags run in wrap order, so the first-wrapped layer is the outermost
        // tag and the response unwinds the tags in reverse.
        let tag = b'A'.saturating_add(u8::try_from(layer).unwrap_or(u8::MAX));
        app = app.wrap(TransformCountingMiddleware::new(
            tag,
            instrumentation.transform_counter(),
        ))?;
    }
    Ok(app)
}

/// Build a factory that records invocations and transforms for each app.
///
/// The returned closure increments `factory_calls` on every invocation, so
/// callers can distinguish "prepared once per server run" from "prepared once
/// per connection" by counting rather than by timing. Each invocation produces
/// a fresh application; prepare it to trigger the transform count.
///
/// # Examples
///
/// ```rust,no_run
/// use wireframe::test_helpers::connection_startup::{
///     ConnectionStartupInstrumentation,
///     StartupFixturePlan,
///     counted_app_factory,
/// };
///
/// let instrumentation = ConnectionStartupInstrumentation::new();
/// let factory = counted_app_factory(instrumentation.clone(), StartupFixturePlan::default());
/// let app = factory()?;
/// drop(app);
/// assert_eq!(instrumentation.snapshot().factory_calls, 1);
/// # Ok::<(), wireframe::WireframeError>(())
/// ```
pub fn counted_app_factory(
    instrumentation: ConnectionStartupInstrumentation,
    plan: StartupFixturePlan,
) -> impl Fn() -> WireframeResult<StartupTestApp> + Clone + Send + Sync + 'static {
    move || {
        instrumentation.record_factory_call();
        build_counted_app(&instrumentation, plan)
    }
}

/// Build a handler that accepts an envelope without changing it.
#[must_use]
pub fn noop_handler() -> Handler<Envelope> { Arc::new(|_: &Envelope| Box::pin(async {})) }

#[cfg(test)]
mod tests {
    //! Unit coverage for the fixture's own accounting.

    use super::{ConnectionStartupInstrumentation, StartupFixturePlan, build_counted_app};

    #[test]
    fn plan_floors_zero_counts_at_one() {
        let plan = StartupFixturePlan::new(0, 0);
        assert_eq!(plan.routes(), 1);
        assert_eq!(plan.middleware_layers(), 1);
        assert_eq!(plan.route_transforms(), 1);
    }

    #[test]
    fn plan_route_ids_are_one_based_and_contiguous() {
        assert_eq!(StartupFixturePlan::new(1, 1).route_ids(), vec![1]);
        assert_eq!(StartupFixturePlan::new(4, 1).route_ids(), vec![1, 2, 3, 4]);
    }

    /// The fixture's two error types (`WireframeError`, `PrepareError`) do not
    /// unify, and `panic_in_result_fn` rejects assertions in a `Result`-returning
    /// test, so this returns `()` and surfaces setup failures with `expect`.
    #[tokio::test]
    async fn preparation_transforms_each_route_once_per_layer() {
        let instrumentation = ConnectionStartupInstrumentation::new();
        let plan = StartupFixturePlan::new(3, 2);
        let app = build_counted_app(&instrumentation, plan).expect("fixture app must build");
        assert_eq!(instrumentation.snapshot().transforms, 0);

        let prepared = app.prepare().await.expect("preparation must succeed");
        assert_eq!(
            instrumentation.snapshot().transforms,
            plan.route_transforms()
        );
        drop(prepared);
    }
}
