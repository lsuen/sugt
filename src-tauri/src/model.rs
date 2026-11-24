use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::gateway_stats::TrafficStatsView;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProviderProtocol {
    #[default]
    #[serde(alias = "open_ai", alias = "OpenAi", alias = "OpenAI")]
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderStatus {
    Unknown,
    Available,
    Unavailable,
}

/// Anthropic 接入点的协议模式（用户可在控制台 Agent 接入点切换）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AnthropicAccessMode {
    /// 自动匹配：识别到 Claude 系客户端优先走原生端点，否则走转换（默认）
    #[default]
    Auto,
    /// 强制走 OpenAI 协议（始终转换）；`open_ai` 为旧 snake_case 兼容 alias
    #[serde(alias = "open_ai")]
    OpenAi,
    /// 强制走 Anthropic 原生协议（服务商不支持时返回明确错误）
    Anthropic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model_name: String,
    #[serde(default)]
    pub protocol: ProviderProtocol,
    pub enabled: bool,
    pub status: ProviderStatus,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub model_alias: Option<String>,
    /// 保存时自动规范化 Base URL（补 /v1、修复火山路径等）；关闭则原样保存
    #[serde(default = "default_auto_adapt_base_url")]
    pub auto_adapt_base_url: bool,
    /// 未调用工具时是否再要。旧配置没有这个字段时视为开启。
    #[serde(default = "default_tool_text_retry_enabled")]
    pub tool_text_retry_enabled: bool,
    /// 第一次之外再要几次。旧配置没有这个字段时为 2。
    #[serde(default = "default_tool_text_retry_extra")]
    pub tool_text_retry_extra: u32,
}

impl ProviderConfig {
    pub fn new(
        name: impl Into<String>,
        provider: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            provider: provider.into(),
            base_url: trim_base_url(base_url.into()),
            api_key: api_key.into(),
            model_name: model_name.into(),
            protocol: ProviderProtocol::OpenAi,
            enabled: true,
            status: ProviderStatus::Unknown,
            last_checked_at: None,
            last_error: None,
            model_alias: None,
            auto_adapt_base_url: default_auto_adapt_base_url(),
            tool_text_retry_enabled: false,
            tool_text_retry_extra: 0,
        }
    }

    pub fn masked_key(&self) -> String {
        mask_secret(&self.api_key)
    }

    /// Agent 看到的模型名。没填别名时固定为 `sutai`，不跟随上游模型名。
    pub fn public_model_id(&self) -> String {
        self.model_alias
            .as_ref()
            .map(|alias| alias.trim())
            .filter(|alias| !alias.is_empty())
            .map(|alias| alias.to_string())
            .unwrap_or_else(|| CLIENT_MODEL_ID.to_string())
    }
}

