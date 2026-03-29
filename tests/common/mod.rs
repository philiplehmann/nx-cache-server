//! Common test utilities for integration tests
//!
//! This module provides reusable helpers for setting up testcontainers
//! and creating test fixtures using the MinIO Rust SDK.

pub mod storage_contract;

#[allow(dead_code)]
pub const SSE_C_KEY: &str = "0123456789abcdef0123456789abcdef";

use minio::s3::creds::StaticProvider;
use minio::s3::http::BaseUrl;
use minio::s3::types::S3Api;
use minio::s3::Client;
use serde::Deserialize;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::{debug, info, warn};

use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, SanType};
use tempfile::TempDir;

use testcontainers::runners::AsyncRunner;
use testcontainers::{core::ContainerPort, core::ExecCommand, core::Mount, GenericImage, ImageExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

use nx_cache_server::domain::config::ResolvedBucketConfig;
use nx_cache_server::domain::storage::{StorageError, StorageProvider};
use nx_cache_server::infra::nx_cache_store::NxCacheStorage;

#[derive(Debug, Deserialize)]
struct TestcontainersConfig {
  images: TestcontainersImages,
}

#[derive(Debug, Deserialize)]
struct TestcontainersImages {
  minio: ImageConfig,
  garage: ImageConfig,
  seaweedfs: ImageConfig,
  rustfs: ImageConfig,
}

#[derive(Debug, Deserialize)]
struct ImageConfig {
  repository: String,
  tag: String,
}

static TESTCONTAINERS_CONFIG: OnceLock<TestcontainersConfig> = OnceLock::new();

fn load_testcontainers_config() -> &'static TestcontainersConfig {
  TESTCONTAINERS_CONFIG.get_or_init(|| {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testcontainers.toml");
    let content = fs::read_to_string(&path)
      .unwrap_or_else(|e| panic!("Failed to read testcontainers.toml: {}", e));
    let config: TestcontainersConfig = toml::from_str(&content).unwrap_or_else(|e| {
      panic!("Failed to parse testcontainers.toml: {}", e);
    });
    validate_image_config(&config.images.minio, "minio");
    validate_image_config(&config.images.garage, "garage");
    validate_image_config(&config.images.seaweedfs, "seaweedfs");
    validate_image_config(&config.images.rustfs, "rustfs");
    config
  })
}

fn validate_image_config(image: &ImageConfig, name: &str) {
  if image.tag.trim().is_empty() || image.tag == "REPLACE_ME" {
    panic!(
      "testcontainers.toml image tag for '{}' must be set to a concrete version",
      name
    );
  }
  if image.repository.trim().is_empty() {
    panic!(
      "testcontainers.toml image repository for '{}' must be set",
      name
    );
  }
}

pub struct RetryConfig {
  retries: usize,
  delay: Duration,
}

type Error = dyn std::error::Error;
type TestResult<T> = Result<T, Box<Error>>;

fn env_usize(key: &str, default: usize) -> usize {
  match std::env::var(key) {
    Ok(value) => match value.parse::<usize>() {
      Ok(parsed) => parsed,
      Err(_) => {
        warn!(key, value, "Invalid usize env override, using default");
        default
      },
    },
    Err(_) => default,
  }
}

fn env_u64(key: &str, default: u64) -> u64 {
  match std::env::var(key) {
    Ok(value) => match value.parse::<u64>() {
      Ok(parsed) => parsed,
      Err(_) => {
        warn!(key, value, "Invalid u64 env override, using default");
        default
      },
    },
    Err(_) => default,
  }
}

pub fn retry_config(prefix: &str, default_retries: usize, default_delay_ms: u64) -> RetryConfig {
  let retries = env_usize(&format!("{prefix}_RETRIES"), default_retries).max(1);
  let delay_ms = env_u64(&format!("{prefix}_DELAY_MS"), default_delay_ms).max(1);
  RetryConfig {
    retries,
    delay: Duration::from_millis(delay_ms),
  }
}

fn box_err(message: impl Into<String>) -> Box<Error> {
  Box::new(std::io::Error::other(message.into()))
}

