# Debugging plan: v0.6 BDD timeout scenarios

The v0.6 BDD run reports five failures. The inbound assembly timeout receives
one completed payload where the scenario expects none. Budget cleanup and
soft-pressure scenarios do not observe their expected payloads. The client
preamble scenario does not record its expected timeout error. The same
scenarios were green on the v0.5 baseline, where step errors were not reliably
propagated.

This plan focuses first on one falsifiable cause. The planning agent is not the
execution agent; an `alchemist` agent will run only the experiment below and
report `falsified`, `not-falsified`, or `inconclusive`.

## H1: Tokio virtual-time advance does not expire assembly state

`MessageAssemblyState` records and compares `std::time::Instant`. The failing
inbound scenario advances Tokio's paused clock by 30 ms before sending the
continuation. That clock advance cannot make the stored standard-library
instant older. The continuation can therefore complete the old assembly and
reach the handler. This may also explain why the budget-cleanup scenario fails
to reclaim an expired assembly.

- **Prediction:** If this hypothesis is correct, a focused rerun of
  `message_assembly_inbound_timeout` will pass when its wait helper also sleeps
  for the same real wall-clock duration, without changing the feature, timeout,
  or assertions.
- **Experiment:** Temporarily replace the Tokio-only advance in
  `MessageAssemblyInboundWorld::wait_millis` with
  `std::thread::sleep(Duration::from_millis(millis))`. Run only
  `scenarios::message_assembly_inbound_scenarios::message_assembly_inbound_timeout`
  using the repository's `tools/dev-fast/config.toml`, all features, and the
  shared Cargo cache. Restore the temporary edit before reporting.
- **Falsifying result:** The focused scenario still receives the completed
  payload after the real delay. That would rule out clock mismatch as a
  sufficient cause and require inspecting when the application purges state.
- **Supporting result:** The scenario observes zero payloads and passes after
  the real delay. This supports a clock mismatch; it does not by itself explain
  the soft-pressure or client-preamble failures.
- **Tooling:** One filtered `cargo test --test bdd` invocation and a temporary
  edit to the named fixture only. No repository-wide gates.

**Experiment result (2026-10-24):** NOT-FALSIFIED. The exact filtered scenario
passed (`1 passed; 0 failed; 206 filtered out`) after the temporary real-time
wait. Evidence is in `/tmp/h1-message-assembly-timeout.out`; the command is in
`/tmp/h1-message-assembly-timeout-command.txt`. The fixture was restored
byte-for-byte. This supports the clock mismatch but does not establish the
cause of the remaining failures.

## Other symptoms to revisit

- Soft pressure uses `tokio::time::sleep`; determine whether the test advances
  virtual time only after the server has entered that sleep.
- Client preamble timeout uses a 50 ms `tokio::time::timeout`; trace whether
  the connection result and failure callback are recorded on the same runtime
  before the scenario assertion. Source inspection found the slow-server
  closure drops its accepted stream before sleeping, which produces EOF rather
  than the expected timeout; the test fixture should keep the stream alive.
- Budget cleanup also depends on assembly expiry and should be rechecked after
  the clock experiment, without weakening its budget assertions.
