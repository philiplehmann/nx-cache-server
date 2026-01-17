//! Integration tests using the common MinIO testcontainer helpers
//!
//! ## Known Issue
//!
//! These tests currently fail due to a checksum mismatch error when using streaming
//! PutObject operations with MinIO:
//!
//! ```
//! XAmzContentSHA256Mismatch: The provided 'x-amz-content-sha256' header does not match
//! what was computed.
//! ```
//!
//! The issue occurs because the AWS SDK computes checksums on streaming request bodies,
//! but the channel-based streaming approach (ReaderStream → Channel → ByteStream) used
//! in `S3Storage::store()` causes the checksum computation to fail.
//!
//! See `debug_minio.rs` for an isolated reproduction of the issue.

mod common;

use std::io::Cursor;
use tokio::io::AsyncReadExt;
use tokio_util::io::ReaderStream;

use common::{unique_bucket_name, MinioTestContainer};
use nx_cache_server::domain::storage::StorageProvider;
use nx_cache_server::domain::yaml_config::{
  ResolvedBucketConfig, ResolvedConfig, ResolvedServiceAccessToken,
};
use nx_cache_server::infra::multi_storage::MultiStorageRouter;

#[tokio::test(flavor = "multi_thread")]
async fn test_basic_store_and_retrieve() {
  // Initialize tracing
  let _ = tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .with_test_writer()
    .try_init();

  // Setup MinIO container
  let minio = MinioTestContainer::start().await;

  println!("MinIO started at: {}", minio.endpoint_url());

  // Create storage with unique bucket
  let bucket_name = unique_bucket_name("basic-test");
  let storage = minio
    .create_storage(&bucket_name)
    .await
    .expect("Failed to create storage");

  println!("Storage created for bucket: {}", bucket_name);

  // Test data
  let hash = "test-object-hash";
  let data = b"Hello from integration test!";

  // Verify object doesn't exist
  println!("Checking if object exists...");
  assert!(!storage.exists(hash).await.unwrap());
  println!("Object does not exist (as expected)");

  // Store data
  println!("Storing object...");
  let cursor = Cursor::new(data.to_vec());
  let stream = ReaderStream::new(cursor);
  match storage.store(hash, stream, Some(data.len() as u64)).await {
    Ok(_) => println!("Store succeeded"),
    Err(e) => {
      eprintln!("Store error details: {:?}", e);
      panic!("Failed to store: {:?}", e);
    },
  }
  println!("Object stored successfully");

  // Verify object exists
  println!("Verifying object exists...");
  assert!(storage.exists(hash).await.unwrap());
  println!("Object exists confirmed");

  // Retrieve data
  println!("Retrieving object...");
  let mut reader = storage.retrieve(hash).await.expect("Failed to retrieve");
  let mut retrieved = Vec::new();
  reader.read_to_end(&mut retrieved).await.unwrap();
  println!("Object retrieved, {} bytes", retrieved.len());

  // Verify data matches
  assert_eq!(retrieved, data);

  println!("✓ Successfully stored and retrieved object");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_objects() {
  let minio = MinioTestContainer::start().await;

  let bucket_name = unique_bucket_name("multi-test");
  let storage = minio.create_storage(&bucket_name).await.unwrap();

  // Store multiple objects
  let objects = vec![("hash1", "data1"), ("hash2", "data2"), ("hash3", "data3")];

  for (hash, data) in &objects {
    let cursor = Cursor::new(data.as_bytes().to_vec());
    let stream = ReaderStream::new(cursor);
    storage
      .store(hash, stream, Some(data.len() as u64))
      .await
      .expect("Failed to store");
  }

  // Verify all objects exist
  for (hash, _) in &objects {
    assert!(
      storage.exists(hash).await.unwrap(),
      "Object {} should exist",
      hash
    );
  }

  // Retrieve and verify all objects
  for (hash, expected_data) in &objects {
    let mut reader = storage.retrieve(hash).await.unwrap();
    let mut retrieved = Vec::new();
    reader.read_to_end(&mut retrieved).await.unwrap();
    assert_eq!(retrieved, expected_data.as_bytes());
  }

  println!(
    "✓ Successfully stored and retrieved {} objects",
    objects.len()
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_storage_errors() {
  let minio = MinioTestContainer::start().await;

  let bucket_name = unique_bucket_name("error-test");
  let storage = minio.create_storage(&bucket_name).await.unwrap();

  // Test NotFound error
  let result = storage.retrieve("nonexistent").await;
  assert!(result.is_err());
  match result {
    Err(nx_cache_server::domain::storage::StorageError::NotFound) => {
      println!("✓ Correctly returned NotFound error");
    },
    _ => panic!("Expected NotFound error"),
  }

  // Test AlreadyExists error
  let hash = "duplicate";
  let data = b"test";

  // First store should succeed
  let cursor = Cursor::new(data.to_vec());
  let stream = ReaderStream::new(cursor);
  storage
    .store(hash, stream, Some(data.len() as u64))
    .await
    .unwrap();

  // Second store should fail
  let cursor = Cursor::new(data.to_vec());
  let stream = ReaderStream::new(cursor);
  let result = storage.store(hash, stream, Some(data.len() as u64)).await;

  assert!(result.is_err());
  match result {
    Err(nx_cache_server::domain::storage::StorageError::AlreadyExists) => {
      println!("✓ Correctly returned AlreadyExists error");
    },
    _ => panic!("Expected AlreadyExists error"),
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_streaming_large_file() {
  let minio = MinioTestContainer::start().await;

  let bucket_name = unique_bucket_name("large-test");
  let storage = minio.create_storage(&bucket_name).await.unwrap();

  // Create 10MB test file
  let size = 10 * 1024 * 1024;
  let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
  let hash = "large-file";

  println!("Storing {}MB file...", size / 1024 / 1024);

  let cursor = Cursor::new(data.clone());
  let stream = ReaderStream::new(cursor);
  storage
    .store(hash, stream, Some(size as u64))
    .await
    .expect("Failed to store large file");

  println!("Retrieving {}MB file...", size / 1024 / 1024);

  let mut reader = storage.retrieve(hash).await.unwrap();
  let mut retrieved = Vec::new();
  reader.read_to_end(&mut retrieved).await.unwrap();

  assert_eq!(retrieved.len(), data.len());
  assert_eq!(retrieved, data);

  println!("✓ Successfully streamed large file");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_connection_verification() {
  let minio = MinioTestContainer::start().await;

  let bucket_name = unique_bucket_name("connection-test");
  let storage = minio.create_storage(&bucket_name).await.unwrap();

  // Test connection should succeed
  storage
    .test_connection()
    .await
    .expect("Connection test should succeed");

  println!("✓ Connection test passed");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_namespaces_in_one_bucket() {
  // Initialize tracing
  let _ = tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .with_test_writer()
    .try_init();

  // Setup MinIO container
  let minio = MinioTestContainer::start().await;
  let bucket_name = unique_bucket_name("multi-namespace");

  println!("MinIO started at: {}", minio.endpoint_url());
  println!("Creating shared bucket: {}", bucket_name);

  // Create the shared bucket
  minio
    .create_bucket(&bucket_name)
    .await
    .expect("Failed to create bucket");

  // Create resolved config with multiple service tokens sharing one bucket
  // but with different prefixes (namespaces)
  let resolved_config = ResolvedConfig {
    buckets: vec![ResolvedBucketConfig {
      name: bucket_name.clone(),
      bucket_name: bucket_name.clone(),
      access_key_id: Some(minio.access_key.clone()),
      secret_access_key: Some(minio.secret_key.clone()),
      session_token: None,
      region: Some("us-east-1".to_string()),
      endpoint_url: Some(minio.endpoint_url()),
      force_path_style: true,
      timeout: 60,
    }],
    service_access_tokens: vec![
      ResolvedServiceAccessToken {
        name: "ci-team".to_string(),
        bucket: bucket_name.clone(),
        prefix: "/ci".to_string(),
        access_token: "token-ci".to_string(),
      },
      ResolvedServiceAccessToken {
        name: "dev-team".to_string(),
        bucket: bucket_name.clone(),
        prefix: "/dev".to_string(),
        access_token: "token-dev".to_string(),
      },
      ResolvedServiceAccessToken {
        name: "prod-team".to_string(),
        bucket: bucket_name.clone(),
        prefix: "/prod".to_string(),
        access_token: "token-prod".to_string(),
      },
      ResolvedServiceAccessToken {
        name: "no-prefix-team".to_string(),
        bucket: bucket_name.clone(),
        prefix: "".to_string(),
        access_token: "token-root".to_string(),
      },
    ],
    port: 3000,
    debug: true,
  };

  // Create MultiStorageRouter from config
  let router = MultiStorageRouter::from_config(&resolved_config)
    .await
    .expect("Failed to create MultiStorageRouter");

  println!(
    "MultiStorageRouter created with {} namespaces",
    resolved_config.service_access_tokens.len()
  );

  // Test data
  let hash = "test-hash-123";
  let ci_data = b"CI team data";
  let dev_data = b"Dev team data";
  let prod_data = b"Prod team data";
  let root_data = b"Root team data";

  // Store data using different tokens (different namespaces)
  println!("\n=== Storing data in different namespaces ===");

  // CI namespace
  println!("Storing in /ci namespace...");
  let cursor = Cursor::new(ci_data.to_vec());
  let stream = ReaderStream::new(cursor);
  router
    .store_with_token("token-ci", hash, stream, Some(ci_data.len() as u64))
    .await
    .expect("Failed to store in CI namespace");

  // Dev namespace
  println!("Storing in /dev namespace...");
  let cursor = Cursor::new(dev_data.to_vec());
  let stream = ReaderStream::new(cursor);
  router
    .store_with_token("token-dev", hash, stream, Some(dev_data.len() as u64))
    .await
    .expect("Failed to store in Dev namespace");

  // Prod namespace
  println!("Storing in /prod namespace...");
  let cursor = Cursor::new(prod_data.to_vec());
  let stream = ReaderStream::new(cursor);
  router
    .store_with_token("token-prod", hash, stream, Some(prod_data.len() as u64))
    .await
    .expect("Failed to store in Prod namespace");

  // Root namespace (no prefix)
  println!("Storing in root namespace (no prefix)...");
  let cursor = Cursor::new(root_data.to_vec());
  let stream = ReaderStream::new(cursor);
  router
    .store_with_token("token-root", hash, stream, Some(root_data.len() as u64))
    .await
    .expect("Failed to store in root namespace");

  // Verify all objects exist in their respective namespaces
  println!("\n=== Verifying object existence ===");
  assert!(
    router.exists_with_token("token-ci", hash).await.unwrap(),
    "Object should exist in CI namespace"
  );
  assert!(
    router.exists_with_token("token-dev", hash).await.unwrap(),
    "Object should exist in Dev namespace"
  );
  assert!(
    router.exists_with_token("token-prod", hash).await.unwrap(),
    "Object should exist in Prod namespace"
  );
  assert!(
    router.exists_with_token("token-root", hash).await.unwrap(),
    "Object should exist in root namespace"
  );
  println!("All objects exist in their namespaces");

  // Retrieve and verify data from each namespace
  println!("\n=== Retrieving and verifying data ===");

  // CI namespace
  println!("Retrieving from /ci namespace...");
  let mut reader = router
    .retrieve_with_token("token-ci", hash)
    .await
    .expect("Failed to retrieve from CI namespace");
  let mut retrieved_ci = Vec::new();
  reader.read_to_end(&mut retrieved_ci).await.unwrap();
  assert_eq!(retrieved_ci, ci_data, "CI data should match");

  // Dev namespace
  println!("Retrieving from /dev namespace...");
  let mut reader = router
    .retrieve_with_token("token-dev", hash)
    .await
    .expect("Failed to retrieve from Dev namespace");
  let mut retrieved_dev = Vec::new();
  reader.read_to_end(&mut retrieved_dev).await.unwrap();
  assert_eq!(retrieved_dev, dev_data, "Dev data should match");

  // Prod namespace
  println!("Retrieving from /prod namespace...");
  let mut reader = router
    .retrieve_with_token("token-prod", hash)
    .await
    .expect("Failed to retrieve from Prod namespace");
  let mut retrieved_prod = Vec::new();
  reader.read_to_end(&mut retrieved_prod).await.unwrap();
  assert_eq!(retrieved_prod, prod_data, "Prod data should match");

  // Root namespace
  println!("Retrieving from root namespace...");
  let mut reader = router
    .retrieve_with_token("token-root", hash)
    .await
    .expect("Failed to retrieve from root namespace");
  let mut retrieved_root = Vec::new();
  reader.read_to_end(&mut retrieved_root).await.unwrap();
  assert_eq!(retrieved_root, root_data, "Root data should match");

  println!("\n=== Testing namespace isolation ===");

  // Verify that objects in different namespaces are truly isolated
  // Store a second object with a different hash in one namespace
  let hash2 = "hash-only-in-ci";
  let ci_exclusive_data = b"Only in CI";

  let cursor = Cursor::new(ci_exclusive_data.to_vec());
  let stream = ReaderStream::new(cursor);
  router
    .store_with_token(
      "token-ci",
      hash2,
      stream,
      Some(ci_exclusive_data.len() as u64),
    )
    .await
    .expect("Failed to store exclusive CI object");

  // Verify it exists in CI but not in other namespaces
  assert!(
    router.exists_with_token("token-ci", hash2).await.unwrap(),
    "Object should exist in CI namespace"
  );
  assert!(
    !router.exists_with_token("token-dev", hash2).await.unwrap(),
    "Object should NOT exist in Dev namespace"
  );
  assert!(
    !router.exists_with_token("token-prod", hash2).await.unwrap(),
    "Object should NOT exist in Prod namespace"
  );
  assert!(
    !router.exists_with_token("token-root", hash2).await.unwrap(),
    "Object should NOT exist in root namespace"
  );

  println!("Namespace isolation verified");

  // Verify actual S3 keys are prefixed correctly
  println!("\n=== Verifying S3 key structure ===");
  let objects = minio
    .list_objects(&bucket_name)
    .await
    .expect("Failed to list objects");

  println!("Objects in bucket:");
  for obj in &objects {
    println!("  - {}", obj);
  }

  // Check that the right prefixes are present (without leading slash in S3)
  assert!(
    objects.contains(&format!("ci/{}", hash)),
    "Should have ci/{} in bucket",
    hash
  );
  assert!(
    objects.contains(&format!("dev/{}", hash)),
    "Should have dev/{} in bucket",
    hash
  );
  assert!(
    objects.contains(&format!("prod/{}", hash)),
    "Should have prod/{} in bucket",
    hash
  );
  assert!(
    objects.contains(&hash.to_string()),
    "Should have {} (no prefix) in bucket",
    hash
  );
  assert!(
    objects.contains(&format!("ci/{}", hash2)),
    "Should have ci/{} in bucket",
    hash2
  );

  println!("\n✓ Successfully tested multiple namespaces in one bucket");
  println!("  - Stored objects in 4 different namespaces (3 with prefixes, 1 without)");
  println!("  - Verified namespace isolation");
  println!("  - Confirmed correct S3 key structure with prefixes");
}
