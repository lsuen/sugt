import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  ArrowLeft, Download, FolderOpen, Plus, PlugZap, RefreshCw, Search, Settings2, Trash2, Unplug,
} from 'lucide-react';
import { GithubProxyModal } from './store/GithubProxyModal';
import type { ParsedGitRepo, SkillCatalogItem, SkillRepoView, StoreSettings } from './store/types';
import { t } from './i18n';

export type MountedAgentRef = { id: string; name: string };

export type LocalSkillView = {
  id: string;
  name: string;
  description?: string | null;
  repo_id: string;
  repo_label: string;
  relative_path: string;
  mounted_agents: MountedAgentRef[];
  mounted_paths?: string[];
  is_local: boolean;
};

export type SkillAgentView = {
  id: string;
  name: string;
  skills_dir: string;
  detected: boolean;
  skills_count?: number;
};

type ToastFn = (message: string, type: 'ok' | 'error' | 'info') => void;
type SkillsTab = 'local' | 'discover';
type SkillsSubview = 'main' | 'repos';

function clipTriggerHint(text: string | null | undefined, max = 96): string | null {
  const raw = (text ?? '').trim().replace(/\s+/g, ' ');
  if (!raw) return null;
  if (raw.length <= max) return raw;
  return `${raw.slice(0, max - 1)}…`;
}

type Props = {
  busy: boolean;
  setBusy: (v: boolean) => void;
  pushToast: ToastFn;
  formatInvokeError: (error: unknown) => string;
  agentFilter?: string;
  onAgentFilterChange?: (agentId: string) => void;
};

