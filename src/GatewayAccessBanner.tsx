import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Code2, Copy, MessageSquare, Play, RefreshCw, X } from 'lucide-react';

type ProviderOption = {
  id: string;
  name: string;
  model_name: string;
  enabled: boolean;
  public_model_id?: string;
  base_url: string;
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
    active_model?: string | null;
    allow_lan_access?: boolean;
  } | null;
  providers: ProviderOption[];
  busy: boolean;
  onRefresh: () => void;
  onActiveProviderChange: (id: string) => void;
  run: (action: () => Promise<unknown>, ok?: string) => Promise<void>;
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void;
};

type ChatMsg = { role: 'user' | 'assistant' | 'system'; content: string };

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
  const [editingKey, setEditingKey] = useState(false);
  const [newKey, setNewKey] = useState('');
  const [codeOpen, setCodeOpen] = useState(false);
  const [codeSample, setCodeSample] = useState('');
  const [chatOpen, setChatOpen] = useState(false);
  const [chatInput, setChatInput] = useState('');
  const [chatBusy, setChatBusy] = useState(false);
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [chatProviderId, setChatProviderId] = useState('');
  const chatEndRef = useRef<HTMLDivElement | null>(null);

  const enabledProviders = providers.filter((p) => p.enabled);
  useEffect(() => {
    const preferred = status?.active_provider_id
      || providers.find((p) => p.enabled)?.id
      || '';
    setUpstreamProviderId((prev) => {
      if (prev && providers.some((p) => p.id === prev && p.enabled)) return prev;
      return preferred;
    });
  }, [status?.active_provider_id, providers]);
  const [upstreamProviderId, setUpstreamProviderId] = useState('');
  const [models, setModels] = useState<string[]>([]);
  const [modelsBusy, setModelsBusy] = useState(false);

  useEffect(() => {
    if (chatOpen) {
      chatEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [messages, chatOpen]);

  useEffect(() => {
    if (!chatOpen) return;
    const preferred = status?.active_provider_id
      || providers.find((p) => p.enabled)?.id
      || '';
    setChatProviderId((prev) => {
      if (prev && providers.some((p) => p.id === prev && p.enabled)) return prev;
      return preferred;
    });
  }, [chatOpen, status?.active_provider_id, providers]);

  const copy = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      pushToast(`已复制 ${label}`, 'ok');
    } catch {
      pushToast('复制失败', 'error');
    }
  };

  const resolveGatewayApiKey = async (): Promise<string> => {
    const fromStatus = (status?.gateway_client_api_key || '').trim();
    if (fromStatus) return fromStatus;
    try {
      const cfg = await invoke<{ gateway_client_api_key?: string }>('get_config');
      const fromConfig = (cfg.gateway_client_api_key || '').trim();
      if (fromConfig) return fromConfig;
    } catch {
      // fall through
    }
    return 'sugt-local-key';
  };

  const openCodeExample = async () => {
    const openaiBase = status?.openai_base_url ?? `${status?.listen_url}/v1`;
    const modelId = status?.public_model_id ?? 'sutai';
    try {
      const apiKey = await resolveGatewayApiKey();
      setCodeSample(pythonExample(openaiBase, modelId, apiKey));
      setCodeOpen(true);
    } catch (e) {
      pushToast(String(e), 'error');
    }
  };

  const openChat = () => {
    setMessages([]);
    setChatInput('');
    setChatOpen(true);
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

  const sendChat = async () => {
    const text = chatInput.trim();
    if (!text || chatBusy) return;
    if (!chatProviderId) {
      pushToast('请先选择模型', 'error');
      return;
    }
    setChatInput('');
    setMessages((prev) => [...prev, { role: 'user', content: text }]);
    setChatBusy(true);
    try {
      const reply = await invoke<string>('quick_gateway_chat', {
        message: text,
        providerId: chatProviderId,
      });
      setMessages((prev) => [...prev, { role: 'assistant', content: reply }]);
    } catch (e) {
      const err = String(e);
      setMessages((prev) => [...prev, { role: 'system', content: err }]);
      pushToast(err, 'error');
    } finally {
      setChatBusy(false);
    }
  };

  const switchUpstream = async (id: string) => {
    setUpstreamProviderId(id);
    setModels([]);
    onActiveProviderChange(id);
  };

  const fetchModels = async () => {
    const provider = providers.find((p) => p.id === upstreamProviderId);
    if (!provider) return;
    setModelsBusy(true);
    try {
      const result = await invoke<string[]>('list_provider_models', { input: { base_url: provider.base_url, api_key: provider.api_key_masked, protocol: provider.protocol === 'open_ai' ? 'openai' : provider.protocol, vendor_id: null, auto_adapt_base_url: true } });
      setModels(result);
    } catch (e) {
      pushToast(String(e), 'error');
    } finally {
      setModelsBusy(false);
    }
  };

  const testCurrent = async () => {
    if (!upstreamProviderId) return;
    await run(() => invoke('test_provider', { id: upstreamProviderId }), '测试完成');
  };

  const openaiBase = status?.openai_base_url ?? `${status?.listen_url}/v1`;
  const anthropicBase = status?.anthropic_base_url ?? status?.listen_url;
  const selectedChat = enabledProviders.find((p) => p.id === chatProviderId);

  return (
    <div className="gateway-access-side">
      <div className="gateway-access-head">
        <strong>Agent 接入点</strong>
        <span className={status?.running ? 'badge ok inline' : 'badge stop inline'}>
          {status?.running ? '网关可用' : '未启动'}
        </span>
      </div>

      <div className="access-model-block">
        <span className="access-label">当前上游</span>
        <strong className="access-model-name">{status?.active_provider ?? '未配置'}</strong>
        <span className="hint compact access-model-sub">{status?.active_model ?? '—'}</span>
        <div className="upstream-actions">
          <button type="button" className="tiny" disabled={modelsBusy} onClick={() => void fetchModels()}>
            <RefreshCw size={12} />{modelsBusy ? '获取中…' : '获取模型列表'}
          </button>
          <button type="button" className="tiny" disabled={busy} onClick={() => void testCurrent()}>
            <Play size={12} />测试
          </button>
        </div>
      </div>

      <div className="access-fields">
        <div className="access-field">
          <span className="access-label">Model ID（对外）</span>
          <div className="access-value-row">
            <code className="access-code">sutai</code>
            <button type="button" className="tiny icon-only" onClick={() => copy('sutai', 'Model ID')} aria-label="复制 Model ID">
              <Copy size={14} />
            </button>
          </div>
        </div>

        <div className="access-field">
          <span className="access-label">OpenAI Base（Codex 等）</span>
          <div className="access-value-row">
            <code className="access-code">{openaiBase}</code>
            <button type="button" className="tiny icon-only" disabled={busy} onClick={() => copy(openaiBase, 'OpenAI Base URL')} aria-label="复制 OpenAI Base">
              <Copy size={14} />
            </button>
          </div>
        </div>

        <div className="access-field">
          <span className="access-label">Anthropic Base</span>
          <div className="access-value-row">
            <code className="access-code">{anthropicBase}</code>
            <button type="button" className="tiny icon-only" disabled={busy} onClick={() => copy(anthropicBase ?? '', 'Anthropic Base URL')} aria-label="复制 Anthropic Base">
              <Copy size={14} />
            </button>
          </div>
        </div>

        <div className="access-field">
          <span className="access-label">网关 API Key</span>
          <div className="access-value-row">
            <code className="access-code">local-key</code>
          </div>
        </div>

        {models.length > 0 && (
          <div className="access-models-list" style={{ padding: '8px 12px' }}>
            <span className="hint compact" style={{ display: 'block', marginBottom: 4 }}>
              可用上游模型（{models.length}）
            </span>
            <div className="model-tags">
              {models.map((m) => (
                <span key={m} className="model-tag">{m}</span>
              ))}
            </div>
          </div>
        )}
      </div>

      <div className="access-actions">
        <button type="button" className="button ghost" onClick={() => void openCodeExample()}>
          <Code2 size={14} />查看代码范例
        </button>
        <button type="button" className="button ghost" onClick={() => void openChat()}>
          <MessageSquare size={14} />直接体验
        </button>
      </div>

      {codeOpen && (
        <div className="modal-backdrop" onClick={() => setCodeOpen(false)}>
          <div className="modal-card" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <strong>代码范例</strong>
              <button type="button" className="tiny icon-only" onClick={() => setCodeOpen(false)}>
                <X size={14} />
              </button>
            </div>
            <pre className="code-block">{codeSample}</pre>
          </div>
        </div>
      )}

      {chatOpen && (
        <div className="modal-backdrop" onClick={() => setChatOpen(false)}>
          <div className="modal-card chat-card" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <strong>直接体验</strong>
              <button type="button" className="tiny icon-only" onClick={() => setChatOpen(false)}>
                <X size={14} />
              </button>
            </div>
            <div className="chat-messages">
              {messages.map((msg, i) => (
                <div key={i} className={`chat-msg ${msg.role}`}>
                  <div className="chat-msg-bubble">{msg.content}</div>
                </div>
              ))}
              <div ref={chatEndRef} />
            </div>
            <div className="chat-input-row">
              <input
                className="app-input"
                value={chatInput}
                placeholder="发送消息测试…"
                disabled={chatBusy}
                onChange={(e) => setChatInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void sendChat(); } }}
              />
              <button type="button" className="button primary" disabled={chatBusy || !chatInput.trim()} onClick={() => void sendChat()}>
                发送
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
