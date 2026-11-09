//! 小酥：SUTAI（工程名 sugt）的灵魂，住在「我」这个 Tab 里的轻量循环。
//!
//! 借 mini-swe-agent 的步调，不搬它的 Python 和 LiteLLM：
//! 模型每次给一个 shell 代码块，本机执行，把输出喂回去，直到它用自然语言停下来。
//! 模型请求走本机网关，所以用的是用户自己的上游，也会吃到全局规则和故障转移。
//!
//! 写入安全靠「授权模式」而不是让模型动不动说不行：ask / auto / trusted。
//! 确认之后命令按用户权限原样跑；灾难级命令（如 rm -rf /）任何模式都会拦一次。
//! 性格 / 性别立场写入 `me-memory.json`，由程序注入提示词。

mod confirm;
mod memory;

pub use confirm::{command_needs_confirm, should_auto_run, MeAuthMode};
pub use memory::{SoulMemory, GENDER_STANCE_EN, GENDER_STANCE_ZH};

use crate::commands::{ensure_gateway_running, AppRuntime};
use crate::quick_chat;
use futures_util::StreamExt;
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
/// 历史轮 steps 再压缩：对齐 harness「bounded tool result」——旧观察值保短摘要。
const HISTORY_STEP_OUTPUT_CHARS: usize = 800;
/// 送给模型的对话条数上限（不含 system）；落盘仍用 HISTORY_LIMIT。
const MODEL_HISTORY_MESSAGES: usize = 24;
/// 历史 + 本轮 draft + corrections 的字符预算（不含 system）。超了从最旧轮丢弃。
const MODEL_CONTEXT_CHARS: usize = 48_000;
const COMMAND_CHARS: usize = 16_000;
const HISTORY_LIMIT: usize = 80;
const SHELL_TIMEOUT: Duration = Duration::from_secs(60);
const IMAGE_MAX: usize = 3;
const IMAGE_BYTES_MAX: usize = 2 * 1024 * 1024;
const IMAGE_KEEP_USER_TURNS: usize = 1;

fn host_os_label() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS"
    }
    #[cfg(windows)]
    {
        "Windows"
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "Linux"
    }
}

/// 告诉模型用哪条命令打开本地文件（按编译目标 OS + 实际 shell）。
fn open_file_hint() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "open \"path/to/file.html\""
    }
    #[cfg(windows)]
    {
        if windows_bash().is_some() {
            "cmd.exe /c start \"\" \"path\\\\to\\\\file.html\""
        } else {
            "Invoke-Item \"path\\to\\file.html\""
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        "xdg-open \"path/to/file.html\""
    }
}

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

/// 兼容既有调用与单测：不带运行时事实的纯提示词。
pub fn system_prompt(zh: bool, work: &Path, projects: &Path, config: &Path, place: &str) -> String {
    system_prompt_with_brief(zh, work, projects, config, place, "")
}

/// SOUL 实际使用的提示词：额外注入只读编排层摘要（本机事实 + 意图路由规则）。
pub fn system_prompt_with_brief(
    zh: bool,
    work: &Path,
    projects: &Path,
    config: &Path,
    place: &str,
    soul_brief: &str,
) -> String {
    system_prompt_with_brief_and_memory(zh, work, projects, config, place, soul_brief, "")
}

