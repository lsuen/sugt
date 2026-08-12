use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use sugt_lib::{
    clients, config, gateway::GatewayState, gateway_daemon, logging,
    model::ProviderConfig, takeover_profiles, trial,
};
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
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

#[derive(Subcommand)]
enum SkillCommand {
    /// 搜索 SUGT 技能仓中的技能
    Search { keyword: String },
    /// 列出本地已安装的技能
    List,
    /// 查看技能详情
    Info { skill_id: String },
    /// 安装技能
    Install { skill_id: String },
    /// 卸载技能
    Uninstall { skill_id: String },
    /// 挂载技能到 Agent
    Mount {
        skill_id: String,
        #[arg(long, default_value = "both")]
        target: String,
    },
    /// 基于项目上下文推荐技能
    Suggest,
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
    Profiles,
    Apply {
        profile: String,
        #[arg(long)]
        start_gateway: bool,
    },
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
            let gateway = GatewayState::new(app_config, Some(paths.config_file.clone()))?;
            let url = gateway.start().await?;
            println!("SUGT gateway listening on {}", url);
            signal::ctrl_c().await?;
            gateway.stop().await?;
        }
        Command::Test => {
            let (paths, app_config) = config::load_or_init_config()?;
            trial::ensure_allowed(&paths)?;
            let gateway = GatewayState::new(app_config.clone(), Some(paths.config_file.clone()))?;
            for provider in app_config.providers {
                let status = gateway.test_provider(&provider.id).await?;
                println!("{}: {:?}", provider.name, status);
            }
        }
        Command::Skill { command } => {
            use sugt_lib::store::catalog as store_catalog;
            use sugt_lib::store::install as store_install;
            use sugt_lib::store::paths::StorePaths;
            use sugt_lib::store::repos as store_repos;
            let (paths, _app_config) = config::load_or_init_config()?;
            let store_paths = StorePaths::from_app(&paths);
            match command {
                SkillCommand::Search { keyword } => {
                    // 确保至少有一个 repo
                    let _ = store_repos::ensure_preferred_repo_cached(&store_paths);
                    let repos = store_repos::load_repos(&store_paths)?;
                    let enabled: Vec<_> = repos.into_iter().filter(|r| r.enabled).collect();
                    if enabled.is_empty() {
                        println!("暂无可用技能仓库，请先在 SUGT 客户端添加");
                        return Ok(());
                    }
                    let catalog = store_catalog::build_catalog(&store_paths)?;
                    let kw = keyword.to_lowercase();
                    let hits: Vec<_> = catalog
                        .into_iter()
                        .filter(|item| {
                            item.name.to_lowercase().contains(&kw)
                                || item.id.to_lowercase().contains(&kw)
                                || item
                                    .description
                                    .as_deref()
                                    .map(|d| d.to_lowercase().contains(&kw))
                                    .unwrap_or(false)
                        })
                        .collect();
                    if hits.is_empty() {
                        println!("未找到匹配 \"{}\" 的技能", keyword);
                    } else {
                        for item in hits {
                            println!("{} | {} | {}", item.id, item.name, item.repo_label);
                            if let Some(desc) = &item.description {
                                println!("  {}", desc);
                            }
                        }
                    }
                }
                SkillCommand::List => {
                    let catalog = store_catalog::build_catalog(&store_paths)?;
                    let staged: Vec<_> = catalog.iter().filter(|i| i.staged).collect();
                    if staged.is_empty() {
                        println!("暂无已安装技能");
                    } else {
                        for item in staged {
                            let mounted = if item.mounted {
                                if item.mounted_claude && item.mounted_codex {
                                    "已挂载(claude+codex)"
                                } else if item.mounted_claude {
                                    "已挂载(claude)"
                                } else if item.mounted_codex {
                                    "已挂载(codex)"
                                } else {
                                    "已挂载"
                                }
                            } else {
                                "未挂载"
                            };
                            println!("{} | {} | {}", item.id, item.name, mounted);
                        }
                    }
                }
                SkillCommand::Info { skill_id } => {
                    let catalog = store_catalog::build_catalog(&store_paths)?;
                    match catalog.into_iter().find(|i| i.id == skill_id) {
                        Some(item) => {
                            println!("ID: {}", item.id);
                            println!("名称: {}", item.name);
                            println!("来源: {}", item.repo_label);
                            println!("路径: {}", item.relative_path);
                            if let Some(desc) = &item.description {
                                println!("描述: {}", desc);
                            }
                            let staged_dir = store_paths.staged_skill_dir(&item.id);
                            let skill_md = staged_dir.join("SKILL.md");
                            if skill_md.is_file() {
                                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                                    println!("\n--- SKILL.md 前 30 行 ---");
                                    for line in content.lines().take(30) {
                                        println!("{}", line);
                                    }
                                }
                            }
                        }
                        None => println!("未找到技能：{}", skill_id),
                    }
                }
                SkillCommand::Install { skill_id } => {
                    store_install::install_skill(&store_paths, &skill_id)?;
                    println!("已安装：{}", skill_id);
                }
                SkillCommand::Uninstall { skill_id } => {
                    store_install::uninstall_skill(&store_paths, &skill_id)?;
                    println!("已卸载：{}", skill_id);
                }
                SkillCommand::Mount { skill_id, target } => {
                    use sugt_lib::store::install::MountTarget;
                    let target = MountTarget::parse(Some(&target));
                    store_install::mount_skill(&store_paths, &skill_id, target)?;
                    println!("已挂载 {} 到 {:?}", skill_id, target);
                }
                SkillCommand::Suggest => {
                    // 简单基于项目目录的关键词匹配
                    let cwd = std::env::current_dir()?;
                    let catalog = store_catalog::build_catalog(&store_paths)?;
                    let project_hint = detect_project_keywords(&cwd);
                    if project_hint.is_empty() {
                        println!("当前目录无明显项目特征，未给出建议");
                        return Ok(());
                    }
                    println!("检测到项目特征: {}", project_hint.join(", "));
                    let mut scored: Vec<_> = catalog
                        .into_iter()
                        .map(|item| {
                            let mut score = 0;
                            let hay = format!(
                                "{} {} {}",
                                item.id.to_lowercase(),
                                item.name.to_lowercase(),
                                item.description
                                    .as_deref()
                                    .unwrap_or("")
                                    .to_lowercase()
                            );
                            for kw in &project_hint {
                                if hay.contains(kw) {
                                    score += 1;
                                }
                            }
                            (item, score)
                        })
                        .filter(|(item, score)| *score > 0 && !item.staged)
                        .collect();
                    scored.sort_by(|a, b| b.1.cmp(&a.1));
                    if scored.is_empty() {
                        println!("未找到匹配当前项目特征的技能");
                    } else {
                        println!("推荐技能：");
                        for (item, score) in scored.into_iter().take(5) {
                            println!("  [score={}] {} | {}", score, item.id, item.name);
                        }
                    }
                }
            }
        }
        Command::Env { command } => {
            let (paths, app_config) = config::load_or_init_config()?;
            match command {
                EnvCommand::Status => {
                    print_env_status(&clients::status(&paths.config_dir, &app_config))
                }
                EnvCommand::Install { start_gateway } => {
                    ensure_gateway_reachable(&paths, &app_config, start_gateway).await?;
                    clients::write_launch_scripts(&paths, &app_config)?;
                    let status = clients::install(&paths.config_dir, &app_config)?;
                    print_env_status(&status);
                    println!("launch_scripts={}", paths.config_dir.display());
                }
                EnvCommand::Repair { start_gateway } => {
                    ensure_gateway_reachable(&paths, &app_config, start_gateway).await?;
                    clients::write_launch_scripts(&paths, &app_config)?;
                    let status = clients::repair(&paths.config_dir, &app_config)?;
                    print_env_status(&status);
                    println!("launch_scripts={}", paths.config_dir.display());
                }
                EnvCommand::Uninstall => {
                    let status = clients::uninstall(&paths.config_dir, &app_config)?;
                    print_env_status(&status);
                }
                EnvCommand::Print => print!("{}", clients::print_launch_script(&app_config)),
                EnvCommand::Profiles => {
                    let views =
                        takeover_profiles::list_profile_views(&paths.config_dir, &app_config);
                    for view in views {
                        println!(
                            "{} [{}] {} configured={} settings={} skills={}",
                            view.id,
                            view.protocol,
                            view.name,
                            view.configured,
                            view.settings_detected,
                            view.skills_detected
                        );
                    }
                }
                EnvCommand::Apply {
                    profile,
                    start_gateway,
                } => {
                    ensure_gateway_reachable(&paths, &app_config, start_gateway).await?;
                    clients::write_launch_scripts(&paths, &app_config)?;
                    let view = takeover_profiles::apply_profile(
                        &paths.config_dir,
                        &app_config,
                        &profile,
                    )?;
                    println!(
                        "applied profile={} configured={}",
                        view.name, view.configured
                    );
                }
            }
        }
    }

    Ok(())
}

