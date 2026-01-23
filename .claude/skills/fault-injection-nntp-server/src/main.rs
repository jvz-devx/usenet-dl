mod config;
mod faults;
mod protocol;
mod server;

use clap::Parser;
use config::Config;
use server::Server;
use std::fs;
use std::path::PathBuf;
use std::process;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "fault-nntp-server")]
#[command(about = "Fault injection NNTP server for testing")]
#[command(version)]
struct Args {
    /// Path to TOML configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Port to listen on (overrides config)
    #[arg(short, long)]
    port: Option<u16>,

    /// Run as background daemon
    #[arg(long)]
    daemon: bool,

    /// Stop running daemon
    #[arg(long)]
    stop: bool,

    /// PID file location
    #[arg(long, default_value = "/tmp/fault-nntp.pid")]
    pid_file: PathBuf,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Handle --stop
    if args.stop {
        if let Err(e) = stop_daemon(&args.pid_file) {
            error!("Failed to stop daemon: {}", e);
            process::exit(1);
        }
        info!("Daemon stopped");
        return;
    }

    // Load configuration
    let config = if let Some(ref path) = args.config {
        match Config::load(path) {
            Ok(c) => {
                info!(path = ?path, "Loaded configuration");
                c
            }
            Err(e) => {
                error!(error = %e, "Failed to load config");
                process::exit(1);
            }
        }
    } else {
        info!("Using default configuration (no faults enabled)");
        Config::default()
    };

    // Handle daemon mode
    if args.daemon {
        match daemonize(&args.pid_file) {
            Ok(_) => {
                info!(pid_file = ?args.pid_file, "Daemon started");
            }
            Err(e) => {
                error!("Failed to daemonize: {}", e);
                process::exit(1);
            }
        }
    }

    // Run server
    let server = Server::new(config);

    // Set up signal handler for graceful shutdown
    let shutdown = server.shutdown_handle();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    if let Err(e) = server.run(args.port).await {
        error!(error = %e, "Server error");
        process::exit(1);
    }

    // Clean up PID file
    if args.daemon {
        let _ = fs::remove_file(&args.pid_file);
    }
}

fn daemonize(pid_file: &PathBuf) -> Result<(), String> {
    // Check if already running
    if pid_file.exists() {
        if let Ok(pid_str) = fs::read_to_string(pid_file) {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                // Check if process is still running
                if is_process_running(pid) {
                    return Err(format!("Daemon already running with PID {}", pid));
                }
            }
        }
        // Stale PID file, remove it
        let _ = fs::remove_file(pid_file);
    }

    // For true daemonization we'd use fork(), but for simplicity
    // we just write our PID and continue running
    let pid = process::id();
    fs::write(pid_file, pid.to_string()).map_err(|e| format!("Failed to write PID file: {}", e))?;

    info!(pid = pid, "Running as daemon");
    Ok(())
}

fn stop_daemon(pid_file: &PathBuf) -> Result<(), String> {
    if !pid_file.exists() {
        return Err("PID file not found - daemon may not be running".to_string());
    }

    let pid_str =
        fs::read_to_string(pid_file).map_err(|e| format!("Failed to read PID file: {}", e))?;

    let pid: i32 = pid_str
        .trim()
        .parse()
        .map_err(|_| "Invalid PID in file".to_string())?;

    // Send SIGTERM
    #[cfg(unix)]
    {
        use std::process::Command;
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .map_err(|e| format!("Failed to send signal: {}", e))?;

        if !status.success() {
            return Err("Failed to stop daemon".to_string());
        }
    }

    #[cfg(not(unix))]
    {
        return Err("Daemon stop not supported on this platform".to_string());
    }

    // Remove PID file
    let _ = fs::remove_file(pid_file);

    Ok(())
}

fn is_process_running(pid: i32) -> bool {
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        false
    }
}
