use crate::{config::AppPaths, model::AppConfig};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

const DUMMY_KEY: &str = "sugt-local-key";

fn client_api_key(config: &AppConfig) -> String {
    client_api_key_for_config(config)
}

pub fn client_api_key_for_config(config: &AppConfig) -> String {
    let key = config.gateway_client_api_key.trim();
    if key.is_empty() {
        DUMMY_KEY.to_string()
    } else {
        key.to_string()
    }
}

/// Claude Code 与 AUTH_TOKEN 冲突的变量（旧版 SUGT 会误设，需清理）
const CLAUDE_CONFLICTING_VARS: &[&str] = &["ANTHROPIC_API_KEY", "CLAUDE_CODE_API_KEY"];

#[derive(Debug, Clone, Serialize)]
pub struct ClientEnvStatus {
    pub client: String,
    pub configured: bool,
    pub variables: BTreeMap<String, String>,
    pub missing: Vec<String>,
    pub issues: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientsEnvStatus {
    pub listen_url: String,
    pub gateway_reachable: bool,
    pub claude: ClientEnvStatus,
    pub codex: ClientEnvStatus,
    pub has_issues: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TakeoverPreviewEntry {
    pub name: String,
    pub new_value: String,
    pub current_value: String,
    /// set | clear | keep
    pub action: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TakeoverPreview {
    pub listen_url: String,
    pub entries: Vec<TakeoverPreviewEntry>,
    pub issues: Vec<String>,
    pub recovery_hint: String,
}

pub fn preview_install(config_dir: &Path, config: &AppConfig) -> TakeoverPreview {
    let listen_url = listen_url(config);
    let status = status(config_dir, config);
    let mut issues = Vec::new();
    if !status.claude.issues.is_empty() {
        issues.extend(status.claude.issues.clone());
    }
    if !status.codex.issues.is_empty() {
        issues.extend(status.codex.issues.clone());
    }

    let mut entries = Vec::new();
    for (name, value) in claude_vars(&listen_url, &client_api_key(config))
        .into_iter()
        .chain(codex_vars(&listen_url, &client_api_key(config)))
    {
        let current = std::env::var(&name).unwrap_or_default();
        let action = if current.is_empty() {
            "set".to_string()
        } else if current == value {
            "keep".to_string()
        } else {
            "set".to_string()
        };
        let current_display = mask_env_for_preview(&name, &current);
        entries.push(TakeoverPreviewEntry {
            name,
            new_value: value,
            current_value: current_display,
            action,
        });
    }

    for name in CLAUDE_CONFLICTING_VARS {
        let current = std::env::var(name).unwrap_or_default();
        if !current.is_empty() {
            entries.push(TakeoverPreviewEntry {
                name: (*name).to_string(),
                new_value: String::new(),
                current_value: mask_env_for_preview(name, &current),
                action: "clear".to_string(),
            });
        }
    }

    TakeoverPreview {
        listen_url,
        entries,
        issues,
        recovery_hint: "如需恢复：在客户端页点击「关闭接管」后新开终端；或手动删除/改回原环境变量".to_string(),
    }
}

fn mask_env_for_preview(name: &str, value: &str) -> String {
    if value.is_empty() {
        return "（未设置）".to_string();
    }
    mask_env_value(name, value)
}

pub fn status(config_dir: &Path, config: &AppConfig) -> ClientsEnvStatus {
    let listen_url = listen_url(config);
    let claude_expected = claude_vars(&listen_url, &client_api_key(config));
    let claude_active = is_claude_takeover_active(config);

    let claude = client_status(
        "Claude Code",
        claude_expected,
        claude_tracked_vars(),
        detect_claude_issues(config),
        if claude_active {
            "仅使用 ANTHROPIC_AUTH_TOKEN 接管，请新开终端后生效"
        } else {
            "未接管：保留你原有的 Anthropic 环境变量"
        },
        claude_active,
    );
    let codex = codex_client_status(config_dir, config);

    ClientsEnvStatus {
        listen_url,
        gateway_reachable: false,
        has_issues: !claude.issues.is_empty() || !codex.issues.is_empty(),
        claude,
        codex,
    }
}

pub fn install(config_dir: &Path, config: &AppConfig) -> Result<ClientsEnvStatus> {
    // 默认只接管 Claude Code，不碰 OPENAI_* / Codex（避免影响 OpenCode 等）
    install_for(config_dir, config, "claude")
}

pub fn install_for(config_dir: &Path, config: &AppConfig, target: &str) -> Result<ClientsEnvStatus> {
    let listen_url = listen_url(config);
    let key = client_api_key(config);
    let target = target.trim().to_ascii_lowercase();
    match target.as_str() {
        "claude" => {
            clear_claude_conflicts()?;
            for (name, value) in claude_vars(&listen_url, &key) {
                set_user_env(&name, &value)?;
            }
        }
        "codex" => {
            for (name, value) in codex_vars(&listen_url, &key) {
                set_user_env(&name, &value)?;
            }
            let _ = crate::codex_config::apply_codex_config(config);
        }
        "all" => {
            // 兼容旧调用：也只装 Claude。Codex 需显式 target=codex
            clear_claude_conflicts()?;
            for (name, value) in claude_vars(&listen_url, &key) {
                set_user_env(&name, &value)?;
            }
        }
        _ => {
            clear_claude_conflicts()?;
            for (name, value) in claude_vars(&listen_url, &key) {
                set_user_env(&name, &value)?;
            }
        }
    }
    Ok(status(config_dir, config))
}

pub fn repair(config_dir: &Path, config: &AppConfig) -> Result<ClientsEnvStatus> {
    // 方案 B：以偏好列表为准重写应接管的环境
    let _ = crate::takeover_profiles::restore_active_profiles(config_dir, config);
    if is_claude_takeover_active(config) {
        let _ = clear_claude_conflicts();
    }
    // 没有任何 Agent 声明 OPENAI_* 时，清掉历史上误写/残留的网关指向
    if !openai_takeover_active(config_dir, config) {
        let _ = uninstall_for(config_dir, config, "codex");
    }
    Ok(status(config_dir, config))
}

/// 若未显式启用任何 OPENAI_* 接管，清除历史上误写入的 OPENAI_* / Codex 覆盖。
pub fn reconcile_non_claude_takeover(
    config_dir: &Path,
    config: &AppConfig,
) -> Result<ClientsEnvStatus> {
    if !openai_takeover_active(config_dir, config) {
        let _ = uninstall_for(config_dir, config, "codex");
    }
    Ok(status(config_dir, config))
}

pub fn uninstall(config_dir: &Path, config: &AppConfig) -> Result<ClientsEnvStatus> {
    uninstall_for(config_dir, config, "all")
}

pub fn uninstall_for(
    config_dir: &Path,
    config: &AppConfig,
    target: &str,
) -> Result<ClientsEnvStatus> {
    let listen_url = listen_url(config);
    let key = client_api_key(config);
    let target = target.trim().to_ascii_lowercase();

    let mut names: Vec<String> = match target.as_str() {
        "claude" => claude_vars(&listen_url, &key)
            .into_keys()
            .chain(CLAUDE_CONFLICTING_VARS.iter().map(|s| s.to_string()))
            .collect(),
        "codex" => codex_vars(&listen_url, &key).into_keys().collect(),
        _ => claude_vars(&listen_url, &key)
            .into_keys()
            .chain(codex_vars(&listen_url, &key).into_keys())
            .chain(CLAUDE_CONFLICTING_VARS.iter().map(|s| s.to_string()))
            .collect(),
    };
    names.sort_unstable();
    names.dedup();

    for name in names {
        let expected = claude_vars(&listen_url, &key)
            .get(&name)
            .cloned()
            .or_else(|| codex_vars(&listen_url, &key).get(&name).cloned())
            .unwrap_or_default();
        // 进程或用户注册表任一指向网关/接管值，都清掉（避免只清进程漏掉 HKCU）
        let process_val = std::env::var(&name).unwrap_or_default();
        let user_val = read_user_env(&name).unwrap_or_default();
        let should = should_clear_takeover_env(&process_val, &expected, &listen_url, &key)
            || should_clear_takeover_env(&user_val, &expected, &listen_url, &key);
        if should {
            let _ = unset_user_env(&name);
        }
    }
    if matches!(target.as_str(), "codex" | "all") {
        let _ = crate::codex_config::clear_sugt_codex_overrides(config);
    }
    Ok(status(config_dir, config))
}

fn should_clear_takeover_env(actual: &str, expected: &str, listen_url: &str, key: &str) -> bool {
    if actual.is_empty() {
        return false;
    }
    if actual == expected || actual == key || actual == DUMMY_KEY {
        return true;
    }
    let lower = actual.to_ascii_lowercase();
    lower.contains(&listen_url.to_ascii_lowercase())
        || lower.contains("127.0.0.1:")
        || lower.contains("localhost:")
}

pub fn print_launch_script(config: &AppConfig) -> String {
    let listen_url = listen_url(config);
    let key = client_api_key(config);
    #[cfg(windows)]
    {
        format!(
            "set ANTHROPIC_BASE_URL={}\r\nset ANTHROPIC_AUTH_TOKEN={}\r\nset OPENAI_BASE_URL={}/v1\r\nset OPENAI_API_BASE={}/v1\r\nset OPENAI_API_KEY={}\r\n",
            listen_url, key, listen_url, listen_url, key
        )
    }
    #[cfg(unix)]
    {
        format!(
            "export ANTHROPIC_BASE_URL={}\nexport ANTHROPIC_AUTH_TOKEN={}\nexport OPENAI_BASE_URL={}/v1\nexport OPENAI_API_BASE={}/v1\nexport OPENAI_API_KEY={}\n",
            listen_url, key, listen_url, listen_url, key
        )
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (listen_url, key);
        String::new()
    }
}

pub fn write_launch_scripts(paths: &AppPaths, config: &AppConfig) -> Result<()> {
    let listen_url = listen_url(config);
    let key = client_api_key(config);
    #[cfg(windows)]
    {
        let claude = format!(
            "@echo off\r\nset ANTHROPIC_BASE_URL={}\r\nset ANTHROPIC_AUTH_TOKEN={}\r\nset ANTHROPIC_API_KEY=\r\nset CLAUDE_CODE_API_KEY=\r\nclaude %*\r\n",
            listen_url, key
        );
        let codex = format!(
            "@echo off\r\nset OPENAI_BASE_URL={}/v1\r\nset OPENAI_API_BASE={}/v1\r\nset OPENAI_API_KEY={}\r\ncodex %*\r\n",
            listen_url, listen_url, key
        );
        std::fs::write(paths.config_dir.join("claude-sugt.cmd"), claude)?;
        std::fs::write(paths.config_dir.join("codex-sugt.cmd"), codex)?;
    }
    #[cfg(unix)]
    {
        // macOS/Linux 使用 .sh 启动脚本，source 后执行对应客户端
        let claude = format!(
            "#!/bin/sh\nexport ANTHROPIC_BASE_URL={}\nexport ANTHROPIC_AUTH_TOKEN={}\nexport ANTHROPIC_API_KEY=\nexport CLAUDE_CODE_API_KEY=\nexec claude \"$@\"\n",
            listen_url, key
        );
        let codex = format!(
            "#!/bin/sh\nexport OPENAI_BASE_URL={}/v1\nexport OPENAI_API_BASE={}/v1\nexport OPENAI_API_KEY={}\nexec codex \"$@\"\n",
            listen_url, listen_url, key
        );
        std::fs::write(paths.config_dir.join("claude-sugt.sh"), claude)?;
        std::fs::write(paths.config_dir.join("codex-sugt.sh"), codex)?;
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (listen_url, key);
    }
    if config.codex_takeover_enabled {
        let _ = crate::codex_config::apply_codex_config(config);
    }
    Ok(())
}

fn listen_url(config: &AppConfig) -> String {
    config.listen_url()
}

fn claude_vars(listen_url: &str, client_key: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("ANTHROPIC_BASE_URL".to_string(), listen_url.to_string()),
        ("ANTHROPIC_AUTH_TOKEN".to_string(), client_key.to_string()),
    ])
}

fn codex_vars(listen_url: &str, client_key: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("OPENAI_BASE_URL".to_string(), format!("{}/v1", listen_url)),
        ("OPENAI_API_BASE".to_string(), format!("{}/v1", listen_url)),
        ("OPENAI_API_KEY".to_string(), client_key.to_string()),
        ("CODEX_API_KEY".to_string(), client_key.to_string()),
    ])
}

