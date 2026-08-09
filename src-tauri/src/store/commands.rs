use crate::commands::AppRuntime;
use crate::clients;
use crate::store::catalog::{build_catalog, SkillCatalogItem};
use crate::store::hub::{
    create_local_skill, list_local_skills, list_skill_agents, mount_skills_to_agents,
    unmount_skills_from_agents, LocalSkillView, SkillAgentView,
};
use crate::store::install::{
    install_skill, mount_skill, MountTarget, resolve_skill_path, uninstall_skill, unmount_skill,
};
use crate::store::paths::StorePaths;
use crate::store::plugins::{plugin_panel, PluginPanelView};
use crate::store::repos::{
    add_repo, ensure_preferred_repo_cached, load_repos, parse_git_url, refresh_all_repos,
    refresh_repo_by_id, remove_repo, test_repo_access, update_repo, ParsedGitRepo,
    PreferredWarmResult, SkillRepo,
};
use crate::store::settings::{load_settings, open_with_editor, save_settings, test_github_proxy, StoreSettings};
use serde::Serialize;
use tauri::State;

fn store_paths(runtime: &AppRuntime) -> StorePaths {
    StorePaths::from_app(&runtime.paths)
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillRepoView {
    pub id: String,
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub weight: i32,
    pub enabled: bool,
    pub clone_url: String,
    pub label: String,
    pub last_refresh_at: Option<String>,
    pub last_error: Option<String>,
    pub skill_count: usize,
}

impl SkillRepoView {
    fn from_repo(repo: &SkillRepo, store_paths: &StorePaths) -> Self {
        Self {
            id: repo.id.clone(),
            owner: repo.owner.clone(),
            repo: repo.repo.clone(),
            branch: repo.branch.clone(),
            weight: repo.weight,
            enabled: repo.enabled,
            clone_url: repo.clone_url(),
            label: repo.label(),
            last_refresh_at: repo.last_refresh_at.clone(),
            last_error: repo.last_error.clone(),
            skill_count: crate::store::install::repo_skill_count(store_paths, &repo.id),
        }
    }
}

fn sorted_repo_views(repos: &[SkillRepo], store_paths: &StorePaths) -> Vec<SkillRepoView> {
    let mut views: Vec<_> = repos
        .iter()
        .map(|repo| SkillRepoView::from_repo(repo, store_paths))
        .collect();
    views.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| a.label.cmp(&b.label))
    });
    views
}

#[tauri::command]
pub async fn store_list_repos(runtime: State<'_, AppRuntime>) -> Result<Vec<SkillRepoView>, String> {
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    let repos = load_repos(&store_paths).map_err(|e| e.to_string())?;
    Ok(sorted_repo_views(&repos, &store_paths))
}

