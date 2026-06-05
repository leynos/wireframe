# Adopt rust-prover-tools for Kani and Verus tooling

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
 `Tolerances`, `Risks`, `Progress`, `Surprises & discoveries`, `Decision log`,
and `Outcomes & retrospective` must be kept up to date as work proceeds.

Status: COMPLETE

The user approved implementation on 2026-05-26 by asking Codex to proceed with
the planned functionality. Roadmap item 15.1.3 remains unchecked until the
implementation, validation, and CodeRabbit review gates complete.

## Purpose / big picture

Roadmap item 15.1.3 establishes reproducible local installation for the formal
verification tools that later Kani, Verus, Stateright, Makefile, and continuous
integration (CI) tasks depend on. The implementation must now use
`rust-prover-tools` from <https://github.com/leynos/rust-prover-tools> instead
of reimplementing Kani and Verus workflows in bespoke repository shell scripts.

After the approved implementation, a contributor can run concise Makefile
targets from the repository root and obtain the repository-pinned versions of
Kani and Verus:

```sh
make install-kani
make install-verus
make check-kani-version
make run-verus
```

Those targets delegate to a pinned `prover-tools` command and contain no Kani
or Verus install logic of their own. Direct use of the CLI is also supported:

```sh
prover-tools kani install --repo-root .
prover-tools kani check-version --repo-root .
prover-tools verus install --repo-root .
prover-tools verus run --repo-root . --proof-file verus/wireframe_proofs.rs
```

`prover-tools kani install` reads `tools/kani/VERSION`, validates
`MAJOR.MINOR.PATCH`, installs `kani-verifier` with `cargo install --locked`,
runs `cargo kani setup`, and confirms that `cargo kani --version` is callable.
`prover-tools verus install` reads `tools/verus/VERSION` and
`tools/verus/SHA256SUMS`, downloads the matching Verus release archive,
verifies its SHA-256 checksum, and stages a local Verus binary.
`prover-tools verus run` resolves the Verus binary, handles the Rust toolchain
reported by `verus --version`, and runs the chosen proof entry point once proof
files exist.

Observable success is:

- `make install-kani` installs the version named in `tools/kani/VERSION`
  through `prover-tools kani install`.
- `make install-verus` installs the version named in `tools/verus/VERSION`
  through `prover-tools verus install` only after the downloaded archive matches
  `tools/verus/SHA256SUMS`.
- `make check-kani-version` verifies that the installed Kani binary matches
  `tools/kani/VERSION`.
- `make run-verus` delegates to
  `prover-tools verus run --proof-file verus/wireframe_proofs.rs`, fails
  clearly when no proof file exists, and succeeds once that proof entry point
  is added by a later roadmap item.
- Unit tests using `rstest` verify the metadata and Makefile target contracts
  without installing external tools.
- Behavioural tests using `rstest-bdd` describe the contributor workflow for
  pinned Kani and Verus tooling.
- `docs/developers-guide.md` explains the local formal-verification tooling
  convention, while `docs/formal-verification-methods-in-wireframe.md` stays
  aligned with the implemented paths.
- `docs/roadmap.md` marks 15.1.3 done only after the implementation and gates
  pass.

## Constraints

- Do not begin implementation until this ExecPlan is explicitly approved.
- Keep 15.1.3 scoped to tool metadata and concise Makefile targets that invoke
  the pinned `rust-prover-tools` entry point. Do not add Kani harness, Verus
  proof, or CI-oriented formal verification targets owned by 15.1.4 and 15.1.5.
- Do not copy Netsuke or Chutoro Kani/Verus shell-script internals into this
  repository. The source of installer and runner behaviour is
  `rust-prover-tools`.
- Prefer Makefile targets over repository shell wrappers. If shell wrappers are
  later required for backwards compatibility, they must be thin delegates to
  the same Makefile targets or pinned `prover-tools` invocation.
- Keep Verus out of the main Cargo build and test flow. It is a standalone
  proof runner, not a Cargo dependency or normal test target.