/// 客户端默认只填这一套模型名。上游怎么换，对外都回这个名字，除非请求里自带了模型名。
pub const CLIENT_MODEL_ID: &str = "sutai";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuitBehavior {
    /// 仅退出程序；若已接管则尝试留下独立网关（不推荐，默认会清理接管）
    ExitOnly,
    /// 退出并停止网关；仍会清理接管写入
    StopGateway,
    /// 退出、停止网关并清理接管（推荐）
    #[default]
    StopAll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub failover_enabled: bool,
    pub active_provider_id: Option<String>,
    pub providers: Vec<ProviderConfig>,
    pub autostart: bool,
    #[serde(default)]
    pub autostart_gateway: bool,
    #[serde(default)]
    pub quit_behavior: QuitBehavior,
    /// 端口被占用时自动尝试备选端口
    #[serde(default = "default_port_fallback_enabled")]
    pub port_fallback_enabled: bool,
    #[serde(default = "default_port_fallback_ports")]
    pub port_fallback_ports: Vec<u16>,
    /// 后台循环检测网关并在不可达时自动拉起
    #[serde(default = "default_gateway_watchdog_enabled")]
    pub gateway_watchdog_enabled: bool,
    /// 允许局域网其它设备连接（绑定 0.0.0.0）
    #[serde(default)]
    pub allow_lan_access: bool,
    /// 客户端连接本地网关时使用的 API Key（可自定义防蹭网）
    #[serde(default = "default_gateway_client_api_key")]
    pub gateway_client_api_key: String,
    /// 是否显式启用 Codex 接管（默认关闭，避免 OPENAI_* 误伤 OpenCode 等）
    #[serde(default)]
    pub codex_takeover_enabled: bool,
    /// 是否显式启用 Claude 接管
    #[serde(default)]
    pub claude_takeover_enabled: bool,
    /// 启动时自动接管（已弃用，保留字段兼容旧配置）
    #[serde(default)]
    pub auto_takeover_enabled: bool,
    /// 应保持接管的模板 ID 列表（退出清环境，下次按此恢复）
    #[serde(default)]
    pub active_takeover_ids: Vec<String>,
    /// 用户已删除实验·免费通道，不再自动植入
    #[serde(default)]
    pub experimental_zen_dismissed: bool,
    /// Anthropic 接入点的协议模式（auto / openai / anthropic）
    #[serde(default)]
    pub anthropic_access_mode: AnthropicAccessMode,
    /// 流量悬浮窗（置顶穿透小窗）配置
    #[serde(default)]
    pub overlay: OverlayConfig,
    /// 全局工作区规则（软约束注入 + 少而精本地服务）
    #[serde(default)]
    pub workspace_rules: WorkspaceRules,
    /// 单次请求内上游重试的总等待预算（毫秒）。空表示 8 分钟。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_retry_budget_ms: Option<u64>,
    /// 单次等待上限（毫秒）。空表示 60 秒。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_retry_single_ms: Option<u64>,
    /// 日志自动清理保留天数；0 表示不自动清理。默认 7，上限 90。
    #[serde(default = "default_log_retention_days")]
    pub log_retention_days: u32,
    /// 是否启用局域网 WebUI 管理面板。默认关闭，关闭时不监听、不占内存。
    #[serde(default)]
    pub webui_enabled: bool,
    /// WebUI 监听端口，默认 8788（与网关 8787 分离）。
    #[serde(default = "default_webui_port")]
    pub webui_port: u16,
    /// 免费通道获取模型列表后是否逐路由真实测速（默认开启）。
    #[serde(default = "default_free_models_fetch_and_test")]
    pub free_models_fetch_and_test: bool,
    /// 自定义通道获取模型后是否逐模型真实测试（默认关闭）。
    #[serde(default)]
    pub custom_models_fetch_and_test: bool,
    /// 用户已确认实验·免费通道免责声明。
    #[serde(default)]
    pub experimental_free_disclaimer_accepted: bool,
    /// 闲时自动对免费通道路由池测速（默认关；会打公开源）。
    #[serde(default)]
    pub experimental_free_idle_probe_enabled: bool,
    /// 闲时探测间隔（秒）。默认 1800；下限 300。
    #[serde(default = "default_experimental_free_idle_probe_interval_secs")]
    pub experimental_free_idle_probe_interval_secs: u64,
    /// 闲时/手动测速后按 t/s（其次延迟）自动切到当前最优路由。
    #[serde(default)]
    pub experimental_free_auto_rotate: bool,
}

fn default_webui_port() -> u16 {
    8788
}

fn default_free_models_fetch_and_test() -> bool {
    true
}

fn default_experimental_free_idle_probe_interval_secs() -> u64 {
    1800
}

/// 闲时探测间隔下限（秒）。
pub const MIN_EXPERIMENTAL_FREE_IDLE_PROBE_INTERVAL_SECS: u64 = 300;

/// 日志保留期默认值（天）。
pub const DEFAULT_LOG_RETENTION_DAYS: u32 = 7;
/// 日志保留期上限（天）；设置页输入会被夹在这个范围内。
pub const MAX_LOG_RETENTION_DAYS: u32 = 90;

fn default_log_retention_days() -> u32 {
    DEFAULT_LOG_RETENTION_DAYS
}

/// 走网关的 agent 共享纪律：规则文本注入模型上下文；webhook 仅留本地代发。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceRules {
    #[serde(default)]
    pub enabled: bool,
    /// 全局规则原文（注入 system / instructions）
    #[serde(default)]
    pub rules_text: String,
    /// 文档习惯列表（可多条：AGENTS.md、目录地图等）
    #[serde(default)]
    pub doc_habits: Vec<DocHabit>,
    /// 文档习惯总开关。旧配置没有该字段时视为开启，避免已勾选的习惯失效。
    #[serde(default = "default_true")]
    pub docs_enabled: bool,
    #[serde(default, alias = "dingtalk_enabled")]
    pub notify_enabled: bool,
    /// 自定义机器人 webhook（不注入模型；由 /v1/_sugt/notify 代发）
    #[serde(default, alias = "dingtalk_webhook")]
    pub notify_webhook: String,
    /// 文档习惯注入文案。空表示使用内置默认；可用 {docs} 占位文档列表。
    #[serde(default)]
    pub inject_template: String,
    // —— 旧字段：读入后迁移到 doc_habits，不再写出 ——
    #[serde(default, skip_serializing)]
    pub map_doc_enabled: bool,
    #[serde(default, skip_serializing)]
    pub map_doc_path: String,
}

