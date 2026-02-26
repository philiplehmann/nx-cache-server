//! Common test utilities for integration tests
//!
//! This module provides reusable helpers for setting up testcontainers
//! and creating test fixtures using the MinIO Rust SDK.

pub mod storage_contract;

use minio::s3::creds::StaticProvider;
use minio::s3::http::BaseUrl;
use minio::s3::types::S3Api;
use minio::s3::Client;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use testcontainers::runners::AsyncRunner;
use testcontainers::{core::ContainerPort, core::ExecCommand, core::Mount, GenericImage, ImageExt};
use testcontainers_modules::minio::MinIO;
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use nx_cache_server::domain::storage::StorageProvider;
use nx_cache_server::domain::yaml_config::ResolvedBucketConfig;
use nx_cache_server::infra::minio::MinioStorage;

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
  #[allow(dead_code)]
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
  pub fn create_storage_config(&self, bucket_name: String) -> ResolvedBucketConfig {
    ResolvedBucketConfig {
      name: "test".to_string(),
      bucket_name,
      access_key_id: Some(self.access_key.clone()),
      secret_access_key: Some(self.secret_key.clone()),
      session_token: None,
      region: Some("us-east-1".to_string()),
      endpoint_url: Some(self.endpoint_url()),
      force_path_style: true,
      timeout: 30,
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
    MinioStorage::from_resolved_bucket(&config)
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

/// RustFS test container wrapper with helper methods
#[allow(dead_code)]
pub struct RustfsTestContainer {
  pub container: testcontainers::ContainerAsync<GenericImage>,
  pub host_port: u16,
  pub access_key: String,
  pub secret_key: String,
}

impl RustfsTestContainer {
  /// Start a new RustFS container with default credentials
  #[allow(dead_code)]
  pub async fn start() -> Self {
    let access_key = "rustfsadmin".to_string();
    let secret_key = "rustfsadmin".to_string();

    let rustfs_image = GenericImage::new("rustfs/rustfs", "1.0.0-alpha.83")
      .with_exposed_port(ContainerPort::Tcp(9000))
      .with_env_var("RUSTFS_ACCESS_KEY", access_key.clone())
      .with_env_var("RUSTFS_SECRET_KEY", secret_key.clone());
    let container = rustfs_image
      .start()
      .await
      .expect("Failed to start RustFS container");
    let host_port = container
      .get_host_port_ipv4(9000)
      .await
      .expect("Failed to get RustFS port");

    Self {
      container,
      host_port,
      access_key,
      secret_key,
    }
  }

  /// Get the endpoint URL for this RustFS instance
  pub fn endpoint_url(&self) -> String {
    format!("http://localhost:{}", self.host_port)
  }

  /// Create a storage config for this RustFS instance
  #[allow(dead_code)]
  pub fn create_storage_config(&self, bucket_name: String) -> ResolvedBucketConfig {
    ResolvedBucketConfig {
      name: "test".to_string(),
      bucket_name,
      access_key_id: Some(self.access_key.clone()),
      secret_access_key: Some(self.secret_key.clone()),
      session_token: None,
      region: Some("us-east-1".to_string()),
      endpoint_url: Some(self.endpoint_url()),
      force_path_style: true,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_rustfs_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let base_url = self.endpoint_url().parse::<BaseUrl>()?;
    let static_provider = StaticProvider::new(&self.access_key, &self.secret_key, None);
    let client = Client::new(base_url, Some(Box::new(static_provider)), None, None)?;
    Ok(client)
  }

  /// Create a bucket in this RustFS instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_rustfs_client().await?;

    let max_retries = 5;
    let retry_delay = tokio::time::Duration::from_millis(500);

    for attempt in 0..max_retries {
      // Check if bucket exists first
      let exists = match client.bucket_exists(bucket_name).send().await {
        Ok(response) => response.exists,
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
          continue;
        },
      };

      if exists {
        return Ok(());
      }

      match client.create_bucket(bucket_name).send().await {
        Ok(_) => return Ok(()),
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
        },
      }
    }

    Ok(())
  }

  /// Create a bucket and return a configured MinioStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<MinioStorage, Box<dyn std::error::Error>> {
    // Wait a bit for RustFS to be fully ready
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Create bucket
    self.create_bucket(bucket_name).await?;

    // Create storage config
    let config = self.create_storage_config(bucket_name.to_string());

    // Create MinioStorage instance (S3-compatible)
    MinioStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| format!("Failed to create MinioStorage: {:?}", e).into())
  }

  /// List objects in a bucket using RustFS client
  #[allow(dead_code)]
  pub async fn list_objects(
    &self,
    bucket_name: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = self.create_rustfs_client().await?;

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

  /// Check if an object exists using RustFS client
  #[allow(dead_code)]
  pub async fn object_exists(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<bool, Box<dyn std::error::Error>> {
    let client = self.create_rustfs_client().await?;

    let result = client.stat_object(bucket_name, object_name).send().await;

    Ok(result.is_ok())
  }

  /// Put an object using RustFS client with raw bytes
  #[allow(dead_code)]
  pub async fn put_object(
    &self,
    bucket_name: &str,
    object_name: &str,
    data: Vec<u8>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_rustfs_client().await?;

    use minio::s3::builders::ObjectContent;
    let content = ObjectContent::from(data);

    client
      .put_object_content(bucket_name, object_name, content)
      .send()
      .await?;

    Ok(())
  }

  /// Get an object using RustFS client
  #[allow(dead_code)]
  pub async fn get_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let client = self.create_rustfs_client().await?;

    let response = client.get_object(bucket_name, object_name).send().await?;

    // Get the content from the response and convert to bytes
    let segmented = response.content.to_segmented_bytes().await?;
    let bytes = segmented.to_bytes();

    Ok(bytes.to_vec())
  }

  /// Delete an object using RustFS client
  #[allow(dead_code)]
  pub async fn delete_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_rustfs_client().await?;

    use minio::s3::builders::ObjectToDelete;

    client
      .delete_object(bucket_name, ObjectToDelete::from(object_name))
      .send()
      .await?;

    Ok(())
  }
}

