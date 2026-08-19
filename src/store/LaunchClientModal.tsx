import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { StoreModal } from './StoreModal';
import type { StoreClientTab } from './types';
import { t } from '../i18n';

type Props = {
  clientTab: StoreClientTab;
  onClose: () => void;
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void;
  formatError: (error: unknown) => string;
};

export function LaunchClientModal({ clientTab, onClose, pushToast, formatError }: Props) {
  const [workDir, setWorkDir] = useState('');
  const [launching, setLaunching] = useState(false);
  const label = clientTab === 'claude' ? 'Claude Code' : 'Codex';

  const launch = async () => {
    setLaunching(true);
    try {
      await invoke('start_gateway');
      await invoke('store_launch_client', {
        client: clientTab,
        workDir: workDir.trim() || null,
      });
      pushToast(t('launch.toastStarted', { label }), 'ok');
      onClose();
    } catch (error) {
      pushToast(formatError(error), 'error');
    } finally {
      setLaunching(false);
    }
  };

  return (
    <StoreModal title={t('launch.title', { label })} onClose={onClose}>
      <p className="hint compact">
        {t('launch.hint')}
      </p>
      <label className="field-label">
        {t('launch.workDirLabel')}
        <input
          placeholder={t('launch.workDirPlaceholder')}
          value={workDir}
          onChange={(e) => setWorkDir(e.target.value)}
        />
      </label>
      <div className="modal-actions">
        <button type="button" className="ghost" onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="primary" disabled={launching} onClick={() => launch()}>
          {launching ? t('common.starting') : t('launch.start')}
        </button>
      </div>
    </StoreModal>
  );
}
