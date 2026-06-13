import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import {
  Activity, Bot, CheckCircle2, CircleStop, Cpu, FileText, FolderOpen, Info,
  Play, PlugZap, Plus, RefreshCw, Save, ShieldCheck, Trash2, Users, Wrench, X, XCircle,
} from 'lucide-react';
import './styles.css';
import { CURRENT_VERSION, RELEASE_NOTES } from './release-notes';
import { StoreClientsPanel } from './store/StoreClientsPanel';
import { TrafficPanel } from './TrafficPanel';
import { PROVIDER_PRESETS, type ProviderForm } from './providerPresets';

type Tab = 'dashboard' | 'models' | 'clients' | 'logs' | 'about';
type ProviderStatus = 'Unknown' | 'Available' | 'Unavailable';
type ProviderProtocol = 'openai' | 'anthropic';

type RuntimeStatus = {
  running: boolean;
  listen_url: string;
  active_model?: string | null;
  active_provider?: string | null;
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
  api_key_masked: string;
  model_name: string;
  protocol: ProviderProtocol | 'open_ai';
  enabled: boolean;
  status: ProviderStatus;
  last_checked_at?: string | null;
  last_error?: string | null;
};

type AppConfig = {
  host: string;
  port: number;
  failover_enabled: boolean;
  active_provider_id?: string | null;
  autostart: boolean;
  autostart_gateway: boolean;
  quit_behavior: QuitBehavior;
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
  claude: ClientEnvStatus;
  codex: ClientEnvStatus;
  has_issues?: boolean;
};

const emptyForm: ProviderForm = {
  name: '',
  provider: 'ModelScope',
  base_url: '',
  api_key: '',
  model_name: '',
  protocol: 'openai',
  enabled: true,
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
      fields?: { message?: string };
    };
    const message = data.fields?.message ?? data.message ?? line;
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
      message: String(message),
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

