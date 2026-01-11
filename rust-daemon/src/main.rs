//! Agent Monitor Daemon - Rust Implementation
//!
//! A high-performance daemon for monitoring AI agent sessions across multiple tools.

mod api;
mod adapters;
mod analytics;
mod config;
mod events;
mod integration;
mod integrations;
mod models;
mod storage;
mod tui;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};
use std::io::{self, BufRead};
use std::os::unix::net::UnixStream;
use std::io::Write;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use crate::config::Config;
// Note: models types used via adapters and storage modules

// Cosmic UI colors (for future use with colored output)
const AURORA_BLUE: &str = "\x1b[38;5;117m";
const COSMIC_VIOLET: &str = "\x1b[38;5;147m";
const STELLAR_WHITE: &str = "\x1b[38;5;231m";
const PULSE_CYAN: &str = "\x1b[38;5;51m";
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

#[derive(Parser)]
#[command(name = "agent-monitor")]
#[command(about = "Monitor AI agent sessions across multiple tools")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Enable debug output
    #[arg(short, long, global = true)]
    debug: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the agent monitor daemon
    Daemon {
        /// Path to config file
        #[arg(short, long)]
        config: Option<String>,

        /// Skip startup animation
        #[arg(long)]
        no_animation: bool,
    },

    /// Handle hook events from Claude Code
    Hook {
        /// Hook event type (SessionStart, PreToolUse, PostToolUse, SubagentStop, etc.)
        event_type: String,
    },

    /// Show daemon status
    Status {
        /// Output as JSON
        #[arg(short, long)]
        json: bool,

        /// Skip animations
        #[arg(long)]
        no_animation: bool,
    },

    /// List sessions
    Sessions {
        /// Maximum number of sessions to show
        #[arg(short, long, default_value = "20")]
        limit: usize,

        /// Show all sessions, not just active
        #[arg(short, long)]
        all: bool,

        /// Output as JSON
        #[arg(short, long)]
        json: bool,
    },

    /// Install Claude Code hooks for real-time monitoring
    InstallHooks,

    /// Manage configuration
    Config {
        /// Show current configuration
        #[arg(short, long)]
        show: bool,

        /// Initialize default configuration file
        #[arg(short, long)]
        init: bool,
    },

    /// Launch web dashboard
    Web {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,

        /// Port to bind to
        #[arg(short, long, default_value = "8765")]
        port: u16,
    },

    /// Interactive live monitoring dashboard
    Watch,

    /// Clear sessions from database
    Clear {
        /// Clear only sessions of specific agent type (cursor, aider, etc.)
        #[arg(short, long)]
        agent_type: Option<String>,

        /// Clear all sessions and events
        #[arg(short = 'A', long)]
        all: bool,
    },

    /// Show version
    Version,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Setup logging (skip for hook command to avoid polluting Claude Code)
    let is_hook = matches!(cli.command, Commands::Hook { .. });

    if !is_hook {
        let level = if cli.debug {
            Level::DEBUG
        } else if cli.verbose {
            Level::INFO
        } else {
            Level::WARN
        };

        let _ = FmtSubscriber::builder()
            .with_max_level(level)
            .with_target(false)
            .try_init();
    }

    match cli.command {
        Commands::Daemon { config, no_animation } => {
            run_daemon(config, no_animation).await?;
        }
        Commands::Hook { event_type } => {
            handle_hook(&event_type).await?;
        }
        Commands::Status { json, no_animation } => {
            show_status(json, no_animation).await?;
        }
        Commands::Sessions { limit, all, json } => {
            list_sessions(limit, all, json).await?;
        }
        Commands::InstallHooks => {
            install_hooks().await?;
        }
        Commands::Config { show, init } => {
            manage_config(show, init).await?;
        }
        Commands::Web { host, port } => {
            run_web(&host, port).await?;
        }
        Commands::Watch => {
            run_watch().await?;
        }
        Commands::Clear { agent_type, all } => {
            run_clear(agent_type, all).await?;
        }
        Commands::Version => {
            print_version();
        }
    }

    Ok(())
}

