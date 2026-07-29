import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FileSearch, Plus, X } from 'lucide-react';
import type { TakeoverProfileView } from './TakeoverProfilesSection';

export type TakeoverProfileForm = {
  id: string;
  name: string;
  vendor: string;
  description: string;
  protocol: 'openai' | 'anthropic';
};

type Tab = 'custom' | 'import' | 'edit';

type Props = {
  open: boolean;
  mode: Tab;
  busy: boolean;
  aiEnabled: boolean;
  initial?: TakeoverProfileView | null;
  onClose: () => void;
  onSaved: () => void;
  onError: (msg: string) => void;
};

const EMPTY_FORM: TakeoverProfileForm = {
  id: '',
  name: '',
  vendor: '自定义',
  description: '',
  protocol: 'openai',
};

const QUICK_PRESETS: Array<{ label: string; form: Partial<TakeoverProfileForm> }> = [
  { label: 'OpenCode', form: { id: 'opencode-custom', name: 'OpenCode', vendor: 'OpenCode', protocol: 'openai' } },
  { label: 'Gemini CLI', form: { id: 'gemini-custom', name: 'Gemini CLI', vendor: 'Google', protocol: 'openai' } },
  { label: 'Aider', form: { id: 'aider-custom', name: 'Aider', vendor: 'Aider', protocol: 'openai' } },
  { label: 'Cline', form: { id: 'cline-custom', name: 'Cline', vendor: 'Cline', protocol: 'openai' } },
];

function buildEnvVars(protocol: 'openai' | 'anthropic') {
  return protocol === 'anthropic'
    ? { ANTHROPIC_BASE_URL: '{anthropic_base}', ANTHROPIC_AUTH_TOKEN: '{client_key}' }
    : {
        OPENAI_API_KEY: '{client_key}',
        OPENAI_BASE_URL: '{openai_base}',
        OPENAI_API_BASE: '{openai_base}',
      };
}

function normalizeProtocol(raw: string): 'openai' | 'anthropic' {
  return raw.toLowerCase().includes('anthropic') ? 'anthropic' : 'openai';
}

