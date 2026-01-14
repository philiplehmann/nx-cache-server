# YAML Configuration Guide

The `nx-cache-yaml` binary supports YAML-based configuration, enabling advanced features like:

- **Multiple S3 buckets** as cache backends
- **Multiple service tokens** with independent bucket assignments
- **Prefix-based isolation** within buckets (e.g., `/ci`, `/team1`)
- **Environment variable substitution** for sensitive credentials
- **Flexible credential management** (explicit, environment variables, or AWS auto-discovery)

## Quick Start

1. Create a configuration file (e.g., `config.yaml`):

```yaml
port: 3000

buckets:
  - name: my-bucket
    bucketName: my-s3-bucket-name
    region: us-west-2

serviceAccessTokens:
  - name: ci-token
    bucket: my-bucket
    prefix: /ci
    accessTokenEnv: CI_ACCESS_TOKEN
```

2. Set the required environment variable:

```bash
export CI_ACCESS_TOKEN=your-secret-token
```

3. Run the server:

```bash
nx-cache-yaml --config config.yaml
```

## Configuration Structure

### Top-Level Options

```yaml
# HTTP server port (optional, defaults to 3000)
port: 3000

# Enable debug logging (optional, defaults to false)
debug: false

buckets:
  # ... bucket configurations

serviceAccessTokens:
  # ... service token configurations
```

### Bucket Configuration

Each bucket represents an S3 or S3-compatible storage backend.

```yaml
buckets:
  - name: unique-bucket-name          # Required: unique identifier for this bucket
    bucketName: actual-s3-bucket      # Required: actual S3 bucket name
    
    # AWS Credentials (optional - see Credential Discovery below)
    accessKeyId: YOUR_ACCESS_KEY
    accessKeyIdEnv: ENV_VAR_NAME
    
    secretAccessKey: YOUR_SECRET_KEY
    secretAccessKeyEnv: ENV_VAR_NAME
    
    sessionToken: YOUR_SESSION_TOKEN  # Optional: for temporary credentials
    sessionTokenEnv: ENV_VAR_NAME
    
    # AWS Region (optional - auto-discovered if not provided)
    region: us-west-2
    
    # Custom endpoint for S3-compatible services (optional)
    endpointUrl: http://localhost:9000
    
    # Force path-style addressing (optional, defaults to false)
    # Required for MinIO and some S3-compatible services
    forcePathStyle: true
    
    # Operation timeout in seconds (optional, defaults to 30)
    timeout: 30
```

#### Force Path-Style Addressing

The `forcePathStyle` property controls how S3 URLs are constructed:

- **Virtual-hosted style** (default, `forcePathStyle: false`): `https://bucket-name.s3.amazonaws.com/key`
- **Path-style** (`forcePathStyle: true`): `https://s3.amazonaws.com/bucket-name/key`

**When to use `forcePathStyle: true`:**
- ✅ MinIO (required)
- ✅ Most self-hosted S3-compatible services
- ✅ Some cloud providers' S3-compatible storage (Hetzner, DigitalOcean Spaces, etc.)
- ✅ When using custom endpoint URLs with bucket names containing dots or special characters

**When to use `forcePathStyle: false` (default):**
- ✅ AWS S3 (standard configuration)
- ✅ AWS S3-compatible services that support virtual-hosted style
- ✅ When following AWS best practices

**Note:** AWS S3 deprecated path-style URLs for new buckets after September 2020, but still supports them for existing buckets. Virtual-hosted style is the recommended approach for AWS S3.

#### Credential Discovery

Buckets support three ways to provide credentials:

1. **Explicit values** (not recommended for production):
   ```yaml
   accessKeyId: AKIAIOSFODNN7EXAMPLE
   secretAccessKey: wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
   ```

2. **Environment variables** (recommended):
   ```yaml
   accessKeyIdEnv: AWS_ACCESS_KEY_ID
   secretAccessKeyEnv: AWS_SECRET_ACCESS_KEY
   ```

