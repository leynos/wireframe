# Add pinned Kani and Verus tooling scripts

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
 `Tolerances`, `Risks`, `Progress`, `Surprises & discoveries`, `Decision log`,
and `Outcomes & retrospective` must be kept up to date as work proceeds.

Status: DRAFT (awaiting approval)

This plan must be approved before implementation starts. Until then, roadmap
item 15.1.3 remains unchecked in `docs/roadmap.md`.

## Purpose / big picture

Roadmap item 15.1.3 establishes reproducible local installation for the formal
verification tools that later Kani, Verus, Stateright, Makefile, and continuous
integration (CI) tasks depend on.

After the approved implementation, a contributor can run
`./scripts/install-kani.sh` and `./scripts/install-verus.sh` from the
repository root and obtain the repository-pinned versions of Kani and Verus.
The Kani installer reads `tools/kani/VERSION`, installs `kani-verifier` with
`cargo install --locked`, runs `cargo kani setup`, and confirms that
`cargo kani --version` is callable. The Verus installer reads
`tools/verus/VERSION` and `tools/verus/SHA256SUMS`, downloads the matching
Verus release archive, verifies its SHA-256 checksum, and stages a local Verus
binary. `scripts/run-verus.sh` resolves `VERUS_BIN`, installs the Rust
toolchain reported by `verus --version` when necessary, and runs the proof
entry point once proof files exist.

Observable success is:

- `./scripts/install-kani.sh` installs the version named in
  `tools/kani/VERSION` and prints a matching `cargo kani --version`.
- `./scripts/install-verus.sh` installs the version named in
  `tools/verus/VERSION` only after the downloaded archive matches
  `tools/verus/SHA256SUMS`.
- `scripts/run-verus.sh` fails clearly when no proof file exists, and succeeds
  once the first `verus/wireframe_proofs.rs` entry point is added by a later
  roadmap item.
- Unit tests using `rstest` verify the metadata and script contracts without
  installing external tools.
- Behavioural tests using `rstest-bdd` describe the contributor workflow for
  pinned Kani and Verus tooling.
- `docs/developers-guide.md` explains the local formal-verification tooling
  convention, while `docs/formal-verification-methods-in-wireframe.md` stays
  aligned with the implemented paths.
- `docs/roadmap.md` marks 15.1.3 done only after the implementation and gates
  pass.

## Constraints

- Do not begin implementation until this ExecPlan is explicitly approved.
- Keep 15.1.3 scoped to tool metadata and repo-local scripts. Do not add the
  formal Makefile targets owned by 15.1.4, and do not add CI jobs owned by
  15.1.5.
- Keep Verus out of the main Cargo build and test flow. It is a standalone
  proof runner, not a Cargo dependency or normal test target.
- Keep Kani harnesses, Verus proofs, and Stateright model expansion out of
  this item. Later roadmap items own those proof obligations.
- Pin tool versions in repository metadata files, not inside script bodies.
- Verify external Verus downloads by SHA-256 before extracting them.
- Make the scripts idempotent and safe to rerun. If a pinned Verus binary is
  already installed, the installer should report that and exit successfully.
- Preserve existing root package and workspace behaviour established by 15.1.1
  and 15.1.2.
- Use `rstest` for unit tests and `rstest-bdd` for workflow-level behavioural
  tests where applicable.
- Do not mutate environment variables directly in tests. If environment-driven
  behaviour needs testing, drive it through subprocess environment injection or
  pure helper functions.
- Keep every Rust source file under the repository's 400-line limit.
- Use en-GB Oxford spelling in documentation.
- Run validation commands sequentially and capture logs with `tee` under
  `/tmp`; do not run format, lint, or tests in parallel.
- Use `coderabbit review --agent` after each major implementation milestone
  and resolve all concerns before moving to the next milestone.
- Commit changes in small, gated commits during implementation. Use a
  file-based commit message with `git commit -F`.

## Tolerances

- If the implementation requires changing more than 12 files before Makefile or
  CI work, stop and reassess scope.
- If scripts require a new Rust crate dependency, stop and ask for approval.
- If a public Wireframe API must change, stop and ask for approval.
- If the chosen Verus release does not publish an `x86-linux` archive, stop and
  choose a supported target strategy with human review.
- If checksum generation requires downloading more than the selected release
  archive for the current Linux target, stop and confirm whether multi-platform
  checksum coverage is required in this task.