async fn wait_for_tcp_ready(
  host: &str,
  port: u16,
  config: RetryConfig,
  label: &str,
) -> TestResult<()> {
  let address = format!("{}:{}", host, port);
  for attempt in 0..config.retries {
    match TcpStream::connect(address.as_str()).await {
      Ok(_) => {
        info!(label, address, "TCP endpoint ready");
        return Ok(());
      },
      Err(e) => {
        debug!(label, address, attempt, error = %e, "TCP endpoint not ready yet");
        if attempt + 1 == config.retries {
          return Err(box_err(format!(
            "{} endpoint not ready at {} after {} attempts",
            label, address, config.retries
          )));
        }
        tokio::time::sleep(config.delay).await;
      },
    }
  }

  Err(box_err(format!(
    "{} endpoint not ready at {} after retries",
    label, address
  )))
}

pub async fn wait_for_storage_ready(
  storage: &NxCacheStorage,
  label: &str,
  config: RetryConfig,
) -> TestResult<()> {
  for attempt in 0..config.retries {
    match storage.test_connection().await {
      Ok(_) => {
        info!(label, "Storage connection ready");
        return Ok(());
      },
      Err(e) => {
        debug!(label, attempt, error = %e, "Storage connection not ready yet");
        if attempt + 1 == config.retries {
          return Err(box_err(format!(
            "{} connection not ready after {} attempts: {}",
            label, config.retries, e
          )));
        }
        tokio::time::sleep(config.delay).await;
      },
    }
  }

  Err(box_err(format!(
    "{} connection not ready after retries",
    label
  )))
}

async fn ensure_bucket_exists(
  client: &Client,
  bucket_name: &str,
  config: RetryConfig,
  label: &str,
) -> TestResult<()> {
  for attempt in 0..config.retries {
    let exists = match client.bucket_exists(bucket_name).send().await {
      Ok(response) => response.exists,
      Err(e) => {
        if attempt + 1 == config.retries {
          return Err(box_err(format!(
            "{} bucket exists check failed after {} attempts: {}",
            label, config.retries, e
          )));
        }
        debug!(label, bucket_name, attempt, error = %e, "Bucket exists check failed");
        tokio::time::sleep(config.delay).await;
        continue;
      },
    };

    if exists {
      return Ok(());
    }

    match client.create_bucket(bucket_name).send().await {
      Ok(_) => return Ok(()),
      Err(e) => {
        if attempt + 1 == config.retries {
          return Err(box_err(format!(
            "{} bucket creation failed after {} attempts: {}",
            label, config.retries, e
          )));
        }
        debug!(label, bucket_name, attempt, error = %e, "Bucket creation failed");
        tokio::time::sleep(config.delay).await;
      },
    }
  }

  Err(box_err(format!(
    "{} not ready after {} attempts",
    label, config.retries
  )))
}

fn create_s3_client(
  base_url: BaseUrl,
  access_key: &str,
  secret_key: &str,
  tls_cert: Option<&Path>,
  insecure_tls: bool,
) -> TestResult<Client> {
  let static_provider = StaticProvider::new(access_key, secret_key, None);
  let ignore_cert_check = if insecure_tls { Some(true) } else { None };
  let client = Client::new(
    base_url,
    Some(Box::new(static_provider)),
    tls_cert,
    ignore_cert_check,
  )?;
  Ok(client)
}

struct TlsMaterial {
  dir: TempDir,
  cert_path: PathBuf,
}

impl TlsMaterial {
  fn dir_path_string(&self) -> String {
    self.dir.path().to_string_lossy().to_string()
  }

  fn cert_path(&self) -> &Path {
    &self.cert_path
  }

  fn cert_path_string(&self) -> String {
    self.cert_path.to_string_lossy().to_string()
  }
}

fn create_tls_certs(
  prefix: &str,
  cert_filename: &str,
  key_filename: &str,
  set_permissions: bool,
) -> TestResult<TlsMaterial> {
  let mut params = CertificateParams::new(vec!["localhost".to_string()])?;
  params
    .subject_alt_names
    .push(SanType::IpAddress("127.0.0.1".parse()?));
  params.distinguished_name = DistinguishedName::new();
  params
    .distinguished_name
    .push(DnType::CommonName, "localhost");

  let key_pair = KeyPair::generate()?;
  let cert = params.self_signed(&key_pair)?;

  let dir = tempfile::Builder::new().prefix(prefix).tempdir()?;

  let cert_path = dir.path().join(cert_filename);
  let key_path = dir.path().join(key_filename);

  fs::write(&cert_path, cert.pem())?;
  fs::write(&key_path, key_pair.serialize_pem())?;

  #[cfg(unix)]
  if set_permissions {
    use std::fs::Permissions;
    fs::set_permissions(&cert_path, Permissions::from_mode(0o644))?;
    fs::set_permissions(&key_path, Permissions::from_mode(0o644))?;
  }

  Ok(TlsMaterial { dir, cert_path })
}

