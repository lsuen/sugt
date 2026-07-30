import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FileSearch, FolderOpen, Pencil, Plus, PlugZap, Search, Settings2, ShieldCheck, Trash2, Unplug, Wrench } from 'lucide-react';
import { TakeoverProfileModal } from './TakeoverProfileModal';
import { AgentDiscoverModal } from './AgentDiscoverModal';
import { AgentAdvancedPanel } from './AgentAdvancedPanel';

export type TakeoverProfileView = {
  id: string;
  name: string;
  vendor: string;
  description: string;
  protocol: string;
  configured: boolean;
  cli_detected?: boolean;
  settings_detected: boolean;
  discovered?: boolean;
  skills_detected: boolean;
  settings_path?: string | null;
  cli_path?: string | null;
  env_vars?: Record<string, string>;
  clear_vars?: string[];
  settings_dirs?: string[];
  skill_dirs?: string[];
  launch_command?: string | null;
  builtin: boolean;
  can_delete?: boolean;
  skills_count?: number;
  plugins_count?: number;
};

type ClientsEnvStatus = {
  has_issues?: boolean;
  claude?: { issues?: string[] };
  codex?: { issues?: string[] };
};

type Props = {
  busy: boolean;
  run: (fn: () => Promise<void>, ok?: string) => void;
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  onClientsChange?: (next: any) => void;
  onOpenSkills?: (agentId: string) => void;
};

function isDiscovered(p: TakeoverProfileView) {
  if (typeof p.discovered === 'boolean') return p.discovered;
  return Boolean(p.cli_detected || p.settings_detected);
}