- Keep Kani harnesses, Verus proofs, and Stateright model expansion out of
  this item. Later roadmap items own those proof obligations.
- Pin tool versions in repository metadata files, not inside Make recipes.
- Pin the `rust-prover-tools` source revision so contributor and CI behaviour
  does not float with the upstream `main` branch.
- Verify external Verus downloads by SHA-256 before extracting them.
- Make the Makefile targets idempotent and safe to rerun. If a pinned Verus
  binary is already installed, `prover-tools verus install` should report that
  and exit successfully.
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
- If the implementation requires a new Rust crate dependency, stop and ask for
  approval.
- If adopting `rust-prover-tools` requires vendoring its package code into this
  repository, stop and ask for approval. The expected path is a pinned external
  tool invocation, not vendored tool code.
- If a public Wireframe API must change, stop and ask for approval.
- If no immutable `rust-prover-tools` release or tag is available, pin a commit
  SHA and record the commit in this plan. If a release or tag becomes available
  before implementation, prefer that stable ref.
- If the chosen Verus release does not publish an `x86-linux` archive, stop and
  choose a supported target strategy with human review.
- If checksum generation requires downloading more than the selected release
  archive for the current Linux target, stop and confirm whether multi-platform
  checksum coverage is required in this task.
- If `make install-kani` or `make install-verus` still fails after three
  implementation attempts for reasons other than host prerequisites, stop and
  record the blocker.
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
  `make test`. Severity: medium. Likelihood: high. Mitigation: test metadata
  and Makefile target contracts in the normal suite, but keep actual Kani and
  Verus installation as explicit validation steps and future formal targets.

- Risk: `rust-prover-tools` currently has no GitHub release or tag. Severity:
  medium. Likelihood: high. Mitigation: pin the upstream commit SHA, record it
  in `tools/rust-prover-tools/REF`, and revisit the pin once upstream publishes
  an immutable release tag.

- Risk: `rust-prover-tools` requires Python 3.14 or newer. Severity: medium.
  Likelihood: medium. Mitigation: use
  `uv tool run --python 3.14 --from <pinned-source> prover-tools ...` so `uv`
  manages the interpreter and tool environment consistently.

- Risk: Make recipes can drift into bespoke workflow logic. Severity: medium.
  Likelihood: medium. Mitigation: keep recipes limited to computing the pinned
  `rust-prover-tools` source and invoking `uv tool run ... prover-tools`; test
  that they reference `prover-tools` rather than duplicating Kani or Verus
  command sequences.

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
- [x] (2026-05-25 00:00 UTC) Updated the draft direction to use
      `rust-prover-tools` instead of bespoke Kani and Verus shell-script
      implementations.
- [x] (2026-05-25 01:38 UTC) Validated the revised plan with targeted Markdown
      lint, `make check-fmt`, `make markdownlint`, `make nixie`, `make lint`,
      and `make test`.
- [x] (2026-05-25 01:44 UTC) Ran `coderabbit review --agent` on the revised
      plan; it reported zero findings.
- [x] (2026-05-26 18:13 UTC) Revised the draft to prefer concise Makefile
      targets that call `prover-tools` over shell wrappers or direct long-form
      CLI commands.
- [x] (2026-05-26 18:20 UTC) Validated the Makefile-target plan revision with
      `make markdownlint`, `make nixie`, `make check-fmt`, and
      `coderabbit review --agent`; CodeRabbit reported zero findings.
- [x] (2026-05-26 21:23 UTC) Treated the user's implementation request as
      explicit approval, changed this ExecPlan to `IN PROGRESS`, and began
      implementation.
- [x] (2026-05-26 21:26 UTC) Re-checked upstream metadata and selected Kani
      `0.67.0`, Verus `0.2026.05.24.ecee80a`, and `rust-prover-tools` commit
      `b07ef696f8373d54ae68e517d39d47a5d27a5bd5`.
- [x] (2026-05-26 22:02 UTC) Added tool metadata pins and Makefile targets for
      `install-kani`, `check-kani-version`, `install-verus`, and `run-verus`.
