/// SUGT 自带技能的注入与管理。
///
/// 接管 Agent 时，把 SUGT 自带的 SKILL.md 复制到 Agent 的技能目录，让 Agent
/// 能直接调用 sugt-cli 搜索/安装/管理其他技能。
use crate::takeover_profiles::{expand_home, TakeoverProfile};
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
/// 优先用 takeover profile 的 skill_dirs，其次用 `~/.agents/skills`、用户主目录下的 `.claude/skills`。
pub fn inject_targets_for_profile(profile: &TakeoverProfile) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for raw in &profile.skill_dirs {
        if let Some(p) = expand_home(raw) {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    // 兜底：把 SUGT skill 注入到 `~/.agents/skills`（跨 Agent 通用）
    if let Some(p) = expand_home("~/.agents/skills") {
        if !out.contains(&p) {
            out.push(p);
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
