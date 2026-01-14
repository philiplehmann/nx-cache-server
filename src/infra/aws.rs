use async_trait::async_trait;
use aws_config::default_provider::credentials::DefaultCredentialsChain;
use aws_config::meta::region::future::ProvideRegion as ProvideRegionFuture;
use aws_config::meta::region::{ProvideRegion, RegionProviderChain};
use aws_credential_types::provider::future::ProvideCredentials as ProvideCredentialsFuture;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{Credentials, ProvideCredentials};
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::operation::head_object::HeadObjectError;
use aws_sdk_s3::{config::Region, Client, Config as S3Config};
use clap::Parser;
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;

use crate::domain::{
    config::{ConfigError, ConfigValidator},
    storage::{StorageError, StorageProvider},
    yaml_config::ResolvedBucketConfig,
};

#[derive(Parser, Debug, Clone)]
pub struct AwsStorageConfig {
    #[arg(
        long,
        env = "AWS_REGION",
        help = "AWS region (e.g., us-west-2). Auto-discovered from environment, AWS config, or EC2/ECS metadata if not provided"
    )]
    pub region: Option<String>,

    #[arg(
        long,
        env = "AWS_ACCESS_KEY_ID",
        help = "AWS access key ID. Optional - uses AWS credential provider chain (environment, config file, IAM roles) if not provided"
    )]
    pub access_key_id: Option<String>,

    #[arg(
        long,
        env = "AWS_SECRET_ACCESS_KEY",
        help = "AWS secret access key. Required if --access-key-id is provided"
    )]
    pub secret_access_key: Option<String>,

    #[arg(
        long,
        env = "AWS_SESSION_TOKEN",
        help = "AWS session token for temporary security credentials. Optional"
    )]
    pub session_token: Option<String>,

    #[arg(
        long,
        env = "S3_BUCKET_NAME",
        help = "S3 bucket name for cache storage"
    )]
    pub bucket_name: String,

    #[arg(
        long,
        env = "S3_ENDPOINT_URL",
        help = "Custom S3 endpoint URL (e.g., http://localhost:9000 for MinIO). Optional - uses AWS S3 if not provided"
    )]
    pub endpoint_url: Option<String>,

    #[arg(
        long,
        env = "S3_TIMEOUT",
        default_value = "30",
        help = "S3 operation timeout in seconds"
    )]
    pub timeout_seconds: u64,
}

impl ProvideRegion for AwsStorageConfig {
    fn region(&self) -> ProvideRegionFuture<'_> {
        let region = self.region.clone();
        ProvideRegionFuture::new(async {
            RegionProviderChain::first_try(region.map(Region::new))
                .or_default_provider()
                .region()
                .await
        })
    }
}

impl ProvideCredentials for AwsStorageConfig {
    fn provide_credentials<'a>(&'a self) -> ProvideCredentialsFuture<'a>
    where
        Self: 'a,
    {
        match (self.access_key_id.as_ref(), self.secret_access_key.as_ref()) {
            (Some(access_key_id), Some(secret_access_key)) => {
                ProvideCredentialsFuture::ready(Ok(Credentials::new(
                    access_key_id,
                    secret_access_key,
                    self.session_token.clone(),
                    None,
                    "nx-cache-server",
                )))
            }
            _ => ProvideCredentialsFuture::new(async {
                DefaultCredentialsChain::builder()
                    .region(self.clone())
                    .build()
                    .await
                    .provide_credentials()
                    .await
            }),
        }
    }
}