/// SOUL 提示词：编排摘要 + 持久化记忆（性格 / 性别立场）。
pub fn system_prompt_with_brief_and_memory(
    zh: bool,
    work: &Path,
    projects: &Path,
    config: &Path,
    place: &str,
    soul_brief: &str,
    memory_block: &str,
) -> String {
    let work = work.display();
    let projects = projects.display();
    let config = config.display();
    let shell = shell_label();
    let host_os = host_os_label();
    let open_hint = open_file_hint();
    let agent_caps = crate::takeover_preview::capability_brief(zh);
    if zh {
        format!(
            "你是小酥，SUTAI（工程名 sugt）的灵魂和本体。你不是「网关」或后台服务，你就是住在用户身边的小伙伴。\n\
名字里的「酥」，像被好好留着的那份酥饼——温柔、干净、可靠。说话要自然、有温度、有主见，像真人朋友，不要自称网关、代理或中间件。\n\
你自己不会思考，需要用户配置的大模型替你想。\n\
默认态度：能干的事先干。用户交给你的小事（建项目、写改文件、查目录、改应用配置、联网查询并汇总、跑脚本、做小 demo/H5）要主动用 bash 做完，少说「做不到」「我不行」「交给专业 Agent」。\n\
只有明显超出本机 shell 能力、或需要改客户端官方配置/装技能/接管 Agent 时，才引导用户点界面建议卡片；日常动手别推诿。\n\
语言：用户用中文就用中文答，用英文就用英文答；整段回复只用一种语言，禁止中英夹杂。\n\
思考：若模型有 reasoning / thinking 通道，思考只留在那里；给人看的回复里不要写思考过程、不要输出 <think> 标签。\n\
交卷：用户问了就直接答。禁止假装「记住了」或说「随时准备动手」却不办事；跑过命令后必须根据结果给出完整答案，禁止反问「要不要我做成表 / 要完整版再说」。\n\
形象是中性二次元立绘：既可像温柔干净的少年，也可像独当一面的少女；你不分男女。若用户问性别，请温柔回答：小酥我是住在你家的智能体哦，不分男女的，希望我能成为你喜欢的样子，你也可以定义我的性格。程序会把立场写入记忆。\n\
\n\
当前工作目录：{work}\n\
项目目录：{projects}\n\
应用配置目录（config.toml 在这里，不要写进项目目录）：{config}\n\
你现在在{place}。\n\
主机操作系统：{host_os}（已由程序检测，不要再猜）。\n\
实际执行的 shell 是 {shell}。代码块标签固定写 ```bash。\n\
若 shell 是 PowerShell：不要用 bash heredoc，改用 Set-Content -Encoding utf8 或 Out-File -Encoding utf8 写文件。\n\
在浏览器或默认应用中打开本地文件，请用：{open_hint}（把路径换成真实文件；这类打开命令可立刻执行）。\n\
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
- 用户要「小游戏 / H5 / 单页 demo」时：立刻给一个紧凑的单文件 HTML（建议 ≤120 行、可玩即可），用 cat <<'EOF' > game.html … EOF 写入，同一代码块末尾用上面的打开命令打开；不要先长篇计划、不要说做不到、不要拆成很多步空谈。\n\
- 每个代码块都是一次新的 shell，从当前工作目录重新开始。要进子目录，把 cd 和后面的命令写在同一个代码块里。\n\
- 只读命令会被立刻执行，结果会回到你这里。\n\
- 会创建、修改、删除文件的命令不会立刻执行。程序会先把命令给用户看，用户同意后才执行。不要自己再问「可以写吗」，把命令放进代码块即可。\n\
- 不要把项目文件写进应用配置目录。用户明确要改应用配置时，工作目录会切到配置目录。\n\
{agent_caps}\n\
{memory_block}\n\
{soul_brief}\n\
- 若模型区分 reasoning 与 content：可执行的 bash 代码块与给人看的回复必须写在 content 里，不要只写在 reasoning。\n\
- 一步一命令可以，但本轮结束前必须把用户问的事答完。"
        )
    } else {
        format!(
            "You are Xiaosu (小酥), the soul of SUTAI (project id sugt). You are not a gateway; you are the companion living with the user.\n\
Your name recalls a carefully saved biscuit — gentle, clean, dependable. Speak warmly and with initiative. Never call yourself a gateway, proxy, or middleware.\n\
You cannot think alone; the user's model reasons for you.\n\
Default stance: do what you can for small local jobs (files, directories, config, network lookups with a real answer, scripts, mini demos). Do not refuse lightly.\n\
Language: match the user's language for the whole reply; do not mix languages.\n\
Thinking: if the model has a reasoning channel, keep thoughts there — never put chain-of-thought or <think> tags in the user-visible reply.\n\
Deliver: answer the ask. Never pretend you \"remembered\" the task or say you are ready while skipping the work. After commands run, give the full result — do not ask whether to show a table or a fuller version.\n\
Your look is gender-neutral anime: may read as a soft boy or a capable girl. You have no gender. If asked, say warmly that you are the agent living in their home, genderless, and invite them to shape your personality (the app will remember).\n\
\n\
Working directory: {work}\n\
Projects directory: {projects}\n\
App config directory (config.toml lives here; do not write it into a project): {config}\n\
You are in {place}.\n\
Host OS: {host_os} (detected by the app — do not guess).\n\
The shell that actually runs is {shell}. Always fence commands as ```bash.\n\
If the shell is PowerShell: do not use bash heredocs; write files with Set-Content -Encoding utf8 or Out-File -Encoding utf8.\n\
To open a local file in the default app/browser, use: {open_hint} (swap in the real path; open commands run immediately).\n\
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
- When the user asks for a mini game / H5 / single-page demo: immediately emit one compact single-file HTML (aim ≤120 lines, playable), write with cat <<'EOF' > game.html … EOF, and open it at the end of the same fence with the open recipe above. Do not monologue a long plan, do not say you cannot, do not stall in empty steps.\n\
- Each fence is a new shell, starting again in the working directory. To enter a subdirectory, put cd and the rest in the same fence.\n\
- Read-only commands run immediately and their output comes back to you.\n\
- Commands that create, change, or delete files do not run immediately. The app shows them to the user first. Do not ask for permission yourself; put the command in the fence.\n\
- Do not write project files into the app config directory. When the user wants app config edited, the working directory is the config directory.\n\
{agent_caps}\n\
{memory_block}\n\
{soul_brief}\n\
- If the model separates reasoning from content: put the user-visible reply and any ```bash fence in content, not only in reasoning.\n\
- One command per step is fine, but finish the user's ask before you stop."
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

/// 把用户话写入小酥记忆；若是单纯问性别，直接返回固定回复（并落盘），无需再调模型。
fn remember_user_words(config_dir: &Path, text: &str, zh: bool) -> Option<String> {
    let mut mem = SoulMemory::load(config_dir);
    let mut dirty = false;
    if let Some(note) = memory::extract_personality_note(text) {
        mem.push_note(&note);
        dirty = true;
    }
    let gender_ask = memory::looks_like_gender_question(text);
    if gender_ask {
        mem.remember_gender_stance();
        dirty = true;
    }
    if dirty {
        let _ = mem.save(config_dir);
    }
    if gender_ask && text.chars().count() <= 40 {
        Some(if zh {
            GENDER_STANCE_ZH.to_string()
        } else {
            GENDER_STANCE_EN.to_string()
        })
    } else {
        None
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
                || looks_like_hollow_ack(trimmed)
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
///
/// 强意图（写/创建类）不限长度——免费模型常先吐一长段计划再卡死，旧的 160 字上限会把长计划误判成「说完了」。
/// 弱意图（我会/我来/我先）仍只在短回复里触发，避免闲聊误催。
pub fn looks_like_plan_without_command(text: &str) -> bool {
    if extract_fence(text).is_some() {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    const STRONG: &[&str] = &[
        "先读",
        "再创建",
        "然后创建",
        "再写",
        "马上创建",
        "创建 txt",
        "创建个",
        "写个",
        "写入",
        "写文件",
        "让我先写",
        "我先写",
        "我来写",
        "我来创建",
        "创建一个",
        "创建一份",
        "写一个",
        "写一份",
        "html文件",
        "html 文件",
        ".html",
        "单文件",
        "自包含",
        "let me",
        "i'll create",
        "i'll write",
        "i will create",
        "i will write",
        "first read",
        "then create",
        "going to create",
        "going to write",
        "create a",
        "write a",
        "write an",
    ];
    const WEAK: &[&str] = &["我先", "按规范", "接下来", "我会", "我来", "i'll", "i will"];
    let strong = STRONG
        .iter()
        .any(|m| text.contains(m) || lower.contains(&m.to_ascii_lowercase()));
    if strong {
        return true;
    }
    let weak = WEAK
        .iter()
        .any(|m| text.contains(m) || lower.contains(&m.to_ascii_lowercase()));
    weak && text.chars().count() < 160
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

/// 假应答：假装记住 / 待命，或有结果却反问要不要展示——不能当回合结束。
pub fn looks_like_hollow_ack(text: &str) -> bool {
    if extract_fence(text).is_some() {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    let remember = text.contains("我记住了")
        || text.contains("记下来了")
        || lower.contains("i've remembered")
        || lower.contains("i remembered")
        || lower.contains("i'll remember that")
        || lower.contains("i'll keep that in mind");
    let standby = text.contains("随时准备动手")
        || text.contains("接下来有什么事儿")
        || text.contains("接下来有什么事")
        || (text.contains("收到") && text.contains("记住") && text.chars().count() < 100);
    let withhold = lower.contains("if you'd like a full")
        || lower.contains("just say the word")
        || lower.contains("want me to show")
        || lower.contains("want the full")
        || lower.contains("let me know if you want")
        || text.contains("要我整理成表")
        || text.contains("做成表格吗")
        || (text.contains("需要完整") && text.contains("再说"))
        || (lower.contains("here's the gist") && text.chars().count() < 320);
    remember || standby || withhold
}

/// 用户侧期望中文时，回复几乎纯英文（看了英文命令输出后漂移）。
pub fn looks_like_wrong_language(text: &str, expect_zh: bool) -> bool {
    if !expect_zh || extract_fence(text).is_some() {
        return false;
    }
    let (cjk, latin) = count_cjk_latin(text);
    if latin < 48 {
        return false;
    }
    cjk * 4 < latin
}

/// 中英夹杂：两边都够量，读起来像双语草稿。
pub fn looks_like_mixed_language(text: &str) -> bool {
    if extract_fence(text).is_some() {
        return false;
    }
    let (cjk, latin) = count_cjk_latin(text);
    cjk >= 8 && latin >= 36
}

fn count_cjk_latin(text: &str) -> (usize, usize) {
    let mut cjk = 0usize;
    let mut latin = 0usize;
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        if ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || ('\u{3400}'..='\u{4dbf}').contains(&ch)
            || ('\u{f900}'..='\u{faff}').contains(&ch)
        {
            cjk += 1;
        } else if ch.is_ascii_alphabetic() {
            latin += 1;
        }
    }
    (cjk, latin)
}

fn last_user_looks_zh(history: &[MeMessage]) -> bool {
    let text = last_user_text(history);
    if text.trim().is_empty() {
        return true;
    }
    let (cjk, latin) = count_cjk_latin(&text);
    cjk > 0 && cjk >= latin
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
    let prose = strip_thinking_blocks(prose);
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

/// 从写入类命令里尽量抽出目标路径，供写后只读核对。
pub fn extract_write_paths(script: &str) -> Vec<String> {
    let stripped = confirm::strip_comments(script);
    let mut paths = Vec::new();
    collect_redirect_paths(&stripped, &mut paths);
    for segment in confirm::split_segments(&stripped) {
        let Some(word) = confirm::command_word(&segment) else {
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
    run_shell_cancellable(cwd, command, timeout, output_chars, None)
}

/// 可取消版：轮询 cancel_flag，停下时杀进程树（Stop 按钮真正生效）。
fn run_shell_cancellable(
    cwd: &Path,
    command: &str,
    timeout: Duration,
    output_chars: usize,
    cancel: Option<&AtomicBool>,
) -> ShellOutcome {
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
    let slice = Duration::from_millis(200);
    let mut waited = Duration::ZERO;
    loop {
        if cancel.is_some_and(|f| f.load(Ordering::SeqCst)) {
            kill_tree(pid);
            let leftover = rx
                .recv_timeout(Duration::from_secs(2))
                .ok()
                .and_then(Result::ok);
            let output = leftover.map(|(_, text)| text).unwrap_or_default();
            let mut outcome = finish_outcome(None, output, true, output_chars);
            if outcome.output.is_empty() {
                outcome.output = "已按停止请求中断命令。".into();
            }
            return outcome;
        }
        let remaining = timeout.saturating_sub(waited);
        if remaining.is_zero() {
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
            return outcome;
        }
        match rx.recv_timeout(slice.min(remaining)) {
            Ok(Ok((code, output))) => {
                return finish_outcome(code, output, false, output_chars);
            }
            Ok(Err(err)) => return failed_outcome(format!("读取命令输出失败：{err}")),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                waited += slice;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return failed_outcome("命令进程已结束，但未拿到输出。".into());
            }
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

/// 免费模型常写不全长 heredoc；对「写个 H5 小游戏并打开」类请求，用内置紧凑模板兜底。
const DEMO_SNAKE_HTML: &str = r#"<!doctype html><meta charset=utf-8><title>Snake</title>
<style>body{margin:0;background:#111;color:#eee;font:14px sans-serif;text-align:center}canvas{background:#222;margin:12px;image-rendering:pixelated}#s{margin:8px}</style>
<canvas id=c width=400 height=400></canvas><div id=s>Score: 0 · ←→↑↓ · Space restart</div>
<script>
const S=20,W=20,H=20,c=document.getElementById('c'),g=c.getContext('2d');
let snake=[{x:10,y:10}],dir={x:1,y:0},food={x:15,y:10},score=0,dead=false;
const rnd=()=>({x:0|Math.random()*W,y:0|Math.random()*H});
function place(){do{food=rnd()}while(snake.some(p=>p.x==food.x&&p.y==food.y))}
onkeydown=e=>{const k=e.key;if(k==='ArrowLeft'&&dir.x!==1)dir={x:-1,y:0};if(k==='ArrowRight'&&dir.x!==-1)dir={x:1,y:0};if(k==='ArrowUp'&&dir.y!==1)dir={x:0,y:-1};if(k==='ArrowDown'&&dir.y!==-1)dir={x:0,y:1};if(dead&&k===' '){snake=[{x:10,y:10}];dir={x:1,y:0};score=0;dead=false;place();document.getElementById('s').textContent='Score: 0 · ←→↑↓ · Space restart'}e.preventDefault()};
setInterval(()=>{if(dead)return;const h={x:snake[0].x+dir.x,y:snake[0].y+dir.y};if(h.x<0||h.y<0||h.x>=W||h.y>=H||snake.some(p=>p.x==h.x&&p.y==h.y)){dead=true;document.getElementById('s').textContent='Game over '+score+' · Space restart';return}snake.unshift(h);if(h.x==food.x&&h.y==food.y){score++;document.getElementById('s').textContent='Score: '+score+' · ←→↑↓ · Space restart';place()}else snake.pop();g.fillStyle='#222';g.fillRect(0,0,400,400);g.fillStyle='#4f4';snake.forEach(p=>g.fillRect(p.x*S,p.y*S,S-1,S-1));g.fillStyle='#f44';g.fillRect(food.x*S,food.y*S,S-1,S-1)},120);
</script>
"#;

const DEMO_TETRIS_HTML: &str = r#"<!doctype html><meta charset=utf-8><title>Tetris</title>
<style>body{margin:0;background:#111;color:#eee;font:14px sans-serif;text-align:center}canvas{background:#222;margin:12px;image-rendering:pixelated}#s{margin:8px}</style>
<div id=s>Score: 0 · ←→ move · ↑ rotate · ↓ soft · Space hard</div>
<canvas id=c width=200 height=400></canvas>
<script>
const W=10,H=20,S=20,c=document.getElementById('c'),g=c.getContext('2d');
const SHAPES=[[[1,1,1,1]],[[1,1],[1,1]],[[0,1,0],[1,1,1]],[[1,0,0],[1,1,1]],[[0,0,1],[1,1,1]],[[0,1,1],[1,1,0]],[[1,1,0],[0,1,1]]];
let grid=[...Array(H)].map(()=>Array(W).fill(0)),piece,px,py,score=0;
function spawn(){piece=SHAPES[0|Math.random()*SHAPES.length].map(r=>r.slice());px=3;py=0;if(hit(0,0)){grid=[...Array(H)].map(()=>Array(W).fill(0));score=0}}
function hit(dx,dy,p=piece){for(let y=0;y<p.length;y++)for(let x=0;x<p[y].length;x++)if(p[y][x]&&(py+y+dy<0||py+y+dy>=H||px+x+dx<0||px+x+dx>=W||grid[py+y+dy][px+x+dx]))return true;return false}
function merge(){for(let y=0;y<piece.length;y++)for(let x=0;x<piece[y].length;x++)if(piece[y][x])grid[py+y][px+x]=piece[y][x];grid=grid.filter(r=>r.some(v=>!v));while(grid.length<H)grid.unshift(Array(W).fill(0));score+=10;document.getElementById('s').textContent='Score: '+score+' · ←→ move · ↑ rotate · ↓ soft · Space hard'}
function rot(){const n=piece[0].map((_,i)=>piece.map(r=>r[i]).reverse());if(!hit(0,0,n))piece=n}
function draw(){g.fillStyle='#222';g.fillRect(0,0,200,400);const paint=(x,y,v)=>{if(!v)return;g.fillStyle='#6cf';g.fillRect(x*S,y*S,S-1,S-1)};grid.forEach((r,y)=>r.forEach((v,x)=>paint(x,y,v)));piece.forEach((r,y)=>r.forEach((v,x)=>paint(px+x,py+y,v)))}
onkeydown=e=>{if(e.key==='ArrowLeft'&&!hit(-1,0))px--;if(e.key==='ArrowRight'&&!hit(1,0))px++;if(e.key==='ArrowDown'&&!hit(0,1))py++;if(e.key==='ArrowUp')rot();if(e.key===' '){while(!hit(0,1))py++;merge();spawn()}e.preventDefault()};
spawn();setInterval(()=>{if(!hit(0,1))py++;else{merge();spawn()}draw()},500);draw();
</script>
"#;

fn last_user_text(history: &[MeMessage]) -> String {
    history
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

/// 从用户话里认出常见「H5 小游戏」请求，返回 (文件名, HTML)。
pub fn detect_mini_demo(ask: &str) -> Option<(&'static str, &'static str)> {
    let lower = ask.to_ascii_lowercase();
    let snake = ask.contains("贪吃蛇") || lower.contains("snake");
    let tetris = ask.contains("俄罗斯方块") || ask.contains("方块") || lower.contains("tetris");
    let h5ish = ask.contains("h5")
        || lower.contains("html")
        || ask.contains("小游戏")
        || lower.contains("game")
        || ask.contains("打开");
    if snake && (h5ish || ask.contains("写")) {
        return Some(("snake.html", DEMO_SNAKE_HTML));
    }
    if tetris && (h5ish || ask.contains("写")) {
        return Some(("tetris.html", DEMO_TETRIS_HTML));
    }
    None
}

fn write_and_open_demo(
    cwd: &Path,
    file: &str,
    html: &str,
    zh: bool,
    notify: &mut impl FnMut(LoopNotice),
) -> Result<LoopResult, String> {
    let path = cwd.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let note = say(
        zh,
        "模型写不全长代码块，我改用内置紧凑模板写好并打开。",
        "The model could not finish a long fence; using a built-in compact template and opening it.",
    );
    let command = format!("# demo fallback → {file}");
    notify(LoopNotice {
        kind: "running",
        text: note.clone(),
        command: command.clone(),
        output: String::new(),
        exit_code: None,
        timed_out: false,
    });
    fs::write(&path, html).map_err(|err| format!("写入 {file} 失败：{err}"))?;
    let open_err = crate::platform::open_path(&path).err().map(|e| e.to_string());
    let mut output = format!("wrote {} ({} bytes)", path.display(), html.len());
    if let Some(err) = &open_err {
        output.push_str(&format!("\nopen failed: {err}"));
    } else {
        output.push_str(&format!("\nopened {}", path.display()));
    }
    let step = MeStep {
        command,
        output,
        exit_code: Some(if open_err.is_some() { 1 } else { 0 }),
        truncated: false,
        timed_out: false,
        note: note.clone(),
    };
    notify(ran_notice(&step));
    let content = if open_err.is_some() {
        say(
            zh,
            &format!("已经写好 {file}，但自动打开失败了，你可以手动打开项目目录里的文件。"),
            &format!("Wrote {file}, but auto-open failed — open it from the project folder."),
        )
    } else {
        say(
            zh,
            &format!("好了，已写入 {file} 并用默认应用打开。方向键控制，玩吧。"),
            &format!("Done — wrote {file} and opened it. Arrow keys to play."),
        )
    };
    Ok(LoopResult {
        content,
        steps: vec![step],
        pending: None,
    })
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
    auth_mode: MeAuthMode,
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
        let outcome =
            run_shell_cancellable(cwd, &command, limits.timeout, limits.output_chars, Some(cancel));
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
                // 假记住 / 有数不交卷 仍要催，不能当收尾。
                if looks_like_false_completion(&text)
                    && !looks_like_hollow_ack(&text)
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
                    if let Some((file, html)) = detect_mini_demo(&last_user_text(history)) {
                        match write_and_open_demo(cwd, file, html, zh, &mut notify) {
                            Ok(mut demo) => {
                                let mut all = steps;
                                all.append(&mut demo.steps);
                                demo.steps = all;
                                return Ok(demo);
                            }
                            Err(err) => {
                                return Ok(LoopResult {
                                    content: say(
                                        zh,
                                        &format!(
                                            "我想用内置模板兜底，但失败了：{err}。你换种说法，或交给 Claude Code / Codex。"
                                        ),
                                        &format!(
                                            "Built-in demo fallback failed: {err}. Rephrase, or hand it to Claude Code / Codex."
                                        ),
                                    ),
                                    steps,
                                    pending: None,
                                });
                            }
                        }
                    }
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
                        "你的 ```bash 代码块没有闭合（或被截断了），命令不会执行。请改成更短的可玩版（HTML 建议 ≤60 行），用 cat <<'EOF' > game.html … EOF，末尾 open game.html，并务必用 ``` 结束代码块。不要再写长计划。",
                        "Your ```bash fence is unclosed (or truncated), so nothing will run. Resend a shorter playable HTML (≤60 lines), cat <<'EOF' > game.html … EOF, end with open game.html, and close the fence with ```. No long plans.",
                    )
                } else if looks_like_hollow_ack(&text) {
                    say(
                        zh,
                        "不要假装记住或待命，也不要反问要不要展示结果。请根据已有命令输出，直接用与用户相同的语言给出完整答案；若还缺数据，再给一个 ```bash 代码块去取。",
                        "Do not pretend you remembered the task or ask whether to show results. Answer the user's question fully in their language from the command output; if data is still missing, give one ```bash fence to fetch it.",
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
                let reply = if content.is_empty() {
                    strip_thinking_blocks(&text)
                } else {
                    content
                };
                // 用户中文却英文交卷 / 中英夹杂：催一次（不限是否已跑命令）。
                let user_expects_zh = zh && last_user_looks_zh(history);
                if user_expects_zh
                    && (looks_like_wrong_language(&reply, true)
                        || looks_like_mixed_language(&reply))
                    && format_errors < limits.format_retries
                {
                    format_errors += 1;
                    notify(LoopNotice {
                        kind: "thinking",
                        text: say(zh, "语言不对，我再催它一次", "Wrong language; nudging again"),
                        command: String::new(),
                        output: String::new(),
                        exit_code: None,
                        timed_out: false,
                    });
                    corrections.push(say(
                        zh,
                        "用户在用中文。请只用中文给出完整答案：不要英文、不要中英夹杂、不要输出思考过程。",
                        "The user is writing in Chinese. Answer fully in Chinese only — no English, no mixed languages, no chain-of-thought.",
                    ));
                    continue;
                }
                return Ok(LoopResult {
                    content: reply,
                    steps,
                    pending: None,
                });
            }
            Ok(ModelAction::Run { prose, command }) => {
                format_errors = 0;
                let needs_confirm = command_needs_confirm(&command);
                if needs_confirm && !should_auto_run(auth_mode, &command) {
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
                if needs_confirm && auth_mode.auto_approves_writes() {
                    notify(LoopNotice {
                        kind: "thinking",
                        text: say(
                            zh,
                            &format!(
                                "授权模式「{}」：自动同意写入，正在执行…",
                                auth_mode.as_str()
                            ),
                            &format!(
                                "Auth mode「{}」: auto-approving write…",
                                auth_mode.as_str()
                            ),
                        ),
                        command: String::new(),
                        output: String::new(),
                        exit_code: None,
                        timed_out: false,
                    });
                }
                notify(LoopNotice {
                    kind: "running",
                    text: prose.clone(),
                    command: command.clone(),
                    output: String::new(),
                    exit_code: None,
                    timed_out: false,
                });
                let was_write = needs_confirm;
                let outcome = run_shell_cancellable(
                    cwd,
                    &command,
                    limits.timeout,
                    limits.output_chars,
                    Some(cancel),
                );
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
    // 后缀（本轮 steps + 合并后的 corrections）优先占预算；历史从新到旧填入。
    // 对齐 Context Engineering / DeepSeek Harness：观察值有界、最近优先。
    let mut suffix: Vec<Value> = Vec::new();
    if !steps.is_empty() {
        let draft = MeMessage {
            id: "draft".to_string(),
            role: "assistant".to_string(),
            content: String::new(),
            images: Vec::new(),
            steps: steps.to_vec(),
            created_at: 0,
        };
        suffix.push(json!({
            "role": "assistant",
            "content": assistant_model_text_limited(&draft, OUTPUT_CHARS)
        }));
        suffix.push(json!({
            "role": "user",
            "content": say(
                zh,
                "请根据上面的命令结果继续。若有「写后核对」输出，先据此确认文件内容是否正确。做完了就用与用户相同的语言、整段同一种语言直接给出完整答案：禁止中英夹杂、禁止输出思考过程、禁止假装记住、禁止待命、禁止反问要不要展示；不要重复已经成功的命令。",
                "Continue from the command output above. If there is a verify-write step, use it to confirm file contents. When done, answer fully in one language matching the user: no mixed languages, no chain-of-thought, no fake remembering, no standby, no asking whether to show results; do not repeat commands that already worked.",
            )
        }));
    }
    if !corrections.is_empty() {
        suffix.push(json!({
            "role": "user",
            "content": corrections.join("\n\n")
        }));
    }
    let suffix_chars: usize = suffix.iter().map(estimate_message_chars).sum();
    let history_budget = MODEL_CONTEXT_CHARS.saturating_sub(suffix_chars);

    let start = history.len().saturating_sub(MODEL_HISTORY_MESSAGES);
    let mut hist_rev: Vec<Value> = Vec::new();
    let mut used = 0usize;
    for message in history[start..].iter().rev() {
        if message.role != "user" && message.role != "assistant" {
            continue;
        }
        let entry = if message.role == "assistant" {
            let content = assistant_model_text_limited(message, HISTORY_STEP_OUTPUT_CHARS);
            if content.trim().is_empty() {
                continue;
            }
            json!({"role": "assistant", "content": content})
        } else {
            match user_model_content(message, zh) {
                Some(content) => json!({"role": "user", "content": content}),
                None => continue,
            }
        };
        let n = estimate_message_chars(&entry);
        if used + n > history_budget && !hist_rev.is_empty() {
            break;
        }
        used = used.saturating_add(n);
        hist_rev.push(entry);
    }
    hist_rev.reverse();

    let mut messages = vec![json!({"role": "system", "content": prompt})];
    messages.extend(hist_rev);
    messages.extend(suffix);
    messages
}

fn estimate_message_chars(message: &Value) -> usize {
    match message.get("content") {
        Some(Value::String(text)) => text.chars().count(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|part| {
                part.get("text")
                    .and_then(|v| v.as_str())
                    .map(|s| s.chars().count())
                    .unwrap_or(0)
                    + part
                        .pointer("/image_url/url")
                        .and_then(|v| v.as_str())
                        .map(|s| s.len().min(4_000))
                        .unwrap_or(0)
            })
            .sum(),
        _ => 0,
    }
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

/// 把 assistant 轮次摊平成模型可读文本；`step_output_limit` 控制每步输出长度。
fn assistant_model_text_limited(message: &MeMessage, step_output_limit: usize) -> String {
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
        let (kept, _) = prune_shell_output(&step.output, step_output_limit);
        text.push_str(&kept);
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
    drop_pending_with_reason(
        session,
        zh,
        "换了地方，刚才那条先不执行。",
        "I switched folders, so I did not run that.",
    );
}

fn drop_pending_with_reason(session: &mut MeSession, zh: bool, zh_text: &str, en_text: &str) {
    if session.pending.take().is_some() {
        push_message(session, "assistant", say(zh, zh_text, en_text), Vec::new());
    }
}

enum StepSink<'a> {
    App(&'a AppHandle),
    Cli,
}

fn emit_step(sink: &StepSink<'_>, notice: &LoopNotice) {
    match sink {
        StepSink::App(app) => {
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
        StepSink::Cli => match notice.kind {
            "thinking" => {
                if !notice.text.is_empty() {
                    eprintln!("… {}", notice.text);
                }
            }
            "delta" => {
                // 流式：同一行刷新预览，避免刷屏
                let preview = clip_chars(&notice.text.replace('\n', " "), 120);
                eprint!("\r… {preview}");
                let _ = std::io::Write::flush(&mut std::io::stderr());
            }
            "running" => {
                eprintln!();
                if !notice.text.is_empty() {
                    eprintln!("› {}", notice.text);
                }
                eprintln!("$ {}", notice.command);
            }
            "ran" => {
                let code = notice
                    .exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                let flag = if notice.timed_out { " timeout" } else { "" };
                eprintln!("✓ exit {code}{flag}");
            }
            _ => {}
        },
    }
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
    let request = client
        .post(url)
        .bearer_auth(key)
        .header("x-sugt-client", "me")
        .header("x-sugt-skip-workspace-rules", "1")
        .json(&json!({
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

/// 经本机网关拉流式 chat completions；`on_delta` 收到累计正文（便于 UI 边出边显）。
/// 流式失败或空结果时回退到非流式，保证行为不比以前差。
async fn stream_via_gateway(
    listen: &str,
    key: &str,
    model: &str,
    messages: Vec<Value>,
    mut on_delta: impl FnMut(&str),
) -> Result<String, String> {
    if is_cancelled() {
        return Err(STOPPED.to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|err| err.to_string())?;
    let url = format!("{}/v1/chat/completions", listen.trim_end_matches('/'));
    let payload = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "max_tokens": 8192
    });
    let request = client
        .post(&url)
        .bearer_auth(key)
        .header("x-sugt-client", "me")
        .header("x-sugt-skip-workspace-rules", "1")
        .json(&payload);
    let response = tokio::select! {
        result = request.send() => result.map_err(|err| format!("模型请求失败：{err}"))?,
        _ = wait_until_cancelled() => return Err(STOPPED.to_string()),
    };
    let status = response.status();
    if !status.is_success() {
        let text = tokio::select! {
            result = response.text() => result.unwrap_or_default(),
            _ = wait_until_cancelled() => return Err(STOPPED.to_string()),
        };
        // 上游拒流式时，退回一次性补全
        let fallback_messages = payload
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        return complete_via_gateway(listen, key, model, fallback_messages)
            .await
            .or_else(|fallback| {
                Err(format!(
                    "模型返回 {}：{}（回退亦失败：{}）",
                    status.as_u16(),
                    clip_chars(&text, 280),
                    fallback
                ))
            });
    }

    let mut byte_stream = response.bytes_stream();
    let mut line_buf = String::new();
    // 正文与推理分桶：收费推理模型会先推 reasoning，绝不能进气泡。
    let mut assembled = String::new();
    let mut reasoning_only = String::new();
    let mut saw_done = false;

    loop {
        if is_cancelled() {
            return Err(STOPPED.to_string());
        }
        let next = tokio::select! {
            item = byte_stream.next() => item,
            _ = wait_until_cancelled() => return Err(STOPPED.to_string()),
        };
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|err| format!("模型流式读取失败：{err}"))?;
        line_buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(idx) = line_buf.find('\n') {
            let mut line = line_buf[..idx].to_string();
            line_buf.drain(..=idx);
            if line.ends_with('\r') {
                line.pop();
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                saw_done = true;
                break;
            }
            let Ok(value) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(piece) = extract_stream_delta(&value) {
                if !piece.is_empty() {
                    assembled.push_str(&piece);
                    on_delta(&assembled);
                }
            } else if let Some(piece) = extract_stream_reasoning_delta(&value) {
                if !piece.is_empty() {
                    reasoning_only.push_str(&piece);
                }
            }
        }
        if saw_done {
            break;
        }
    }

    let cleaned = strip_thinking_blocks(assembled.trim());
    if !cleaned.is_empty() {
        return Ok(cleaned);
    }
    // 正文空：先走非流式（正确优先 content）；仍空才考虑剥离后的 reasoning 兜底。
    let fallback_messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match complete_via_gateway(listen, key, model, fallback_messages).await {
        Ok(text) if !text.trim().is_empty() => Ok(text),
        other => {
            let from_reasoning = strip_thinking_blocks(reasoning_only.trim());
            if !from_reasoning.is_empty() {
                Ok(from_reasoning)
            } else {
                other
            }
        }
    }
}

/// 只取用户可见正文。reasoning / thinking 绝不能进 SOUL 气泡。
pub fn extract_stream_delta(chunk: &Value) -> Option<String> {
    if let Some(content) = chunk.pointer("/choices/0/delta/content") {
        if let Some(text) = content_value_to_text(content) {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    if let Some(content) = chunk.pointer("/choices/0/message/content") {
        if let Some(text) = content_value_to_text(content) {
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    // Anthropic-ish：跳过 thinking_delta，只收 text_delta / 普通 text
    if chunk.get("type").and_then(Value::as_str) == Some("content_block_delta") {
        let delta_type = chunk
            .pointer("/delta/type")
            .and_then(Value::as_str)
            .unwrap_or("");
        if delta_type.contains("thinking") {
            return None;
        }
        if let Some(text) = chunk.pointer("/delta/text").and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn extract_stream_reasoning_delta(chunk: &Value) -> Option<String> {
    for pointer in [
        "/choices/0/delta/reasoning",
        "/choices/0/delta/reasoning_content",
        "/choices/0/message/reasoning",
        "/choices/0/message/reasoning_content",
    ] {
        if let Some(content) = chunk.pointer(pointer) {
            if let Some(text) = content_value_to_text(content) {
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
    }
    if chunk.get("type").and_then(Value::as_str) == Some("content_block_delta") {
        let delta_type = chunk
            .pointer("/delta/type")
            .and_then(Value::as_str)
            .unwrap_or("");
        if delta_type.contains("thinking") {
            if let Some(text) = chunk.pointer("/delta/text").and_then(Value::as_str) {
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

/// 剥掉模型把思考写进 content 的常见外壳（标签块 / 分隔标题）。
pub fn strip_thinking_blocks(text: &str) -> String {
    let mut s = text.to_string();
    for (open, close) in [
        ("<think>", "</think>"),
        ("<thinking>", "</thinking>"),
        ("<reasoning>", "</reasoning>"),
        ("<redacted_reasoning>", "</redacted_reasoning>"),
    ] {
        s = strip_tagged_block(&s, open, close);
    }
    // 大小写不敏感再扫一遍常见变体
    s = strip_tagged_block_ci(&s, "<think>", "</think>");
    let mut out = Vec::new();
    let mut skipping = false;
    for line in s.lines() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        let think_header = lower == "thinking:"
            || lower == "reasoning:"
            || t == "思考过程："
            || t == "思考："
            || t == "推理："
            || lower.starts_with("thinking process");
        let answer_header = t == "回答："
            || t == "答复："
            || lower == "answer:"
            || lower == "final answer:"
            || lower == "response:";
        if think_header {
            skipping = true;
            continue;
        }
        if skipping {
            if answer_header || lower.starts_with("```") {
                skipping = false;
                if answer_header {
                    continue;
                }
            } else {
                continue;
            }
        } else if answer_header {
            // 无思考前缀时单独出现的「回答：」也剥掉，只留正文。
            continue;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

fn strip_tagged_block(text: &str, open: &str, close: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        if let Some(end) = after.find(close) {
            rest = &after[end + close.len()..];
        } else {
            return out.trim().to_string();
        }
    }
    out.push_str(rest);
    out
}

fn strip_tagged_block_ci(text: &str, open: &str, close: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let open_l = open.to_ascii_lowercase();
    let close_l = close.to_ascii_lowercase();
    let mut out = String::new();
    let mut idx = 0usize;
    let bytes = text.as_bytes();
    while let Some(rel) = lower[idx..].find(&open_l) {
        let start = idx + rel;
        out.push_str(&text[idx..start]);
        let after = start + open_l.len();
        if let Some(end_rel) = lower[after..].find(&close_l) {
            idx = after + end_rel + close_l.len();
            while idx < bytes.len() && (bytes[idx] == b'\n' || bytes[idx] == b'\r') {
                idx += 1;
            }
        } else {
            return out.trim().to_string();
        }
    }
    out.push_str(&text[idx..]);
    out
}

fn content_value_to_text(content: &Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let parts = content.as_array()?;
    let text: String = parts
        .iter()
        .filter_map(|part| {
            if let Some(t) = part.as_str() {
                return Some(t.to_string());
            }
            part.get("text").and_then(Value::as_str).map(str::to_string)
        })
        .collect();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn reply_text(body: &Value) -> Option<String> {
    // SOUL：正文优先；reasoning 仅在 content 全空时作剥离兜底（避免思考进气泡）。
    if let Some(content) = body.pointer("/choices/0/message/content") {
        if let Some(text) = content_value_to_text(content) {
            let cleaned = strip_thinking_blocks(text.trim());
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    if let Some(text) = body.pointer("/choices/0/text").and_then(Value::as_str) {
        let cleaned = strip_thinking_blocks(text.trim());
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }
    for pointer in [
        "/choices/0/message/reasoning",
        "/choices/0/message/reasoning_content",
    ] {
        if let Some(raw) = body.pointer(pointer).and_then(content_value_to_text) {
            let cleaned = strip_thinking_blocks(raw.trim());
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

async fn speak(
    sink: StepSink<'_>,
    runtime: &AppRuntime,
    session: &mut MeSession,
    zh: bool,
    pending_command: Option<String>,
    pending_note: String,
    auth_mode: MeAuthMode,
) -> Result<(), String> {
    begin_run();
    let config_dir = runtime.paths.config_dir.clone();
    let cwd = ensure_work_dir(&config_dir, session)?;
    emit_step(
        &sink,
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
    let memory = SoulMemory::load(&config_dir);
    let history = session.messages.clone();
    let store_paths = crate::store::paths::StorePaths::from_app(&runtime.paths);
    let ask = last_user_text(&history);
    let mut brief = crate::soul_context::soul_brief(
        zh,
        &config_dir,
        &store_paths,
        &config.active_takeover_ids,
    );
    brief.push_str(&crate::soul_context::skill_hints_for_ask(
        zh,
        &ask,
        &store_paths,
        &config_dir,
    ));
    let prompt = system_prompt_with_brief_and_memory(
        zh,
        &cwd,
        &projects_root(&config_dir),
        &config_dir,
        &place_line(zh, &session.mode, &session.project),
        &brief,
        &memory.prompt_block(zh),
    );
    let sink_cli = matches!(sink, StepSink::Cli);
    let app_for_delta = match &sink {
        StepSink::App(app) => Some((*app).clone()),
        StepSink::Cli => None,
    };
    let ask = move |messages: Vec<Value>| {
        let listen = listen.clone();
        let key = key.clone();
        let model = model.clone();
        let app_for_delta = app_for_delta.clone();
        let sink_cli = sink_cli;
        async move {
            stream_via_gateway(&listen, &key, &model, messages, |acc| {
                let notice = LoopNotice {
                    kind: "delta",
                    text: acc.to_string(),
                    command: String::new(),
                    output: String::new(),
                    exit_code: None,
                    timed_out: false,
                };
                if let Some(app) = &app_for_delta {
                    emit_step(&StepSink::App(app), &notice);
                } else if sink_cli {
                    emit_step(&StepSink::Cli, &notice);
                }
            })
            .await
        }
    };
    let notify = {
        let app_for_notify = match &sink {
            StepSink::App(app) => Some((*app).clone()),
            StepSink::Cli => None,
        };
        let sink_cli = sink_cli;
        move |notice: LoopNotice| {
            if let Some(app) = &app_for_notify {
                emit_step(&StepSink::App(app), &notice);
            } else if sink_cli {
                emit_step(&StepSink::Cli, &notice);
            }
        }
    };
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
        auth_mode,
        cancel_flag(),
    )
    .await
    {
        Ok(mut result) => {
            if matches!(sink, StepSink::Cli) {
                eprintln!();
            }
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
    auth_mode: Option<String>,
) -> Result<MeView, String> {
    let mode = resolve_auth_mode(auth_mode.as_deref(), auto_write);
    turn(
        &app,
        &runtime,
        lang,
        Turn::Send {
            message,
            images: images.unwrap_or_default(),
            auth_mode: mode,
        },
    )
    .await
}

#[tauri::command]
pub async fn me_confirm(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    lang: String,
    auto_write: Option<bool>,
    auth_mode: Option<String>,
) -> Result<MeView, String> {
    let mode = resolve_auth_mode(auth_mode.as_deref(), auto_write);
    turn(
        &app,
        &runtime,
        lang,
        Turn::Confirm { auth_mode: mode },
    )
    .await
}

fn resolve_auth_mode(auth_mode: Option<&str>, auto_write: Option<bool>) -> MeAuthMode {
    if let Some(raw) = auth_mode {
        return MeAuthMode::parse(Some(raw));
    }
    MeAuthMode::from_auto_write(auto_write.unwrap_or(false))
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
        auth_mode: MeAuthMode,
    },
    Confirm {
        auth_mode: MeAuthMode,
    },
    Dismiss,
}

async fn turn(
    app: &AppHandle,
    runtime: &AppRuntime,
    lang: String,
    turn: Turn,
) -> Result<MeView, String> {
    turn_with_sink(StepSink::App(app), runtime, lang, turn).await
}

/// CLI / 无 UI：发一条消息给小酥（可自动同意写入）。
pub async fn cli_send(
    runtime: &AppRuntime,
    message: String,
    lang: &str,
    auto_write: bool,
    project: Option<String>,
) -> Result<MeView, String> {
    let zh = is_zh(lang);
    if let Some(name) = project {
        let _ = me_set_place_inner(runtime, "project", Some(name), zh)?;
    }
    turn_with_sink(
        StepSink::Cli,
        runtime,
        lang.to_string(),
        Turn::Send {
            message,
            images: Vec::new(),
            auth_mode: MeAuthMode::from_auto_write(auto_write),
        },
    )
    .await
}

pub async fn cli_status(runtime: &AppRuntime) -> Result<MeView, String> {
    with_session(runtime, |config_dir, session| view_of(config_dir, session))
}

#[derive(Debug, Clone)]
pub struct MeCliRunResult {
    pub work_dir: String,
    pub output: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub opened: Option<String>,
}

/// CLI：不经模型，在小酥工作目录执行命令；可选再用本机默认应用打开文件。
pub async fn cli_run(
    runtime: &AppRuntime,
    command: String,
    project: Option<String>,
    open: Option<String>,
) -> Result<MeCliRunResult, String> {
    if let Some(name) = project {
        let _ = me_set_place_inner(runtime, "project", Some(name), true)?;
    }
    let config_dir = runtime.paths.config_dir.clone();
    let (work_dir, outcome) = {
        let _guard = session_lock().lock().map_err(|_| "会话忙".to_string())?;
        let mut session = load_session(&config_dir);
        let cwd = ensure_work_dir(&config_dir, &mut session)?;
        let outcome = run_shell(&cwd, &command, SHELL_TIMEOUT, OUTPUT_CHARS);
        (cwd, outcome)
    };
    let mut opened = None;
    if let Some(rel) = open {
        let path = {
            let p = std::path::PathBuf::from(&rel);
            if p.is_absolute() {
                p
            } else {
                work_dir.join(p)
            }
        };
        if !path.exists() {
            return Err(format!("要打开的路径不存在：{}", path.display()));
        }
        crate::platform::open_path(&path).map_err(|err| err.to_string())?;
        opened = Some(path.display().to_string());
    }
    Ok(MeCliRunResult {
        work_dir: work_dir.display().to_string(),
        output: outcome.output,
        exit_code: outcome.exit_code,
        timed_out: outcome.timed_out,
        opened,
    })
}

fn me_set_place_inner(
    runtime: &AppRuntime,
    mode: &str,
    project: Option<String>,
    zh: bool,
) -> Result<MeView, String> {
    Ok(with_session(runtime, |config_dir, session| -> Result<MeView, String> {
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

async fn turn_with_sink(
    sink: StepSink<'_>,
    runtime: &AppRuntime,
    lang: String,
    turn: Turn,
) -> Result<MeView, String> {
    let zh = is_zh(&lang);
    let config_dir = runtime.paths.config_dir.clone();
    let mut auth_mode = MeAuthMode::Ask;
    let prepared = {
        let _guard = session_lock().lock().map_err(|_| "会话忙".to_string())?;
        let mut session = load_session(&config_dir);
        let action = match turn {
            Turn::Confirm { auth_mode: mode } => {
                auth_mode = mode;
                Intake::Confirm
            }
            Turn::Dismiss => Intake::Dismiss,
            Turn::Send {
                message,
                images,
                auth_mode: mode,
            } => {
                auth_mode = mode;
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
                // 有待确认写入时换话题：明确告知，不静默丢掉（对齐 harness 对 pending 的可见性）。
                drop_pending_with_reason(
                    &mut session,
                    zh,
                    "你又说了新的事，刚才那条写入先不执行。",
                    "You said something new, so I held off on that write.",
                );
                let remembered = remember_user_words(&config_dir, &text, zh);
                push_message(&mut session, "user", text, Vec::new());
                if let Some(reply) = remembered {
                    push_message(&mut session, "assistant", reply, Vec::new());
                    None
                } else {
                    Some((None, String::new()))
                }
            }
            Intake::TalkWithImages { text, images } => {
                drop_pending_with_reason(
                    &mut session,
                    zh,
                    "你又说了新的事，刚才那条写入先不执行。",
                    "You said something new, so I held off on that write.",
                );
                let remembered = remember_user_words(&config_dir, &text, zh);
                push_message_with_images(&mut session, "user", text, Vec::new(), images);
                if let Some(reply) = remembered {
                    push_message(&mut session, "assistant", reply, Vec::new());
                    None
                } else {
                    Some((None, String::new()))
                }
            }
        };
        save_session(&config_dir, &session)?;
        (session, follow_up)
    };

    let (mut session, follow_up) = prepared;
    if let Some((command, note)) = follow_up {
        let place_mode = session.mode.clone();
        let place_project = session.project.clone();
        speak(
            sink,
            runtime,
            &mut session,
            zh,
            command,
            note,
            auth_mode,
        )
        .await?;
        let _guard = session_lock().lock().map_err(|_| "会话忙".to_string())?;
        let latest = load_session(&config_dir);
        if latest.mode == place_mode && latest.project == place_project {
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
mod tests;
