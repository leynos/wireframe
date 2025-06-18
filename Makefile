.PHONY: lint test fmt

# Run Clippy lints across all targets and features, failing on warnings
lint:
	cargo clippy --all-targets --all-features -- -D warnings

# Execute tests with warnings treated as errors
# --quiet ensures less verbose output on success
# Use RUSTFLAGS to deny warnings at compile time
# so new warnings cause failures

test:
	RUSTFLAGS="-D warnings" cargo test --quiet

# Format the entire workspace
fmt:
	cargo fmt --all
