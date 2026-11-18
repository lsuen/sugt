/** 「接入密钥」tab：客户端连接本机网关时要携带的 key（入站鉴权），
 * 与「上游 Key 轮换」区分开——那个页面管的是 SUGT 调用上游服务商用的 key。
 * 命令契约：list_client_keys / create_client_key / update_client_key / delete_client_key，
 * 入参统一以 { input: {...} } 包装。删除走 ConfirmModal，结果与失败统一 pushToast。 */
import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Copy, Plus, RefreshCw, Trash2 } from 'lucide-react'
import { ConfirmModal } from './ConfirmModal'
import { t } from './i18n'

type ClientKeyUsage = {
  requestCount: number
  lastUsedAt: string | null
  lastError: string | null
}

type ClientKeyView = {
  id: string
  name: string
  key: string
  keyMasked: string
  enabled: boolean
  usage: ClientKeyUsage
}

type Props = {
  pushToast: (msg: string, type?: 'ok' | 'error' | 'info') => void
}

/** 最近使用时间按本地时区展示；无法解析时退回原始字符串。 */
function formatTime(value: string): string {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
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
    if (toggling) return
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

  const confirmDelete = async () => {
    if (!deleteTarget) return
    const target = deleteTarget
    setDeleteTarget(null)
    setDeleting(true)
    try {
      await invoke('delete_client_key', { input: { id: target.id } })
      pushToast(t('clientKeys.toastDeleted'), 'ok')
      setRevealed((prev) => (prev?.id === target.id ? null : prev))
      await load()
    } catch (error) {
      pushToast(String(error), 'error')
    } finally {
      setDeleting(false)
    }
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
                  <th>{t('clientKeys.colName')}</th>
                  <th>{t('clientKeys.colKey')}</th>
                  <th>{t('clientKeys.colEnabled')}</th>
                  <th>{t('clientKeys.colRequests')}</th>
                  <th>{t('clientKeys.colLastUsed')}</th>
                  <th>{t('clientKeys.colLastError')}</th>
                  <th>{t('clientKeys.colActions')}</th>
                </tr>
              </thead>
              <tbody>
                {keys.map((entry) => (
                  <tr key={entry.id} className={entry.enabled ? undefined : 'is-off'}>
                    <td className="client-key-name">{entry.name}</td>
                    <td>
                      <code className="client-key-code">{entry.keyMasked}</code>
                    </td>
                    <td>
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
                    </td>
                    <td>{entry.usage.requestCount}</td>
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
                          className="ghost tiny-btn danger-link"
                          disabled={deleting || toggling === entry.id}
                          onClick={() => setDeleteTarget({ id: entry.id, name: entry.name })}
                        >
                          <Trash2 size={13} />
                          {t('clientKeys.delete')}
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
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
    </div>
  )
}
