import React, { Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import {
  invoke, emit, listen,
  isBrowserMode, getSessionToken, setSessionToken,
  AUTH_EXPIRED_EVENT,
} from './transport';
import {
  Activity, Bot, CheckCircle2, CircleStop, Cpu, FolderOpen, Play, Plus,
  ScrollText, Settings2, ShieldCheck, Sparkles, Trash2, Users, Wand2, X, XCircle,
} from 'lucide-react';
import { BrandMark } from './BrandMark';
import { ModalPortal } from './ModalPortal';
import { bootstrapTheme, useTheme } from './theme';
import './styles.css';
import { GatewayAccessBanner } from './GatewayAccessBanner';
import type { SettingsTab } from './SettingsPanel';
import type { OverlayConfigView } from './pet/config';
import { TrafficPanel } from './TrafficPanel';
import { ProviderModal } from './ProviderModal';
import type { WorkspaceRulesView } from './WorkspaceRulesPanel';
import { FreeChannelSettingsModal } from './FreeChannelSettingsModal';
// 面板走独立 chunk：首屏只留控制台需要的组件，其余在空闲时预取。
import {
  AccessInfoCard,
  ApiKeyPoolPanel,
  ClientKeyPanel,
  MePanel,
  ModelListPanel,
  RequestRulesSection,
  SettingsPanel,
  SkillsPanel,
  StrategyBoxPanel,
  TakeoverProfilesSection,
  WorkspaceRulesPanel,
  preloadPanels,
  preloadPanelPreview,
} from './lazyPanels';
import type { ProviderForm } from './providerPresets';
import { t, type TKey } from './i18n';
import { useNoticeQueue } from './notify/useNoticeQueue';
import type { Notice, NoticeInput } from './notify/messageModel';
import { editionLabel } from './AboutSection';
import { StatusBar } from './StatusBar';
import { loadStatusBarEnabled, persistStatusBarEnabled } from './uiPrefs';

bootstrapTheme();

// 左侧导航：门面层（控制台 / SOUL）在分割线之上，工具层（网关 / 规则 / 接入 / 技能 / 设置）在下。
// 组内用横向子 Tab；当前组由当前叶子反推，避免「当前组」与「当前页」两套状态打架。
type NavGroup = 'overview' | 'soul' | 'gateway' | 'rules' | 'access' | 'skills' | 'settings';
type NavLeaf =
  | 'dashboard' | 'me'
  | 'models' | 'modelList' | 'strategyBox' | 'keyPool'
  | 'rulesInject' | 'rulesRewrite'
  | 'clients' | 'clientKeys' | 'skills'
  | 'settings';

type NavLeafDef = { id: NavLeaf; labelKey: TKey };
type NavGroupDef = { id: NavGroup; icon: typeof Activity; labelKey: TKey; leaves: NavLeafDef[] };

/** 门面层：分割线之上；其余组一律视为工具层。 */
const FACADE_GROUP_IDS: ReadonlySet<NavGroup> = new Set(['overview', 'soul']);

const NAV_GROUPS: NavGroupDef[] = [
  {
    id: 'overview',
    icon: Activity,
    labelKey: 'nav.group.overview',
    leaves: [
      { id: 'dashboard', labelKey: 'nav.dashboard' },
    ],
  },
  {
    id: 'soul',
    icon: Sparkles,
    labelKey: 'nav.me',
    leaves: [
      { id: 'me', labelKey: 'nav.me' },
    ],
  },
  {
    id: 'gateway',
    icon: Bot,
    labelKey: 'nav.group.gateway',
    leaves: [
      { id: 'models', labelKey: 'nav.models' },
      { id: 'modelList', labelKey: 'nav.modelList' },
      { id: 'strategyBox', labelKey: 'nav.strategyBox' },
      { id: 'keyPool', labelKey: 'nav.keyPool' },
    ],
  },
  {
    id: 'rules',
    icon: ScrollText,
    labelKey: 'nav.group.rules',
    leaves: [
      { id: 'rulesInject', labelKey: 'nav.rulesInject' },
      { id: 'rulesRewrite', labelKey: 'nav.rulesRewrite' },
    ],
  },
  {
    id: 'access',
    icon: Users,
    labelKey: 'nav.group.access',
    leaves: [
      { id: 'clients', labelKey: 'nav.clients' },
      { id: 'clientKeys', labelKey: 'nav.clientKeys' },
    ],
  },
  {
    id: 'skills',
    icon: Wand2,
    labelKey: 'nav.group.skills',
    leaves: [
      { id: 'skills', labelKey: 'nav.skills' },
    ],
  },
  {
    id: 'settings',
    icon: Settings2,
    labelKey: 'nav.group.settings',
    leaves: [
      { id: 'settings', labelKey: 'nav.settings' },
    ],
  },
];

const NAV_GROUP_STORAGE_KEY = 'sugt_nav_group';
const NAV_LEAF_STORAGE_PREFIX = 'sugt_nav_leaf_';

const NAV_DEFAULT_LEAF: Record<NavGroup, NavLeaf> = {
  overview: 'dashboard',
  soul: 'me',
  gateway: 'models',
  rules: 'rulesInject',
  access: 'clients',
  skills: 'skills',
  settings: 'settings',
};

function navGroupOf(leaf: NavLeaf): NavGroup {
  const found = NAV_GROUPS.find((group) => group.leaves.some((item) => item.id === leaf));
  return found ? found.id : 'overview';
}

function isNavGroup(value: string | null): value is NavGroup {
  return NAV_GROUPS.some((group) => group.id === value);
}

/** 读取某组上次停留的叶子；取值必须属于该组，非法一律回退默认叶子。 */
function storedLeafFor(group: NavGroup): NavLeaf {
  const def = NAV_GROUPS.find((item) => item.id === group);
  try {
    const raw = localStorage.getItem(`${NAV_LEAF_STORAGE_PREFIX}${group}`);
    if (raw && def?.leaves.some((item) => item.id === raw)) return raw as NavLeaf;
  } catch {
    /* localStorage 不可用（隐私模式）时按默认叶子启动 */
  }
  return NAV_DEFAULT_LEAF[group];
}

/** 哪些导航组有可预取的面板 chunk（overview 全是首屏同步组件，无需预取）。 */
const PRELOADABLE_GROUPS = new Set<string>(['soul', 'gateway', 'rules', 'access', 'skills', 'settings']);
function isPreloadableGroup(group: NavGroup): boolean {
  return PRELOADABLE_GROUPS.has(group);
}

/**
 * 旧版：SOUL 在「总览」叶子、技能在「接入」叶子。
 * 升为一级主 Tab 后迁到 soul / skills 组，避免旧 localStorage 指到空页。
 */
function migrateNavStorage() {
  try {
    const overviewLeaf = localStorage.getItem(`${NAV_LEAF_STORAGE_PREFIX}overview`);
    if (overviewLeaf === 'me') {
      localStorage.setItem(NAV_GROUP_STORAGE_KEY, 'soul');
      localStorage.setItem(`${NAV_LEAF_STORAGE_PREFIX}soul`, 'me');
      localStorage.removeItem(`${NAV_LEAF_STORAGE_PREFIX}overview`);
    }
    const accessLeaf = localStorage.getItem(`${NAV_LEAF_STORAGE_PREFIX}access`);
    if (accessLeaf === 'skills') {
      localStorage.setItem(NAV_GROUP_STORAGE_KEY, 'skills');
      localStorage.setItem(`${NAV_LEAF_STORAGE_PREFIX}skills`, 'skills');
      localStorage.removeItem(`${NAV_LEAF_STORAGE_PREFIX}access`);
    }
  } catch {
    /* 隐私模式无持久化，跳过迁移 */
  }
}

function initialLeaf(): NavLeaf {
  migrateNavStorage();
  try {
    const raw = localStorage.getItem(NAV_GROUP_STORAGE_KEY);
    return storedLeafFor(isNavGroup(raw) ? raw : 'overview');
  } catch {
    return NAV_DEFAULT_LEAF.overview;
  }
}

type ProviderStatus = 'Unknown' | 'Available' | 'Unavailable';
type ProviderProtocol = 'openai' | 'anthropic';

/** 与 src-tauri/src/probe.rs 的 ProbeReport 对齐（camelCase） */
type ProbeReport = {
  ok: boolean;
  reply: string | null;
  latencyMs: number;
  tokensPerSec: number;
  completionTokens: number;
  estimated: boolean;
  error: string | null;
};

/** 与 src-tauri/src/gateway.rs 的 ProviderTestOutcome 对齐 */
type ProviderTestOutcome = {
  status: ProviderStatus;
  report: ProbeReport;
};

type RuntimeStatus = {
  running: boolean;
  listen_url: string;
  preferred_port?: number;
  listen_port?: number;
  openai_base_url?: string;
  anthropic_base_url?: string;
  gateway_client_api_key?: string;
  gateway_client_api_key_masked?: string;
  public_model_id?: string | null;
  allow_lan_access?: boolean;
  anthropic_access_mode?: 'auto' | 'openai' | 'anthropic';
  active_model?: string | null;
  active_provider?: string | null;
  active_provider_id?: string | null;
  last_proxy_provider?: string | null;
  last_proxy_path?: string | null;
  last_proxy_client?: string | null;
  last_proxy_mode?: string | null;
  last_proxy_failover?: boolean;
  last_proxy_at?: string | null;
  last_proxy_workspace_rules?: boolean | null;
  config_dir: string;
  log_file: string;
  trial: TrialStatus;
  traffic: import('./providerPresets').TrafficStats;
};

type TrialStatus = {
  edition: string;
  product_line: string;
  product_label: string;
  trial_enabled: boolean;
  valid: boolean;
  status: string;
  message: string;
  build_id: string;
  expires_at?: string | null;
  expires_date?: string | null;
  days_remaining?: number | null;
};

type ProviderView = {
  id: string;
  name: string;
  provider: string;
  base_url: string;
  api_key: string;
  api_key_masked: string;
  model_name: string;
  /** 与后端 ProviderView.model_alias 对齐：用户自定义公开别名，切换模型时须原样保留 */
  model_alias?: string | null;
  /** 与后端 ProviderView.public_model_id 对齐：后端下发的对外 model id */
  public_model_id?: string;
  protocol: ProviderProtocol | 'open_ai';
  enabled: boolean;
  status: ProviderStatus;
  last_checked_at?: string | null;
  last_error?: string | null;
  auto_adapt_base_url?: boolean;
  tool_text_retry_enabled?: boolean;
  tool_text_retry_extra?: number;
  experimental?: boolean;
};

type AppConfig = {
  host: string;
  port: number;
  failover_enabled: boolean;
  active_provider_id?: string | null;
  autostart: boolean;
  autostart_gateway: boolean;
  auto_takeover_enabled?: boolean;
  claude_takeover_enabled?: boolean;
  codex_takeover_enabled?: boolean;
  quit_behavior: QuitBehavior;
  close_to_tray?: boolean;
  port_fallback_enabled?: boolean;
  gateway_watchdog_enabled?: boolean;
  allow_lan_access?: boolean;
  port_fallback_ports?: number[];
  upstream_retry_budget_ms?: number | null;
  upstream_retry_single_ms?: number | null;
  providers: unknown[];
  overlay?: OverlayConfigView;
  workspace_rules?: WorkspaceRulesView;
};

type QuitBehavior = 'exit_only' | 'stop_gateway' | 'stop_all';

type ClientEnvStatus = {
  client: string;
  configured: boolean;
  variables?: Record<string, string>;
  missing?: string[];
  issues?: string[];
  note: string;
};

type ClientsEnvStatus = {
  listen_url: string;
  gateway_reachable?: boolean;
  claude: ClientEnvStatus;
  codex: ClientEnvStatus;
  has_issues?: boolean;
};

const emptyForm: ProviderForm = {
  name: '',
  provider: '',
  base_url: '',
  api_key: '',
  model_name: '',
  protocol: 'openai',
  enabled: true,
  auto_adapt_base_url: true,
  tool_text_retry_enabled: false,
  tool_text_retry_extra: '',
};

function formatInvokeError(error: unknown): string {
  const text = String(error);
  const marker = 'gateway_not_running:';
  const idx = text.indexOf(marker);
  if (idx >= 0) return text.slice(idx + marker.length);
  return text;
}

// 对外 pushToast 的 kind 取值保持不变；内部颜色/定级统一交给 messageModel.guessSeverity。 —— 孙文龙 sun
type ToastKind = 'ok' | 'error' | 'info';

// 主窗口 Toast 宿主：渲染通知队列的可视切片，支持分类/级别、合并计数、轻量动作与被挤出计数。 —— 孙文龙 sun
function ToastHost({
  notices, hidden, onDismiss, onAction, onPauseChange,
}: {
  notices: Notice[];
  hidden: number;
  onDismiss: (id: number) => void;
  onAction: (notice: Notice) => void;
  onPauseChange: (paused: boolean) => void;
}) {
  if (!notices.length) return null;
  // 存在 error/warn 时容器用 assertive，其余用 polite；单条紧急项另带 role="alert"。 —— 孙文龙 sun
  const urgent = notices.some((n) => n.severity === 'error' || n.severity === 'warn');
  return (
    <ModalPortal>
      <div
        className="toast-host"
        aria-live={urgent ? 'assertive' : 'polite'}
        onMouseEnter={() => onPauseChange(true)}
        onMouseLeave={() => onPauseChange(false)}
      >
        {notices.map((notice) => {
          const alert = notice.severity === 'error' || notice.severity === 'warn';
          return (
            <div
              key={notice.id}
              className="toast"
              data-category={notice.category}
              data-severity={notice.severity}
              role={alert ? 'alert' : 'status'}
            >
              <span className="toast-cat">{t(`notice.category.${notice.category}` as TKey)}</span>
              <div className="toast-text">
                {notice.title ? <strong className="toast-title">{notice.title}</strong> : null}
                <span>{notice.text}</span>
              </div>
              {notice.count > 1 ? (
                <span className="toast-count">{t('notice.count', { n: notice.count })}</span>
              ) : null}
              {notice.action ? (
                <button type="button" className="toast-action" onClick={() => onAction(notice)}>
                  {t(notice.action.label as TKey)}
                </button>
              ) : null}
              <button
                type="button"
                className="toast-close"
                onClick={() => onDismiss(notice.id)}
                aria-label={t('common.close')}
              >×</button>
            </div>
          );
        })}
        {hidden > 0 ? (
          <div className="toast-more">{t('notice.suppressed', { n: hidden })}</div>
        ) : null}
      </div>
    </ModalPortal>
  );
}

function normalizeProtocol(value: ProviderView['protocol']): ProviderProtocol {
  return value === 'anthropic' ? 'anthropic' : 'openai';
}

function protocolLabel(protocol: ProviderProtocol) {
  return protocol === 'anthropic' ? t('common.protocolAnthropic') : t('common.protocolOpenai');
}

/** 面板 chunk 尚未就绪时的占位：保持与真实页面同构的骨架，不闪白屏。
 *  正常路径下首屏会空闲预取，用户点开时通常已经加载完成，几乎看不到它。 */
function PanelFallback() {
  return (
    <div className="tab-body panel-fallback" aria-busy="true">
      <div className="panel-fallback-header">
        <span className="panel-fallback-bar" style={{ width: 168 }} />
        <span className="panel-fallback-bar" style={{ width: 260, height: 10 }} />
      </div>
      <div className="panel-fallback-card" />
      <div className="panel-fallback-card" style={{ height: 132 }} />
      <span className="panel-fallback-hint">{t('common.loading')}</span>
    </div>
  );
}

function Modal({
  title,
  onClose,
  children,
  closeOnOverlay = false,
}: {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  closeOnOverlay?: boolean;
}) {
  return (
    <ModalPortal>
      <div
        className="modal-overlay"
        onClick={closeOnOverlay ? onClose : undefined}
      >
        <div className="modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-head">
            <h3>{title}</h3>
            <button className="icon-btn" onClick={onClose} aria-label={t('common.close')}><X size={18} /></button>
          </div>
          <div className="modal-body">{children}</div>
        </div>
      </div>
    </ModalPortal>
  );
}

function AppMain() {
  const [tab, setTabState] = useState<NavLeaf>(initialLeaf);
  // 当前组由当前叶子反推；切叶子时把组与组内位置都写回 localStorage，切组能回到上次那一页。
  const group = navGroupOf(tab);
  const setTab = useCallback((leaf: NavLeaf) => {
    setTabState(leaf);
    const nextGroup = navGroupOf(leaf);
    try {
      localStorage.setItem(NAV_GROUP_STORAGE_KEY, nextGroup);
      localStorage.setItem(`${NAV_LEAF_STORAGE_PREFIX}${nextGroup}`, leaf);
    } catch {
      /* 隐私模式下只丢「上次位置」记忆，不影响当前导航 */
    }
  }, []);
  // 设置子页状态提升到 App：状态栏「打开日志页」等外部入口可精确跳转，切主 Tab 不丢位置。
  const [settingsTab, setSettingsTab] = useState<SettingsTab>('overview');
  /** SOUL `[[SUGT_ROUTE]]preview`：切到 Clients 后由 TakeoverProfilesSection 自动打开影响预览 */
  const [pendingTakeoverPreview, setPendingTakeoverPreview] = useState(false);
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [logs, setLogs] = useState<string[]>([]);
  // 日志内容签名：日志没变化时跳过 setLogs，避免新数组引用触发无谓重渲染。
  // 与 StatusBar 的 ref 定时器配合，双重切断「拉日志→重渲染→再拉日志」的自激循环。
  const logsSignatureRef = useRef<string>('');
  const [clients, setClients] = useState<ClientsEnvStatus | null>(null);
  const [form, setForm] = useState<ProviderForm>(emptyForm);
  const [busy, setBusy] = useState(false);
  /** 一键测试：只在途的这几行禁用操作，避免整页锁死。 */
  const [testingIds, setTestingIds] = useState<Set<string>>(new Set());
  // 主窗口 Toast 统一接入通知队列：去重合并 / 优先级截取 / TTL 清理 / 悬停暂停。 —— 孙文龙 sun
  const {
    visible: visibleNotices,
    hidden: hiddenNotices,
    push,
    dismiss,
    setPaused,
  } = useNoticeQueue({ surface: 'toast' });
  const [providerModal, setProviderModal] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ProviderView | null>(null);
  const [freeSettingsOpen, setFreeSettingsOpen] = useState(false);
  const [freeSettingsRequireAccept, setFreeSettingsRequireAccept] = useState(false);
  const [skillsAgentFilter, setSkillsAgentFilter] = useState('');
  const [trialDetail, setTrialDetail] = useState(false);
  const versionClicks = useRef(0);
  const versionClickTimer = useRef<number | null>(null);
  const {
    themeId,
    catalog: themeCatalog,
    setTheme,
    registerTheme,
    error: themeError,
    customBg,
    setCustomBackground,
    setCustomBackgroundOpacity,
    clearCustomBackground,
  } = useTheme();
  const [statusBarEnabled, setStatusBarEnabled] = useState(() => loadStatusBarEnabled());
  const [logDrawerOpen, setLogDrawerOpen] = useState(false);

  const activeProviderId = config?.active_provider_id ?? providers[0]?.id;

  // 对外签名保持 pushToast(text, kind?) 不变：所有子面板经 props 调用，调用点零改动。 —— 孙文龙 sun
  const pushToast = useCallback((text: string, kind?: ToastKind) => {
    if (kind) {
      // 显式 kind：作为 severity，统一归入 result 分类。
      push({ text, category: 'result', severity: kind });
    } else {
      // 缺省 kind：交由 messageModel.guessSeverity 依据文案自动定级。
      push(text);
    }
  }, [push]);

  // 预留结构化入口：后续可直接推送带分类 / 标题 / 动作的完整通知。 —— 孙文龙 sun
  const pushNotice = useCallback((input: NoticeInput) => {
    push(input);
  }, [push]);

  // 隐藏入口：连续点击版本号卡片 10 次，或 Ctrl+Alt+Shift+V，查看有效期详情
  const onVersionCardClick = useCallback(() => {
    versionClicks.current += 1;
    if (versionClickTimer.current) window.clearTimeout(versionClickTimer.current);
    versionClickTimer.current = window.setTimeout(() => {
      versionClicks.current = 0;
    }, 2000);
    if (versionClicks.current >= 10) {
      versionClicks.current = 0;
      setTrialDetail(true);
    }
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.altKey && e.shiftKey && (e.key === 'V' || e.key === 'v')) {
        e.preventDefault();
        setTrialDetail(true);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  // 桌面精灵右键菜单「打开设置」：跨窗口跳转到对应设置子页。
  useEffect(() => {
    const un = listen<string>('sugt://navigate-settings', (e) => {
      const target = e.payload;
      if (target === 'pet') {
        setTab('settings');
        setSettingsTab('pet');
      } else if (target === 'logs') {
        setTab('settings');
        setSettingsTab('logs');
      } else {
        setTab('settings');
        setSettingsTab('overview');
      }
    });
    return () => {
      un.then((fn) => fn()).catch(() => undefined);
    };
  }, [setTab]);

  // 首次关窗进托盘提示：Rust emit `sugt://close-to-tray`，localStorage + 进程内标记只弹一次。
  useEffect(() => {
    const tipKey = 'sugt_close_tray_tip_seen';
    let seen = false;
    try {
      seen = Boolean(localStorage.getItem(tipKey));
    } catch {
      // 隐私模式读失败：本进程仍只提示一次。
    }
    const un = listen('sugt://close-to-tray', () => {
      if (seen) return;
      seen = true;
      try {
        localStorage.setItem(tipKey, '1');
      } catch {
        // 写失败不影响提示；seen 已挡住本进程内重复。
      }
      pushToast(t('set.closeToTrayToast'), 'info');
    });
    return () => {
      un.then((fn) => fn()).catch(() => undefined);
    };
  }, [pushToast]);

  // Toast 轻量动作：复用桌面精灵「打开设置子页」的既有事件，由上方 listen('sugt://navigate-settings')
  // 统一完成 setTab/setSettingsTab；此处只发事件、仅跳转，不做任何写操作 / 破坏性操作。 —— 孙文龙 sun
  const onNoticeAction = useCallback((notice: Notice) => {
    const act = notice.action;
    if (!act || act.kind !== 'navigate') return;
    // target 形如 'settings/logs' → 取子页 'logs'，交给既有监听器映射。
    const sub = act.target?.split('/').pop() || 'overview';
    emit('sugt://navigate-settings', sub).catch(() => undefined);
  }, []);

  // 首屏各数据源独立到达即渲染，不用 Promise.all 等最慢的一项（get_status 含
  // 1200ms 流量探测、get_clients_env_status 在 Windows 跑 reg query），避免
  // "窗口出来了但页面空白，过一会才一起出现"。任何一项失败只保留旧值，
  // 全部失败时向上抛，交由调用方给出一次可见提示。
  const refreshCore = useCallback(async () => {
    const results = await Promise.allSettled([
      invoke<RuntimeStatus>('get_status').then(setStatus),
      invoke<ProviderView[]>('list_providers').then(setProviders),
      invoke<AppConfig>('get_config').then(setConfig),
      invoke<ClientsEnvStatus>('get_clients_env_status').then(setClients),
    ]);
    if (results.every((item) => item.status === 'rejected')) {
      throw new Error('状态数据全部加载失败');
    }
  }, []);

  const refreshLogs = useCallback(async () => {
    const nextLogs = await invoke<string[]>('read_logs', { lines: 200 });
    const signature = nextLogs.join('\n');
    if (signature === logsSignatureRef.current) return;
    logsSignatureRef.current = signature;
    setLogs(nextLogs);
  }, []);

  // 日志仅在「设置-日志」子页可见时高频拉取；状态栏抽屉由 StatusBar 自己的定时器负责，
  // 两处不重复轮询。抽屉打开时数据仍共用同一份 logs。
  const logsVisible = tab === 'settings' && settingsTab === 'logs';

  const refresh = useCallback(async () => {
    await refreshCore();
    if (logsVisible) await refreshLogs();
  }, [refreshCore, refreshLogs, logsVisible]);

  useEffect(() => {
    refreshCore().catch((error) => pushToast(formatInvokeError(error), 'error'));
    const timer = window.setInterval(() => refresh().catch(() => undefined), 3000);
    return () => window.clearInterval(timer);
  }, [refresh, refreshCore]);

  // 首屏渲染完成后静默预取其余面板 chunk：用户点开任何页面时模块已在内存，
  // 不用等下载/解析，做到"点击即显示"。
  useEffect(() => {
    preloadPanels();
  }, []);

  useEffect(() => {
    if (logsVisible) refreshLogs().catch(() => undefined);
  }, [logsVisible, refreshLogs]);

  useEffect(() => {
    if (!logsVisible) return undefined;
    const timer = window.setInterval(() => refreshLogs().catch(() => undefined), 2000);
    return () => window.clearInterval(timer);
  }, [logsVisible, refreshLogs]);

  const run = async (action: () => Promise<unknown>, ok?: string) => {
    setBusy(true);
    try {
      await action();
      if (ok) pushToast(ok, 'ok');
      await refresh();
    } catch (error) {
      // 失败 Toast 附轻量动作：仅跳转到「设置-日志」子页，不做任何写操作 / 破坏性操作。 —— 孙文龙 sun
      pushNotice({
        text: formatInvokeError(error),
        category: 'result',
        severity: 'error',
        action: { label: 'notice.action.viewLogs', kind: 'navigate', target: 'settings/logs' },
      });
    } finally {
      setBusy(false);
    }
  };

  const openNewProvider = () => {
    setForm(emptyForm);
    setProviderModal(true);
  };

  const openEditProvider = (provider: ProviderView) => {
    if (provider.experimental) {
      pushToast(t('models.experimentalReadonly'), 'info');
      return;
    }
    setForm({
      id: provider.id,
      name: provider.name,
      provider: provider.provider,
      base_url: provider.base_url,
      api_key: provider.api_key,
      model_name: provider.model_name,
      protocol: normalizeProtocol(provider.protocol),
      enabled: provider.enabled,
      auto_adapt_base_url: provider.auto_adapt_base_url ?? true,
      tool_text_retry_enabled: provider.tool_text_retry_enabled ?? true,
      tool_text_retry_extra:
        (provider.tool_text_retry_enabled ?? true) && provider.tool_text_retry_extra
          ? String(provider.tool_text_retry_extra)
          : '',
    });
    setProviderModal(true);
  };

  const saveProvider = async () => {
    if (form.tool_text_retry_enabled) {
      const times = Number(form.tool_text_retry_extra);
      if (!Number.isInteger(times) || times < 1 || times > 5) {
        pushToast(t('pm.toolRetryInvalid'), 'error');
        return;
      }
    }
    setBusy(true);
    try {
      await invoke('save_provider', {
        input: {
          ...form,
          tool_text_retry_extra: form.tool_text_retry_enabled
            ? Number(form.tool_text_retry_extra)
            : 0,
        },
      });
      pushToast(form.id ? t('models.toastUpdated') : t('models.toastAdded'), 'ok');
      setProviderModal(false);
      setForm(emptyForm);
      await refresh();
    } catch (error) {
      pushToast(String(error), 'error');
    } finally {
      setBusy(false);
    }
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    const id = deleteTarget.id;
    setDeleteTarget(null);
    await run(() => invoke('delete_provider', { id }), t('models.toastDeleted'));
  };

  const setProviderEnabled = async (provider: ProviderView, enabled: boolean) => {
    if (enabled && provider.experimental) {
      try {
        const cfg = await invoke<{ experimental_free_disclaimer_accepted?: boolean }>('get_config');
        if (!cfg.experimental_free_disclaimer_accepted) {
          setFreeSettingsRequireAccept(true);
          setFreeSettingsOpen(true);
          pushToast(t('freeSettings.needAcceptFirst'), 'info');
          return;
        }
      } catch {
        /* 读配置失败时仍走后端门禁 */
      }
    }
    await run(
      () => invoke('set_provider_enabled', { id: provider.id, enabled }),
      enabled ? t('models.toastEnabled') : t('models.toastDisabled'),
    );
  };

  /** 探测报告 → toast；文案复用模型配置弹窗的 pm.* key，保持两处口径一致。 */
  const probeToast = (report: ProbeReport) => {
    if (!report.ok) {
      pushToast(t('pm.probeFail', { error: report.error ?? '—' }), 'error');
      return;
    }
    const estimated = report.estimated ? t('pm.probeEstimated') : '';
    pushToast(
      report.tokensPerSec > 0
        ? t('pm.probeOk', {
          latency: report.latencyMs,
          speed: report.tokensPerSec.toFixed(1),
          estimated,
        })
        : t('pm.probeOkNoSpeed', { latency: report.latencyMs }),
      'ok',
    );
  };

  const testSingleProvider = async (provider: ProviderView) => {
    setBusy(true);
    try {
      const outcome = await invoke<ProviderTestOutcome>('test_provider', { id: provider.id });
      probeToast(outcome.report);
      await refresh();
    } catch (error) {
      pushNotice({
        text: formatInvokeError(error),
        category: 'result',
        severity: 'error',
        action: { label: 'notice.action.viewLogs', kind: 'navigate', target: 'settings/logs' },
      });
    } finally {
      setBusy(false);
    }
  };

  const testAllProviders = async () => {
    if (providers.length === 0) return;
    const queue = [...providers];
    const workers = Math.min(3, queue.length);
    setTestingIds(new Set(queue.map((provider) => provider.id)));
    let available = 0;
    let unavailable = 0;
    let latencySum = 0;
    let speedSum = 0;
    let okCount = 0;
    // 探测是真实对话（单次最长 30s）：并发上限 3，每条完成即更新该行，不等整批。
    const worker = async () => {
      while (true) {
        const provider = queue.shift();
        if (!provider) return;
        try {
          const outcome = await invoke<ProviderTestOutcome>('test_provider', { id: provider.id });
          // 后端已写回状态；本地立即同步该行，用户不用等整轮结束。
          setProviders((prev) =>
            prev.map((item) =>
              item.id === provider.id
                ? {
                  ...item,
                  status: outcome.status,
                  last_error: outcome.report.ok ? null : outcome.report.error,
                }
                : item,
            ),
          );
          if (outcome.status === 'Available') available += 1;
          if (outcome.status === 'Unavailable') unavailable += 1;
          if (outcome.report.ok) {
            latencySum += outcome.report.latencyMs;
            speedSum += outcome.report.tokensPerSec;
            okCount += 1;
          }
        } catch (error) {
          pushToast(formatInvokeError(error), 'error');
        } finally {
          setTestingIds((prev) => {
            const next = new Set(prev);
            next.delete(provider.id);
            return next;
          });
        }
      }
    };
    await Promise.all(Array.from({ length: workers }, () => worker()));
    const summary = t('models.toastTestAll', { a: available, b: unavailable });
    const metrics = okCount > 0
      ? t('models.toastTestAllProbe', {
        latency: Math.round(latencySum / okCount),
        speed: (speedSum / okCount).toFixed(1),
      })
      : '';
    pushToast(`${summary}${metrics}`, unavailable > 0 ? 'info' : 'ok');
    // 真实数据已逐行回填，末尾只做一次一致性刷新。
    await refresh();
  };

  const statusBadge = useMemo(() => {
    if (!status) return <span className="badge neutral">{t('common.loading')}</span>;
    return status.running ? <span className="badge ok">{t('common.running')}</span> : <span className="badge stop">{t('common.stopped')}</span>;
  }, [status]);

  const tabTitle: Record<NavLeaf, string> = {
    dashboard: t('tab.dashboard.title'),
    me: t('tab.me.title'),
    models: t('tab.models.title'),
    modelList: t('tab.modelList.title'),
    strategyBox: t('tab.strategyBox.title'),
    keyPool: t('tab.keyPool.title'),
    rulesInject: t('tab.rulesInject.title'),
    rulesRewrite: t('tab.rulesRewrite.title'),
    clients: t('tab.clients.title'),
    skills: t('tab.skills.title'),
    clientKeys: t('tab.clientKeys.title'),
    settings: t('tab.settings.title'),
  };

  const tabDesc: Record<NavLeaf, string> = {
    dashboard: t('tab.dashboard.desc'),
    me: t('tab.me.desc'),
    models: t('tab.models.desc'),
    modelList: t('tab.modelList.desc'),
    strategyBox: t('tab.strategyBox.desc'),
    keyPool: t('tab.keyPool.desc'),
    rulesInject: t('tab.rulesInject.desc'),
    rulesRewrite: t('tab.rulesRewrite.desc'),
    clients: t('tab.clients.desc'),
    skills: t('tab.skills.desc'),
    clientKeys: t('tab.clientKeys.desc'),
    settings: t('tab.settings.desc'),
  };

  const activeGroup = NAV_GROUPS.find((item) => item.id === group) ?? NAV_GROUPS[0];

  return (
    <main className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <BrandMark size="md" />
          <div className="brand-text">
            <h1>SUTAI</h1>
            <span>{t('brand.tagline')}</span>
          </div>
        </div>
        <div className="nav-scroll">
        {/* 门面：总览 → SOUL；分割线；工具：网关 / 规则 / 接入 / 设置 */}
        {NAV_GROUPS.map((item, index) => {
          const Icon = item.icon;
          const active = group === item.id;
          const prev = index > 0 ? NAV_GROUPS[index - 1] : null;
          const showDivider = Boolean(
            prev && FACADE_GROUP_IDS.has(prev.id) && !FACADE_GROUP_IDS.has(item.id),
          );
          const className = [
            'nav',
            active ? 'active' : '',
            item.id === 'soul' ? 'nav--soul' : '',
          ].filter(Boolean).join(' ');
          return (
            <React.Fragment key={item.id}>
              {showDivider ? <div className="nav-divider" role="separator" aria-hidden="true" /> : null}
              <button
                className={className}
                onClick={() => setTab(storedLeafFor(item.id))}
                onMouseEnter={() => {
                  if (isPreloadableGroup(item.id)) preloadPanelPreview(item.id);
                }}
                onFocus={() => {
                  if (isPreloadableGroup(item.id)) preloadPanelPreview(item.id);
                }}
              >
                <Icon size={18} />{t(item.labelKey)}
              </button>
            </React.Fragment>
          );
        })}
        </div>
        <div className="sidebar-footer">
          <ShieldCheck size={18} />
          <span>{t('about.brand')}</span>
          {/* 不占主 Tab 栏位，但保留直达设置子页的快捷入口 */}
          <div className="sidebar-footer-links">
            <button type="button" className="ghost tiny-btn" onClick={() => { setTab('settings'); setSettingsTab('logs'); }}>{t('nav.logs')}</button>
            <button type="button" className="ghost tiny-btn" onClick={() => { setTab('settings'); setSettingsTab('about'); }}>{t('nav.about')}</button>
          </div>
        </div>
      </aside>

      <section className="content">
        <header className="topbar">
          <div>
            <h2>{tabTitle[tab]}</h2>
            {tabDesc[tab] ? <p>{tabDesc[tab]}</p> : null}
          </div>
        </header>

        {/* 组内横向子 Tab：只有多叶子且非设置组（设置自带子 Tab 栏）时才出现 */}
        {activeGroup.leaves.length > 1 && activeGroup.id !== 'settings' ? (
          <div className="group-tabs" role="tablist" aria-label={t('nav.groupTabs')}>
            {activeGroup.leaves.map((leaf) => (
              <button
                key={leaf.id}
                type="button"
                role="tab"
                aria-selected={tab === leaf.id}
                className={tab === leaf.id ? 'ghost tiny-btn active' : 'ghost tiny-btn'}
                onClick={() => setTab(leaf.id)}
              >
                {t(leaf.labelKey)}
              </button>
            ))}
          </div>
        ) : null}

        {/* 面板按需加载：dashboard 全是同步组件，立刻渲染；其余面板 chunk
            通常在空闲预取阶段已就绪，点击即显示，Suspense 只兜底极端情况。 */}
        <Suspense fallback={<PanelFallback />}>
        {tab === 'dashboard' && (
          <div className="tab-body dashboard-body">
          <section className="page-grid dashboard-layout in-tab">
            <div className="dashboard-main">
              <div className="card hero-card dashboard-hero">
                <div className="hero-title">
                  <Cpu size={28} />
                  <div>
                    <h3>{t('dash.service')}</h3>
                    <p>{status?.listen_url ?? 'http://127.0.0.1:8787'}</p>
                    {status?.preferred_port != null
                      && status?.listen_port != null
                      && status.preferred_port !== status.listen_port ? (
                      <p className="hint compact">
                        {t('dash.portDrift', {
                          preferred: String(status.preferred_port),
                          listen: String(status.listen_port),
                        })}
                      </p>
                    ) : null}
                  </div>
                  {statusBadge}
                </div>
                <div className="actions dashboard-actions">
                  <button className="primary" disabled={busy || status?.running} onClick={() => run(() => invoke('start_gateway'), t('dash.toastStarted'))}><Play size={17} />{t('dash.start')}</button>
                  <button className="danger" disabled={busy || !status?.running} onClick={() => run(() => invoke('stop_gateway'), t('dash.toastStopped'))}><CircleStop size={17} />{t('dash.stop')}</button>
                  <button className="ghost" onClick={() => invoke('open_config_dir')}><FolderOpen size={17} />{t('dash.configDir')}</button>
                </div>
              </div>

              <TrafficPanel
                traffic={status?.traffic}
                running={status?.running}
              />
            </div>

            <div className="card dashboard-access-card">
              <GatewayAccessBanner
                status={status}
                providers={providers.map((p) => ({
                    id: p.id,
                    name: p.name,
                    model_name: p.model_name,
                    // 别名与文本重试配置必须原样透传：控制台切模型会走 save_provider 回写这些字段，
                    // 漏传会被后端回落成 CLIENT_MODEL_ID / 默认重试次数，静默清空用户配置（WB-001/006）。
                    model_alias: p.model_alias,
                    public_model_id: p.public_model_id,
                    enabled: p.enabled,
                    base_url: p.base_url,
                    api_key: p.api_key,
                    api_key_masked: p.api_key_masked,
                    provider: p.provider,
                    protocol: p.protocol,
                    status: p.status,
                    last_checked_at: p.last_checked_at,
                    last_error: p.last_error,
                    auto_adapt_base_url: p.auto_adapt_base_url,
                    tool_text_retry_enabled: p.tool_text_retry_enabled,
                    tool_text_retry_extra: p.tool_text_retry_extra,
                    experimental: p.experimental,
                  }))}
                busy={busy}
                onRefresh={() => refresh().catch(() => undefined)}
                onActiveProviderChange={(id) => run(() => invoke('set_active_provider', { id }), t('dash.toastSwitched'))}
                run={run}
                pushToast={pushToast}
                onOpenFreeSettings={() => {
                  setFreeSettingsRequireAccept(false);
                  setFreeSettingsOpen(true);
                }}
              />
            </div>
          </section>
          </div>
        )}

        {tab === 'me' && (
          <div className="tab-body me-tab">
            <MePanel
              onOpenClients={() => setTab('clients')}
              onOpenModels={() => setTab('models')}
              onOpenSkills={() => setTab('skills')}
              onRoute={(route) => {
                if (route === 'preview') {
                  setTab('clients');
                  setPendingTakeoverPreview(true);
                  return;
                }
                if (route === 'clients') { setTab('clients'); return; }
                if (route === 'skills') { setTab('skills'); return; }
                if (route === 'models') { setTab('models'); return; }
                if (route === 'logs') {
                  setTab('settings');
                  setSettingsTab('logs');
                  return;
                }
                if (route === 'pet') {
                  setTab('settings');
                  setSettingsTab('pet');
                  return;
                }
                setTab('settings');
                setSettingsTab('overview');
              }}
            />
          </div>
        )}

        {tab === 'rulesInject' && (
          <div className="tab-body">
            <WorkspaceRulesPanel
              rules={config?.workspace_rules}
              busy={busy}
              run={run}
              onSaved={(workspaceRules) => {
                setConfig((prev) => (prev ? { ...prev, workspace_rules: workspaceRules } : prev));
              }}
            />
          </div>
        )}

        {tab === 'rulesRewrite' && (
          <div className="tab-body">
            <RequestRulesSection pushToast={pushToast} run={run} />
          </div>
        )}

        {tab === 'settings' && (
          <div className="tab-body">
          <SettingsPanel
            config={config}
            statusListenUrl={status?.listen_url ?? 'http://127.0.0.1:8787'}
            run={run}
            onNavigate={(next) => setTab(next === 'rules' ? 'rulesInject' : next)}
            pushToast={pushToast}
            formatInvokeError={formatInvokeError}
            themeId={themeId}
            themeCatalog={themeCatalog}
            onTheme={setTheme}
            onThemeImport={registerTheme}
            themeError={themeError}
            customBg={customBg}
            onCustomBgFile={setCustomBackground}
            onCustomBgOpacity={setCustomBackgroundOpacity}
            onClearCustomBg={clearCustomBackground}
            statusBarEnabled={statusBarEnabled}
            onStatusBarEnabled={(enabled) => {
              setStatusBarEnabled(enabled);
              persistStatusBarEnabled(enabled);
              if (!enabled) setLogDrawerOpen(false);
            }}
            settingsTab={settingsTab}
            onSettingsTabChange={setSettingsTab}
            status={status}
            logs={logs}
            busy={busy}
            setBusy={setBusy}
            onRefreshLogs={() => { refreshLogs().catch(() => undefined); }}
            onVersionCardClick={onVersionCardClick}
          />
          </div>
        )}

        {tab === 'models' && (
          <div className="tab-body">
          <section className="card">
            <div className="section-title">
              <h3>{t('models.title')}</h3>
              <div className="title-actions">
                {!providers.some((p) => p.id === 'sugt-free' || p.id === 'sugt-zen-free') && (
                  <button
                    className="ghost tiny-btn"
                    disabled={busy}
                    title={t('models.restoreZenTitle')}
                    onClick={() =>
                      run(async () => {
                        const list = await invoke<ProviderView[]>('restore_experimental_free');
                        setProviders(list);
                      }, t('models.toastZenRestored'))
                    }
                  >
                    {t('models.restoreZen')}
                  </button>
                )}
                {testingIds.size > 0 && (
                  <span className="hint compact">
                    {t('models.testAllProgress', {
                      done: providers.length - testingIds.size,
                      total: providers.length,
                    })}
                  </span>
                )}
                <button className="ghost tiny-btn" disabled={busy || testingIds.size > 0 || providers.length === 0} onClick={testAllProviders}>{t('models.testAll')}</button>
                <button className="primary tiny-btn" disabled={busy} onClick={openNewProvider}><Plus size={14} />{t('models.add')}</button>
              </div>
            </div>
            {providers.length === 0 && <p className="hint">{t('models.empty')}</p>}
            <div className="provider-list-scroll">
              {providers.map((provider) => {
                const isActive = provider.id === activeProviderId;
                // 一键测试期间只锁正在测的这一行，其余行照常操作。
                const testingThisRow = busy || testingIds.has(provider.id);
                const className = `provider${isActive ? ' active' : ''}${provider.enabled ? '' : ' disabled'}`;
                return (
                  <div className={className} key={provider.id}>
                    <div
                      className="provider-main"
                      onClick={() => openEditProvider(provider)}
                      style={provider.experimental ? { cursor: 'default' } : undefined}
                    >
                      <div className="provider-title">
                        <strong>{provider.name}</strong>
                        {provider.experimental && <span className="badge inline" title={t('models.experimentalTitle')}>{t('models.experimental')}</span>}
                        {isActive && <span className="badge ok inline">{t('common.default')}</span>}
                        <span className={provider.enabled ? 'badge neutral inline' : 'badge stop inline'}>{provider.enabled ? t('common.enabled') : t('common.disabled')}</span>
                      </div>
                      <span className="provider-sub">{protocolLabel(normalizeProtocol(provider.protocol))} · {provider.model_name}</span>
                      {provider.experimental ? (
                        <span className="hint compact">{t('models.freeRowHint')}</span>
                      ) : (
                        <code className="provider-url" title={provider.base_url}>{provider.base_url}</code>
                      )}
                      {provider.last_error && <span className="error-text">{provider.last_error}</span>}
                    </div>
                    <div className="provider-actions">
                      {testingIds.has(provider.id) ? (
                        <span className="spin" title={t('modelList.probing')}>
                          <Activity size={18} aria-label={t('modelList.probing')} />
                        </span>
                      ) : provider.status === 'Available' ? <CheckCircle2 className="ok-text" size={18} /> : provider.status === 'Unavailable' ? <XCircle className="bad-text" size={18} /> : <span className="dot" />}
                      {provider.experimental && (
                        <button
                          className="tiny"
                          disabled={testingThisRow}
                          onClick={() => {
                            setFreeSettingsRequireAccept(false);
                            setFreeSettingsOpen(true);
                          }}
                        >
                          {t('freeSettings.open')}
                        </button>
                      )}
                      <button className="tiny" disabled={testingThisRow || isActive || !provider.enabled} onClick={() => run(() => invoke('set_active_provider', { id: provider.id }), t('models.toastDefault'))}>{isActive ? t('models.isDefault') : t('models.setDefault')}</button>
                      <button className="tiny" disabled={testingThisRow} onClick={() => setProviderEnabled(provider, !provider.enabled)}>{provider.enabled ? t('common.disabled') : t('common.enabled')}</button>
                      <button className="tiny" disabled={testingThisRow} onClick={() => void testSingleProvider(provider)}>{t('common.test')}</button>
                      <button className="tiny danger-link" disabled={testingThisRow} onClick={() => setDeleteTarget(provider)}><Trash2 size={14} /></button>
                    </div>
                  </div>
                );
              })}
            </div>
          </section>
          </div>
        )}

        {tab === 'modelList' && (
          <div className="tab-body">
            <AccessInfoCard pushToast={pushToast} />
            <ModelListPanel
              providers={providers}
              pushToast={pushToast}
              run={run}
            />
          </div>
        )}

        {tab === 'strategyBox' && (
          <div className="tab-body">
            <StrategyBoxPanel
              providers={providers}
              pushToast={pushToast}
              run={run}
            />
          </div>
        )}

        {tab === 'keyPool' && (
          <div className="tab-body">
            <ApiKeyPoolPanel
              providers={providers}
              pushToast={pushToast}
              run={run}
            />
          </div>
        )}

        {tab === 'clients' && (
          <div className="tab-body tab-body-fill">
            <div className="clients-layout">
              <div className="card">
                <TakeoverProfilesSection
                  busy={busy}
                  run={run}
                  pushToast={pushToast}
                  onClientsChange={setClients}
                  openPreviewRequest={pendingTakeoverPreview}
                  onOpenPreviewRequestHandled={() => setPendingTakeoverPreview(false)}
                  onOpenSkills={(agentId) => {
                    setSkillsAgentFilter(agentId);
                    setTab('skills');
                  }}
                />
              </div>
            </div>
          </div>
        )}

        {tab === 'skills' && (
          <div className="tab-body tab-body-fill">
            <SkillsPanel
              busy={busy}
              setBusy={setBusy}
              pushToast={pushToast}
              formatInvokeError={formatInvokeError}
              agentFilter={skillsAgentFilter}
              onAgentFilterChange={setSkillsAgentFilter}
            />
          </div>
        )}

        {tab === 'clientKeys' && (
          <div className="tab-body">
            <ClientKeyPanel pushToast={pushToast} />
          </div>
        )}
        </Suspense>

      </section>

      {providerModal && (
        <ProviderModal
          form={form}
          busy={busy}
          isEdit={Boolean(form.id)}
          onChange={setForm}
          onClose={() => setProviderModal(false)}
          onSave={saveProvider}
          onError={(msg) => pushToast(msg, 'error')}
          onInfo={(msg) => pushToast(msg, 'ok')}
        />
      )}

      {deleteTarget && (
        <Modal title={t('delete.title')} onClose={() => setDeleteTarget(null)} closeOnOverlay>
          <p>{t('delete.body', { name: deleteTarget.name })}</p>
          {deleteTarget.experimental && (
            <p className="hint compact">{t('delete.zenHint')}</p>
          )}
          <div className="modal-actions">
            <button className="ghost" onClick={() => setDeleteTarget(null)}>{t('common.cancel')}</button>
            <button className="danger" disabled={busy} onClick={confirmDelete}>{t('common.delete')}</button>
          </div>
        </Modal>
      )}

      <FreeChannelSettingsModal
        open={freeSettingsOpen}
        busy={busy}
        requireAccept={freeSettingsRequireAccept}
        currentRoute={providers.find((p) => p.experimental)?.model_name}
        onClose={() => {
          setFreeSettingsOpen(false);
          setFreeSettingsRequireAccept(false);
        }}
        onSaved={() => {
          void refresh();
        }}
        run={run}
        pushToast={pushToast}
      />

      {trialDetail && (
        <Modal title={t('about.trialDetail')} onClose={() => setTrialDetail(false)} closeOnOverlay>
          <div className="trial-detail-grid">
            <div><strong>{t('about.edition')}</strong><span>{editionLabel(status?.trial)}</span></div>
            <div><strong>{t('about.product')}</strong><span>{status?.trial.product_label ?? '-'}</span></div>
            <div><strong>{t('about.buildId')}</strong><span>{status?.trial.build_id ?? '-'}</span></div>
            <div><strong>{t('about.status')}</strong><span>{status?.trial.status ?? '-'}</span></div>
            <div><strong>{t('about.valid')}</strong><span>{status?.trial.valid ? 'true' : 'false'}</span></div>
            <div><strong>{t('about.trialEnabled')}</strong><span>{status?.trial.trial_enabled ? 'true' : 'false'}</span></div>
            <div><strong>{t('about.expires')}</strong><span>{status?.trial.expires_date ?? '-'}</span></div>
            <div><strong>{t('about.daysLeft')}</strong><span>{status?.trial.days_remaining ?? '-'}</span></div>
            <div className="trial-detail-msg"><strong>{t('about.message')}</strong><span>{status?.trial.message ?? '-'}</span></div>
          </div>
        </Modal>
      )}

      {statusBarEnabled ? (
        <StatusBar
          running={status?.running}
          listenUrl={status?.listen_url}
          activeProvider={status?.active_provider ?? status?.last_proxy_provider}
          traffic={status?.traffic}
          logs={logs}
          drawerOpen={logDrawerOpen}
          onToggleDrawer={() => setLogDrawerOpen((v) => !v)}
          onOpenLogsTab={() => {
            setLogDrawerOpen(false);
            setTab('settings');
            setSettingsTab('logs');
          }}
          onRefreshLogs={() => {
            refreshLogs().catch(() => undefined);
          }}
        />
      ) : null}

      <ToastHost
        notices={visibleNotices}
        hidden={hiddenNotices}
        onDismiss={dismiss}
        onAction={onNoticeAction}
        onPauseChange={setPaused}
      />
    </main>
  );
}

// 浏览器模式登录门控：桌面端直接渲染管理界面，浏览器端需先用 gateway_client_api_key 换 token。
function App() {
  const [ready, setReady] = useState(() => !isBrowserMode() || Boolean(getSessionToken()));
  const [error, setError] = useState<string | null>(null);
  const [loggingIn, setLoggingIn] = useState(false);
  const [keyDraft, setKeyDraft] = useState('');

  const login = useCallback(async (key: string) => {
    const trimmed = key.trim();
    if (!trimmed) {
      setError(t('webui.keyRequired'));
      return;
    }
    setLoggingIn(true);
    setError(null);
    try {
      const res = await fetch('/api/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ key: trimmed }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = (await res.json()) as { token?: string; error?: string };
      if (!data.token) throw new Error(data.error || t('webui.loginFailed'));
      setSessionToken(data.token);
      // 令牌已换到 sessionStorage，输入框里的明文 key 立即清掉。
      setKeyDraft('');
      setReady(true);
    } catch (e) {
      setError(`${t('webui.loginFailed')}：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setLoggingIn(false);
    }
  }, []);

  // invoke 收到 401 会清 token 并抛此事件：回到登录态（仅浏览器模式）。
  useEffect(() => {
    if (!isBrowserMode()) return undefined;
    const off = listen(AUTH_EXPIRED_EVENT, () => {
      setError(t('webui.authExpired'));
      setReady(false);
    });
    return () => {
      void off.then((fn) => fn()).catch(() => undefined);
    };
  }, []);

  if (!ready) {
    return (
      <main className="browser-only webui-login">
        <h2>{t('webui.loginTitle')}</h2>
        <p>{t('webui.loginDesc')}</p>
        <form
          className="webui-login-form"
          onSubmit={(event) => {
            event.preventDefault();
            void login(keyDraft);
          }}
        >
          <input
            type="password"
            autoFocus
            autoComplete="current-password"
            aria-label={t('webui.keyPlaceholder')}
            placeholder={t('webui.keyPlaceholder')}
            value={keyDraft}
            onChange={(event) => setKeyDraft(event.target.value)}
            disabled={loggingIn}
          />
          <button type="submit" disabled={loggingIn}>
            {loggingIn ? t('webui.loggingIn') : error ? t('webui.retryButton') : t('webui.loginButton')}
          </button>
        </form>
        {error ? <p className="error">{error}</p> : null}
        <p className="hint">{t('webui.tip')}</p>
      </main>
    );
  }

  return <AppMain />;
}

createRoot(document.getElementById('root')!).render(<App />);
