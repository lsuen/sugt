import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, Download, RefreshCw } from 'lucide-react';
import { t } from './i18n';

export type AgentAdvancedSkill = {
  folder: string;
  name: string;
  description?: string | null;
  path: string;
  sugt_mark?: string | null;
  sugt_skill_id?: string | null;
  in_library: boolean;
};

export type AgentAdvancedPlugin = {
  name: string;
  path: string;
  source?: string | null;
};

export type AgentAdvancedConfig = {
  path?: string | null;
  exists: boolean;
  content?: string | null;
  truncated: boolean;
  error?: string | null;
  candidates: string[];
};

export type AgentAdvancedView = {
  profile_id: string;
  profile_name: string;
  settings_root?: string | null;
  settings_root_exists: boolean;
  skills_dir?: string | null;
  skills_dir_exists: boolean;
  skills_dirs?: string[];
  skill_dirs_text?: string;
  plugins_dir?: string | null;
  plugins_dir_exists: boolean;
  skills: AgentAdvancedSkill[];
  plugins: AgentAdvancedPlugin[];
  config: AgentAdvancedConfig;
};

type Tab = 'skills' | 'plugins' | 'config';

type Props = {
  profileId: string;
  profileName: string;
  busy: boolean;
  onBack: () => void;
  onOpenSkillsHub?: (agentId: string) => void;
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void;
};

function pathHint(tab: Tab, data: AgentAdvancedView | null) {
  if (!data) return null;
  if (tab === 'skills') {
    const dirs = data.skills_dirs?.length
      ? data.skills_dirs
      : data.skills_dir
        ? [data.skills_dir]
        : [];
    return {
      path: dirs.length ? dirs.join(' · ') : null,
      exists: dirs.length > 0,
      label: dirs.length > 1 ? t('agent.skillsDirCount', { n: dirs.length }) : t('agent.skillsDir'),
    };
  }
  if (tab === 'plugins') {
    return {
      path: data.plugins_dir,
      exists: data.plugins_dir_exists,
      label: t('agent.pluginsDir'),
    };
  }
  return {
    path: data.config.path || data.settings_root,
    exists: data.config.exists || data.settings_root_exists,
    label: t('agent.config'),
  };
}

