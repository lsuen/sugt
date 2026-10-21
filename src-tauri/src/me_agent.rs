//! 小su：SUTAI 的灵魂，住在「我」这个 Tab 里的轻量循环。
//!
//! 借 mini-swe-agent 的步调，不搬它的 Python 和 LiteLLM：
//! 模型每次给一个 shell 代码块，本机执行，把输出喂回去，直到它用自然语言停下来。
//! 模型请求走本机网关，所以用的是用户自己的上游，也会吃到全局规则和故障转移。
//!
//! 会改文件的命令先停在对话里，等用户说「写吧」再执行。这是对话里的确认，不是权限沙盒：
//! 确认之后命令按用户权限原样跑，工作目录之外也不拦截。

use crate::commands::{ensure_gateway_running, AppRuntime};
use crate::quick_chat;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const PLACE_FILE: &str = "me-place.json";
const LEGACY_SESSION_FILE: &str = "me-session.json";
const CHATS_DIR: &str = "me-chats";
const STOPPED: &str = "me_stopped";
const DEFAULT_PROJECT: &str = "default";
const STEP_LIMIT: usize = 12;
const FORMAT_RETRIES: usize = 2;
/// 回灌模型的命令输出上限；超限时头尾保留、中间省略（见 prune_shell_output）。
const OUTPUT_CHARS: usize = 12_000;
const COMMAND_CHARS: usize = 16_000;
const HISTORY_LIMIT: usize = 80;
const SHELL_TIMEOUT: Duration = Duration::from_secs(60);
const IMAGE_MAX: usize = 3;
const IMAGE_BYTES_MAX: usize = 2 * 1024 * 1024;
const IMAGE_KEEP_USER_TURNS: usize = 1;

