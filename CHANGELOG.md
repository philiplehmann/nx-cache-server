# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

#### YAML Configuration Support
- **New `nx-cache-server` binary** with YAML-based configuration replacing CLI arguments
- **Multiple S3 bucket support** - Configure multiple buckets as cache backends
- **Multiple service tokens** - Independent authentication tokens with different configurations
- **Prefix-based isolation** - Logical separation within buckets (e.g., `/ci`, `/team1`, `/backend`)
- **Environment variable substitution** - Load secrets from environment variables for better security
- **Comprehensive configuration validation** - Detailed error messages with helpful guidance

#### Bucket Configuration Features
- `buckets` array for configuring multiple S3 or S3-compatible storage backends
- Per-bucket configuration options:
  - `name` - Unique identifier for the bucket
  - `bucketName` - Actual S3 bucket name
  - `region` - AWS region (with auto-discovery fallback)
  - `accessKeyId` / `accessKeyIdEnv` - AWS access key (direct or from env var)
  - `secretAccessKey` / `secretAccessKeyEnv` - AWS secret key (direct or from env var)
  - `sessionToken` / `sessionTokenEnv` - AWS session token for temporary credentials
  - `endpointUrl` - Custom endpoint for S3-compatible services (MinIO, Hetzner, etc.)
  - `forcePathStyle` - Force path-style URL addressing (required for MinIO)
  - `timeout` - S3 operation timeout in seconds

#### Service Token Configuration Features
- `serviceAccessTokens` array for configuring multiple authentication tokens
- Per-token configuration options:
  - `name` - Identifier for logging and debugging
  - `bucket` - Reference to bucket configuration by name
  - `prefix` - Path prefix for cache isolation (e.g., `/ci`, `/team1`)
  - `accessToken` / `accessTokenEnv` - Bearer token (direct or from env var)

#### New Infrastructure Components
- **MultiStorageRouter** - Routes requests to appropriate bucket based on authenticated token
- **ResolvedConfig** - Fully validated configuration with environment variables loaded
- **YamlConfig** - YAML parsing and validation with detailed error messages
- Prefix normalization (ensures leading `/`, removes trailing `/`)
- Token-to-bucket-to-storage routing with O(1) lookup performance

#### Documentation
- **Complete YAML Configuration Guide** (`docs/yaml-configuration.md`)
  - Detailed explanations of all configuration options
  - Credential discovery methods (explicit, environment variables, AWS auto-discovery)
  - Force path-style addressing explanation
  - Security best practices
  - Kubernetes deployment examples
  - Docker configurations
  - Troubleshooting guide
- **Quick Start Guide** (`docs/QUICKSTART.md`)
  - 5-minute setup instructions
  - Installation steps for all platforms
  - Configuration examples
  - Testing and verification steps
  - Common troubleshooting scenarios
- **Configuration Guide** (`docs/migration-to-yaml.md`)
  - YAML configuration patterns
  - Multi-team setup examples
  - Multi-environment examples
  - Secret management integration
- **Example Configurations**
  - `examples/config.minimal.yaml` - Simplest possible setup
  - `examples/config.example.yaml` - Comprehensive example with all options

### Changed

#### Breaking Changes
- **Removed legacy `nx-cache-aws` binary** - Use `nx-cache-server` with YAML config instead
- **Server architecture refactored** - Now uses `MultiStorageRouter` for multi-bucket support
- **Middleware updated** - Authenticated token stored in request extensions for handler access
- **AppState simplified** - No longer generic over storage provider type

#### Improvements
- **Better error messages** - Configuration validation provides detailed, actionable error messages
- **Enhanced security** - Secrets loaded from environment variables rather than hardcoded
- **Improved logging** - Request logs now include bucket name and prefix information
- **Configuration as code** - YAML files can be version-controlled (minus secrets)
- **Flexible deployment** - Single configuration file for all environments

### Fixed
- Credential provider now uses `Arc` instead of `Box` for better performance
- Request body borrow issues resolved in handlers
- HashSet contains calls now use proper string references

## Features by Use Case

### Single Bucket Setup
Perfect for small teams or simple deployments:
```yaml
port: 3000
buckets:
  - name: main
    bucketName: my-cache
    region: us-west-2
serviceAccessTokens:
  - name: default
    bucket: main
    prefix: ""
    accessTokenEnv: NX_CACHE_TOKEN
```