- [x] (2026-05-26 22:29 UTC) Cleared CodeRabbit's Stage C findings by adding
      checksum provenance and structured `rust-prover-tools` pin metadata;
      re-review reported zero findings.
- [x] (2026-05-26 22:38 UTC) Added network-free `rstest` and `rstest-bdd`
      coverage for formal-tooling metadata and Makefile target contracts.
- [x] (2026-05-26 22:39 UTC) Validated the new targeted tests with
      `cargo test --test formal_tooling --all-features` and
      `cargo test --test bdd --all-features formal_tooling`.
- [x] (2026-05-26 22:51 UTC) Fixed Clippy feedback in the new tests by
      returning explicit errors instead of panicking inside `Result` tests.
- [x] (2026-05-26 22:55 UTC) Validated the test milestone with
      `make check-fmt`, `make lint`, `make test`, `make markdownlint`, and
      `make nixie`.
- [x] (2026-05-26 23:05 UTC) Ran `coderabbit review --agent` for the test
      milestone; it reported zero findings.
- [x] (2026-05-26 23:11 UTC) Updated the formal-verification design document
      and developers' guide to describe the implemented Makefile targets and
      pinned `rust-prover-tools` delegation model.
- [x] (2026-05-26 23:23 UTC) Ran `coderabbit review --agent` for the
      documentation milestone; it reported zero findings.
- [x] (2026-05-26 23:27 UTC) Ran `make install-kani`,
      `make check-kani-version`, and `make install-verus`; all succeeded
      through the pinned `prover-tools` entry point.
- [x] (2026-05-26 23:28 UTC) Ran `make run-verus`; it failed as expected with
      `Verus proof file not found: verus/wireframe_proofs.rs`, confirming the
      missing-proof diagnostic for later roadmap work.
- [x] (2026-05-26 23:31 UTC) Marked roadmap item 15.1.3 done and updated its
      success criteria to name the implemented Make targets.
- [x] (2026-05-26 23:35 UTC) Excluded `.verus/**` from Markdown lint ignores
      after the local Verus install introduced upstream Markdown files under
      `.verus/`.
- [x] (2026-05-26 23:39 UTC) Re-ran final validation after roadmap completion:
      `make check-fmt`, `make lint`, `make test`, `make markdownlint`, and
      `make nixie` all passed.
- [x] (2026-05-26 23:50 UTC) Ran final `coderabbit review --agent`; it
      reported zero findings.
- [x] Implement the approved plan in small gated commits.
- [x] Mark roadmap item 15.1.3 done after implementation validation passes.

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
  scripts. Impact: 15.1.3 must create metadata directories and add Makefile
  entry points.

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

- Observation: `rust-prover-tools` provides a Python package console script
  named `prover-tools`, not shell scripts. Evidence: its `pyproject.toml`
  declares `prover-tools = "rust_prover_tools.cli:main"`, and its users' guide
  documents `prover-tools kani install`, `prover-tools kani check-version`,
  `prover-tools verus install`, and `prover-tools verus run`. Impact: Wireframe
  should call that CLI through concise Makefile targets.

- Observation: `rust-prover-tools` has no GitHub release or tag at the time of
  this plan update, and its `main` branch was at commit
  `b07ef696f8373d54ae68e517d39d47a5d27a5bd5` on 2026-05-24. Evidence:
  `gh release list`, `gh api repos/leynos/rust-prover-tools/tags`, and
  `gh api repos/leynos/rust-prover-tools/commits/main`. Impact: pin that commit
  until upstream publishes a release tag.

- Observation: GitHub's GraphQL release listing was rate-limited during
  implementation, but REST API calls still returned the needed release data.
  Evidence: `gh release list` returned `GraphQL: API rate limit already
  exceeded`, while `gh api repos/.../releases?per_page=8` returned Kani and
  Verus release metadata. Impact: implementation used REST results to refresh
  pins.