fn create_minio_tls_certs() -> TestResult<TlsMaterial> {
  create_tls_certs("minio-tls-", "public.crt", "private.key", false)
}

fn create_rustfs_tls_certs() -> TestResult<TlsMaterial> {
  create_tls_certs("rustfs-tls-", "rustfs_cert.pem", "rustfs_key.pem", true)
}

/// MinIO test container wrapper with helper methods
#[allow(dead_code)]
pub struct MinioTestContainer {
  pub container: testcontainers::ContainerAsync<GenericImage>,
  pub host_port: u16,
  pub access_key: String,
  pub secret_key: String,
  pub use_https: bool,
  _tls: Option<TlsMaterial>,
}

impl MinioTestContainer {
  /// Start a new MinIO container with default credentials
  #[allow(dead_code)]
  pub async fn start() -> Self {
    Self::start_result()
      .await
      .expect("Failed to start MinIO container")
  }

  #[allow(dead_code)]
  pub async fn start_result() -> Result<Self, Box<dyn std::error::Error>> {
    let config = load_testcontainers_config();
    let image = &config.images.minio;

    let minio_image = GenericImage::new(image.repository.as_str(), image.tag.as_str())
      .with_exposed_port(ContainerPort::Tcp(9000))
      .with_env_var("MINIO_ROOT_USER", "minioadmin")
      .with_env_var("MINIO_ROOT_PASSWORD", "minioadmin")
      .with_cmd(["server", "/data", "--console-address", ":9001"]);
    let container = minio_image
      .start()
      .await
      .map_err(|e| box_err(format!("Failed to start MinIO container: {}", e)))?;
    let host_port = container
      .get_host_port_ipv4(9000)
      .await
      .map_err(|e| box_err(format!("Failed to get MinIO port: {}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_MINIO_READINESS", 30, 500);
    wait_for_tcp_ready("127.0.0.1", host_port, readiness, "MinIO").await?;
    info!(service = "minio", host_port, "MinIO container ready");

    Ok(Self {
      container,
      host_port,
      access_key: "minioadmin".to_string(),
      secret_key: "minioadmin".to_string(),
      use_https: false,
      _tls: None,
    })
  }

  /// Start a new MinIO container with SSE settings enabled
  #[allow(dead_code)]
  pub async fn start_with_sse() -> Self {
    Self::start_with_sse_result()
      .await
      .expect("Failed to start MinIO SSE container")
  }

  #[allow(dead_code)]
  pub async fn start_with_sse_result() -> Result<Self, Box<dyn std::error::Error>> {
    let config = load_testcontainers_config();
    let image = &config.images.minio;

    let tls = create_minio_tls_certs()?;

    let minio_image = GenericImage::new(image.repository.as_str(), image.tag.as_str())
      .with_exposed_port(ContainerPort::Tcp(9000))
      .with_env_var("MINIO_ROOT_USER", "minioadmin")
      .with_env_var("MINIO_ROOT_PASSWORD", "minioadmin")
      .with_env_var(
        "MINIO_KMS_SECRET_KEY",
        "test-kms-key:MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=",
      )
      .with_env_var("MINIO_API_ALLOW_NON_TLS_SSE_C", "on")
      .with_mount(Mount::bind_mount(
        tls.dir_path_string(),
        "/root/.minio/certs",
      ))
      .with_cmd([
        "server",
        "/data",
        "--console-address",
        ":9001",
        "--certs-dir",
        "/root/.minio/certs",
      ]);
    let container = minio_image
      .start()
      .await
      .map_err(|e| box_err(format!("Failed to start MinIO container: {}", e)))?;
    let host_port = container
      .get_host_port_ipv4(9000)
      .await
      .map_err(|e| box_err(format!("Failed to get MinIO port: {}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_MINIO_SSE_READINESS", 30, 500);
    wait_for_tcp_ready("127.0.0.1", host_port, readiness, "MinIO SSE").await?;
    info!(service = "minio", host_port, "MinIO SSE container ready");

    Ok(Self {
      container,
      host_port,
      access_key: "minioadmin".to_string(),
      secret_key: "minioadmin".to_string(),
      use_https: true,
      _tls: Some(tls),
    })
  }

  /// Get the endpoint URL for this MinIO instance
  pub fn endpoint_url(&self) -> String {
    let scheme = if self.use_https { "https" } else { "http" };
    format!("{}://localhost:{}", scheme, self.host_port)
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
      tls_ca_file: self._tls.as_ref().map(|tls| tls.cert_path_string()),
      insecure_tls: if self.use_https { Some(true) } else { None },
      force_path_style: true,
      sse: None,
      timeout: 30,
    }
  }

  /// Create a MinIO client for bucket management
  pub async fn create_minio_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let base_url = self.endpoint_url().parse::<BaseUrl>()?;
    let tls_cert = self._tls.as_ref().map(|tls| tls.cert_path());
    create_s3_client(
      base_url,
      &self.access_key,
      &self.secret_key,
      tls_cert,
      self.use_https,
    )
  }

  /// Create a bucket in this MinIO instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_minio_client().await?;
    let retry = retry_config("NX_CACHE_TEST_MINIO_BUCKET", 10, 300);
    ensure_bucket_exists(&client, bucket_name, retry, "MinIO bucket").await
  }

  /// Create a bucket and return a configured NxCacheStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<NxCacheStorage, Box<dyn std::error::Error>> {
    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());