### Multi-Team with Prefix Isolation
Share one bucket with logical separation:
```yaml
buckets:
  - name: shared
    bucketName: company-cache
    region: us-west-2
serviceAccessTokens:
  - name: frontend-team
    bucket: shared
    prefix: /frontend
    accessTokenEnv: FRONTEND_TOKEN
  - name: backend-team
    bucket: shared
    prefix: /backend
    accessTokenEnv: BACKEND_TOKEN
```

### Multi-Environment with Separate Buckets
Different buckets for production and staging:
```yaml
buckets:
  - name: production
    bucketName: prod-cache
    region: us-west-2
  - name: staging
    bucketName: staging-cache
    region: us-east-1
serviceAccessTokens:
  - name: prod-ci
    bucket: production
    prefix: /ci
    accessTokenEnv: PROD_TOKEN
  - name: staging-ci
    bucket: staging
    prefix: /ci
    accessTokenEnv: STAGING_TOKEN
```

### MinIO / S3-Compatible Services
Works with any S3-compatible storage:
```yaml
buckets:
  - name: local
    bucketName: nx-cache
    accessKeyId: minioadmin
    secretAccessKey: minioadmin
    region: us-east-1
    endpointUrl: http://localhost:9000
    forcePathStyle: true  # Required for MinIO
serviceAccessTokens:
  - name: dev
    bucket: local
    prefix: /dev
    accessToken: local-dev-token
```

## Migration Guide

### From CLI Arguments to YAML

**Before:**
```bash
export S3_BUCKET_NAME="my-cache"
export AWS_REGION="us-west-2"
export SERVICE_ACCESS_TOKEN="my-token"
nx-cache-aws
```

**After:**
```yaml
# config.yaml
port: 3000
buckets:
  - name: main
    bucketName: my-cache
    region: us-west-2
serviceAccessTokens:
  - name: default
    bucket: main
    prefix: ""
    accessTokenEnv: SERVICE_ACCESS_TOKEN
```

```bash
export SERVICE_ACCESS_TOKEN="my-token"
nx-cache-server --config config.yaml
```

## Technical Details

### Architecture Changes

1. **Storage Layer**
   - New `MultiStorageRouter` manages multiple S3Storage instances
   - Token-based routing: Token → Service Config → Bucket → Storage
   - Prefix handling: Automatically prepends prefix to all cache keys

2. **Configuration Layer**
   - `YamlConfig` - Raw YAML structure with validation
   - `ResolvedConfig` - Processed config with env vars loaded
   - `ResolvedBucketConfig` / `ResolvedServiceAccessToken` - Runtime configs

3. **Server Layer**
   - Simplified `AppState` (no generic type parameter)
   - `AuthenticatedToken` extension stores validated token
   - Handlers access token from request extensions

### Performance Characteristics

- **Configuration loading**: O(n) where n = number of buckets + tokens
- **Token lookup**: O(1) via HashMap
- **Storage routing**: O(1) after token validation
- **Memory usage**: < 4MB during normal operation (unchanged)
- **Request latency**: < 1ms overhead for token routing

### Security Enhancements

1. **No secrets in configuration files** - Use `*Env` fields to reference environment variables
2. **Constant-time token comparison** - Prevents timing attacks
3. **IAM role support** - Auto-discovery when credentials omitted
4. **Multiple isolation mechanisms** - Buckets, prefixes, and tokens
5. **Comprehensive validation** - Catches misconfigurations at startup

## Compatibility

- ✅ **Backward compatible at HTTP API level** - Nx clients require no changes
- ❌ **Binary not backward compatible** - Must migrate from `nx-cache-aws` to `nx-cache-server`
- ✅ **S3 data compatible** - Existing cache data accessible if using same bucket without prefix
- ⚠️ **Prefix change requires cache rebuild** - New prefix = new cache namespace

## Dependencies

### New Dependencies
- `serde_yml` 0.0.12 - YAML parsing and serialization (replaces deprecated `serde_yaml`)

### Existing Dependencies (unchanged)
- `tokio` 1.0 - Async runtime
- `axum` 0.8 - Web framework
- `aws-sdk-s3` 1.0 - S3 client
- `serde` 1.0 - Serialization framework
- `clap` 4.0 - CLI argument parsing

## Testing

- ✅ All existing tests passing (21 tests)
- ✅ New YAML configuration validation tests
- ✅ Prefix normalization tests
- ✅ Multi-storage routing tests
- ✅ No regression in existing functionality

## Contributors

Thank you to everyone who contributed to this release!

---

For more information, see the [documentation](docs/) directory.