static SEAWEEDFS_TEST_MUTEX: OnceLock<Arc<Mutex<()>>> = OnceLock::new();

/// SeaweedFS test container wrapper with helper methods
#[allow(dead_code)]
pub struct SeaweedfsTestContainer {
  pub container: testcontainers::ContainerAsync<GenericImage>,
  pub host_port: u16,
  pub access_key: String,
  pub secret_key: String,
  _seaweedfs_lock: tokio::sync::OwnedMutexGuard<()>,
}

impl SeaweedfsTestContainer {
  /// Start a new SeaweedFS container with default credentials
  #[allow(dead_code)]
  pub async fn start() -> Self {
    let lock = SEAWEEDFS_TEST_MUTEX
      .get_or_init(|| Arc::new(Mutex::new(())))
      .clone()
      .lock_owned()
      .await;

    let access_key = "admin".to_string();
    let secret_key = "key".to_string();

    let host_port = std::net::TcpListener::bind("127.0.0.1:0")
      .expect("Failed to bind random host port for SeaweedFS")
      .local_addr()
      .expect("Failed to read bound port for SeaweedFS")
      .port();

    let seaweedfs_image = GenericImage::new("chrislusf/seaweedfs", "latest")
      .with_mapped_port(host_port, ContainerPort::Tcp(8333))
      .with_env_var("AWS_ACCESS_KEY_ID", access_key.clone())
      .with_env_var("AWS_SECRET_ACCESS_KEY", secret_key.clone())
      .with_cmd(["server", "-s3", "-dir=/data"]);
    let container = seaweedfs_image
      .start()
      .await
      .expect("Failed to start SeaweedFS container");

    let readiness_retries = 30;
    let readiness_delay = tokio::time::Duration::from_millis(500);
    for attempt in 0..readiness_retries {
      if TcpStream::connect(format!("127.0.0.1:{}", host_port))
        .await
        .is_ok()
      {
        break;
      }
      if attempt + 1 == readiness_retries {
        panic!("SeaweedFS S3 endpoint not ready");
      }
      tokio::time::sleep(readiness_delay).await;
    }

    Self {
      container,
      host_port,
      access_key,
      secret_key,
      _seaweedfs_lock: lock,
    }
  }

  /// Get the endpoint URL for this SeaweedFS instance
  pub fn endpoint_url(&self) -> String {
    format!("http://localhost:{}", self.host_port)
  }

