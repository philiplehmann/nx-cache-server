# YAML Configuration Guide

This guide shows you how to use the `nx-cache-server` binary with YAML configuration.

## Why YAML Configuration?

YAML configuration provides:

- ✅ **Multiple S3 buckets** - Different backends for different teams/environments
- ✅ **Prefix-based isolation** - Logical separation within buckets (e.g., `/ci`, `/team1`)
- ✅ **Multiple service tokens** - Independent tokens with different configurations
- ✅ **Environment variable substitution** - Better secret management
- ✅ **Version control friendly** - Configuration as code
- ✅ **Easier to maintain** - Clear structure, comments, validation

## Configuration Examples

### Example 1: Simple Single Bucket

**config.yaml:**
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
    accessToken: ci-token-123
```

**Run:**
```bash
nx-cache-server --config config.yaml
```

**Using environment variable for token (recommended):**

**config.yaml:**
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
    accessTokenEnv: SERVICE_ACCESS_TOKEN
```

```bash
export SERVICE_ACCESS_TOKEN="ci-token-123"
nx-cache-server --config config.yaml
```

### Example 2: With Explicit AWS Credentials

**config.yaml:**
```yaml
port: 3000

buckets:
  - name: main
    bucketName: my-cache
    region: us-west-2
    accessKeyIdEnv: AWS_ACCESS_KEY_ID
    secretAccessKeyEnv: AWS_SECRET_ACCESS_KEY

serviceAccessTokens:
  - name: default
    bucket: main
    prefix: ""
    accessTokenEnv: SERVICE_ACCESS_TOKEN
```

```bash
export AWS_ACCESS_KEY_ID="AKIAIOSFODNN7EXAMPLE"
export AWS_SECRET_ACCESS_KEY="wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
export SERVICE_ACCESS_TOKEN="ci-token-123"

nx-cache-server --config config.yaml
```

### Example 3: Multiple Tokens with Prefix Isolation

**config.yaml:**
```yaml
port: 3000

buckets:
  - name: shared
    bucketName: my-cache
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

```bash
export FRONTEND_TOKEN="frontend-token"
export BACKEND_TOKEN="backend-token"

nx-cache-server --config config.yaml
```

Both teams share one bucket with prefix isolation!

### Example 4: MinIO/S3-Compatible Service

**config.yaml:**
```yaml
port: 3000

buckets:
  - name: local
    bucketName: nx-cache
    region: us-east-1
    endpointUrl: http://localhost:9000
    forcePathStyle: true  # Required for MinIO
    accessKeyId: minioadmin
    secretAccessKey: minioadmin

serviceAccessTokens:
  - name: dev
    bucket: local
    prefix: ""
    accessToken: dev-token
```

```bash
nx-cache-server --config config.yaml
```

## Getting Started

### Step 1: Install Binary

Download the `nx-cache-server` binary from the releases page:

```bash
# macOS ARM64
curl -L https://github.com/philiplehmann/nx-cache-server/releases/latest/download/nx-cache-server-macos-arm64 -o nx-cache-server
chmod +x nx-cache-server

# Linux x86_64
curl -L https://github.com/philiplehmann/nx-cache-server/releases/latest/download/nx-cache-server-linux-x86_64 -o nx-cache-server
chmod +x nx-cache-server
```

### Step 2: Create YAML Configuration

Create your `config.yaml` file using the examples above.

**Minimal template:**

```yaml
port: 3000  # or your port

buckets:
  - name: main
    bucketName: YOUR_BUCKET_NAME
    region: YOUR_REGION
    # Add credentials if needed:
    # accessKeyIdEnv: AWS_ACCESS_KEY_ID
    # secretAccessKeyEnv: AWS_SECRET_ACCESS_KEY
    # endpointUrl: http://localhost:9000  # for MinIO

serviceAccessTokens:
  - name: default
    bucket: main
    prefix: ""
    accessTokenEnv: SERVICE_ACCESS_TOKEN
```

### Step 3: Test the Server

Start the server and verify it works:

```bash
export SERVICE_ACCESS_TOKEN="your-token"
nx-cache-server --config config.yaml

# Test health check
curl http://localhost:3001/health

# Test cache operation
echo "test" > test.txt
HASH=$(sha256sum test.txt | cut -d' ' -f1)
curl -X PUT "http://localhost:3000/v1/cache/$HASH" \
  -H "Authorization: Bearer $SERVICE_ACCESS_TOKEN" \
  --data-binary @test.txt
```

### Step 4: Update Deployment Configuration

Update any scripts, Docker files, or Kubernetes manifests:

**Docker example:**

```dockerfile
COPY config.yaml /app/config.yaml
CMD ["nx-cache-server", "--config", "/app/config.yaml"]
```

**Kubernetes example:**

```yaml
volumeMounts:
  - name: config
    mountPath: /config
volumes:
  - name: config
    configMap:
      name: nx-cache-config
env:
  - name: SERVICE_ACCESS_TOKEN
    valueFrom:
      secretKeyRef:
        name: nx-cache-secrets
        key: token
args: ["--config", "/config/config.yaml"]
```

## Common Configuration Patterns

### Pattern 1: Team Isolation with Prefixes

Single server with prefix isolation:

```yaml
port: 3000

buckets:
  - name: shared
    bucketName: cache
    region: us-west-2

serviceAccessTokens:
  - name: frontend-team
    bucket: shared
    prefix: /frontend
    accessToken: frontend-token
  
  - name: backend-team
    bucket: shared
    prefix: /backend
    accessToken: backend-token
```

**Benefits:**
- Single server process
- Same port for all teams
- Logical isolation via prefixes
- Easier to manage

### Pattern 2: Separate Buckets per Environment

```yaml
port: 3000

buckets:
  - name: production
    bucketName: prod-cache
    region: us-west-2
    timeout: 60
  
  - name: staging
    bucketName: staging-cache
    region: us-east-1
    timeout: 30

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

### Pattern 3: Secret Management Integration

Reference secrets from secret management:

```yaml
serviceAccessTokens:
  - name: ci
    bucket: main
    prefix: /ci
    accessTokenEnv: CI_TOKEN  # Loaded from Vault, AWS Secrets Manager, etc.
```

```bash
# In production, fetch from secret manager
export CI_TOKEN=$(aws secretsmanager get-secret-value --secret-id ci-token --query SecretString --output text)
nx-cache-server --config config.yaml
```

## FAQ

**Q: Can I validate my YAML before running?**

A: Yes, the server validates on startup and provides clear error messages. You can also use YAML validators online.

**Q: How do I handle secrets securely?**

A: Use the `*Env` fields (e.g., `accessTokenEnv`, `secretAccessKeyEnv`) to reference environment variables rather than hardcoding secrets in the YAML file.

**Q: Can I use multiple buckets in different regions?**

A: Yes! Simply configure multiple buckets with different regions and assign tokens to the appropriate bucket.

**Q: What happens if I use prefixes?**

A: Artifacts are stored at `{prefix}/{hash}` instead of just `{hash}`. This provides logical isolation but means different prefixes can't share cached artifacts.

## Need Help?

- 📚 [Complete Configuration Guide](yaml-configuration.md)
- 🚀 [Quick Start Guide](QUICKSTART.md)
- 📋 [Example Configurations](../examples/)
- 🐛 [Report Issues](https://github.com/philiplehmann/nx-cache-server/issues)

Happy caching! 🎉
