use crate::server::{error::ServerError, middleware::AuthenticatedToken, validation, AppState};
use axum::{
  body::Body,
  extract::{Path, Request, State},
  http::StatusCode,
  response::IntoResponse,
};
use tokio_stream::StreamExt;

pub async fn store_artifact(
  Path(hash): Path<String>,
  State(state): State<AppState>,
  request: Request,
) -> Result<impl IntoResponse, ServerError> {
  if validation::validate_hash(&hash).is_err() {
    return Ok((
      StatusCode::FORBIDDEN,
      [("Content-Type", "text/plain")],
      "Access forbidden",
    ));
  }

  // Extract the authenticated token from request extensions BEFORE consuming the request
  let token = request
    .extensions()
    .get::<AuthenticatedToken>()
    .cloned()
    .ok_or(ServerError::Unauthorized)?;

  // Extract Content-Length header before consuming the request
  let content_length = request
    .headers()
    .get(axum::http::header::CONTENT_LENGTH)
    .and_then(|v| v.to_str().ok())
    .and_then(|s| s.parse::<u64>().ok());

  // Check if artifact already exists
  match state.storage.exists_with_token(&token.0, &hash).await {
    Ok(true) => {
      return Ok((
        StatusCode::CONFLICT,
        [("Content-Type", "text/plain")],
        "Cannot override an existing record",
      ));
    },
    Ok(false) => {},
    Err(err) => {
      tracing::error!("Storage error on exists: {}", err);
      return Ok((
        StatusCode::FORBIDDEN,
        [("Content-Type", "text/plain")],
        "Access forbidden",
      ));
    },
  }

  // convert body directly to AsyncRead without buffering
  let body_stream = request.into_body().into_data_stream();

  // Map the stream to convert axum errors to io::Error
  let io_stream = body_stream.map(|result| result.map_err(std::io::Error::other));

  let body_reader = tokio_util::io::StreamReader::new(io_stream);
  let reader_stream = tokio_util::io::ReaderStream::new(body_reader);

  if let Err(err) = state
    .storage
    .store_with_token(&token.0, &hash, reader_stream, content_length)
    .await
  {
    if matches!(err, crate::domain::storage::StorageError::AlreadyExists) {
      return Ok((
        StatusCode::CONFLICT,
        [("Content-Type", "text/plain")],
        "Cannot override an existing record",
      ));
    }

    tracing::error!("Storage error on store: {}", err);
    return Ok((
      StatusCode::FORBIDDEN,
      [("Content-Type", "text/plain")],
      "Access forbidden",
    ));
  }

  Ok((StatusCode::OK, [("Content-Type", "text/plain")], ""))
}

pub async fn retrieve_artifact(
  Path(hash): Path<String>,
  State(state): State<AppState>,
  request: Request,
) -> Result<impl IntoResponse, ServerError> {
  validation::validate_hash(&hash)?;

  // Extract the authenticated token from request extensions
  let token = request
    .extensions()
    .get::<AuthenticatedToken>()
    .cloned()
    .ok_or(ServerError::Unauthorized)?;

  let reader = state.storage.retrieve_with_token(&token.0, &hash).await?;
  let stream = tokio_util::io::ReaderStream::new(reader);
  let body = Body::from_stream(stream);

  Ok((
    StatusCode::OK,
    [("content-type", "application/octet-stream")],
    body,
  ))
}

pub async fn health_check() -> impl IntoResponse {
  (StatusCode::OK, "OK")
}