- If `./scripts/install-kani.sh` or `./scripts/install-verus.sh` still fails
  after three implementation attempts for reasons other than host
  prerequisites, stop and record the blocker.
- If `make check-fmt`, `make lint`, or `make test` fails for a pre-existing
  reason, capture the log, identify the unrelated baseline failure, and ask
  whether to proceed with a narrowly justified commit.
- If `coderabbit review --agent` reports concerns that require crossing any of
  these tolerances, stop and seek approval.

## Risks

- Risk: Verus release assets and version naming are date-and-hash based rather
  than plain semantic versions. Severity: medium. Likelihood: high. Mitigation:
  keep the exact Verus token in `tools/verus/VERSION`, derive the archive name
  from that token, and validate the archive with `tools/verus/SHA256SUMS`.

- Risk: Kani's latest release or required Rust nightly may change before
  implementation starts. Severity: low. Likelihood: medium. Mitigation:
  re-check the official Kani release list at implementation time, pin one
  explicit `MAJOR.MINOR.PATCH` release, and rely on `cargo kani setup` to
  install Kani's required nightly.

- Risk: installing formal-verification tools is too expensive for ordinary
  `make test`. Severity: medium. Likelihood: high. Mitigation: test script
  metadata and contract helpers in the normal suite, but keep actual Kani and
  Verus installation as explicit validation steps and future formal targets.

- Risk: shell scripts become hard to test if all behaviour is inline. Severity:
  medium. Likelihood: medium. Mitigation: keep script branches simple, validate
  syntax with `bash -n`, and test observable file contracts through Rust tests
  rather than reimplementing the shell logic.

- Risk: the first Verus runner has no proof file to run until later roadmap
  work. Severity: low. Likelihood: high. Mitigation: make the runner fail with
  a clear "proof file not found" diagnostic during 15.1.3 and document that
  15.3/15.4 work will add the proof entry point.

## Progress

- [x] (2026-05-20 23:19 UTC) Loaded the `leta`, `kani`, `verus`,
      `execplans`, `firecrawl-mcp`, `pr-creation`, `commit-message`,
      `en-gb-oxendict-style`, and `rust-router` skills relevant to this
      planning task.
- [x] (2026-05-20 23:19 UTC) Created a `leta` workspace for this repository.
- [x] (2026-05-20 23:19 UTC) Renamed the branch to
      `15-1-3-kani-and-verus-metadata-plus-scripts`.
- [x] (2026-05-20 23:19 UTC) Used a Wyvern agent team for read-only planning
      reconnaissance covering local formal-verification requirements, Netsuke
      and Chutoro script prior art, and validation conventions.
- [x] (2026-05-20 23:19 UTC) Used Firecrawl to resolve external release and
      prior-art context, then verified current release details through GitHub
      release metadata.
- [x] (2026-05-20 23:19 UTC) Drafted this pre-implementation ExecPlan.
- [x] (2026-05-20 23:40 UTC) Validated the draft with targeted Markdown lint,
      `make check-fmt`, `make markdownlint`, `make nixie`, `make lint`, and
      `make test`.
- [x] (2026-05-20 23:52 UTC) Ran `coderabbit review --agent`, applied its two
      minor prose findings, and confirmed targeted Markdown lint still passes.
- [ ] Obtain explicit approval for this ExecPlan.
- [ ] Implement the approved plan in small gated commits.
- [ ] Mark roadmap item 15.1.3 done after implementation validation passes.

## Surprises & discoveries

- Observation: Netsuke currently carries the hardened Kani installer and Kani
  version checker, but does not appear to carry Verus installer and runner
  scripts. Evidence: repository script listings showed `install-kani.sh` and
  `check-kani-version.sh` in Netsuke, while Verus scripts were found in
  Chutoro. Impact: use Netsuke for the Kani pattern and Chutoro for the Verus
  pattern.

- Observation: this branch has no `tools/` directory yet, and only
  `scripts/doctest-benchmark.sh` exists under `scripts/`. Evidence: local
  reconnaissance found no `tools/` directory and no formal-verification
  scripts. Impact: 15.1.3 must create both metadata directories and new scripts.

- Observation: the latest stable Kani release visible through GitHub metadata
  during planning was `kani-0.67.0`, published on 2026-01-16. Evidence:
  `gh release list --repo model-checking/kani --limit 5`. Impact:
  implementation should re-check this before pinning, but `0.67.0` is a
  reasonable candidate.

