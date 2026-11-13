import React, { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Play, RefreshCw, Shuffle } from 'lucide-react'
import { t } from './i18n'

/**
 * 实验免费通道：打开控制台只列出路由和主机，不打真实补全。
 * 手动刷新才探测可用性，并且不改当前首选模型。
 */
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
  experimental?: boolean
  tool_text_retry_enabled?: boolean
  tool_text_retry_extra?: number
}

type FreeModelProbe = { name: string; available: boolean }
type FreeRouteChoice = { model: string; host: string }

type ModelOption = { name: string; host?: string; available: boolean | null }

type Props = {
  providers: ProviderOption[]
  activeProviderId: string
  busy: boolean
  onActiveProviderChange: (id: string) => void
  onRefresh: () => void
  run: (action: () => Promise<unknown>, ok?: string) => Promise<void>
  pushToast: (msg: string, type: 'ok' | 'error' | 'info') => void
  /** 行尾右对齐的协议循环切换按钮回调（auto → openai → anthropic） */
  onCycleMode: () => void
}

export function UpstreamSelector({
  providers,
  activeProviderId,
  busy,
  onActiveProviderChange,
  onRefresh,
  run,
  pushToast,
  onCycleMode,
}: Props) {
  const [upstreamProviderId, setUpstreamProviderId] = useState('')
  const [models, setModels] = useState<ModelOption[]>([])
  const [modelsBusy, setModelsBusy] = useState(false)
  const [modelsLoaded, setModelsLoaded] = useState(false)

  const enabledProviders = providers.filter((p) => p.enabled)
  const activeProvider = providers.find((p) => p.id === upstreamProviderId)
  const isExperimental = Boolean(activeProvider?.experimental)

  // 同步外部 active_provider_id
  useEffect(() => {
    const preferred =
      activeProviderId || providers.find((p) => p.enabled)?.id || ''
    setUpstreamProviderId((prev) => {
      if (prev && providers.some((p) => p.id === prev && p.enabled)) return prev
      return preferred
    })
  }, [activeProviderId, providers])

  // 打开控制台只拉路由表。探测留给手动刷新。
  useEffect(() => {
    if (!upstreamProviderId || !isExperimental) return
    void loadFreeRoutes()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [upstreamProviderId, isExperimental])

  const switchUpstream = async (id: string) => {
    setUpstreamProviderId(id)
    setModels([])
    setModelsLoaded(false)
    onActiveProviderChange(id)
  }

  const loadFreeRoutes = async () => {
    const choices = await invoke<FreeRouteChoice[]>('list_experimental_free_candidates')
    setModels(
      choices.map((choice) => ({
        name: choice.model,
        host: choice.host,
        available: null,
      })),
    )
    setModelsLoaded(true)
  }

  const fetchModels = async () => {
    const provider = providers.find((p) => p.id === upstreamProviderId)
    if (!provider) return
    setModelsBusy(true)
    try {
      if (provider.experimental) {
        const choices = await invoke<FreeRouteChoice[]>(
          'list_experimental_free_candidates',
        )
        const probes = await invoke<FreeModelProbe[]>(
          'probe_experimental_free_models',
        )
        const availability = new Map(probes.map((probe) => [probe.name, probe.available]))
        setModels(
          choices.map((choice) => ({
            name: choice.model,
            host: choice.host,
            available: availability.get(choice.model) ?? false,
          })),
        )
        setModelsLoaded(true)
        if (!probes.some((probe) => probe.available)) {
          pushToast(t('upstream.freeEmpty'), 'error')
        }
        onRefresh()
      } else {
        const result = await invoke<string[]>('list_provider_models', {
          input: {
            base_url: provider.base_url,
            api_key: provider.api_key,
            protocol:
              provider.protocol === 'open_ai' ? 'openai' : provider.protocol,
            vendor_id: null,
            auto_adapt_base_url: true,
          },
        })
        setModels(result.map((name) => ({ name, available: null })))
        setModelsLoaded(true)
      }
    } catch (e) {
      pushToast(String(e), 'error')
    } finally {
      setModelsBusy(false)
    }
  }

  const selectModel = async (model: string, opts?: { silent?: boolean }) => {
    const provider = providers.find((p) => p.id === upstreamProviderId)
    if (!provider) return
    const input = {
      id: provider.id,
      name: provider.name,
      provider: provider.provider,
      base_url: provider.base_url,
      api_key: provider.api_key,
      model_name: model,
      model_alias: 'sutai',
      protocol:
        provider.protocol === 'open_ai' ? 'openai' : provider.protocol,
      enabled: provider.enabled,
      auto_adapt_base_url: true,
      tool_text_retry_enabled: provider.tool_text_retry_enabled ?? true,
      tool_text_retry_extra: provider.tool_text_retry_extra ?? 2,
    }
    if (opts?.silent) {
      try {
        await invoke('save_provider', { input })
        onRefresh()
      } catch (e) {
        pushToast(String(e), 'error')
      }
      return
    }
    await run(() => invoke('save_provider', { input }), t('upstream.toastSwitched'))
    onRefresh()
  }

  const testCurrent = async () => {
    if (!upstreamProviderId) return
    await run(
      () => invoke('test_provider', { id: upstreamProviderId }),
      t('upstream.toastTested'),
    )
  }

  const selectValue =
    models.find((m) => m.name === activeProvider?.model_name)?.name ??
    activeProvider?.model_name ??
    ''

  return (
    <>
      <div className="access-mode-row">
        <select
          className="app-select upstream-select"
          value={upstreamProviderId}
          disabled={busy || enabledProviders.length === 0}
          onChange={(e) => void switchUpstream(e.target.value)}
        >
          {enabledProviders.length === 0 && (
            <option value="">{t('upstream.noModels')}</option>
          )}
          {enabledProviders.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="tiny icon-only access-mode-toggle"
          onClick={onCycleMode}
          aria-label={t('upstream.switchProtocol')}
          title={t('upstream.switchProtocolTitle')}
        >
          <Shuffle size={14} />
        </button>
      </div>

      {upstreamProviderId && activeProvider && (
        <div className="access-field">
          <div className="access-value-row">
            {modelsLoaded ? (
              <select
                className="app-select access-model-select"
                value={selectValue}
                disabled={modelsBusy || models.length === 0}
                onChange={(e) => void selectModel(e.target.value)}
              >
                {models.length === 0 && (
                  <option value="">{t('upstream.freeEmptyOption')}</option>
                )}
                {models.map((m) => (
                  <option key={m.name} value={m.name}>
                    {m.available === true ? '● ' : m.available === false ? '○ ' : ''}
                    {m.name}
                    {m.host ? ` · ${m.host}` : ''}
                  </option>
                ))}
              </select>
            ) : (
              <code className="access-code">{activeProvider.model_name}</code>
            )}
            {isExperimental && modelsBusy && (
              <span className="hint compact upstream-probe-hint">
                {t('upstream.probing')}
              </span>
            )}
            {isExperimental && !modelsBusy && models.some((m) => m.available) && (
              <span
                className="upstream-probe-ok"
                title={t('upstream.probeOk')}
                aria-label={t('upstream.probeOk')}
              />
            )}
            <button
              type="button"
              className="tiny icon-only"
              disabled={modelsBusy}
              onClick={() => void fetchModels()}
              aria-label={t('upstream.fetchModels')}
            >
              <RefreshCw size={14} className={modelsBusy ? 'spin' : ''} />
            </button>
            <button
              type="button"
              className="tiny icon-only"
              disabled={busy}
              onClick={() => void testCurrent()}
              aria-label={t('upstream.test')}
            >
              <Play size={14} />
            </button>
          </div>
        </div>
      )}
    </>
  )
}
