.PHONY: help all clean test test-doc doctest-benchmark bench-codec build release lint fmt check-fmt markdownlint nixie typecheck install-kani check-kani-version install-verus run-verus

CRATE ?= wireframe
CARGO ?= cargo
BUILD_JOBS ?=
CLIPPY_FLAGS ?= --all-targets --all-features -- -D warnings
RUSTDOC_FLAGS ?= --cfg docsrs -D warnings
MDLINT ?= markdownlint-cli2
NIXIE ?= nixie
WHITAKER ?= whitaker
PROVER_TOOLS_REF_FILE ?= tools/rust-prover-tools/REF
PROVER_TOOLS_REF ?= $(shell awk '/^ref:/ { print $$2 }' $(PROVER_TOOLS_REF_FILE))
PROVER_TOOLS_SOURCE ?= git+https://github.com/leynos/rust-prover-tools.git@$(PROVER_TOOLS_REF)
PROVER_TOOLS ?= uv tool run --python 3.14 --from "$(PROVER_TOOLS_SOURCE)" prover-tools
VERUS_PROOF_FILE ?= verus/wireframe_proofs.rs

build: target/debug/lib$(CRATE).rlib ## Build debug binary
release: target/release/lib$(CRATE).rlib ## Build release binary

all: release ## Default target builds release binary

clean: ## Remove build artifacts
	$(CARGO) clean

test-bdd: ## Run rstest-bdd tests only
	RUSTFLAGS="-D warnings" $(CARGO) test --test bdd --all-features $(BUILD_JOBS)

test: ## Run all tests (bdd + unit/integration)
	RUSTFLAGS="-D warnings" $(CARGO) test --all-targets --all-features $(BUILD_JOBS)

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

markdownlint: ## Lint Markdown files
	$(MDLINT) "**/*.md"

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
