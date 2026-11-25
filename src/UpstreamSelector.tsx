import React, { useEffect, useMemo, useRef, useState } from 'react'
import { invokeWithDeadline } from './transport';
import { ChevronDown, Play, RefreshCw, RotateCcw, Shuffle } from 'lucide-react'
import { t } from './i18n'
import { FreeChannelWelcome } from './FreeChannelWelcome'

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
  /** 与后端 ProviderView 对齐：下列字段随 providers 原样透传，不在本组件改写 */
  public_model_id?: string
  status?: 'Unknown' | 'Available' | 'Unavailable'
  last_checked_at?: string | null
  last_error?: string | null
  auto_adapt_base_url?: boolean
  tool_text_retry_enabled?: boolean
  tool_text_retry_extra?: number
  /** 用户配置的公开别名；切换模型时原样回传，避免被前端默认值覆盖 */
  model_alias?: string | null
  experimental?: boolean
}

type FreeModelProbe = {
  name: string
  available: boolean
  latencyMs: number | null
  tokensPerSec: number | null
  chatOnly?: boolean
  source?: 'live' | 'cache'
  category?: string
}
type FreeRouteChoice = { model: string; host: string }

type ModelOption = {
  name: string
  host?: string
  available: boolean | null
  boxName?: string
  latencyMs?: number | null
  tokensPerSec?: number | null
  chatOnly?: boolean
  source?: 'live' | 'cache'
  /** 真实在途探测（含排队中）：仅这些行显示行内转圈，结果返回即清除 */
  probing?: boolean
}

/** get_config 里本组件只关心「获取模型行为」两个开关（snake_case，缺省按产品默认） */
type FetchModeConfig = {
  free_models_fetch_and_test?: boolean
  custom_models_fetch_and_test?: boolean
}

/** 与 src-tauri/src/strategy_box.rs 的 StrategyBox 对齐（这里只关心对外 model id） */
type StrategyBoxOption = { id: string; name: string; publicModelId: string; enabled: boolean }

/** probe_models 写回注册表的真实探测报告 */
type RegistryProbeView = {
  ok: boolean
  latencyMs: number
  tokensPerSec: number
  error?: string | null
}

/** 与 src-tauri/src/model_registry.rs 的 ModelEntry 对齐（这里只关心启用状态与探测结果） */
type ModelEntry = {
  modelName: string
  enabled: boolean
  lastProbe?: RegistryProbeView | null
}

const FREE_PROVIDER_ID = 'sugt-free'

/** 逐条探测的并发上限：先显示列表，再按真实结果逐行回填。 */
const PROBE_CONCURRENCY = 3

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
  /** 首启欢迎条「打开通道设置」→ 打开 FreeChannelSettingsModal */
  onOpenFreeSettings?: () => void
}

/** 触发器/行主文案：只留点 + id + 短 host，能力与指标放右侧徽章，避免撑破容器 */
function modelRowLabel(m: ModelOption): string {
  const host = m.host ? ` · ${shortHost(m.host)}` : ''
  return `${m.available === true ? '● ' : m.available === false ? '○ ' : ''}${m.name}${
    m.boxName ? t('upstream.strategyBoxTag') : ''
  }${host}`
}

function shortHost(host: string): string {
  return host.replace(/^api\./, '').replace(/\.ai$/, '').slice(0, 18)
}

/** 紧凑指标：≥1s 用秒，否则 ms；速度取整 */
function modelMetricsLabel(m: ModelOption): string | null {
  if (m.latencyMs == null && !m.chatOnly && m.source !== 'cache') return null
  const parts: string[] = []
  if (m.latencyMs != null) {
    parts.push(
      m.latencyMs >= 1000
        ? `${(m.latencyMs / 1000).toFixed(1)}s`
        : `${m.latencyMs}ms`,
    )
  }
  if (m.tokensPerSec != null && m.tokensPerSec > 0) {
    parts.push(`${m.tokensPerSec >= 10 ? m.tokensPerSec.toFixed(0) : m.tokensPerSec.toFixed(1)}t/s`)
  }
  if (m.chatOnly) parts.push(t('upstream.chatOnlyShort'))
  if (m.source === 'cache') parts.push(t('upstream.cacheShort'))
  return parts.length ? parts.join(' · ') : null
}

function probeToModelFields(probe: FreeModelProbe | undefined): Pick<
  ModelOption,
  'available' | 'latencyMs' | 'tokensPerSec' | 'chatOnly' | 'source'
