/** 「接入密钥」tab：客户端连接本机网关时要携带的 key（入站鉴权），
 * 与「上游 Key 轮换」区分开——那个页面管的是 SUGT 调用上游服务商用的 key。
 * 命令契约：list_client_keys / create_client_key / update_client_key / delete_client_key / reset_client_key_usage，
 * 入参统一以 { input: {...} } 包装。删除 / 重置走 ConfirmModal，结果与失败统一 pushToast。
 * 主界面只做认钥 / 复制 / 启停 / 用量摘要：名称 + Key 脱敏 + 启用 + 用量一行 + ⋯ 菜单（详情 / 重置用量 / 删除）。
 * 创建走「添加密钥」弹窗，改 key / 配额 / 最近使用 / 最近错误 / 客户端明细都在详情弹窗里，
 * 全部绑定 list_client_keys 的真实数据，不渲染任何示例行。
 * 主密钥（primary）不可删除、不可停用，key 改完保存后广播 sugt:access-info-changed，
 * 让「接入信息卡」重新拉取 get_local_access_info。 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from './transport';
import { Copy, MoreHorizontal, Plus, RefreshCw, RotateCcw, Trash2 } from 'lucide-react'
import { ConfirmModal } from './ConfirmModal'
import { ModalPortal } from './ModalPortal'
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

/** ⋯ 菜单锚点：fixed 定位，经 ModalPortal 挂到 body，避免被卡片 / 滚动容器裁切。 */
type RowMenu = { id: string; right: number; top?: number; bottom?: number }

const EMPTY_QUOTA: ClientKeyQuota = { requestLimit: null, tokenLimit: null }
const EMPTY_DRAFT: QuotaDraft = { request: '', token: '' }

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
  const floored = Math.floor(value)
  // 超过 2^53 的数 JS 无法精确表示，传给后端 u64 会悄悄变样，按非法输入处理。
  return Number.isSafeInteger(floored) ? floored : undefined
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

/** Esc 关闭：只在 active 期间挂监听，避免影响同页其它弹窗。 */
function useEscClose(active: boolean, onClose: () => void) {
  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, onClose])
}