  /// Create a storage config for this SeaweedFS instance
  #[allow(dead_code)]
  pub fn create_storage_config(&self, bucket_name: String) -> ResolvedBucketConfig {
    ResolvedBucketConfig {
      name: "test".to_string(),
      bucket_name,
      access_key_id: Some(self.access_key.clone()),
      secret_access_key: Some(self.secret_key.clone()),
      session_token: None,
      region: Some("us-east-1".to_string()),
      endpoint_url: Some(self.endpoint_url()),
      force_path_style: true,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_seaweedfs_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "us-east-1".to_string();
    base_url.virtual_style = false;
    let static_provider = StaticProvider::new(&self.access_key, &self.secret_key, None);
    let client = Client::new(base_url, Some(Box::new(static_provider)), None, None)?;
    Ok(client)
  }

  /// Create a bucket in this SeaweedFS instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_seaweedfs_client().await?;

    let max_retries = 20;
    let retry_delay = tokio::time::Duration::from_secs(1);

    for attempt in 0..max_retries {
      let exists = match client.bucket_exists(bucket_name).send().await {
        Ok(response) => response.exists,
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
          continue;
        },
      };

      if exists {
        return Ok(());
      }

      match client.create_bucket(bucket_name).send().await {
        Ok(_) => break,
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
        },
      }
    }

    let readiness_retries = 10;
    let readiness_delay = tokio::time::Duration::from_millis(500);
    for attempt in 0..readiness_retries {
      let exists = client.bucket_exists(bucket_name).send().await?.exists;
      if exists {
        return Ok(());
      }
      if attempt + 1 == readiness_retries {
        break;
      }
      tokio::time::sleep(readiness_delay).await;
    }

    Err("SeaweedFS bucket not ready after creation".into())
  }

  /// Create a bucket and return a configured MinioStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<MinioStorage, Box<dyn std::error::Error>> {
    // Wait a bit for SeaweedFS to be fully ready
    tokio::time::sleep(tokio::time::Duration::from_secs(8)).await;

    // Create bucket
    self.create_bucket(bucket_name).await?;

    // Create storage config
    let config = self.create_storage_config(bucket_name.to_string());

    // Create MinioStorage instance (S3-compatible)
    let storage = MinioStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| {
        Box::<dyn std::error::Error>::from(format!("Failed to create MinioStorage: {:?}", e))
      })?;

    let max_retries = 10;
    let retry_delay = tokio::time::Duration::from_secs(1);
    for attempt in 0..max_retries {
      if storage.test_connection().await.is_ok() {
        return Ok(storage);
      }
      if attempt + 1 == max_retries {
        break;
      }
      tokio::time::sleep(retry_delay).await;
    }

    Err("SeaweedFS bucket not ready after creation".into())
  }

  /// List objects in a bucket using SeaweedFS client
  #[allow(dead_code)]
  pub async fn list_objects(
    &self,
    bucket_name: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = self.create_seaweedfs_client().await?;

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

  /// Check if an object exists using SeaweedFS client
  #[allow(dead_code)]
  pub async fn object_exists(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<bool, Box<dyn std::error::Error>> {
    let client = self.create_seaweedfs_client().await?;

    let result = client.stat_object(bucket_name, object_name).send().await;

    Ok(result.is_ok())
  }

  /// Put an object using SeaweedFS client with raw bytes
  #[allow(dead_code)]
  pub async fn put_object(
    &self,
    bucket_name: &str,
    object_name: &str,
    data: Vec<u8>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let storage = self
      .create_storage(bucket_name)
      .await
      .map_err(|e| format!("Failed to create SeaweedFS storage: {:?}", e))?;

    let data_len = data.len() as u64;
    let cursor = std::io::Cursor::new(data);
    let reader_stream = tokio_util::io::ReaderStream::new(cursor);

    storage
      .store(object_name, reader_stream, Some(data_len))
      .await
      .map_err(|e| format!("Failed to store object via SeaweedFS storage: {:?}", e))?;

    Ok(())
  }

  /// Get an object using SeaweedFS client
  #[allow(dead_code)]
  pub async fn get_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let client = self.create_seaweedfs_client().await?;

    let response = client.get_object(bucket_name, object_name).send().await?;

    let segmented = response.content.to_segmented_bytes().await?;
    let bytes = segmented.to_bytes();

    Ok(bytes.to_vec())
  }

  /// Delete an object using SeaweedFS client
  #[allow(dead_code)]
  pub async fn delete_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_seaweedfs_client().await?;

    use minio::s3::builders::ObjectToDelete;

    client
      .delete_object(bucket_name, ObjectToDelete::from(object_name))
      .send()
      .await?;

    Ok(())
  }
}

/// LocalStack test container wrapper with helper methods
#[allow(dead_code)]
pub struct LocalstackTestContainer {
  pub container: testcontainers::ContainerAsync<GenericImage>,
  pub host_port: u16,
  pub access_key: String,
  pub secret_key: String,
}

impl LocalstackTestContainer {
  /// Start a new LocalStack container with S3 enabled
  #[allow(dead_code)]
  pub async fn start() -> Self {
    let access_key = "test".to_string();
    let secret_key = "test".to_string();

    let localstack_image = GenericImage::new("localstack/localstack", "latest")
      .with_exposed_port(ContainerPort::Tcp(4566))
      .with_env_var("SERVICES", "s3")
      .with_env_var("AWS_DEFAULT_REGION", "us-east-1")
      .with_env_var("AWS_ACCESS_KEY_ID", access_key.clone())
      .with_env_var("AWS_SECRET_ACCESS_KEY", secret_key.clone());

    let container = localstack_image
      .start()
      .await
      .expect("Failed to start LocalStack container");

    let host_port = container
      .get_host_port_ipv4(4566)
      .await
      .expect("Failed to get LocalStack port");

    let readiness_retries = 30;
    let readiness_delay = tokio::time::Duration::from_millis(500);
    for attempt in 0..readiness_retries {
      if TcpStream::connect(format!("127.0.0.1:{}", host_port))
        .await
        .is_ok()
      {
        break;
      }
      if attempt + 1 == readiness_retries {
        panic!("LocalStack S3 endpoint not ready");
      }
      tokio::time::sleep(readiness_delay).await;
    }

    Self {
      container,
      host_port,
      access_key,
      secret_key,
    }
  }

