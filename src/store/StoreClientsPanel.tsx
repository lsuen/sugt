import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  ArrowLeft, BookOpen, FolderOpen, Package, PlugZap, Play, RefreshCw,
  Search, ShieldCheck, Sparkles, Wrench,
} from 'lucide-react';
import { SkillRow } from './SkillRow';
import type {
  DiscoverFilter,
  MountTarget,
  PluginPanelView,
  SkillCatalogItem,
  SkillRepoView,
  StoreClientPaths,
  StoreClientTab,
  StoreContentTab,
  StoreSettings,
  StoreSubview,
} from './types';

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

type RuntimeStatus = {
  running: boolean;
  listen_url: string;
  trial: { product_line: string; product_label: string };
};

type ToastFn = (message: string, type: 'ok' | 'error' | 'info') => void;

type Props = {
  busy: boolean;
  setBusy: (v: boolean) => void;
  status: RuntimeStatus | null;
  clients: ClientsEnvStatus | null;
  onClientsChange: (next: ClientsEnvStatus) => void;
  pushToast: ToastFn;
  clientsHint: string;
  formatInvokeError: (error: unknown) => string;
};

const DISCOVER_FILTERS: { id: DiscoverFilter; label: string }[] = [
  { id: 'all', label: '全部' },
  { id: 'available', label: '可安装' },
  { id: 'staged', label: '已暂存' },
  { id: 'mounted', label: '已挂载' },
];

const CONTENT_TABS: { id: StoreContentTab; label: string }[] = [
  { id: 'env', label: '环境' },
  { id: 'skills', label: '技能' },
  { id: 'plugins', label: '插件' },
];

function ClientStatusBadge({ client, listenUrl }: { client: ClientEnvStatus; listenUrl: string }) {
  const [hover, setHover] = useState(false);
  return (
    <span className="badge-wrap" onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}>
      <span className={`badge popover-trigger ${client.configured ? 'ok' : 'stop'}`}>
        {client.configured ? '已接管' : '未接管'}
      </span>
      {hover && (
        <div className="badge-popover" role="tooltip">
          <div className="badge-popover-title">{client.client}</div>
          {client.variables && (
            <div className="env-vars compact">
              {Object.entries(client.variables).map(([name, value]) => (
                <div className="env-row" key={name}><span>{name}</span><code>{value || '（未设置）'}</code></div>
              ))}
            </div>
          )}
          <p className="hint compact popover-foot">网关 {listenUrl}</p>
        </div>
      )}
    </span>
  );
}

function StatusBanner({ text }: { text: string }) {
  return (
    <div className="store-status-banner" role="status">
      <RefreshCw size={14} className="store-spin" />
      <span>{text}</span>
    </div>
  );
}

