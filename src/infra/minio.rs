use async_trait::async_trait;
use futures_util::StreamExt;
use minio::s3::builders::ObjectContent;
use minio::s3::creds::StaticProvider;
use minio::s3::http::BaseUrl;
use minio::s3::sse::{Sse, SseCustomerKey, SseKms, SseS3};
use minio::s3::types::{S3Api, ToStream};
use minio::s3::Client;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncRead;
use tokio::time::sleep;
use tokio_util::io::{ReaderStream, StreamReader};

use crate::domain::{
  config::{ResolvedBucketConfig, ResolvedSseConfig},
  storage::{StorageError, StorageProvider},
};

#[derive(Clone)]
pub struct NxCacheStorage {
  client: Client,
  bucket_name: String,
  sse: Option<Arc<dyn Sse>>,
  sse_customer_key: Option<SseCustomerKey>,
}

impl NxCacheStorage {
  fn is_not_found_error(error_message: &str) -> bool {
    error_message.contains("404")
      || error_message.contains("Not Found")
      || error_message.contains("NoSuchKey")
  }

  fn is_retryable_error(error_message: &str) -> bool {
    let message = error_message.to_ascii_lowercase();
    message.contains("incompletemessage")
      || message.contains("timed out")
      || message.contains("timeout")
      || message.contains("connection reset")
      || message.contains("connection closed")
      || message.contains("broken pipe")
      || message.contains("sendrequest")
  }

  fn retry_delay(attempt: usize) -> Duration {
    let base_ms = match attempt {
      1 => 100,
      2 => 300,
      _ => 900,
    };
    let jitter_ms = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|duration| (duration.subsec_nanos() % 50) as u64)
      .unwrap_or(0);
    Duration::from_millis(base_ms + jitter_ms)
  }

  /// Create NxCacheStorage from a resolved bucket configuration
  pub async fn from_resolved_bucket(
    bucket_config: &ResolvedBucketConfig,
  ) -> Result<Self, StorageError> {
    let endpoint = bucket_config
      .endpoint_url
      .as_ref()
      .ok_or_else(|| {
        tracing::error!("MinIO endpoint URL is required");
        StorageError::OperationFailed
      })?
      .clone();

    let mut base_url = BaseUrl::from_str(&endpoint).map_err(|e| {
      tracing::error!("Invalid MinIO endpoint URL: {:?}", e);
      StorageError::OperationFailed
    })?;
    if let Some(region) = &bucket_config.region {
      base_url.region = region.clone();
    }
    if bucket_config.force_path_style {
      base_url.virtual_style = false;
    }

    let access_key = bucket_config.access_key_id.as_ref().ok_or_else(|| {
      tracing::error!("MinIO access key is required");
      StorageError::OperationFailed
    })?;

    let secret_key = bucket_config.secret_access_key.as_ref().ok_or_else(|| {
      tracing::error!("MinIO secret key is required");
      StorageError::OperationFailed
    })?;

    let static_provider = StaticProvider::new(access_key, secret_key, None);

    let (sse, sse_customer_key) = match &bucket_config.sse {
      None => (None, None),
      Some(ResolvedSseConfig::SseS3) => (Some(Arc::new(SseS3::new()) as Arc<dyn Sse>), None),
      Some(ResolvedSseConfig::SseKms { key_id, context }) => {
        let sse = SseKms::new(key_id, context.as_deref());
        (Some(Arc::new(sse) as Arc<dyn Sse>), None)
      },
      Some(ResolvedSseConfig::SseC { key }) => {
        let ssec = SseCustomerKey::new(key);
        let sse: Arc<dyn Sse> = Arc::new(ssec.clone());
        (Some(sse), Some(ssec))
      },
    };

    let ssl_cert_file = std::env::var("SSL_CERT_FILE")
      .ok()
      .map(|value| value.trim().to_string())
      .filter(|value| !value.is_empty())
      .map(PathBuf::from)
      .filter(|path| path.is_file());

    let ignore_cert_check = std::env::var("NX_CACHE_SERVER_INSECURE_TLS")
      .ok()
      .map(|value| value.trim().to_ascii_lowercase())
      .and_then(|value| match value.as_str() {
        "1" | "true" | "yes" | "y" => Some(true),
        "0" | "false" | "no" | "n" => Some(false),
        _ => None,
      });

    let client = Client::new(
      base_url,
      Some(Box::new(static_provider)),
      ssl_cert_file.as_deref(),
      ignore_cert_check,
    )
    .map_err(|e| {
      tracing::error!("Failed to create MinIO client: {:?}", e);
      StorageError::OperationFailed
    })?;

    Ok(Self {
      client,
      bucket_name: bucket_config.bucket_name.clone(),
      sse,
      sse_customer_key,
    })
  }
}

