# Nx Custom Remote Cache Server

[![Release](https://github.com/philiplehmann/nx-cache-server/actions/workflows/release.yml/badge.svg)](https://github.com/philiplehmann/nx-cache-server/actions/workflows/release.yml)

A lightweight, high-performance Nx cache server that bridges Nx CLI clients with cloud storage providers for caching build artifacts. Built in Rust with a focus on maximum performance and minimal memory usage - less than 4MB during regular operation! 🚀

## Features

- **AWS S3 Integration**: Direct streaming integration with AWS S3 and S3-compatible services
- **Multiple Backends**: Support for multiple S3 buckets with flexible YAML configuration
- **Prefix-Based Isolation**: Logical isolation within buckets using prefixes (e.g., `/ci`, `/team1`)
- **Multiple Service Tokens**: Independent tokens with different bucket and prefix assignments
- **Memory Efficient**: Direct streaming with less than 4MB RAM usage during typical operation
- **High Performance**: Built with Rust and Axum for maximum throughput
- **Zero Dependencies**: Self-contained single executable with no external dependencies required
- **Nx API Compliant**: Full implementation of the [Nx custom remote cache OpenAPI specification](https://nx.dev/recipes/running-tasks/self-hosted-caching#build-your-own-caching-server)
- **Security First**: Bearer token authentication with constant-time comparison
- **Self-Hosted & Private**: Full control over your data with zero telemetry

## Quick Start

### Prerequisites

Access to S3-compatible services

### Development Setup

If you want to build from source, you'll need Rust installed. We recommend using [asdf](https://asdf-vm.com/) for managing the Rust version:

```bash
# Install the Rust plugin
asdf plugin add rust https://github.com/asdf-community/asdf-rust.git

# Install the version specified in .tool-versions
asdf install
```

### Installation

#### Binary

##### Step 1: Download the binary
Go to [Releases page](https://github.com/philiplehmann/nx-cache-server/releases) and download the binary for your operating system.

Alternatively, use command line tools:
```bash
# Download the binary
curl -L https://github.com/philiplehmann/nx-cache-server/releases/download/<VERSION>/nx-cache-server-<VERSION>-<PLATFORM> -o nx-cache-server

# Replace:
#  <VERSION> with the version tag (e.g., v1.1.0)
#  <PLATFORM> with your platform (e.g., linux-x86_64, macos-arm64, macos-x86_64, windows-x86_64.exe).
```

##### Step 2: Make executable (Linux/macOS only)
```bash
chmod +x nx-cache-server
```

#### Docker

##### Step 1: Pull the image
```bash
docker pull philiplehmann/nx-cache-server:<VERSION>
```

##### Step 2: Run the container
```bash
docker run \
  -p 3000:3000 \
  -e CI_ACCESS_TOKEN=your-secret-token \
  -v /path/to/config.yaml:/nx-cache-server/config.yaml \
  philiplehmann/nx-cache-server:<VERSION>
```

## Configuration

### YAML Configuration

**Features:**
- ✅ Multiple S3 buckets as backends
- ✅ Multiple service tokens with independent bucket assignments
- ✅ Prefix-based isolation (e.g., `/ci`, `/team1`)
- ✅ Environment variable substitution for secrets
- ✅ Flexible credential management

Create a `config.yaml` file:

```yaml
port: 3000

buckets:
  - name: production
    bucketName: my-nx-cache
    region: us-west-2

serviceAccessTokens:
  - name: ci-pipeline
    bucket: production
    prefix: /ci
    accessTokenEnv: CI_ACCESS_TOKEN
```

Set environment variables and run:

```bash
export CI_ACCESS_TOKEN=your-secret-token
nx-cache-server --config config.yaml
```

**📋 [Example Configurations](examples/)** - Ready-to-use configuration files:
- [`config.minimal.yaml`](examples/config.minimal.yaml) - Simplest setup for quick start
- [`config.example.yaml`](examples/config.example.yaml) - Comprehensive example with all options
- [`docker-compose.yaml`](examples/docker-compose.yaml) - Docker Compose example with all options
- [`kustomize.yaml`](examples/kustomize.yaml) - Kubernetes example with all options

#### Step 3 (optional): Verify the service is up and running
```bash
curl http://localhost:3000/health
```
You should receive an "OK" response.

### Client Configuration

To configure your Nx workspace to use this cache server, set the following environment variables:

```bash
# Point Nx to your cache server
export NX_SELF_HOSTED_REMOTE_CACHE_SERVER="http://localhost:3000"

# Authentication token (must match one of the tokens from SERVICE_ACCESS_TOKENS on the server)
export NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN="token1"

# Optional: Disable TLS certificate validation (e.g. for development/testing environment)
export NODE_TLS_REJECT_UNAUTHORIZED="0"
```

Once configured, Nx will automatically use your cache server for storing and retrieving build artifacts.