fn claude_tracked_vars() -> Vec<String> {
    let mut vars = vec![
        "ANTHROPIC_BASE_URL".to_string(),
        "ANTHROPIC_AUTH_TOKEN".to_string(),
    ];
    vars.extend(
        CLAUDE_CONFLICTING_VARS
            .iter()
            .map(|name| (*name).to_string()),
    );
    vars
}

fn codex_tracked_vars() -> Vec<String> {
    vec![
        "OPENAI_BASE_URL".to_string(),
        "OPENAI_API_BASE".to_string(),
        "OPENAI_API_KEY".to_string(),
        "CODEX_API_KEY".to_string(),
    ]
}

fn codex_client_status(config_dir: &Path, config: &AppConfig) -> ClientEnvStatus {
    let listen_url = listen_url(config);
    let expected = codex_vars(&listen_url, &client_api_key(config));
    let tracked = codex_tracked_vars();
    let cfg_status = crate::codex_config::codex_config_status(config);
    let openai_active = openai_takeover_active(config_dir, config);

    let mut variables = BTreeMap::new();
    let mut missing = Vec::new();

    for name in tracked {
        let actual = effective_env_value(&name);
        variables.insert(name.clone(), mask_env_value(&name, &actual));
    }

    variables.insert(
        "CODEX_CONFIG".to_string(),
        cfg_status.openai_base_url.clone().unwrap_or_else(|| "未写入".to_string()),
    );
    if let Some(model) = &cfg_status.model {
        variables.insert("CODEX_MODEL".to_string(), model.clone());
    }

    for (name, expected_value) in &expected {
        let actual = effective_env_value(name);
        if actual != *expected_value {
            missing.push(name.clone());
        }
    }

    let env_ok = missing.is_empty();
    let config_ok = cfg_status.matches_gateway && cfg_status.auth_configured;
    let configured = openai_active
        && config.codex_takeover_enabled
        && ((env_ok || config_ok) && cfg_status.matches_gateway);

    let mut issues = Vec::new();
    if config.codex_takeover_enabled && !cfg_status.matches_gateway {
        issues.push("Codex config.toml 的 openai_base_url 未指向本地网关".to_string());
    }
    // 仅当没有任何 Agent 声明 OPENAI_*（含 OpenCode 等）时，才把指向网关的变量当残留
    if !openai_active {
        for name in ["OPENAI_BASE_URL", "OPENAI_API_BASE"] {
            let actual = effective_env_value(name);
            let expected_value = expected.get(name).cloned().unwrap_or_default();
            if !actual.is_empty()
                && ((!expected_value.is_empty() && actual == expected_value)
                    || urls_point_to_gateway(&actual, &listen_url))
            {
                issues.push(format!(
                    "检测到残留 {} 指向本机网关，可能影响 OpenCode；可点「修复」清理",
                    name
                ));
            }
        }
    }

    let note = if !openai_active {
        "未接管 OpenAI 兼容客户端：不会改写 OPENAI_*，OpenCode 等可直连原服务".to_string()
    } else if !config.codex_takeover_enabled {
        "已有 Agent 使用 OPENAI_* 指向网关（非 Codex）；这是预期行为".to_string()
    } else if config_ok {
        "Codex 以 ~/.codex/config.toml 为准；接管后请新开终端".to_string()
    } else if cfg_status.openai_base_url.is_some() {
        "config.toml 中 openai_base_url 与网关不一致，请点「修复」".to_string()
    } else {
        "需写入 ~/.codex/config.toml，请点「接管」".to_string()
    };

    ClientEnvStatus {
        client: "Codex".to_string(),
        configured,
        variables,
        missing: if config_ok || !config.codex_takeover_enabled {
            vec![]
        } else {
            missing
        },
        issues,
        note,
    }
}