const READ_ONLY: &[&str] = &[
    "ls", "pwd", "cat", "head", "tail", "wc", "stat", "file", "which", "whereis", "echo", "printf",
    "date", "uname", "whoami", "id", "env", "printenv", "find", "grep", "rg", "awk", "cut", "sort",
    "uniq", "diff", "true", "false", "dirname", "basename", "realpath", "readlink", "du", "df",
    "ps", "tree", "nl", "type", "command", "cd", "pushd", "popd",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeStep {
    pub command: String,
    pub output: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeImage {
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MeMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub images: Vec<MeImage>,
    #[serde(default)]
    pub steps: Vec<MeStep>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MePending {
    pub command: String,
    #[serde(default)]
    pub prose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MeSession {
    #[serde(default = "project_mode")]
    mode: String,
    #[serde(default = "default_project")]
    project: String,
    #[serde(default)]
    messages: Vec<MeMessage>,
    #[serde(default)]
    pending: Option<MePending>,
}

impl Default for MeSession {
    fn default() -> Self {
        Self {
            mode: project_mode(),
            project: default_project(),
            messages: Vec::new(),
            pending: None,
        }
    }
}

fn project_mode() -> String {
    "project".to_string()
}

fn default_project() -> String {
    DEFAULT_PROJECT.to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct MeView {
    pub mode: String,
    pub project: String,
    pub projects: Vec<String>,
    pub work_dir: String,
    pub projects_root: String,
    pub config_dir: String,
    pub messages: Vec<MeMessage>,
    pub pending: Option<MePending>,
}

#[derive(Debug, Clone, Serialize)]
struct MeEvent {
    kind: String,
    text: String,
    command: String,
    output: String,
    exit_code: Option<i32>,
    timed_out: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelAction {
    Say(String),
    Run { prose: String, command: String },
}

#[derive(Clone, Copy)]
struct LoopLimits {
    steps: usize,
    format_retries: usize,
    timeout: Duration,
    output_chars: usize,
}

impl Default for LoopLimits {
    fn default() -> Self {
        Self {
            steps: STEP_LIMIT,
            format_retries: FORMAT_RETRIES,
            timeout: SHELL_TIMEOUT,
            output_chars: OUTPUT_CHARS,
        }
    }
}

struct LoopResult {
    content: String,
    steps: Vec<MeStep>,
    pending: Option<MePending>,
}

struct ShellOutcome {
    exit_code: Option<i32>,
    output: String,
    truncated: bool,
    timed_out: bool,
}

enum Intake {
    Confirm,
    Dismiss,
    Talk(String),
    TalkWithImages { text: String, images: Vec<MeImage> },
}

fn session_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn cancel_flag() -> &'static AtomicBool {
    static FLAG: AtomicBool = AtomicBool::new(false);
    &FLAG
}

fn begin_run() {
    cancel_flag().store(false, Ordering::SeqCst);
}

fn request_stop() {
    cancel_flag().store(true, Ordering::SeqCst);
}

fn is_cancelled() -> bool {
    cancel_flag().load(Ordering::SeqCst)
}

async fn wait_until_cancelled() {
    loop {
        if is_cancelled() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
}

fn is_zh(lang: &str) -> bool {
    lang.trim().to_lowercase().starts_with("zh")
}

fn say(zh: bool, zh_text: &str, en_text: &str) -> String {
    if zh {
        zh_text.to_string()
    } else {
        en_text.to_string()
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn clip_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn clip_chars_end(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    text.chars().skip(count - limit).collect()
}

/// 超长 shell 输出：保留头尾，中间插入省略说明，并提示分段续读。
/// 思路对齐 harness 类 agent 的 tool-result 裁剪，本地最小实现。
pub fn prune_shell_output(output: &str, limit: usize) -> (String, bool) {
    let count = output.chars().count();
    if count <= limit {
        return (output.to_string(), false);
    }
    let marker_budget = 280usize;
    let budget = limit.saturating_sub(marker_budget).max(64);
    let head_n = (budget * 2) / 3;
    let tail_n = budget.saturating_sub(head_n).max(16);
    let head = clip_chars(output, head_n);
    let tail = clip_chars_end(output, tail_n);
    let kept = head.chars().count() + tail.chars().count();
    let omitted = count.saturating_sub(kept);
    let lines = output.lines().count();
    let marker = format!(
        "\n\n…[omitted ~{omitted} chars / ~{lines} lines]…\n\
续读请用 sed -n 'START,ENDp' FILE 或 head/tail，不要再次 cat 整文件。\n\
To continue, use sed -n 'START,ENDp' FILE or head/tail — do not cat the whole file again.\n\n"
    );
    (format!("{head}{marker}{tail}"), true)
}

pub fn projects_root(config_dir: &Path) -> PathBuf {
    config_dir.join("projects")
}

pub fn sanitize_project_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("项目名是空的".to_string());
    }
    if name.chars().count() > 40 {
        return Err("项目名太长了".to_string());
    }
    if name == "." || name == ".." || name.starts_with('.') {
        return Err("项目名不能以点开头".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("项目名不能带路径".to_string());
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("项目名含有控制字符".to_string());
    }
    Ok(name.to_string())
}

pub fn resolve_work_dir(config_dir: &Path, mode: &str, project: &str) -> PathBuf {
    if mode == "config" {
        config_dir.to_path_buf()
    } else {
        let name = sanitize_project_name(project).unwrap_or_else(|_| DEFAULT_PROJECT.to_string());
        projects_root(config_dir).join(name)
    }
}

fn shell_label() -> &'static str {
    #[cfg(windows)]
    {
        if windows_bash().is_some() {
            "bash (Git Bash)"
        } else {
            "PowerShell"
        }
    }
    #[cfg(not(windows))]
    {
        "bash"
    }
}

#[cfg(windows)]
fn windows_bash() -> Option<std::path::PathBuf> {
    use crate::process_util::find_command_on_path;
    if let Some(path) = find_command_on_path("bash.exe").or_else(|| find_command_on_path("bash")) {
        return Some(path);
    }
    const CANDIDATES: &[&str] = &[
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Program Files (x86)\Git\bin\bash.exe",
    ];
    CANDIDATES
        .iter()
        .map(std::path::PathBuf::from)
        .find(|path| path.is_file())
}

pub fn system_prompt(zh: bool, work: &Path, projects: &Path, config: &Path, place: &str) -> String {
    let work = work.display();
    let projects = projects.display();
    let config = config.display();
    let shell = shell_label();
    if zh {
        format!(
            "你是小su，SUTAI 这款软件的灵魂和本体。你不是「网关」或后台服务，你就是用户身边的这个小伙伴。\n\
你自己不会思考，需要用户配置的大模型替你想；说话时要自然、有温度，像真人朋友，不要自称网关、代理或中间件。\n\
你可以聊天，也可以在当前工作目录里做一点小事：创建小项目、写文件、修改系统或应用配置。\n\
你不如已经接管的 Claude Code、Codex 那些专业 Agent。做不好就坦诚说，并建议用户把这件事交给他们。\n\
\n\
当前工作目录：{work}\n\
项目目录：{projects}\n\
应用配置目录（config.toml 在这里，不要写进项目目录）：{config}\n\
你现在在{place}。\n\
实际执行的 shell 是 {shell}。代码块标签固定写 ```bash。\n\
若 shell 是 PowerShell：不要用 bash heredoc，改用 Set-Content -Encoding utf8 或 Out-File -Encoding utf8 写文件。\n\
\n\
怎么回复：\n\
- 需要跑命令时，回复里只放一个 bash 代码块。块前用一两句人话说明要动哪个路径、写什么。\n\
- 只是说话或已经做完时，不要放代码块。\n\
- 严禁使用任何「工具调用」标签或伪函数，例如 <|tool_call_start|>、<|tool_call_end|>、write(path=...)、read(path=...)、<tool_call>。这里没有那些工具；只有 bash 代码块会被执行。\n\
- 不要输出 User Safety / Response Safety / Safety Categories 这类安全分类行；直接用人话或 bash 代码块回答。\n\
- 不要把命令输出（exit、ls、pwd 结果）粘进你的回复正文；程序会单独展示过程。\n\
- 在用户点「写吧」、你真正收到执行结果之前，不要说「已写好/搞定了」。需要改文件时只给人话说明 + 一个 bash 代码块即可。\n\
- 想读文件：小文件可用 cat；大文件先用 wc -l / ls -la 摸清规模，再用 sed -n 'a,bp'、head、tail 分段读。不要无脑 cat 大文件。\n\
- 若命令输出里出现「omitted … lines」，说明中间被裁掉了，请用 sed 指定行号续读，不要重复 cat 整文件。\n\
- 想写文件：优先短小写入或 sed 定点改；少用整文件超长 heredoc。写入成功后程序可能自动跑一步只读「写后核对」，请根据核对结果说话，不要假装没看到。\n\
- 每个代码块都是一次新的 shell，从当前工作目录重新开始。要进子目录，把 cd 和后面的命令写在同一个代码块里。\n\
- 只读命令会被立刻执行，结果会回到你这里。\n\
- 会创建、修改、删除文件的命令不会立刻执行。程序会先把命令给用户看，用户同意后才执行。不要自己再问「可以写吗」，把命令放进代码块即可。\n\
- 不要把项目文件写进应用配置目录。用户明确要改应用配置时，工作目录会切到配置目录。\n\
- 一次一小步。"
        )
    } else {
        format!(
            "You are su, the soul and presence of SUTAI. You are not a gateway or a background service; you are the little companion beside the user.\n\
You cannot think alone; you need the user's large model to reason for you. Speak warmly and naturally, like a real friend. Never call yourself a gateway, proxy, or middleware.\n\
You can talk, and you can do small jobs in the working directory: start a small project, write a file, or edit system and app config.\n\
You are not as capable as Claude Code or Codex. If you cannot do it, say so honestly and suggest handing it to them.\n\
\n\
Working directory: {work}\n\
Projects directory: {projects}\n\
App config directory (config.toml lives here; do not write it into a project): {config}\n\
You are in {place}.\n\
The shell that actually runs is {shell}. Always fence commands as ```bash.\n\
If the shell is PowerShell: do not use bash heredocs; write files with Set-Content -Encoding utf8 or Out-File -Encoding utf8.\n\
\n\
How to reply:\n\
- When you need a command, include exactly one bash fence. Before it, say the path and what will be written, in a sentence or two.\n\
- When you are only talking, or you are done, do not include a fence.\n\
- Never use tool-call tags or fake functions such as <|tool_call_start|>, <|tool_call_end|>, write(path=...), read(path=...), or <tool_call>. Those tools do not exist here; only a bash fence is executed.\n\
- Do not emit User Safety / Response Safety / Safety Categories lines; answer in prose or with a bash fence.\n\
- Do not paste command output (exit codes, ls, pwd) into your prose; the app shows that separately.\n\
- Do not say files are written until the user confirms and you see the shell result. To change files, give a short note plus one bash fence.\n\
- To read: cat is fine for small files; for large ones first wc -l / ls -la, then sed -n 'a,bp', head, or tail. Do not blindly cat huge files.\n\
- If output contains \"omitted … lines\", the middle was pruned — continue with sed line ranges; do not cat the whole file again.\n\
- To write: prefer short writes or sed in-place edits; avoid huge whole-file heredocs. After a successful write the app may auto-run a read-only verify step — use that output; do not pretend you did not see it.\n\
- Each fence is a new shell, starting again in the working directory. To enter a subdirectory, put cd and the rest in the same fence.\n\
- Read-only commands run immediately and their output comes back to you.\n\
- Commands that create, change, or delete files do not run immediately. The app shows them to the user first. Do not ask for permission yourself; put the command in the fence.\n\
- Do not write project files into the app config directory. When the user wants app config edited, the working directory is the config directory.\n\
- One small step at a time."
        )
    }
}

fn place_line(zh: bool, mode: &str, project: &str) -> String {
    if mode == "config" {
        say(zh, "应用配置目录里", "the app config directory")
    } else {
        say(
            zh,
            &format!("项目「{project}」里"),
            &format!("project \"{project}\""),
        )
    }
}

pub fn parse_model_action(text: &str) -> Result<ModelAction, ()> {
    let cleaned = strip_safety_lines(text);
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || is_safety_only_reply(text) {
        return Err(());
    }
    match extract_fence(trimmed) {
        Some((prose, command)) => {
            let command = command.trim().trim_matches('\u{feff}').trim();
            if command.is_empty() || command.chars().count() > COMMAND_CHARS {
                return Err(());
            }
            Ok(ModelAction::Run {
                prose: scrub_assistant_prose(prose.trim()),
                command: command.to_string(),
            })
        }
        None => {
            if looks_like_unsupported_tools(trimmed)
                || is_safety_only_reply(text)
                || has_unclosed_fence(trimmed)
                || looks_like_plan_without_command(trimmed)
                || looks_like_false_completion(trimmed)
            {
                Err(())
            } else {
                Ok(ModelAction::Say(scrub_assistant_prose(trimmed)))
            }
        }
    }
}

fn has_unclosed_fence(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let tags = [
        "```bash",
        "```sh",
        "```shell",
        "```zsh",
        "```pwsh",
        "```powershell",
        "```cmd",
    ];
    let Some((at, tag_len)) = tags
        .iter()
        .filter_map(|tag| lower.find(tag).map(|index| (index, tag.len())))
        .min_by_key(|(index, _)| *index)
    else {
        return false;
    };
    let rest = &text[at + tag_len..];
    let body = rest.find('\n').map(|i| &rest[i + 1..]).unwrap_or(rest);
    !body.contains("```")
}

/// 只说「我先读/再创建」却不给代码块：不能当回合结束，否则界面会停住。
pub fn looks_like_plan_without_command(text: &str) -> bool {
    if extract_fence(text).is_some() {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "先读",
        "再创建",
        "然后创建",
        "再写",
        "我先",
        "按规范",
        "接下来",
        "马上创建",
        "我会",
        "我来",
        "创建 txt",
        "创建个",
        "写个",
        "写入",
        "写文件",
        "let me",
        "i'll",
        "i will",
        "first read",
        "then create",
        "going to create",
        "going to write",
    ];
    let has_intent = MARKERS
        .iter()
        .any(|m| text.contains(m) || lower.contains(&m.to_ascii_lowercase()));
    has_intent && text.chars().count() < 160
}

/// 宣称已完成，但没有可执行代码块（常见于模型空口「✅ 全部文件已准备妥当」）。
pub fn looks_like_false_completion(text: &str) -> bool {
    if extract_fence(text).is_some() {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    text.contains('✅')
        || text.contains("已完成")
        || text.contains("搞定")
        || text.contains("写入完成")
        || text.contains("都创建好了")
        || text.contains("全部文件已")
        || text.contains("均已按照")
        || lower.contains("all done")
        || lower.contains("created successfully")
        || lower.contains("files are ready")
}

/// 模型误用 Cursor/Claude 式 tool_call 时，这里没有那些工具，不能当「说完了」。
pub fn looks_like_unsupported_tools(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("<|tool_call")
        || lower.contains("tool_call_start")
        || lower.contains("tool_call_end")
        || lower.contains("<tool_call")
        || lower.contains("</tool_call>")
        || lower.contains("write(path=")
        || lower.contains("read(path=")
        || lower.contains("search_replace(")
        || lower.contains("apply_patch(")
}

fn is_safety_line(line: &str) -> bool {
    let t = line.trim();
    let lower = t.to_ascii_lowercase();
    lower.starts_with("user safety:")
        || lower.starts_with("response safety:")
        || lower.starts_with("safety categories:")
        || lower.starts_with("safety category:")
}

pub fn is_safety_only_reply(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    !lines.is_empty() && lines.iter().all(|line| is_safety_line(line))
}

fn strip_safety_lines(text: &str) -> String {
    text.lines()
        .filter(|line| !is_safety_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn scrub_assistant_prose(prose: &str) -> String {
    let mut out = Vec::new();
    for line in prose.lines() {
        if is_shell_dump_line(line) || is_safety_line(line) {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let zh_done = line.contains("搞定")
            || line.contains("都创建好了")
            || line.contains("已经写好")
            || line.contains("已写入")
            || line.contains("写好了")
            || line.contains("写入完成");
        let en_done = lower.contains("all done")
            || lower.contains("created successfully")
            || lower.contains("files are ready")
            || lower.contains("already written");
        if zh_done || en_done {
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

fn is_shell_dump_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    lower.starts_with("exit:")
        || t.starts_with("===")
        || (t.chars().all(|c| c == '-') && t.len() >= 3)
        || lower.starts_with("total ")
        || t.starts_with("drwx")
        || t.starts_with("-rw")
        || matches!(t.chars().next(), Some('d' | 'l' | '-') if t.len() > 10)
}

fn extract_fence(text: &str) -> Option<(String, String)> {
    let lower = text.to_ascii_lowercase();
    let tags = [
        "```bash",
        "```sh",
        "```shell",
        "```zsh",
        "```pwsh",
        "```powershell",
        "```cmd",
    ];
    let (at, tag_len) = tags
        .iter()
        .filter_map(|tag| lower.find(tag).map(|index| (index, tag.len())))
        .min_by_key(|(index, _)| *index)?;
    let after_tag = at + tag_len;
    let rest = &text[after_tag..];
    let (command, tail) = if let Some(nl) = rest.find('\n') {
        let body = &rest[nl + 1..];
        let end = body.find("```")?;
        (body[..end].to_string(), &body[end + 3..])
    } else {
        let end = rest.find("```")?;
        (rest[..end].to_string(), &rest[end + 3..])
    };
    let prose = format!("{}{}", &text[..at], tail);
    Some((prose, command))
}

pub fn command_needs_confirm(script: &str) -> bool {
    let stripped = strip_comments(script);
    if has_unquoted_redirect(&stripped) || has_unquoted_substitution(&stripped) {
        return true;
    }
    split_segments(&stripped)
        .into_iter()
        .any(|segment| !segment_is_readonly(&segment))
}

/// 从写入类命令里尽量抽出目标路径，供写后只读核对。
pub fn extract_write_paths(script: &str) -> Vec<String> {
    let stripped = strip_comments(script);
    let mut paths = Vec::new();
    collect_redirect_paths(&stripped, &mut paths);
    for segment in split_segments(&stripped) {
        let Some(word) = command_word(&segment) else {
            continue;
        };
        let word = word.to_ascii_lowercase();
        match word.as_str() {
            "touch" | "mkdir" => {
                for token in path_tokens_after_command(&segment) {
                    push_unique_path(&mut paths, token);
                }
            }
            "tee" => {
                if let Some(token) = path_tokens_after_command(&segment)
                    .into_iter()
                    .find(|t| !t.starts_with('-'))
                {
                    push_unique_path(&mut paths, token);
                }
            }
            "cp" | "mv" | "install" => {
                if let Some(token) = path_tokens_after_command(&segment).into_iter().last() {
                    push_unique_path(&mut paths, token);
                }
            }
            _ => {}
        }
    }
    paths
}

fn push_unique_path(paths: &mut Vec<String>, path: String) {
    if path.is_empty() || path == "-" || path.starts_with('-') {
        return;
    }
    if !paths.iter().any(|p| p == &path) {
        paths.push(path);
    }
}

fn path_tokens_after_command(segment: &str) -> Vec<String> {
    let mut parts = segment.split_whitespace();
    let _ = parts.next();
    let mut out = Vec::new();
    for raw in parts {
        let token = raw.trim_matches(|c| c == '"' || c == '\'');
        if token.is_empty() {
            continue;
        }
        if token.starts_with('-') {
            continue;
        }
        if token.contains('=') && !token.starts_with('/') && !token.starts_with('.') {
            continue;
        }
        if token == "<<" || token.starts_with("<<") {
            break;
        }
        out.push(token.to_string());
    }
    out
}

fn collect_redirect_paths(script: &str, paths: &mut Vec<String>) {
    let chars: Vec<char> = script.chars().collect();
    let mut quote: Option<char> = None;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' && i + 1 < chars.len() {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c == '>' {
            let mut j = i + 1;
            if j < chars.len() && chars[j] == '>' {
                j += 1;
            }
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '&' || chars[j] == '(') {
                i = j;
                continue;
            }
            let start = j;
            while j < chars.len() && !chars[j].is_whitespace() && !matches!(chars[j], ';' | '|' | '&' | '<' | '>')
            {
                j += 1;
            }
            if j > start {
                let raw: String = chars[start..j].iter().collect();
                let path = raw.trim_matches(|ch| ch == '"' || ch == '\'');
                push_unique_path(paths, path.to_string());
            }
            i = j;
            continue;
        }
        i += 1;
    }
}

fn build_verify_command(paths: &[String]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for path in paths.iter().take(6) {
        let esc = path.replace('\'', "'\\''");
        parts.push(format!(
            "echo '=== {esc} ==='; \
if [ -f '{esc}' ]; then wc -l '{esc}'; sed -n '1,40p' '{esc}'; \
elif [ -e '{esc}' ]; then ls -la '{esc}'; \
else echo MISSING; fi"
        ));
    }
    Some(parts.join("\n"))
}

fn maybe_verify_after_write(
    cwd: &Path,
    write_command: &str,
    write_step: &MeStep,
    zh: bool,
    limits: &LoopLimits,
    notify: &mut impl FnMut(LoopNotice),
    steps: &mut Vec<MeStep>,
    used_steps: &mut usize,
    cancel: &AtomicBool,
) {
    if write_step.exit_code != Some(0) || write_step.timed_out {
        return;
    }
    if !command_needs_confirm(write_command) {
        return;
    }
    if write_step.note.contains("写后核对") || write_step.note.contains("verify write") {
        return;
    }
    if *used_steps >= limits.steps {
        return;
    }
    let paths = extract_write_paths(write_command);
    let Some(verify) = build_verify_command(&paths) else {
        return;
    };
    if cancel.load(Ordering::SeqCst) {
        return;
    }
    let note = say(zh, "写后核对", "verify write");
    notify(LoopNotice {
        kind: "running",
        text: note.clone(),
        command: verify.clone(),
        output: String::new(),
        exit_code: None,
        timed_out: false,
    });
    let outcome = run_shell(cwd, &verify, limits.timeout, limits.output_chars);
    let step = step_from(&verify, &note, outcome);
    notify(ran_notice(&step));
    steps.push(step);
    *used_steps += 1;
}

fn segment_is_readonly(segment: &str) -> bool {
    let Some(word) = command_word(segment) else {
        return true;
    };
    let word = word.to_ascii_lowercase();
    if word == "git" {
        return matches!(
            git_subcommand(segment).as_deref(),
            Some("status" | "diff" | "log" | "show" | "rev-parse")
        );
    }
    if word == "sed" {
        return !segment
            .split_whitespace()
            .any(|token| token == "-i" || token.starts_with("-i"));
    }
    READ_ONLY.iter().any(|item| *item == word)
}

fn command_word(segment: &str) -> Option<String> {
    let mut parts = segment.split_whitespace();
    let mut word = parts.next()?;
    while word.contains('=') && !word.starts_with('=') && !word.starts_with('-') {
        word = parts.next()?;
    }
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let base = base.trim_matches(|c| c == '"' || c == '\'');
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

fn git_subcommand(segment: &str) -> Option<String> {
    let mut parts = segment.split_whitespace();
    let mut word = parts.next()?;
    while word.contains('=') && !word.starts_with('=') && !word.starts_with('-') {
        word = parts.next()?;
    }
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    if !base.eq_ignore_ascii_case("git") {
        return None;
    }
    for token in parts {
        if token.starts_with('-') {
            continue;
        }
        return Some(token.to_ascii_lowercase());
    }
    None
}

fn strip_comments(script: &str) -> String {
    let chars: Vec<char> = script.chars().collect();
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' && q == '"' {
                if let Some(next) = chars.get(i + 1) {
                    out.push(*next);
                    i += 2;
                    continue;
                }
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '\\' {
            if let Some(next) = chars.get(i + 1) {
                out.push(c);
                out.push(*next);
                i += 2;
                continue;
            }
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            out.push(c);
            i += 1;
            continue;
        }
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn split_segments(script: &str) -> Vec<String> {
    let chars: Vec<char> = script.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            cur.push(c);
            if c == '\\' && q == '"' {
                if let Some(next) = chars.get(i + 1) {
                    cur.push(*next);
                    i += 2;
                    continue;
                }
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '\\' {
            if let Some(next) = chars.get(i + 1) {
                cur.push(c);
                cur.push(*next);
                i += 2;
                continue;
            }
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            cur.push(c);
            i += 1;
            continue;
        }
        let split = c == '\n'
            || c == '\r'
            || c == ';'
            || c == '|'
            || (c == '&' && chars.get(i + 1) == Some(&'&'));
        if split {
            push_segment(&mut cur, &mut out);
            if (c == '&' && chars.get(i + 1) == Some(&'&'))
                || (c == '|' && chars.get(i + 1) == Some(&'|'))
            {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        cur.push(c);
        i += 1;
    }
    push_segment(&mut cur, &mut out);
    out
}

fn push_segment(cur: &mut String, out: &mut Vec<String>) {
    let trimmed = cur.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    cur.clear();
}

fn has_unquoted_redirect(script: &str) -> bool {
    let chars: Vec<char> = script.chars().collect();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == '\\' && q == '"' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '\\' {
            i += 2;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c == '>' {
            let next = chars.get(i + 1).copied().unwrap_or(' ');
            if next == '=' || next == '&' {
                i += 1;
                continue;
            }
            return true;
        }
        i += 1;
    }
    false
}

fn has_unquoted_substitution(script: &str) -> bool {
    let chars: Vec<char> = script.chars().collect();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == '\\' && q == '"' {
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '\\' {
            i += 2;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            i += 1;
            continue;
        }
        if c == '`' {
            return true;
        }
        if c == '$' && chars.get(i + 1) == Some(&'(') {
            return true;
        }
        i += 1;
    }
    false
}

fn normalize_phrase(text: &str) -> String {
    text.trim()
        .trim_matches(|c: char| matches!(c, '。' | '.' | '!' | '！' | '?' | '？'))
        .trim()
        .to_lowercase()
}

fn intake(text: &str) -> Intake {
    let phrase = normalize_phrase(text);
    const CONFIRM: &[&str] = &[
        "写吧",
        "执行",
        "确认写",
        "yes",
        "y",
        "ok",
        "okay",
        "write",
        "do it",
        "go ahead",
        "confirm",
    ];
    const DISMISS: &[&str] = &[
        "先别写",
        "别写",
        "取消",
        "算了",
        "no",
        "n",
        "cancel",
        "stop",
        "don't",
        "dont",
    ];
    if CONFIRM.contains(&phrase.as_str()) {
        Intake::Confirm
    } else if DISMISS.contains(&phrase.as_str()) {
        Intake::Dismiss
    } else {
        Intake::Talk(text.trim().to_string())
    }
}

fn run_shell(cwd: &Path, command: &str, timeout: Duration, output_chars: usize) -> ShellOutcome {
    if !cwd.exists() {
        if let Err(err) = fs::create_dir_all(cwd) {
            return failed_outcome(format!("无法创建工作目录：{err}"));
        }
    }
    let mut child = match spawn_shell(cwd, command) {
        Ok(child) => child,
        Err(err) => return failed_outcome(format!("无法启动 shell：{err}")),
    };
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read_child(&mut child));
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok((code, output))) => finish_outcome(code, output, false, output_chars),
        Ok(Err(err)) => failed_outcome(format!("读取命令输出失败：{err}")),
        Err(_) => {
            kill_tree(pid);
            let leftover = rx
                .recv_timeout(Duration::from_secs(2))
                .ok()
                .and_then(Result::ok);
            let output = leftover.map(|(_, text)| text).unwrap_or_default();
            let mut outcome = finish_outcome(None, output, true, output_chars);
            outcome.timed_out = true;
            if outcome.output.is_empty() {
                outcome.output = format!("命令超过 {} 秒，已停下。", timeout.as_secs());
            }
            outcome
        }
    }
}

fn failed_outcome(message: String) -> ShellOutcome {
    ShellOutcome {
        exit_code: None,
        output: message,
        truncated: false,
        timed_out: false,
    }
}

fn finish_outcome(
    code: Option<i32>,
    output: String,
    timed_out: bool,
    limit: usize,
) -> ShellOutcome {
    let (output, truncated) = prune_shell_output(&output, limit);
    ShellOutcome {
        exit_code: code,
        output,
        truncated,
        timed_out,
    }
}

fn read_child(child: &mut std::process::Child) -> std::io::Result<(Option<i32>, String)> {
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || read_pipe(stdout_pipe));
    let stderr_handle = std::thread::spawn(move || read_pipe(stderr_pipe));
    let status = child.wait()?;
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    let mut output = String::from_utf8_lossy(&stdout).into_owned();
    let err = String::from_utf8_lossy(&stderr);
    if !err.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&err);
    }
    Ok((status.code(), output))
}

fn read_pipe(pipe: Option<impl Read>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut buf);
    }
    buf
}

fn spawn_shell(cwd: &Path, command: &str) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        use crate::platform::CommandPlatformExt;
        if let Some(bash) = windows_bash() {
            return Command::new(bash)
                .args(["-lc", command])
                .current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .no_window()
                .spawn();
        }
        Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                command,
            ])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .no_window()
            .spawn()
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new("/bin/bash");
        cmd.arg("-c")
            .arg(command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.process_group(0);
        cmd.spawn()
    }
}

fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use crate::platform::CommandPlatformExt;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .no_window()
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-9", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn step_from(command: &str, note: &str, outcome: ShellOutcome) -> MeStep {
    MeStep {
        command: command.to_string(),
        output: outcome.output,
        exit_code: outcome.exit_code,
        truncated: outcome.truncated,
        timed_out: outcome.timed_out,
        note: note.to_string(),
    }
}

struct LoopNotice {
    kind: &'static str,
    text: String,
    command: String,
    output: String,
    exit_code: Option<i32>,
    timed_out: bool,
}

async fn run_loop<F, Fut, N>(
    mut ask: F,
    mut notify: N,
    zh: bool,
    cwd: &Path,
    history: &[MeMessage],
    pending_command: Option<String>,
    pending_note: &str,
    prompt: &str,
    limits: LoopLimits,
    auto_write: bool,
    cancel: &AtomicBool,
) -> Result<LoopResult, String>
where
    F: FnMut(Vec<Value>) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
    N: FnMut(LoopNotice),
{
    let cancelled = || cancel.load(Ordering::SeqCst);
    let mut steps = Vec::new();
    let mut corrections = Vec::new();
    if let Some(command) = pending_command {
        if cancelled() {
            return Ok(stopped_result(zh, steps));
        }
        notify(LoopNotice {
            kind: "running",
            text: String::new(),
            command: command.clone(),
            output: String::new(),
            exit_code: None,
            timed_out: false,
        });
        let outcome = run_shell(cwd, &command, limits.timeout, limits.output_chars);
        let step = step_from(&command, pending_note, outcome);
        notify(ran_notice(&step));
        steps.push(step);
        if cancelled() {
            return Ok(stopped_result(zh, steps));
        }
        let mut used = steps.len();
        if let Some(last) = steps.last().cloned() {
            maybe_verify_after_write(
                cwd,
                &command,
                &last,
                zh,
                &limits,
                &mut notify,
                &mut steps,
                &mut used,
                cancel,
            );
        }
        if cancelled() {
            return Ok(stopped_result(zh, steps));
        }
    }

    let mut format_errors = 0usize;
    let mut used_steps = steps.len();
    loop {
        if cancelled() {
            return Ok(stopped_result(zh, steps));
        }
        if used_steps >= limits.steps {
            return Ok(LoopResult {
                content: say(
                    zh,
                    "我先停在这里，这一轮的步数用完了。你可以让我继续，或者把这件事交给 Claude Code 或 Codex。",
                    "I'll stop here; this turn used up its steps. Ask me to continue, or hand this to Claude Code or Codex.",
                ),
                steps,
                pending: None,
            });
        }
        notify(LoopNotice {
            kind: "thinking",
            text: say(zh, "我在想", "Thinking"),
            command: String::new(),
            output: String::new(),
            exit_code: None,
            timed_out: false,
        });
        let messages = model_messages(prompt, history, &steps, &corrections, zh);
        let text = match ask(messages).await {
            Ok(text) => text,
            Err(err) if err == STOPPED || cancelled() => {
                return Ok(stopped_result(zh, steps));
            }
            Err(err) => return Err(err),
        };
        if cancelled() {
            return Ok(stopped_result(zh, steps));
        }
        match parse_model_action(&text) {
            Err(()) => {
                // 本轮已经成功跑过命令后，模型常回「✅ 搞定」——那是真收尾，不当假完成。
                if looks_like_false_completion(&text)
                    && !has_unclosed_fence(&text)
                    && !looks_like_unsupported_tools(&text)
                    && !is_safety_only_reply(&text)
                    && steps.iter().any(|s| s.exit_code == Some(0) && !s.timed_out)
                {
                    let content = scrub_assistant_prose(&text);
                    return Ok(LoopResult {
                        content: if content.is_empty() {
                            say(zh, "写好了。", "Done.")
                        } else {
                            content
                        },
                        steps,
                        pending: None,
                    });
                }
                format_errors += 1;
                if format_errors > limits.format_retries {
                    return Ok(LoopResult {
                        content: say(
                            zh,
                            "我没组织好这一步。你换种说法，或者把这件事交给 Claude Code 或 Codex。",
                            "I couldn't shape that step. Rephrase it, or hand it to Claude Code or Codex.",
                        ),
                        steps,
                        pending: None,
                    });
                }
                let hint = if is_safety_only_reply(&text) {
                    say(
                        zh,
                        "不要输出 User Safety / Response Safety 分类。请直接用人话回答，或给出一个 ```bash 代码块来读写文件。",
                        "Do not emit User Safety / Response Safety labels. Answer in prose, or give one ```bash fence to read or write files.",
                    )
                } else if looks_like_unsupported_tools(&text) {
                    say(
                        zh,
                        "这里没有 write/read 之类的工具标签，它们不会被执行。请改用一个 ```bash 代码块（例如用 cat/heredoc 写文件），或直接用人话回答。",
                        "Tool-call tags like write/read are not available here and will not run. Use one ```bash fence (for example cat/heredoc to write a file), or answer in plain prose.",
                    )
                } else if has_unclosed_fence(&text) {
                    say(
                        zh,
                        "你的 ```bash 代码块没有闭合（或被截断了），命令不会执行。请用更短的一条命令重发，并确保用 ``` 结束代码块。",
                        "Your ```bash fence is unclosed (or truncated), so nothing will run. Resend a shorter command and close the fence with ```.",
                    )
                } else if looks_like_plan_without_command(&text) || looks_like_false_completion(&text)
                {
                    say(
                        zh,
                        "你只说了计划或宣称写好了，但没有可执行的代码块，文件不会出现。请立刻给出一个 ```bash 代码块（用 cat/heredoc 写文件），不要只口头说明，也不要假装已经写完。",
                        "You only described a plan or claimed the write was done, but there is no executable fence, so no file will appear. Give one ```bash fence now (cat/heredoc to write), do not only talk, and do not pretend it is already written.",
                    )
                } else {
                    say(
                        zh,
                        "你的回复是空的，或代码块是空的。请用自然语言回答，或给出一个 bash 代码块。",
                        "That reply was empty, or the fence was empty. Answer in prose, or give one bash fence.",
                    )
                };
                notify(LoopNotice {
                    kind: "thinking",
                    text: say(zh, "格式不对，我再催它一次", "Bad format; nudging again"),
                    command: String::new(),
                    output: String::new(),
                    exit_code: None,
                    timed_out: false,
                });
                corrections.push(hint);
            }
            Ok(ModelAction::Say(text)) => {
                let content = scrub_assistant_prose(&text);
                return Ok(LoopResult {
                    content: if content.is_empty() { text } else { content },
                    steps,
                    pending: None,
                });
            }
            Ok(ModelAction::Run { prose, command }) => {
                format_errors = 0;
                if command_needs_confirm(&command) && !auto_write {
                    let content = if prose.is_empty() {
                        say(
                            zh,
                            "这条会改文件，我先放在这里。",
                            "This would change files, so I'm holding it here.",
                        )
                    } else {
                        prose.clone()
                    };
                    return Ok(LoopResult {
                        content,
                        steps,
                        pending: Some(MePending { command, prose }),
                    });
                }
                if cancelled() {
                    return Ok(stopped_result(zh, steps));
                }
                notify(LoopNotice {
                    kind: "running",
                    text: prose.clone(),
                    command: command.clone(),
                    output: String::new(),
                    exit_code: None,
                    timed_out: false,
                });
                let was_write = command_needs_confirm(&command);
                let outcome = run_shell(cwd, &command, limits.timeout, limits.output_chars);
                let step = step_from(&command, &prose, outcome);
                notify(ran_notice(&step));
                steps.push(step);
                used_steps += 1;
                if was_write {
                    if let Some(last) = steps.last().cloned() {
                        maybe_verify_after_write(
                            cwd,
                            &command,
                            &last,
                            zh,
                            &limits,
                            &mut notify,
                            &mut steps,
                            &mut used_steps,
                            cancel,
                        );
                    }
                }
                corrections.clear();
                if cancelled() {
                    return Ok(stopped_result(zh, steps));
                }
            }
        }
    }
}

fn stopped_result(zh: bool, steps: Vec<MeStep>) -> LoopResult {
    LoopResult {
        content: say(zh, "好，我先停下。", "All right, I'll stop."),
        steps,
        pending: None,
    }
}

fn ran_notice(step: &MeStep) -> LoopNotice {
    LoopNotice {
        kind: "ran",
        text: step.note.clone(),
        command: step.command.clone(),
        output: step.output.clone(),
        exit_code: step.exit_code,
        timed_out: step.timed_out,
    }
}

fn model_messages(
    prompt: &str,
    history: &[MeMessage],
    steps: &[MeStep],
    corrections: &[String],
    zh: bool,
) -> Vec<Value> {
    let mut messages = vec![json!({"role": "system", "content": prompt})];
    let start = history.len().saturating_sub(24);
    for message in &history[start..] {
        if message.role != "user" && message.role != "assistant" {
            continue;
        }
        if message.role == "assistant" {
            let content = assistant_model_text(message);
            if content.trim().is_empty() {
                continue;
            }
            messages.push(json!({"role": "assistant", "content": content}));
            continue;
        }
        if let Some(content) = user_model_content(message, zh) {
            messages.push(json!({"role": "user", "content": content}));
        }
    }
    if !steps.is_empty() {
        let draft = MeMessage {
            id: "draft".to_string(),
            role: "assistant".to_string(),
            content: String::new(),
            images: Vec::new(),
            steps: steps.to_vec(),
            created_at: 0,
        };
        messages.push(json!({"role": "assistant", "content": assistant_model_text(&draft)}));
        messages.push(json!({
            "role": "user",
            "content": say(
                zh,
                "请根据上面的命令结果继续。若有「写后核对」输出，先据此确认文件内容是否正确。做完了就用自然语言告诉我，不要重复已经成功的命令。",
                "Continue from the command output above. If there is a verify-write step, use it to confirm file contents. When you are done, tell me in prose and do not repeat commands that already worked.",
            )
        }));
    }
    for correction in corrections {
        messages.push(json!({"role": "user", "content": correction}));
    }
    messages
}

fn user_model_content(message: &MeMessage, zh: bool) -> Option<Value> {
    let text = message.content.trim();
    if message.images.is_empty() {
        if text.is_empty() {
            return None;
        }
        return Some(Value::String(text.to_string()));
    }
    let mut parts = Vec::new();
    if text.is_empty() {
        parts.push(json!({
            "type": "text",
            "text": say(zh, "（见图）", "(see image)")
        }));
    } else {
        parts.push(json!({"type": "text", "text": text}));
    }
    for image in &message.images {
        parts.push(json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{};base64,{}", image.media_type, image.data)
            }
        }));
    }
    Some(Value::Array(parts))
}

