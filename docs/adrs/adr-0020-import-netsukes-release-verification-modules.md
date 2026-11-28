# Architectural decision record (ADR) 020: import Netsuke's release-verification modules

## Status

Accepted.

Wireframe vendors the crates.io observation and byte-verification modules from
`leynos/netsuke` into `scripts/release`, without Netsuke's upload command, its
upload metadata builder, or its checksum sidecars. The modules run on Python
3.14.

## Date

2026-11-28.

## Context and problem statement

An upload to crates.io cannot be undone, and a release that half-succeeds
leaves the registry in a state the next run must read correctly. Before
publishing, a release has to know whether each version is absent, already
published with the same bytes, published with different bytes, or yanked. After
publishing, it has to confirm that the registry serves the bytes it meant to
upload.

`leynos/netsuke` already answers both questions. Its release scripts probe the
registry over an allowlisted HTTPS transport, translate the registry's two JSON
shapes into typed observations, decide from those observations whether to
publish, skip, or fail, and compare a downloaded archive's digest with a
trusted one. They are covered by example and property tests.

Wireframe publishes differently. lading uploads the workspace's crates in a
declared order, so Wireframe needs the observation and verification half of
Netsuke's tooling but not the half that uploads.

## Decision drivers

- The checks that guard an irreversible upload should already have survived
  use, not be written fresh for this repository.
- No module that observes or verifies may hold a registry credential, so a bug
  in one cannot publish.
- A transport failure must never read as absence, since absence is the state
  that permits a publish.
- Code that this repository does not run should not be imported, because it
  would still have to be reviewed and kept passing.

## Options considered

### Option A: write new verification code

Write probe, reconciliation, and verification code for Wireframe alone. This
costs the same review as an import and loses the test history and the registry
behaviour Netsuke measured, such as the XML refusal body that marks a missing
archive.

### Option B: depend on Netsuke's scripts as a package

Netsuke does not publish its release scripts as a package, so this would mean
building and versioning one first. Every release would also download that
package before it could check anything.

### Option C: vendor the modules that apply

Copy the modules at a recorded commit, drop the ones that do not apply, and
adapt names and the user agent. The tests come with them.

| Topic                            | Option A | Option B        | Option C     |
| -------------------------------- | -------- | --------------- | ------------ |
| Proven against the live registry | No       | Yes             | Yes          |
| Extra download per release       | No       | Yes             | No           |
| Picks up Netsuke fixes           | No       | By version bump | By re-import |
| Unused upload code present       | No       | Yes             | No           |

_Table 1: Ways to obtain release-verification code._

## Decision outcome

Option C. `scripts/release` holds Netsuke's modules as of commit
`56d1e0a224cec931eed9b55948b5f112113aba69`:

- `crates_io_registry_http` and `crates_io_transport_http` send requests only
  over HTTPS to `crates.io` and `static.crates.io`, recheck every redirect
  target against that allowlist, and bound archive polling;
- `crates_io_transport_cli` records one registry probe or one downloaded
  archive for the offline steps;
- `release_registry_response` and `release_crates_io_adapter` translate
  responses into typed observations;
- `release_reconcile_crates` and `verify_crates_io_bytes` decide what those
  observations permit; and
- `release_digest` and `release_digest_files` hash files and compare digests in
  constant time.

Three parts of Netsuke's tooling are excluded:

- the upload command, because lading publishes here;
- the upload metadata builder, which only the upload command consumed; and
- the checksum-sidecar code, which describes binary release assets, and this
  library ships none.

With the upload code gone, no imported module accepts a registry credential.

The modules require Python 3.14. They annotate parameters with types imported
only under `typing.TYPE_CHECKING`, which works because Python 3.14 evaluates
annotations lazily, and they use the `type` statement.
`make test-release-scripts` and `make lint-python-docs` therefore run on Python
3.14 through `uv`, the same interpreter lading needs.

## Known risks and limitations

- The copy drifts from Netsuke. A fix there does not reach Wireframe until
  someone re-imports it; `scripts/release/README.md` records the source commit
  so a re-import can start from a diff.
- Local changes after the import, such as reporting a probe's local write
  failure apart from a transport failure, are not yet upstream. A re-import
  must keep them or carry them to Netsuke first.
- Python 3.14 is newer than many distributions ship. Contributors need `uv`,
  which fetches the interpreter, rather than a system Python.

## References

- [`scripts/release/README.md`](../../scripts/release/README.md) records the
  provenance and the module roles.
- The developers' guide section
  [Release verification scripts](../developers-guide.md#release-verification-scripts)
  describes each module's boundary.
