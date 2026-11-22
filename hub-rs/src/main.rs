use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use vtally_hub::appconfig::AppConfig;
use vtally_hub::hub::{Hub, HubOptions};
use vtally_hub::mixer::config::TEST;
use vtally_hub::{mixer, persistence, server, udp};

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum Env {
    Production,
    Development,
}

#[derive(Parser, Debug)]
#[command(version, about = "vTally Hub: connects video mixers and tally lights")]
struct Args {
    /// "development" additionally offers a simulated mixer and logs more
    #[arg(long, value_enum, default_value = "production")]
    env: Env,

    /// Start with a scripted test mixer and without reading or writing a configuration file
    #[arg(long)]
    with_test: bool,

    /// Port of the web interface
    #[arg(long, env = "PORT", default_value_t = 3000)]
    port: u16,

    /// UDP port the tallies report to
    #[arg(long, default_value_t = 7411)]
    tally_port: u16,

    /// Configuration file [default: ~/.wifi-tally.json]
    #[arg(long, env = "CONFIG_FILE")]
    config_file: Option<PathBuf>,

    /// Directory with the built web frontend [default: "frontend" next to the executable]
    #[arg(long, env = "VTALLY_FRONTEND_DIR")]
    frontend_dir: Option<PathBuf>,
}

fn default_frontend_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("frontend")))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("frontend"))
}

async fn listen(port: u16) -> Result<TcpListener> {
    // dual stack, so that browsers reach the hub through IPv4 and IPv6
    match TcpListener::bind(("::", port)).await {
        Ok(l) => Ok(l),
        Err(_) => TcpListener::bind(("0.0.0.0", port))
            .await
            .with_context(|| format!("could not listen on port {port}")),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.ok() };
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("could not install SIGTERM handler");
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    ctrl_c.await;
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let is_dev = args.env == Env::Development;

    let default_filter = if is_dev {
        "debug,hyper=info,engineioxide=info,socketioxide=info"
    } else {
        "info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)),
        )
        .init();

    let options = HubOptions {
        http_port: args.port,
        tally_port: args.tally_port,
        is_dev,
        is_test: args.with_test,
        ..HubOptions::default()
    };

    let config_path = args
        .config_file
        .clone()
        .unwrap_or_else(persistence::default_path);
    let mut config = AppConfig::default();
    if options.is_test {
        info!("Starting test environment");
        config.mixer = Some(TEST.to_string());
    } else {
        persistence::load(&config_path, &mut config)?;
    }

    let hub = Hub::new(options.clone(), config);
    if options.is_test {
        hub.set_tally_timeouts(Duration::from_millis(1000), Duration::from_millis(3000));
    }

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let persistence_task = (!options.is_test)
        .then(|| tokio::spawn(persistence::run(hub.clone(), config_path, shutdown_rx)));

    let udp_socket = udp::bind(options.tally_port).await.with_context(|| {
        format!(
            "could not listen for tallies on UDP port {}",
            options.tally_port
        )
    })?;
    tokio::spawn(udp::run(hub.clone(), udp_socket));
    tokio::spawn(mixer::driver::run(hub.clone()));

    let frontend_dir = args.frontend_dir.unwrap_or_else(default_frontend_dir);
    info!("Serving frontend from directory {}", frontend_dir.display());
    let app = server::build(hub.clone(), Some(frontend_dir));

    let listener = listen(options.http_port).await?;
    info!(
        "Web Server available on http://localhost:{}",
        options.http_port
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    info!("Shutting down");
    let _ = shutdown_tx.send(true);
    if let Some(task) = persistence_task {
        if tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .is_err()
        {
            warn!("timed out while saving the configuration");
        }
    }
    Ok(())
}
