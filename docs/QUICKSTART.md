# Quick Start Guide

Get up and running with the Nx Cache Server in 5 minutes!

## Prerequisites

- An S3 bucket (AWS S3 or S3-compatible service like MinIO)
- AWS credentials with permissions: `s3:GetObject`, `s3:PutObject`, `s3:HeadObject`

## Installation

### Download the Binary

```bash
# macOS ARM64 (Apple Silicon)
curl -L https://github.com/philiplehmann/nx-cache-server/releases/latest/download/nx-cache-server-macos-arm64 -o nx-cache-server
chmod +x nx-cache-server

# macOS x86_64 (Intel)
curl -L https://github.com/philiplehmann/nx-cache-server/releases/latest/download/nx-cache-server-macos-x86_64 -o nx-cache-server
chmod +x nx-cache-server

# Linux x86_64
curl -L https://github.com/philiplehmann/nx-cache-server/releases/latest/download/nx-cache-server-linux-x86_64 -o nx-cache-server
chmod +x nx-cache-server

# Windows x86_64
curl -L https://github.com/philiplehmann/nx-cache-server/releases/latest/download/nx-cache-server-windows-x86_64.exe -o nx-cache-server.exe
```

### Move to PATH (Optional)

```bash
# macOS/Linux
sudo mv nx-cache-server /usr/local/bin/

# Verify installation
nx-cache-server --help
```

## Configuration

### Step 1: Create Configuration File

Create a file named `config.yaml`:

```yaml
port: 3000

buckets:
  - name: main
    bucketName: my-nx-cache-bucket  # Replace with your S3 bucket name
    region: us-west-2               # Replace with your AWS region

serviceAccessTokens:
  - name: default
    bucket: main
    prefix: ""
    accessTokenEnv: NX_CACHE_TOKEN
```

**Important:** Replace `my-nx-cache-bucket` with your actual S3 bucket name and `us-west-2` with your AWS region.

### Step 2: Generate Access Token

```bash
# Generate a secure random token
TOKEN=$(openssl rand -base64 32)
echo "Your access token: $TOKEN"

# Save it for later
export NX_CACHE_TOKEN="$TOKEN"
```

### Step 3: Set AWS Credentials

Choose one of these methods:

#### Option A: Use IAM Role (Recommended for AWS infrastructure)

If running on EC2, ECS, or Lambda, no credentials needed - the IAM role will be used automatically!

#### Option B: Use AWS CLI Configuration

```bash
# Configure AWS CLI (if not already done)
aws configure
```

#### Option C: Use Environment Variables

```bash
export AWS_ACCESS_KEY_ID="your-access-key-id"
export AWS_SECRET_ACCESS_KEY="your-secret-access-key"
```

## Run the Server

```bash
# Make sure NX_CACHE_TOKEN is set
export NX_CACHE_TOKEN="your-token-from-step-2"

# Start the server
nx-cache-server --config config.yaml
```

You should see:

```
2024-01-15T10:00:00.000000Z  INFO nx_cache_server: Loading configuration from: config.yaml
2024-01-15T10:00:00.000000Z  INFO nx_cache_server: Configuration loaded successfully
2024-01-15T10:00:00.000000Z  INFO nx_cache_server:   Buckets: 1
2024-01-15T10:00:00.000000Z  INFO nx_cache_server:     - main (my-nx-cache-bucket)
2024-01-15T10:00:00.000000Z  INFO nx_cache_server:   Service Tokens: 1
2024-01-15T10:00:00.000000Z  INFO nx_cache_server:     - default -> bucket: main, prefix: 
2024-01-15T10:00:00.000000Z  INFO nx_cache_server: Storage initialized successfully
2024-01-15T10:00:00.000000Z  INFO nx_cache_server: Server starting on port 3000
2024-01-15T10:00:00.000000Z  INFO nx_cache_server::server: Server starting with 1 configured token(s)
2024-01-15T10:00:00.000000Z  INFO nx_cache_server::server:   - Token configured: default
2024-01-15T10:00:00.000000Z  INFO nx_cache_server::server: Server running on port 3000
```

