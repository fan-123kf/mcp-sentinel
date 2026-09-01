use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::{info, Level};
use tracing_subscriber::EnvFilter;

mod backend;
mod config;
mod decision_trace;
mod gateway;
mod governance;
mod health;
mod router;
mod storage;

use config::Config;
use storage::StorageManager;

#[derive(Parser)]
#[command(name = "mcp-sentinel")]
#[command(about = "Intelligent MCP gateway with health-driven adaptive routing", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, value_name = "FILE", default_value = "sentinel.toml")]
    config: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the gateway server
    Start,
    /// Show current gateway status (config + db summary)
    Status,
    /// Generate health report
    Report {
        #[arg(short, long)]
        output: Option<PathBuf>,

        #[arg(long, default_value = "7")]
        days: u64,
    },
    /// List all tools with health scores
    Tools {
        #[arg(long, default_value = "health_score")]
        sort_by: String,
    },
    /// Generate a pruned sentinel.toml from cleanup suggestions
    /// (FORMAT = toml)
    GenConfig {
        #[arg(long, value_name = "FORMAT", default_value = "toml")]
        format: String,
        /// When true, also remove tools unused for 3+ days (vs default 7)
        #[arg(long)]
        aggressive: bool,
        /// Write to this path instead of stdout
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Start => {
            info!("Starting mcp-sentinel gateway...");
            start_server(cli.config).await?;
        }
        Commands::Status => {
            show_status(cli.config).await?;
        }
        Commands::Report { output, days } => {
            generate_report(cli.config, output, days).await?;
        }
        Commands::Tools { sort_by } => {
            list_tools(cli.config, &sort_by).await?;
        }
        Commands::GenConfig {
            format,
            aggressive,
            output,
        } => {
            gen_config(cli.config, &format, aggressive, output).await?;
        }
    }

    Ok(())
}

async fn start_server(config_path: PathBuf) -> anyhow::Result<()> {
    // Load configuration
    let config = Config::load(&config_path)?;
    info!("Configuration loaded from {}", config_path.display());
    info!("Gateway will listen on port {}", config.gateway.port);

    // Initialize storage
    let storage = StorageManager::new(&config.storage.db_path).await?;
    let storage = std::sync::Arc::new(storage);
    info!("Storage initialized at {}", config.storage.db_path);

    // Initialize health manager with storage
    let health_manager = health::HealthManager::new()
        .with_failure_limit(config.health.consecutive_failure_limit)
        .with_storage(storage.clone());

    // Initialize backend manager
    let backend_manager = backend::BackendManager::new(&config, health_manager.clone()).await?;

    // Register all tools in storage
    let tools = backend_manager.list_all_tools().await;
    info!("Loaded {} tools from backends", tools.len());

    for tool in &tools {
        let registry = storage::ToolRegistry {
            tool_id: tool.tool_id.clone(),
            server_name: tool.server_name.clone().unwrap_or_default(),
            tool_name: tool.name.clone(),
            description: tool.description.clone(),
            schema_json: serde_json::to_string(&tool.input_schema).unwrap_or_default(),
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
        };
        storage.register_tool(registry).await?;
    }

    // Initialize router
    let router = router::SemanticRouter::new(health_manager.clone());
    router.index_tools(tools).await;

    // Start daily aggregation task
    let storage_clone = storage.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3600)); // Every hour
        loop {
            interval.tick().await;
            let yesterday = (chrono::Utc::now() - chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string();
            if let Err(e) = storage_clone.aggregate_daily_stats(&yesterday).await {
                tracing::warn!("Failed to aggregate daily stats: {}", e);
            }
        }
    });

    // Start cleanup task
    let storage_clone = storage.clone();
    let retention_days = config.storage.retention_days;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(86400)); // Daily
        loop {
            interval.tick().await;
            if let Err(e) = storage_clone.cleanup_old_records(retention_days).await {
                tracing::warn!("Failed to cleanup old records: {}", e);
            } else {
                tracing::info!("Cleaned up records older than {} days", retention_days);
            }
        }
    });

    // Start gateway server
    gateway::start_gateway(
        config,
        backend_manager,
        router,
        health_manager,
        Some(storage),
    )
    .await?;

    Ok(())
}

async fn generate_report(
    config_path: PathBuf,
    output: Option<PathBuf>,
    days: u64,
) -> anyhow::Result<()> {
    let config = Config::load(&config_path)?;
    let storage = StorageManager::new(&config.storage.db_path).await?;
    let storage = std::sync::Arc::new(storage);

    let health_manager = health::HealthManager::new().with_storage(storage.clone());

    // Load health data from storage
    let tools = storage.get_all_registered_tools().await?;
    println!("Generating health report for {} tools...", tools.len());

    let report = health::generate_health_report(&health_manager, Some(&storage), days).await?;

    if let Some(output_path) = output {
        tokio::fs::write(&output_path, &report).await?;
        println!("✅ Report written to: {}", output_path.display());
    } else {
        println!("{}", report);
    }

    Ok(())
}

