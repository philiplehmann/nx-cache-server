use crate::infra::multi_storage::MultiStorageRouter;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
  pub storage: Arc<MultiStorageRouter>,
}