- Observation: the latest non-prerelease Verus release visible through GitHub
  metadata during planning was `release/0.2026.05.17.e479cce`, published on
  2026-05-18; a newer rolling prerelease existed on 2026-05-20. Evidence:
  `gh release list --repo verus-lang/verus --limit 5`. Impact: prefer the
  latest non-prerelease release unless a reason to pin the rolling prerelease
  is documented.

## Decision log

- Decision: keep this pull request plan-only and leave 15.1.3 unchecked.
  Rationale: the user explicitly required plan approval before implementation.
  Date/Author: 2026-05-20 / Codex.

- Decision: follow Netsuke's Kani installer shape for `install-kani.sh`.
  Rationale: it already validates `tools/kani/VERSION`, enforces
  `MAJOR.MINOR.PATCH`, uses `cargo install --locked`, runs `cargo kani setup`,
  and confirms the installed command is callable. Date/Author: 2026-05-20 /
  Codex.

- Decision: follow Chutoro's Verus installer and runner shape for
  `install-verus.sh` and `run-verus.sh`. Rationale: Wireframe's formal
  verification design document already cites Chutoro for Verus tooling, and
  Chutoro's scripts implement version files, checksums, binary resolution, and
  toolchain bootstrap. Date/Author: 2026-05-20 / Codex.

- Decision: add tests for metadata and script contracts, not full external tool
  installation, to the normal test suite. Rationale: Kani and Verus
  installation belongs to explicit formal-verification validation, while normal
  `make test` should stay deterministic and avoid network-dependent tool
  installs. Date/Author: 2026-05-20 / Codex.

- Decision: use `coderabbit review --agent` as a milestone gate after the
  script/metadata implementation and again before the final implementation
  commit. Rationale: the user requested CodeRabbit validation after each major
  milestone, and script tooling benefits from an independent review pass.
  Date/Author: 2026-05-20 / Codex.

## Outcomes & retrospective

This section is intentionally empty while the plan is in draft. During
implementation, it must record what landed, which validation commands passed,
which CodeRabbit concerns were resolved, any deviations from the plan, and what
remains for 15.1.4 and later formal-verification work.

Draft validation passed on 2026-05-20 before the plan-only commit:
`markdownlint-cli2 docs/execplans/15-1-3-kani-and-verus-metadata-plus-scripts.md`,
`make check-fmt`, `make markdownlint`, `make nixie`, `make lint`, and
`make test`.

`coderabbit review --agent` completed once and reported two minor prose
findings. Both were applied. Two follow-up attempts to re-run CodeRabbit were
blocked by recoverable service rate limits after the requested wait windows.

## Context and orientation

The formal-verification phase in `docs/roadmap.md` starts at phase 15. Roadmap
item 15.1.1 has already converted the repository into a hybrid Cargo workspace,
and 15.1.2 has already added `crates/wireframe-verification` as an internal
Stateright crate.

Roadmap item 15.1.3 is the next infrastructure item. It adds the pinned Kani
and Verus metadata plus local installation and runner scripts that later
roadmap items will use. The relevant design source is
`docs/formal-verification-methods-in-wireframe.md`, especially "Recommended
repository layout" and "Why Verus should not live inside the main build".

Terms used in this plan:

- Kani is a Rust bounded model checker. It exhaustively explores every input
  within configured bounds and is suitable for small parser, framing, and state
  invariants.
- Verus is a deductive verifier for Rust-like proof files. It verifies
  mathematical specifications and proofs with an SMT solver and should stay
  separate from the normal Cargo build.
- Stateright is a Rust model checker for abstract state machines. Wireframe's
  Stateright crate already exists from 15.1.2 and is not expanded by 15.1.3.
- `SHA256SUMS` is a plain checksum manifest. For Verus, each line should name
  one expected archive digest and archive file, for example
  `HEX  verus-VERSION-x86-linux.zip`.

Relevant local files and directories:

- `docs/roadmap.md` owns roadmap item 15.1.3 and must be marked done only after
  implementation completes.
- `docs/formal-verification-methods-in-wireframe.md` already documents the
  preferred `scripts/` and `tools/` layout.
- `docs/developers-guide.md` should explain how contributors install and use
  the pinned formal-verification tools.
- `docs/users-guide.md` should not need a behaviour update unless the
  implementation changes public library behaviour. This task is expected to be
  internal tooling only.
- `scripts/` currently contains the doctest benchmark helper and will gain
  formal-verification scripts.