    let storage = NxCacheStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| box_err(format!("Failed to create NxCacheStorage: {:?}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_MINIO_STORAGE_READY", 10, 500);
    wait_for_storage_ready(&storage, "MinIO storage", readiness).await?;

    Ok(storage)
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
  pub use_https: bool,
  _tls: Option<TlsMaterial>,
}

impl RustfsTestContainer {
  /// Start a new RustFS container with default credentials
  #[allow(dead_code)]
  pub async fn start() -> Self {
    Self::start_result()
      .await
      .expect("Failed to start RustFS container")
  }

  #[allow(dead_code)]
  pub async fn start_result() -> Result<Self, Box<dyn std::error::Error>> {
    let access_key = "rustfsadmin".to_string();
    let secret_key = "rustfsadmin".to_string();

    let tls = create_rustfs_tls_certs()?;

    let config = load_testcontainers_config();
    let image = &config.images.rustfs;

    let rustfs_image = GenericImage::new(image.repository.as_str(), image.tag.as_str())
      .with_exposed_port(ContainerPort::Tcp(9000))
      .with_env_var("RUSTFS_ACCESS_KEY", access_key.clone())
      .with_env_var("RUSTFS_SECRET_KEY", secret_key.clone())
      .with_env_var("RUSTFS_TLS_PATH", "/opt/tls")
      .with_mount(Mount::bind_mount(tls.dir_path_string(), "/opt/tls"));
    let container = rustfs_image
      .start()
      .await
      .map_err(|e| box_err(format!("Failed to start RustFS container: {}", e)))?;
    let host_port = container
      .get_host_port_ipv4(9000)
      .await
      .map_err(|e| box_err(format!("Failed to get RustFS port: {}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_RUSTFS_READINESS", 30, 500);
    wait_for_tcp_ready("127.0.0.1", host_port, readiness, "RustFS").await?;
    info!(service = "rustfs", host_port, "RustFS container ready");

    Ok(Self {
      container,
      host_port,
      access_key,
      secret_key,
      use_https: true,
      _tls: Some(tls),
    })
  }

  /// Get the endpoint URL for this RustFS instance
  pub fn endpoint_url(&self) -> String {
    let scheme = if self.use_https { "https" } else { "http" };
    format!("{}://localhost:{}", scheme, self.host_port)
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
      tls_ca_file: self._tls.as_ref().map(|tls| tls.cert_path_string()),
      insecure_tls: if self.use_https { Some(true) } else { None },
      force_path_style: true,
      sse: None,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_rustfs_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let base_url = self.endpoint_url().parse::<BaseUrl>()?;
    let tls_cert = self._tls.as_ref().map(|tls| tls.cert_path());
    create_s3_client(
      base_url,
      &self.access_key,
      &self.secret_key,
      tls_cert,
      self.use_https,
    )
  }

  /// Create a bucket in this RustFS instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_rustfs_client().await?;
    let retry = retry_config("NX_CACHE_TEST_RUSTFS_BUCKET", 5, 500);
    ensure_bucket_exists(&client, bucket_name, retry, "RustFS bucket").await
  }

  /// Create a bucket and return a configured NxCacheStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<NxCacheStorage, Box<dyn std::error::Error>> {
    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());