fn detect_project_keywords(dir: &std::path::Path) -> Vec<String> {
    let mut hints = Vec::new();
    let check_files: &[(&str, &[&str])] = &[
        ("package.json", &["javascript", "typescript", "node", "react", "vue"]),
        ("Cargo.toml", &["rust", "cargo"]),
        ("go.mod", &["go", "golang"]),
        ("pyproject.toml", &["python"]),
        ("requirements.txt", &["python"]),
        ("pom.xml", &["java", "maven"]),
        ("build.gradle", &["java", "gradle", "android"]),
        ("Gemfile", &["ruby"]),
        ("composer.json", &["php"]),
        ("*.csproj", &["csharp", "dotnet", ".net"]),
        ("tsconfig.json", &["typescript"]),
    ];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return hints;
    };
    let names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_lowercase()))
        .collect();
    for (file, kws) in check_files {
        let target = file.to_lowercase();
        let hit = if target.starts_with('*') {
            let suffix = target.trim_start_matches('*');
            names.iter().any(|n| n.ends_with(suffix))
        } else {
            names.iter().any(|n| n == &target)
        };
        if hit {
            for kw in *kws {
                if !hints.iter().any(|h: &String| h == kw) {
                    hints.push((*kw).to_string());
                }
            }
        }
    }
    hints
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