/// 当前偏好中是否有 Agent 会写入 OPENAI_*（Codex / OpenCode / 自定义等）
fn openai_takeover_active(config_dir: &Path, config: &AppConfig) -> bool {
    if config.codex_takeover_enabled {
        return true;
    }
    for id in &config.active_takeover_ids {
        if let Some(profile) = crate::takeover_profiles::find_profile(config_dir, id) {
            if profile.env_vars.keys().any(|k| {
                let u = k.to_ascii_uppercase();
                matches!(
                    u.as_str(),
                    "OPENAI_BASE_URL" | "OPENAI_API_BASE" | "OPENAI_API_KEY" | "CODEX_API_KEY"
                )
            }) {
                return true;
            }
        }
    }
    false
}

/// 进程环境优先；若为空则回看用户环境（Windows HKCU），与新开终端实际继承一致
fn effective_env_value(name: &str) -> String {
    let process = std::env::var(name).unwrap_or_default();
    if !process.is_empty() {
        return process;
    }
    read_user_env(name).unwrap_or_default()
}

fn read_user_env(name: &str) -> Option<String> {
    #[cfg(windows)]
    {
        read_user_env_windows(name)
    }
    #[cfg(unix)]
    {
        read_user_env_unix(name)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = name;
        None
    }
}

