# Integration Tests

This directory contains integration tests for `nx-cache-server` using [testcontainers-rs](https://docs.rs/testcontainers/latest/testcontainers/) and the MinIO SDK.

## Overview

The integration tests use Docker containers to test the application against real MinIO infrastructure. All tests are fully functional and passing using the official MinIO Rust SDK (`minio-rs`).

## Prerequisites

- Docker must be installed and running
- Rust toolchain (automatically handled by `cargo test`)

## Running Tests

Run all integration tests:

```bash
cargo test --test '*'
```

Run a specific test file:

```bash
cargo test --test integration_test
cargo test --test integration_minio
```

Run a specific test:

```bash
cargo test --test integration_test test_basic_store_and_retrieve
```

Run with output:

```bash
cargo test --test integration_test -- --nocapture
```

## References

- [testcontainers-rs documentation](https://docs.rs/testcontainers/latest/testcontainers/)
- [testcontainers-modules MinIO](https://docs.rs/testcontainers-modules/latest/testcontainers_modules/minio/)
- [MinIO Docker documentation](https://min.io/docs/minio/container/index.html)
- [MinIO Rust SDK documentation](https://docs.rs/minio/latest/minio/)
- [MinIO SDK Examples](https://github.com/minio/minio-rs/tree/main/examples)
