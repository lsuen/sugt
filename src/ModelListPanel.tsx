/** 「模型列表」主 tab（v1.1.11 R2）。
 * 作用：把某个服务商返回的模型摊成可观测、可治理的清单 —— 批量探测连通性/延迟/速度，
 * 逐条启用或禁用。数据来自 model_registry 的三条命令，registry 状态与 provider 配置解耦。
 * 批量探测按 3 条一批调用，批间 await，保证 UI 不卡死且进度逐批回填。 */
import React, { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from './transport';
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

/** 分批并发：每批最多 3 个 model_name。 */
const PROBE_BATCH = 3

function formatProbeTime(iso: string): string {
  const date = new Date(iso)
  if (Number.isNaN(date.getTime())) return iso
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`
}

export function ModelListPanel({ providers, pushToast, run }: Props) {
  const [providerId, setProviderId] = useState('')
  const [entries, setEntries] = useState<ModelEntry[]>([])
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [probing, setProbing] = useState(false)
  const [applying, setApplying] = useState(false)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [rowBusy, setRowBusy] = useState<Set<string>>(new Set())
  const [confirmTestAll, setConfirmTestAll] = useState(false)
  const selectAllRef = useRef<HTMLInputElement>(null)

  // 服务商列表每 3 秒会被外层刷新替换新数组；这里只在当前选中消失时才切换，
  // 避免刷新把用户手选的 provider 打回第一个。
  useEffect(() => {
    setProviderId((prev) => {
      if (prev && providers.some((p) => p.id === prev)) return prev
      return providers.find((p) => p.enabled)?.id ?? providers[0]?.id ?? ''
    })
  }, [providers])

  const load = useCallback(async (pid: string) => {
    if (!pid) {
      setEntries([])
      setLoadError(null)
      return
    }
    setLoading(true)
    setLoadError(null)
    try {
      const list = await invoke<ModelEntry[]>('list_model_registry', {
        input: { provider_id: pid },
      })
      setEntries(list)
      setSelected((prev) => {
        const keep = new Set<string>()
        for (const entry of list) if (prev.has(entry.id)) keep.add(entry.id)
        return keep
      })
    } catch (error) {
      setEntries([])
      setLoadError(String(error))
    } finally {
      setLoading(false)
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

  /** 逐批探测并即时合并结果；返回成功/失败条数。 */
  const probe = async (names: string[]): Promise<{ ok: number; fail: number }> => {
    let ok = 0
    let fail = 0
    if (!providerId || names.length === 0) return { ok, fail }
    setProbing(true)
    try {
      for (let i = 0; i < names.length; i += PROBE_BATCH) {
        const batch = names.slice(i, i + PROBE_BATCH)
        const result = await invoke<ModelEntry[]>('probe_models', {
          input: { provider_id: providerId, model_names: batch },
        })
        const merged = new Map(result.map((entry) => [entry.id, entry]))
        for (const entry of result) {
          if (entry.lastProbe?.ok) ok += 1
          else fail += 1
        }
        setEntries((prev) => prev.map((entry) => merged.get(entry.id) ?? entry))
      }
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setProbing(false)
    }
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

  const busy = loading || probing || applying

  const emptyText = !providerId
    ? t('modelList.pickProvider')
    : loading
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
          disabled={busy || entries.length === 0}
          onClick={() => setConfirmTestAll(true)}
        >
          <Play size={14} />
          {t('modelList.testAll')}
        </button>
        <button
          type="button"
          className="ghost tiny-btn"
          disabled={busy || entries.length === 0}
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
        {probing && (
          <span className="hint compact model-list-note">
            {t('modelList.probing')}
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
              <th>{t('modelList.colName')}</th>
              <th>{t('modelList.colStatus')}</th>
              <th>{t('modelList.colLatency')}</th>
              <th>{t('modelList.colSpeed')}</th>
              <th>{t('modelList.colTestedAt')}</th>
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
                  <td>
                    <code className="model-list-name">{entry.modelName}</code>
                  </td>
                  <td>
                    <span className="model-list-status" title={statusLabel}>
                      <span className={`model-list-dot ${statusClass}`} />
                      {statusLabel}
                    </span>
                  </td>
                  <td>
                    {probeResult?.ok && probeResult.latencyMs > 0
                      ? `${probeResult.latencyMs} ms`
                      : '—'}
                  </td>
                  <td>
                    {probeResult?.ok && probeResult.tokensPerSec > 0
                      ? `${probeResult.tokensPerSec.toFixed(1)} t/s`
                      : '—'}
                  </td>
                  <td>
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