> {
  return {
    // 没有探测结果就是「未知」，不能显示成「不可用」——那是编造的结论。
    available: probe?.available ?? null,
    latencyMs: probe?.latencyMs ?? null,
    tokensPerSec: probe?.tokensPerSec ?? null,
    chatOnly: Boolean(probe?.chatOnly),
    source: probe?.source,
  }
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
                title={isPlaceholder ? m.name : `${m.name}${m.host ? ` @ ${m.host}` : ''}${metrics ? ` · ${metrics}` : ''}`}
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
                {m.probing && (
                  <span className="spin" title={t('upstream.probing')}>
                    <RefreshCw size={12} aria-label={t('upstream.probing')} />
                  </span>
                )}
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
  onOpenFreeSettings,
}: Props) {
  const [upstreamProviderId, setUpstreamProviderId] = useState('')
  const [models, setModels] = useState<ModelOption[]>([])
  const [modelsBusy, setModelsBusy] = useState(false)
  const [fetchError, setFetchError] = useState<string | null>(null)
  const [testing, setTesting] = useState(false)
  const [modelsLoaded, setModelsLoaded] = useState(false)
  const [probeProgress, setProbeProgress] = useState<{ done: number; total: number } | null>(null)
  const [boxModels, setBoxModels] = useState<StrategyBoxOption[]>([])

  // 策略盒子对外 model id 也是可选上游：挂载时拉一次，失败静默忽略，不打扰控制台。
  useEffect(() => {
    invokeWithDeadline<StrategyBoxOption[]>('list_strategy_boxes')
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
      const entries = await invokeWithDeadline<ModelEntry[]>('list_model_registry', {
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
    const [choices, cached] = await Promise.all([
      invokeWithDeadline<FreeRouteChoice[]>('list_experimental_free_candidates'),
      invokeWithDeadline<FreeModelProbe[]>('list_experimental_free_probe_cache').catch(
        () => [] as FreeModelProbe[],
      ),
    ])
    const visible = disabled
      ? choices.filter((choice) => !disabled.has(choice.model))
      : choices
    const byName = new Map(cached.map((probe) => [probe.name, probe]))
    setModels(
      visible.map((choice) => ({
        name: choice.model,
        host: choice.host,
        ...probeToModelFields(byName.get(choice.model)),
      })),
    )
    setModelsLoaded(true)
  }

  /**
   * 并发上限 3 的逐条探测：每条 resolve 立即回填，未完成的保留行内转圈；
   * 单条失败/超时只让该行回到「未知」，不拖垮整轮。
   */
  const probeInPool = async <T,>(
    items: T[],
    keyOf: (item: T) => string,
    probeOne: (item: T) => Promise<void>,
  ): Promise<void> => {
    if (items.length === 0) return
    let done = 0
    setProbeProgress({ done: 0, total: items.length })
    const queue = [...items]
    const worker = async () => {
      while (true) {
        const item = queue.shift()
        if (item === undefined) return
        try {
          await probeOne(item)
        } catch {
          // 失败/超时不伪造可用状态，只让该行保持「未知」。
        } finally {
          setModels((prev) =>
            prev.map((row) =>
              row.name === keyOf(item) ? { ...row, probing: false } : row,
            ),
          )
          done += 1
          setProbeProgress({ done, total: items.length })
        }
      }
    }
    const workers = Math.min(PROBE_CONCURRENCY, items.length)
    await Promise.all(Array.from({ length: workers }, () => worker()))
  }

  const fetchModels = async () => {
    const provider = providers.find((p) => p.id === upstreamProviderId)
    if (!provider) return
    setModelsBusy(true)
    setFetchError(null)
    setProbeProgress(null)
    try {
      const cfg = await invokeWithDeadline<FetchModeConfig>('get_config')
      if (provider.experimental) {
        const disabled = await loadDisabledModels(FREE_PROVIDER_ID)
        const choices = await invokeWithDeadline<FreeRouteChoice[]>(
          'list_experimental_free_candidates',
        )
        const visible = disabled
          ? choices.filter((choice) => !disabled.has(choice.model))
          : choices
        // 先用真实缓存把列表立即显示出来，用户不用等整轮测速；
        // 缓存里没有的路由保持「未知」，绝不显示成「不可用」。
        const cached = await invokeWithDeadline<FreeModelProbe[]>(
          'list_experimental_free_probe_cache',
        ).catch(() => [] as FreeModelProbe[])
        const cachedByName = new Map(cached.map((probe) => [probe.name, probe]))
        setModels(
          visible.map((choice) => ({
            name: choice.model,
            host: choice.host,
            ...probeToModelFields(cachedByName.get(choice.model)),
          })),
        )
        setModelsLoaded(true)

        // 只有开启「获取并测试」才打真实补全：对可见路由逐条调用单模型命令，
        // 缓存仅用于立即兜底展示，绝不冒充真实探测结果。
        if (cfg.free_models_fetch_and_test ?? true) {
          setModels((prev) =>
            prev.map((row) =>
              visible.some((choice) => choice.model === row.name)
                ? { ...row, probing: true }
                : row,
            ),
          )
          let anyAvailable = false
          await probeInPool(visible, (choice) => choice.model, async (choice) => {
            const probe = await invokeWithDeadline<FreeModelProbe>(
              'probe_experimental_free_model',
              { model: choice.model },
            )
            setModels((prev) =>
              prev.map((row) =>
                row.name === probe.name ? { ...row, ...probeToModelFields(probe) } : row,
              ),
            )
            if (probe.available) anyAvailable = true
          })
          if (!anyAvailable) pushToast(t('upstream.freeEmpty'), 'error')
        }
        pushToast(t('upstream.fetchedModels', { count: visible.length }), 'ok')
        onRefresh()
      } else {
        const entries = await invokeWithDeadline<ModelEntry[]>('list_model_registry', {
          input: { provider_id: provider.id },
        })
        const shouldProbe = cfg.custom_models_fetch_and_test ?? false
        const rows: ModelOption[] = entries
          .filter((entry) => entry.enabled)
          .map((entry) => ({ name: entry.modelName, available: null, probing: shouldProbe }))
        // 先让列表立即出现（available:null = 未知），再逐条真实探测回填。
        setModels(rows)
        setModelsLoaded(true)
        if (shouldProbe) {
          await probeInPool(rows, (row) => row.name, async (row) => {
            const result = await invokeWithDeadline<ModelEntry[]>('probe_models', {
              input: { provider_id: provider.id, model_names: [row.name] },
            })
            const report =
              result.find((entry) => entry.modelName === row.name)?.lastProbe ??
              result[0]?.lastProbe
            if (!report) return
            setModels((prev) =>
              prev.map((item) =>
                item.name === row.name
                  ? {
                      ...item,
                      available: report.ok,
                      latencyMs: report.latencyMs,
                      tokensPerSec: report.tokensPerSec,
                    }
                  : item,
              ),
            )
          })
        }
        pushToast(t('upstream.fetchedModels', { count: rows.length }), 'ok')
        onRefresh()
      }
    } catch (e) {
      const message = String(e)
      setFetchError(message)
      pushToast(message, 'error')
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
      // 保留该服务商已有的别名：后端 commands.rs 会把空串过滤成 None 并回落 CLIENT_MODEL_ID，
      // 也就是「传空串 = 清空别名」，并非「不改写」。别名必须原样回传，否则切模型会静默清空用户别名。
      model_alias: provider.model_alias ?? '',
      protocol:
        provider.protocol === 'open_ai' ? 'openai' : provider.protocol,
      enabled: provider.enabled,
      auto_adapt_base_url: true,
      tool_text_retry_enabled: provider.tool_text_retry_enabled ?? true,
      tool_text_retry_extra: provider.tool_text_retry_extra ?? 2,
    }
    if (opts?.silent) {
      try {
        await invokeWithDeadline('save_provider', { input })
        onRefresh()
      } catch (e) {
        pushToast(String(e), 'error')
      }
      return
    }
    await run(() => invokeWithDeadline('save_provider', { input }), t('upstream.toastSwitched'))
    onRefresh()
  }

  const testCurrent = async () => {
    if (!upstreamProviderId || testing) return
    setTesting(true)
    try {
      const outcome = await invokeWithDeadline<ProviderTestOutcome>('test_provider', {
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

  /** 只强制重测当前选中的免费路由，不全量打网。 */
  const reprobeCurrent = async () => {
    const model = activeProvider?.model_name
    if (!model || !isExperimental || modelsBusy) return
    setModelsBusy(true)
    setProbeProgress(null)
    try {
      const probe = await invokeWithDeadline<FreeModelProbe>('probe_experimental_free_model', {
        model,
      })
      setModels((prev) =>
        prev.map((row) =>
          row.name === probe.name
            ? { ...row, ...probeToModelFields(probe) }
            : row,
        ),
      )
      if (probe.available) {
        pushToast(
          probe.chatOnly
            ? t('upstream.reprobeChatOnly', { model: probe.name })
            : t('upstream.reprobeOk', { model: probe.name }),
          'ok',
        )
      } else {
        pushToast(t('upstream.reprobeFail', { model: probe.name }), 'error')
      }
      onRefresh()
    } catch (e) {
      pushToast(String(e), 'error')
    } finally {
      setModelsBusy(false)
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
      <FreeChannelWelcome providers={providers} onOpenFreeSettings={onOpenFreeSettings} />
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
            {modelsBusy && (isExperimental || probeProgress !== null) && (
              <span className="hint compact upstream-probe-hint">
                {t('upstream.probing')}
                {probeProgress !== null &&
                  ` · ${t('upstream.probeProgress', {
                    done: probeProgress.done,
                    total: probeProgress.total,
                  })}`}
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
              title={t('upstream.fetchModels')}
            >
              <RefreshCw size={14} className={modelsBusy ? 'spin' : ''} />
            </button>
            {isExperimental && (
              <button
                type="button"
                className="tiny icon-only"
                disabled={modelsBusy || !activeProvider?.model_name}
                onClick={() => void reprobeCurrent()}
                aria-label={t('upstream.reprobeOne')}
                title={t('upstream.reprobeOneTitle')}
              >
                <RotateCcw size={14} className={modelsBusy ? 'spin' : ''} />
              </button>
            )}
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
          {fetchError && !modelsBusy && (
            <p className="hint compact error-text" role="alert">
              {fetchError}
            </p>
          )}
        </div>
      )}
    </>
  )
}
