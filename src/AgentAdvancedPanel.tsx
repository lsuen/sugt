import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, Download, RefreshCw } from 'lucide-react';

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
      label: dirs.length > 1 ? `技能目录(${dirs.length})` : '技能目录',
    };
  }
  if (tab === 'plugins') {
    return {
      path: data.plugins_dir,
      exists: data.plugins_dir_exists,
      label: '插件目录',
    };
  }
  return {
    path: data.config.path || data.settings_root,
    exists: data.config.exists || data.settings_root_exists,
    label: '配置',
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
      pushToast('已保存路径', 'ok');
    } catch (e) {
      pushToast(String(e), 'error');
    } finally {
      setSavingPath(false);
    }
  };

  const importSkill = async (skill: AgentAdvancedSkill) => {
    if (skill.in_library) {
      pushToast('已在中控库中', 'info');
      return;
    }
    setImporting(skill.path);
    try {
      await invoke('import_agent_skill', { sourcePath: skill.path });
      pushToast(`已入库「${skill.name}」`, 'ok');
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
            <ArrowLeft size={14} />返回
          </button>
          <h3>{profileName} · 高级</h3>
        </div>
        <div className="title-actions">
          {onOpenSkillsHub && (
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={busy}
              onClick={() => onOpenSkillsHub(profileId)}
            >
              技能中控
            </button>
          )}
          <button type="button" className="ghost tiny-btn" disabled={busy || loading} onClick={() => void reload()}>
            <RefreshCw size={14} />刷新
          </button>
        </div>
      </div>

      <div className="skills-tabs agent-advanced-tabs">
        <button type="button" className={tab === 'skills' ? 'skills-tab active' : 'skills-tab'} onClick={() => setTab('skills')}>
          Skills{data ? ` (${data.skills.length})` : ''}
        </button>
        <button type="button" className={tab === 'plugins' ? 'skills-tab active' : 'skills-tab'} onClick={() => setTab('plugins')}>
          插件{data ? ` (${data.plugins.length})` : ''}
        </button>
        <button type="button" className={tab === 'config' ? 'skills-tab active' : 'skills-tab'} onClick={() => setTab('config')}>
          配置
        </button>
      </div>

      <div className="agent-advanced-pathbar">
        {hint?.path && hint.exists ? (
          <p className="hint compact agent-advanced-abspath" title={hint.path}>
            {hint.label}：{hint.path}
          </p>
        ) : (
          <p className="hint compact">未找到{hint?.label || '目录'}，可在下方配置</p>
        )}
        <label className="field-label">
          Agent 根目录
          <input
            placeholder="例如 ~/.claude 或 ~/.config/opencode"
            value={pathDraft}
            disabled={savingPath || busy}
            onChange={(e) => setPathDraft(e.target.value)}
          />
        </label>
        <label className="field-label">
          技能目录（多个用 ; 分隔）
          <input
            placeholder="例如 ~/.claude/skills;~/.dev-agents/skills"
            value={skillDirsDraft}
            disabled={savingPath || busy}
            onChange={(e) => setSkillDirsDraft(e.target.value)}
          />
        </label>
        <p className="hint compact">留空技能目录则按该 Agent 内置指纹自动发现；保存后以手动列表为准，内置源仍会合并存在的目录。</p>
        <div className="agent-advanced-path-edit">
          <button type="button" className="primary tiny-btn" disabled={savingPath || busy} onClick={() => void savePath()}>
            {savingPath ? '保存中…' : '保存路径'}
          </button>
        </div>
      </div>

      {loading && !data && <p className="hint compact">加载中…</p>}

      {tab === 'skills' && data && (
        <div className="agent-advanced-list">
          {data.skills.length === 0 && <p className="hint compact">该目录下暂无技能（需含 SKILL.md）</p>}
          {data.skills.map((skill) => (
            <div key={skill.path} className="agent-advanced-row">
              <div className="agent-advanced-row-main">
                <div className="agent-advanced-row-title">
                  <strong>{skill.name}</strong>
                  {skill.sugt_mark && (
                    <span className="badge ok inline" title={skill.sugt_mark === 'mounted' ? '由 SUGT 挂载' : '已在中控库'}>
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
                title={skill.in_library ? '已在中控库' : '复制到 SUGT 技能库'}
                onClick={() => void importSkill(skill)}
              >
                <Download size={14} />
                {skill.in_library ? '已入库' : importing === skill.path ? '入库中…' : '入库'}
              </button>
            </div>
          ))}
        </div>
      )}

      {tab === 'plugins' && data && (
        <div className="agent-advanced-list">
          {data.plugins.length === 0 && <p className="hint compact">未发现插件目录或目录为空</p>}
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
            <p className="hint compact agent-advanced-abspath">文件：{data.config.path}</p>
          )}
          {data.config.content ? (
            <pre className="agent-advanced-config-pre">{data.config.content}</pre>
          ) : (
            data.config.candidates.length > 0 && (
              <div className="hint compact">
                尝试路径：
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
