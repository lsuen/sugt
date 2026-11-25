/** 「模型列表」主 tab（v1.1.11 R2）。
 * 作用：把某个服务商返回的模型摊成可观测、可治理的清单 —— 批量探测连通性/延迟/速度，
 * 逐条启用或禁用。数据来自 model_registry 的三条命令，registry 状态与 provider 配置解耦。
 * 列表先显示再加载；探测按并发上限 3 逐条进行，每条完成即回填真实数据，
 * 未完成的行内转圈并显示进度，绝不因单条超时/失败卡住整页。 */
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { invoke, invokeWithDeadline } from './transport';
import { Play, RefreshCw } from 'lucide-react'
import { ConfirmModal } from './ConfirmModal'
import { t } from './i18n'

type ModelProbe = {
  ok: boolean
  latencyMs: number
  tokensPerSec: number
  completionTokens: number
  estimated: boolean
  probedAt: string
  error: string | null
  chatOnly?: boolean
}

type ModelEntry = {
  id: string
  providerId: string
  modelName: string
  enabled: boolean
  lastProbe: ModelProbe | null
}

type ProviderOption = {
  id: string
  name: string
  enabled: boolean
  experimental?: boolean
}

type Props = {
  providers: ProviderOption[]
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void
  run: (action: () => Promise<unknown>, ok?: string) => Promise<void>
}

/** 逐条探测的并发上限：不等整批返回，每条 resolve 就立即回填。 */
const PROBE_CONCURRENCY = 3

function formatProbeTime(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  const pad = (n: number) => String(n).padStart(2, '0')
  // 缩短列宽：不要年份
  return `${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`
}