/// Print cosmic-styled version
fn print_version() {
    println!("{}✦  ✧ ★    ⋆  ✶    ★   ⋆  ✦{}", DIM, RESET);
    println!(
        "  {}✦{} {}Agent Monitor{} {}v{}{}",
        BOLD, RESET,
        BOLD, RESET,
        AURORA_BLUE, env!("CARGO_PKG_VERSION"), RESET
    );
    println!("{}  ⋆    ✶     ★   ⋆  ✧  ★{}", DIM, RESET);
}

/// Print cosmic-styled banner
fn print_banner(no_animation: bool) {
    if no_animation {
        println!(
            "{}✦ AGENT MONITOR ✦{} v{}",
            AURORA_BLUE, RESET, env!("CARGO_PKG_VERSION")
        );
        return;
    }

    let starfield = "  ✦     ⋆   ★       ✧    ✶   ⋆  ★      ✦    ";
    println!("{}{}{}", DIM, starfield, RESET);
    println!();
    println!(
        "   {}{}░█▀█░█▀▀░█▀▀░█▀█░▀█▀░░░█▄█░█▀█░█▀█░▀█▀░▀█▀░█▀█░█▀▄{}",
        BOLD, AURORA_BLUE, RESET
    );
    println!(
        "   {}{}░█▀█░█░█░█▀▀░█░█░░█░░░░█░█░█░█░█░█░░█░░░█░░█░█░█▀▄{}",
        BOLD, AURORA_BLUE, RESET
    );
    println!(
        "   {}{}░▀░▀░▀▀▀░▀▀▀░▀░▀░░▀░░░░▀░▀░▀▀▀░▀░▀░▀▀▀░░▀░░▀▀▀░▀░▀{}",
        BOLD, AURORA_BLUE, RESET
    );
    println!();
    println!(
        "         {}✦ AI Session Monitoring • Real-time Insights ✦{}",
        COSMIC_VIOLET, RESET
    );
    println!();
    println!("{}{}{}", DIM, starfield, RESET);
    println!();
}

async fn run_daemon(config_path: Option<String>, no_animation: bool) -> Result<()> {
    print_banner(no_animation);

    let config = match config_path {
        Some(path) => Config::load(&path)?,
        None => Config::default(),
    };

    println!(
        "{}╭─────────────────────────────────────────────────────╮{}",
        AURORA_BLUE, RESET
    );
    println!(
        "{}│{} {}✦ Daemon Starting{}                                  {}│{}",
        AURORA_BLUE, RESET, BOLD, RESET, AURORA_BLUE, RESET
    );
    println!(
        "{}│{}                                                     {}│{}",
        AURORA_BLUE, RESET, AURORA_BLUE, RESET
    );
    println!(
        "{}│{} {}📁 Data:{}   {:?}",
        AURORA_BLUE, RESET, DIM, RESET, config.data_dir
    );
    println!(
        "{}│{} {}🔌 Socket:{} {:?}",
        AURORA_BLUE, RESET, DIM, RESET, config.socket_path
    );
    println!(
        "{}│{} {}🌐 Port:{}   {}",
        AURORA_BLUE, RESET, DIM, RESET, config.http_port
    );
    println!(
        "{}╰─────────────────────────────────────────────────────╯{}",
        AURORA_BLUE, RESET
    );
    println!();

    info!("Starting Agent Monitor Daemon");

    // Initialize storage
    let storage = storage::Storage::new(&config.db_path).await?;
    storage.initialize().await?;

    // Initialize event bus
    let event_bus = events::EventBus::new();

    // Initialize adapters (all available)
    let mut adapters = adapters::AdapterRegistry::new(&config, event_bus.clone(), storage.clone());
    adapters.register_all().await?;

    // Start adapters
    adapters.start_all().await?;

    // Start IPC server
    let ipc_server = api::IpcServer::new(&config.socket_path, storage.clone());
    tokio::spawn(async move {
        if let Err(e) = ipc_server.run().await {
            tracing::error!("IPC server error: {}", e);
        }
    });

    println!("  {}● Connected{} - Daemon running", PULSE_CYAN, RESET);
    info!("Daemon started successfully");

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;

    println!();
    println!("{}─────────────────────────────────────────{}", AURORA_BLUE, RESET);
    println!("  {}✦ Shutting down gracefully...{}", COSMIC_VIOLET, RESET);
    println!("{}─────────────────────────────────────────{}", AURORA_BLUE, RESET);

    adapters.stop_all().await?;

    Ok(())
}

