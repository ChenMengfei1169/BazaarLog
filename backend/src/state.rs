// Shared application state cheaply cloned into every request handler.
use std::sync::Arc;

use sqlx::AnyPool;

use crate::{cache::Cache, config::Config};

#[derive(Clone)]
pub struct AppState {
    pub pool: AnyPool,
    pub config: Config,
    pub cache: Arc<Cache>,
}