function formatLatency(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`
}

function formatSpeed(tps: number): string {
  return tps >= 10 ? `${tps.toFixed(0)}t/s` : `${tps.toFixed(1)}t/s`
}

export function ModelListPanel({ providers, pushToast, run }: Props) {
  const [providerId, setProviderId] = useState('')
  const [entries, setEntries] = useState<ModelEntry[]>([])
  const [loading, setLoading] = useState(false)
  /** 与 loading 独立：true 表示「已成功加载过（哪怕是空列表）」，避免空列表误显示成加载中。 */
  const [loaded, setLoaded] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [probing, setProbing] = useState(false)
  /** 行内在途的真实探测：只有这些行显示「测试中」，其余立即显示实测结果。 */
  const [probeBusy, setProbeBusy] = useState<Set<string>>(new Set())
  const [probeDone, setProbeDone] = useState(0)
  const [probeTotal, setProbeTotal] = useState(0)
  const [applying, setApplying] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [rowBusy, setRowBusy] = useState<Set<string>>(new Set())
  const [confirmTestAll, setConfirmTestAll] = useState(false)
  const selectAllRef = useRef<HTMLInputElement>(null)
  /** 递增请求 token：切 provider / 卸载后丢弃过期响应，避免竞态与卸载后 setState。 */
  const requestTokenRef = useRef(0)
  const mountedRef = useRef(true)

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  // 服务商列表每 3 秒会被外层刷新替换新数组；这里只在当前选中消失时才切换，
  // 避免刷新把用户手选的 provider 打回第一个。
  useEffect(() => {
    setProviderId((prev) => {
      if (prev && providers.some((p) => p.id === prev)) return prev
      return providers.find((p) => p.enabled)?.id ?? providers[0]?.id ?? ''
    })
  }, [providers])

  const load = useCallback(async (pid: string) => {
    const token = ++requestTokenRef.current
    // 重新加载即作废上一轮探测：切 provider 后旧行不能继续转圈。
    setProbing(false)
    setProbeBusy(new Set())
    setProbeDone(0)
    setProbeTotal(0)
    if (!pid) {
      setEntries([])
      setLoaded(false)
      setLoadError(null)
      setLoading(false)
      return
    }
    setLoading(true)
    setLoadError(null)
    try {
      const list = await invokeWithDeadline<ModelEntry[]>('list_model_registry', {
        input: { provider_id: pid },
      })
      if (!mountedRef.current || token !== requestTokenRef.current) return
      setEntries(list)
      setLoaded(true)
      setSelected((prev) => {
        const keep = new Set<string>()
        for (const entry of list) if (prev.has(entry.id)) keep.add(entry.id)
        return keep
      })
    } catch (error) {
      if (!mountedRef.current || token !== requestTokenRef.current) return
      // 缓存优先：加载失败时保留既有 entries，只把错误显示出来。
      setLoaded(true)
      setLoadError(String(error))
    } finally {
      if (mountedRef.current && token === requestTokenRef.current) setLoading(false)
    }
  }, [])

  useEffect(() => {
    void load(providerId)
  }, [providerId, load])

  const allSelected = entries.length > 0 && entries.every((entry) => selected.has(entry.id))
  const someSelected = entries.some((entry) => selected.has(entry.id))

  useEffect(() => {
    if (selectAllRef.current) {
      selectAllRef.current.indeterminate = someSelected && !allSelected
    }
  }, [someSelected, allSelected])

  const toggleSelectAll = () => {
    setSelected(allSelected ? new Set() : new Set(entries.map((entry) => entry.id)))
  }

  const toggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  /** 逐条并发探测（上限 3）：每条返回立即回填该条真实数据；未完成的留在行内转圈。 */
  const probe = async (names: string[]): Promise<{ ok: number; fail: number }> => {
    let ok = 0
    let fail = 0
    if (!providerId || names.length === 0) return { ok, fail }
    const pid = providerId
    const token = requestTokenRef.current
    setProbing(true)
    setProbeDone(0)
    setProbeTotal(names.length)
    setProbeBusy(new Set(names))
    const queue = [...names]
    const worker = async () => {
      while (true) {
        const name = queue.shift()
        if (name === undefined) return
        if (!mountedRef.current || token !== requestTokenRef.current) return
        try {
          const result = await invokeWithDeadline<ModelEntry[]>('probe_models', {
            input: { provider_id: pid, model_names: [name] },
          })
          if (!mountedRef.current || token !== requestTokenRef.current) return
          const hit = result.find((entry) => entry.modelName === name) ?? result[0]
          if (hit) {
            setEntries((prev) =>
              prev.map((entry) =>
                entry.id === hit.id || entry.modelName === hit.modelName ? hit : entry,
              ),
            )
            if (hit.lastProbe?.ok) ok += 1
            else fail += 1
          }
        } catch {
          // 单条失败/超时不影响其它条目：该行保持「未知」，不计入 ok/fail。
        } finally {
          if (mountedRef.current && token === requestTokenRef.current) {
            setProbeBusy((prev) => {
              const next = new Set(prev)
              next.delete(name)
              return next
            })
            setProbeDone((prev) => prev + 1)
          }
        }
      }
    }
    const workers = Math.min(PROBE_CONCURRENCY, names.length)
    await Promise.all(Array.from({ length: workers }, () => worker()))
    if (mountedRef.current && token === requestTokenRef.current) setProbing(false)
    return { ok, fail }
  }

  const testAll = async () => {
    setConfirmTestAll(false)
    const names = entries.map((entry) => entry.modelName)
    if (names.length === 0) return
    const { ok, fail } = await probe(names)
    if (ok + fail === 0) return
    pushToast(t('modelList.toastProbeDone', { ok, fail }), fail > 0 ? 'info' : 'ok')
  }

  const testSelected = async () => {
    const names = entries
      .filter((entry) => selected.has(entry.id))
      .map((entry) => entry.modelName)
    if (names.length === 0) {
      pushToast(t('modelList.noSelection'), 'info')
      return
    }
    const { ok, fail } = await probe(names)
    if (ok + fail === 0) return
    pushToast(t('modelList.toastProbeDone', { ok, fail }), fail > 0 ? 'info' : 'ok')
  }

  const applyToSelected = async (enabled: boolean) => {
    const targets = entries.filter((entry) => selected.has(entry.id))
    if (targets.length === 0) {
      pushToast(t('modelList.noSelection'), 'info')
      return
    }
    await run(async () => {
      setApplying(true)
      try {
        for (const entry of targets) {
          await invoke('set_model_enabled', {
            input: {
              provider_id: entry.providerId,
              model_name: entry.modelName,
              enabled,
            },
          })
        }
        await load(providerId)
      } finally {
        setApplying(false)
      }
    }, t('modelList.toastToggled', { count: targets.length }))
  }

  /** 一键禁用探测失败且仍启用的模型；未知（未测）不动。 */
  const disableUnavailable = async () => {
    const targets = entries.filter(
      (entry) => entry.enabled && entry.lastProbe && !entry.lastProbe.ok,
    )
    if (targets.length === 0) {
      pushToast(t('modelList.noSelection'), 'info')
      return
    }
    await run(async () => {
      setApplying(true)
      try {
        for (const entry of targets) {
          await invoke('set_model_enabled', {
            input: {
              provider_id: entry.providerId,
              model_name: entry.modelName,
              enabled: false,
            },
          })
        }
        await load(providerId)
      } finally {
        setApplying(false)
      }
    }, t('modelList.toastDisabledUnavailable', { count: targets.length }))
  }

  const toggleRow = async (entry: ModelEntry, enabled: boolean) => {
    setRowBusy((prev) => new Set(prev).add(entry.id))
    setEntries((prev) =>
      prev.map((item) => (item.id === entry.id ? { ...item, enabled } : item)),
    )
    try {
      await invoke('set_model_enabled', {
        input: {
          provider_id: entry.providerId,
          model_name: entry.modelName,
          enabled,
        },
      })
      pushToast(
        enabled ? t('models.toastEnabled') : t('models.toastDisabled'),
        'ok',
      )
    } catch (error) {
      // 失败回滚到原状态，避免开关与服务端不一致。
      setEntries((prev) =>
        prev.map((item) =>
          item.id === entry.id ? { ...item, enabled: !enabled } : item,
        ),
      )
      pushToast(String(error), 'error')
    } finally {
      setRowBusy((prev) => {
        const next = new Set(prev)
        next.delete(entry.id)
        return next
      })
    }
  }

  // 探测中不再禁用整个页面：只有加载 / 写入这类全局操作才进入 busy，
  // 行开关、选择框等保持可用，避免「整页卡死」。
  const busy = loading || applying

  const emptyText = !providerId
    ? t('modelList.pickProvider')
    : loading && !loaded
      ? t('common.loading')
      : loadError
        ? t('modelList.loadFailed')
        : t('modelList.empty')

  return (
    <section className="card">
      <div className="model-list-toolbar">
        <select
          className="app-select"
          value={providerId}
          disabled={busy || providers.length === 0}
          onChange={(event) => {
            setSelected(new Set())
            setProviderId(event.target.value)
          }}
        >
          {providers.length === 0 && (
            <option value="">{t('modelList.pickProvider')}</option>
          )}
          {providers.map((provider) => (
            <option key={provider.id} value={provider.id}>
              {provider.name}
              {provider.enabled ? '' : t('modelList.providerDisabled')}
            </option>
          ))}
        </select>

        <button
          type="button"
          className="ghost tiny-btn"
          disabled={busy || !providerId}
          onClick={() => void load(providerId)}
        >
          <RefreshCw size={14} className={loading ? 'spin' : ''} />
          {t('modelList.refresh')}
        </button>
        <button
          type="button"
          className="ghost tiny-btn"
          disabled={busy || probing || entries.length === 0}
          onClick={() => setConfirmTestAll(true)}
        >
          <Play size={14} />
          {t('modelList.testAll')}
        </button>
        <button
          type="button"
          className="ghost tiny-btn"
          disabled={busy || probing || entries.length === 0}
          onClick={() => void testSelected()}
        >
          {t('modelList.testSelected')}
        </button>
        <button
          type="button"
          className="ghost tiny-btn"
          disabled={busy || entries.length === 0}
          onClick={() => void applyToSelected(true)}
        >
          {t('modelList.enableSelected')}
        </button>
        <button
          type="button"
          className="ghost tiny-btn"
          disabled={busy || entries.length === 0}
          onClick={() => void applyToSelected(false)}
        >
          {t('modelList.disableSelected')}
        </button>
        <button
          type="button"
          className="ghost tiny-btn"
          disabled={busy || entries.length === 0}
          onClick={() => void disableUnavailable()}
          title={t('modelList.disableUnavailable')}
        >
          {t('modelList.disableUnavailable')}
        </button>
        {probing && (
          <span className="hint compact model-list-note">
            {t('modelList.probing')}
            {' · '}
            {t('modelList.probeProgress', { done: probeDone, total: probeTotal })}
          </span>
        )}
      </div>

      {loadError && <p className="error-text model-list-note">{loadError}</p>}

      <div className="model-list-wrap">
        <table className="model-list-table">
          <thead>
            <tr>
              <th className="model-list-check">
                <input
                  ref={selectAllRef}
                  type="checkbox"
                  aria-label={t('modelList.selectAll')}
                  checked={allSelected}
                  disabled={entries.length === 0}
                  onChange={toggleSelectAll}
                />
              </th>
              <th className="model-list-col-name">{t('modelList.colName')}</th>
              <th>{t('modelList.colStatus')}</th>
              <th className="model-list-col-metric">{t('modelList.colLatency')}</th>
              <th className="model-list-col-metric">{t('modelList.colSpeed')}</th>
              <th className="model-list-col-time">{t('modelList.colTestedAt')}</th>
              <th>{t('modelList.colEnabled')}</th>
            </tr>
          </thead>
          <tbody>
            {entries.length === 0 && (
              <tr>
                <td colSpan={7} className="model-list-empty">
                  {emptyText}
                </td>
              </tr>
            )}
            {entries.map((entry) => {
              const probeResult = entry.lastProbe
              const statusClass = probeResult
                ? probeResult.ok
                  ? 'is-ok'
                  : 'is-fail'
                : 'is-unknown'
              const statusLabel = probeResult
                ? probeResult.ok
                  ? t('modelList.statusOk')
                  : t('modelList.statusFail')
                : t('modelList.statusUnknown')
              return (
                <tr key={entry.id} className={entry.enabled ? '' : 'is-disabled'}>
                  <td className="model-list-check">
                    <input
                      type="checkbox"
                      aria-label={entry.modelName}
                      checked={selected.has(entry.id)}
                      onChange={() => toggleSelect(entry.id)}
                    />
                  </td>
                  <td className="model-list-col-name">
                    <code className="model-list-name-marquee" title={entry.modelName}>
                      <span>{entry.modelName}</span>
                    </code>
                    {probeResult?.ok && (
                      <span className="model-list-cap">
                        {probeResult.chatOnly
                          ? t('modelList.capChatOnly')
                          : t('modelList.capTools')}
                      </span>
                    )}
                  </td>
                  <td>
                    {probeBusy.has(entry.modelName) ? (
                      <span
                        className="model-list-status hint compact"
                        title={t('modelList.probing')}
                      >
                        <RefreshCw size={12} className="spin" />
                        {t('modelList.probing')}
                      </span>
                    ) : (
                      <span
                        className="model-list-status"
                        title={probeResult?.error || statusLabel}
                      >
                        <span className={`model-list-dot ${statusClass}`} />
                        {statusLabel}
                      </span>
                    )}
                  </td>
                  <td className="model-list-col-metric">
                    {probeResult?.ok && probeResult.latencyMs > 0
                      ? formatLatency(probeResult.latencyMs)
                      : '—'}
                  </td>
                  <td className="model-list-col-metric">
                    {probeResult?.ok && probeResult.tokensPerSec > 0
                      ? formatSpeed(probeResult.tokensPerSec)
                      : '—'}
                  </td>
                  <td className="model-list-col-time">
                    {probeResult ? formatProbeTime(probeResult.probedAt) : '—'}
                  </td>
                  <td>
                    <input
                      className="model-list-switch"
                      type="checkbox"
                      aria-label={t('modelList.colEnabled')}
                      checked={entry.enabled}
                      disabled={busy || rowBusy.has(entry.id)}
                      onChange={(event) =>
                        void toggleRow(entry, event.target.checked)
                      }
                    />
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      <ConfirmModal
        open={confirmTestAll}
        danger={false}
        title={t('modelList.confirmTestAllTitle')}
        body={t('modelList.confirmTestAllBody', { count: entries.length })}
        busy={probing}
        onConfirm={() => void testAll()}
        onCancel={() => setConfirmTestAll(false)}
      />
    </section>
  )
}