export function ClientKeyPanel({ pushToast }: Props) {
  const [keys, setKeys] = useState<ClientKeyView[]>([])
  const [loading, setLoading] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  // 添加弹窗 A
  const [addOpen, setAddOpen] = useState(false)
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
  // 详情弹窗 B：按 id 绑定列表里的真实行，草稿只在弹窗打开期间存在。
  const [detailId, setDetailId] = useState<string | null>(null)
  const [detailKey, setDetailKey] = useState('')
  const [detailQuota, setDetailQuota] = useState<QuotaDraft>(EMPTY_DRAFT)
  const [savingKey, setSavingKey] = useState(false)
  const [savingQuota, setSavingQuota] = useState(false)
  // 行内 ⋯ 菜单
  const [menu, setMenu] = useState<RowMenu | null>(null)
  const menuRef = useRef<HTMLDivElement>(null)

  const detail = detailId ? (keys.find((item) => item.id === detailId) ?? null) : null
  const menuEntry = menu ? (keys.find((item) => item.id === menu.id) ?? null) : null
  const confirmOpen = deleteTarget !== null || resetTarget !== null

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const list = await invoke<ClientKeyView[]>('list_client_keys')
      setKeys(list)
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

  // 详情对应的行被删除 / 列表读取失败后清空：关掉弹窗，不留悬空草稿。
  useEffect(() => {
    if (detailId && !loading && !keys.some((item) => item.id === detailId)) {
      setDetailId(null)
    }
  }, [detailId, keys, loading])

  const closeAdd = useCallback(() => {
    if (creating) return
    setAddOpen(false)
    setName('')
    setCustomKey('')
  }, [creating])

  const closeDetail = useCallback(() => {
    setDetailId(null)
    setDetailKey('')
  }, [])

  const closeMenu = useCallback(() => setMenu(null), [])

  // Esc：确认弹窗在最上层时由它自己处理，避免一次 Esc 同时关掉两层。
  useEscClose(addOpen, closeAdd)
  useEscClose(detail !== null && !confirmOpen, closeDetail)
  useEscClose(menu !== null, closeMenu)

  // ⋯ 菜单：点到菜单外 / 滚动 / 改窗口大小都关闭，菜单不跟随滚动。
  useEffect(() => {
    if (!menu) return
    const onDown = (event: MouseEvent) => {
      const target = event.target as Node | null
      if (target && menuRef.current?.contains(target)) return
      if (target instanceof Element && target.closest('[data-client-key-more]')) return
      setMenu(null)
    }
    const onAway = () => setMenu(null)
    window.addEventListener('mousedown', onDown)
    window.addEventListener('resize', onAway)
    window.addEventListener('scroll', onAway, true)
    return () => {
      window.removeEventListener('mousedown', onDown)
      window.removeEventListener('resize', onAway)
      window.removeEventListener('scroll', onAway, true)
    }
  }, [menu])

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
    setMenu(null)
    void load()
  }

  const openDetail = (entry: ClientKeyView) => {
    setMenu(null)
    setDetailId(entry.id)
    setDetailKey('')
    setDetailQuota(quotaToDraft(quotaOf(entry)))
  }

  const openMenu = (entry: ClientKeyView, anchor: HTMLElement) => {
    if (menu?.id === entry.id) {
      setMenu(null)
      return
    }
    const rect = anchor.getBoundingClientRect()
    const right = Math.max(8, window.innerWidth - rect.right)
    // 菜单约 3 项（~120px）；下方放不下就向上展开。
    if (rect.bottom + 140 > window.innerHeight) {
      setMenu({ id: entry.id, right, bottom: window.innerHeight - rect.top + 4 })
    } else {
      setMenu({ id: entry.id, right, top: rect.bottom + 4 })
    }
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
      setAddOpen(false)
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
    if (savingKey) return
    const next = detailKey.trim()
    if (!next) {
      pushToast(t('clientKeys.errorKeyRequired'), 'error')
      return
    }
    setSavingKey(true)
    try {
      const updated = await invoke<ClientKeyView>('update_client_key', {
        input: { id: entry.id, key: next },
      })
      setKeys((prev) => prev.map((item) => (item.id === updated.id ? updated : item)))
      setDetailKey('')
      pushToast(t('clientKeys.toastSaved'), 'ok')
      // 主密钥在后端会同步到 get_local_access_info，接入信息卡需要重新拉取。
      if (updated.primary ?? entry.primary) {
        window.dispatchEvent(new CustomEvent('sugt:access-info-changed'))
      }
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setSavingKey(false)
    }
  }

  const saveQuota = async (entry: ClientKeyView) => {
    if (savingQuota) return
    const requestLimit = parseLimit(detailQuota.request)
    const tokenLimit = parseLimit(detailQuota.token)
    if (requestLimit === undefined || tokenLimit === undefined) {
      pushToast(t('clientKeys.errorQuotaInvalid'), 'error')
      return
    }
    setSavingQuota(true)
    try {
      const updated = await invoke<ClientKeyView>('update_client_key', {
        input: { id: entry.id, quota: { requestLimit, tokenLimit } },
      })
      setKeys((prev) => prev.map((item) => (item.id === updated.id ? updated : item)))
      setDetailQuota(quotaToDraft(quotaOf(updated)))
      pushToast(t('clientKeys.toastSaved'), 'ok')
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setSavingQuota(false)
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
      setDetailId((prev) => (prev === target.id ? null : prev))
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

  const detailQuotaSaved = detail ? quotaOf(detail) : EMPTY_QUOTA
  const detailClients = detail?.usage.clients ?? []

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
            <button
              type="button"
              className="primary tiny-btn client-key-add-btn"
              disabled={deleting}
              onClick={() => setAddOpen(true)}
            >
              <Plus size={14} />
              {t('clientKeys.add')}
            </button>
          </div>
        </div>

        <p className="hint compact client-key-note">{t('clientKeys.hint')}</p>

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
          <div className="client-key-list" role="table" aria-label={t('clientKeys.title')}>
            <div className="client-key-row client-key-head" role="row">
              <span role="columnheader">{t('clientKeys.colName')}</span>
              <span role="columnheader">{t('clientKeys.colKey')}</span>
              <span role="columnheader">{t('clientKeys.colEnabled')}</span>
              <span role="columnheader">{t('clientKeys.colUsage')}</span>
              <span role="columnheader" className="client-key-sr-only">
                {t('clientKeys.colActions')}
              </span>
            </div>
            {keys.map((entry) => {
              const primary = Boolean(entry.primary)
              const quota = quotaOf(entry)
              const over =
                (quota.requestLimit !== null && entry.usage.requestCount > quota.requestLimit) ||
                (quota.tokenLimit !== null && entry.usage.totalTokens > quota.tokenLimit)
              const rowClass = [
                'client-key-row',
                entry.enabled ? '' : 'is-off',
                primary ? 'is-primary' : '',
              ]
                .filter(Boolean)
                .join(' ')
              return (
                <div
                  key={entry.id}
                  className={rowClass}
                  role="row"
                  onClick={() => openDetail(entry)}
                >
                  <div className="client-key-cell client-key-name-cell" role="cell">
                    <div className="client-key-name-inner">
                      <span className="client-key-name">{entry.name}</span>
                      {primary && (
                        <span className="client-key-primary-badge">
                          {t('clientKeys.primaryBadge')}
                        </span>
                      )}
                    </div>
                  </div>
                  <div
                    className="client-key-cell client-key-key-cell"
                    role="cell"
                    onClick={(event) => event.stopPropagation()}
                  >
                    <code className="client-key-code" title={entry.keyMasked}>
                      {entry.keyMasked}
                    </code>
                    <button
                      type="button"
                      className="ghost tiny icon-only"
                      aria-label={t('clientKeys.copy')}
                      title={t('clientKeys.copy')}
                      onClick={() => void copy(entry.key)}
                    >
                      <Copy size={13} />
                    </button>
                  </div>
                  <div
                    className="client-key-cell client-key-enabled-cell"
                    role="cell"
                    onClick={(event) => event.stopPropagation()}
                  >
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
                  </div>
                  <div className="client-key-cell client-key-usage-cell" role="cell">
                    <span className="client-key-usage-value">
                      {t('clientKeys.usageSummary', {
                        requests: formatCount(entry.usage.requestCount),
                        tokens: formatCount(entry.usage.totalTokens),
                      })}
                    </span>
                    {over && (
                      <span className="client-key-over-badge" title={t('clientKeys.overQuotaHint')}>
                        {t('clientKeys.overQuota')}
                      </span>
                    )}
                  </div>
                  <div
                    className="client-key-cell client-key-more-cell"
                    role="cell"
                    onClick={(event) => event.stopPropagation()}
                  >
                    <button
                      type="button"
                      data-client-key-more
                      className="ghost tiny icon-only"
                      aria-haspopup="menu"
                      aria-expanded={menu?.id === entry.id}
                      aria-label={t('clientKeys.more')}
                      title={t('clientKeys.more')}
                      onClick={(event) => openMenu(entry, event.currentTarget)}
                    >
                      <MoreHorizontal size={16} />
                    </button>
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </section>

      {menu && menuEntry && (
        <ModalPortal>
          <div
            ref={menuRef}
            className="client-key-menu"
            role="menu"
            style={{ right: menu.right, top: menu.top, bottom: menu.bottom }}
          >
            <button type="button" role="menuitem" onClick={() => openDetail(menuEntry)}>
              {t('clientKeys.detail')}
            </button>
            <button
              type="button"
              role="menuitem"
              disabled={resetting || deleting}
              onClick={() => {
                setMenu(null)
                setResetTarget({ id: menuEntry.id, name: menuEntry.name })
              }}
            >
              {t('clientKeys.resetUsage')}
            </button>
            {!menuEntry.primary && (
              <button
                type="button"
                role="menuitem"
                className="is-danger"
                disabled={deleting || toggling === menuEntry.id}
                onClick={() => {
                  setMenu(null)
                  setDeleteTarget({ id: menuEntry.id, name: menuEntry.name })
                }}
              >
                {t('clientKeys.delete')}
              </button>
            )}
          </div>
        </ModalPortal>
      )}

      {addOpen && (
        <ModalPortal>
          <div
            className="modal-overlay"
            onMouseDown={(event) => {
              if (event.target === event.currentTarget) closeAdd()
            }}
          >
            <form
              className="modal modal-compact"
              role="dialog"
              aria-modal="true"
              aria-label={t('clientKeys.addTitle')}
              onSubmit={(event) => {
                event.preventDefault()
                void create()
              }}
            >
              <div className="modal-head">
                <h3>{t('clientKeys.addTitle')}</h3>
              </div>
              <div className="modal-body">
                <label className="field-label">
                  {t('clientKeys.nameLabel')}
                  <input
                    autoFocus
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
              </div>
              <div className="modal-body client-key-modal-foot">
                <div className="modal-actions">
                  <button type="button" className="ghost" disabled={creating} onClick={closeAdd}>
                    {t('common.cancel')}
                  </button>
                  <button type="submit" className="primary" disabled={creating}>
                    {creating ? t('clientKeys.creating') : t('clientKeys.create')}
                  </button>
                </div>
              </div>
            </form>
          </div>
        </ModalPortal>
      )}

      {detail && (
        <ModalPortal>
          <div
            className="modal-overlay"
            onMouseDown={(event) => {
              if (event.target === event.currentTarget && !confirmOpen) closeDetail()
            }}
          >
            <div
              className="modal client-key-detail-modal"
              role="dialog"
              aria-modal="true"
              aria-label={t('clientKeys.detailTitle')}
            >
              <div className="modal-head">
                <h3 className="client-key-detail-heading">
                  <span className="client-key-name">{detail.name}</span>
                  {detail.primary && (
                    <span className="client-key-primary-badge">{t('clientKeys.primaryBadge')}</span>
                  )}
                </h3>
              </div>
              <div className="modal-body">
                <section className="client-key-section">
                  <span className="client-key-section-title">{t('clientKeys.colKey')}</span>
                  <code className="client-key-code" title={detail.keyMasked}>
                    {detail.keyMasked}
                  </code>
                  <div className="client-key-inline-form">
                    <input
                      className="field-input"
                      value={detailKey}
                      placeholder={t('clientKeys.newKeyLabel')}
                      autoComplete="off"
                      spellCheck={false}
                      disabled={savingKey}
                      onChange={(event) => setDetailKey(event.target.value)}
                    />
                    <button
                      type="button"
                      className="ghost tiny-btn"
                      disabled={savingKey}
                      onClick={() => void saveKey(detail)}
                    >
                      {t('clientKeys.saveKey')}
                    </button>
                  </div>
                  {detail.primary && (
                    <span className="client-key-sync-hint">{t('clientKeys.primarySyncHint')}</span>
                  )}
                </section>

                <section className="client-key-section">
                  <span className="client-key-section-title">{t('clientKeys.colQuota')}</span>
                  <div className="client-key-quota-grid">
                    <label className="field-label">
                      {t('clientKeys.requestLimit')}
                      <input
                        className="field-input"
                        inputMode="numeric"
                        value={detailQuota.request}
                        placeholder={t('clientKeys.limitUnlimited')}
                        disabled={savingQuota}
                        onChange={(event) =>
                          setDetailQuota((prev) => ({ ...prev, request: event.target.value }))
                        }
                      />
                      <span className="client-key-usage-value">
                        {formatCount(detail.usage.requestCount)}
                        {detailQuotaSaved.requestLimit !== null &&
                          ` / ${formatCount(detailQuotaSaved.requestLimit)}`}
                      </span>
                      <QuotaMeter
                        used={detail.usage.requestCount}
                        limit={detailQuotaSaved.requestLimit}
                      />
                    </label>
                    <label className="field-label">
                      {t('clientKeys.tokenLimit')}
                      <input
                        className="field-input"
                        inputMode="numeric"
                        value={detailQuota.token}
                        placeholder={t('clientKeys.limitUnlimited')}
                        disabled={savingQuota}
                        onChange={(event) =>
                          setDetailQuota((prev) => ({ ...prev, token: event.target.value }))
                        }
                      />
                      <span className="client-key-usage-value">
                        {formatCount(detail.usage.totalTokens)}
                        {detailQuotaSaved.tokenLimit !== null &&
                          ` / ${formatCount(detailQuotaSaved.tokenLimit)}`}
                      </span>
                      <QuotaMeter
                        used={detail.usage.totalTokens}
                        limit={detailQuotaSaved.tokenLimit}
                      />
                    </label>
                  </div>
                  <div className="client-key-inline-form">
                    <span className="client-key-limit-hint">{t('clientKeys.overQuotaHint')}</span>
                    <button
                      type="button"
                      className="ghost tiny-btn"
                      disabled={savingQuota}
                      onClick={() => void saveQuota(detail)}
                    >
                      {t('clientKeys.saveQuota')}
                    </button>
                  </div>
                </section>

                <section className="client-key-section">
                  <dl className="client-key-facts">
                    <dt>{t('clientKeys.colLastUsed')}</dt>
                    <dd>
                      {detail.usage.lastUsedAt
                        ? formatTime(detail.usage.lastUsedAt)
                        : t('clientKeys.neverUsed')}
                    </dd>
                    <dt>{t('clientKeys.colLastError')}</dt>
                    <dd className={detail.usage.lastError ? 'client-key-error' : undefined}>
                      {detail.usage.lastError ?? '—'}
                    </dd>
                  </dl>
                </section>

                <section className="client-key-section">
                  <span className="client-key-section-title">{t('clientKeys.clients')}</span>
                  {detailClients.length === 0 ? (
                    <span className="client-key-note">{t('clientKeys.noClients')}</span>
                  ) : (
                    <div className="client-key-client-wrap">
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
                          {detailClients.map((client, index) => (
                            <tr key={`${client.label}-${client.ip}-${index}`}>
                              <td>{client.label}</td>
                              <td>
                                <code>{client.ip}</code>
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
                    </div>
                  )}
                </section>

                <div className="modal-actions modal-actions-spread">
                  <div className="client-key-detail-danger">
                    <button
                      type="button"
                      className="ghost tiny-btn"
                      disabled={resetting || deleting}
                      onClick={() => setResetTarget({ id: detail.id, name: detail.name })}
                    >
                      <RotateCcw size={13} />
                      {t('clientKeys.resetUsage')}
                    </button>
                    {!detail.primary && (
                      <button
                        type="button"
                        className="ghost tiny-btn danger-link"
                        disabled={deleting || toggling === detail.id}
                        onClick={() => setDeleteTarget({ id: detail.id, name: detail.name })}
                      >
                        <Trash2 size={13} />
                        {t('clientKeys.delete')}
                      </button>
                    )}
                  </div>
                  <button type="button" className="ghost" onClick={closeDetail}>
                    {t('common.close')}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </ModalPortal>
      )}

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