#[tauri::command]
pub async fn store_parse_repo_url(
    _runtime: State<'_, AppRuntime>,
    url: String,
) -> Result<ParsedGitRepo, String> {
    parse_git_url(&url).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_test_repo(
    _runtime: State<'_, AppRuntime>,
    url: String,
    branch: String,
) -> Result<String, String> {
    let url = url.clone();
    let branch = branch.clone();
    tokio::task::spawn_blocking(move || test_repo_access(&url, &branch).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn store_add_repo(
    runtime: State<'_, AppRuntime>,
    url: String,
    branch: String,
    weight: Option<i32>,
    owner: Option<String>,
    repo: Option<String>,
) -> Result<Vec<SkillRepoView>, String> {
    let store_paths = store_paths(&runtime);
    let resolved_url = if !url.trim().is_empty() {
        url
    } else if let (Some(owner), Some(repo)) = (owner, repo) {
        format!("{}/{}", owner.trim(), repo.trim())
    } else {
        return Err("请填写完整 git 地址".to_string());
    };
    let repos = add_repo(
        &store_paths,
        &resolved_url,
        &branch,
        weight.unwrap_or(0),
    )
    .map_err(|e| e.to_string())?;
    Ok(sorted_repo_views(&repos, &store_paths))
}

#[tauri::command]
pub async fn store_update_repo(
    runtime: State<'_, AppRuntime>,
    repo_id: String,
    branch: Option<String>,
    weight: Option<i32>,
    enabled: Option<bool>,
) -> Result<Vec<SkillRepoView>, String> {
    let store_paths = store_paths(&runtime);
    let repos = update_repo(
        &store_paths,
        &repo_id,
        branch.as_deref(),
        weight,
        enabled,
    )
    .map_err(|e| e.to_string())?;
    Ok(sorted_repo_views(&repos, &store_paths))
}

#[tauri::command]
pub async fn store_remove_repo(
    runtime: State<'_, AppRuntime>,
    repo_id: String,
) -> Result<Vec<SkillRepoView>, String> {
    let store_paths = store_paths(&runtime);
    let repos = remove_repo(&store_paths, &repo_id).map_err(|e| e.to_string())?;
    Ok(sorted_repo_views(&repos, &store_paths))
}

#[tauri::command]
pub async fn store_refresh_repo(
    runtime: State<'_, AppRuntime>,
    repo_id: String,
) -> Result<Vec<SkillRepoView>, String> {
    let store_paths = store_paths(&runtime);
    let repo_id = repo_id.clone();
    tokio::task::spawn_blocking(move || {
        refresh_repo_by_id(&store_paths, &repo_id).map_err(|e| e.to_string())?;
        let repos = load_repos(&store_paths).map_err(|e| e.to_string())?;
        Ok(sorted_repo_views(&repos, &store_paths))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn store_refresh_all_repos(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<SkillRepoView>, String> {
    let store_paths = store_paths(&runtime);
    tokio::task::spawn_blocking(move || {
        let repos = refresh_all_repos(&store_paths).map_err(|e| e.to_string())?;
        Ok(sorted_repo_views(&repos, &store_paths))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn store_ensure_preferred_repo(
    runtime: State<'_, AppRuntime>,
) -> Result<PreferredWarmResult, String> {
    let store_paths = store_paths(&runtime);
    tokio::task::spawn_blocking(move || {
        ensure_preferred_repo_cached(&store_paths).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn store_list_catalog(
    runtime: State<'_, AppRuntime>,
    query: Option<String>,
) -> Result<Vec<SkillCatalogItem>, String> {
    let store_paths = store_paths(&runtime);
    let items = build_catalog(&store_paths).map_err(|e| e.to_string())?;
    if let Some(query) = query.filter(|q| !q.trim().is_empty()) {
        let q = query.to_lowercase();
        Ok(items
            .into_iter()
            .filter(|item| {
                item.name.to_lowercase().contains(&q)
                    || item
                        .description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&q))
                        .unwrap_or(false)
                    || item.repo_label.to_lowercase().contains(&q)
            })
            .collect())
    } else {
        Ok(items)
    }
}

#[tauri::command]
pub async fn store_install_skill(
    runtime: State<'_, AppRuntime>,
    skill_id: String,
) -> Result<SkillCatalogItem, String> {
    let store_paths = store_paths(&runtime);
    install_skill(&store_paths, &skill_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_uninstall_skill(
    runtime: State<'_, AppRuntime>,
    skill_id: String,
) -> Result<(), String> {
    let store_paths = store_paths(&runtime);
    uninstall_skill(&store_paths, &skill_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_mount_skill(
    runtime: State<'_, AppRuntime>,
    skill_id: String,
    target: Option<String>,
) -> Result<SkillCatalogItem, String> {
    let store_paths = store_paths(&runtime);
    let target = MountTarget::parse(target.as_deref());
    mount_skill(&store_paths, &skill_id, target).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_unmount_skill(
    runtime: State<'_, AppRuntime>,
    skill_id: String,
    target: Option<String>,
) -> Result<(), String> {
    let store_paths = store_paths(&runtime);
    let target = MountTarget::parse(target.as_deref());
    unmount_skill(&store_paths, &skill_id, target).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_get_plugin_panel(
    _runtime: State<'_, AppRuntime>,
    client: String,
) -> Result<PluginPanelView, String> {
    plugin_panel(&client).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreClientPathsView {
    pub staging_dir: String,
    pub claude_skills: String,
    pub codex_skills: String,
    pub claude_plugins: String,
}

#[tauri::command]
pub async fn store_get_client_paths(runtime: State<'_, AppRuntime>) -> Result<StoreClientPathsView, String> {
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    Ok(StoreClientPathsView {
        staging_dir: store_paths.skills_dir.display().to_string(),
        claude_skills: StorePaths::claude_skills_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        codex_skills: StorePaths::codex_skills_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        claude_plugins: StorePaths::claude_plugins_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    })
}

#[tauri::command]
pub async fn store_get_settings(runtime: State<'_, AppRuntime>) -> Result<StoreSettings, String> {
    let store_paths = store_paths(&runtime);
    load_settings(&store_paths).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_set_settings(
    runtime: State<'_, AppRuntime>,
    settings: StoreSettings,
) -> Result<StoreSettings, String> {
    let store_paths = store_paths(&runtime);
    save_settings(&store_paths, &settings).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
pub async fn store_test_github_proxy(
    _runtime: State<'_, AppRuntime>,
    prefix: String,
) -> Result<String, String> {
    test_github_proxy(&prefix).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_launch_client(
    runtime: State<'_, AppRuntime>,
    client: String,
    work_dir: Option<String>,
) -> Result<(), String> {
    let config = runtime.config.read().await.clone();
    clients::write_launch_scripts(&runtime.paths, &config).map_err(|e| e.to_string())?;

    let script_name = if client.trim().eq_ignore_ascii_case("codex") {
        "codex-sugt.cmd"
    } else {
        "claude-sugt.cmd"
    };
    let script = runtime.paths.config_dir.join(script_name);
    if !script.exists() {
        return Err(format!("启动脚本不存在：{}", script.display()));
    }

    let work_dir = work_dir
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| directories::UserDirs::new().map(|u| u.home_dir().to_path_buf()))
        .unwrap_or_else(|| runtime.paths.config_dir.clone());

    if !work_dir.is_dir() {
        return Err(format!("工作目录无效：{}", work_dir.display()));
    }

    #[cfg(windows)]
    {
        use std::process::Stdio;
        // CREATE_NO_WINDOW 仅作用于本进程；start 仍会打开用户可见的新终端
        crate::process_util::hidden_command("cmd")
            .args([
                "/c",
                "start",
                "",
                "/D",
                work_dir.as_os_str().to_string_lossy().as_ref(),
                script.as_os_str().to_string_lossy().as_ref(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        Err("当前平台暂不支持从新终端启动客户端".to_string())
    }
}

#[tauri::command]
pub async fn store_open_skill(
    runtime: State<'_, AppRuntime>,
    skill_id: String,
    staged: Option<bool>,
    client: Option<String>,
) -> Result<(), String> {
    let store_paths = store_paths(&runtime);
    let settings = load_settings(&store_paths).map_err(|e| e.to_string())?;
    let path = resolve_skill_path(
        &store_paths,
        &skill_id,
        staged.unwrap_or(true),
        client.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    open_with_editor(&settings.editor_command, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_open_staging_dir(runtime: State<'_, AppRuntime>) -> Result<(), String> {
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    let settings = load_settings(&store_paths).map_err(|e| e.to_string())?;
    open_with_editor(&settings.editor_command, &store_paths.skills_dir)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_list_local_skills(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<LocalSkillView>, String> {
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    list_local_skills(&store_paths, &runtime.paths.config_dir).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_list_skill_agents(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<SkillAgentView>, String> {
    Ok(list_skill_agents(&runtime.paths.config_dir))
}

#[tauri::command]
pub async fn store_mount_skills(
    runtime: State<'_, AppRuntime>,
    skill_ids: Vec<String>,
    agent_ids: Vec<String>,
    custom_path: Option<String>,
) -> Result<usize, String> {
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    mount_skills_to_agents(
        &store_paths,
        &runtime.paths.config_dir,
        &skill_ids,
        &agent_ids,
        custom_path.as_deref(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_unmount_skills(
    runtime: State<'_, AppRuntime>,
    skill_ids: Vec<String>,
    agent_ids: Vec<String>,
    custom_path: Option<String>,
) -> Result<usize, String> {
    let store_paths = store_paths(&runtime);
    unmount_skills_from_agents(
        &store_paths,
        &runtime.paths.config_dir,
        &skill_ids,
        &agent_ids,
        custom_path.as_deref(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_create_local_skill(
    runtime: State<'_, AppRuntime>,
    name: String,
) -> Result<LocalSkillView, String> {
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    create_local_skill(&store_paths, &name).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_agent_advanced(
    runtime: State<'_, AppRuntime>,
    profile_id: String,
) -> Result<crate::agent_advanced::AgentAdvancedView, String> {
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    crate::agent_advanced::get_agent_advanced(
        &store_paths,
        &runtime.paths.config_dir,
        &profile_id,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn import_agent_skill(
    runtime: State<'_, AppRuntime>,
    source_path: String,
) -> Result<LocalSkillView, String> {
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    crate::agent_advanced::import_skill_to_library(&store_paths, &source_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_agent_advanced_path(
    runtime: State<'_, AppRuntime>,
    profile_id: String,
    path: Option<String>,
    skill_dirs: Option<String>,
) -> Result<crate::agent_advanced::AgentAdvancedView, String> {
    crate::agent_advanced::save_agent_root_path(
        &runtime.paths.config_dir,
        &profile_id,
        path,
        skill_dirs,
    )
    .map_err(|e| e.to_string())?;
    let store_paths = store_paths(&runtime);
    crate::agent_advanced::get_agent_advanced(
        &store_paths,
        &runtime.paths.config_dir,
        &profile_id,
    )
    .map_err(|e| e.to_string())
}