3. **AWS auto-discovery** (omit both - most secure):
   - Environment variables (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`)
   - AWS credentials file (`~/.aws/credentials`)
   - IAM role (EC2 instances, ECS tasks, Lambda)
   - AWS SSO

### Service Access Token Configuration

Each service token represents a client (e.g., CI pipeline, team, developer) that can access the cache.

```yaml
serviceAccessTokens:
  - name: unique-token-name    # Required: identifier for logging/debugging
    bucket: bucket-name        # Required: references a bucket by its 'name'
    prefix: /path              # Optional: prefix for all cache keys
    
    # Access token (required via one of these)
    accessToken: bearer-token-value
    accessTokenEnv: ENV_VAR_NAME
```

#### Prefix Behavior

Prefixes provide logical isolation within a bucket:

- **With prefix** `/ci`: stores cache at `/ci/{hash}`
- **Without prefix** (empty string): stores cache at `{hash}`
- **Nested prefixes** `/team1/subteam`: stores cache at `/team1/subteam/{hash}`

Prefixes are automatically normalized:
- Leading `/` is added if missing
- Trailing `/` is removed
- Empty strings remain empty (root level)

## Configuration Examples

### Example 1: Simple Setup (Single Bucket, IAM Credentials)

```yaml
port: 3000

buckets:
  - name: main
    bucketName: my-nx-cache
    region: us-west-2

serviceAccessTokens:
  - name: ci-pipeline
    bucket: main
    prefix: /ci
    accessTokenEnv: CI_TOKEN
```

### Example 2: Multi-Team with Prefixes

```yaml
buckets:
  - name: shared-bucket
    bucketName: company-nx-cache
    region: us-west-2

serviceAccessTokens:
  - name: frontend-team
    bucket: shared-bucket
    prefix: /frontend
    accessTokenEnv: FRONTEND_TOKEN
  
  - name: backend-team
    bucket: shared-bucket
    prefix: /backend
    accessTokenEnv: BACKEND_TOKEN
  
  - name: mobile-team
    bucket: shared-bucket
    prefix: /mobile
    accessTokenEnv: MOBILE_TOKEN
```

### Example 3: Multiple Buckets (Production + Staging)

```yaml
buckets:
  - name: production
    bucketName: prod-nx-cache
    region: us-west-2
    timeout: 45
  
  - name: staging
    bucketName: staging-nx-cache
    region: us-east-1
    timeout: 30

serviceAccessTokens:
  - name: prod-ci
    bucket: production
    prefix: /ci
    accessTokenEnv: PROD_CI_TOKEN
  
  - name: staging-ci
    bucket: staging
    prefix: /ci
    accessTokenEnv: STAGING_CI_TOKEN
```

### Example 4: MinIO (S3-Compatible Storage)

```yaml
buckets:
  - name: local-minio
    bucketName: nx-cache
    accessKeyId: minioadmin
    secretAccessKey: minioadmin
    region: us-east-1
    endpointUrl: http://localhost:9000
    forcePathStyle: true  # Required for MinIO

serviceAccessTokens:
  - name: local-dev
    bucket: local-minio
    prefix: /dev
    accessToken: dev-token-12345
```

### Example 5: Multiple Regions and Environments

```yaml
buckets:
  - name: us-prod
    bucketName: us-prod-cache
    region: us-west-2
  
  - name: eu-prod
    bucketName: eu-prod-cache
    region: eu-west-1
  
  - name: ap-prod
    bucketName: ap-prod-cache
    region: ap-southeast-1

serviceAccessTokens:
  - name: us-ci
    bucket: us-prod
    prefix: /ci
    accessTokenEnv: US_CI_TOKEN
  
  - name: eu-ci
    bucket: eu-prod
    prefix: /ci
    accessTokenEnv: EU_CI_TOKEN
  
  - name: ap-ci
    bucket: ap-prod
    prefix: /ci
    accessTokenEnv: AP_CI_TOKEN
```

## Running the Server

### Using Command Line

```bash
# Specify config file
nx-cache-yaml --config /path/to/config.yaml

# Using environment variable
export CONFIG_FILE=/path/to/config.yaml
nx-cache-yaml

# Enable debug logging
nx-cache-yaml --config config.yaml --debug
```

### Using Docker

```bash
docker run -v $(pwd)/config.yaml:/config.yaml \
  -e CI_TOKEN=your-token \
  -p 3000:3000 \
  nx-cache-yaml --config /config.yaml
```

### Using Kubernetes

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: nx-cache-config
data:
  config.yaml: |
    port: 3000
    buckets:
      - name: k8s-bucket
        bucketName: k8s-nx-cache
        region: us-west-2
    serviceAccessTokens:
      - name: ci-token
        bucket: k8s-bucket
        prefix: /ci
        accessTokenEnv: CI_TOKEN
---
apiVersion: v1
kind: Secret
metadata:
  name: nx-cache-secrets
stringData:
  CI_TOKEN: your-secret-token
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: nx-cache-server
spec:
  replicas: 1
  selector:
    matchLabels:
      app: nx-cache
  template:
    metadata:
      labels:
        app: nx-cache
    spec:
      containers:
      - name: server
        image: nx-cache-yaml:latest
        args: ["--config", "/config/config.yaml"]
        envFrom:
        - secretRef:
            name: nx-cache-secrets
        volumeMounts:
        - name: config
          mountPath: /config
        ports:
        - containerPort: 3000
      volumes:
      - name: config
        configMap:
          name: nx-cache-config
```

## Security Best Practices

### 1. Never Commit Secrets

❌ **DON'T**:
```yaml
serviceAccessTokens:
  - name: ci
    bucket: main
    accessToken: my-secret-token-123  # Exposed in git!
```

✅ **DO**:
```yaml
serviceAccessTokens:
  - name: ci
    bucket: main
    accessTokenEnv: CI_TOKEN  # Value in environment
```

### 2. Use IAM Roles When Possible

On AWS infrastructure (EC2, ECS, Lambda), omit credentials entirely:

```yaml
buckets:
  - name: main
    bucketName: my-cache
    region: us-west-2
    # No credentials - uses IAM role automatically
```

### 3. Rotate Tokens Regularly

Generate new tokens periodically and update environment variables:

```bash
# Generate new token
NEW_TOKEN=$(openssl rand -base64 32)

# Update in your secret management system
aws secretsmanager update-secret \
  --secret-id ci-token \
  --secret-string "$NEW_TOKEN"
```

### 4. Use Least-Privilege Policies

Grant only necessary S3 permissions:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:GetObject",
        "s3:PutObject",
        "s3:HeadObject"
      ],
      "Resource": "arn:aws:s3:::my-nx-cache/*"
    }
  ]
}
```

## Validation

The server validates configuration on startup:

- ✅ At least one bucket is configured
- ✅ At least one service token is configured
- ✅ Bucket names are unique
- ✅ Service token names are unique
- ✅ Service tokens reference existing buckets
- ✅ Credentials are provided in pairs (access key + secret key)
- ✅ Environment variables exist when referenced

Validation errors show helpful messages:

```
Configuration error: Service token 'ci-token' references non-existent bucket 'wrong-bucket'
```

## Troubleshooting

### Issue: "Environment variable not found"

**Problem**: Referenced environment variable is not set.

```
Configuration error: Service token 'ci': environment variable 'CI_TOKEN' not found
```

**Solution**: Set the environment variable before running:
```bash
export CI_TOKEN=your-token
nx-cache-yaml --config config.yaml
```

### Issue: "AWS_REGION must be set"

**Problem**: Region couldn't be auto-discovered and wasn't specified.

**Solution**: Add `region` to bucket configuration:
```yaml
buckets:
  - name: main
    bucketName: my-cache
    region: us-west-2  # Add this