- Observation: the latest non-prerelease Verus release available during
  implementation was `release/0.2026.05.24.ecee80a`, published on
  2026-05-25, with an `x86-linux` archive. Evidence:
  `gh api repos/verus-lang/verus/releases?per_page=8`. Impact: pin Verus to
  `0.2026.05.24.ecee80a` rather than the older planning candidate.

- Observation: the `verus-0.2026.05.24.ecee80a-x86-linux.zip` archive has
  SHA-256 checksum
  `323a44c0d787ce9a788665e1c6922360c44a72d1b9696359ec4f7bf5fbbc63e6`.
  Evidence: `sha256sum /tmp/verus-0.2026.05.24.ecee80a-x86-linux.zip`.
  Impact: record that checksum in `tools/verus/SHA256SUMS`.

- Observation: CodeRabbit flagged that the Verus checksum needs a visible
  repository audit trail tying it to the official artifact. Evidence:
  `coderabbit review --agent` reported one critical finding against
  `tools/verus/SHA256SUMS`. Impact: add `tools/verus/README.md` with the
  official release URL and checksum reproduction command before committing the
  metadata milestone.

- Observation: CodeRabbit's re-review found that a bare
  `tools/rust-prover-tools/REF` commit SHA lacks enough repository context for
  reviewers. Evidence: `coderabbit review --agent` reported a major finding
  against `tools/rust-prover-tools/REF`. Impact: make the file structured with
  `repository:`, `branch:`, `ref:`, and `verify:` lines, and teach the
  Makefile to extract the `ref:` value.

- Observation: repo-local Verus installs contain upstream Markdown under
  `.verus/`, and `make markdownlint` scans that directory unless it is
  explicitly ignored. Evidence: after `make install-verus`,
  `make markdownlint` reported violations in
  `.verus/0.2026.05.24.ecee80a/verus/vstd/*.md`. Impact: add `.verus/**` to
  `.markdownlint-cli2.jsonc` and keep `.verus/` in `.gitignore`.

## Decision log

- Decision: keep this pull request plan-only and leave 15.1.3 unchecked.
  Rationale: the user explicitly required plan approval before implementation.
  Date/Author: 2026-05-20 / Codex.

- Decision: use Netsuke's Kani installer as prior art for the behaviour
  expected behind `make install-kani` and `make check-kani-version`. Rationale:
  it already validates `tools/kani/VERSION`, enforces `MAJOR.MINOR.PATCH`, uses
  `cargo install --locked`, runs `cargo kani setup`, and confirms the installed
  command is callable. Date/Author: 2026-05-20 / Codex.

- Decision: use Chutoro's Verus installer and runner as prior art for the
  behaviour expected behind `make install-verus` and `make run-verus`.
  Rationale: Wireframe's formal verification design document already cites
  Chutoro for Verus tooling, and Chutoro's scripts implement version files,
  checksums, binary resolution, and toolchain bootstrap. Date/Author:
  2026-05-20 / Codex.

- Decision: add tests for metadata and Makefile target contracts, not full
  external tool installation, to the normal test suite. Rationale: Kani and
  Verus installation belongs to explicit formal-verification validation, while
  normal `make test` should stay deterministic and avoid network-dependent tool
  installs. Date/Author: 2026-05-20 / Codex.

- Decision: use `coderabbit review --agent` as a milestone gate after the
  Makefile/metadata implementation and again before the final implementation
  commit. Rationale: the user requested CodeRabbit validation after each major
  milestone, and tooling entry points benefit from an independent review pass.
  Date/Author: 2026-05-20 / Codex.

- Decision: replace bespoke shell-script implementation with a pinned
  `rust-prover-tools` CLI invocation. Rationale: the user requested
  `rust-prover-tools` in place of bespoke scripts, and that package preserves
  the required Kani and Verus behaviours behind a tested noun/verb CLI.
  Date/Author: 2026-05-25 / Codex.

- Decision: expose the local prover workflows as concise Makefile targets.
  Rationale: the user prefers Makefile targets that call the `prover-tools`
  entry point over shell wrappers or direct long-form `uv tool run` commands.
  Date/Author: 2026-05-26 / Codex.

