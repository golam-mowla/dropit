# Contributing to Drop It

Thank you for contributing to Drop It! We welcome improvements, bug reports, and optimizations that align with our core design goals.

## Core Priorities

1. **Transfer speed**: Always maintain peak streaming throughput. Avoid unnecessary copies, allocations, and intermediate buffers.
2. **Low RAM/CPU**: Keep idle footprint below 25 MB and active transfers below 60 MB. Avoid heavy dependencies or runtimes.
3. **Security**: Preserve strict TLS 1.3, private IP boundaries, path sanitization, and cryptographic identity.
4. **Simplicity**: Zero-config, single binary, embedded assets with no build step required for UI.

## Development Workflow

### Setup & Requirements
- Rust stable toolchain (1.85+ recommended)
- Standard Cargo tools: `cargo fmt`, `cargo clippy`, `cargo test`

### Running Checks
Before submitting a pull request, ensure all CI checks pass locally:
```bash
# Check code formatting
cargo fmt --check

# Check clippy warnings
cargo clippy -- -D warnings

# Run all unit and integration tests
cargo test
```

### Commit Guidelines
- Keep commits focused and logically grouped.
- Write clear, imperative commit messages (e.g., `Add rate limiting for upload offers`).
