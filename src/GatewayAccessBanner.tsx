import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Code2, Copy, MessageSquare, Pencil, Play, RefreshCw, X } from 'lucide-react';

type ProviderOption = {
  id: string;
  name: string;
  model_name: string;
  enabled: boolean;
  base_url: string;
  api_key: string;          // 未脱敏，用于请求上游
  api_key_masked: string;   // 脱敏展示用
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
  const [codeOpen, setCodeOpen] = useState(false);
  const [codeSample, setCodeSample] = useState('');
  const [chatOpen, setChatOpen] = useState(false);
  const [chatInput, setChatInput] = useState('');
  const [chatBusy, setChatBusy] = useState(false);
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [chatProviderId, setChatProviderId] = useState('');
  const chatEndRef = useRef<HTMLDivElement | null>(null);

  // 网关 API Key 编辑状态
  const [editingKey, setEditingKey] = useState(false);
  const [keyInput, setKeyInput] = useState('sugt-local-key');

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
  const [modelsLoaded, setModelsLoaded] = useState(false);

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

  // 网关 API Key：优先从 status 读取，否则 fallback
  const resolveGatewayApiKey = (): string => {
    const key = (status?.gateway_client_api_key || '').trim();
    return key || 'sugt-local-key';
  };

  const openCodeExample = async () => {
    const openaiBase = status?.openai_base_url ?? `${status?.listen_url}/v1`;
    const modelId = 'sutai';
    setCodeSample(pythonExample(openaiBase, modelId, resolveGatewayApiKey())); setCodeOpen(true);
  };

  const openChat = () => {
    setMessages([]);
    setChatInput('');
    setChatOpen(true);
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
    setModelsLoaded(false);
    onActiveProviderChange(id);
  };

  const fetchModels = async () => {
    const provider = providers.find((p) => p.id === upstreamProviderId);
    if (!provider) return;
    setModelsBusy(true);
    try {
      const result = await invoke<string[]>('list_provider_models', { input: { base_url: provider.base_url, api_key: provider.api_key, protocol: provider.protocol === 'open_ai' ? 'openai' : provider.protocol, vendor_id: null, auto_adapt_base_url: true } });
      setModels(result);
      setModelsLoaded(true);
    } catch (e) {
      pushToast(String(e), 'error');
    } finally {
      setModelsBusy(false);
    }
  };

  const selectModel = async (model: string) => {
    const provider = providers.find((p) => p.id === upstreamProviderId);
    if (!provider) return;
    await run(
      () => invoke('save_provider', {
        input: {
          id: provider.id,
          name: provider.name,
          provider: provider.provider,
          base_url: provider.base_url,
          api_key: provider.api_key,
          model_name: model,
          model_alias: 'sutai',
          protocol: provider.protocol === 'open_ai' ? 'openai' : provider.protocol,
          enabled: provider.enabled,
          auto_adapt_base_url: true,
        },
      }),
      '模型已切换',
    );
    onRefresh();
  };

  const testCurrent = async () => {
    if (!upstreamProviderId) return;
    await run(() => invoke('test_provider', { id: upstreamProviderId }), '测试完成');
  };

  const openaiBase = status?.openai_base_url ?? `${status?.listen_url}/v1`;
  const anthropicBase = status?.anthropic_base_url ?? status?.listen_url;
  const activeProvider = providers.find((p) => p.id === upstreamProviderId);
  const gatewayKey = resolveGatewayApiKey();

  return (
    <div className="gateway-access-side">
      <div className="gateway-access-head">
        <strong>Agent 接入点</strong>
        <span className={status?.running ? 'badge ok inline' : 'badge stop inline'}>
          {status?.running ? '网关可用' : '未启动'}
        </span>
      </div>

      {/* 上游配置区：当前上游 + 模型 ID，统一容器避免布局跳动 */}
      <div className="access-upstream-section">
        {/* 当前上游：从模型配置 tab 读取 enabled providers */}
        <div className="access-model-block">
          <span className="access-label">当前上游</span>
          <select
            className="app-select upstream-select"
            value={upstreamProviderId}
            disabled={busy || enabledProviders.length === 0}
            onChange={(e) => void switchUpstream(e.target.value)}
          >
            {enabledProviders.length === 0 && <option value="">暂无已启用模型</option>}
            {enabledProviders.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name} · {p.model_name}
              </option>
            ))}
          </select>
        </div>

        {/* 模型 ID：获取模型列表后切换为下拉，选中后实时同步到模型配置 */}
        {upstreamProviderId && activeProvider && (
          <div className="access-model-select-block">
            <span className="access-label">模型 ID</span>
            <div className="access-value-row">
              {modelsLoaded ? (
                <select
                  className="app-select access-model-select"
                  value={activeProvider.model_name}
                  disabled={modelsBusy}
                  onChange={(e) => void selectModel(e.target.value)}
                >
                  {models.map((m) => (
                    <option key={m} value={m}>{m}</option>
                  ))}
                </select>
              ) : (
                <code className="access-code">{activeProvider.model_name}</code>
              )}
              <button type="button" className="tiny icon-only" disabled={modelsBusy} onClick={() => void fetchModels()} aria-label="获取模型列表">
                <RefreshCw size={14} />
              </button>
              <button type="button" className="tiny icon-only" disabled={busy} onClick={() => void testCurrent()} aria-label="测试">
                <Play size={14} />
              </button>
            </div>
          </div>
        )}
      </div>

      {/* 连接信息 */}
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
        <button type="button" className="button ghost" onClick={() => void openCodeExample()}>
          <Code2 size={14} />查看代码范例
        </button>
        <button type="button" className="button ghost" onClick={() => void openChat()}>
          <MessageSquare size={14} />直接体验
        </button>
      </div>

      {/* 代码范例弹窗 */}
      {codeOpen && (
        <div className="modal-overlay" onClick={() => setCodeOpen(false)}>
          <div className="modal modal-compact" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>代码范例</h3>
              <button type="button" className="tiny icon-only" onClick={() => setCodeOpen(false)}>
                <X size={14} />
              </button>
            </div>
            <div className="modal-body">
              <pre className="code-sample">{codeSample}</pre>
            </div>
          </div>
        </div>
      )}

      {/* 直接体验弹窗 */}
      {chatOpen && (
        <div className="modal-overlay" onClick={() => setChatOpen(false)}>
          <div className="modal modal-compact quick-chat-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>直接体验</h3>
              <button type="button" className="tiny icon-only" onClick={() => setChatOpen(false)}>
                <X size={14} />
              </button>
            </div>
            <div className="modal-body">
              <div className="quick-chat-body">
                <div className="quick-chat-messages">
                  {messages.map((msg, i) => (
                    <div key={i} className={`quick-chat-bubble ${msg.role}`}>
                      <div className="quick-chat-role">{msg.role}</div>
                      <div className="quick-chat-text">{msg.content}</div>
                    </div>
                  ))}
                  <div ref={chatEndRef} />
                </div>
                <div className="quick-chat-input-row">
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
          </div>
        </div>
      )}
    </div>
  );
}
