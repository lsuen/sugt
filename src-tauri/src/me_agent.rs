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
const STEP_LIMIT: usize = 8;
const FORMAT_RETRIES: usize = 2;
const OUTPUT_CHARS: usize = 6_000;
const COMMAND_CHARS: usize = 16_000;
const HISTORY_LIMIT: usize = 80;
const SHELL_TIMEOUT: Duration = Duration::from_secs(25);
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
    if cfg!(windows) {
        "PowerShell"
    } else {
        "bash"
    }
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
\n\
怎么回复：\n\
- 需要跑命令时，回复里只放一个 bash 代码块。块前用一两句人话说明要动哪个路径、写什么。\n\
- 只是说话或已经做完时，不要放代码块。\n\
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
\n\
How to reply:\n\
- When you need a command, include exactly one bash fence. Before it, say the path and what will be written, in a sentence or two.\n\
- When you are only talking, or you are done, do not include a fence.\n\
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
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(());
    }
    match extract_fence(trimmed) {
        Some((prose, command)) => {
            let command = command.trim().trim_matches('\u{feff}').trim();
            if command.is_empty() || command.chars().count() > COMMAND_CHARS {
                return Err(());
            }
            Ok(ModelAction::Run {
                prose: prose.trim().to_string(),
                command: command.to_string(),
            })
        }
        None => Ok(ModelAction::Say(trimmed.to_string())),
    }
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
    let count = output.chars().count();
    if count <= limit {
        return ShellOutcome {
            exit_code: code,
            output,
            truncated: false,
            timed_out,
        };
    }
    ShellOutcome {
        exit_code: code,
        output: format!("{}\n…", clip_chars(&output, limit)),
        truncated: true,
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
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", command])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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
                corrections.push(say(
                    zh,
                    "你的回复是空的，或代码块是空的。请用自然语言回答，或给出一个 bash 代码块。",
                    "That reply was empty, or the fence was empty. Answer in prose, or give one bash fence.",
                ));
            }
            Ok(ModelAction::Say(text)) => {
                return Ok(LoopResult {
                    content: text,
                    steps,
                    pending: None,
                });
            }
            Ok(ModelAction::Run { prose, command }) => {
                format_errors = 0;
                if command_needs_confirm(&command) {
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
                let outcome = run_shell(cwd, &command, limits.timeout, limits.output_chars);
                let step = step_from(&command, &prose, outcome);
                notify(ran_notice(&step));
                steps.push(step);
                used_steps += 1;
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
                "请根据上面的命令结果继续。做完了就用自然语言告诉我，不要重复已经成功的命令。",
                "Continue from the command output above. When you are done, tell me in prose and do not repeat commands that already worked.",
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
        "max_tokens": 4096
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
        cancel_flag(),
    )
    .await
    {
        Ok(result) => {
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
) -> Result<MeView, String> {
    turn(
        &app,
        &runtime,
        lang,
        Turn::Send {
            message,
            images: images.unwrap_or_default(),
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
    let prepared = {
        let _guard = session_lock().lock().map_err(|_| "会话忙".to_string())?;
        let mut session = load_session(&config_dir);
        let action = match turn {
            Turn::Confirm => Intake::Confirm,
            Turn::Dismiss => Intake::Dismiss,
            Turn::Send { message, images } => {
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
        speak(app, runtime, &mut session, zh, command, note).await?;
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
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert!(held.pending.is_some());
        assert!(!dir.join("subdir").exists());
        assert!(held.content.contains("建一个目录"));

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
            &AtomicBool::new(false),
        )
        .await
        .unwrap();
        assert!(dir.join("subdir").is_dir());
        assert_eq!(done.content, "建好了");
        assert_eq!(done.steps.len(), 1);
        assert!(done.pending.is_none());
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
        let result = run_loop(ask, |_| {}, true, &dir, &[], None, "", "system", limits, &AtomicBool::new(false))
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
