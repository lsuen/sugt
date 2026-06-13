use crate::commands::AppRuntime;
use crate::product;
use crate::store::catalog::{build_catalog, SkillCatalogItem};
use crate::store::install::{
    install_skill, mount_skill, MountTarget, resolve_skill_path, uninstall_skill, unmount_skill,
};
use crate::store::paths::StorePaths;
use crate::store::plugins::{plugin_panel, PluginPanelView};
use crate::store::repos::{
    add_repo, load_repos, refresh_all_repos, refresh_repo_by_id, remove_repo, SkillRepo,
};
use crate::store::settings::{load_settings, open_with_editor, save_settings, StoreSettings};
use serde::Serialize;
use tauri::State;

fn ensure_store_edition() -> Result<(), String> {
    if product::is_store_edition() {
        Ok(())
    } else {
        Err("商店功能仅在商店版可用".to_string())
    }
}

fn store_paths(runtime: &AppRuntime) -> StorePaths {
    StorePaths::from_app(&runtime.paths)
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillRepoView {
    pub id: String,
    pub owner: String,
    pub repo: String,
    pub branch: String,
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
            enabled: repo.enabled,
            clone_url: repo.clone_url(),
            label: repo.label(),
            last_refresh_at: repo.last_refresh_at.clone(),
            last_error: repo.last_error.clone(),
            skill_count: crate::store::install::repo_skill_count(store_paths, &repo.id),
        }
    }
}

#[tauri::command]
pub async fn store_list_repos(runtime: State<'_, AppRuntime>) -> Result<Vec<SkillRepoView>, String> {
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    let repos = load_repos(&store_paths).map_err(|e| e.to_string())?;
    Ok(repos
        .iter()
        .map(|repo| SkillRepoView::from_repo(repo, &store_paths))
        .collect())
}

#[tauri::command]
pub async fn store_add_repo(
    runtime: State<'_, AppRuntime>,
    owner: String,
    repo: String,
    branch: String,
) -> Result<Vec<SkillRepoView>, String> {
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    let repos = add_repo(&store_paths, &owner, &repo, &branch).map_err(|e| e.to_string())?;
    Ok(repos
        .iter()
        .map(|item| SkillRepoView::from_repo(item, &store_paths))
        .collect())
}

#[tauri::command]
pub async fn store_remove_repo(
    runtime: State<'_, AppRuntime>,
    repo_id: String,
) -> Result<Vec<SkillRepoView>, String> {
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    let repos = remove_repo(&store_paths, &repo_id).map_err(|e| e.to_string())?;
    Ok(repos
        .iter()
        .map(|item| SkillRepoView::from_repo(item, &store_paths))
        .collect())
}

#[tauri::command]
pub async fn store_refresh_repo(
    runtime: State<'_, AppRuntime>,
    repo_id: String,
) -> Result<Vec<SkillRepoView>, String> {
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    let repo_id = repo_id.clone();
    tokio::task::spawn_blocking(move || {
        refresh_repo_by_id(&store_paths, &repo_id).map_err(|e| e.to_string())?;
        let repos = load_repos(&store_paths).map_err(|e| e.to_string())?;
        Ok(repos
            .iter()
            .map(|item| SkillRepoView::from_repo(item, &store_paths))
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn store_refresh_all_repos(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<SkillRepoView>, String> {
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    tokio::task::spawn_blocking(move || {
        let repos = refresh_all_repos(&store_paths).map_err(|e| e.to_string())?;
        Ok(repos
            .iter()
            .map(|item| SkillRepoView::from_repo(item, &store_paths))
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn store_list_catalog(
    runtime: State<'_, AppRuntime>,
    query: Option<String>,
) -> Result<Vec<SkillCatalogItem>, String> {
    ensure_store_edition()?;
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
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    install_skill(&store_paths, &skill_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_uninstall_skill(
    runtime: State<'_, AppRuntime>,
    skill_id: String,
) -> Result<(), String> {
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    uninstall_skill(&store_paths, &skill_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_mount_skill(
    runtime: State<'_, AppRuntime>,
    skill_id: String,
    target: Option<String>,
) -> Result<SkillCatalogItem, String> {
    ensure_store_edition()?;
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
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    let target = MountTarget::parse(target.as_deref());
    unmount_skill(&store_paths, &skill_id, target).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_get_plugin_panel(
    _runtime: State<'_, AppRuntime>,
    client: String,
) -> Result<PluginPanelView, String> {
    ensure_store_edition()?;
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
    ensure_store_edition()?;
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
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    load_settings(&store_paths).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_set_settings(
    runtime: State<'_, AppRuntime>,
    settings: StoreSettings,
) -> Result<StoreSettings, String> {
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    save_settings(&store_paths, &settings).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
pub async fn store_open_skill(
    runtime: State<'_, AppRuntime>,
    skill_id: String,
    staged: Option<bool>,
    client: Option<String>,
) -> Result<(), String> {
    ensure_store_edition()?;
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
    ensure_store_edition()?;
    let store_paths = store_paths(&runtime);
    store_paths.ensure_dirs().map_err(|e| e.to_string())?;
    let settings = load_settings(&store_paths).map_err(|e| e.to_string())?;
    open_with_editor(&settings.editor_command, &store_paths.skills_dir)
        .map_err(|e| e.to_string())
}
