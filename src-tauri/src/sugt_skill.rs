/// SUGT 自带技能的注入与管理。
///
/// 接管 Agent 时，把 SUGT 自带的 SKILL.md 复制到 Agent 的技能目录，让 Agent
/// 能直接调用 sugt-cli 搜索/安装/管理其他技能。
use crate::agent_adapters;
use crate::takeover_profiles::TakeoverProfile;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// 内置 SUGT skill 的来源（相对本 crate 根目录）
pub const SUGT_SKILL_NAME: &str = "sugt";
const SUGT_SKILL_MARKER_FILE: &str = "SKILL.md";
const SUGT_SKILL_MARKER_CONTENT: &str = "sugt-cli";

/// SUGT skill 的内置源（编译期嵌入）
pub const EMBEDDED_SKILL: &str = include_str!("../skills/sugt/SKILL.md");

/// 找出 Agent 接管后应注入 SUGT skill 的目标目录。
///
/// 目录只取「已存在」的 profile/adapter 候选；全部不存在时，才创建一个明确的
/// canonical 目录。避免为任意自定义路径凭空 create_dir_all 造出无效目录。
pub fn inject_targets_for_profile(profile: &TakeoverProfile) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut raws: Vec<String> = profile.skill_dirs.clone();
    if let Some(adapter) = agent_adapters::by_id(&profile.id) {
        raws.extend(adapter.skill_dirs.iter().map(|s| s.to_string()));
    }
    for raw in raws {
        if let Some(path) = agent_adapters::expand_home(&raw) {
            if path.is_dir() && !out.contains(&path) {
                out.push(path);
            }
        }
    }
    if out.is_empty() {
        if let Ok(dir) = agent_adapters::canonical_skill_dir(&profile.id) {
            out.push(dir);
        }
    }
    out
}

/// 把内置 SUGT skill 注入到所有目标目录。
///
/// 同一目标目录只注入一次（用 marker 文件判断）；删除 SUGT skill 不会影响其他技能。
pub fn inject_sugt_skill(targets: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut injected = Vec::new();
    let dest_filename = SUGT_SKILL_NAME;
    for target in targets {
        let dest_dir = target.join(dest_filename);
        let marker = dest_dir.join(SUGT_SKILL_MARKER_FILE);
        // 已注入过（marker 存在且包含 SUGT 特征）则跳过
        if marker.is_file() {
            if let Ok(existing) = std::fs::read_to_string(&marker) {
                if existing.contains(SUGT_SKILL_MARKER_CONTENT) {
                    continue;
                }
            }
        }
        std::fs::create_dir_all(&dest_dir)
            .with_context(|| format!("创建 skill 目录失败：{}", dest_dir.display()))?;
        std::fs::write(&marker, EMBEDDED_SKILL)
            .with_context(|| format!("写入 SKILL.md 失败：{}", marker.display()))?;
        injected.push(dest_dir);
    }
    Ok(injected)
}

/// 从所有目标目录移除 SUGT skill（仅删除由 SUGT 注入的目录，避免误删）。
pub fn remove_sugt_skill(targets: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for target in targets {
        let dest_dir = target.join(SUGT_SKILL_NAME);
        let marker = dest_dir.join(SUGT_SKILL_MARKER_FILE);
        if !marker.is_file() {
            continue;
        }
        let is_sugt_owned = std::fs::read_to_string(&marker)
            .map(|c| c.contains(SUGT_SKILL_MARKER_CONTENT))
            .unwrap_or(false);
        if !is_sugt_owned {
            continue;
        }
        std::fs::remove_dir_all(&dest_dir)
            .with_context(|| format!("移除 SUGT skill 失败：{}", dest_dir.display()))?;
        removed.push(dest_dir);
    }
    Ok(removed)
}

/// 检测 SUGT skill 在某个目标目录中是否已注入
pub fn is_sugt_skill_injected(target: &Path) -> bool {
    let marker = target.join(SUGT_SKILL_NAME).join(SUGT_SKILL_MARKER_FILE);
    if !marker.is_file() {
        return false;
    }
    std::fs::read_to_string(&marker)
        .map(|c| c.contains(SUGT_SKILL_MARKER_CONTENT))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_skill_has_marker() {
        assert!(EMBEDDED_SKILL.contains(SUGT_SKILL_MARKER_CONTENT));
        assert!(EMBEDDED_SKILL.contains("sugt-cli skill search"));
    }
}
