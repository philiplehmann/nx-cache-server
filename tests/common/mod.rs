//! Common test utilities for integration tests
//!
//! This module provides reusable helpers for setting up testcontainers
//! and creating test fixtures using the MinIO Rust SDK.

use minio::s3::creds::StaticProvider;
use minio::s3::http::BaseUrl;
use minio::s3::types::S3Api;
use minio::s3::Client;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::minio::MinIO;

use nx_cache_server::infra::minio::{MinioStorage, MinioStorageConfig};

/// MinIO test container wrapper with helper methods
#[allow(dead_code)]
pub struct MinioTestContainer {
  pub container: testcontainers::ContainerAsync<MinIO>,
  pub host_port: u16,
  pub access_key: String,
  pub secret_key: String,
}

impl MinioTestContainer {
  /// Start a new MinIO container with default credentials
  pub async fn start() -> Self {
    let minio_image = MinIO::default();
    let container = minio_image
      .start()
      .await
      .expect("Failed to start MinIO container");
    let host_port = container
      .get_host_port_ipv4(9000)
      .await
      .expect("Failed to get MinIO port");

    Self {
      container,
      host_port,
      access_key: "minioadmin".to_string(),
      secret_key: "minioadmin".to_string(),
    }
  }

  /// Get the endpoint URL for this MinIO instance
  pub fn endpoint_url(&self) -> String {
    format!("http://localhost:{}", self.host_port)
  }

  /// Create a storage config for this MinIO instance
  #[allow(dead_code)]
  pub fn create_storage_config(&self, bucket_name: String) -> MinioStorageConfig {
    MinioStorageConfig {
      endpoint: self.endpoint_url(),
      access_key: self.access_key.clone(),
      secret_key: self.secret_key.clone(),
      bucket_name,
      region: "us-east-1".to_string(),
      use_ssl: false,
    }
  }

  /// Create a MinIO client for bucket management
  pub async fn create_minio_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let base_url = self.endpoint_url().parse::<BaseUrl>()?;
    let static_provider = StaticProvider::new(&self.access_key, &self.secret_key, None);
    let client = Client::new(base_url, Some(Box::new(static_provider)), None, None)?;
    Ok(client)
  }

  /// Create a bucket in this MinIO instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_minio_client().await?;

    // Check if bucket exists first
    let exists = client.bucket_exists(bucket_name).send().await?.exists;

    if !exists {
      client.create_bucket(bucket_name).send().await?;
    }

    Ok(())
  }

  /// Create a bucket and return a configured MinioStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<MinioStorage, Box<dyn std::error::Error>> {
    // Wait a bit for MinIO to be fully ready
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Create bucket
    self.create_bucket(bucket_name).await?;

    // Create storage config
    let config = self.create_storage_config(bucket_name.to_string());

    // Create MinioStorage instance
    MinioStorage::new(&config)
      .await
      .map_err(|e| format!("Failed to create MinioStorage: {:?}", e).into())
  }

  /// List objects in a bucket using MinIO client
  #[allow(dead_code)]
  pub async fn list_objects(
    &self,
    bucket_name: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = self.create_minio_client().await?;

    use futures_util::StreamExt;
    use minio::s3::types::ToStream;

    let mut stream = client
      .list_objects(bucket_name)
      .recursive(true)
      .to_stream()
      .await;

    let mut keys = Vec::new();
    while let Some(result) = stream.next().await {
      let response = result?;
      for item in response.contents {
        keys.push(item.name);
      }
    }

    Ok(keys)
  }

  /// Check if an object exists using MinIO client
  #[allow(dead_code)]
  pub async fn object_exists(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<bool, Box<dyn std::error::Error>> {
    let client = self.create_minio_client().await?;

    let result = client.stat_object(bucket_name, object_name).send().await;

    Ok(result.is_ok())
  }

  /// Put an object using MinIO client with raw bytes
  #[allow(dead_code)]
  pub async fn put_object(
    &self,
    bucket_name: &str,
    object_name: &str,
    data: Vec<u8>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_minio_client().await?;

    use minio::s3::builders::ObjectContent;
    let content = ObjectContent::from(data);

    client
      .put_object_content(bucket_name, object_name, content)
      .send()
      .await?;

    Ok(())
  }

  /// Get an object using MinIO client
  #[allow(dead_code)]
  pub async fn get_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let client = self.create_minio_client().await?;

    let response = client.get_object(bucket_name, object_name).send().await?;

    // Get the content from the response and convert to bytes
    let segmented = response.content.to_segmented_bytes().await?;
    let bytes = segmented.to_bytes();

    Ok(bytes.to_vec())
  }

  /// Delete an object using MinIO client
  #[allow(dead_code)]
  pub async fn delete_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_minio_client().await?;

    use minio::s3::builders::ObjectToDelete;

    client
      .delete_object(bucket_name, ObjectToDelete::from(object_name))
      .send()
      .await?;

    Ok(())
  }
}

/// Helper to generate unique bucket names for tests
pub fn unique_bucket_name(prefix: &str) -> String {
  use std::time::{SystemTime, UNIX_EPOCH};
  let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap()
    .as_millis();
  format!("{}-{}", prefix, timestamp)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_unique_bucket_name() {
    let name1 = unique_bucket_name("test");
    // Sleep to ensure different timestamps
    std::thread::sleep(std::time::Duration::from_millis(2));
    let name2 = unique_bucket_name("test");
    assert_ne!(name1, name2, "Bucket names should be unique");
    assert!(name1.starts_with("test-"));
  }
}