/// macOS/Linux：从 `~/.zshrc` 解析 `export NAME=...` 行（接管写入的持久化变量）
#[cfg(unix)]
fn read_user_env_unix(name: &str) -> Option<String> {
    let rc = std::fs::read_to_string(shell_rc_path()).ok()?;
    for line in rc.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("export ") else {
            continue;
        };
        let Some((key, value)) = rest.split_once('=') else {
            continue;
        };
        if key.trim() == name {
            return Some(unquote(value.trim()));
        }
    }
    None
}

/// `~/.zshrc` 路径（macOS 默认 shell 配置）
#[cfg(unix)]
fn shell_rc_path() -> std::path::PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".zshrc"))
        .unwrap_or_else(|| std::path::PathBuf::from(".zshrc"))
}

/// 去掉 shell 引号包裹（支持单引号与双引号）
#[cfg(unix)]
fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let (head, tail) = (bytes[0], bytes[value.len() - 1]);
        if (head == b'\'' && tail == b'\'') || (head == b'"' && tail == b'"') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// 判断一行是否为 `export <name>=...`（用于删除接管写入的行）
#[cfg(unix)]
fn is_export_line(line: &str, name: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("export ") else {
        return false;
    };
    rest.split('=').next().map(|key| key.trim() == name).unwrap_or(false)
}

