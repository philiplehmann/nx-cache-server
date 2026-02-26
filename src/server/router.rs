use crate::server::{app_state::AppState, handlers, middleware};
use axum::{
  middleware::from_fn_with_state,
  routing::{get, put},
  Router,
};

pub fn create_router(app_state: &AppState) -> Router<AppState> {
  let protected_routes = Router::new()
    .route("/v1/cache/{hash}", get(handlers::retrieve_artifact))
    .route("/v1/cache/{hash}", put(handlers::store_artifact))
    .route_layer(from_fn_with_state(
      app_state.clone(),
      middleware::auth_middleware,
    ));

  Router::new()
    .route("/health", get(handlers::health_check))
    .merge(protected_routes)
}