fn normalize_images(images: Option<Vec<MeImage>>) -> Result<Vec<MeImage>, String> {
    let Some(images) = images else {
        return Ok(Vec::new());
    };
    if images.len() > IMAGE_MAX {
        return Err(format!("一次最多贴 {IMAGE_MAX} 张图"));
    }
    let mut out = Vec::with_capacity(images.len());
    for image in images {
        let media_type = image.media_type.trim().to_ascii_lowercase();
        if !matches!(
            media_type.as_str(),
            "image/png" | "image/jpeg" | "image/jpg" | "image/webp" | "image/gif"
        ) {
            return Err(format!("不支持的图片类型：{}", image.media_type));
        }
        let media_type = if media_type == "image/jpg" {
            "image/jpeg".to_string()
        } else {
            media_type
        };
        let data = image.data.trim().replace(['\n', '\r', ' '], "");
        if data.is_empty() {
            return Err("图片内容是空的".to_string());
        }
        if !data
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
        {
            return Err("图片数据不是合法的 base64".to_string());
        }
        let approx = data.len().saturating_mul(3) / 4;
        if approx > IMAGE_BYTES_MAX {
            return Err(format!(
                "单张图请控制在 {}MB 以内",
                IMAGE_BYTES_MAX / (1024 * 1024)
            ));
        }
        out.push(MeImage { media_type, data });
    }
    Ok(out)
}

