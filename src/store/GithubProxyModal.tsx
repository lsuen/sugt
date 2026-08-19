import React, { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { StoreModal } from './StoreModal';
import { t } from '../i18n';

const TEST_REPO = 'https://github.com/lsuen/testconnect';

type Props = {
  prefix: string;
  onClose: () => void;
  onSaved: (prefix: string) => void;
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void;
  formatError: (error: unknown) => string;
};

export function GithubProxyModal({ prefix, onClose, onSaved, pushToast, formatError }: Props) {
  const [value, setValue] = useState(prefix);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testHint, setTestHint] = useState<string | null>(null);

  const previewUrl = useMemo(() => {
    const p = value.trim().replace(/\/+$/, '');
    if (!p) return TEST_REPO;
    return `${p}/${TEST_REPO}`;
  }, [value]);

  const test = async () => {
    setTesting(true);
    setTestHint(null);
    try {
      const msg = await invoke<string>('store_test_github_proxy', { prefix: value.trim() });
      setTestHint(msg);
      pushToast(t('gproxy.toastTested'), 'ok');
    } catch (error) {
      const msg = formatError(error);
      setTestHint(msg);
      pushToast(msg, 'error');
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    setSaving(true);
    try {
      const settings = await invoke<{ editor_command: string; github_proxy_prefix: string }>('store_get_settings');
      const next = { ...settings, github_proxy_prefix: value.trim() };
      await invoke('store_set_settings', { settings: next });
      onSaved(value.trim());
      pushToast(t('gproxy.toastSaved'), 'ok');
      onClose();
    } catch (error) {
      pushToast(formatError(error), 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <StoreModal title={t('gproxy.title')} onClose={onClose} wide>
      <p className="hint compact">
        {t('gproxy.hint')}
      </p>
      <label className="field-label">
        {t('gproxy.prefixLabel')}
        <input
          placeholder={t('gproxy.prefixPlaceholder')}
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
      </label>
      <p className="hint compact">
        {t('gproxy.preview')}<code className="store-path-code">{previewUrl}</code>
      </p>
      <p className="hint compact">
        {t('gproxy.testRepoHint')}<code className="store-path-code">{TEST_REPO}</code>
      </p>
      {testHint && <p className={`hint compact${testHint.includes('成功') ? ' store-ok-hint' : ' store-error'}`}>{testHint}</p>}
      <div className="modal-actions">
        <button type="button" className="ghost" onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="ghost" disabled={testing || saving} onClick={() => test()}>
          {testing ? t('common.testing') : t('common.testConnection')}
        </button>
        <button type="button" className="primary" disabled={testing || saving} onClick={() => save()}>
          {saving ? t('common.saving') : t('common.save')}
        </button>
      </div>
    </StoreModal>
  );
}
