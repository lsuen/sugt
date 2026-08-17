import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Code2, Copy, MessageSquare, Pencil } from 'lucide-react';
import { UpstreamSelector } from './UpstreamSelector';
import { QuickChatModal } from './QuickChatModal';
import { CodeExampleModal } from './CodeExampleModal';

type ProviderOption = {
  id: string;
  name: string;
  model_name: string;
  enabled: boolean;
  base_url: string;
  api_key: string;
  api_key_masked: string;
  protocol: 'openai' | 'anthropic' | 'open_ai';
  provider: string;
};

type Props = {
  status: {
    running?: boolean;
    listen_url?: string;
    openai_base_url?: string;
    anthropic_base_url?: string;
    gateway_client_api_key?: string;
    gateway_client_api_key_masked?: string;
    public_model_id?: string | null;
    active_provider?: string | null;
    active_provider_id?: string | null;
    allow_lan_access?: boolean;
  } | null;
  providers: ProviderOption[];
  busy: boolean;
  onRefresh: () => void;
  onActiveProviderChange: (id: string) => void;
  run: (action: () => Promise<unknown>, ok?: string) => Promise<void>;
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void;
};

function pythonExample(openaiBase: string, modelId: string, apiKey: string): string {
  const safeKey = apiKey.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
  return `import requests

BASE = "${openaiBase}"
API_KEY = "${safeKey}"
MODEL = "${modelId}"

resp = requests.post(
    f"{BASE}/chat/completions",
    headers={
        "Authorization": f"Bearer {API_KEY}",
        "Content-Type": "application/json",
    },
    json={
        "model": MODEL,
        "messages": [{"role": "user", "content": "你好，介绍一下你自己"}],
        "stream": False,
    },
    timeout=60,
)
resp.raise_for_status()
print(resp.json()["choices"][0]["message"]["content"])
`;
}

export function GatewayAccessBanner({ status, providers, busy, onRefresh, onActiveProviderChange, run, pushToast }: Props) {
  const [codeOpen, setCodeOpen] = useState(false);
  const [codeSample, setCodeSample] = useState('');
  const [chatOpen, setChatOpen] = useState(false);

  // 网关 API Key 编辑状态
  const [editingKey, setEditingKey] = useState(false);
  const [keyInput, setKeyInput] = useState('sugt-local-key');

  const openaiBase = status?.openai_base_url ?? `${status?.listen_url}/v1`;
  const anthropicBase = status?.anthropic_base_url ?? status?.listen_url;
  const activeProviderId = status?.active_provider_id ?? '';

  // 网关 API Key：优先从 status 读取，否则 fallback
  const resolveGatewayApiKey = (): string => {
    const key = (status?.gateway_client_api_key || '').trim();
    return key || 'sugt-local-key';
  };

  const gatewayKey = resolveGatewayApiKey();

  const copy = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      pushToast(`已复制 ${label}`, 'ok');
    } catch {
      pushToast('复制失败', 'error');
    }
  };

  const openCodeExample = () => {
    const modelId = 'sutai';
    setCodeSample(pythonExample(openaiBase, modelId, gatewayKey));
    setCodeOpen(true);
  };

  const saveGatewayKey = async () => {
    const trimmed = keyInput.trim();
    if (!trimmed) {
      pushToast('API Key 不能为空', 'error');
      return;
    }
    try {
      await invoke('update_gateway_settings', { input: { gatewayClientApiKey: trimmed } });
      pushToast('网关 API Key 已更新', 'ok');
      setEditingKey(false);
      onRefresh();
    } catch (e) {
      pushToast(String(e), 'error');
    }
  };

  return (
    <div className="gateway-access-side">
      <div className="gateway-access-head">
        <strong>Agent 接入点</strong>
        <span className={status?.running ? 'badge ok inline' : 'badge stop inline'}>
          {status?.running ? '网关可用' : '未启动'}
        </span>
      </div>

      {/* 上游配置区 */}
      <UpstreamSelector
        providers={providers}
        activeProviderId={activeProviderId}
        busy={busy}
        onActiveProviderChange={onActiveProviderChange}
        onRefresh={onRefresh}
        run={run}
        pushToast={pushToast}
      />

      {/* 连接信息 */}
      <div className="access-fields">
        <div className="access-field">
          <span className="access-label">Model ID（对外）</span>
          <div className="access-value-row">
            <code className="access-code">sutai</code>
            <button type="button" className="tiny icon-only" onClick={() => void copy('sutai', 'Model ID')} aria-label="复制 Model ID">
              <Copy size={14} />
            </button>
          </div>
        </div>

        <div className="access-field">
          <span className="access-label">OpenAI Base（Codex 等）</span>
          <div className="access-value-row">
            <code className="access-code">{openaiBase}</code>
            <button type="button" className="tiny icon-only" disabled={busy} onClick={() => void copy(openaiBase, 'OpenAI Base URL')} aria-label="复制 OpenAI Base">
              <Copy size={14} />
            </button>
          </div>
        </div>

        <div className="access-field">
          <span className="access-label">Anthropic Base</span>
          <div className="access-value-row">
            <code className="access-code">{anthropicBase}</code>
            <button type="button" className="tiny icon-only" disabled={busy} onClick={() => void copy(anthropicBase ?? '', 'Anthropic Base URL')} aria-label="复制 Anthropic Base">
              <Copy size={14} />
            </button>
          </div>
        </div>

        <div className="access-field">
          <span className="access-label">网关 API Key</span>
          {editingKey ? (
            <div className="access-key-edit">
              <input
                className="app-input"
                value={keyInput}
                onChange={(e) => setKeyInput(e.target.value)}
                placeholder="输入新的 API Key"
                autoFocus
                onBlur={() => void saveGatewayKey()}
                onKeyDown={(e) => { if (e.key === 'Enter') void saveGatewayKey(); if (e.key === 'Escape') { setEditingKey(false); setKeyInput(gatewayKey); } }}
              />
            </div>
          ) : (
            <div className="access-value-row">
              <code className="access-code">{gatewayKey}</code>
              <button type="button" className="tiny icon-only" onClick={() => { setKeyInput(gatewayKey); setEditingKey(true); }} aria-label="编辑 API Key">
                <Pencil size={14} />
              </button>
            </div>
          )}
        </div>
      </div>

      {/* 底部操作按钮 */}
      <div className="access-actions">
        <button type="button" className="button ghost" onClick={openCodeExample}>
          <Code2 size={14} />查看代码范例
        </button>
        <button type="button" className="button ghost" onClick={() => setChatOpen(true)}>
          <MessageSquare size={14} />直接体验
        </button>
      </div>

      <CodeExampleModal
        open={codeOpen}
        onClose={() => setCodeOpen(false)}
        code={codeSample}
      />

      <QuickChatModal
        open={chatOpen}
        onClose={() => setChatOpen(false)}
        providers={providers}
        activeProviderId={activeProviderId}
        pushToast={pushToast}
      />
    </div>
  );
}
