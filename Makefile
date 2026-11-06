.PHONY: help all clean test test-doc test-documentation-contracts test-wireframe-testing test-ui test-feature-gate test-pool-no-metrics unsafe-boundary bless-ui test-workflow-contracts test-loom doctest-benchmark
.PHONY: lint-python-docs
.PHONY: bench-codec bench-runtime-ownership bench build dev-build dev-test release lint fmt check-fmt markdownlint nixie typecheck
.PHONY: spelling
# Kept off the build/test line above so that unrelated edits to that long line
# cannot collide with the lint targets during a stack restack.
.PHONY: lint-doc lint-clippy lint-whitaker
.PHONY: install-whitaker
.PHONY: install-kani check-kani-version install-verus run-verus test-verification kani \
	kani-full verus formal-pr formal-nightly formal

CRATE ?= wireframe
CARGO ?= cargo
BUILD_JOBS ?=
DEV_FAST_CONFIG := tools/dev-fast/config.toml
DEV_FAST := --config $(DEV_FAST_CONFIG)
# `mold` is a native Linux linker, so select it only when both the host and
# effective Cargo target are Linux. Make carries the flag in RUSTFLAGS rather
# than leaving it to the dev-fast fragment because Cargo compares a
# `[target.<cfg>]` table against the compilation target, not the host; only
# Make can tell whether the machine doing the build actually has `mold`.
# `host-tuple` names the host's own triple, so it takes the host's OS. Standard
# Rust target triples encode Linux as `-unknown-linux-`; callers using
# a custom or non-standard target, or a `build.target` set in Cargo's own
# configuration (which Make cannot read), can name the target's OS with
# CARGO_BUILD_TARGET_OS, which wins whether or not CARGO_BUILD_TARGET is set.
CARGO_BUILD_TARGET_OS ?=
# The machine doing the build; overridable so a contract can read the recipes a
# non-Linux host would run.
BUILD_HOST_OS ?= $(shell uname -s)
DEV_EFFECTIVE_TARGET_OS = $(or $(CARGO_BUILD_TARGET_OS),$(if $(filter-out host-tuple,$(CARGO_BUILD_TARGET)),$(if $(findstring -unknown-linux-,$(CARGO_BUILD_TARGET)),Linux),$(BUILD_HOST_OS)))
DEV_LINUX_LINK_ARG := $(if $(filter Linux,$(BUILD_HOST_OS)),$(if $(filter Linux,$(DEV_EFFECTIVE_TARGET_OS)),-Clink-arg=-fuse-ld=mold))
# The build standard's parallel frontend. `.cargo/config.toml` carries it and
# `mold` in its `rustflags` sources, but an assigned `RUSTFLAGS` replaces those
# sources, so every development recipe below restates both.
DEV_THREADS_FLAG ?= -Zthreads=8
DEV_STANDARD_FLAGS = $(DEV_THREADS_FLAG) $(DEV_LINUX_LINK_ARG)
DEV_WARNING_FLAGS = $(strip $(RUSTFLAGS) -D warnings $(DEV_STANDARD_FLAGS))
DEV_BUILD_FLAGS = $(strip $(RUSTFLAGS) $(DEV_STANDARD_FLAGS))
# Release builds take neither flag: assigning `RUSTFLAGS`, even to an empty
# inherited value, displaces every `rustflags` source in the configuration.
RELEASE_BUILD_ENV = RUSTFLAGS="$${RUSTFLAGS-}"
DEV_BUILD_ENV = RUSTFLAGS="$(DEV_BUILD_FLAGS)"
CLIPPY_FLAGS ?= --workspace --all-targets --all-features -- -D warnings
DOC_FLAGS ?= --workspace --all-features --no-deps
RUSTDOC_FLAGS ?= --cfg docsrs -D warnings
MDLINT ?= $(shell command -v markdownlint-cli2 2>/dev/null || printf '%s' "$$HOME/.bun/bin/markdownlint-cli2")
# `make fmt` and `make check-fmt` call mdtablefix directly. `--git` selects the
# Markdown files Git tracks and `--include-untracked` adds the untracked files
# Git does not ignore, so a new document is formatted before it is staged.
# Both modes need mdtablefix 0.6.0 or later; CI pins the version at the
# install-mdtablefix step.
MDTABLEFIX ?= mdtablefix
MDTABLEFIX_SELECT = --git --include-untracked
MDTABLEFIX_RULES = --wrap --renumber --breaks --ellipsis --fences
WHITAKER ?= whitaker
# Whitaker pin
#
# Every component of the lint suite is pinned here, in one place, so a reviewed
# bump is a single edit. `WHITAKER_SUITE_REV` is a full commit on
# `leynos/whitaker`'s default branch rather than a floating release tag: the
# rolling tag moves, and a moving suite would change this repository's lint
# results without a reviewed diff. A pinned suite is built from source, so the
# installer also needs the suite's own `rust-toolchain.toml` components, which
# it installs through rustup.
#
# `cargo-dylint` and `dylint-link` are pinned to exactly 6.3.0 because that is
# the Dylint release the pinned Whitaker suite builds against
# (`installer/dependency-binaries.toml`). A different major or minor can reject
# libraries compiled by the suite or fail to find its driver.
WHITAKER_INSTALLER ?= whitaker-installer
WHITAKER_INSTALLER_VERSION ?= 0.2.11
CARGO_DYLINT_VERSION ?= 6.3.0
DYLINT_LINK_VERSION ?= 6.3.0
WHITAKER_SUITE_REV ?= 00186d43f5121df92bcc207e48b3b2a4ca945937
# `--all` selects every discovered lint library. The arguments after `--` are
# forwarded to the `cargo check` that Whitaker drives, so they widen package,
# target and feature coverage there rather than being read as lint options.
WHITAKER_FLAGS ?= --all -- --workspace --all-targets --all-features
NIXIE_VERSION ?= 1.1.0
UV ?= uv
UV_ENV = UV_CACHE_DIR=.uv-cache UV_TOOL_DIR=.uv-tools

