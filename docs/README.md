# Nx Cache Server Documentation

Welcome to the Nx Cache Server documentation! This directory contains comprehensive guides to help you get started and make the most of your cache server.

## 📚 Documentation

### Getting Started

- **[Quick Start Guide](QUICKSTART.md)** - Get up and running in 5 minutes
  - Installation instructions for all platforms
  - Minimal configuration example
  - Testing and verification steps
  - Common troubleshooting scenarios

### Configuration

- **[YAML Configuration Guide](yaml-configuration.md)** - Complete reference
  - All configuration options explained
  - Credential management (explicit, environment variables, IAM roles)
  - Force path-style addressing
  - Multiple buckets and service tokens
  - Prefix-based isolation
  - Security best practices
  - Kubernetes and Docker deployment examples
  - Performance tuning
  - Troubleshooting guide

- **[Configuration Examples](migration-to-yaml.md)** - Common patterns
  - Simple single-bucket setup
  - Multi-team with prefix isolation
  - Multi-environment with separate buckets
  - MinIO and S3-compatible services
  - Secret management integration

## 🚀 Quick Links

### Common Use Cases

#### Single Bucket Setup
Perfect for small teams or simple deployments:
```yaml
port: 3000
buckets:
  - name: main
    bucketName: my-nx-cache
    region: us-west-2
serviceAccessTokens:
  - name: default
    bucket: main
    prefix: ""
    accessTokenEnv: NX_CACHE_TOKEN
```

#### Multi-Team with Prefixes
Share one bucket with logical separation:
```yaml
buckets:
  - name: shared
    bucketName: company-cache
    region: us-west-2
serviceAccessTokens:
  - name: frontend
    bucket: shared
    prefix: /frontend
    accessTokenEnv: FRONTEND_TOKEN
  - name: backend
    bucket: shared
    prefix: /backend
    accessTokenEnv: BACKEND_TOKEN
```

#### MinIO Setup
For local development or self-hosted storage:
```yaml
buckets:
  - name: local
    bucketName: nx-cache
    accessKeyId: minioadmin
    secretAccessKey: minioadmin
    region: us-east-1
    endpointUrl: http://localhost:9000
    forcePathStyle: true
serviceAccessTokens:
  - name: dev
    bucket: local
    prefix: /dev
    accessToken: local-dev-token
```

## 📋 Example Configurations

Ready-to-use configuration files in the [`examples/`](../examples/) directory:

- **[config.minimal.yaml](../examples/config.minimal.yaml)** - Simplest setup for quick start
- **[config.example.yaml](../examples/config.example.yaml)** - Comprehensive example with all options

## 🔑 Key Features

### Multiple S3 Buckets
Configure multiple buckets as backends for different teams or environments:
- Different AWS accounts or regions
- Separate production and staging storage
- Cost allocation by bucket

### Service Tokens with Prefixes
Independent authentication tokens with flexible routing:
- Each token can target a specific bucket
- Optional prefix for namespace isolation
- Named tokens for better logging and debugging

### Environment Variable Substitution
Secure secret management:
- Reference environment variables in config
- Keep secrets out of version control
- Integration with secret management systems (Vault, AWS Secrets Manager, etc.)

### Flexible Credential Management
Multiple ways to provide AWS credentials:
- **Explicit values** - For testing or non-production
- **Environment variables** - Standard approach
- **AWS auto-discovery** - IAM roles, EC2 instance profiles, ECS task roles, SSO

### S3-Compatible Services
Works with any S3-compatible storage:
- MinIO (self-hosted)
- DigitalOcean Spaces
- Hetzner Object Storage
- Backblaze B2
- Any service with S3-compatible API

## 🛠️ Configuration Options

### Bucket Configuration

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `name` | string | Yes | Unique identifier for this bucket |
| `bucketName` | string | Yes | Actual S3 bucket name |
| `region` | string | No | AWS region (auto-discovered if omitted) |
| `accessKeyId` | string | No | AWS access key (or use `accessKeyIdEnv`) |
| `accessKeyIdEnv` | string | No | Environment variable holding access key |
| `secretAccessKey` | string | No | AWS secret key (or use `secretAccessKeyEnv`) |
| `secretAccessKeyEnv` | string | No | Environment variable holding secret key |
| `sessionToken` | string | No | AWS session token for temporary credentials |
| `sessionTokenEnv` | string | No | Environment variable holding session token |
| `endpointUrl` | string | No | Custom S3 endpoint (for MinIO, etc.) |
| `forcePathStyle` | boolean | No | Force path-style URLs (required for MinIO) |
| `timeout` | number | No | S3 operation timeout in seconds (default: 30) |

### Service Token Configuration

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `name` | string | Yes | Identifier for logging/debugging |
| `bucket` | string | Yes | Reference to bucket by name |
| `prefix` | string | No | Path prefix for cache isolation (e.g., `/ci`) |
| `accessToken` | string | No* | Bearer token (or use `accessTokenEnv`) |
| `accessTokenEnv` | string | No* | Environment variable holding token |

\* Either `accessToken` or `accessTokenEnv` must be provided

## 🔒 Security Best Practices

1. **Use environment variables for secrets** - Never commit tokens or credentials to git
2. **Use IAM roles when possible** - Omit credentials on AWS infrastructure
3. **Rotate tokens regularly** - Generate new tokens periodically
4. **Use least-privilege policies** - Grant only necessary S3 permissions
5. **Enable prefix isolation** - Separate teams/projects with prefixes
6. **Use HTTPS in production** - Especially for `endpointUrl` settings

## 🐛 Troubleshooting

### Common Issues

**"Environment variable not found"**
- Solution: Set the referenced environment variable before running the server

**"Failed to initialize storage"**
- Solution: Check AWS credentials, bucket exists, and IAM permissions

**"Configuration validation error"**
- Solution: Review error message - it will tell you exactly what's wrong

**"Authentication failed"**
- Solution: Ensure client token matches server configuration exactly

### Debug Mode

Enable detailed logging:
```bash
nx-cache-server --config config.yaml --debug
```

Or in config:
```yaml
debug: true
```

## 📞 Getting Help

- 🐛 [Report Issues](https://github.com/philiplehmann/nx-cache-server/issues)
- 💬 [Discussions](https://github.com/philiplehmann/nx-cache-server/discussions)
- 📖 [Main README](../README.md)
- 📝 [Changelog](../CHANGELOG.md)

## 🎯 Next Steps

1. Follow the [Quick Start Guide](QUICKSTART.md) to get your server running
2. Review the [YAML Configuration Guide](yaml-configuration.md) for advanced features
3. Check out [example configurations](../examples/) for your use case
4. Configure your Nx workspace to use the cache server

Happy caching! ⚡
