
# Build the project in debug mode
build:
    cargo build

# Build the project in release mode
build-release:
    cargo build --release

# Run the project in debug mode
run:
    cargo run

# Run the project in release mode
run-release:
    cargo run --release

# Watch for changes and rebuild automatically
watch:
    cargo watch -x run

# Run tests
test:
    cargo test

# Run tests with output
test-verbose:
    cargo test -- --nocapture

# Check code formatting
fmt-check:
    cargo fmt -- --check

# Format code
fmt:
    cargo fmt

# Run clippy lints
lint:
    cargo clippy -- -D warnings

# Clean build artifacts
clean:
    cargo clean

# Generate documentation
docs:
    cargo doc --no-deps --open

# Update dependencies
update:
    cargo update