async fn list_tools(config_path: PathBuf, sort_by: &str) -> anyhow::Result<()> {
    let config = Config::load(&config_path)?;
    let storage = StorageManager::new(&config.storage.db_path).await?;
    let storage = std::sync::Arc::new(storage);

    let health_manager = health::HealthManager::new().with_storage(storage.clone());

    let mut scores = health_manager.get_all_scores().await;

    // Sort by requested field
    match sort_by {
        "health_score" => {
            scores.sort_by(|a, b| {
                b.health_score
                    .partial_cmp(&a.health_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        "tool_id" => {
            scores.sort_by(|a, b| a.tool_id.cmp(&b.tool_id));
        }
        _ => {}
    }

    println!(
        "\n{:<50} {:>12} {:>10}",
        "Tool ID", "Health Score", "Status"
    );
    println!("{}", "-".repeat(75));

    for score in scores {
        let status = if score.zombie {
            "🧟 ZOMBIE"
        } else if score.degraded {
            "⚠️  DEGRADED"
        } else {
            "✅ HEALTHY"
        };

        println!(
            "{:<50} {:>12.3} {:>10}",
            score.tool_id, score.health_score, status
        );
    }

    println!();
    Ok(())
}

async fn show_status(config_path: PathBuf) -> anyhow::Result<()> {
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            println!("❌ Cannot load config {}: {}", config_path.display(), e);
            println!("   Hint: copy sentinel.toml.example to sentinel.toml");
            return Ok(());
        }
    };

    let storage = match StorageManager::new(&config.storage.db_path).await {
        Ok(s) => std::sync::Arc::new(s),
        Err(e) => {
            println!("❌ Cannot open database {}: {}", config.storage.db_path, e);
            return Ok(());
        }
    };

    let health_manager = health::HealthManager::new().with_storage(storage.clone());

    let scores = health_manager.get_all_scores().await;
    let total = scores.len();
    let healthy = scores.iter().filter(|s| !s.degraded && !s.zombie).count();
    let degraded = scores.iter().filter(|s| s.degraded).count();
    let zombie = scores.iter().filter(|s| s.zombie).count();

    let registered = storage
        .get_all_registered_tools()
        .await
        .unwrap_or_default()
        .len();
    let active_backends = config.backends.len();

    println!("📊 mcp-sentinel Status");
    println!("{}", "-".repeat(40));
    println!("Config file:       {}", config_path.display());
    println!("Gateway port:      {}", config.gateway.port);
    println!("Backends:          {} configured", active_backends);
    println!("Database:          {}", config.storage.db_path);
    println!();
    println!("Tools (in-memory): {} tracked", total);
    println!("  ✅ Healthy:      {}", healthy);
    println!("  ⚠️  Degraded:    {}", degraded);
    println!("  🧟 Zombie:       {}", zombie);
    println!();
    println!("Tools (registered in db): {}", registered);
    println!("Log level:         {}", config.gateway.log_level);
    println!("Routing strategy:  {}", config.routing.strategy);
    println!("Health weight:     {}", config.routing.health_weight);
    Ok(())
}

async fn gen_config(
    config_path: PathBuf,
    format: &str,
    aggressive: bool,
    output: Option<PathBuf>,
) -> anyhow::Result<()> {
    if format != "toml" {
        anyhow::bail!(
            "Unsupported format '{}'. Only 'toml' is currently supported.",
            format
        );
    }

    let config = Config::load(&config_path)?;
    let storage = std::sync::Arc::new(StorageManager::new(&config.storage.db_path).await?);

    let health_manager = health::HealthManager::new().with_storage(storage.clone());

    let suggestions =
        health::generate_cleanup_suggestions(&health_manager, Some(&storage), aggressive).await?;

    let zombie_server_names: Vec<String> = suggestions
        .get("zombie_servers")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.get("server").and_then(|v| v.as_str()))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    let removed: Vec<&String> = config
        .backends
        .keys()
        .filter(|name| zombie_server_names.iter().any(|z| z == *name))
        .collect();

    let mut pruned = config.clone();
    for name in &removed {
        pruned.backends.remove(*name);
    }

    let serialized = toml::to_string_pretty(&pruned)
        .map_err(|e| anyhow::anyhow!("Failed to serialize config: {}", e))?;

    let header = format!(
        "# Generated by `mcp-sentinel gen-config`\n# Original: {}\n# Removed backends: {}\n# Estimated token savings: {} tokens/turn\n\n",
        config_path.display(),
        if removed.is_empty() {
            "(none)".to_string()
        } else {
            removed
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        },
        suggestions
            .get("estimated_token_savings")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    );

    let body = format!("{}{}", header, serialized);

    if let Some(path) = output {
        tokio::fs::write(&path, &body).await?;
        println!(
            "✅ Pruned config written to {} (removed {} zombie backend(s))",
            path.display(),
            removed.len()
        );
    } else {
        print!("{}", body);
    }

    Ok(())
}