/// Handle hook events from Claude Code.
/// This is called by Claude Code hooks with event data on stdin.
async fn handle_hook(event_type: &str) -> Result<()> {
    // Read stdin (non-blocking check, then read)
    let stdin = io::stdin();
    let mut stdin_data = String::new();

    // Try to read available data
    for line in stdin.lock().lines() {
        match line {
            Ok(l) => stdin_data.push_str(&l),
            Err(_) => break,
        }
        break; // Only read one line/block
    }

    // Parse the event data
    let event_data: serde_json::Value = if !stdin_data.is_empty() {
        serde_json::from_str(&stdin_data).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Build hook message
    let message = serde_json::json!({
        "type": "hook_event",
        "event_type": event_type,
        "timestamp": Utc::now().to_rfc3339(),
        "data": event_data,
    });

    // Try to send to daemon via Unix socket
    let socket_path = "/tmp/agent-monitor.sock";
    if let Ok(mut stream) = UnixStream::connect(socket_path) {
        let msg = serde_json::to_string(&message)? + "\n";
        let _ = stream.write_all(msg.as_bytes());
    }
    // Silently fail if daemon isn't running - don't block Claude Code

    Ok(())
}

async fn show_status(json_output: bool, no_animation: bool) -> Result<()> {
    let config = Config::default();

    if !config.db_path.exists() {
        if json_output {
            println!(r#"{{"error": "Database not found"}}"#);
        } else {
            println!("{}✗ Error:{} Database not found. Is the daemon running?",
                "\x1b[38;5;196m", RESET);
        }
        return Ok(());
    }

    let storage = storage::Storage::new(&config.db_path).await?;
    let sessions = storage.get_active_sessions(100).await?;
    let metrics = storage.get_summary_metrics(24).await?;

    if json_output {
        let output = serde_json::json!({
            "active_sessions": sessions.len(),
            "metrics": metrics,
            "sessions": sessions,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Cosmic-styled output
    if !no_animation {
        println!("{}  ✦   ⋆  ★    ✧  ✶    ★   ⋆{}", DIM, RESET);
    }

    println!(
        "{}╭──────────────────────── ✦ Agent Monitor Status ✦ ────────────────────────╮{}",
        AURORA_BLUE, RESET
    );
    println!(
        "{}│{}                                                                          {}│{}",
        AURORA_BLUE, RESET, AURORA_BLUE, RESET
    );
    println!(
        "{}│{}  {}● Active Sessions:{} {}{}                                                    {}│{}",
        AURORA_BLUE, RESET, BOLD, RESET, PULSE_CYAN, sessions.len(), AURORA_BLUE, RESET
    );
    println!(
        "{}│{}                                                                          {}│{}",
        AURORA_BLUE, RESET, AURORA_BLUE, RESET
    );
    println!(
        "{}│{}  {}📊 24-Hour Summary{}                                                      {}│{}",
        AURORA_BLUE, RESET, BOLD, RESET, AURORA_BLUE, RESET
    );
    println!(
        "{}│{}  {}──────────────────────────────{}                                          {}│{}",
        AURORA_BLUE, RESET, DIM, RESET, AURORA_BLUE, RESET
    );
    println!(
        "{}│{}     Sessions:  {:>8}                                                  {}│{}",
        AURORA_BLUE, RESET, metrics.total_sessions, AURORA_BLUE, RESET
    );
    println!(
        "{}│{}     Messages:  {:>8}                                                  {}│{}",
        AURORA_BLUE, RESET, metrics.total_messages, AURORA_BLUE, RESET
    );
    println!(
        "{}│{}     Cost:      {}${:>7.2}{}                                                  {}│{}",
        AURORA_BLUE, RESET, COSMIC_VIOLET, metrics.total_cost, RESET, AURORA_BLUE, RESET
    );
    println!(
        "{}│{}                                                                          {}│{}",
        AURORA_BLUE, RESET, AURORA_BLUE, RESET
    );
    println!(
        "{}╰──────────────────────────────────────────────────────────────────────────╯{}",
        AURORA_BLUE, RESET
    );

    // Sessions table
    if !sessions.is_empty() {
        println!();
        println!(
            "{}                         ✦ Active Sessions ✦{}",
            AURORA_BLUE, RESET
        );
        println!(
            "{}╭───────────────────┬─────────────┬──────────┬──────────┬────────╮{}",
            AURORA_BLUE, RESET
        );
        println!(
            "{}│{} {}Project{}           {}│{} {}Type{}        {}│{} {}Messages{} {}│{} {}Duration{} {}│{} {}Status{} {}│{}",
            AURORA_BLUE, RESET, BOLD, RESET, AURORA_BLUE, RESET, BOLD, RESET,
            AURORA_BLUE, RESET, BOLD, RESET, AURORA_BLUE, RESET, BOLD, RESET,
            AURORA_BLUE, RESET, BOLD, RESET, AURORA_BLUE, RESET
        );
        println!(
            "{}├───────────────────┼─────────────┼──────────┼──────────┼────────┤{}",
            AURORA_BLUE, RESET
        );

        for session in &sessions {
            let project = session.project_path.split('/').last().unwrap_or("—");
            let project_display = if project.len() > 17 {
                format!("{}…", &project[..16])
            } else {
                format!("{:<17}", project)
            };

            let status = if session.status.to_string() == "active" {
                format!("{}🟢{}", PULSE_CYAN, RESET)
            } else {
                "⚪".to_string()
            };

            let duration = format_duration(session.duration_seconds);

            println!(
                "{}│{} {:<17} {}│{} {:<11} {}│{} {:>8} {}│{} {:>8} {}│{} {:^6} {}│{}",
                AURORA_BLUE, RESET, project_display,
                AURORA_BLUE, RESET, session.agent_type.to_string(),
                AURORA_BLUE, RESET, session.message_count,
                AURORA_BLUE, RESET, duration,
                AURORA_BLUE, RESET, status, AURORA_BLUE, RESET
            );
        }

        println!(
            "{}╰───────────────────┴─────────────┴──────────┴──────────┴────────╯{}",
            AURORA_BLUE, RESET
        );
    }

    if !no_animation {
        println!("{}  ⋆    ✶     ★   ⋆  ✧  ★{}", DIM, RESET);
    }

    Ok(())
}

async fn list_sessions(limit: usize, all: bool, json_output: bool) -> Result<()> {
    let config = Config::default();
    let storage = storage::Storage::new(&config.db_path).await?;

    let sessions = if all {
        storage.get_recent_sessions(168, limit).await?
    } else {
        storage.get_active_sessions(limit).await?
    };

    if json_output {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }

    if sessions.is_empty() {
        println!("{}✦ No sessions found{}", COSMIC_VIOLET, RESET);
        return Ok(());
    }

    println!("{}  ✦   ⋆  ★    ✧  ✶    ★   ⋆{}", DIM, RESET);
    println!(
        "{}                              ✦ Sessions ✦{}",
        AURORA_BLUE, RESET
    );
    println!(
        "{}╭──────────┬──────────┬─────────────┬──────────┬──────────┬─────────┬─────────╮{}",
        AURORA_BLUE, RESET
    );
    println!(
        "{}│{} {}ID{}       {}│{} {}Project{}  {}│{} {}Type{}        {}│{} {}Status{}   {}│{} {}Messages{} {}│{} {}Tokens{}  {}│{} {}Cost{}    {}│{}",
        AURORA_BLUE, RESET, BOLD, RESET, AURORA_BLUE, RESET, BOLD, RESET,
        AURORA_BLUE, RESET, BOLD, RESET, AURORA_BLUE, RESET, BOLD, RESET,
        AURORA_BLUE, RESET, BOLD, RESET, AURORA_BLUE, RESET, BOLD, RESET,
        AURORA_BLUE, RESET, BOLD, RESET, AURORA_BLUE, RESET
    );
    println!(
        "{}├──────────┼──────────┼─────────────┼──────────┼──────────┼─────────┼─────────┤{}",
        AURORA_BLUE, RESET
    );

    for session in &sessions {
        let project = session.project_path.split('/').last().unwrap_or("—");
        let project_display = if project.len() > 8 {
            format!("{}…", &project[..7])
        } else {
            format!("{:<8}", project)
        };

        let status = match session.status.to_string().as_str() {
            "active" => format!("{}● active{}", PULSE_CYAN, RESET),
            "completed" => format!("{}✓ done{}", COSMIC_VIOLET, RESET),
            "crashed" => format!("{}✗ crash{}", "\x1b[38;5;196m", RESET),
            s => format!("○ {}", s),
        };

        let tokens = format_tokens(session.tokens_input + session.tokens_output);
        let cost = format!("${:.2}", session.estimated_cost);

        println!(
            "{}│{} {:<8} {}│{} {:<8} {}│{} {:<11} {}│{} {:<8} {}│{} {:>8} {}│{} {:>7} {}│{} {:>7} {}│{}",
            AURORA_BLUE, RESET, &session.id[..8],
            AURORA_BLUE, RESET, project_display,
            AURORA_BLUE, RESET, session.agent_type.to_string(),
            AURORA_BLUE, RESET, status,
            AURORA_BLUE, RESET, session.message_count,
            AURORA_BLUE, RESET, tokens,
            AURORA_BLUE, RESET, cost, AURORA_BLUE, RESET
        );
    }

    println!(
        "{}╰──────────┴──────────┴─────────────┴──────────┴──────────┴─────────┴─────────╯{}",
        AURORA_BLUE, RESET
    );
    println!("{}  ⋆    ✶     ★   ⋆  ✧  ★{}", DIM, RESET);

    Ok(())
}

async fn install_hooks() -> Result<()> {
    println!("{}  ✦   ⋆  ★    ✧  ✶{}", DIM, RESET);
    println!("  {}✦ Installing Claude Code Hooks...{}", AURORA_BLUE, RESET);

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let hooks_dir = format!("{}/.claude/hooks", home);

    // Create hooks directory
    std::fs::create_dir_all(&hooks_dir)?;

    // Get the path to our binary
    let exe_path = std::env::current_exe()?;
    let exe_str = exe_path.to_string_lossy();

    // Hook events to install
    let hook_events = [
        "SessionStart",
        "PreToolUse",
        "PostToolUse",
        "SubagentStop",
    ];

    for event in hook_events {
        let hook_file = format!("{}/{}.sh", hooks_dir, event);
        let hook_content = format!(
            r#"#!/bin/bash
# Agent Monitor hook for {}
"{}" hook {} < /dev/stdin
"#,
            event, exe_str, event
        );

        std::fs::write(&hook_file, hook_content)?;

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&hook_file)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook_file, perms)?;
        }

        println!("  {}✓{} Installed {}", PULSE_CYAN, RESET, event);
    }

    println!("  {}✦ Hooks installed successfully!{}", PULSE_CYAN, RESET);
    println!("{}  ⋆    ✶     ★   ⋆{}", DIM, RESET);

    Ok(())
}

async fn manage_config(show: bool, init: bool) -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let config_path = format!("{}/.config/agent-monitor/config.json", home);

    if init {
        let config = Config::default();
        let config_dir = format!("{}/.config/agent-monitor", home);
        std::fs::create_dir_all(&config_dir)?;
        config.save(&config_path)?;
        println!("{}✦ Configuration created at {}{}", PULSE_CYAN, config_path, RESET);
        return Ok(());
    }

    if show || !init {
        let config = if std::path::Path::new(&config_path).exists() {
            Config::load(&config_path)?
        } else {
            println!("{}✦ No config file found, showing defaults{}", COSMIC_VIOLET, RESET);
            Config::default()
        };

        println!(
            "{}╭────────────────────── ✦ Configuration ✦ ──────────────────────╮{}",
            AURORA_BLUE, RESET
        );
        println!(
            "{}│{}  data_dir:    {:?}",
            AURORA_BLUE, RESET, config.data_dir
        );
        println!(
            "{}│{}  socket_path: {:?}",
            AURORA_BLUE, RESET, config.socket_path
        );
        println!(
            "{}│{}  db_path:     {:?}",
            AURORA_BLUE, RESET, config.db_path
        );
        println!(
            "{}│{}  http_port:   {}",
            AURORA_BLUE, RESET, config.http_port
        );
        println!(
            "{}╰─────────────────────────────────────────────────────────────────╯{}",
            AURORA_BLUE, RESET
        );
    }

    Ok(())
}

async fn run_web(host: &str, port: u16) -> Result<()> {
    println!("{}  ✦   ⋆  ★    ✧  ✶{}", DIM, RESET);
    println!("  {}✦ Starting Web Dashboard{}", AURORA_BLUE, RESET);
    println!("  {}🌐 http://{}:{}{}", COSMIC_VIOLET, host, port, RESET);
    println!("{}  ⋆    ✶     ★   ⋆{}", DIM, RESET);
    println!();

    let config = Config::default();
    let storage = storage::Storage::new(&config.db_path).await?;

    api::run_web_server(host, port, storage).await?;

    Ok(())
}

/// Format duration in human-readable form
fn format_duration(seconds: f64) -> String {
    if seconds >= 3600.0 {
        format!("{:.1}h", seconds / 3600.0)
    } else if seconds >= 60.0 {
        format!("{:.1}m", seconds / 60.0)
    } else {
        format!("{:.0}s", seconds)
    }
}

/// Format token count in human-readable form
fn format_tokens(count: i64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        format!("{}", count)
    }
}

