import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import {
  Activity, Bot, CheckCircle2, CircleStop, Cpu, FileText, FolderOpen, Info,
  Play, Plus, RefreshCw, ShieldCheck, Sparkles, Trash2, Users, X, XCircle, Settings2,
} from 'lucide-react';
import './styles.css';
import { GatewayAccessBanner } from './GatewayAccessBanner';
import { SettingsPanel, type OverlayConfigView } from './SettingsPanel';
import { CURRENT_VERSION, RELEASE_NOTES } from './release-notes';
import { SkillsPanel } from './SkillsPanel';
import { TrafficPanel } from './TrafficPanel';
import { ProviderModal } from './ProviderModal';
import { TakeoverProfilesSection } from './TakeoverProfilesSection';
import type { ProviderForm } from './providerPresets';
import { lang, t } from './i18n';

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
  last_proxy_client?: string | null;
  last_proxy_mode?: string | null;
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
  overlay?: OverlayConfigView;
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
        time = date.toLocaleTimeString(lang() === 'zh' ? 'zh-CN' : undefined, { hour12: false });
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
    return <div className="log-empty">{t('logs.empty')}</div>;
  }

  return (
    <>
      <div className="log-toolbar">
        <span className="hint compact">{t('logs.count', { n: lines.length })}</span>
        <button
          type="button"
          className={`ghost tiny-btn${autoScroll ? ' active' : ''}`}
          onClick={() => {
            setAutoScroll(true);
            endRef.current?.scrollIntoView({ behavior: 'smooth' });
          }}
        >
          {autoScroll ? t('logs.follow') : t('logs.toBottom')}
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
          <button type="button" className="toast-close" onClick={() => onDismiss(toast.id)} aria-label={t('common.close')}>×</button>
        </div>
      ))}
    </div>
  );
}

function normalizeProtocol(value: ProviderView['protocol']): ProviderProtocol {
  return value === 'anthropic' ? 'anthropic' : 'openai';
}