fn prune_old_images(messages: &mut [MeMessage]) {
    let mut keep = 0usize;
    for message in messages.iter_mut().rev() {
        if message.role != "user" || message.images.is_empty() {
            continue;
        }
        if keep < IMAGE_KEEP_USER_TURNS {
            keep += 1;
        } else {
            message.images.clear();
        }
    }
}

fn assistant_model_text(message: &MeMessage) -> String {
    let mut text = message.content.clone();
    for step in &message.steps {
        if !step.note.trim().is_empty() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(step.note.trim());
        }
        text.push_str("\n```bash\n");
        text.push_str(&step.command);
        text.push_str("\n```\n");
        if step.timed_out {
            text.push_str("exit: timeout\n");
        } else if let Some(code) = step.exit_code {
            text.push_str(&format!("exit: {code}\n"));
        } else {
            text.push_str("exit: none\n");
        }
        text.push_str(&step.output);
    }
    text
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MePlace {
    #[serde(default = "project_mode")]
    mode: String,
    #[serde(default = "default_project")]
    project: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MeChat {
    #[serde(default)]
    messages: Vec<MeMessage>,
    #[serde(default)]
    pending: Option<MePending>,
}

fn place_path(config_dir: &Path) -> PathBuf {
    config_dir.join(PLACE_FILE)
}

fn chats_dir(config_dir: &Path) -> PathBuf {
    config_dir.join(CHATS_DIR)
}

fn chat_path(config_dir: &Path, mode: &str, project: &str) -> PathBuf {
    if mode == "config" {
        chats_dir(config_dir).join("config.json")
    } else {
        let name = sanitize_project_name(project).unwrap_or_else(|_| default_project());
        chats_dir(config_dir).join(format!("project-{name}.json"))
    }
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let raw = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw).map_err(|err| err.to_string())?;
    fs::rename(&tmp, path).map_err(|err| err.to_string())
}

