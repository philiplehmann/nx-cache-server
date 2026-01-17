//! OpenAPI 3.0.0 Compliance Tests
//!
//! This test suite validates that all HTTP responses strictly adhere to the
//! Nx remote cache OpenAPI specification (Nx 20.8+).
//!
//! Specifically validates:
//! - Correct HTTP status codes
//! - Content-Type headers match specification
//! - Response body formats
//! - Error message formats

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
use tower::util::ServiceExt;

/// Helper to create a test app with MinIO backend
async fn create_test_app(minio: &MinioTestContainer) -> (Router, String) {
  let bucket_name = unique_bucket_name("openapi-test");

  minio
    .create_bucket(&bucket_name)
    .await
    .expect("Failed to create bucket");

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
    service_access_tokens: vec![ResolvedServiceAccessToken {
      name: "test-token".to_string(),
      bucket: bucket_name.clone(),
      prefix: "/test".to_string(),
      access_token: "valid-test-token".to_string(),
    }],
    port: 3000,
    debug: true,
  };

  let storage = MultiStorageRouter::from_config(&resolved_config)
    .await
    .expect("Failed to create MultiStorageRouter");

  let app_state = AppState {
    storage: Arc::new(storage),
  };

  let app = create_router(&app_state).with_state(app_state);

  (app, bucket_name)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_200_response_format() {
  let minio = MinioTestContainer::start().await;
  let (app, _) = create_test_app(&minio).await;

  let hash = "openapi-put-200";
  let data = b"test data";

  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer valid-test-token")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // OpenAPI spec: 200 "Successfully uploaded the output"
  assert_eq!(
    response.status(),
    StatusCode::OK,
    "PUT should return 200 OK"
  );

  println!("✓ PUT /v1/cache/{{hash}} returns 200 OK per spec");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_401_response_format() {
  let minio = MinioTestContainer::start().await;
  let (app, _) = create_test_app(&minio).await;

  let hash = "openapi-put-401";
  let data = b"test data";

  // Test without Authorization header
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let response = app.clone().oneshot(request).await.unwrap();

  // OpenAPI spec: 401 with text/plain content type
  assert_eq!(
    response.status(),
    StatusCode::UNAUTHORIZED,
    "PUT without auth should return 401"
  );

  let content_type = response
    .headers()
    .get(header::CONTENT_TYPE)
    .expect("401 response must have Content-Type header");

  assert_eq!(
    content_type.to_str().unwrap(),
    "text/plain",
    "401 response must have Content-Type: text/plain per spec"
  );

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  let error_message = String::from_utf8(body.to_vec()).unwrap();
  assert!(
    !error_message.is_empty(),
    "401 response must include error message"
  );

  println!("✓ PUT /v1/cache/{{hash}} returns 401 with text/plain per spec");

  // Test with invalid token
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer invalid-token-xyz")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  assert_eq!(
    response.status(),
    StatusCode::UNAUTHORIZED,
    "PUT with invalid token should return 401"
  );

  let content_type = response
    .headers()
    .get(header::CONTENT_TYPE)
    .expect("401 response must have Content-Type header");

  assert_eq!(
    content_type.to_str().unwrap(),
    "text/plain",
    "401 response must have Content-Type: text/plain per spec"
  );

  println!("✓ PUT /v1/cache/{{hash}} returns 401 with text/plain for invalid token");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_409_response_format() {
  let minio = MinioTestContainer::start().await;
  let (app, _) = create_test_app(&minio).await;

  let hash = "openapi-put-409";
  let data = b"original data";

  // First PUT - should succeed
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer valid-test-token")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let app_clone = app.clone();
  let response = app_clone.oneshot(request).await.unwrap();
  assert_eq!(response.status(), StatusCode::OK);

  // Second PUT - should return 409
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer valid-test-token")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // OpenAPI spec: 409 "Cannot override an existing record"
  assert_eq!(
    response.status(),
    StatusCode::CONFLICT,
    "PUT duplicate should return 409 CONFLICT"
  );

  // While spec doesn't explicitly require text/plain for 409,
  // we should be consistent with other error responses
  let content_type = response
    .headers()
    .get(header::CONTENT_TYPE)
    .expect("409 response should have Content-Type header");

  assert_eq!(
    content_type.to_str().unwrap(),
    "text/plain",
    "409 response should have Content-Type: text/plain for consistency"
  );

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  let error_message = String::from_utf8(body.to_vec()).unwrap();
  assert_eq!(
    error_message, "Cannot override an existing record",
    "409 response must match spec message"
  );

  println!("✓ PUT /v1/cache/{{hash}} returns 409 with text/plain per spec");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_200_response_format() {
  let minio = MinioTestContainer::start().await;
  let (app, _) = create_test_app(&minio).await;

  let hash = "openapi-get-200";
  let data = b"test binary data";

  // First PUT the artifact
  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer valid-test-token")
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
    .header(header::AUTHORIZATION, "Bearer valid-test-token")
    .body(Body::empty())
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // OpenAPI spec: 200 with application/octet-stream
  assert_eq!(
    response.status(),
    StatusCode::OK,
    "GET should return 200 OK"
  );

  let content_type = response
    .headers()
    .get(header::CONTENT_TYPE)
    .expect("200 response must have Content-Type header");

  assert_eq!(
    content_type.to_str().unwrap(),
    "application/octet-stream",
    "GET 200 response must have Content-Type: application/octet-stream per spec"
  );

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  assert_eq!(body.as_ref(), data, "Body should match uploaded data");

  println!("✓ GET /v1/cache/{{hash}} returns 200 with application/octet-stream per spec");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_401_response_format() {
  let minio = MinioTestContainer::start().await;
  let (app, _) = create_test_app(&minio).await;

  let hash = "openapi-get-401";

  // Test without Authorization header
  let request = Request::builder()
    .method("GET")
    .uri(format!("/v1/cache/{}", hash))
    .body(Body::empty())
    .unwrap();

  let response = app.clone().oneshot(request).await.unwrap();

  // OpenAPI spec: 401 (implicit from security requirement) with text/plain
  assert_eq!(
    response.status(),
    StatusCode::UNAUTHORIZED,
    "GET without auth should return 401"
  );

  let content_type = response
    .headers()
    .get(header::CONTENT_TYPE)
    .expect("401 response must have Content-Type header");

  assert_eq!(
    content_type.to_str().unwrap(),
    "text/plain",
    "401 response must have Content-Type: text/plain per spec"
  );

  println!("✓ GET /v1/cache/{{hash}} returns 401 with text/plain per spec");

  // Test with invalid token
  let request = Request::builder()
    .method("GET")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer invalid-token-xyz")
    .body(Body::empty())
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  assert_eq!(
    response.status(),
    StatusCode::UNAUTHORIZED,
    "GET with invalid token should return 401"
  );

  let content_type = response
    .headers()
    .get(header::CONTENT_TYPE)
    .expect("401 response must have Content-Type header");

  assert_eq!(
    content_type.to_str().unwrap(),
    "text/plain",
    "401 response must have Content-Type: text/plain per spec"
  );

  println!("✓ GET /v1/cache/{{hash}} returns 401 with text/plain for invalid token");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_404_response_format() {
  let minio = MinioTestContainer::start().await;
  let (app, _) = create_test_app(&minio).await;

  let hash = "openapi-get-404-nonexistent";

  let request = Request::builder()
    .method("GET")
    .uri(format!("/v1/cache/{}", hash))
    .header(header::AUTHORIZATION, "Bearer valid-test-token")
    .body(Body::empty())
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // OpenAPI spec: 404 "The record was not found"
  assert_eq!(
    response.status(),
    StatusCode::NOT_FOUND,
    "GET nonexistent should return 404"
  );

  // While spec doesn't explicitly require text/plain for 404,
  // we should be consistent with other error responses
  let content_type = response
    .headers()
    .get(header::CONTENT_TYPE)
    .expect("404 response should have Content-Type header");

  assert_eq!(
    content_type.to_str().unwrap(),
    "text/plain",
    "404 response should have Content-Type: text/plain for consistency"
  );

  let body = axum::body::to_bytes(response.into_body(), usize::MAX)
    .await
    .unwrap();
  let error_message = String::from_utf8(body.to_vec()).unwrap();
  assert_eq!(
    error_message, "The record was not found",
    "404 response message should match spec"
  );

  println!("✓ GET /v1/cache/{{hash}} returns 404 with text/plain per spec");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_request_with_invalid_hash_format() {
  let minio = MinioTestContainer::start().await;
  let (app, _) = create_test_app(&minio).await;

  // Test with invalid characters in hash (using special chars that validation rejects)
  let invalid_hash = "invalid@hash#with$special%chars";
  let data = b"test";

  let request = Request::builder()
    .method("PUT")
    .uri(format!("/v1/cache/{}", invalid_hash))
    .header(header::AUTHORIZATION, "Bearer valid-test-token")
    .header(header::CONTENT_TYPE, "application/octet-stream")
    .header(header::CONTENT_LENGTH, data.len())
    .body(Body::from(data.to_vec()))
    .unwrap();

  let response = app.oneshot(request).await.unwrap();

  // Should return 400 Bad Request for invalid hash format
  assert_eq!(
    response.status(),
    StatusCode::BAD_REQUEST,
    "Invalid hash format should return 400"
  );

  let content_type = response
    .headers()
    .get(header::CONTENT_TYPE)
    .expect("400 response should have Content-Type header");

  assert_eq!(
    content_type.to_str().unwrap(),
    "text/plain",
    "400 response should have Content-Type: text/plain"
  );

  println!("✓ Invalid hash format returns 400 with text/plain");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_all_error_responses_have_text_plain() {
  let minio = MinioTestContainer::start().await;
  let (app, _) = create_test_app(&minio).await;

  // Collection of all error scenarios that should return text/plain
  let scenarios = vec![
    (
      "401 - No auth header",
      StatusCode::UNAUTHORIZED,
      Request::builder()
        .method("GET")
        .uri("/v1/cache/test-hash")
        .body(Body::empty())
        .unwrap(),
    ),
    (
      "401 - Invalid token",
      StatusCode::UNAUTHORIZED,
      Request::builder()
        .method("GET")
        .uri("/v1/cache/test-hash")
        .header(header::AUTHORIZATION, "Bearer invalid")
        .body(Body::empty())
        .unwrap(),
    ),
    (
      "400 - Invalid hash",
      StatusCode::BAD_REQUEST,
      Request::builder()
        .method("GET")
        .uri("/v1/cache/invalid@hash#special")
        .header(header::AUTHORIZATION, "Bearer valid-test-token")
        .body(Body::empty())
        .unwrap(),
    ),
    (
      "404 - Not found",
      StatusCode::NOT_FOUND,
      Request::builder()
        .method("GET")
        .uri("/v1/cache/nonexistent-artifact-12345")
        .header(header::AUTHORIZATION, "Bearer valid-test-token")
        .body(Body::empty())
        .unwrap(),
    ),
  ];

  for (description, expected_status, request) in scenarios {
    let response = app.clone().oneshot(request).await.unwrap();

    assert_eq!(
      response.status(),
      expected_status,
      "Scenario '{}' should return {}",
      description,
      expected_status
    );

    let content_type = response
      .headers()
      .get(header::CONTENT_TYPE)
      .unwrap_or_else(|| panic!("Scenario '{}' missing Content-Type header", description));

    assert_eq!(
      content_type.to_str().unwrap(),
      "text/plain",
      "Scenario '{}' must have Content-Type: text/plain",
      description
    );

    println!("✓ {} returns text/plain", description);
  }

  println!("✓ All error responses have Content-Type: text/plain");
}
