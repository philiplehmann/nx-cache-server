use clap::Parser;
use nx_cache_server::domain::yaml_config::YamlConfig;
use nx_cache_server::infra::multi_storage::MultiStorageRouter;
use nx_cache_server::server::run_server;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "nx-cache-server")]
#[command(about = "Nx Remote Cache Server - S3 Backend with YAML Configuration")]
struct Cli {
    #[arg(
        short = 'c',
        long = "config",
        env = "CONFIG_FILE",
        help = "Path to YAML configuration file"
    )]
    config_file: PathBuf,

    #[arg(long, env = "DEBUG", help = "Enable debug logging")]
    debug: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize logging
    if cli.debug {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();
    } else {
        tracing_subscriber::fmt::init();
    }

    tracing::info!("Loading configuration from: {}", cli.config_file.display());

    // Load and parse YAML configuration
    let yaml_config = match YamlConfig::from_file(&cli.config_file) {
        Ok(config) => config,
        Err(e) => {
            eprintln!();
            eprintln!("Failed to load configuration file: {}", e);
            eprintln!();
            std::process::exit(1);
        }
    };

    // Resolve environment variables
    let resolved_config = match yaml_config.resolve_env_vars() {
        Ok(config) => config,
        Err(e) => {
            eprintln!();
            eprintln!("Configuration error: {}", e);
            eprintln!();
            std::process::exit(1);
        }
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
        }
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
