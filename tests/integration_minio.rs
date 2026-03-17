mod common;
use common::storage_contract::{
  run_duplicate_store_fails, run_helper_operations_contract, run_large_file_streaming,
  run_retrieve_nonexistent_fails, run_store_and_retrieve,
};
use common::MinioTestContainer;
use nx_cache_server::domain::config::ResolvedSseConfig;
use nx_cache_server::infra::nx_cache_store::NxCacheStorage;

const SSE_C_KEY: &str = "0123456789abcdef0123456789abcdef";

/// Integration test that verifies NxCacheStorage works with MinIO
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_integration_store_and_retrieve() {
  let container = MinioTestContainer::start().await;

  run_store_and_retrieve("MinIO", |bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Test that storing duplicate objects returns AlreadyExists error
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_duplicate_store_fails() {
  let container = MinioTestContainer::start().await;

  run_duplicate_store_fails(|bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Test retrieving non-existent object returns NotFound error
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_retrieve_nonexistent_fails() {
  let container = MinioTestContainer::start().await;

  run_retrieve_nonexistent_fails(|bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Test storing and retrieving large data (streaming)
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_large_file_streaming() {
  let container = MinioTestContainer::start().await;

  run_large_file_streaming(|bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_minio_sse_s3_store_and_retrieve() {
  let container = MinioTestContainer::start_with_sse().await;

  run_store_and_retrieve("MinIO SSE-S3", |bucket_name| {
    let container = &container;
    async move {
      container.create_bucket(bucket_name.as_str()).await?;

      let mut config = container.create_storage_config(bucket_name);
      config.sse = Some(ResolvedSseConfig::SseS3);

      let storage = NxCacheStorage::from_resolved_bucket(&config)
        .await
        .map_err(|e| format!("Failed to create NxCacheStorage: {:?}", e))?;

      Ok(storage)
    }
  })
  .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_minio_sse_c_store_and_retrieve() {
  let container = MinioTestContainer::start_with_sse().await;

  run_store_and_retrieve("MinIO SSE-C", |bucket_name| {
    let container = &container;
    async move {
      container.create_bucket(bucket_name.as_str()).await?;

      let mut config = container.create_storage_config(bucket_name);
      config.sse = Some(ResolvedSseConfig::SseC {
        key: SSE_C_KEY.to_string(),
      });

      let storage = NxCacheStorage::from_resolved_bucket(&config)
        .await
        .map_err(|e| format!("Failed to create NxCacheStorage: {:?}", e))?;

      Ok(storage)
    }
  })
  .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_minio_sse_kms_store_and_retrieve() {
  let container = MinioTestContainer::start_with_sse().await;

  run_store_and_retrieve("MinIO SSE-KMS", |bucket_name| {
    let container = &container;
    async move {
      container.create_bucket(bucket_name.as_str()).await?;

      let mut config = container.create_storage_config(bucket_name);
      config.sse = Some(ResolvedSseConfig::SseKms {
        key_id: "test-kms-key".to_string(),
        context: None,
      });

      let storage = NxCacheStorage::from_resolved_bucket(&config)
        .await
        .map_err(|e| format!("Failed to create NxCacheStorage SSE-KMS: {:?}", e))?;

      Ok(storage)
    }
  })
  .await;
}

/// Test using helper methods to verify direct MinIO operations
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_helper_operations() {
  let container = MinioTestContainer::start().await;

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
