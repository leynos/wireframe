.PHONY: help all clean test build release lint fmt check-fmt markdownlint nixie

CRATE ?= wireframe
CARGO ?= cargo
BUILD_JOBS ?=
CLIPPY_FLAGS ?= --all-targets --all-features -- -D warnings
MDLINT ?= markdownlint
NIXIE ?= nixie

build: target/debug/lib$(CRATE) ## Build debug binary
release: target/release/lib$(CRATE) ## Build release binary

all: release ## Default target builds release binary

clean: ## Remove build artifacts
	$(CARGO) clean

test: ## Run tests with warnings treated as errors
	RUSTFLAGS="-D warnings" $(CARGO) test --all-targets --all-features $(BUILD_JOBS)

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
	$(CARGO) clippy $(CLIPPY_FLAGS)

fmt: ## Format Rust and Markdown sources
	$(CARGO) fmt --all
	mdformat-all

check-fmt: ## Verify formatting
	$(CARGO) fmt --all -- --check

markdownlint: ## Lint Markdown files
	find . -type f -name '*.md' -not -path './target/*' -print0 | xargs -0 -- $(MDLINT)

nixie: ## Validate Mermaid diagrams
	find . -type f -name '*.md' -not -path './target/*' -print0 | xargs -0 -- $(NIXIE)

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