export function TakeoverProfileModal({ open, mode, busy, aiEnabled, initial, onClose, onSaved, onError }: Props) {
  const [tab, setTab] = useState<Tab>(mode);
  const [form, setForm] = useState<TakeoverProfileForm>(EMPTY_FORM);
  const [parsePath, setParsePath] = useState('');
  const [saving, setSaving] = useState(false);
  const editing = mode === 'edit' && Boolean(initial);

  useEffect(() => {
    if (!open) return;
    setTab(mode === 'edit' ? 'custom' : mode);
    setParsePath('');
    if (mode === 'edit' && initial) {
      setForm({
        id: initial.id,
        name: initial.name,
        vendor: initial.vendor,
        description: initial.description,
        protocol: normalizeProtocol(initial.protocol),
      });
    } else {
      setForm(EMPTY_FORM);
    }
  }, [open, mode, initial]);

  if (!open) return null;

  const saveCustom = async () => {
    const id = form.id.trim();
    const name = form.name.trim();
    if (!id || !name) {
      onError('请填写客户端 ID 与名称');
      return;
    }
    setSaving(true);
    try {
      const keepEnv = editing && initial?.env_vars && Object.keys(initial.env_vars).length > 0
        && normalizeProtocol(initial.protocol) === form.protocol;
      await invoke('save_takeover_profile', {
        input: {
          id,
          name,
          vendor: form.vendor.trim() || '自定义',
          description: form.description.trim() || '用户自定义 Agent 接管模板',
          protocol: form.protocol,
          envVars: keepEnv ? initial!.env_vars : buildEnvVars(form.protocol),
          clearVars: editing ? (initial?.clear_vars ?? []) : [],
          settingsDirs: editing ? (initial?.settings_dirs ?? []) : [],
          settingsPath: editing ? (initial?.settings_path ?? null) : null,
          skillDirs: editing ? (initial?.skill_dirs ?? []) : [],
          launchCommand: editing ? (initial?.launch_command ?? null) : null,
        },
      });
      onSaved();
      onClose();
    } catch (e) {
      onError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const parseFile = async () => {
    if (!parsePath.trim()) {
      onError('请输入配置文件路径');
      return;
    }
    setSaving(true);
    try {
      await invoke('parse_takeover_config_path', { path: parsePath.trim(), save: true });
      onSaved();
      onClose();
    } catch (e) {
      onError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const applyPreset = (preset: Partial<TakeoverProfileForm>) => {
    setForm((prev) => ({
      ...prev,
      ...preset,
      description: prev.description || '自定义 Agent 接管模板',
    }));
  };

  const disabled = busy || saving;

  return (
    <div className="modal-overlay">
      <div className="modal takeover-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{editing ? '编辑接管客户端' : '添加接管客户端'}</h3>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="关闭">
            <X size={18} />
          </button>
        </div>

        {!editing && (
          <div className="takeover-modal-tabs">
            <button
              type="button"
              className={tab === 'custom' ? 'takeover-modal-tab active' : 'takeover-modal-tab'}
              onClick={() => setTab('custom')}
            >
              <Plus size={14} />自定义
            </button>
            <button
              type="button"
              className={tab === 'import' ? 'takeover-modal-tab active' : 'takeover-modal-tab'}
              onClick={() => setTab('import')}
              disabled={!aiEnabled}
              title={!aiEnabled ? '需先配置大模型服务' : undefined}
            >
              <FileSearch size={14} />从配置导入
            </button>
          </div>
        )}

        <div className="modal-body takeover-modal-body">
          {(tab === 'custom' || editing) && (
            <>
              <p className="hint compact">
                {editing
                  ? '修改后保存；已接管的条目下次启动仍会按偏好恢复。'
                  : '填写后保存到列表，再点「接管」写入环境；可随时取消接管。'}
              </p>
              {!editing && (
                <div className="takeover-preset-chips">
                  <span className="hint compact">快速填充</span>
                  {QUICK_PRESETS.map((p) => (
                    <button key={p.label} type="button" className="tiny ghost preset-chip" disabled={disabled} onClick={() => applyPreset(p.form)}>
                      {p.label}
                    </button>
                  ))}
                </div>
              )}
              <label className="field-label">
                客户端 ID
                <input
                  placeholder="如 my-agent（英文标识）"
                  value={form.id}
                  disabled={editing || disabled}
                  onChange={(e) => setForm({ ...form, id: e.target.value })}
                />
              </label>
              <label className="field-label">
                显示名称
                <input placeholder="在列表中显示的名称" value={form.name} disabled={disabled} onChange={(e) => setForm({ ...form, name: e.target.value })} />
              </label>
              <label className="field-label">
                厂商 / 来源
                <input placeholder="如 自定义、某团队" value={form.vendor} disabled={disabled} onChange={(e) => setForm({ ...form, vendor: e.target.value })} />
              </label>
              <label className="field-label">
                协议类型
                <select value={form.protocol} disabled={disabled} onChange={(e) => setForm({ ...form, protocol: e.target.value as 'openai' | 'anthropic' })}>
                  <option value="openai">OpenAI 兼容</option>
                  <option value="anthropic">Anthropic</option>
                </select>
              </label>
              <label className="field-label">
                说明
                <input placeholder="可选" value={form.description} disabled={disabled} onChange={(e) => setForm({ ...form, description: e.target.value })} />
              </label>
            </>
          )}

          {tab === 'import' && !editing && (
            <>
              <p className="hint compact">粘贴本地配置文件路径，解析后生成接管模板。</p>
              <label className="field-label">
                配置文件路径
                <input placeholder="例如 C:\\Users\\你\\.claude\\settings.json" value={parsePath} disabled={disabled} onChange={(e) => setParsePath(e.target.value)} />
              </label>
            </>
          )}
        </div>

        <div className="modal-actions">
          <button type="button" className="ghost" disabled={disabled} onClick={onClose}>取消</button>
          {tab === 'import' && !editing ? (
            <button type="button" className="primary" disabled={disabled} onClick={() => void parseFile()}>解析并保存</button>
          ) : (
            <button type="button" className="primary" disabled={disabled} onClick={() => void saveCustom()}>
              {editing ? '保存' : '添加'}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