## Configure Your Nx Workspace

In your Nx workspace, set the access token:

```bash
# In your Nx monorepo directory
export NX_CLOUD_AUTH_TOKEN="$NX_CACHE_TOKEN"
```

Or add to your `nx.json`:

```json
{
  "tasksRunnerOptions": {
    "default": {
      "runner": "nx/tasks-runners/default",
      "options": {
        "cacheableOperations": ["build", "test", "lint"],
        "accessToken": "your-token-from-step-2"
      }
    }
  },
  "nxCloudUrl": "http://localhost:3000"
}
```

## Test It!

```bash
# In your Nx workspace
nx build your-project

# Run it again - should be instant from cache!
nx build your-project
```

## Verify It's Working

### Check Server Logs

Look for authentication and cache hit/miss messages:

```
2024-01-15T10:05:00.000000Z  INFO nx_cache_server::server::middleware: Authenticated request from: default (bucket: main, prefix: )
```

### Health Check

```bash
curl http://localhost:3000/health
# Should return: OK
```

### Manual Cache Test

```bash
# Store an artifact
echo "test content" > test.txt
HASH=$(sha256sum test.txt | cut -d' ' -f1)
curl -X PUT "http://localhost:3000/v1/cache/$HASH" \
  -H "Authorization: Bearer $NX_CACHE_TOKEN" \
  --data-binary @test.txt

# Retrieve it
curl "http://localhost:3000/v1/cache/$HASH" \
  -H "Authorization: Bearer $NX_CACHE_TOKEN"
# Should output: test content
```

## What's Next?

### Advanced Configurations

**Multiple Teams with Prefixes:**

```yaml
buckets:
  - name: shared
    bucketName: company-nx-cache
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

**Multiple Buckets (Prod + Staging):**

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

**MinIO (Local Development):**

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

### Production Deployment

See the [complete documentation](yaml-configuration.md) for:
- Kubernetes deployment examples
- Docker configurations
- Security best practices
- Monitoring and logging
- Performance tuning

## Troubleshooting

### Error: "Failed to load configuration file"

**Problem:** Config file not found or invalid YAML syntax.

**Solution:**
```bash
# Check file exists
ls -la config.yaml

# Validate YAML syntax online or with:
python3 -c "import yaml; yaml.safe_load(open('config.yaml'))"
```

### Error: "Environment variable not found: NX_CACHE_TOKEN"

**Problem:** Required environment variable not set.

**Solution:**
```bash
export NX_CACHE_TOKEN="your-token-here"
```

### Error: "Failed to initialize storage"

**Problem:** AWS credentials invalid or bucket doesn't exist.

**Solutions:**
1. Verify bucket exists: `aws s3 ls s3://your-bucket-name`
2. Check credentials: `aws sts get-caller-identity`
3. Verify region is correct in config.yaml

### Error: "Authentication failed: invalid token"

**Problem:** Token mismatch between server and client.

**Solution:**
```bash
# Server token (in config)
echo $NX_CACHE_TOKEN

# Client token (in Nx workspace)
echo $NX_CLOUD_AUTH_TOKEN

# These MUST match exactly!
```

### Nx Client Not Using Cache Server

**Problem:** Nx still hitting default cloud or not caching at all.

**Solution:**
```bash
# Set the cache URL
export NX_CLOUD_URL="http://localhost:3000"

# Or in nx.json
{
  "nxCloudUrl": "http://localhost:3000"
}
```

## Getting Help

- 📚 [Full YAML Configuration Guide](yaml-configuration.md)
- 📋 [Example Configurations](../examples/)
- 🐛 [Report Issues](https://github.com/philiplehmann/nx-cache-server/issues)
- 💬 [Discussions](https://github.com/philiplehmann/nx-cache-server/discussions)

## Success! 🎉

Your Nx cache server is now running and your builds should be blazing fast with shared caching! The first build will populate the cache, and subsequent builds will be near-instant.

Enjoy your productivity boost! ⚡
