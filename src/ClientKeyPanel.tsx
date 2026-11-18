/** 「接入密钥」tab：客户端连接本机网关时要携带的 key（入站鉴权），
 * 与「上游 Key 轮换」区分开——那个页面管的是 SUGT 调用上游服务商用的 key。
 * 命令契约：list_client_keys / create_client_key / update_client_key / delete_client_key / reset_client_key_usage，
 * 入参统一以 { input: {...} } 包装。删除 / 重置走 ConfirmModal，结果与失败统一 pushToast。
 * 每行展示请求次数 / 累计 Token / 最近使用 / 配额用量，并可展开客户端明细（label + ip + 用量）。
 * 主密钥（primary）不可删除、不可停用，key 改完保存后广播 sugt:access-info-changed，
 * 让「接入信息卡」重新拉取 get_local_access_info。 */
import { Fragment, useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  ChevronDown,
  ChevronRight,
  Copy,
  KeyRound,
  RefreshCw,
  RotateCcw,
  Trash2,
} from 'lucide-react'
import { ConfirmModal } from './ConfirmModal'
import { t } from './i18n'

type ClientKeyQuota = {
  requestLimit: number | null
  tokenLimit: number | null
}

type ClientKeyClient = {
  label: string
  ip: string
  requestCount: number
  totalTokens: number
  lastSeenAt: string
}

type ClientKeyUsage = {
  requestCount: number
  totalTokens: number
  lastUsedAt: string | null
  lastError: string | null
  /** 旧版本后端可能没有这个字段，读取时统一兜底成空数组。 */
  clients?: ClientKeyClient[]
}

type ClientKeyView = {
  id: string
  name: string
  key: string
  keyMasked: string
  enabled: boolean
  primary: boolean
  /** 旧版本后端可能没有这个字段，读取时统一兜底成「不限」。 */
  quota?: ClientKeyQuota
  usage: ClientKeyUsage
}

type QuotaDraft = { request: string; token: string }

const EMPTY_QUOTA: ClientKeyQuota = { requestLimit: null, tokenLimit: null }

type Props = {
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void
}

