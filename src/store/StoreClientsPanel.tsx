import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { TakeoverProfilesSection } from '../TakeoverProfilesSection';
import { invoke } from '@tauri-apps/api/core';
import {
  ArrowLeft, BookOpen, FolderOpen, Globe, Package, PlugZap, Play, RefreshCw,
  Search, ShieldCheck, Sparkles, Terminal, Wrench,
} from 'lucide-react';
import { GithubProxyModal } from './GithubProxyModal';
import { LaunchClientModal } from './LaunchClientModal';
import { SkillRow } from './SkillRow';
import { t } from '../i18n';
import type {
  DiscoverFilter,
  MountTarget,
  PluginPanelView,
  SkillCatalogItem,
  SkillRepoView,
  ParsedGitRepo,
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
  gateway_reachable?: boolean;
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

const DISCOVER_FILTERS: { id: DiscoverFilter }[] = [
  { id: 'all' },
  { id: 'available' },
  { id: 'staged' },
  { id: 'mounted' },
];

const CONTENT_TABS: { id: StoreContentTab }[] = [
  { id: 'env' },
  { id: 'templates' },
  { id: 'skills' },
  { id: 'plugins' },
];

function ClientStatusBadge({ client, listenUrl }: { client: ClientEnvStatus; listenUrl: string }) {
  const [hover, setHover] = useState(false);
  return (
    <span className="badge-wrap" onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}>
      <span className={`badge popover-trigger ${client.configured ? 'ok' : 'stop'}`}>
        {client.configured ? t('store.takenOver') : t('store.notTakenOver')}
      </span>
      {hover && (
        <div className="badge-popover" role="tooltip">
          <div className="badge-popover-title">{client.client}</div>
          {client.variables && (
            <div className="env-vars compact">
              {Object.entries(client.variables).map(([name, value]) => (
                <div className="env-row" key={name}><span>{name}</span><code>{value || t('store.unset')}</code></div>
              ))}
            </div>
          )}
          <p className="hint compact popover-foot">{t('store.gatewayUrl', { url: listenUrl })}</p>
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
  const [settings, setSettings] = useState<StoreSettings>({ editor_command: '', github_proxy_prefix: '' });
  const [search, setSearch] = useState('');
  const [repoFilter, setRepoFilter] = useState('');
  const [newRepo, setNewRepo] = useState({ url: '', branch: 'main', weight: '100' });
  const [repoTestHint, setRepoTestHint] = useState<string | null>(null);
  const [parsedPreview, setParsedPreview] = useState<ParsedGitRepo | null>(null);
  const [statusText, setStatusText] = useState<string | null>(null);
  const [proxyModalOpen, setProxyModalOpen] = useState(false);
  const [launchModalOpen, setLaunchModalOpen] = useState(false);

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
    }, t('store.installedStaging'));

  const mountSkill = (skillId: string, target: MountTarget) =>
    run(async () => {
      await invoke('store_mount_skill', { skillId, target });
      await loadCatalog();
    }, t('store.mountDone'));

  const unmountSkill = (skillId: string, target: MountTarget) =>
    run(async () => {
      await invoke('store_unmount_skill', { skillId, target });
      await loadCatalog();
    }, t('store.unmountDone'));

  const uninstallSkill = (skillId: string) =>
    run(async () => {
      await invoke('store_uninstall_skill', { skillId });
      await loadCatalog();
    }, t('store.removedStaging'));

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
  const proxyActive = Boolean(settings.github_proxy_prefix?.trim());
  const clientLabel = clientTab === 'claude' ? 'Claude Code' : 'Codex';

  const discoverFilterLabel = (id: DiscoverFilter): string => {
    switch (id) {
      case 'all': return t('store.filterAll');
      case 'available': return t('store.filterAvailable');
      case 'staged': return t('store.filterStaged');
      case 'mounted': return t('store.filterMounted');
    }
  };

  const contentTabLabel = (id: StoreContentTab): string => {
    switch (id) {
      case 'env': return t('store.tabEnv');
      case 'templates': return t('store.tabTemplates');
      case 'skills': return t('store.tabSkills');
      case 'plugins': return t('store.tabPlugins');
    }
  };

  const copyGatewayUrl = () => {
    const url = clients?.listen_url ?? status?.listen_url;
    if (!url) {
      pushToast(t('store.gatewayUrlUnknown'), 'info');
      return;
    }
    navigator.clipboard.writeText(url).then(
      () => pushToast(t('store.gatewayUrlCopied'), 'ok'),
      () => pushToast(t('gateway.copyFailed'), 'error'),
    );
  };

  const proxyModal = proxyModalOpen ? (
    <GithubProxyModal
      prefix={settings.github_proxy_prefix}
      onClose={() => setProxyModalOpen(false)}
      onSaved={(prefix) => setSettings((s) => ({ ...s, github_proxy_prefix: prefix }))}
      pushToast={pushToast}
      formatError={formatInvokeError}
    />
  ) : null;

  const launchModal = launchModalOpen ? (
    <LaunchClientModal
      clientTab={clientTab}
      onClose={() => setLaunchModalOpen(false)}
      pushToast={pushToast}
      formatError={formatInvokeError}
    />
  ) : null;

  const skillList = (items: SkillCatalogItem[]) => (
    <div className="store-scroll-panel">
      <div className="store-skill-list">
        {items.length === 0 && (
          <div className="store-empty">
            {discoverFilter === 'staged'
              ? t('store.emptyStaged')
              : t('store.emptyNoMatch')}
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
    <section className="page-grid single store-subview in-tab">
        <div className="card store-card store-card-fill">
          <div className="section-title">
            <div className="store-title-row">
              <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('main')}>
                <ArrowLeft size={14} />{t('store.back')}
              </button>
              <h3>{t('store.discoverTitle')}</h3>
            </div>
            <div className="title-actions store-action-bar">
              <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setProxyModalOpen(true)}>
                <Globe size={14} />{t('store.githubProxy')}{proxyActive ? t('store.proxyEnabled') : ''}
              </button>
              <button
                type="button"
                className="ghost tiny-btn"
                disabled={busy}
                onClick={() => withStatus(t('store.refreshingAll'), async () => {
                  const next = await invoke<SkillRepoView[]>('store_refresh_all_repos');
                  setRepos(next);
                  await loadCatalog();
                }, t('store.reposRefreshed'))}
              >
                <RefreshCw size={14} />{t('store.refreshAll')}
              </button>
            </div>
          </div>
          {statusText && <StatusBanner text={statusText} />}
          {proxyActive && (
            <p className="hint compact">{t('store.proxyHint')}<code className="store-path-code">{settings.github_proxy_prefix.trim().replace(/\/+$/, '')}</code></p>
          )}
          <div className="store-filter-tabs">
            {DISCOVER_FILTERS.map((tab) => (
              <button
                key={tab.id}
                type="button"
                className={discoverFilter === tab.id ? 'store-tab active' : 'store-tab'}
                onClick={() => setDiscoverFilter(tab.id)}
              >
                {discoverFilterLabel(tab.id)}
              </button>
            ))}
          </div>
          <div className="store-filters">
            <div className="store-search">
              <Search size={16} />
              <input placeholder={t('store.searchPlaceholder')} value={search} onChange={(e) => setSearch(e.target.value)} />
            </div>
            <select value={repoFilter} onChange={(e) => setRepoFilter(e.target.value)}>
              <option value="">{t('store.allRepos')}</option>
              {repos.map((repo) => (
                <option key={repo.id} value={repo.id}>{repo.label} ({repo.skill_count})</option>
              ))}
            </select>
          </div>
          <p className="hint compact">
            {t('store.skillFormatHint')}
            {clientTab === 'claude' ? ' ~/.claude/skills' : ' ~/.agents/skills'}。
          </p>
          {skillList(filteredCatalog)}
        </div>
        {proxyModal}
      </section>
    );
  }

  if (subview === 'repos') {
  return (
    <section className="page-grid single store-subview in-tab">
        <div className="card store-card store-card-fill">
          <div className="section-title">
            <div className="store-title-row">
              <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('main')}>
                <ArrowLeft size={14} />{t('store.back')}
              </button>
              <h3>{t('store.manageReposTitle')}</h3>
            </div>
          </div>
          {statusText && <StatusBanner text={statusText} />}
          <div className="store-repo-form store-repo-form-grid">
            <input
              className="store-repo-url"
              placeholder={t('store.gitUrlPlaceholder')}
              value={newRepo.url}
              onChange={(e) => {
                setNewRepo({ ...newRepo, url: e.target.value });
                setRepoTestHint(null);
                setParsedPreview(null);
              }}
            />
            <input
              placeholder={t('skill.branchPlaceholder')}
              value={newRepo.branch}
              onChange={(e) => setNewRepo({ ...newRepo, branch: e.target.value })}
            />
            <input
              placeholder={t('skill.weightPlaceholder')}
              title={t('store.weightTitle')}
              value={newRepo.weight}
              onChange={(e) => setNewRepo({ ...newRepo, weight: e.target.value })}
            />
            <button
              type="button"
              className="tiny"
              disabled={busy}
              onClick={() => run(async () => {
                const parsed = await invoke<ParsedGitRepo>('store_parse_repo_url', { url: newRepo.url });
                setParsedPreview(parsed);
                const msg = await invoke<string>('store_test_repo', {
                  url: newRepo.url,
                  branch: newRepo.branch || 'main',
                });
                setRepoTestHint(msg);
              }, t('store.repoTested'))}
            >
              {t('store.testParse')}
            </button>
            <button
              type="button"
              className="tiny primary"
              disabled={busy}
              onClick={() => run(async () => {
                const weight = Number.parseInt(newRepo.weight || '0', 10);
                if (Number.isNaN(weight)) throw new Error(t('store.weightMustBeNumber'));
                const next = await invoke<SkillRepoView[]>('store_add_repo', {
                  url: newRepo.url,
                  branch: newRepo.branch || 'main',
                  weight,
                });
                setRepos(next);
                setNewRepo({ url: '', branch: 'main', weight: '100' });
                setRepoTestHint(null);
                setParsedPreview(null);
              }, t('store.repoSaved'))}
            >
              {t('common.save')}
            </button>
          </div>
          {parsedPreview && (
            <p className="hint compact store-ok-hint">
              {t('store.parsedHint', { host: parsedPreview.host, label: parsedPreview.label })}
              {parsedPreview.is_github ? t('store.parsedGithub') : t('store.parsedDirect')}
            </p>
          )}
          {repoTestHint && (
            <p className={`hint compact${repoTestHint.includes('成功') || repoTestHint.includes('可访问') ? ' store-ok-hint' : ' store-error'}`}>
              {repoTestHint}
            </p>
          )}
          <div className="store-scroll-panel">
            <div className="store-repo-list">
              {repos.map((repo) => (
                <div className="store-repo-row" key={repo.id}>
                  <div className="store-repo-meta">
                    <strong className="store-repo-name">{repo.label}</strong>
                    <span className="hint compact store-repo-stats">
                      {t('store.repoWeight', { weight: repo.weight, branch: repo.branch, count: repo.skill_count })}
                    </span>
                    <code className="store-path-code">{repo.clone_url}</code>
                    {repo.last_error && <p className="hint compact store-error">{repo.last_error}</p>}
                  </div>
                  <div className="store-skill-actions store-repo-actions">
                    <button
                      type="button"
                      className="tiny"
                      disabled={busy}
                      title={t('store.weightUpTitle')}
                      onClick={() => run(async () => {
                        const next = await invoke<SkillRepoView[]>('store_update_repo', {
                          repoId: repo.id,
                          weight: repo.weight + 10,
                        });
                        setRepos(next);
                        await loadCatalog();
                      }, t('store.weightUpdated'))}
                    >
                      {t('store.weightUp')}
                    </button>
                    <button
                      type="button"
                      className="tiny"
                      disabled={busy}
                      onClick={() => withStatus(t('store.refreshingRepo', { label: repo.label }), async () => {
                        const next = await invoke<SkillRepoView[]>('store_refresh_repo', { repoId: repo.id });
                        setRepos(next);
                        await loadCatalog();
                      }, t('store.repoRefreshed'))}
                    >
                      <RefreshCw size={14} />{t('common.refresh')}
                    </button>
                    <button
                      type="button"
                      className="tiny danger-link"
                      disabled={busy}
                      onClick={() => run(async () => {
                        const next = await invoke<SkillRepoView[]>('store_remove_repo', { repoId: repo.id });
                        setRepos(next);
                        await loadCatalog();
                      }, t('store.repoDeleted'))}
                    >
                      {t('common.delete')}
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </div>
          <div className="store-settings-block">
            <h4>{t('store.proxyBlockTitle')}</h4>
            <p className="hint compact">
              {proxyActive
                ? t('store.proxyBlockEnabled', { prefix: settings.github_proxy_prefix.trim() })
                : t('store.proxyBlockDisabled')}
            </p>
            <button type="button" className="tiny" disabled={busy} onClick={() => setProxyModalOpen(true)}>
              <Globe size={14} />{t('store.configProxy')}
            </button>
          </div>
          <div className="store-settings-block">
            <h4>{t('store.editorCommand')}</h4>
            <p className="hint compact">{t('store.editorHint')}</p>
            <div className="store-repo-form">
              <input
                placeholder={t('store.editorPlaceholder')}
                value={settings.editor_command}
                onChange={(e) => setSettings({ ...settings, editor_command: e.target.value })}
              />
              <button type="button" className="tiny" disabled={busy} onClick={() => run(async () => {
                await invoke('store_set_settings', { settings });
              }, t('store.settingsSaved'))}>
                {t('common.save')}
              </button>
            </div>
          </div>
        </div>
        {proxyModal}
      </section>
    );
  }

  return (
    <section className="page-grid single in-tab">
      <div className="card store-card store-card-fill">
        <div className="section-title">
          <h3>{t('store.title')}</h3>
          <div className="title-actions store-action-bar">
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('discover')}>
              <Sparkles size={14} />{t('store.discoverTitle')}
            </button>
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('repos')}>
              <BookOpen size={14} />{t('store.manageRepos')}
            </button>
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(() => invoke('store_open_staging_dir'))}>
              <FolderOpen size={14} />{t('store.stagingDir')}
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
              {contentTabLabel(tab.id)}
            </button>
          ))}
        </div>

        {statusText && <StatusBanner text={statusText} />}

        {contentTab === 'env' && (
          <div className="store-content-body">
            <div className="store-env-grid">
              <div className="store-env-stat">
                <strong>{t('store.gateway')}</strong>
                <span className={status?.running ? 'badge ok' : 'badge stop'}>
                  {status?.running ? t('common.running') : t('store.notStarted')}
                </span>
              </div>
              <div className="store-env-stat">
                <strong>{t('store.takeoverStatus', { client: clientLabel })}</strong>
                <span className={activeClient?.configured ? 'badge ok' : 'badge stop'}>
                  {activeClient?.configured ? t('store.takenOver') : t('store.notTakenOver')}
                </span>
              </div>
              <div className="store-env-stat">
                <strong>{t('store.proxyStatus')}</strong>
                <span className={proxyActive ? 'badge ok' : 'badge'}>{proxyActive ? t('common.enabled') : t('store.direct')}</span>
              </div>
            </div>
            <div className="section-title store-takeover-head">
              <span className="hint">{t('store.takeoverHint', { client: clientLabel })}</span>
              <div className="title-actions store-action-bar">
                <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(async () => {
                  const next = await invoke<ClientsEnvStatus>('get_clients_env_status');
                  onClientsChange(next);
                  pushToast(next.has_issues ? t('store.toastIssues') : t('store.toastOk'), next.has_issues ? 'info' : 'ok');
                })}><ShieldCheck size={14} />{t('store.check')}</button>
                <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(async () => {
                  const next = await invoke<ClientsEnvStatus>('repair_clients_env');
                  onClientsChange(next);
                }, t('store.repaired'))}><Wrench size={14} />{t('client.repair')}</button>
                {status?.running ? (
                  <button type="button" className="primary tiny-btn" disabled={busy} onClick={() => run(async () => {
                    const next = await invoke<ClientsEnvStatus>('install_clients_env', { client: clientTab });
                    onClientsChange(next);
                  }, t('store.takenOverToast', { client: clientLabel }))}><PlugZap size={14} />{t('client.takeover')}</button>
                ) : (
                  <button type="button" className="primary tiny-btn" disabled={busy} onClick={() => run(async () => {
                    const next = await invoke<ClientsEnvStatus>('install_clients_env', { autoStart: true, client: clientTab });
                    onClientsChange(next);
                  }, t('store.startedAndTakenOver', { client: clientLabel }))}><Play size={14} />{t('store.startAndTakeover')}</button>
                )}
                <button type="button" className="ghost tiny-btn" disabled={busy || !activeClient?.configured} onClick={() => run(async () => {
                  const next = await invoke<ClientsEnvStatus>('uninstall_clients_env', { client: clientTab });
                  onClientsChange(next);
                }, t('store.cancelledTakeover', { client: clientLabel }))}>{t('store.cancelTakeover')}</button>
              </div>
            </div>
            {clientsHint ? <p className="hint">{clientsHint}</p> : null}
            <div className="store-env-actions">
              <button type="button" className="tiny" disabled={busy} onClick={() => setLaunchModalOpen(true)}>
                <Terminal size={14} />{t('store.newTerminal', { client: clientLabel })}
              </button>
              <button type="button" className="tiny" disabled={busy} onClick={() => copyGatewayUrl()}>
                {t('store.copyGatewayUrl')}
              </button>
              <button type="button" className="tiny" disabled={busy} onClick={() => run(() => invoke('store_open_staging_dir'))}>
                <FolderOpen size={14} />{t('store.openStagingDir')}
              </button>
            </div>
            {activeClient && (
              <div className="client-card store-client-panel">
                <div className="client-head">
                  <strong>{activeClient.client}</strong>
                  <ClientStatusBadge client={activeClient} listenUrl={clients?.listen_url ?? status?.listen_url ?? ''} />
                </div>
                <p className="hint compact">{activeClient.note}</p>
                {activeClient.variables && (
                  <div className="env-vars compact store-env-vars">
                    {Object.entries(activeClient.variables).map(([name, value]) => (
                      <div className="env-row" key={name}>
                        <span>{name}</span>
                        <code>{value}</code>
                      </div>
                    ))}
                  </div>
                )}
                {activeClient.missing && activeClient.missing.length > 0 && (
                  <p className="hint compact store-error">{t('store.pendingWrite', { list: activeClient.missing.join('、') })}</p>
                )}
              </div>
            )}
          </div>
        )}

        {contentTab === 'templates' && (
          <div className="store-content-body store-templates-body">
            <TakeoverProfilesSection busy={busy} run={run} pushToast={pushToast} onClientsChange={onClientsChange} />
          </div>
        )}

        {contentTab === 'skills' && (
          <div className="store-content-body">
            <p className="hint compact">
              {t('store.skillsSharedHint')}
              <code className="store-path-code">{skillsPath ?? '—'}</code>
            </p>
            <div className="store-scroll-panel">
              <div className="store-skill-list compact">
                {clientStagedSkills.length === 0 ? (
                  <div className="store-empty">{t('store.noStagedSkills')}</div>
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
                <p className="hint compact">{t('store.pluginsDoc')}<a href={pluginPanel.guide.docs_url} target="_blank" rel="noreferrer">{pluginPanel.guide.docs_url}</a></p>
              )}
              <p className="hint compact">{t('store.skillsDirLabel')}<code className="store-path-code">{pluginPanel.guide.skills_path}</code></p>
              {pluginPanel.guide.plugins_path && (
                <p className="hint compact">{t('store.pluginsDirLabel')}<code className="store-path-code">{pluginPanel.guide.plugins_path}</code></p>
              )}
            </div>
            <div className="store-scroll-panel store-scroll-short">
              {pluginPanel.items.length === 0 ? (
                <div className="store-empty">
                  {clientTab === 'codex'
                    ? t('store.noPluginsCodex')
                    : t('store.noPluginsClaude')}
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
      {proxyModal}
      {launchModal}
    </section>
  );
}
