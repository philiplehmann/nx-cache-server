use clap::Parser;
use nx_cache_server::domain::config::Config;
use nx_cache_server::infra::multi_storage::MultiStorageRouter;
use nx_cache_server::server::run_server;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Parser)]
#[command(name = "nx-cache-server")]
#[command(about = "Nx Remote Cache Server - S3 Backend with YAML/TOML Configuration")]
struct Cli {
  #[arg(
    short = 'c',
    long = "config",
    env = "CONFIG_FILE",
    required_unless_present = "healthcheck",
    help = "Path to YAML or TOML configuration file"
  )]
  config_file: Option<PathBuf>,

  #[arg(
    long,
    help = "Query /health on the local server and exit 0 (healthy) or 1 (unhealthy); uses the config port if given, else 3000"
  )]
  healthcheck: bool,

  #[arg(long, env = "DEBUG", help = "Enable debug logging")]
  debug: bool,
}

/// Sends a plain HTTP request to the local `/health` endpoint and checks for a 2xx status.
async fn check_health(port: u16) -> bool {
  let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)).await else {
    return false;
  };
  let request = "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
  if stream.write_all(request.as_bytes()).await.is_err() {
    return false;
  }
  let mut buf = [0u8; 32];
  let mut read = 0;
  while read < 12 {
    match stream.read(&mut buf[read..]).await {
      Ok(0) | Err(_) => break,
      Ok(n) => read += n,
    }
  }
  // "HTTP/1.1 2xx"
  read >= 12 && buf.starts_with(b"HTTP/1.") && buf[9] == b'2'
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let cli = Cli::parse();

  if cli.healthcheck {
    let port = cli
      .config_file
      .as_deref()
      .and_then(|path| Config::from_file(path).ok())
      .map_or(3000, |config| config.port);
    let healthy = tokio::time::timeout(Duration::from_secs(4), check_health(port))
      .await
      .unwrap_or(false);
    std::process::exit(if healthy { 0 } else { 1 });
  }
  let config_file = cli.config_file.expect("config file is required");

  // Initialize logging
  if cli.debug {
    tracing_subscriber::fmt()
      .with_max_level(tracing::Level::DEBUG)
      .init();
  } else {
    tracing_subscriber::fmt::init();
  }

  tracing::info!("Loading configuration from: {}", config_file.display());

  // Load and parse configuration
  let config = match Config::from_file(&config_file) {
    Ok(config) => config,
    Err(e) => {
      eprintln!();
      eprintln!("Failed to load configuration file: {}", e);
      eprintln!();
      std::process::exit(1);
    },
  };

  // Resolve environment variables
  let resolved_config = match config.resolve_env_vars() {
    Ok(config) => config,
    Err(e) => {
      eprintln!();
      eprintln!("Configuration error: {}", e);
      eprintln!();
      std::process::exit(1);
    },
  };

  tracing::info!("Configuration loaded successfully");
  tracing::info!("  Buckets: {}", resolved_config.buckets.len());
  for bucket in &resolved_config.buckets {
    tracing::info!("    - {} ({})", bucket.name, bucket.bucket_name);
  }
  tracing::info!(
    "  Service Tokens: {}",
    resolved_config.service_access_tokens.len()
  );
  for token in &resolved_config.service_access_tokens {
    tracing::info!(
      "    - {} -> bucket: {}, prefix: {}",
      token.name,
      token.bucket,
      token.prefix
    );
  }

  // Initialize multi-storage router
  let storage = match MultiStorageRouter::from_config(&resolved_config).await {
    Ok(storage) => storage,
    Err(e) => {
      eprintln!();
      eprintln!("Failed to initialize storage: {}", e);
      eprintln!();
      eprintln!("Please check your AWS credentials and bucket configurations.");
      std::process::exit(1);
    },
  };

  tracing::info!("Storage initialized successfully");

  // Test bucket connectivity
  tracing::info!("Testing bucket connectivity...");
  if let Err(e) = storage.test_all_buckets().await {
    eprintln!();
    eprintln!("Bucket connectivity test failed: {}", e);
    eprintln!();
    eprintln!("Please verify:");
    eprintln!("  - AWS credentials are valid");
    eprintln!("  - Bucket names are correct");
    eprintln!("  - Buckets exist and are accessible");
    eprintln!("  - Region is correct");
    eprintln!("  - Network connectivity to S3/endpoint");
    std::process::exit(1);
  }

  // Run server
  tracing::info!("Server starting on port {}", resolved_config.port);
  if let Err(e) = run_server(storage, &resolved_config).await {
    eprintln!();
    eprintln!("Server error: {}", e);
    std::process::exit(1);
  }

  Ok(())
}
