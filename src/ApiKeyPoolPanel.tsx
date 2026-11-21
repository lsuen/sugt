/** 「API Key 池」主 tab（PRD v1.1.11 T5.3）。
 * 作用：一个 provider 绑定一个 key 池，池内多个 key 按轮询 / 最少使用自动切换，
 * 并把每个 key 的请求数、token 用量、最近错误与配额告警摊开可观测。
 * 本组件只负责编辑与展示；选路、累计、落盘全部由 Rust 网关完成，
 * 命令为 list_api_key_pools / save_api_key_pool / delete_api_key_pool / reset_key_usage。
 * 删除池会连同该池全部 key 与用量记录一起移除，统一走 ConfirmModal。 */
import { useCallback, useEffect, useState } from 'react'
import { invoke } from './transport';
import { Plus, RefreshCw, Trash2 } from 'lucide-react'
import { ConfirmModal } from './ConfirmModal'
import { t, type TKey } from './i18n'

type KeySelection = 'roundRobin' | 'leastUsed'

type KeyUsage = {
  requestCount: number
  totalTokens: number
  lastError: string | null
}

type KeyQuota = {
  requestLimit: number | null
  tokenLimit: number | null
}

type ApiKeyEntryView = {
  id: string
  key: string
  keyMasked: string
  enabled: boolean
  usage: KeyUsage
  quota: KeyQuota | null
}

type ApiKeyPoolView = {
  id: string
  name: string
  providerId: string
  selection: KeySelection
  keys: ApiKeyEntryView[]
}

type ProviderOption = {
  id: string
  name: string
  enabled: boolean
}

type Props = {
  providers: ProviderOption[]
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void
  run: (action: () => Promise<unknown>, ok?: string) => Promise<void>
}

/** 编辑态 key：配额先存字符串（留空 = 不设配额），保存时再校验转数字。 */
type DraftKey = {
  id: string
  key: string
  keyMasked: string
  enabled: boolean
  usage: KeyUsage
  requestLimit: string
  tokenLimit: string
  /** 原配置里是否有配额对象：后端在 quota 为 null 时会保留旧配额，清除需要显式传空对象。 */
  hadQuota: boolean
}

type DraftPool = {
  id: string
  name: string
  providerId: string
  selection: KeySelection
  keys: DraftKey[]
  newKey: string
  saving: boolean
  error: string | null
}

/** 与 Rust `model::mask_secret` 对齐：≤8 位整体打码，否则保留前 4 位与后 4 位。 */
function maskKey(key: string): string {
  if (key.length <= 8) return '********'
  return `${key.slice(0, 4)}****${key.slice(-4)}`
}

function parseLimit(value: string): number | null {
  const trimmed = value.trim()
  if (!trimmed) return null
  const parsed = Number(trimmed)
  return Number.isInteger(parsed) && parsed >= 1 ? parsed : null
}

function keyOverQuota(key: DraftKey): boolean {
  const requestLimit = parseLimit(key.requestLimit)
  const tokenLimit = parseLimit(key.tokenLimit)
  return (
    (requestLimit !== null && key.usage.requestCount >= requestLimit) ||
    (tokenLimit !== null && key.usage.totalTokens >= tokenLimit)
  )
}

function toDraftKey(entry: ApiKeyEntryView): DraftKey {
  const quota = entry.quota
  return {
    id: entry.id,
    key: entry.key,
    keyMasked: entry.keyMasked,
    enabled: entry.enabled,
    usage: entry.usage,
    requestLimit: quota?.requestLimit != null ? String(quota.requestLimit) : '',
    tokenLimit: quota?.tokenLimit != null ? String(quota.tokenLimit) : '',
    hadQuota: quota !== null,
  }
}

function toDraftPool(pool: ApiKeyPoolView): DraftPool {
  return {
    id: pool.id,
    name: pool.name,
    providerId: pool.providerId,
    selection: pool.selection,
    keys: pool.keys.map(toDraftKey),
    newKey: '',
    saving: false,
    error: null,
  }
}