    let storage = NxCacheStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| box_err(format!("Failed to create NxCacheStorage: {:?}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_RUSTFS_STORAGE_READY", 10, 500);
    wait_for_storage_ready(&storage, "RustFS storage", readiness).await?;

    Ok(storage)
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
  pub use_https: bool,
  _tls: Option<TlsMaterial>,
  _seaweedfs_lock: tokio::sync::OwnedMutexGuard<()>,
}

impl SeaweedfsTestContainer {
  /// Start a new SeaweedFS container with default credentials
  #[allow(dead_code)]
  pub async fn start() -> Self {
    Self::start_result()
      .await
      .expect("Failed to start SeaweedFS container")
  }

  #[allow(dead_code)]
  pub async fn start_result() -> Result<Self, Box<dyn std::error::Error>> {
    let lock = SEAWEEDFS_TEST_MUTEX
      .get_or_init(|| Arc::new(Mutex::new(())))
      .clone()
      .lock_owned()
      .await;

    let access_key = "admin".to_string();
    let secret_key = "key".to_string();

    let tls = create_minio_tls_certs()?;

    let host_port = std::net::TcpListener::bind("127.0.0.1:0")
      .map_err(|e| {
        box_err(format!(
          "Failed to bind random host port for SeaweedFS: {}",
          e
        ))
      })?
      .local_addr()
      .map_err(|e| box_err(format!("Failed to read bound port for SeaweedFS: {}", e)))?
      .port();

    let config = load_testcontainers_config();
    let image = &config.images.seaweedfs;

    let seaweedfs_image = GenericImage::new(image.repository.as_str(), image.tag.as_str())
      .with_mapped_port(host_port, ContainerPort::Tcp(8333))
      .with_env_var("AWS_ACCESS_KEY_ID", access_key.clone())
      .with_env_var("AWS_SECRET_ACCESS_KEY", secret_key.clone())
      .with_mount(Mount::bind_mount(tls.dir_path_string(), "/opt/tls"))
      .with_cmd([
        "server",
        "-s3",
        "-dir=/data",
        "-s3.cert.file=/opt/tls/public.crt",
        "-s3.key.file=/opt/tls/private.key",
      ]);
    let container = seaweedfs_image
      .start()
      .await
      .map_err(|e| box_err(format!("Failed to start SeaweedFS container: {}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_SEAWEEDFS_READINESS", 30, 500);
    wait_for_tcp_ready("127.0.0.1", host_port, readiness, "SeaweedFS").await?;
    info!(
      service = "seaweedfs",
      host_port, "SeaweedFS container ready"
    );

    Ok(Self {
      container,
      host_port,
      access_key,
      secret_key,
      use_https: true,
      _tls: Some(tls),
      _seaweedfs_lock: lock,
    })
  }

  /// Get the endpoint URL for this SeaweedFS instance
  pub fn endpoint_url(&self) -> String {
    let scheme = if self.use_https { "https" } else { "http" };
    format!("{}://localhost:{}", scheme, self.host_port)
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
      tls_ca_file: self._tls.as_ref().map(|tls| tls.cert_path_string()),
      insecure_tls: if self.use_https { Some(true) } else { None },
      force_path_style: true,
      sse: None,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_seaweedfs_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "us-east-1".to_string();
    base_url.virtual_style = false;
    let tls_cert = self._tls.as_ref().map(|tls| tls.cert_path());
    create_s3_client(
      base_url,
      &self.access_key,
      &self.secret_key,
      tls_cert,
      self.use_https,
    )
  }

  /// Create a bucket in this SeaweedFS instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_seaweedfs_client().await?;

    let retry = retry_config("NX_CACHE_TEST_SEAWEEDFS_BUCKET", 20, 1000);
    ensure_bucket_exists(&client, bucket_name, retry, "SeaweedFS bucket").await?;

    let readiness = retry_config("NX_CACHE_TEST_SEAWEEDFS_BUCKET_READY", 10, 500);
    for attempt in 0..readiness.retries {
      let exists = client.bucket_exists(bucket_name).send().await?.exists;
      if exists {
        return Ok(());
      }
      if attempt + 1 == readiness.retries {
        break;
      }
      tokio::time::sleep(readiness.delay).await;
    }

    Err(box_err("SeaweedFS bucket not ready after creation"))
  }

  /// Create a bucket and return a configured NxCacheStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<NxCacheStorage, Box<dyn std::error::Error>> {
    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());