/// shell 单引号转义，值内单引号以 `'\''` 形式安全写出
#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(windows)]
fn read_user_env_windows(name: &str) -> Option<String> {
    use std::process::Stdio;
    let output = crate::process_util::hidden_command("reg")
        .args(["query", r"HKCU\Environment", "/v", name])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts[0].eq_ignore_ascii_case(name) {
            if parts[1] == "REG_SZ" || parts[1] == "REG_EXPAND_SZ" {
                return Some(parts[2..].join(" "));
            }
        }
    }
    None
}

fn is_claude_takeover_active(config: &AppConfig) -> bool {
    config.claude_takeover_enabled
        || config
            .active_takeover_ids
            .iter()
            .any(|id| id.eq_ignore_ascii_case("claude-code"))
}

fn detect_claude_issues(config: &AppConfig) -> Vec<String> {
    let mut issues = Vec::new();
    let auth_token = effective_env_value("ANTHROPIC_AUTH_TOKEN");
    let api_key = effective_env_value("ANTHROPIC_API_KEY");
    let claude_code_key = effective_env_value("CLAUDE_CODE_API_KEY");
    let base_url = effective_env_value("ANTHROPIC_BASE_URL");
    let listen = listen_url(config);
    let active = is_claude_takeover_active(config);

    if !active {
        // 未接管：有个人 API Key 是正常的；只提示指向本机网关的残留
        if !base_url.is_empty() && urls_point_to_gateway(&base_url, &listen) {
            issues.push(
                "检测到残留 ANTHROPIC_BASE_URL 指向本机网关，可点「修复」或对 Claude 点「接管/取消」清理"
                    .to_string(),
            );
        }
        return issues;
    }

    // 已接管：AUTH_TOKEN 与其它 Key 并存会让 Claude Code 报错
    if !auth_token.is_empty() && !api_key.is_empty() {
        issues.push(
            "同时设置了 ANTHROPIC_AUTH_TOKEN 与 ANTHROPIC_API_KEY，Claude Code 会报警".to_string(),
        );
    }
    if !auth_token.is_empty() && !claude_code_key.is_empty() {
        issues.push(
            "同时设置了 ANTHROPIC_AUTH_TOKEN 与 CLAUDE_CODE_API_KEY，建议仅保留 AUTH_TOKEN"
                .to_string(),
        );
    }
    issues
}

