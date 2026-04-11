mod common;
use common::storage_contract::{
  run_duplicate_store_fails, run_helper_operations_contract, run_large_file_streaming,
  run_retrieve_nonexistent_fails, run_store_and_retrieve,
};
use common::{retry_config, wait_for_storage_ready, SeaweedfsTestContainer, SSE_C_KEY};
use nx_cache_server::domain::config::ResolvedSseConfig;
use nx_cache_server::infra::nx_cache_store::NxCacheStorage;

/// Integration test that verifies NxCacheStorage works with SeaweedFS (S3-compatible)
#[tokio::test(flavor = "multi_thread")]
async fn test_seaweedfs_integration_store_and_retrieve() {
  let container = SeaweedfsTestContainer::start().await;

  run_store_and_retrieve("SeaweedFS", |bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Integration test that verifies SSE-S3 works with SeaweedFS (S3-compatible)
#[tokio::test(flavor = "multi_thread")]
async fn test_seaweedfs_sse_s3_store_and_retrieve() {
  let container = SeaweedfsTestContainer::start().await;

  run_store_and_retrieve("SeaweedFS SSE-S3", |bucket_name| {
    let container = &container;
    async move {
      container.create_bucket(bucket_name.as_str()).await?;

      let mut config = container.create_storage_config(bucket_name);
      config.sse = Some(ResolvedSseConfig::SseS3);

      let storage = NxCacheStorage::from_resolved_bucket(&config)
        .await
        .map_err(|e| format!("Failed to create SeaweedFS SSE-S3 storage: {:?}", e))?;

      let readiness = retry_config("NX_CACHE_TEST_SEAWEEDFS_STORAGE_READY", 10, 1000);
      wait_for_storage_ready(&storage, "SeaweedFS SSE-S3 storage", readiness).await?;
      Ok(storage)
    }
  })
  .await;
}

/// Integration test that verifies SSE-C works with SeaweedFS (S3-compatible)
#[tokio::test(flavor = "multi_thread")]
async fn test_seaweedfs_sse_c_store_and_retrieve() {
  let container = SeaweedfsTestContainer::start().await;

  run_store_and_retrieve("SeaweedFS SSE-C", |bucket_name| {
    let container = &container;
    async move {
      container.create_bucket(bucket_name.as_str()).await?;

      let mut config = container.create_storage_config(bucket_name);
      config.sse = Some(ResolvedSseConfig::SseC {
        key: SSE_C_KEY.to_string(),
      });

      let storage = NxCacheStorage::from_resolved_bucket(&config)
        .await
        .map_err(|e| format!("Failed to create SeaweedFS SSE-C storage: {:?}", e))?;

      let readiness = retry_config("NX_CACHE_TEST_SEAWEEDFS_STORAGE_READY", 10, 1000);
      wait_for_storage_ready(&storage, "SeaweedFS SSE-C storage", readiness).await?;
      Ok(storage)
    }
  })
  .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "SeaweedFS does not support SSE-KMS in tests"]
async fn test_seaweedfs_sse_kms_store_and_retrieve() {
  let container = SeaweedfsTestContainer::start().await;

  run_store_and_retrieve("SeaweedFS SSE-KMS", |bucket_name| {
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
        .map_err(|e| format!("Failed to create SeaweedFS SSE-KMS storage: {:?}", e))?;

      let readiness = retry_config("NX_CACHE_TEST_SEAWEEDFS_STORAGE_READY", 10, 1000);
      wait_for_storage_ready(&storage, "SeaweedFS SSE-KMS storage", readiness).await?;
      Ok(storage)
    }
  })
  .await;
}

/// Test that storing duplicate objects returns AlreadyExists error
#[tokio::test(flavor = "multi_thread")]
async fn test_seaweedfs_duplicate_store_fails() {
  let container = SeaweedfsTestContainer::start().await;

  run_duplicate_store_fails(|bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Test retrieving non-existent object returns NotFound error
#[tokio::test(flavor = "multi_thread")]
async fn test_seaweedfs_retrieve_nonexistent_fails() {
  let container = SeaweedfsTestContainer::start().await;

  run_retrieve_nonexistent_fails(|bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Test storing and retrieving large data (streaming)
#[tokio::test(flavor = "multi_thread")]
async fn test_seaweedfs_large_file_streaming() {
  let container = SeaweedfsTestContainer::start().await;

  run_large_file_streaming(|bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
  })
  .await;
}

/// Test using helper methods to verify direct SeaweedFS operations
#[tokio::test(flavor = "multi_thread")]
async fn test_seaweedfs_helper_operations() {
  let container = SeaweedfsTestContainer::start().await;

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