/// Run the interactive TUI watch mode
async fn run_watch() -> Result<()> {
    let config = Config::default();

    // Check if database exists
    if !config.db_path.exists() {
        eprintln!("{}✗ Error:{} Database not found at {:?}",
            "\x1b[38;5;196m", RESET, config.db_path);
        eprintln!("{}  Hint:{} Run 'agent-monitor daemon' first to initialize the database.",
            AURORA_BLUE, RESET);
        return Ok(());
    }

    let storage = storage::Storage::new(&config.db_path).await?;

    // Run the TUI
    tui::run_tui(storage).await?;

    Ok(())
}

/// Clear sessions from database
async fn run_clear(agent_type: Option<String>, all: bool) -> Result<()> {
    let config = Config::default();

    if !config.db_path.exists() {
        eprintln!("{}✗ Error:{} Database not found at {:?}",
            "\x1b[38;5;196m", RESET, config.db_path);
        return Ok(());
    }

    let storage = storage::Storage::new(&config.db_path).await?;

    if all {
        println!("{}⟳ Clearing all sessions and events...{}", PULSE_CYAN, RESET);
        storage.clear_all().await?;
        println!("{}✓ All sessions cleared{}", AURORA_BLUE, RESET);
    } else if let Some(agent) = agent_type {
        println!("{}⟳ Clearing {} sessions...{}", PULSE_CYAN, agent, RESET);
        let count = storage.delete_sessions_by_type(&agent).await?;
        println!("{}✓ Cleared {} {} sessions{}", AURORA_BLUE, count, agent, RESET);
    } else {
        eprintln!("{}✗ Error:{} Please specify --agent-type or --all", "\x1b[38;5;196m", RESET);
        eprintln!("  Examples:");
        eprintln!("    agent-monitor clear --agent-type cursor");
        eprintln!("    agent-monitor clear --all");
    }

    Ok(())
}
