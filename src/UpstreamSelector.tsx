import React, { useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from './transport';
import { ChevronDown, Play, RefreshCw, Shuffle } from 'lucide-react'
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

type FreeModelProbe = {
  name: string
  available: boolean
  latencyMs: number | null
  tokensPerSec: number | null
}
type FreeRouteChoice = { model: string; host: string }

/** test_provider_models 的单模型探测结果（仅自定义通道「获取并测试」用） */
type ModelTestResult = {
  modelName: string
  available: boolean
  latencyMs: number | null
  tokensPerSec: number | null
}

type ModelOption = {
  name: string
  host?: string
  available: boolean | null
  boxName?: string
  latencyMs?: number | null
  tokensPerSec?: number | null
}

/** get_config 里本组件只关心「获取模型行为」两个开关（snake_case，缺省按产品默认） */
type FetchModeConfig = {
  free_models_fetch_and_test?: boolean
  custom_models_fetch_and_test?: boolean
}

/** 与 src-tauri/src/strategy_box.rs 的 StrategyBox 对齐（这里只关心对外 model id） */
type StrategyBoxOption = { id: string; name: string; publicModelId: string; enabled: boolean }

/** 与 src-tauri/src/model_registry.rs 的 ModelEntry 对齐（这里只关心启用状态） */
type ModelEntry = { modelName: string; enabled: boolean }

const FREE_PROVIDER_ID = 'sugt-free'

/** 与 src-tauri/src/gateway.rs 的 ProviderTestOutcome 对齐（真实 LLM 探测报告） */
type ProviderTestReport = {
  ok: boolean
  reply: string | null
  latencyMs: number
  tokensPerSec: number
  completionTokens: number
  estimated: boolean
  error: string | null
}
type ProviderTestOutcome = {
  status: 'Unknown' | 'Available' | 'Unavailable'
  report: ProviderTestReport
}

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

/** 行文本拼装规则与原生 option 时代一致：●/○ 点 + name + 盒子 tag + · host */
function modelRowLabel(m: ModelOption): string {
  return `${m.available === true ? '● ' : m.available === false ? '○ ' : ''}${m.name}${
    m.boxName ? t('upstream.strategyBoxTag') : ''
  }${m.host ? ` · ${m.host}` : ''}`
}

/** 指标区只在 latencyMs 有值时出现；tokensPerSec 缺失则只展示延迟 */
function modelMetricsLabel(m: ModelOption): string | null {
  if (m.latencyMs == null) return null
  const speed = m.tokensPerSec != null ? ` ${m.tokensPerSec.toFixed(1)}t/s` : ''
  return `${m.latencyMs}ms${speed}`
}

/**
 * 模型下拉：原生 <option> 无法把指标排到行尾，这里换成自定义 listbox。
 * 触发器外观沿用 app-select；面板绝对定位，行右侧居右显示延迟与 t/s。
 */
function ModelListbox({
  models,
  selectValue,
  placeholderName,
  disabled,
  emptyText,
  onSelect,
}: {
  models: ModelOption[]
  selectValue: string
  /** 当前模型被禁用后不在列表里，仍占一行可重新选中 */
  placeholderName: string | null
  disabled: boolean
  emptyText: string
  onSelect: (name: string) => void
}) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onPointerDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', onPointerDown)
    return () => document.removeEventListener('mousedown', onPointerDown)
  }, [open])

  const rows: ModelOption[] =
    placeholderName && !models.some((m) => m.name === placeholderName)
      ? [{ name: placeholderName, available: null }, ...models]
      : models

  const selected = rows.find((m) => m.name === selectValue)
  const triggerText = selected
    ? selected.name === placeholderName
      ? `${placeholderName}${t('upstream.modelDisabled')}`
      : modelRowLabel(selected)
    : emptyText

  const choose = (name: string) => {
    setOpen(false)
    onSelect(name)
  }

  return (
    <div
      ref={rootRef}
      className="model-listbox"
      onKeyDown={(e) => {
        if (e.key === 'Escape' && open) {
          e.stopPropagation()
          setOpen(false)
        }
      }}
    >
      <button
        type="button"
        className="app-select model-listbox-trigger"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="model-listbox-trigger-text">{triggerText}</span>
        <ChevronDown size={14} />
      </button>
      {open && !disabled && (
        <div className="model-listbox-panel" role="listbox">
          {rows.map((m) => {
            const isPlaceholder = m.name === placeholderName
            const metrics = isPlaceholder ? null : modelMetricsLabel(m)
            return (
              <div
                key={m.name}
                role="option"
                aria-selected={m.name === selectValue}
                tabIndex={0}
                className="model-listbox-option"
                onClick={() => choose(m.name)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault()
                    choose(m.name)
                  }
                }}
              >
                <span className="model-listbox-name">
                  {isPlaceholder ? `${m.name}${t('upstream.modelDisabled')}` : modelRowLabel(m)}
                </span>
                {metrics && <span className="model-listbox-metrics">{metrics}</span>}
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
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
  const [testing, setTesting] = useState(false)
  const [modelsLoaded, setModelsLoaded] = useState(false)
  const [boxModels, setBoxModels] = useState<StrategyBoxOption[]>([])

  // 策略盒子对外 model id 也是可选上游：挂载时拉一次，失败静默忽略，不打扰控制台。
  useEffect(() => {
    invoke<StrategyBoxOption[]>('list_strategy_boxes')
      .then((list) => setBoxModels(list.filter((box) => box.enabled && box.publicModelId)))
      .catch(() => undefined)
  }, [])

  // 模型下拉 = 当前服务商的启用模型 + 启用盒子的对外 model id（同名时以真实模型为准）。
  const allModels = useMemo(() => {
    const merged: ModelOption[] = [...models]
    for (const box of boxModels) {
      if (merged.some((model) => model.name === box.publicModelId)) continue
      merged.push({ name: box.publicModelId, available: null, boxName: box.name })
    }
    return merged
  }, [models, boxModels])

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

  /**
   * 取注册表里被禁用的模型名。「禁用即隐藏」：这些模型不出现在控制台。
   * 拉注册表失败时退化为 null（不过滤），只提示不报错。
   */
  const loadDisabledModels = async (providerId: string): Promise<Set<string> | null> => {
    try {
      const entries = await invoke<ModelEntry[]>('list_model_registry', {
        input: { provider_id: providerId },
      })
      return new Set(entries.filter((entry) => !entry.enabled).map((entry) => entry.modelName))
    } catch (e) {
      pushToast(String(e), 'info')
      return null
    }
  }

  const loadFreeRoutes = async () => {
    const disabled = await loadDisabledModels(FREE_PROVIDER_ID)
    const choices = await invoke<FreeRouteChoice[]>('list_experimental_free_candidates')
    const visible = disabled
      ? choices.filter((choice) => !disabled.has(choice.model))
      : choices
    setModels(
      visible.map((choice) => ({
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
      const cfg = await invoke<FetchModeConfig>('get_config')
      if (provider.experimental) {
        // 「只获取」：维持纯路由列表，不打真实补全、无指标。
        if (!(cfg.free_models_fetch_and_test ?? true)) {
          await loadFreeRoutes()
          return
        }
        const disabled = await loadDisabledModels(FREE_PROVIDER_ID)
        const choices = await invoke<FreeRouteChoice[]>(
          'list_experimental_free_candidates',
        )
        const visible = disabled
          ? choices.filter((choice) => !disabled.has(choice.model))
          : choices
        const probes = await invoke<FreeModelProbe[]>(
          'probe_experimental_free_models',
        )
        const byName = new Map(probes.map((probe) => [probe.name, probe]))
        setModels(
          visible.map((choice) => {
            const probe = byName.get(choice.model)
            return {
              name: choice.model,
              host: choice.host,
              available: probe?.available ?? false,
              latencyMs: probe?.latencyMs ?? null,
              tokensPerSec: probe?.tokensPerSec ?? null,
            }
          }),
        )
        setModelsLoaded(true)
        if (!visible.some((choice) => byName.get(choice.model)?.available)) {
          pushToast(t('upstream.freeEmpty'), 'error')
        }
        onRefresh()
      } else {
        const entries = await invoke<ModelEntry[]>('list_model_registry', {
          input: { provider_id: provider.id },
        })
        const rows: ModelOption[] = entries
          .filter((entry) => entry.enabled)
          .map((entry) => ({ name: entry.modelName, available: null }))
        // 「获取并测试」：逐模型真实探测，按 modelName 合并指标；未测到/失败的行不显示指标。
        if (cfg.custom_models_fetch_and_test ?? false) {
          const tests = await invoke<ModelTestResult[]>('test_provider_models', {
            id: provider.id,
          })
          const byModel = new Map(tests.map((item) => [item.modelName, item]))
          for (const row of rows) {
            const hit = byModel.get(row.name)
            if (!hit) continue
            row.available = hit.available
            row.latencyMs = hit.latencyMs
            row.tokensPerSec = hit.tokensPerSec
          }
        }
        setModels(rows)
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
    if (!upstreamProviderId || testing) return
    setTesting(true)
    try {
      const outcome = await invoke<ProviderTestOutcome>('test_provider', {
        id: upstreamProviderId,
      })
      if (outcome.report.ok) {
        pushToast(
          t('upstream.toastRespOk', {
            latency: outcome.report.latencyMs,
            speed: outcome.report.tokensPerSec.toFixed(1),
            reply: outcome.report.reply ?? '—',
          }),
          'ok',
        )
      } else {
        // 后端已带错误分类与引导文案，原样展示。
        pushToast(t('upstream.toastRespFail', { error: outcome.report.error ?? '—' }), 'error')
      }
    } catch (e) {
      pushToast(String(e), 'error')
    } finally {
      setTesting(false)
    }
  }

  const selectValue =
    allModels.find((m) => m.name === activeProvider?.model_name)?.name ??
    activeProvider?.model_name ??
    ''

  // 当前模型被禁用后不再出现在 models 里；仍要留一项占位，否则下拉会显示成空白。
  const currentModelName = activeProvider?.model_name ?? ''
  const currentModelHidden =
    modelsLoaded &&
    currentModelName !== '' &&
    !allModels.some((m) => m.name === currentModelName)

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
              <ModelListbox
                models={allModels}
                selectValue={selectValue}
                placeholderName={currentModelHidden ? currentModelName : null}
                disabled={modelsBusy || (allModels.length === 0 && !currentModelHidden)}
                emptyText={t('upstream.noModels')}
                onSelect={(name) => void selectModel(name)}
              />
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
              disabled={busy || testing}
              onClick={() => void testCurrent()}
              aria-label={t('upstream.test')}
            >
              <Play size={14} className={testing ? 'spin' : ''} />
            </button>
          </div>
        </div>
      )}
    </>
  )
}
