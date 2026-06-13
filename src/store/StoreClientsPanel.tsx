import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  ArrowLeft, BookOpen, Download, ExternalLink, FolderOpen, Package,
  PlugZap, Play, RefreshCw, Search, ShieldCheck, Sparkles, Trash2, Wrench,
} from 'lucide-react';
import type {
  PluginItemView,
  SkillCatalogItem,
  SkillRepoView,
  StoreClientTab,
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
          {client.issues && client.issues.length > 0 && (
            <ul className="badge-popover-issues">{client.issues.map((issue) => <li key={issue}>{issue}</li>)}</ul>
          )}
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

export function StoreClientsPanel({
  busy, setBusy, status, clients, onClientsChange, pushToast, clientsHint, formatInvokeError,
}: Props) {
  const [subview, setSubview] = useState<StoreSubview>('main');
  const [clientTab, setClientTab] = useState<StoreClientTab>('claude');
  const [catalog, setCatalog] = useState<SkillCatalogItem[]>([]);
  const [repos, setRepos] = useState<SkillRepoView[]>([]);
  const [plugins, setPlugins] = useState<PluginItemView[]>([]);
  const [settings, setSettings] = useState<StoreSettings>({ editor_command: '' });
  const [search, setSearch] = useState('');
  const [repoFilter, setRepoFilter] = useState('');
  const [newRepo, setNewRepo] = useState({ owner: '', repo: '', branch: 'main' });

  const stagedSkills = useMemo(() => catalog.filter((s) => s.staged), [catalog]);

  const loadStoreData = useCallback(async () => {
    const [catalogItems, repoItems, pluginItems, storeSettings] = await Promise.all([
      invoke<SkillCatalogItem[]>('store_list_catalog', { query: null }),
      invoke<SkillRepoView[]>('store_list_repos'),
      invoke<PluginItemView[]>('store_list_plugins'),
      invoke<StoreSettings>('store_get_settings'),
    ]);
    setCatalog(catalogItems);
    setRepos(repoItems);
    setPlugins(pluginItems);
    setSettings(storeSettings);
  }, []);

  useEffect(() => {
    loadStoreData().catch((error) => pushToast(formatInvokeError(error), 'error'));
  }, [loadStoreData, pushToast, formatInvokeError]);

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

  const refreshCatalog = async (query?: string) => {
    const items = await invoke<SkillCatalogItem[]>('store_list_catalog', { query: query ?? null });
    setCatalog(items);
  };

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
    return items;
  }, [catalog, repoFilter, search]);

  const activeClient = clientTab === 'claude' ? clients?.claude : clients?.codex;

  if (subview === 'discover') {
    return (
      <section className="page-grid single store-subview">
        <div className="card store-card">
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
                onClick={() => run(async () => {
                  const next = await invoke<SkillRepoView[]>('store_refresh_all_repos');
                  setRepos(next);
                  await refreshCatalog();
                }, '仓库已刷新')}
              >
                <RefreshCw size={14} />刷新全部仓库
              </button>
            </div>
          </div>
          <div className="store-filters">
            <div className="store-search">
              <Search size={16} />
              <input
                placeholder="搜索技能名称或描述"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <select value={repoFilter} onChange={(e) => setRepoFilter(e.target.value)}>
              <option value="">全部仓库</option>
              {repos.map((repo) => (
                <option key={repo.id} value={repo.id}>{repo.label} ({repo.skill_count})</option>
              ))}
            </select>
          </div>
          <p className="hint compact">安装到暂存区后，可在 Claude 面板「挂到 Claude」；需先刷新仓库拉取清单。</p>
          <div className="store-skill-list">
            {filteredCatalog.length === 0 && (
              <div className="store-empty">暂无技能。请先在「管理技能仓库」刷新仓库，或调整筛选条件。</div>
            )}
            {filteredCatalog.map((skill) => (
              <div className="store-skill-row" key={skill.id}>
                <div className="store-skill-main">
                  <strong>{skill.name}</strong>
                  <span className="hint compact">{skill.repo_label} · {skill.relative_path}</span>
                  {skill.description && <p className="hint compact">{skill.description}</p>}
                  <div className="store-tags">
                    {skill.staged && <span className="badge ok">已暂存</span>}
                    {skill.mounted && <span className="badge ok">已挂载</span>}
                  </div>
                </div>
                <div className="store-skill-actions">
                  {!skill.staged && (
                    <button
                      type="button"
                      className="tiny"
                      disabled={busy}
                      onClick={() => run(async () => {
                        await invoke('store_install_skill', { skillId: skill.id });
                        await refreshCatalog();
                      }, '已安装到暂存区')}
                    >
                      <Download size={14} />安装
                    </button>
                  )}
                  {skill.staged && !skill.mounted && (
                    <button
                      type="button"
                      className="tiny"
                      disabled={busy}
                      onClick={() => run(async () => {
                        await invoke('store_mount_skill', { skillId: skill.id });
                        await refreshCatalog();
                      }, '已挂到 Claude')}
                    >
                      <PlugZap size={14} />挂到 Claude
                    </button>
                  )}
                  {skill.staged && (
                    <button
                      type="button"
                      className="tiny danger-link"
                      disabled={busy}
                      onClick={() => run(async () => {
                        await invoke('store_uninstall_skill', { skillId: skill.id });
                        await refreshCatalog();
                      }, '已从暂存区移除')}
                    >
                      <Trash2 size={14} />
                    </button>
                  )}
                  {skill.mounted && (
                    <button
                      type="button"
                      className="tiny"
                      disabled={busy}
                      onClick={() => run(async () => {
                        await invoke('store_unmount_skill', { skillId: skill.id });
                        await refreshCatalog();
                      }, '已从 Claude 卸载')}
                    >
                      取消挂载
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      </section>
    );
  }

  if (subview === 'repos') {
    return (
      <section className="page-grid single store-subview">
        <div className="card store-card">
          <div className="section-title">
            <div className="store-title-row">
              <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('main')}>
                <ArrowLeft size={14} />返回
              </button>
              <h3>管理技能仓库</h3>
            </div>
          </div>
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
                    onClick={() => run(async () => {
                      const next = await invoke<SkillRepoView[]>('store_refresh_repo', { repoId: repo.id });
                      setRepos(next);
                      await refreshCatalog();
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
                      await refreshCatalog();
                    }, '仓库已删除')}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            ))}
          </div>
          <div className="store-settings-block">
            <h4>编辑器命令</h4>
            <p className="hint compact">留空则用系统文件管理器打开；可填 code、cursor 等。</p>
            <div className="store-repo-form">
              <input
                placeholder="例如 code 或 cursor"
                value={settings.editor_command}
                onChange={(e) => setSettings({ ...settings, editor_command: e.target.value })}
              />
              <button
                type="button"
                className="tiny"
                disabled={busy}
                onClick={() => run(async () => {
                  await invoke('store_set_settings', { settings });
                }, '设置已保存')}
              >
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
      <div className="card store-card">
        <div className="section-title">
          <h3>客户端 · 商店版</h3>
          <div className="title-actions">
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('discover')}>
              <Sparkles size={14} />发现技能
            </button>
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('repos')}>
              <BookOpen size={14} />管理仓库
            </button>
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(() => invoke('store_open_staging_dir'), '已打开暂存目录')}>
              <FolderOpen size={14} />暂存目录
            </button>
          </div>
        </div>

        <div className="store-client-tabs">
          <button type="button" className={clientTab === 'claude' ? 'store-tab active' : 'store-tab'} onClick={() => setClientTab('claude')}>Claude</button>
          <button type="button" className={clientTab === 'codex' ? 'store-tab active' : 'store-tab'} onClick={() => setClientTab('codex')}>Codex</button>
        </div>

        <div className="section-title store-takeover-head">
          <span className="hint">网关接管</span>
          <div className="title-actions">
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(async () => {
              const next = await invoke<ClientsEnvStatus>('get_clients_env_status');
              onClientsChange(next);
              pushToast(next.has_issues ? '检测到接管问题' : '接管状态正常', next.has_issues ? 'info' : 'ok');
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

        {clientTab === 'claude' && (
          <>
            <div className="store-section">
              <div className="section-title"><h4>插件（只读）</h4><Package size={16} /></div>
              <p className="hint compact">在线安装开发中，请使用 Claude Code 官方方式安装插件。</p>
              {plugins.length === 0 ? (
                <div className="store-empty">未检测到本地插件目录或为空</div>
              ) : (
                <div className="store-mini-list">
                  {plugins.map((plugin) => (
                    <div className="store-mini-row" key={plugin.path}>
                      <span>{plugin.name}</span>
                      <span className="hint compact">{plugin.source ?? plugin.path}</span>
                    </div>
                  ))}
                </div>
              )}
              <button type="button" className="tiny" disabled title="开发中">在线安装（开发中）</button>
            </div>

            <div className="store-section">
              <div className="section-title"><h4>暂存技能</h4><Sparkles size={16} /></div>
              {stagedSkills.length === 0 ? (
                <div className="store-empty">暂无暂存技能，点击右上角「发现技能」安装</div>
              ) : (
                <div className="store-skill-list compact">
                  {stagedSkills.map((skill) => (
                    <div className="store-skill-row" key={skill.id}>
                      <div className="store-skill-main">
                        <strong>{skill.name}</strong>
                        <span className="hint compact">{skill.repo_label}</span>
                        <div className="store-tags">
                          {skill.mounted && <span className="badge ok">已挂载</span>}
                        </div>
                      </div>
                      <div className="store-skill-actions">
                        {!skill.mounted && (
                          <button type="button" className="tiny" disabled={busy} onClick={() => run(async () => {
                            await invoke('store_mount_skill', { skillId: skill.id });
                            await refreshCatalog();
                          }, '已挂到 Claude')}>挂到 Claude</button>
                        )}
                        <button type="button" className="tiny" disabled={busy} onClick={() => run(() => invoke('store_open_skill', { skillId: skill.id, staged: true }))}>
                          <ExternalLink size={14} />打开
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </>
        )}

        {clientTab === 'codex' && (
          <div className="store-empty">Codex 技能商店能力规划中，当前仅展示环境接管状态。</div>
        )}
      </div>
    </section>
  );
}