export function AgentAdvancedPanel({
  profileId,
  profileName,
  busy,
  onBack,
  onOpenSkillsHub,
  pushToast,
}: Props) {
  const [tab, setTab] = useState<Tab>('skills');
  const [data, setData] = useState<AgentAdvancedView | null>(null);
  const [loading, setLoading] = useState(false);
  const [pathDraft, setPathDraft] = useState('');
  const [skillDirsDraft, setSkillDirsDraft] = useState('');
  const [savingPath, setSavingPath] = useState(false);
  const [importing, setImporting] = useState<string | null>(null);

  const reload = async () => {
    setLoading(true);
    try {
      const next = await invoke<AgentAdvancedView>('get_agent_advanced', { profileId });
      setData(next);
      setPathDraft(next.settings_root?.trim() || '');
      setSkillDirsDraft(
        next.skill_dirs_text?.trim()
          || (next.skills_dirs?.length ? next.skills_dirs.join(';') : ''),
      );
    } catch (e) {
      pushToast(String(e), 'error');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId]);

  const savePath = async () => {
    setSavingPath(true);
    try {
      const next = await invoke<AgentAdvancedView>('save_agent_advanced_path', {
        profileId,
        path: pathDraft.trim() || null,
        skillDirs: skillDirsDraft,
      });
      setData(next);
      setPathDraft(next.settings_root?.trim() || '');
      setSkillDirsDraft(next.skill_dirs_text?.trim() || '');
      pushToast(t('agent.toastSaved'), 'ok');
    } catch (e) {
      pushToast(String(e), 'error');
    } finally {
      setSavingPath(false);
    }
  };

  const importSkill = async (skill: AgentAdvancedSkill) => {
    if (skill.in_library) {
      pushToast(t('agent.toastInLibrary'), 'info');
      return;
    }
    setImporting(skill.path);
    try {
      await invoke('import_agent_skill', { sourcePath: skill.path });
      pushToast(t('agent.toastImported', { name: skill.name }), 'ok');
      await reload();
    } catch (e) {
      pushToast(String(e), 'error');
    } finally {
      setImporting(null);
    }
  };

  const hint = pathHint(tab, data);

  return (
    <div className="agent-advanced">
      <div className="section-title takeover-section-head">
        <div className="agent-advanced-title">
          <button type="button" className="ghost tiny-btn" disabled={busy || savingPath} onClick={onBack}>
            <ArrowLeft size={14} />{t('agent.back')}
          </button>
          <h3>{t('agent.advanced', { name: profileName })}</h3>
        </div>
        <div className="title-actions">
          {onOpenSkillsHub && (
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={busy}
              onClick={() => onOpenSkillsHub(profileId)}
            >
              {t('agent.skillsHub')}
            </button>
          )}
          <button type="button" className="ghost tiny-btn" disabled={busy || loading} onClick={() => void reload()}>
            <RefreshCw size={14} />{t('agent.refresh')}
          </button>
        </div>
      </div>

      <div className="skills-tabs agent-advanced-tabs">
        <button type="button" className={tab === 'skills' ? 'skills-tab active' : 'skills-tab'} onClick={() => setTab('skills')}>
          Skills{data ? ` (${data.skills.length})` : ''}
        </button>
        <button type="button" className={tab === 'plugins' ? 'skills-tab active' : 'skills-tab'} onClick={() => setTab('plugins')}>
          {t('agent.pluginsTab')}{data ? ` (${data.plugins.length})` : ''}
        </button>
        <button type="button" className={tab === 'config' ? 'skills-tab active' : 'skills-tab'} onClick={() => setTab('config')}>
          {t('agent.configTab')}
        </button>
      </div>

      <div className="agent-advanced-pathbar">
        {hint?.path && hint.exists ? (
          <p className="hint compact agent-advanced-abspath" title={hint.path}>
            {hint.label}：{hint.path}
          </p>
        ) : (
          <p className="hint compact">{t('agent.notFound', { label: hint?.label ?? t('agent.dir') })}</p>
        )}
        <label className="field-label">
          {t('agent.rootDir')}
          <input
            placeholder={t('agent.rootPlaceholder')}
            value={pathDraft}
            disabled={savingPath || busy}
            onChange={(e) => setPathDraft(e.target.value)}
          />
        </label>
        <label className="field-label">
          {t('agent.skillsDirLabel')}
          <input
            placeholder={t('agent.skillsPlaceholder')}
            value={skillDirsDraft}
            disabled={savingPath || busy}
            onChange={(e) => setSkillDirsDraft(e.target.value)}
          />
        </label>
        <p className="hint compact">{t('agent.skillsHint')}</p>
        <div className="agent-advanced-path-edit">
          <button type="button" className="primary tiny-btn" disabled={savingPath || busy} onClick={() => void savePath()}>
            {savingPath ? t('agent.savingPath') : t('agent.savePath')}
          </button>
        </div>
      </div>

      {loading && !data && <p className="hint compact">{t('agent.loading')}</p>}

      {tab === 'skills' && data && (
        <div className="agent-advanced-list">
          {data.skills.length === 0 && <p className="hint compact">{t('agent.noSkills')}</p>}
          {data.skills.map((skill) => (
            <div key={skill.path} className="agent-advanced-row">
              <div className="agent-advanced-row-main">
                <div className="agent-advanced-row-title">
                  <strong>{skill.name}</strong>
                  {skill.sugt_mark && (
                    <span className="badge ok inline" title={skill.sugt_mark === 'mounted' ? t('agent.mountedBy') : t('agent.inLibrary')}>
                      sugt
                    </span>
                  )}
                </div>
                {skill.description && <span className="hint compact">{skill.description}</span>}
                <code className="takeover-env-path">{skill.path}</code>
              </div>
              <button
                type="button"
                className="tiny ghost"
                disabled={busy || skill.in_library || importing === skill.path}
                title={skill.in_library ? t('agent.inLibrary') : t('agent.copyToLibrary')}
                onClick={() => void importSkill(skill)}
              >
                <Download size={14} />
                {skill.in_library ? t('agent.imported') : importing === skill.path ? t('agent.importing') : t('agent.import')}
              </button>
            </div>
          ))}
        </div>
      )}

      {tab === 'plugins' && data && (
        <div className="agent-advanced-list">
          {data.plugins.length === 0 && <p className="hint compact">{t('agent.noPlugins')}</p>}
          {data.plugins.map((p) => (
            <div key={p.path} className="agent-advanced-row">
              <div className="agent-advanced-row-main">
                <strong>{p.name}</strong>
                {p.source && <span className="hint compact">{p.source}</span>}
                <code className="takeover-env-path">{p.path}</code>
              </div>
            </div>
          ))}
        </div>
      )}

      {tab === 'config' && data && (
        <div className="agent-advanced-config">
          {data.config.error && !data.config.content && (
            <p className="hint compact">{data.config.error}</p>
          )}
          {data.config.path && (
            <p className="hint compact agent-advanced-abspath">{t('agent.filePrefix')}{data.config.path}</p>
          )}
          {data.config.content ? (
            <pre className="agent-advanced-config-pre">{data.config.content}</pre>
          ) : (
            data.config.candidates.length > 0 && (
              <div className="hint compact">
                {t('agent.tryPaths')}
                <ul className="agent-advanced-candidates">
                  {data.config.candidates.map((c) => (
                    <li key={c}>{c}</li>
                  ))}
                </ul>
              </div>
            )
          )}
        </div>
      )}
    </div>
  );
}