- Decision: pin Verus to the latest non-prerelease release available during
  implementation, `0.2026.05.24.ecee80a`, rather than the rolling prerelease.
  Rationale: the plan prefers stable Verus releases unless a specific reason
  exists to adopt rolling prerelease builds, and no such reason exists for
  15.1.3. Date/Author: 2026-05-26 / Codex.

- Decision: store the `rust-prover-tools` pin as structured metadata rather
  than a bare SHA. Rationale: the Makefile still consumes the immutable commit
  ref, while reviewers can see the repository, branch, and verification command
  without consulting planning history. Date/Author: 2026-05-26 / Codex.

## Outcomes & retrospective

Roadmap item 15.1.3 landed as repository-local formal-verification tool
metadata plus thin Makefile entry points backed by the pinned
`rust-prover-tools` CLI. The implementation added:

- `tools/kani/VERSION` pinned to Kani `0.67.0`.
- `tools/verus/VERSION`, `tools/verus/SHA256SUMS`, and
  `tools/verus/README.md` pinned to Verus `0.2026.05.24.ecee80a` for
  `x86-linux`.
- `tools/rust-prover-tools/REF` with repository, branch, commit ref, and
  verification command metadata for
  `b07ef696f8373d54ae68e517d39d47a5d27a5bd5`.
- `make install-kani`, `make check-kani-version`, `make install-verus`, and
  `make run-verus`.
- Network-free `rstest` and `rstest-bdd` coverage for metadata and Makefile
  target contracts.
- Developer-facing documentation for the formal-tooling convention.

Validation passed with `make check-fmt`, `make lint`, `make test`,
`make markdownlint`, and `make nixie`. Contributor workflow smoke checks passed
for `make install-kani`, `make check-kani-version`, and `make install-verus`.
`make run-verus` failed with the expected missing proof file diagnostic:
`Verus proof file not found: verus/wireframe_proofs.rs`. The repo-local
`.verus/` install path is ignored by Git and Markdown lint to prevent local
tool installs from polluting validation.

CodeRabbit review ran after the Makefile/metadata, test, documentation, and
final validation milestones. The first Makefile/metadata review raised
checksum provenance and pin-context concerns; both were resolved. Later
milestone reviews, including the final review, reported zero findings.

15.1.4 and later formal-verification work still own the actual `make kani`,
`make kani-full`, `make verus`, CI formal jobs, Kani harnesses, and Verus proof
files.

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
and Verus metadata plus local installation and runner entry points that later
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
- `rust-prover-tools` is a Python package that exposes the `prover-tools`
  console command. It owns the Kani and Verus install, version-check, checksum,
  binary-resolution, toolchain, and proof-run behaviour that this repository
  should reuse.

Relevant local files and directories:

- `docs/roadmap.md` owns roadmap item 15.1.3 and must be marked done only after
  implementation completes.
- `docs/formal-verification-methods-in-wireframe.md` already documents the
  preferred formal-verification tooling layout and should be aligned with the
  Makefile target interface.
- `docs/developers-guide.md` should explain how contributors install and use
  the pinned formal-verification tools.
- `docs/users-guide.md` should not need a behaviour update unless the
  implementation changes public library behaviour. This task is expected to be
  internal tooling only.
- `Makefile` currently contains the repository's local validation targets and
  should gain the formal-verification tool entry points.
- `tools/` does not exist yet and will be created for tool pins.
- `tools/rust-prover-tools/REF` should pin the upstream tool revision used by
  `uv tool run`.
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
- Use current GitHub metadata when re-checking Kani, Verus, and
  `rust-prover-tools` versions before pinning.
- Use `commit-message` for file-based commits and `pr-creation` for the draft
  pull request.

## Plan of work

Stage A is the approval gate. Keep this ExecPlan as `Status: DRAFT` until the
user explicitly approves it. Do not create `tools/`, Makefile targets, test
files, or roadmap updates before approval.

