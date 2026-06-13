import React, { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { StoreModal } from './StoreModal';

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
      pushToast('代理测试通过', 'ok');
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
      pushToast('GitHub 代理已保存', 'ok');
      onClose();
    } catch (error) {
      pushToast(formatError(error), 'error');
    } finally {
      setSaving(false);
    }
  };

  return (
    <StoreModal title="GitHub 克隆代理" onClose={onClose} wide>
      <p className="hint compact">
        国内网络可配置加速前缀，刷新技能仓库时 git clone 将访问拼接后的地址。留空则直连 GitHub。
      </p>
      <label>
        代理前缀
        <input
          placeholder="例如 https://ghfast.top"
          value={value}
          onChange={(e) => setValue(e.target.value)}
        />
      </label>
      <p className="hint compact">
        预览：<code className="store-path-code">{previewUrl}</code>
      </p>
      <p className="hint compact">
        测试将访问公开仓库 <code className="store-path-code">{TEST_REPO}</code>
      </p>
      {testHint && <p className={`hint compact${testHint.includes('成功') ? ' store-ok-hint' : ' store-error'}`}>{testHint}</p>}
      <div className="modal-actions">
        <button type="button" className="ghost" onClick={onClose}>取消</button>
        <button type="button" className="ghost" disabled={testing || saving} onClick={() => test()}>
          {testing ? '测试中…' : '测试连接'}
        </button>
        <button type="button" className="primary" disabled={testing || saving} onClick={() => save()}>
          {saving ? '保存中…' : '保存'}
        </button>
      </div>
    </StoreModal>
  );
}
