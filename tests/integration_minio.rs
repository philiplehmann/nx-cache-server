use std::io::Cursor;
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;

mod common;
use common::MinioTestContainer;

use nx_cache_server::domain::storage::StorageProvider;

/// Integration test that verifies MinioStorage works with MinIO
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_integration_store_and_retrieve() {
  // Start MinIO container
  let container = MinioTestContainer::start().await;

  println!("MinIO running at {}", container.endpoint_url());

  // Create bucket and storage
  let bucket_name = "test-bucket";
  let storage = container
    .create_storage(bucket_name)
    .await
    .expect("Failed to create storage");

  println!("MinIO: Bucket '{}' created successfully", bucket_name);

  // Test connection
  storage
    .test_connection()
    .await
    .expect("Failed to connect to MinIO");

  // Test data
  let test_hash = "test-hash-12345";
  let test_data = b"Hello, MinIO integration test!";
  let test_data_len = test_data.len() as u64;

  // Verify object doesn't exist yet
  let exists = storage
    .exists(test_hash)
    .await
    .expect("Failed to check existence");
  assert!(!exists, "Object should not exist yet");

  // Store the data
  let cursor = Cursor::new(test_data.to_vec());
  let reader_stream = ReaderStream::new(cursor);

  storage
    .store(test_hash, reader_stream, Some(test_data_len))
    .await
    .expect("Failed to store data");

  println!("Successfully stored object with hash: {}", test_hash);

  // Verify object now exists
  let exists = storage
    .exists(test_hash)
    .await
    .expect("Failed to check existence");
  assert!(exists, "Object should exist after store");

  // Retrieve the data
  let mut reader = storage
    .retrieve(test_hash)
    .await
    .expect("Failed to retrieve data");

  let mut retrieved_data = Vec::new();
  reader
    .read_to_end(&mut retrieved_data)
    .await
    .expect("Failed to read retrieved data");

  // Verify the data matches
  assert_eq!(
    retrieved_data, test_data,
    "Retrieved data should match stored data"
  );

  println!("Successfully retrieved and verified object");
}

/// Test that storing duplicate objects returns AlreadyExists error
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_duplicate_store_fails() {
  let container = MinioTestContainer::start().await;

  let bucket_name = "test-bucket-duplicate";
  let storage = container
    .create_storage(bucket_name)
    .await
    .expect("Failed to create storage");

  let test_hash = "duplicate-hash";
  let test_data = b"Test data";

  // Store once
  let cursor = Cursor::new(test_data.to_vec());
  let reader_stream = ReaderStream::new(cursor);
  storage
    .store(test_hash, reader_stream, Some(test_data.len() as u64))
    .await
    .expect("First store should succeed");

  // Try to store again - should fail
  let cursor = Cursor::new(test_data.to_vec());
  let reader_stream = ReaderStream::new(cursor);
  let result = storage
    .store(test_hash, reader_stream, Some(test_data.len() as u64))
    .await;

  assert!(result.is_err(), "Duplicate store should fail");
  match result {
    Err(nx_cache_server::domain::storage::StorageError::AlreadyExists) => {
      println!("Correctly received AlreadyExists error");
    },
    _ => panic!("Expected AlreadyExists error"),
  }
}

/// Test retrieving non-existent object returns NotFound error
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_retrieve_nonexistent_fails() {
  let container = MinioTestContainer::start().await;

  let bucket_name = "test-bucket-notfound";
  let storage = container
    .create_storage(bucket_name)
    .await
    .expect("Failed to create storage");

  // Try to retrieve non-existent object
  let result = storage.retrieve("nonexistent-hash").await;

  assert!(
    result.is_err(),
    "Retrieve should fail for non-existent object"
  );
  match result {
    Err(nx_cache_server::domain::storage::StorageError::NotFound) => {
      println!("Correctly received NotFound error");
    },
    _ => panic!("Expected NotFound error"),
  }
}

/// Test storing and retrieving large data (streaming)
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_large_file_streaming() {
  let container = MinioTestContainer::start().await;

  let bucket_name = "test-bucket-large";
  let storage = container
    .create_storage(bucket_name)
    .await
    .expect("Failed to create storage");

  // Create 5MB of test data
  let data_size = 5 * 1024 * 1024; // 5MB
  let test_data: Vec<u8> = (0..data_size).map(|i| (i % 256) as u8).collect();
  let test_hash = "large-file-hash";

  // Store large file
  let cursor = Cursor::new(test_data.clone());
  let reader_stream = ReaderStream::new(cursor);

  storage
    .store(test_hash, reader_stream, Some(data_size as u64))
    .await
    .expect("Failed to store large file");

  println!("Successfully stored {}MB file", data_size / 1024 / 1024);

  // Retrieve and verify
  let mut reader = storage
    .retrieve(test_hash)
    .await
    .expect("Failed to retrieve large file");

  let mut retrieved_data = Vec::new();
  reader
    .read_to_end(&mut retrieved_data)
    .await
    .expect("Failed to read large file");

  assert_eq!(
    retrieved_data.len(),
    test_data.len(),
    "Retrieved file size should match"
  );
  assert_eq!(retrieved_data, test_data, "Retrieved data should match");

  println!("Successfully retrieved and verified large file");
}

/// Test using helper methods to verify direct MinIO operations
#[tokio::test(flavor = "multi_thread")]
async fn test_minio_helper_operations() {
  let container = MinioTestContainer::start().await;

  let bucket_name = "test-bucket-helpers";
  container
    .create_bucket(bucket_name)
    .await
    .expect("Failed to create bucket");

  let test_object = "helper-test-object";
  let test_data = b"Helper test data";

  // Initially object should not exist
  let exists = container
    .object_exists(bucket_name, test_object)
    .await
    .expect("Failed to check existence");
  assert!(!exists, "Object should not exist initially");

  // Put object using helper
  container
    .put_object(bucket_name, test_object, test_data.to_vec())
    .await
    .expect("Failed to put object");

  // Object should now exist
  let exists = container
    .object_exists(bucket_name, test_object)
    .await
    .expect("Failed to check existence");
  assert!(exists, "Object should exist after put");

  // Get object using helper
  let retrieved_data = container
    .get_object(bucket_name, test_object)
    .await
    .expect("Failed to get object");

  assert_eq!(
    retrieved_data,
    test_data.to_vec(),
    "Retrieved data should match"
  );

  // List objects
  let objects = container
    .list_objects(bucket_name)
    .await
    .expect("Failed to list objects");

  assert_eq!(objects.len(), 1, "Should have one object");
  assert_eq!(objects[0], test_object, "Object name should match");

  // Delete object
  container
    .delete_object(bucket_name, test_object)
    .await
    .expect("Failed to delete object");

  // Object should no longer exist
  let exists = container
    .object_exists(bucket_name, test_object)
    .await
    .expect("Failed to check existence");
  assert!(!exists, "Object should not exist after delete");

  println!("All helper operations successful");
}