# The CV-005 CodeScene contracts live in shared-actions and run from a full
# commit, so a fix is a pin bump. `.github/cv005.toml` holds this repository's
# only parameters.
CV005_CONTRACTS_REF ?= f84a7def5646a6fc7ba77c1f486906f66a0fed96
CV005_CONTRACTS = $(UV_ENV) $(UV) tool run --python 3.13 \
	--from 'git+https://github.com/leynos/shared-actions@$(CV005_CONTRACTS_REF)\#subdirectory=packages/cv005-contracts' \
	cv005-contracts

NIXIE = $(UV_ENV) $(UV) tool run --python 3.14 \
	--from nixie-cli@$(NIXIE_VERSION) nixie
TYPOS_CONFIG_BUILDER_VERSION ?= v0.1.3
TYPOS_CONFIG_BUILDER = $(UV_ENV) $(UV) tool run --python 3.14 --from \
	"git+https://github.com/leynos/typos-config-builder.git@$(TYPOS_CONFIG_BUILDER_VERSION)" \
	typos-config-builder
PYTHON_NO_BYTECODE_ENV := PYTHONDONTWRITEBYTECODE=1
PROVER_TOOLS_REF_FILE ?= tools/rust-prover-tools/REF
PROVER_TOOLS_REF ?= $(shell awk '/^ref:/ { print $$2 }' $(PROVER_TOOLS_REF_FILE))
PROVER_TOOLS_SOURCE ?= git+https://github.com/leynos/rust-prover-tools.git@$(PROVER_TOOLS_REF)
PROVER_TOOLS ?= uv tool run --python 3.14 --from "$(PROVER_TOOLS_SOURCE)" prover-tools
VERUS_PROOF_FILE ?= verus/wireframe_proofs.rs
VERIFICATION_CRATE ?= wireframe-verification
# The Loom models live in their own package so a `--cfg loom` build never
# compiles the TCP test harness. The preemption bound keeps each model's
# exploration finite; raising it is a decision, not a tuning knob.
LOOM_CRATE ?= wireframe-loom
LOOM_MAX_PREEMPTIONS ?= 3
FORMAL_STUB ?= ./scripts/formal-stub.sh
FORMAL_STRICT ?=
export FORMAL_STRICT

build: target/debug/lib$(CRATE).rlib ## Build debug binary
dev-build: ## Build with the explicit development backend
	$(DEV_BUILD_ENV) $(CARGO) $(DEV_FAST) build $(BUILD_JOBS) --lib
dev-test: test ## Test with the explicit development backend
release: target/release/lib$(CRATE).rlib ## Build release binary

all: release ## Default target builds release binary

clean: ## Remove build artefacts
	$(CARGO) clean

test-bdd: ## Run rstest-bdd tests only
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test --test bdd --all-features $(BUILD_JOBS)

test: ## Run all tests (bdd + unit/integration)
	$(CARGO) fetch --locked --target "$${CARGO_BUILD_TARGET:-host-tuple}"
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test --workspace --all-targets --all-features $(BUILD_JOBS)