  /// Get the endpoint URL for this LocalStack instance
  pub fn endpoint_url(&self) -> String {
    format!("http://localhost:{}", self.host_port)
  }

  /// Create a storage config for this LocalStack instance
  #[allow(dead_code)]
  pub fn create_storage_config(&self, bucket_name: String) -> ResolvedBucketConfig {
    ResolvedBucketConfig {
      name: "test".to_string(),
      bucket_name,
      access_key_id: Some(self.access_key.clone()),
      secret_access_key: Some(self.secret_key.clone()),
      session_token: None,
      region: Some("us-east-1".to_string()),
      endpoint_url: Some(self.endpoint_url()),
      force_path_style: true,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_localstack_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "us-east-1".to_string();
    base_url.virtual_style = false;
    let static_provider = StaticProvider::new(&self.access_key, &self.secret_key, None);
    let client = Client::new(base_url, Some(Box::new(static_provider)), None, None)?;
    Ok(client)
  }

  /// Create a bucket in this LocalStack instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_localstack_client().await?;

    let max_retries = 10;
    let retry_delay = tokio::time::Duration::from_millis(500);

    for attempt in 0..max_retries {
      let exists = match client.bucket_exists(bucket_name).send().await {
        Ok(response) => response.exists,
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
          continue;
        },
      };

      if exists {
        return Ok(());
      }

      match client.create_bucket(bucket_name).send().await {
        Ok(_) => return Ok(()),
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
        },
      }
    }

    Ok(())
  }

  /// Create a bucket and return a configured MinioStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<MinioStorage, Box<dyn std::error::Error>> {
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());

    MinioStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| format!("Failed to create MinioStorage: {:?}", e).into())
  }

  /// List objects in a bucket using LocalStack client
  #[allow(dead_code)]
  pub async fn list_objects(
    &self,
    bucket_name: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = self.create_localstack_client().await?;

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

  /// Check if an object exists using LocalStack client
  #[allow(dead_code)]
  pub async fn object_exists(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<bool, Box<dyn std::error::Error>> {
    let client = self.create_localstack_client().await?;

    let result = client.stat_object(bucket_name, object_name).send().await;

    Ok(result.is_ok())
  }

  /// Put an object using LocalStack client with raw bytes
  #[allow(dead_code)]
  pub async fn put_object(
    &self,
    bucket_name: &str,
    object_name: &str,
    data: Vec<u8>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_localstack_client().await?;

    use minio::s3::builders::ObjectContent;
    let content = ObjectContent::from(data);

    client
      .put_object_content(bucket_name, object_name, content)
      .send()
      .await?;

    Ok(())
  }

  /// Get an object using LocalStack client
  #[allow(dead_code)]
  pub async fn get_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let client = self.create_localstack_client().await?;

    let response = client.get_object(bucket_name, object_name).send().await?;

    let segmented = response.content.to_segmented_bytes().await?;
    let bytes = segmented.to_bytes();

    Ok(bytes.to_vec())
  }

  /// Delete an object using LocalStack client
  #[allow(dead_code)]
  pub async fn delete_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_localstack_client().await?;

    use minio::s3::builders::ObjectToDelete;

    client
      .delete_object(bucket_name, ObjectToDelete::from(object_name))
      .send()
      .await?;

    Ok(())
  }
}

/// S3Mock test container wrapper with helper methods
#[allow(dead_code)]
pub struct S3MockTestContainer {
  pub container: testcontainers::ContainerAsync<GenericImage>,
  pub host_port: u16,
  pub access_key: String,
  pub secret_key: String,
}

impl S3MockTestContainer {
  /// Start a new S3Mock container
  #[allow(dead_code)]
  pub async fn start() -> Self {
    let access_key = "test".to_string();
    let secret_key = "test".to_string();

    let s3mock_image = GenericImage::new("adobe/s3mock", "latest")
      .with_exposed_port(ContainerPort::Tcp(9090))
      .with_env_var("AWS_ACCESS_KEY_ID", access_key.clone())
      .with_env_var("AWS_SECRET_ACCESS_KEY", secret_key.clone());

    let container = s3mock_image
      .start()
      .await
      .expect("Failed to start S3Mock container");

    let host_port = container
      .get_host_port_ipv4(9090)
      .await
      .expect("Failed to get S3Mock port");

    let readiness_retries = 30;
    let readiness_delay = tokio::time::Duration::from_millis(500);
    for attempt in 0..readiness_retries {
      if TcpStream::connect(format!("127.0.0.1:{}", host_port))
        .await
        .is_ok()
      {
        break;
      }
      if attempt + 1 == readiness_retries {
        panic!("S3Mock S3 endpoint not ready");
      }
      tokio::time::sleep(readiness_delay).await;
    }

    Self {
      container,
      host_port,
      access_key,
      secret_key,
    }
  }