Stage B creates the metadata pins. Re-check the latest stable Kani and Verus
releases from official GitHub release metadata. Prefer the latest stable Kani
release that supports `cargo install --locked kani-verifier --version`. Prefer
the latest non-prerelease Verus release unless a documented reason exists to
pin a rolling prerelease. Re-check `leynos/rust-prover-tools` for releases or
tags; if none exist, pin the current commit SHA. Create `tools/kani/VERSION`
with the Kani `MAJOR.MINOR.PATCH` value. Create `tools/verus/VERSION` with the
Verus release token without the `release/` prefix. Create
`tools/verus/SHA256SUMS` with at least the checksum for the Linux archive used
by the installer, `verus-${VERSION}-x86-linux.zip`. Create
`tools/rust-prover-tools/REF` with the selected upstream release tag or commit
SHA. If multi-platform archive checksums are added now, document that decision
in this plan.

Stage C creates the `rust-prover-tools` invocation layer in `Makefile`. Add
concise `.PHONY` targets named `install-kani`, `check-kani-version`,
`install-verus`, and `run-verus`. Each target reads
`tools/rust-prover-tools/REF`, constructs the pinned Git source, and invokes
`uv tool run --python 3.14 --from "$${source}" prover-tools ...`. The recipes
should be short enough to audit directly and should not call prover-specific
subcommands themselves except through `prover-tools`. For example, the Kani
installer target should be equivalent to:

```sh
uv tool run --python 3.14 \
  --from "git+https://github.com/leynos/rust-prover-tools.git@$(cat tools/rust-prover-tools/REF)" \
  prover-tools kani install --repo-root .
```

`install-verus` invokes `prover-tools verus install --repo-root .` while
preserving `VERUS_TARGET` and `VERUS_INSTALL_DIR` through the environment.
`run-verus` invokes
`prover-tools verus run --repo-root . --proof-file "$${VERUS_PROOF_FILE:-verus/wireframe_proofs.rs}"`
 while preserving `VERUS_BIN`, `VERUS_TARGET`, and `VERUS_INSTALL_DIR` through
the environment. The recipes must not run `cargo install`, `cargo kani setup`,
`curl`, `unzip`, `sha256sum`, `shasum`, or `rustup` directly; `prover-tools`
owns those steps. Do not add shell wrappers unless a later approved revision
explicitly asks for them.

Run `coderabbit review --agent` after Stage C. If it reports concerns, resolve
them before adding tests.

Stage D adds tests. Add `rstest` coverage for repository tooling contracts:
required metadata files exist, Kani's pin is non-empty and matches
`MAJOR.MINOR.PATCH`, Verus' pin is non-empty, `tools/verus/SHA256SUMS` contains
an entry for the configured Linux archive, and `tools/rust-prover-tools/REF` is
non-empty. Add coverage that the `Makefile` declares the four `.PHONY` local
targets, each target references `prover-tools`, and the recipes do not contain
the old implementation commands such as `cargo install`, `cargo kani setup`,
`curl`, `unzip`, `sha256sum`, `shasum`, or `rustup toolchain install`. Add a
`rstest-bdd` feature and scenario describing the contributor workflow: the
repository declares pinned Kani, Verus, and `rust-prover-tools` metadata, the
Makefile entry points are present, and the metadata is consistent. Keep these
tests network-free.

Stage E updates internal documentation. Update
`docs/formal-verification-methods-in-wireframe.md` where the implemented
Makefile target contracts differ from the current design text. Update
`docs/developers-guide.md` with the contributor-facing convention for invoking
the pinned `prover-tools` CLI through the Makefile targets. Do not update
`docs/users-guide.md` unless public library behaviour changes, which is not
expected.