test-wireframe-testing: ## Run the wireframe_testing companion crate's tests
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test -p wireframe_testing --all-targets --all-features $(BUILD_JOBS)

test-workflow-contracts: ## Validate workflow invocation contracts
	$(CV005_CONTRACTS) check --repository .
	$(PYTHON_NO_BYTECODE_ENV) uv run --with 'pytest>=8' --with 'pyyaml>=6' --with 'hypothesis>=6' pytest tests/workflow_contracts -q

test-loom: ## Run Loom models and test-support API checks under --cfg loom
	LOOM_MAX_PREEMPTIONS=$(LOOM_MAX_PREEMPTIONS) RUSTFLAGS="--cfg loom" $(CARGO) test -p $(LOOM_CRATE) $(BUILD_JOBS)
	CARGO_ENCODED_RUSTFLAGS="--cfg=loom" $(CARGO) test -p wireframe_testing --test loom_api_surface --features runtime-benchmarks $(BUILD_JOBS)

test-doc: ## Run doctests across all features
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test --workspace --doc --all-features $(BUILD_JOBS)

# Examples are built first because the suite runs the documented example pair.
test-documentation-contracts: ## Validate documented examples, commands and links
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) build --examples --all-features $(BUILD_JOBS)
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test --test documentation_examples_loader_tests --test documentation_examples_tests --all-features $(BUILD_JOBS)

test-ui: ## Run trybuild compile-time API tests
	$(CARGO) test --test compile_error
	CARGO_ENCODED_RUSTFLAGS="--cfg=loom" $(CARGO) test -p wireframe_testing --test loom_api_surface --features runtime-benchmarks

# Trybuild forwards the harness features to its fixture, and this workspace's
# `wireframe_testing` dev-dependency enables `wireframe/test-support`, so no
# trybuild case can observe a feature-gated API as absent. A standalone consumer
# with its own lock file is the only way to check that direction.
test-feature-gate: ## Verify feature-gated APIs are actually absent when disabled
	./scripts/check-feature-gate.sh

test-pool-no-metrics: ## Check the pool builds without metrics
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) check --lib --no-default-features --features pool,serializer-bincode $(BUILD_JOBS)

# The contract test reads sources as text, so it can only show that the
# prohibition is present. These probes compile deliberate unsafe code and show
# that it is actually rejected, by both the crate attribute and the manifest
# table.
unsafe-boundary: ## Prove the unsafe_code prohibition rejects unsafe Rust
	./scripts/check-unsafe-boundary.sh

# Developer-only: review every changed snapshot; this target must not run in CI.
bless-ui: ## Refresh trybuild snapshots for deliberate fixture changes
	TRYBUILD=overwrite $(CARGO) test --test compile_error
	CARGO_ENCODED_RUSTFLAGS="--cfg=loom" TRYBUILD=overwrite $(CARGO) test -p wireframe_testing --test loom_api_surface --features runtime-benchmarks

doctest-benchmark: ## Check runnable/no_run doctest ratios
	./scripts/doctest-benchmark.sh

bench-codec: ## Run codec and pipeline performance benchmarks
	RUSTFLAGS="-D warnings" $(CARGO) bench --bench codec_performance --bench codec_performance_alloc --bench pipeline_performance --bench pipeline_performance_alloc --features test-support,benchmark-counters $(BUILD_JOBS)

bench-runtime-ownership: ## Run runtime-ownership baseline benchmarks
	RUSTFLAGS="-D warnings" $(CARGO) bench --bench runtime_ownership_baselines --bench runtime_ownership_baselines_alloc --features test-support,pool $(BUILD_JOBS)

bench: bench-codec bench-runtime-ownership ## Run all benchmark suites

typecheck: ## Run a workspace typecheck
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) check --workspace --all-targets --all-features $(BUILD_JOBS)

# will match target/debug/libmy_library.rlib and target/release/libmy_library.rlib
target/%/lib$(CRATE).rlib: ## Build library in debug or release
	$(if $(findstring release,$(@)),$(RELEASE_BUILD_ENV),$(DEV_BUILD_ENV)) $(CARGO) $(if $(findstring release,$(@)),,$(DEV_FAST)) build $(BUILD_JOBS) \
	  $(if $(findstring release,$(@)),--release)            \
	  --lib
	@# copy the .rlib into your own target tree
	install -Dm644                                           \
	  target/$(if $(findstring release,$(@)),release,debug)/lib$(CRATE).rlib \
	  $@

