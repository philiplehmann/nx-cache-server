use crate::domain::yaml_config::ResolvedConfig;
use crate::infra::multi_storage::MultiStorageRouter;
use crate::server::app_state::AppState;
use crate::server::router::create_router;
use std::sync::Arc;

pub async fn run_server(
  storage: MultiStorageRouter,
  config: &ResolvedConfig,
) -> Result<(), std::io::Error> {
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
