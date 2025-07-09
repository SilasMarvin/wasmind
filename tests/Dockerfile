# Optimized Docker build for integration tests
FROM rust:1.86

# Install dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Copy dependency files first for better caching
COPY Cargo.toml Cargo.lock ./

# Copy source files
COPY src/ ./src/
COPY tests/ ./tests/
COPY default_config.toml headless_config.toml ./

# Build and cache dependencies by building without running tests
RUN cargo build --release --no-default-features --features headless

# Build the integration tests (this will reuse cached dependencies)
RUN cargo test --release --no-default-features --features headless --no-run

# Set environment variables
ENV RUST_BACKTRACE=1

# Default command runs all integration tests with headless feature
# If TEST_NAME is provided, run only that test
CMD sh -c 'if [ -n "$TEST_NAME" ]; then cargo test --release --no-default-features --features headless "$TEST_NAME" -- --nocapture; else cargo test --release --no-default-features --features headless -- --nocapture; fi'