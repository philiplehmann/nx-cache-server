use crate::server::AppState;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use subtle::ConstantTimeEq;
use tracing;

/// Extension type to carry the authenticated token through the request
#[derive(Clone)]
pub struct AuthenticatedToken(pub String);

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Extract Bearer token from Authorization header
    let token = request
        .headers()
        .get("authorization")
        .and_then(|header| header.to_str().ok())
        .and_then(|auth_value| auth_value.strip_prefix("Bearer "));

    let token = match token {
        Some(t) => t,
        None => return Err(StatusCode::UNAUTHORIZED),
    };

    // Check token against all configured tokens using constant-time comparison
    let mut matched_token: Option<String> = None;

    for token_value in state.storage.tokens() {
        if bool::from(token.as_bytes().ct_eq(token_value.as_bytes())) {
            matched_token = Some(token_value.clone());
            break;
        }
    }

    match matched_token {
        Some(token_value) => {
            // Get the token configuration to log the name
            if let Some(config) = state.storage.get_token_config(&token_value) {
                tracing::info!(
                    "Authenticated request from: {} (bucket: {}, prefix: {})",
                    config.name,
                    config.bucket,
                    config.prefix
                );
            }

            // Store the token in request extensions for handlers to use
            request.extensions_mut().insert(AuthenticatedToken(token_value));
            Ok(next.run(request).await)
        }
        None => {
            tracing::warn!("Authentication failed: invalid token");
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}
