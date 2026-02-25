.PHONY: help all clean test test-doc doctest-benchmark bench-codec build release lint fmt check-fmt markdownlint nixie typecheck

CRATE ?= wireframe
CARGO ?= cargo
BUILD_JOBS ?=
CLIPPY_FLAGS ?= --all-targets --all-features -- -D warnings
RUSTDOC_FLAGS ?= --cfg docsrs -D warnings
MDLINT ?= markdownlint-cli2
NIXIE ?= nixie

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

fmt: ## Format Rust and Markdown sources
	$(CARGO) fmt --all
	mdformat-all

check-fmt: ## Verify formatting
	$(CARGO) fmt --all -- --check

markdownlint: ## Lint Markdown files
	$(MDLINT) "**/*.md"

nixie: ## Validate Mermaid diagrams
	$(NIXIE) --no-sandbox

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