- `tools/` does not exist yet and will be created for tool pins.
- `tests/workspace_manifest.rs`, `tests/features/workspace_manifest.feature`,
  and `tests/scenarios/workspace_manifest_scenarios.rs` show the existing
  pattern for testing repository-level infrastructure with `rstest` and
  `rstest-bdd`.

Relevant skills:

- Use `leta` for code navigation and symbol lookups.
- Use `kani` when designing future bounded model-checking harnesses, but do
  not add harnesses in 15.1.3.
- Use `verus` when designing future proof files, but do not add proofs in
  15.1.3.
- Use `rust-router` before Rust changes; route to smaller Rust skills only if
  implementation touches Rust APIs beyond repository-contract tests.
- Use `firecrawl-mcp` or current GitHub release metadata when re-checking
  external tool versions before pinning.
- Use `commit-message` for file-based commits and `pr-creation` for the draft
  pull request.

## Plan of work

Stage A is the approval gate. Keep this ExecPlan as `Status: DRAFT` until the
user explicitly approves it. Do not create `tools/`, new scripts, test files,
or roadmap updates before approval.

Stage B creates the metadata pins. Re-check the latest stable Kani and Verus
releases from official GitHub release metadata. Prefer the latest stable Kani
release that supports `cargo install --locked kani-verifier --version`. Prefer
the latest non-prerelease Verus release unless a documented reason exists to
pin a rolling prerelease. Create `tools/kani/VERSION` with the Kani
`MAJOR.MINOR.PATCH` value. Create `tools/verus/VERSION` with the Verus release
token without the `release/` prefix. Create `tools/verus/SHA256SUMS` with at
least the checksum for the Linux archive used by the installer,
`verus-${VERSION}-x86-linux.zip`. If multi-platform archive checksums are added
now, document that decision in this plan.

Stage C creates the scripts. Add `scripts/install-kani.sh`,
`scripts/install-verus.sh`, and `scripts/run-verus.sh`, all executable. The
Kani installer should follow Netsuke's pattern: locate the repository root,
require `cargo`, read and validate `tools/kani/VERSION`, run
`cargo install --locked kani-verifier --version "$kani_version"`, run
`cargo kani setup`, and print `cargo kani --version`. The Verus installer
should follow Chutoro's pattern: require `tools/verus/VERSION` and
`tools/verus/SHA256SUMS`, accept `VERUS_TARGET` with default `x86-linux`, accept
 `VERUS_INSTALL_DIR` with a repository-local default, derive the GitHub release
archive URL, verify SHA-256 with `sha256sum` or `shasum`, extract with `unzip`,
and stage the binary under a stable `verus` directory. The Verus runner should
resolve `VERUS_BIN` from a binary path, install directory, or command on
`PATH`; call the installer if the default binary is missing; parse the required
Rust toolchain from `verus --version`; install that toolchain with `rustup`
when missing; and run `${VERUS_PROOF_FILE:-verus/wireframe_proofs.rs}`.

Run `coderabbit review --agent` after Stage C. If it reports concerns, resolve
them before adding tests.

Stage D adds tests. Add `rstest` coverage for repository tooling contracts:
required metadata files exist, Kani's pin is non-empty and matches
`MAJOR.MINOR.PATCH`, Verus' pin is non-empty, `tools/verus/SHA256SUMS` contains
an entry for the configured Linux archive, all three scripts exist, and each
script starts with a Bash shebang and `set -euo pipefail`. Add syntax checks
with `bash -n` either as Rust integration tests that spawn Bash or as explicit
validation commands recorded in this plan. Add a `rstest-bdd` feature and
scenario describing the contributor workflow: the repository declares pinned
Kani and Verus tooling, the install scripts are present, and the pin metadata
is consistent. Keep these tests network-free.

Stage E updates internal documentation. Update
`docs/formal-verification-methods-in-wireframe.md` only where the implemented
script contracts differ from the current design text. Update
`docs/developers-guide.md` with the contributor-facing convention for running
`./scripts/install-kani.sh`, `./scripts/install-verus.sh`, and
`scripts/run-verus.sh`. Do not update `docs/users-guide.md` unless public
library behaviour changes, which is not expected.