function emptyPool(providerId: string): DraftPool {
  return {
    id: '',
    name: '',
    providerId,
    selection: 'roundRobin',
    keys: [],
    newKey: '',
    saving: false,
    error: null,
  }
}

function newDraftKey(key: string): DraftKey {
  return {
    id: '',
    key,
    keyMasked: maskKey(key),
    enabled: true,
    usage: { requestCount: 0, totalTokens: 0, lastError: null },
    requestLimit: '',
    tokenLimit: '',
    hadQuota: false,
  }
}

/** 前端先挡明显错误，后端仍会返回完整校验原因并原样 toast。 */
function validatePool(pool: DraftPool): TKey | null {
  if (!pool.name.trim()) return 'keyPool.errorNameRequired'
  if (!pool.providerId.trim()) return 'keyPool.errorProviderRequired'
  if (pool.keys.length === 0) return 'keyPool.errorPoolKeyRequired'
  for (const key of pool.keys) {
    if (!key.key.trim()) return 'keyPool.errorKeyRequired'
    for (const raw of [key.requestLimit, key.tokenLimit]) {
      if (!raw.trim()) continue
      if (parseLimit(raw) === null) return 'keyPool.errorQuotaRange'
    }
  }
  return null
}

function quotaPayload(key: DraftKey): KeyQuota | null {
  const requestLimit = parseLimit(key.requestLimit)
  const tokenLimit = parseLimit(key.tokenLimit)
  if (requestLimit === null && tokenLimit === null) {
    // 后端在 quota 为 null 时会保留旧配额，显式空对象才能真正清除已有限制。
    return key.hadQuota ? { requestLimit: null, tokenLimit: null } : null
  }
  return { requestLimit, tokenLimit }
}

