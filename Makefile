.PHONY: help all clean test test-doc test-workflow-contracts doctest-benchmark
.PHONY: bench-codec build release lint fmt check-fmt markdownlint nixie typecheck
.PHONY: spelling spelling-phrase-check spelling-config spelling-config-write spelling-helper-test
.PHONY: install-kani check-kani-version install-verus run-verus

CRATE ?= wireframe
CARGO ?= cargo
BUILD_JOBS ?=
CLIPPY_FLAGS ?= --all-targets --all-features -- -D warnings
RUSTDOC_FLAGS ?= --cfg docsrs -D warnings
MDLINT ?= markdownlint-cli2
WHITAKER ?= whitaker
NIXIE_VERSION ?= 1.1.0
PATHSPEC_VERSION ?= 1.1.1
RUFF_VERSION ?= 0.15.12
TYPOS_VERSION ?= 1.48.0
UV ?= uv
UV_ENV = UV_CACHE_DIR=.uv-cache UV_TOOL_DIR=.uv-tools
NIXIE = $(UV_ENV) $(UV) tool run --python 3.14 \
	--from nixie-cli@$(NIXIE_VERSION) nixie
TYPOS_CONFIG_BUILDER_COMMIT := b604f198797fdd36a567dd0f8f07b13f9539b241
TYPOS_CONFIG_BUILDER_SOURCE := git+https://github.com/leynos/typos-config-builder.git@$(TYPOS_CONFIG_BUILDER_COMMIT)
TYPOS_CONFIG_BUILDER := $(UV_ENV) $(UV) tool run --python 3.14 \
	--from "$(TYPOS_CONFIG_BUILDER_SOURCE)" typos-config-builder
SPELLING_PY_SRCS := \
	scripts/typos_rollout_check.py scripts/tests/test_typos_rollout_check.py
SPELLING_PY_TESTS := scripts/tests/test_typos_rollout_check.py
SPELLING_COVERAGE_ARGS := --cov=typos_rollout_check --cov-fail-under=90
PYTHON_NO_BYTECODE_ENV := PYTHONDONTWRITEBYTECODE=1
SPELLING_COVERAGE_FILE ?= /tmp/wireframe-spelling-helper.coverage
SPELLING_HELPER_PYTEST = PYTHONPATH=scripts $(PYTHON_NO_BYTECODE_ENV) \
	COVERAGE_FILE=$(SPELLING_COVERAGE_FILE) $(UV_ENV) $(UV) run --no-project \
	--python 3.14 --with pathspec==$(PATHSPEC_VERSION) --with pytest==9.0.2 \
	--with pytest-cov==7.0.0 python -m pytest
PROVER_TOOLS_REF_FILE ?= tools/rust-prover-tools/REF
PROVER_TOOLS_REF ?= $(shell awk '/^ref:/ { print $$2 }' $(PROVER_TOOLS_REF_FILE))
PROVER_TOOLS_SOURCE ?= git+https://github.com/leynos/rust-prover-tools.git@$(PROVER_TOOLS_REF)
PROVER_TOOLS ?= uv tool run --python 3.14 --from "$(PROVER_TOOLS_SOURCE)" prover-tools
VERUS_PROOF_FILE ?= verus/wireframe_proofs.rs

build: target/debug/lib$(CRATE).rlib ## Build debug binary
release: target/release/lib$(CRATE).rlib ## Build release binary

all: release ## Default target builds release binary

clean: ## Remove build artefacts
	$(CARGO) clean

test-bdd: ## Run rstest-bdd tests only
	RUSTFLAGS="-D warnings" $(CARGO) test --test bdd --all-features $(BUILD_JOBS)

test: ## Run all tests (bdd + unit/integration)
	RUSTFLAGS="-D warnings" $(CARGO) test --all-targets --all-features $(BUILD_JOBS)

test-workflow-contracts: ## Validate the mutation-testing caller contract
	$(PYTHON_NO_BYTECODE_ENV) uv run --with 'pytest>=8' --with 'pyyaml>=6' pytest tests/workflow_contracts -q

test-doc: ## Run doctests across all features
	RUSTFLAGS="-D warnings" $(CARGO) test --doc --all-features $(BUILD_JOBS)

doctest-benchmark: ## Check runnable/no_run doctest ratios
	./scripts/doctest-benchmark.sh

bench-codec: ## Run codec performance benchmarks
	RUSTFLAGS="-D warnings" $(CARGO) bench --bench codec_performance --bench codec_performance_alloc --features test-support $(BUILD_JOBS)

typecheck: ## Run a workspace typecheck
	RUSTFLAGS="-D warnings" $(CARGO) check --all-targets --all-features $(BUILD_JOBS)

# will match target/debug/libmy_library.rlib and target/release/libmy_library.rlib
target/%/lib$(CRATE).rlib: ## Build library in debug or release
	$(CARGO) build $(BUILD_JOBS)                            \
	  $(if $(findstring release,$(@)),--release)            \
	  --lib
	@# copy the .rlib into your own target tree
	install -Dm644                                           \
	  target/$(if $(findstring release,$(@)),release,debug)/lib$(CRATE).rlib \
	  $@

lint: ## Run Clippy with warnings denied
	RUSTDOCFLAGS="$(RUSTDOC_FLAGS)" $(CARGO) doc --no-deps
	$(CARGO) clippy $(CLIPPY_FLAGS)
	RUSTFLAGS="-D warnings" $(WHITAKER) --all -- --all-targets --all-features

fmt: ## Format Rust and Markdown sources
	$(CARGO) fmt --all
	mdformat-all

check-fmt: ## Verify formatting
	$(CARGO) fmt --all -- --check

markdownlint: spelling ## Lint Markdown and enforce en-GB-oxendict spelling
	$(MDLINT) "**/*.md"

spelling: spelling-phrase-check ## Enforce en-GB-oxendict spelling
	@git ls-files -z | xargs -0 -r env $(UV_ENV) \
		$(UV) tool run typos@$(TYPOS_VERSION) --config typos.toml --force-exclude --hidden

spelling-phrase-check: spelling-config
	@PYTHONPATH=scripts $(PYTHON_NO_BYTECODE_ENV) $(UV_ENV) $(UV) run --no-project --python 3.14 \
		scripts/typos_rollout_check.py --repository .

spelling-config: spelling-helper-test
	@git ls-files --error-unmatch typos.toml >/dev/null
	@$(TYPOS_CONFIG_BUILDER) --repository . --check

spelling-config-write: spelling-helper-test
	@$(TYPOS_CONFIG_BUILDER) --repository .

spelling-helper-test:
	@$(UV_ENV) $(UV) tool run ruff@$(RUFF_VERSION) format --isolated --target-version py313 --check $(SPELLING_PY_SRCS)
	@$(UV_ENV) $(UV) tool run ruff@$(RUFF_VERSION) check --isolated --target-version py313 $(SPELLING_PY_SRCS)
	@$(SPELLING_HELPER_PYTEST) $(SPELLING_PY_TESTS) -c /dev/null --rootdir=. -p no:cacheprovider $(SPELLING_COVERAGE_ARGS)

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

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
