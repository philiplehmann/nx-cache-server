use async_trait::async_trait;
use minio::s3::builders::ObjectContent;
use minio::s3::creds::StaticProvider;
use minio::s3::http::BaseUrl;
use minio::s3::types::S3Api;
use minio::s3::Client;
use std::str::FromStr;
use tokio::io::AsyncRead;
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

    let base_url = BaseUrl::from_str(&endpoint).map_err(|e| {
      tracing::error!("Invalid MinIO endpoint URL: {:?}", e);
      StorageError::OperationFailed
    })?;

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
        if err_msg.contains("404") || err_msg.contains("Not Found") || err_msg.contains("NoSuchKey")
        {
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
    let response = self
      .client
      .get_object(&self.bucket_name, hash)
      .send()
      .await
      .map_err(|e| {
        let err_msg = e.to_string();
        if err_msg.contains("404") || err_msg.contains("Not Found") || err_msg.contains("NoSuchKey")
        {
          StorageError::NotFound
        } else {
          tracing::error!("MinIO get_object failed: {:?}", e);
          StorageError::OperationFailed
        }
      })?;

    let (stream, _size) = response.content.to_stream().await.map_err(|e| {
      tracing::error!("Error streaming MinIO response content: {:?}", e);
      StorageError::OperationFailed
    })?;

    let reader = StreamReader::new(stream);
    Ok(Box::new(reader))
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