Stage F validates the implementation. Run all commands sequentially with `tee`
logs under `/tmp`, using the current branch name in each log file. The required
implementation gates are `make check-fmt`, `make lint`, and `make test`. Also
run `make markdownlint` and `make nixie` if documentation changed. Run
`./scripts/install-kani.sh` and `./scripts/install-verus.sh` to prove the
roadmap success criteria. Run `bash -n` for all new shell scripts. If
`scripts/run-verus.sh` is expected to fail because no proof file exists yet,
capture and document the clear "proof file not found" diagnostic rather than
treating that as a failed 15.1.3 acceptance criterion.

Stage G performs final review and roadmap completion. Run
`coderabbit review --agent` again. Resolve all concerns. Mark roadmap item
15.1.3 done in `docs/roadmap.md`, update this ExecPlan's progress and outcomes,
run the relevant validation again if the final edits touched validated files,
and commit the final implementation state.

## Concrete steps

All commands run from the repository root:

```sh
cd /home/leynos/.lody/repos/github---leynos---wireframe/worktrees/33edb5cf-ed18-43de-b3d5-4dc774ec7610
```

Approval gate:

```sh
git status --short --branch
```

Expected output includes:

```plaintext
## 15-1-3-kani-and-verus-metadata-plus-scripts...origin/main
```

Re-check releases before choosing pins:

```sh
gh release list --repo model-checking/kani --limit 5
gh release list --repo verus-lang/verus --limit 5
gh release view release/0.2026.05.17.e479cce --repo verus-lang/verus \
  --json tagName,name,isPrerelease,publishedAt,assets
```

Create metadata and scripts after approval:

```sh
mkdir -p tools/kani tools/verus
$EDITOR tools/kani/VERSION
$EDITOR tools/verus/VERSION
$EDITOR tools/verus/SHA256SUMS
$EDITOR scripts/install-kani.sh
$EDITOR scripts/install-verus.sh
$EDITOR scripts/run-verus.sh
chmod +x scripts/install-kani.sh scripts/install-verus.sh scripts/run-verus.sh
```

Use `apply_patch` rather than editor commands when this plan is implemented by
an agent.

Run shell syntax checks:

```sh
bash -n scripts/install-kani.sh
bash -n scripts/install-verus.sh
bash -n scripts/run-verus.sh
```

Run installation smoke checks:

```sh
branch="$(git branch --show-current)"
./scripts/install-kani.sh 2>&1 |
  tee "/tmp/install-kani-wireframe-${branch}.out"
VERUS_INSTALL_DIR="${PWD}/.verus/$(tr -d '[:space:]' < tools/verus/VERSION)" \
  ./scripts/install-verus.sh 2>&1 |
  tee "/tmp/install-verus-wireframe-${branch}.out"
```

Run repository gates:

```sh
branch="$(git branch --show-current)"
make check-fmt 2>&1 | tee "/tmp/check-fmt-wireframe-${branch}.out"
make lint 2>&1 | tee "/tmp/lint-wireframe-${branch}.out"
make test 2>&1 | tee "/tmp/test-wireframe-${branch}.out"
make markdownlint 2>&1 | tee "/tmp/markdownlint-wireframe-${branch}.out"
make nixie 2>&1 | tee "/tmp/nixie-wireframe-${branch}.out"
```

Run CodeRabbit after major milestones:

```sh
coderabbit review --agent
```

Commit with a file-based message:

```sh
git status --short
git diff --cached
COMMIT_MSG_DIR="$(mktemp -d)"
cat > "${COMMIT_MSG_DIR}/COMMIT_MSG.md" << 'ENDOFMSG'
Add formal verification tooling scripts

Add pinned Kani and Verus metadata plus repository-local helper scripts so
contributors can install the supported formal-verification toolchain
reproducibly.
ENDOFMSG
git commit -F "${COMMIT_MSG_DIR}/COMMIT_MSG.md"
rm -rf "${COMMIT_MSG_DIR}"
```

## Validation and acceptance

The approved implementation is accepted only when these checks pass or their
documented baseline failures are explicitly approved:

- `bash -n scripts/install-kani.sh`
- `bash -n scripts/install-verus.sh`
- `bash -n scripts/run-verus.sh`
- `./scripts/install-kani.sh`
- `./scripts/install-verus.sh`
- `make check-fmt`
- `make lint`
- `make test`
- `make markdownlint`, if documentation changed
- `make nixie`, if Markdown diagrams are present or changed
- `coderabbit review --agent` with all concerns resolved

Expected test coverage:

- New `rstest` unit/integration tests fail before the metadata and scripts
  exist and pass after implementation.
- New `rstest-bdd` scenario coverage fails before the repository declares the
  formal-verification tooling workflow and passes after implementation.