```

### Issue: "Failed to initialize storage"

**Problem**: Invalid AWS credentials or bucket doesn't exist.

**Solution**: 
1. Verify bucket exists: `aws s3 ls s3://your-bucket-name`
2. Check credentials: `aws sts get-caller-identity`
3. Verify IAM permissions include `s3:GetObject`, `s3:PutObject`, `s3:HeadObject`

### Issue: "Authentication failed"

**Problem**: Client is using wrong or expired token.

**Solution**: Verify the token matches the configuration:
```bash
# In Nx workspace
export NX_CLOUD_AUTH_TOKEN=your-token-from-config
```

## Migration from CLI Arguments

If you're using the legacy `nx-cache-server` binary with CLI arguments, here's how to migrate:

### Before (CLI):
```bash
nx-cache-server \
  --bucket-name my-cache \
  --region us-west-2 \
  --service-access-token ci-token-123 \
  --port 3000
```

### After (YAML):
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
    accessToken: ci-token-123
```

```bash
nx-cache-yaml --config config.yaml
```

## Performance Considerations

### Timeout Settings

Adjust timeouts based on your network and file sizes:

```yaml
buckets:
  - name: main
    bucketName: my-cache
    region: us-west-2
    timeout: 60  # Increase for large artifacts
```

### Multiple Buckets vs. Prefixes

**Use multiple buckets when:**
- Different regions (lower latency)
- Different AWS accounts
- Different retention policies
- Separate billing

**Use prefixes when:**
- Same region/account
- Logical separation (teams, projects)
- Simplified management
- Cost optimization (single bucket)

## Example: Complete Production Setup

```yaml
# Production configuration with multiple teams and environments
port: 3000
debug: false

buckets:
  # Production bucket with IAM role
  - name: production
    bucketName: company-nx-cache-prod
    region: us-west-2
    timeout: 45

  # Staging bucket with explicit credentials from env
  - name: staging
    bucketName: company-nx-cache-staging
    accessKeyIdEnv: STAGING_AWS_ACCESS_KEY_ID
    secretAccessKeyEnv: STAGING_AWS_SECRET_ACCESS_KEY
    region: us-east-1
    timeout: 30

serviceAccessTokens:
  # Production CI/CD
  - name: prod-ci-2026
    bucket: production
    prefix: /ci
    accessTokenEnv: PROD_CI_TOKEN

  # Production - Frontend team
  - name: prod-frontend-2026
    bucket: production
    prefix: /frontend
    accessTokenEnv: PROD_FRONTEND_TOKEN

  # Production - Backend team
  - name: prod-backend-2026
    bucket: production
    prefix: /backend
    accessTokenEnv: PROD_BACKEND_TOKEN

  # Staging - All teams share
  - name: staging-2026
    bucket: staging
    prefix: /staging
    accessTokenEnv: STAGING_TOKEN
```

Environment variables (in CI/CD or secret manager):
```bash
export PROD_CI_TOKEN="$(generate_secure_token)"
export PROD_FRONTEND_TOKEN="$(generate_secure_token)"
export PROD_BACKEND_TOKEN="$(generate_secure_token)"
export STAGING_TOKEN="$(generate_secure_token)"
export STAGING_AWS_ACCESS_KEY_ID="AKIA..."
export STAGING_AWS_SECRET_ACCESS_KEY="..."
```

This setup provides:
- ✅ Separation between production and staging
- ✅ Team isolation via prefixes in production
- ✅ Secure credential management
- ✅ Easy token rotation
- ✅ Clear audit trail (named tokens)
