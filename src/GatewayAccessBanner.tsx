import React, { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Code2, Copy, MessageSquare } from 'lucide-react'
import { UpstreamSelector } from './UpstreamSelector'
import { QuickChatModal } from './QuickChatModal'
import { CodeExampleModal } from './CodeExampleModal'
import { t, type TKey } from './i18n'

type AccessMode = 'auto' | 'openai' | 'anthropic'

/** 协议循环切换顺序：auto（最推荐）→ openai（使用最多）→ anthropic（原生） */
const MODE_ORDER: AccessMode[] = ['auto', 'openai', 'anthropic']
const MODE_LABEL: Record<AccessMode, string> = {
  auto: 'auto',
  openai: 'OpenAI',
  anthropic: 'Anthropic',
}

/** 后端 ProxyHit.mode 实际协议路径 → i18n 键（与网关日志 mode= 一一对应） */
const PROXY_MODE_KEY: Record<string, TKey> = {
  anthropic_native: 'proto.anthropicNative',
  openai_adapter: 'proto.openaiAdapter',
  openai_direct: 'proto.openaiDirect',
  responses_native: 'proto.responsesNative',
  chat_completions_adapter: 'proto.chatAdapter',
}

/** 协议模式可读标签；未知名保持原样。t() 依赖文件底部 dict，须在调用时求值（模块级会 TDZ 报错） */
function proxyModeLabel(mode: string): string {
  return PROXY_MODE_KEY[mode] ? t(PROXY_MODE_KEY[mode]) : mode
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
    last_proxy_client?: string | null
    last_proxy_mode?: string | null
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

  // 状态标识（流量视图）：优先显示最近一次请求实际走的协议模式，
  // 与网关日志 mode= 一一对应；无流量时回退显示用户接入配置（auto / openai / anthropic）
  const lastMode = status?.last_proxy_mode
  const modeBadge = lastMode ? proxyModeLabel(lastMode) : accessMode
  const modeBadgeTitle = lastMode
    ? t('gateway.modeBadgeActual', { mode: proxyModeLabel(lastMode) })
    : accessMode === 'auto'
      ? t('gateway.modeBadgeAuto')
      : t('gateway.modeBadgeForced', { mode: MODE_LABEL[accessMode] })

  // 最近一次代理命中的客户端标签（如 claude-cli），后端无命中时为 null
  const lastClient = status?.last_proxy_client ?? ''

  const copy = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text)
      pushToast(t('gateway.copied', { label }), 'ok')
    } catch {
      pushToast(t('gateway.copyFailed'), 'error')
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
    }, t('gateway.cycleTo', { mode: MODE_LABEL[next] }))
  }

  const openCodeExample = () => {
    const modelId = 'sutai'
    setCodeSample(pythonExample(openaiBase, modelId, gatewayKey))
    setCodeOpen(true)
  }

  return (
    <div className="gateway-access-side">
      <div className="gateway-access-head">
        <strong>{t('gateway.title')}</strong>
        <span
          className={status?.running ? 'badge ok inline' : 'badge stop inline'}
        >
          {status?.running ? t('gateway.up') : t('gateway.down')}
        </span>
      </div>

      <div className="access-config-block">
        {/* LLM 上游：服务商 / 模型 / 接入协议（徽标为同行小字副标） */}
        <div className="access-group">
          <div className="access-group-title">
            <span>{t('gateway.upstream')}</span>
            <span className="access-group-sub" title={modeBadgeTitle}>
              {modeBadge}
            </span>
          </div>
          <UpstreamSelector
            providers={providers}
            activeProviderId={activeProviderId}
            busy={busy}
            onActiveProviderChange={onActiveProviderChange}
            onRefresh={onRefresh}
            run={run}
            pushToast={pushToast}
            onCycleMode={() => void cycleMode()}
          />
        </div>

        {/* Agent 接入：客户端连接信息（副标为最近调用来源，只显示最近一次） */}
        <div className="access-group">
          <div className="access-group-title">
            <span>{t('gateway.agent')}</span>
            {lastClient && (
              <span className="access-group-sub" title={t('gateway.lastSource')}>
                {lastClient}
              </span>
            )}
          </div>
          <div className="access-field">
            <span className="access-label"> {t('gateway.modelId')}</span>
            <div className="access-value-row">
              <code className="access-code">sutai</code>
              <button
                type="button"
                className="tiny icon-only"
                onClick={() => void copy('sutai', t('gateway.modelId'))}
                aria-label={t('gateway.modelId')}
              >
                <Copy size={14} />
              </button>
            </div>
          </div>

          <div className="access-field">
            <span className="access-label">{t('gateway.openaiBase')}</span>
            <div className="access-value-row">
              <code className="access-code">{openaiBase}</code>
              <button
                type="button"
                className="tiny icon-only"
                disabled={busy}
                onClick={() => void copy(openaiBase, t('gateway.openaiBase'))}
                aria-label={t('gateway.openaiBase')}
              >
                <Copy size={14} />
              </button>
            </div>
          </div>

          <div className="access-field">
            <span className="access-label">{t('gateway.anthropicBase')}</span>
            <div className="access-value-row">
              <code className="access-code">{anthropicBase}</code>
              <button
                type="button"
                className="tiny icon-only"
                disabled={busy}
                onClick={() =>
                  void copy(anthropicBase ?? '', t('gateway.anthropicBase'))
                }
                aria-label={t('gateway.anthropicBase')}
              >
                <Copy size={14} />
              </button>
            </div>
          </div>

          <div className="access-field">
            <span className="access-label">{t('gateway.apiKey')}</span>
            <div className="access-value-row">
              <code className="access-code">{gatewayKey}</code>
              <button
                type="button"
                className="tiny icon-only"
                onClick={() => void copy(gatewayKey, t('gateway.apiKey'))}
                aria-label={t('gateway.apiKey')}
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
          {t('gateway.codeExample')}
        </button>
        <button
          type="button"
          className="button ghost"
          onClick={() => setChatOpen(true)}
        >
          <MessageSquare size={14} />
          {t('gateway.quickChat')}
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