/// 单条文档习惯：开干前读、有变时更新。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DocHabit {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub path: String,
    /// 可选补充说明；空则用默认「阅读并更新」话术
    #[serde(default)]
    pub note: String,
}

impl Default for WorkspaceRules {
    fn default() -> Self {
        Self {
            enabled: false,
            rules_text: default_rules_text(),
            doc_habits: default_doc_habits(),
            docs_enabled: true,
            notify_enabled: false,
            notify_webhook: String::new(),
            inject_template: String::new(),
            map_doc_enabled: false,
            map_doc_path: String::new(),
        }
    }
}

/// 只放进全新配置。用户清空后保存的空字符串不要在 normalize 里填回。
fn default_rules_text() -> String {
    "用用户本次消息的语言回复。\n\
只做当前要求的事；要扩大范围时先说明。\n\
先查已有材料再动手。不知道就说明不知道，不要编造文件、接口、数据或已经完成的结果。\n\
声称完成前先核对自己做过的改动；核不出就标明未核实。\n\
未经明确要求，不要提交、推送、删除，也不要向外发送消息。\n"
        .to_string()
}

fn default_doc_habits() -> Vec<DocHabit> {
    vec![
        DocHabit {
            enabled: true,
            path: "AGENTS.md".to_string(),
            note: String::new(),
        },
        DocHabit {
            enabled: true,
            path: "docs/MAP.md".to_string(),
            note: String::new(),
        },
    ]
}

impl WorkspaceRules {
    /// 兼容旧 map_doc_*；清理空路径；补齐默认建议项（AGENTS.md / docs/MAP.md）。
    pub fn normalize(&mut self) {
        if self.map_doc_enabled || !self.map_doc_path.trim().is_empty() {
            let path = {
                let p = self.map_doc_path.trim();
                if p.is_empty() {
                    "docs/MAP.md".to_string()
                } else {
                    p.to_string()
                }
            };
            if let Some(existing) = self
                .doc_habits
                .iter_mut()
                .find(|h| h.path.eq_ignore_ascii_case(&path))
            {
                if self.map_doc_enabled {
                    existing.enabled = true;
                }
            } else {
                self.doc_habits.push(DocHabit {
                    enabled: self.map_doc_enabled,
                    path,
                    note: String::new(),
                });
            }
        }
        self.map_doc_enabled = false;
        self.map_doc_path.clear();

        for habit in &mut self.doc_habits {
            habit.path = habit.path.trim().to_string();
            habit.note = habit.note.trim().to_string();
        }
        self.doc_habits.retain(|h| !h.path.is_empty());
        if self.doc_habits.is_empty() {
            self.doc_habits = default_doc_habits();
        } else if let Some(index) = self
            .doc_habits
            .iter()
            .position(|h| h.path.eq_ignore_ascii_case("AGENTS.md"))
        {
            if index != 0 {
                let habit = self.doc_habits.remove(index);
                self.doc_habits.insert(0, habit);
            }
        } else {
            self.doc_habits.insert(
                0,
                DocHabit {
                    enabled: false,
                    path: "AGENTS.md".to_string(),
                    note: String::new(),
                },
            );
        }
        self.notify_webhook = self.notify_webhook.trim().to_string();
        let template = self.inject_template.trim().to_string();
        self.inject_template = if template == crate::workspace_rules::default_inject_template() {
            String::new()
        } else {
            template
        };
    }
}

