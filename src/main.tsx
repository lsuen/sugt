import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import { Activity, Bot, CheckCircle2, CircleStop, Cpu, FolderOpen, Info, Play, PlugZap, Plus, RefreshCw, Save, ShieldCheck, Trash2, XCircle } from 'lucide-react';
import './styles.css';

type ProviderStatus = 'Unknown' | 'Available' | 'Unavailable';

type RuntimeStatus = {
  running: boolean;
  listen_url: string;
  active_model?: string | null;
  active_provider?: string | null;
  config_dir: string;
  log_file: string;
};

type ProviderView = {
  id: string;
  name: string;
  provider: string;
  base_url: string;
  api_key_masked: string;
  model_name: string;
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
  variables: Record<string, string>;
  missing: string[];
  note: string;
};

type ClientsEnvStatus = {
  listen_url: string;
  claude: ClientEnvStatus;
  codex: ClientEnvStatus;
};

type ProviderForm = {
  id?: string;
  name: string;
  provider: string;
  base_url: string;
  api_key: string;
  model_name: string;
  enabled: boolean;
};

const emptyForm: ProviderForm = {
  name: '',
  provider: 'OpenAI Compatible',
  base_url: '',
  api_key: '',
  model_name: '',
  enabled: true,
};

function App() {
  const [tab, setTab] = useState<'console' | 'models' | 'about'>('console');
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [logs, setLogs] = useState<string[]>([]);
  const [clients, setClients] = useState<ClientsEnvStatus | null>(null);
  const [form, setForm] = useState<ProviderForm>(emptyForm);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string>('');

  const activeProviderId = config?.active_provider_id ?? providers[0]?.id;

  const refresh = useCallback(async () => {
    const [nextStatus, nextProviders, nextConfig, nextLogs, nextClients] = await Promise.all([
      invoke<RuntimeStatus>('get_status'),
      invoke<ProviderView[]>('list_providers'),
      invoke<AppConfig>('get_config'),
      invoke<string[]>('read_logs', { lines: 120 }),
      invoke<ClientsEnvStatus>('get_clients_env_status'),
    ]);
    setStatus(nextStatus);
    setProviders(nextProviders);
    setConfig(nextConfig);
    setLogs(nextLogs);
    setClients(nextClients);
  }, []);

  useEffect(() => {
    refresh().catch((error) => setMessage(String(error)));
    const timer = window.setInterval(() => refresh().catch(() => undefined), 2500);
    return () => window.clearInterval(timer);
  }, [refresh]);

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

  const editProvider = (provider: ProviderView) => {
    setForm({
      id: provider.id,
      name: provider.name,
      provider: provider.provider,
      base_url: provider.base_url,
      api_key: provider.api_key_masked,
      model_name: provider.model_name,
      enabled: provider.enabled,
    });
  };

  const saveProvider = () => run(
    () => invoke('save_provider', { input: form }),
    form.id ? '模型配置已更新' : '模型配置已添加',
  ).then(() => setForm(emptyForm));

  const statusBadge = useMemo(() => {
    if (!status) return <span className="badge neutral">加载中</span>;
    return status.running ? <span className="badge ok">运行中</span> : <span className="badge stop">已停止</span>;
  }, [status]);

  const clientCard = (client?: ClientEnvStatus) => {
    if (!client) return null;
    return (
      <div className="client-card">
        <div className="client-head">
          <strong>{client.client}</strong>
          <span className={client.configured ? 'badge ok' : 'badge stop'}>{client.configured ? '已接管' : '未接管'}</span>
        </div>
        <div className="env-list">
          {Object.entries(client.variables).map(([name, value]) => <code key={name}>{name}={value}</code>)}
        </div>
        <p className="hint">{client.note}</p>
      </div>
    );
  };

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
        <button className={tab === 'console' ? 'nav active' : 'nav'} onClick={() => setTab('console')}><Activity size={18} />控制台</button>
        <button className={tab === 'models' ? 'nav active' : 'nav'} onClick={() => setTab('models')}><Bot size={18} />模型配置</button>
        <button className={tab === 'about' ? 'nav active' : 'nav'} onClick={() => setTab('about')}><Info size={18} />关于</button>
        <div className="sidebar-footer">
          <ShieldCheck size={18} />
          <span>QM科技内部版</span>
        </div>
      </aside>

      <section className="content">
        <header className="topbar">
          <div>
            <h2>{tab === 'console' ? '控制台' : tab === 'models' ? '模型配置' : '关于 SUGT'}</h2>
            <p>Claude Code / Codex / OpenAI 标准协议本地中转网关</p>
          </div>
          <button className="ghost" disabled={busy} onClick={() => refresh()}><RefreshCw size={16} />刷新</button>
        </header>

        {message && <div className={message.includes('error') || message.includes('失败') ? 'notice error' : 'notice'}>{message}</div>}

        {tab === 'console' && (
          <section className="grid console-grid">
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
                <div className="metric"><span>当前服务商</span><strong>{status?.active_provider ?? '未配置'}</strong></div>
                <div className="metric"><span>当前模型名</span><strong>{status?.active_model ?? '未配置'}</strong></div>
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
                <input type="checkbox" checked={Boolean(config?.autostart)} onChange={(event) => run(() => invoke('set_autostart', { enabled: event.target.checked }), '开机启动设置已更新')} />
              </label>
              <label className="switch-line">
                <span>故障转移</span>
                <input type="checkbox" checked={Boolean(config?.failover_enabled)} onChange={(event) => run(() => invoke('set_failover', { enabled: event.target.checked }), '故障转移设置已更新')} />
              </label>
              <div className="hint">OpenAI Base URL：<code>{status?.listen_url}/v1</code>；Anthropic Base URL：<code>{status?.listen_url}</code>。</div>
            </div>

            <div className="card client-panel">
              <div className="section-title">
                <h3>Claude / Codex 接管</h3>
                <button className="tiny" disabled={busy} onClick={() => run(() => invoke('install_clients_env'), '已写入用户环境变量；请重新打开终端后启动 claude/codex')}><PlugZap size={14} />一键接管</button>
              </div>
              <div className="client-grid">
                {clientCard(clients?.claude)}
                {clientCard(clients?.codex)}
              </div>
              <p className="hint">需要先启动 SUGT 服务；环境变量写入后仅对新打开的终端生效。</p>
            </div>

            <div className="card log-card">
              <h3>实时日志调试</h3>
              <pre>{logs.length ? logs.join('\n') : '暂无日志。启动服务或测试模型后会显示最新日志。'}</pre>
            </div>
          </section>
        )}

        {tab === 'models' && (
          <section className="models-layout">
            <div className="card provider-list">
              <div className="section-title"><h3>多模型配置列表</h3><span>{providers.length} 个配置</span></div>
              {providers.map((provider) => (
                <div className={provider.id === activeProviderId ? 'provider active' : 'provider'} key={provider.id} onClick={() => editProvider(provider)}>
                  <div className="provider-main">
                    <strong>{provider.name}</strong>
                    <span>{provider.provider} · {provider.model_name}</span>
                    <code>{provider.base_url}</code>
                  </div>
                  <div className="provider-actions">
                    {provider.status === 'Available' ? <CheckCircle2 className="ok-text" size={18} /> : provider.status === 'Unavailable' ? <XCircle className="bad-text" size={18} /> : <span className="dot" />}
                    <button className="tiny" onClick={(event) => { event.stopPropagation(); run(() => invoke('set_active_provider', { id: provider.id }), '当前模型已切换'); }}>使用</button>
                    <button className="tiny" onClick={(event) => { event.stopPropagation(); run(() => invoke('test_provider', { id: provider.id }), '连接测试完成'); }}>测试</button>
                    <button className="tiny danger-link" onClick={(event) => { event.stopPropagation(); run(() => invoke('delete_provider', { id: provider.id }), '模型配置已删除'); }}><Trash2 size={14} /></button>
                  </div>
                </div>
              ))}
            </div>

            <div className="card form-card">
              <div className="section-title"><h3>{form.id ? '编辑模型配置' : '添加模型配置'}</h3><button className="tiny" onClick={() => setForm(emptyForm)}><Plus size={14} />新建</button></div>
              <label>配置名称<input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="例如：生产 Qwen" /></label>
              <label>服务商<input value={form.provider} onChange={(e) => setForm({ ...form, provider: e.target.value })} placeholder="OpenAI Compatible" /></label>
              <label>Base URL<input value={form.base_url} onChange={(e) => setForm({ ...form, base_url: e.target.value })} placeholder="https://example.com/v1" /></label>
              <label>API Key<input type="password" value={form.api_key} onChange={(e) => setForm({ ...form, api_key: e.target.value })} placeholder={form.id ? '留空或保留掩码则不修改' : 'sk-...'} /></label>
              <label>Model Name<input value={form.model_name} onChange={(e) => setForm({ ...form, model_name: e.target.value })} placeholder="模型名" /></label>
              <label className="switch-line"><span>启用</span><input type="checkbox" checked={form.enabled} onChange={(e) => setForm({ ...form, enabled: e.target.checked })} /></label>
              <button className="primary full" disabled={busy} onClick={saveProvider}><Save size={16} />保存配置</button>
            </div>
          </section>
        )}

        {tab === 'about' && (
          <section className="card about-card">
            <div className="about-logo">SUGT</div>
            <h3>SUGT - su gateway</h3>
            <p>作者：孙文龙</p>
            <p>为QM科技定制开发，仅限内部人员使用，未经授权禁止私自传播。</p>
            <div className="about-grid">
              <div><strong>核心功能</strong><span>OpenAI 标准协议本地代理、多模型故障转移、实时日志、GUI/CLI 双模式。</span></div>
              <div><strong>适配工具</strong><span>Claude Code、Codex 以及其他支持 OpenAI Base URL 的客户端。</span></div>
              <div><strong>配置目录</strong><span>{status?.config_dir}</span></div>
              <div><strong>日志文件</strong><span>{status?.log_file}</span></div>
            </div>
          </section>
        )}
      </section>
    </main>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