Stage F validates the implementation. Run all commands sequentially with `tee`
logs under `/tmp`, using the current branch name in each log file. The required
implementation gates are `make check-fmt`, `make lint`, and `make test`. Also
run `make markdownlint` and `make nixie` if documentation changed. Run
`make install-kani`, `make check-kani-version`, and `make install-verus` to
prove the local contributor workflow. If `make run-verus` is expected to fail
because no proof file exists yet, capture and document the clear "proof file
not found" diagnostic rather than treating that as a failed 15.1.3 acceptance
criterion.

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
gh release list --repo leynos/rust-prover-tools --limit 10
gh api repos/leynos/rust-prover-tools/tags
gh api repos/leynos/rust-prover-tools/commits/main --jq .sha
```

Create metadata and Makefile targets after approval:

```sh
mkdir -p tools/kani tools/verus tools/rust-prover-tools
$EDITOR tools/kani/VERSION
$EDITOR tools/verus/VERSION
$EDITOR tools/verus/SHA256SUMS
$EDITOR tools/rust-prover-tools/REF
$EDITOR Makefile
```

Use `apply_patch` rather than editor commands when this plan is implemented by
an agent.

Run installation smoke checks through the Makefile targets:

```sh
branch="$(git branch --show-current)"
make install-kani 2>&1 |
  tee "/tmp/install-kani-wireframe-${branch}.out"
make check-kani-version 2>&1 |
  tee "/tmp/check-kani-version-wireframe-${branch}.out"
VERUS_INSTALL_DIR="${PWD}/.verus/$(tr -d '[:space:]' < tools/verus/VERSION)" \
  make install-verus 2>&1 |
  tee "/tmp/install-verus-wireframe-${branch}.out"
```

Run the Verus proof runner smoke check. Until later roadmap work adds the proof
file, the expected result is a clear diagnostic naming the missing proof file:

```sh
branch="$(git branch --show-current)"
make run-verus 2>&1 | tee "/tmp/run-verus-wireframe-${branch}.out"
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
Adopt rust-prover-tools for formal tooling