- No Kani harness or Verus proof is required for 15.1.3 because this item
  installs tool plumbing rather than proving protocol properties.

Expected contributor behaviour after implementation:

```sh
./scripts/install-kani.sh
./scripts/install-verus.sh
```

Both commands should exit 0 on a host with the required external prerequisites.
`scripts/run-verus.sh` should either run the selected proof file or fail with a
clear diagnostic naming the missing proof file until later Verus proof work
adds it.

## Idempotence and recovery

The scripts must be safe to rerun. `install-kani.sh` may reinstall the pinned
Cargo binary because `cargo install --locked --version` is the supported
installer path. `install-verus.sh` should skip downloading when the pinned
binary already exists at the requested install directory.

If a Verus download fails before extraction, rerun the same command. Temporary
download directories must be cleaned with a shell `trap`.

If a Verus checksum fails, delete the temporary archive, keep the existing
installed binary untouched, and fail with expected and actual SHA-256 values.

If the implementation needs rollback before commit, remove only files created
by this roadmap item:

```sh
rm -rf tools/kani tools/verus
rm -f scripts/install-kani.sh scripts/install-verus.sh scripts/run-verus.sh
```

Do not revert unrelated user or agent changes.

## Artifacts and notes

Planning sources checked:

- `docs/roadmap.md` item 15.1.3 defines the success criteria for
  `./scripts/install-kani.sh` and `./scripts/install-verus.sh`.
- `docs/formal-verification-methods-in-wireframe.md` defines the preferred
  `scripts/` and `tools/` layout and keeps Verus outside the main build.
- Netsuke's `scripts/install-kani.sh` and `scripts/check-kani-version.sh`
  provide the hardened Kani installer and version-check patterns.
- Chutoro's `scripts/install-verus.sh`, `scripts/run-verus.sh`,
  `tools/verus/VERSION`, and `tools/verus/SHA256SUMS` provide the Verus
  installer, runner, and checksum patterns.
- Kani GitHub release metadata listed `kani-0.67.0` as the latest release at
  planning time.
- Verus GitHub release metadata listed `release/0.2026.05.17.e479cce` as the
  latest non-prerelease release at planning time.

The plan-only pull request should mention this file and should not claim that
formal-verification tooling has been implemented yet.

## Interfaces and dependencies

The implementation should create these repository-local interfaces:

```plaintext
tools/kani/VERSION
tools/verus/VERSION
tools/verus/SHA256SUMS
scripts/install-kani.sh
scripts/install-verus.sh
scripts/run-verus.sh
```

`scripts/install-kani.sh` accepts no required arguments. It depends on `cargo`
and the pinned `tools/kani/VERSION` file.

`scripts/install-verus.sh` accepts these environment variables:

- `VERUS_TARGET`, defaulting to `x86-linux`.
- `VERUS_INSTALL_DIR`, defaulting to a repository-local directory for the
  pinned version.

It depends on `curl`, `unzip`, and either `sha256sum` or `shasum`.

`scripts/run-verus.sh` accepts these environment variables:

- `VERUS_BIN`, which may be a binary path, install directory, or command name.
- `VERUS_INSTALL_DIR`, used to derive the default binary path.
- `VERUS_PROOF_FILE`, defaulting to `verus/wireframe_proofs.rs`.

It depends on `rustup` when the required Verus Rust toolchain is missing.

This task should not add new Cargo dependencies. If tests need helper code,
prefer local test helper functions following the existing workspace-manifest
test style.

## References

- Kani releases: <https://github.com/model-checking/kani/releases>
- Verus releases: <https://github.com/verus-lang/verus/releases>
- Netsuke Kani installer:
  <https://raw.githubusercontent.com/leynos/netsuke/main/scripts/install-kani.sh>
- Netsuke Kani version checker:
  <https://raw.githubusercontent.com/leynos/netsuke/main/scripts/check-kani-version.sh>
- Chutoro Verus installer:
  <https://raw.githubusercontent.com/leynos/chutoro/main/scripts/install-verus.sh>
- Chutoro Verus runner:
  <https://raw.githubusercontent.com/leynos/chutoro/main/scripts/run-verus.sh>

Revision note: initial draft created from roadmap item 15.1.3, Wireframe's
formal-verification design document, Wyvern planning reconnaissance, Firecrawl
research, and current GitHub release metadata. This establishes a
pre-implementation approval gate and does not authorize implementation until
the plan is approved.