  /// Get the endpoint URL for this S3Mock instance
  pub fn endpoint_url(&self) -> String {
    format!("http://localhost:{}", self.host_port)
  }

  /// Create a storage config for this S3Mock instance
  #[allow(dead_code)]
  pub fn create_storage_config(&self, bucket_name: String) -> ResolvedBucketConfig {
    ResolvedBucketConfig {
      name: "test".to_string(),
      bucket_name,
      access_key_id: Some(self.access_key.clone()),
      secret_access_key: Some(self.secret_key.clone()),
      session_token: None,
      region: Some("us-east-1".to_string()),
      endpoint_url: Some(self.endpoint_url()),
      force_path_style: true,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_s3mock_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "us-east-1".to_string();
    base_url.virtual_style = false;
    let static_provider = StaticProvider::new(&self.access_key, &self.secret_key, None);
    let client = Client::new(base_url, Some(Box::new(static_provider)), None, None)?;
    Ok(client)
  }

  /// Create a bucket in this S3Mock instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_s3mock_client().await?;

    let max_retries = 10;
    let retry_delay = tokio::time::Duration::from_millis(500);

    for attempt in 0..max_retries {
      let exists = match client.bucket_exists(bucket_name).send().await {
        Ok(response) => response.exists,
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
          continue;
        },
      };

      if exists {
        return Ok(());
      }

      match client.create_bucket(bucket_name).send().await {
        Ok(_) => return Ok(()),
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
        },
      }
    }

    Ok(())
  }

  /// Create a bucket and return a configured MinioStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<MinioStorage, Box<dyn std::error::Error>> {
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());

    MinioStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| format!("Failed to create MinioStorage: {:?}", e).into())
  }

  /// List objects in a bucket using S3Mock client
  #[allow(dead_code)]
  pub async fn list_objects(
    &self,
    bucket_name: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = self.create_s3mock_client().await?;

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

  /// Check if an object exists using S3Mock client
  #[allow(dead_code)]
  pub async fn object_exists(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<bool, Box<dyn std::error::Error>> {
    let client = self.create_s3mock_client().await?;

    let result = client.stat_object(bucket_name, object_name).send().await;

    Ok(result.is_ok())
  }

  /// Put an object using S3Mock client with raw bytes
  #[allow(dead_code)]
  pub async fn put_object(
    &self,
    bucket_name: &str,
    object_name: &str,
    data: Vec<u8>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_s3mock_client().await?;

    use minio::s3::builders::ObjectContent;
    let content = ObjectContent::from(data);

    client
      .put_object_content(bucket_name, object_name, content)
      .send()
      .await?;

    Ok(())
  }

  /// Get an object using S3Mock client
  #[allow(dead_code)]
  pub async fn get_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let client = self.create_s3mock_client().await?;

    let response = client.get_object(bucket_name, object_name).send().await?;

    let segmented = response.content.to_segmented_bytes().await?;
    let bytes = segmented.to_bytes();

    Ok(bytes.to_vec())
  }

  /// Delete an object using S3Mock client
  #[allow(dead_code)]
  pub async fn delete_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_s3mock_client().await?;

    use minio::s3::builders::ObjectToDelete;

    client
      .delete_object(bucket_name, ObjectToDelete::from(object_name))
      .send()
      .await?;

    Ok(())
  }
}

/// GoFakeS3 test container wrapper with helper methods
#[allow(dead_code)]
pub struct GoFakeS3TestContainer {
  pub container: testcontainers::ContainerAsync<GenericImage>,
  pub host_port: u16,
  pub access_key: String,
  pub secret_key: String,
}

impl GoFakeS3TestContainer {
  /// Start a new GoFakeS3 container
  #[allow(dead_code)]
  pub async fn start() -> Self {
    let access_key = "test".to_string();
    let secret_key = "test".to_string();

    let fakes3_image = GenericImage::new("gspaeth/go-fakes3", "latest")
      .with_exposed_port(ContainerPort::Tcp(4567))
      .with_cmd(["-port", "4567"])
      .with_env_var("AWS_ACCESS_KEY_ID", access_key.clone())
      .with_env_var("AWS_SECRET_ACCESS_KEY", secret_key.clone());

    let container = fakes3_image
      .start()
      .await
      .expect("Failed to start GoFakeS3 container");

    let host_port = container
      .get_host_port_ipv4(4567)
      .await
      .expect("Failed to get GoFakeS3 port");

    let readiness_retries = 30;
    let readiness_delay = tokio::time::Duration::from_millis(500);
    for attempt in 0..readiness_retries {
      if TcpStream::connect(format!("127.0.0.1:{}", host_port))
        .await
        .is_ok()
      {
        break;
      }
      if attempt + 1 == readiness_retries {
        panic!("GoFakeS3 endpoint not ready");
      }
      tokio::time::sleep(readiness_delay).await;
    }

    Self {
      container,
      host_port,
      access_key,
      secret_key,
    }
  }