    let storage = NxCacheStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| box_err(format!("Failed to create NxCacheStorage: {:?}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_SEAWEEDFS_STORAGE_READY", 10, 1000);
    wait_for_storage_ready(&storage, "SeaweedFS storage", readiness).await?;

    Ok(storage)
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
    let retry = retry_config("NX_CACHE_TEST_SEAWEEDFS_PUT", 5, 500);

    for attempt in 0..retry.retries {
      let cursor = std::io::Cursor::new(data.clone());
      let reader_stream = tokio_util::io::ReaderStream::new(cursor);

      match storage
        .store(object_name, reader_stream, Some(data_len))
        .await
      {
        Ok(()) => return Ok(()),
        Err(StorageError::OperationFailed) => {
          if attempt + 1 == retry.retries {
            return Err("Failed to store object via SeaweedFS storage after retries".into());
          }
          tokio::time::sleep(retry.delay).await;
        },
        Err(e) => {
          return Err(format!("Failed to store object via SeaweedFS storage: {:?}", e).into());
        },
      }
    }

    Err("Failed to store object via SeaweedFS storage after retries".into())
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
    Self::start_result()
      .await
      .expect("Failed to start LocalStack container")
  }

  #[allow(dead_code)]
  pub async fn start_result() -> Result<Self, Box<dyn std::error::Error>> {
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
      .map_err(|e| box_err(format!("Failed to start LocalStack container: {}", e)))?;

    let host_port = container
      .get_host_port_ipv4(4566)
      .await
      .map_err(|e| box_err(format!("Failed to get LocalStack port: {}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_LOCALSTACK_READINESS", 30, 500);
    wait_for_tcp_ready("127.0.0.1", host_port, readiness, "LocalStack").await?;
    info!(
      service = "localstack",
      host_port, "LocalStack container ready"
    );

    Ok(Self {
      container,
      host_port,
      access_key,
      secret_key,
    })
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
      tls_ca_file: None,
      insecure_tls: None,
      force_path_style: true,
      sse: None,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_localstack_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "us-east-1".to_string();
    base_url.virtual_style = false;
    create_s3_client(base_url, &self.access_key, &self.secret_key, None, false)
  }

  /// Create a bucket in this LocalStack instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_localstack_client().await?;
    let retry = retry_config("NX_CACHE_TEST_LOCALSTACK_BUCKET", 10, 500);
    ensure_bucket_exists(&client, bucket_name, retry, "LocalStack bucket").await
  }

  /// Create a bucket and return a configured NxCacheStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<NxCacheStorage, Box<dyn std::error::Error>> {
    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());

    let storage = NxCacheStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| box_err(format!("Failed to create NxCacheStorage: {:?}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_LOCALSTACK_STORAGE_READY", 10, 500);
    wait_for_storage_ready(&storage, "LocalStack storage", readiness).await?;

    Ok(storage)
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
    Self::start_result()
      .await
      .expect("Failed to start S3Mock container")
  }

  #[allow(dead_code)]
  pub async fn start_result() -> Result<Self, Box<dyn std::error::Error>> {
    let access_key = "test".to_string();
    let secret_key = "test".to_string();

    let s3mock_image = GenericImage::new("adobe/s3mock", "latest")
      .with_exposed_port(ContainerPort::Tcp(9090))
      .with_env_var("AWS_ACCESS_KEY_ID", access_key.clone())
      .with_env_var("AWS_SECRET_ACCESS_KEY", secret_key.clone());

    let container = s3mock_image
      .start()
      .await
      .map_err(|e| box_err(format!("Failed to start S3Mock container: {}", e)))?;

    let host_port = container
      .get_host_port_ipv4(9090)
      .await
      .map_err(|e| box_err(format!("Failed to get S3Mock port: {}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_S3MOCK_READINESS", 30, 500);
    wait_for_tcp_ready("127.0.0.1", host_port, readiness, "S3Mock").await?;
    info!(service = "s3mock", host_port, "S3Mock container ready");

    Ok(Self {
      container,
      host_port,
      access_key,
      secret_key,
    })
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
      tls_ca_file: None,
      insecure_tls: None,
      force_path_style: true,
      sse: None,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_s3mock_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "us-east-1".to_string();
    base_url.virtual_style = false;
    create_s3_client(base_url, &self.access_key, &self.secret_key, None, false)
  }

  /// Create a bucket in this S3Mock instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_s3mock_client().await?;
    let retry = retry_config("NX_CACHE_TEST_S3MOCK_BUCKET", 10, 500);
    ensure_bucket_exists(&client, bucket_name, retry, "S3Mock bucket").await
  }

  /// Create a bucket and return a configured NxCacheStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<NxCacheStorage, Box<dyn std::error::Error>> {
    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());