export function StoreClientsPanel({
  busy, setBusy, status, clients, onClientsChange, pushToast, clientsHint, formatInvokeError,
}: Props) {
  const [subview, setSubview] = useState<StoreSubview>('main');
  const [clientTab, setClientTab] = useState<StoreClientTab>('claude');
  const [contentTab, setContentTab] = useState<StoreContentTab>('env');
  const [discoverFilter, setDiscoverFilter] = useState<DiscoverFilter>('all');
  const [catalog, setCatalog] = useState<SkillCatalogItem[]>([]);
  const [repos, setRepos] = useState<SkillRepoView[]>([]);
  const [pluginPanel, setPluginPanel] = useState<PluginPanelView | null>(null);
  const [paths, setPaths] = useState<StoreClientPaths | null>(null);
  const [settings, setSettings] = useState<StoreSettings>({ editor_command: '' });
  const [search, setSearch] = useState('');
  const [repoFilter, setRepoFilter] = useState('');
  const [newRepo, setNewRepo] = useState({ owner: '', repo: '', branch: 'main' });
  const [statusText, setStatusText] = useState<string | null>(null);

  const loadCatalog = useCallback(async () => {
    const items = await invoke<SkillCatalogItem[]>('store_list_catalog', { query: null });
    setCatalog(items);
  }, []);

  const loadPluginPanel = useCallback(async (client: StoreClientTab) => {
    const panel = await invoke<PluginPanelView>('store_get_plugin_panel', { client });
    setPluginPanel(panel);
  }, []);

  const loadStoreData = useCallback(async () => {
    const [catalogItems, repoItems, pathView, storeSettings] = await Promise.all([
      invoke<SkillCatalogItem[]>('store_list_catalog', { query: null }),
      invoke<SkillRepoView[]>('store_list_repos'),
      invoke<StoreClientPaths>('store_get_client_paths'),
      invoke<StoreSettings>('store_get_settings'),
    ]);
    setCatalog(catalogItems);
    setRepos(repoItems);
    setPaths(pathView);
    setSettings(storeSettings);
    await loadPluginPanel(clientTab);
  }, [clientTab, loadPluginPanel]);

  useEffect(() => {
    loadStoreData().catch((error) => pushToast(formatInvokeError(error), 'error'));
  }, [loadStoreData, pushToast, formatInvokeError]);

  useEffect(() => {
    loadPluginPanel(clientTab).catch((error) => pushToast(formatInvokeError(error), 'error'));
  }, [clientTab, loadPluginPanel, pushToast, formatInvokeError]);

  const withStatus = async (message: string, task: () => Promise<void>, okMessage?: string) => {
    setStatusText(message);
    setBusy(true);
    try {
      await task();
      if (okMessage) pushToast(okMessage, 'ok');
    } catch (error) {
      pushToast(formatInvokeError(error), 'error');
    } finally {
      setBusy(false);
      setStatusText(null);
    }
  };

  const run = async (task: () => Promise<void>, okMessage?: string) => {
    setBusy(true);
    try {
      await task();
      if (okMessage) pushToast(okMessage, 'ok');
    } catch (error) {
      pushToast(formatInvokeError(error), 'error');
    } finally {
      setBusy(false);
    }
  };

  const installSkill = (skillId: string) =>
    run(async () => {
      await invoke('store_install_skill', { skillId });
      await loadCatalog();
    }, '已安装到暂存区');

  const mountSkill = (skillId: string, target: MountTarget) =>
    run(async () => {
      await invoke('store_mount_skill', { skillId, target });
      await loadCatalog();
    }, '挂载完成');

  const unmountSkill = (skillId: string, target: MountTarget) =>
    run(async () => {
      await invoke('store_unmount_skill', { skillId, target });
      await loadCatalog();
    }, '已取消挂载');

  const uninstallSkill = (skillId: string) =>
    run(async () => {
      await invoke('store_uninstall_skill', { skillId });
      await loadCatalog();
    }, '已从暂存区移除');

  const openSkill = (skillId: string, staged: boolean) =>
    run(() => invoke('store_open_skill', { skillId, staged, client: clientTab }));

  const filteredCatalog = useMemo(() => {
    let items = catalog;
    if (repoFilter) items = items.filter((item) => item.repo_id === repoFilter);
    if (search.trim()) {
      const q = search.trim().toLowerCase();
      items = items.filter(
        (item) =>
          item.name.toLowerCase().includes(q)
          || (item.description?.toLowerCase().includes(q) ?? false)
          || item.repo_label.toLowerCase().includes(q),
      );
    }
    switch (discoverFilter) {
      case 'available':
        items = items.filter((item) => !item.staged);
        break;
      case 'staged':
        items = items.filter((item) => item.staged);
        break;
      case 'mounted':
        items = items.filter((item) => item.mounted);
        break;
      default:
        break;
    }
    return items;
  }, [catalog, repoFilter, search, discoverFilter]);

  const clientStagedSkills = useMemo(
    () => catalog.filter((s) => s.staged),
    [catalog],
  );

  const activeClient = clientTab === 'claude' ? clients?.claude : clients?.codex;
  const skillsPath = clientTab === 'claude' ? paths?.claude_skills : paths?.codex_skills;

  const skillList = (items: SkillCatalogItem[]) => (
    <div className="store-scroll-panel">
      <div className="store-skill-list">
        {items.length === 0 && (
          <div className="store-empty">
            {discoverFilter === 'staged'
              ? '暂无暂存技能。切换到「可安装」或「全部」安装后，可在此直接挂载。'
              : '暂无匹配技能。请先刷新仓库或调整筛选。'}
          </div>
        )}
        {items.map((skill) => (
          <SkillRow
            key={skill.id}
            skill={skill}
            busy={busy}
            clientTab={clientTab}
            onInstall={installSkill}
            onMount={mountSkill}
            onUnmount={unmountSkill}
            onUninstall={uninstallSkill}
            onOpen={openSkill}
          />
        ))}
      </div>
    </div>
  );

  if (subview === 'discover') {
    return (
      <section className="page-grid single store-subview">
        <div className="card store-card store-card-fill">
          <div className="section-title">
            <div className="store-title-row">
              <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('main')}>
                <ArrowLeft size={14} />返回
              </button>
              <h3>发现技能</h3>
            </div>
            <div className="title-actions">
              <button
                type="button"
                className="ghost tiny-btn"
                disabled={busy}
                onClick={() => withStatus('正在刷新全部仓库…', async () => {
                  const next = await invoke<SkillRepoView[]>('store_refresh_all_repos');
                  setRepos(next);
                  await loadCatalog();
                }, '仓库已刷新')}
              >
                <RefreshCw size={14} />刷新全部
              </button>
            </div>
          </div>
          {statusText && <StatusBanner text={statusText} />}
          <div className="store-filter-tabs">
            {DISCOVER_FILTERS.map((tab) => (
              <button
                key={tab.id}
                type="button"
                className={discoverFilter === tab.id ? 'store-tab active' : 'store-tab'}
                onClick={() => setDiscoverFilter(tab.id)}
              >
                {tab.label}
              </button>
            ))}
          </div>
          <div className="store-filters">
            <div className="store-search">
              <Search size={16} />
              <input placeholder="搜索技能" value={search} onChange={(e) => setSearch(e.target.value)} />
            </div>
            <select value={repoFilter} onChange={(e) => setRepoFilter(e.target.value)}>
              <option value="">全部仓库</option>
              {repos.map((repo) => (
                <option key={repo.id} value={repo.id}>{repo.label} ({repo.skill_count})</option>
              ))}
            </select>
          </div>
          <p className="hint compact">
            技能格式与 Claude / Codex 通用（SKILL.md）。安装到暂存区后，在「已暂存」筛选中可直接挂载到
            {clientTab === 'claude' ? ' ~/.claude/skills' : ' ~/.agents/skills'}。
          </p>
          {skillList(filteredCatalog)}
        </div>
      </section>
    );
  }

  if (subview === 'repos') {
    return (
      <section className="page-grid single store-subview">
        <div className="card store-card store-card-fill">
          <div className="section-title">
            <div className="store-title-row">
              <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('main')}>
                <ArrowLeft size={14} />返回
              </button>
              <h3>管理技能仓库</h3>
            </div>
          </div>
          {statusText && <StatusBanner text={statusText} />}
          <div className="store-repo-form">
            <input placeholder="owner" value={newRepo.owner} onChange={(e) => setNewRepo({ ...newRepo, owner: e.target.value })} />
            <input placeholder="repo" value={newRepo.repo} onChange={(e) => setNewRepo({ ...newRepo, repo: e.target.value })} />
            <input placeholder="branch" value={newRepo.branch} onChange={(e) => setNewRepo({ ...newRepo, branch: e.target.value })} />
            <button
              type="button"
              className="tiny"
              disabled={busy}
              onClick={() => run(async () => {
                const next = await invoke<SkillRepoView[]>('store_add_repo', newRepo);
                setRepos(next);
                setNewRepo({ owner: '', repo: '', branch: 'main' });
              }, '仓库已添加')}
            >
              添加
            </button>
          </div>
          <div className="store-scroll-panel">
            <div className="store-repo-list">
              {repos.map((repo) => (
                <div className="store-repo-row" key={repo.id}>
                  <div>
                    <strong>{repo.label}</strong>
                    <span className="hint compact">{repo.branch} · {repo.skill_count} 个技能</span>
                    {repo.last_error && <p className="hint compact store-error">{repo.last_error}</p>}
                  </div>
                  <div className="store-skill-actions">
                    <button
                      type="button"
                      className="tiny"
                      disabled={busy}
                      onClick={() => withStatus(`正在刷新 ${repo.label}…`, async () => {
                        const next = await invoke<SkillRepoView[]>('store_refresh_repo', { repoId: repo.id });
                        setRepos(next);
                        await loadCatalog();
                      }, '仓库已刷新')}
                    >
                      <RefreshCw size={14} />刷新
                    </button>
                    <button
                      type="button"
                      className="tiny danger-link"
                      disabled={busy}
                      onClick={() => run(async () => {
                        const next = await invoke<SkillRepoView[]>('store_remove_repo', { repoId: repo.id });
                        setRepos(next);
                        await loadCatalog();
                      }, '仓库已删除')}
                    >
                      删除
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </div>
          <div className="store-settings-block">
            <h4>编辑器命令</h4>
            <p className="hint compact">留空则用文件管理器打开目录；可填 code、cursor 等（后台启动，不弹控制台）。</p>
            <div className="store-repo-form">
              <input
                placeholder="例如 code 或 cursor"
                value={settings.editor_command}
                onChange={(e) => setSettings({ ...settings, editor_command: e.target.value })}
              />
              <button type="button" className="tiny" disabled={busy} onClick={() => run(async () => {
                await invoke('store_set_settings', { settings });
              }, '设置已保存')}>
                保存
              </button>
            </div>
          </div>
        </div>
      </section>
    );
  }

  return (
    <section className="page-grid single">
      <div className="card store-card store-card-fill">
        <div className="section-title">
          <h3>客户端 · 商店版</h3>
          <div className="title-actions">
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('discover')}>
              <Sparkles size={14} />发现技能
            </button>
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('repos')}>
              <BookOpen size={14} />管理仓库
            </button>
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(() => invoke('store_open_staging_dir'))}>
              <FolderOpen size={14} />暂存目录
            </button>
          </div>
        </div>

        <div className="store-client-tabs">
          <button type="button" className={clientTab === 'claude' ? 'store-tab active' : 'store-tab'} onClick={() => setClientTab('claude')}>Claude</button>
          <button type="button" className={clientTab === 'codex' ? 'store-tab active' : 'store-tab'} onClick={() => setClientTab('codex')}>Codex</button>
        </div>

        <div className="store-content-tabs">
          {CONTENT_TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              className={contentTab === tab.id ? 'store-content-tab active' : 'store-content-tab'}
              onClick={() => setContentTab(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>

        {statusText && <StatusBanner text={statusText} />}

        {contentTab === 'env' && (
          <div className="store-content-body">
            <div className="section-title store-takeover-head">
              <span className="hint">网关接管</span>
              <div className="title-actions">
                <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(async () => {
                  const next = await invoke<ClientsEnvStatus>('get_clients_env_status');
                  onClientsChange(next);
                  pushToast(next.has_issues ? '检测到接管问题' : '状态正常', next.has_issues ? 'info' : 'ok');
                })}><ShieldCheck size={14} />检查</button>
                <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(async () => {
                  const next = await invoke<ClientsEnvStatus>('repair_clients_env');
                  onClientsChange(next);
                }, '已修复')}><Wrench size={14} />修复</button>
                <button type="button" className="primary tiny-btn" disabled={busy || !status?.running} onClick={() => run(async () => {
                  const next = await invoke<ClientsEnvStatus>('install_clients_env');
                  onClientsChange(next);
                }, '接管完成')}><PlugZap size={14} />一键接管</button>
                <button type="button" className="ghost tiny-btn" disabled={busy || Boolean(status?.running)} onClick={() => run(async () => {
                  const next = await invoke<ClientsEnvStatus>('install_clients_env', { autoStart: true });
                  onClientsChange(next);
                }, '网关已启动并完成接管')}><Play size={14} />启动并接管</button>
                <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(async () => {
                  const next = await invoke<ClientsEnvStatus>('uninstall_clients_env');
                  onClientsChange(next);
                }, '已关闭接管')}>关闭接管</button>
              </div>
            </div>
            <p className="hint">{clientsHint}</p>
            {activeClient && (
              <div className="client-card store-client-panel">
                <div className="client-head">
                  <strong>{activeClient.client}</strong>
                  <ClientStatusBadge client={activeClient} listenUrl={clients?.listen_url ?? status?.listen_url ?? ''} />
                </div>
                <p className="hint compact">{activeClient.note}</p>
              </div>
            )}
          </div>
        )}

        {contentTab === 'skills' && (
          <div className="store-content-body">
            <p className="hint compact">
              技能与 Claude Code、Codex 共用 SKILL.md 格式；挂载目录：
              <code className="store-path-code">{skillsPath ?? '—'}</code>
            </p>
            <div className="store-scroll-panel">
              <div className="store-skill-list compact">
                {clientStagedSkills.length === 0 ? (
                  <div className="store-empty">暂无暂存技能。点击「发现技能」安装，或在「已暂存」页签中挂载。</div>
                ) : (
                  clientStagedSkills.map((skill) => (
                    <SkillRow
                      key={skill.id}
                      skill={skill}
                      busy={busy}
                      clientTab={clientTab}
                      onInstall={installSkill}
                      onMount={mountSkill}
                      onUnmount={unmountSkill}
                      onUninstall={uninstallSkill}
                      onOpen={openSkill}
                    />
                  ))
                )}
              </div>
            </div>
          </div>
        )}

        {contentTab === 'plugins' && pluginPanel && (
          <div className="store-content-body">
            <div className="store-guide-card">
              <div className="section-title"><h4>{pluginPanel.guide.title}</h4><Package size={16} /></div>
              <p className="hint compact">{pluginPanel.guide.summary}</p>
              <ul className="store-guide-commands">
                {pluginPanel.guide.commands.map((cmd) => (
                  <li key={cmd}><code>{cmd}</code></li>
                ))}
              </ul>
              {pluginPanel.guide.docs_url && (
                <p className="hint compact">文档：<a href={pluginPanel.guide.docs_url} target="_blank" rel="noreferrer">{pluginPanel.guide.docs_url}</a></p>
              )}
              <p className="hint compact">技能目录：<code className="store-path-code">{pluginPanel.guide.skills_path}</code></p>
              {pluginPanel.guide.plugins_path && (
                <p className="hint compact">插件目录：<code className="store-path-code">{pluginPanel.guide.plugins_path}</code></p>
              )}
            </div>
            <div className="store-scroll-panel store-scroll-short">
              {pluginPanel.items.length === 0 ? (
                <div className="store-empty">
                  {clientTab === 'codex'
                    ? 'Codex 无独立本地插件目录；扩展与技能请使用上方官方命令。'
                    : '未检测到已安装插件，请按上方说明在终端或 Claude Code 内安装。'}
                </div>
              ) : (
                <div className="store-mini-list">
                  {pluginPanel.items.map((plugin) => (
                    <div className="store-mini-row" key={plugin.path}>
                      <span>{plugin.name}</span>
                      <span className="hint compact">{plugin.source ?? plugin.path}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
