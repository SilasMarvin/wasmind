# Docker image for running integration tests
# This provides a sandboxed environment for running cargo test

FROM rust:1.86

# Install dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create working directory
WORKDIR /workspace

# Copy the entire project (excluding what's in .dockerignore)
COPY . .

# Set environment variables that might be needed
ENV RUST_BACKTRACE=1

# Default command runs all integration tests with headless feature
# If TEST_NAME is provided, run only that test
CMD sh -c 'if [ -n "$TEST_NAME" ]; then cargo test --release --no-default-features --features headless "$TEST_NAME" -- --nocapture; else cargo test --release --no-default-features --features headless -- --nocapture; fi'