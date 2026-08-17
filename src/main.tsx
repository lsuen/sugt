import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import {
  Activity, Bot, CheckCircle2, CircleStop, Cpu, FileText, FolderOpen, Info,
  Play, Plus, RefreshCw, ShieldCheck, Sparkles, Trash2, Users, X, XCircle, Settings2,
} from 'lucide-react';
import './styles.css';
import { GatewayAccessBanner } from './GatewayAccessBanner';
import { SettingsPanel } from './SettingsPanel';
import { CURRENT_VERSION, RELEASE_NOTES } from './release-notes';
import { SkillsPanel } from './SkillsPanel';
import { TrafficPanel } from './TrafficPanel';
import { ProviderModal } from './ProviderModal';
import { TakeoverProfilesSection } from './TakeoverProfilesSection';
import type { ProviderForm } from './providerPresets';

type Tab = 'dashboard' | 'models' | 'clients' | 'skills' | 'settings' | 'logs' | 'about';
type ProviderStatus = 'Unknown' | 'Available' | 'Unavailable';
type ProviderProtocol = 'openai' | 'anthropic';

type RuntimeStatus = {
  running: boolean;
  listen_url: string;
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
  last_proxy_failover?: boolean;
  last_proxy_at?: string | null;
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
  protocol: ProviderProtocol | 'open_ai';
  enabled: boolean;
  status: ProviderStatus;
  last_checked_at?: string | null;
  last_error?: string | null;
  auto_adapt_base_url?: boolean;
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
  port_fallback_enabled?: boolean;
  gateway_watchdog_enabled?: boolean;
  allow_lan_access?: boolean;
  port_fallback_ports?: number[];
  providers: unknown[];
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
};

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

function formatInvokeError(error: unknown): string {
  const text = String(error);
  const marker = 'gateway_not_running:';
  const idx = text.indexOf(marker);
  if (idx >= 0) return text.slice(idx + marker.length);
  return text;
}

type ParsedLogLine = {
  key: string;
  time?: string;
  level: string;
  message: string;
  target?: string;
};

function parseLogLine(line: string, index: number): ParsedLogLine {
  try {
    const data = JSON.parse(line) as {
      timestamp?: string;
      level?: string;
      message?: string;
      target?: string;
      fields?: Record<string, unknown>;
    };
    const fields = data.fields ?? {};
    const base = String(fields.message ?? data.message ?? line);
    const extras = ['client', 'provider', 'model', 'path', 'mode', 'latency_ms', 'status', 'failover', 'error', 'reason']
      .filter((key) => fields[key] !== undefined && fields[key] !== null && fields[key] !== '')
      .map((key) => `${key}=${String(fields[key])}`);
    const message = extras.length ? `${base} · ${extras.join(' · ')}` : base;
    let time: string | undefined;
    if (data.timestamp) {
      const date = new Date(data.timestamp);
      if (!Number.isNaN(date.getTime())) {
        time = date.toLocaleTimeString('zh-CN', { hour12: false });
      }
    }
    return {
      key: `${index}-${data.timestamp ?? line.slice(0, 24)}`,
      time,
      level: (data.level ?? 'INFO').toUpperCase(),
      message,
      target: data.target,
    };
  } catch {
    return { key: `${index}-raw`, level: 'LOG', message: line };
  }
}