#[async_trait]
impl StorageProvider for NxCacheStorage {
  async fn exists(&self, hash: &str) -> Result<bool, StorageError> {
    match self
      .client
      .stat_object(&self.bucket_name, hash)
      .ssec(self.sse_customer_key.clone())
      .send()
      .await
    {
      Ok(_) => Ok(true),
      Err(e) => {
        let err_msg = e.to_string();
        // MinIO returns 404 for non-existent objects
        if Self::is_not_found_error(&err_msg) {
          Ok(false)
        } else if self.sse_customer_key.is_some() {
          tracing::debug!(
            "MinIO stat_object failed with SSE-C, falling back to list_objects: {:?}",
            e
          );

          let mut stream = self
            .client
            .list_objects(&self.bucket_name)
            .recursive(true)
            .to_stream()
            .await;

          while let Some(result) = stream.next().await {
            let response = result.map_err(|e| {
              tracing::error!("MinIO list_objects failed: {:?}", e);
              StorageError::OperationFailed
            })?;

            if response.contents.iter().any(|item| item.name == hash) {
              return Ok(true);
            }
          }

          Ok(false)
        } else {
          tracing::error!("MinIO stat_object failed: {:?}", e);
          Err(StorageError::OperationFailed)
        }
      },
    }
  }

  async fn store(
    &self,
    hash: &str,
    data: ReaderStream<impl AsyncRead + Send + Unpin + 'static>,
    content_length: Option<u64>,
  ) -> Result<(), StorageError> {
    if self.exists(hash).await? {
      return Err(StorageError::AlreadyExists);
    }

    let content = ObjectContent::new_from_stream(data, content_length);
    let sse_enabled = self.sse.is_some();
    let sse_customer_key_enabled = self.sse_customer_key.is_some();

    self
      .client
      .put_object_content(&self.bucket_name, hash, content)
      .sse(self.sse.clone())
      .send()
      .await
      .map_err(|e| {
        tracing::error!(
          "MinIO put_object_content failed (sse_enabled={}, sse_customer_key_enabled={}): {:?}",
          sse_enabled,
          sse_customer_key_enabled,
          e
        );
        StorageError::OperationFailed
      })?;

    Ok(())
  }

  async fn retrieve(&self, hash: &str) -> Result<Box<dyn AsyncRead + Send + Unpin>, StorageError> {
    const MAX_ATTEMPTS: usize = 3;

    for attempt in 1..=MAX_ATTEMPTS {
      let response = match self
        .client
        .get_object(&self.bucket_name, hash)
        .ssec(self.sse_customer_key.clone())
        .send()
        .await
      {
        Ok(response) => response,
        Err(e) => {
          let err_msg = e.to_string();
          if Self::is_not_found_error(&err_msg) {
            return Err(StorageError::NotFound);
          }

          if Self::is_retryable_error(&err_msg) && attempt < MAX_ATTEMPTS {
            let delay = Self::retry_delay(attempt);
            tracing::debug!(
              "MinIO get_object transient error, retrying (attempt {}/{}, delay {:?}): {:?}",
              attempt,
              MAX_ATTEMPTS,
              delay,
              e
            );
            sleep(delay).await;
            continue;
          }

          tracing::error!("MinIO get_object failed: {:?}", e);
          return Err(StorageError::OperationFailed);
        },
      };

      let (stream, _size) = match response.content.to_stream().await {
        Ok((stream, size)) => (stream, size),
        Err(e) => {
          let err_msg = e.to_string();
          if Self::is_retryable_error(&err_msg) && attempt < MAX_ATTEMPTS {
            let delay = Self::retry_delay(attempt);
            tracing::debug!(
              "MinIO stream transient error, retrying (attempt {}/{}, delay {:?}): {:?}",
              attempt,
              MAX_ATTEMPTS,
              delay,
              e
            );
            sleep(delay).await;
            continue;
          }

          tracing::error!("Error streaming MinIO response content: {:?}", e);
          return Err(StorageError::OperationFailed);
        },
      };

      let reader = StreamReader::new(stream);
      return Ok(Box::new(reader));
    }

    Err(StorageError::OperationFailed)
  }
}

impl NxCacheStorage {
  /// Test bucket connectivity by checking if bucket exists
  /// This verifies that credentials are valid and the bucket is accessible
  pub async fn test_connection(&self) -> Result<(), StorageError> {
    tracing::debug!("Testing connection to bucket: {}", self.bucket_name);

    // Check if bucket exists
    let exists = self
      .client
      .bucket_exists(&self.bucket_name)
      .send()
      .await
      .map_err(|e| {
        tracing::error!("Failed to check bucket '{}': {:?}", self.bucket_name, e);
        StorageError::OperationFailed
      })?;

    if !exists.exists {
      tracing::error!("Bucket '{}' does not exist", self.bucket_name);
      return Err(StorageError::OperationFailed);
    }

    tracing::info!("Successfully connected to bucket: {}", self.bucket_name);
    Ok(())
  }
}