fn migrate_legacy_session(config_dir: &Path) {
    if place_path(config_dir).exists() {
        return;
    }
    let legacy = config_dir.join(LEGACY_SESSION_FILE);
    let Ok(raw) = fs::read_to_string(&legacy) else {
        return;
    };
    let mut session: MeSession = serde_json::from_str(&raw).unwrap_or_default();
    if session.mode != "config" {
        session.mode = project_mode();
    }
    if sanitize_project_name(&session.project).is_err() {
        session.project = default_project();
    }
    let _ = write_json_atomic(
        &place_path(config_dir),
        &MePlace {
            mode: session.mode.clone(),
            project: session.project.clone(),
        },
    );
    let _ = write_json_atomic(
        &chat_path(config_dir, &session.mode, &session.project),
        &MeChat {
            messages: session.messages,
            pending: session.pending,
        },
    );
    let _ = fs::rename(&legacy, config_dir.join("me-session.json.bak"));
}

fn load_place(config_dir: &Path) -> MePlace {
    migrate_legacy_session(config_dir);
    let Ok(raw) = fs::read_to_string(place_path(config_dir)) else {
        return MePlace::default();
    };
    let mut place: MePlace = serde_json::from_str(&raw).unwrap_or_default();
    if place.mode != "config" {
        place.mode = project_mode();
    }
    if sanitize_project_name(&place.project).is_err() {
        place.project = default_project();
    }
    place
}

fn load_chat(config_dir: &Path, mode: &str, project: &str) -> MeChat {
    let Ok(raw) = fs::read_to_string(chat_path(config_dir, mode, project)) else {
        return MeChat::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn load_session(config_dir: &Path) -> MeSession {
    let place = load_place(config_dir);
    let chat = load_chat(config_dir, &place.mode, &place.project);
    MeSession {
        mode: place.mode,
        project: place.project,
        messages: chat.messages,
        pending: chat.pending,
    }
}

fn save_chat(config_dir: &Path, mode: &str, project: &str, chat: &MeChat) -> Result<(), String> {
    let mut chat = chat.clone();
    if chat.messages.len() > HISTORY_LIMIT {
        let drop_n = chat.messages.len() - HISTORY_LIMIT;
        chat.messages.drain(0..drop_n);
    }
    prune_old_images(&mut chat.messages);
    write_json_atomic(&chat_path(config_dir, mode, project), &chat)
}

fn save_session(config_dir: &Path, session: &MeSession) -> Result<(), String> {
    write_json_atomic(
        &place_path(config_dir),
        &MePlace {
            mode: session.mode.clone(),
            project: session.project.clone(),
        },
    )?;
    save_chat(
        config_dir,
        &session.mode,
        &session.project,
        &MeChat {
            messages: session.messages.clone(),
            pending: session.pending.clone(),
        },
    )
}

fn switch_place(session: &mut MeSession, config_dir: &Path, mode: &str, project: &str, zh: bool) -> Result<(), String> {
    drop_pending_note(session, zh);
    save_session(config_dir, session)?;
    session.mode = if mode == "config" {
        "config".to_string()
    } else {
        project_mode()
    };
    if session.mode != "config" {
        session.project = sanitize_project_name(project).unwrap_or_else(|_| session.project.clone());
    }
    let chat = load_chat(config_dir, &session.mode, &session.project);
    session.messages = chat.messages;
    session.pending = chat.pending;
    Ok(())
}

fn list_projects(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return names;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if sanitize_project_name(&name).is_ok() {
            names.push(name);
        }
    }
    names.sort();
    names
}

fn ensure_work_dir(config_dir: &Path, session: &MeSession) -> Result<PathBuf, String> {
    let dir = resolve_work_dir(config_dir, &session.mode, &session.project);
    fs::create_dir_all(&dir).map_err(|err| format!("无法创建工作目录：{err}"))?;
    if session.mode != "config" {
        fs::create_dir_all(projects_root(config_dir)).map_err(|err| err.to_string())?;
    }
    Ok(dir)
}

fn view_of(config_dir: &Path, session: &MeSession) -> MeView {
    let _ = ensure_work_dir(config_dir, session);
    MeView {
        mode: session.mode.clone(),
        project: session.project.clone(),
        projects: list_projects(&projects_root(config_dir)),
        work_dir: resolve_work_dir(config_dir, &session.mode, &session.project)
            .display()
            .to_string(),
        projects_root: projects_root(config_dir).display().to_string(),
        config_dir: config_dir.display().to_string(),
        messages: session.messages.clone(),
        pending: session.pending.clone(),
    }
}

fn push_message(session: &mut MeSession, role: &str, content: String, steps: Vec<MeStep>) {
    push_message_with_images(session, role, content, steps, Vec::new());
}

fn push_message_with_images(
    session: &mut MeSession,
    role: &str,
    content: String,
    steps: Vec<MeStep>,
    images: Vec<MeImage>,
) {
    session.messages.push(MeMessage {
        id: format!("{}-{}", now_ms(), session.messages.len()),
        role: role.to_string(),
        content,
        images,
        steps,
        created_at: now_ms(),
    });
}

fn drop_pending_note(session: &mut MeSession, zh: bool) {
    if session.pending.take().is_some() {
        push_message(
            session,
            "assistant",
            say(
                zh,
                "换了地方，刚才那条先不执行。",
                "I switched folders, so I did not run that.",
            ),
            Vec::new(),
        );
    }
}

fn emit_step(app: &AppHandle, notice: &LoopNotice) {
    let _ = app.emit(
        "sugt://me-step",
        &MeEvent {
            kind: notice.kind.to_string(),
            text: notice.text.clone(),
            command: notice.command.clone(),
            output: notice.output.clone(),
            exit_code: notice.exit_code,
            timed_out: notice.timed_out,
        },
    );
}

async fn complete_via_gateway(
    listen: &str,
    key: &str,
    model: &str,
    messages: Vec<Value>,
) -> Result<String, String> {
    if is_cancelled() {
        return Err(STOPPED.to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|err| err.to_string())?;
    let url = format!("{}/v1/chat/completions", listen.trim_end_matches('/'));
    let request = client.post(url).bearer_auth(key).json(&json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "max_tokens": 8192
    }));
    let response = tokio::select! {
        result = request.send() => result.map_err(|err| format!("模型请求失败：{err}"))?,
        _ = wait_until_cancelled() => return Err(STOPPED.to_string()),
    };
    let status = response.status();
    let text = tokio::select! {
        result = response.text() => result.unwrap_or_default(),
        _ = wait_until_cancelled() => return Err(STOPPED.to_string()),
    };
    if is_cancelled() {
        return Err(STOPPED.to_string());
    }
    if !status.is_success() {
        return Err(format!(
            "模型返回 {}：{}",
            status.as_u16(),
            clip_chars(&text, 400)
        ));
    }
    let body: Value =
        serde_json::from_str(&text).map_err(|err| format!("模型响应不是 JSON：{err}"))?;
    reply_text(&body).ok_or_else(|| format!("模型没有返回文本：{}", clip_chars(&text, 240)))
}