function LogViewer({ lines }: { lines: string[] }) {
  const endRef = useRef<HTMLDivElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  const entries = useMemo(() => lines.map((line, index) => parseLogLine(line, index)), [lines]);

  useEffect(() => {
    if (!autoScroll) return;
    endRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [entries, autoScroll]);

  const onScroll = () => {
    const el = viewportRef.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
    setAutoScroll(atBottom);
  };

  if (!lines.length) {
    return <div className="log-empty">暂无日志，启动网关后这里会显示运行记录</div>;
  }

  return (
    <>
      <div className="log-toolbar">
        <span className="hint compact">共 {lines.length} 行 · 最新在底部</span>
        <button
          type="button"
          className={`ghost tiny-btn${autoScroll ? ' active' : ''}`}
          onClick={() => {
            setAutoScroll(true);
            endRef.current?.scrollIntoView({ behavior: 'smooth' });
          }}
        >
          {autoScroll ? '跟随最新' : '滚到底部'}
        </button>
      </div>
      <div className="log-viewport" ref={viewportRef} onScroll={onScroll}>
        {entries.map((entry) => (
          <div className={`log-line level-${entry.level.toLowerCase()}`} key={entry.key}>
            {entry.time && <span className="log-time">{entry.time}</span>}
            <span className={`log-level lv-${entry.level.toLowerCase()}`}>{entry.level}</span>
            <span className="log-msg" title={entry.target}>{entry.message}</span>
          </div>
        ))}
        <div ref={endRef} />
      </div>
    </>
  );
}

function isErrorLike(text: string): boolean {
  const lower = text.toLowerCase();
  return lower.includes('error') || text.includes('失败') || lower.includes('invalid') || text.includes('不可用');
}

type ToastKind = 'ok' | 'error' | 'info';

type ToastItem = {
  id: number;
  text: string;
  kind: ToastKind;
};

function ToastHost({ toasts, onDismiss }: { toasts: ToastItem[]; onDismiss: (id: number) => void }) {
  if (!toasts.length) return null;
  return (
    <div className="toast-host" aria-live="polite">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast toast-${toast.kind}`} role="status">
          <span className="toast-text">{toast.text}</span>
          <button type="button" className="toast-close" onClick={() => onDismiss(toast.id)} aria-label="关闭">×</button>
        </div>
      ))}
    </div>
  );
}

function normalizeProtocol(value: ProviderView['protocol']): ProviderProtocol {
  return value === 'anthropic' ? 'anthropic' : 'openai';
}

function protocolLabel(protocol: ProviderProtocol) {
  return protocol === 'anthropic' ? 'Anthropic 原生' : 'OpenAI 兼容';
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
    <div
      className="modal-overlay"
      onClick={closeOnOverlay ? onClose : undefined}
    >
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{title}</h3>
          <button className="icon-btn" onClick={onClose} aria-label="关闭"><X size={18} /></button>
        </div>
        <div className="modal-body">{children}</div>
      </div>
    </div>
  );
}

function App() {
  const [tab, setTab] = useState<Tab>('dashboard');
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [logs, setLogs] = useState<string[]>([]);
  const [clients, setClients] = useState<ClientsEnvStatus | null>(null);
  const [form, setForm] = useState<ProviderForm>(emptyForm);
  const [busy, setBusy] = useState(false);
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const toastSeq = useRef(0);
  const [providerModal, setProviderModal] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ProviderView | null>(null);
  const [skillsAgentFilter, setSkillsAgentFilter] = useState('');

  const activeProviderId = config?.active_provider_id ?? providers[0]?.id;

  const pushToast = useCallback((text: string, kind?: ToastKind) => {
    const trimmed = text.trim();
    if (!trimmed) return;
    const resolved = kind ?? (isErrorLike(trimmed) ? 'error' : 'ok');
    const id = ++toastSeq.current;
    setToasts((prev) => [...prev.slice(-2), { id, text: trimmed, kind: resolved }]);
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((item) => item.id !== id));
    }, 4200);
  }, []);

  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((item) => item.id !== id));
  }, []);

  const refreshCore = useCallback(async () => {
    const [nextStatus, nextProviders, nextConfig, nextClients] = await Promise.all([
      invoke<RuntimeStatus>('get_status'),
      invoke<ProviderView[]>('list_providers'),
      invoke<AppConfig>('get_config'),
      invoke<ClientsEnvStatus>('get_clients_env_status'),
    ]);
    setStatus(nextStatus);
    setProviders(nextProviders);
    setConfig(nextConfig);
    setClients(nextClients);
  }, []);

  const refreshLogs = useCallback(async () => {
    const nextLogs = await invoke<string[]>('read_logs', { lines: 200 });
    setLogs(nextLogs);
  }, []);

  const refresh = useCallback(async () => {
    await refreshCore();
    if (tab === 'logs') await refreshLogs();
  }, [refreshCore, refreshLogs, tab]);

  useEffect(() => {
    refreshCore().catch((error) => pushToast(formatInvokeError(error), 'error'));
    const timer = window.setInterval(() => refresh().catch(() => undefined), 3000);
    return () => window.clearInterval(timer);
  }, [refresh, refreshCore]);

  useEffect(() => {
    if (tab === 'logs') refreshLogs().catch(() => undefined);
  }, [tab, refreshLogs]);

  useEffect(() => {
    if (tab !== 'logs') return undefined;
    const timer = window.setInterval(() => refreshLogs().catch(() => undefined), 2000);
    return () => window.clearInterval(timer);
  }, [tab, refreshLogs]);

  const run = async (action: () => Promise<unknown>, ok?: string) => {
    setBusy(true);
    try {
      await action();
      if (ok) pushToast(ok, 'ok');
      await refresh();
    } catch (error) {
      pushToast(formatInvokeError(error), 'error');
    } finally {
      setBusy(false);
    }
  };

  const openNewProvider = () => {
    setForm(emptyForm);
    setProviderModal(true);
  };

  const openEditProvider = (provider: ProviderView) => {
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
    });
    setProviderModal(true);
  };

  const saveProvider = async () => {
    setBusy(true);
    try {
      await invoke('save_provider', { input: form });
      pushToast(form.id ? '模型配置已更新' : '模型配置已添加', 'ok');
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
    await run(() => invoke('delete_provider', { id }), '模型配置已删除');
  };

  const setProviderEnabled = async (provider: ProviderView, enabled: boolean) => {
    await run(() => invoke('set_provider_enabled', { id: provider.id, enabled }), enabled ? '模型已启用' : '模型已禁用');
  };

  const testAllProviders = async () => {
    setBusy(true);
    let available = 0;
    let unavailable = 0;

    try {
      for (const provider of providers) {
        const result = await invoke<ProviderStatus>('test_provider', { id: provider.id });
        if (result === 'Available') available += 1;
        if (result === 'Unavailable') unavailable += 1;
      }
      pushToast(`一键测试完成：${available} 个可用，${unavailable} 个不可用`, unavailable > 0 ? 'info' : 'ok');
      await refresh();
    } catch (error) {
      pushToast(String(error), 'error');
    } finally {
      setBusy(false);
    }
  };

  const statusBadge = useMemo(() => {
    if (!status) return <span className="badge neutral">加载中</span>;
    return status.running ? <span className="badge ok">运行中</span> : <span className="badge stop">已停止</span>;
  }, [status]);

  const tabTitle = {
    dashboard: '控制台',
    models: '模型配置',
    clients: '客户端',
    skills: '技能',
    settings: '设置',
    logs: '运行日志',
    about: '关于 SUTAI',
  }[tab];

  const tabDesc = {
    dashboard: '启停本地网关，查看当前模型与流量',
    models: '配置上游 API、协议与实验免费通道',
    clients: '发现本机工具、一键接管与高级配置',
    skills: '发现、入库并挂到各客户端',
    settings: '启动、守护、代理与实验功能',
    logs: '网关实时日志与调试信息',
    about: '版本、更新说明与项目信息',
  }[tab];

  if (!isTauriRuntime()) {
    return (
      <main className="browser-only">
        <h2>SUTAI 需在 Tauri 窗口中运行</h2>
        <p>当前在普通浏览器中打开，没有 Tauri IPC，因此会出现 <code>invoke</code> 报错。</p>
        <p>请在项目根目录执行：</p>
        <pre>npm run tauri dev</pre>
        <p className="hint">不要单独运行 <code>npm run dev</code> 后手动打开 http://127.0.0.1:1420。</p>
      </main>
    );
  }

  return (
    <main className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-logo">SA</div>
          <div>
            <h1>SUTAI</h1>
            <span>朴素的 AI 时代中控台</span>
          </div>
        </div>
        <button className={tab === 'dashboard' ? 'nav active' : 'nav'} onClick={() => setTab('dashboard')}><Activity size={18} />控制台</button>
        <button className={tab === 'models' ? 'nav active' : 'nav'} onClick={() => setTab('models')}><Bot size={18} />模型配置</button>
        <button className={tab === 'clients' ? 'nav active' : 'nav'} onClick={() => setTab('clients')}><Users size={18} />客户端</button>
        <button className={tab === 'skills' ? 'nav active' : 'nav'} onClick={() => setTab('skills')}><Sparkles size={18} />技能</button>
        <button className={tab === 'settings' ? 'nav active' : 'nav'} onClick={() => setTab('settings')}><Settings2 size={18} />设置</button>
        <button className={tab === 'logs' ? 'nav active' : 'nav'} onClick={() => setTab('logs')}><FileText size={18} />日志</button>
        <button className={tab === 'about' ? 'nav active' : 'nav'} onClick={() => setTab('about')}><Info size={18} />关于</button>
        <div className="sidebar-footer">
          <ShieldCheck size={18} />
          <span>异常设计</span>
        </div>
      </aside>

      <section className="content">
        <header className="topbar">
          <div>
            <h2>{tabTitle}</h2>
            <p>{tabDesc}</p>
          </div>
        </header>

        {tab === 'dashboard' && (
          <div className="tab-body dashboard-body">
          <section className="page-grid dashboard-layout in-tab">
            <div className="dashboard-main">
              <div className="card hero-card dashboard-hero">
                <div className="hero-title">
                  <Cpu size={28} />
                  <div>
                    <h3>服务状态</h3>
                    <p>{status?.listen_url ?? 'http://127.0.0.1:8787'}</p>
                  </div>
                  {statusBadge}
                </div>
                {status?.last_proxy_provider && (
                  <p className="hint compact hero-proxy-hint">
                    最近命中 {status.last_proxy_provider}
                    {status.last_proxy_failover ? '（故障转移）' : ''}
                    · {status.last_proxy_path ?? '-'}
                    {status.last_proxy_at ? ` · ${new Date(status.last_proxy_at).toLocaleString()}` : ''}
                  </p>
                )}
                <div className="actions dashboard-actions">
                  <button className="primary" disabled={busy || status?.running} onClick={() => run(() => invoke('start_gateway'), '服务已启动')}><Play size={17} />启动</button>
                  <button className="danger" disabled={busy || !status?.running} onClick={() => run(() => invoke('stop_gateway'), '服务已停止')}><CircleStop size={17} />停止</button>
                  <button className="ghost" onClick={() => invoke('open_config_dir')}><FolderOpen size={17} />配置目录</button>
                </div>
              </div>

              <TrafficPanel
                traffic={status?.traffic}
                running={status?.running}
                lastProvider={status?.last_proxy_provider}
                lastPath={status?.last_proxy_path}
              />
            </div>

            <div className="card dashboard-access-card">
              <GatewayAccessBanner
                status={status}
                providers={providers.map((p) => ({
                    id: p.id,
                    name: p.name,
                    model_name: p.model_name,
                    enabled: p.enabled,
                    base_url: p.base_url,
                    api_key: p.api_key,
                    api_key_masked: p.api_key_masked,
                    provider: p.provider,
                    protocol: p.protocol,
                  }))}
                busy={busy}
                onRefresh={() => refresh().catch(() => undefined)}
                onActiveProviderChange={(id) => run(() => invoke('set_active_provider', { id }), '已切换上游')}
                run={run}
                pushToast={pushToast}
              />
            </div>
          </section>
          </div>
        )}

        {tab === 'settings' && (
          <div className="tab-body">
          <SettingsPanel
            config={config}
            statusListenUrl={status?.listen_url ?? 'http://127.0.0.1:8787'}
            run={run}
            onNavigate={setTab}
            formatInvokeError={formatInvokeError}
          />
          </div>
        )}

        {tab === 'models' && (
          <div className="tab-body">
          <section className="card">
            <div className="section-title">
              <h3>模型列表</h3>
              <div className="title-actions">
                {!providers.some((p) => p.id === 'sugt-zen-free') && (
                  <button
                    className="ghost tiny-btn"
                    disabled={busy}
                    title="恢复实验免费通道（不稳定，可删）"
                    onClick={() =>
                      run(async () => {
                        const list = await invoke<ProviderView[]>('restore_experimental_zen');
                        setProviders(list);
                      }, '已恢复实验免费源')
                    }
                  >
                    恢复实验源
                  </button>
                )}
                <button className="ghost tiny-btn" disabled={busy || providers.length === 0} onClick={testAllProviders}>一键测试</button>
                <button className="primary tiny-btn" disabled={busy} onClick={openNewProvider}><Plus size={14} />添加模型</button>
              </div>
            </div>
            {providers.length === 0 && <p className="hint">暂无配置。点击「添加模型」并从下拉选择服务商预设。</p>}
            <div className="provider-list-scroll">
              {providers.map((provider) => {
                const isActive = provider.id === activeProviderId;
                const className = `provider${isActive ? ' active' : ''}${provider.enabled ? '' : ' disabled'}`;
                return (
                  <div className={className} key={provider.id}>
                    <div className="provider-main" onClick={() => openEditProvider(provider)}>
                      <div className="provider-title">
                        <strong>{provider.name}</strong>
                        {provider.experimental && <span className="badge inline" title="免费额度不稳定，可删；失效后可改 API Key / 地址继续用">实验</span>}
                        {isActive && <span className="badge ok inline">默认</span>}
                        <span className={provider.enabled ? 'badge neutral inline' : 'badge stop inline'}>{provider.enabled ? '启用' : '禁用'}</span>
                      </div>
                      <span className="provider-sub">{protocolLabel(normalizeProtocol(provider.protocol))} · {provider.model_name}</span>
                      {provider.experimental && (
                        <span className="hint compact">免费通道不稳定；失效后可编辑改 Key/地址，或删除后点「恢复实验源」</span>
                      )}
                      <code className="provider-url" title={provider.base_url}>{provider.base_url}</code>
                      {provider.last_error && <span className="error-text">{provider.last_error}</span>}
                    </div>
                    <div className="provider-actions">
                      {provider.status === 'Available' ? <CheckCircle2 className="ok-text" size={18} /> : provider.status === 'Unavailable' ? <XCircle className="bad-text" size={18} /> : <span className="dot" />}
                      <button className="tiny" disabled={busy || isActive || !provider.enabled} onClick={() => run(() => invoke('set_active_provider', { id: provider.id }), '已设为默认')}>{isActive ? '已默认' : '设为默认'}</button>
                      <button className="tiny" disabled={busy} onClick={() => setProviderEnabled(provider, !provider.enabled)}>{provider.enabled ? '禁用' : '启用'}</button>
                      <button className="tiny" disabled={busy} onClick={() => run(() => invoke('test_provider', { id: provider.id }), '测试完成')}>测试</button>
                      <button className="tiny danger-link" disabled={busy} onClick={() => setDeleteTarget(provider)}><Trash2 size={14} /></button>
                    </div>
                  </div>
                );
              })}
            </div>
          </section>
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

        {tab === 'logs' && (
          <div className="tab-body tab-body-fill">
          <section className="card log-card-full">
            <div className="section-title">
              <h3>运行日志</h3>
              <button className="ghost tiny-btn" disabled={busy} onClick={() => refreshLogs()}><RefreshCw size={14} />刷新</button>
            </div>
            <LogViewer lines={logs} />
          </section>
          </div>
        )}

        {tab === 'about' && (
          <div className="tab-body">
          <section className="card about-card">
            <div className="about-logo">SUTAI</div>
            <h3>SUTAI · 朴素的 AI 时代中控台</h3>
            {/* <p className="hint compact about-lead">
              在本机统一接入大模型服务，方便 agent工具调用。
              <br />
              支持模型配置、一键接管、技能发现与挂载。
            </p> */}
            <p>作者：孙文龙 · 异常设计</p>
            <p className={status?.trial.valid ? 'trial-note' : 'trial-note expired'}>
              {status?.trial.message ?? '当前为开发运行，无试用期限制'}
            </p>
            <div className="about-grid">
              <div><strong>版本</strong><span>v{CURRENT_VERSION}</span></div>
              <div><strong>产品形态</strong><span>{status?.trial.product_label ?? '全功能版'}</span></div>
              <div><strong>授权</strong><span>{status?.trial.message ?? '开发模式'}</span></div>
              <div><strong>配置目录</strong><span>{status?.config_dir}</span></div>
              <div><strong>日志文件</strong><span>{status?.log_file}</span></div>
              <div><strong>本机接入</strong><span>{status?.listen_url ?? 'http://127.0.0.1:8787'}</span></div>
            </div>
            <div className="release-notes-frame">
              <div className="release-notes-head">
                <strong>本版本更新</strong>
                <span className="hint compact">v{CURRENT_VERSION}</span>
              </div>
              <ul className="release-notes-list">
                {RELEASE_NOTES.map((note) => (
                  <li key={note.text} className={`release-note ${note.type}`}>
                    <span className="release-tag">{note.type === 'feat' ? '新功能' : '修复'}</span>
                    {note.text}
                  </li>
                ))}
              </ul>
            </div>
          </section>
          </div>
        )}
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
        <Modal title="确认删除" onClose={() => setDeleteTarget(null)} closeOnOverlay>
          <p>确定删除模型配置「{deleteTarget.name}」？</p>
          {deleteTarget.experimental && (
            <p className="hint compact">这是实验免费源，删除后不会自动再添加；需要时可点「恢复实验源」。</p>
          )}
          <div className="modal-actions">
            <button className="ghost" onClick={() => setDeleteTarget(null)}>取消</button>
            <button className="danger" disabled={busy} onClick={confirmDelete}>删除</button>
          </div>
        </Modal>
      )}

      <ToastHost toasts={toasts} onDismiss={dismissToast} />
    </main>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