function protocolLabel(protocol: ProviderProtocol) {
  return protocol === 'anthropic' ? t('common.protocolAnthropic') : t('common.protocolOpenai');
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
          <button className="icon-btn" onClick={onClose} aria-label={t('common.close')}><X size={18} /></button>
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
    await run(() => invoke('set_provider_enabled', { id: provider.id, enabled }), enabled ? t('models.toastEnabled') : t('models.toastDisabled'));
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
      pushToast(t('models.toastTestAll', { a: available, b: unavailable }), unavailable > 0 ? 'info' : 'ok');
      await refresh();
    } catch (error) {
      pushToast(String(error), 'error');
    } finally {
      setBusy(false);
    }
  };

  const statusBadge = useMemo(() => {
    if (!status) return <span className="badge neutral">{t('common.loading')}</span>;
    return status.running ? <span className="badge ok">{t('common.running')}</span> : <span className="badge stop">{t('common.stopped')}</span>;
  }, [status]);

  const tabTitle = {
    dashboard: t('tab.dashboard.title'),
    models: t('tab.models.title'),
    clients: t('tab.clients.title'),
    skills: t('tab.skills.title'),
    settings: t('tab.settings.title'),
    logs: t('tab.logs.title'),
    about: t('tab.about.title'),
  }[tab];

  const tabDesc = {
    dashboard: t('tab.dashboard.desc'),
    models: t('tab.models.desc'),
    clients: t('tab.clients.desc'),
    skills: t('tab.skills.desc'),
    settings: t('tab.settings.desc'),
    logs: t('tab.logs.desc'),
    about: t('tab.about.desc'),
  }[tab];

  if (!isTauriRuntime()) {
    return (
      <main className="browser-only">
        <h2>{t('browser.title')}</h2>
        <p>{t('browser.desc')}</p>
        <p>{t('browser.root')}</p>
        <pre>npm run tauri dev</pre>
        <p className="hint">{t('browser.hint')}</p>
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
            <span>{t('brand.tagline')}</span>
          </div>
        </div>
        <button className={tab === 'dashboard' ? 'nav active' : 'nav'} onClick={() => setTab('dashboard')}><Activity size={18} />{t('nav.dashboard')}</button>
        <button className={tab === 'models' ? 'nav active' : 'nav'} onClick={() => setTab('models')}><Bot size={18} />{t('nav.models')}</button>
        <button className={tab === 'clients' ? 'nav active' : 'nav'} onClick={() => setTab('clients')}><Users size={18} />{t('nav.clients')}</button>
        <button className={tab === 'skills' ? 'nav active' : 'nav'} onClick={() => setTab('skills')}><Sparkles size={18} />{t('nav.skills')}</button>
        <button className={tab === 'settings' ? 'nav active' : 'nav'} onClick={() => setTab('settings')}><Settings2 size={18} />{t('nav.settings')}</button>
        <button className={tab === 'logs' ? 'nav active' : 'nav'} onClick={() => setTab('logs')}><FileText size={18} />{t('nav.logs')}</button>
        <button className={tab === 'about' ? 'nav active' : 'nav'} onClick={() => setTab('about')}><Info size={18} />{t('nav.about')}</button>
        <div className="sidebar-footer">
          <ShieldCheck size={18} />
          <span>{t('about.brand')}</span>
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
                    <h3>{t('dash.service')}</h3>
                    <p>{status?.listen_url ?? 'http://127.0.0.1:8787'}</p>
                  </div>
                  {statusBadge}
                </div>
                {status?.last_proxy_provider && (
                  <p className="hint compact hero-proxy-hint">
                    {t('dash.recent')} {status.last_proxy_provider}
                    {status.last_proxy_failover ? t('dash.failover') : ''}
                    · {status.last_proxy_path ?? '-'}
                    {status.last_proxy_at ? ` · ${new Date(status.last_proxy_at).toLocaleString()}` : ''}
                  </p>
                )}
                <div className="actions dashboard-actions">
                  <button className="primary" disabled={busy || status?.running} onClick={() => run(() => invoke('start_gateway'), t('dash.toastStarted'))}><Play size={17} />{t('dash.start')}</button>
                  <button className="danger" disabled={busy || !status?.running} onClick={() => run(() => invoke('stop_gateway'), t('dash.toastStopped'))}><CircleStop size={17} />{t('dash.stop')}</button>
                  <button className="ghost" onClick={() => invoke('open_config_dir')}><FolderOpen size={17} />{t('dash.configDir')}</button>
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
                onActiveProviderChange={(id) => run(() => invoke('set_active_provider', { id }), t('dash.toastSwitched'))}
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
              <h3>{t('models.title')}</h3>
              <div className="title-actions">
                {!providers.some((p) => p.id === 'sugt-zen-free') && (
                  <button
                    className="ghost tiny-btn"
                    disabled={busy}
                    title={t('models.restoreZenTitle')}
                    onClick={() =>
                      run(async () => {
                        const list = await invoke<ProviderView[]>('restore_experimental_zen');
                        setProviders(list);
                      }, t('models.toastZenRestored'))
                    }
                  >
                    {t('models.restoreZen')}
                  </button>
                )}
                <button className="ghost tiny-btn" disabled={busy || providers.length === 0} onClick={testAllProviders}>{t('models.testAll')}</button>
                <button className="primary tiny-btn" disabled={busy} onClick={openNewProvider}><Plus size={14} />{t('models.add')}</button>
              </div>
            </div>
            {providers.length === 0 && <p className="hint">{t('models.empty')}</p>}
            <div className="provider-list-scroll">
              {providers.map((provider) => {
                const isActive = provider.id === activeProviderId;
                const className = `provider${isActive ? ' active' : ''}${provider.enabled ? '' : ' disabled'}`;
                return (
                  <div className={className} key={provider.id}>
                    <div className="provider-main" onClick={() => openEditProvider(provider)}>
                      <div className="provider-title">
                        <strong>{provider.name}</strong>
                        {provider.experimental && <span className="badge inline" title={t('models.experimentalTitle')}>{t('models.experimental')}</span>}
                        {isActive && <span className="badge ok inline">{t('common.default')}</span>}
                        <span className={provider.enabled ? 'badge neutral inline' : 'badge stop inline'}>{provider.enabled ? t('common.enabled') : t('common.disabled')}</span>
                      </div>
                      <span className="provider-sub">{protocolLabel(normalizeProtocol(provider.protocol))} · {provider.model_name}</span>
                      {provider.experimental && (
                        <span className="hint compact">{t('models.restoreZenHint')}</span>
                      )}
                      <code className="provider-url" title={provider.base_url}>{provider.base_url}</code>
                      {provider.last_error && <span className="error-text">{provider.last_error}</span>}
                    </div>
                    <div className="provider-actions">
                      {provider.status === 'Available' ? <CheckCircle2 className="ok-text" size={18} /> : provider.status === 'Unavailable' ? <XCircle className="bad-text" size={18} /> : <span className="dot" />}
                      <button className="tiny" disabled={busy || isActive || !provider.enabled} onClick={() => run(() => invoke('set_active_provider', { id: provider.id }), t('models.toastDefault'))}>{isActive ? t('models.isDefault') : t('models.setDefault')}</button>
                      <button className="tiny" disabled={busy} onClick={() => setProviderEnabled(provider, !provider.enabled)}>{provider.enabled ? t('common.disabled') : t('common.enabled')}</button>
                      <button className="tiny" disabled={busy} onClick={() => run(() => invoke('test_provider', { id: provider.id }), t('models.toastTested'))}>{t('common.test')}</button>
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
              <h3>{t('logs.title')}</h3>
              <button className="ghost tiny-btn" disabled={busy} onClick={() => refreshLogs()}><RefreshCw size={14} />{t('common.refresh')}</button>
            </div>
            <LogViewer lines={logs} />
          </section>
          </div>
        )}

        {tab === 'about' && (
          <div className="tab-body">
          <section className="card about-card">
            <div className="about-logo">SUTAI</div>
            <h3>{t('about.title')}</h3>
            <p>{t('about.author')}</p>
            <p className={status?.trial.valid ? 'trial-note' : 'trial-note expired'}>
              {status?.trial.message ?? t('about.devMode')}
            </p>
            <div className="about-grid">
              <div><strong>{t('about.version')}</strong><span>v{CURRENT_VERSION}</span></div>
              <div><strong>{t('about.product')}</strong><span>{status?.trial.product_label ?? t('about.full')}</span></div>
              <div><strong>{t('about.license')}</strong><span>{status?.trial.message ?? t('about.devMode')}</span></div>
              <div><strong>{t('about.configDir')}</strong><span>{status?.config_dir}</span></div>
              <div><strong>{t('about.logFile')}</strong><span>{status?.log_file}</span></div>
              <div><strong>{t('about.listen')}</strong><span>{status?.listen_url ?? 'http://127.0.0.1:8787'}</span></div>
            </div>
            <div className="release-notes-frame">
              <div className="release-notes-head">
                <strong>{t('about.releaseHead')}</strong>
                <span className="hint compact">v{CURRENT_VERSION}</span>
              </div>
              <ul className="release-notes-list">
                {RELEASE_NOTES.map((note) => (
                  <li key={note.text} className={`release-note ${note.type}`}>
                    <span className="release-tag">{note.type === 'feat' ? t('about.tagFeat') : t('about.tagFix')}</span>
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

      <ToastHost toasts={toasts} onDismiss={dismissToast} />
    </main>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