/// 桌面精灵（小酥）窗口配置：置顶 + 透明 + 元素级穿透。
/// 流量 HUD 悬浮窗已废弃，本结构只服务精灵一个功能域；旧配置里的
/// `show_*` / `layout` / `pet_enabled` 由 serde 忽略，升级时不会丢弃其它设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayConfig {
    /// 启用桌面精灵：整窗唯一开关（旧配置的同名语义为「整窗」）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 立绘不透明度 0.1~1.0
    #[serde(default = "default_overlay_opacity")]
    pub opacity: f64,
    /// 编辑模式：关闭鼠标穿透以便拖动窗口 / 点保存（会话性）
    #[serde(default)]
    pub edit: bool,
    #[serde(default = "default_overlay_x")]
    pub x: i32,
    #[serde(default = "default_overlay_y")]
    pub y: i32,
    #[serde(default = "default_overlay_width")]
    pub width: f64,
    #[serde(default = "default_overlay_height")]
    pub height: f64,
    /// 精灵互动：悬停/点击/拖动；默认开启（全功能）
    #[serde(default = "default_true")]
    pub pet_interactive: bool,
    /// 拖动结束后自动贴边吸附（默认开启，对应桌面精灵「松手即停靠」行为）
    #[serde(default = "default_true")]
    pub pet_snap: bool,
    /// 无活动多久后进入休眠（秒）；旧配置缺省时按 5 分钟
    #[serde(default = "default_pet_sleep_after_secs")]
    pub pet_sleep_after_secs: u32,
    /// 结果气泡展示时长（毫秒）
    #[serde(default = "default_pet_bubble_ms")]
    pub pet_bubble_ms: u32,
    /// 视线跟随（转头 / 眼神高光）；关闭后保持正视
    #[serde(default = "default_true")]
    pub pet_gaze: bool,
    /// 随机眨眼；睡眠 / 离线态本身已是闭眼帧，不受此开关影响
    #[serde(default = "default_true")]
    pub pet_blink: bool,
    /// 免打扰：不出结果气泡（含轻碰），状态动画仍可切换
    #[serde(default)]
    pub pet_quiet: bool,
    /// 长期互动：等同常开 pet_interactive，与会话性 interactive 合并判定
    #[serde(default = "default_true")]
    pub pet_always_interactive: bool,
    /// 允许在当前显示器可用区内自主走动；关闭后精灵只在原地播动作
    #[serde(default = "default_true")]
    pub pet_roam: bool,
    /// 鼠标穿透：默认关闭（整窗可交互，拖拽/右键可用）；开启后由前端按立绘命中框动态切换
    #[serde(default)]
    pub pet_click_through: bool,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            opacity: default_overlay_opacity(),
            edit: false,
            // -1 表示「未定位」，由 overlay::apply 创建窗口时落到主屏右下角
            x: -1,
            y: -1,
            // 精灵默认紧凑窗：贴合立绘，减少透明命中框
            width: default_pet_width(),
            height: default_pet_height(),
            pet_interactive: true,
            pet_snap: true,
            pet_sleep_after_secs: default_pet_sleep_after_secs(),
            pet_bubble_ms: default_pet_bubble_ms(),
            pet_gaze: true,
            pet_blink: true,
            pet_quiet: false,
            pet_always_interactive: true,
            pet_roam: true,
            pet_click_through: false,
        }
    }
}

/// 按域增量补丁：只携带本次真正改动的字段。
/// 多窗口（设置页 / 精灵菜单 / 透明窗保存位置）并发写入时，全量快照会互相覆盖；
/// 后端以当前配置为基准 merge，保证任一写入点都不会抹掉别人的改动。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayConfigPatch {
    pub enabled: Option<bool>,
    pub opacity: Option<f64>,
    pub edit: Option<bool>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub pet_interactive: Option<bool>,
    pub pet_snap: Option<bool>,
    pub pet_sleep_after_secs: Option<u32>,
    pub pet_bubble_ms: Option<u32>,
    pub pet_gaze: Option<bool>,
    pub pet_blink: Option<bool>,
    pub pet_quiet: Option<bool>,
    pub pet_always_interactive: Option<bool>,
    pub pet_roam: Option<bool>,
    pub pet_click_through: Option<bool>,
}

impl OverlayConfigPatch {
    /// 把非空字段写进目标配置；未携带的字段保持原值。
    pub fn apply_to(&self, cfg: &mut OverlayConfig) {
        macro_rules! merge {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(value) = self.$field {
                    cfg.$field = value;
                })+
            };
        }
        merge!(
            enabled, opacity, edit, x, y, width, height,
            pet_interactive, pet_snap, pet_sleep_after_secs, pet_bubble_ms,
            pet_gaze, pet_blink, pet_quiet, pet_always_interactive,
            pet_roam, pet_click_through,
        );
    }
}

fn default_overlay_opacity() -> f64 {
    0.7
}

fn default_true() -> bool {
    true
}

fn default_overlay_x() -> i32 {
    -1
}

