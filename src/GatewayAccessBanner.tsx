import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Code2, Copy, MessageSquare } from 'lucide-react';
import { UpstreamSelector } from './UpstreamSelector';
import { QuickChatModal } from './QuickChatModal';
import { CodeExampleModal } from './CodeExampleModal';

type AccessMode = 'auto' | 'openai' | 'anthropic';

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
    anthropic_access_mode?: AccessMode;
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
  const [accessMode, setAccessMode] = useState<AccessMode>('auto');

  const openaiBase = status?.openai_base_url ?? `${status?.listen_url}/v1`;
  const anthropicBase = status?.anthropic_base_url ?? status?.listen_url;
  const activeProviderId = status?.active_provider_id ?? '';

  // 同步后端保存的接入协议模式
  useEffect(() => {
    if (status?.anthropic_access_mode) setAccessMode(status.anthropic_access_mode);
  }, [status?.anthropic_access_mode]);

  const gatewayKey = (status?.gateway_client_api_key || '').trim() || 'sugt-local-key';

  // Auto 模式按当前上游协议展示实际走向：auto-openai / auto-anthropic
  const activeProvider =
    providers.find((p) => p.id === activeProviderId) ?? providers.find((p) => p.enabled);
  const autoLabel = activeProvider?.protocol === 'anthropic' ? 'auto-anthropic' : 'auto-openai';

  const copy = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      pushToast(`已复制 ${label}`, 'ok');
    } catch {
      pushToast('复制失败', 'error');
    }
  };

  const switchAccessMode = async (mode: AccessMode) => {
    if (mode === accessMode) return;
    setAccessMode(mode);
    await run(
      () => invoke('update_gateway_settings', { input: { anthropicAccessMode: mode } }),
      '接入协议已切换',
    );
  };

  const modeHint: Record<AccessMode, string> = {
    auto: 'Claude 客户端优先原生端点，失败自动降级转换',
    openai: '所有请求统一按 OpenAI 协议转换',
    anthropic: '所有请求统一走 Anthropic 原生协议',
  };

  const openCodeExample = () => {
    const modelId = 'sutai';
    setCodeSample(pythonExample(openaiBase, modelId, gatewayKey));
    setCodeOpen(true);
  };

  return (
    <div className="gateway-access-side">
      <div className="gateway-access-head">
        <strong>Agent 接入点</strong>
        <span className={status?.running ? 'badge ok inline' : 'badge stop inline'}>
          {status?.running ? '网关可用' : '未启动'}
        </span>
      </div>

      {/* 上游 + 下游统一配置区 */}
      <div className="access-config-block">
        {/* 接入协议切换：auto（默认）/ openai / anthropic */}
        <div className="access-field">
          <span className="access-label">接入协议</span>
          <div className="access-mode-switch" role="group" aria-label="接入协议">
            <button
              type="button"
              className={accessMode === 'auto' ? 'active' : ''}
              onClick={() => void switchAccessMode('auto')}
              title="自动：Claude 客户端优先原生端点，失败降级转换"
            >
              {accessMode === 'auto' ? autoLabel : 'auto'}
            </button>
            <button
              type="button"
              className={accessMode === 'openai' ? 'active' : ''}
              onClick={() => void switchAccessMode('openai')}
              title="强制走 OpenAI 协议（请求转换为 chat/completions）"
            >
              OpenAI
            </button>
            <button
              type="button"
              className={accessMode === 'anthropic' ? 'active' : ''}
              onClick={() => void switchAccessMode('anthropic')}
              title="强制走 Anthropic 原生协议（服务商不支持时返回明确错误）"
            >
              Anthropic
            </button>
          </div>
          <p className="hint compact access-mode-hint">{modeHint[accessMode]}</p>
        </div>

        {/* 上游：当前上游 + 模型 ID */}
        <UpstreamSelector
          providers={providers}
          activeProviderId={activeProviderId}
          busy={busy}
          onActiveProviderChange={onActiveProviderChange}
          onRefresh={onRefresh}
          run={run}
          pushToast={pushToast}
        />

        {/* 下游：对外连接信息 */}
        <div className="access-field">
          <span className="access-label">对外 Model ID</span>
          <div className="access-value-row">
            <code className="access-code">sutai</code>
            <button type="button" className="tiny icon-only" onClick={() => void copy('sutai', 'Model ID')} aria-label="复制 Model ID">
              <Copy size={14} />
            </button>
          </div>
        </div>

        <div className="access-field">
          <span className="access-label">OpenAI Base</span>
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
          <div className="access-value-row">
            <code className="access-code">{gatewayKey}</code>
            <button type="button" className="tiny icon-only" onClick={() => void copy(gatewayKey, '网关 API Key')} aria-label="复制网关 API Key">
              <Copy size={14} />
            </button>
          </div>
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