export function ApiKeyPoolPanel({ providers, pushToast, run }: Props) {
  const [pools, setPools] = useState<DraftPool[]>([])
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [resetting, setResetting] = useState<string | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<{ index: number; id: string; name: string } | null>(null)
  const [deleting, setDeleting] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const list = await invoke<ApiKeyPoolView[]>('list_api_key_pools')
      setPools(list.map(toDraftPool))
    } catch (error) {
      // 刷新失败保留错误行，由上层提示重试，而不是白屏。
      setPools([])
      setLoadError(String(error))
    } finally {
      setLoading(false)
    }
  }, [])

  // 切 tab 会卸载再挂载，挂载拉一次即等价于切回来刷新。
  useEffect(() => {
    void load()
  }, [load])

  const savingAny = pools.some((pool) => pool.saving)

  const patchPool = (index: number, patch: Partial<DraftPool>) => {
    setPools((prev) => prev.map((pool, i) => (i === index ? { ...pool, ...patch } : pool)))
  }

  const patchKey = (poolIndex: number, keyIndex: number, patch: Partial<DraftKey>) => {
    setPools((prev) =>
      prev.map((pool, i) =>
        i === poolIndex
          ? {
            ...pool,
            error: null,
            keys: pool.keys.map((key, ki) => (ki === keyIndex ? { ...key, ...patch } : key)),
          }
          : pool,
      ),
    )
  }

  /** 新建池默认挑一个尚未被其他池占用的 provider（网关只认第一个绑定它的池）。 */
  const pickFreeProvider = (current: DraftPool[]): string => {
    const used = new Set(current.map((pool) => pool.providerId).filter(Boolean))
    return (
      providers.find((provider) => provider.enabled && !used.has(provider.id))?.id ??
      providers.find((provider) => !used.has(provider.id))?.id ??
      ''
    )
  }

  const addPool = () => {
    setPools((prev) => [...prev, emptyPool(pickFreeProvider(prev))])
  }

  const addKey = (poolIndex: number) => {
    setPools((prev) =>
      prev.map((pool, i) => {
        if (i !== poolIndex) return pool
        const key = pool.newKey.trim()
        if (!key) return { ...pool, error: t('keyPool.errorKeyRequired') }
        return {
          ...pool,
          newKey: '',
          error: null,
          keys: [...pool.keys, newDraftKey(key)],
        }
      }),
    )
  }

  const removeKey = (poolIndex: number, keyIndex: number) => {
    setPools((prev) =>
      prev.map((pool, i) =>
        i === poolIndex
          ? { ...pool, error: null, keys: pool.keys.filter((_, ki) => ki !== keyIndex) }
          : pool,
      ),
    )
  }

  const savePool = async (poolIndex: number) => {
    const pool = pools[poolIndex]
    if (!pool || pool.saving) return
    const invalid = validatePool(pool)
    if (invalid) {
      patchPool(poolIndex, { error: t(invalid) })
      pushToast(t(invalid), 'error')
      return
    }
    patchPool(poolIndex, { saving: true, error: null })
    try {
      const spec = {
        id: pool.id,
        name: pool.name.trim(),
        providerId: pool.providerId,
        selection: pool.selection,
        keys: pool.keys.map((key) => ({
          id: key.id,
          key: key.key,
          enabled: key.enabled,
          usage: key.usage,
          quota: quotaPayload(key),
        })),
      }
      await invoke('save_api_key_pool', { input: { spec } })
      pushToast(t('keyPool.toastSaved'), 'ok')
      await load()
    } catch (error) {
      patchPool(poolIndex, { saving: false, error: String(error) })
      pushToast(String(error), 'error')
    }
  }

  const resetUsage = async (poolIndex: number, keyId: string | null) => {
    const pool = pools[poolIndex]
    if (!pool?.id) return
    const token = `${pool.id}:${keyId ?? '*'}`
    setResetting(token)
    try {
      await invoke('reset_key_usage', {
        input: keyId ? { poolId: pool.id, keyId } : { poolId: pool.id },
      })
      // 直接改本地草稿的用量，不整表重载：避免把用户尚未保存的编辑冲掉。
      setPools((prev) =>
        prev.map((item, i) =>
          i === poolIndex
            ? {
              ...item,
              keys: item.keys.map((key) =>
                !keyId || key.id === keyId
                  ? { ...key, usage: { requestCount: 0, totalTokens: 0, lastError: null } }
                  : key,
              ),
            }
            : item,
        ),
      )
      pushToast(t(keyId ? 'keyPool.toastResetKey' : 'keyPool.toastResetPool'), 'ok')
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setResetting(null)
    }
  }

  const confirmDelete = async () => {
    if (!deleteTarget) return
    const target = deleteTarget
    setDeleteTarget(null)
    // 尚未保存的新池只存在于本地草稿，直接从列表移除即可。
    if (!target.id) {
      setPools((prev) => prev.filter((_, i) => i !== target.index))
      return
    }
    setDeleting(true)
    try {
      await run(
        () => invoke('delete_api_key_pool', { input: { poolId: target.id } }),
        t('keyPool.toastDeleted'),
      )
      await load()
    } finally {
      setDeleting(false)
    }
  }

  return (
    <div className="key-pool-panel">
      <section className="card">
        <div className="section-title">
          <h3>{t('keyPool.title')}</h3>
          <div className="title-actions">
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={loading || savingAny || deleting}
              onClick={() => void load()}
            >
              <RefreshCw size={14} className={loading ? 'spin' : ''} />
              {t('keyPool.refresh')}
            </button>
            <button
              type="button"
              className="primary tiny-btn"
              disabled={loading || savingAny || deleting}
              onClick={addPool}
            >
              <Plus size={14} />
              {t('keyPool.newPool')}
            </button>
          </div>
        </div>

        <p className="hint compact key-pool-note">{t('keyPool.hint')}</p>

        {loadError && (
          <div className="request-rules-error">
            <p className="error-text key-pool-note">{t('keyPool.loadFailed', { error: loadError })}</p>
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={loading}
              onClick={() => void load()}
            >
              {t('keyPool.retry')}
            </button>
          </div>
        )}

        {!loading && !loadError && pools.length === 0 && (
          <p className="hint key-pool-empty-text">{t('keyPool.empty')}</p>
        )}
      </section>

      {pools.map((pool, poolIndex) => (
        <section className="card key-pool-card" key={pool.id || `draft-${poolIndex}`}>
          <div className="key-pool-head">
            <label className="field-label key-pool-name">
              {t('keyPool.nameLabel')}
              <input
                value={pool.name}
                placeholder={t('keyPool.namePlaceholder')}
                disabled={pool.saving}
                onChange={(event) => patchPool(poolIndex, { name: event.target.value, error: null })}
              />
            </label>

            <label className="field-label">
              {t('keyPool.providerLabel')}
              <select
                className="app-select"
                value={pool.providerId}
                disabled={pool.saving || providers.length === 0}
                onChange={(event) => patchPool(poolIndex, { providerId: event.target.value, error: null })}
              >
                {!pool.providerId && (
                  <option value="" disabled>
                    {t('keyPool.pickProvider')}
                  </option>
                )}
                {providers.map((provider) => {
                  const boundByOther = pools.some(
                    (other, i) => i !== poolIndex && other.providerId === provider.id,
                  )
                  return (
                    <option key={provider.id} value={provider.id} disabled={boundByOther}>
                      {provider.name}
                      {provider.enabled ? '' : t('keyPool.providerDisabled')}
                      {boundByOther ? t('keyPool.providerBound') : ''}
                    </option>
                  )
                })}
              </select>
            </label>

            <label className="field-label">
              {t('keyPool.selectionLabel')}
              <select
                className="app-select"
                value={pool.selection}
                disabled={pool.saving}
                onChange={(event) =>
                  patchPool(poolIndex, { selection: event.target.value as KeySelection })
                }
              >
                <option value="roundRobin">{t('keyPool.selectionRoundRobin')}</option>
                <option value="leastUsed">{t('keyPool.selectionLeastUsed')}</option>
              </select>
            </label>

            <div className="title-actions key-pool-actions">
              <button
                type="button"
                className="primary tiny-btn"
                disabled={pool.saving || deleting}
                onClick={() => void savePool(poolIndex)}
              >
                {pool.saving ? t('keyPool.saving') : t('keyPool.save')}
              </button>
              <button
                type="button"
                className="ghost tiny-btn"
                disabled={!pool.id || pool.saving || resetting !== null || deleting}
                onClick={() => void resetUsage(poolIndex, null)}
              >
                {t('keyPool.resetPool')}
              </button>
              <button
                type="button"
                className="ghost tiny-btn danger-link"
                disabled={pool.saving || deleting}
                onClick={() =>
                  setDeleteTarget({ index: poolIndex, id: pool.id, name: pool.name.trim() || t('keyPool.title') })
                }
              >
                <Trash2 size={13} />
                {t('keyPool.deletePool')}
              </button>
            </div>
          </div>

          <div className="key-pool-wrap">
            <table className="key-pool-table">
              <thead>
                <tr>
                  <th>{t('keyPool.colKey')}</th>
                  <th>{t('keyPool.colEnabled')}</th>
                  <th>{t('keyPool.colRequests')}</th>
                  <th>{t('keyPool.colTokens')}</th>
                  <th>{t('keyPool.colLastError')}</th>
                  <th>{t('keyPool.colQuota')}</th>
                  <th>{t('keyPool.colActions')}</th>
                </tr>
              </thead>
              <tbody>
                {pool.keys.length === 0 && (
                  <tr>
                    <td colSpan={7} className="key-pool-empty">
                      {t('keyPool.emptyKeys')}
                    </td>
                  </tr>
                )}
                {pool.keys.map((key, keyIndex) => {
                  const over = keyOverQuota(key)
                  const rowClass = `${key.enabled ? '' : 'is-disabled'}${over ? ' is-over-quota' : ''}`.trim()
                  return (
                    <tr key={key.id || `new-${keyIndex}`} className={rowClass || undefined}>
                      <td>
                        <code className="key-pool-key" title={t('keyPool.maskedHint')}>
                          {key.keyMasked}
                        </code>
                      </td>
                      <td>
                        <input
                          className="key-pool-switch"
                          type="checkbox"
                          aria-label={t('keyPool.colEnabled')}
                          checked={key.enabled}
                          disabled={pool.saving}
                          onChange={(event) =>
                            patchKey(poolIndex, keyIndex, { enabled: event.target.checked })
                          }
                        />
                      </td>
                      <td>{key.usage.requestCount}</td>
                      <td>{key.usage.totalTokens}</td>
                      <td
                        className="key-pool-error-cell"
                        title={key.usage.lastError ?? undefined}
                      >
                        {key.usage.lastError ?? '-'}
                      </td>
                      <td>
                        <div className="key-pool-quota">
                          <input
                            type="number"
                            min={1}
                            value={key.requestLimit}
                            placeholder={t('keyPool.quotaRequestPlaceholder')}
                            aria-label={t('keyPool.quotaRequestPlaceholder')}
                            disabled={pool.saving}
                            onChange={(event) =>
                              patchKey(poolIndex, keyIndex, { requestLimit: event.target.value })
                            }
                          />
                          <input
                            type="number"
                            min={1}
                            value={key.tokenLimit}
                            placeholder={t('keyPool.quotaTokenPlaceholder')}
                            aria-label={t('keyPool.quotaTokenPlaceholder')}
                            disabled={pool.saving}
                            onChange={(event) =>
                              patchKey(poolIndex, keyIndex, { tokenLimit: event.target.value })
                            }
                          />
                        </div>
                        {over && <span className="key-pool-alert">{t('keyPool.quotaAlert')}</span>}
                      </td>
                      <td>
                        <div className="key-pool-row-actions">
                          <button
                            type="button"
                            className="ghost tiny-btn"
                            disabled={!pool.id || !key.id || pool.saving || resetting !== null || deleting}
                            onClick={() => void resetUsage(poolIndex, key.id)}
                          >
                            {t('keyPool.resetKey')}
                          </button>
                          <button
                            type="button"
                            className="ghost tiny-btn danger-link"
                            disabled={pool.saving}
                            onClick={() => removeKey(poolIndex, keyIndex)}
                          >
                            <Trash2 size={13} />
                            {t('keyPool.deleteKey')}
                          </button>
                        </div>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </div>

          <p className="hint compact key-pool-note">{t('keyPool.quotaHint')}</p>

          <div className="key-pool-add">
            <input
              value={pool.newKey}
              placeholder={t('keyPool.keyPlaceholder')}
              aria-label={t('keyPool.keyPlaceholder')}
              disabled={pool.saving}
              onChange={(event) => patchPool(poolIndex, { newKey: event.target.value, error: null })}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault()
                  addKey(poolIndex)
                }
              }}
            />
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={pool.saving}
              onClick={() => addKey(poolIndex)}
            >
              <Plus size={13} />
              {t('keyPool.addKey')}
            </button>
          </div>

          {pool.error && <p className="error-text key-pool-note">{pool.error}</p>}
        </section>
      ))}

      <ConfirmModal
        open={deleteTarget !== null}
        danger
        busy={deleting}
        title={t('keyPool.deleteConfirmTitle')}
        body={
          deleteTarget
            ? t('keyPool.deleteConfirmBody', { name: deleteTarget.name })
            : undefined
        }
        confirmLabel={t('keyPool.deletePool')}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleteTarget(null)}
      />
    </div>
  )
}
