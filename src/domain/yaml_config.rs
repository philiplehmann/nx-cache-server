use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum YamlConfigError {
  #[error("Failed to read config file: {0}")]
  FileRead(#[from] std::io::Error),
  #[error("Failed to parse YAML: {0}")]
  YamlParse(#[from] serde_yml::Error),
  #[error("Configuration validation error: {0}")]
  Validation(String),
  #[error("Environment variable not found: {0}")]
  EnvVarNotFound(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketConfig {
  /// Unique name for this bucket configuration
  pub name: String,

  /// S3 bucket name
  pub bucket_name: String,

  /// AWS Access Key ID (optional - auto-discovered if not provided)
  #[serde(skip_serializing_if = "Option::is_none")]
  pub access_key_id: Option<String>,

  /// Environment variable name holding the AWS Access Key ID
  #[serde(skip_serializing_if = "Option::is_none")]
  pub access_key_id_env: Option<String>,

  /// AWS Secret Access Key (optional - auto-discovered if not provided)
  #[serde(skip_serializing_if = "Option::is_none")]
  pub secret_access_key: Option<String>,

  /// Environment variable name holding the AWS Secret Access Key
  #[serde(skip_serializing_if = "Option::is_none")]
  pub secret_access_key_env: Option<String>,

  /// AWS Session Token (optional)
  #[serde(skip_serializing_if = "Option::is_none")]
  pub session_token: Option<String>,

  /// Environment variable name holding the AWS Session Token
  #[serde(skip_serializing_if = "Option::is_none")]
  pub session_token_env: Option<String>,

  /// AWS Region (optional - auto-discovered if not provided)
  #[serde(skip_serializing_if = "Option::is_none")]
  pub region: Option<String>,

  /// Custom S3 endpoint URL (for MinIO, etc.)
  #[serde(skip_serializing_if = "Option::is_none")]
  pub endpoint_url: Option<String>,

  /// Force path-style addressing (required for MinIO and some S3-compatible services)
  #[serde(default)]
  pub force_path_style: bool,

  /// S3 operation timeout in seconds
  #[serde(default = "default_timeout")]
  pub timeout: u64,
}

fn default_timeout() -> u64 {
  30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceAccessTokenConfig {
  /// Unique name for this service token
  pub name: String,

  /// Reference to the bucket name this token uses
  pub bucket: String,

  /// Prefix path in the bucket (e.g., "/ci", "/team1")
  #[serde(default)]
  pub prefix: String,

  /// Bearer token for authentication
  #[serde(skip_serializing_if = "Option::is_none")]
  pub access_token: Option<String>,

  /// Environment variable name holding the access token
  #[serde(skip_serializing_if = "Option::is_none")]
  pub access_token_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YamlConfig {
  /// List of bucket configurations
  pub buckets: Vec<BucketConfig>,

  /// List of service access tokens
  pub service_access_tokens: Vec<ServiceAccessTokenConfig>,

  /// HTTP server port (optional, defaults to 3000)
  #[serde(default = "default_port")]
  pub port: u16,

  /// Enable debug logging
  #[serde(default)]
  pub debug: bool,
}

fn default_port() -> u16 {
  3000
}

impl YamlConfig {
  /// Load configuration from a YAML file
  pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, YamlConfigError> {
    let content = fs::read_to_string(path)?;
    let config: YamlConfig = serde_yml::from_str(&content)?;
    config.validate()?;
    Ok(config)
  }

  /// Validate the configuration
  pub fn validate(&self) -> Result<(), YamlConfigError> {
    // Validate we have at least one bucket
    if self.buckets.is_empty() {
      return Err(YamlConfigError::Validation(
        "At least one bucket must be configured".to_string(),
      ));
    }

    // Validate bucket names are unique
    let mut bucket_names = std::collections::HashSet::new();
    for bucket in &self.buckets {
      if bucket.name.is_empty() {
        return Err(YamlConfigError::Validation(
          "Bucket name cannot be empty".to_string(),
        ));
      }
      if bucket.bucket_name.is_empty() {
        return Err(YamlConfigError::Validation(format!(
          "Bucket '{}' must have a bucketName",
          bucket.name
        )));
      }
      if !bucket_names.insert(&bucket.name) {
        return Err(YamlConfigError::Validation(format!(
          "Duplicate bucket name: {}",
          bucket.name
        )));
      }
    }

    // Validate we have at least one service token
    if self.service_access_tokens.is_empty() {
      return Err(YamlConfigError::Validation(
        "At least one service access token must be configured".to_string(),
      ));
    }

    // Validate service token names are unique
    let mut token_names = std::collections::HashSet::new();
    for token in &self.service_access_tokens {
      if token.name.is_empty() {
        return Err(YamlConfigError::Validation(
          "Service token name cannot be empty".to_string(),
        ));
      }
      if !token_names.insert(&token.name) {
        return Err(YamlConfigError::Validation(format!(
          "Duplicate service token name: {}",
          token.name
        )));
      }

      // Validate bucket reference exists
      if !bucket_names.contains(&token.bucket) {
        return Err(YamlConfigError::Validation(format!(
          "Service token '{}' references non-existent bucket '{}'",
          token.name, token.bucket
        )));
      }

      // Validate token is provided via value or env var
      if token.access_token.is_none() && token.access_token_env.is_none() {
        return Err(YamlConfigError::Validation(format!(
          "Service token '{}' must have either accessToken or accessTokenEnv",
          token.name
        )));
      }
    }

    // Validate port
    if self.port == 0 {
      return Err(YamlConfigError::Validation(
        "Port must be greater than 0".to_string(),
      ));
    }

    Ok(())
  }

  /// Resolve all environment variables and return a resolved configuration
  pub fn resolve_env_vars(&self) -> Result<ResolvedConfig, YamlConfigError> {
    let mut resolved_buckets = Vec::new();

    for bucket in &self.buckets {
      let access_key_id =
        Self::resolve_optional_env(&bucket.access_key_id, &bucket.access_key_id_env)?;

      let secret_access_key =
        Self::resolve_optional_env(&bucket.secret_access_key, &bucket.secret_access_key_env)?;

      let session_token =
        Self::resolve_optional_env(&bucket.session_token, &bucket.session_token_env)?;

      // Validate credential pairs
      match (&access_key_id, &secret_access_key) {
        (Some(_), None) => {
          return Err(YamlConfigError::Validation(format!(
            "Bucket '{}': if accessKeyId is provided, secretAccessKey must also be provided",
            bucket.name
          )));
        },
        (None, Some(_)) => {
          return Err(YamlConfigError::Validation(format!(
            "Bucket '{}': if secretAccessKey is provided, accessKeyId must also be provided",
            bucket.name
          )));
        },
        _ => {},
      }

      resolved_buckets.push(ResolvedBucketConfig {
        name: bucket.name.clone(),
        bucket_name: bucket.bucket_name.clone(),
        access_key_id,
        secret_access_key,
        session_token,
        region: bucket.region.clone(),
        endpoint_url: bucket.endpoint_url.clone(),
        force_path_style: bucket.force_path_style,
        timeout: bucket.timeout,
      });
    }

    let mut resolved_tokens = Vec::new();
    for token in &self.service_access_tokens {
      let access_token = Self::resolve_required_env(
        &token.access_token,
        &token.access_token_env,
        &format!("Service token '{}' accessToken", token.name),
      )?;

      resolved_tokens.push(ResolvedServiceAccessToken {
        name: token.name.clone(),
        bucket: token.bucket.clone(),
        prefix: Self::normalize_prefix(&token.prefix),
        access_token,
      });
    }

    Ok(ResolvedConfig {
      buckets: resolved_buckets,
      service_access_tokens: resolved_tokens,
      port: self.port,
      debug: self.debug,
    })
  }

  /// Resolve an optional field that can be a value or env var reference
  fn resolve_optional_env(
    value: &Option<String>,
    env_var: &Option<String>,
  ) -> Result<Option<String>, YamlConfigError> {
    match (value, env_var) {
      (Some(v), _) => Ok(Some(v.clone())),
      (None, Some(env_name)) => match std::env::var(env_name) {
        Ok(v) => Ok(Some(v)),
        Err(_) => Ok(None), // Environment variable not set is OK for optional fields
      },
      (None, None) => Ok(None),
    }
  }

  /// Resolve a required field that must be a value or env var reference
  fn resolve_required_env(
    value: &Option<String>,
    env_var: &Option<String>,
    field_name: &str,
  ) -> Result<String, YamlConfigError> {
    match (value, env_var) {
      (Some(v), _) => Ok(v.clone()),
      (None, Some(env_name)) => std::env::var(env_name).map_err(|_| {
        YamlConfigError::EnvVarNotFound(format!(
          "{}: environment variable '{}' not found",
          field_name, env_name
        ))
      }),
      (None, None) => Err(YamlConfigError::Validation(format!(
        "{}: must be provided",
        field_name
      ))),
    }
  }

  /// Normalize prefix to ensure it starts with / and doesn't end with /
  fn normalize_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
      return String::new();
    }

    let mut normalized = if !trimmed.starts_with('/') {
      format!("/{}", trimmed)
    } else {
      trimmed.to_string()
    };

    // Remove trailing slash
    if normalized.len() > 1 && normalized.ends_with('/') {
      normalized.pop();
    }

    normalized
  }
}

/// Fully resolved configuration with all environment variables loaded
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
  pub buckets: Vec<ResolvedBucketConfig>,
  pub service_access_tokens: Vec<ResolvedServiceAccessToken>,
  pub port: u16,
  pub debug: bool,
}

#[derive(Debug, Clone)]
pub struct ResolvedBucketConfig {
  pub name: String,
  pub bucket_name: String,
  pub access_key_id: Option<String>,
  pub secret_access_key: Option<String>,
  pub session_token: Option<String>,
  pub region: Option<String>,
  pub endpoint_url: Option<String>,
  pub force_path_style: bool,
  pub timeout: u64,
}

#[derive(Debug, Clone)]
pub struct ResolvedServiceAccessToken {
  pub name: String,
  pub bucket: String,
  pub prefix: String,
  pub access_token: String,
}

impl ResolvedConfig {
  /// Get bucket configuration by name
  pub fn get_bucket(&self, name: &str) -> Option<&ResolvedBucketConfig> {
    self.buckets.iter().find(|b| b.name == name)
  }

  /// Find service token by access token value
  pub fn find_service_token(&self, token: &str) -> Option<&ResolvedServiceAccessToken> {
    self
      .service_access_tokens
      .iter()
      .find(|t| t.access_token == token)
  }

  /// Build a token registry mapping tokens to their configurations
  pub fn build_token_registry(&self) -> HashMap<String, ResolvedServiceAccessToken> {
    self
      .service_access_tokens
      .iter()
      .map(|t| (t.access_token.clone(), t.clone()))
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_normalize_prefix() {
    assert_eq!(YamlConfig::normalize_prefix(""), "");
    assert_eq!(YamlConfig::normalize_prefix("/"), "/");
    assert_eq!(YamlConfig::normalize_prefix("/ci"), "/ci");
    assert_eq!(YamlConfig::normalize_prefix("ci"), "/ci");
    assert_eq!(YamlConfig::normalize_prefix("/ci/"), "/ci");
    assert_eq!(YamlConfig::normalize_prefix("ci/"), "/ci");
    assert_eq!(
      YamlConfig::normalize_prefix("/team1/subteam"),
      "/team1/subteam"
    );
    assert_eq!(YamlConfig::normalize_prefix("  /ci  "), "/ci");
  }

  #[test]
  fn test_validation_empty_buckets() {
    let config = YamlConfig {
      buckets: vec![],
      service_access_tokens: vec![ServiceAccessTokenConfig {
        name: "test".to_string(),
        bucket: "bucket1".to_string(),
        prefix: "/ci".to_string(),
        access_token: Some("token".to_string()),
        access_token_env: None,
      }],
      port: 3000,
      debug: false,
    };

    assert!(config.validate().is_err());
  }

  #[test]
  fn test_validation_empty_tokens() {
    let config = YamlConfig {
      buckets: vec![BucketConfig {
        name: "bucket1".to_string(),
        bucket_name: "my-bucket".to_string(),
        access_key_id: None,
        access_key_id_env: None,
        secret_access_key: None,
        secret_access_key_env: None,
        session_token: None,
        session_token_env: None,
        region: Some("us-west-2".to_string()),
        endpoint_url: None,
        force_path_style: false,
        timeout: 30,
      }],
      service_access_tokens: vec![],
      port: 3000,
      debug: false,
    };

    assert!(config.validate().is_err());
  }

  #[test]
  fn test_validation_duplicate_bucket_names() {
    let config = YamlConfig {
      buckets: vec![
        BucketConfig {
          name: "bucket1".to_string(),
          bucket_name: "my-bucket-1".to_string(),
          access_key_id: None,
          access_key_id_env: None,
          secret_access_key: None,
          secret_access_key_env: None,
          session_token: None,
          session_token_env: None,
          region: Some("us-west-2".to_string()),
          endpoint_url: None,
          force_path_style: false,
          timeout: 30,
        },
        BucketConfig {
          name: "bucket1".to_string(),
          bucket_name: "my-bucket-2".to_string(),
          access_key_id: None,
          access_key_id_env: None,
          secret_access_key: None,
          secret_access_key_env: None,
          session_token: None,
          session_token_env: None,
          region: Some("us-west-2".to_string()),
          endpoint_url: None,
          force_path_style: false,
          timeout: 30,
        },
      ],
      service_access_tokens: vec![ServiceAccessTokenConfig {
        name: "test".to_string(),
        bucket: "bucket1".to_string(),
        prefix: "/ci".to_string(),
        access_token: Some("token".to_string()),
        access_token_env: None,
      }],
      port: 3000,
      debug: false,
    };

    assert!(config.validate().is_err());
  }

  #[test]
  fn test_validation_nonexistent_bucket_reference() {
    let config = YamlConfig {
      buckets: vec![BucketConfig {
        name: "bucket1".to_string(),
        bucket_name: "my-bucket".to_string(),
        access_key_id: None,
        access_key_id_env: None,
        secret_access_key: None,
        secret_access_key_env: None,
        session_token: None,
        session_token_env: None,
        region: Some("us-west-2".to_string()),
        endpoint_url: None,
        force_path_style: false,
        timeout: 30,
      }],
      service_access_tokens: vec![ServiceAccessTokenConfig {
        name: "test".to_string(),
        bucket: "bucket2".to_string(), // Non-existent bucket
        prefix: "/ci".to_string(),
        access_token: Some("token".to_string()),
        access_token_env: None,
      }],
      port: 3000,
      debug: false,
    };

    assert!(config.validate().is_err());
  }

  #[test]
  fn test_validation_success() {
    let config = YamlConfig {
      buckets: vec![BucketConfig {
        name: "bucket1".to_string(),
        bucket_name: "my-bucket".to_string(),
        access_key_id: None,
        access_key_id_env: None,
        secret_access_key: None,
        secret_access_key_env: None,
        session_token: None,
        session_token_env: None,
        region: Some("us-west-2".to_string()),
        endpoint_url: None,
        force_path_style: false,
        timeout: 30,
      }],
      service_access_tokens: vec![ServiceAccessTokenConfig {
        name: "test".to_string(),
        bucket: "bucket1".to_string(),
        prefix: "/ci".to_string(),
        access_token: Some("token".to_string()),
        access_token_env: None,
      }],
      port: 3000,
      debug: false,
    };

    assert!(config.validate().is_ok());
  }
}