/** 最近使用时间按本地时区展示；无法解析时退回原始字符串。 */
function formatTime(value: string): string {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

/** 计数按本地数字格式展示（千分位）。 */
function formatCount(value: number): string {
  return value.toLocaleString()
}

function quotaOf(entry: ClientKeyView): ClientKeyQuota {
  return entry.quota ?? EMPTY_QUOTA
}

function quotaToDraft(quota: ClientKeyQuota): QuotaDraft {
  return {
    request: quota.requestLimit === null ? '' : String(quota.requestLimit),
    token: quota.tokenLimit === null ? '' : String(quota.tokenLimit),
  }
}

/** 空串 = 不限（null）；非法输入返回 undefined，由调用方提示。 */
function parseLimit(raw: string): number | null | undefined {
  const text = raw.trim()
  if (!text) return null
  const value = Number(text)
  if (!Number.isFinite(value) || value < 0) return undefined
  return Math.floor(value)
}

/** 用量进度条：limit 为空时只留数字，超限改用 danger 色，仅提示不拦截请求。 */
function QuotaMeter({ used, limit }: { used: number; limit: number | null }) {
  if (limit === null) return null
  const over = used > limit
  const pct = limit <= 0 ? (used > 0 ? 100 : 0) : Math.min(100, (used / limit) * 100)
  return (
    <div className="client-key-meter" title={`${formatCount(used)} / ${formatCount(limit)}`}>
      <span
        className={over ? 'client-key-meter-fill is-over' : 'client-key-meter-fill'}
        style={{ width: `${pct}%` }}
      />
    </div>
  )
}

export function ClientKeyPanel({ pushToast }: Props) {
  const [keys, setKeys] = useState<ClientKeyView[]>([])
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [name, setName] = useState('')
  const [customKey, setCustomKey] = useState('')
  const [creating, setCreating] = useState(false)
  // 新建成功后完整 key 只在这里展示一次：不落盘、不随列表刷新复现，关掉即只剩脱敏值。
  const [revealed, setRevealed] = useState<{ id: string; key: string } | null>(null)
  const [toggling, setToggling] = useState<string | null>(null)
  const [deleteTarget, setDeleteTarget] = useState<{ id: string; name: string } | null>(null)
  const [deleting, setDeleting] = useState(false)
  const [resetTarget, setResetTarget] = useState<{ id: string; name: string } | null>(null)
  const [resetting, setResetting] = useState(false)
  // 行内改 key：只允许改非空值，保存后主密钥需要通知接入信息卡刷新。
  const [keyEdit, setKeyEdit] = useState<{ id: string; value: string } | null>(null)
  const [savingKey, setSavingKey] = useState<string | null>(null)
  // 配额输入草稿（留空 = 不限），按 key id 保存，避免整表受控输入互相拖慢。
  const [quotaDraft, setQuotaDraft] = useState<Record<string, QuotaDraft>>({})
  const [savingQuota, setSavingQuota] = useState<string | null>(null)
  // 客户端明细展开状态。
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const list = await invoke<ClientKeyView[]>('list_client_keys')
      setKeys(list)
      setQuotaDraft(
        Object.fromEntries(list.map((entry) => [entry.id, quotaToDraft(quotaOf(entry))])),
      )
    } catch (error) {
      // 读取失败保留错误行供重试，不清空已展示数据之外的任何本地状态。
      setKeys([])
      setLoadError(String(error))
    } finally {
      setLoading(false)
    }
  }, [])

  // 切 tab 会卸载再挂载，挂载拉一次即等价于切回来刷新。
  useEffect(() => {
    void load()
  }, [load])

  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      pushToast(t('clientKeys.toastCopied'), 'ok')
    } catch (error) {
      pushToast(String(error), 'error')
    }
  }

  const refresh = () => {
    setRevealed(null)
    setKeyEdit(null)
    void load()
  }

  const create = async () => {
    if (creating) return
    const trimmed = name.trim()
    if (!trimmed) {
      pushToast(t('clientKeys.errorNameRequired'), 'error')
      return
    }
    setCreating(true)
    try {
      const input: { name: string; key?: string } = { name: trimmed }
      const custom = customKey.trim()
      if (custom) input.key = custom
      const created = await invoke<ClientKeyView>('create_client_key', { input })
      setName('')
      setCustomKey('')
      setRevealed({ id: created.id, key: created.key })
      pushToast(t('clientKeys.toastCreated'), 'ok')
      await load()
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setCreating(false)
    }
  }

  const toggle = async (entry: ClientKeyView) => {
    if (toggling || entry.primary) return
    setToggling(entry.id)
    try {
      const updated = await invoke<ClientKeyView>('update_client_key', {
        input: { id: entry.id, enabled: !entry.enabled },
      })
      setKeys((prev) => prev.map((item) => (item.id === updated.id ? updated : item)))
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setToggling(null)
    }
  }

  const saveKey = async (entry: ClientKeyView) => {
    if (!keyEdit || keyEdit.id !== entry.id || savingKey) return
    const next = keyEdit.value.trim()
    if (!next) {
      pushToast(t('clientKeys.errorKeyRequired'), 'error')
      return
    }
    setSavingKey(entry.id)
    try {
      const updated = await invoke<ClientKeyView>('update_client_key', {
        input: { id: entry.id, key: next },
      })
      setKeys((prev) => prev.map((item) => (item.id === updated.id ? updated : item)))
      setQuotaDraft((prev) => ({
        ...prev,
        [updated.id]: quotaToDraft(quotaOf(updated)),
      }))
      setKeyEdit(null)
      pushToast(t('clientKeys.toastSaved'), 'ok')
      // 主密钥在后端会同步到 get_local_access_info，接入信息卡需要重新拉取。
      if (updated.primary ?? entry.primary) {
        window.dispatchEvent(new CustomEvent('sugt:access-info-changed'))
      }
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setSavingKey(null)
    }
  }

  const saveQuota = async (entry: ClientKeyView) => {
    if (savingQuota) return
    const draft = quotaDraft[entry.id] ?? quotaToDraft(quotaOf(entry))
    const requestLimit = parseLimit(draft.request)
    const tokenLimit = parseLimit(draft.token)
    if (requestLimit === undefined || tokenLimit === undefined) {
      pushToast(t('clientKeys.errorQuotaInvalid'), 'error')
      return
    }
    setSavingQuota(entry.id)
    try {
      const updated = await invoke<ClientKeyView>('update_client_key', {
        input: { id: entry.id, quota: { requestLimit, tokenLimit } },
      })
      setKeys((prev) => prev.map((item) => (item.id === updated.id ? updated : item)))
      setQuotaDraft((prev) => ({
        ...prev,
        [updated.id]: quotaToDraft(quotaOf(updated)),
      }))
      pushToast(t('clientKeys.toastSaved'), 'ok')
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setSavingQuota(null)
    }
  }

  const confirmDelete = async () => {
    if (!deleteTarget) return
    const target = deleteTarget
    setDeleteTarget(null)
    setDeleting(true)
    try {
      await invoke('delete_client_key', { input: { id: target.id } })
      pushToast(t('clientKeys.toastDeleted'), 'ok')
      setRevealed((prev) => (prev?.id === target.id ? null : prev))
      setKeyEdit((prev) => (prev?.id === target.id ? null : prev))
      await load()
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setDeleting(false)
    }
  }

  const confirmReset = async () => {
    if (!resetTarget) return
    const target = resetTarget
    setResetTarget(null)
    setResetting(true)
    try {
      await invoke('reset_client_key_usage', { input: { id: target.id } })
      pushToast(t('clientKeys.toastReset'), 'ok')
      await load()
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setResetting(false)
    }
  }

  const toggleExpanded = (id: string) => {
    setExpanded((prev) => ({ ...prev, [id]: !prev[id] }))
  }

  return (
    <div className="client-key-panel">
      <section className="card">
        <div className="section-title">
          <h3>{t('clientKeys.title')}</h3>
          <div className="title-actions">
            <button
              type="button"
              className="ghost tiny-btn"
              disabled={loading || creating || deleting}
              onClick={refresh}
            >
              <RefreshCw size={14} className={loading ? 'spin' : ''} />
              {t('clientKeys.refresh')}
            </button>
          </div>
        </div>

        <p className="hint compact client-key-note">{t('clientKeys.hint')}</p>

        <form
          className="client-key-form"
          onSubmit={(event) => {
            event.preventDefault()
            void create()
          }}
        >
          <label className="field-label">
            {t('clientKeys.nameLabel')}
            <input
              value={name}
              placeholder={t('clientKeys.namePlaceholder')}
              disabled={creating}
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          <label className="field-label">
            {t('clientKeys.keyLabel')}
            <input
              value={customKey}
              placeholder={t('clientKeys.keyPlaceholder')}
              autoComplete="off"
              spellCheck={false}
              disabled={creating}
              onChange={(event) => setCustomKey(event.target.value)}
            />
          </label>
          <button type="submit" className="primary tiny-btn client-key-submit" disabled={creating}>
            {creating ? t('clientKeys.creating') : t('clientKeys.create')}
          </button>
        </form>

        {revealed && (
          <div className="client-key-reveal">
            <div className="client-key-reveal-head">
              <span className="client-key-reveal-title">{t('clientKeys.revealTitle')}</span>
              <button
                type="button"
                className="ghost tiny-btn"
                onClick={() => setRevealed(null)}
              >
                {t('clientKeys.revealDismiss')}
              </button>
            </div>
            <p className="hint compact client-key-note">{t('clientKeys.revealHint')}</p>
            <div className="client-key-reveal-row">
              <code className="client-key-reveal-code">{revealed.key}</code>
              <button
                type="button"
                className="tiny icon-only"
                aria-label={t('clientKeys.copy')}
                title={t('clientKeys.copy')}
                onClick={() => void copy(revealed.key)}
              >
                <Copy size={14} />
              </button>
            </div>
          </div>
        )}

        {loadError && (
          <div className="request-rules-error">
            <p className="error-text client-key-note">
              {t('clientKeys.loadFailed', { error: loadError })}
            </p>
            <button type="button" className="ghost tiny-btn" disabled={loading} onClick={refresh}>
              {t('clientKeys.retry')}
            </button>
          </div>
        )}
      </section>

      <section className="card">
        {!loading && !loadError && keys.length === 0 && (
          <p className="hint client-key-empty-text">{t('clientKeys.empty')}</p>
        )}

        {keys.length > 0 && (
          <div className="client-key-table-wrap">
            <table className="client-key-table">
              <thead>
                <tr>
                  <th aria-label={t('clientKeys.clients')} />
                  <th>{t('clientKeys.colName')}</th>
                  <th>{t('clientKeys.colKey')}</th>
                  <th>{t('clientKeys.colEnabled')}</th>
                  <th>{t('clientKeys.colRequests')}</th>
                  <th>{t('clientKeys.colTokens')}</th>
                  <th>{t('clientKeys.colLastUsed')}</th>
                  <th>{t('clientKeys.colLastError')}</th>
                  <th>{t('clientKeys.colQuota')}</th>
                  <th>{t('clientKeys.colActions')}</th>
                </tr>
              </thead>
              <tbody>
                {keys.map((entry) => {
                  const primary = Boolean(entry.primary)
                  const quota = quotaOf(entry)
                  const clients = entry.usage.clients ?? []
                  const editing = keyEdit?.id === entry.id
                  const draft = quotaDraft[entry.id] ?? quotaToDraft(quota)
                  const requestOver =
                    quota.requestLimit !== null && entry.usage.requestCount > quota.requestLimit
                  const tokenOver =
                    quota.tokenLimit !== null && entry.usage.totalTokens > quota.tokenLimit
                  const isExpanded = Boolean(expanded[entry.id])
                  const rowClass = [entry.enabled ? '' : 'is-off', primary ? 'is-primary' : '']
                    .filter(Boolean)
                    .join(' ')
                  return (
                    <Fragment key={entry.id}>
                      <tr className={rowClass || undefined}>
                        <td>
                          <button
                            type="button"
                            className="ghost tiny icon-only client-key-expand-btn"
                            aria-expanded={isExpanded}
                            aria-label={
                              isExpanded
                                ? t('clientKeys.collapse')
                                : t('clientKeys.expand')
                            }
                            title={isExpanded ? t('clientKeys.collapse') : t('clientKeys.expand')}
                            onClick={() => toggleExpanded(entry.id)}
                          >
                            {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                          </button>
                        </td>
                        <td className="client-key-name-cell">
                          <span className="client-key-name">{entry.name}</span>
                          {primary && (
                            <span className="client-key-primary-badge">
                              {t('clientKeys.primaryBadge')}
                            </span>
                          )}
                        </td>
                        <td>
                          {editing ? (
                            <div className="client-key-edit-cell">
                              <input
                                className="field-input"
                                value={keyEdit?.value ?? ''}
                                placeholder={t('clientKeys.newKeyLabel')}
                                autoComplete="off"
                                spellCheck={false}
                                disabled={savingKey === entry.id}
                                onChange={(event) =>
                                  setKeyEdit({ id: entry.id, value: event.target.value })
                                }
                              />
                              <div className="client-key-edit-actions">
                                <button
                                  type="button"
                                  className="primary tiny-btn"
                                  disabled={savingKey === entry.id}
                                  onClick={() => void saveKey(entry)}
                                >
                                  {t('clientKeys.save')}
                                </button>
                                <button
                                  type="button"
                                  className="ghost tiny-btn"
                                  disabled={savingKey === entry.id}
                                  onClick={() => setKeyEdit(null)}
                                >
                                  {t('common.cancel')}
                                </button>
                              </div>
                              {primary && (
                                <span className="client-key-sync-hint">
                                  {t('clientKeys.primarySyncHint')}
                                </span>
                              )}
                            </div>
                          ) : (
                            <code className="client-key-code">{entry.keyMasked}</code>
                          )}
                        </td>
                        <td>
                          {primary ? (
                            <span className="client-key-disabled-note">
                              {t('clientKeys.primaryCannotDisable')}
                            </span>
                          ) : (
                            <input
                              className="client-key-switch"
                              type="checkbox"
                              aria-label={
                                entry.enabled
                                  ? t('clientKeys.toggleDisable')
                                  : t('clientKeys.toggleEnable')
                              }
                              checked={entry.enabled}
                              disabled={toggling === entry.id || deleting}
                              onChange={() => void toggle(entry)}
                            />
                          )}
                        </td>
                        <td>
                          <div className="client-key-usage">
                            <span className="client-key-usage-value">
                              {formatCount(entry.usage.requestCount)}
                            </span>
                            <QuotaMeter used={entry.usage.requestCount} limit={quota.requestLimit} />
                            {requestOver && (
                              <span
                                className="client-key-over-badge"
                                title={t('clientKeys.overQuotaHint')}
                              >
                                {t('clientKeys.overQuota')}
                              </span>
                            )}
                          </div>
                        </td>
                        <td>
                          <div className="client-key-usage">
                            <span className="client-key-usage-value">
                              {formatCount(entry.usage.totalTokens)}
                            </span>
                            <QuotaMeter used={entry.usage.totalTokens} limit={quota.tokenLimit} />
                            {tokenOver && (
                              <span
                                className="client-key-over-badge"
                                title={t('clientKeys.overQuotaHint')}
                              >
                                {t('clientKeys.overQuota')}
                              </span>
                            )}
                          </div>
                        </td>
                        <td className="client-key-last-used">
                          {entry.usage.lastUsedAt
                            ? formatTime(entry.usage.lastUsedAt)
                            : t('clientKeys.neverUsed')}
                        </td>
                        <td className="client-key-error-cell">
                          {entry.usage.lastError ? (
                            <span className="client-key-error" title={entry.usage.lastError}>
                              {entry.usage.lastError}
                            </span>
                          ) : (
                            '-'
                          )}
                        </td>
                        <td>
                          <div className="client-key-quota-cell">
                            <div className="client-key-quota-inputs">
                              <input
                                className="field-input"
                                inputMode="numeric"
                                value={draft.request}
                                placeholder={t('clientKeys.requestLimit')}
                                title={t('clientKeys.limitUnlimited')}
                                disabled={savingQuota === entry.id}
                                onChange={(event) =>
                                  setQuotaDraft((prev) => ({
                                    ...prev,
                                    [entry.id]: { ...draft, request: event.target.value },
                                  }))
                                }
                              />
                              <input
                                className="field-input"
                                inputMode="numeric"
                                value={draft.token}
                                placeholder={t('clientKeys.tokenLimit')}
                                title={t('clientKeys.limitUnlimited')}
                                disabled={savingQuota === entry.id}
                                onChange={(event) =>
                                  setQuotaDraft((prev) => ({
                                    ...prev,
                                    [entry.id]: { ...draft, token: event.target.value },
                                  }))
                                }
                              />
                            </div>
                            <span className="client-key-limit-hint">
                              {t('clientKeys.limitUnlimited')}
                            </span>
                            <button
                              type="button"
                              className="ghost tiny-btn"
                              disabled={savingQuota === entry.id}
                              onClick={() => void saveQuota(entry)}
                            >
                              {t('clientKeys.save')}
                            </button>
                          </div>
                        </td>
                        <td>
                          <div className="client-key-row-actions">
                            <button
                              type="button"
                              className="ghost tiny-btn"
                              aria-label={t('clientKeys.copy')}
                              title={t('clientKeys.copy')}
                              onClick={() => void copy(entry.key)}
                            >
                              <Copy size={13} />
                              {t('clientKeys.copy')}
                            </button>
                            <button
                              type="button"
                              className="ghost tiny-btn"
                              disabled={savingKey === entry.id}
                              onClick={() =>
                                setKeyEdit({ id: entry.id, value: '' })
                              }
                            >
                              <KeyRound size={13} />
                              {t('clientKeys.editKey')}
                            </button>
                            <button
                              type="button"
                              className="ghost tiny-btn"
                              disabled={resetting || deleting}
                              onClick={() => setResetTarget({ id: entry.id, name: entry.name })}
                            >
                              <RotateCcw size={13} />
                              {t('clientKeys.resetUsage')}
                            </button>
                            {!primary && (
                              <button
                                type="button"
                                className="ghost tiny-btn danger-link"
                                disabled={deleting || toggling === entry.id}
                                onClick={() =>
                                  setDeleteTarget({ id: entry.id, name: entry.name })
                                }
                              >
                                <Trash2 size={13} />
                                {t('clientKeys.delete')}
                              </button>
                            )}
                          </div>
                        </td>
                      </tr>
                      {isExpanded && (
                        <tr className="client-key-detail-row">
                          <td colSpan={10}>
                            <div className="client-key-detail">
                              <span className="client-key-detail-title">
                                {t('clientKeys.clients')}
                              </span>
                              {clients.length === 0 ? (
                                <span className="client-key-note">{t('clientKeys.noClients')}</span>
                              ) : (
                                <table className="client-key-client-table">
                                  <thead>
                                    <tr>
                                      <th>{t('clientKeys.colClient')}</th>
                                      <th>{t('clientKeys.colIp')}</th>
                                      <th>{t('clientKeys.colRequests')}</th>
                                      <th>{t('clientKeys.colTokens')}</th>
                                      <th>{t('clientKeys.colLastSeen')}</th>
                                    </tr>
                                  </thead>
                                  <tbody>
                                    {clients.map((client, index) => (
                                      <tr key={`${client.label}-${client.ip}-${index}`}>
                                        <td>{client.label}</td>
                                        <td>
                                          <code className="client-key-code">{client.ip}</code>
                                        </td>
                                        <td>{formatCount(client.requestCount)}</td>
                                        <td>{formatCount(client.totalTokens)}</td>
                                        <td className="client-key-client-time">
                                          {formatTime(client.lastSeenAt)}
                                        </td>
                                      </tr>
                                    ))}
                                  </tbody>
                                </table>
                              )}
                            </div>
                          </td>
                        </tr>
                      )}
                    </Fragment>
                  )
                })}
              </tbody>
            </table>
          </div>
        )}
      </section>

      <ConfirmModal
        open={deleteTarget !== null}
        danger
        busy={deleting}
        title={t('clientKeys.deleteConfirmTitle')}
        body={
          deleteTarget
            ? t('clientKeys.deleteConfirmBody', { name: deleteTarget.name })
            : undefined
        }
        confirmLabel={t('clientKeys.delete')}
        onConfirm={() => void confirmDelete()}
        onCancel={() => setDeleteTarget(null)}
      />

      <ConfirmModal
        open={resetTarget !== null}
        busy={resetting}
        title={t('clientKeys.resetConfirmTitle')}
        body={
          resetTarget
            ? t('clientKeys.resetConfirmBody', { name: resetTarget.name })
            : undefined
        }
        confirmLabel={t('clientKeys.resetUsage')}
        onConfirm={() => void confirmReset()}
        onCancel={() => setResetTarget(null)}
      />
    </div>
  )
}
