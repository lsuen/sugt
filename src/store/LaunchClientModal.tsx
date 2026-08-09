import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { StoreModal } from './StoreModal';
import type { StoreClientTab } from './types';

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
      pushToast(`已在新终端启动 ${label}`, 'ok');
      onClose();
    } catch (error) {
      pushToast(formatError(error), 'error');
    } finally {
      setLaunching(false);
    }
  };

  return (
    <StoreModal title={`启动 ${label}`} onClose={onClose}>
      <p className="hint compact">
        将打开新的命令行窗口，并注入 SUGT 网关环境变量（使用配置目录中的启动脚本）。
      </p>
      <label className="field-label">
        工作目录（可选）
        <input
          placeholder="留空则使用用户主目录"
          value={workDir}
          onChange={(e) => setWorkDir(e.target.value)}
        />
      </label>
      <div className="modal-actions">
        <button type="button" className="ghost" onClick={onClose}>取消</button>
        <button type="button" className="primary" disabled={launching} onClick={() => launch()}>
          {launching ? '启动中…' : '启动'}
        </button>
      </div>
    </StoreModal>
  );
}