fn urls_point_to_gateway(actual: &str, listen: &str) -> bool {
    let a = actual.trim().trim_end_matches('/');
    let l = listen.trim().trim_end_matches('/');
    if a.is_empty() || l.is_empty() {
        return false;
    }
    a.eq_ignore_ascii_case(l) || a.to_ascii_lowercase().starts_with(&format!("{}/", l.to_ascii_lowercase()))
}

fn clear_claude_conflicts() -> Result<()> {
    for name in CLAUDE_CONFLICTING_VARS {
        let _ = unset_user_env(name);
    }
    Ok(())
}

fn client_status(
    client: &str,
    expected: BTreeMap<String, String>,
    tracked: Vec<String>,
    issues: Vec<String>,
    note: &str,
    takeover_active: bool,
) -> ClientEnvStatus {
    let mut variables = BTreeMap::new();
    let mut missing = Vec::new();

    for name in tracked {
        let actual = effective_env_value(&name);
        variables.insert(name.clone(), mask_env_value(&name, &actual));
    }

    for (name, expected_value) in &expected {
        let actual = effective_env_value(name);
        if actual != *expected_value {
            missing.push(name.clone());
        }
    }

    // 未接管时「缺变量」不是问题；已接管时以变量是否命中为准
    let configured = if takeover_active {
        missing.is_empty()
    } else {
        false
    };

    ClientEnvStatus {
        client: client.to_string(),
        configured,
        variables,
        missing: if takeover_active { missing } else { vec![] },
        issues,
        note: note.to_string(),
    }
}

fn mask_env_value(name: &str, value: &str) -> String {
    if value.is_empty() {
        return "未设置".to_string();
    }
    if name.contains("KEY") || name.contains("TOKEN") {
        return crate::model::mask_secret(value);
    }
    value.to_string()
}

