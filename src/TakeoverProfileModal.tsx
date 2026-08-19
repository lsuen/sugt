import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { FileSearch, Plus, X } from 'lucide-react';
import type { TakeoverProfileView } from './TakeoverProfilesSection';
import { t } from './i18n';

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
  vendor: '',
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
      onError(t('tpl.errorIdName'));
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
          vendor: form.vendor.trim() || t('tpl.vendorDefault'),
          description: form.description.trim() || t('tpl.defaultDesc'),
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
      onError(t('tpl.errorPath'));
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
      description: prev.description || t('tpl.defaultDesc'),
    }));
  };

  const disabled = busy || saving;

  return (
    <div className="modal-overlay">
      <div className="modal takeover-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>{editing ? t('tpl.editTitle') : t('tpl.addTitle')}</h3>
          <button type="button" className="icon-btn" onClick={onClose} aria-label={t('common.close')}>
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
              <Plus size={14} />{t('tpl.custom')}
            </button>
            <button
              type="button"
              className={tab === 'import' ? 'takeover-modal-tab active' : 'takeover-modal-tab'}
              onClick={() => setTab('import')}
              disabled={!aiEnabled}
              title={!aiEnabled ? t('tpl.needModel') : undefined}
            >
              <FileSearch size={14} />{t('tpl.importFromConfig')}
            </button>
          </div>
        )}

        <div className="modal-body takeover-modal-body">
          {(tab === 'custom' || editing) && (
            <>
              <p className="hint compact">
                {editing
                  ? t('tpl.editHint')
                  : t('tpl.addHint')}
              </p>
              {!editing && (
                <div className="takeover-preset-chips">
                  <span className="hint compact">{t('tpl.quickFill')}</span>
                  {QUICK_PRESETS.map((p) => (
                    <button key={p.label} type="button" className="tiny ghost preset-chip" disabled={disabled} onClick={() => applyPreset(p.form)}>
                      {p.label}
                    </button>
                  ))}
                </div>
              )}
              <label className="field-label">
                {t('tpl.clientId')}
                <input
                  placeholder={t('tpl.clientIdPlaceholder')}
                  value={form.id}
                  disabled={editing || disabled}
                  onChange={(e) => setForm({ ...form, id: e.target.value })}
                />
              </label>
              <label className="field-label">
                {t('tpl.displayName')}
                <input placeholder={t('tpl.displayNamePlaceholder')} value={form.name} disabled={disabled} onChange={(e) => setForm({ ...form, name: e.target.value })} />
              </label>
              <label className="field-label">
                {t('tpl.vendor')}
                <input placeholder={t('tpl.vendorPlaceholder')} value={form.vendor} disabled={disabled} onChange={(e) => setForm({ ...form, vendor: e.target.value })} />
              </label>
              <label className="field-label">
                {t('tpl.protocol')}
                <select value={form.protocol} disabled={disabled} onChange={(e) => setForm({ ...form, protocol: e.target.value as 'openai' | 'anthropic' })}>
                  <option value="openai">{t('common.protocolOpenai')}</option>
                  <option value="anthropic">{t('common.protocolAnthropic')}</option>
                </select>
              </label>
              <label className="field-label">
                {t('tpl.description')}
                <input placeholder={t('tpl.descriptionPlaceholder')} value={form.description} disabled={disabled} onChange={(e) => setForm({ ...form, description: e.target.value })} />
              </label>
            </>
          )}

          {tab === 'import' && !editing && (
            <>
              <p className="hint compact">{t('tpl.importHint')}</p>
              <label className="field-label">
                {t('tpl.configPath')}
                <input placeholder={t('tpl.configPathPlaceholder')} value={parsePath} disabled={disabled} onChange={(e) => setParsePath(e.target.value)} />
              </label>
            </>
          )}
        </div>

        <div className="modal-actions">
          <button type="button" className="ghost" disabled={disabled} onClick={onClose}>{t('common.cancel')}</button>
          {tab === 'import' && !editing ? (
            <button type="button" className="primary" disabled={disabled} onClick={() => void parseFile()}>{t('tpl.parseAndSave')}</button>
          ) : (
            <button type="button" className="primary" disabled={disabled} onClick={() => void saveCustom()}>
              {editing ? t('common.save') : t('client.add')}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