fn default_overlay_y() -> i32 {
    -1
}

fn default_overlay_width() -> f64 {
    // 缺省即精灵紧凑窗；HUD 已废弃，不再有第二种窗口形状。
    default_pet_width()
}

fn default_overlay_height() -> f64 {
    default_pet_height()
}

fn default_pet_width() -> f64 {
    // v1.4 产品默认中档（198×168）；小 160×136 / 大 240×204 仍可由设置切换。
    198.0
}

fn default_pet_height() -> f64 {
    168.0
}

fn default_pet_sleep_after_secs() -> u32 {
    300
}

fn default_pet_bubble_ms() -> u32 {
    4000
}

fn default_gateway_client_api_key() -> String {
    // 期望：与鉴权层共用唯一真相源，避免两处字面量漂移。
    crate::gateway_auth::DUMMY_LOCAL_KEY.to_string()
}

pub(crate) fn is_loopback_bind_host(host: &str) -> bool {
    matches!(
        host.trim().to_ascii_lowercase().as_str(),
        "" | "127.0.0.1" | "::1" | "localhost"
    )
}

fn default_port_fallback_enabled() -> bool {
    true
}

fn default_port_fallback_ports() -> Vec<u16> {
    vec![9878, 18887, 28887]
}

fn default_gateway_watchdog_enabled() -> bool {
    true
}

fn default_auto_adapt_base_url() -> bool {
    true
}

fn default_tool_text_retry_enabled() -> bool {
    true
}

fn default_tool_text_retry_extra() -> u32 {
    2
}

impl AppConfig {
    /// 实际 bind 地址
    pub fn bind_host(&self) -> String {
        if self.allow_lan_access {
            "0.0.0.0".to_string()
        } else if self.host.is_empty() {
            "127.0.0.1".to_string()
        } else {
            self.host.clone()
        }
    }

    /// 是否绑定到非回环地址（局域网 / 公网可达）。
    /// 此时网关必须要求客户端携带有效 API Key，避免上游 Key 被蹭用。
    pub fn binds_non_loopback(&self) -> bool {
        !is_loopback_bind_host(&self.bind_host())
    }

    /// 写入客户端环境变量 / 健康检查用的主机名
    pub fn client_host(&self) -> String {
        if self.host == "0.0.0.0" || self.allow_lan_access {
            "127.0.0.1".to_string()
        } else {
            self.host.clone()
        }
    }

    pub fn listen_url(&self) -> String {
        format!("http://{}:{}", self.client_host(), self.port)
    }

