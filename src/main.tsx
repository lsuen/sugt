import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import {
  Activity, Bot, CheckCircle2, CircleStop, Cpu, FileText, FolderOpen, Info,
  Play, PlugZap, Plus, RefreshCw, Save, ShieldCheck, Trash2, Users, Wrench, X, XCircle,
} from 'lucide-react';
import './styles.css';

type Tab = 'dashboard' | 'models' | 'clients' | 'logs' | 'about';
type ProviderStatus = 'Unknown' | 'Available' | 'Unavailable';
type ProviderProtocol = 'openai' | 'anthropic';

type RuntimeStatus = {
  running: boolean;
  listen_url: string;
  active_model?: string | null;
  active_provider?: string | null;
  config_dir: string;
  log_file: string;
  trial: TrialStatus;
};

type TrialStatus = {
  edition: string;
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
  providers: unknown[];
};

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

type ProviderForm = {
  id?: string;
  name: string;
  provider: string;
  base_url: string;
  api_key: string;
  model_name: string;
  protocol: ProviderProtocol;
  enabled: boolean;
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

const MODELSCOPE_OPENAI: Partial<ProviderForm> = {
  provider: 'ModelScope',
  base_url: 'https://api-inference.modelscope.cn/v1',
  model_name: 'deepseek-ai/DeepSeek-V4-Flash',
  protocol: 'openai',
};

const MODELSCOPE_ANTHROPIC: Partial<ProviderForm> = {
  provider: 'ModelScope',
  base_url: 'https://api-inference.modelscope.cn',
  model_name: 'deepseek-ai/DeepSeek-V4-Flash',
  protocol: 'anthropic',
};

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

function App() {
  const [tab, setTab] = useState<Tab>('dashboard');
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [logs, setLogs] = useState<string[]>([]);
  const [clients, setClients] = useState<ClientsEnvStatus | null>(null);
  const [form, setForm] = useState<ProviderForm>(emptyForm);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [providerModal, setProviderModal] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ProviderView | null>(null);

  const activeProviderId = config?.active_provider_id ?? providers[0]?.id;

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
    refreshCore().catch((error) => setMessage(String(error)));
    const timer = window.setInterval(() => refresh().catch(() => undefined), 3000);
    return () => window.clearInterval(timer);
  }, [refresh, refreshCore]);

  useEffect(() => {
    if (tab === 'logs') refreshLogs().catch(() => undefined);
  }, [tab, refreshLogs]);

  const run = async (action: () => Promise<unknown>, ok: string) => {
    setBusy(true);
    setMessage('');
    try {
      await action();
      setMessage(ok);
      await refresh();
    } catch (error) {
      setMessage(String(error));
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
    setMessage('');
    try {
      await invoke('save_provider', { input: form });
      setMessage(form.id ? '模型配置已更新' : '模型配置已添加');
      setProviderModal(false);
      setForm(emptyForm);
      await refresh();
    } catch (error) {
      setMessage(String(error));
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
    setMessage('');
    let available = 0;
    let unavailable = 0;

    try {
      for (const provider of providers) {
        const result = await invoke<ProviderStatus>('test_provider', { id: provider.id });
        if (result === 'Available') available += 1;
        if (result === 'Unavailable') unavailable += 1;
      }
      setMessage(`一键测试完成：${available} 个可用，${unavailable} 个不可用`);
      await refresh();
    } catch (error) {
      setMessage(String(error));
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
          <button className="ghost" disabled={busy} onClick={() => refresh()}><RefreshCw size={16} />刷新</button>
        </header>

        {message && (
          <div className={message.includes('error') || message.includes('失败') || message.includes('invalid') ? 'notice error' : 'notice'}>
            {message}
          </div>
        )}

        {tab === 'dashboard' && (
          <section className="page-grid">
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
              </div>
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
              <div className="hint compact">
                本地 OpenAI：<code>{status?.listen_url}/v1</code><br />
                本地 Anthropic：<code>{status?.listen_url}</code>
              </div>
            </div>
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
            {providers.length === 0 && <p className="hint">暂无配置。点击「添加模型」或使用魔搭预设。</p>}
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
          <section className="page-grid single">
            <div className="card">
              <div className="section-title">
                <h3>Claude / Codex 接管</h3>
                <div className="title-actions">
                  <button className="ghost tiny-btn" disabled={busy} onClick={() => {
                    setBusy(true);
                    setMessage('');
                    invoke<ClientsEnvStatus>('get_clients_env_status')
                      .then((next) => {
                        setClients(next);
                        setMessage(next.has_issues ? '检测到接管问题，可点击「修复接管」' : '接管状态正常');
                      })
                      .catch((error) => setMessage(String(error)))
                      .finally(() => setBusy(false));
                  }}><ShieldCheck size={14} />接管检查</button>
                  <button className="ghost tiny-btn" disabled={busy} onClick={() => run(() => invoke('repair_clients_env'), '已修复接管冲突，请新开终端')}><Wrench size={14} />修复接管</button>
                  <button className="primary tiny-btn" disabled={busy} onClick={() => run(() => invoke('install_clients_env'), '接管完成，请新开终端')}><PlugZap size={14} />一键接管</button>
                  <button className="ghost tiny-btn" disabled={busy} onClick={() => run(() => invoke('uninstall_clients_env'), '已关闭 SUGT 接管，请新开终端')}>关闭接管</button>
                </div>
              </div>
              <p className="hint">写入用户环境变量，仅对新打开的终端生效。Claude 仅设置 ANTHROPIC_AUTH_TOKEN，避免与 ANTHROPIC_API_KEY 冲突。</p>
              {clients?.has_issues && (
                <div className="notice error compact-notice">检测到接管冲突或旧版残留变量，请点击「修复接管」后新开终端。</div>
              )}
              <div className="client-grid">
                {[clients?.claude, clients?.codex].filter(Boolean).map((client) => (
                  <div className="client-card" key={client!.client}>
                    <div className="client-head">
                      <strong>{client!.client}</strong>
                      <span className={client!.configured ? 'badge ok' : 'badge stop'}>{client!.configured ? '已接管' : '未接管'}</span>
                    </div>
                    <p className="hint compact">{client!.note}</p>
                    {client!.issues && client!.issues.length > 0 && (
                      <ul className="issue-list">
                        {client!.issues.map((issue) => (
                          <li key={issue}>{issue}</li>
                        ))}
                      </ul>
                    )}
                    {client!.variables && (
                      <div className="env-vars">
                        {Object.entries(client!.variables).map(([name, value]) => (
                          <div className="env-row" key={name}><span>{name}</span><code>{value}</code></div>
                        ))}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
          </section>
        )}

        {tab === 'logs' && (
          <section className="card log-card-full">
            <div className="section-title">
              <h3>运行日志</h3>
              <button className="ghost tiny-btn" onClick={() => refreshLogs()}><RefreshCw size={14} />刷新日志</button>
            </div>
            <pre>{logs.length ? logs.join('\n') : '暂无日志'}</pre>
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
              <div><strong>版本类型</strong><span>{status?.trial.edition ?? 'dev'} · {status?.trial.build_id ?? 'dev'}</span></div>
              <div><strong>试用状态</strong><span>{status?.trial.status ?? 'valid'}</span></div>
            </div>
          </section>
        )}
      </section>

      {providerModal && (
        <Modal title={form.id ? '编辑模型' : '添加模型'} onClose={() => setProviderModal(false)}>
          <div className="preset-row">
            <span className="hint">快速预设：</span>
            <button className="tiny" type="button" onClick={() => applyPreset(MODELSCOPE_OPENAI)}>魔搭 OpenAI</button>
            <button className="tiny" type="button" onClick={() => applyPreset(MODELSCOPE_ANTHROPIC)}>魔搭 Anthropic</button>
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
    </main>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