function Modal({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  return (
    <div className="modal-overlay" onClick={onClose}>
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

function ClientStatusBadge({ client, listenUrl }: { client: ClientEnvStatus; listenUrl: string }) {
  const [hover, setHover] = useState(false);

  return (
    <span
      className="badge-wrap"
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      <span className={`badge popover-trigger ${client.configured ? 'ok' : 'stop'}`}>
        {client.configured ? '已接管' : '未接管'}
      </span>
      {hover && (
        <div className="badge-popover" role="tooltip">
          <div className="badge-popover-title">{client.client}</div>
          {client.issues && client.issues.length > 0 && (
            <ul className="badge-popover-issues">
              {client.issues.map((issue) => <li key={issue}>{issue}</li>)}
            </ul>
          )}
          {!client.configured && client.missing && client.missing.length > 0 && (
            <p className="hint compact">待写入：{client.missing.join('、')}</p>
          )}
          {client.variables && (
            <div className="env-vars compact">
              {Object.entries(client.variables).map(([name, value]) => (
                <div className="env-row" key={name}>
                  <span>{name}</span>
                  <code>{value || '（未设置）'}</code>
                </div>
              ))}
            </div>
          )}
          <p className="hint compact popover-foot">网关 {listenUrl} · 客户端页可「关闭接管」恢复</p>
        </div>
      )}
    </span>
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

  const clientsHint = useMemo(() => {
    const parts = ['写入用户环境变量，仅对新打开的终端生效。悬停「已接管/未接管」标签可查看变量详情。'];
    if (!status?.running) parts.push('网关未运行时请用「启动网关并接管」。');
    if (clients?.has_issues) parts.push('检测到冲突，请点击「修复接管」。');
    return parts.join(' ');
  }, [status?.running, clients?.has_issues]);

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

  const run = async (action: () => Promise<unknown>, ok: string) => {
    setBusy(true);
    try {
      await action();
      pushToast(ok, 'ok');
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
      api_key: provider.api_key_masked,
      model_name: provider.model_name,
      protocol: normalizeProtocol(provider.protocol),
      enabled: provider.enabled,
    });
    setProviderModal(true);
  };

  const applyPreset = (preset: Partial<ProviderForm>) => {
    setForm((prev) => ({ ...prev, ...preset }));
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
    clients: '客户端接管',
    logs: '运行日志',
    about: '关于 SUGT',
  }[tab];

  const tabDesc = {
    dashboard: '启动/停止本地网关，查看当前模型',
    models: '管理上游 API 与协议类型',
    clients: 'Claude Code / Codex 环境变量接管',
    logs: '网关实时日志与调试信息',
    about: '版本与项目信息',
  }[tab];

  if (!isTauriRuntime()) {
    return (
      <main className="browser-only">
        <h2>SUGT 需在 Tauri 窗口中运行</h2>
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
          <div className="brand-logo">SG</div>
          <div>
            <h1>SUGT</h1>
            <span>su gateway</span>
          </div>
        </div>
        <button className={tab === 'dashboard' ? 'nav active' : 'nav'} onClick={() => setTab('dashboard')}><Activity size={18} />控制台</button>
        <button className={tab === 'models' ? 'nav active' : 'nav'} onClick={() => setTab('models')}><Bot size={18} />模型配置</button>
        <button className={tab === 'clients' ? 'nav active' : 'nav'} onClick={() => setTab('clients')}><Users size={18} />客户端</button>
        <button className={tab === 'logs' ? 'nav active' : 'nav'} onClick={() => setTab('logs')}><FileText size={18} />日志</button>
        <button className={tab === 'about' ? 'nav active' : 'nav'} onClick={() => setTab('about')}><Info size={18} />关于</button>
        <div className="sidebar-footer">
          <ShieldCheck size={18} />
          <span>QM科技内部版</span>
        </div>
      </aside>

      <section className="content">
        <header className="topbar">
          <div>
            <h2>{tabTitle}</h2>
            <p>{tabDesc}</p>
          </div>
          <span className="hint compact topbar-meta">每 3 秒自动刷新状态</span>
        </header>

        {tab === 'dashboard' && (
          <section className="page-grid dashboard-grid">
            <div className="card hero-card">
              <div className="hero-title">
                <Cpu size={28} />
                <div>
                  <h3>服务状态</h3>
                  <p>{status?.listen_url ?? 'http://127.0.0.1:8787'}</p>
                </div>
                {statusBadge}
              </div>
              <div className="metric-row">
                <div className="metric"><span>当前服务商</span><strong title={status?.active_provider ?? ''}>{status?.active_provider ?? '未配置'}</strong></div>
                <div className="metric"><span>当前模型</span><strong title={status?.active_model ?? ''}>{status?.active_model ?? '未配置'}</strong></div>
                <div className="metric">
                  <span>最近请求命中</span>
                  <strong title={status?.last_proxy_provider ?? ''}>
                    {status?.last_proxy_provider
                      ? `${status.last_proxy_provider}${status.last_proxy_failover ? '（故障转移）' : ''}`
                      : '暂无'}
                  </strong>
                </div>
              </div>
              {status?.last_proxy_provider && (
                <p className="hint compact">
                  路径 {status.last_proxy_path ?? '-'}
                  {status.last_proxy_at ? ` · ${new Date(status.last_proxy_at).toLocaleString()}` : ''}
                </p>
              )}
              <div className="actions">
                <button className="primary" disabled={busy || status?.running} onClick={() => run(() => invoke('start_gateway'), '服务已启动')}><Play size={17} />启动</button>
                <button className="danger" disabled={busy || !status?.running} onClick={() => run(() => invoke('stop_gateway'), '服务已停止')}><CircleStop size={17} />停止</button>
                <button className="ghost" onClick={() => invoke('open_config_dir')}><FolderOpen size={17} />配置目录</button>
              </div>
            </div>

            <div className="card switches">
              <h3>运行选项</h3>
              <label className="switch-line">
                <span>开机启动</span>
                <input type="checkbox" checked={Boolean(config?.autostart)} onChange={(e) => run(() => invoke('set_autostart', { enabled: e.target.checked }), '已更新')} />
              </label>
              <label className="switch-line">
                <span>故障转移</span>
                <input type="checkbox" checked={Boolean(config?.failover_enabled)} onChange={(e) => run(() => invoke('set_failover', { enabled: e.target.checked }), '已更新')} />
              </label>
              <label className="switch-line">
                <span>启动时自动开网关</span>
                <input type="checkbox" checked={Boolean(config?.autostart_gateway)} onChange={(e) => run(() => invoke('set_autostart_gateway', { enabled: e.target.checked }), '已更新')} />
              </label>
              <label className="switch-line select-line">
                <span>托盘退出时</span>
                <select
                  value={config?.quit_behavior ?? 'exit_only'}
                  onChange={(e) => run(() => invoke('set_quit_behavior', { behavior: e.target.value as QuitBehavior }), '已更新')}
                >
                  <option value="exit_only">仅退出程序</option>
                  <option value="stop_gateway">退出并停止网关</option>
                  <option value="stop_all">退出、停网关并关接管</option>
                </select>
              </label>
              <div className="hint compact">
                本地 OpenAI：<code>{status?.listen_url}/v1</code><br />
                本地 Anthropic：<code>{status?.listen_url}</code>
              </div>
            </div>

            <TrafficPanel traffic={status?.traffic} running={status?.running} />
          </section>
        )}

        {tab === 'models' && (
          <section className="card">
            <div className="section-title">
              <h3>模型列表</h3>
              <div className="title-actions">
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
                        {isActive && <span className="badge ok inline">默认使用</span>}
                        <span className={provider.enabled ? 'badge neutral inline' : 'badge stop inline'}>{provider.enabled ? '已启用' : '已禁用'}</span>
                      </div>
                      <span>{protocolLabel(normalizeProtocol(provider.protocol))} · {provider.model_name}</span>
                      <code>{provider.base_url}</code>
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
        )}

        {tab === 'clients' && (
          status?.trial.product_line === 'store' ? (
            <StoreClientsPanel
              busy={busy}
              setBusy={setBusy}
              status={status}
              clients={clients}
              onClientsChange={setClients}
              pushToast={pushToast}
              clientsHint={clientsHint}
              formatInvokeError={formatInvokeError}
            />
          ) : (
          <section className="page-grid single">
            <div className="card">
              <div className="section-title">
                <h3>Claude / Codex 接管</h3>
                <div className="title-actions">
                  <button className="ghost tiny-btn" disabled={busy} onClick={() => {
                    setBusy(true);
                    invoke<ClientsEnvStatus>('get_clients_env_status')
                      .then((next) => {
                        setClients(next);
                        pushToast(
                          next.has_issues ? '检测到接管问题，可点击「修复接管」' : '接管状态正常',
                          next.has_issues ? 'info' : 'ok',
                        );
                      })
                      .catch((error) => pushToast(formatInvokeError(error), 'error'))
                      .finally(() => setBusy(false));
                  }}><ShieldCheck size={14} />接管检查</button>
                  <button className="ghost tiny-btn" disabled={busy} onClick={() => run(() => invoke('repair_clients_env'), '已修复接管冲突，请新开终端')}><Wrench size={14} />修复接管</button>
                  <button
                    className="primary tiny-btn"
                    disabled={busy || !status?.running}
                    title={!status?.running ? '请先启动网关' : undefined}
                    onClick={() => run(() => invoke('install_clients_env'), '接管完成，请新开终端')}
                  ><PlugZap size={14} />一键接管</button>
                  <button
                    className="ghost tiny-btn"
                    disabled={busy || Boolean(status?.running)}
                    title={status?.running ? '网关已在运行，请使用一键接管' : undefined}
                    onClick={() => run(() => invoke('install_clients_env', { autoStart: true }), '网关已启动并完成接管，请新开终端')}
                  ><Play size={14} />启动网关并接管</button>
                  <button className="ghost tiny-btn" disabled={busy} onClick={() => run(() => invoke('uninstall_clients_env'), '已关闭 SUGT 接管，请新开终端')}>关闭接管</button>
                </div>
              </div>
              <p className="hint">{clientsHint}</p>
              <div className="client-grid">
                {[clients?.claude, clients?.codex].filter(Boolean).map((client) => (
                  <div className="client-card" key={client!.client}>
                    <div className="client-head">
                      <strong>{client!.client}</strong>
                      <ClientStatusBadge client={client!} listenUrl={clients?.listen_url ?? status?.listen_url ?? ''} />
                    </div>
                    <p className="hint compact">{client!.note}</p>
                  </div>
                ))}
              </div>
            </div>
          </section>
          )
        )}

        {tab === 'logs' && (
          <section className="card log-card-full">
            <div className="section-title">
              <h3>运行日志</h3>
              <button className="ghost tiny-btn" disabled={busy} onClick={() => refreshLogs()}><RefreshCw size={14} />刷新</button>
            </div>
            <LogViewer lines={logs} />
          </section>
        )}

        {tab === 'about' && (
          <section className="card about-card">
            <div className="about-logo">SUGT</div>
            <h3>SUGT - su gateway</h3>
            <p>作者：孙文龙 · QM科技内部版</p>
            <p className={status?.trial.valid ? 'trial-note' : 'trial-note expired'}>{status?.trial.message ?? '开发模式，无有效期限制'}</p>
            <div className="about-grid">
              <div><strong>配置目录</strong><span>{status?.config_dir}</span></div>
              <div><strong>日志文件</strong><span>{status?.log_file}</span></div>
              <div><strong>版本</strong><span>v{CURRENT_VERSION} · {status?.trial.product_label ?? '功能版'} · {status?.trial.edition ?? 'dev'}</span></div>
              <div><strong>试用状态</strong><span>{status?.trial.status ?? 'valid'}</span></div>
            </div>
            <div className="release-notes-frame">
              <div className="release-notes-head">
                <strong>v{CURRENT_VERSION} 更新说明</strong>
                <span className="hint compact">仅展示当前版本改动</span>
              </div>
              <ul className="release-notes-list">
                {RELEASE_NOTES.map((note) => (
                  <li key={note.text} className={`release-note ${note.type}`}>
                    <span className="release-tag">{note.type}</span>
                    {note.text}
                  </li>
                ))}
              </ul>
            </div>
          </section>
        )}
      </section>

      {providerModal && (
        <Modal title={form.id ? '编辑模型' : '添加模型'} onClose={() => setProviderModal(false)}>
          <div className="preset-row">
            <label className="preset-select-label">
              快速预设
              <select
                defaultValue=""
                onChange={(e) => {
                  const preset = PROVIDER_PRESETS.find((item) => item.id === e.target.value);
                  if (preset) applyPreset(preset.form);
                  e.target.value = '';
                }}
              >
                <option value="">选择国内服务商预设…</option>
                {PROVIDER_PRESETS.map((preset) => (
                  <option key={preset.id} value={preset.id}>{preset.label}</option>
                ))}
              </select>
            </label>
          </div>
          <label>配置名称<input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="例如：魔搭 DeepSeek" /></label>
          <label>服务商<input value={form.provider} onChange={(e) => setForm({ ...form, provider: e.target.value })} /></label>
          <label>协议类型
            <select value={form.protocol} onChange={(e) => setForm({ ...form, protocol: e.target.value as ProviderProtocol })}>
              <option value="openai">OpenAI 兼容（Claude 自动转换，推荐魔搭）</option>
              <option value="anthropic">Anthropic 原生（base 不带 /v1）</option>
            </select>
          </label>
          <label>Base URL
            <input value={form.base_url} onChange={(e) => setForm({ ...form, base_url: e.target.value })} placeholder={form.protocol === 'openai' ? 'https://api-inference.modelscope.cn/v1' : 'https://api-inference.modelscope.cn'} />
          </label>
          <label>API Key<input type="password" value={form.api_key} onChange={(e) => setForm({ ...form, api_key: e.target.value })} placeholder={form.id ? '留空则不修改' : 'ms-...'} /></label>
          <label>Model Name<input value={form.model_name} onChange={(e) => setForm({ ...form, model_name: e.target.value })} placeholder="deepseek-ai/DeepSeek-V4-Flash" /></label>
          <label className="switch-line"><span>启用</span><input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} /></label>
          <div className="modal-actions">
            <button className="ghost" onClick={() => setProviderModal(false)}>取消</button>
            <button className="primary" disabled={busy} onClick={saveProvider}><Save size={16} />保存</button>
          </div>
        </Modal>
      )}

      {deleteTarget && (
        <Modal title="确认删除" onClose={() => setDeleteTarget(null)}>
          <p>确定删除模型配置「{deleteTarget.name}」？</p>
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
