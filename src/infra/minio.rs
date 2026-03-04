use async_trait::async_trait;
use minio::s3::builders::ObjectContent;
use minio::s3::creds::StaticProvider;
use minio::s3::http::BaseUrl;
use minio::s3::types::S3Api;
use minio::s3::Client;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncRead;
use tokio::time::sleep;
use tokio_util::io::{ReaderStream, StreamReader};

use crate::domain::{
  storage::{StorageError, StorageProvider},
  yaml_config::ResolvedBucketConfig,
};

#[derive(Clone)]
pub struct MinioStorage {
  client: Client,
  bucket_name: String,
}

impl MinioStorage {
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

  /// Create MinioStorage from a resolved bucket configuration
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

    let client =
      Client::new(base_url, Some(Box::new(static_provider)), None, None).map_err(|e| {
        tracing::error!("Failed to create MinIO client: {:?}", e);
        StorageError::OperationFailed
      })?;

    Ok(Self {
      client,
      bucket_name: bucket_config.bucket_name.clone(),
    })
  }
}

#[async_trait]
impl StorageProvider for MinioStorage {
  async fn exists(&self, hash: &str) -> Result<bool, StorageError> {
    match self
      .client
      .stat_object(&self.bucket_name, hash)
      .send()
      .await
    {
      Ok(_) => Ok(true),
      Err(e) => {
        let err_msg = e.to_string();
        // MinIO returns 404 for non-existent objects
        if Self::is_not_found_error(&err_msg) {
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

    self
      .client
      .put_object_content(&self.bucket_name, hash, content)
      .send()
      .await
      .map_err(|e| {
        tracing::error!("MinIO put_object_content failed: {:?}", e);
        StorageError::OperationFailed
      })?;

    Ok(())
  }

  async fn retrieve(&self, hash: &str) -> Result<Box<dyn AsyncRead + Send + Unpin>, StorageError> {
    const MAX_ATTEMPTS: usize = 3;

    for attempt in 1..=MAX_ATTEMPTS {
      let response = match self.client.get_object(&self.bucket_name, hash).send().await {
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

impl MinioStorage {
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