    let storage = NxCacheStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| box_err(format!("Failed to create NxCacheStorage: {:?}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_S3MOCK_STORAGE_READY", 10, 500);
    wait_for_storage_ready(&storage, "S3Mock storage", readiness).await?;

    Ok(storage)
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
    Self::start_result()
      .await
      .expect("Failed to start GoFakeS3 container")
  }

  #[allow(dead_code)]
  pub async fn start_result() -> Result<Self, Box<dyn std::error::Error>> {
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
      .map_err(|e| box_err(format!("Failed to start GoFakeS3 container: {}", e)))?;

    let host_port = container
      .get_host_port_ipv4(4567)
      .await
      .map_err(|e| box_err(format!("Failed to get GoFakeS3 port: {}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_GOFAKES3_READINESS", 30, 500);
    wait_for_tcp_ready("127.0.0.1", host_port, readiness, "GoFakeS3").await?;
    info!(service = "gofakes3", host_port, "GoFakeS3 container ready");

    Ok(Self {
      container,
      host_port,
      access_key,
      secret_key,
    })
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
      tls_ca_file: None,
      insecure_tls: None,
      force_path_style: true,
      sse: None,
      timeout: 30,
    }
  }

  /// Create an S3-compatible client for bucket management
  pub async fn create_gofakes3_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "us-east-1".to_string();
    base_url.virtual_style = false;
    create_s3_client(base_url, &self.access_key, &self.secret_key, None, false)
  }

  /// Create a bucket in this GoFakeS3 instance
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = self.create_gofakes3_client().await?;
    let retry = retry_config("NX_CACHE_TEST_GOFAKES3_BUCKET", 10, 500);
    ensure_bucket_exists(&client, bucket_name, retry, "GoFakeS3 bucket").await
  }

  /// Create a bucket and return a configured NxCacheStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<NxCacheStorage, Box<dyn std::error::Error>> {
    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());

    let storage = NxCacheStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| box_err(format!("Failed to create NxCacheStorage: {:?}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_GOFAKES3_STORAGE_READY", 10, 500);
    wait_for_storage_ready(&storage, "GoFakeS3 storage", readiness).await?;

    Ok(storage)
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
    Self::start_result()
      .await
      .expect("Failed to start Garage container")
  }

