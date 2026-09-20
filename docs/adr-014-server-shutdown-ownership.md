# Architectural decision record (ADR) 014: server shutdown ownership and terminal observation

## Status

Proposed.

First proposed on 2026-09-20 in issue
[#657](https://github.com/leynos/wireframe/issues/657).

## Date

2026-09-20.

## Context and problem statement

`WireframeServer::run_with_shutdown` owns its cancellation token and task
tracker as local state. A caller may resolve the supplied shutdown future, or
abort the supervisor task, but neither operation gives it an awaitable proof
that accept loops have exited and the listener has been released. A retained
join handle can expose an abort or panic, but callers commonly do not own one,
and an aborted handle does not establish that the workers have drained.

This makes deterministic listener hand-off impossible for test harnesses,
restart supervisors, and deployment orchestrators without probing the socket.
It also lets a panicking supervisor be silently detached when nobody retained
its join handle.

ADR 013 sections 9 and 10 establish the required vocabulary for the client
pool: an awaitable shutdown protocol, non-blocking shutdown delivery, retained
task observation, and a distinct abnormal-termination outcome. This ADR
complements that decision by applying those sections to the server runtime. It
does not supersede ADR 011, ADR 012, or ADR 013.

## Traceability

This ADR governs the server-runtime half of Epic
[#635](https://github.com/leynos/wireframe/issues/635).

Primary code surfaces:

- `src/server/runtime.rs` and `src/server/runtime/server.rs`;
- `src/server/runtime/supervisor.rs`;
- `src/server/shutdown.rs`;
- `src/server/error.rs`;
- `src/metrics.rs`.

Related issues and decisions:

- Epic [#635](https://github.com/leynos/wireframe/issues/635);
- ADR 011 proposal [#636](https://github.com/leynos/wireframe/issues/636),
  especially requirements R5 and R6;
- ADR 012 proposal [#637](https://github.com/leynos/wireframe/issues/637);
- ADR 013 proposal [#638](https://github.com/leynos/wireframe/issues/638),
  sections 9 and 10;
- [#642](https://github.com/leynos/wireframe/issues/642), application
  preparation before readiness;
- [#656](https://github.com/leynos/wireframe/issues/656), cancellation when
  the foreground supervisor future is dropped;
- [#657](https://github.com/leynos/wireframe/issues/657), this implementation.

## Decision drivers

- Let callers request shutdown without waiting for mailbox capacity or a task.
- Provide a single awaitable terminal outcome after listener release.
- Preserve graceful draining for in-flight connection tasks.
- Retain the supervisor join handle until its terminal state is observed.
- Report a supervisor crash distinctly from clean shutdown and startup errors.
- Keep cloneable public control separate from single-owner runtime state.
- Reuse the vocabulary and lifecycle pattern settled by ADR 013.

## Options considered

### Option A: retain foreground-only shutdown

Continue requiring callers to own `run_with_shutdown` and arrange their own
shutdown future. This preserves the public API, but cannot establish listener
release after an abort and leaves detached supervisor failure unobserved.

### Option B: return a cloneable shutdown control with terminal observation (preferred)

Prepare the application, move the running state into one supervisor task, and
return a cloneable control handle. `stop()` cancels a token immediately;
`drained()` observes one terminal descriptor published after the supervisor
closes and drains its tracker. A separate observer exclusively owns the
supervisor join handle.

### Option C: send shutdown through a bounded command mailbox

An actor mailbox could acknowledge shutdown explicitly. The accept-loop runtime
does not otherwise need an actor protocol, and a full mailbox can delay the
very command needed to make progress unless it reserves shutdown capacity.

| Topic                       | Option A: foreground only | Option B: control and observer | Option C: mailbox       |
| --------------------------- | ------------------------- | ------------------------------ | ----------------------- |
| Non-blocking stop           | Caller-dependent          | Yes, cancellation token        | Needs reserved capacity |
| Awaitable listener release  | No                        | Yes                            | Yes                     |
| Join-handle observation     | Incidental                | Required                       | Required                |
| New coordination state      | None                      | One terminal descriptor        | Command protocol        |
| Preserves server simplicity | Partial                   | Yes                            | No                      |

_Table 1: Trade-offs for server shutdown ownership._

## Decision outcome

Adopt Option B.

### 1. Cloneable control over single-owner state

`ServerShutdown` is a cloneable control handle over a single shared inner
state. Cloning only increases that state root's reference count; it does not
clone the listener, prepared application, tracker, or supervisor. This is the
legitimate shared-handle shape retained by ADR 011 R6. The supervisor task
alone owns the coupled runtime state, satisfying ADR 011 R5's single-owner
rule. If the terminal protocol later acquires coupled transitions beyond this
one terminal descriptor, it must become an actor rather than distribute those
invariants across handle clones.

For screen readers: the figure shows one running supervisor owning the listener
and tracker. Many cloneable `ServerShutdown` controls can cancel the shared
token and receive the same terminal descriptor. One observer owns the
supervisor join handle until it publishes that descriptor.

```text
ServerShutdown clones ── stop() ──> CancellationToken
        │                                  │
        └──── drained() <── watch outcome <┴── observer <── JoinHandle
                                                    │
                                             supervisor task
                                             listener + tracker
```

_Figure 1: Server shutdown ownership and terminal observation._

### 2. Non-blocking cancellation and drain acknowledgement

`stop()` calls `CancellationToken::cancel()`. It is synchronous, idempotent,
and cannot wait behind a saturated component. The supervisor reacts by stopping
accept loops, then closes its `TaskTracker` and awaits it. Connection tasks are
still tracked without receiving the cancellation token, so their existing
graceful-drain semantics do not change.

`drained()` waits for the terminal descriptor. A clean descriptor is published
only after the supervisor has returned from its tracker drain; therefore a
successful result proves every accept loop has exited and the listener has been
released. Concurrent callers observe the same descriptor.

### 3. Observe abnormal termination

The observer retains the supervisor `JoinHandle`. A join panic or unexpected
supervisor failure publishes `ServerError::AbnormalTermination` with the
captured diagnostic message, emits the error-level
`server_supervisor_abnormal_termination` event, and increments
`wireframe_server_supervisor_abnormal_terminations_total`. The panic message is
not a metric label, so the metric remains low-cardinality.

Application factory and preparation failures remain the typed startup errors
introduced by #642. They occur before `spawn()` returns a control handle and
are not reported as supervisor crashes.

## Consequences

### Positive

- A caller can deterministically hand off a listener after `drained()`.
- Saturation cannot deadlock shutdown delivery.
- Supervisor panics are no longer silently detached.
- Clean stops, startup failures, and abnormal termination remain
  programmatically distinguishable.
- The server adopts ADR 013's established lifecycle vocabulary.

### Negative

- `spawn()` is asynchronous because preparation must preserve #642's typed
  startup failures before a running handle exists.
- The implementation adds a terminal watch channel and observer task.
- The public contract exposes one additional lifecycle type and error variant.

## Rejected shortcuts

- Do not make callers poll the listener: it is timing-dependent and cannot
  prove all accept loops have exited.
- Do not abort the supervisor as the shutdown protocol: cancellation does not
  establish a completed drain.
- Do not drop the supervisor join handle: that hides panic information.
- Do not route shutdown only through a potentially saturated mailbox.
- Do not pass the stop token into connection tasks: that would change graceful
  drain, which is out of scope.
- Do not convert #642 startup errors into `AbnormalTermination` merely to make
  terminal descriptors cloneable.

## Migration plan

1. Keep `run()` and `run_with_shutdown()` unchanged for foreground users.
2. Add `ServerShutdown`, terminal observation, abnormal error reporting, and
   observability to the bound-server runtime.
3. Move lifecycle consumers that need deterministic listener hand-off to
   `spawn()`, `stop()`, and `drained()`.
4. Update the testing lifecycle harness separately; it is a consumer, not part
   of this runtime contract.

## Verification

- Request shutdown, await `drained()`, and verify the address immediately
  refuses connections.
- Keep one connection in flight across `stop()` and verify it continues to
  drain before `drained()` resolves.
- Panic the supervisor observer input and verify the abnormal error, structured
  error event, and counter.
- Race several stop and drain callers and verify they converge to one outcome.
- Stop a busy server and verify delivery is synchronous while the eventual
  drain completes.
- Run formatting, type checking, linting, unit, and integration suites.

## Outstanding decisions

- The testing harness migration is tracked separately and may choose its own
  convenience wrapper over `ServerShutdown`.
- Automatic restart after `AbnormalTermination` remains out of scope, matching
  ADR 013 section 10.

## References

- [ADR 011: runtime ownership and task-lifetime boundaries](adr-011-runtime-ownership-and-task-lifetime-boundaries.md)
- [ADR 012: prepared application templates and connection-local runtimes](adr-012-prepared-application-and-connection-runtime.md)
- [ADR 013: single-owner client-pool scheduler and slot graph](adr-013-client-pool-scheduler-and-slot-ownership.md)
- [Issue #657](https://github.com/leynos/wireframe/issues/657)
