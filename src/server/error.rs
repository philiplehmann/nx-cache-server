use crate::domain::storage::StorageError;
use axum::{
  http::StatusCode,
  response::{IntoResponse, Response},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServerError {
  #[error("Bad request")]
  BadRequest,

  #[error("Unauthorized")]
  Unauthorized,

  #[error("Internal server error")]
  InternalError,

  #[error("Storage error: {0}")]
  Storage(#[from] StorageError),
}

impl IntoResponse for ServerError {
  fn into_response(self) -> Response {
    let (status, message) = match self {
      // Map domain errors to HTTP responses
      ServerError::Storage(StorageError::NotFound) => {
        (StatusCode::NOT_FOUND, "The record was not found")
      },
      ServerError::Storage(StorageError::AlreadyExists) => {
        (StatusCode::CONFLICT, "Cannot override an existing record")
      },
      ServerError::Storage(StorageError::OperationFailed) => {
        (StatusCode::NOT_FOUND, "The record was not found")
      },

      // HTTP-specific errors
      ServerError::BadRequest => (StatusCode::NOT_FOUND, "The record was not found"),
      ServerError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized"),
      ServerError::InternalError => (StatusCode::NOT_FOUND, "The record was not found"),
    };

    (status, [("Content-Type", "text/plain")], message).into_response()
  }
}
