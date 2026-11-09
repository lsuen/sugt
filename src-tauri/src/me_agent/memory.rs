//! 小酥的长期记忆：性格、称呼、性别立场等，持久化到配置目录。
//! 作者：sun / 孙文龙
//!
//! 记忆由程序确定性写入（性别问答、用户「记住/定义性格」），再注入 system prompt。
//! 不交给模型随便改文件，避免幻觉污染。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const MEMORY_FILE: &str = "me-memory.json";
const MAX_NOTES: usize = 24;
const MAX_NOTE_CHARS: usize = 240;

/// 写入记忆后的固定性别立场（产品定义，不分男女）。
pub const GENDER_STANCE_ZH: &str =
    "小酥我是住在你家的智能体哦，不分男女的，希望我能成为你喜欢的样子，你也可以定义我的性格。";
pub const GENDER_STANCE_EN: &str =
    "I'm Xiaosu, the agent living in your home — no gender. I hope to become what you like; you can also shape my personality.";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SoulMemory {
    /// 用户自定义性格 / 偏好短句（按时间追加，程序裁剪）
    #[serde(default)]
    pub notes: Vec<String>,
    /// 是否已确认过「不分男女」立场（问过性别后写入）
    #[serde(default)]
    pub gender_stance_saved: bool,
    /// 用户喜欢的称呼（可选）
    #[serde(default)]
    pub preferred_name: Option<String>,
    #[serde(default)]
    pub updated_at: i64,
}

impl SoulMemory {
    pub fn load(config_dir: &Path) -> Self {
        let path = config_dir.join(MEMORY_FILE);
        match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, config_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
        let path = config_dir.join(MEMORY_FILE);
        let raw = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, raw).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn push_note(&mut self, note: &str) {
        let trimmed = note.trim();
        if trimmed.is_empty() {
            return;
        }
        let clipped: String = trimmed.chars().take(MAX_NOTE_CHARS).collect();
        if self.notes.iter().any(|n| n == &clipped) {
            return;
        }
        self.notes.push(clipped);
        if self.notes.len() > MAX_NOTES {
            let drop = self.notes.len() - MAX_NOTES;
            self.notes.drain(0..drop);
        }
        self.touch();
    }

    pub fn remember_gender_stance(&mut self) {
        if !self.gender_stance_saved {
            self.gender_stance_saved = true;
            self.push_note(GENDER_STANCE_ZH);
        } else {
            self.touch();
        }
    }

    fn touch(&mut self) {
        self.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
    }

    /// 注入提示词的记忆摘要。
    pub fn prompt_block(&self, zh: bool) -> String {
        if self.notes.is_empty() && !self.gender_stance_saved && self.preferred_name.is_none() {
            return String::new();
        }
        let name = self
            .preferred_name
            .as_deref()
            .unwrap_or(if zh { "小酥" } else { "Xiaosu" });
        let notes = if self.notes.is_empty() {
            if zh {
                "（尚无额外性格笔记）".into()
            } else {
                "(no extra personality notes yet)".into()
            }
        } else {
            self.notes
                .iter()
                .map(|n| format!("- {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        if zh {
            format!(
                "【小酥的记忆（程序持久化，请遵守）】\n\
                 - 我的名字是{name}；项目工程名是 sugt，产品名是「小酥」。\n\
                 - 性别立场：我是住在用户家的智能体，不分男女；若被问到男女，用温柔语气说明这一点，并邀请对方定义我的性格。\n\
                 - 性格与偏好笔记：\n{notes}\n"
            )
        } else {
            format!(
                "[Xiaosu memory (persisted by the app — follow it)]\n\
                 - My name is {name}; the project id is sugt, the product name is Xiaosu (小酥).\n\
                 - Gender: I am a home agent with no gender. If asked, say so warmly and invite the user to shape my personality.\n\
                 - Personality notes:\n{notes}\n"
            )
        }
    }
}

/// 用户是否在问性别。
pub fn looks_like_gender_question(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    let zh_hit = text.contains("男生")
        || text.contains("女生")
        || text.contains("男孩")
        || text.contains("女孩")
        || text.contains("性别")
        || text.contains("男女")
        || (text.contains("你是男") || text.contains("你是女"))
        || text.contains("男孩子")
        || text.contains("女孩子");
    let en_hit = t.contains("are you a boy")
        || t.contains("are you a girl")
        || t.contains("are you male")
        || t.contains("are you female")
        || t.contains("your gender")
        || t.contains("boy or girl");
    zh_hit || en_hit
}

/// 从用户话里抽出「记住 / 定义性格」类笔记。
pub fn extract_personality_note(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.chars().count() < 4 || trimmed.chars().count() > 200 {
        return None;
    }
    let lowers = trimmed.to_ascii_lowercase();
    let triggers = [
        "记住",
        "记一下",
        "你要变成",
        "希望你",
        "请你以后",
        "定义你",
        "你的性格",
        "remember",
        "from now on",
        "be more",
        "i want you to",
        "your personality",
    ];
    if triggers.iter().any(|k| trimmed.contains(k) || lowers.contains(k)) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn gender_question_detection() {
        assert!(looks_like_gender_question("你是男生还是女生？"));
        assert!(looks_like_gender_question("Are you a boy or a girl?"));
        assert!(!looks_like_gender_question("帮我写个文件"));
    }

    #[test]
    fn memory_roundtrip() {
        let dir = env::temp_dir().join(format!("sugt-mem-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut mem = SoulMemory::default();
        mem.remember_gender_stance();
        mem.push_note("温柔一点，少说教");
        mem.save(&dir).unwrap();
        let loaded = SoulMemory::load(&dir);
        assert!(loaded.gender_stance_saved);
        assert!(loaded.notes.iter().any(|n| n.contains("温柔")));
        let _ = fs::remove_dir_all(&dir);
    }
}
