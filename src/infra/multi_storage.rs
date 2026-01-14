use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;

use crate::domain::{
    storage::{StorageError, StorageProvider},
    yaml_config::{ResolvedConfig, ResolvedServiceAccessToken},
};
use crate::infra::aws::S3Storage;

/// Storage router that manages multiple S3 buckets and routes requests
/// based on access tokens and their associated prefixes
#[derive(Clone)]
pub struct MultiStorageRouter {
    /// Map of bucket name to storage instance
    storages: Arc<HashMap<String, Arc<S3Storage>>>,
    /// Map of access token to service configuration
    token_map: Arc<HashMap<String, ResolvedServiceAccessToken>>,
}

impl MultiStorageRouter {
    /// Create a new multi-storage router from resolved configuration
    pub async fn from_config(config: &ResolvedConfig) -> Result<Self, StorageError> {
        let mut storages = HashMap::new();

        // Initialize storage for each bucket
        for bucket_config in &config.buckets {
            let storage = S3Storage::from_resolved_bucket(bucket_config).await?;
            storages.insert(bucket_config.name.clone(), Arc::new(storage));
        }

        let token_map = config.build_token_registry();

        Ok(Self {
            storages: Arc::new(storages),
            token_map: Arc::new(token_map),
        })
    }

    /// Test connectivity to all configured buckets
    /// This should be called during startup to validate bucket access
    pub async fn test_all_buckets(&self) -> Result<(), StorageError> {
        tracing::info!("Testing connectivity to all configured buckets...");

        for (bucket_name, storage) in self.storages.iter() {
            tracing::info!("Testing bucket: {}", bucket_name);
            storage.test_connection().await?;
        }

        tracing::info!("All bucket connectivity tests passed");
        Ok(())
    }

    /// Get storage and prefix for a given access token
    fn resolve_storage(&self, token: &str) -> Result<(Arc<S3Storage>, String), StorageError> {
        let service_config = self
            .token_map
            .get(token)
            .ok_or(StorageError::OperationFailed)?;

        let storage = self
            .storages
            .get(&service_config.bucket)
            .ok_or(StorageError::OperationFailed)?;

        Ok((storage.clone(), service_config.prefix.clone()))
    }

    /// Build the full key with prefix
    fn build_key(prefix: &str, hash: &str) -> String {
        if prefix.is_empty() {
            hash.to_string()
        } else {
            format!("{}/{}", prefix, hash)
        }
    }

    /// Check if object exists for the given token and hash
    pub async fn exists_with_token(&self, token: &str, hash: &str) -> Result<bool, StorageError> {
        let (storage, prefix) = self.resolve_storage(token)?;
        let key = Self::build_key(&prefix, hash);
        storage.exists(&key).await
    }

    /// Store object for the given token and hash
    pub async fn store_with_token(
        &self,
        token: &str,
        hash: &str,
        data: ReaderStream<impl AsyncRead + Send + Unpin + 'static>,
    ) -> Result<(), StorageError> {
        let (storage, prefix) = self.resolve_storage(token)?;
        let key = Self::build_key(&prefix, hash);
        storage.store(&key, data).await
    }

    /// Retrieve object for the given token and hash
    pub async fn retrieve_with_token(
        &self,
        token: &str,
        hash: &str,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>, StorageError> {
        let (storage, prefix) = self.resolve_storage(token)?;
        let key = Self::build_key(&prefix, hash);
        storage.retrieve(&key).await
    }

    /// Get the service configuration for a token
    pub fn get_token_config(&self, token: &str) -> Option<&ResolvedServiceAccessToken> {
        self.token_map.get(token)
    }

    /// Get all configured tokens
    pub fn tokens(&self) -> impl Iterator<Item = &String> {
        self.token_map.keys()
    }

    /// Get token names
    pub fn token_names(&self) -> impl Iterator<Item = &String> {
        self.token_map.values().map(|t| &t.name)
    }
}

// Implement StorageProvider for MultiStorageRouter
// Note: These implementations require a token context, so they're provided
// via the *_with_token methods above. The trait implementations are for
// compatibility but will fail if called directly without token context.
#[async_trait]
impl StorageProvider for MultiStorageRouter {
    async fn exists(&self, _hash: &str) -> Result<bool, StorageError> {
        Err(StorageError::OperationFailed)
    }

    async fn store(
        &self,
        _hash: &str,
        _data: ReaderStream<impl AsyncRead + Send + Unpin>,
    ) -> Result<(), StorageError> {
        Err(StorageError::OperationFailed)
    }

    async fn retrieve(
        &self,
        _hash: &str,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>, StorageError> {
        Err(StorageError::OperationFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_key_with_prefix() {
        let key = MultiStorageRouter::build_key("/ci", "abc123");
        assert_eq!(key, "/ci/abc123");
    }

    #[test]
    fn test_build_key_without_prefix() {
        let key = MultiStorageRouter::build_key("", "abc123");
        assert_eq!(key, "abc123");
    }

    #[test]
    fn test_build_key_with_nested_prefix() {
        let key = MultiStorageRouter::build_key("/team1/subteam", "abc123");
        assert_eq!(key, "/team1/subteam/abc123");
    }
}
