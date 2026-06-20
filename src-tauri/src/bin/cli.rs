use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use sugt_lib::{clients, config, gateway::GatewayState, gateway_daemon, logging, model::ProviderConfig, trial};
use std::time::Duration;
use tokio::signal;

#[derive(Parser)]
#[command(name = "sugt-cli", version, about = "SUGT su gateway CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init {
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    Status,
    Serve {
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
    Test,
    Env {
        #[command(subcommand)]
        command: EnvCommand,
    },
}

#[derive(Subcommand)]
enum EnvCommand {
    Status,
    Install {
        /// 网关未运行时自动在后台启动 sugt-cli serve
        #[arg(long)]
        start_gateway: bool,
    },
    Repair {
        #[arg(long)]
        start_gateway: bool,
    },
    Uninstall,
    Print,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = config::ensure_paths()?;
    logging::init(&paths.log_file)?;

    match cli.command {
        Command::Init {
            base_url,
            api_key,
            model,
        } => {
            let (paths, mut app_config) = config::load_or_init_config()?;
            if let (Some(base_url), Some(api_key), Some(model)) = (base_url, api_key, model) {
                let provider = ProviderConfig::new(
                    "CLI 初始化模型",
                    "OpenAI Compatible",
                    base_url,
                    api_key,
                    model,
                );
                app_config.active_provider_id = Some(provider.id.clone());
                app_config.providers.push(provider);
                config::save_config(&paths, &app_config)?;
            }
            println!("config_dir={}", paths.config_dir.display());
            println!("config_file={}", paths.config_file.display());
        }
        Command::Status => {
            let (paths, app_config) = config::load_or_init_config()?;
            let trial_status = trial::status(&paths);
            println!("SUGT config: {}", paths.config_file.display());
            println!("listen: http://{}:{}", app_config.host, app_config.port);
            println!("failover: {}", app_config.failover_enabled);
            println!("trial: {}", trial_status.message);
            for provider in app_config.providers {
                let active = app_config.active_provider_id.as_deref() == Some(provider.id.as_str());
                println!(
                    "{} [{}] {} -> {} ({})",
                    if active { "*" } else { "-" },
                    provider.provider,
                    provider.model_name,
                    provider.base_url,
                    provider.masked_key()
                );
            }
        }
        Command::Serve { host, port } => {
            let (paths, mut app_config) = config::load_or_init_config()?;
            trial::ensure_allowed(&paths)?;
            if let Some(host) = host {
                app_config.host = host;
            }
            if let Some(port) = port {
                app_config.port = port;
            }
            let gateway = GatewayState::new(app_config)?;
            let url = gateway.start().await?;
            println!("SUGT gateway listening on {}", url);
            signal::ctrl_c().await?;
            gateway.stop().await?;
        }
        Command::Test => {
            let (paths, app_config) = config::load_or_init_config()?;
            trial::ensure_allowed(&paths)?;
            let gateway = GatewayState::new(app_config.clone())?;
            for provider in app_config.providers {
                let status = gateway.test_provider(&provider.id).await?;
                println!("{}: {:?}", provider.name, status);
            }
        }
        Command::Env { command } => {
            let (paths, app_config) = config::load_or_init_config()?;
            match command {
                EnvCommand::Status => print_env_status(&clients::status(&app_config)),
                EnvCommand::Install { start_gateway } => {
                    ensure_gateway_reachable(&paths, &app_config, start_gateway).await?;
                    clients::write_launch_scripts(&paths, &app_config)?;
                    let status = clients::install(&app_config)?;
                    print_env_status(&status);
                    println!("launch_scripts={}", paths.config_dir.display());
                }
                EnvCommand::Repair { start_gateway } => {
                    ensure_gateway_reachable(&paths, &app_config, start_gateway).await?;
                    clients::write_launch_scripts(&paths, &app_config)?;
                    let status = clients::repair(&app_config)?;
                    print_env_status(&status);
                    println!("launch_scripts={}", paths.config_dir.display());
                }
                EnvCommand::Uninstall => {
                    let status = clients::uninstall(&app_config)?;
                    print_env_status(&status);
                }
                EnvCommand::Print => print!("{}", clients::print_launch_script(&app_config)),
            }
        }
    }

    Ok(())
}

fn print_env_status(status: &clients::ClientsEnvStatus) {
    println!("listen_url={}", status.listen_url);
    for client in [&status.claude, &status.codex] {
        println!("{} configured={}", client.client, client.configured);
        for (name, value) in &client.variables {
            println!("  {}={}", name, value);
        }
        if !client.issues.is_empty() {
            println!("  issues={}", client.issues.join(" | "));
        }
        if !client.missing.is_empty() {
            println!("  missing={}", client.missing.join(","));
        }
    }
}

async fn ensure_gateway_reachable(
    paths: &config::AppPaths,
    app_config: &sugt_lib::model::AppConfig,
    auto_start: bool,
) -> Result<()> {
    let health_url = gateway_daemon::health_url(&app_config.client_host(), app_config.port);
    if gateway_daemon::is_health_url_ok(&health_url).await {
        return Ok(());
    }
    if !auto_start {
        bail!(
            "本地网关未运行：请先执行 sugt-cli serve，或加上 --start-gateway 自动启动"
        );
    }
    trial::ensure_allowed(paths)?;
    gateway_daemon::spawn_detached(&paths.config_dir)?;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if gateway_daemon::is_health_url_ok(&health_url).await {
            return Ok(());
        }
    }
    bail!("已尝试后台启动网关，但健康检查仍未通过，请手动执行 sugt-cli serve")
}