  /// Get the endpoint URL for this GoFakeS3 instance
  pub fn endpoint_url(&self) -> String {
    format!("http://localhost:{}", self.host_port)
  }

  /// Create a storage config for this GoFakeS3 instance
  #[allow(dead_code)]
  pub fn create_storage_config(&self, bucket_name: String) -> ResolvedBucketConfig {
    ResolvedBucketConfig {
      name: "test".to_string(),
      bucket_name,
      access_key_id: Some(self.access_key.clone()),
      secret_access_key: Some(self.secret_key.clone()),
      session_token: None,
      region: Some("us-east-1".to_string()),
      endpoint_url: Some(self.endpoint_url()),
      force_path_style: true,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_gofakes3_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "us-east-1".to_string();
    base_url.virtual_style = false;
    let static_provider = StaticProvider::new(&self.access_key, &self.secret_key, None);
    let client = Client::new(base_url, Some(Box::new(static_provider)), None, None)?;
    Ok(client)
  }

  /// Create a bucket in this GoFakeS3 instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_gofakes3_client().await?;

    let max_retries = 10;
    let retry_delay = tokio::time::Duration::from_millis(500);

    for attempt in 0..max_retries {
      let exists = match client.bucket_exists(bucket_name).send().await {
        Ok(response) => response.exists,
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
          continue;
        },
      };

      if exists {
        return Ok(());
      }

      match client.create_bucket(bucket_name).send().await {
        Ok(_) => return Ok(()),
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(Box::new(e));
          }
          tokio::time::sleep(retry_delay).await;
        },
      }
    }

    Ok(())
  }

  /// Create a bucket and return a configured MinioStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<MinioStorage, Box<dyn std::error::Error>> {
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());

    MinioStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| format!("Failed to create MinioStorage: {:?}", e).into())
  }

  /// List objects in a bucket using GoFakeS3 client
  #[allow(dead_code)]
  pub async fn list_objects(
    &self,
    bucket_name: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = self.create_gofakes3_client().await?;

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

  /// Check if an object exists using GoFakeS3 client
  #[allow(dead_code)]
  pub async fn object_exists(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<bool, Box<dyn std::error::Error>> {
    let client = self.create_gofakes3_client().await?;

    let result = client.stat_object(bucket_name, object_name).send().await;

    Ok(result.is_ok())
  }

  /// Put an object using GoFakeS3 client with raw bytes
  #[allow(dead_code)]
  pub async fn put_object(
    &self,
    bucket_name: &str,
    object_name: &str,
    data: Vec<u8>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_gofakes3_client().await?;

    use minio::s3::builders::ObjectContent;
    let content = ObjectContent::from(data);

    client
      .put_object_content(bucket_name, object_name, content)
      .send()
      .await?;

    Ok(())
  }

  /// Get an object using GoFakeS3 client
  #[allow(dead_code)]
  pub async fn get_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let client = self.create_gofakes3_client().await?;

    let response = client.get_object(bucket_name, object_name).send().await?;

    let segmented = response.content.to_segmented_bytes().await?;
    let bytes = segmented.to_bytes();

    Ok(bytes.to_vec())
  }

  /// Delete an object using GoFakeS3 client
  #[allow(dead_code)]
  pub async fn delete_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_gofakes3_client().await?;

    use minio::s3::builders::ObjectToDelete;

    client
      .delete_object(bucket_name, ObjectToDelete::from(object_name))
      .send()
      .await?;

    Ok(())
  }
}

static GARAGE_TEST_MUTEX: OnceLock<Arc<Mutex<()>>> = OnceLock::new();

/// Garage test container wrapper with helper methods
#[allow(dead_code)]
pub struct GarageTestContainer {
  pub container: testcontainers::ContainerAsync<GenericImage>,
  pub host_port: u16,
  pub access_key: String,
  pub secret_key: String,
  pub key_name: String,
  pub base_dir: PathBuf,
  _garage_lock: tokio::sync::OwnedMutexGuard<()>,
}

