mod common;
use common::storage_contract::{
  run_duplicate_store_fails, run_helper_operations_contract, run_large_file_streaming,
  run_retrieve_nonexistent_fails, run_store_and_retrieve,
};
use common::SeaweedfsTestContainer;

/// Integration test that verifies MinioStorage works with SeaweedFS (S3-compatible)
#[tokio::test(flavor = "multi_thread")]
async fn test_seaweedfs_integration_store_and_retrieve() {
  let container = SeaweedfsTestContainer::start().await;

  run_store_and_retrieve("SeaweedFS", |bucket_name| {
    let container = &container;
    async move { container.create_storage(bucket_name.as_str()).await }
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
