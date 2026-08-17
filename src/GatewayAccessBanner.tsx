import React, { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Code2, Copy, MessageSquare } from 'lucide-react'
import { UpstreamSelector } from './UpstreamSelector'
import { QuickChatModal } from './QuickChatModal'
import { CodeExampleModal } from './CodeExampleModal'

type AccessMode = 'auto' | 'openai' | 'anthropic'

/** 协议循环切换顺序：auto（最推荐）→ openai（使用最多）→ anthropic（原生） */
const MODE_ORDER: AccessMode[] = ['auto', 'openai', 'anthropic']
const MODE_LABEL: Record<AccessMode, string> = {
  auto: 'auto',
  openai: 'OpenAI',
  anthropic: 'Anthropic',
}

type ProviderOption = {
  id: string
  name: string
  model_name: string
  enabled: boolean
  base_url: string
  api_key: string
  api_key_masked: string
  protocol: 'openai' | 'anthropic' | 'open_ai'
  provider: string
}

type Props = {
  status: {
    running?: boolean
    listen_url?: string
    openai_base_url?: string
    anthropic_base_url?: string
    gateway_client_api_key?: string
    gateway_client_api_key_masked?: string
    public_model_id?: string | null
    active_provider?: string | null
    active_provider_id?: string | null
    allow_lan_access?: boolean
    anthropic_access_mode?: AccessMode
  } | null
  providers: ProviderOption[]
  busy: boolean
  onRefresh: () => void
  onActiveProviderChange: (id: string) => void
  run: (action: () => Promise<unknown>, ok?: string) => Promise<void>
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void
}

function pythonExample(
  openaiBase: string,
  modelId: string,
  apiKey: string,
): string {
  const safeKey = apiKey.replace(/\\/g, '\\\\').replace(/"/g, '\\"')
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
`
}

export function GatewayAccessBanner({
  status,
  providers,
  busy,
  onRefresh,
  onActiveProviderChange,
  run,
  pushToast,
}: Props) {
  const [codeOpen, setCodeOpen] = useState(false)
  const [codeSample, setCodeSample] = useState('')
  const [chatOpen, setChatOpen] = useState(false)
  const [accessMode, setAccessMode] = useState<AccessMode>('auto')

  const openaiBase = status?.openai_base_url ?? `${status?.listen_url}/v1`
  const anthropicBase = status?.anthropic_base_url ?? status?.listen_url
  const activeProviderId = status?.active_provider_id ?? ''

  // 同步后端保存的接入协议模式
  useEffect(() => {
    if (status?.anthropic_access_mode)
      setAccessMode(status.anthropic_access_mode)
  }, [status?.anthropic_access_mode])

  const gatewayKey =
    (status?.gateway_client_api_key || '').trim() || 'sugt-local-key'

  // 协议状态徽标：默认 auto；显式切换后显示 auto-openai / auto-anthropic
  const modeBadge = accessMode === 'auto' ? 'auto' : `auto-${accessMode}`
  const modeBadgeTitle =
    accessMode === 'auto'
      ? '接入协议：auto（自动，推荐）'
      : `接入协议：${MODE_LABEL[accessMode]}（强制）`

  const copy = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text)
      pushToast(`已复制 ${label}`, 'ok')
    } catch {
      pushToast('复制失败', 'error')
    }
  }

  // 循环切换：auto → openai → anthropic
  const cycleMode = async () => {
    const idx = MODE_ORDER.indexOf(accessMode)
    const next = MODE_ORDER[(idx + 1) % MODE_ORDER.length]
    if (next === accessMode) return
    await run(async () => {
      await invoke('update_gateway_settings', {
        input: { anthropicAccessMode: next },
      })
      setAccessMode(next)
    }, `接入协议 → ${MODE_LABEL[next]}`)
  }

  const openCodeExample = () => {
    const modelId = 'sutai'
    setCodeSample(pythonExample(openaiBase, modelId, gatewayKey))
    setCodeOpen(true)
  }

  return (
    <div className="gateway-access-side">
      <div className="gateway-access-head">
        <strong>Agent 接入点</strong>
        <span
          className={status?.running ? 'badge ok inline' : 'badge stop inline'}
        >
          {status?.running ? '网关可用' : '未启动'}
        </span>
      </div>

      <div className="access-config-block">
        {/* LLM 上游：服务商 / 模型 / 接入协议 */}
        <div className="access-group">
          <div className="access-group-title">LLM 上游</div>
          <UpstreamSelector
            providers={providers}
            activeProviderId={activeProviderId}
            busy={busy}
            onActiveProviderChange={onActiveProviderChange}
            onRefresh={onRefresh}
            run={run}
            pushToast={pushToast}
            modeBadge={modeBadge}
            modeBadgeTitle={modeBadgeTitle}
            onCycleMode={() => void cycleMode()}
          />
        </div>

        {/* Agent 接入：客户端连接信息 */}
        <div className="access-group">
          <div className="access-group-title">Agent 接入</div>
          <div className="access-field">
            <span className="access-label"> Model ID</span>
            <div className="access-value-row">
              <code className="access-code">sutai</code>
              <button
                type="button"
                className="tiny icon-only"
                onClick={() => void copy('sutai', 'Model ID')}
                aria-label="复制 Model ID"
              >
                <Copy size={14} />
              </button>
            </div>
          </div>

          <div className="access-field">
            <span className="access-label">OpenAI Base</span>
            <div className="access-value-row">
              <code className="access-code">{openaiBase}</code>
              <button
                type="button"
                className="tiny icon-only"
                disabled={busy}
                onClick={() => void copy(openaiBase, 'OpenAI Base URL')}
                aria-label="复制 OpenAI Base"
              >
                <Copy size={14} />
              </button>
            </div>
          </div>

          <div className="access-field">
            <span className="access-label">Anthropic Base</span>
            <div className="access-value-row">
              <code className="access-code">{anthropicBase}</code>
              <button
                type="button"
                className="tiny icon-only"
                disabled={busy}
                onClick={() =>
                  void copy(anthropicBase ?? '', 'Anthropic Base URL')
                }
                aria-label="复制 Anthropic Base"
              >
                <Copy size={14} />
              </button>
            </div>
          </div>

          <div className="access-field">
            <span className="access-label">网关 API Key</span>
            <div className="access-value-row">
              <code className="access-code">{gatewayKey}</code>
              <button
                type="button"
                className="tiny icon-only"
                onClick={() => void copy(gatewayKey, '网关 API Key')}
                aria-label="复制网关 API Key"
              >
                <Copy size={14} />
              </button>
            </div>
          </div>
        </div>
      </div>

      {/* 底部操作按钮 */}
      <div className="access-actions">
        <button
          type="button"
          className="button ghost"
          onClick={openCodeExample}
        >
          <Code2 size={14} />
          查看代码范例
        </button>
        <button
          type="button"
          className="button ghost"
          onClick={() => setChatOpen(true)}
        >
          <MessageSquare size={14} />
          直接体验
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
  )
}
