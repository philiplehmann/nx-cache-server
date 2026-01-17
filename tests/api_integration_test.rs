//! HTTP API Integration tests aligned with Nx OpenAPI 3.0.0 specification
//!
//! Tests the actual HTTP endpoints at `/v1/cache/{hash}` according to the
//! official Nx remote cache API spec for Nx 20.8+
//!
//! Validates:
//! - PUT /v1/cache/{hash} - Upload task output
//! - GET /v1/cache/{hash} - Download task output
//! - Bearer token authentication
//! - HTTP status codes (200, 401, 403, 404, 409)
//! - Content-Type headers
//! - Error response formats

mod common;

use axum::{
  body::Body,
  http::{header, Request, StatusCode},
  Router,
};
use common::{unique_bucket_name, MinioTestContainer};
use nx_cache_server::domain::yaml_config::{
  ResolvedBucketConfig, ResolvedConfig, ResolvedServiceAccessToken,
};
use nx_cache_server::infra::multi_storage::MultiStorageRouter;
use nx_cache_server::server::{create_router, AppState};
use std::sync::Arc;
use tower::util::ServiceExt; // for `oneshot` and `ready`

/// Helper to create a test app with MinIO backend
async fn create_test_app(minio: &MinioTestContainer) -> (Router, String) {
  let bucket_name = unique_bucket_name("api-test");

  // Create bucket in MinIO
  minio
    .create_bucket(&bucket_name)
    .await
    .expect("Failed to create bucket");

  // Create resolved config with test tokens
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
        name: "test-read-write".to_string(),
        bucket: bucket_name.clone(),
        prefix: "/test".to_string(),
        access_token: "test-token-rw".to_string(),
      },
      ResolvedServiceAccessToken {
        name: "another-namespace".to_string(),
        bucket: bucket_name.clone(),
        prefix: "/other".to_string(),
        access_token: "test-token-other".to_string(),
      },
    ],
    port: 3000,
    debug: true,
  };

  // Create storage router
  let storage = MultiStorageRouter::from_config(&resolved_config)
    .await
    .expect("Failed to create MultiStorageRouter");

  // Create app state and router
  let app_state = AppState {
    storage: Arc::new(storage),
  };

  let app = create_router(&app_state).with_state(app_state);

  (app, bucket_name)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_artifact_success() {
  let _ = tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .with_test_writer()
    .try_init();

  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "test-hash-put-success";
  let data = b"Hello from API test!";

  // PUT /v1/cache/{hash}
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // Spec requires 200 OK
  assert_eq!(response.status(), StatusCode::OK);

  println!("✓ PUT returned 200 OK");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_artifact_success() {
  let _ = tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .with_test_writer()
    .try_init();

  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "test-hash-get-success";
  let data = b"Test data for retrieval";

  // First, PUT the artifact
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let app_clone = app.clone();
  let response = app_clone.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);

  // Now GET the artifact
  let request = Request::builder()
    .method("GET")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .body(Body::empty())
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // Spec requires 200 OK and application/octet-stream
  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response.headers().get(header::CONTENT_TYPE).unwrap(),
    "application/octet-stream"
  );

  // Verify body content
  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  assert_eq!(body.as_ref(), data);

  println!("✓ GET returned 200 OK with correct content-type and body");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_artifact_not_found() {
  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "nonexistent-hash";

  let request = Request::builder()
    .method("GET")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .body(Body::empty())
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // Spec requires 404 Not Found
  assert_eq!(response.status(), StatusCode::NOT_FOUND);

  println!("✓ GET nonexistent artifact returned 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_artifact_missing_auth() {
  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "test-hash-no-auth";
  let data = b"test data";

  // PUT without Authorization header
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // Spec requires 401 Unauthorized
  assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

  println!("✓ PUT without auth returned 401");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_artifact_missing_auth() {
  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "test-hash";

  // GET without Authorization header
  let request = Request::builder()
    .method("GET")
    .uri(format!("/v1/cache/{}", hash))
    .body(Body::empty())
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // Spec requires 401 Unauthorized
  assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

  println!("✓ GET without auth returned 401");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_artifact_invalid_token() {
  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "test-hash-invalid";
  let data = b"test data";

  // PUT with invalid token
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer invalid-token-xyz")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // Spec requires 401 for invalid token
  assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

  println!("✓ PUT with invalid token returned 401");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_artifact_conflict() {
  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "test-hash-conflict";
  let data = b"original data";

  // First PUT - should succeed
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let app_clone = app.clone();
  let response = app_clone.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);

  // Second PUT with same hash - should fail with 409 Conflict
  let data2 = b"different data";
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data2.len())
    .body(Body::from(data2.to_vec()))
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // Spec requires 409 Conflict when trying to override existing record
  assert_eq!(response.status(), StatusCode::CONFLICT);

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  let error_message = String::from_utf8(body.to_vec()).unwrap();
  assert_eq!(error_message, "Cannot override an existing record");

  println!("✓ PUT duplicate artifact returned 409 Conflict");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_namespace_isolation() {
  let _ = tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .with_test_writer()
    .try_init();

  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "shared-hash-123";
  let data1 = b"Data from namespace 1";
  let data2 = b"Data from namespace 2";

  // PUT with first token (namespace /test)
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data1.len())
    .body(Body::from(data1.to_vec()))
    .unwrap();

  let app_clone = app.clone();
  let response = app_clone.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);

  // PUT with second token (namespace /other) - same hash, different namespace
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-other")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data2.len())
    .body(Body::from(data2.to_vec()))
    .unwrap();

  let app_clone2 = app.clone();
  let response = app_clone2.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);

  // GET with first token - should get data1
  let request = Request::builder()
    .method("GET")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .body(Body::empty())
    .unwrap();

  let app_clone3 = app.clone();
  let response = app_clone3.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);
  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  assert_eq!(body.as_ref(), data1);

  // GET with second token - should get data2
  let request = Request::builder()
    .method("GET")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-other")
    .body(Body::empty())
    .unwrap();

  let response = app.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);
  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  assert_eq!(body.as_ref(), data2);

  println!("✓ Namespace isolation working correctly");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_content_length_header() {
  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "test-content-length";
  let data = b"Test data with known length";

  // PUT with Content-Length header (required by spec)
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let response = app.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);

  println!("✓ PUT with Content-Length succeeded");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_health_check_endpoint() {
  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  // Health check should not require authentication
  let request = Request::builder()
    .method("GET")
    .uri("/health")
    .body(Body::empty())
    .unwrap();

  let response = app.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  assert_eq!(body.as_ref(), b"OK");

  println!("✓ Health check endpoint working");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_large_artifact_streaming() {
  let minio = MinioTestContainer::start().await;
  let (app, _bucket) = create_test_app(&minio).await;

  let hash = "large-artifact";
  // Create 5MB test data
  let size = 5 * 1024 * 1024;
  let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();

  // PUT large artifact
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.clone()))
    .unwrap();

  let app_clone = app.clone();
  let response = app_clone.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);

  // GET large artifact
  let request = Request::builder()
    .method("GET")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer test-token-rw")
    .body(Body::empty())
    .unwrap();

  let response = app.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  assert_eq!(body.len(), data.len());
  assert_eq!(body.as_ref(), data.as_slice());

  println!("✓ Large artifact (5MB) streamed successfully");
}