export function SkillsPanel({
  busy, setBusy, pushToast, formatInvokeError, agentFilter = '', onAgentFilterChange,
}: Props) {
  const [tab, setTab] = useState<SkillsTab>('discover');
  const [subview, setSubview] = useState<SkillsSubview>('main');
  const [localSkills, setLocalSkills] = useState<LocalSkillView[]>([]);
  const [agents, setAgents] = useState<SkillAgentView[]>([]);
  const [catalog, setCatalog] = useState<SkillCatalogItem[]>([]);
  const [repos, setRepos] = useState<SkillRepoView[]>([]);
  const [settings, setSettings] = useState<StoreSettings>({ editor_command: '', github_proxy_prefix: '' });
  const [search, setSearch] = useState('');
  const [repoFilter, setRepoFilter] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [mountOpen, setMountOpen] = useState(false);
  const [mountMode, setMountMode] = useState<'mount' | 'unmount'>('mount');
  const [pickedAgents, setPickedAgents] = useState<Set<string>>(new Set());
  const [customPath, setCustomPath] = useState('');
  const [newSkillName, setNewSkillName] = useState('');
  const [newRepo, setNewRepo] = useState({ url: '', branch: 'main', weight: '100' });
  const [repoTestHint, setRepoTestHint] = useState<string | null>(null);
  const [parsedPreview, setParsedPreview] = useState<ParsedGitRepo | null>(null);
  const [proxyModalOpen, setProxyModalOpen] = useState(false);

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

  const reloadLocal = useCallback(async () => {
    const [skills, agentList] = await Promise.all([
      invoke<LocalSkillView[]>('store_list_local_skills'),
      invoke<SkillAgentView[]>('store_list_skill_agents'),
    ]);
    setLocalSkills(skills);
    setAgents(agentList);
  }, []);

  const reloadDiscover = useCallback(async () => {
    const [items, repoItems, storeSettings] = await Promise.all([
      invoke<SkillCatalogItem[]>('store_list_catalog', { query: null }),
      invoke<SkillRepoView[]>('store_list_repos'),
      invoke<StoreSettings>('store_get_settings'),
    ]);
    setCatalog(items);
    setRepos(repoItems);
    setSettings(storeSettings);
  }, []);

  const ensurePreferredRepo = useCallback(async () => {
    const result = await invoke<{ warmed: boolean; skipped: boolean; message: string }>(
      'store_ensure_preferred_repo',
    );
    if (result.warmed) {
      pushToast(result.message, 'ok');
      await reloadDiscover();
    } else if (!result.skipped && result.message) {
      pushToast(result.message, 'info');
      await reloadDiscover();
    }
  }, [pushToast, reloadDiscover]);

  useEffect(() => {
    reloadLocal().catch((e) => pushToast(formatInvokeError(e), 'error'));
  }, [reloadLocal, pushToast, formatInvokeError]);

  useEffect(() => {
    if (tab === 'discover' || subview === 'repos') {
      (async () => {
        try {
          if (tab === 'discover') {
            await ensurePreferredRepo();
          }
          await reloadDiscover();
        } catch (e) {
          pushToast(formatInvokeError(e), 'error');
        }
      })();
    }
  }, [tab, subview, ensurePreferredRepo, reloadDiscover, pushToast, formatInvokeError]);

  useEffect(() => {
    if (agentFilter) setTab('local');
  }, [agentFilter]);

  const filteredLocal = useMemo(() => {
    const q = search.trim().toLowerCase();
    return localSkills.filter((s) => {
      if (agentFilter && !s.mounted_agents.some((a) => a.id === agentFilter)) return false;
      if (!q) return true;
      return [s.name, s.repo_label, s.description ?? ''].join(' ').toLowerCase().includes(q);
    });
  }, [localSkills, search, agentFilter]);

  const filteredCatalog = useMemo(() => {
    const q = search.trim().toLowerCase();
    return catalog.filter((s) => {
      if (repoFilter && s.repo_id !== repoFilter) return false;
      if (s.staged) return false; // 发现页只看未入库
      if (!q) return true;
      return [s.name, s.repo_label, s.description ?? ''].join(' ').toLowerCase().includes(q);
    });
  }, [catalog, search, repoFilter]);

  const repoProblems = useMemo(
    () => repos.filter((r) => r.enabled && r.last_error),
    [repos],
  );

  const enabledRepoIds = useMemo(
    () => new Set(repos.filter((r) => r.enabled).map((r) => r.id)),
    [repos],
  );

  const toggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const openMount = (mode: 'mount' | 'unmount') => {
    if (selected.size === 0) {
      pushToast(t('skill.selectSkillsFirst'), 'info');
      return;
    }
    setMountMode(mode);
    setPickedAgents(agentFilter ? new Set([agentFilter]) : new Set());
    setCustomPath('');
    setMountOpen(true);
  };

  const confirmMount = () =>
    run(async () => {
      const skillIds = Array.from(selected);
      const agentIds = Array.from(pickedAgents);
      const path = customPath.trim();
      if (agentIds.length === 0 && !path) {
        throw new Error(t('skill.errorMountTarget'));
      }
      const payload = { skillIds, agentIds, customPath: path || null };
      if (mountMode === 'mount') {
        await invoke('store_mount_skills', payload);
      } else {
        await invoke('store_unmount_skills', payload);
      }
      setMountOpen(false);
      setSelected(new Set());
      setCustomPath('');
      await reloadLocal();
    }, mountMode === 'mount' ? t('skill.mountDone') : t('skill.unmountDone'));

  const createSkill = () =>
    run(async () => {
      await invoke('store_create_local_skill', { name: newSkillName });
      setNewSkillName('');
      await reloadLocal();
    }, t('skill.createdLocal'));

  const installSkill = (id: string) =>
    run(async () => {
      await invoke('store_install_skill', { skillId: id });
      await Promise.all([reloadLocal(), reloadDiscover()]);
    }, t('skill.installed'));

  const uninstallSkill = (id: string) =>
    run(async () => {
      await invoke('store_uninstall_skill', { skillId: id });
      setSelected((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
      await reloadLocal();
    }, t('skill.deletedLocal'));

  if (subview === 'repos') {
    return (
      <div className="skills-panel">
        <div className="card">
          <div className="section-title">
            <div className="store-title-row">
              <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('main')}>
                <ArrowLeft size={14} />{t('agent.back')}
              </button>
              <h3>{t('skill.reposConfig')}</h3>
            </div>
            <div className="title-actions">
              <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setProxyModalOpen(true)}>
                {t('skill.gitHubProxy')}
              </button>
              <button
                type="button"
                className="ghost tiny-btn"
                disabled={busy}
                onClick={() => run(async () => {
                  const next = await invoke<SkillRepoView[]>('store_refresh_all_repos');
                  setRepos(next);
                  await reloadDiscover();
                }, t('skill.reposRefreshed'))}
              >
                <RefreshCw size={14} />{t('skill.refreshAll')}
              </button>
            </div>
          </div>

          <div className="store-repo-form store-repo-form-grid">
            <input
              className="store-repo-url"
              placeholder={t('skill.gitUrlPlaceholder')}
              value={newRepo.url}
              onChange={(e) => {
                setNewRepo({ ...newRepo, url: e.target.value });
                setRepoTestHint(null);
                setParsedPreview(null);
              }}
            />
            <input placeholder={t('skill.branchPlaceholder')} value={newRepo.branch} onChange={(e) => setNewRepo({ ...newRepo, branch: e.target.value })} />
            <input placeholder={t('skill.weightPlaceholder')} value={newRepo.weight} onChange={(e) => setNewRepo({ ...newRepo, weight: e.target.value })} />
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={busy || !newRepo.url.trim()}
              onClick={() => run(async () => {
                const parsed = await invoke<ParsedGitRepo>('store_parse_repo_url', { url: newRepo.url.trim() });
                setParsedPreview(parsed);
                const hint = await invoke<string>('store_test_repo', { url: newRepo.url.trim(), branch: newRepo.branch.trim() || 'main' });
                setRepoTestHint(hint);
              })}
            >
              {t('common.test')}
            </button>
            <button
              type="button"
              className="primary tiny-btn"
              disabled={busy || !newRepo.url.trim()}
              onClick={() => run(async () => {
                const next = await invoke<SkillRepoView[]>('store_add_repo', {
                  url: newRepo.url.trim(),
                  branch: newRepo.branch.trim() || 'main',
                  weight: Number(newRepo.weight) || 100,
                });
                setRepos(next);
                setNewRepo({ url: '', branch: 'main', weight: '100' });
                setParsedPreview(null);
                setRepoTestHint(null);
              }, t('skill.repoAdded'))}
            >
              {t('common.save')}
            </button>
          </div>
          {parsedPreview && <p className="hint compact">{t('skill.parsed', { label: parsedPreview.label, url: parsedPreview.clone_url })}</p>}
          {repoTestHint && <p className="hint compact">{repoTestHint}</p>}

          <div className="takeover-profile-list" style={{ marginTop: 12 }}>
            {repos.map((repo) => (
              <div key={repo.id} className="takeover-profile-row">
                <div className="takeover-profile-row-main">
                  <div className="takeover-profile-row-title">
                    <strong>{repo.label}</strong>
                    <span className="badge inline">{t('skill.skillCount', { n: repo.skill_count })}</span>
                    {!repo.enabled && <span className="badge stop inline">{t('common.disabled')}</span>}
                  </div>
                  <span className="hint compact takeover-profile-row-meta">{t('skill.weightMeta', { url: repo.clone_url, branch: repo.branch, weight: repo.weight })}</span>
                  {repo.last_error && <span className="error-text">{repo.last_error}</span>}
                </div>
                <div className="takeover-profile-row-actions">
                  <button type="button" className="tiny ghost" disabled={busy} onClick={() => run(async () => {
                    const next = await invoke<SkillRepoView[]>('store_refresh_repo', { repoId: repo.id });
                    setRepos(next);
                  }, t('skill.toastRefreshed'))}>{t('common.refresh')}</button>
                  <button type="button" className="tiny ghost danger-link" disabled={busy} onClick={() => run(async () => {
                    const next = await invoke<SkillRepoView[]>('store_remove_repo', { repoId: repo.id });
                    setRepos(next);
                  }, t('skill.toastDeleted'))}>{t('common.delete')}</button>
                </div>
              </div>
            ))}
            {repos.length === 0 && <p className="hint compact">{t('skill.noRepos')}</p>}
          </div>
        </div>
        {proxyModalOpen && (
          <GithubProxyModal
            prefix={settings.github_proxy_prefix}
            onClose={() => setProxyModalOpen(false)}
            onSaved={() => {
              setProxyModalOpen(false);
              reloadDiscover().catch(() => undefined);
              pushToast(t('skill.proxySaved'), 'ok');
            }}
            pushToast={pushToast}
            formatError={formatInvokeError}
          />
        )}
      </div>
    );
  }

  return (
    <div className="skills-panel">
      <div className="card">
        <div className="section-title">
          <h3>{t('skill.title')}</h3>
          <div className="title-actions">
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setSubview('repos')}>
              <Settings2 size={14} />{t('skill.reposConfig')}
            </button>
            <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => run(async () => {
              await invoke('store_open_staging_dir');
            })}>
              <FolderOpen size={14} />{t('skill.localDir')}
            </button>
          </div>
        </div>

        <div className="skills-tabs">
          <button type="button" className={tab === 'local' ? 'skills-tab active' : 'skills-tab'} onClick={() => setTab('local')}>{t('skill.tabLocal')}</button>
          <button type="button" className={tab === 'discover' ? 'skills-tab active' : 'skills-tab'} onClick={() => setTab('discover')}>{t('skill.tabDiscover')}</button>
        </div>

        <div className="skills-toolbar">
          <div className="store-search">
            <Search size={16} />
            <input placeholder={t('skill.searchPlaceholder')} value={search} onChange={(e) => setSearch(e.target.value)} />
          </div>
          {tab === 'local' && (
            <select
              value={agentFilter}
              onChange={(e) => onAgentFilterChange?.(e.target.value)}
              title={t('skill.filterMountedTitle')}
            >
              <option value="">{t('skill.mountedAll')}</option>
              {agents.map((a) => (
                <option key={a.id} value={a.id}>{a.name}</option>
              ))}
            </select>
          )}
          {tab === 'discover' && (
            <select value={repoFilter} onChange={(e) => setRepoFilter(e.target.value)}>
              <option value="">{t('skill.allRepos')}</option>
              {repos.map((repo) => (
                <option key={repo.id} value={repo.id}>{repo.label}</option>
              ))}
            </select>
          )}
          {tab === 'local' && (
            <>
              <button type="button" className="primary tiny-btn" disabled={busy || selected.size === 0} onClick={() => openMount('mount')}>
                <PlugZap size={14} />{t('skill.mount')}
              </button>
              <button type="button" className="ghost tiny-btn" disabled={busy || selected.size === 0} onClick={() => openMount('unmount')}>
                <Unplug size={14} />{t('skill.unmount')}
              </button>
            </>
          )}
        </div>

        {tab === 'local' && (
          <>
            <div className="skills-create-row">
              <input
                placeholder={t('skill.newNamePlaceholder')}
                value={newSkillName}
                disabled={busy}
                onChange={(e) => setNewSkillName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && newSkillName.trim()) createSkill();
                }}
              />
              <button type="button" className="primary tiny-btn" disabled={busy || !newSkillName.trim()} onClick={() => createSkill()}>
                <Plus size={14} />{t('skill.new')}
              </button>
            </div>
            <div className="skills-list">
              {filteredLocal.map((skill) => (
                <label key={skill.id} className={`skills-row${selected.has(skill.id) ? ' selected' : ''}`}>
                  <input
                    type="checkbox"
                    checked={selected.has(skill.id)}
                    disabled={busy}
                    onChange={() => toggleSelect(skill.id)}
                  />
                  <div className="skills-row-main">
                    <div className="skills-row-title">
                      <strong>{skill.name}</strong>
                      {skill.is_local && <span className="badge inline">{t('skill.selfBuilt')}</span>}
                      {skill.mounted_agents.map((a) => (
                        <span key={a.id} className="badge ok inline skills-pill">{a.name}</span>
                      ))}
                      {(skill.mounted_paths ?? []).map((p) => (
                        <span key={p} className="badge inline skills-pill" title={p}>{t('skill.project')}</span>
                      ))}
                      {skill.mounted_agents.length === 0 && !(skill.mounted_paths?.length) && (
                        <span className="badge inline">{t('skill.notMounted')}</span>
                      )}
                    </div>
                    <span className="hint compact">{skill.repo_label}</span>
                    {clipTriggerHint(skill.description) && (
                      <span className="hint compact skills-trigger-hint" title={skill.description ?? undefined}>
                        {t('skill.applicable', { hint: clipTriggerHint(skill.description) ?? '' })}
                      </span>
                    )}
                  </div>
                  <div className="skills-row-actions">
                    <button type="button" className="tiny ghost" disabled={busy} onClick={(e) => {
                      e.preventDefault();
                      run(async () => { await invoke('store_open_skill', { skillId: skill.id, staged: true }); });
                    }}>{t('common.open')}</button>
                    <button type="button" className="tiny ghost danger-link" disabled={busy} onClick={(e) => {
                      e.preventDefault();
                      if (window.confirm(t('skill.confirmDeleteLocal', { name: skill.name }))) uninstallSkill(skill.id);
                    }}>
                      <Trash2 size={14} />
                    </button>
                  </div>
                </label>
              ))}
              {filteredLocal.length === 0 && <p className="hint compact">{t('skill.localEmpty')}</p>}
            </div>
          </>
        )}

        {tab === 'discover' && (
          <div className="skills-list">
            <div className="skills-discover-actions">
              <button
                type="button"
                className="ghost tiny-btn"
                disabled={busy}
                onClick={() => run(async () => {
                  // 发现页同步推荐仓：Gitee + Anthropic（含 frontend-design）
                  const ids = ['swlgitee-sun-skills', 'anthropics-skills'];
                  let next = repos;
                  for (const id of ids) {
                    const target = next.find((r) => r.id === id)
                      ?? (id === 'swlgitee-sun-skills'
                        ? next.find((r) => r.clone_url.toLowerCase().includes('gitee.com/swlgitee/sun-skills'))
                        : undefined);
                    if (!target) continue;
                    next = await invoke<SkillRepoView[]>('store_refresh_repo', { repoId: target.id });
                    setRepos(next);
                  }
                  await reloadDiscover();
                }, t('skill.toastRecommendedRefreshed'))}
              >
                <RefreshCw size={14} />{t('skill.refreshRecommended')}
              </button>
              <button
                type="button"
                className="ghost tiny-btn"
                disabled={busy}
                onClick={() => run(async () => {
                  const next = await invoke<SkillRepoView[]>('store_refresh_all_repos');
                  setRepos(next);
                  await reloadDiscover();
                }, t('skill.toastAllRefreshed'))}
              >
                <RefreshCw size={14} />{t('skill.refreshAll')}
              </button>
            </div>
            {repoProblems.length > 0 && (
              <p className="hint compact" style={{ color: 'var(--danger, #c44)' }}>
                {t('skill.repoProblems')}
                {repoProblems.map((r) => `${r.label}`).join('、')}
                {t('skill.repoProblemsTail')}
              </p>
            )}
            {filteredCatalog.map((skill) => (
              <div key={skill.id} className="skills-row">
                <div className="skills-row-main">
                  <div className="skills-row-title">
                    <strong>{skill.name}</strong>
                    <span className="badge inline">{skill.repo_label}</span>
                  </div>
                  {clipTriggerHint(skill.description) ? (
                    <span className="hint compact skills-trigger-hint" title={skill.description ?? undefined}>
                      {t('skill.applicable', { hint: clipTriggerHint(skill.description) ?? '' })}
                    </span>
                  ) : (
                    <span className="hint compact">{skill.relative_path}</span>
                  )}
                </div>
                <div className="skills-row-actions">
                  <button type="button" className="tiny primary" disabled={busy} onClick={() => installSkill(skill.id)}>
                    <Download size={14} />{t('skill.install')}
                  </button>
                </div>
              </div>
            ))}
            {filteredCatalog.length === 0 && (
              <p className="hint compact">
                {enabledRepoIds.size === 0
                  ? t('skill.noReposEnabled')
                  : t('skill.noDiscoverable')}
              </p>
            )}
          </div>
        )}
      </div>

      {mountOpen && (
        <div className="modal-overlay">
          <div className="modal takeover-env-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>{mountMode === 'mount' ? t('skill.mountToAgent') : t('skill.unmountTitle')}</h3>
              <button type="button" className="icon-btn" onClick={() => setMountOpen(false)} aria-label={t('common.close')}>×</button>
            </div>
            <div className="modal-body">
              <p className="hint compact">{t('skill.mountModalHint', { n: selected.size })}</p>
              <div className="skills-agent-pick">
                {agents.map((agent) => (
                  <label key={agent.id} className="skills-agent-option">
                    <input
                      type="checkbox"
                      checked={pickedAgents.has(agent.id)}
                      onChange={() => {
                        setPickedAgents((prev) => {
                          const next = new Set(prev);
                          if (next.has(agent.id)) next.delete(agent.id);
                          else next.add(agent.id);
                          return next;
                        });
                      }}
                    />
                    <span className="skills-agent-meta">
                      <strong>
                        {agent.name}
                        <span className="hint compact">
                          {' '}· {agent.detected ? t('skill.detected') : t('skill.dirNotCreated')}
                          {typeof agent.skills_count === 'number' ? t('skill.skillsCountSuffix', { n: agent.skills_count }) : ''}
                        </span>
                      </strong>
                      <code className="skills-agent-path">{agent.skills_dir}</code>
                    </span>
                  </label>
                ))}
                {agents.length === 0 && <p className="hint compact">{t('skill.noAgentsMount')}</p>}
              </div>
              <label className="field-label">
                {t('skill.customDirLabel')}
                <input
                  placeholder={t('skill.customDirPlaceholder')}
                  value={customPath}
                  onChange={(e) => setCustomPath(e.target.value)}
                />
              </label>
            </div>
            <div className="modal-actions">
              <button type="button" className="ghost" onClick={() => setMountOpen(false)}>{t('common.cancel')}</button>
              <button
                type="button"
                className="primary"
                disabled={busy || (pickedAgents.size === 0 && !customPath.trim())}
                onClick={() => confirmMount()}
              >
                {t('skill.confirm')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