  #[allow(dead_code)]
  pub async fn start_result() -> Result<Self, Box<dyn std::error::Error>> {
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

    fs::create_dir_all(&base_dir)
      .map_err(|e| box_err(format!("Failed to create Garage base dir: {}", e)))?;

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
    fs::write(&config_path, config)
      .map_err(|e| box_err(format!("Failed to write Garage config: {}", e)))?;

    let host_port = std::net::TcpListener::bind("127.0.0.1:0")
      .map_err(|e| box_err(format!("Failed to bind random host port for Garage: {}", e)))?
      .local_addr()
      .map_err(|e| box_err(format!("Failed to read bound port for Garage: {}", e)))?
      .port();

    let config = load_testcontainers_config();
    let image = &config.images.garage;

    let garage_image = GenericImage::new(image.repository.as_str(), image.tag.as_str())
      .with_mapped_port(host_port, ContainerPort::Tcp(3900))
      .with_cmd(["/garage", "-c", "/etc/garage.toml", "server"])
      .with_mount(Mount::bind_mount(
        config_path.to_string_lossy().to_string(),
        "/etc/garage.toml",
      ));

    let container = garage_image
      .start()
      .await
      .map_err(|e| box_err(format!("Failed to start Garage container: {}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_GARAGE_READINESS", 30, 500);
    wait_for_tcp_ready("127.0.0.1", host_port, readiness, "Garage").await?;
    info!(service = "garage", host_port, "Garage container ready");

    let mut instance = Self {
      container,
      host_port,
      access_key: String::new(),
      secret_key: String::new(),
      key_name: format!("test-key-{}", timestamp),
      base_dir,
      _garage_lock: lock,
    };

    instance.init_layout().await?;
    instance.init_key().await?;

    Ok(instance)
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
      tls_ca_file: None,
      insecure_tls: None,
      force_path_style: true,
      sse: None,
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
    let retry = retry_config("NX_CACHE_TEST_GARAGE_EXEC_WAIT", 10, 200);
    for _ in 0..retry.retries {
      if exit_code.is_some() {
        break;
      }
      tokio::time::sleep(retry.delay).await;
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
    let retry = retry_config("NX_CACHE_TEST_GARAGE_INIT_LAYOUT", 10, 500);

    for attempt in 0..retry.retries {
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
          if attempt + 1 == retry.retries {
            return Err(e);
          }
        },
      }

      tokio::time::sleep(retry.delay).await;
    }

    Err("Failed to initialize Garage layout".into())
  }

  #[allow(dead_code)]
  async fn init_key(&mut self) -> Result<(), Box<dyn std::error::Error>> {
    let retry = retry_config("NX_CACHE_TEST_GARAGE_INIT_KEY", 5, 500);

    for attempt in 0..retry.retries {
      let output = match self.exec_garage(&["key", "create", &self.key_name]).await {
        Ok(output) => output,
        Err(e) => {
          let message = e.to_string();
          let is_transient =
            message.contains("database is locked") || message.contains("ServiceUnavailable");
          if is_transient && attempt + 1 < retry.retries {
            tokio::time::sleep(retry.delay).await;
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

      if attempt + 1 == retry.retries {
        return Err(format!("Garage key parse failed. Output:\n{}", output).into());
      }

      tokio::time::sleep(retry.delay).await;
    }

    Err("Garage key parse failed after retries".into())
  }

  /// Create a bucket and allow access for the test key
  pub async fn create_bucket(&self, bucket_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let retry = retry_config("NX_CACHE_TEST_GARAGE_BUCKET_CREATE", 5, 500);

    for attempt in 0..retry.retries {
      match self.exec_garage(&["bucket", "create", bucket_name]).await {
        Ok(_) => {
          break;
        },
        Err(e) => {
          if attempt + 1 == retry.retries {
            return Err(format!("Failed to create Garage bucket. Last error: {}", e).into());
          }
        },
      }

      tokio::time::sleep(retry.delay).await;
    }

    let allow_retry = retry_config("NX_CACHE_TEST_GARAGE_BUCKET_ALLOW", 10, 500);
    for attempt in 0..allow_retry.retries {
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
          if attempt + 1 == allow_retry.retries {
            return Err(format!("Failed to allow Garage bucket access. Last error: {}", e).into());
          }
        },
      }

      tokio::time::sleep(allow_retry.delay).await;
    }

    Ok(())
  }

  /// Create a Garage client for direct S3 operations
  pub async fn create_garage_client(&self) -> Result<Client, Box<dyn std::error::Error>> {
    let mut base_url = self.endpoint_url().parse::<BaseUrl>()?;
    base_url.region = "garage".to_string();
    base_url.virtual_style = false;
    create_s3_client(base_url, &self.access_key, &self.secret_key, None, false)
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

  /// Create a bucket and return a configured NxCacheStorage instance
  #[allow(dead_code)]
  pub async fn create_storage(
    &self,
    bucket_name: &str,
  ) -> Result<NxCacheStorage, Box<dyn std::error::Error>> {
    self.create_bucket(bucket_name).await?;

    let config = self.create_storage_config(bucket_name.to_string());
    let storage = NxCacheStorage::from_resolved_bucket(&config)
      .await
      .map_err(|e| box_err(format!("Failed to create NxCacheStorage: {:?}", e)))?;

    let readiness = retry_config("NX_CACHE_TEST_GARAGE_STORAGE_READY", 10, 500);
    wait_for_storage_ready(&storage, "Garage storage", readiness).await?;

    Ok(storage)
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
