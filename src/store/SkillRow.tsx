import React from 'react';
import { Download, ExternalLink, PlugZap, Trash2 } from 'lucide-react';
import type { MountTarget, SkillCatalogItem, StoreClientTab } from './types';

type Props = {
  skill: SkillCatalogItem;
  busy: boolean;
  clientTab: StoreClientTab;
  onInstall: (id: string) => void;
  onMount: (id: string, target: MountTarget) => void;
  onUnmount: (id: string, target: MountTarget) => void;
  onUninstall: (id: string) => void;
  onOpen: (id: string, staged: boolean) => void;
};

export function SkillRow({
  skill, busy, clientTab, onInstall, onMount, onUnmount, onUninstall, onOpen,
}: Props) {
  const clientMounted = clientTab === 'claude' ? skill.mounted_claude : skill.mounted_codex;
  const otherMounted = clientTab === 'claude' ? skill.mounted_codex : skill.mounted_claude;
  const mountLabel = clientTab === 'claude' ? '挂到 Claude' : '挂到 Codex';
  const mountTarget: MountTarget = clientTab;

  return (
    <div className="store-skill-row">
      <div className="store-skill-main">
        <strong>{skill.name}</strong>
        <span className="hint compact">{skill.repo_label} · {skill.relative_path}</span>
        {skill.description && <p className="hint compact">{skill.description}</p>}
        <div className="store-tags">
          {skill.staged && <span className="badge ok">已暂存</span>}
          {skill.mounted_claude && <span className="badge ok">Claude</span>}
          {skill.mounted_codex && <span className="badge ok">Codex</span>}
        </div>
      </div>
      <div className="store-skill-actions">
        {!skill.staged && (
          <button type="button" className="tiny" disabled={busy} onClick={() => onInstall(skill.id)}>
            <Download size={14} />安装
          </button>
        )}
        {skill.staged && !clientMounted && (
          <button type="button" className="tiny" disabled={busy} onClick={() => onMount(skill.id, mountTarget)}>
            <PlugZap size={14} />{mountLabel}
          </button>
        )}
        {skill.staged && !otherMounted && clientMounted && (
          <button type="button" className="tiny" disabled={busy} onClick={() => onMount(skill.id, clientTab === 'claude' ? 'codex' : 'claude')}>
            同步到{clientTab === 'claude' ? ' Codex' : ' Claude'}
          </button>
        )}
        {skill.staged && !skill.mounted && (
          <button type="button" className="tiny" disabled={busy} onClick={() => onMount(skill.id, 'both')}>
            一键双端挂载
          </button>
        )}
        {clientMounted && (
          <button type="button" className="tiny" disabled={busy} onClick={() => onUnmount(skill.id, mountTarget)}>
            取消{clientTab === 'claude' ? ' Claude' : ' Codex'}挂载
          </button>
        )}
        {skill.staged && (
          <button type="button" className="tiny danger-link" disabled={busy} onClick={() => onUninstall(skill.id)}>
            <Trash2 size={14} />
          </button>
        )}
        {skill.staged && (
          <button type="button" className="tiny" disabled={busy} onClick={() => onOpen(skill.id, true)}>
            <ExternalLink size={14} />打开
          </button>
        )}
      </div>
    </div>
  );
}