impl ConfigValidator for AwsStorageConfig {
    async fn validate(&self) -> Result<(), ConfigError> {
        if self.bucket_name.is_empty() {
            return Err(ConfigError::MissingField("S3_BUCKET_NAME"));
        }
        if let Some(endpoint_url) = &self.endpoint_url {
            if !endpoint_url.starts_with("http://") && !endpoint_url.starts_with("https://") {
                return Err(ConfigError::Invalid(
                    "S3 endpoint URL must start with http:// or https://",
                ));
            }
        }
        match (self.access_key_id.as_ref(), self.secret_access_key.as_ref()) {
            (Some(..), None) => return Err(ConfigError::MissingField("AWS_SECRET_ACCESS_KEY")),
            (None, Some(..)) => return Err(ConfigError::MissingField("AWS_ACCESS_KEY_ID")),
            _ => {}
        }
        if self.region().await.is_none() {
            return Err(ConfigError::MissingField("AWS_REGION"));
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct S3Storage {
    client: Client,
    bucket_name: String,
}

impl S3Storage {
    pub async fn new(config: &AwsStorageConfig) -> Result<Self, StorageError> {
        // Resolve region once - validation already ensured it exists
        let region = config.region().await.ok_or_else(|| {
            tracing::error!("AWS_REGION must be set");
            StorageError::OperationFailed
        })?;

        let mut s3_config_builder = S3Config::builder()
            .behavior_version_latest()
            .region(region)
            .credentials_provider(config.clone())
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(std::time::Duration::from_secs(config.timeout_seconds))
                    .build(),
            );

        // Configure for custom S3-compatible endpoints (MinIO, Hetzner, etc.)
        if let Some(endpoint_url) = &config.endpoint_url {
            s3_config_builder = s3_config_builder
                .endpoint_url(endpoint_url)
                .force_path_style(true); // Required for most S3-compatible services
        }

        let s3_config = s3_config_builder.build();

        let client = Client::from_conf(s3_config);

        Ok(Self {
            client,
            bucket_name: config.bucket_name.clone(),
        })
    }

    /// Create S3Storage from a resolved bucket configuration
    pub async fn from_resolved_bucket(
        bucket_config: &ResolvedBucketConfig,
    ) -> Result<Self, StorageError> {
        // Resolve region
        let region_chain = RegionProviderChain::first_try(
            bucket_config.region.as_ref().map(|r| Region::new(r.clone())),
        )
        .or_default_provider();

        let region = region_chain.region().await.ok_or_else(|| {
            tracing::error!(
                "AWS_REGION must be set for bucket '{}'",
                bucket_config.name
            );
            StorageError::OperationFailed
        })?;

        // Build credentials provider
        let credentials_provider: Arc<dyn ProvideCredentials> =
            match (&bucket_config.access_key_id, &bucket_config.secret_access_key) {
                (Some(access_key_id), Some(secret_access_key)) => {
                    Arc::new(Credentials::new(
                        access_key_id,
                        secret_access_key,
                        bucket_config.session_token.clone(),
                        None,
                        "nx-cache-server",
                    ))
                }
                _ => Arc::new(
                    DefaultCredentialsChain::builder()
                        .region(region.clone())
                        .build()
                        .await,
                ),
            };

        let mut s3_config_builder = S3Config::builder()
            .behavior_version_latest()
            .region(region)
            .credentials_provider(credentials_provider)
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(std::time::Duration::from_secs(bucket_config.timeout))
                    .build(),
            );

        // Configure for custom S3-compatible endpoints (MinIO, Hetzner, etc.)
        if let Some(endpoint_url) = &bucket_config.endpoint_url {
            s3_config_builder = s3_config_builder.endpoint_url(endpoint_url);
        }

        // Force path-style addressing if configured (required for MinIO and some S3-compatible services)
        if bucket_config.force_path_style {
            s3_config_builder = s3_config_builder.force_path_style(true);
        }

        let s3_config = s3_config_builder.build();
        let client = Client::from_conf(s3_config);

        Ok(Self {
            client,
            bucket_name: bucket_config.bucket_name.clone(),
        })
    }
}

#[async_trait]
impl StorageProvider for S3Storage {
    async fn exists(&self, hash: &str) -> Result<bool, StorageError> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket_name)
            .key(hash)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => match e.into_service_error() {
                HeadObjectError::NotFound(_) => Ok(false),
                other => {
                    tracing::error!("S3 head_object failed: {:?}", other);
                    Err(StorageError::OperationFailed)
                }
            },
        }
    }

    async fn store(
        &self,
        hash: &str,
        data: ReaderStream<impl AsyncRead + Send + Unpin + 'static>,
    ) -> Result<(), StorageError> {
        if self.exists(hash).await? {
            return Err(StorageError::AlreadyExists);
        }

        // Convert ReaderStream to ByteStream without buffering entire content
        // Use a channel to bridge the non-Sync stream to a Sync body


        // Create a channel for streaming data
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(16);

        // Spawn a task to forward the stream to the channel
        // This allows the stream processing to happen in a separate task
        tokio::spawn(async move {
            tokio::pin!(data);
            while let Some(result) = data.next().await {
                if tx.send(result).await.is_err() {
                    break;
                }
            }
        });

        // Create a stream from the receiver
        let recv_stream = tokio_stream::wrappers::ReceiverStream::new(rx);

        // Map to frames for http-body 1.0
        let frame_stream = recv_stream.map(|result| {
            result
                .map(|bytes| hyper::body::Frame::data(bytes))
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        });

        // Create a StreamBody and box it (the receiver stream is Sync)
        let stream_body = http_body_util::StreamBody::new(frame_stream);
        let boxed_body = http_body_util::combinators::BoxBody::new(stream_body);

        // Convert to AWS ByteStream using the http-body 1.0 API
        let byte_stream = aws_sdk_s3::primitives::ByteStream::from_body_1_x(boxed_body);

        self.client
            .put_object()
            .bucket(&self.bucket_name)
            .key(hash)
            .body(byte_stream)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("S3 put_object failed: {:?}", e);
                StorageError::OperationFailed
            })?;

        Ok(())
    }

    async fn retrieve(
        &self,
        hash: &str,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>, StorageError> {
        let result = self
            .client
            .get_object()
            .bucket(&self.bucket_name)
            .key(hash)
            .send()
            .await
            .map_err(|e| match e.into_service_error() {
                GetObjectError::NoSuchKey(_) => StorageError::NotFound,
                other => {
                    tracing::error!("S3 get_object failed: {:?}", other);
                    StorageError::OperationFailed
                }
            })?;

        // Direct streaming - no buffering
        Ok(Box::new(result.body.into_async_read()))
    }
}

impl S3Storage {
    /// Test bucket connectivity by performing a list_objects_v2 operation
    /// This verifies that credentials are valid and the bucket is accessible
    pub async fn test_connection(&self) -> Result<(), StorageError> {
        tracing::debug!("Testing connection to bucket: {}", self.bucket_name);

        self.client
            .list_objects_v2()
            .bucket(&self.bucket_name)
            .max_keys(1) // Only need to list one object to verify connectivity
            .send()
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to connect to bucket '{}': {:?}",
                    self.bucket_name,
                    e
                );
                StorageError::OperationFailed
            })?;

        tracing::info!("Successfully connected to bucket: {}", self.bucket_name);
        Ok(())
    }
}