    pub fn port_candidates(&self) -> Vec<u16> {
        let mut ports = vec![self.port];
        if self.port_fallback_enabled {
            for port in &self.port_fallback_ports {
                if !ports.contains(port) {
                    ports.push(*port);
                }
            }
        }
        ports
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8787,
            failover_enabled: true,
            active_provider_id: None,
            providers: Vec::new(),
            autostart: false,
            autostart_gateway: false,
            quit_behavior: QuitBehavior::default(),
            port_fallback_enabled: default_port_fallback_enabled(),
            port_fallback_ports: default_port_fallback_ports(),
            gateway_watchdog_enabled: default_gateway_watchdog_enabled(),
            allow_lan_access: false,
            gateway_client_api_key: default_gateway_client_api_key(),
            codex_takeover_enabled: false,
            claude_takeover_enabled: false,
            auto_takeover_enabled: false,
            active_takeover_ids: Vec::new(),
            experimental_zen_dismissed: false,
            anthropic_access_mode: AnthropicAccessMode::default(),
            overlay: OverlayConfig::default(),
            workspace_rules: WorkspaceRules::default(),
            upstream_retry_budget_ms: None,
            upstream_retry_single_ms: None,
            log_retention_days: default_log_retention_days(),
            webui_enabled: false,
            webui_port: default_webui_port(),
            free_models_fetch_and_test: default_free_models_fetch_and_test(),
            custom_models_fetch_and_test: false,
            experimental_free_disclaimer_accepted: false,
            experimental_free_idle_probe_enabled: false,
            experimental_free_idle_probe_interval_secs:
                default_experimental_free_idle_probe_interval_secs(),
            experimental_free_auto_rotate: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyHit {
    pub provider_id: String,
    pub provider_name: String,
    pub path: String,
    /// 发起请求的客户端标签（如 claude-cli / codex），来自 User-Agent
    pub client: String,
    /// 实际走的协议路径：anthropic_native / openai_adapter / openai_direct /
    /// responses_native / chat_completions_adapter
    pub mode: String,
    pub failover: bool,
    /// 本请求是否注入了全局工作区规则
    #[serde(default)]
    pub workspace_rules_injected: bool,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    pub running: bool,
    pub listen_url: String,
    /// 用户配置的首选监听口（`config.toml` 的 `port`，不被回退绑定改写）。
    pub preferred_port: u16,
    /// 本次实际可达的监听口（回退绑定时可与 preferred 不同）。
    pub listen_port: u16,
    pub openai_base_url: String,
    pub anthropic_base_url: String,
    pub gateway_client_api_key: String,
    pub gateway_client_api_key_masked: String,
    pub public_model_id: Option<String>,
    pub allow_lan_access: bool,
    pub active_model: Option<String>,
    pub active_provider: Option<String>,
    pub active_provider_id: Option<String>,
    pub last_proxy_provider: Option<String>,
    pub last_proxy_path: Option<String>,
    /// 最近一次代理命中的客户端标签（如 claude-cli），无命中时为 None
    pub last_proxy_client: Option<String>,
    /// 最近一次代理命中的实际协议模式（anthropic_native / openai_adapter 等），无命中时为 None
    pub last_proxy_mode: Option<String>,
    pub last_proxy_failover: bool,
    pub last_proxy_at: Option<DateTime<Utc>>,
    /// 最近一次代理是否注入了全局规则（无命中或未知时为 None）
    pub last_proxy_workspace_rules: Option<bool>,
    pub config_dir: String,
    pub log_file: String,
    pub trial: TrialStatusView,
    pub traffic: TrafficStatsView,
    /// Anthropic 接入点协议模式（auto / openai / anthropic）
    pub anthropic_access_mode: AnthropicAccessMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrialStatusView {
    pub edition: String,
    pub product_line: String,
    pub product_label: String,
    pub trial_enabled: bool,
    pub valid: bool,
    pub status: String,
    pub message: String,
    pub build_id: String,
    pub expires_at: Option<String>,
    pub expires_date: Option<String>,
    pub days_remaining: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInput {
    pub id: Option<String>,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model_name: String,
    #[serde(default)]
    pub model_alias: Option<String>,
    #[serde(default)]
    pub protocol: ProviderProtocol,
    pub enabled: bool,
    #[serde(default = "default_auto_adapt_base_url")]
    pub auto_adapt_base_url: bool,
    #[serde(default)]
    pub tool_text_retry_enabled: bool,
    #[serde(default)]
    pub tool_text_retry_extra: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    /// 本地配置中的完整 Key，编辑/拉模型列表直接可用
    pub api_key: String,
    pub api_key_masked: String,
    pub model_name: String,
    pub model_alias: Option<String>,
    pub public_model_id: String,
    pub protocol: ProviderProtocol,
    pub enabled: bool,
    pub status: ProviderStatus,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub auto_adapt_base_url: bool,
    pub tool_text_retry_enabled: bool,
    pub tool_text_retry_extra: u32,
    /// 实验性免费通道（内置只读，可删可开关）
    pub experimental: bool,
}

impl From<&ProviderConfig> for ProviderView {
    fn from(value: &ProviderConfig) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
            provider: value.provider.clone(),
            base_url: value.base_url.clone(),
            api_key: value.api_key.clone(),
            api_key_masked: value.masked_key(),
            model_name: value.model_name.clone(),
            model_alias: value.model_alias.clone(),
            public_model_id: value.public_model_id(),
            protocol: value.protocol.clone(),
            enabled: value.enabled,
            status: value.status.clone(),
            last_checked_at: value.last_checked_at,
            last_error: value.last_error.clone(),
            auto_adapt_base_url: value.auto_adapt_base_url,
            tool_text_retry_enabled: value.tool_text_retry_enabled,
            tool_text_retry_extra: value.tool_text_retry_extra,
            experimental: crate::experimental_free::is_experimental_free(value),
        }
    }
}

pub fn trim_base_url(mut base_url: String) -> String {
    while base_url.ends_with('/') {
        base_url.pop();
    }
    base_url
}

/// OpenAI 兼容上游常见已带版本路径的后缀（火山 /api/v3、/api/coding/v3、智谱 /paas/v4 等），不应再追加 /v1。
pub fn openai_base_has_version_suffix(base: &str) -> bool {
    let base = trim_base_url(base.to_string());
    if base.ends_with("/v1")
        || base.ends_with("/api/v3")
        || base.ends_with("/api/coding/v3")
        || base.ends_with("/coding/v3")
        || base.ends_with("/paas/v4")
        || base.ends_with("/v4")
    {
        return true;
    }
    // 火山方舟 Coding Plan 等路径以 /v3 结尾
    if base.ends_with("/v3") && (base.contains("volces.com") || base.contains("/api/")) {
        return true;
    }
    false
}

/// 修复误追加 /v1 导致的双重后缀。
pub fn repair_openai_base_url(base_url: String) -> String {
    let base = trim_base_url(base_url);
    if base.ends_with("/api/v3/v1") || base.ends_with("/api/coding/v3/v1") {
        return base.strip_suffix("/v1").unwrap_or(&base).to_string();
    }
    if base.ends_with("/paas/v4/v1") {
        return base.strip_suffix("/v1").unwrap_or(&base).to_string();
    }
    if base.ends_with("/v4/v1") && base.contains("/paas/") {
        return base.strip_suffix("/v1").unwrap_or(&base).to_string();
    }
    if base.ends_with("/v3/v1") && base.contains("volces.com") {
        return base.strip_suffix("/v1").unwrap_or(&base).to_string();
    }
    base
}

/// 按协议规范化 Base URL（魔搭 OpenAI 需 /v1，Anthropic 原生通常不带 /v1）
pub fn normalize_base_url_for_protocol(base_url: String, protocol: &ProviderProtocol) -> String {
    let base = repair_openai_base_url(trim_base_url(base_url));
    match protocol {
        ProviderProtocol::OpenAi => {
            if openai_base_has_version_suffix(&base) {
                base
            } else {
                format!("{}/v1", base)
            }
        }
        ProviderProtocol::Anthropic => base,
    }
}

/// 保存/加载时解析 Base URL：`auto_adapt` 为 false 时仅去尾部斜杠。
pub fn resolve_provider_base_url(
    base_url: String,
    protocol: &ProviderProtocol,
    auto_adapt: bool,
) -> String {
    let trimmed = trim_base_url(base_url);
    if !auto_adapt {
        tracing::info!(
            raw = %trimmed,
            protocol = ?protocol,
            "provider base_url kept as configured (auto_adapt disabled)"
        );
        return trimmed;
    }
    let normalized = normalize_base_url_for_protocol(trimmed.clone(), protocol);
    if normalized != trimmed {
        tracing::info!(
            raw = %trimmed,
            normalized = %normalized,
            protocol = ?protocol,
            "provider base_url auto-adapted"
        );
    } else {
        tracing::debug!(
            raw = %trimmed,
            protocol = ?protocol,
            "provider base_url unchanged after auto-adapt check"
        );
    }
    normalized
}

pub fn mask_secret(secret: &str) -> String {
    // WB-009：按字符计数与切片，不能用字节下标。任意中文 / emoji key 都不能切在
    // UTF-8 字符中间 panic；短串（<= 8 字符）统一返回固定星号，不泄露长度信息。
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 8 {
        return "********".to_string();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}****{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_secret_handles_ascii_unicode_and_short_input() {
        // ASCII：保留前 4 后 4，与旧行为一致。
        assert_eq!(mask_secret("abcdefghijk"), "abcd****hijk");
        // 短串（含恰好 8 字符）：固定星号，不泄露长度。
        assert_eq!(mask_secret("abcd"), "********");
        assert_eq!(mask_secret("12345678"), "********");
        // 纯中文：按字符计数，9 字符必然安全脱敏，旧实现按字节必 panic。
        assert_eq!(mask_secret("密钥测试一二三四五"), "密钥测试****二三四五");
        // emoji 混排：按字符计数，不切在 UTF-8 中间。
        assert_eq!(mask_secret("🔑🔑🔑abcde🔑🔑🔑"), "🔑🔑🔑a****e🔑🔑🔑");
    }

    #[test]
    fn old_provider_without_tool_retry_fields_stays_on() {
        let raw = r#"{
            "id":"p","name":"n","provider":"openai","base_url":"https://x",
            "api_key":"k","model_name":"m","enabled":true,"status":"Unknown",
            "last_checked_at":null,"last_error":null
        }"#;
        let parsed: ProviderConfig = serde_json::from_str(raw).unwrap();
        assert!(parsed.tool_text_retry_enabled);
        assert_eq!(parsed.tool_text_retry_extra, 2);
        let fresh = ProviderConfig::new("n", "openai", "https://x", "k", "m");
        assert!(!fresh.tool_text_retry_enabled);
        assert_eq!(fresh.tool_text_retry_extra, 0);
    }

    #[test]
    fn deserializes_protocol_aliases() {
        assert_eq!(
            serde_json::from_str::<ProviderProtocol>(r#""openai""#).unwrap(),
            ProviderProtocol::OpenAi
        );
        assert_eq!(
            serde_json::from_str::<ProviderProtocol>(r#""open_ai""#).unwrap(),
            ProviderProtocol::OpenAi
        );
        assert_eq!(
            serde_json::from_str::<ProviderProtocol>(r#""anthropic""#).unwrap(),
            ProviderProtocol::Anthropic
        );
    }

    #[test]
    fn serializes_protocol_as_lowercase() {
        assert_eq!(
            serde_json::to_string(&ProviderProtocol::OpenAi).unwrap(),
            r#""openai""#
        );
    }

    #[test]
    fn normalizes_openai_base_url() {
        let url = normalize_base_url_for_protocol(
            "https://api-inference.modelscope.cn".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://api-inference.modelscope.cn/v1");
    }

    #[test]
    fn keeps_volcengine_api_v3_without_extra_v1() {
        let url = normalize_base_url_for_protocol(
            "https://ark.cn-beijing.volces.com/api/v3".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/v3");
    }

    #[test]
    fn repairs_volcengine_api_v3_v1_suffix() {
        let url = normalize_base_url_for_protocol(
            "https://ark.cn-beijing.volces.com/api/v3/v1".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/v3");
    }

    #[test]
    fn keeps_volcengine_coding_v3_without_extra_v1() {
        let url = normalize_base_url_for_protocol(
            "https://ark.cn-beijing.volces.com/api/coding/v3".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/coding/v3");
    }

    #[test]
    fn repairs_volcengine_coding_v3_v1_suffix() {
        let url = normalize_base_url_for_protocol(
            "https://ark.cn-beijing.volces.com/api/coding/v3/v1".into(),
            &ProviderProtocol::OpenAi,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/coding/v3");
    }

    #[test]
    fn resolve_skips_adapt_when_disabled() {
        let url = resolve_provider_base_url(
            "https://ark.cn-beijing.volces.com/api/coding/v3".into(),
            &ProviderProtocol::OpenAi,
            false,
        );
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/coding/v3");
    }

    #[test]
    fn keeps_anthropic_base_without_v1() {
        let url = normalize_base_url_for_protocol(
            "https://api-inference.modelscope.cn".into(),
            &ProviderProtocol::Anthropic,
        );
        assert_eq!(url, "https://api-inference.modelscope.cn");
    }

    #[test]
    fn overlay_patch_only_touches_carried_fields() {
        let mut cfg = OverlayConfig::default();
        let original_width = cfg.width;
        let original_gaze = cfg.pet_gaze;
        let patch = OverlayConfigPatch {
            enabled: Some(false),
            x: Some(320),
            ..OverlayConfigPatch::default()
        };
        patch.apply_to(&mut cfg);
        assert!(!cfg.enabled, "携带的 enabled 应生效");
        assert_eq!(cfg.x, 320, "携带的 x 应生效");
        assert_eq!(cfg.width, original_width, "未携带的 width 不得被重置");
        assert_eq!(cfg.pet_gaze, original_gaze, "未携带的行为开关不得被重置");
    }

    #[test]
    fn overlay_deserialize_ignores_legacy_hud_fields() {
        // 老配置里 HUD 专属字段与 pet_enabled 已废弃，反序列化必须忽略而不是报错，
        // 且精灵字段要按原值恢复。
        let raw = r#"{
            "enabled": true,
            "pet_enabled": false,
            "show_tokens": false,
            "show_requests": false,
            "layout": "row",
            "width": 240.0,
            "height": 204.0,
            "pet_quiet": true,
            "opacity": 0.8
        }"#;
        let cfg: OverlayConfig = serde_json::from_str(raw).expect("旧配置应可解析");
        assert!(cfg.enabled);
        assert_eq!(cfg.width, 240.0);
        assert_eq!(cfg.height, 204.0);
        assert!(cfg.pet_quiet);
        assert_eq!(cfg.opacity, 0.8);
        assert!(cfg.pet_interactive, "缺省的精灵开关应走 default_true");
    }
}