impl GarageTestContainer {
  /// Start a new Garage container with a minimal single-node config
  #[allow(dead_code)]
  pub async fn start() -> Self {
    let lock = GARAGE_TEST_MUTEX
      .get_or_init(|| Arc::new(Mutex::new(())))
      .clone()
      .lock_owned()
      .await;

    let timestamp = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap()
      .as_nanos();
    let base_dir = std::env::temp_dir().join(format!("garage-test-{}", timestamp));
    let config_path = base_dir.join("garage.toml");

    fs::create_dir_all(&base_dir).expect("Failed to create Garage base dir");

    let config = r#"
metadata_dir = "/var/lib/garage/meta"
data_dir = "/var/lib/garage/data"
db_engine = "sqlite"

replication_factor = 1

rpc_bind_addr = "[::]:3901"
rpc_public_addr = "127.0.0.1:3901"
rpc_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

[s3_api]
s3_region = "garage"
api_bind_addr = "[::]:3900"
root_domain = ".s3.garage.localhost"

[s3_web]
bind_addr = "[::]:3902"
root_domain = ".web.garage.localhost"
index = "index.html"

[admin]
api_bind_addr = "[::]:3903"
admin_token = "test-admin-token"
metrics_token = "test-metrics-token"
"#;
    fs::write(&config_path, config).expect("Failed to write Garage config");

    let host_port = std::net::TcpListener::bind("127.0.0.1:0")
      .expect("Failed to bind random host port for Garage")
      .local_addr()
      .expect("Failed to read bound port for Garage")
      .port();

    let garage_image = GenericImage::new("dxflrs/garage", "v2.2.0")
      .with_mapped_port(host_port, ContainerPort::Tcp(3900))
      .with_cmd(["/garage", "-c", "/etc/garage.toml", "server"])
      .with_mount(Mount::bind_mount(
        config_path.to_string_lossy().to_string(),
        "/etc/garage.toml",
      ));

    let container = garage_image
      .start()
      .await
      .expect("Failed to start Garage container");

    let mut instance = Self {
      container,
      host_port,
      access_key: String::new(),
      secret_key: String::new(),
      key_name: format!("test-key-{}", timestamp),
      base_dir,
      _garage_lock: lock,
    };

    instance
      .init_layout()
      .await
      .expect("Failed to initialize Garage layout");
    instance
      .init_key()
      .await
      .expect("Failed to initialize Garage key");

    instance
  }

  /// Get the endpoint URL for this Garage instance
  pub fn endpoint_url(&self) -> String {
    format!("http://localhost:{}", self.host_port)
  }

  /// Create a storage config for this Garage instance
  #[allow(dead_code)]
  pub fn create_storage_config(&self, bucket_name: String) -> ResolvedBucketConfig {
    ResolvedBucketConfig {
      name: "test".to_string(),
      bucket_name,
      access_key_id: Some(self.access_key.clone()),
      secret_access_key: Some(self.secret_key.clone()),
      session_token: None,
      region: Some("garage".to_string()),
      endpoint_url: Some(self.endpoint_url()),
      force_path_style: true,
      timeout: 30,
    }
  }

  async fn exec_garage(&self, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let mut command = vec!["/garage", "-c", "/etc/garage.toml"];
    command.extend_from_slice(args);

    let mut output = self.container.exec(ExecCommand::new(command)).await?;

    let stdout_bytes = output.stdout_to_vec().await?;
    let stderr_bytes = output.stderr_to_vec().await?;
    let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
    let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();

    let mut exit_code = output.exit_code().await?;
    let max_retries = 10;
    let retry_delay = tokio::time::Duration::from_millis(200);
    for _ in 0..max_retries {
      if exit_code.is_some() {
        break;
      }
      tokio::time::sleep(retry_delay).await;
      exit_code = output.exit_code().await?;
    }

    if exit_code != Some(0) {
      let combined = if stderr.trim().is_empty() {
        stdout.trim_end().to_string()
      } else {
        format!("{}\n{}", stdout.trim_end(), stderr.trim_end())
      };
      return Err(
        format!(
          "Garage command failed (exit code {:?}). Output:\n{}",
          exit_code, combined
        )
        .into(),
      );
    }

    if stderr.trim().is_empty() {
      Ok(stdout)
    } else {
      Ok(format!("{}\n{}", stdout.trim_end(), stderr.trim_end()))
    }
  }

  #[allow(dead_code)]
  async fn init_layout(&self) -> Result<(), Box<dyn std::error::Error>> {
    let max_retries = 10;
    let retry_delay = tokio::time::Duration::from_millis(500);

    for attempt in 0..max_retries {
      match self.exec_garage(&["status"]).await {
        Ok(status) => {
          let node_id = status
            .lines()
            .filter_map(|line| {
              let trimmed = line.trim();
              if trimmed.is_empty()
                || trimmed.starts_with('=')
                || trimmed.starts_with("ID ")
                || trimmed.starts_with("INFO ")
                || trimmed.contains("garage_")
              {
                return None;
              }

              let candidate = trimmed.split_whitespace().next()?;
              let is_hex = candidate.chars().all(|c| c.is_ascii_hexdigit());
              if is_hex {
                Some(candidate)
              } else {
                None
              }
            })
            .next();

          if let Some(node_id) = node_id {
            self
              .exec_garage(&["layout", "assign", "-z", "dc1", "-c", "1G", node_id])
              .await?;

            let mut saw_invalid_version = false;
            for version in 1..=5 {
              match self
                .exec_garage(&["layout", "apply", "--version", &version.to_string()])
                .await
              {
                Ok(_) => return Ok(()),
                Err(e) => {
                  if e.to_string().contains("Invalid new layout version") {
                    saw_invalid_version = true;
                    continue;
                  }
                  return Err(e);
                },
              }
            }

            if saw_invalid_version {
              return Ok(());
            }
          }
        },
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(e);
          }
        },
      }