export function TakeoverProfilesSection({ busy, run, pushToast, onClientsChange, onOpenSkills }: Props) {
  const [profiles, setProfiles] = useState<TakeoverProfileView[]>([]);
  const [aiEnabled, setAiEnabled] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [modalMode, setModalMode] = useState<'custom' | 'import' | 'edit'>('custom');
  const [editing, setEditing] = useState<TakeoverProfileView | null>(null);
  const [discoverOpen, setDiscoverOpen] = useState(false);
  const [advancedId, setAdvancedId] = useState<string | null>(null);
  const [envTarget, setEnvTarget] = useState<TakeoverProfileView | null>(null);
  const [envPathDraft, setEnvPathDraft] = useState('');
  const [envSaving, setEnvSaving] = useState(false);

  const reload = () =>
    invoke<TakeoverProfileView[]>('list_takeover_profiles').then(setProfiles).catch(() => undefined);

  useEffect(() => {
    reload();
    invoke<boolean>('can_ai_parse_takeover').then(setAiEnabled).catch(() => undefined);
  }, []);

  const openAdd = () => {
    setEditing(null);
    setModalMode('custom');
    setModalOpen(true);
  };

  const openImport = () => {
    setEditing(null);
    setModalMode('import');
    setModalOpen(true);
  };

  const openEdit = (p: TakeoverProfileView) => {
    setEditing(p);
    setModalMode('edit');
    setModalOpen(true);
  };

  const openEnv = (p: TakeoverProfileView) => {
    setEnvTarget(p);
    setEnvPathDraft(p.settings_path?.trim() || '');
  };

  const apply = (id: string, name: string) =>
    run(async () => {
      await invoke('apply_takeover_profile', { profileId: id, autoStart: true });
      await reload();
    }, `已接管 ${name}`);

  const release = (id: string, name: string) =>
    run(async () => {
      await invoke('release_takeover_profile', { profileId: id });
      await reload();
    }, `已取消 ${name}`);

  const remove = (p: TakeoverProfileView) => {
    if (!window.confirm(`确定删除「${p.name}」？`)) return;
    run(async () => {
      await invoke('delete_takeover_profile', { profileId: p.id });
      await reload();
    }, `已删除 ${p.name}`);
  };

  const checkEnv = () =>
    run(async () => {
      const next = await invoke<ClientsEnvStatus>('get_clients_env_status');
      onClientsChange?.(next);
      await reload();
      const firstIssue =
        next.claude?.issues?.[0] || next.codex?.issues?.[0] || null;
      if (firstIssue) {
        pushToast(firstIssue, 'info');
      } else {
        pushToast('环境正常', 'ok');
      }
    });

  const repairEnv = () =>
    run(async () => {
      const next = await invoke<ClientsEnvStatus>('repair_clients_env');
      onClientsChange?.(next);
      await reload();
    }, '已修复，请新开终端');

  const saveEnvPath = async () => {
    if (!envTarget) return;
    setEnvSaving(true);
    try {
      const path = envPathDraft.trim();
      await invoke('set_takeover_settings_path', {
        profileId: envTarget.id,
        path: path || null,
      });
      pushToast(path ? '已保存配置目录' : '已恢复自动检测', 'ok');
      setEnvTarget(null);
      await reload();
    } catch (e) {
      pushToast(String(e), 'error');
    } finally {
      setEnvSaving(false);
    }
  };

  const advancedProfile = advancedId
    ? profiles.find((p) => p.id === advancedId) ?? null
    : null;

  if (advancedProfile) {
    return (
      <AgentAdvancedPanel
        profileId={advancedProfile.id}
        profileName={advancedProfile.name}
        busy={busy}
        onBack={() => {
          setAdvancedId(null);
          reload();
        }}
        onOpenSkillsHub={onOpenSkills}
        pushToast={pushToast}
      />
    );
  }

  return (
    <div className="takeover-profiles">
      <div className="section-title takeover-section-head">
        <h3>Agent</h3>
        <div className="title-actions">
          <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => checkEnv()}>
            <ShieldCheck size={14} />检查
          </button>
          <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => repairEnv()}>
            <Wrench size={14} />修复
          </button>
          <button type="button" className="ghost tiny-btn" disabled={busy || !aiEnabled} title={!aiEnabled ? '需先配置大模型' : undefined} onClick={() => openImport()}>
            <FileSearch size={14} />导入
          </button>
          <button type="button" className="ghost tiny-btn" disabled={busy} onClick={() => setDiscoverOpen(true)}>
            <Search size={14} />发现
          </button>
          <button type="button" className="primary tiny-btn" disabled={busy} onClick={() => openAdd()}>
            <Plus size={14} />添加
          </button>
        </div>
      </div>

      <div className="takeover-profile-list">
        {profiles.map((p) => (
          <TakeoverProfileRow
            key={p.id}
            profile={p}
            busy={busy}
            onApply={() => apply(p.id, p.name)}
            onRelease={() => release(p.id, p.name)}
            onEdit={() => openEdit(p)}
            onDelete={() => remove(p)}
            onEnv={() => openEnv(p)}
            onAdvanced={() => setAdvancedId(p.id)}
          />
        ))}
        {profiles.length === 0 && <p className="hint compact">暂无 Agent，点击「添加」。</p>}
      </div>

      <TakeoverProfileModal
        open={modalOpen}
        mode={modalMode}
        busy={busy}
        aiEnabled={aiEnabled}
        initial={editing}
        onClose={() => {
          setModalOpen(false);
          setEditing(null);
        }}
        onSaved={() => {
          reload();
          pushToast(
            modalMode === 'import' ? '已导入' : modalMode === 'edit' ? '已保存' : '已添加',
            'ok',
          );
        }}
        onError={(msg) => pushToast(msg, 'error')}
      />

      <AgentDiscoverModal
        open={discoverOpen}
        busy={busy}
        onClose={() => setDiscoverOpen(false)}
        onAdded={(count) => {
          reload();
          pushToast(`已添加 ${count} 个 Agent`, 'ok');
        }}
        onError={(msg) => pushToast(msg, 'error')}
      />

      {envTarget && (
        <div className="modal-overlay">
          <div className="modal takeover-env-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>{envTarget.name} · 环境</h3>
              <button type="button" className="icon-btn" onClick={() => setEnvTarget(null)} aria-label="关闭">×</button>
            </div>
            <div className="modal-body">
              <div className="takeover-env-grid">
                <div className="takeover-env-stat">
                  <span className="hint compact">CLI</span>
                  <span className={envTarget.cli_detected ? 'badge ok inline' : 'badge inline'}>
                    {envTarget.cli_detected ? '已发现' : '未发现'}
                  </span>
                  {envTarget.cli_path ? (
                    <code className="takeover-env-path">{envTarget.cli_path}</code>
                  ) : envTarget.launch_command ? (
                    <span className="hint compact">查找：{envTarget.launch_command}</span>
                  ) : null}
                </div>
                <div className="takeover-env-stat">
                  <span className="hint compact">配置目录</span>
                  <span className={envTarget.settings_detected ? 'badge ok inline' : 'badge inline'}>
                    {envTarget.settings_detected ? '已发现' : '未发现'}
                  </span>
                  {envTarget.settings_path ? (
                    <code className="takeover-env-path">{envTarget.settings_path}</code>
                  ) : (
                    <span className="hint compact">未找到默认目录</span>
                  )}
                </div>
              </div>
              <div className="takeover-env-summary">
                <span className="hint compact">
                  技能目录 {envTarget.skills_count ?? 0} 项
                  {(envTarget.plugins_count ?? 0) > 0 ? ` · 插件 ${envTarget.plugins_count}` : ''}
                </span>
                {onOpenSkills && (
                  <button
                    type="button"
                    className="tiny ghost"
                    disabled={busy || envSaving}
                    onClick={() => {
                      const id = envTarget.id;
                      setEnvTarget(null);
                      onOpenSkills(id);
                    }}
                  >
                    在技能页查看
                  </button>
                )}
              </div>
              <label className="field-label">
                手动指定配置目录（可选）
                <input
                  placeholder="留空则自动检测，如 ~/.claude"
                  value={envPathDraft}
                  disabled={envSaving}
                  onChange={(e) => setEnvPathDraft(e.target.value)}
                />
              </label>
            </div>
            <div className="modal-actions">
              <button type="button" className="ghost" disabled={envSaving} onClick={() => setEnvTarget(null)}>取消</button>
              <button type="button" className="ghost" disabled={envSaving} onClick={() => setEnvPathDraft('')}>清空</button>
              <button type="button" className="primary" disabled={envSaving} onClick={() => void saveEnvPath()}>保存</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function TakeoverProfileRow({
  profile: p,
  busy,
  onApply,
  onRelease,
  onEdit,
  onDelete,
  onEnv,
  onAdvanced,
}: {
  profile: TakeoverProfileView;
  busy: boolean;
  onApply: () => void;
  onRelease: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onEnv: () => void;
  onAdvanced: () => void;
}) {
  const discovered = isDiscovered(p);
  const metaParts = [
    p.vendor,
    p.protocol,
    discovered && p.settings_path ? shortPath(p.settings_path) : null,
    p.skills_detected ? 'Skill' : null,
  ].filter(Boolean);

  return (
    <div className={`takeover-profile-row${p.configured ? ' configured' : ''}${discovered ? ' discovered' : ''}`}>
      <div className="takeover-profile-row-main">
        <div className="takeover-profile-row-title">
          <strong>{p.name}</strong>
          <span className={p.configured ? 'badge ok inline' : 'badge inline'}>{p.configured ? '已接管' : '未接管'}</span>
          <span className={discovered ? 'badge ok inline' : 'badge inline'}>{discovered ? '已发现' : '未发现'}</span>
        </div>
        <span className="hint compact takeover-profile-row-meta">{metaParts.join(' · ')}</span>
      </div>
      <div className="takeover-profile-row-actions">
        <button
          type="button"
          className={`tiny ghost takeover-env-btn${discovered ? ' detected' : ''}`}
          disabled={busy}
          title="查看安装与配置目录"
          onClick={() => onEnv()}
        >
          <FolderOpen size={14} />环境
        </button>
        <button type="button" className="tiny ghost" disabled={busy} title="高级：技能 / 插件 / 配置" onClick={() => onAdvanced()}>
          <Settings2 size={14} />高级
        </button>
        <button type="button" className="tiny ghost" disabled={busy} title="编辑" onClick={() => onEdit()}>
          <Pencil size={14} />
        </button>
        <button type="button" className="tiny ghost danger-link" disabled={busy} title="删除" onClick={() => onDelete()}>
          <Trash2 size={14} />
        </button>
        {p.configured ? (
          <button type="button" className="tiny ghost takeover-enable-btn" disabled={busy} onClick={() => onRelease()}>
            <Unplug size={14} />取消接管
          </button>
        ) : (
          <button type="button" className="tiny primary takeover-enable-btn" disabled={busy} onClick={() => onApply()}>
            <PlugZap size={14} />接管
          </button>
        )}
      </div>
    </div>
  );
}

function shortPath(path: string) {
  const normalized = path.replace(/\\/g, '/');
  const parts = normalized.split('/').filter(Boolean);
  if (parts.length <= 3) return path;
  return `…/${parts.slice(-2).join('/')}`;
}
