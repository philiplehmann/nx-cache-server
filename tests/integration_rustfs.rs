mod common;
use common::storage_contract::{
  run_duplicate_store_fails, run_helper_operations_contract, run_large_file_streaming,
  run_retrieve_nonexistent_fails, run_store_and_retrieve,
};
use common::{RustfsTestContainer, SSE_C_KEY};
use minio::s3::types::S3Api;
use nx_cache_server::domain::config::ResolvedSseConfig;
use nx_cache_server::domain::storage::StorageProvider;
use nx_cache_server::infra::nx_cache_store::NxCacheStorage;
use std::io::Cursor;
use tokio::io::AsyncReadExt;

use tokio_util::io::ReaderStream;

/// Integration test that verifies NxCacheStorage works with RustFS (S3-compatible)
#[tokio::test(flavor = "multi_thread")]
async fn test_rustfs_integration_store_and_retrieve() {
  let container = RustfsTestContainer::start().await;

  run_store_and_retrieve("RustFS", |bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Test that storing duplicate objects returns AlreadyExists error
#[tokio::test(flavor = "multi_thread")]
async fn test_rustfs_duplicate_store_fails() {
  let container = RustfsTestContainer::start().await;

  run_duplicate_store_fails(|bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Test retrieving non-existent object returns NotFound error
#[tokio::test(flavor = "multi_thread")]
async fn test_rustfs_retrieve_nonexistent_fails() {
  let container = RustfsTestContainer::start().await;

  run_retrieve_nonexistent_fails(|bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Test storing and retrieving large data (streaming)
#[tokio::test(flavor = "multi_thread")]
async fn test_rustfs_large_file_streaming() {
  let container = RustfsTestContainer::start().await;

  run_large_file_streaming(|bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "RustFS requires KMS config for SSE-S3 in tests"]
async fn test_rustfs_sse_s3_store_and_retrieve() {
  let container = RustfsTestContainer::start().await;

  run_store_and_retrieve("RustFS SSE-S3", |bucket_name| {
    let container = &container;
    async move {
      container.create_bucket(bucket_name.as_str()).await?;

      let mut config = container.create_storage_config(bucket_name);
      config.sse = Some(ResolvedSseConfig::SseS3);

      let storage = NxCacheStorage::from_resolved_bucket(&config).await?;
      Ok(storage)
    }
  })
  .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rustfs_sse_c_store_and_retrieve() {
  let container = RustfsTestContainer::start().await;

  run_store_and_retrieve("RustFS SSE-C", |bucket_name| {
    let container = &container;
    async move {
      container.create_bucket(bucket_name.as_str()).await?;

      let mut config = container.create_storage_config(bucket_name);
      config.sse = Some(ResolvedSseConfig::SseC {
        key: SSE_C_KEY.to_string(),
      });

      let storage = NxCacheStorage::from_resolved_bucket(&config).await?;
      Ok(storage)
    }
  })
  .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "RustFS test container does not configure KMS, so SSE-KMS is not supported in this environment"]
async fn test_rustfs_sse_kms_store_and_retrieve() {
  let container = RustfsTestContainer::start().await;
  let bucket_name = "test-bucket";

  container
    .create_bucket(bucket_name)
    .await
    .expect("Failed to create RustFS bucket");

  let mut config = container.create_storage_config(bucket_name.to_string());
  config.sse = Some(ResolvedSseConfig::SseKms {
    key_id: "test-kms-key".to_string(),
    context: None,
  });

  let storage = NxCacheStorage::from_resolved_bucket(&config)
    .await
    .expect("Failed to create RustFS SSE-KMS storage");

  let test_hash = "test-hash-12345";
  let test_data = b"RustFS SSE-KMS test data".to_vec();
  let test_data_len = test_data.len() as u64;

  let cursor = Cursor::new(test_data.clone());
  let reader_stream = ReaderStream::new(cursor);

  storage
    .store(test_hash, reader_stream, Some(test_data_len))
    .await
    .expect("Failed to store data with SSE-KMS");

  let client = container
    .create_rustfs_client()
    .await
    .expect("Failed to create RustFS client");
  let stat = client
    .stat_object(bucket_name, test_hash)
    .send()
    .await
    .expect("Failed to stat object for SSE-KMS headers");

  let sse_header = stat
    .headers
    .get("x-amz-server-side-encryption")
    .and_then(|value| value.to_str().ok())
    .unwrap_or_default();

  assert!(
    matches!(sse_header, "aws:kms" | "aws:kms:dsse"),
    "Expected SSE-KMS header, got {:?}",
    sse_header
  );

  let mut reader = storage
    .retrieve(test_hash)
    .await
    .expect("Failed to retrieve data");
  let mut retrieved_data = Vec::new();
  reader
    .read_to_end(&mut retrieved_data)
    .await
    .expect("Failed to read retrieved data");

  assert_eq!(
    retrieved_data, test_data,
    "Retrieved data should match stored data"
  );
}

/// Test using helper methods to verify direct RustFS operations
#[tokio::test(flavor = "multi_thread")]
async fn test_rustfs_helper_operations() {
  let container = RustfsTestContainer::start().await;

  run_helper_operations_contract(
    |bucket_name| {
      let container = &container;
      async move { container.create_bucket(bucket_name.as_str()).await }
    },
    |bucket_name, object_name| {
      let container = &container;
      async move {
        container
          .object_exists(bucket_name.as_str(), object_name.as_str())
          .await
      }
    },
    |bucket_name, object_name, data| {
      let container = &container;
      async move {
        container
          .put_object(bucket_name.as_str(), object_name.as_str(), data)
          .await
      }
    },
    |bucket_name, object_name| {
      let container = &container;
      async move {
        container
          .get_object(bucket_name.as_str(), object_name.as_str())
          .await
      }
    },
    |bucket_name| {
      let container = &container;
      async move { container.list_objects(bucket_name.as_str()).await }
    },
    |bucket_name, object_name| {
      let container = &container;
      async move {
        container
          .delete_object(bucket_name.as_str(), object_name.as_str())
          .await
      }
    },
  )
  .await;
}