      tokio::time::sleep(retry_delay).await;
    }

    Err("Failed to initialize Garage layout".into())
  }

  #[allow(dead_code)]
  async fn init_key(&mut self) -> Result<(), Box<dyn std::error::Error>> {
    let max_retries = 5;
    let retry_delay = tokio::time::Duration::from_millis(500);

    for attempt in 0..max_retries {
      let output = match self.exec_garage(&["key", "create", &self.key_name]).await {
        Ok(output) => output,
        Err(e) => {
          let message = e.to_string();
          let is_transient =
            message.contains("database is locked") || message.contains("ServiceUnavailable");
          if is_transient && attempt + 1 < max_retries {
            tokio::time::sleep(retry_delay).await;
            continue;
          }
          return Err(e);
        },
      };
      let mut key_id = None;
      let mut secret_key = None;

      for line in output.lines() {
        if let Some((_, value)) = line.split_once("Key ID:") {
          key_id = Some(value.trim().to_string());
        }
        if let Some((_, value)) = line.split_once("Secret key:") {
          secret_key = Some(value.trim().to_string());
        }
      }

      if let (Some(key_id), Some(secret_key)) = (key_id, secret_key) {
        self.access_key = key_id;
        self.secret_key = secret_key;
        return Ok(());
      }

      if attempt + 1 == max_retries {
        return Err(format!("Garage key parse failed. Output:\n{}", output).into());
      }

      tokio::time::sleep(retry_delay).await;
    }

    Err("Garage key parse failed after retries".into())
  }

  /// Create a bucket and allow access for the test key
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let max_retries = 5;
    let retry_delay = tokio::time::Duration::from_millis(500);

    for attempt in 0..max_retries {
      match self.exec_garage(&["bucket", "create", bucket_name]).await {
        Ok(_) => {
          break;
        },
        Err(e) => {
          if attempt + 1 == max_retries {
            return Err(format!("Failed to create Garage bucket. Last error: {}", e).into());
          }
        },
      }

      tokio::time::sleep(retry_delay).await;
    }

    let allow_retries = 10;
    for attempt in 0..allow_retries {
      let result = self
        .exec_garage(&[
          "bucket",
          "allow",
          "--read",
          "--write",
          "--owner",
          bucket_name,
          "--key",
          &self.access_key,
        ])
        .await;

      match result {
        Ok(_) => {
          break;
        },
        Err(e) => {
          if attempt + 1 == allow_retries {
            return Err(format!("Failed to allow Garage bucket access. Last error: {}", e).into());
          }
        },
      }

      tokio::time::sleep(retry_delay).await;
    }

    Ok(())
  }

  /// Create a Garage client for direct S3 operations
  pub async fn create_garage_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "garage".to_string();
    base_url.virtual_style = false;
    let static_provider = StaticProvider::new(&self.access_key, &self.secret_key, None);
    let client = Client::new(base_url, Some(Box::new(static_provider)), None, None)?;
    Ok(client)
  }

  /// List objects in a bucket using Garage client
  #[allow(dead_code)]
  pub async fn list_objects(
    &self,
    bucket_name: &str,
  ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let client = self.create_garage_client().await?;

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

  /// Check if an object exists using Garage client
  #[allow(dead_code)]
  pub async fn object_exists(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<bool, Box<dyn std::error::Error>> {
    let client = self.create_garage_client().await?;
    let result = client.stat_object(bucket_name, object_name).send().await;
    Ok(result.is_ok())
  }

  /// Put an object using Garage client with raw bytes
  #[allow(dead_code)]
  pub async fn put_object(
    &self,
    bucket_name: &str,
    object_name: &str,
    data: Vec<u8>,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_garage_client().await?;

    use minio::s3::builders::ObjectContent;
    let content = ObjectContent::from(data);

    client
      .put_object_content(bucket_name, object_name, content)
      .send()
      .await?;

    Ok(())
  }

  /// Get an object using Garage client
  #[allow(dead_code)]
  pub async fn get_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let client = self.create_garage_client().await?;

    let response = client.get_object(bucket_name, object_name).send().await?;

    let segmented = response.content.to_segmented_bytes().await?;
    let bytes = segmented.to_bytes();

    Ok(bytes.to_vec())
  }

  /// Delete an object using Garage client
  #[allow(dead_code)]
  pub async fn delete_object(
    &self,
    bucket_name: &str,
    object_name: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_garage_client().await?;

    use minio::s3::builders::ObjectToDelete;

    client
      .delete_object(bucket_name, ObjectToDelete::from(object_name))
      .send()
      .await?;

    Ok(())
  }

  /// Create a bucket and return a configured MinioStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<MinioStorage, Box<dyn std::error::Error>> {
    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());
    MinioStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| format!("Failed to create MinioStorage: {:?}", e).into())
  }
}

/// Helper to generate unique bucket names for tests
pub fn unique_bucket_name(prefix: &str) -> String {
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
