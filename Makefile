.PHONY: help all clean test test-doc test-workflow-contracts doctest-benchmark
.PHONY: bench-codec build dev-build dev-test release lint fmt check-fmt markdownlint nixie typecheck
.PHONY: spelling
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
# Standard Rust target triples encode Linux as `-unknown-linux-`; callers using
# a custom or non-standard target can set CARGO_BUILD_TARGET_OS=Linux
# explicitly.
CARGO_BUILD_TARGET_OS ?=
DEV_EFFECTIVE_TARGET_OS = $(if $(CARGO_BUILD_TARGET),$(or $(CARGO_BUILD_TARGET_OS),$(if $(findstring -unknown-linux-,$(CARGO_BUILD_TARGET)),Linux)),$(shell uname -s))
DEV_LINUX_LINK_ARG := $(if $(filter Linux,$(shell uname -s)),$(if $(filter Linux,$(DEV_EFFECTIVE_TARGET_OS)),-Clink-arg=-fuse-ld=mold))
DEV_WARNING_FLAGS = $(strip $(RUSTFLAGS) -D warnings $(DEV_LINUX_LINK_ARG))
DEV_BUILD_FLAGS = $(strip $(RUSTFLAGS) $(DEV_LINUX_LINK_ARG))
DEV_BUILD_ENV = RUSTFLAGS="$(DEV_BUILD_FLAGS)"
CLIPPY_FLAGS ?= --workspace --all-targets --all-features -- -D warnings
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
NIXIE_VERSION ?= 1.1.0
UV ?= uv
UV_ENV = UV_CACHE_DIR=.uv-cache UV_TOOL_DIR=.uv-tools
NIXIE = $(UV_ENV) $(UV) tool run --python 3.14 \
	--from nixie-cli@$(NIXIE_VERSION) nixie
TYPOS_CONFIG_BUILDER_VERSION ?= v0.1.1
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
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test --workspace --all-targets --all-features $(BUILD_JOBS)

test-workflow-contracts: ## Validate workflow invocation contracts
	$(PYTHON_NO_BYTECODE_ENV) uv run --with 'pytest>=8' --with 'pyyaml>=6' pytest tests/workflow_contracts -q

test-doc: ## Run doctests across all features
	# `wireframe_testing` doctests need generic app types that standalone snippets
	# cannot infer; issue #578 tracks their repair.
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) test --workspace --exclude wireframe_testing --doc --all-features $(BUILD_JOBS)

doctest-benchmark: ## Check runnable/no_run doctest ratios
	./scripts/doctest-benchmark.sh

bench-codec: ## Run codec performance benchmarks
	RUSTFLAGS="-D warnings" $(CARGO) bench --bench codec_performance --bench codec_performance_alloc --features test-support $(BUILD_JOBS)

typecheck: ## Run a workspace typecheck
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) check --workspace --all-targets --all-features $(BUILD_JOBS)

# will match target/debug/libmy_library.rlib and target/release/libmy_library.rlib
target/%/lib$(CRATE).rlib: ## Build library in debug or release
	$(if $(findstring release,$(@)),,$(DEV_BUILD_ENV)) $(CARGO) $(if $(findstring release,$(@)),,$(DEV_FAST)) build $(BUILD_JOBS) \
	  $(if $(findstring release,$(@)),--release)            \
	  --lib
	@# copy the .rlib into your own target tree
	install -Dm644                                           \
	  target/$(if $(findstring release,$(@)),release,debug)/lib$(CRATE).rlib \
	  $@

lint: ## Run Clippy with warnings denied
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" RUSTDOCFLAGS="$(RUSTDOC_FLAGS)" $(CARGO) $(DEV_FAST) doc --workspace --no-deps
	RUSTFLAGS="$(DEV_WARNING_FLAGS)" $(CARGO) $(DEV_FAST) clippy $(CLIPPY_FLAGS)
	RUSTFLAGS="-D warnings" $(WHITAKER) --all -- --all-targets --all-features

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