Add pinned Kani, Verus, and rust-prover-tools metadata plus local entry
points so contributors can install the supported formal-verification
toolchain reproducibly without bespoke prover shell logic.
ENDOFMSG
git commit -F "${COMMIT_MSG_DIR}/COMMIT_MSG.md"
rm -rf "${COMMIT_MSG_DIR}"
```

## Validation and acceptance

The approved implementation is accepted only when these checks pass or their
documented baseline failures are explicitly approved:

- `make install-kani`
- `make check-kani-version`
- `make install-verus`
- `make run-verus`, accepting a clear missing-proof-file diagnostic until a
  later roadmap item adds `verus/wireframe_proofs.rs`
- `make check-fmt`
- `make lint`
- `make test`
- `make markdownlint`, if documentation changed
- `make nixie`, if Markdown diagrams are present or changed
- `coderabbit review --agent` with all concerns resolved

Expected test coverage:

- New `rstest` unit/integration tests fail before the metadata and
  `rust-prover-tools` entry points exist and pass after implementation.
- New `rstest-bdd` scenario coverage fails before the repository declares the
  formal-verification tooling workflow and passes after implementation.
- No Kani harness or Verus proof is required for 15.1.3 because this item
  installs tool plumbing rather than proving protocol properties.

Expected contributor behaviour after implementation:

```sh
make install-kani
make check-kani-version
make install-verus
```

The commands should exit 0 on a host with the required external prerequisites
by delegating to the pinned `rust-prover-tools` CLI. `make run-verus` should
either run the selected proof file or fail with a clear diagnostic naming the
missing proof file until later Verus proof work adds it.

## Idempotence and recovery

The entry points must be safe to rerun. `prover-tools kani install` may
reinstall the pinned Cargo binary because `cargo install --locked --version` is
the supported installer path. `prover-tools verus install` should skip
downloading when the pinned binary already exists at the requested install
directory.

If a Verus download fails before extraction, rerun the same command. Temporary
download directories must be cleaned with a shell `trap`.

If a Verus checksum fails, delete the temporary archive, keep the existing
installed binary untouched, and fail with expected and actual SHA-256 values.

If the implementation needs rollback before commit, remove only files created
by this roadmap item:

```sh
rm -rf tools/kani tools/verus tools/rust-prover-tools
```

Do not revert unrelated user or agent changes.

## Artifacts and notes

Planning sources checked:

- `docs/roadmap.md` item 15.1.3 originally defines script-based success
  criteria; this plan revises the contributor interface to concise Makefile
  targets that invoke `prover-tools`.
- `docs/formal-verification-methods-in-wireframe.md` defines the preferred
  formal-verification layout and keeps Verus outside the main build.
- `rust-prover-tools` provides the `prover-tools` CLI that replaces the
  bespoke shell-script implementation for Kani installation, Kani version
  checks, Verus installation, and Verus proof execution.
- Kani GitHub release metadata listed `kani-0.67.0` as the latest release at
  planning time.
- Verus GitHub release metadata listed `release/0.2026.05.17.e479cce` as the
  latest non-prerelease release at planning time.
- `rust-prover-tools` had no release or tag on 2026-05-25; the observed
  `main` commit was `b07ef696f8373d54ae68e517d39d47a5d27a5bd5`.

The plan-only pull request should mention this file and should not claim that
formal-verification tooling has been implemented yet.

## Interfaces and dependencies

The implementation should create these repository-local interfaces:

```plaintext
tools/kani/VERSION
tools/verus/VERSION
tools/verus/SHA256SUMS
tools/rust-prover-tools/REF
Makefile target: install-kani
Makefile target: check-kani-version
Makefile target: install-verus
Makefile target: run-verus
```

`tools/rust-prover-tools/REF` contains an immutable upstream release tag or
commit SHA. Until upstream publishes a release tag, use
`b07ef696f8373d54ae68e517d39d47a5d27a5bd5`.

The implementation depends on `uv` and Python 3.14 because `rust-prover-tools`
declares `requires-python = ">=3.14"`.

`make install-kani` accepts no required arguments. It depends on `uv`, the
pinned `tools/rust-prover-tools/REF`, and the `tools/kani/VERSION` file read by
`prover-tools kani install`.

`make check-kani-version` accepts no required arguments. It uses the same pinned
`rust-prover-tools` source and the `tools/kani/VERSION` file read by
`prover-tools kani check-version`.

`make install-verus` accepts these environment variables and passes them
through to `prover-tools verus install`:

- `VERUS_TARGET`, defaulting to `x86-linux`.
- `VERUS_INSTALL_DIR`, defaulting to a repository-local directory for the
  pinned version.

The external `curl`, `unzip`, and checksum tool requirements belong to
`prover-tools verus install`, not to the Make recipe implementation.

`make run-verus` accepts these environment variables and passes them through to
`prover-tools verus run`:

- `VERUS_BIN`, which may be a binary path, install directory, or command name.
- `VERUS_INSTALL_DIR`, used to derive the default binary path.
- `VERUS_PROOF_FILE`, defaulting to `verus/wireframe_proofs.rs`.

The `rustup` requirement belongs to `prover-tools verus run` when the required
Verus Rust toolchain is missing.

This task should not add new Cargo dependencies. If tests need helper code,
prefer local test helper functions following the existing workspace-manifest
test style.

## References

- Kani releases: <https://github.com/model-checking/kani/releases>
- Verus releases: <https://github.com/verus-lang/verus/releases>
- rust-prover-tools repository: <https://github.com/leynos/rust-prover-tools>
- rust-prover-tools users' guide:
  <https://github.com/leynos/rust-prover-tools/blob/main/docs/users-guide.md>
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

Revision note: updated on 2026-05-25 to replace bespoke Kani and Verus
shell-script implementation with a pinned `rust-prover-tools` CLI invocation.
Updated again on 2026-05-26 to make concise Makefile targets the preferred
local contributor interface for invoking `prover-tools`.

Revision note: updated on 2026-06-05 after review feedback. The implementation
now has a dedicated regression test asserting that `make run-verus` passes
`--proof-file "$(VERUS_PROOF_FILE)"`, the formal-tooling behavioural fixture no
longer carries an artificial lint workaround, and `AGENTS.md` now describes
Cargo dependency policy as implicit SemVer-compatible requirements rather than
conflicting explicit version ranges.
