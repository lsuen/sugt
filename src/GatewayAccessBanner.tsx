import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Code2, Copy, KeyRound, MessageSquare, Pencil, Play, RefreshCw, X } from 'lucide-react';

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
          api_key: provider.api_key_masked,
          model_name: model,
          model_alias: 'sutai',
          protocol: provider.protocol === 'open_ai' ? 'openai' : provider.protocol,
          enabled: provider.enabled,
          auto_adapt_base_url: true,
        },
      }),
      '模型已切换',
    );
  };

  const testCurrent = async () => {
    if (!upstreamProviderId) return;
    await run(
      () => invoke('test_provider', { id: upstreamProviderId }),
      '测试完成',
    );
  };


  const openaiBase = status?.openai_base_url ?? `${status?.listen_url}/v1`;
  const anthropicBase = status?.anthropic_base_url ?? status?.listen_url;
  const displayKey = (status?.gateway_client_api_key || '').trim()
    || status?.gateway_client_api_key_masked
    || 'sugt-local-key';
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

      {upstreamProviderId && (
        <div className="access-upstream-controls">
          <div className="access-field" style={{marginBottom: 8}}>
            <span className="access-label">模型 ID（对外）</span>
            <div className="access-value-row">
              <select
                className="app-select"
                value={status?.public_model_id ?? ''}
                disabled={modelsBusy}
                onChange={(e) => void selectModel(e.target.value)}
              >
                <option value="">sutai（默认）</option>
                {models.map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
              </select>
              <button type="button" className="tiny" disabled={modelsBusy} onClick={() => void fetchModels()}>
                <RefreshCw size={12} />{modelsBusy ? '获取中…' : '获取模型列表'}
              </button>
              <button type="button" className="tiny" disabled={busy} onClick={() => void testCurrent()}>
                <Play size={12} />测试
              </button>
            </div>
            {models.length > 0 && (
              <span className="hint compact" style={{marginTop: 4}}>已加载 {models.length} 个上游模型，选中后自动同步</span>
            )}
          </div>
        </div>
      )}

      <div className="access-fields">

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
          <span className="access-label"><KeyRound size={12} /> 网关 API Key</span>
          {editingKey ? (
            <div className="key-edit access-key-edit">
              <input value={newKey} onChange={(e) => setNewKey(e.target.value)} placeholder="本地网关 Key" />
              <button type="button" className="tiny primary" disabled={busy} onClick={() => saveKey()}>保存</button>
              <button type="button" className="tiny ghost" onClick={() => setEditingKey(false)}>取消</button>
            </div>
          ) : (
            <div className="access-value-row">
              <code className="access-code">{displayKey}</code>
              <button type="button" className="tiny icon-only" disabled={busy} onClick={() => { setNewKey((status?.gateway_client_api_key || '').trim()); setEditingKey(true); }} aria-label="编辑 API Key">
                <Pencil size={14} />
              </button>
            </div>
          )}
        </div>
      </div>

      <div className="access-foot-actions">
        <button type="button" className="tiny" disabled={busy} onClick={() => void openCodeExample()}>
          <Code2 size={14} />查看代码范例
        </button>
        <button type="button" className="tiny primary" disabled={busy} onClick={openChat}>
          <MessageSquare size={14} />直接体验
        </button>
      </div>

      {codeOpen && (
        <div className="modal-overlay" onClick={() => setCodeOpen(false)}>
          <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>Python 请求网关范例</h3>
              <button type="button" className="ghost tiny-btn" onClick={() => setCodeOpen(false)} aria-label="关闭">
                <X size={16} />
              </button>
            </div>
            <div className="modal-body">
              <p className="hint compact">
                可直接复制运行；已写入当前本地网关真实 API Key 与 Model ID。
                {status?.allow_lan_access ? ' · 已允许局域网' : ''}
              </p>
              <pre className="code-sample">{codeSample}</pre>
              <div className="modal-actions">
                <button type="button" className="ghost" onClick={() => setCodeOpen(false)}>关闭</button>
                <button type="button" className="primary" onClick={() => copy(codeSample, 'Python 范例')}>复制代码</button>
              </div>
            </div>
          </div>
        </div>
      )}

      {chatOpen && (
        <div className="modal-overlay">
          <div className="modal modal-wide quick-chat-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>直接体验</h3>
              <button type="button" className="ghost tiny-btn" onClick={() => setChatOpen(false)} aria-label="关闭">
                <X size={16} />
              </button>
            </div>
            <div className="modal-body quick-chat-body">
              <label className="field-label quick-chat-model">
                选择模型
                <select
                  className="app-select"
                  value={chatProviderId}
                  disabled={chatBusy || enabledProviders.length === 0}
                  onChange={(e) => setChatProviderId(e.target.value)}
                >
                  {enabledProviders.length === 0 && <option value="">暂无已启用模型</option>}
                  {enabledProviders.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name} · {p.model_name}
                    </option>
                  ))}
                </select>
              </label>
              <div className="quick-chat-messages">
                {messages.length === 0 && (
                  <p className="hint compact">
                    {selectedChat
                      ? `向「${selectedChat.name}」发送消息（直连该上游）。`
                      : '请先在模型配置中添加并启用模型。'}
                  </p>
                )}
                {messages.map((m, i) => (
                  <div key={`${m.role}-${i}`} className={`quick-chat-bubble ${m.role}`}>
                    <span className="quick-chat-role">
                      {m.role === 'user' ? '我' : m.role === 'assistant' ? '模型' : '系统'}
                    </span>
                    <div className="quick-chat-text">{m.content}</div>
                  </div>
                ))}
                <div ref={chatEndRef} />
              </div>
              <div className="quick-chat-input-row">
                <input
                  value={chatInput}
                  placeholder={chatBusy ? '等待回复…' : '输入消息，回车发送'}
                  disabled={chatBusy || !chatProviderId}
                  onChange={(e) => setChatInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && !e.shiftKey) {
                      e.preventDefault();
                      void sendChat();
                    }
                  }}
                />
                <button type="button" className="primary" disabled={chatBusy || !chatInput.trim() || !chatProviderId} onClick={() => void sendChat()}>
                  {chatBusy ? '发送中…' : '发送'}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