#[cfg(windows)]
fn set_user_env(name: &str, value: &str) -> Result<()> {
    use std::process::Stdio;
    let status = crate::process_util::hidden_command("setx")
        .arg(name)
        .arg(value)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        std::env::set_var(name, value);
        Ok(())
    } else {
        Err(anyhow!("setx 写入 {} 失败", name))
    }
}

#[cfg(windows)]
fn unset_user_env(name: &str) -> Result<()> {
    use std::process::Stdio;
    let status = crate::process_util::hidden_command("reg")
        .args(["delete", r"HKCU\Environment", "/v", name, "/f"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    std::env::remove_var(name);
    if status.success() {
        Ok(())
    } else {
        // 变量不存在时 reg delete 也会失败，清理场景视为成功
        Ok(())
    }
}

#[cfg(unix)]
fn set_user_env(name: &str, value: &str) -> Result<()> {
    let path = shell_rc_path();
    let rc = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = rc
        .lines()
        .filter(|line| !is_export_line(line, name))
        .map(str::to_string)
        .collect();
    lines.push(format!("export {}={}", name, shell_quote(value)));
    std::fs::write(&path, lines.join("\n") + "\n")?;
    std::env::set_var(name, value);
    Ok(())
}

#[cfg(unix)]
fn unset_user_env(name: &str) -> Result<()> {
    let path = shell_rc_path();
    let rc = std::fs::read_to_string(&path).unwrap_or_default();
    if rc.is_empty() {
        return Ok(());
    }
    let mut lines: Vec<String> = rc
        .lines()
        .filter(|line| !is_export_line(line, name))
        .map(str::to_string)
        .collect();
    // 无改动时避免无谓写盘（防止创建不存在的文件）
    if lines.len() == rc.lines().count() {
        return Ok(());
    }
    std::fs::write(&path, lines.join("\n") + "\n")?;
    std::env::remove_var(name);
    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn set_user_env(_name: &str, _value: &str) -> Result<()> {
    Err(anyhow!(
        "当前平台暂不支持自动写入用户环境变量，请使用 sugt-cli env print 输出临时启动变量"
    ))
}

#[cfg(not(any(windows, unix)))]
fn unset_user_env(_name: &str) -> Result<()> {
    Err(anyhow!("当前平台暂不支持自动删除用户环境变量"))
}

#[cfg(windows)]
pub fn set_user_env_public(name: &str, value: &str) -> Result<()> {
    set_user_env(name, value)
}

#[cfg(windows)]
pub fn unset_user_env_public(name: &str) -> Result<()> {
    unset_user_env(name)
}

#[cfg(unix)]
pub fn set_user_env_public(name: &str, value: &str) -> Result<()> {
    set_user_env(name, value)
}

#[cfg(unix)]
pub fn unset_user_env_public(name: &str) -> Result<()> {
    unset_user_env(name)
}

#[cfg(not(any(windows, unix)))]
pub fn set_user_env_public(name: &str, value: &str) -> Result<()> {
    set_user_env(name, value)
}

#[cfg(not(any(windows, unix)))]
pub fn unset_user_env_public(name: &str) -> Result<()> {
    unset_user_env(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_vars_only_use_auth_token() {
        let vars = claude_vars("http://127.0.0.1:8787", "sugt-local-key");
        assert!(vars.contains_key("ANTHROPIC_AUTH_TOKEN"));
        assert!(!vars.contains_key("ANTHROPIC_API_KEY"));
        assert!(!vars.contains_key("CLAUDE_CODE_API_KEY"));
    }

    #[test]
    fn gateway_url_match_includes_v1_suffix() {
        let listen = "http://127.0.0.1:8787";
        assert!(urls_point_to_gateway("http://127.0.0.1:8787/v1", listen));
        assert!(urls_point_to_gateway("http://127.0.0.1:8787/", listen));
        assert!(!urls_point_to_gateway("https://api.openai.com/v1", listen));
    }
}
