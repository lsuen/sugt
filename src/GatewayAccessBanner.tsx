import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Copy, KeyRound, Pencil } from 'lucide-react';

type Props = {
  status: {
    running?: boolean;
    listen_url?: string;
    openai_base_url?: string;
    anthropic_base_url?: string;
    gateway_client_api_key_masked?: string;
    public_model_id?: string | null;
    allow_lan_access?: boolean;
  } | null;
  busy: boolean;
  onRefresh: () => void;
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void;
};

export function GatewayAccessBanner({ status, busy, onRefresh, pushToast }: Props) {
  const [editingKey, setEditingKey] = useState(false);
  const [newKey, setNewKey] = useState('');

  const copy = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      pushToast(`已复制 ${label}`, 'ok');
    } catch {
      pushToast('复制失败', 'error');
    }
  };

  const saveKey = async () => {
    const trimmed = newKey.trim();
    if (!trimmed) {
      pushToast('API Key 不能为空', 'error');
      return;
    }
    try {
      await invoke('update_gateway_settings', { input: { gatewayClientApiKey: trimmed } });
      pushToast('网关 API Key 已更新，请修复接管', 'ok');
      setEditingKey(false);
      setNewKey('');
      onRefresh();
    } catch (e) {
      pushToast(String(e), 'error');
    }
  };

  if (!status?.listen_url) return null;

  return (
    <div className="gateway-access-banner">
      <div className="gateway-access-head">
        <strong>任意 Agent 接入点</strong>
        <span className={status.running ? 'badge ok' : 'badge stop'}>
          {status.running ? '网关可用' : '网关未启动'}
        </span>
      </div>
      <p className="hint compact">OpenAI 兼容与 Anthropic 兼容客户端均可指向此处，无需为每个 Agent 单独配上游。</p>
      <div className="gateway-access-grid">
        <div className="gateway-access-item">
          <span>OpenAI Base URL</span>
          <code>{status.openai_base_url ?? `${status.listen_url}/v1`}</code>
          <button type="button" className="tiny" disabled={busy} onClick={() => copy(status.openai_base_url ?? `${status.listen_url}/v1`, 'OpenAI Base URL')}>
            <Copy size={14} />
          </button>
        </div>
        <div className="gateway-access-item">
          <span>Anthropic Base URL</span>
          <code>{status.anthropic_base_url ?? status.listen_url}</code>
          <button type="button" className="tiny" disabled={busy} onClick={() => copy(status.anthropic_base_url ?? status.listen_url!, 'Anthropic Base URL')}>
            <Copy size={14} />
          </button>
        </div>
        <div className="gateway-access-item">
          <span>Model ID（对外）</span>
          <code>{status.public_model_id ?? '（未配置默认模型）'}</code>
        </div>
        <div className="gateway-access-item key-row">
          <span><KeyRound size={14} /> 网关 API Key</span>
          {editingKey ? (
            <div className="key-edit">
              <input value={newKey} onChange={(e) => setNewKey(e.target.value)} placeholder="自定义 Key，防内网蹭用" />
              <button type="button" className="tiny primary" disabled={busy} onClick={() => saveKey()}>保存</button>
              <button type="button" className="tiny ghost" onClick={() => setEditingKey(false)}>取消</button>
            </div>
          ) : (
            <>
              <code>{status.gateway_client_api_key_masked ?? '—'}</code>
              <button type="button" className="tiny" disabled={busy} onClick={() => setEditingKey(true)}>
                <Pencil size={14} /> 编辑
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