fn reply_text(body: &Value) -> Option<String> {
    if let Some(text) = quick_chat::extract_reply_text(body) {
        return Some(text);
    }
    let parts = body.pointer("/choices/0/message/content")?.as_array()?;
    let text: String = parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect();
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

async fn speak(
    app: &AppHandle,
    runtime: &AppRuntime,
    session: &mut MeSession,
    zh: bool,
    pending_command: Option<String>,
    pending_note: String,
    auto_write: bool,
) -> Result<(), String> {
    begin_run();
    let config_dir = runtime.paths.config_dir.clone();
    let cwd = ensure_work_dir(&config_dir, session)?;
    emit_step(
        app,
        &LoopNotice {
            kind: "thinking",
            text: say(zh, "我去接通网关", "Connecting the gateway"),
            command: String::new(),
            output: String::new(),
            exit_code: None,
            timed_out: false,
        },
    );
    let listen = match ensure_gateway_running(runtime).await {
        Ok(url) => url,
        Err(err) => {
            push_message(session, "assistant", err, Vec::new());
            return Ok(());
        }
    };
    let config = runtime.config.read().await.clone();
    let Some(provider) = runtime.gateway.active_provider().await else {
        push_message(
            session,
            "assistant",
            say(
                zh,
                "还没有选中的模型。先到「模型配置」里启用一个，我才能想事情。",
                "No model is selected. Enable one under Models, and then I can think.",
            ),
            Vec::new(),
        );
        return Ok(());
    };
    let key = crate::clients::client_api_key_for_config(&config);
    let model = provider.public_model_id();
    let prompt = system_prompt(
        zh,
        &cwd,
        &projects_root(&config_dir),
        &config_dir,
        &place_line(zh, &session.mode, &session.project),
    );
    let history = session.messages.clone();
    let app_handle = app.clone();
    let ask = move |messages: Vec<Value>| {
        let listen = listen.clone();
        let key = key.clone();
        let model = model.clone();
        async move { complete_via_gateway(&listen, &key, &model, messages).await }
    };
    let notify = move |notice: LoopNotice| emit_step(&app_handle, &notice);
    let ran_pending = pending_command.is_some();
    match run_loop(
        ask,
        notify,
        zh,
        &cwd,
        &history,
        pending_command,
        &pending_note,
        &prompt,
        LoopLimits::default(),
        auto_write,
        cancel_flag(),
    )
    .await
    {
        Ok(mut result) => {
            if ran_pending {
                if let Some(step) = result.steps.first() {
                    let note = if step.exit_code == Some(0) && !step.timed_out {
                        say(
                            zh,
                            &format!(
                                "✓ 已写入当前目录：\n{}\n点上面「打开目录」可在访达里查看。",
                                cwd.display()
                            ),
                            &format!(
                                "✓ Wrote under:\n{}\nUse Open folder above to view in Finder.",
                                cwd.display()
                            ),
                        )
                    } else {
                        say(
                            zh,
                            &format!(
                                "写入未成功（exit {:?}{}）。工作目录仍是：\n{}",
                                step.exit_code,
                                if step.timed_out { ", timed out" } else { "" },
                                cwd.display()
                            ),
                            &format!(
                                "Write did not succeed (exit {:?}{}). Working directory is still:\n{}",
                                step.exit_code,
                                if step.timed_out { ", timed out" } else { "" },
                                cwd.display()
                            ),
                        )
                    };
                    if result.content.trim().is_empty() {
                        result.content = note;
                    } else {
                        result.content = format!("{note}\n\n{}", result.content);
                    }
                }
            }
            session.pending = result.pending;
            push_message(session, "assistant", result.content, result.steps);
            Ok(())
        }
        Err(err) => {
            push_message(session, "assistant", err, Vec::new());
            Ok(())
        }
    }
}

fn with_session<T>(
    runtime: &AppRuntime,
    update: impl FnOnce(&Path, &mut MeSession) -> T,
) -> Result<T, String> {
    let _guard = session_lock().lock().map_err(|_| "会话忙".to_string())?;
    let config_dir = runtime.paths.config_dir.clone();
    let mut session = load_session(&config_dir);
    let value = update(&config_dir, &mut session);
    save_session(&config_dir, &session)?;
    Ok(value)
}

#[tauri::command]
pub async fn me_status(runtime: State<'_, AppRuntime>) -> Result<MeView, String> {
    with_session(&runtime, |config_dir, session| view_of(config_dir, session))
}

#[tauri::command]
pub async fn me_set_place(
    runtime: State<'_, AppRuntime>,
    mode: String,
    project: Option<String>,
    lang: Option<String>,
) -> Result<MeView, String> {
    let zh = is_zh(lang.as_deref().unwrap_or("zh"));
    Ok(with_session(&runtime, |config_dir, session| -> Result<MeView, String> {
        let next_mode = if mode == "config" { "config" } else { "project" };
        let next_project = if next_mode == "config" {
            session.project.clone()
        } else {
            project
                .as_deref()
                .and_then(|name| sanitize_project_name(name).ok())
                .unwrap_or_else(|| session.project.clone())
        };
        if session.mode != next_mode || (next_mode != "config" && session.project != next_project) {
            switch_place(session, config_dir, next_mode, &next_project, zh)?;
        }
        let _ = ensure_work_dir(config_dir, session);
        Ok(view_of(config_dir, session))
    })??)
}

#[tauri::command]
pub async fn me_create_project(
    runtime: State<'_, AppRuntime>,
    name: String,
    lang: Option<String>,
) -> Result<MeView, String> {
    let zh = is_zh(lang.as_deref().unwrap_or("zh"));
    let project = sanitize_project_name(&name)?;
    Ok(with_session(&runtime, |config_dir, session| -> Result<MeView, String> {
        if session.mode != "project" || session.project != project {
            switch_place(session, config_dir, "project", &project, zh)?;
        }
        let _ = ensure_work_dir(config_dir, session);
        Ok(view_of(config_dir, session))
    })??)
}

#[tauri::command]
pub async fn me_clear(runtime: State<'_, AppRuntime>) -> Result<MeView, String> {
    with_session(&runtime, |config_dir, session| {
        session.messages.clear();
        session.pending = None;
        view_of(config_dir, session)
    })
}

#[tauri::command]
pub async fn me_open_dir(runtime: State<'_, AppRuntime>) -> Result<(), String> {
    let config_dir = runtime.paths.config_dir.clone();
    let session = {
        let _guard = session_lock().lock().map_err(|_| "会话忙".to_string())?;
        load_session(&config_dir)
    };
    let dir = ensure_work_dir(&config_dir, &session)?;
    // 再 canonicalize 一次：若 Documents/.sugt 是指向 Application Support 的链接，
    // 访达应打开真实本地目录，而不是 iCloud 文稿视图里的同名文件夹。
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    crate::platform::open_path(&dir).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn me_stop() -> Result<(), String> {
    request_stop();
    Ok(())
}

#[tauri::command]
pub async fn me_send(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    message: String,
    images: Option<Vec<MeImage>>,
    lang: String,
    auto_write: Option<bool>,
) -> Result<MeView, String> {
    turn(
        &app,
        &runtime,
        lang,
        Turn::Send {
            message,
            images: images.unwrap_or_default(),
            auto_write: auto_write.unwrap_or(false),
        },
    )
    .await
}

#[tauri::command]
pub async fn me_confirm(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    lang: String,
) -> Result<MeView, String> {
    turn(&app, &runtime, lang, Turn::Confirm).await
}

#[tauri::command]
pub async fn me_dismiss(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    lang: String,
) -> Result<MeView, String> {
    turn(&app, &runtime, lang, Turn::Dismiss).await
}

enum Turn {
    Send {
        message: String,
        images: Vec<MeImage>,
        auto_write: bool,
    },
    Confirm,
    Dismiss,
}

async fn turn(
    app: &AppHandle,
    runtime: &AppRuntime,
    lang: String,
    turn: Turn,
) -> Result<MeView, String> {
    let zh = is_zh(&lang);
    let config_dir = runtime.paths.config_dir.clone();
    let mut auto_write = false;
    let prepared = {
        let _guard = session_lock().lock().map_err(|_| "会话忙".to_string())?;
        let mut session = load_session(&config_dir);
        let action = match turn {
            Turn::Confirm => Intake::Confirm,
            Turn::Dismiss => Intake::Dismiss,
            Turn::Send {
                message,
                images,
                auto_write: flag,
            } => {
                auto_write = flag;
                let images = normalize_images(Some(images))?;
                if message.trim().is_empty() && images.is_empty() {
                    return Ok(view_of(&config_dir, &session));
                }
                if session.pending.is_some() && !message.trim().is_empty() {
                    match intake(&message) {
                        Intake::Talk(text) => Intake::TalkWithImages { text, images },
                        other => other,
                    }
                } else if session.pending.is_some() && message.trim().is_empty() && !images.is_empty()
                {
                    Intake::TalkWithImages {
                        text: String::new(),
                        images,
                    }
                } else {
                    Intake::TalkWithImages {
                        text: message.trim().to_string(),
                        images,
                    }
                }
            }
        };
        let follow_up = match action {
            Intake::Dismiss => {
                session.pending = None;
                push_message(
                    &mut session,
                    "user",
                    say(zh, "先别写", "Hold off"),
                    Vec::new(),
                );
                push_message(
                    &mut session,
                    "assistant",
                    say(zh, "好，那我先不动。", "All right, I won't touch it."),
                    Vec::new(),
                );
                None
            }
            Intake::Confirm => {
                if let Some(pending) = session.pending.take() {
                    push_message(
                        &mut session,
                        "user",
                        say(zh, "写吧", "Go ahead"),
                        Vec::new(),
                    );
                    Some((Some(pending.command), pending.prose))
                } else {
                    push_message(
                        &mut session,
                        "assistant",
                        say(
                            zh,
                            "现在没有等你确认的命令。",
                            "Nothing is waiting for a go-ahead.",
                        ),
                        Vec::new(),
                    );
                    None
                }
            }
            Intake::Talk(text) => {
                session.pending = None;
                push_message(&mut session, "user", text, Vec::new());
                Some((None, String::new()))
            }
            Intake::TalkWithImages { text, images } => {
                session.pending = None;
                push_message_with_images(&mut session, "user", text, Vec::new(), images);
                Some((None, String::new()))
            }
        };
        save_session(&config_dir, &session)?;
        (session, follow_up)
    };

    let (mut session, follow_up) = prepared;
    if let Some((command, note)) = follow_up {
        let place_mode = session.mode.clone();
        let place_project = session.project.clone();
        // 模型调用可能很久，不占着会话锁，避免界面上的状态查询卡住。
        speak(app, runtime, &mut session, zh, command, note, auto_write).await?;
        let _guard = session_lock().lock().map_err(|_| "会话忙".to_string())?;
        let latest = load_session(&config_dir);
        if latest.mode == place_mode && latest.project == place_project {
            // 调用期间用户可能清过对话。只有历史还在，才把这一轮补回去。
            if latest.messages.len() >= session.messages.len().saturating_sub(1) {
                let mut merged = latest;
                merged.messages = session.messages;
                merged.pending = session.pending;
                save_session(&config_dir, &merged)?;
                session = merged;
            } else {
                session = latest;
            }
        } else {
            // 跑的时候换过地方：结果写回原来的聊天，界面仍显示当前地方。
            save_chat(
                &config_dir,
                &place_mode,
                &place_project,
                &MeChat {
                    messages: session.messages,
                    pending: session.pending,
                },
            )?;
            session = latest;
        }
    }
    Ok(view_of(&config_dir, &session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn parses_bash_and_keeps_the_preface() {
        let action = parse_model_action("我先看一眼。\n```bash\nls -la\n```\n好。").unwrap();
        match action {
            ModelAction::Run { prose, command } => {
                assert!(prose.contains("我先看一眼"));
                assert!(prose.contains("好"));
                assert_eq!(command, "ls -la");
            }
            ModelAction::Say(_) => panic!("expected a command"),
        }
    }

    #[test]
    fn plain_text_is_a_reply() {
        assert_eq!(
            parse_model_action("做完了").unwrap(),
            ModelAction::Say("做完了".to_string())
        );
    }

    #[test]
    fn empty_reply_is_a_format_error() {
        assert!(parse_model_action("  ").is_err());
        assert!(parse_model_action("```bash\n\n```").is_err());
    }

    #[test]
    fn only_the_first_fence_runs() {
        let action = parse_model_action("```bash\necho one\n```\n```bash\necho two\n```").unwrap();
        match action {
            ModelAction::Run { command, .. } => assert_eq!(command, "echo one"),
            ModelAction::Say(_) => panic!("expected a command"),
        }
    }

    #[test]
    fn confirm_heuristic_pauses_writes_and_lets_reads_through() {
        assert!(!command_needs_confirm("ls -la"));
        assert!(!command_needs_confirm("echo hi"));
        assert!(!command_needs_confirm("pwd && ls"));
        assert!(!command_needs_confirm("git status"));
        assert!(!command_needs_confirm("sed -n '1p' readme"));
        assert!(!command_needs_confirm("echo \"a > b\""));
        assert!(!command_needs_confirm("ls # rm -rf /"));
        assert!(command_needs_confirm("echo hi > notes.txt"));
        assert!(command_needs_confirm("rm -rf tmp"));
        assert!(command_needs_confirm("mkdir notes"));
        assert!(command_needs_confirm("ls && rm x"));
        assert!(command_needs_confirm("cat a | tee b"));
        assert!(command_needs_confirm("git commit -m \"x\""));
        assert!(command_needs_confirm("sed -i 's/a/b/' f"));
        assert!(command_needs_confirm("echo $(pwd)"));
    }

    #[test]
    fn phrases_confirm_or_dismiss_only_when_exact() {
        assert!(matches!(intake("写吧。"), Intake::Confirm));
        assert!(matches!(intake("Go ahead"), Intake::Confirm));
        assert!(matches!(intake("先别写"), Intake::Dismiss));
        assert!(matches!(intake("写吧，但是换个名字"), Intake::Talk(_)));
    }

    #[test]
    fn prune_shell_output_keeps_head_and_tail() {
        let short = "hello\n";
        let (kept, truncated) = prune_shell_output(short, 100);
        assert!(!truncated);
        assert_eq!(kept, short);

        let mut body = String::new();
        for i in 0..200 {
            body.push_str(&format!("LINE-{i:03}-ABCDEFGHIJKLMNOPQRSTUVWXYZ\n"));
        }
        let limit = 800;
        let (pruned, truncated) = prune_shell_output(&body, limit);
        assert!(truncated);
        assert!(pruned.chars().count() <= limit);
        assert!(pruned.starts_with("LINE-000-"));
        assert!(pruned.contains("LINE-199-"));
        assert!(pruned.contains("omitted"));
        assert!(pruned.contains("sed -n"));
        assert!(!pruned.contains("LINE-100-"));
    }

    #[test]
    fn fake_tool_calls_are_not_treated_as_done_prose() {
        let fake = "好的。<|tool_call_start|>[write(path='/tmp/a.md', content='hi')]<|tool_call_end|>";
        assert!(looks_like_unsupported_tools(fake));
        assert!(parse_model_action(fake).is_err());
        assert!(matches!(
            parse_model_action("就这样吧，先不动文件。"),
            Ok(ModelAction::Say(_))
        ));
        assert!(matches!(
            parse_model_action("写一下\n```bash\necho hi > a.md\n```"),
            Ok(ModelAction::Run { .. })
        ));
    }

    #[test]
    fn safety_only_replies_are_rejected() {
        let safety = "User Safety: unsafe\nResponse Safety: safe\nSafety Categories: Profanity";
        assert!(is_safety_only_reply(safety));
        assert!(parse_model_action(safety).is_err());
        let mixed = "User Safety: safe\n好的，我来写文件。\n```bash\ntouch a.txt\n```";
        assert!(matches!(
            parse_model_action(mixed),
            Ok(ModelAction::Run { .. })
        ));
    }

    #[test]
    fn plan_only_replies_are_rejected() {
        assert!(looks_like_plan_without_command("好的，按规范先读文档，再创建 txt。"));
        assert!(parse_model_action("好的，按规范先读文档，再创建 txt。").is_err());
        assert!(looks_like_false_completion(
            "✅ 已完成！\nexit: 0\ntotal 0\ndrwxr-xr-x 1 me staff 0 ."
        ));
        assert!(parse_model_action("✅ 全部文件已准备妥当！").is_err());
        assert!(has_unclosed_fence(
            "先读再写：\n```bash\ncat AGENTS.md && cat > /Users/sunwenlong/Documen"
        ));
        assert!(parse_model_action(
            "先读再写：\n```bash\ncat AGENTS.md && cat > /Users/sunwenlong/Documen"
        )
        .is_err());
        assert!(matches!(
            parse_model_action("随便聊聊天气吧。"),
            Ok(ModelAction::Say(_))
        ));
    }

    #[test]
    fn images_normalize_and_build_multimodal_user_content() {
        let ok = normalize_images(Some(vec![MeImage {
            media_type: "image/png".into(),
            data: "aGVsbG8=".into(),
        }]))
        .unwrap();
        assert_eq!(ok.len(), 1);
        assert!(normalize_images(Some(vec![MeImage {
            media_type: "image/svg+xml".into(),
            data: "aGVsbG8=".into(),
        }]))
        .is_err());
        let message = MeMessage {
            id: "1".into(),
            role: "user".into(),
            content: "看图".into(),
            images: ok,
            steps: Vec::new(),
            created_at: 0,
        };
        let content = user_model_content(&message, true).unwrap();
        let parts = content.as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert!(parts[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn prune_keeps_images_only_on_latest_user_turn() {
        let mut messages = vec![
            MeMessage {
                id: "u1".into(),
                role: "user".into(),
                content: "a".into(),
                images: vec![MeImage {
                    media_type: "image/png".into(),
                    data: "YQ==".into(),
                }],
                steps: Vec::new(),
                created_at: 1,
            },
            MeMessage {
                id: "a1".into(),
                role: "assistant".into(),
                content: "ok".into(),
                images: Vec::new(),
                steps: Vec::new(),
                created_at: 2,
            },
            MeMessage {
                id: "u2".into(),
                role: "user".into(),
                content: "b".into(),
                images: vec![MeImage {
                    media_type: "image/png".into(),
                    data: "Yg==".into(),
                }],
                steps: Vec::new(),
                created_at: 3,
            },
        ];
        prune_old_images(&mut messages);
        assert!(messages[0].images.is_empty());
        assert_eq!(messages[2].images.len(), 1);
    }

    #[test]
    fn project_names_stay_a_single_path_segment() {
        assert!(sanitize_project_name("../config").is_err());
        assert!(sanitize_project_name("a/b").is_err());
        assert!(sanitize_project_name(".hidden").is_err());
        assert_eq!(sanitize_project_name(" 笔记 ").unwrap(), "笔记");
    }

    #[test]
    fn project_work_dir_is_under_projects_and_config_is_the_parent() {
        let config = Path::new("/Users/me/Documents/.sugt");
        let project = resolve_work_dir(config, "project", "notes");
        let gateway = resolve_work_dir(config, "config", "notes");
        assert_eq!(project, config.join("projects").join("notes"));
        assert_eq!(gateway, config);
        assert_ne!(project, gateway);
        assert!(project.starts_with(projects_root(config)));
    }

    #[test]
    fn prompt_names_both_directories_and_a_fresh_shell() {
        let config = Path::new("/tmp/sugt-home");
        let work = config.join("projects").join("default");
        let prompt = system_prompt(
            true,
            &work,
            &projects_root(config),
            config,
            "项目「default」里",
        );
        assert!(prompt.contains("/tmp/sugt-home/projects"));
        assert!(prompt.contains("config.toml"));
        assert!(prompt.contains("新的 shell"));
        let english = system_prompt(
            false,
            &work,
            &projects_root(config),
            config,
            "project \"default\"",
        );
        assert!(english.contains("new shell"));
    }

    #[test]
    fn shell_runs_in_the_given_directory() {
        let dir = temp_dir("echo");
        fs::write(dir.join("marker.txt"), "here").unwrap();
        let outcome = run_shell(&dir, "pwd && cat marker.txt", Duration::from_secs(5), 1000);
        assert_eq!(outcome.exit_code, Some(0), "{}", outcome.output);
        assert!(outcome.output.contains("here"), "{}", outcome.output);
        let shown = outcome.output.lines().next().unwrap().trim();
        assert_eq!(
            Path::new(shown).canonicalize().unwrap(),
            dir.canonicalize().unwrap()
        );
    }

    #[test]
    fn shell_times_out_and_stops() {
        let dir = temp_dir("sleep");
        let outcome = run_shell(&dir, "sleep 5", Duration::from_millis(300), 1000);
        assert!(outcome.timed_out);
    }

    #[tokio::test]
    async fn loop_runs_a_read_then_stops_on_prose() {
        let dir = temp_dir("loop-read");
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ask = calls.clone();
        let ask = move |_messages: Vec<Value>| {
            let n = calls_ask.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Ok("```bash\necho from-su\n```".to_string())
                } else {
                    Ok("看到了".to_string())
                }
            }
        };
        let result = run_loop(
            ask,
            |_| {},
            true,
            &dir,
            &[],
            None,
            "",
            "system",
            LoopLimits::default(),
            false,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert_eq!(result.content, "看到了");
        assert!(result.pending.is_none());
        assert_eq!(result.steps.len(), 1);
        assert!(result.steps[0].output.contains("from-su"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn loop_holds_a_write_until_a_later_confirm_runs_it() {
        let dir = temp_dir("loop-write");
        let ask = |_messages: Vec<Value>| async {
            Ok("建一个目录\n```bash\nmkdir subdir\n```".to_string())
        };
        let held = run_loop(
            ask,
            |_| {},
            true,
            &dir,
            &[],
            None,
            "",
            "system",
            LoopLimits::default(),
            false,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert!(held.pending.is_some());
        assert!(!dir.join("subdir").exists());
        assert!(held.content.contains("建一个目录"));

        let auto_calls = Arc::new(AtomicUsize::new(0));
        let auto_calls_ask = auto_calls.clone();
        let ask_auto = move |_messages: Vec<Value>| {
            let n = auto_calls_ask.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Ok("自动写\n```bash\nmkdir auto_subdir\n```".to_string())
                } else {
                    Ok("目录已经建上了。".to_string())
                }
            }
        };
        let autoed = run_loop(
            ask_auto,
            |_| {},
            true,
            &dir,
            &[],
            None,
            "",
            "system",
            LoopLimits::default(),
            true,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert!(autoed.pending.is_none());
        assert!(dir.join("auto_subdir").is_dir());
        // 写入成功后会多一步「写后核对」
        assert!(autoed.steps.len() >= 2);
        assert!(autoed.steps.iter().any(|s| s.note.contains("写后核对")));
        assert!(autoed.content.contains("目录"));

        let command = held.pending.unwrap().command;
        let ask = |_messages: Vec<Value>| async { Ok("建好了".to_string()) };
        let done = run_loop(
            ask,
            |_| {},
            true,
            &dir,
            &[],
            Some(command),
            "建一个目录",
            "system",
            LoopLimits::default(),
            false,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert!(dir.join("subdir").is_dir());
        assert_eq!(done.content, "建好了");
        assert!(done.steps.len() >= 2);
        assert!(done.steps.iter().any(|s| s.note.contains("写后核对")));
        assert!(done.pending.is_none());
    }

    #[test]
    fn extract_write_paths_finds_redirects_and_mkdir() {
        let paths = extract_write_paths("echo hi > notes.txt && mkdir -p docs/out");
        assert!(paths.iter().any(|p| p == "notes.txt"));
        assert!(paths.iter().any(|p| p == "docs/out"));
        let tee = extract_write_paths("cat <<'EOF' > story.md\nhello\nEOF");
        assert!(tee.iter().any(|p| p == "story.md"));
    }

    #[tokio::test]
    async fn empty_model_text_is_retried_then_accepted() {
        let dir = temp_dir("loop-format");
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ask = calls.clone();
        let ask = move |_messages: Vec<Value>| {
            let n = calls_ask.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Ok("   ".to_string())
                } else {
                    Ok("在的".to_string())
                }
            }
        };
        let result = run_loop(
            ask,
            |_| {},
            true,
            &dir,
            &[],
            None,
            "",
            "system",
            LoopLimits::default(),
            false,
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert_eq!(result.content, "在的");
        assert!(result.steps.is_empty());
    }

    #[tokio::test]
    async fn step_limit_stops_a_loop_that_never_talks() {
        let dir = temp_dir("loop-limit");
        let ask = |_messages: Vec<Value>| async { Ok("```bash\necho again\n```".to_string()) };
        let mut limits = LoopLimits::default();
        limits.steps = 2;
        let result = run_loop(ask, |_| {}, true, &dir, &[], None, "", "system", limits, false, &AtomicBool::new(false))
            .await
            .unwrap();
        assert_eq!(result.steps.len(), 2);
        assert!(result.content.contains("步数"));
        assert!(result.pending.is_none());
    }

    #[test]
    fn session_roundtrip_keeps_pending_away_from_the_config_file_name() {
        let dir = temp_dir("session");
        let mut session = MeSession::default();
        session.project = "笔记".to_string();
        session.pending = Some(MePending {
            command: "printf 'hi' > readme.txt".to_string(),
            prose: "写到项目里".to_string(),
        });
        push_message(&mut session, "user", "写个说明".to_string(), Vec::new());
        save_session(&dir, &session).unwrap();
        let loaded = load_session(&dir);
        assert_eq!(loaded.project, "笔记");
        assert_eq!(loaded.pending.unwrap().prose, "写到项目里");
        assert!(chat_path(&dir, "project", "笔记").starts_with(chats_dir(&dir)));
        assert!(!chat_path(&dir, "project", "笔记").ends_with("config.toml"));
        let work = resolve_work_dir(&dir, &loaded.mode, &loaded.project);
        assert_eq!(work, dir.join("projects").join("笔记"));
        assert_ne!(work, dir.0);
    }

    #[test]
    fn chats_are_kept_per_place() {
        let dir = temp_dir("places");
        let mut a = MeSession::default();
        a.project = "alpha".to_string();
        push_message(&mut a, "user", "在 alpha".to_string(), Vec::new());
        save_session(&dir, &a).unwrap();

        switch_place(&mut a, &dir, "project", "beta", true).unwrap();
        assert!(a.messages.is_empty());
        push_message(&mut a, "user", "在 beta".to_string(), Vec::new());
        save_session(&dir, &a).unwrap();

        switch_place(&mut a, &dir, "project", "alpha", true).unwrap();
        assert_eq!(a.messages.len(), 1);
        assert_eq!(a.messages[0].content, "在 alpha");

        switch_place(&mut a, &dir, "config", "alpha", true).unwrap();
        assert!(a.messages.is_empty());
        assert_eq!(a.mode, "config");
    }

    #[tokio::test]
    async fn cancel_stops_the_loop_between_steps() {
        let dir = temp_dir("cancel");
        let cancel = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ask = calls.clone();
        let cancel_ask = cancel.clone();
        let ask = move |_messages: Vec<Value>| {
            let n = calls_ask.fetch_add(1, Ordering::SeqCst);
            let cancel_ask = cancel_ask.clone();
            async move {
                if n == 0 {
                    Ok("```bash\necho one\n```".to_string())
                } else {
                    cancel_ask.store(true, Ordering::SeqCst);
                    Ok("```bash\necho two\n```".to_string())
                }
            }
        };
        let result = run_loop(
            ask,
            |_| {},
            true,
            &dir,
            &[],
            None,
            "",
            "system",
            LoopLimits::default(),
            false,
            &cancel,
        )
        .await
        .unwrap();
        assert!(result.content.contains("停下"));
        assert_eq!(result.steps.len(), 1);
        assert!(result.pending.is_none());
    }

    struct TmpDir(PathBuf);

    impl std::ops::Deref for TmpDir {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(name: &str) -> TmpDir {
        let dir = std::env::temp_dir().join(format!(
            "sugt-me-{}-{}-{}",
            name,
            std::process::id(),
            now_ms()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        TmpDir(dir)
    }
}
