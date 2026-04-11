//! Shared storage contract test helpers.
//!
//! These helpers let individual integration tests focus on provider setup while
//! sharing the same test logic across S3-compatible backends.

use std::future::Future;
use std::io::Cursor;

use tokio::io::AsyncReadExt;
use tokio::time::Duration;
use tokio_util::io::ReaderStream;

use nx_cache_server::domain::storage::{StorageError, StorageProvider};

async fn store_with_retry<S: StorageProvider>(
  storage: &S,
  hash: &str,
  data: &[u8],
  content_length: u64,
  retries: usize,
  delay: Duration,
) -> Result<(), StorageError> {
  for attempt in 0..retries {
    let cursor = Cursor::new(data.to_vec());
    let reader_stream = ReaderStream::new(cursor);

    match storage
      .store(hash, reader_stream, Some(content_length))
      .await
    {
      Ok(()) => return Ok(()),
      Err(StorageError::OperationFailed) => {
        if attempt + 1 == retries {
          return Err(StorageError::OperationFailed);
        }
        tokio::time::sleep(delay).await;
      },
      Err(err) => return Err(err),
    }
  }

  Err(StorageError::OperationFailed)
}

#[allow(dead_code)]
pub async fn run_store_and_retrieve<S, F, Fut>(provider_name: &str, create_storage: F)
where
  S: StorageProvider,
  F: Fn(String) -> Fut,
  Fut: Future<Output = Result<S, Box<dyn std::error::Error>>>,
{
  let bucket_name = "test-bucket";
  let storage = create_storage(bucket_name.to_string())
    .await
    .expect("Failed to create storage");

  let test_hash = "test-hash-12345";
  let test_data = format!("Hello, {} integration test!", provider_name).into_bytes();
  let test_data_len = test_data.len() as u64;

  let exists = storage
    .exists(test_hash)
    .await
    .expect("Failed to check existence");
  assert!(!exists, "Object should not exist yet");

  let store_result = store_with_retry(
    &storage,
    test_hash,
    &test_data,
    test_data_len,
    5,
    Duration::from_millis(500),
  )
  .await;

  match store_result {
    Ok(()) | Err(StorageError::AlreadyExists) => {},
    Err(err) => panic!("Failed to store data: {:?}", err),
  }

  let exists = storage
    .exists(test_hash)
    .await
    .expect("Failed to check existence");
  assert!(exists, "Object should exist after store");

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

#[allow(dead_code)]
pub async fn run_duplicate_store_fails<S, F, Fut>(create_storage: F)
where
  S: StorageProvider,
  F: Fn(String) -> Fut,
  Fut: Future<Output = Result<S, Box<dyn std::error::Error>>>,
{
  let bucket_name = "test-bucket-duplicate";
  let storage = create_storage(bucket_name.to_string())
    .await
    .expect("Failed to create storage");

  let test_hash = "duplicate-hash";
  let test_data = b"Test data";

  let first_store = store_with_retry(
    &storage,
    test_hash,
    test_data,
    test_data.len() as u64,
    5,
    Duration::from_millis(500),
  )
  .await;

  match first_store {
    Ok(()) | Err(StorageError::AlreadyExists) => {},
    Err(err) => panic!("First store should succeed: {:?}", err),
  }

  let cursor = Cursor::new(test_data.to_vec());
  let reader_stream = ReaderStream::new(cursor);
  let result = storage
    .store(test_hash, reader_stream, Some(test_data.len() as u64))
    .await;

  assert!(result.is_err(), "Duplicate store should fail");
  match result {
    Err(StorageError::AlreadyExists) => {},
    _ => panic!("Expected AlreadyExists error"),
  }
}

#[allow(dead_code)]
pub async fn run_retrieve_nonexistent_fails<S, F, Fut>(create_storage: F)
where
  S: StorageProvider,
  F: Fn(String) -> Fut,
  Fut: Future<Output = Result<S, Box<dyn std::error::Error>>>,
{
  let bucket_name = "test-bucket-notfound";
  let storage = create_storage(bucket_name.to_string())
    .await
    .expect("Failed to create storage");

  let result = storage.retrieve("nonexistent-hash").await;

  assert!(
    result.is_err(),
    "Retrieve should fail for non-existent object"
  );
  match result {
    Err(StorageError::NotFound) => {},
    _ => panic!("Expected NotFound error"),
  }
}

#[allow(dead_code)]
pub async fn run_large_file_streaming<S, F, Fut>(create_storage: F)
where
  S: StorageProvider,
  F: Fn(String) -> Fut,
  Fut: Future<Output = Result<S, Box<dyn std::error::Error>>>,
{
  let bucket_name = "test-bucket-large";
  let storage = create_storage(bucket_name.to_string())
    .await
    .expect("Failed to create storage");

  let data_size = 5 * 1024 * 1024;
  let test_data: Vec<u8> = (0..data_size).map(|i| (i % 256) as u8).collect();
  let test_hash = "large-file-hash";

  let store_result = store_with_retry(
    &storage,
    test_hash,
    &test_data,
    data_size as u64,
    5,
    Duration::from_millis(500),
  )
  .await;

  match store_result {
    Ok(()) | Err(StorageError::AlreadyExists) => {},
    Err(err) => panic!("Failed to store large file: {:?}", err),
  }

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
}

#[allow(dead_code)]
pub async fn run_helper_operations_contract<
  FCreate,
  FExists,
  FPut,
  FGet,
  FList,
  FDelete,
  FutCreate,
  FutExists,
  FutPut,
  FutGet,
  FutList,
  FutDelete,
>(
  create_bucket: FCreate,
  object_exists: FExists,
  put_object: FPut,
  get_object: FGet,
  list_objects: FList,
  delete_object: FDelete,
) where
  FCreate: Fn(String) -> FutCreate,
  FutCreate: Future<Output = Result<(), Box<dyn std::error::Error>>>,
  FExists: Fn(String, String) -> FutExists,
  FutExists: Future<Output = Result<bool, Box<dyn std::error::Error>>>,
  FPut: Fn(String, String, Vec<u8>) -> FutPut,
  FutPut: Future<Output = Result<(), Box<dyn std::error::Error>>>,
  FGet: Fn(String, String) -> FutGet,
  FutGet: Future<Output = Result<Vec<u8>, Box<dyn std::error::Error>>>,
  FList: Fn(String) -> FutList,
  FutList: Future<Output = Result<Vec<String>, Box<dyn std::error::Error>>>,
  FDelete: Fn(String, String) -> FutDelete,
  FutDelete: Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
  let bucket_name = "test-bucket-helpers";
  create_bucket(bucket_name.to_string())
    .await
    .expect("Failed to create bucket");

  let test_object = "helper-test-object";
  let test_data = b"Helper test data";

  let exists = object_exists(bucket_name.to_string(), test_object.to_string())
    .await
    .expect("Failed to check existence");
  assert!(!exists, "Object should not exist initially");

  put_object(
    bucket_name.to_string(),
    test_object.to_string(),
    test_data.to_vec(),
  )
  .await
  .expect("Failed to put object");

  let exists = object_exists(bucket_name.to_string(), test_object.to_string())
    .await
    .expect("Failed to check existence");
  assert!(exists, "Object should exist after put");

  let retrieved_data = get_object(bucket_name.to_string(), test_object.to_string())
    .await
    .expect("Failed to get object");

  assert_eq!(
    retrieved_data,
    test_data.to_vec(),
    "Retrieved data should match"
  );

  let objects = list_objects(bucket_name.to_string())
    .await
    .expect("Failed to list objects");

  assert_eq!(objects.len(), 1, "Should have one object");
  assert_eq!(objects[0], test_object, "Object name should match");

  delete_object(bucket_name.to_string(), test_object.to_string())
    .await
    .expect("Failed to delete object");

  let exists = object_exists(bucket_name.to_string(), test_object.to_string())
    .await
    .expect("Failed to check existence");
  assert!(!exists, "Object should not exist after delete");
}
