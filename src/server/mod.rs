pub mod error;
pub mod handlers;
pub mod middleware;
pub mod validation;

use crate::domain::yaml_config::ResolvedConfig;
use crate::infra::multi_storage::MultiStorageRouter;
use axum::{
  middleware::from_fn_with_state,
  routing::{get, put},
  Router,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
  pub storage: Arc<MultiStorageRouter>,
}

pub fn create_router(app_state: &AppState) -> Router<AppState> {
  let protected_routes = Router::new()
    .route("/v1/cache/{hash}", get(handlers::retrieve_artifact))
    .route("/v1/cache/{hash}", put(handlers::store_artifact))
    .route_layer(from_fn_with_state(
      app_state.clone(),
      middleware::auth_middleware,
    ));

  // Combine public and protected routes
  Router::new()
    .route("/health", get(handlers::health_check)) // Public route - no auth required
    .merge(protected_routes)
}

pub async fn run_server(
  storage: MultiStorageRouter,
  config: &ResolvedConfig,
) -> Result<(), std::io::Error> {
  // Log all configured tokens on server start
  tracing::info!(
    "Server starting with {} configured token(s)",
    storage.token_names().count()
  );
  for name in storage.token_names() {
    tracing::info!("  - Token configured: {}", name);
  }

  let app_state = AppState {
    storage: Arc::new(storage),
  };

  let app = create_router(&app_state).with_state(app_state);
  let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;

  tracing::info!("Server running on port {}", config.port);
  axum::serve(listener, app).await?;

  Ok(())
}