# `lint` is the one aggregate CI and contributor tooling invoke. Splitting it
# into named leaves keeps each stage runnable on its own while leaving the
# aggregate recipe-free, so the pipeline cannot drift between the two.
#
# Each leaf is one recipe line: a bare `make -j lint` would otherwise start the
# three linters together and interleave their diagnostics. Prerequisites of a
# single target run in parallel under `-j`, so ordering comes from
# `.NOTPARALLEL` and not from the order they are named.
.NOTPARALLEL: lint

lint: lint-doc lint-clippy lint-whitaker lint-python-docs ## Run rustdoc, Clippy, Whitaker and the Python docstring check

# The workflow contract tests are the repository's only Python; interrogate holds
# docstring presence for them. Ruff-style docstring wording is not enforced here.
lint-python-docs: ## Require a docstring on every workflow contract module, class and function
	$(PYTHON_NO_BYTECODE_ENV) uv run --with 'interrogate>=1.7,<2' interrogate --fail-under 100 tests/workflow_contracts

lint-doc: ## Build workspace rustdoc for every feature with warnings denied
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" RUSTDOCFLAGS="$(RUSTDOC_FLAGS)" $(CARGO) $(DEV_FAST) doc $(DOC_FLAGS)

lint-clippy: ## Run Clippy over the whole workspace with warnings denied
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) clippy $(CLIPPY_FLAGS)

# Whitaker keeps its own installer-managed toolchain: no `DEV_FAST` config and
# no Make-owned development linker flag reach this line. `WHITAKER_FLAGS`
# carries the workspace, all-target and all-feature selection after `--`, so
# they reach the `cargo check` Whitaker drives rather than being read as lint
# options.
lint-whitaker: ## Run Whitaker over every workspace member
	RUSTFLAGS="-D warnings" $(WHITAKER) $(WHITAKER_FLAGS)

# Installation stays explicit rather than a `lint` prerequisite: a contributor
# with an existing installation should not pay for it on every run, and CI
# installs once before its single `make lint` step. The suite revision is
# resolved outside any Whitaker checkout so the installer clones its own copy.
install-whitaker: ## Install the pinned Whitaker suite and Dylint tools
	$(CARGO) install --locked --version $(CARGO_DYLINT_VERSION) cargo-dylint
	$(CARGO) install --locked --version $(DYLINT_LINK_VERSION) dylint-link
	$(CARGO) install --locked --version $(WHITAKER_INSTALLER_VERSION) $(WHITAKER_INSTALLER)
	$(WHITAKER_INSTALLER) --suite-version $(WHITAKER_SUITE_REV)

fmt: ## Format Rust and Markdown sources
	$(CARGO) fmt --all
	$(MDTABLEFIX) --in-place $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)
	$(MDLINT) --fix "**/*.md"

check-fmt: ## Verify formatting
	$(CARGO) fmt --all -- --check
	$(MDTABLEFIX) --check $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)

markdownlint: spelling ## Lint Markdown and enforce en-GB-oxendict spelling
	$(MDLINT) "**/*.md"

spelling: ## Enforce en-GB-oxendict spelling
	$(TYPOS_CONFIG_BUILDER) gate --repository . --scope all

nixie: ## Validate Mermaid diagrams
	$(NIXIE) --no-sandbox

install-kani: ## Install the pinned Kani verifier
	$(PROVER_TOOLS) kani install --repo-root .

check-kani-version: ## Check the installed Kani verifier version
	$(PROVER_TOOLS) kani check-version --repo-root .

install-verus: ## Install the pinned Verus verifier
	$(PROVER_TOOLS) verus install --repo-root .

run-verus: ## Run the configured Verus proof entry point
	$(PROVER_TOOLS) verus run --repo-root . --proof-file "$(VERUS_PROOF_FILE)"

test-verification: ## Run the Stateright verification crate tests
	RUSTFLAGS="-D warnings" $(CARGO) test -p $(VERIFICATION_CRATE) $(BUILD_JOBS)

kani: ## Run Kani smoke harnesses (stub until roadmap 15.3.1)
	@$(FORMAL_STUB) kani "roadmap 15.3.1 adds src/frame Kani smoke harnesses"

kani-full: ## Run every Kani harness (stub until roadmap 15.3.x)
	@$(FORMAL_STUB) kani-full "roadmap 15.3.x adds the full Kani harness set"

verus: ## Run Verus proofs (stub until roadmap 15.5.2)
	@$(FORMAL_STUB) verus "roadmap 15.5.2 adds verus/wireframe_proofs.rs"

formal-pr: test-verification kani verus ## Fast pull-request formal gate

formal-nightly: test-verification kani-full verus ## Deeper scheduled formal gate

formal: formal-pr ## Default formal suite (alias for the PR gate)

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
