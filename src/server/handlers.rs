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
    validation::validate_hash(&hash)?;

    // Extract the authenticated token from request extensions BEFORE consuming the request
    let token = request
        .extensions()
        .get::<AuthenticatedToken>()
        .cloned()
        .ok_or(ServerError::Unauthorized)?;

    // Check if artifact already exists
    if state.storage.exists_with_token(&token.0, &hash).await? {
        return Ok((StatusCode::CONFLICT, "Cannot override an existing record"));
    }

    // convert body directly to AsyncRead without buffering
    let body_stream = request.into_body().into_data_stream();

    // Map the stream to convert axum errors to io::Error
    let io_stream = body_stream.map(|result| {
        result.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    });

    let body_reader = tokio_util::io::StreamReader::new(io_stream);
    let reader_stream = tokio_util::io::ReaderStream::new(body_reader);

    state
        .storage
        .store_with_token(&token.0, &hash, reader_stream)
        .await?;

    Ok((StatusCode::ACCEPTED, ""))
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
